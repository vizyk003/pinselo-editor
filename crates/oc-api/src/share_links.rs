//! Port of services/api/src/shareLinks.ts — public link helpers (g131 custom slugs, g135 embeds and oEmbed).

use serde_json::{Map, Value, json};

/// Words a custom link can't be: app routes, brand names and words that read as official.
pub const RESERVED_SLUGS: &[&str] = &[
    "about", "account", "admin", "administrator", "api", "app", "assets", "auth", "billing", "blog", "brand", "canvaskit", "collab", "contact", "dashboard", "design",
    "designs", "dev", "docs", "download", "edit", "editor", "embed", "help", "home", "invite", "legal", "login", "logout", "me", "new", "null", "oembed", "official",
    "opencanvas", "password", "pinselo", "pricing", "print", "privacy", "projects", "public", "register", "reset", "root", "search", "security", "settings", "share", "shared",
    "signin", "signup", "site", "sites", "staff", "static", "status", "support", "system", "team", "teams", "template", "templates", "terms", "test", "trash",
    "undefined", "upload", "uploads", "v1", "verify", "view", "www",
];

pub const SLUG_MIN: usize = 3;
pub const SLUG_MAX: usize = 60;

/// JS `\s` (WhiteSpace + LineTerminator).
fn js_space(c: char) -> bool {
    crate::js::is_ws(c)
}

/// `normalizeSlug(raw)`: lower-case, spaces and underscores to hyphens, hyphens collapsed and trimmed.
pub fn normalize_slug(raw: &str) -> String {
    let lower = crate::js::trim(raw).to_lowercase();
    let mut out = String::new();
    let mut in_run = false;
    for c in lower.chars() {
        if js_space(c) || c == '_' {
            if !in_run {
                out.push('-');
            }
            in_run = true;
        } else {
            out.push(c);
            in_run = false;
        }
    }
    let mut collapsed = String::new();
    for c in out.chars() {
        if c == '-' && collapsed.ends_with('-') {
            continue;
        }
        collapsed.push(c);
    }
    collapsed.trim_matches('-').to_owned()
}

/// `looksLikeDesignId(s)`: 16 base62 characters.
pub fn looks_like_design_id(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|c| c.is_ascii_alphanumeric())
}

/// `slugProblem(slug)`.
pub fn slug_problem(slug: &str) -> Option<&'static str> {
    let len = crate::js::len(slug);
    if !(SLUG_MIN..=SLUG_MAX).contains(&len) {
        return Some("length");
    }
    let ok = !slug.is_empty() && slug.split('-').all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    if !ok {
        return Some("chars");
    }
    if RESERVED_SLUGS.contains(&slug) || RESERVED_SLUGS.contains(&slug.replace('-', "").as_str()) {
        return Some("reserved");
    }
    if looks_like_design_id(slug) {
        return Some("id_like");
    }
    None
}

pub fn slug_message(problem: &str) -> String {
    match problem {
        "length" => format!("Use {SLUG_MIN} to {SLUG_MAX} characters."),
        "chars" => "Use only letters, numbers and hyphens.".into(),
        "reserved" => "That link is reserved. Try another one.".into(),
        _ => "Add a hyphen or pick a different link.".into(),
    }
}

/// WHATWG `URL.host` (host plus a non-default port).
fn url_host(u: &url::Url) -> String {
    let h = u.host_str().unwrap_or("");
    match u.port() {
        Some(p) => format!("{h}:{p}"),
        None => h.to_owned(),
    }
}

/// `viewKeyOf(url, webOrigin)`: the design id or slug of a public view URL on our own origin, or `None`.
pub fn view_key_of(url: &str, web_origin: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let own = url::Url::parse(web_origin).ok()?;
    if url_host(&u) != url_host(&own) || (u.scheme() != "https" && u.scheme() != "http") {
        return None;
    }
    let rest = u.path().strip_prefix("/view/")?;
    let key = rest.strip_suffix('/').unwrap_or(rest);
    let n = key.len();
    ((3..=64).contains(&n) && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')).then(|| key.to_owned())
}

/// `embedSize(w, h, maxWidth, maxHeight)`.
pub fn embed_size(w: f64, h: f64, max_width: Option<f64>, max_height: Option<f64>) -> (f64, f64) {
    let ratio = if w > 0.0 && h > 0.0 { h / w } else { 9.0 / 16.0 };
    let mut width = max_width.filter(|m| *m > 0.0).unwrap_or(800.0).clamp(100.0, 2000.0);
    let mut height = crate::js::round(width * ratio);
    if let Some(mh) = max_height.filter(|m| *m > 0.0)
        && height > mh
    {
        height = f64::max(50.0, crate::js::round(mh));
        width = crate::js::round(height / ratio);
    }
    (width, height)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

/// `iframeHtml(src, title, width, height)`.
pub fn iframe_html(src: &str, title: &str, width: f64, height: f64) -> String {
    format!(
        "<iframe src=\"{}\" width=\"{}\" height=\"{}\" title=\"{}\" style=\"border:0;max-width:100%\" loading=\"lazy\" allowfullscreen></iframe>",
        esc(src),
        crate::js::num(width),
        crate::js::num(height),
        esc(title)
    )
}

pub struct OembedInput<'a> {
    pub web_origin: &'a str,
    pub brand: &'a str,
    pub key: &'a str,
    pub title: &'a str,
    pub author: &'a str,
    pub w: f64,
    pub h: f64,
    pub max_width: Option<f64>,
    pub max_height: Option<f64>,
    pub thumb: Option<(String, u32, u32)>,
}

fn num(x: f64) -> Value {
    crate::http::jsjson::number_value(x).unwrap_or(Value::Null)
}

/// `oembedFor(o)`: oEmbed 1.0 "rich" response for a public design.
pub fn oembed_for(o: &OembedInput) -> Value {
    let (width, height) = embed_size(o.w, o.h, o.max_width, o.max_height);
    let src = format!("{}/view/{}?embed=1", o.web_origin, crate::js::encode_uri_component(o.key));
    let mut m = Map::new();
    m.insert("version".into(), json!("1.0"));
    m.insert("type".into(), json!("rich"));
    m.insert("provider_name".into(), json!(o.brand));
    m.insert("provider_url".into(), json!(o.web_origin));
    m.insert("title".into(), json!(o.title));
    m.insert("author_name".into(), json!(o.author));
    m.insert("html".into(), json!(iframe_html(&src, o.title, width, height)));
    m.insert("width".into(), num(width));
    m.insert("height".into(), num(height));
    if let Some((url, w, h)) = &o.thumb {
        m.insert("thumbnail_url".into(), json!(url));
        m.insert("thumbnail_width".into(), json!(w));
        m.insert("thumbnail_height".into(), json!(h));
    }
    m.insert("cache_age".into(), json!(300));
    Value::Object(m)
}

/// `pngSize(buf)`: width and height from a PNG's IHDR chunk.
pub fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 24 || b[0] != 0x89 || b[1] != 0x50 || b[2] != 0x4e || b[3] != 0x47 {
        return None;
    }
    let u = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    Some((u(16), u(20)))
}

/// `containsPattern(q)`: LIKE pattern for "contains", with `\`, `%` and `_` taken literally (ESCAPE '\').
pub fn contains_pattern(q: &str) -> String {
    let mut out = String::from("%");
    for c in q.chars() {
        if c == '\\' || c == '%' || c == '_' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(normalize_slug("  My__Cool  Link-- "), "my-cool-link");
        assert_eq!(slug_problem("ab"), Some("length"));
        assert_eq!(slug_problem("a.b.c"), Some("chars"));
        assert_eq!(slug_problem("sign-up"), Some("reserved"));
        assert_eq!(slug_problem("abcdefgh12345678"), Some("id_like"));
        assert_eq!(slug_problem("my-link"), None);
        assert_eq!(view_key_of("https://app.example/view/my-link/", "https://app.example"), Some("my-link".into()));
        assert_eq!(view_key_of("https://evil.example/view/my-link", "https://app.example"), None);
        assert_eq!(embed_size(1000.0, 500.0, None, None), (800.0, 400.0));
        assert_eq!(embed_size(1000.0, 500.0, Some(400.0), Some(100.0)), (200.0, 100.0));
        assert_eq!(contains_pattern("a%b_c\\"), "%a\\%b\\_c\\\\%");
    }
}
