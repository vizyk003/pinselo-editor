// SPDX-License-Identifier: AGPL-3.0-only
//! ComplexPlugin: COMPLEX and the IM* functions (complex numbers as "a+bi" strings).

use super::super::coerce::{complex_to_string, to_complex};
use super::super::func::{Arg, Call};
use super::super::value::{ErrorType, Scalar, Val};
use super::{Kind, err, js_pow, num, s, simple};

type C = (f64, f64);

fn c(a: &Arg) -> C {
    match a {
        Arg::Complex(re, im) => (*re, *im),
        _ => (f64::NAN, f64::NAN),
    }
}

fn out(v: C) -> Val {
    match complex_to_string(v.0, v.1, "i") {
        Some(t) => s(t),
        None => err(ErrorType::Num),
    }
}

fn add(a: C, b: C) -> C {
    (a.0 + b.0, a.1 + b.1)
}

fn sub(a: C, b: C) -> C {
    (a.0 - b.0, a.1 - b.1)
}

fn mul(a: C, b: C) -> C {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + b.0 * a.1)
}

fn div(a: C, b: C) -> C {
    let denom = js_pow(b.0, 2.0) + js_pow(b.1, 2.0);
    let (nr, ni) = mul(a, (b.0, -b.1));
    (nr / denom, ni / denom)
}

fn cos(a: C) -> C {
    (a.0.cos() * a.1.cosh(), -a.0.sin() * a.1.sinh())
}

fn cosh(a: C) -> C {
    (a.0.cosh() * a.1.cos(), a.0.sinh() * a.1.sin())
}

fn sin(a: C) -> C {
    (a.0.sin() * a.1.cosh(), a.0.cos() * a.1.sinh())
}

fn sinh(a: C) -> C {
    (a.0.sinh() * a.1.cos(), a.0.cosh() * a.1.sin())
}

fn exp(a: C) -> C {
    (a.0.exp() * a.1.cos(), a.0.exp() * a.1.sin())
}

fn abs(a: C) -> f64 {
    (a.0 * a.0 + a.1 * a.1).sqrt()
}

fn ln(a: C) -> C {
    (abs(a).ln(), a.1.atan2(a.0))
}

fn power(a: C, n: f64) -> C {
    let (re, im) = ln(a);
    exp((n * re, n * im))
}

fn complex(_: &Call, a: &[Arg]) -> Val {
    let unit = a[2].str();
    if unit != "i" && unit != "j" {
        return err(ErrorType::Value);
    }
    match complex_to_string(a[0].num(), a[1].num(), unit) {
        Some(t) => s(t),
        None => err(ErrorType::Num),
    }
}

/// `coerceComplexExactRanges`: scalars must parse, range values that do not are skipped.
fn complex_args(args: &[Arg]) -> Result<Vec<C>, ErrorType> {
    enum Item<'a> {
        R(&'a super::super::value::RangeVal),
        V(C),
    }
    let mut items = Vec::new();
    for a in args {
        match a {
            Arg::R(r) => items.push(Item::R(r)),
            Arg::S(Scalar::Empty) | Arg::Missing => {}
            Arg::S(sc) => items.push(Item::V(to_complex(&sc.clone().raw())?)),
            Arg::Complex(re, im) => items.push(Item::V((*re, *im))),
        }
    }
    let mut out = Vec::new();
    for it in items {
        match it {
            Item::V(v) => out.push(v),
            Item::R(r) => {
                for v in r.values() {
                    match v {
                        Scalar::Err(e) => return Err(*e),
                        Scalar::Num(..) | Scalar::Str(_) => {
                            if let Ok(x) = to_complex(&v.clone().raw()) {
                                out.push(x);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(out)
}

fn improduct(_: &Call, a: &[Arg]) -> Val {
    match complex_args(a) {
        Ok(v) => out(v.into_iter().fold((1.0, 0.0), mul)),
        Err(e) => err(e),
    }
}

fn imsum(_: &Call, a: &[Arg]) -> Val {
    match complex_args(a) {
        Ok(v) => out(v.into_iter().fold((0.0, 0.0), add)),
        Err(e) => err(e),
    }
}

fn one(a: &[Arg], f: fn(C) -> C) -> Val {
    out(f(c(&a[0])))
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    simple(&[
        ("COMPLEX", complex),
        ("IMABS", |_, a| num(abs(c(&a[0])))),
        ("IMAGINARY", |_, a| num(c(&a[0]).1)),
        ("IMREAL", |_, a| num(c(&a[0]).0)),
        ("IMARGUMENT", |_, a| {
            let (re, im) = c(&a[0]);
            if re == 0.0 && im == 0.0 { err(ErrorType::Div0) } else { num(im.atan2(re)) }
        }),
        ("IMCONJUGATE", |_, a| {
            let (re, im) = c(&a[0]);
            out((re, -im))
        }),
        ("IMCOS", |_, a| one(a, cos)),
        ("IMCOSH", |_, a| one(a, cosh)),
        ("IMCOT", |_, a| one(a, |x| div(cos(x), sin(x)))),
        ("IMCSC", |_, a| one(a, |x| div((1.0, 0.0), sin(x)))),
        ("IMCSCH", |_, a| one(a, |x| div((1.0, 0.0), sinh(x)))),
        ("IMSEC", |_, a| one(a, |x| div((1.0, 0.0), cos(x)))),
        ("IMSECH", |_, a| one(a, |x| div((1.0, 0.0), cosh(x)))),
        ("IMSIN", |_, a| one(a, sin)),
        ("IMSINH", |_, a| one(a, sinh)),
        ("IMTAN", |_, a| one(a, |x| div(sin(x), cos(x)))),
        ("IMDIV", |_, a| out(div(c(&a[0]), c(&a[1])))),
        ("IMPRODUCT", improduct),
        ("IMSUM", imsum),
        ("IMSUB", |_, a| out(sub(c(&a[0]), c(&a[1])))),
        ("IMEXP", |_, a| one(a, exp)),
        ("IMLN", |_, a| one(a, ln)),
        ("IMLOG10", |_, a| {
            let (re, im) = ln(c(&a[0]));
            let k = 10f64.ln();
            out((re / k, im / k))
        }),
        ("IMLOG2", |_, a| {
            let (re, im) = ln(c(&a[0]));
            let k = 2f64.ln();
            out((re / k, im / k))
        }),
        ("IMPOWER", |_, a| out(power(c(&a[0]), a[1].num()))),
        ("IMSQRT", |_, a| out(power(c(&a[0]), 0.5))),
    ])
}
