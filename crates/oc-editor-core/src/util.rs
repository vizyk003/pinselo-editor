// SPDX-License-Identifier: Apache-2.0
//! Small JS-semantics helpers shared by the modules (JSON plumbing, `Math`, UTF-16 strings).

use serde::Serialize;
use serde_json::{Map, Value};

pub use oc_geometry::jsmath::{atan2, cos, hypot, max, min, sin};
pub use oc_schema::js::round;

/// Serialise with JS number formatting (`1080`, not `1080.0`). Serialising the crate's own types cannot fail.
pub fn jv<T: Serialize + ?Sized>(t: &T) -> Value {
    oc_schema::js::to_value(t).unwrap_or(Value::Null)
}

/// `jv` for values that serialise to an object.
pub fn jm<T: Serialize + ?Sized>(t: &T) -> Map<String, Value> {
    match jv(t) {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// Build a JSON object from `(key, value)` pairs.
pub fn obj<const N: usize>(pairs: [(&str, Value); N]) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// A JS number as JSON (integers without a fraction, like `JSON.stringify`).
pub fn num(x: f64) -> Value {
    oc_crdt::value::f64_to_json(x)
}

/// `{ ...a, ...b }`.
pub fn merge(a: &Map<String, Value>, b: &Map<String, Value>) -> Map<String, Value> {
    let mut out = a.clone();
    for (k, v) in b {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// Parse a typed value from JSON (`None` when the shape does not match).
pub fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> Option<T> {
    serde_json::from_value(v.clone()).ok()
}

/// JS truthiness of an optional JSON value.
pub fn truthy(v: Option<&Value>) -> bool {
    oc_crdt::value::truthy_opt(v)
}

pub fn f(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

pub fn s(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

// ------------------------------------------------------------------ UTF-16 strings (JS string indexes)

pub fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn from_utf16(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
}

/// `s.length`.
pub fn len16(s: &str) -> i64 {
    s.encode_utf16().count() as i64
}

/// `s.slice(a, b)` with JS clamping of non-negative indexes.
pub fn slice16(s: &str, a: i64, b: i64) -> String {
    let u = utf16(s);
    let n = u.len() as i64;
    let a = a.clamp(0, n) as usize;
    let b = b.clamp(0, n) as usize;
    if b <= a { String::new() } else { from_utf16(&u[a..b]) }
}

/// `s.lastIndexOf(c, from)` for a single UTF-16 unit (`from` < 0 searches nothing … JS clamps to 0).
pub fn last_index_of16(s: &str, c: u16, from: i64) -> i64 {
    let u = utf16(s);
    if u.is_empty() {
        return -1;
    }
    let start = from.clamp(0, u.len() as i64 - 1) as usize;
    (0..=start).rev().find(|&i| u[i] == c).map_or(-1, |i| i as i64)
}

/// `s.indexOf(c, from)` for a single UTF-16 unit.
pub fn index_of16(s: &str, c: u16, from: i64) -> i64 {
    let u = utf16(s);
    let start = from.max(0) as usize;
    (start..u.len()).find(|&i| u[i] == c).map_or(-1, |i| i as i64)
}

/// Extended grapheme cluster boundaries of `s` as UTF-16 offsets (always starts with 0 and ends with `s.length`),
/// like `Intl.Segmenter(…, { granularity: 'grapheme' })` in the TS editor.
pub fn grapheme_bounds16(s: &str) -> Vec<i64> {
    let u = utf16(s);
    let mut b: Vec<i64> = icu_segmenter::GraphemeClusterSegmenter::new().segment_utf16(&u).map(|i| i as i64).collect();
    if b.first() != Some(&0) {
        b.insert(0, 0);
    }
    if b.last() != Some(&(u.len() as i64)) {
        b.push(u.len() as i64);
    }
    b
}

/// The grapheme boundary before `offset` (the start of the cluster `offset` is in or follows; 0 at the start).
/// Caret steps use it so they never split a surrogate pair, a combining sequence or a ZWJ emoji.
pub fn prev_grapheme16(s: &str, offset: i64) -> i64 {
    grapheme_bounds16(s).into_iter().rev().find(|&b| b < offset).unwrap_or(0)
}

/// The grapheme boundary after `offset` (`s.length` at the end).
pub fn next_grapheme16(s: &str, offset: i64) -> i64 {
    let b = grapheme_bounds16(s);
    let end = *b.last().unwrap_or(&0);
    b.into_iter().find(|&b| b > offset).unwrap_or(end)
}

/// UTF-16 offset of a byte index in `s`.
pub fn byte_to_utf16(s: &str, byte: usize) -> i64 {
    s[..byte].encode_utf16().count() as i64
}

/// JS `\s` test.
pub fn has_js_space(s: &str) -> bool {
    s.chars().any(oc_schema::js::is_js_space)
}

/// `Array#sort` comparator `a - b` as an ordering (NaN compares equal).
pub fn num_cmp(a: f64, b: f64) -> std::cmp::Ordering {
    let d = a - b;
    if d < 0.0 {
        std::cmp::Ordering::Less
    } else if d > 0.0 {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Equal
    }
}
