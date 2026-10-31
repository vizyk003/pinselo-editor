// SPDX-License-Identifier: Apache-2.0
//! Docs → pages (DC-020), port of `services/render/src/docLayout.ts`: lays a flowing document (the ProseMirror tree
//! in the `docBody` fragment) onto fixed pages with the renderer's text engine, so a doc exports to PDF/PNG and
//! converts into an editable design. Paragraphs that don't fit split at a line boundary. Rich blocks (DC-030):
//! charts become chart elements, embedded designs a rendered picture (or a locked placeholder), callouts a
//! tinted box, page breaks start a new page, a table of contents lists the headings, video embeds become links.
//!
//! [`blocks_of`] is pure (golden parity: `tests/golden/render-svc/docs.json`); [`doc_to_design`] measures text
//! with [`Renderer::layout_text`].

use std::sync::LazyLock;

use futures::future::BoxFuture;
use oc_crdt::doc_body::{PmNode, node_text};
use oc_schema::color::hex;
use oc_schema::design::AssetRef;
use oc_schema::doc_blocks::{
    DOC_CONTENT_WIDTH, DocChartData, callout_style, callout_tone, doc_block_width,
    doc_chart_element, parse_doc_chart,
};
use oc_schema::elements::Element;
use oc_schema::factory::{
    LineOpts, MetaInit, ShapeOpts, TableOpts, TextOpts, base_init, empty_design, make_image,
    make_line, make_shape, make_table, make_text, place_all_elements, xf,
};
use oc_schema::primitives::{ColorValue, LinkTarget, Paint};
use oc_schema::text::{
    ListKind, ListSpec, Paragraph, Run, RunStyle, RunStyleOverride, TextAlign, TextBody,
    apply_override,
};
use oc_schema::tree::sorted_pages;
use oc_schema::{DesignDoc, Extra};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::pm::{
    attr, attr_nonempty, attr_num, attr_str, attr_truthy, children, js_max, js_min, mark_attr,
    marks,
};
use crate::renderer::{RenderError, RenderHost, Renderer};

fn c(h: &str) -> ColorValue {
    hex(h).expect("literal colour")
}

fn solid(h: &str) -> Paint {
    Paint::solid_color(c(h))
}

fn base_run() -> RunStyle {
    RunStyle {
        family: "Inter".into(),
        weight: 400.0,
        italic: false,
        size: 16.0,
        fill: solid("#1f2328"),
        tracking: 0.0,
        underline: None,
        strike: None,
        case_transform: None,
        highlight: None,
        baseline: None,
        extra: Extra::new(),
    }
}

/// Callout box colours.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoxStyle {
    pub bg: String,
    pub border: String,
}

/// A laid-out block of the doc.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Block {
    #[serde(rename_all = "camelCase")]
    Text {
        gap_before: f64,
        gap_after: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        quote: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<bool>,
        #[serde(rename = "box", skip_serializing_if = "Option::is_none")]
        bx: Option<BoxStyle>,
        body: TextBody,
    },
    Hr,
    Pagebreak,
    Image {
        src: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        alt: Option<Value>,
    },
    Table {
        rows: Vec<Vec<String>>,
        header: bool,
    },
    Chart {
        data: DocChartData,
        width: f64,
        height: f64,
    },
    #[serde(rename_all = "camelCase")]
    Embed {
        design_id: String,
        page_id: Option<String>,
        width: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    /// The TS throws (an invalid colour in a mark).
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Renderer(#[from] RenderError),
}

static RGB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"rgba?\((\d+),\s*(\d+),\s*(\d+)").expect("regex"));
static COLORISH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#|rgb").expect("regex"));
static ASSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/api/assets/([\w-]+)").expect("regex"));
static HTTP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://").expect("regex"));

fn rgb_to_hex(s: &str) -> String {
    match RGB.captures(s) {
        Some(m) => format!(
            "#{}",
            (1..=3)
                .map(|i| format!("{:02x}", m[i].parse::<f64>().unwrap_or(f64::NAN) as u64))
                .collect::<String>()
        ),
        None => s.to_string(),
    }
}

fn hex_or_err(s: &str) -> Result<ColorValue, LayoutError> {
    hex(s).map_err(|e| LayoutError::Invalid(e.to_string()))
}

fn runs_of(
    nodes: &[PmNode],
    base: &RunStyleOverride,
    prefix: &str,
) -> Result<Vec<Run>, LayoutError> {
    let run = |text: &str, style: RunStyleOverride| Run {
        text: text.into(),
        style,
        link: None,
        extra: Extra::new(),
    };
    let mut runs: Vec<Run> = if prefix.is_empty() {
        vec![]
    } else {
        vec![run(prefix, base.clone())]
    };
    for n in nodes {
        if n.kind == "hardBreak" {
            runs.push(run(" ", base.clone()));
            continue;
        }
        let text = match (&*n.kind, &n.text) {
            ("text", Some(t)) if !t.is_empty() => t,
            _ => continue,
        };
        let mut style = base.clone();
        let mut link = None;
        for m in marks(n) {
            match m.kind.as_str() {
                "bold" => style.weight = Some(700.0),
                "italic" => style.italic = Some(true),
                "underline" => style.underline = Some(true),
                "strike" => style.strike = Some(true),
                "code" => style.family = Some("JetBrains Mono".into()),
                "highlight" => {
                    let v = mark_attr(m, "color")
                        .filter(|v| !v.is_null())
                        .map_or_else(|| "#fde68a".to_string(), oc_schema::js::value_to_js_string);
                    style.highlight = Some(hex_or_err(&v)?);
                }
                "link" => {
                    if let Some(h) = mark_attr(m, "href").and_then(Value::as_str) {
                        link = Some(LinkTarget::url(h));
                        style.fill = Some(solid("#6d28d9"));
                        style.underline = Some(true);
                    }
                }
                "textStyle" => {
                    if let Some(col) = mark_attr(m, "color")
                        .and_then(Value::as_str)
                        .filter(|s| COLORISH.is_match(s))
                    {
                        style.fill = Some(Paint::solid_color(hex_or_err(&rgb_to_hex(col))?));
                    }
                    if let Some(f) = mark_attr(m, "fontFamily").and_then(Value::as_str) {
                        let cleaned: String =
                            f.chars().filter(|c| *c != '"' && *c != '\'').collect();
                        style.family = Some(
                            oc_schema::js::trim(cleaned.split(',').next().unwrap_or(""))
                                .to_string(),
                        );
                    }
                }
                _ => {}
            }
        }
        runs.push(Run {
            text: text.clone(),
            style,
            link,
            extra: Extra::new(),
        });
    }
    Ok(if runs.is_empty() {
        vec![run("", base.clone())]
    } else {
        runs
    })
}

fn align_of(a: Option<&Value>) -> TextAlign {
    match a.and_then(Value::as_str) {
        Some("center") => TextAlign::Center,
        Some("right") => TextAlign::End,
        Some("justify") => TextAlign::Justify,
        _ => TextAlign::Start,
    }
}

/// Headings in document order (table of contents), empty ones left out.
pub fn doc_headings(root: &PmNode) -> Vec<(f64, String)> {
    fn walk(n: &PmNode, out: &mut Vec<(f64, String)>) {
        if n.kind == "heading" {
            out.push((attr_num(n, "level", 1.0), node_text(n)));
        } else {
            for c in children(n) {
                walk(c, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.retain(|(_, t)| !oc_schema::js::trim(t).is_empty());
    out
}

fn para_of(runs: Vec<Run>, align: TextAlign, line_height: f64) -> Paragraph {
    Paragraph {
        runs,
        align,
        line_height,
        space_after: 0.0,
        list: None,
        extra: Extra::new(),
    }
}

fn ov(f: impl FnOnce(&mut RunStyleOverride)) -> RunStyleOverride {
    let mut o = RunStyleOverride::default();
    f(&mut o);
    o
}

struct B<'r> {
    root: &'r PmNode,
    out: Vec<Block>,
}

impl B<'_> {
    #[allow(clippy::too_many_arguments)]
    fn para(
        &mut self,
        n: &PmNode,
        base: &RunStyleOverride,
        line_height: f64,
        list: Option<ListSpec>,
        prefix: &str,
        gaps: (f64, f64),
        quote: bool,
        code: bool,
        bx: Option<BoxStyle>,
    ) -> Result<(), LayoutError> {
        let mut p = para_of(
            runs_of(children(n), base, prefix)?,
            align_of(attr(n, "textAlign")),
            line_height,
        );
        p.list = list;
        self.out.push(Block::Text {
            gap_before: gaps.0,
            gap_after: gaps.1,
            quote: quote.then_some(true),
            code: code.then_some(true),
            bx,
            body: TextBody {
                paragraphs: vec![p],
                default_run: apply_override(&base_run(), base),
                extra: Extra::new(),
            },
        });
        Ok(())
    }

    fn list(&mut self, n: &PmNode, kind: &str, level: f64) -> Result<(), LayoutError> {
        for item in children(n) {
            for c in children(item) {
                match c.kind.as_str() {
                    "paragraph" => {
                        let prefix = if kind == "task" {
                            if attr_truthy(item, "checked") {
                                "☑ "
                            } else {
                                "☐ "
                            }
                        } else {
                            ""
                        };
                        let list = (kind != "task").then(|| ListSpec {
                            kind: if kind == "bullet" {
                                ListKind::Bullet
                            } else {
                                ListKind::Number
                            },
                            level,
                            extra: Extra::new(),
                        });
                        self.para(
                            c,
                            &RunStyleOverride::default(),
                            1.5,
                            list,
                            prefix,
                            (0.0, 4.0),
                            false,
                            false,
                            None,
                        )?;
                    }
                    "bulletList" => self.list(c, "bullet", level + 1.0)?,
                    "orderedList" => self.list(c, "number", level + 1.0)?,
                    "taskList" => self.list(c, "task", level + 1.0)?,
                    _ => {}
                }
            }
        }
        self.last_gap(12.0);
        Ok(())
    }

    fn last_gap(&mut self, g: f64) {
        if let Some(Block::Text { gap_after, .. }) = self.out.last_mut() {
            *gap_after = g;
        }
    }

    fn simple(
        &mut self,
        gap_before: f64,
        gap_after: f64,
        runs: Vec<Run>,
        bx: Option<BoxStyle>,
        line_height: f64,
    ) {
        self.out.push(Block::Text {
            gap_before,
            gap_after,
            quote: None,
            code: None,
            bx,
            body: TextBody {
                paragraphs: vec![para_of(runs, TextAlign::Start, line_height)],
                default_run: base_run(),
                extra: Extra::new(),
            },
        });
    }

    fn block(&mut self, n: &PmNode, bx: Option<&BoxStyle>) -> Result<(), LayoutError> {
        let none = RunStyleOverride::default();
        match n.kind.as_str() {
            "heading" => {
                let lvl = attr_num(n, "level", 1.0);
                let (size, weight) = match lvl {
                    1.0 => (34.0, 700.0),
                    2.0 => (26.0, 700.0),
                    _ => (20.0, 700.0),
                };
                let base = ov(|o| {
                    o.size = Some(size);
                    o.weight = Some(weight);
                });
                self.para(
                    n,
                    &base,
                    1.2,
                    None,
                    "",
                    (if lvl == 1.0 { 20.0 } else { 14.0 }, 8.0),
                    false,
                    false,
                    None,
                )?;
            }
            "paragraph" => {
                let gaps = if bx.is_some() {
                    (0.0, 6.0)
                } else {
                    (0.0, 12.0)
                };
                self.para(n, &none, 1.5, None, "", gaps, false, false, bx.cloned())?;
            }
            "bulletList" => self.list(n, "bullet", 0.0)?,
            "orderedList" => self.list(n, "number", 0.0)?,
            "taskList" => self.list(n, "task", 0.0)?,
            "blockquote" => {
                let base = ov(|o| {
                    o.italic = Some(true);
                    o.fill = Some(solid("#4b5563"));
                });
                for c in children(n).iter().filter(|c| c.kind == "paragraph") {
                    self.para(c, &base, 1.5, None, "", (0.0, 8.0), true, false, None)?;
                }
            }
            "codeBlock" => {
                let base = ov(|o| {
                    o.family = Some("JetBrains Mono".into());
                    o.size = Some(14.0);
                });
                self.para(n, &base, 1.45, None, "", (4.0, 16.0), false, true, None)?;
            }
            "horizontalRule" => self.out.push(Block::Hr),
            "image" => {
                if let Some(src) = attr_str(n, "src") {
                    self.out.push(Block::Image {
                        src: src.into(),
                        alt: attr(n, "alt").cloned(),
                    });
                }
            }
            "table" => {
                let rows: Vec<Vec<String>> = children(n)
                    .iter()
                    .map(|tr| {
                        children(tr)
                            .iter()
                            .map(|cell| {
                                children(cell)
                                    .iter()
                                    .map(|p| {
                                        runs_of(children(p), &none, "").map(|rs| {
                                            rs.iter().map(|r| r.text.as_str()).collect::<String>()
                                        })
                                    })
                                    .collect::<Result<Vec<_>, _>>()
                                    .map(|v| v.join("\n"))
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let header = children(n)
                    .first()
                    .is_some_and(|r| children(r).iter().any(|c| c.kind == "tableHeader"));
                if !rows.is_empty() {
                    self.out.push(Block::Table { rows, header });
                }
            }
            "pageBreak" => self.out.push(Block::Pagebreak),
            "callout" => {
                let tone = callout_style(&callout_tone(attr(n, "tone")));
                let style = BoxStyle {
                    bg: tone.bg.into(),
                    border: tone.border.into(),
                };
                let start = self.out.len();
                for c in children(n) {
                    self.block(c, Some(&style))?;
                }
                if self.out.len() > start {
                    self.last_gap(14.0);
                }
            }
            "chart" => {
                let hv = attr(n, "height").map_or(f64::NAN, oc_schema::js::value_to_number);
                self.out.push(Block::Chart {
                    data: parse_doc_chart(attr(n, "data")),
                    width: doc_block_width(attr(n, "width"), None),
                    height: js_max(
                        120.0,
                        js_min(900.0, if hv == 0.0 || hv.is_nan() { 360.0 } else { hv }),
                    ),
                });
            }
            "designEmbed" => {
                if let Some(id) = attr_nonempty(n, "designId") {
                    self.out.push(Block::Embed {
                        design_id: id.into(),
                        page_id: attr_nonempty(n, "pageId").map(str::to_string),
                        width: doc_block_width(attr(n, "width"), None),
                        title: attr_str(n, "title").map(str::to_string),
                    });
                }
            }
            "videoEmbed" => {
                let Some(url) = attr_str(n, "url").filter(|u| HTTP.is_match(u)) else {
                    return Ok(());
                };
                let title = attr_nonempty(n, "title").unwrap_or(url);
                let runs = vec![
                    Run {
                        text: "Video: ".into(),
                        style: ov(|o| o.weight = Some(700.0)),
                        link: None,
                        extra: Extra::new(),
                    },
                    Run {
                        text: title.into(),
                        style: ov(|o| {
                            o.fill = Some(solid("#6d28d9"));
                            o.underline = Some(true);
                        }),
                        link: Some(LinkTarget::url(url)),
                        extra: Extra::new(),
                    },
                ];
                self.simple(
                    0.0,
                    14.0,
                    runs,
                    Some(BoxStyle {
                        bg: "#f3f4f6".into(),
                        border: "#6b7280".into(),
                    }),
                    1.5,
                );
            }
            "tableOfContents" => {
                let head = Run {
                    text: "Contents".into(),
                    style: ov(|o| {
                        o.weight = Some(700.0);
                        o.size = Some(18.0);
                    }),
                    link: None,
                    extra: Extra::new(),
                };
                self.simple(4.0, 6.0, vec![head], None, 1.4);
                for (level, text) in doc_headings(self.root) {
                    let indent = crate::pm::repeat("    ", js_max(0.0, level - 1.0));
                    self.simple(
                        0.0,
                        2.0,
                        vec![Run {
                            text: format!("{indent}{text}"),
                            style: ov(|o| o.fill = Some(solid("#4b5563"))),
                            link: None,
                            extra: Extra::new(),
                        }],
                        None,
                        1.4,
                    );
                }
                self.last_gap(16.0);
            }
            _ => {}
        }
        Ok(())
    }
}

/// `blocksOf(root)`.
pub fn blocks_of(root: &PmNode) -> Result<Vec<Block>, LayoutError> {
    let mut b = B {
        root,
        out: Vec::new(),
    };
    for n in children(root) {
        b.block(n, None)?;
    }
    Ok(b.out)
}

/// Split a single-paragraph body after `chars` UTF-16 units (at a line start).
fn split_body(body: &TextBody, chars: usize) -> (TextBody, TextBody) {
    let p = &body.paragraphs[0];
    let (mut a, mut b): (Vec<Run>, Vec<Run>) = (vec![], vec![]);
    let mut pos = 0;
    for r in &p.runs {
        let u: Vec<u16> = r.text.encode_utf16().collect();
        let end = pos + u.len();
        if end <= chars {
            a.push(r.clone());
        } else if pos >= chars {
            b.push(r.clone());
        } else {
            let k = chars - pos;
            a.push(Run {
                text: String::from_utf16_lossy(&u[..k]),
                ..r.clone()
            });
            b.push(Run {
                text: String::from_utf16_lossy(&u[k..]),
                ..r.clone()
            });
        }
        pos = end;
    }
    let empty = || vec![Run::plain("")];
    let first = Paragraph {
        runs: if a.is_empty() { empty() } else { a },
        ..p.clone()
    };
    let rest = Paragraph {
        runs: if b.is_empty() { empty() } else { b },
        list: None,
        ..p.clone()
    };
    (
        TextBody {
            paragraphs: vec![first],
            ..body.clone()
        },
        TextBody {
            paragraphs: vec![rest],
            ..body.clone()
        },
    )
}

/// Assets and embedded pictures for the layout.
pub trait DocLayoutDeps: Send + Sync {
    fn asset<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<AssetRef>>;
    /// A rendered picture of an embedded design's page registered as an asset, or `None` when the viewer may
    /// not see that design (a locked placeholder is drawn instead).
    fn embed<'a>(
        &'a self,
        design_id: &'a str,
        page_id: Option<&'a str>,
        width_px: f64,
    ) -> BoxFuture<'a, Option<AssetRef>>;
}

fn shape(x: f64, y: f64, w: f64, h: f64, fill: &str, radius: Option<f64>) -> Element {
    let params = radius.map(|r| [("cornerRadius".to_string(), r)].into_iter().collect());
    Element::Shape(make_shape(
        base_init(xf(x, y, w, h, 0.0)),
        ShapeOpts {
            kind: Some(oc_schema::elements::ShapeKind::Rect),
            fill: Some(solid(fill)),
            params,
            ..ShapeOpts::default()
        },
    ))
}

/// `docToDesign(r, body, meta, deps)` from the doc body JSON.
pub async fn doc_to_design(
    r: &dyn Renderer,
    host: &dyn RenderHost,
    root: &PmNode,
    title: &str,
    w: f64,
    h: f64,
    deps: &dyn DocLayoutDeps,
) -> Result<DesignDoc, LayoutError> {
    let blocks = blocks_of(root)?;
    let page_w = if w != 0.0 && !w.is_nan() { w } else { 816.0 };
    let page_h = if h != 0.0 && !h.is_nan() { h } else { 1056.0 };
    let (mx, my) = (80.0, 72.0);
    let cw = page_w - mx * 2.0;
    let mut pages: Vec<Vec<Element>> = vec![vec![]];
    let mut y = my;
    let mut assets: Vec<AssetRef> = Vec::new();
    for b in blocks {
        match b {
            Block::Pagebreak => {
                if !pages.last().expect("page").is_empty() || y > my {
                    pages.push(vec![]);
                    y = my;
                }
            }
            Block::Chart {
                data,
                width,
                height,
            } => {
                let bw = cw.min((width / DOC_CONTENT_WIDTH) * cw);
                let bh = (page_h - my * 2.0).min(height * (bw / width));
                if y + bh > page_h - my {
                    pages.push(vec![]);
                    y = my;
                }
                pages
                    .last_mut()
                    .expect("page")
                    .push(Element::Chart(doc_chart_element(
                        &data,
                        xf(mx + (cw - bw) / 2.0, y, bw, bh, 0.0),
                    )));
                y += bh + 16.0;
            }
            Block::Embed {
                design_id,
                page_id,
                width,
                title,
            } => {
                let bw = cw.min((width / DOC_CONTENT_WIDTH) * cw);
                let a = deps
                    .embed(
                        &design_id,
                        page_id.as_deref(),
                        oc_schema::js::round(bw * 2.0),
                    )
                    .await;
                let x = mx + (cw - bw) / 2.0;
                let Some(a) = a else {
                    let bh = 160f64.min(bw * 0.5);
                    if y + bh > page_h - my {
                        pages.push(vec![]);
                        y = my;
                    }
                    let page = pages.last_mut().expect("page");
                    page.push(shape(x, y, bw, bh, "#f3f4f6", Some(8.0)));
                    let style = ov(|o| {
                        o.size = Some(16.0);
                        o.family = Some("Inter".into());
                        o.fill = Some(solid("#6b7280"));
                    });
                    page.push(Element::Text(make_text(
                        base_init(xf(x, y + bh / 2.0 - 14.0, bw, 28.0, 0.0)),
                        TextOpts {
                            text: Some("Embedded design".into()),
                            style: Some(style),
                            align: Some(TextAlign::Center),
                            body: None,
                        },
                    )));
                    y += bh + 16.0;
                    continue;
                };
                let or = |v: f64| if v != 0.0 && !v.is_nan() { v } else { bw };
                let h0 = (bw * or(a.h)) / or(a.w);
                let fit = 1f64.min((page_h - my * 2.0) / h0);
                if y + h0 * fit > page_h - my {
                    pages.push(vec![]);
                    y = my;
                }
                let mut img = make_image(
                    base_init(xf(mx + (cw - bw * fit) / 2.0, y, bw * fit, h0 * fit, 0.0)),
                    &a.id,
                );
                img.base.alt_text = Some(
                    title
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| "Embedded design".into()),
                );
                pages.last_mut().expect("page").push(Element::Image(img));
                assets.push(a);
                y += h0 * fit + 16.0;
            }
            Block::Hr => {
                if y + 24.0 > page_h - my {
                    pages.push(vec![]);
                    y = my;
                }
                pages
                    .last_mut()
                    .expect("page")
                    .push(Element::Line(make_line(
                        base_init(xf(mx, y + 12.0, cw, 1.0, 0.0)),
                        LineOpts {
                            color: Some(c("#e5e7eb")),
                            width: Some(1.0),
                            ..LineOpts::default()
                        },
                    )));
                y += 30.0;
            }
            Block::Image { src, alt } => {
                let Some(id) = ASSET
                    .captures(&src)
                    .and_then(|m| m.get(1))
                    .map(|m| m.as_str().to_string())
                else {
                    continue;
                };
                let Some(a) = deps.asset(&id).await else {
                    continue;
                };
                let iw = cw.min(if a.w != 0.0 && !a.w.is_nan() { a.w } else { cw });
                let or = |v: f64| if v != 0.0 && !v.is_nan() { v } else { iw };
                let ih = (iw * or(a.h)) / or(a.w);
                let fit = 1f64.min((page_h - my * 2.0) / ih);
                if y + ih * fit > page_h - my {
                    pages.push(vec![]);
                    y = my;
                }
                let mut img = make_image(base_init(xf(mx, y, iw * fit, ih * fit, 0.0)), &a.id);
                if let Some(alt) = alt.as_ref().filter(|v| oc_schema::tree::truthy(v)) {
                    img.base.alt_text = Some(oc_schema::js::value_to_js_string(alt));
                }
                pages.last_mut().expect("page").push(Element::Image(img));
                assets.push(a);
                y += ih * fit + 16.0;
            }
            Block::Table { rows, header } => {
                let th = 34.0 * rows.len() as f64;
                if y + th > page_h - my {
                    pages.push(vec![]);
                    y = my;
                }
                let mut t = make_table(
                    base_init(xf(mx, y, cw, th, 0.0)),
                    TableOpts {
                        data: Some(rows),
                        ..TableOpts::default()
                    },
                );
                t.style.size = 14.0;
                t.style.family = "Inter".into();
                if header {
                    t.style.header_fill = Some(solid("#f5f3ff"));
                }
                pages.last_mut().expect("page").push(Element::Table(t));
                y += th + 16.0;
            }
            Block::Text {
                gap_before,
                gap_after,
                quote,
                code,
                bx,
                body,
            } => {
                // Empty paragraphs keep their space but produce no element.
                if !body.paragraphs.iter().any(|p| {
                    p.runs
                        .iter()
                        .any(|r| !oc_schema::js::trim(&r.text).is_empty())
                }) {
                    y += body.default_run.size * 1.5 + gap_after;
                    continue;
                }
                let (quote, code) = (quote.unwrap_or(false), code.unwrap_or(false));
                let mut body = body;
                y += gap_before;
                let pad = if code || bx.is_some() { 24.0 } else { 0.0 };
                let inset = if quote { 20.0 } else { 0.0 }
                    + if code { 28.0 } else { 0.0 }
                    + if bx.is_some() { 40.0 } else { 0.0 };
                let place = |pages: &mut Vec<Vec<Element>>, y: &mut f64, bd: TextBody, hh: f64| {
                    let page = pages.last_mut().expect("page");
                    let x = mx
                        + if quote { 20.0 } else { 0.0 }
                        + if code { 14.0 } else { 0.0 }
                        + if bx.is_some() { 20.0 } else { 0.0 };
                    if quote {
                        page.push(shape(mx, *y, 4.0, hh, "#c4b5fd", None));
                    }
                    if code {
                        page.push(shape(mx, *y, cw, hh, "#f3f4f6", Some(8.0)));
                    }
                    if let Some(b) = &bx {
                        page.push(shape(mx, *y, cw, hh, &b.bg, Some(6.0)));
                        page.push(shape(mx, *y, 4.0, hh, &b.border, None));
                    }
                    let padded = code || bx.is_some();
                    let el = make_text(
                        base_init(xf(
                            x,
                            *y + if padded { 12.0 } else { 0.0 },
                            cw - (x - mx) * if padded { 2.0 } else { 1.0 },
                            hh - if padded { 24.0 } else { 0.0 },
                            0.0,
                        )),
                        TextOpts {
                            body: Some(bd),
                            ..TextOpts::default()
                        },
                    );
                    page.push(Element::Text(el));
                    *y += hh;
                };
                for _guard in 0..200 {
                    let m = r.layout_text(host, &body, cw - inset).await?;
                    let th = m.height + pad;
                    let room = page_h - my - y;
                    if th <= room {
                        place(&mut pages, &mut y, body.clone(), th);
                        break;
                    }
                    // Split at the last line that fits (keep at least two lines together).
                    let mut fit = 0;
                    for (i, lm) in m.lines.iter().enumerate() {
                        if lm.baseline + lm.descent <= room - pad {
                            fit = i + 1;
                        }
                    }
                    if fit < 2 || m.lines.len() < 3 {
                        pages.push(vec![]);
                        y = my;
                        continue;
                    }
                    let at = m.lines.get(fit).map_or(0, |l| l.start);
                    let (first, rest) = split_body(&body, at);
                    let first_h = r.layout_text(host, &first, cw - inset).await?.height;
                    place(&mut pages, &mut y, first, first_h + pad);
                    pages.push(vec![]);
                    y = my;
                    body = rest;
                }
                y += gap_after;
            }
        }
    }
    let mut design = empty_design(&MetaInit {
        w: page_w,
        h: page_h,
        pages: Some(pages.len()),
        title: Some(title.into()),
        ..MetaInit::default()
    });
    let ids: Vec<String> = sorted_pages(&design.pages)
        .iter()
        .map(|p| p.id.to_string())
        .collect();
    for (i, mut els) in pages.into_iter().enumerate() {
        place_all_elements(&ids[i], &mut els, None);
        for e in els {
            let id = e.id().unwrap_or("").to_string();
            design.elements.insert(id, e);
        }
    }
    design.assets = assets.into_iter().map(|a| (a.id.clone(), a)).collect();
    Ok(design)
}
