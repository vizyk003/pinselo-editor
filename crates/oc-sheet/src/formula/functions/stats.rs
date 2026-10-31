// SPDX-License-Identifier: AGPL-3.0-only
//! Aggregations: NumericAggregationPlugin (SUM, COUNT, AVERAGE, VAR/STDEV, …, SUBTOTAL — evaluated over the
//! argument ASTs like HyperFormula's `reduce`), ConditionalAggregationPlugin (SUMIF(S), COUNTIF(S),
//! AVERAGEIF, MINIFS, MAXIFS), MedianPlugin, PercentilePlugin, CountBlank, CountUnique, SUMPRODUCT and the
//! descriptive part of StatisticalAggregationPlugin.

use super::super::ast::Ast;
use super::super::coerce::{self, eq_matcher, float_cmp, neq_matcher};
use super::super::func::{Arg, ArgType, Call, Meta, Param};
use super::super::interp::{Interp, State};
use super::super::value::{ErrorType, NumType, RangeVal, Scalar, Val};
use super::{Custom, Kind, err, exact_numbers, js_pow, num, numbers_exact_ranges, simple};

// ------------------------------------------------------------------ NumericAggregationPlugin

#[derive(Clone, Copy, Debug)]
struct Moments {
    sumsq: f64,
    sum: f64,
    count: f64,
}

impl Moments {
    const EMPTY: Moments = Moments { sumsq: 0.0, sum: 0.0, count: 0.0 };
    fn single(x: f64) -> Moments {
        Moments { sumsq: x * x, sum: x, count: 1.0 }
    }
    fn compose(self, o: Moments) -> Moments {
        Moments { sumsq: self.sumsq + o.sumsq, sum: self.sum + o.sum, count: self.count + o.count }
    }
    fn average(&self) -> Option<f64> {
        (self.count > 0.0).then(|| self.sum / self.count)
    }
    fn var_s(&self) -> Option<f64> {
        (self.count > 1.0).then(|| (self.sumsq - self.sum * self.sum / self.count) / (self.count - 1.0))
    }
    fn var_p(&self) -> Option<f64> {
        (self.count > 0.0).then(|| (self.sumsq - self.sum * self.sum / self.count) / self.count)
    }
}

/// A coercion function: `None` = skip, `Some(Err)` = error, `Some(Ok(x))` = number.
type Coercion = fn(&Scalar) -> Option<Result<f64, ErrorType>>;

fn strictly_numbers(s: &Scalar) -> Option<Result<f64, ErrorType>> {
    match s {
        Scalar::Num(x, _) => Some(Ok(*x)),
        Scalar::Err(e) => Some(Err(*e)),
        _ => None,
    }
}

fn numbers_booleans(s: &Scalar) -> Option<Result<f64, ErrorType>> {
    match s {
        Scalar::Bool(b) => Some(Ok(if *b { 1.0 } else { 0.0 })),
        Scalar::Num(x, _) => Some(Ok(*x)),
        Scalar::Err(e) => Some(Err(*e)),
        Scalar::Str(_) => Some(Ok(0.0)),
        Scalar::Empty => None,
    }
}

fn count_numbers(s: &Scalar) -> Option<Result<f64, ErrorType>> {
    Some(Ok(if s.is_number() { 1.0 } else { 0.0 }))
}

fn count_non_empty(s: &Scalar) -> Option<Result<f64, ErrorType>> {
    Some(Ok(if matches!(s, Scalar::Empty) { 0.0 } else { 1.0 }))
}

fn range_cells(it: &Interp, ast: &Ast, state: &State) -> Result<Vec<Scalar>, ErrorType> {
    let r = it.abs_range(ast, state)?;
    let empty_axis = match ast {
        Ast::ColRange { .. } => it.env.sheet_height(r.sheet) == 0,
        Ast::RowRange { .. } => it.env.sheet_width(r.sheet) == 0,
        _ => false,
    };
    if empty_axis {
        return Ok(vec![]);
    }
    let mut out = Vec::with_capacity(r.height() * r.width());
    for row in r.r0..=r.r1 {
        for col in r.c0..=r.c1 {
            out.push(it.env.cell(r.sheet, row, col));
        }
    }
    Ok(out)
}

/// HyperFormula's `NumericAggregationPlugin.reduce`.
fn reduce<T: Copy>(
    it: &Interp,
    args: &[Ast],
    state: &State,
    init: T,
    reducing: impl Fn(T, T) -> T,
    map: impl Fn(f64) -> T,
    coercion: Coercion,
) -> Result<T, ErrorType> {
    if args.is_empty() {
        return Err(ErrorType::Na);
    }
    let mut acc = init;
    for arg in args {
        if matches!(arg, Ast::Range { .. } | Ast::ColRange { .. } | Ast::RowRange { .. }) {
            let cells = range_cells(it, arg, state)?;
            let mut val: Result<T, ErrorType> = Ok(init);
            for c in &cells {
                let Some(x) = coercion(c) else { continue };
                val = match (val, x) {
                    (Err(e), _) => Err(e),
                    (Ok(_), Err(e)) => Err(e),
                    (Ok(v), Ok(n)) => Ok(reducing(v, map(n))),
                };
            }
            acc = reducing(val?, acc);
            continue;
        }
        let value = it.eval(arg, state);
        match value {
            Val::R(r) => {
                let mut left: Result<T, ErrorType> = Ok(acc);
                for v in r.values() {
                    let Some(x) = coercion(v) else { continue };
                    left = match (left, x) {
                        (Err(e), _) => Err(e),
                        (Ok(_), Err(e)) => Err(e),
                        (Ok(l), Ok(n)) => Ok(reducing(l, map(n))),
                    };
                }
                acc = left?;
            }
            Val::S(sv) => {
                let c = if matches!(arg, Ast::Cell { .. }) {
                    coercion(&sv)
                } else {
                    coercion(&coerce::to_number_or_error(&sv))
                };
                match c {
                    None => {}
                    Some(Err(e)) => return Err(e),
                    Some(Ok(n)) => acc = reducing(acc, map(n)),
                }
            }
        }
    }
    Ok(acc)
}

fn zero_for_infinite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

fn res(r: Result<f64, ErrorType>) -> Val {
    match r {
        Ok(v) => num(v),
        Err(e) => err(e),
    }
}

fn moments_to(r: Result<Moments, ErrorType>, f: impl Fn(&Moments) -> Option<f64>) -> Val {
    match r {
        Err(e) => err(e),
        Ok(m) => f(&m).map_or(err(ErrorType::Div0), num),
    }
}

fn do_sum(it: &Interp, a: &[Ast], s: &State) -> Val {
    res(reduce(it, a, s, 0.0, coerce::add_raw, |x| x, strictly_numbers))
}
fn do_count(it: &Interp, a: &[Ast], s: &State) -> Val {
    res(reduce(it, a, s, 0.0, |l, r| l + r, |x| x, count_numbers))
}
fn do_counta(it: &Interp, a: &[Ast], s: &State) -> Val {
    res(reduce(it, a, s, 0.0, |l, r| l + r, |x| x, count_non_empty))
}
fn do_max(it: &Interp, a: &[Ast], s: &State) -> Val {
    res(reduce(it, a, s, f64::NEG_INFINITY, js_max, |x| x, strictly_numbers).map(zero_for_infinite))
}
fn do_min(it: &Interp, a: &[Ast], s: &State) -> Val {
    res(reduce(it, a, s, f64::INFINITY, js_min, |x| x, strictly_numbers).map(zero_for_infinite))
}
fn do_product(it: &Interp, a: &[Ast], s: &State) -> Val {
    res(reduce(it, a, s, 1.0, |l, r| l * r, |x| x, strictly_numbers))
}
fn aggregate(it: &Interp, a: &[Ast], s: &State) -> Result<Moments, ErrorType> {
    reduce(it, a, s, Moments::EMPTY, Moments::compose, Moments::single, strictly_numbers)
}
fn aggregate_a(it: &Interp, a: &[Ast], s: &State) -> Result<Moments, ErrorType> {
    reduce(it, a, s, Moments::EMPTY, Moments::compose, Moments::single, numbers_booleans)
}
fn do_average(it: &Interp, a: &[Ast], s: &State) -> Val {
    moments_to(aggregate(it, a, s), Moments::average)
}
fn do_var_s(it: &Interp, a: &[Ast], s: &State) -> Val {
    moments_to(aggregate(it, a, s), Moments::var_s)
}
fn do_var_p(it: &Interp, a: &[Ast], s: &State) -> Val {
    moments_to(aggregate(it, a, s), Moments::var_p)
}
fn do_stdev_s(it: &Interp, a: &[Ast], s: &State) -> Val {
    moments_to(aggregate(it, a, s), |m| m.var_s().map(f64::sqrt))
}
fn do_stdev_p(it: &Interp, a: &[Ast], s: &State) -> Val {
    moments_to(aggregate(it, a, s), |m| m.var_p().map(f64::sqrt))
}

/// JS `Math.max` (NaN-propagating).
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

fn subtotal(it: &Interp, args: &[Ast], state: &State, _: &Meta) -> Val {
    if args.len() < 2 {
        return err(ErrorType::Na);
    }
    let v = it.eval(&args[0], state);
    let p = Param { ty: ArgType::Number, ..Param::default() };
    let t = match v {
        Val::S(sv) => match coerce::to_number_or_error(&sv) {
            Scalar::Num(x, _) => Some(x),
            _ => None,
        },
        Val::R(r) => super::super::func::range_to_scalar(&r, state).and_then(|sv| coerce::to_number_or_error(&sv).as_num()),
    };
    let _ = p;
    let rest = &args[1..];
    match t.map(|x| x as i64).filter(|x| t == Some(*x as f64)) {
        Some(1) | Some(101) => do_average(it, rest, state),
        Some(2) | Some(102) => do_count(it, rest, state),
        Some(3) | Some(103) => do_counta(it, rest, state),
        Some(4) | Some(104) => do_max(it, rest, state),
        Some(5) | Some(105) => do_min(it, rest, state),
        Some(6) | Some(106) => do_product(it, rest, state),
        Some(7) | Some(107) => do_stdev_s(it, rest, state),
        Some(8) | Some(108) => do_stdev_p(it, rest, state),
        Some(9) | Some(109) => do_sum(it, rest, state),
        Some(10) | Some(110) => do_var_s(it, rest, state),
        Some(11) | Some(111) => do_var_p(it, rest, state),
        _ => err(ErrorType::Value),
    }
}

// ------------------------------------------------------------------ ConditionalAggregationPlugin

enum CritValue {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
}

#[derive(Clone, Copy, PartialEq)]
enum CritOp {
    Gt,
    Ge,
    Lt,
    Le,
    Ne,
    Eq,
}

/// A parsed criterion as a predicate over raw cell values.
pub struct Criterion {
    f: Box<dyn Fn(&Scalar) -> bool>,
}

impl Criterion {
    pub fn test(&self, v: &Scalar) -> bool {
        (self.f)(v)
    }
}

/// `CriterionBuilder.fromCellValue`.
pub fn build_criterion(raw: &Scalar) -> Option<Criterion> {
    let (op, value) = match raw {
        Scalar::Num(x, _) => (CritOp::Eq, CritValue::Num(*x)),
        Scalar::Bool(b) => (CritOp::Eq, CritValue::Bool(*b)),
        Scalar::Str(s) => {
            // /([<>=]+)(.*)/ — unanchored; `.` stops at line terminators.
            let chars: Vec<char> = s.chars().collect();
            let found = chars.iter().position(|c| matches!(c, '<' | '>' | '='));
            let (op_str, val): (Option<String>, String) = match found {
                Some(i) => {
                    let mut j = i;
                    while j < chars.len() && matches!(chars[j], '<' | '>' | '=') {
                        j += 1;
                    }
                    let mut k = j;
                    while k < chars.len() && !matches!(chars[k], '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                        k += 1;
                    }
                    (Some(chars[i..j].iter().collect()), chars[j..k].iter().collect())
                }
                None => (None, s.to_string()),
            };
            let op = match op_str.as_deref() {
                None => Some(CritOp::Eq),
                Some(">") => Some(CritOp::Gt),
                Some(">=") => Some(CritOp::Ge),
                Some("<") => Some(CritOp::Lt),
                Some("<=") => Some(CritOp::Le),
                Some("<>") => Some(CritOp::Ne),
                Some("=") => Some(CritOp::Eq),
                Some(_) => None,
            };
            let number = coerce::to_maybe_number(&Scalar::str(val.as_str())).and_then(|x| x.as_num());
            let lower = val.to_lowercase();
            let boolv = if lower == "true" {
                Some(true)
            } else if lower == "false" {
                Some(false)
            } else {
                None
            };
            let op = op?;
            if val.is_empty() {
                (op, CritValue::Null)
            } else if let Some(n) = number {
                (op, CritValue::Num(n))
            } else if matches!(op, CritOp::Eq | CritOp::Ne) {
                (op, boolv.map_or(CritValue::Str(val), CritValue::Bool))
            } else {
                return None;
            }
        }
        _ => return None,
    };
    let f: Box<dyn Fn(&Scalar) -> bool> = match (op, value) {
        (CritOp::Gt | CritOp::Ge | CritOp::Lt | CritOp::Le, CritValue::Num(n)) => Box::new(move |v: &Scalar| match v {
            Scalar::Num(x, _) => {
                let c = float_cmp(*x, n);
                match op {
                    CritOp::Gt => c > 0,
                    CritOp::Ge => c >= 0,
                    CritOp::Lt => c < 0,
                    _ => c <= 0,
                }
            }
            _ => false,
        }),
        (CritOp::Gt | CritOp::Ge | CritOp::Lt | CritOp::Le, _) => Box::new(|_: &Scalar| false),
        (CritOp::Eq, CritValue::Num(n)) => Box::new(move |v: &Scalar| match v {
            Scalar::Num(x, _) => float_cmp(*x, n) == 0,
            Scalar::Str(s) if !s.is_empty() => coerce::to_maybe_number(v).and_then(|x| x.as_num()).is_some_and(|x| float_cmp(x, n) == 0),
            _ => false,
        }),
        (CritOp::Eq, CritValue::Str(p)) => Box::new(eq_matcher(&p)),
        (CritOp::Eq, CritValue::Bool(b)) => Box::new(move |v: &Scalar| matches!(v, Scalar::Bool(x) if *x == b)),
        (CritOp::Eq, CritValue::Null) => Box::new(|v: &Scalar| matches!(v, Scalar::Empty)),
        (CritOp::Ne, CritValue::Num(n)) => Box::new(move |v: &Scalar| match v {
            Scalar::Num(x, _) => float_cmp(*x, n) != 0,
            Scalar::Str(s) => s.is_empty() || coerce::to_maybe_number(v).and_then(|x| x.as_num()).is_none_or(|x| float_cmp(x, n) != 0),
            _ => true,
        }),
        (CritOp::Ne, CritValue::Str(p)) => Box::new(neq_matcher(&p)),
        (CritOp::Ne, CritValue::Bool(b)) => Box::new(move |v: &Scalar| !matches!(v, Scalar::Bool(x) if *x == b)),
        (CritOp::Ne, CritValue::Null) => Box::new(|v: &Scalar| !matches!(v, Scalar::Empty)),
    };
    Some(Criterion { f })
}

/// Raw values of a range argument, row-major.
fn raw_values(r: &RangeVal) -> Vec<Scalar> {
    r.values().map(|v| v.clone().raw()).collect()
}

/// `computeConditionalAggregationFunction`: the values of `values` that pass every condition.
fn conditional(values: &RangeVal, conds: &[Arg]) -> Result<Vec<Scalar>, ErrorType> {
    let mut parsed = Vec::new();
    let mut i = 0;
    while i + 1 < conds.len() {
        let range = conds[i].to_range();
        let crit = build_criterion(&conds[i + 1].scalar()).ok_or(ErrorType::Value)?;
        parsed.push((range, crit));
        i += 2;
    }
    for (r, _) in &parsed {
        if !r.same_dimensions(values) {
            return Err(ErrorType::Value);
        }
    }
    let vals: Vec<Scalar> = values.values().cloned().collect();
    let cond_vals: Vec<Vec<Scalar>> = parsed.iter().map(|(r, _)| raw_values(r)).collect();
    let mut out = Vec::new();
    for (idx, v) in vals.into_iter().enumerate() {
        if parsed.iter().enumerate().all(|(k, (_, c))| cond_vals[k].get(idx).is_some_and(|x| c.test(x))) {
            out.push(v);
        }
    }
    Ok(out)
}

fn sum_filtered(vals: Vec<Scalar>) -> Val {
    let mut acc = Scalar::num(0.0);
    for v in vals {
        let mapped = match v {
            Scalar::Err(e) => Scalar::Err(e),
            Scalar::Num(x, _) => Scalar::num(x),
            _ => continue,
        };
        acc = coerce::nonstrict_add(&acc, &mapped);
    }
    Val::S(acc)
}

fn sumif(_: &Call, a: &[Arg]) -> Val {
    let cond = a[0].to_range();
    let values = if a[2].is_missing() { cond.clone() } else { a[2].to_range() };
    match conditional(&values, &[Arg::R(cond), a[1].clone()]) {
        Ok(v) => sum_filtered(v),
        Err(e) => err(e),
    }
}

fn sumifs(_: &Call, a: &[Arg]) -> Val {
    match conditional(&a[0].to_range(), &a[1..]) {
        Ok(v) => sum_filtered(v),
        Err(e) => err(e),
    }
}

fn countif(_: &Call, a: &[Arg]) -> Val {
    let cond = a[0].to_range();
    match conditional(&cond, &[Arg::R(cond.clone()), a[1].clone()]) {
        Ok(v) => num(v.len() as f64),
        Err(e) => err(e),
    }
}

fn countifs(_: &Call, a: &[Arg]) -> Val {
    match conditional(&a[0].to_range(), a) {
        Ok(v) => num(v.len() as f64),
        Err(e) => err(e),
    }
}

fn averageif(_: &Call, a: &[Arg]) -> Val {
    let cond = a[0].to_range();
    let values = if a[2].is_missing() { cond.clone() } else { a[2].to_range() };
    match conditional(&values, &[Arg::R(cond), a[1].clone()]) {
        Ok(v) => {
            let mut sum = 0.0;
            let mut count = 0.0;
            for x in v {
                if let Scalar::Num(n, _) = x {
                    sum += n;
                    count += 1.0;
                }
            }
            let avg = if count > 0.0 { sum / count } else { f64::NAN };
            // `averageValue() || #DIV/0!` — 0 and NaN are falsy.
            if avg == 0.0 || avg.is_nan() { err(ErrorType::Div0) } else { num(avg) }
        }
        Err(e) => err(e),
    }
}

fn min_max_ifs(a: &[Arg], max: bool) -> Val {
    match conditional(&a[0].to_range(), &a[1..]) {
        Ok(v) => {
            let mut acc = if max { f64::NEG_INFINITY } else { f64::INFINITY };
            for x in v {
                let mapped = match x {
                    Scalar::Num(n, _) => n,
                    // Math.min(x, CellError) is NaN.
                    Scalar::Err(_) => f64::NAN,
                    _ => continue,
                };
                acc = if max { js_max(acc, mapped) } else { js_min(acc, mapped) };
            }
            num(zero_for_infinite(acc))
        }
        Err(e) => err(e),
    }
}

// ------------------------------------------------------------------ Median, Percentile, misc

fn sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v
}

fn median(_: &Call, a: &[Arg]) -> Val {
    let v = match numbers_exact_ranges(a) {
        Ok(v) => sorted(v),
        Err(e) => return err(e),
    };
    if v.is_empty() {
        return err(ErrorType::Num);
    }
    let n = v.len();
    if n % 2 == 0 { num((v[n / 2 - 1] + v[n / 2]) / 2.0) } else { num(v[n / 2]) }
}

fn large_small(a: &[Arg], large: bool) -> Val {
    let r = a[0].to_range();
    let v = match exact_numbers(r.values()) {
        Ok(v) => sorted(v),
        Err(e) => return err(e),
    };
    let n = a[1].num().trunc();
    if n > v.len() as f64 {
        return err(ErrorType::Num);
    }
    let n = n as usize;
    if large { num(v[v.len() - n]) } else { num(v[n - 1]) }
}

fn sorted_values(a: &Arg) -> Result<Vec<f64>, ErrorType> {
    let v = exact_numbers(a.to_range().values())?;
    if v.is_empty() {
        return Err(ErrorType::Num);
    }
    Ok(sorted(v))
}

fn pct_inc(v: &[f64], k: f64) -> f64 {
    let n = v.len();
    let rank = k * (n as f64 - 1.0);
    let lo = rank.floor();
    let frac = rank - lo;
    let lo = lo as usize;
    if lo + 1 < n { v[lo] + frac * (v[lo + 1] - v[lo]) } else { v[lo] }
}

fn pct_exc(v: &[f64], k: f64) -> Val {
    let n = v.len() as f64;
    let rank = k * (n + 1.0);
    if rank < 1.0 || rank > n {
        return err(ErrorType::Num);
    }
    let lo = rank.floor();
    let frac = rank - lo;
    let lo = lo as usize;
    if lo < v.len() { num(v[lo - 1] + frac * (v[lo] - v[lo - 1])) } else { num(v[lo - 1]) }
}

fn countunique(_: &Call, a: &[Arg]) -> Val {
    let mut values: Vec<Scalar> = Vec::new();
    let mut errors: Vec<ErrorType> = Vec::new();
    for x in a {
        match x.scalar() {
            Scalar::Err(e) => {
                if !errors.contains(&e) {
                    errors.push(e);
                }
            }
            Scalar::Str(s) if s.is_empty() => {}
            v => {
                let v = match v {
                    Scalar::Num(n, _) => Scalar::Num(if n == 0.0 { 0.0 } else { n }, NumType::Raw),
                    o => o,
                };
                let dup = values.iter().any(|u| match (u, &v) {
                    (Scalar::Num(p, _), Scalar::Num(q, _)) => p == q || (p.is_nan() && q.is_nan()),
                    (p, q) => p == q,
                });
                if !dup {
                    values.push(v);
                }
            }
        }
    }
    num((values.len() + errors.len()) as f64)
}

fn sumproduct(_: &Call, a: &[Arg]) -> Val {
    let ranges: Vec<RangeVal> = a.iter().map(Arg::to_range).collect();
    let (w, h) = (ranges[0].width(), ranges[0].height());
    if ranges.iter().any(|r| r.width() != w || r.height() != h) {
        return err(ErrorType::Value);
    }
    let flat: Vec<Vec<Scalar>> = ranges.iter().map(|r| r.values().cloned().collect()).collect();
    let mut ret = 0.0;
    for i in 0..w * h {
        let mut acc = 1.0;
        for f in &flat {
            let v = &f[i];
            if let Scalar::Err(e) = v {
                return err(*e);
            }
            match coerce::to_number_or_error(v) {
                Scalar::Num(x, _) => acc *= x,
                _ => acc = 0.0,
            }
        }
        ret += acc;
    }
    num(ret)
}

// ------------------------------------------------------------------ StatisticalAggregationPlugin (descriptive)

/// jStat `sum` (iterates from the end).
pub(super) fn jsum(v: &[f64]) -> f64 {
    let mut s = 0.0;
    for x in v.iter().rev() {
        s += x;
    }
    s
}

pub(super) fn jmean(v: &[f64]) -> f64 {
    jsum(v) / v.len() as f64
}

pub(super) fn sumsqerr(v: &[f64]) -> f64 {
    let m = jmean(v);
    let mut s = 0.0;
    for x in v.iter().rev() {
        let t = x - m;
        s += t * t;
    }
    s
}

pub(super) fn jvariance(v: &[f64], sample: bool) -> f64 {
    sumsqerr(v) / (v.len() as f64 - if sample { 1.0 } else { 0.0 })
}

pub(super) fn jstdev(v: &[f64], sample: bool) -> f64 {
    jvariance(v, sample).sqrt()
}

fn covariance(x: &[f64], y: &[f64]) -> f64 {
    let u = jmean(x);
    let v = jmean(y);
    let dev: Vec<f64> = x.iter().zip(y).map(|(a, b)| (a - u) * (b - v)).collect();
    jsum(&dev) / (x.len() as f64 - 1.0)
}

fn corrcoeff(x: &[f64], y: &[f64]) -> f64 {
    covariance(x, y) / jstdev(x, true) / jstdev(y, true)
}

fn two_arrays(a: &[Arg]) -> Result<(Vec<f64>, Vec<f64>), ErrorType> {
    let x = a[0].to_range();
    let y = a[1].to_range();
    if x.width() * x.height() != y.width() * y.height() {
        return Err(ErrorType::Na);
    }
    let (mut ax, mut ay) = (Vec::new(), Vec::new());
    for (vx, vy) in x.values().zip(y.values()) {
        if let Scalar::Err(e) = vx {
            return Err(*e);
        }
        if let Scalar::Err(e) = vy {
            return Err(*e);
        }
        if let (Scalar::Num(p, _), Scalar::Num(q, _)) = (vx, vy) {
            ax.push(*p);
            ay.push(*q);
        }
    }
    Ok((ax, ay))
}

fn with_two(a: &[Arg], f: impl Fn(&[f64], &[f64]) -> Val) -> Val {
    match two_arrays(a) {
        Ok((x, y)) => f(&x, &y),
        Err(e) => err(e),
    }
}

fn with_exact(a: &[Arg], f: impl Fn(&[f64]) -> Val) -> Val {
    match numbers_exact_ranges(a) {
        Ok(v) => f(&v),
        Err(e) => err(e),
    }
}

fn custom(f: Custom) -> Kind {
    Kind::Custom(f)
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    let mut v: Vec<(&'static str, Kind)> = vec![
        ("SUM", custom(|it, a, s, _| do_sum(it, a, s))),
        ("SUMSQ", custom(|it, a, s, _| res(reduce(it, a, s, 0.0, coerce::add_raw, |x| js_pow(x, 2.0), strictly_numbers)))),
        ("MAX", custom(|it, a, s, _| do_max(it, a, s))),
        ("MAXA", custom(|it, a, s, _| res(reduce(it, a, s, f64::NEG_INFINITY, js_max, |x| x, numbers_booleans).map(zero_for_infinite)))),
        ("MIN", custom(|it, a, s, _| do_min(it, a, s))),
        ("MINA", custom(|it, a, s, _| res(reduce(it, a, s, f64::INFINITY, js_min, |x| x, numbers_booleans).map(zero_for_infinite)))),
        ("COUNT", custom(|it, a, s, _| do_count(it, a, s))),
        ("COUNTA", custom(|it, a, s, _| do_counta(it, a, s))),
        ("AVERAGE", custom(|it, a, s, _| do_average(it, a, s))),
        ("AVERAGEA", custom(|it, a, s, _| moments_to(aggregate_a(it, a, s), Moments::average))),
        ("VAR.S", custom(|it, a, s, _| do_var_s(it, a, s))),
        ("VAR.P", custom(|it, a, s, _| do_var_p(it, a, s))),
        ("VARA", custom(|it, a, s, _| moments_to(aggregate_a(it, a, s), Moments::var_s))),
        ("VARPA", custom(|it, a, s, _| moments_to(aggregate_a(it, a, s), Moments::var_p))),
        ("STDEV.S", custom(|it, a, s, _| do_stdev_s(it, a, s))),
        ("STDEV.P", custom(|it, a, s, _| do_stdev_p(it, a, s))),
        ("STDEVA", custom(|it, a, s, _| moments_to(aggregate_a(it, a, s), |m| m.var_s().map(f64::sqrt)))),
        ("STDEVPA", custom(|it, a, s, _| moments_to(aggregate_a(it, a, s), |m| m.var_p().map(f64::sqrt)))),
        ("PRODUCT", custom(|it, a, s, _| do_product(it, a, s))),
        ("SUBTOTAL", custom(subtotal)),
    ];
    v.extend(simple(&[
        ("SUMIF", sumif),
        ("SUMIFS", sumifs),
        ("COUNTIF", countif),
        ("COUNTIFS", countifs),
        ("AVERAGEIF", averageif),
        ("MINIFS", |_, a| min_max_ifs(a, false)),
        ("MAXIFS", |_, a| min_max_ifs(a, true)),
        ("MEDIAN", median),
        ("LARGE", |_, a| large_small(a, true)),
        ("SMALL", |_, a| large_small(a, false)),
        ("PERCENTILE.INC", |_, a| match sorted_values(&a[0]) {
            Ok(v) => num(pct_inc(&v, a[1].num())),
            Err(e) => err(e),
        }),
        ("PERCENTILE.EXC", |_, a| match sorted_values(&a[0]) {
            Ok(v) => pct_exc(&v, a[1].num()),
            Err(e) => err(e),
        }),
        ("QUARTILE.INC", |_, a| {
            let q = a[1].num().trunc();
            if q < 0.0 || q > 4.0 {
                return err(ErrorType::Num);
            }
            match sorted_values(&a[0]) {
                Ok(v) => num(pct_inc(&v, q / 4.0)),
                Err(e) => err(e),
            }
        }),
        ("QUARTILE.EXC", |_, a| {
            let q = a[1].num().trunc();
            if !(1.0..=3.0).contains(&q) {
                return err(ErrorType::Num);
            }
            match sorted_values(&a[0]) {
                Ok(v) => pct_exc(&v, q / 4.0),
                Err(e) => err(e),
            }
        }),
        ("COUNTBLANK", |_, a| num(a.iter().filter(|x| matches!(x, Arg::S(Scalar::Empty))).count() as f64)),
        ("COUNTUNIQUE", countunique),
        ("SUMPRODUCT", sumproduct),
        ("AVEDEV", |_, a| {
            with_exact(a, |v| {
                if v.is_empty() {
                    return err(ErrorType::Div0);
                }
                let avg = jmean(v);
                num(v.iter().fold(0.0, |s, b| s + (b - avg).abs()) / v.len() as f64)
            })
        }),
        ("DEVSQ", |_, a| with_exact(a, |v| num(if v.is_empty() { 0.0 } else { sumsqerr(v) }))),
        ("GEOMEAN", |_, a| {
            with_exact(a, |v| {
                if v.is_empty() {
                    return err(ErrorType::Num);
                }
                if v.iter().any(|x| *x <= 0.0) {
                    return err(ErrorType::Num);
                }
                let mut prod = 1.0;
                for x in v.iter().rev() {
                    prod *= x;
                }
                num(js_pow(prod, 1.0 / v.len() as f64))
            })
        }),
        ("HARMEAN", |_, a| {
            with_exact(a, |v| {
                if v.is_empty() {
                    return err(ErrorType::Num);
                }
                if v.iter().any(|x| *x <= 0.0) {
                    return err(ErrorType::Num);
                }
                num(v.len() as f64 / v.iter().fold(0.0, |s, b| s + 1.0 / b))
            })
        }),
        ("CORREL", |_, a| with_two(a, |x, y| if x.len() <= 1 { err(ErrorType::Div0) } else { num(corrcoeff(x, y)) })),
        ("RSQ", |_, a| with_two(a, |x, y| if x.len() <= 1 { err(ErrorType::Div0) } else { num(js_pow(corrcoeff(x, y), 2.0)) })),
        ("COVARIANCE.P", |_, a| {
            with_two(a, |x, y| {
                let n = x.len() as f64;
                if n < 1.0 {
                    err(ErrorType::Div0)
                } else if n == 1.0 {
                    num(0.0)
                } else {
                    num(covariance(x, y) * (n - 1.0) / n)
                }
            })
        }),
        ("COVARIANCE.S", |_, a| with_two(a, |x, y| if x.len() <= 1 { err(ErrorType::Div0) } else { num(covariance(x, y)) })),
        ("SLOPE", |_, a| {
            with_two(a, |x, y| {
                let n = x.len() as f64;
                if n <= 1.0 { err(ErrorType::Div0) } else { num(covariance(x, y) * (n - 1.0) / sumsqerr(y)) }
            })
        }),
        ("STEYX", |_, a| {
            with_two(a, |x, y| {
                let n = x.len() as f64;
                if n <= 2.0 {
                    return err(ErrorType::Div0);
                }
                num(((sumsqerr(x) - js_pow(covariance(x, y) * (n - 1.0), 2.0) / sumsqerr(y)) / (n - 2.0)).sqrt())
            })
        }),
        ("SKEW", |_, a| {
            with_exact(a, |v| {
                let n = v.len() as f64;
                if n < 3.0 {
                    return err(ErrorType::Div0);
                }
                let avg = jmean(v);
                let s = jstdev(v, true);
                if s == 0.0 {
                    return err(ErrorType::Div0);
                }
                num(v.iter().fold(0.0, |acc, b| acc + js_pow((b - avg) / s, 3.0)) * n / (n - 1.0) / (n - 2.0))
            })
        }),
        ("SKEW.P", |_, a| {
            with_exact(a, |v| {
                let n = v.len() as f64;
                if n < 3.0 {
                    return err(ErrorType::Div0);
                }
                let avg = jmean(v);
                let s = jstdev(v, false);
                if s == 0.0 {
                    return err(ErrorType::Div0);
                }
                num(v.iter().fold(0.0, |acc, b| acc + js_pow((b - avg) / s, 3.0)) / n)
            })
        }),
    ]));
    v
}
