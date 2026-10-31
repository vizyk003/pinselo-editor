// SPDX-License-Identifier: Apache-2.0
//! Small helpers over ProseMirror JSON nodes ([`oc_crdt::doc_body::PmNode`]) with JS semantics.

use oc_crdt::doc_body::{PmMark, PmNode};
use serde_json::Value;

/// `n.attrs?.[k]` (`None` for a missing attribute; JSON `null` is `Some(Null)`).
pub fn attr<'a>(n: &'a PmNode, k: &str) -> Option<&'a Value> {
    n.attrs.as_ref().and_then(|a| a.get(k))
}

/// `n.attrs?.[k] ?? fallback` as a JSON value (`null` counts as missing).
pub fn attr_or<'a>(n: &'a PmNode, k: &str) -> Option<&'a Value> {
    attr(n, k).filter(|v| !v.is_null())
}

/// `typeof n.attrs?.[k] === 'string' ? n.attrs[k] : None`.
pub fn attr_str<'a>(n: &'a PmNode, k: &str) -> Option<&'a str> {
    attr(n, k).and_then(Value::as_str)
}

/// A string attribute that is also non-empty (`typeof x === 'string' && x`).
pub fn attr_nonempty<'a>(n: &'a PmNode, k: &str) -> Option<&'a str> {
    attr_str(n, k).filter(|s| !s.is_empty())
}

/// `Number(n.attrs?.[k] ?? fallback)`.
pub fn attr_num(n: &PmNode, k: &str, fallback: f64) -> f64 {
    attr_or(n, k).map_or(fallback, oc_schema::js::value_to_number)
}

/// JS truthiness of an attribute (`undefined` is falsy).
pub fn attr_truthy(n: &PmNode, k: &str) -> bool {
    attr(n, k).is_some_and(oc_schema::tree::truthy)
}

pub fn children(n: &PmNode) -> &[PmNode] {
    n.content.as_deref().unwrap_or(&[])
}

pub fn marks(n: &PmNode) -> &[PmMark] {
    n.marks.as_deref().unwrap_or(&[])
}

/// `m.attrs?.[k]`.
pub fn mark_attr<'a>(m: &'a PmMark, k: &str) -> Option<&'a Value> {
    m.attrs.as_ref().and_then(|a| a.get(k))
}

/// `Math.max(a, b)` (NaN propagates).
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// `Math.min(a, b)` (NaN propagates).
pub fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// `s.repeat(n)` with `ToIntegerOrInfinity` (NaN → 0, fractions truncated; negatives are clamped to 0 here, where
/// JS throws — no caller passes one).
pub fn repeat(s: &str, n: f64) -> String {
    if n.is_nan() || n < 1.0 {
        return String::new();
    }
    s.repeat(n.trunc() as usize)
}

/// `String#slice(0, n)` on UTF-16 code units; a surrogate pair cut in half becomes U+FFFD (what TextEncoder
/// writes for the lone surrogate JS leaves).
pub fn slice16(s: &str, n: usize) -> String {
    let u: Vec<u16> = s.encode_utf16().collect();
    if u.len() <= n {
        return s.to_string();
    }
    String::from_utf16_lossy(&u[..n])
}

/// `String#trimEnd`.
pub fn trim_end(s: &str) -> &str {
    s.trim_end_matches(oc_schema::js::is_js_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_helpers() {
        assert_eq!(repeat("#", f64::NAN), "");
        assert_eq!(repeat("#", 2.5), "##");
        assert!(js_max(1.0, js_min(6.0, f64::NAN)).is_nan());
        assert_eq!(slice16("ab🎉", 3), "ab\u{FFFD}");
        assert_eq!(slice16("ab🎉", 4), "ab🎉");
        assert_eq!(trim_end("x \u{a0}\n"), "x");
    }
}
