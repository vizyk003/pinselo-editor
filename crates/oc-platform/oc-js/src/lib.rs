//! JavaScript-compatible primitives.
//!
//! Several platform contracts (audit hashes, webhook bodies, ticket bodies) are defined in terms of what
//! V8 prints: `JSON.stringify`, `Number.prototype.toString`, `Array.prototype.sort` (UTF-16 order),
//! `parseInt(s, 10)`, `encodeURIComponent` and `Date.prototype.toISOString`. This crate reproduces those
//! byte-for-byte so Rust and TypeScript produce identical strings.

use std::cmp::Ordering;
use std::fmt::{self, Write as _};

use chrono::{DateTime, Utc};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

/// `String(n)` / `JSON.stringify(n)` for a finite number. Non-finite values follow `String(n)`.
pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if n == 0.0 {
        return "0".into(); // also -0
    }
    ryu_js::Buffer::new().format_finite(n).to_owned()
}

/// `JSON.stringify(s)` for a string (quotes included).
pub fn stringify_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    push_str_json(&mut out, s);
    out
}

fn push_str_json(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Compare two strings the way `Array.prototype.sort()` does by default: by UTF-16 code units.
pub fn cmp_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// JSON with recursively sorted keys (UTF-16 order), primitives printed by `JSON.stringify`.
/// Mirrors `canonical()` in `packages/audit/src/index.ts`. Numbers are coerced to IEEE doubles first,
/// exactly as `JSON.parse` would.
pub fn canonical(v: &serde_json::Value) -> String {
    let mut out = String::new();
    canonical_into(&mut out, v);
    out
}

fn canonical_into(out: &mut String, v: &serde_json::Value) {
    use serde_json::Value;
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number_to_string(n.as_f64().unwrap_or(f64::NAN))),
        Value::String(s) => push_str_json(out, s),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical_into(out, x);
            }
            out.push(']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort_by(|a, b| cmp_utf16(a, b));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_str_json(out, k);
                out.push(':');
                canonical_into(out, &m[k.as_str()]);
            }
            out.push('}');
        }
    }
}

/// An order-preserving JSON value: what `JSON.parse` yields in JS (object keys keep source order,
/// numbers are doubles). `to_string()` is `JSON.stringify` of it.
#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsValue>),
    Object(Vec<(String, JsValue)>),
}

impl JsValue {
    /// `JSON.parse(text)`.
    pub fn parse(text: &str) -> Result<JsValue, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// `JSON.stringify(value)` (compact).
    pub fn stringify(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            JsValue::Null => out.push_str("null"),
            JsValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            JsValue::Number(n) => {
                if n.is_finite() {
                    out.push_str(&number_to_string(*n))
                } else {
                    out.push_str("null")
                }
            }
            JsValue::String(s) => push_str_json(out, s),
            JsValue::Array(a) => {
                out.push('[');
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    x.write(out);
                }
                out.push(']');
            }
            JsValue::Object(m) => {
                out.push('{');
                for (i, (k, x)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    push_str_json(out, k);
                    out.push(':');
                    x.write(out);
                }
                out.push('}');
            }
        }
    }
}

impl fmt::Display for JsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.stringify())
    }
}

impl<'de> Deserialize<'de> for JsValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = JsValue;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_unit<E>(self) -> Result<JsValue, E> {
                Ok(JsValue::Null)
            }
            fn visit_none<E>(self) -> Result<JsValue, E> {
                Ok(JsValue::Null)
            }
            fn visit_bool<E>(self, b: bool) -> Result<JsValue, E> {
                Ok(JsValue::Bool(b))
            }
            fn visit_i64<E>(self, n: i64) -> Result<JsValue, E> {
                Ok(JsValue::Number(n as f64))
            }
            fn visit_u64<E>(self, n: u64) -> Result<JsValue, E> {
                Ok(JsValue::Number(n as f64))
            }
            fn visit_f64<E>(self, n: f64) -> Result<JsValue, E> {
                Ok(JsValue::Number(n))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<JsValue, E> {
                Ok(JsValue::String(s.to_owned()))
            }
            fn visit_string<E: de::Error>(self, s: String) -> Result<JsValue, E> {
                Ok(JsValue::String(s))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<JsValue, A::Error> {
                let mut v = Vec::new();
                while let Some(x) = a.next_element()? {
                    v.push(x);
                }
                Ok(JsValue::Array(v))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<JsValue, A::Error> {
                let mut v: Vec<(String, JsValue)> = Vec::new();
                while let Some((k, x)) = a.next_entry::<String, JsValue>()? {
                    // JSON.parse: a repeated key keeps its first position and takes the last value.
                    if let Some(slot) = v.iter_mut().find(|(kk, _)| *kk == k) {
                        slot.1 = x;
                    } else {
                        v.push((k, x));
                    }
                }
                Ok(JsValue::Object(v))
            }
        }
        d.deserialize_any(V)
    }
}

impl From<&serde_json::Value> for JsValue {
    fn from(v: &serde_json::Value) -> Self {
        use serde_json::Value;
        match v {
            Value::Null => JsValue::Null,
            Value::Bool(b) => JsValue::Bool(*b),
            Value::Number(n) => JsValue::Number(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => JsValue::String(s.clone()),
            Value::Array(a) => JsValue::Array(a.iter().map(Into::into).collect()),
            Value::Object(m) => JsValue::Object(m.iter().map(|(k, x)| (k.clone(), x.into())).collect()),
        }
    }
}

/// ECMAScript `WhiteSpace` + `LineTerminator` (what `parseInt` and `String#trim` skip).
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{09}' | '\u{0A}' | '\u{0B}' | '\u{0C}' | '\u{0D}' | '\u{20}' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
            | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `Number.parseInt(s, 10)`; `None` is `NaN`.
pub fn parse_int(s: &str) -> Option<f64> {
    let s = s.trim_start_matches(is_js_whitespace);
    let (neg, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let end = rest.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    let v: f64 = rest[..end].parse().ok()?;
    Some(if neg { -v } else { v })
}

/// `encodeURIComponent(s)`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// `new Date(t).toISOString()`: millisecond precision (truncated), always `Z`.
pub fn iso_string(t: &DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// `Date.now()`.
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_like_v8() {
        for (n, s) in [
            (1.0, "1"),
            (-0.0, "0"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1.5e-7, "1.5e-7"),
            (0.000001, "0.000001"),
            (123.456, "123.456"),
            (-5e-324, "-5e-324"),
        ] {
            assert_eq!(number_to_string(n), s);
        }
    }

    #[test]
    fn parse_int_like_js() {
        assert_eq!(parse_int(" 42"), Some(42.0));
        assert_eq!(parse_int("123abc"), Some(123.0));
        assert_eq!(parse_int("-7"), Some(-7.0));
        assert_eq!(parse_int("0x10"), Some(0.0));
        assert_eq!(parse_int("1e3"), Some(1.0));
        assert_eq!(parse_int("abc"), None);
        assert_eq!(parse_int(""), None);
        assert_eq!(parse_int("+"), None);
    }

    #[test]
    fn utf16_sort() {
        // U+FF61 sorts after U+1F600 by code point, but before it in UTF-16 (surrogate 0xD83D < 0xFF61).
        assert_eq!(cmp_utf16("\u{1F600}", "\u{FF61}"), Ordering::Less);
    }

    #[test]
    fn js_value_roundtrip_keeps_order() {
        let v = JsValue::parse(r#"{"b": 1.50, "a": [true, null, "x\u0001"], "b": 2}"#).unwrap();
        assert_eq!(v.stringify(), r#"{"b":2,"a":[true,null,"x\u0001"]}"#);
    }

    #[test]
    fn uri_component() {
        assert_eq!(encode_uri_component("Noto Sans JP & ü"), "Noto%20Sans%20JP%20%26%20%C3%BC");
    }
}
