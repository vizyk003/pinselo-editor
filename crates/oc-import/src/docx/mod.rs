// SPDX-License-Identifier: Apache-2.0
//! Word → HTML: a port of mammoth 1.12.3 (`mammoth.convertToHtml`) with the options of
//! `POST /api/docs/import-docx` (services/api/src/routes/docs.ts): `externalFileAccess: false`, the style map
//! `u => u`, `strike => s`, `p[style-name='Quote'] => blockquote > p:fresh`,
//! `p[style-name='Intense Quote'] => blockquote > p:fresh` before mammoth's default style map, and
//! `images.imgElement` (alt text from the drawing description) with the caller answering each picture's `src`.
//!
//! Structure follows mammoth: read the package (relationships, content types, styles, numbering, footnotes,
//! endnotes, comments, the main document) into a document tree (docx/body-reader.js), convert it to an HTML tree
//! with the style map (document-to-html.js), simplify (merge non-fresh elements, drop empty ones) and write.
//! Messages are mammoth's (`"warning: …"` / `"error: …"`), de-duplicated, in mammoth's order. Where mammoth throws
//! (malformed XML, a missing main document, references to missing relationships or notes, …) the conversion fails.
//!
//! Not ported: a style map embedded in the package by `mammoth.embedStyleMap` (`mammoth/style-map`), RTL/bidi
//! concerns (mammoth has none), and the image cap of the route (100 pictures; the route applies it).

mod dingbats;
mod xml;

use std::collections::HashMap;
use std::io::Cursor;

use crate::common::{ImportError, MAX_PART_BYTES, MAX_XML_PART_BYTES, read_limited};
use xml::{XElement, XNode};

/// An image found in the document, handed to the caller's ingest step. `content_type` is the package's content
/// type for the part (`""` when it has none, mammoth's `null`).
pub struct DocxImage<'a> {
    pub content_type: &'a str,
    pub bytes: &'a [u8],
}

/// `mammoth.convertToHtml` result: HTML and the messages (`"warning: …"` / `"error: …"`).
pub struct DocxHtml {
    pub html: String,
    pub messages: Vec<String>,
}

type Res<T> = Result<T, String>;

/// Convert with the route's options; `image` answers the `src` for each picture (called once per picture, in
/// document order, only for pictures whose bytes could be read).
pub fn docx_to_html(bytes: &[u8], image: &mut dyn FnMut(&DocxImage<'_>) -> String) -> Result<DocxHtml, ImportError> {
    convert(bytes, image).map_err(ImportError::Invalid)
}

// ================================================================ package

struct Package {
    zip: zip::ZipArchive<Cursor<Vec<u8>>>,
}

impl Package {
    fn exists(&mut self, name: &str) -> bool {
        self.zip.by_name(name).map(|f| !f.is_dir()).unwrap_or(false)
    }

    fn read(&mut self, name: &str) -> Res<Option<Vec<u8>>> {
        self.read_limited(name, MAX_PART_BYTES)
    }

    fn read_limited(&mut self, name: &str, limit: u64) -> Res<Option<Vec<u8>>> {
        let f = match self.zip.by_name(name) {
            Ok(f) if !f.is_dir() => f,
            _ => return Ok(None),
        };
        read_limited(f, limit).map(Some).map_err(|e| e.to_string())
    }

    fn xml(&mut self, name: &str) -> Res<Option<XElement>> {
        match self.read_limited(name, MAX_XML_PART_BYTES)? {
            Some(b) => xml::parse(&b).map(Some),
            None => Ok(None),
        }
    }
}

fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    }
}

/// zipfile.joinPath: empty parts dropped, an absolute part restarts the path.
fn join_path(parts: &[&str]) -> String {
    let mut relevant: Vec<&str> = Vec::new();
    for p in parts.iter().filter(|p| !p.is_empty()) {
        if p.starts_with('/') {
            relevant = vec![p];
        } else {
            relevant.push(p);
        }
    }
    relevant.join("/")
}

fn rels_filename(name: &str) -> String {
    let (dir, base) = split_path(name);
    join_path(&[dir, "_rels", &format!("{base}.rels")])
}

#[derive(Default)]
struct Rels {
    by_id: HashMap<String, String>,
    by_type: Vec<(String, String)>,
}

impl Rels {
    fn read(el: &XElement) -> Rels {
        let mut r = Rels::default();
        for c in el.elements("relationships:Relationship") {
            let id = c.attr("Id").unwrap_or("undefined").to_owned();
            if let Some(t) = c.attr("Target") {
                r.by_id.insert(id, t.to_owned());
            } else {
                r.by_id.remove(&id);
            }
            if let (Some(ty), Some(t)) = (c.attr("Type"), c.attr("Target")) {
                r.by_type.push((ty.to_owned(), t.to_owned()));
            }
        }
        r
    }

    fn target(&self, id: &str) -> Option<&str> {
        self.by_id.get(id).map(String::as_str)
    }

    fn targets_by_type<'a>(&'a self, ty: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.by_type.iter().filter(move |(t, _)| t == ty).map(|(_, v)| v.as_str())
    }
}

fn read_rels(pkg: &mut Package, name: &str) -> Res<Rels> {
    Ok(pkg.xml(name)?.map(|e| Rels::read(&e)).unwrap_or_default())
}

#[derive(Default)]
struct ContentTypes {
    overrides: HashMap<String, String>,
    defaults: HashMap<String, String>,
}

impl ContentTypes {
    fn read(el: &XElement) -> Res<ContentTypes> {
        let mut c = ContentTypes::default();
        for child in &el.children {
            let XNode::Element(child) = child else { continue };
            if child.name == "content-types:Default" {
                c.defaults.insert(child.attr("Extension").unwrap_or("undefined").to_owned(), child.attr("ContentType").unwrap_or("undefined").to_owned());
            }
            if child.name == "content-types:Override" {
                let name = child.attr("PartName").ok_or("Cannot read properties of undefined (reading 'charAt')")?;
                let name = name.strip_prefix('/').unwrap_or(name);
                c.overrides.insert(name.to_owned(), child.attr("ContentType").unwrap_or("undefined").to_owned());
            }
        }
        Ok(c)
    }

    fn find(&self, path: &str) -> Option<String> {
        if let Some(o) = self.overrides.get(path).filter(|o| !o.is_empty()) {
            return Some(o.clone());
        }
        let ext = path.rsplit('.').next().unwrap_or("");
        if let Some(d) = self.defaults.get(ext) {
            return Some(d.clone());
        }
        let fallback = match ext.to_lowercase().as_str() {
            "png" => "png",
            "gif" => "gif",
            "jpeg" | "jpg" => "jpeg",
            "tif" | "tiff" => "tiff",
            "bmp" => "bmp",
            _ => return None,
        };
        Some(format!("image/{fallback}"))
    }
}

// ---------------------------------------------------------------- JS object helpers

/// A JS object with string keys: insertion order, except integer-like keys first in ascending order.
struct JsObj<V> {
    entries: Vec<(String, V)>,
}

impl<V> Default for JsObj<V> {
    fn default() -> Self {
        JsObj { entries: Vec::new() }
    }
}

fn array_index(k: &str) -> Option<u64> {
    if k == "0" || (!k.is_empty() && !k.starts_with('0') && k.bytes().all(|b| b.is_ascii_digit()) && k.len() <= 10) {
        k.parse::<u64>().ok().filter(|n| *n < 4_294_967_295)
    } else {
        None
    }
}

impl<V> JsObj<V> {
    fn set(&mut self, k: String, v: V) {
        if let Some(e) = self.entries.iter_mut().find(|(x, _)| *x == k) {
            e.1 = v;
        } else {
            self.entries.push((k, v));
        }
    }

    fn get(&self, k: &str) -> Option<&V> {
        self.entries.iter().find(|(x, _)| x == k).map(|(_, v)| v)
    }

    fn contains(&self, k: &str) -> bool {
        self.get(k).is_some()
    }

    fn values(&self) -> Vec<&V> {
        let mut ints: Vec<(u64, &V)> = self.entries.iter().filter_map(|(k, v)| array_index(k).map(|i| (i, v))).collect();
        ints.sort_by_key(|(i, _)| *i);
        let mut out: Vec<&V> = ints.into_iter().map(|(_, v)| v).collect();
        out.extend(self.entries.iter().filter(|(k, _)| array_index(k).is_none()).map(|(_, v)| v));
        out
    }
}

fn key(v: Option<&str>) -> String {
    v.unwrap_or("undefined").to_owned()
}

// ---------------------------------------------------------------- styles and numbering

#[derive(Clone)]
struct Style {
    name: Option<String>,
}

#[derive(Default)]
struct Styles {
    paragraph: HashMap<String, Style>,
    character: HashMap<String, Style>,
    table: HashMap<String, Style>,
    numbering: HashMap<String, Option<String>>,
}

impl Styles {
    fn read(root: &XElement) -> Styles {
        let mut s = Styles::default();
        for el in root.elements("w:style") {
            let id = key(el.attr("w:styleId"));
            let ty = el.attr("w:type");
            if ty == Some("numbering") {
                let num_id = el.first_or_empty("w:pPr").first_or_empty("w:numPr").first_or_empty("w:numId").attr("w:val").map(str::to_owned);
                s.numbering.entry(id).or_insert(num_id);
                continue;
            }
            let style = Style { name: el.first("w:name").and_then(|n| n.attr("w:val")).map(str::to_owned) };
            let set = match ty {
                Some("paragraph") => &mut s.paragraph,
                Some("character") => &mut s.character,
                Some("table") => &mut s.table,
                _ => continue,
            };
            set.entry(id).or_insert(style);
        }
        s
    }
}

#[derive(Clone, Debug)]
struct Level {
    is_ordered: bool,
    level: String,
}

struct AbstractNum {
    levels: JsObj<(Level, Option<String>)>,
    num_style_link: Option<String>,
}

#[derive(Default)]
struct Numbering {
    nums: JsObj<Option<String>>,
    abstract_nums: JsObj<AbstractNum>,
    by_paragraph_style: HashMap<String, Level>,
}

impl Numbering {
    fn read(root: &XElement) -> Res<Numbering> {
        let mut n = Numbering::default();
        for el in root.elements("w:abstractNum") {
            let mut levels: JsObj<(Level, Option<String>)> = JsObj::default();
            let mut without_index: Option<(Level, Option<String>)> = None;
            for lvl in el.elements("w:lvl") {
                let is_ordered = lvl.first_or_empty("w:numFmt").attr("w:val") != Some("bullet");
                let pstyle = lvl.first_or_empty("w:pStyle").attr("w:val").map(str::to_owned);
                match lvl.attr("w:ilvl") {
                    None => without_index = Some((Level { is_ordered, level: "0".into() }, pstyle)),
                    Some(i) => levels.set(i.to_owned(), (Level { is_ordered, level: i.to_owned() }, pstyle)),
                }
            }
            if let Some(l) = without_index
                && !levels.contains("0")
            {
                levels.set("0".into(), l);
            }
            let link = el.first_or_empty("w:numStyleLink").attr("w:val").map(str::to_owned);
            n.abstract_nums.set(key(el.attr("w:abstractNumId")), AbstractNum { levels, num_style_link: link });
        }
        for el in root.elements("w:num") {
            let abs = el.first("w:abstractNumId").ok_or("Cannot read properties of undefined (reading 'attributes')")?;
            n.nums.set(key(el.attr("w:numId")), abs.attr("w:val").map(str::to_owned));
        }
        for a in n.abstract_nums.values() {
            for (level, pstyle) in a.levels.values() {
                if let Some(p) = pstyle {
                    n.by_paragraph_style.insert(p.clone(), level.clone());
                }
            }
        }
        Ok(n)
    }

    fn find_level(&self, num_id: Option<&str>, level: &str, styles: &Styles) -> Res<Option<Level>> {
        let mut seen: Vec<String> = Vec::new();
        let mut num_id = key(num_id);
        loop {
            if seen.contains(&num_id) {
                return Ok(None);
            }
            seen.push(num_id.clone());
            let Some(num) = self.nums.get(&num_id) else { return Ok(None) };
            let Some(abs) = self.abstract_nums.get(&key(num.as_deref())) else { return Ok(None) };
            match &abs.num_style_link {
                None => return Ok(abs.levels.get(level).map(|(l, _)| l.clone())),
                Some(link) => {
                    let style = styles.numbering.get(link).ok_or("Cannot read properties of undefined (reading 'numId')")?;
                    num_id = key(style.as_deref());
                }
            }
        }
    }
}

// ================================================================ document model

#[derive(Clone, Debug)]
struct Para {
    children: Vec<D>,
    style_id: Option<String>,
    style_name: Option<String>,
    numbering: Option<Level>,
}

#[derive(Clone, Debug)]
struct RunEl {
    children: Vec<D>,
    style_id: Option<String>,
    style_name: Option<String>,
    bold: bool,
    underline: bool,
    italic: bool,
    strike: bool,
    all_caps: bool,
    small_caps: bool,
    vertical: String,
}

#[derive(Clone, Debug)]
enum ImgSource {
    /// A part of the package (`uriToZipEntryName('word', target)`).
    Part(String),
    /// A linked file (never read: external file access is disabled).
    External(String),
}

#[derive(Clone, Debug)]
struct Img {
    source: ImgSource,
    alt: Option<String>,
    content_type: Option<String>,
}

#[derive(Clone, Debug)]
enum D {
    Paragraph(Para),
    Run(RunEl),
    Text(String),
    Tab,
    Checkbox(bool),
    Hyperlink { children: Vec<D>, href: Option<String>, anchor: Option<String>, target_frame: Option<String> },
    NoteRef { note_type: &'static str, note_id: Option<String> },
    CommentRef,
    Image(Img),
    Table { children: Vec<D> },
    Row { children: Vec<D>, is_header: bool },
    Cell { children: Vec<D>, col_span: f64, row_span: u32, vmerge: Option<bool> },
    Break(&'static str),
    Bookmark(Option<String>),
}

struct Note {
    note_type: &'static str,
    note_id: Option<String>,
    body: Vec<D>,
}

// ================================================================ body reader

/// `ReadResult`: elements plus "extra" elements hoisted to the enclosing paragraph (`w:pict` contents).
#[derive(Default)]
struct Rr {
    value: Vec<D>,
    extra: Vec<D>,
}

impl Rr {
    fn one(d: D) -> Rr {
        Rr { value: vec![d], extra: vec![] }
    }
}

enum Field {
    Begin(XElement),
    Hyperlink { href: Option<String>, anchor: Option<String> },
    Checkbox(bool),
    Unknown,
}

struct Ctx<'a> {
    rels: &'a Rels,
    content_types: &'a ContentTypes,
    styles: &'a Styles,
    numbering: &'a Numbering,
}

struct BodyReader<'a, 'm> {
    ctx: &'a Ctx<'a>,
    fields: Vec<Field>,
    instr: Vec<String>,
    deleted: Vec<XNode>,
    msgs: &'m mut Vec<String>,
}

fn warn(msgs: &mut Vec<String>, m: impl AsRef<str>) {
    msgs.push(format!("warning: {}", m.as_ref()));
}

const IGNORED: &[&str] = &[
    "office-word:wrap", "v:shadow", "v:shapetype", "w:annotationRef", "w:bookmarkEnd", "w:sectPr", "w:proofErr", "w:lastRenderedPageBreak",
    "w:commentRangeStart", "w:commentRangeEnd", "w:del", "w:footnoteRef", "w:endnoteRef", "w:pPr", "w:rPr", "w:tblPr", "w:tblGrid", "w:trPr",
    "w:tcPr",
];

/// JS `\s`.
fn js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn read_bool_el(el: Option<&XElement>) -> bool {
    el.is_some_and(|e| !matches!(e.attr("w:val"), Some("false") | Some("0")))
}

/// JS `parseInt(s, radix)` (NaN as `None`).
fn js_parse_int(s: Option<&str>, radix: u32) -> Option<f64> {
    let s = s?.trim_start_matches(js_space);
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = if radix == 16 { s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s) } else { s };
    let digits: String = s.chars().take_while(|c| c.is_digit(radix)).collect();
    if digits.is_empty() {
        return None;
    }
    let v = digits.chars().fold(0f64, |acc, c| acc * radix as f64 + c.to_digit(radix).unwrap_or(0) as f64);
    Some(if neg { -v } else { v })
}

/// The `HYPERLINK` field instruction: `/^\s*HYPERLINK\s+(\\l\s+)?(?:"(.*)"|([^\\]\S*))/`.
fn parse_hyperlink_instr(s: &str) -> Option<(Option<String>, Option<String>)> {
    let s = s.trim_start_matches(js_space);
    let s = s.strip_prefix("HYPERLINK")?;
    let rest = s.trim_start_matches(js_space);
    if rest.len() == s.len() {
        return None;
    }
    // Optional `\l` + whitespace (backtracks when not followed by whitespace).
    let (is_anchor, rest) = match rest.strip_prefix("\\l") {
        Some(r) if r.starts_with(js_space) => (true, r.trim_start_matches(js_space)),
        _ => (false, rest),
    };
    let is_line_end = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    let location = if let Some(q) = rest.strip_prefix('"') {
        let line = &q[..q.find(is_line_end).unwrap_or(q.len())];
        line.rfind('"').map(|i| line[..i].to_owned())
    } else {
        None
    };
    let location = match location {
        Some(l) => l,
        None => {
            let mut chars = rest.chars();
            let first = chars.next()?;
            if first == '\\' {
                return None;
            }
            let mut out = first.to_string();
            out.extend(chars.take_while(|c| !js_space(*c)));
            out
        }
    };
    Some(if is_anchor { (None, Some(location)) } else { (Some(location), None) })
}

impl<'a, 'm> BodyReader<'a, 'm> {
    fn new(ctx: &'a Ctx<'a>, msgs: &'m mut Vec<String>) -> Self {
        BodyReader { ctx, fields: vec![], instr: vec![], deleted: vec![], msgs }
    }

    fn read_all(&mut self, nodes: &[XNode]) -> Res<Rr> {
        let mut out = Rr::default();
        for n in nodes {
            let r = self.read(n)?;
            out.value.extend(r.value);
            out.extra.extend(r.extra);
        }
        Ok(out)
    }

    fn children(&mut self, el: &XElement) -> Res<Rr> {
        self.read_all(&el.children)
    }

    fn read_style(&mut self, el: &XElement, tag: &str, kind: &str) -> (Option<String>, Option<String>) {
        let Some(s) = el.first(tag) else { return (None, None) };
        let Some(id) = s.attr("w:val") else { return (None, None) };
        if id.is_empty() {
            return (Some(String::new()), None);
        }
        let set = match kind {
            "Paragraph" => &self.ctx.styles.paragraph,
            "Run" => &self.ctx.styles.character,
            _ => &self.ctx.styles.table,
        };
        match set.get(id) {
            Some(st) => (Some(id.to_owned()), st.name.clone()),
            None => {
                warn(self.msgs, format!("{kind} style with ID {id} was referenced but not defined in the document"));
                (Some(id.to_owned()), None)
            }
        }
    }

    fn numbering_props(&self, style_id: Option<&str>, num_pr: &XElement) -> Res<Option<Level>> {
        let level = num_pr.first_or_empty("w:ilvl").attr("w:val");
        let num_id = num_pr.first_or_empty("w:numId").attr("w:val");
        if let (Some(level), Some(_)) = (level, num_id) {
            return self.ctx.numbering.find_level(num_id, level, self.ctx.styles);
        }
        if let Some(sid) = style_id
            && let Some(l) = self.ctx.numbering.by_paragraph_style.get(sid)
        {
            return Ok(Some(l.clone()));
        }
        if num_id.is_some() {
            return self.ctx.numbering.find_level(num_id, "0", self.ctx.styles);
        }
        Ok(None)
    }

    fn hyperlink_options(&self) -> Option<(Option<String>, Option<String>)> {
        self.fields.iter().rev().find_map(|f| match f {
            Field::Hyperlink { href, anchor } => Some((href.clone(), anchor.clone())),
            _ => None,
        })
    }

    fn parse_instr(&self, fld_char: Option<&XElement>) -> Field {
        let text = self.instr.concat();
        if let Some((href, anchor)) = parse_hyperlink_instr(&text) {
            return Field::Hyperlink { href, anchor };
        }
        if text.contains("FORMCHECKBOX") {
            let empty = XElement::default();
            let cb = fld_char.unwrap_or(&empty).first_or_empty("w:ffData").first_or_empty("w:checkBox");
            let checked = match cb.first("w:checked") {
                Some(c) => read_bool_el(Some(c)),
                None => read_bool_el(cb.first("w:default")),
            };
            return Field::Checkbox(checked);
        }
        Field::Unknown
    }

    fn read_fld_char(&mut self, el: &XElement) -> Res<Rr> {
        match el.attr("w:fldCharType") {
            Some("begin") => {
                self.fields.push(Field::Begin(el.clone()));
                self.instr.clear();
            }
            Some("end") => {
                let Some(f) = self.fields.pop() else {
                    warn(self.msgs, "Ignoring complex field end character without corresponding start character");
                    return Ok(Rr::default());
                };
                let f = match f {
                    Field::Begin(fc) => self.parse_instr(Some(&fc)),
                    other => other,
                };
                if let Field::Checkbox(checked) = f {
                    return Ok(Rr::one(D::Checkbox(checked)));
                }
            }
            Some("separate") => {
                let Some(f) = self.fields.pop() else {
                    warn(self.msgs, "Ignoring complex field separator character without corresponding start character");
                    return Ok(Rr::default());
                };
                let parsed = match &f {
                    Field::Begin(fc) => self.parse_instr(Some(fc)),
                    _ => self.parse_instr(None),
                };
                self.fields.push(parsed);
            }
            _ => {}
        }
        Ok(Rr::default())
    }

    fn read_symbol(&mut self, el: &XElement) -> Res<Rr> {
        let font = el.attr("w:font").ok_or("Cannot read properties of undefined (reading 'toUpperCase')")?;
        let ch = el.attr("w:char");
        let lookup = |code: Option<f64>| -> Option<char> {
            let code = code?;
            let font = font.to_uppercase();
            let (_, table) = dingbats::DINGBATS.iter().find(|(f, _)| *f == font)?;
            let cp = table.iter().find(|(c, _)| *c as f64 == code)?.1;
            char::from_u32(cp)
        };
        let mut u = lookup(js_parse_int(ch, 16));
        if u.is_none()
            && let Some(c) = ch
            && c.starts_with("F0")
            && c.chars().count() == 4
            && !c.chars().any(|x| matches!(x, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
        {
            u = lookup(js_parse_int(Some(&c[2..]), 16));
        }
        match u {
            Some(c) => Ok(Rr::one(D::Text(c.to_string()))),
            None => {
                warn(self.msgs, format!("A w:sym element with an unsupported character was ignored: char {} in font {font}", ch.unwrap_or("undefined")));
                Ok(Rr::default())
            }
        }
    }

    fn read(&mut self, node: &XNode) -> Res<Rr> {
        let XNode::Element(el) = node else { return Ok(Rr::default()) };
        let one = |d| Ok(Rr::one(d));
        match el.name.as_str() {
            "w:p" => self.read_paragraph(el),
            "w:r" => self.read_run(el),
            "w:fldChar" => self.read_fld_char(el),
            "w:instrText" => {
                self.instr.push(el.text()?);
                Ok(Rr::default())
            }
            "w:t" => one(D::Text(el.text()?)),
            "w:tab" => one(D::Tab),
            "w:noBreakHyphen" => one(D::Text("\u{2011}".into())),
            "w:softHyphen" => one(D::Text("\u{00AD}".into())),
            "w:sym" => self.read_symbol(el),
            "w:hyperlink" => self.read_hyperlink(el),
            "w:tbl" => self.read_table(el),
            "w:tr" => {
                let props = el.first_or_empty("w:trPr");
                if props.first("w:del").is_some() {
                    return Ok(Rr::default());
                }
                let is_header = props.first("w:tblHeader").is_some();
                let r = self.children(el)?;
                Ok(Rr { value: vec![D::Row { children: r.value, is_header }], extra: r.extra })
            }
            "w:tc" => {
                let r = self.children(el)?;
                let props = el.first_or_empty("w:tcPr");
                let col_span = match props.first_or_empty("w:gridSpan").attr("w:val") {
                    Some(g) if !g.is_empty() => js_parse_int(Some(g), 10).unwrap_or(f64::NAN),
                    _ => 1.0,
                };
                let vmerge = props.first("w:vMerge").map(|v| match v.attr("w:val") {
                    Some(val) => val == "continue" || val.is_empty(),
                    None => true,
                });
                Ok(Rr { value: vec![D::Cell { children: r.value, col_span, row_span: 1, vmerge }], extra: r.extra })
            }
            "w:footnoteReference" => one(D::NoteRef { note_type: "footnote", note_id: el.attr("w:id").map(str::to_owned) }),
            "w:endnoteReference" => one(D::NoteRef { note_type: "endnote", note_id: el.attr("w:id").map(str::to_owned) }),
            "w:commentReference" => one(D::CommentRef),
            "w:br" => match el.attr("w:type") {
                None | Some("textWrapping") => one(D::Break("line")),
                Some("page") => one(D::Break("page")),
                Some("column") => one(D::Break("column")),
                Some(t) => {
                    warn(self.msgs, format!("Unsupported break type: {t}"));
                    Ok(Rr::default())
                }
            },
            "w:bookmarkStart" => {
                let name = el.attr("w:name");
                if name == Some("_GoBack") { Ok(Rr::default()) } else { one(D::Bookmark(name.map(str::to_owned))) }
            }
            "mc:AlternateContent" => {
                let fb = el.first_or_empty("mc:Fallback").clone();
                self.children(&fb)
            }
            "w:sdt" => self.read_sdt(el),
            "w:ins" | "w:object" | "w:smartTag" | "w:drawing" | "v:roundrect" | "v:shape" | "v:textbox" | "w:txbxContent" | "v:group"
            | "v:rect" => self.children(el),
            "w:pict" => {
                let r = self.children(el)?;
                let mut extra = r.extra;
                extra.extend(r.value);
                Ok(Rr { value: vec![], extra })
            }
            "wp:inline" | "wp:anchor" => self.read_drawing(el),
            "v:imagedata" => match el.attr("r:id").filter(|r| !r.is_empty()) {
                Some(rid) => {
                    let file = self.embedded_image(rid)?;
                    self.read_image(file, None)
                }
                None => {
                    warn(self.msgs, "A v:imagedata element without a relationship ID was ignored");
                    Ok(Rr::default())
                }
            },
            name if IGNORED.contains(&name) => Ok(Rr::default()),
            name => {
                warn(self.msgs, format!("An unrecognised element was ignored: {name}"));
                Ok(Rr::default())
            }
        }
    }

    fn read_paragraph(&mut self, el: &XElement) -> Res<Rr> {
        let ppr = el.first_or_empty("w:pPr");
        if ppr.first_or_empty("w:rPr").first("w:del").is_some() {
            self.deleted.extend(el.children.iter().cloned());
            return Ok(Rr::default());
        }
        let mut nodes = std::mem::take(&mut self.deleted);
        nodes.extend(el.children.iter().cloned());
        let (style_id, style_name) = self.read_style(ppr, "w:pStyle", "Paragraph");
        let numbering = self.numbering_props(style_id.as_deref(), ppr.first_or_empty("w:numPr"))?;
        let r = self.read_all(&nodes)?;
        let p = D::Paragraph(Para { children: r.value, style_id: style_id.filter(|s| !s.is_empty()), style_name: style_name.filter(|s| !s.is_empty()), numbering });
        let mut value = vec![p];
        value.extend(r.extra);
        Ok(Rr { value, extra: vec![] })
    }

    fn read_run(&mut self, el: &XElement) -> Res<Rr> {
        let rpr = el.first_or_empty("w:rPr");
        let (style_id, style_name) = self.read_style(rpr, "w:rStyle", "Run");
        let underline = rpr.first("w:u").is_some_and(|u| !matches!(u.attr("w:val"), None | Some("false") | Some("0") | Some("none")));
        let vertical = rpr.first_or_empty("w:vertAlign").attr("w:val").filter(|v| !v.is_empty()).unwrap_or("baseline").to_owned();
        let props = (
            read_bool_el(rpr.first("w:b")),
            read_bool_el(rpr.first("w:i")),
            read_bool_el(rpr.first("w:strike")),
            read_bool_el(rpr.first("w:caps")),
            read_bool_el(rpr.first("w:smallCaps")),
        );
        let r = self.children(el)?;
        let mut children = r.value;
        if let Some((href, anchor)) = self.hyperlink_options() {
            children = vec![D::Hyperlink { children, href, anchor, target_frame: None }];
        }
        Ok(Rr {
            value: vec![D::Run(RunEl {
                children,
                style_id: style_id.filter(|s| !s.is_empty()),
                style_name: style_name.filter(|s| !s.is_empty()),
                bold: props.0,
                underline,
                italic: props.1,
                strike: props.2,
                all_caps: props.3,
                small_caps: props.4,
                vertical,
            })],
            extra: r.extra,
        })
    }

    fn read_hyperlink(&mut self, el: &XElement) -> Res<Rr> {
        let rid = el.attr("r:id").filter(|s| !s.is_empty());
        let anchor = el.attr("w:anchor").filter(|s| !s.is_empty());
        let r = self.children(el)?;
        let target_frame = el.attr("w:tgtFrame").filter(|s| !s.is_empty()).map(str::to_owned);
        let value = if let Some(rid) = rid {
            let mut href = self.ctx.rels.target(rid).map(str::to_owned);
            if let Some(a) = anchor {
                let h = href.ok_or("Cannot read properties of undefined (reading 'indexOf')")?;
                let base = h.find('#').map_or(h.as_str(), |i| &h[..i]);
                href = Some(format!("{base}#{a}"));
            }
            vec![D::Hyperlink { children: r.value, href, anchor: None, target_frame }]
        } else if let Some(a) = anchor {
            vec![D::Hyperlink { children: r.value, href: None, anchor: Some(a.to_owned()), target_frame }]
        } else {
            r.value
        };
        Ok(Rr { value, extra: r.extra })
    }

    fn read_sdt(&mut self, el: &XElement) -> Res<Rr> {
        let r = self.read_all(&el.first_or_empty("w:sdtContent").children)?;
        let Some(cb) = el.first_or_empty("w:sdtPr").first("wordml:checkbox") else { return Ok(r) };
        let checked = cb.first("wordml:checked").is_some_and(|c| !matches!(c.attr("wordml:val"), Some("false") | Some("0")));
        let mut replaced = false;
        fn walk(d: &mut D, checked: bool, replaced: &mut bool) {
            match d {
                D::Paragraph(p) => p.children.iter_mut().for_each(|c| walk(c, checked, replaced)),
                D::Run(r) => r.children.iter_mut().for_each(|c| walk(c, checked, replaced)),
                D::Hyperlink { children, .. } | D::Table { children, .. } | D::Row { children, .. } | D::Cell { children, .. } => {
                    children.iter_mut().for_each(|c| walk(c, checked, replaced));
                }
                D::Text(t) if !t.is_empty() && !*replaced => {
                    *replaced = true;
                    *d = D::Checkbox(checked);
                }
                _ => {}
            }
        }
        let mut value = r.value;
        for d in &mut value {
            walk(d, checked, &mut replaced);
        }
        Ok(Rr { value: if replaced { value } else { vec![D::Checkbox(checked)] }, extra: r.extra })
    }

    fn read_table(&mut self, el: &XElement) -> Res<Rr> {
        // The table style's messages come after the rows' (mammoth combines them last).
        let mut style_msgs = Vec::new();
        std::mem::swap(self.msgs, &mut style_msgs);
        // (No table style map entries: the style only matters for its warning.)
        self.read_style(el.first_or_empty("w:tblPr"), "w:tblStyle", "Table");
        std::mem::swap(self.msgs, &mut style_msgs);
        let r = self.children(el)?;
        let mut rows = r.value;
        calculate_row_spans(&mut rows, self.msgs);
        self.msgs.extend(style_msgs);
        Ok(Rr {
            value: vec![D::Table { children: rows }],
            extra: r.extra,
        })
    }

    fn read_drawing(&mut self, el: &XElement) -> Res<Rr> {
        let mut blips: Vec<&XElement> = Vec::new();
        for g in el.elements("a:graphic") {
            for gd in g.elements("a:graphicData") {
                for p in gd.elements("pic:pic") {
                    for bf in p.elements("pic:blipFill") {
                        blips.extend(bf.elements("a:blip"));
                    }
                }
            }
        }
        let mut out = Rr::default();
        for blip in blips {
            let doc_pr = el.first_or_empty("wp:docPr");
            let descr = doc_pr.attr("descr");
            let alt = if descr.is_none_or(|d| d.chars().all(js_space)) { doc_pr.attr("title") } else { descr };
            let file = if let Some(rid) = blip.attr("r:embed").filter(|s| !s.is_empty()) {
                self.embedded_image(rid)?
            } else if let Some(rid) = blip.attr("r:link").filter(|s| !s.is_empty()) {
                ImgSource::External(self.ctx.rels.target(rid).ok_or("Cannot read properties of undefined (reading 'split')")?.to_owned())
            } else {
                warn(self.msgs, "Could not find image file for a:blip element");
                continue;
            };
            let mut r = self.read_image(file, alt.map(str::to_owned))?;
            if let Some(rid) = doc_pr.first_or_empty("a:hlinkClick").attr("r:id").filter(|s| !s.is_empty()) {
                let href = self.ctx.rels.target(rid).map(str::to_owned);
                r.value = vec![D::Hyperlink { children: r.value, href, anchor: None, target_frame: None }];
            }
            out.value.extend(r.value);
        }
        Ok(out)
    }

    fn embedded_image(&self, rid: &str) -> Res<ImgSource> {
        let target = self.ctx.rels.target(rid).ok_or("Cannot read properties of undefined (reading 'charAt')")?;
        Ok(ImgSource::Part(match target.strip_prefix('/') {
            Some(t) => t.to_owned(),
            None => format!("word/{target}"),
        }))
    }

    fn read_image(&mut self, source: ImgSource, alt: Option<String>) -> Res<Rr> {
        let path = match &source {
            ImgSource::Part(p) | ImgSource::External(p) => p,
        };
        let content_type = self.ctx.content_types.find(path);
        if !matches!(content_type.as_deref(), Some("image/png" | "image/gif" | "image/jpeg" | "image/svg+xml" | "image/tiff")) {
            warn(self.msgs, format!("Image of type {} is unlikely to display in web browsers", content_type.as_deref().unwrap_or("null")));
        }
        Ok(Rr::one(D::Image(Img { source, alt, content_type })))
    }
}

/// `calculateRowSpans`: vertically merged cells become a row span of the cell above.
fn calculate_row_spans(rows: &mut [D], msgs: &mut Vec<String>) {
    fn clear(d: &mut D) {
        match d {
            D::Cell { vmerge, children, .. } => {
                *vmerge = None;
                children.iter_mut().for_each(clear);
            }
            D::Paragraph(p) => p.children.iter_mut().for_each(clear),
            D::Run(r) => r.children.iter_mut().for_each(clear),
            D::Hyperlink { children, .. } | D::Table { children, .. } | D::Row { children, .. } => children.iter_mut().for_each(clear),
            _ => {}
        }
    }
    if rows.iter().any(|r| !matches!(r, D::Row { .. })) {
        rows.iter_mut().for_each(clear);
        warn(msgs, "unexpected non-row element in table, cell merging may be incorrect");
        return;
    }
    if rows.iter().any(|r| matches!(r, D::Row { children, .. } if children.iter().any(|c| !matches!(c, D::Cell { .. })))) {
        rows.iter_mut().for_each(clear);
        warn(msgs, "unexpected non-cell element in table row, cell merging may be incorrect");
        return;
    }
    let num_key = |n: f64| if n.is_nan() { "NaN".to_owned() } else { js_num(n) };
    let mut columns: HashMap<String, (usize, usize)> = HashMap::new();
    for ri in 0..rows.len() {
        let D::Row { children, .. } = &rows[ri] else { continue };
        let n = children.len();
        let mut index = 0f64;
        for ci in 0..n {
            let (vm, span) = match &rows[ri] {
                D::Row { children, .. } => match &children[ci] {
                    D::Cell { vmerge, col_span, .. } => (*vmerge, *col_span),
                    _ => (None, 1.0),
                },
                _ => (None, 1.0),
            };
            let k = num_key(index);
            if vm == Some(true)
                && let Some(&(ar, ac)) = columns.get(&k)
            {
                if let D::Row { children, .. } = &mut rows[ar]
                    && let D::Cell { row_span, .. } = &mut children[ac]
                {
                    *row_span += 1;
                }
            } else {
                columns.insert(k, (ri, ci));
                if let D::Row { children, .. } = &mut rows[ri]
                    && let D::Cell { vmerge, .. } = &mut children[ci]
                {
                    *vmerge = Some(false);
                }
            }
            index += span;
        }
    }
    for r in rows.iter_mut() {
        if let D::Row { children, .. } = r {
            children.retain(|c| !matches!(c, D::Cell { vmerge: Some(true), .. }));
            for c in children.iter_mut() {
                if let D::Cell { vmerge, .. } = c {
                    *vmerge = None;
                }
            }
        }
    }
}

/// JS number → string for the integers this module formats.
fn js_num(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

// ================================================================ style map

#[derive(Clone)]
enum NameOp {
    Equal(String),
}

#[derive(Clone)]
enum Matcher {
    Paragraph { style_id: Option<&'static str>, style_name: Option<NameOp>, list: Option<(usize, bool)> },
    Run { style_name: Option<NameOp> },
    Underline,
    Strikethrough,
}

#[derive(Clone, Debug)]
struct Tag {
    name: String,
    names: Vec<String>,
    attrs: Vec<(String, Option<String>)>,
    fresh: bool,
}

impl Tag {
    fn new(name: &str, fresh: bool) -> Tag {
        Tag { name: name.into(), names: vec![name.into()], attrs: vec![], fresh }
    }

    fn with_attrs(name: &str, fresh: bool, attrs: Vec<(&str, Option<String>)>) -> Tag {
        Tag { attrs: attrs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(), ..Tag::new(name, fresh) }
    }

    /// `matchesElement`: `this` (the later element) accepts the earlier element's tag name and attributes.
    fn matches(&self, other: &Tag) -> bool {
        self.names.contains(&other.name)
            && self.attrs.len() == other.attrs.len()
            && self.attrs.iter().all(|(k, v)| other.attrs.iter().any(|(k2, v2)| k == k2 && v == v2))
    }
}

#[derive(Clone)]
struct HtmlPath(Vec<Tag>);

/// `ul|ol > li:fresh`-style paths (tag choices, `:fresh`; no attributes in the maps used here).
fn path(s: &str) -> HtmlPath {
    if s.is_empty() {
        return HtmlPath(vec![]);
    }
    HtmlPath(
        s.split(" > ")
            .map(|el| {
                let (names, fresh) = match el.strip_suffix(":fresh") {
                    Some(n) => (n, true),
                    None => (el, false),
                };
                let names: Vec<String> = names.split('|').map(str::to_owned).collect();
                Tag { name: names[0].clone(), names, attrs: vec![], fresh }
            })
            .collect(),
    )
}

impl HtmlPath {
    fn wrap(&self, mut nodes: Vec<H>) -> Vec<H> {
        for t in self.0.iter().rev() {
            nodes = vec![H::El(t.clone(), nodes)];
        }
        nodes
    }
}

struct Mapping {
    from: Matcher,
    to: HtmlPath,
}

fn style_map() -> Vec<Mapping> {
    let eq = |s: &str| Some(NameOp::Equal(s.to_owned()));
    let p_name = |n: &str, to: &str| Mapping { from: Matcher::Paragraph { style_id: None, style_name: eq(n), list: None }, to: path(to) };
    let p_id = |id: &'static str, to: &str| Mapping { from: Matcher::Paragraph { style_id: Some(id), style_name: None, list: None }, to: path(to) };
    let r_name = |n: &str, to: &str| Mapping { from: Matcher::Run { style_name: eq(n) }, to: path(to) };
    let list = |level: usize, ordered: bool, to: &str| Mapping { from: Matcher::Paragraph { style_id: None, style_name: None, list: Some((level, ordered)) }, to: path(to) };
    let mut m = vec![
        // The route's style map.
        Mapping { from: Matcher::Underline, to: path("u") },
        Mapping { from: Matcher::Strikethrough, to: path("s") },
        p_name("Quote", "blockquote > p:fresh"),
        p_name("Intense Quote", "blockquote > p:fresh"),
    ];
    // mammoth's default style map.
    for (i, id) in ["Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6"].into_iter().enumerate() {
        m.push(p_id(id, &format!("h{}:fresh", i + 1)));
    }
    for prefix in ["Heading", "heading"] {
        for i in 1..=6 {
            m.push(p_name(&format!("{prefix} {i}"), &format!("h{i}:fresh")));
        }
    }
    m.push(p_id("Heading", "h1:fresh"));
    m.push(p_name("Heading", "h1:fresh"));
    m.push(r_name("Strong", "strong"));
    m.push(p_name("footnote text", "p:fresh"));
    m.push(r_name("footnote reference", ""));
    m.push(p_name("endnote text", "p:fresh"));
    m.push(r_name("endnote reference", ""));
    m.push(p_name("annotation text", "p:fresh"));
    m.push(r_name("annotation reference", ""));
    m.push(p_name("Footnote", "p:fresh"));
    m.push(r_name("Footnote anchor", ""));
    m.push(p_name("Endnote", "p:fresh"));
    m.push(r_name("Endnote anchor", ""));
    for (ordered, tag) in [(false, "ul"), (true, "ol")] {
        for level in 0..5 {
            let mut s = String::new();
            for _ in 0..level {
                s.push_str("ul|ol > li > ");
            }
            s.push_str(&format!("{tag} > li:fresh"));
            m.push(list(level, ordered, &s));
        }
    }
    m.push(r_name("Hyperlink", ""));
    m.push(p_name("Normal", "p:fresh"));
    m.push(p_id("Body", "p:fresh"));
    m.push(p_name("Body", "p:fresh"));
    m
}

/// JS `Number(s)` for list levels (`level == levelIndex`).
fn js_number(s: &str) -> f64 {
    let t = s.trim_matches(js_space);
    if t.is_empty() {
        return 0.0;
    }
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(h, 16).map_or(f64::NAN, |v| v as f64);
    }
    if t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')) {
        t.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

fn name_matches(op: &Option<NameOp>, name: &Option<String>) -> bool {
    match op {
        None => true,
        Some(NameOp::Equal(want)) => name.as_ref().is_some_and(|n| !n.is_empty() && n.to_uppercase() == want.to_uppercase()),
    }
}

enum Probe<'a> {
    Paragraph(&'a Para),
    Run(&'a RunEl),
    /// `{type: 'underline'}` etc.
    Property(&'static str),
}

fn find_style<'m>(map: &'m [Mapping], probe: &Probe<'_>) -> Option<&'m HtmlPath> {
    map.iter()
        .find(|m| match (&m.from, probe) {
            (Matcher::Paragraph { style_id, style_name, list }, Probe::Paragraph(p)) => {
                style_id.is_none_or(|id| p.style_id.as_deref() == Some(id))
                    && name_matches(style_name, &p.style_name)
                    && list.is_none_or(|(level, ordered)| p.numbering.as_ref().is_some_and(|n| js_number(&n.level) == level as f64 && n.is_ordered == ordered))
            }
            (Matcher::Run { style_name }, Probe::Run(r)) => name_matches(style_name, &r.style_name),
            (Matcher::Underline, Probe::Property("underline")) | (Matcher::Strikethrough, Probe::Property("strikethrough")) => true,
            _ => false,
        })
        .map(|m| &m.to)
}

// ================================================================ HTML

#[derive(Clone, Debug)]
enum H {
    El(Tag, Vec<H>),
    Text(String),
    Force,
    Deferred(usize),
}

fn fresh(name: &str, attrs: Vec<(&str, Option<String>)>, children: Vec<H>) -> H {
    H::El(Tag::with_attrs(name, true, attrs), children)
}

fn is_void(tag: &Tag, children: &[H]) -> bool {
    children.is_empty() && matches!(tag.name.as_str(), "br" | "hr" | "img" | "input")
}

struct Converter<'a> {
    map: &'a [Mapping],
    msgs: Vec<String>,
    note_number: u32,
    note_refs: Vec<(&'static str, Option<String>)>,
    images: Vec<Img>,
}

fn unrecognised(kind: &str, name: &Option<String>, id: &str) -> String {
    format!("warning: Unrecognised {kind} style: '{}' (Style ID: {id})", name.as_deref().unwrap_or("null"))
}

impl Converter<'_> {
    fn all(&mut self, els: &[D], header: bool) -> Res<Vec<H>> {
        let mut out = Vec::new();
        for e in els {
            out.extend(self.one(e, header)?);
        }
        Ok(out)
    }

    fn one(&mut self, el: &D, header: bool) -> Res<Vec<H>> {
        Ok(match el {
            D::Paragraph(p) => {
                let path = match find_style(self.map, &Probe::Paragraph(p)) {
                    Some(path) => path.clone(),
                    None => {
                        if let Some(id) = &p.style_id {
                            self.msgs.push(unrecognised("paragraph", &p.style_name, id));
                        }
                        HtmlPath(vec![Tag::new("p", true)])
                    }
                };
                let content = self.all(&p.children, header)?;
                path.wrap(content)
            }
            D::Run(r) => {
                let mut paths: Vec<HtmlPath> = Vec::new();
                let prop = |c: &Self, name: &'static str, default: Option<&str>| -> HtmlPath {
                    find_style(c.map, &Probe::Property(name)).cloned().unwrap_or_else(|| HtmlPath(default.map(|d| vec![Tag::new(d, false)]).unwrap_or_default()))
                };
                // `highlight` paths only come from a style map entry (none in this map), so highlights are dropped.
                if r.small_caps {
                    paths.push(prop(self, "smallCaps", None));
                }
                if r.all_caps {
                    paths.push(prop(self, "allCaps", None));
                }
                if r.strike {
                    paths.push(prop(self, "strikethrough", Some("s")));
                }
                if r.underline {
                    paths.push(prop(self, "underline", None));
                }
                if r.vertical == "subscript" {
                    paths.push(HtmlPath(vec![Tag::new("sub", false)]));
                }
                if r.vertical == "superscript" {
                    paths.push(HtmlPath(vec![Tag::new("sup", false)]));
                }
                if r.italic {
                    paths.push(prop(self, "italic", Some("em")));
                }
                if r.bold {
                    paths.push(prop(self, "bold", Some("strong")));
                }
                match find_style(self.map, &Probe::Run(r)) {
                    Some(p) => paths.push(p.clone()),
                    None => {
                        if let Some(id) = &r.style_id {
                            self.msgs.push(unrecognised("run", &r.style_name, id));
                        }
                        paths.push(HtmlPath(vec![]));
                    }
                }
                let mut nodes = self.all(&r.children, header)?;
                for p in &paths {
                    nodes = p.wrap(nodes);
                }
                nodes
            }
            D::Text(t) => vec![H::Text(t.clone())],
            D::Tab => vec![H::Text("\t".into())],
            D::Hyperlink { children, href, anchor, target_frame } => {
                let href = match anchor.as_deref().filter(|a| !a.is_empty()) {
                    Some(a) => Some(format!("#{a}")),
                    None => href.clone(),
                };
                let mut attrs = vec![("href", href)];
                if let Some(t) = target_frame {
                    attrs.push(("target", Some(t.clone())));
                }
                let c = self.all(children, header)?;
                vec![H::El(Tag::with_attrs("a", false, attrs), c)]
            }
            D::Checkbox(checked) => {
                let mut attrs = vec![("type", Some("checkbox".to_owned()))];
                if *checked {
                    attrs.push(("checked", Some("checked".to_owned())));
                }
                vec![fresh("input", attrs, vec![])]
            }
            D::Bookmark(name) => vec![fresh("a", vec![("id", Some(key(name.as_deref())))], vec![H::Force])],
            D::NoteRef { note_type, note_id } => {
                self.note_refs.push((note_type, note_id.clone()));
                let id = key(note_id.as_deref());
                let n = self.note_number;
                self.note_number += 1;
                let a = fresh("a", vec![("href", Some(format!("#{note_type}-{id}"))), ("id", Some(format!("{note_type}-ref-{id}")))], vec![H::Text(format!("[{n}]"))]);
                vec![fresh("sup", vec![], vec![a])]
            }
            // `comment-reference` has no style map entry: comments are ignored.
            D::CommentRef => vec![],
            D::Image(img) => {
                self.images.push(img.clone());
                vec![H::Deferred(self.images.len() - 1)]
            }
            D::Table { children, .. } => {
                let body_index = children.iter().position(|c| !matches!(c, D::Row { is_header: true, .. })).unwrap_or(children.len());
                let mut out = vec![H::Force];
                if body_index == 0 {
                    out.extend(self.all(children, false)?);
                } else {
                    let head = self.all(&children[..body_index], true)?;
                    let body = self.all(&children[body_index..], false)?;
                    out.push(fresh("thead", vec![], head));
                    out.push(fresh("tbody", vec![], body));
                }
                vec![fresh("table", vec![], out)]
            }
            D::Row { children, .. } => {
                let mut c = vec![H::Force];
                c.extend(self.all(children, header)?);
                vec![fresh("tr", vec![], c)]
            }
            D::Cell { children, col_span, row_span, .. } => {
                let mut attrs = Vec::new();
                if *col_span != 1.0 {
                    attrs.push(("colspan", Some(js_num(*col_span))));
                }
                if *row_span != 1 {
                    attrs.push(("rowspan", Some(row_span.to_string())));
                }
                let mut c = vec![H::Force];
                c.extend(self.all(children, header)?);
                vec![fresh(if header { "th" } else { "td" }, attrs, c)]
            }
            D::Break(kind) => {
                if *kind == "line" {
                    vec![fresh("br", vec![], vec![])]
                } else {
                    vec![]
                }
            }
        })
    }

    fn note(&mut self, n: &Note) -> Res<Vec<H>> {
        let id = format!("{}-{}", n.note_type, key(n.note_id.as_deref()));
        let mut body = self.all(&n.body, false)?;
        let back = fresh("a", vec![("href", Some(format!("#{}-ref-{}", n.note_type, key(n.note_id.as_deref()))))], vec![H::Text("↑".into())]);
        body.push(H::El(Tag::new("p", false), vec![H::Text(" ".into()), back]));
        Ok(vec![fresh("li", vec![("id", Some(id))], body)])
    }
}

fn remove_empty(nodes: Vec<H>) -> Vec<H> {
    nodes
        .into_iter()
        .filter_map(|n| match n {
            H::El(tag, children) => {
                let children = remove_empty(children);
                if children.is_empty() && !is_void(&tag, &children) { None } else { Some(H::El(tag, children)) }
            }
            H::Text(t) if t.is_empty() => None,
            other => Some(other),
        })
        .collect()
}

fn collapse(nodes: Vec<H>) -> Vec<H> {
    let mut out = Vec::new();
    for n in nodes {
        let n = match n {
            H::El(tag, children) => H::El(tag, collapse(children)),
            other => other,
        };
        append_child(&mut out, n);
    }
    out
}

fn append_child(children: &mut Vec<H>, child: H) {
    if let H::El(tag, grand) = &child
        && !tag.fresh
        && let Some(H::El(last_tag, last_children)) = children.last_mut()
        && tag.matches(last_tag)
    {
        for g in grand.clone() {
            append_child(last_children, g);
        }
        return;
    }
    children.push(child);
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

fn write(out: &mut String, nodes: &[H]) -> Res<()> {
    for n in nodes {
        match n {
            H::El(tag, children) => {
                let mut attrs = String::new();
                for (k, v) in &tag.attrs {
                    let v = v.as_deref().ok_or("Cannot read properties of undefined (reading 'replace')")?;
                    attrs.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
                }
                if is_void(tag, children) {
                    out.push_str(&format!("<{}{attrs} />", tag.name));
                } else {
                    out.push_str(&format!("<{}{attrs}>", tag.name));
                    write(out, children)?;
                    out.push_str(&format!("</{}>", tag.name));
                }
            }
            H::Text(t) => out.push_str(&escape_text(t)),
            H::Force | H::Deferred(_) => {}
        }
    }
    Ok(())
}

fn collect_deferred(nodes: &[H], out: &mut Vec<usize>) {
    for n in nodes {
        match n {
            H::Deferred(i) => out.push(*i),
            H::El(_, c) => collect_deferred(c, out),
            _ => {}
        }
    }
}

fn replace_deferred(nodes: Vec<H>, values: &HashMap<usize, Vec<H>>) -> Vec<H> {
    nodes
        .into_iter()
        .flat_map(|n| match n {
            H::Deferred(i) => values.get(&i).cloned().unwrap_or_default(),
            H::El(t, c) => vec![H::El(t, replace_deferred(c, values))],
            other => vec![other],
        })
        .collect()
}

// ================================================================ conversion

fn read_notes(pkg: &mut Package, path: &str, kind: &'static str, ctx_parts: (&ContentTypes, &Styles, &Numbering), msgs: &mut Vec<String>) -> Res<Vec<Note>> {
    let rels = read_rels(pkg, &rels_filename(path))?;
    let Some(root) = pkg.xml(path)? else { return Ok(vec![]) };
    let ctx = Ctx { rels: &rels, content_types: ctx_parts.0, styles: ctx_parts.1, numbering: ctx_parts.2 };
    let mut out = Vec::new();
    for el in root.elements(&format!("w:{kind}")) {
        if matches!(el.attr("w:type"), Some("continuationSeparator" | "separator")) {
            continue;
        }
        let body = BodyReader::new(&ctx, msgs).children(el)?.value;
        out.push(Note { note_type: kind, note_id: el.attr("w:id").map(str::to_owned), body });
    }
    Ok(out)
}

fn convert(bytes: &[u8], image: &mut dyn FnMut(&DocxImage<'_>) -> String) -> Res<DocxHtml> {
    let zip = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).map_err(|e| e.to_string())?;
    let mut pkg = Package { zip };

    let content_types = match pkg.xml("[Content_Types].xml")? {
        Some(e) => ContentTypes::read(&e)?,
        None => ContentTypes::default(),
    };
    // findPartPaths
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
    let package_rels = read_rels(&mut pkg, "_rels/.rels")?;
    let find_part = |pkg: &mut Package, rels: &Rels, ty: &str, base: &str, fallback: &str| -> String {
        for t in rels.targets_by_type(ty) {
            let j = join_path(&[base, t]);
            let j = j.strip_prefix('/').unwrap_or(&j).to_owned();
            if pkg.exists(&j) {
                return j;
            }
        }
        fallback.to_owned()
    };
    let main = find_part(&mut pkg, &package_rels, &format!("{REL}officeDocument"), "", "word/document.xml");
    if !pkg.exists(&main) {
        return Err("Could not find main document part. Are you sure this is a valid .docx file?".into());
    }
    let doc_rels = read_rels(&mut pkg, &rels_filename(&main))?;
    let base = split_path(&main).0.to_owned();
    let mut related = |name: &str| find_part(&mut pkg, &doc_rels, &format!("{REL}{name}"), &base, &format!("word/{name}.xml"));
    let (comments_p, endnotes_p, footnotes_p, numbering_p, styles_p) = (related("comments"), related("endnotes"), related("footnotes"), related("numbering"), related("styles"));

    let styles = match pkg.xml(&styles_p)? {
        Some(e) => Styles::read(&e),
        None => Styles::default(),
    };
    let numbering = match pkg.xml(&numbering_p)? {
        Some(e) => Numbering::read(&e)?,
        None => Numbering::default(),
    };

    let mut msgs: Vec<String> = Vec::new();
    let parts = (&content_types, &styles, &numbering);
    let mut notes = read_notes(&mut pkg, &footnotes_p, "footnote", parts, &mut msgs)?;
    notes.extend(read_notes(&mut pkg, &endnotes_p, "endnote", parts, &mut msgs)?);
    {
        // Comments are read (their messages count) but never shown: `comment-reference` maps to nothing.
        let rels = read_rels(&mut pkg, &rels_filename(&comments_p))?;
        if let Some(root) = pkg.xml(&comments_p)? {
            let ctx = Ctx { rels: &rels, content_types: &content_types, styles: &styles, numbering: &numbering };
            for el in root.elements("w:comment") {
                BodyReader::new(&ctx, &mut msgs).children(el)?;
            }
        }
    }
    let rels = read_rels(&mut pkg, &rels_filename(&main))?;
    let root = pkg.xml(&main)?.ok_or("Could not find main document part")?;
    let body = root.first("w:body").ok_or("Could not find the body element: are you sure this is a docx file?")?;
    let ctx = Ctx { rels: &rels, content_types: &content_types, styles: &styles, numbering: &numbering };
    let children = BodyReader::new(&ctx, &mut msgs).children(body)?.value;

    // document-to-html
    let map = style_map();
    let mut conv = Converter { map: &map, msgs: Vec::new(), note_number: 1, note_refs: Vec::new(), images: Vec::new() };
    let mut html = conv.all(&children, false)?;
    let refs = conv.note_refs.clone();
    let mut note_nodes = Vec::new();
    for (ty, id) in refs {
        // `_.indexBy`: the last note with a key wins.
        let note = notes
            .iter()
            .rev()
            .find(|n| n.note_type == ty && key(n.note_id.as_deref()) == key(id.as_deref()))
            .ok_or("Cannot read properties of null (reading 'type')")?;
        note_nodes.extend(conv.note(note)?);
    }
    html.push(fresh("ol", vec![], note_nodes));
    html.push(fresh("dl", vec![], vec![]));

    let mut order = Vec::new();
    collect_deferred(&html, &mut order);
    let mut values: HashMap<usize, Vec<H>> = HashMap::new();
    for i in order {
        let img = conv.images[i].clone();
        let bytes = match &img.source {
            ImgSource::Part(p) => pkg.read(p)?,
            ImgSource::External(_) => None,
        };
        let Some(bytes) = bytes else {
            let m = match &img.source {
                ImgSource::Part(_) => "Cannot read properties of null (reading 'async')".to_owned(),
                ImgSource::External(u) => format!("could not read external image '{u}', external file access is disabled"),
            };
            conv.msgs.push(format!("error: {m}"));
            values.insert(i, vec![]);
            continue;
        };
        let src = image(&DocxImage { content_type: img.content_type.as_deref().unwrap_or(""), bytes: &bytes });
        let mut attrs = Vec::new();
        if let Some(a) = img.alt.filter(|a| !a.is_empty()) {
            attrs.push(("alt", Some(a)));
        }
        attrs.push(("src", Some(src)));
        values.insert(i, vec![fresh("img", attrs, vec![])]);
    }
    let html = collapse(remove_empty(replace_deferred(html, &values)));
    let mut out = String::new();
    write(&mut out, &html)?;

    msgs.extend(conv.msgs);
    let mut messages: Vec<String> = Vec::new();
    for m in msgs {
        if !messages.contains(&m) {
            messages.push(m);
        }
    }
    Ok(DocxHtml { html: out, messages })
}
