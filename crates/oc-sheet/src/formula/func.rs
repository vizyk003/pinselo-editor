// SPDX-License-Identifier: AGPL-3.0-only
//! HyperFormula's `FunctionPlugin.runFunction` machinery: argument evaluation, arity checks, coercion to the
//! declared parameter types, default values, repeated trailing parameters, vectorisation over ranges in
//! array mode and the `returnNumberType` wrapper.

use super::ast::Ast;
use super::coerce;
use super::interp::{Interp, State};
use super::value::{ErrorType, NumType, RangeVal, Scalar, Val};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArgType {
    Number,
    Integer,
    String,
    Boolean,
    #[default]
    Scalar,
    Noerror,
    Any,
    Range,
    Complex,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Def {
    Num(f64),
    Bool(bool),
    Str(&'static str),
}

impl Def {
    fn scalar(&self) -> Scalar {
        match self {
            Def::Num(v) => Scalar::num(*v),
            Def::Bool(b) => Scalar::Bool(*b),
            Def::Str(s) => Scalar::str(*s),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Param {
    pub ty: ArgType,
    pub optional: bool,
    pub default: Option<Def>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub greater_than: Option<f64>,
    pub less_than: Option<f64>,
    pub pass_subtype: bool,
    pub empty_as_default: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub params: Vec<Param>,
    pub repeat_last: usize,
    pub expand_ranges: bool,
    pub return_type: Option<NumType>,
    pub vectorization_forbidden: bool,
    /// `enableArrayArithmeticForArguments`
    pub array_fn: bool,
    pub volatile: bool,
    /// `doesNotNeedArgumentsToBeComputed`
    pub no_arg_deps: bool,
    /// Has a `sizeOfResultArrayMethod` (may produce a spilled array).
    pub array_result: bool,
}

/// A coerced argument (`undefined` = `Missing`).
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Missing,
    S(Scalar),
    R(RangeVal),
    /// A COMPLEX parameter: (re, im).
    Complex(f64, f64),
}

impl Arg {
    pub fn is_missing(&self) -> bool {
        matches!(self, Arg::Missing)
    }
    /// Raw number (NUMBER/INTEGER parameters).
    pub fn num(&self) -> f64 {
        match self {
            Arg::S(Scalar::Num(v, _)) => *v,
            _ => f64::NAN,
        }
    }
    pub fn opt_num(&self) -> Option<f64> {
        match self {
            Arg::S(Scalar::Num(v, _)) => Some(*v),
            _ => None,
        }
    }
    pub fn str(&self) -> &str {
        match self {
            Arg::S(Scalar::Str(s)) => s,
            _ => "",
        }
    }
    pub fn opt_str(&self) -> Option<&str> {
        match self {
            Arg::S(Scalar::Str(s)) => Some(s),
            _ => None,
        }
    }
    pub fn bool(&self) -> bool {
        matches!(self, Arg::S(Scalar::Bool(true)))
    }
    pub fn opt_bool(&self) -> Option<bool> {
        match self {
            Arg::S(Scalar::Bool(b)) => Some(*b),
            _ => None,
        }
    }
    pub fn scalar(&self) -> Scalar {
        match self {
            Arg::S(s) => s.clone(),
            Arg::Missing => Scalar::Empty,
            Arg::R(r) => r.data.first().and_then(|row| row.first()).cloned().unwrap_or(Scalar::Empty),
            Arg::Complex(re, im) => Scalar::str(super::coerce::complex_to_string(*re, *im, "i").unwrap_or_default()),
        }
    }
    pub fn range(&self) -> Option<&RangeVal> {
        match self {
            Arg::R(r) => Some(r),
            _ => None,
        }
    }
    /// `coerceToRange`.
    pub fn to_range(&self) -> RangeVal {
        match self {
            Arg::R(r) => r.clone(),
            Arg::S(s) => RangeVal::from_scalar(s.clone()),
            Arg::Missing => RangeVal::from_scalar(Scalar::Empty),
            Arg::Complex(..) => RangeVal::from_scalar(self.scalar()),
        }
    }
    pub fn into_val(self) -> Val {
        match self {
            Arg::S(s) => Val::S(s),
            Arg::R(r) => Val::R(r),
            Arg::Missing => Val::S(Scalar::Empty),
            Arg::Complex(..) => Val::S(self.scalar()),
        }
    }
}

/// Context handed to function implementations.
pub struct Call<'a, 'b> {
    pub it: &'a Interp<'b>,
    pub state: State,
    pub args_ast: &'a [Ast],
}

pub type Impl = fn(&Call, &[Arg]) -> Val;

fn param_list(meta: &Meta, n: usize) -> Vec<Param> {
    let mut ps = meta.params.clone();
    if meta.repeat_last > 0 && !ps.is_empty() {
        while n > ps.len() {
            let tail: Vec<Param> = ps[ps.len() - meta.repeat_last..].to_vec();
            ps.extend(tail);
        }
    }
    ps
}

fn is_range_as_scalar(v: &Val, p: Option<&Param>) -> bool {
    matches!((v, p), (Val::R(_), Some(p)) if !matches!(p.ty, ArgType::Range | ArgType::Any))
}

/// `coerceRangeToScalar`.
pub fn range_to_scalar(r: &RangeVal, state: &State) -> Option<Scalar> {
    match r.range {
        None => r.data.first().and_then(|row| row.first()).cloned(),
        Some(range) => {
            if state.sheet != range.sheet {
                return None;
            }
            if range.width() == 1 {
                let off = state.row as i64 - range.r0 as i64;
                if off >= 0 && (off as usize) < range.height() {
                    return r.data.get(off as usize).and_then(|row| row.first()).cloned();
                }
            } else if range.height() == 1 {
                let off = state.col as i64 - range.c0 as i64;
                if off >= 0 && (off as usize) < range.width() {
                    return r.data.first().and_then(|row| row.get(off as usize)).cloned();
                }
            }
            None
        }
    }
}

enum Coerced {
    Undefined,
    Ok(Arg),
}

fn coerce_to_type(v: Val, p: &Param, state: &State) -> Coerced {
    let scalar = match v {
        Val::R(r) => match p.ty {
            ArgType::Range | ArgType::Any => return Coerced::Ok(Arg::R(r)),
            _ => match range_to_scalar(&r, state) {
                Some(s) => s,
                None => return Coerced::Undefined,
            },
        },
        Val::S(s) => s,
    };
    let ret: Option<Arg> = match p.ty {
        ArgType::Integer | ArgType::Number => {
            let c = coerce::to_number_or_error(&scalar);
            match &c {
                Scalar::Num(value, _) => {
                    let value = *value;
                    if p.max.is_some_and(|m| value > m) || p.less_than.is_some_and(|m| value >= m) {
                        return Coerced::Ok(Arg::S(Scalar::Err(ErrorType::Num)));
                    }
                    if p.min.is_some_and(|m| value < m) || p.greater_than.is_some_and(|m| value <= m) {
                        return Coerced::Ok(Arg::S(Scalar::Err(ErrorType::Num)));
                    }
                    if p.ty == ArgType::Integer && !oc_schema::js::is_integer(value) {
                        return Coerced::Ok(Arg::S(Scalar::Err(ErrorType::Num)));
                    }
                    Some(Arg::S(c))
                }
                _ => Some(Arg::S(c)),
            }
        }
        ArgType::String => Some(Arg::S(coerce::to_string(&scalar))),
        ArgType::Boolean => coerce::to_boolean(&scalar).map(Arg::S),
        ArgType::Scalar | ArgType::Noerror | ArgType::Any => Some(Arg::S(scalar)),
        ArgType::Range => {
            if let Scalar::Err(_) = scalar {
                return Coerced::Ok(Arg::S(scalar));
            }
            Some(Arg::R(RangeVal::from_scalar(scalar)))
        }
        ArgType::Complex => match coerce::to_complex(&scalar.clone().raw()) {
            Ok((re, im)) => Some(Arg::Complex(re, im)),
            Err(e) => Some(Arg::S(Scalar::Err(e))),
        },
    };
    match ret {
        None => Coerced::Undefined,
        Some(a) if p.pass_subtype => Coerced::Ok(a),
        Some(Arg::S(s)) => Coerced::Ok(Arg::S(s.raw())),
        Some(a) => Coerced::Ok(a),
    }
}

/// `returnNumberWrapper`.
pub fn wrap_return(v: Val, ty: Option<NumType>) -> Val {
    match (v, ty) {
        (Val::S(Scalar::Num(x, _)), Some(t)) => Val::S(Scalar::Num(x, t)),
        (v, _) => v,
    }
}

impl Interp<'_> {
    /// Evaluate the arguments of a call: (value, ignorable, syntactically empty).
    fn evaluate_arguments(&self, args: &[Ast], state: &State, meta: &Meta) -> Vec<(Val, bool, bool)> {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            let empty = matches!(a, Ast::Empty { .. });
            let v = self.eval(a, state);
            if meta.expand_ranges
                && let Val::R(r) = &v
            {
                for s in r.values() {
                    out.push((Val::S(s.clone()), true, empty));
                }
                continue;
            }
            out.push((v, false, empty));
        }
        out
    }

    /// `runFunction`.
    pub fn run_function(&self, args: &[Ast], state: &State, meta: &Meta, f: Impl) -> Val {
        let evaluated = self.evaluate_arguments(args, state, meta);
        self.run_with_values(evaluated, args, state, meta, f)
    }

    pub fn run_with_values(&self, evaluated: Vec<(Val, bool, bool)>, args: &[Ast], state: &State, meta: &Meta, f: Impl) -> Val {
        let n = evaluated.len();
        let params = param_list(meta, n);
        let vectorize = state.arrays && !meta.vectorization_forbidden;
        if n > params.len() {
            return Val::err(ErrorType::Na);
        }
        if n < params.len() && !params[n..].iter().all(|p| p.optional || p.default.is_some()) {
            return Val::err(ErrorType::Na);
        }
        let (h, w) = if vectorize {
            let mut h = 1;
            let mut w = 1;
            for (i, (v, _, _)) in evaluated.iter().enumerate() {
                if is_range_as_scalar(v, params.get(i))
                    && let Val::R(r) = v
                {
                    h = h.max(r.height());
                    w = w.max(r.width());
                }
            }
            (h, w)
        } else {
            (1, 1)
        };
        let call = Call { it: self, state: *state, args_ast: args };
        let cell = |row: usize, col: usize| -> Val {
            let vals: Vec<(Val, bool, bool)> = evaluated
                .iter()
                .enumerate()
                .map(|(i, (v, ig, em))| {
                    if vectorize
                        && is_range_as_scalar(v, params.get(i))
                        && let Val::R(r) = v
                    {
                        let tr = if r.height() == 1 { 0 } else { row };
                        let tc = if r.width() == 1 { 0 } else { col };
                        let s = r.data.get(tr).and_then(|x| x.get(tc)).cloned();
                        return (s.map_or(Val::S(Scalar::Empty), Val::S), *ig, *em);
                    }
                    (v.clone(), *ig, *em)
                })
                .collect();
            self.single_cell(&call, vals, &params, meta, f)
        };
        if h == 1 && w == 1 {
            return cell(0, 0);
        }
        let mut data = Vec::with_capacity(h);
        for r in 0..h {
            let mut row = Vec::with_capacity(w);
            for c in 0..w {
                row.push(match cell(r, c) {
                    Val::S(s) => s,
                    Val::R(_) => Scalar::Err(ErrorType::Value),
                });
            }
            data.push(row);
        }
        Val::R(RangeVal::only_values(data))
    }

    fn single_cell(&self, call: &Call, vals: Vec<(Val, bool, bool)>, params: &[Param], meta: &Meta, f: Impl) -> Val {
        let mut coerced: Vec<Arg> = Vec::with_capacity(params.len());
        let mut vals = vals.into_iter();
        for p in params {
            let raw = vals.next();
            let value = match &raw {
                None => p.default.as_ref().map(|d| Val::S(d.scalar())),
                Some((_, _, true)) if p.empty_as_default && p.default.is_some() => p.default.as_ref().map(|d| Val::S(d.scalar())),
                Some((v, _, _)) => Some(v.clone()),
            };
            let Some(value) = value else {
                coerced.push(Arg::Missing);
                continue;
            };
            let ignorable = raw.as_ref().is_some_and(|x| x.1);
            match coerce_to_type(value, p, &call.state) {
                Coerced::Undefined => {
                    if !ignorable {
                        return Val::err(ErrorType::Value);
                    }
                    coerced.push(Arg::Missing);
                }
                Coerced::Ok(Arg::S(Scalar::Err(e))) if p.ty != ArgType::Scalar => return Val::err(e),
                Coerced::Ok(a) => coerced.push(a),
            }
        }
        wrap_return(f(call, &coerced), meta.return_type)
    }
}
