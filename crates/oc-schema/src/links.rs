// SPDX-License-Identifier: Apache-2.0
//! Link targets that are safe to write into exports and published sites (SEC-01). Port of
//! packages/schema/src/links.ts: a `url` link may use http(s), mailto or tel, or be relative (no scheme); `page`
//! links point at a page of the design. Everything else (`javascript:`, `data:`, `vbscript:`, `file:`, …) is
//! rejected on write and dropped again at every output.

use serde_json::Value;

use crate::DesignDoc;
use crate::elements::Element;
use crate::primitives::LinkTarget;
use crate::text::Paragraph;

pub const LINK_SCHEMES: &[&str] = &["http", "https", "mailto", "tel"];
/// Longest accepted URL, in UTF-16 units (JS `length`).
pub const MAX_LINK_URL: usize = 4096;

/// True for a URL a link may point at. Browsers drop ASCII control characters and whitespace inside a scheme
/// (`java\tscript:`), so they are removed before the scheme is read.
pub fn is_safe_link_url(url: &str) -> bool {
    if url.encode_utf16().count() > MAX_LINK_URL {
        return false;
    }
    let s: String = url.chars().filter(|c| !(*c <= '\u{20}' || *c == '\u{7f}')).collect();
    let mut chars = s.char_indices();
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() => {}
        _ => return true,
    }
    for (i, c) in chars {
        if c == ':' {
            return LINK_SCHEMES.contains(&s[..i].to_ascii_lowercase().as_str());
        }
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-') {
            return true;
        }
    }
    true
}

/// A link value (raw JSON) is acceptable: absent, not an object (left to the schema), a page link, or a safe url
/// link.
pub fn is_safe_link(v: Option<&Value>) -> bool {
    let Some(Value::Object(m)) = v else { return true };
    if m.get("kind").and_then(Value::as_str) != Some("url") {
        return true;
    }
    matches!(m.get("url"), Some(Value::String(u)) if is_safe_link_url(u))
}

impl LinkTarget {
    /// False for a `url` link whose scheme is not allowed.
    pub fn is_safe(&self) -> bool {
        match self {
            LinkTarget::Url(u) => is_safe_link_url(&u.url),
            LinkTarget::Page(_) => true,
        }
    }
}

/// The link if it is safe to output.
pub fn safe_link(link: Option<&LinkTarget>) -> Option<&LinkTarget> {
    link.filter(|l| l.is_safe())
}

fn strip_paragraphs(ps: &mut [Paragraph]) {
    for p in ps {
        for r in &mut p.runs {
            if r.link.as_ref().is_some_and(|l| !l.is_safe()) {
                r.link = None;
            }
            if !is_safe_link(r.style.extra.get("link")) {
                r.style.extra.remove("link");
            }
        }
    }
}

/// Remove unsafe links from an element (its own link and its text runs).
pub fn strip_unsafe_element_links(el: &mut Element) {
    if let Some(b) = el.base_mut()
        && b.link.as_ref().is_some_and(|l| !l.is_safe())
    {
        b.link = None;
    }
    if let Element::Text(t) = el {
        strip_paragraphs(&mut t.body.paragraphs);
    }
}

/// The design with every unsafe link removed. Exporters call this first.
pub fn strip_unsafe_links(doc: &mut DesignDoc) {
    for el in doc.elements.values_mut() {
        strip_unsafe_element_links(el);
    }
}

fn element_has_unsafe_links(el: &Element) -> bool {
    if el.base().and_then(|b| b.link.as_ref()).is_some_and(|l| !l.is_safe()) {
        return true;
    }
    match el {
        Element::Text(t) => t.body.paragraphs.iter().flat_map(|p| &p.runs).any(|r| {
            r.link.as_ref().is_some_and(|l| !l.is_safe()) || !is_safe_link(r.style.extra.get("link"))
        }),
        _ => false,
    }
}

/// `withSafeLinks`: the design without unsafe links, borrowed when it has none (exporters call this first).
pub fn with_safe_links(doc: &DesignDoc) -> std::borrow::Cow<'_, DesignDoc> {
    if !doc.elements.values().any(element_has_unsafe_links) {
        return std::borrow::Cow::Borrowed(doc);
    }
    let mut d = doc.clone();
    strip_unsafe_links(&mut d);
    std::borrow::Cow::Owned(d)
}

/// Remove unsafe links from a raw JSON element (its link, and its text runs' links and link marks). True when
/// something was removed.
fn strip_value_element(el: &mut Value) -> bool {
    let Some(m) = el.as_object_mut() else { return false };
    let mut changed = false;
    if m.contains_key("link") && !is_safe_link(m.get("link")) {
        m.remove("link");
        changed = true;
    }
    let Some(ps) = m.get_mut("body").and_then(|b| b.get_mut("paragraphs")).and_then(Value::as_array_mut) else { return changed };
    for p in ps {
        let Some(runs) = p.get_mut("runs").and_then(Value::as_array_mut) else { continue };
        for r in runs.iter_mut().filter_map(Value::as_object_mut) {
            if r.contains_key("link") && !is_safe_link(r.get("link")) {
                r.remove("link");
                changed = true;
            }
            if let Some(st) = r.get_mut("style").and_then(Value::as_object_mut)
                && st.contains_key("link")
                && !is_safe_link(st.get("link"))
            {
                st.remove("link");
                changed = true;
            }
        }
    }
    changed
}

/// [`with_safe_links`] for a raw JSON design (server write paths that store a whole document).
pub fn with_safe_links_value(doc: &Value) -> std::borrow::Cow<'_, Value> {
    let mut d = doc.clone();
    let mut changed = false;
    if let Some(els) = d.get_mut("elements").and_then(Value::as_object_mut) {
        for el in els.values_mut() {
            changed |= strip_value_element(el);
        }
    }
    if changed { std::borrow::Cow::Owned(d) } else { std::borrow::Cow::Borrowed(doc) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_list() {
        for ok in ["https://x.test/a?b#c", "http://x", "HTTPS://X", "mailto:a@b.c", "tel:+1", "#top", "/p", "page-2", "//x.test", ""] {
            assert!(is_safe_link_url(ok), "{ok}");
        }
        for bad in ["javascript:alert(1)", " JaVaScRiPt:x", "java\tscript:x", "java\nscript:x", "\u{1}javascript:x", "data:text/html,x", "vbscript:x", "file:///etc/passwd", "blob:x"] {
            assert!(!is_safe_link_url(bad), "{bad:?}");
        }
        assert!(!is_safe_link_url(&format!("https://x/{}", "a".repeat(MAX_LINK_URL))));
    }

    #[test]
    fn strips_raw_json_designs() {
        let doc = serde_json::json!({ "elements": {
            "a": { "link": { "kind": "url", "url": "javascript:x" } },
            "b": { "link": { "kind": "url", "url": "https://ok" } },
            "t": { "body": { "paragraphs": [{ "runs": [{ "text": "x", "style": { "link": { "kind": "url", "url": "data:x" } }, "link": { "kind": "url", "url": "vbscript:x" } }] }] } },
        } });
        let out = with_safe_links_value(&doc);
        assert!(out["elements"]["a"].get("link").is_none());
        assert_eq!(out["elements"]["b"]["link"]["url"], "https://ok");
        let run = &out["elements"]["t"]["body"]["paragraphs"][0]["runs"][0];
        assert!(run.get("link").is_none() && run["style"].get("link").is_none());
        let clean = serde_json::json!({ "elements": { "b": { "link": { "kind": "url", "url": "https://ok" } } } });
        assert!(matches!(with_safe_links_value(&clean), std::borrow::Cow::Borrowed(_)));
    }
}
