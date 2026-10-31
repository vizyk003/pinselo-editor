// SPDX-License-Identifier: AGPL-3.0-only
//! DatabasePlugin: DSUM, DAVERAGE, DCOUNT, DCOUNTA, DGET, DMAX, DMIN, DPRODUCT, DSTDEV(P), DVAR(P).

use super::super::func::{Arg, Call};
use super::super::value::{ErrorType, RangeVal, Scalar, Val};
use super::stats::{Criterion, build_criterion};
use super::{Kind, err, js_pow, num, simple};

struct Entry {
    col: usize,
    crit: Criterion,
}

fn field_index(db: &RangeVal, field: &Scalar) -> Result<usize, ErrorType> {
    if let Scalar::Err(e) = field {
        return Err(*e);
    }
    let headers = db.data.first().cloned().unwrap_or_default();
    if let Scalar::Str(f) = field {
        let lower = f.to_lowercase();
        return headers
            .iter()
            .position(|h| matches!(h, Scalar::Str(s) if s.to_lowercase() == lower))
            .ok_or(ErrorType::Value);
    }
    let n = match field {
        Scalar::Bool(b) => f64::from(u8::from(*b)),
        Scalar::Num(v, _) => *v,
        _ => return Err(ErrorType::Value),
    };
    let index = n.trunc();
    if !index.is_finite() || index < 1.0 || index > headers.len() as f64 {
        return Err(ErrorType::Value);
    }
    Ok(index as usize - 1)
}

fn criteria_rows(db: &RangeVal, criteria: &RangeVal) -> Result<Vec<Vec<Entry>>, ErrorType> {
    let db_headers = db.data.first().cloned().unwrap_or_default();
    let headers = criteria.data.first().cloned().unwrap_or_default();
    for h in &headers {
        if let Scalar::Err(e) = h {
            return Err(*e);
        }
    }
    let mapping: Vec<Option<usize>> = headers
        .iter()
        .map(|h| match h {
            Scalar::Str(s) => {
                let l = s.to_lowercase();
                db_headers.iter().position(|d| matches!(d, Scalar::Str(x) if x.to_lowercase() == l))
            }
            _ => None,
        })
        .collect();
    let mut rows = Vec::new();
    for r in 1..criteria.data.len() {
        let mut row = Vec::new();
        for (c, m) in mapping.iter().enumerate() {
            let Some(col) = m else { continue };
            let v = criteria.data[r].get(c).cloned().unwrap_or(Scalar::Empty);
            match v {
                Scalar::Empty => continue,
                Scalar::Err(e) => return Err(e),
                other => match build_criterion(&other.raw()) {
                    Some(crit) => row.push(Entry { col: *col, crit }),
                    None => return Err(ErrorType::Value),
                },
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

fn matches(row: &[Scalar], crit: &[Vec<Entry>]) -> bool {
    if crit.is_empty() {
        return false;
    }
    crit.iter().any(|cr| cr.iter().all(|e| e.crit.test(&row.get(e.col).cloned().unwrap_or(Scalar::Empty).raw())))
}

/// Matching rows' field values (header row skipped).
fn selected(a: &[Arg]) -> Result<Vec<Scalar>, ErrorType> {
    let db = a[0].to_range();
    let field = field_index(&db, &a[1].scalar())?;
    let crit = criteria_rows(&db, &a[2].to_range())?;
    Ok(db.data.iter().skip(1).filter(|row| matches(row, &crit)).map(|row| row.get(field).cloned().unwrap_or(Scalar::Empty)).collect())
}

/// Numbers among the matching values; the first error wins.
fn numbers(a: &[Arg]) -> Result<Vec<f64>, ErrorType> {
    let mut out = Vec::new();
    for v in selected(a)? {
        match v {
            Scalar::Err(e) => return Err(e),
            Scalar::Num(x, _) => out.push(x),
            _ => {}
        }
    }
    Ok(out)
}

fn with_numbers(a: &[Arg], f: impl Fn(&[f64]) -> Val) -> Val {
    match numbers(a) {
        Ok(v) => f(&v),
        Err(e) => err(e),
    }
}

fn dsum(_: &Call, a: &[Arg]) -> Val {
    with_numbers(a, |v| num(v.iter().fold(0.0, |s, x| s + x)))
}

fn daverage(_: &Call, a: &[Arg]) -> Val {
    with_numbers(a, |v| if v.is_empty() { err(ErrorType::Div0) } else { num(v.iter().fold(0.0, |s, x| s + x) / v.len() as f64) })
}

fn dcount(_: &Call, a: &[Arg]) -> Val {
    with_numbers(a, |v| num(v.len() as f64))
}

fn dcounta(_: &Call, a: &[Arg]) -> Val {
    match selected(a) {
        Ok(vals) => {
            let mut n = 0.0;
            for v in vals {
                match v {
                    Scalar::Err(e) => return err(e),
                    Scalar::Empty => {}
                    _ => n += 1.0,
                }
            }
            num(n)
        }
        Err(e) => err(e),
    }
}

fn dproduct(_: &Call, a: &[Arg]) -> Val {
    with_numbers(a, |v| if v.is_empty() { num(0.0) } else { num(v.iter().fold(1.0, |s, x| s * x)) })
}

fn dmax(_: &Call, a: &[Arg]) -> Val {
    with_numbers(a, |v| if v.is_empty() { num(0.0) } else { num(v.iter().fold(f64::NEG_INFINITY, |m, x| if *x > m { *x } else { m })) })
}

fn dmin(_: &Call, a: &[Arg]) -> Val {
    with_numbers(a, |v| if v.is_empty() { num(0.0) } else { num(v.iter().fold(f64::INFINITY, |m, x| if *x < m { *x } else { m })) })
}

fn dget(_: &Call, a: &[Arg]) -> Val {
    match selected(a) {
        Ok(vals) => {
            if vals.len() > 1 {
                return err(ErrorType::Num);
            }
            match vals.into_iter().next() {
                None => err(ErrorType::Value),
                Some(Scalar::Err(e)) => err(e),
                Some(Scalar::Empty) => num(0.0),
                Some(v) => Val::S(v),
            }
        }
        Err(e) => err(e),
    }
}

fn sq_dev(v: &[f64]) -> f64 {
    let mean = v.iter().fold(0.0, |s, x| s + x) / v.len() as f64;
    v.iter().fold(0.0, |s, x| s + js_pow(x - mean, 2.0))
}

fn dstdev(_: &Call, a: &[Arg]) -> Val {
    with_numbers(a, |v| if v.len() <= 1 { err(ErrorType::Div0) } else { num((sq_dev(v) / (v.len() as f64 - 1.0)).sqrt()) })
}

fn dstdevp(_: &Call, a: &[Arg]) -> Val {
    with_numbers(a, |v| if v.is_empty() { err(ErrorType::Div0) } else { num((sq_dev(v) / v.len() as f64).sqrt()) })
}

fn dvar(_: &Call, a: &[Arg]) -> Val {
    with_numbers(a, |v| if v.len() <= 1 { err(ErrorType::Div0) } else { num(sq_dev(v) / (v.len() as f64 - 1.0)) })
}

fn dvarp(_: &Call, a: &[Arg]) -> Val {
    with_numbers(a, |v| if v.is_empty() { err(ErrorType::Div0) } else { num(sq_dev(v) / v.len() as f64) })
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    simple(&[
        ("DSUM", dsum),
        ("DAVERAGE", daverage),
        ("DCOUNT", dcount),
        ("DCOUNTA", dcounta),
        ("DPRODUCT", dproduct),
        ("DMAX", dmax),
        ("DMIN", dmin),
        ("DGET", dget),
        ("DSTDEV", dstdev),
        ("DSTDEVP", dstdevp),
        ("DVAR", dvar),
        ("DVARP", dvarp),
    ])
}
