// SPDX-License-Identifier: Apache-2.0
//! SVG helpers shared by the API (sanitising uploads), the renderers (recoloured variants) and the editor (colour
//! swatches). Pure string functions: no DOM.
//!
//! The JS regexes are translated to `fancy-regex` (backreferences/look-ahead) with JS semantics kept explicit:
//! `\s` → the JS whitespace class, `\d`/`\w` → ASCII classes.

use std::sync::LazyLock;

use fancy_regex::{Captures, Regex};
use indexmap::IndexMap;
use quick_xml::escape::{escape, unescape};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use serde::{Deserialize, Serialize};

use crate::js::{self, JS_S};

/// Translates a JS regex source: `[\s\S]` → any char, `\s` → the JS whitespace class (inside or outside classes).
pub(crate) fn js_regex_source(p: &str) -> String {
    let p = p.replace(r"[\s\S]", "(?s:.)");
    let mut out = String::with_capacity(p.len() + 64);
    let mut in_class = false;
    let mut chars = p.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                let next = chars.next().unwrap_or('\\');
                match (next, in_class) {
                    ('s', true) => out.push_str(JS_S),
                    ('s', false) => out.push_str(&format!("[{JS_S}]")),
                    ('S', false) => out.push_str(&format!("[^{JS_S}]")),
                    _ => {
                        out.push('\\');
                        out.push(next);
                    }
                }
            }
            '[' if !in_class => {
                in_class = true;
                out.push(ch);
                // A leading `^` or `]` belongs to the class.
            }
            ']' if in_class => {
                in_class = false;
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

pub(crate) fn re(p: &str) -> Regex {
    let src = js_regex_source(p);
    Regex::new(&src).unwrap_or_else(|e| panic!("bad regex {src}: {e}"))
}

/// `String#replace(regex /g, fn)`.
fn replace_all_with(r: &Regex, s: &str, mut f: impl FnMut(&Captures) -> String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for caps in r.captures_iter(s) {
        let Ok(caps) = caps else { break };
        let m = caps.get(0).expect("group 0");
        out.push_str(&s[last..m.start()]);
        out.push_str(&f(&caps));
        last = m.end();
    }
    out.push_str(&s[last..]);
    out
}

/// `String#replace(regex /g, '')`.
fn strip(r: &Regex, s: &str) -> String {
    replace_all_with(r, s, |_| String::new())
}

fn is_match(r: &Regex, s: &str) -> bool {
    r.is_match(s).unwrap_or(false)
}

fn group<'a>(c: &'a Captures, i: usize) -> &'a str {
    c.get(i).map_or("", |m| m.as_str())
}

// ------------------------------------------------------------------ sanitising

const DANGEROUS_ELEMENTS: &[&str] =
    &["script", "foreignObject", "iframe", "embed", "object", "audio", "video", "canvas", "handler", "listener", "set"];

struct SanitizeRes {
    doctype: Regex,
    entity: Regex,
    pi: Regex,
    comment: Regex,
    cdata: Regex,
    tags: Vec<(Regex, Regex, Regex)>,
    animate: Regex,
    handler: Regex,
    href: Regex,
    raster: Regex,
    url: Regex,
    import: Regex,
    javascript: Regex,
    vbscript: Regex,
    image_set: Regex,
}

static SANITIZE: LazyLock<SanitizeRes> = LazyLock::new(|| SanitizeRes {
    doctype: re(r"(?i)<!DOCTYPE[^\[>]*(\[[\s\S]*?\])?\s*>"),
    entity: re(r"(?i)<!ENTITY[\s\S]*?>"),
    pi: re(r"(?i)<\?(?!xml\s)[\s\S]*?\?>"),
    comment: re(r"<!--[\s\S]*?-->"),
    cdata: re(r"<!\[CDATA\[([\s\S]*?)\]\]>"),
    tags: DANGEROUS_ELEMENTS
        .iter()
        .map(|t| {
            (
                re(&format!(r"(?i)<{t}\b[^>]*?/>")),
                re(&format!(r"(?i)<{t}\b[\s\S]*?</{t}\s*>")),
                re(&format!(r"(?i)<{t}\b[^>]*>")),
            )
        })
        .collect(),
    animate: re(
        r#"(?i)<animate[A-Za-z0-9_]*\b[^>]*attributeName\s*=\s*["'][^"']*href[^"']*["'][^>]*?(/>|>[\s\S]*?</animate[A-Za-z0-9_]*\s*>)"#,
    ),
    handler: re(r#"(?i)\son[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)"#),
    href: re(r#"(?i)(\s(?:xlink:)?href\s*=\s*)(["'])([\s\S]*?)\2"#),
    raster: re(r"(?i)^\s*data:image/(png|jpe?g|gif|webp);"),
    url: re(r#"(?i)url\(\s*(?!['"]?\s*#)[^)]*\)"#),
    import: re(r"(?i)@import[^;]*;?"),
    javascript: re(r"(?i)javascript\s*:"),
    vbscript: re(r"(?i)vbscript\s*:"),
    image_set: re(r"(?i)image-set\("),
});

/// Strip active content and external references from an SVG.
pub fn sanitize_svg(svg: &str) -> String {
    let r = &*SANITIZE;
    let mut s = svg.strip_prefix('\u{FEFF}').unwrap_or(svg).to_string();
    s = strip(&r.doctype, &s);
    s = strip(&r.entity, &s);
    s = strip(&r.pi, &s);
    s = strip(&r.comment, &s);
    s = replace_all_with(&r.cdata, &s, |c| group(c, 1).to_string());
    for (a, b, c) in &r.tags {
        s = strip(a, &s);
        s = strip(b, &s);
        s = strip(c, &s);
    }
    s = strip(&r.animate, &s);
    s = strip(&r.handler, &s);
    s = replace_all_with(&r.href, &s, |c| {
        let (pre, q, v) = (group(c, 1), group(c, 2), group(c, 3));
        let keep = js::trim(v).starts_with('#') || is_match(&r.raster, v);
        format!("{pre}{q}{}{q}", if keep { v } else { "#" })
    });
    s = replace_all_with(&r.url, &s, |_| "none".to_string());
    s = strip(&r.import, &s);
    s = strip(&r.javascript, &s);
    s = strip(&r.vbscript, &s);
    clean_markup(&s)
}

/// CSS `\75` and `\u` escapes resolved, so an escaped `url(` can't hide from the patterns.
fn css_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(char::is_ascii_hexdigit) {
            hex.extend(chars.next());
        }
        if hex.is_empty() {
            out.extend(chars.next());
        } else {
            out.push(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).unwrap_or('\u{FFFD}'));
            if chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                chars.next();
            }
        }
    }
    out
}

/// The decoded value without external references and script URLs; `None` when it has none.
fn clean_css(decoded: &str) -> Option<String> {
    let r = &*SANITIZE;
    let v = if decoded.contains('\\') { css_unescape(decoded) } else { decoded.to_string() };
    if ![&r.url, &r.import, &r.image_set, &r.javascript, &r.vbscript].iter().any(|x| is_match(x, &v)) {
        return None;
    }
    let v = replace_all_with(&r.url, &v, |_| "none".to_string());
    let v = strip(&r.image_set, &strip(&r.import, &v));
    Some(strip(&r.vbscript, &strip(&r.javascript, &v)))
}

/// One start tag: `None` drops the element, otherwise the tag to write (byte-identical when nothing in it is unsafe).
fn clean_tag(e: &BytesStart, raw: &str, empty: bool) -> Option<String> {
    let r = &*SANITIZE;
    let name = std::str::from_utf8(e.name().0).ok()?;
    // `<svg/onload=x>`: an HTML parser ends the name at the `/`, XML has no such name.
    if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')) {
        return None;
    }
    // Local names: `<x:script xmlns:x="http://www.w3.org/2000/svg">` is a script too.
    let local = |n: &str| n.rsplit(':').next().unwrap_or(n).to_ascii_lowercase();
    let element = local(name);
    if DANGEROUS_ELEMENTS.iter().any(|t| t.eq_ignore_ascii_case(&element)) {
        return None;
    }
    let mut changed = false;
    let mut out = format!("<{name}");
    for attr in e.html_attributes() {
        let attr = attr.ok()?;
        let key = std::str::from_utf8(attr.key.0).ok()?;
        let value = std::str::from_utf8(&attr.value).ok()?;
        let key_local = local(key);
        // `x="1"/onclick=y`: an HTML parser takes the `/` as a separator, XML reads it as part of the name.
        if key_local.starts_with("on") || !key.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')) {
            changed = true;
            continue;
        }
        let Ok(decoded) = unescape(value) else {
            changed = true;
            continue;
        };
        if element.starts_with("animate") && key_local == "attributename" {
            let target = local(js::trim(&decoded));
            if target.contains("href") || target.starts_with("on") {
                return None;
            }
        }
        let replaced = if key_local == "href" || key_local == "src" {
            (!js::trim(&decoded).starts_with('#') && !is_match(&r.raster, &decoded)).then(|| "#".to_string())
        } else {
            clean_css(&decoded)
        };
        changed |= replaced.is_some();
        let kept = replaced.as_deref().map_or_else(|| value.replace('"', "&quot;"), |v| escape(v).into_owned());
        out.push_str(&format!(" {key}=\"{kept}\""));
    }
    if !changed {
        return Some(raw.to_string());
    }
    out.push_str(if empty { "/>" } else { ">" });
    Some(out)
}

/// The contents of a `<style>`: unchanged, or re-escaped without what `clean_css` removed.
fn clean_style(raw: &str) -> String {
    match unescape(raw) {
        Ok(decoded) => clean_css(&decoded).map_or_else(|| raw.to_string(), |v| escape(&v).into_owned()),
        Err(_) => String::new(),
    }
}

/// What the patterns above can't see: entity and CSS escapes, namespace prefixes, tags only an HTML parser would
/// read. Parses with quick-xml and copies everything safe through unchanged; the rest of a document that stops
/// parsing is dropped.
fn clean_markup(svg: &str) -> String {
    let mut reader = Reader::from_str(svg);
    reader.config_mut().check_end_names = false;
    reader.config_mut().allow_unmatched_ends = true;
    let mut out = String::with_capacity(svg.len());
    let mut skip = 0usize;
    let mut style: Option<String> = None;
    loop {
        let start = reader.buffer_position() as usize;
        let Ok(event) = reader.read_event() else { break };
        let raw = &svg[start..reader.buffer_position() as usize];
        if matches!(event, Event::Eof) {
            break;
        }
        if skip > 0 {
            match event {
                Event::Start(_) => skip += 1,
                Event::End(_) => skip -= 1,
                _ => {}
            }
            continue;
        }
        if let Some(css) = &mut style {
            match event {
                // Unlike HTML, XML parses elements inside a <style>; they are not CSS.
                Event::Start(_) => skip = 1,
                Event::Empty(_) => {}
                Event::End(_) => {
                    out.push_str(&clean_style(css));
                    out.push_str(raw);
                    style = None;
                }
                _ => css.push_str(raw),
            }
            continue;
        }
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = matches!(event, Event::Empty(_));
                match clean_tag(e, raw, empty) {
                    Some(tag) => {
                        out.push_str(&tag);
                        if !empty && e.local_name().as_ref().eq_ignore_ascii_case(b"style") {
                            style = Some(String::new());
                        }
                    }
                    None => skip = usize::from(!empty),
                }
            }
            Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
            _ => out.push_str(raw),
        }
    }
    if let Some(css) = style {
        out.push_str(&clean_style(&css));
    }
    out
}

// ------------------------------------------------------------------ colours

const NAMED: &[(&str, &str)] = &[
    ("black", "#000000"), ("white", "#ffffff"), ("red", "#ff0000"), ("green", "#008000"), ("blue", "#0000ff"),
    ("yellow", "#ffff00"), ("orange", "#ffa500"), ("purple", "#800080"), ("gray", "#808080"), ("grey", "#808080"),
    ("silver", "#c0c0c0"), ("maroon", "#800000"), ("olive", "#808000"), ("lime", "#00ff00"), ("aqua", "#00ffff"),
    ("cyan", "#00ffff"), ("teal", "#008080"), ("navy", "#000080"), ("fuchsia", "#ff00ff"), ("magenta", "#ff00ff"),
    ("pink", "#ffc0cb"), ("brown", "#a52a2a"), ("gold", "#ffd700"), ("indigo", "#4b0082"), ("violet", "#ee82ee"),
    ("coral", "#ff7f50"), ("salmon", "#fa8072"), ("tomato", "#ff6347"), ("crimson", "#dc143c"), ("khaki", "#f0e68c"),
    ("beige", "#f5f5dc"), ("ivory", "#fffff0"), ("lavender", "#e6e6fa"), ("tan", "#d2b48c"),
    ("chocolate", "#d2691e"), ("darkgray", "#a9a9a9"), ("darkgrey", "#a9a9a9"), ("lightgray", "#d3d3d3"),
    ("lightgrey", "#d3d3d3"), ("whitesmoke", "#f5f5f5"), ("gainsboro", "#dcdcdc"), ("dimgray", "#696969"),
    ("dimgrey", "#696969"),
];

/// The colour key for `currentColor`.
pub const CURRENT_COLOR: &str = "currentColor";
pub const MAX_SVG_COLORS: usize = 64;

struct ColorRes {
    short: Regex,
    long: Regex,
    rgba: Regex,
    attr: Regex,
    decl: Regex,
    style_el: Regex,
    style_attr: Regex,
    ends_ws: Regex,
    any_fill: Regex,
    shape_el: Regex,
    current: Regex,
    svg_open: Regex,
    key: Regex,
    pair: Regex,
    color_key: Regex,
    color_value: Regex,
}

const PAINT_PROPS: &str = "fill|stroke|stop-color|color|flood-color|lighting-color";

static COLORS: LazyLock<ColorRes> = LazyLock::new(|| ColorRes {
    short: re(r"^#([0-9a-f]{3,4})$"),
    long: re(r"^#([0-9a-f]{6}|[0-9a-f]{8})$"),
    rgba: re(r"^rgba?\(\s*([0-9.]+%?)\s*[, ]\s*([0-9.]+%?)\s*[, ]\s*([0-9.]+%?)\s*(?:[,/]\s*([0-9.]+%?)\s*)?\)$"),
    attr: re(&format!(r#"(\s)({PAINT_PROPS})(\s*=\s*)(["'])([^"']*)\4"#)),
    decl: re(&format!(r#"(^|[;{{\s"'])({PAINT_PROPS})(\s*:\s*)([^;}}"'!]+)"#)),
    style_el: re(r"(?i)<style\b([^>]*)>([\s\S]*?)</style\s*>"),
    style_attr: re(r#"(?i)(\sstyle\s*=\s*)(["'])([\s\S]*?)\2"#),
    ends_ws: re(r"\s$"),
    any_fill: re(r#"(?i)[\s;{"']fill\s*[=:]"#),
    shape_el: re(r"(?i)<(path|rect|circle|ellipse|polygon|polyline|text)\b"),
    current: re(r"(?i)currentColor"),
    svg_open: re(r"(?i)<svg\b"),
    key: re(r"^([^/]+)/recolor/([0-9a-fc.\-]{1,2000})$"),
    pair: re(r"^(cc|[0-9a-f]{6}|[0-9a-f]{8})-([0-9a-f]{6}|[0-9a-f]{8})$"),
    color_key: re(r"^(currentColor|#[0-9a-f]{6}([0-9a-f]{2})?)$"),
    color_value: re(r"^#[0-9a-f]{6}([0-9a-f]{2})?$"),
});

/// `Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, '0')` (NaN → "NaN").
fn h2(n: f64) -> String {
    let v = js::round(n);
    if v.is_nan() {
        return "NaN".into();
    }
    format!("{:02x}", v.clamp(0.0, 255.0) as i64)
}

/// Canonical form of an SVG paint value: `#rrggbb`, `#rrggbbaa` (alpha < 1 only) or `currentColor`; `None` for
/// anything that is not a plain colour.
pub fn normalize_svg_color(raw: &str) -> Option<String> {
    let c = &*COLORS;
    let v = js::trim(raw).to_lowercase();
    if v.is_empty() {
        return None;
    }
    if v == "currentcolor" {
        return Some(CURRENT_COLOR.into());
    }
    if let Ok(Some(m)) = c.short.captures(&v) {
        let x: String = group(&m, 1).chars().flat_map(|ch| [ch, ch]).collect();
        return Some(if x.len() == 8 && x.ends_with("ff") { format!("#{}", &x[..6]) } else { format!("#{x}") });
    }
    if let Ok(Some(m)) = c.long.captures(&v) {
        let g = group(&m, 1);
        return Some(if g.len() == 8 && g.ends_with("ff") { format!("#{}", &g[..6]) } else { format!("#{g}") });
    }
    if let Ok(Some(m)) = c.rgba.captures(&v) {
        let ch = |s: &str| {
            if let Some(p) = s.strip_suffix('%') { (js::parse_float(p) / 100.0) * 255.0 } else { js::parse_float(s) }
        };
        let a = match m.get(4) {
            None => 1.0,
            Some(x) => {
                let s = x.as_str();
                if let Some(p) = s.strip_suffix('%') { js::parse_float(p) / 100.0 } else { js::parse_float(s) }
            }
        };
        let base = format!("#{}{}{}", h2(ch(group(&m, 1))), h2(ch(group(&m, 2))), h2(ch(group(&m, 3))));
        return Some(if a >= 1.0 { base } else { format!("{base}{}", h2(a * 255.0)) });
    }
    NAMED.iter().find(|(k, _)| *k == v).map(|(_, h)| (*h).to_string())
}

/// Visit every colour-valued paint property (attributes, style attributes, `<style>` rules); `f` may return a
/// replacement value.
fn map_paints(svg: &str, f: &mut dyn FnMut(&str, &str) -> Option<String>) -> String {
    let c = &*COLORS;
    let decls = |css: &str, f: &mut dyn FnMut(&str, &str) -> Option<String>| {
        replace_all_with(&c.decl, css, |m| {
            let (pre, prop, sep, val) = (group(m, 1), group(m, 2), group(m, 3), group(m, 4));
            match normalize_svg_color(val).and_then(|cv| f(&cv, &prop.to_lowercase())) {
                Some(r) if !r.is_empty() => {
                    format!("{pre}{prop}{sep}{r}{}", if is_match(&c.ends_ws, val) { " " } else { "" })
                }
                _ => group(m, 0).to_string(),
            }
        })
    };
    let s = replace_all_with(&c.style_el, svg, |m| format!("<style{}>{}</style>", group(m, 1), decls(group(m, 2), f)));
    let s = replace_all_with(&c.style_attr, &s, |m| {
        let (pre, q) = (group(m, 1), group(m, 2));
        format!("{pre}{q}{}{q}", decls(group(m, 3), f))
    });
    replace_all_with(&c.attr, &s, |m| {
        let (ws, prop, eq, q, val) = (group(m, 1), group(m, 2), group(m, 3), group(m, 4), group(m, 5));
        match normalize_svg_color(val).and_then(|cv| f(&cv, &prop.to_lowercase())) {
            Some(r) if !r.is_empty() => format!("{ws}{prop}{eq}{q}{r}{q}"),
            _ => group(m, 0).to_string(),
        }
    })
}

/// True when the SVG never sets a fill, i.e. its shapes paint with the implicit default black.
fn implicit_black_fill(svg: &str) -> bool {
    let c = &*COLORS;
    !is_match(&c.any_fill, svg) && is_match(&c.shape_el, svg)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SvgColorUse {
    pub color: String,
    pub uses: usize,
}

/// Distinct colours an SVG paints with (fills, strokes, gradient stops, `currentColor`), most used first.
pub fn extract_svg_colors(svg: &str) -> Vec<SvgColorUse> {
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    map_paints(svg, &mut |c, _| {
        *counts.entry(c.to_string()).or_insert(0) += 1;
        None
    });
    if implicit_black_fill(svg) {
        *counts.entry("#000000".to_string()).or_insert(0) += 1;
    }
    let mut out: Vec<SvgColorUse> = counts.into_iter().map(|(color, uses)| SvgColorUse { color, uses }).collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.uses));
    out
}

/// Keep only well-formed entries (canonical keys, hex replacements) that actually change something.
pub fn clean_svg_color_map<'a, I>(map: I) -> IndexMap<String, String>
where
    I: IntoIterator<Item = (&'a String, &'a String)>,
{
    let mut out = IndexMap::new();
    for (k, v) in map {
        let (Some(from), Some(to)) = (normalize_svg_color(k), normalize_svg_color(v)) else { continue };
        if to == CURRENT_COLOR || from == to {
            continue;
        }
        out.insert(from, to);
    }
    out
}

/// Rewrite every occurrence of each mapped colour (produces a derived SVG).
pub fn recolor_svg(svg: &str, map: &IndexMap<String, String>) -> String {
    let c = &*COLORS;
    let m = clean_svg_color_map(map);
    if m.is_empty() {
        return svg.to_string();
    }
    let mut s = map_paints(svg, &mut |col, _| m.get(col).cloned());
    if let Some(cc) = m.get(CURRENT_COLOR) {
        s = replace_all_with(&c.current, &s, |_| cc.clone());
    }
    if let Some(black) = m.get("#000000")
        && implicit_black_fill(svg)
        && let Ok(Some(found)) = c.svg_open.find(&s)
    {
        s = format!("{}<svg fill=\"{black}\"{}", &s[..found.start()], &s[found.end()..]);
    }
    s
}

// ------------------------------------------------------------------ derived asset keys

fn enc(c: &str) -> &str {
    if c == CURRENT_COLOR { "cc" } else { &c[1..] }
}
fn dec(c: &str) -> String {
    if c == "cc" { CURRENT_COLOR.to_string() } else { format!("#{c}") }
}

/// Asset key of the recoloured variant of an SVG asset; the asset itself when nothing is remapped.
pub fn recolor_key(asset: &str, map: Option<&IndexMap<String, String>>) -> String {
    let empty = IndexMap::new();
    let m = clean_svg_color_map(map.unwrap_or(&empty));
    let mut keys: Vec<&String> = m.keys().collect();
    keys.sort_by(|a, b| js::str_cmp(a, b));
    let pairs: Vec<String> =
        keys.into_iter().take(MAX_SVG_COLORS).map(|k| format!("{}-{}", enc(k), enc(&m[k]))).collect();
    if pairs.is_empty() { asset.to_string() } else { format!("{asset}/recolor/{}", pairs.join(".")) }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecolorKey {
    pub asset: String,
    pub map: IndexMap<String, String>,
}

/// Inverse of `recolor_key`; `None` for anything else (strict: keys come from URLs).
pub fn parse_recolor_key(key: &str) -> Option<RecolorKey> {
    let c = &*COLORS;
    let m = c.key.captures(key).ok()??;
    let mut map = IndexMap::new();
    let pairs: Vec<&str> = group(&m, 2).split('.').collect();
    if pairs.len() > MAX_SVG_COLORS {
        return None;
    }
    for p in pairs {
        let x = c.pair.captures(p).ok()??;
        map.insert(dec(group(&x, 1)), dec(group(&x, 2)));
    }
    Some(RecolorKey { asset: group(&m, 1).to_string(), map })
}

/// `SVG_COLOR_KEY.test(s)` (validation pattern for `ImageEl.svgColors` keys).
pub fn is_svg_color_key(s: &str) -> bool {
    is_match(&COLORS.color_key, s)
}

/// `SVG_COLOR_VALUE.test(s)`.
pub fn is_svg_color_value(s: &str) -> bool {
    is_match(&COLORS.color_value, s)
}

/// Source of the key pattern (zod error messages quote it).
pub const SVG_COLOR_KEY_SOURCE: &str = "/^(currentColor|#[0-9a-f]{6}([0-9a-f]{2})?)$/";
pub const SVG_COLOR_VALUE_SOURCE: &str = "/^#[0-9a-f]{6}([0-9a-f]{2})?$/";

#[cfg(test)]
mod tests {
    use super::sanitize_svg;

    #[test]
    fn sanitize_sees_through_encoding_and_prefixes() {
        for bad in [
            r#"<svg/onload=alert(1)>"#,
            r#"<svg><rect x='1'onclick=alert(1) /></svg>"#,
            r#"<svg><a href=java&#115;cript:alert(1)><text>x</text></a></svg>"#,
            r#"<svg><a><animate attributeName='&#104;ref' values='java&#115;cript:alert(1)'/><text>x</text></a></svg>"#,
            r#"<svg><animate attributeName=href to=javascript:alert(1) /></svg>"#,
            r#"<x:svg xmlns:x="http://www.w3.org/2000/svg"><x:script>alert(1)</x:script></x:svg>"#,
            r#"<svg><a y:href="javascript:alert(1)" xmlns:y="http://www.w3.org/1999/xlink"/></svg>"#,
            r#"<svg><rect style='fill:u&#114;l(//evil.example/x)'/></svg>"#,
            r#"<svg><style>rect{fill:\75rl(//evil.example/x)}</style></svg>"#,
            r#"<svg><style>a{fill:&#117;rl(//evil.example/x)}</style></svg>"#,
            r#"<svg><style><x:script xmlns:x="http://www.w3.org/2000/svg">alert(1)</x:script></style></svg>"#,
            r#"<svg><image href=//evil.example/a.png /></svg>"#,
            r#"<svg><use href=data:image/svg+xml;base64,AAA /></svg>"#,
            r#"<svg><rect x="1"/onclick=alert(1) /></svg>"#,
            "<svg><a x=\"1\"/href=\"java&#9;script:alert(1)\"><text>x</text></a></svg>",
        ] {
            let out = sanitize_svg(bad);
            for needle in ["alert", "evil.example", "svg+xml"] {
                assert!(!out.contains(needle), "{bad} -> {out}");
            }
        }
    }

    #[test]
    fn sanitize_leaves_clean_svg_untouched() {
        let good = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox='0 0 1 1'><style>.a{fill:url(#g)}</style><defs><linearGradient id="g"/></defs><use xlink:href="#g"/><text class="a">a &amp; b</text></svg>"##;
        assert_eq!(sanitize_svg(good), good);
    }
}
