// SPDX-License-Identifier: AGPL-3.0-only
//! LookupPlugin: VLOOKUP, HLOOKUP, XLOOKUP, MATCH — with HyperFormula's search strategies (linear search over
//! normalised values for unsorted data, binary search with empty-cell compaction for sorted ranges, wildcard
//! matching for exact-match modes).

use std::cmp::Ordering;

use super::super::coerce::{Pattern, normalize_base, requires_regex};
use super::super::func::{Arg, Call};
use super::super::value::{ErrorType, RangeVal, Scalar, Val};
use super::{Kind, err, num, simple};

const NOT_FOUND: i64 = -1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    None,
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoMatch {
    NotFound,
    LowerBound,
    UpperBound,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchOpts {
    pub ordering: Order,
    pub if_no_match: NoMatch,
    pub last: bool,
}

/// A JS value as the search code sees it (`getRawValue` of an interpreter value).
#[derive(Debug, Clone, PartialEq)]
enum Js {
    Empty,
    Num(f64),
    Str(String),
    Bool(bool),
    Err,
}

impl Js {
    fn of(s: &Scalar) -> Js {
        match s {
            Scalar::Empty => Js::Empty,
            Scalar::Num(v, _) => Js::Num(*v),
            Scalar::Str(t) => Js::Str(t.to_string()),
            Scalar::Bool(b) => Js::Bool(*b),
            Scalar::Err(_) => Js::Err,
        }
    }
    fn normalized(self) -> Js {
        match self {
            Js::Str(s) => Js::Str(normalize_base(&s)),
            o => o,
        }
    }
    fn type_tag(&self) -> u8 {
        match self {
            Js::Empty => 0,
            Js::Num(_) => 1,
            Js::Str(_) => 2,
            Js::Bool(_) => 3,
            Js::Err => 4,
        }
    }
    /// `===`
    fn strict_eq(&self, o: &Js) -> bool {
        match (self, o) {
            (Js::Empty, Js::Empty) => true,
            (Js::Num(a), Js::Num(b)) => a == b,
            (Js::Str(a), Js::Str(b)) => a == b,
            (Js::Bool(a), Js::Bool(b)) => a == b,
            _ => false,
        }
    }
}

fn js_str_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `compare` of `binarySearch.ts`.
fn compare(left: &Js, right: &Js) -> i32 {
    if left.type_tag() == right.type_tag() {
        return match (left, right) {
            (Js::Empty, _) | (Js::Err, _) => 0,
            (Js::Num(a), Js::Num(b)) => {
                if a < b {
                    -1
                } else if a > b {
                    1
                } else {
                    0
                }
            }
            (Js::Str(a), Js::Str(b)) => match js_str_cmp(a, b) {
                Ordering::Less => -1,
                Ordering::Greater => 1,
                Ordering::Equal => 0,
            },
            (Js::Bool(a), Js::Bool(b)) => (*a as i32) - (*b as i32),
            _ => 0,
        };
    }
    if *left == Js::Empty {
        return -1;
    }
    if *right == Js::Empty {
        return 1;
    }
    if *right == Js::Err {
        return -1;
    }
    match (left, right) {
        (Js::Num(_), Js::Str(_)) | (Js::Num(_), Js::Bool(_)) => -1,
        (Js::Str(_), Js::Num(_)) => 1,
        (Js::Str(_), Js::Bool(_)) => -1,
        _ => 1,
    }
}

/// `findLastMatchingIndex`.
fn last_matching(pred: impl Fn(i64) -> bool, start: i64, end: i64) -> i64 {
    let (mut s, mut e) = (start, end);
    while s < e {
        let pivot = (s + e + 1).div_euclid(2);
        if pred(pivot) {
            s = pivot;
        } else {
            e = pivot - 1;
        }
    }
    if s == e && pred(s) {
        return s;
    }
    NOT_FOUND
}

/// `findLastOccurrenceInOrderedRange` over the (raw, not normalised) values of a sorted range.
fn ordered_find(key: &Js, values: &[Js], desc: bool, if_no_match: NoMatch) -> i64 {
    let end = values.len() as i64 - 1;
    let get = |i: i64| -> &Js { &values[i as usize] };
    let cmp = |l: &Js, r: &Js| if desc { -compare(l, r) } else { compare(l, r) };
    let next_non_empty = |from: i64| -> Option<i64> { (from..=end).find(|&i| *get(i) != Js::Empty) };
    let probed_empty = std::cell::Cell::new(false);
    let direct = last_matching(
        |i| {
            let v = get(i);
            if *v == Js::Empty {
                probed_empty.set(true);
            }
            cmp(key, v) >= 0
        },
        0,
        end,
    );
    let found = if !probed_empty.get() {
        direct
    } else if if_no_match == NoMatch::NotFound && direct != NOT_FOUND && get(direct).strict_eq(key) {
        return direct;
    } else {
        let idx: Vec<i64> = (0..=end).filter(|&i| *get(i) != Js::Empty).collect();
        let f = last_matching(|c| cmp(key, get(idx[c as usize])) >= 0, 0, idx.len() as i64 - 1);
        if f == NOT_FOUND { NOT_FOUND } else { idx[f as usize] }
    };
    let found_value = if found == NOT_FOUND { Js::Empty } else { get(found).clone() };
    if found_value.strict_eq(key) && found != NOT_FOUND {
        return found;
    }
    match if_no_match {
        NoMatch::LowerBound => {
            if found == NOT_FOUND {
                if !desc {
                    return NOT_FOUND;
                }
                return next_non_empty(0).unwrap_or(NOT_FOUND);
            }
            if found_value.type_tag() != key.type_tag() {
                return NOT_FOUND;
            }
            if !desc {
                return found;
            }
            next_non_empty(found + 1).unwrap_or(NOT_FOUND)
        }
        NoMatch::UpperBound => {
            if found == NOT_FOUND {
                if desc {
                    return NOT_FOUND;
                }
                return next_non_empty(0).unwrap_or(NOT_FOUND);
            }
            if found_value.type_tag() != key.type_tag() {
                return NOT_FOUND;
            }
            if desc {
                return found;
            }
            next_non_empty(found + 1).unwrap_or(NOT_FOUND)
        }
        NoMatch::NotFound => NOT_FOUND,
    }
}

/// `findNormalizedValue` (linear search over normalised values).
fn normalized_find(key: &Js, values: &[Js], if_no_match: NoMatch, last: bool) -> i64 {
    let arr: Vec<Js> = values.iter().cloned().map(Js::normalized).collect();
    let order: Box<dyn Iterator<Item = usize>> = if last { Box::new((0..arr.len()).rev()) } else { Box::new(0..arr.len()) };
    if if_no_match == NoMatch::NotFound {
        for i in order {
            if arr[i].strict_eq(key) {
                return i as i64;
            }
        }
        return NOT_FOUND;
    }
    let lower = if_no_match == NoMatch::LowerBound;
    let cmp = |l: &Js, r: &Js| if lower { compare(l, r) } else { -compare(l, r) };
    let mut best = Js::Num(if lower { f64::NEG_INFINITY } else { f64::INFINITY });
    let mut best_index = NOT_FOUND;
    for i in order {
        let v = &arr[i];
        if v.strict_eq(key) {
            return i as i64;
        }
        if *v == Js::Empty {
            continue;
        }
        if cmp(v, key) > 0 {
            continue;
        }
        if cmp(&best, v) < 0 {
            best = v.clone();
            best_index = i as i64;
        }
    }
    best_index
}

/// `basicFind`: `range_backed` = the searched values come from a sheet range (binary search when sorted).
pub fn basic_find(key: &Scalar, values: &[Scalar], range_backed: bool, opts: SearchOpts) -> i64 {
    let key = Js::of(key).normalized();
    let vals: Vec<Js> = values.iter().map(Js::of).collect();
    if !range_backed || opts.ordering == Order::None {
        return normalized_find(&key, &vals, opts.if_no_match, opts.last);
    }
    ordered_find(&key, &vals, opts.ordering == Order::Desc, opts.if_no_match)
}

/// `searchInRange`: wildcard patterns go through `advancedFind`.
fn search(key: &Scalar, values: &[Scalar], range_backed: bool, wildcard: bool, opts: SearchOpts) -> i64 {
    if wildcard
        && let Scalar::Str(k) = key
        && requires_regex(k)
    {
        let p = Pattern::new(k, true);
        let hit = |v: &Scalar| matches!(v, Scalar::Str(s) if p.matches(s));
        let found = if opts.last { values.iter().rposition(hit) } else { values.iter().position(hit) };
        return found.map_or(NOT_FOUND, |i| i as i64);
    }
    basic_find(key, values, range_backed, opts)
}

fn zero_if_empty(s: Scalar) -> Scalar {
    if s == Scalar::Empty { Scalar::num(0.0) } else { s }
}

fn first_col(r: &RangeVal) -> Vec<Scalar> {
    r.data.iter().map(|row| row.first().cloned().unwrap_or(Scalar::Empty)).collect()
}

fn first_row(r: &RangeVal) -> Vec<Scalar> {
    r.data.first().cloned().unwrap_or_default()
}

/// A cell of a looked-up range at a (possibly fractional) offset: fractional offsets address no cell.
fn cell_at(r: &RangeVal, row: f64, col: f64) -> Scalar {
    if row.fract() != 0.0 || col.fract() != 0.0 || row < 0.0 || col < 0.0 {
        return Scalar::Empty;
    }
    r.data.get(row as usize).and_then(|x| x.get(col as usize)).cloned().unwrap_or(Scalar::Empty)
}

fn sorted_opts(sorted: bool) -> SearchOpts {
    SearchOpts {
        ordering: if sorted { Order::Asc } else { Order::None },
        if_no_match: if sorted { NoMatch::LowerBound } else { NoMatch::NotFound },
        last: false,
    }
}

fn vlookup(_: &Call, a: &[Arg]) -> Val {
    let key = zero_if_empty(a[0].scalar());
    let Some(r) = a[1].range() else { return err(ErrorType::Value) };
    let Some(range) = r.range else { return err(ErrorType::Value) };
    let index = a[2].num();
    if index < 1.0 {
        return err(ErrorType::Value);
    }
    if index > range.width() as f64 {
        return err(ErrorType::Ref);
    }
    let opts = sorted_opts(a[3].bool());
    let row = search(&key, &first_col(r), true, opts.ordering == Order::None, opts);
    if row == NOT_FOUND {
        return err(ErrorType::Na);
    }
    Val::S(zero_if_empty(cell_at(r, row as f64, index - 1.0)))
}

fn hlookup(_: &Call, a: &[Arg]) -> Val {
    let key = zero_if_empty(a[0].scalar());
    let Some(r) = a[1].range() else { return err(ErrorType::Value) };
    let Some(range) = r.range else { return err(ErrorType::Value) };
    let index = a[2].num();
    if index < 1.0 {
        return err(ErrorType::Value);
    }
    if index > range.height() as f64 {
        return err(ErrorType::Ref);
    }
    let opts = sorted_opts(a[3].bool());
    let col = search(&key, &first_row(r), true, opts.ordering == Order::None, opts);
    if col == NOT_FOUND {
        return err(ErrorType::Na);
    }
    Val::S(zero_if_empty(cell_at(r, index - 1.0, col as f64)))
}

fn xlookup(_: &Call, a: &[Arg]) -> Val {
    let key = zero_if_empty(a[0].scalar());
    let lookup = a[1].to_range();
    let ret = a[2].to_range();
    let not_found = a[3].scalar();
    let match_mode = a[4].num();
    let search_mode = a[5].num();
    if ![0.0, -1.0, 1.0, 2.0].contains(&match_mode) {
        return err(ErrorType::Value);
    }
    if ![1.0, -1.0, 2.0, -2.0].contains(&search_mode) {
        return err(ErrorType::Value);
    }
    let opts = SearchOpts {
        ordering: if search_mode == 2.0 {
            Order::Asc
        } else if search_mode == -2.0 {
            Order::Desc
        } else {
            Order::None
        },
        last: search_mode == -1.0,
        if_no_match: if match_mode == -1.0 {
            NoMatch::LowerBound
        } else if match_mode == 1.0 {
            NoMatch::UpperBound
        } else {
            NoMatch::NotFound
        },
    };
    let vertical = lookup.width() == 1 && ret.height() == lookup.height();
    let horizontal = lookup.height() == 1 && ret.width() == lookup.width();
    if !vertical && !horizontal {
        return err(ErrorType::Value);
    }
    let values = if vertical { first_col(&lookup) } else { first_row(&lookup) };
    let found = search(&key, &values, !lookup.is_adhoc(), match_mode == 2.0, opts);
    if found == NOT_FOUND {
        // `notFoundFlag == ErrorType.NA` (the default is the enum string "NA").
        return match &not_found {
            Scalar::Str(s) if &**s == "NA" => err(ErrorType::Na),
            o => Val::S(o.clone()),
        };
    }
    let i = found as usize;
    let data: Vec<Vec<Scalar>> = if vertical {
        vec![ret.data[i].iter().cloned().map(zero_if_empty).collect()]
    } else {
        ret.data.iter().map(|row| vec![zero_if_empty(row.get(i).cloned().unwrap_or(Scalar::Empty))]).collect()
    };
    Val::R(RangeVal::only_values(data))
}

fn match_fn(_: &Call, a: &[Arg]) -> Val {
    let key = zero_if_empty(a[0].scalar());
    let r = a[1].to_range();
    let t = a[2].num();
    if ![-1.0, 0.0, 1.0].contains(&t) {
        return err(ErrorType::Value);
    }
    if r.width() > 1 && r.height() > 1 {
        return err(ErrorType::Na);
    }
    let opts = if t == 0.0 {
        SearchOpts { ordering: Order::None, if_no_match: NoMatch::NotFound, last: false }
    } else {
        SearchOpts {
            ordering: if t == -1.0 { Order::Desc } else { Order::Asc },
            if_no_match: if t == -1.0 { NoMatch::UpperBound } else { NoMatch::LowerBound },
            last: false,
        }
    };
    let values: Vec<Scalar> = if r.is_adhoc() {
        r.values().cloned().collect()
    } else if r.width() == 1 {
        first_col(&r)
    } else {
        first_row(&r)
    };
    let index = basic_find(&key, &values, !r.is_adhoc(), opts);
    if index == NOT_FOUND {
        return err(ErrorType::Na);
    }
    num(index as f64 + 1.0)
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    simple(&[("VLOOKUP", vlookup), ("HLOOKUP", hlookup), ("XLOOKUP", xlookup), ("MATCH", match_fn)])
}
