//! JavaScript string/number semantics the routes depend on (`.length`, `.slice`, `.trim()`, `Math.round`, …).

/// `s.length` (UTF-16 code units).
pub fn len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s.slice(start, end)` in UTF-16 code units (a split surrogate pair is replaced, as Rust strings cannot hold
/// a lone surrogate).
pub fn slice(s: &str, start: usize, end: usize) -> String {
    let u: Vec<u16> = s.encode_utf16().collect();
    let e = end.min(u.len());
    let st = start.min(e);
    String::from_utf16_lossy(&u[st..e])
}

/// `s.slice(0, n)`.
pub fn take(s: &str, n: usize) -> String {
    slice(s, 0, n)
}

/// JS whitespace for `trim()`: WhiteSpace + LineTerminator.
pub fn is_ws(c: char) -> bool {
    oc_js::is_js_whitespace(c)
}

/// `s.trim()`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_ws)
}

/// `Math.round(x)` (halves round up).
pub fn round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// `String(n)` for a number.
pub fn num(n: f64) -> String {
    oc_js::number_to_string(n)
}

/// `encodeURIComponent(s)`.
pub fn encode_uri_component(s: &str) -> String {
    oc_js::encode_uri_component(s)
}

/// A JS-style truthiness test of an optional string (`if (x)`).
pub fn truthy(s: Option<&str>) -> bool {
    s.is_some_and(|v| !v.is_empty())
}
