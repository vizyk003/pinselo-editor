// SPDX-License-Identifier: Apache-2.0
//! JavaScript-compatibility helpers.
//!
//! The TypeScript implementation relies on a few JS built-ins whose semantics differ from Rust's standard
//! library (`Math.round`, `Number#toString`, `Number#toFixed`, UTF-16 string lengths, `String#trim`,
//! `encodeURIComponent`). These helpers reproduce them exactly so outputs match byte-for-byte.

use serde::Serialize;
use serde_json::Value;

/// `Math.round`: rounds half toward +∞ (`Math.round(-2.5) === -2`).
pub fn round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// `Number.isInteger`.
pub fn is_integer(x: f64) -> bool {
    x.is_finite() && x.fract() == 0.0
}

/// `String(n)` / `Number#toString()` for finite and non-finite numbers.
pub fn num_to_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v == 0.0 {
        return "0".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let neg = v < 0.0;
    // Shortest round-trip digits, as JS does.
    let e = format!("{:e}", v.abs());
    let (mant, exp) = e.split_once('e').expect("exponent form");
    let exp: i32 = exp.parse().expect("exponent");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp + 1;
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if k <= n && n <= 21 {
        out.push_str(&digits);
        for _ in 0..(n - k) {
            out.push('0');
        }
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        for _ in 0..(-n) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if n > 0 { '+' } else { '-' });
        out.push_str(&(n - 1).abs().to_string());
    }
    out
}

/// `Number#toFixed(digits)`: exact decimal value, ties rounded up (away from zero in magnitude).
pub fn to_fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() || x.abs() >= 1e21 {
        return num_to_string(x);
    }
    let neg = x < 0.0;
    // Exact decimal expansion (f64 values have at most 1074 fractional digits).
    let exact = format!("{:.1100}", x.abs());
    let (int_part, frac_part) = exact.split_once('.').expect("fixed format");
    let mut ds: Vec<u8> = int_part.bytes().chain(frac_part.bytes().take(digits)).map(|b| b - b'0').collect();
    let round_up = frac_part.as_bytes().get(digits).is_some_and(|b| *b >= b'5');
    if round_up {
        let mut i = ds.len();
        loop {
            if i == 0 {
                ds.insert(0, 1);
                break;
            }
            i -= 1;
            if ds[i] == 9 {
                ds[i] = 0;
            } else {
                ds[i] += 1;
                break;
            }
        }
    }
    let int_len = ds.len() - digits;
    let mut s: String = ds[..int_len].iter().map(|d| (b'0' + d) as char).collect();
    if digits > 0 {
        s.push('.');
        s.extend(ds[int_len..].iter().map(|d| (b'0' + d) as char));
    }
    // JS keeps the sign of negative inputs (including "-0.00").
    if neg { format!("-{s}") } else { s }
}

/// `+x.toFixed(digits)`.
pub fn to_fixed_num(x: f64, digits: usize) -> f64 {
    let v: f64 = to_fixed(x, digits).parse().unwrap_or(f64::NAN);
    if v == 0.0 { 0.0 } else { v }
}

/// JS string comparison (`a < b`): lexicographic over UTF-16 code units.
pub fn str_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.as_bytes().cmp(b.as_bytes());
    }
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `String#length` (UTF-16 code units).
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// JS `WhiteSpace` + `LineTerminator` (what `String#trim` and regex `\s` match).
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{0009}' | '\u{000A}' | '\u{000B}' | '\u{000C}' | '\u{000D}' | '\u{0020}' | '\u{00A0}' | '\u{1680}'
            | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// Character class body equivalent to JS regex `\s`.
pub const JS_S: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// `String#trim`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `decodeURIComponent`; `None` where JS throws `URIError` (malformed escapes or invalid UTF-8).
pub fn decode_uri_component(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            let v = u8::from_str_radix(hex, 16).ok()?;
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// JS `parseFloat` over a prefix (used for CSS-ish numeric tokens already matched by a regex).
pub fn parse_float(s: &str) -> f64 {
    let t = trim(s);
    let mut end = 0;
    let b = t.as_bytes();
    let mut seen_dot = false;
    let mut seen_digit = false;
    if end < b.len() && (b[end] == b'+' || b[end] == b'-') {
        end += 1;
    }
    while end < b.len() {
        match b[end] {
            b'0'..=b'9' => seen_digit = true,
            b'.' if !seen_dot => seen_dot = true,
            _ => break,
        }
        end += 1;
    }
    if !seen_digit {
        return f64::NAN;
    }
    t[..end].trim_end_matches('.').parse().unwrap_or(f64::NAN)
}

/// JS `parseInt(s, 10)`.
pub fn parse_int10(s: &str) -> f64 {
    let t = trim(s);
    let (neg, rest) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return f64::NAN;
    }
    let v: f64 = digits.parse().unwrap_or(f64::NAN);
    if neg { -v } else { v }
}

/// JS `Number(s)` / unary `+s` for strings.
pub fn string_to_number(s: &str) -> f64 {
    let t = trim(s);
    if t.is_empty() {
        return 0.0;
    }
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(h, 16).map(|v| v as f64).unwrap_or(f64::NAN);
    }
    if let Some(h) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        return u64::from_str_radix(h, 8).map(|v| v as f64).unwrap_or(f64::NAN);
    }
    if let Some(h) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        return u64::from_str_radix(h, 2).map(|v| v as f64).unwrap_or(f64::NAN);
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let ok = t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    if !ok {
        return f64::NAN;
    }
    t.parse().unwrap_or(f64::NAN)
}

/// Unary `+v` / `Number(v)` on an arbitrary JSON value.
pub fn value_to_number(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => string_to_number(s),
        Value::Array(a) => match a.len() {
            0 => 0.0,
            1 => value_to_number(&Value::String(value_to_js_string(&a[0]))),
            _ => f64::NAN,
        },
        Value::Object(_) => f64::NAN,
    }
}

/// `String(v)` on an arbitrary JSON value (`null`/`undefined` aside, which callers handle).
pub fn value_to_js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => num_to_string(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(|x| if x.is_null() { String::new() } else { value_to_js_string(x) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// Rewrites integral floats (`1080.0`) as integers (`1080`) so serialised JSON matches `JSON.stringify`.
pub fn normalize_numbers(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if n.is_f64() {
                let f = n.as_f64().unwrap_or(0.0);
                if f.is_finite() && f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
                    *v = Value::from(f as i64);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(normalize_numbers),
        Value::Object(m) => m.values_mut().for_each(normalize_numbers),
        _ => {}
    }
}

/// Serialises to a JSON value with JS number formatting (integral floats become integers).
pub fn to_value<T: Serialize + ?Sized>(t: &T) -> serde_json::Result<Value> {
    let mut v = serde_json::to_value(t)?;
    normalize_numbers(&mut v);
    Ok(v)
}

/// Serialises to a JSON string equivalent to `JSON.stringify` of the TS object (same keys and number forms).
pub fn to_string<T: Serialize + ?Sized>(t: &T) -> serde_json::Result<String> {
    serde_json::to_string(&to_value(t)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_strings() {
        assert_eq!(num_to_string(1.0), "1");
        assert_eq!(num_to_string(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(num_to_string(1e21), "1e+21");
        assert_eq!(num_to_string(1.5e-7), "1.5e-7");
        assert_eq!(num_to_string(0.000001), "0.000001");
        assert_eq!(num_to_string(-123.456), "-123.456");
        assert_eq!(num_to_string(123456789012345680000.0), "123456789012345680000");
    }

    #[test]
    fn fixed() {
        assert_eq!(to_fixed(0.0625, 3), "0.063");
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(0.5, 0), "1");
        assert_eq!(to_fixed(-0.0001, 2), "-0.00");
        assert_eq!(to_fixed(99.995, 2), "100.00");
        assert_eq!(to_fixed(9.9999, 2), "10.00");
    }

    #[test]
    fn rounding() {
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(0.49999999999999994), 0.0);
    }
}
