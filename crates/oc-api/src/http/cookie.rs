//! Cookies as `@fastify/cookie` 11 (on `cookie` 2.0) reads and writes them.
//!
//! * [`parse`]: `parseCookie(header)`: `name=value` pairs split on `;`, spaces/tabs trimmed, the first
//!   occurrence of a name wins, values with `%` are `decodeURIComponent`d (kept as-is when that fails).
//! * [`serialize`]: `stringifySetCookie`: `name=value; Max-Age; Domain; Path; Expires; HttpOnly; Secure;
//!   SameSite` in that order; a value outside the cookie-octet set is `encodeURIComponent`d. `setCookie`
//!   defaults `sameSite: 'lax'`; `clearCookie` sets `Max-Age=0`, `Expires` at the epoch and `Path=/`.
//! * [`sign`] / [`unsign`]: `value + "." + base64(HMAC-SHA256(secret, value))` without `=` padding.

use std::collections::HashMap;

use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// `request.cookies`.
pub fn parse(header: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let s: Vec<u16> = header.encode_utf16().collect();
    let len = s.len();
    if len < 2 {
        return out;
    }
    let find = |c: u16, from: usize| s[from.min(len)..].iter().position(|&x| x == c).map_or(len, |p| p + from);
    let slice = |a: usize, b: usize| -> String {
        let (mut a, mut b) = (a, b);
        while a < b && (s[a] == 32 || s[a] == 9) {
            a += 1;
        }
        while b > a && (s[b - 1] == 32 || s[b - 1] == 9) {
            b -= 1;
        }
        String::from_utf16_lossy(&s[a..b])
    };
    let mut index = 0usize;
    while index < len {
        let eq = find(b'=' as u16, index);
        if eq == len {
            break;
        }
        let end = find(b';' as u16, index);
        if eq > end {
            // backtrack on the prior semicolon
            index = s[..eq].iter().rposition(|&x| x == b';' as u16).map_or(0, |p| p + 1);
            continue;
        }
        let key = slice(index, eq);
        out.entry(key).or_insert_with(|| decode(&slice(eq + 1, end)));
        index = end + 1;
    }
    out
}

fn decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    decode_uri_component(s).unwrap_or_else(|| s.to_owned())
}

/// `decodeURIComponent(s)`; `None` where JS throws a `URIError`.
pub fn decode_uri_component(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = b.get(i + 1..i + 3)?;
            let v = u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()?;
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn is_cookie_octet(s: &str) -> bool {
    s.bytes().all(|c| matches!(c, b'!' | b'#' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b'-' | b'.' | b'/' | b'0'..=b'9' | b':' | b'<' | b'=' | b'>' | b'?' | b'@' | b'A'..=b'Z' | b'[' | b']' | b'^' | b'_' | b'`' | b'a'..=b'z' | b'{' | b'|' | b'}' | b'~'))
}

#[derive(Debug, Clone, Default)]
pub struct CookieOpts {
    pub max_age: Option<i64>,
    pub domain: Option<String>,
    pub path: Option<String>,
    /// `expires` as ms since the epoch.
    pub expires_ms: Option<i64>,
    pub http_only: bool,
    pub secure: bool,
    /// `None` = the `setCookie` default (`lax`); `Some("")` = no SameSite attribute.
    pub same_site: Option<&'static str>,
}

/// `reply.setCookie(name, value, opts)` → the `Set-Cookie` header value.
pub fn serialize(name: &str, value: &str, o: &CookieOpts) -> String {
    let v = if is_cookie_octet(value) { value.to_owned() } else { oc_js::encode_uri_component(value) };
    let mut s = format!("{name}={v}");
    if let Some(m) = o.max_age {
        s.push_str(&format!("; Max-Age={m}"));
    }
    if let Some(d) = o.domain.as_deref().filter(|d| !d.is_empty()) {
        s.push_str(&format!("; Domain={d}"));
    }
    if let Some(p) = o.path.as_deref().filter(|p| !p.is_empty()) {
        s.push_str(&format!("; Path={p}"));
    }
    if let Some(ms) = o.expires_ms {
        s.push_str(&format!("; Expires={}", utc_string(ms)));
    }
    if o.http_only {
        s.push_str("; HttpOnly");
    }
    if o.secure {
        s.push_str("; Secure");
    }
    match o.same_site.unwrap_or("lax") {
        "" => {}
        "strict" => s.push_str("; SameSite=Strict"),
        "none" => s.push_str("; SameSite=None"),
        _ => s.push_str("; SameSite=Lax"),
    }
    s
}

/// `reply.clearCookie(name, { path })`.
pub fn clear(name: &str, path: &str) -> String {
    serialize(name, "", &CookieOpts { max_age: Some(0), path: Some(path.to_owned()), expires_ms: Some(0), ..Default::default() })
}

/// `Date.prototype.toUTCString()`.
pub fn utc_string(ms: i64) -> String {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms.max(0) as u64);
    httpdate::fmt_http_date(t)
}

fn mac(secret: &str, value: &str) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    m.update(value.as_bytes());
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(m.finalize().into_bytes())
}

/// `signCookie(value)`.
pub fn sign(secret: &str, value: &str) -> String {
    format!("{value}.{}", mac(secret, value))
}

/// `unsignCookie(signed)`: the original value when the signature is valid.
pub fn unsign(secret: &str, signed: &str) -> Option<String> {
    // JS: slice(0, lastIndexOf('.')) — with no '.', lastIndexOf is -1 and the value loses its last character.
    let units: Vec<u16> = signed.encode_utf16().collect();
    let dot = units.iter().rposition(|&c| c == b'.' as u16);
    let value_end = match dot {
        Some(d) => d,
        None => units.len().saturating_sub(1),
    };
    let value = String::from_utf16_lossy(&units[..value_end]);
    let actual = String::from_utf16_lossy(&units[dot.map_or(0, |d| d + 1)..]);
    let expected = mac(secret, &value);
    (expected.len() == actual.len() && bool::from(expected.as_bytes().ct_eq(actual.as_bytes()))).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_like_fastify() {
        assert_eq!(clear("oc_session", "/"), "oc_session=; Max-Age=0; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT; SameSite=Lax");
        let o = CookieOpts { max_age: Some(2_592_000), path: Some("/".into()), http_only: true, ..Default::default() };
        assert_eq!(serialize("oc_session", "abc_-", &o), "oc_session=abc_-; Max-Age=2592000; Path=/; HttpOnly; SameSite=Lax");
        assert_eq!(serialize("x", "{\"a\":1}", &CookieOpts::default()), "x=%7B%22a%22%3A1%7D; SameSite=Lax");
    }

    #[test]
    fn parse_like_cookie_2() {
        let c = parse("a=1; b = 2 ;a=3; c=%7Bx%7D; d=%zz");
        assert_eq!(c["a"], "1");
        assert_eq!(c["b"], "2");
        assert_eq!(c["c"], "{x}");
        assert_eq!(c["d"], "%zz");
    }

    #[test]
    fn sign_roundtrip() {
        let s = sign("secret", "hello");
        assert_eq!(unsign("secret", &s).as_deref(), Some("hello"));
        assert_eq!(unsign("other", &s), None);
        assert_eq!(unsign("secret", "nodot"), None);
    }
}
