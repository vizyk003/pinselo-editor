// SPDX-License-Identifier: AGPL-3.0-only
//! Array-producing plugins: ArrayPlugin (ARRAYFORMULA, ARRAY_CONSTRAIN, FILTER, VSTACK, HSTACK), MatrixPlugin
//! (MMULT, TRANSPOSE, MAXPOOL, MEDIANPOOL), SequencePlugin, SortPlugin, UniquePlugin. Result sizes are
//! predicted statically in `formula::array_size`.

use super::super::coerce;
use super::super::func::{Arg, Call};
use super::super::value::{ErrorType, RangeVal, Scalar, Val};
use super::{Kind, err, simple};

/// Configured sheet limits (`maxRows` / `maxColumns`).
const MAX_ROWS: f64 = 200_000.0;
const MAX_COLUMNS: f64 = 1000.0;

/// Stable merge sort with a three-way comparator that never panics on inconsistent orderings.
pub fn stable_sort<T: Clone>(v: &mut [T], cmp: &impl Fn(&T, &T) -> i32) {
    let n = v.len();
    if n <= 1 {
        return;
    }
    let mid = n / 2;
    stable_sort(&mut v[..mid], cmp);
    stable_sort(&mut v[mid..], cmp);
    let left: Vec<T> = v[..mid].to_vec();
    let right: Vec<T> = v[mid..].to_vec();
    let (mut i, mut j, mut k) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        if cmp(&right[j], &left[i]) < 0 {
            v[k] = right[j].clone();
            j += 1;
        } else {
            v[k] = left[i].clone();
            i += 1;
        }
        k += 1;
    }
    while i < left.len() {
        v[k] = left[i].clone();
        i += 1;
        k += 1;
    }
    while j < right.len() {
        v[k] = right[j].clone();
        j += 1;
        k += 1;
    }
}

fn first_error(r: &RangeVal) -> Option<ErrorType> {
    r.values().find_map(|v| if let Scalar::Err(e) = v { Some(*e) } else { None })
}

fn arrayformula(_: &Call, a: &[Arg]) -> Val {
    a[0].clone().into_val()
}

fn array_constrain(_: &Call, a: &[Arg]) -> Val {
    let r = a[0].to_range();
    let rows = a[1].num().min(r.height() as f64);
    let cols = a[2].num().min(r.width() as f64);
    let mut out = Vec::new();
    let mut i = 0.0;
    while i < rows {
        let row = &r.data[i as usize];
        out.push(row[..(cols.max(0.0) as usize).min(row.len())].to_vec());
        i += 1.0;
    }
    if out.is_empty() || out[0].is_empty() {
        return err(ErrorType::Value);
    }
    Val::R(RangeVal::only_values(out))
}

fn filter(_: &Call, a: &[Arg]) -> Val {
    let vals = a[0].to_range();
    let filters: Vec<RangeVal> = a[1..].iter().map(Arg::to_range).collect();
    for f in &filters {
        if vals.width() != f.width() || vals.height() != f.height() {
            return err(ErrorType::Na);
        }
    }
    if vals.width() > 1 && vals.height() > 1 {
        return err(ErrorType::Na);
    }
    let mut ret = Vec::new();
    for i in 0..vals.height() {
        let mut row = Vec::new();
        for j in 0..vals.width() {
            let ok = filters.iter().all(|f| coerce::to_boolean(&f.data[i][j]) == Some(Scalar::Bool(true)));
            if ok {
                row.push(vals.data[i][j].clone());
            }
        }
        if !row.is_empty() {
            ret.push(row);
        }
    }
    if ret.is_empty() {
        return err(ErrorType::Na);
    }
    Val::R(RangeVal::only_values(ret))
}

fn vstack(_: &Call, a: &[Arg]) -> Val {
    let ranges: Vec<RangeVal> = a.iter().map(Arg::to_range).collect();
    let width = ranges.iter().map(RangeVal::width).max().unwrap_or(0);
    let mut out = Vec::new();
    for r in &ranges {
        for row in &r.data {
            let mut p: Vec<Scalar> = row.iter().take(width).cloned().collect();
            while p.len() < width {
                p.push(Scalar::Err(ErrorType::Na));
            }
            out.push(p);
        }
    }
    if out.is_empty() || out[0].is_empty() {
        return err(ErrorType::Value);
    }
    Val::R(RangeVal::only_values(out))
}

fn hstack(_: &Call, a: &[Arg]) -> Val {
    let ranges: Vec<RangeVal> = a.iter().map(Arg::to_range).collect();
    let height = ranges.iter().map(RangeVal::height).max().unwrap_or(0);
    let mut out: Vec<Vec<Scalar>> = vec![Vec::new(); height];
    for r in &ranges {
        let w = r.width();
        for (row, line) in out.iter_mut().enumerate() {
            for col in 0..w {
                line.push(r.data.get(row).and_then(|x| x.get(col)).cloned().unwrap_or(Scalar::Err(ErrorType::Na)));
            }
        }
    }
    if out.is_empty() || out[0].is_empty() {
        return err(ErrorType::Value);
    }
    Val::R(RangeVal::only_values(out))
}

fn raw_numbers(r: &RangeVal) -> Vec<Vec<f64>> {
    r.data.iter().map(|row| row.iter().map(|v| v.as_num().unwrap_or(f64::NAN)).collect()).collect()
}

fn numbers_val(data: Vec<Vec<f64>>) -> Val {
    if data.is_empty() || data[0].is_empty() {
        return err(ErrorType::Value);
    }
    Val::R(RangeVal::only_values(data.into_iter().map(|row| row.into_iter().map(Scalar::num).collect()).collect()))
}

fn mmult(_: &Call, a: &[Arg]) -> Val {
    let (l, r) = (a[0].to_range(), a[1].to_range());
    if !l.has_only_numbers() || !r.has_only_numbers() {
        return err(ErrorType::Value);
    }
    if r.height() != l.width() {
        return err(ErrorType::Value);
    }
    let (ln, rn) = (raw_numbers(&l), raw_numbers(&r));
    let w = l.width();
    let out: Vec<Vec<f64>> = (0..l.height())
        .map(|y| {
            (0..r.width())
                .map(|x| {
                    let mut sum = 0.0;
                    for i in 0..w {
                        sum += ln[y][i] * rn[i][x];
                    }
                    sum
                })
                .collect()
        })
        .collect();
    numbers_val(out)
}

/// Output size of a pooling window, `None` when it is not a positive whole number of windows.
fn pool_size(w: usize, h: usize, window: f64, stride: f64) -> Option<(usize, usize)> {
    let ow = 1.0 + (w as f64 - window) / stride;
    let oh = 1.0 + (h as f64 - window) / stride;
    if !(ow >= 1.0 && oh >= 1.0 && ow.fract() == 0.0 && oh.fract() == 0.0 && window >= 1.0 && stride >= 1.0) {
        return None;
    }
    Some((ow as usize, oh as usize))
}

fn pool(a: &[Arg], median: bool) -> Val {
    let m = a[0].to_range();
    let window = a[1].num();
    let stride = a.get(2).and_then(Arg::opt_num).unwrap_or(window);
    if !m.has_only_numbers() {
        return err(ErrorType::Value);
    }
    let Some((ow, oh)) = pool_size(m.width(), m.height(), window, stride) else { return err(ErrorType::Value) };
    let data = raw_numbers(&m);
    let (wn, st) = (window as usize, stride as usize);
    let out: Vec<Vec<f64>> = (0..oh)
        .map(|ty| {
            (0..ow)
                .map(|tx| {
                    let (cx, cy) = (tx * st, ty * st);
                    let at = |i: usize, j: usize| data.get(cy + i).and_then(|r| r.get(cx + j)).copied().unwrap_or(f64::NAN);
                    let mut mx = at(0, 0);
                    for i in 0..wn {
                        for j in 0..wn {
                            mx = js_max(mx, at(i, j));
                        }
                    }
                    if !median {
                        return mx;
                    }
                    let mut mn = at(0, 0);
                    for i in 0..wn {
                        for j in 0..wn {
                            mn = js_min(mn, at(i, j));
                        }
                    }
                    let n = (wn * wn) as f64;
                    let (mut lo, mut hi) = (mn, mx);
                    let mut result = 42.0;
                    for _ in 0..32 {
                        let guess = (lo + hi) / 2.0;
                        let mut count = 0.0;
                        for i in 0..wn {
                            for j in 0..wn {
                                if at(i, j) > guess {
                                    count += 1.0;
                                }
                            }
                        }
                        let target = if wn % 2 == 0 { n / 2.0 } else { (n - 1.0) / 2.0 };
                        if count == target {
                            result = guess;
                            break;
                        } else if count > target {
                            lo = guess;
                        } else {
                            hi = guess;
                        }
                    }
                    result
                })
                .collect()
        })
        .collect();
    numbers_val(out)
}

fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

fn maxpool(_: &Call, a: &[Arg]) -> Val {
    pool(a, false)
}

fn medianpool(_: &Call, a: &[Arg]) -> Val {
    pool(a, true)
}

fn transpose(_: &Call, a: &[Arg]) -> Val {
    let m = a[0].to_range();
    let (w, h) = (m.width(), m.height());
    if w == 0 || h == 0 {
        return err(ErrorType::Value);
    }
    let out: Vec<Vec<Scalar>> = (0..w).map(|i| (0..h).map(|j| m.data[j][i].clone()).collect()).collect();
    Val::R(RangeVal::only_values(out))
}

fn sequence(_: &Call, a: &[Arg]) -> Val {
    let (rows, cols, start, step) = (a[0].num(), a[1].num(), a[2].num(), a[3].num());
    if !rows.is_finite() || !cols.is_finite() {
        return err(ErrorType::Value);
    }
    if rows < 0.0 || cols < 0.0 {
        return err(ErrorType::Value);
    }
    let (nr, nc) = (rows.trunc(), cols.trunc());
    if nr < 1.0 || nc < 1.0 {
        return err(ErrorType::Value);
    }
    if nr > MAX_ROWS || nc > MAX_COLUMNS {
        return err(ErrorType::Value);
    }
    let (nr, nc) = (nr as usize, nc as usize);
    let out: Vec<Vec<f64>> =
        (0..nr).map(|r| (0..nc).map(|c| start + ((r * nc + c) as f64) * step).collect()).collect();
    numbers_val(out)
}

fn sort(_: &Call, a: &[Arg]) -> Val {
    let r = a[0].to_range();
    let (sort_index, order, by_col) = (a[1].num(), a[2].num(), a[3].bool());
    if order != 1.0 && order != -1.0 {
        return err(ErrorType::Value);
    }
    if let Some(e) = first_error(&r) {
        return err(e);
    }
    if r.height() == 0 || r.width() == 0 {
        return err(ErrorType::Na);
    }
    let index = sort_index.trunc();
    let dim = if by_col { r.height() } else { r.width() } as f64;
    if index < 1.0 {
        return err(ErrorType::Value);
    }
    if index > dim {
        return err(ErrorType::Value);
    }
    let key = index as usize - 1;
    let ord = order as i32;
    let cmp = |x: &Scalar, y: &Scalar| -> i32 {
        let (xe, ye) = (*x == Scalar::Empty, *y == Scalar::Empty);
        if xe || ye {
            return if xe && ye {
                0
            } else if xe {
                1
            } else {
                -1
            };
        }
        let c = coerce::compare(x, y);
        if c < 0 {
            -ord
        } else if c > 0 {
            ord
        } else {
            0
        }
    };
    if by_col {
        let mut order_ix: Vec<usize> = (0..r.width()).collect();
        let keys = &r.data[key];
        stable_sort(&mut order_ix, &|c1: &usize, c2: &usize| cmp(&keys[*c1], &keys[*c2]));
        let out: Vec<Vec<Scalar>> = r.data.iter().map(|row| order_ix.iter().map(|c| row[*c].clone()).collect()).collect();
        return Val::R(RangeVal::only_values(out));
    }
    let mut rows = r.data.clone();
    stable_sort(&mut rows, &|r1: &Vec<Scalar>, r2: &Vec<Scalar>| cmp(&r1[key], &r2[key]));
    Val::R(RangeVal::only_values(rows))
}

fn unique(_: &Call, a: &[Arg]) -> Val {
    let r = a[0].to_range();
    let (by_col, exactly_once) = (a[1].bool(), a[2].bool());
    if let Some(e) = first_error(&r) {
        return err(e);
    }
    let transpose_data = |d: &[Vec<Scalar>]| -> Vec<Vec<Scalar>> {
        if d.is_empty() {
            return vec![];
        }
        (0..d[0].len()).map(|c| d.iter().map(|row| row[c].clone()).collect()).collect()
    };
    let vectors = if by_col { transpose_data(&r.data) } else { r.data.clone() };
    let equal = |v1: &[Scalar], v2: &[Scalar]| v1.iter().enumerate().all(|(i, x)| v2.get(i).is_some_and(|y| coerce::compare(x, y) == 0));
    let mut distinct: Vec<Vec<Scalar>> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();
    for v in vectors {
        match distinct.iter().position(|d| equal(d, &v)) {
            Some(i) => counts[i] += 1,
            None => {
                distinct.push(v);
                counts.push(1);
            }
        }
    }
    let kept: Vec<Vec<Scalar>> =
        if exactly_once { distinct.into_iter().zip(counts).filter(|(_, c)| *c == 1).map(|(d, _)| d).collect() } else { distinct };
    if kept.is_empty() {
        return err(ErrorType::Na);
    }
    let out = if by_col { transpose_data(&kept) } else { kept };
    if out.is_empty() || out[0].is_empty() {
        return err(ErrorType::Na);
    }
    Val::R(RangeVal::only_values(out))
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    simple(&[
        ("ARRAYFORMULA", arrayformula),
        ("ARRAY_CONSTRAIN", array_constrain),
        ("FILTER", filter),
        ("VSTACK", vstack),
        ("HSTACK", hstack),
        ("MMULT", mmult),
        ("MAXPOOL", maxpool),
        ("MEDIANPOOL", medianpool),
        ("TRANSPOSE", transpose),
        ("SEQUENCE", sequence),
        ("SORT", sort),
        ("UNIQUE", unique),
    ])
}
