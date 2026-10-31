// SPDX-License-Identifier: Apache-2.0
//! Docs → Word (DC-031), port of `services/render/src/docx.ts`.
//!
//! The TS builds the package with the `docx` npm package (9.7.2). This port writes the same OOXML itself: the
//! [`xml`] builder serialises like the `xml` package `docx` uses, the fixed parts are copied verbatim ([`parts`]),
//! and every construct `docx.ts` uses (paragraph/run properties, numbering instances, hyperlinks, bookmarks,
//! content-control checkboxes, inline pictures, tables with merged cells, page breaks) is emitted with the
//! element order, attribute values, relationship ids and validation errors of the package.
//!
//! Why not a docx crate: parity is measured part by part against the TS output (`tests/golden/render-svc/docx/`,
//! document.xml / styles / numbering / rels compared after normalisation). `docx-rs`, the maintained crate, has
//! its own defaults (styles, numbering definitions, settings, section and relationship layout), so its packages
//! could never match; the `zip` crate (already a workspace dependency) plus `quick-xml` (escaping here, parsing in
//! the tests) give exact control at a fraction of the code.
//!
//! Random ids: the package names hyperlink relationships with nanoid (`rId<21 random chars>`); here they are
//! `rIdl<n>`. Readers only need them to be unique.

pub mod parts;
pub mod xml;

use std::collections::HashMap;
use std::io::Write as _;

use futures::future::BoxFuture;
use oc_crdt::doc_body::{PmMark, PmNode, node_text};
use oc_schema::doc_blocks::{
    DOC_CONTENT_WIDTH, DocChartData, callout_style, callout_tone, doc_block_width, parse_doc_chart,
};
use oc_schema::js;
use regex::Regex;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::sync::LazyLock;

use crate::pm::{
    attr, attr_nonempty, attr_num, attr_or, attr_str, attr_truthy, children, js_max, js_min,
    mark_attr, marks, slice16,
};
use crate::renderer::RenderError;
use xml::{El, val};

/// Image kinds Word accepts here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Png,
    Jpg,
    Gif,
}

impl ImageType {
    pub fn ext(self) -> &'static str {
        match self {
            ImageType::Png => "png",
            ImageType::Jpg => "jpg",
            ImageType::Gif => "gif",
        }
    }

    pub fn parse(s: &str) -> Option<ImageType> {
        match s {
            "png" => Some(ImageType::Png),
            "jpg" => Some(ImageType::Jpg),
            "gif" => Some(ImageType::Gif),
            _ => None,
        }
    }
}

/// `DocxImage`: encoded bytes and their pixel size.
#[derive(Debug, Clone, PartialEq)]
pub struct DocxImage {
    pub data: Vec<u8>,
    pub kind: ImageType,
    pub w: f64,
    pub h: f64,
}

/// How a dependency call failed.
#[derive(Debug)]
pub enum DepError {
    /// An ordinary failure: the block is skipped or drawn as a placeholder (the TS `.catch(() => null)`).
    Failed(String),
    /// Aborts the export (the renderer is missing or broke): the caller answers with its status.
    Fatal(RenderError),
}

pub type DepResult = Result<Option<DocxImage>, DepError>;

/// `DocxDeps`: image bytes for the blocks that need pictures.
pub trait DocxDeps: Send {
    /// An `image` node's src (an /api/assets/:id URL) → bytes; `None` skips the image.
    fn image<'a>(&'a mut self, src: &'a str) -> BoxFuture<'a, DepResult>;
    /// A rendered picture of an embedded design page; `None` when the viewer may not see it.
    fn embed<'a>(
        &'a mut self,
        design_id: &'a str,
        page_id: Option<&'a str>,
        width_px: f64,
    ) -> BoxFuture<'a, DepResult>;
    /// A chart block rendered to a picture at the given size (px).
    fn chart<'a>(&'a mut self, data: &'a DocChartData, w: f64, h: f64) -> BoxFuture<'a, DepResult>;
}

#[derive(Debug, Clone, Default)]
pub struct DocxMeta {
    pub title: String,
    /// The document font; the most-used font of the doc when `None`.
    pub font: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DocxError {
    /// The `docx` package rejects the input (a non-integer measure, a colour that is not 6 hex digits).
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Renderer(#[from] RenderError),
    #[error("zip: {0}")]
    Zip(String),
}

/// The package as named parts (in the order the `docx` package zips them).
#[derive(Debug, Clone, Default)]
pub struct DocxPackage {
    pub parts: Vec<(String, Vec<u8>)>,
}

impl DocxPackage {
    pub fn part(&self, name: &str) -> Option<&[u8]> {
        self.parts
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.as_slice())
    }

    /// The .docx bytes (deflated zip).
    pub fn to_zip(&self) -> Result<Vec<u8>, DocxError> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in &self.parts {
            zip.start_file(name.as_str(), opts)
                .map_err(|e| DocxError::Zip(e.to_string()))?;
            zip.write_all(bytes)
                .map_err(|e| DocxError::Zip(e.to_string()))?;
        }
        Ok(zip
            .finish()
            .map_err(|e| DocxError::Zip(e.to_string()))?
            .into_inner())
    }
}

/// px → half-points (Word font sizes).
fn hp(px: f64) -> f64 {
    js::round(px * 0.75 * 2.0)
}

const MONO: &str = "JetBrains Mono";

static RGB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"rgba?\((\d+),\s*(\d+),\s*(\d+)").expect("regex"));
static HEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^#([0-9a-f]{3}|[0-9a-f]{6})([0-9a-f]{2})?$").expect("regex"));
static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(https?:|mailto:)").expect("regex"));
static HTTP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://").expect("regex"));
static ASSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/api/assets/([\w-]+)").expect("regex"));

/// `hexOf`: CSS `rgb()`/`#hex` → `RRGGBB` (alpha dropped). Channels above 255 give more digits, which the
/// package then rejects.
pub fn hex_of(c: Option<&Value>) -> Option<String> {
    let c = c?.as_str()?;
    if let Some(m) = RGB.captures(c) {
        return Some(
            (1..=3)
                .map(|i| {
                    let v: f64 = m[i].parse().unwrap_or(f64::NAN);
                    format!("{:02x}", v as u64)
                })
                .collect::<String>()
                .to_uppercase(),
        );
    }
    let t = js::trim(c);
    let m = HEX.captures(t)?;
    let h = &m[1];
    let v = if h.len() == 3 {
        h.chars().flat_map(|x| [x, x]).collect()
    } else {
        h.to_string()
    };
    Some(v.to_uppercase())
}

fn family_of(f: Option<&Value>) -> Option<String> {
    let f = f?.as_str().filter(|s| !s.is_empty())?;
    let cleaned: String = f.chars().filter(|c| *c != '"' && *c != '\'').collect();
    Some(js::trim(cleaned.split(',').next().unwrap_or("")).to_string())
}

/// `docDefaultFont`: the most-used font in the doc's text-style marks (by UTF-16 length), else Inter.
pub fn doc_default_font(root: &PmNode) -> String {
    let mut order: Vec<String> = Vec::new();
    let mut count: HashMap<String, usize> = HashMap::new();
    fn walk(n: &PmNode, order: &mut Vec<String>, count: &mut HashMap<String, usize>) {
        if n.kind == "text" {
            let f = marks(n)
                .iter()
                .find(|m| m.kind == "textStyle")
                .and_then(|m| family_of(mark_attr(m, "fontFamily")))
                .unwrap_or_else(|| "Inter".into());
            let len = n.text.as_deref().map_or(0, js::utf16_len);
            if !count.contains_key(&f) {
                order.push(f.clone());
            }
            *count.entry(f).or_insert(0) += len;
        }
        for c in children(n) {
            walk(c, order, count);
        }
    }
    walk(root, &mut order, &mut count);
    // Stable sort by count, descending: ties keep first-seen order.
    order.sort_by(|a, b| count[b].cmp(&count[a]));
    order.into_iter().next().unwrap_or_else(|| "Inter".into())
}

/// Paragraph alignment (`AlignmentType`): `start`/`left`/unknown → none.
fn align_of(a: Option<&Value>) -> Option<&'static str> {
    match a.and_then(Value::as_str) {
        Some("center") => Some("center"),
        Some("right") => Some("right"),
        Some("justify") => Some("both"),
        _ => None,
    }
}

// ------------------------------------------------------------------ validation (the package's value checks)

/// `decimalNumber`: NaN throws, otherwise floored.
fn decimal(v: f64) -> Result<String, DocxError> {
    if v.is_nan() {
        return Err(DocxError::Invalid(format!(
            "Invalid value '{}' specified. Must be an integer.",
            js::num_to_string(v)
        )));
    }
    Ok(js::num_to_string(v.floor()))
}

/// `unsignedDecimalNumber`.
fn unsigned(v: f64) -> Result<String, DocxError> {
    let s = decimal(v)?;
    if v.floor() < 0.0 {
        return Err(DocxError::Invalid(format!(
            "Invalid value '{}' specified. Must be a positive integer.",
            js::num_to_string(v)
        )));
    }
    Ok(s)
}

/// `hexColorValue`: `auto` or 6 hex digits (a leading `#` is dropped).
fn hex_color(v: &str) -> Result<String, DocxError> {
    if v == "auto" {
        return Ok(v.into());
    }
    let h = v.strip_prefix('#').unwrap_or(v);
    if js::utf16_len(h) != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(DocxError::Invalid(format!(
            "Invalid hex value '{h}'. Expected 6 digit hex value"
        )));
    }
    Ok(h.to_string())
}

// ------------------------------------------------------------------ run / paragraph models

/// `IRunOptions` (the fields docx.ts sets).
#[derive(Debug, Clone, Default)]
struct RunOpts {
    font: Option<String>,
    bold: bool,
    italics: bool,
    strike: bool,
    color: Option<String>,
    size: Option<f64>,
    underline: bool,
    /// `shading: { type: CLEAR, color: 'auto', fill }`.
    shading: Option<String>,
    sub: bool,
    sup: bool,
}

fn shd(fill: &str) -> Result<El, DocxError> {
    Ok(El::new("w:shd")
        .a("w:fill", hex_color(fill)?)
        .a("w:color", "auto")
        .a("w:val", "clear"))
}

fn run_props(o: &RunOpts) -> Result<Option<El>, DocxError> {
    let mut p = El::new("w:rPr");
    if let Some(f) = o.font.as_deref().filter(|f| !f.is_empty()) {
        p.push(
            El::new("w:rFonts")
                .a("w:ascii", f)
                .a("w:cs", f)
                .a("w:eastAsia", f)
                .a("w:hAnsi", f),
        );
    }
    if o.bold {
        p.push(El::new("w:b"));
        p.push(El::new("w:bCs"));
    }
    if o.italics {
        p.push(El::new("w:i"));
        p.push(El::new("w:iCs"));
    }
    if o.strike {
        p.push(El::new("w:strike"));
    }
    if let Some(c) = o.color.as_deref().filter(|c| !c.is_empty()) {
        p.push(val("w:color", hex_color(c)?));
    }
    if let Some(s) = o.size {
        let v = unsigned(s)?;
        p.push(val("w:sz", v.clone()));
        if s != 0.0 {
            p.push(val("w:szCs", v));
        }
    }
    if o.underline {
        p.push(val("w:u", "single"));
    }
    if let Some(fill) = &o.shading {
        p.push(shd(fill)?);
    }
    if o.sub {
        p.push(val("w:vertAlign", "subscript"));
    }
    if o.sup {
        p.push(val("w:vertAlign", "superscript"));
    }
    Ok(if p.kids.is_empty() { None } else { Some(p) })
}

/// `new TextRun({ ...opts, text, break })`.
fn text_run(o: &RunOpts, text: Option<&str>, breaks: usize) -> Result<El, DocxError> {
    let mut r = El::new("w:r");
    if let Some(p) = run_props(o)? {
        r.push(p);
    }
    for _ in 0..breaks {
        r.push(El::new("w:br"));
    }
    if let Some(t) = text {
        r.push(El::new("w:t").a("xml:space", "preserve").t(t));
    }
    Ok(r)
}

#[derive(Debug, Clone)]
struct Border {
    side: &'static str,
    size: f64,
    color: String,
    space: f64,
}

/// `IParagraphOptions` (the fields docx.ts sets).
#[derive(Debug, Clone, Default)]
struct ParaOpts {
    heading: Option<&'static str>,
    /// `numbering: { reference, level, instance }` — `(reference, instance)` resolved to a numId at write time.
    numbering: Option<(&'static str, f64, u32)>,
    borders: Vec<Border>,
    shading: Option<String>,
    spacing: Option<(Option<f64>, Option<f64>)>,
    indent: Option<Indent>,
    alignment: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Indent {
    left: Option<f64>,
    right: Option<f64>,
    hanging: Option<f64>,
}

const HEADINGS: [&str; 6] = [
    "Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6",
];

/// The document being written: relationships, media and numbering instances, in creation order.
struct Doc {
    body: Vec<El>,
    links: Vec<String>,
    media: Vec<(String, Vec<u8>)>,
    numbering: Vec<(&'static str, u32)>,
}

impl Doc {
    fn num_id(&mut self, reference: &'static str, instance: u32) -> usize {
        if let Some(i) = self
            .numbering
            .iter()
            .position(|(r, n)| *r == reference && *n == instance)
        {
            return i + 2;
        }
        self.numbering.push((reference, instance));
        self.numbering.len() + 1
    }

    fn para(&mut self, o: &ParaOpts, kids: Vec<El>) -> Result<El, DocxError> {
        let mut p = El::new("w:p");
        let mut pr = El::new("w:pPr");
        if let Some(h) = o.heading {
            pr.push(val("w:pStyle", h));
        }
        if let Some((reference, level, instance)) = o.numbering {
            if o.heading.is_none() {
                pr.push(val("w:pStyle", "ListParagraph"));
            }
            let id = self.num_id(reference, instance);
            pr.push(
                El::new("w:numPr")
                    .c(val("w:ilvl", js::num_to_string(level)))
                    .c(val("w:numId", id.to_string())),
            );
        }
        if !o.borders.is_empty() {
            let mut b = El::new("w:pBdr");
            for side in ["w:top", "w:bottom", "w:left", "w:right"] {
                for x in o.borders.iter().filter(|x| x.side == side) {
                    b.push(
                        El::new(side)
                            .a("w:val", "single")
                            .a("w:color", hex_color(&x.color)?)
                            .a("w:sz", unsigned(x.size)?)
                            .a("w:space", unsigned(x.space)?),
                    );
                }
            }
            pr.push(b);
        }
        if let Some(fill) = &o.shading {
            pr.push(shd(fill)?);
        }
        if let Some((before, after)) = o.spacing {
            let mut s = El::new("w:spacing");
            if let Some(a) = after {
                s = s.a("w:after", js::num_to_string(a));
            }
            if let Some(b) = before {
                s = s.a("w:before", js::num_to_string(b));
            }
            pr.push(s);
        }
        if let Some(ind) = o.indent {
            let mut e = El::new("w:ind");
            if let Some(l) = ind.left {
                e = e.a("w:left", decimal(l)?);
            }
            if let Some(r) = ind.right {
                e = e.a("w:right", decimal(r)?);
            }
            if let Some(h) = ind.hanging {
                e = e.a("w:hanging", unsigned(h)?);
            }
            pr.push(e);
        }
        if let Some(a) = o.alignment {
            pr.push(val("w:jc", a));
        }
        if !pr.kids.is_empty() {
            p.push(pr);
        }
        for k in kids {
            p.push(k);
        }
        Ok(p)
    }

    fn hyperlink(&mut self, url: &str, runs: Vec<El>) -> El {
        self.links.push(url.to_string());
        let mut h = El::new("w:hyperlink")
            .a("w:history", "1")
            .a("r:id", format!("rIdl{}", self.links.len()));
        h.kids.extend(runs.into_iter().map(xml::Node::El));
        h
    }

    /// `new ImageRun(...)`: the picture is stored once per content (sha1 name) and referenced by a placeholder
    /// that becomes `rId<7 + media index>`.
    fn image_run(&mut self, img: &DocxImage, max_w: f64, alt: Option<&str>) -> El {
        let w = max_w.min(if img.w != 0.0 && !img.w.is_nan() {
            img.w
        } else {
            max_w
        });
        let w_or = |v: f64| if v != 0.0 && !v.is_nan() { v } else { w };
        let h = js::round((w * w_or(img.h)) / w_or(img.w));
        let name = format!(
            "{}.{}",
            hex::encode(Sha1::digest(&img.data)),
            img.kind.ext()
        );
        let idx = match self.media.iter().position(|(n, _)| *n == name) {
            Some(i) => i,
            None => {
                self.media.push((name, img.data.clone()));
                self.media.len() - 1
            }
        };
        let (cx, cy) = (js::round(js::round(w) * 9525.0), js::round(h * 9525.0));
        let (cx, cy) = (js::num_to_string(cx), js::num_to_string(cy));
        let doc_pr = match alt.filter(|a| !a.is_empty()) {
            Some(a) => El::new("wp:docPr")
                .a("id", "1")
                .a("name", slice16(a, 100))
                .a("descr", a)
                .a("title", slice16(a, 100)),
            None => El::new("wp:docPr")
                .a("id", "1")
                .a("name", "")
                .a("descr", "")
                .a("title", ""),
        };
        const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
        const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
        let inline = El::new("wp:inline")
            .a("distT", "0")
            .a("distB", "0")
            .a("distL", "0")
            .a("distR", "0")
            .c(El::new("wp:extent").a("cx", cx.clone()).a("cy", cy.clone()))
            .c(El::new("wp:effectExtent")
                .a("t", "0")
                .a("r", "0")
                .a("b", "0")
                .a("l", "0"))
            .c(doc_pr)
            .c(
                El::new("wp:cNvGraphicFramePr").c(El::new("a:graphicFrameLocks")
                    .a("xmlns:a", A)
                    .a("noChangeAspect", "1")),
            )
            .c(El::new("a:graphic")
                .a("xmlns:a", A)
                .c(El::new("a:graphicData").a("uri", PIC).c(El::new("pic:pic")
                    .a("xmlns:pic", PIC)
                    .c(El::new("pic:nvPicPr")
                        .c(El::new("pic:cNvPr")
                            .a("id", "0")
                            .a("name", "")
                            .a("descr", ""))
                        .c(El::new("pic:cNvPicPr").c(El::new("a:picLocks")
                            .a("noChangeAspect", "1")
                            .a("noChangeArrowheads", "1"))))
                    .c(El::new("pic:blipFill")
                        .c(El::new("a:blip")
                            .a("r:embed", format!("rId{}", 7 + idx))
                            .a("cstate", "none"))
                        .c(El::new("a:srcRect"))
                        .c(El::new("a:stretch").c(El::new("a:fillRect"))))
                    .c(El::new("pic:spPr")
                        .a("bwMode", "auto")
                        .c(El::new("a:xfrm")
                            .c(El::new("a:off").a("x", "0").a("y", "0"))
                            .c(El::new("a:ext").a("cx", cx).a("cy", cy)))
                        .c(El::new("a:prstGeom")
                            .a("prst", "rect")
                            .c(El::new("a:avLst")))))));
        El::new("w:r").c(El::new("w:drawing").c(inline))
    }
}

/// `runOpts(marks, extra)`.
fn run_opts(marks: &[PmMark], extra: &RunOpts) -> (RunOpts, Option<String>) {
    let mut o = extra.clone();
    let mut link: Option<String> = None;
    for m in marks {
        match m.kind.as_str() {
            "bold" => o.bold = true,
            "italic" => o.italics = true,
            "underline" => o.underline = true,
            "strike" => o.strike = true,
            "code" => {
                o.font = Some(MONO.into());
                o.shading = Some("F3F4F6".into());
            }
            "highlight" => {
                o.shading = Some(hex_of(mark_attr(m, "color")).unwrap_or_else(|| "FDE68A".into()))
            }
            "subscript" => o.sub = true,
            "superscript" => o.sup = true,
            "link" => {
                if let Some(h) = mark_attr(m, "href")
                    .and_then(Value::as_str)
                    .filter(|h| LINK.is_match(h))
                {
                    link = Some(h.to_string());
                }
            }
            "textStyle" => {
                if let Some(c) = hex_of(mark_attr(m, "color")) {
                    o.color = Some(c);
                }
                if let Some(f) = family_of(mark_attr(m, "fontFamily")).filter(|f| !f.is_empty()) {
                    o.font = Some(f);
                }
            }
            _ => {}
        }
    }
    if link.is_some() {
        if o.color.is_none() {
            o.color = Some("6D28D9".into());
        }
        o.underline = true;
    }
    (o, link)
}

/// Inline content → paragraph children (consecutive runs of one link share a hyperlink).
fn inline(doc: &mut Doc, nodes: &[PmNode], extra: &RunOpts) -> Result<Vec<El>, DocxError> {
    let mut out: Vec<El> = Vec::new();
    let mut group: Option<(String, Vec<El>)> = None;
    fn flush(doc: &mut Doc, out: &mut Vec<El>, group: &mut Option<(String, Vec<El>)>) {
        if let Some((link, runs)) = group.take() {
            out.push(doc.hyperlink(&link, runs));
        }
    }
    for n in nodes {
        if n.kind == "hardBreak" {
            flush(doc, &mut out, &mut group);
            out.push(text_run(extra, None, 1)?);
            continue;
        }
        let text = match (&*n.kind, &n.text) {
            ("text", Some(t)) if !t.is_empty() => t,
            _ => continue,
        };
        let (opts, link) = run_opts(marks(n), extra);
        let run = text_run(&opts, Some(text), 0)?;
        match link {
            Some(link) => {
                if group.as_ref().is_some_and(|(l, _)| *l != link) {
                    flush(doc, &mut out, &mut group);
                }
                group.get_or_insert_with(|| (link, Vec::new())).1.push(run);
            }
            None => {
                flush(doc, &mut out, &mut group);
                out.push(run);
            }
        }
    }
    flush(doc, &mut out, &mut group);
    Ok(out)
}

#[derive(Clone, Copy, PartialEq)]
enum ListKind {
    Bullet,
    Number,
    Task,
}

#[derive(Clone)]
struct BoxStyle {
    bg: String,
    border: String,
}

struct Writer<'d> {
    deps: &'d mut dyn DocxDeps,
    doc: Doc,
    number_instance: u32,
    heading_no: usize,
    has_toc: bool,
    headings: Vec<(f64, String, String)>,
}

async fn dep(fut: BoxFuture<'_, DepResult>) -> Result<Option<DocxImage>, DocxError> {
    match fut.await {
        Ok(v) => Ok(v),
        Err(DepError::Failed(_)) => Ok(None),
        Err(DepError::Fatal(e)) => Err(DocxError::Renderer(e)),
    }
}

impl Writer<'_> {
    fn push_para(&mut self, o: &ParaOpts, kids: Vec<El>) -> Result<(), DocxError> {
        let p = self.doc.para(o, kids)?;
        self.doc.body.push(p);
        Ok(())
    }

    fn list<'a>(
        &'a mut self,
        n: &'a PmNode,
        kind: ListKind,
        level: u32,
        instance: u32,
    ) -> BoxFuture<'a, Result<(), DocxError>> {
        Box::pin(async move {
            for item in children(n) {
                let mut first = true;
                for c in children(item) {
                    match c.kind.as_str() {
                        "paragraph" => {
                            let lvl = level.min(8);
                            let alignment = align_of(attr(c, "textAlign"));
                            if kind == ListKind::Task {
                                let checked = attr_truthy(item, "checked");
                                let mut kids = Vec::new();
                                if first {
                                    kids.push(checkbox(checked));
                                    kids.push(text_run(&RunOpts::default(), Some(" "), 0)?);
                                }
                                let extra = if checked {
                                    RunOpts {
                                        color: Some("6B7280".into()),
                                        ..RunOpts::default()
                                    }
                                } else {
                                    RunOpts::default()
                                };
                                kids.extend(inline(&mut self.doc, children(c), &extra)?);
                                let o = ParaOpts {
                                    indent: Some(Indent {
                                        left: Some(360.0 * (lvl as f64 + 1.0)),
                                        hanging: Some(360.0),
                                        ..Indent::default()
                                    }),
                                    alignment,
                                    ..ParaOpts::default()
                                };
                                self.push_para(&o, kids)?;
                            } else {
                                let mut o = ParaOpts {
                                    alignment,
                                    ..ParaOpts::default()
                                };
                                if first {
                                    let reference = if kind == ListKind::Bullet {
                                        "oc-bullet"
                                    } else {
                                        "oc-number"
                                    };
                                    o.numbering = Some((
                                        reference,
                                        f64::from(lvl),
                                        if kind == ListKind::Number {
                                            instance
                                        } else {
                                            0
                                        },
                                    ));
                                } else {
                                    o.indent = Some(Indent {
                                        left: Some(720.0 * (lvl as f64 + 1.0)),
                                        ..Indent::default()
                                    });
                                }
                                // Children first, then the paragraph (numbering instances are created when the
                                // paragraph is formatted, after its runs exist; ids do not depend on it).
                                let kids = inline(&mut self.doc, children(c), &RunOpts::default())?;
                                self.push_para(&o, kids)?;
                            }
                            first = false;
                        }
                        "bulletList" => self.list(c, ListKind::Bullet, level + 1, instance).await?,
                        "orderedList" => {
                            self.number_instance += 1;
                            let i = self.number_instance;
                            self.list(c, ListKind::Number, level + 1, i).await?;
                        }
                        "taskList" => self.list(c, ListKind::Task, level + 1, instance).await?,
                        _ => self.block(c, None).await?,
                    }
                }
            }
            Ok(())
        })
    }

    fn block<'a>(
        &'a mut self,
        n: &'a PmNode,
        bx: Option<&'a BoxStyle>,
    ) -> BoxFuture<'a, Result<(), DocxError>> {
        Box::pin(async move {
            match n.kind.as_str() {
                "heading" => {
                    let lvl = js_max(1.0, js_min(6.0, attr_num(n, "level", 1.0)));
                    let heading = if lvl.fract() == 0.0 && (1.0..=6.0).contains(&lvl) {
                        Some(HEADINGS[lvl as usize - 1])
                    } else {
                        None
                    };
                    let kids = inline(&mut self.doc, children(n), &RunOpts::default())?;
                    let id = if self.has_toc {
                        let id = self.headings.get(self.heading_no).map(|h| h.2.clone());
                        self.heading_no += 1;
                        id
                    } else {
                        None
                    };
                    let kids = match id {
                        Some(id) => {
                            let mut v =
                                vec![El::new("w:bookmarkStart").a("w:name", id).a("w:id", "1")];
                            v.extend(kids);
                            v.push(El::new("w:bookmarkEnd").a("w:id", "1"));
                            v
                        }
                        None => kids,
                    };
                    self.push_para(
                        &ParaOpts {
                            heading,
                            alignment: align_of(attr(n, "textAlign")),
                            ..ParaOpts::default()
                        },
                        kids,
                    )?;
                }
                "paragraph" => {
                    let mut o = ParaOpts {
                        alignment: align_of(attr(n, "textAlign")),
                        ..ParaOpts::default()
                    };
                    if let Some(b) = bx {
                        o.shading = Some(b.bg.clone());
                        o.borders = vec![Border {
                            side: "w:left",
                            size: 24.0,
                            color: b.border.clone(),
                            space: 8.0,
                        }];
                        o.indent = Some(Indent {
                            left: Some(200.0),
                            right: Some(200.0),
                            hanging: None,
                        });
                    }
                    let kids = inline(&mut self.doc, children(n), &RunOpts::default())?;
                    self.push_para(&o, kids)?;
                }
                "bulletList" => self.list(n, ListKind::Bullet, 0, 0).await?,
                "orderedList" => {
                    self.number_instance += 1;
                    let i = self.number_instance;
                    self.list(n, ListKind::Number, 0, i).await?;
                }
                "taskList" => self.list(n, ListKind::Task, 0, 0).await?,
                "blockquote" => {
                    for c in children(n) {
                        if c.kind == "paragraph" {
                            let extra = RunOpts {
                                italics: true,
                                color: Some("4B5563".into()),
                                ..RunOpts::default()
                            };
                            let kids = inline(&mut self.doc, children(c), &extra)?;
                            let o = ParaOpts {
                                indent: Some(Indent {
                                    left: Some(480.0),
                                    ..Indent::default()
                                }),
                                borders: vec![Border {
                                    side: "w:left",
                                    size: 18.0,
                                    color: "C4B5FD".into(),
                                    space: 12.0,
                                }],
                                ..ParaOpts::default()
                            };
                            self.push_para(&o, kids)?;
                        } else {
                            self.block(c, None).await?;
                        }
                    }
                }
                "codeBlock" => {
                    let text = node_text(n);
                    let mono = RunOpts {
                        font: Some(MONO.into()),
                        size: Some(hp(14.0)),
                        ..RunOpts::default()
                    };
                    let kids = text
                        .split('\n')
                        .enumerate()
                        .map(|(i, l)| text_run(&mono, Some(l), usize::from(i > 0)))
                        .collect::<Result<Vec<_>, _>>()?;
                    let o = ParaOpts {
                        shading: Some("F3F4F6".into()),
                        spacing: Some((Some(80.0), Some(160.0))),
                        ..ParaOpts::default()
                    };
                    self.push_para(&o, kids)?;
                }
                "horizontalRule" => {
                    let o = ParaOpts {
                        borders: vec![Border {
                            side: "w:bottom",
                            size: 6.0,
                            color: "E5E7EB".into(),
                            space: 1.0,
                        }],
                        ..ParaOpts::default()
                    };
                    self.push_para(&o, vec![])?;
                }
                "pageBreak" => {
                    let br = El::new("w:r").c(El::new("w:br").a("w:type", "page"));
                    self.push_para(&ParaOpts::default(), vec![br])?;
                }
                "image" => {
                    let src = attr_str(n, "src").unwrap_or("");
                    let img = if src.is_empty() {
                        None
                    } else {
                        dep(self.deps.image(src)).await?
                    };
                    if let Some(img) = img {
                        let r = self
                            .doc
                            .image_run(&img, DOC_CONTENT_WIDTH, attr_str(n, "alt"));
                        self.push_para(&ParaOpts::default(), vec![r])?;
                    }
                }
                "designEmbed" => {
                    let Some(id) = attr_nonempty(n, "designId") else {
                        return Ok(());
                    };
                    let w = doc_block_width(attr(n, "width"), None);
                    let img = dep(self.deps.embed(id, attr_nonempty(n, "pageId"), w * 2.0)).await?;
                    match img {
                        Some(img) => {
                            let h = js::round(
                                (w * img.h)
                                    / if img.w != 0.0 && !img.w.is_nan() {
                                        img.w
                                    } else {
                                        1.0
                                    },
                            );
                            let title = attr_str(n, "title").unwrap_or("Embedded design");
                            let r = self
                                .doc
                                .image_run(&DocxImage { w, h, ..img }, w, Some(title));
                            self.push_para(
                                &ParaOpts {
                                    alignment: Some("center"),
                                    ..ParaOpts::default()
                                },
                                vec![r],
                            )?;
                        }
                        None => {
                            let run = text_run(
                                &RunOpts {
                                    color: Some("6B7280".into()),
                                    ..RunOpts::default()
                                },
                                Some("Embedded design"),
                                0,
                            )?;
                            self.push_para(
                                &ParaOpts {
                                    alignment: Some("center"),
                                    shading: Some("F3F4F6".into()),
                                    ..ParaOpts::default()
                                },
                                vec![run],
                            )?;
                        }
                    }
                }
                "chart" => {
                    let data = parse_doc_chart(attr(n, "data"));
                    let w = doc_block_width(attr(n, "width"), None);
                    let hv = attr(n, "height").map_or(f64::NAN, js::value_to_number);
                    let h = js_max(
                        120.0,
                        js_min(900.0, if hv == 0.0 || hv.is_nan() { 360.0 } else { hv }),
                    );
                    if let Some(img) = dep(self.deps.chart(&data, w, h)).await? {
                        let alt = format!(
                            "Chart: {}",
                            data.series
                                .iter()
                                .map(|s| s.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        let r = self
                            .doc
                            .image_run(&DocxImage { w, h, ..img }, w, Some(&alt));
                        self.push_para(
                            &ParaOpts {
                                alignment: Some("center"),
                                ..ParaOpts::default()
                            },
                            vec![r],
                        )?;
                    }
                }
                "callout" => {
                    let tone = callout_style(&callout_tone(attr(n, "tone")));
                    let style = BoxStyle {
                        bg: tone.bg[1..].to_uppercase(),
                        border: tone.border[1..].to_uppercase(),
                    };
                    for c in children(n) {
                        self.block(c, Some(&style)).await?;
                    }
                }
                "videoEmbed" => {
                    let Some(url) = attr_str(n, "url").filter(|u| HTTP.is_match(u)) else {
                        return Ok(());
                    };
                    let title = attr_nonempty(n, "title").unwrap_or(url);
                    let play = text_run(&RunOpts::default(), Some("▶ "), 0)?;
                    let link = text_run(
                        &RunOpts {
                            color: Some("6D28D9".into()),
                            underline: true,
                            ..RunOpts::default()
                        },
                        Some(title),
                        0,
                    )?;
                    let h = self.doc.hyperlink(url, vec![link]);
                    self.push_para(
                        &ParaOpts {
                            shading: Some("F3F4F6".into()),
                            ..ParaOpts::default()
                        },
                        vec![play, h],
                    )?;
                }
                "tableOfContents" => {
                    let head = text_run(
                        &RunOpts {
                            bold: true,
                            size: Some(hp(18.0)),
                            ..RunOpts::default()
                        },
                        Some("Contents"),
                        0,
                    )?;
                    self.push_para(
                        &ParaOpts {
                            spacing: Some((None, Some(80.0))),
                            ..ParaOpts::default()
                        },
                        vec![head],
                    )?;
                    for (level, text, id) in self.headings.clone() {
                        let run = text_run(
                            &RunOpts {
                                color: Some("4B5563".into()),
                                ..RunOpts::default()
                            },
                            Some(&text),
                            0,
                        )?;
                        let link = El::new("w:hyperlink")
                            .a("w:history", "1")
                            .a("w:anchor", id)
                            .c(run);
                        self.push_para(
                            &ParaOpts {
                                indent: Some(Indent {
                                    left: Some(360.0 * (level - 1.0)),
                                    ..Indent::default()
                                }),
                                ..ParaOpts::default()
                            },
                            vec![link],
                        )?;
                    }
                }
                "table" => self.table(n)?,
                _ => {
                    for c in children(n) {
                        self.block(c, bx).await?;
                    }
                }
            }
            Ok(())
        })
    }

    fn table(&mut self, n: &PmNode) -> Result<(), DocxError> {
        let rows: Vec<&PmNode> = children(n)
            .iter()
            .filter(|r| r.kind == "tableRow")
            .collect();
        if rows.is_empty() {
            return Ok(());
        }
        let span_of = |c: &PmNode, k: &str| attr(c, k).map_or(f64::NAN, js::value_to_number);
        let sum = |r: &PmNode| {
            children(r)
                .iter()
                .map(|c| {
                    let v = span_of(c, "colspan");
                    if v == 0.0 || v.is_nan() { 1.0 } else { v }
                })
                .sum::<f64>()
        };
        let cols = rows.iter().fold(1.0, |m, r| js_max(m, sum(r)));
        // `Array.from({ length: cols })` (ToLength floors), each `Math.floor(9360 / cols)`.
        let width = (9360.0 / cols).floor();
        let grid = (0..cols.floor() as usize)
            .map(|_| Ok(El::new("w:gridCol").a("w:w", unsigned(width)?)))
            .collect::<Result<Vec<_>, DocxError>>()?;

        struct Cell {
            col_span: Option<f64>,
            row_span: Option<f64>,
            cont: bool,
            el: El,
        }
        let margins = || {
            El::new("w:tcMar")
                .c(El::new("w:top").a("w:type", "dxa").a("w:w", "60"))
                .c(El::new("w:left").a("w:type", "dxa").a("w:w", "100"))
                .c(El::new("w:bottom").a("w:type", "dxa").a("w:w", "60"))
                .c(El::new("w:right").a("w:type", "dxa").a("w:w", "100"))
        };
        let mut table_rows: Vec<Vec<Cell>> = Vec::new();
        for r in &rows {
            let mut cells = Vec::new();
            for cell in children(r) {
                let header = cell.kind == "tableHeader";
                let cs = span_of(cell, "colspan");
                let rs = span_of(cell, "rowspan");
                let col_span = (cs > 1.0).then_some(cs);
                let row_span = (rs > 1.0).then_some(rs);
                let mut pr = El::new("w:tcPr");
                if let Some(c) = col_span {
                    pr.push(val("w:gridSpan", decimal(c)?));
                }
                if row_span.is_some() {
                    pr.push(val("w:vMerge", "restart"));
                }
                if header {
                    pr.push(shd("F5F3FF")?);
                }
                pr.push(margins());
                let mut tc = El::new("w:tc").c(pr);
                let paras: Vec<&PmNode> = children(cell)
                    .iter()
                    .filter(|p| p.kind == "paragraph")
                    .collect();
                let extra = if header {
                    RunOpts {
                        bold: true,
                        ..RunOpts::default()
                    }
                } else {
                    RunOpts::default()
                };
                if paras.is_empty() {
                    tc.push(self.doc.para(&ParaOpts::default(), vec![])?);
                }
                for p in paras {
                    let kids = inline(&mut self.doc, children(p), &extra)?;
                    tc.push(self.doc.para(
                        &ParaOpts {
                            alignment: align_of(attr(p, "textAlign")),
                            ..ParaOpts::default()
                        },
                        kids,
                    )?);
                }
                cells.push(Cell {
                    col_span,
                    row_span,
                    cont: false,
                    el: tc,
                });
            }
            table_rows.push(cells);
        }
        // Row spans: the package inserts a `vMerge="continue"` cell at the same column of the next row (which
        // cascades while the remaining span is above 1).
        for ri in 0..table_rows.len().saturating_sub(1) {
            let mut column = 0.0;
            let mut inserts: Vec<(f64, Cell)> = Vec::new();
            for c in &table_rows[ri] {
                if let Some(rs) = c.row_span.filter(|rs| *rs > 1.0) {
                    let mut pr = El::new("w:tcPr");
                    if let Some(cs) = c.col_span {
                        pr.push(val("w:gridSpan", decimal(cs)?));
                    }
                    pr.push(val("w:vMerge", "continue"));
                    let el = El::new("w:tc").c(pr).c(El::new("w:p"));
                    inserts.push((
                        column,
                        Cell {
                            col_span: c.col_span,
                            row_span: Some(rs - 1.0),
                            cont: true,
                            el,
                        },
                    ));
                }
                column += c.col_span.unwrap_or(1.0);
            }
            for (col, cell) in inserts {
                let next = &mut table_rows[ri + 1];
                // `columnIndexToRootIndex(col, true)`: the cell whose span covers `col`, or the end.
                let mut at = next.len();
                let mut acc = 0.0;
                for (i, c) in next.iter().enumerate() {
                    if acc > col {
                        break;
                    }
                    acc += c.col_span.unwrap_or(1.0);
                    at = i;
                    if acc > col {
                        break;
                    }
                }
                if acc <= col {
                    at = next.len();
                }
                next.insert(at, cell);
            }
        }
        let mut tbl = El::new("w:tbl").c(El::new("w:tblPr")
            .c(El::new("w:tblW").a("w:type", "pct").a("w:w", "100%"))
            .c({
                let b = |side: &'static str| {
                    El::new(side)
                        .a("w:val", "single")
                        .a("w:color", "auto")
                        .a("w:sz", "4")
                };
                El::new("w:tblBorders")
                    .c(b("w:top"))
                    .c(b("w:left"))
                    .c(b("w:bottom"))
                    .c(b("w:right"))
                    .c(b("w:insideH"))
                    .c(b("w:insideV"))
            }));
        let mut g = El::new("w:tblGrid");
        g.kids.extend(grid.into_iter().map(xml::Node::El));
        tbl.push(g);
        for cells in table_rows {
            let mut tr = El::new("w:tr");
            for c in cells {
                let _ = c.cont;
                tr.push(c.el);
            }
            tbl.push(tr);
        }
        self.doc.body.push(tbl);
        // Word merges adjacent tables; keep a paragraph between blocks.
        self.push_para(&ParaOpts::default(), vec![])
    }
}

fn checkbox(checked: bool) -> El {
    let state =
        |name: &'static str, v: &str| El::new(name).a("w14:val", v).a("w14:font", "MS Gothic");
    El::new("w:sdt")
        .c(El::new("w:sdtPr").c(El::new("w14:checkbox")
            .c(El::new("w14:checked").a("w14:val", if checked { "1" } else { "0" }))
            .c(state("w14:checkedState", "2612"))
            .c(state("w14:uncheckedState", "2610"))))
        .c(El::new("w:sdtContent").c(El::new("w:r").c(El::new("w:sym")
            .a("w:char", if checked { "2612" } else { "2610" })
            .a("w:font", "MS Gothic"))))
}

fn esc(s: &str) -> String {
    quick_xml::escape::escape(s).into_owned()
}

fn rels(entries: &[(String, &str, String, bool)]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (id, ty, target, external) in entries {
        s.push_str(&format!(
            r#"<Relationship Id="{}" Type="{}" Target="{}"{}/>"#,
            esc(id),
            ty,
            esc(target),
            if *external {
                r#" TargetMode="External""#
            } else {
                ""
            }
        ));
    }
    s.push_str("</Relationships>");
    s
}

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The `docx` package's `xml`-escaped `<w:rFonts>` for a font ('' = none).
fn rfonts(font: &str) -> String {
    if font.is_empty() {
        return String::new();
    }
    El::new("w:rFonts")
        .a("w:ascii", font)
        .a("w:cs", font)
        .a("w:eastAsia", font)
        .a("w:hAnsi", font)
        .to_xml()
}

/// `exportDocx(root, meta, deps)` as a package of parts. `now` stamps docProps/core.xml.
pub async fn docx_package(
    root: &PmNode,
    meta: &DocxMeta,
    deps: &mut dyn DocxDeps,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<DocxPackage, DocxError> {
    let font = meta.font.clone().unwrap_or_else(|| doc_default_font(root));
    fn find(n: &PmNode) -> bool {
        n.kind == "tableOfContents" || children(n).iter().any(find)
    }
    let has_toc = find(root);
    let mut headings = Vec::new();
    if has_toc {
        fn walk(n: &PmNode, out: &mut Vec<(f64, String, String)>) {
            if n.kind == "heading" {
                let id = format!("oc_h{}", out.len() + 1);
                out.push((
                    attr_or(n, "level").map_or(1.0, js::value_to_number),
                    node_text(n),
                    id,
                ));
            } else {
                for c in children(n) {
                    walk(c, out);
                }
            }
        }
        walk(root, &mut headings);
    }
    let mut w = Writer {
        deps,
        doc: Doc {
            body: Vec::new(),
            links: Vec::new(),
            media: Vec::new(),
            numbering: Vec::new(),
        },
        number_instance: 0,
        heading_no: 0,
        has_toc,
        headings,
    };
    for n in children(root) {
        w.block(n, None).await?;
    }
    let mut doc = w.doc;
    if doc.body.is_empty() {
        doc.body.push(El::new("w:p"));
    }

    let mut document = String::from(parts::DOCUMENT_START);
    for b in &doc.body {
        b.write(&mut document);
    }
    document.push_str(parts::DOCUMENT_END);

    let mut entries: Vec<(String, &str, String, bool)> = vec![
        (
            "rId1".into(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles",
            "styles.xml".into(),
            false,
        ),
        (
            "rId2".into(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering",
            "numbering.xml".into(),
            false,
        ),
        (
            "rId3".into(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes",
            "footnotes.xml".into(),
            false,
        ),
        (
            "rId4".into(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes",
            "endnotes.xml".into(),
            false,
        ),
        (
            "rId5".into(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings",
            "settings.xml".into(),
            false,
        ),
        (
            "rId6".into(),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments",
            "comments.xml".into(),
            false,
        ),
    ];
    let link_ty = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    for (i, url) in doc.links.iter().enumerate() {
        entries.push((format!("rIdl{}", i + 1), link_ty, url.clone(), true));
    }
    for (i, (name, _)) in doc.media.iter().enumerate() {
        entries.push((
            format!("rId{}", 7 + i),
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
            format!("media/{name}"),
            false,
        ));
    }
    entries.push((
        format!("rId{}", entries.len() + 1),
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable",
        "fontTable.xml".into(),
        false,
    ));
    let _ = REL;

    let rf = rfonts(&font);
    let styles = parts::STYLES
        .iter()
        .enumerate()
        .fold(String::new(), |mut s, (i, p)| {
            if i > 0 {
                s.push_str(&rf);
            }
            s.push_str(p);
            s
        });

    let mut numbering = String::from(parts::NUMBERING_START);
    let num = |id: usize, abs: usize| {
        format!(
            r#"<w:num w:numId="{id}"><w:abstractNumId w:val="{abs}"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="1"/></w:lvlOverride></w:num>"#
        )
    };
    numbering.push_str(&num(1, 1));
    for (i, (reference, _)) in doc.numbering.iter().enumerate() {
        numbering.push_str(&num(i + 2, if *reference == "oc-bullet" { 2 } else { 3 }));
    }
    numbering.push_str("</w:numbering>");

    let stamp = now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let core = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>{}</dc:title><dc:creator>Pinselo</dc:creator><cp:lastModifiedBy>Un-named</cp:lastModifiedBy><cp:revision>1</cp:revision><dcterms:created xsi:type="dcterms:W3CDTF">{stamp}</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">{stamp}</dcterms:modified></cp:coreProperties>"#,
        esc(&meta.title)
    );

    let s = |x: &str| x.as_bytes().to_vec();
    let mut parts_out: Vec<(String, Vec<u8>)> = vec![
        (
            "word/_rels/document.xml.rels".into(),
            rels(&entries).into_bytes(),
        ),
        ("word/document.xml".into(), document.into_bytes()),
        ("word/styles.xml".into(), styles.into_bytes()),
        ("docProps/core.xml".into(), core.into_bytes()),
        ("word/numbering.xml".into(), numbering.into_bytes()),
        ("_rels/.rels".into(), s(parts::PACKAGE_RELS)),
        ("[Content_Types].xml".into(), s(parts::CONTENT_TYPES)),
        ("docProps/custom.xml".into(), s(parts::CUSTOM)),
        ("docProps/app.xml".into(), s(parts::APP)),
        ("word/footnotes.xml".into(), s(parts::FOOTNOTES)),
        ("word/_rels/footnotes.xml.rels".into(), s(parts::EMPTY_RELS)),
        ("word/endnotes.xml".into(), s(parts::ENDNOTES)),
        ("word/_rels/endnotes.xml.rels".into(), s(parts::EMPTY_RELS)),
        ("word/settings.xml".into(), s(parts::SETTINGS)),
        ("word/comments.xml".into(), s(parts::COMMENTS)),
        ("word/_rels/comments.xml.rels".into(), s(parts::EMPTY_RELS)),
        ("word/fontTable.xml".into(), s(parts::FONT_TABLE)),
        ("word/_rels/fontTable.xml.rels".into(), s(parts::EMPTY_RELS)),
    ];
    for (name, data) in doc.media {
        parts_out.push((format!("word/media/{name}"), data));
    }
    Ok(DocxPackage { parts: parts_out })
}

/// `exportDocx(root, meta, deps)`: the .docx bytes.
pub async fn export_docx(
    root: &PmNode,
    meta: &DocxMeta,
    deps: &mut dyn DocxDeps,
) -> Result<Vec<u8>, DocxError> {
    docx_package(root, meta, deps, chrono::Utc::now())
        .await?
        .to_zip()
}

/// The asset id in an `/api/assets/:id` URL.
pub fn asset_id_of(src: &str) -> Option<&str> {
    ASSET
        .captures(src)
        .and_then(|m| m.get(1))
        .map(|m| m.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn colours() {
        assert_eq!(
            hex_of(Some(&json!("rgb(12, 200, 255)"))).as_deref(),
            Some("0CC8FF")
        );
        assert_eq!(hex_of(Some(&json!("#abc"))).as_deref(), Some("AABBCC"));
        assert_eq!(hex_of(Some(&json!("#abcde"))).as_deref(), Some("AABBCC"));
        assert_eq!(hex_of(Some(&json!("#abcd"))), None);
        assert_eq!(hex_of(Some(&json!(" #123456 "))).as_deref(), Some("123456"));
        assert_eq!(
            hex_of(Some(&json!("rgb(300, 0, 0)"))).as_deref(),
            Some("12C0000")
        );
        assert!(hex_color("12C0000").is_err());
        assert_eq!(
            family_of(Some(&json!("\"Playfair Display\", serif"))).as_deref(),
            Some("Playfair Display")
        );
        assert_eq!(family_of(Some(&json!(",x"))).as_deref(), Some(""));
    }
}
