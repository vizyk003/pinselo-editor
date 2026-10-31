// SPDX-License-Identifier: AGPL-3.0-only
//! BooleanPlugin: TRUE, FALSE, IF, IFS, AND, OR, NOT, XOR, SWITCH, IFERROR, IFNA, CHOOSE.

use super::super::coerce;
use super::super::func::{Arg, Call};
use super::super::value::{ErrorType, Scalar, Val};
use super::{Kind, err, simple};

fn truthy(a: &Arg) -> Option<bool> {
    match a {
        Arg::Missing => None,
        Arg::S(Scalar::Bool(b)) => Some(*b),
        Arg::S(_) | Arg::R(_) | Arg::Complex(..) => Some(true),
    }
}

fn if_(_: &Call, a: &[Arg]) -> Val {
    if a[0].bool() { a[1].clone().into_val() } else { a[2].clone().into_val() }
}

fn ifs(_: &Call, a: &[Arg]) -> Val {
    let mut i = 0;
    while i < a.len() {
        if a[i].bool() {
            return a.get(i + 1).cloned().map_or(Val::empty(), Arg::into_val);
        }
        i += 2;
    }
    err(ErrorType::Na)
}

fn and(_: &Call, a: &[Arg]) -> Val {
    Val::bool(a.iter().filter_map(truthy).all(|b| b))
}

fn or(_: &Call, a: &[Arg]) -> Val {
    Val::bool(a.iter().filter_map(truthy).any(|b| b))
}

fn not(_: &Call, a: &[Arg]) -> Val {
    Val::bool(!a[0].bool())
}

fn xor(_: &Call, a: &[Arg]) -> Val {
    Val::bool(a.iter().filter_map(truthy).filter(|b| *b).count() % 2 == 1)
}

fn switch(_: &Call, a: &[Arg]) -> Val {
    let selector = a[0].scalar();
    let args = &a[1..];
    let n = args.len();
    let mut i = 0;
    while i + 1 < n {
        if let Arg::S(Scalar::Err(_)) = args[i] {
            i += 2;
            continue;
        }
        if coerce::compare(&selector, &args[i].scalar()) == 0 {
            return args[i + 1].clone().into_val();
        }
        i += 2;
    }
    if i < n { args[i].clone().into_val() } else { err(ErrorType::Na) }
}

fn iferror(_: &Call, a: &[Arg]) -> Val {
    if let Arg::S(Scalar::Err(_)) = a[0] { a[1].clone().into_val() } else { a[0].clone().into_val() }
}

fn ifna(_: &Call, a: &[Arg]) -> Val {
    if let Arg::S(Scalar::Err(ErrorType::Na)) = a[0] { a[1].clone().into_val() } else { a[0].clone().into_val() }
}

fn choose(_: &Call, a: &[Arg]) -> Val {
    let sel = a[0].num();
    let args = &a[1..];
    if sel > args.len() as f64 {
        return err(ErrorType::Num);
    }
    args[sel as usize - 1].clone().into_val()
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    simple(&[
        ("TRUE", |_, _| Val::bool(true)),
        ("FALSE", |_, _| Val::bool(false)),
        ("IF", if_),
        ("IFS", ifs),
        ("AND", and),
        ("OR", or),
        ("NOT", not),
        ("XOR", xor),
        ("SWITCH", switch),
        ("IFERROR", iferror),
        ("IFNA", ifna),
        ("CHOOSE", choose),
    ])
}
