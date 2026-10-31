// SPDX-License-Identifier: Apache-2.0
//! Docs → Markdown (DC-032), port of `services/render/src/docMarkdown.ts`: GitHub-flavoured Markdown from the
//! doc's ProseMirror tree. Headings, marks (bold/italic/strike/code/links; underline/highlight/colour have no
//! Markdown form and are dropped), nested lists, task lists, quotes, fenced code, tables, images, callouts (as
//! GitHub alert quotes), page breaks, a table of contents (heading links), and design/chart/video blocks as
//! links/text. Output is byte-identical to the TS (`tests/golden/render-svc/docs.json`).

use std::collections::HashMap;
use std::sync::LazyLock;

use oc_crdt::doc_body::{PmNode, node_text};
use oc_schema::doc_blocks::{CalloutTone, callout_tone, parse_doc_chart};
use oc_schema::js::{self, JS_S};
use regex::Regex;
use serde_json::{Map, Value};

use crate::pm::{
    attr, attr_num, attr_or, attr_str, attr_truthy, children, js_max, js_min, marks, repeat,
    trim_end,
};

#[derive(Debug, Clone, Default)]
pub struct MarkdownOptions {
    /// Absolute origin for /api/... asset and design links (so the file works outside the app).
    pub origin: Option<String>,
}

static LINE_START: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^([{JS_S}]*)([#>+-]|[0-9]+\.)([{JS_S}])")).expect("regex")
});
static SLUG_DROP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"[^\p{{L}}\p{{N}}{JS_S}-]")).expect("regex"));
static WS_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"[{JS_S}]+")).expect("regex"));
static LINK_HREF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(https?:|mailto:|/)").expect("regex"));
static HTTP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://").expect("regex"));

/// `esc`: Markdown-special characters backslash-escaped, plus a leading block marker (`#`, `>`, `+`, `-`, `1.`).
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|') {
            out.push('\\');
        }
        out.push(c);
    }
    LINE_START.replace(&out, "${1}\\${2}${3}").into_owned()
}

fn abs(u: &str, o: &MarkdownOptions) -> String {
    let full = match &o.origin {
        Some(origin) if u.starts_with('/') && !origin.is_empty() => format!("{origin}{u}"),
        _ => u.to_string(),
    };
    full.replace(')', "%29").replace(' ', "%20")
}

/// `slug`: GitHub-style heading anchor.
pub fn slug(s: &str) -> String {
    let lower = s.to_lowercase();
    let t = js::trim(&lower);
    let kept = SLUG_DROP.replace_all(t, "");
    WS_RUN.replace_all(&kept, "-").into_owned()
}

fn leading_ws(s: &str) -> usize {
    s.len() - s.trim_start_matches(js::is_js_space).len()
}

fn trailing_ws(s: &str) -> usize {
    s.len() - s.trim_end_matches(js::is_js_space).len()
}

fn inline_md(nodes: &[PmNode], opts: &MarkdownOptions) -> String {
    let mut out = String::new();
    for n in nodes {
        if n.kind == "hardBreak" {
            out.push_str("  \n");
            continue;
        }
        let text = match (&*n.kind, &n.text) {
            ("text", Some(t)) if !t.is_empty() => t,
            _ => continue,
        };
        // `new Map(marks.map(m => [m.type, m.attrs ?? {}]))`: the last mark of a type wins.
        let mut by: HashMap<&str, Option<&Map<String, Value>>> = HashMap::new();
        for m in marks(n) {
            by.insert(m.kind.as_str(), m.attrs.as_ref());
        }
        let t = if by.contains_key("code") {
            format!("`{}`", text.replace('`', "ˋ"))
        } else {
            esc(text)
        };
        // Keep surrounding whitespace outside the markers (`** bold**` is not bold).
        let lead = leading_ws(&t);
        let trail = trailing_ws(&t);
        let mut core = if lead <= t.len() - trail {
            t[lead..t.len() - trail].to_string()
        } else {
            String::new()
        };
        if !core.is_empty() {
            if by.contains_key("bold") {
                core = format!("**{core}**");
            }
            if by.contains_key("italic") {
                core = format!("*{core}*");
            }
            if by.contains_key("strike") {
                core = format!("~~{core}~~");
            }
            let href = by
                .get("link")
                .and_then(|a| a.and_then(|m| m.get("href")))
                .and_then(Value::as_str);
            if let Some(href) = href
                && LINK_HREF.is_match(href)
            {
                core = format!("[{core}]({})", abs(href, opts));
            }
        }
        out.push_str(&t[..lead]);
        out.push_str(&core);
        out.push_str(&t[t.len() - trail..]);
    }
    out
}

fn indent(s: &str, pad: &str) -> String {
    s.split('\n')
        .map(|l| {
            if l.is_empty() {
                l.to_string()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Clone, Copy, PartialEq)]
enum ListKind {
    Bullet,
    Number,
    Task,
}

struct Writer<'a> {
    root: &'a PmNode,
    opts: &'a MarkdownOptions,
}

impl Writer<'_> {
    fn list(&self, n: &PmNode, kind: ListKind) -> String {
        let start = attr_num(n, "start", 1.0);
        let start = if start == 0.0 || start.is_nan() {
            1.0
        } else {
            start
        };
        children(n)
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let marker = match kind {
                    ListKind::Number => format!("{}.", js::num_to_string(start + i as f64)),
                    ListKind::Task => format!(
                        "- [{}]",
                        if attr_truthy(item, "checked") {
                            'x'
                        } else {
                            ' '
                        }
                    ),
                    ListKind::Bullet => "-".to_string(),
                };
                let pad = " ".repeat(if kind == ListKind::Number {
                    js::utf16_len(&marker) + 1
                } else {
                    2
                });
                let parts: Vec<String> = children(item)
                    .iter()
                    .filter_map(|c| self.block(c))
                    .collect();
                let first = parts.first().cloned().unwrap_or_default();
                let rest = &parts[parts.len().min(1)..];
                let tail = if rest.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n{}",
                        rest.iter()
                            .map(|r| indent(r, &pad))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                };
                format!("{marker} {first}{tail}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn quoted(lines: &str) -> String {
        lines
            .split('\n')
            .map(|l| trim_end(&format!("> {l}")).to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn blocks_of(&self, n: &PmNode) -> Vec<String> {
        children(n).iter().filter_map(|c| self.block(c)).collect()
    }

    fn block(&self, n: &PmNode) -> Option<String> {
        let opts = self.opts;
        match n.kind.as_str() {
            "heading" => {
                let level = js_max(1.0, js_min(6.0, attr_num(n, "level", 1.0)));
                Some(format!(
                    "{} {}",
                    repeat("#", level),
                    inline_md(children(n), opts)
                ))
            }
            "paragraph" => Some(inline_md(children(n), opts)),
            "bulletList" => Some(self.list(n, ListKind::Bullet)),
            "orderedList" => Some(self.list(n, ListKind::Number)),
            "taskList" => Some(self.list(n, ListKind::Task)),
            "blockquote" => Some(Self::quoted(&self.blocks_of(n).join("\n\n"))),
            "callout" => {
                let kind = match callout_tone(attr(n, "tone")) {
                    CalloutTone::Warning => "WARNING",
                    CalloutTone::Danger => "CAUTION",
                    CalloutTone::Success => "TIP",
                    _ => "NOTE",
                };
                let inner = self.blocks_of(n).join("\n\n");
                Some(Self::quoted(&format!("[!{kind}]\n{inner}")))
            }
            "codeBlock" => {
                let code = node_text(n);
                let fence = if code.contains("```") { "~~~~" } else { "```" };
                Some(format!(
                    "{fence}{}\n{code}\n{fence}",
                    attr_str(n, "language").unwrap_or("")
                ))
            }
            "horizontalRule" => Some("---".into()),
            "pageBreak" => Some(r#"<div style="page-break-after: always"></div>"#.into()),
            "image" => {
                let src = attr_str(n, "src").unwrap_or("");
                if src.is_empty() {
                    return None;
                }
                let alt = attr_or(n, "alt").map_or_else(String::new, js::value_to_js_string);
                Some(format!("![{}]({})", esc(&alt), abs(src, opts)))
            }
            "designEmbed" => {
                let id = attr_str(n, "designId").unwrap_or("");
                if id.is_empty() {
                    return None;
                }
                let title = attr_str(n, "title")
                    .filter(|t| !t.is_empty())
                    .unwrap_or("Embedded design");
                Some(format!(
                    "[![{}]({})]({})",
                    esc(title),
                    abs(&format!("/api/designs/{id}/thumbnail.png"), opts),
                    abs(&format!("/design/{id}"), opts)
                ))
            }
            "chart" => {
                let d = parse_doc_chart(attr(n, "data"));
                let head = format!(
                    "| {} |",
                    std::iter::once(String::new())
                        .chain(d.series.iter().map(|s| esc(&s.name)))
                        .collect::<Vec<_>>()
                        .join(" | ")
                );
                let sep = format!(
                    "| {} |",
                    std::iter::once("---".to_string())
                        .chain(d.series.iter().map(|_| "---:".to_string()))
                        .collect::<Vec<_>>()
                        .join(" | ")
                );
                let mut lines = vec![
                    format!("*Chart ({})*", d.kind.as_str()),
                    String::new(),
                    head,
                    sep,
                ];
                for (i, l) in d.labels.iter().enumerate() {
                    let cells = std::iter::once(esc(l))
                        .chain(d.series.iter().map(|s| {
                            s.values
                                .get(i)
                                .map_or_else(String::new, |v| js::num_to_string(*v))
                        }))
                        .collect::<Vec<_>>();
                    lines.push(format!("| {} |", cells.join(" | ")));
                }
                Some(lines.join("\n"))
            }
            "videoEmbed" => {
                let url = attr_str(n, "url").filter(|u| HTTP.is_match(u))?;
                let title = attr_str(n, "title")
                    .filter(|t| !t.is_empty())
                    .unwrap_or(url);
                Some(format!("[▶ {}]({})", esc(title), abs(url, opts)))
            }
            "tableOfContents" => {
                let mut hs: Vec<(f64, String)> = Vec::new();
                fn walk(x: &PmNode, hs: &mut Vec<(f64, String)>) {
                    if x.kind == "heading" {
                        hs.push((attr_num(x, "level", 1.0), node_text(x)));
                    } else {
                        for c in children(x) {
                            walk(c, hs);
                        }
                    }
                }
                walk(self.root, &mut hs);
                let s = hs
                    .iter()
                    .filter(|(_, t)| !js::trim(t).is_empty())
                    .map(|(level, t)| {
                        format!(
                            "{}- [{}](#{})",
                            repeat("  ", js_max(0.0, level - 1.0)),
                            esc(t),
                            slug(t)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if s.is_empty() { None } else { Some(s) }
            }
            "table" => {
                let rows: Vec<Vec<String>> = children(n)
                    .iter()
                    .map(|r| {
                        children(r)
                            .iter()
                            .map(|c| {
                                children(c)
                                    .iter()
                                    .map(|p| inline_md(children(p), opts))
                                    .collect::<Vec<_>>()
                                    .join("<br>")
                                    .replace('\n', " ")
                            })
                            .collect()
                    })
                    .collect();
                if rows.is_empty() {
                    return None;
                }
                let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
                let pad = |r: &[String]| {
                    let mut v = r.to_vec();
                    v.resize(cols.max(r.len()), String::new());
                    v.join(" | ")
                };
                let mut lines = vec![
                    format!("| {} |", pad(&rows[0])),
                    format!("| {} |", vec!["---"; cols].join(" | ")),
                ];
                for r in &rows[1..] {
                    lines.push(format!("| {} |", pad(r)));
                }
                Some(lines.join("\n"))
            }
            _ => {
                if children(n).is_empty() {
                    None
                } else {
                    Some(self.blocks_of(n).join("\n\n"))
                }
            }
        }
    }
}

/// `docToMarkdown(root, opts)`.
pub fn doc_to_markdown(root: &PmNode, opts: &MarkdownOptions) -> String {
    let w = Writer { root, opts };
    let blocks: Vec<String> = children(root)
        .iter()
        .filter_map(|n| w.block(n))
        .filter(|b| !b.is_empty())
        .collect();
    format!("{}\n", blocks.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_slugs() {
        assert_eq!(esc("# a *b*"), "\\# a \\*b\\*");
        assert_eq!(esc("  12. x"), "  \\12. x");
        assert_eq!(esc("#tag"), "#tag");
        assert_eq!(
            slug(" Ünïcödé — Straße & 日本語! "),
            "ünïcödé-straße-日本語"
        );
    }
}
