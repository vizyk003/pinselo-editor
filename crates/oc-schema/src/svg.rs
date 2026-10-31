// SPDX-License-Identifier: Apache-2.0
//! SVG helpers shared by the API (sanitising uploads), the renderers (recoloured variants) and the editor (colour
//! swatches). Pure string functions: no DOM.
//!
//! The JS regexes are translated to `fancy-regex` (backreferences/look-ahead) with JS semantics kept explicit:
//! `\s` → the JS whitespace class, `\d`/`\w` → ASCII classes.

use std::sync::LazyLock;

use fancy_regex::{Captures, Regex};
use indexmap::IndexMap;
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
    s
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
