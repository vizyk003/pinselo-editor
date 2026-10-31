// SPDX-License-Identifier: AGPL-3.0-only
//! Built-in functions: dispatch by canonical name, shared helpers, one module per HyperFormula plugin group.

use std::collections::HashMap;
use std::sync::LazyLock;

use super::ast::Ast;
use super::func::{Arg, Call, Impl, Meta};
use super::interp::{Interp, State};
use super::meta_table;
use super::value::{ErrorType, NumType, RangeVal, Scalar, Val};

mod array;
mod complex;
mod database;
mod date;
mod financial;
mod info;
mod jstat;
mod logic;
mod lookup;
mod math;
mod statistical;
mod stats;
mod text;

/// AST-level implementation (functions that do not go through `runFunction`).
pub type Custom = fn(&Interp, &[Ast], &State, &Meta) -> Val;

pub enum Kind {
    Simple(Impl),
    Custom(Custom),
}

static METAS: LazyLock<HashMap<&'static str, Meta>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for name in implemented() {
        if let Some(meta) = meta_table::meta(name) {
            m.insert(name, meta);
        }
    }
    m
});

static IMPLS: LazyLock<HashMap<&'static str, Kind>> = LazyLock::new(|| {
    let mut m: HashMap<&'static str, Kind> = HashMap::new();
    for (k, v) in logic::functions()
        .into_iter()
        .chain(info::functions())
        .chain(text::functions())
        .chain(math::functions())
        .chain(stats::functions())
        .chain(lookup::functions())
        .chain(date::functions())
        .chain(array::functions())
        .chain(financial::functions())
        .chain(statistical::functions())
        .chain(complex::functions())
        .chain(database::functions())
    {
        m.insert(k, v);
    }
    m
});

/// Every implemented canonical function name (sorted).
pub fn implemented() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = IMPLS.keys().copied().collect();
    v.sort();
    v
}

/// True when `name` (or its alias target) is implemented.
pub fn is_implemented(name: &str) -> bool {
    let canon = meta_table::alias(name).unwrap_or(name);
    IMPLS.contains_key(canon)
}

/// Metadata of a canonical or alias name.
pub fn meta_of(name: &str) -> Option<&'static Meta> {
    let canon = meta_table::alias(name).unwrap_or(name);
    METAS.get(canon)
}

/// Call a function by (upper-cased) name.
pub fn call(it: &Interp, name: &str, args: &[Ast], state: &State) -> Val {
    let canon = meta_table::alias(name).unwrap_or(name);
    let (Some(kind), Some(meta)) = (IMPLS.get(canon), METAS.get(canon)) else {
        return Val::err(ErrorType::Name);
    };
    let st = State { arrays: state.arrays || meta.array_fn, ..*state };
    match kind {
        Kind::Simple(f) => it.run_function(args, &st, meta, *f),
        Kind::Custom(f) => f(it, args, &st, meta),
    }
}

// ------------------------------------------------------------------ shared helpers

pub fn num(v: f64) -> Val {
    Val::num(v)
}

pub fn err(e: ErrorType) -> Val {
    Val::err(e)
}

pub fn s(v: impl Into<std::sync::Arc<str>>) -> Val {
    Val::str(v)
}

/// JS `Math.trunc`.
pub fn trunc(x: f64) -> f64 {
    x.trunc()
}

/// JS `Math.pow` (NaN for `1 ** NaN` and `(±1) ** ±Infinity`, unlike `powf`).
pub fn js_pow(b: f64, e: f64) -> f64 {
    if e.is_nan() || (b.abs() == 1.0 && e.is_infinite()) {
        return f64::NAN;
    }
    b.powf(e)
}

/// JS `%` (truncated remainder, sign of the dividend).
pub fn js_rem(a: f64, b: f64) -> f64 {
    a % b
}

/// `ToIntegerOrInfinity`.
pub fn to_int(x: f64) -> f64 {
    if x.is_nan() { 0.0 } else { x.trunc() }
}

/// UTF-16 code units of a string.
pub fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn from_u16(v: &[u16]) -> String {
    String::from_utf16_lossy(v)
}

/// `String#slice(start, end?)`.
pub fn js_slice(s: &str, start: f64, end: Option<f64>) -> String {
    let u = u16s(s);
    let len = u.len() as f64;
    let rel = |x: f64| -> usize {
        let x = to_int(x);
        let v = if x < 0.0 { (len + x).max(0.0) } else { x.min(len) };
        v as usize
    };
    let a = rel(start);
    let b = end.map_or(u.len(), rel);
    if a >= b { String::new() } else { from_u16(&u[a..b]) }
}

/// `String#substring(start, end?)`.
pub fn js_substring(s: &str, start: f64, end: Option<f64>) -> String {
    let u = u16s(s);
    let len = u.len() as f64;
    let clamp = |x: f64| to_int(x).clamp(0.0, len) as usize;
    let a = clamp(start);
    let b = end.map_or(u.len(), clamp);
    let (a, b) = if a > b { (b, a) } else { (a, b) };
    from_u16(&u[a..b])
}

/// `String#length`.
pub fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `String#indexOf(search, from)` in UTF-16 units.
pub fn js_index_of(hay: &str, needle: &str) -> i64 {
    let h = u16s(hay);
    let n = u16s(needle);
    if n.is_empty() {
        return 0;
    }
    if n.len() > h.len() {
        return -1;
    }
    for i in 0..=h.len() - n.len() {
        if h[i..i + n.len()] == n[..] {
            return i as i64;
        }
    }
    -1
}

/// Numbers of a range (`manyToExactNumbers`): numbers only, first error wins.
pub fn exact_numbers<'a>(vals: impl Iterator<Item = &'a Scalar>) -> Result<Vec<f64>, ErrorType> {
    let mut out = Vec::new();
    for v in vals {
        match v {
            Scalar::Err(e) => return Err(*e),
            Scalar::Num(x, _) => out.push(*x),
            _ => {}
        }
    }
    Ok(out)
}

/// `coerceNumbersExactRanges`: scalars coerced to numbers (errors propagate), ranges contribute numbers only.
pub fn numbers_exact_ranges(args: &[Arg]) -> Result<Vec<f64>, ErrorType> {
    numbers_with(args, |vals| exact_numbers(vals.iter()))
}

/// `coerceNumbersCoerceRangesDropNulls`: range values coerced too (non-coercible dropped).
pub fn numbers_coerce_ranges(args: &[Arg]) -> Result<Vec<f64>, ErrorType> {
    numbers_with(args, |vals| {
        let mut out = Vec::new();
        for v in vals {
            match v {
                Scalar::Err(e) => return Err(*e),
                Scalar::Empty => {}
                other => {
                    if let Scalar::Num(x, _) = super::coerce::to_number_or_error(other) {
                        out.push(x);
                    }
                }
            }
        }
        Ok(out)
    })
}

fn numbers_with(args: &[Arg], range_fn: impl Fn(&[Scalar]) -> Result<Vec<f64>, ErrorType>) -> Result<Vec<f64>, ErrorType> {
    enum Item<'a> {
        R(&'a RangeVal),
        N(f64),
    }
    let mut items = Vec::new();
    for a in args {
        match a {
            Arg::R(r) => items.push(Item::R(r)),
            Arg::S(sv) => match super::coerce::to_number_or_error(sv) {
                Scalar::Num(x, _) => items.push(Item::N(x)),
                Scalar::Err(e) => return Err(e),
                _ => return Err(ErrorType::Value),
            },
            Arg::Missing | Arg::Complex(..) => {}
        }
    }
    let mut out = Vec::new();
    for it in items {
        match it {
            Item::N(x) => out.push(x),
            Item::R(r) => {
                let flat: Vec<Scalar> = r.values().cloned().collect();
                out.extend(range_fn(&flat)?);
            }
        }
    }
    Ok(out)
}

/// A typed number result.
pub fn typed(v: f64, t: NumType) -> Val {
    Val::S(Scalar::Num(v, t))
}

/// Convenience: build a `Vec<(&'static str, Kind)>` of simple implementations.
pub fn simple(list: &[(&'static str, Impl)]) -> Vec<(&'static str, Kind)> {
    list.iter().map(|(n, f)| (*n, Kind::Simple(*f))).collect()
}

/// The call's scalar value (for `Call`-level helpers).
pub fn call_state(c: &Call) -> State {
    c.state
}
