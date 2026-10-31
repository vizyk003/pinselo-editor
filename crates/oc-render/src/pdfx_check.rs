// SPDX-License-Identifier: Apache-2.0
//! A structural PDF/X checker for the files [`crate::pdf::write_pdf`] produces (classic cross-reference table, no
//! object streams — what krilla / pdf-writer write). It is not a full ISO 15930 validator (use callas pdfToolbox or
//! Acrobat Preflight for that); it checks what our writer controls and what most often breaks print jobs:
//!
//! - header version (PDF/X-1a:2003 ≤ 1.4, PDF/X-4 ≤ 1.6), no encryption, a file `/ID`;
//! - one `/OutputIntents` entry with `/S /GTS_PDFX`, an `/OutputConditionIdentifier` and an embedded
//!   `/DestOutputProfile` (ICC stream with `/N`);
//! - document info: `/GTS_PDFXVersion` (+ `/GTS_PDFXConformance` for X-1a), `/Trapped` True/False, `/Title`,
//!   `/CreationDate`, `/ModDate`; XMP carrying `pdfxid:GTS_PDFXVersion` (X-4);
//! - every page has a `/TrimBox` inside its `/BleedBox` inside its `/MediaBox`;
//! - fonts: every font descriptor embeds its program (`/FontFile*`), every simple/CID font has a descriptor
//!   (Type 3 fonts are self-contained);
//! - colour: no `DeviceRGB` with a CMYK output intent (dictionaries and `rg`/`RG` in every content stream); for
//!   X-1a additionally no `ICCBased`, `CalRGB`, `Lab` or other CIE spaces and every image `DeviceCMYK`,
//!   `DeviceGray`, `Separation`/`DeviceN` or `Indexed` over those;
//! - transparency (X-1a): no soft masks (`/SMask` on images or graphics states), no constant alpha below 1, no
//!   blend modes other than Normal/Compatible, no transparency groups;
//! - no annotations other than `PrinterMark`/`TrapNet` (they would sit inside the bleed box).
//!
//! It also lists what it saw (colour spaces of images, content operators) for tests and the render service log.

use std::collections::{BTreeMap, BTreeSet};

use crate::pdf::PdfStandard;

/// A parsed PDF value (enough of the syntax for dictionaries written by pdf-writer).
#[derive(Debug, Clone, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Num(f64),
    Name(String),
    Str(Vec<u8>),
    Array(Vec<Obj>),
    Dict(BTreeMap<String, Obj>),
    Ref(u32),
}

impl Obj {
    fn dict(&self) -> Option<&BTreeMap<String, Obj>> {
        match self {
            Obj::Dict(d) => Some(d),
            _ => None,
        }
    }
    fn name(&self) -> Option<&str> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }
    fn num(&self) -> Option<f64> {
        match self {
            Obj::Num(n) => Some(*n),
            _ => None,
        }
    }
    fn text(&self) -> Option<String> {
        match self {
            Obj::Str(s) => Some(decode_text(s)),
            _ => None,
        }
    }
}

fn decode_text(s: &[u8]) -> String {
    if s.starts_with(&[0xFE, 0xFF]) {
        let u: Vec<u16> = s[2..].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        String::from_utf16_lossy(&u)
    } else {
        s.iter().map(|b| *b as char).collect()
    }
}

struct Lexer<'a> {
    b: &'a [u8],
    i: usize,
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\r' | b'\t' | 0x0C | 0)
}

fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

impl<'a> Lexer<'a> {
    fn skip(&mut self) {
        while self.i < self.b.len() {
            if is_ws(self.b[self.i]) {
                self.i += 1;
            } else if self.b[self.i] == b'%' {
                while self.i < self.b.len() && self.b[self.i] != b'\n' && self.b[self.i] != b'\r' {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    fn word(&mut self) -> &'a [u8] {
        let s = self.i;
        while self.i < self.b.len() && !is_ws(self.b[self.i]) && !is_delim(self.b[self.i]) {
            self.i += 1;
        }
        &self.b[s..self.i]
    }

    fn literal(&mut self) -> Vec<u8> {
        // At '('.
        self.i += 1;
        let mut depth = 1;
        let mut out = Vec::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'\\' if self.i < self.b.len() => {
                    let e = self.b[self.i];
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                if self.i < self.b.len() && (b'0'..=b'7').contains(&self.b[self.i]) {
                                    v = v * 8 + u32::from(self.b[self.i] - b'0');
                                    self.i += 1;
                                }
                            }
                            out.push(v as u8);
                        }
                        b'\r' | b'\n' => {}
                        _ => out.push(e),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn hex(&mut self) -> Vec<u8> {
        // At '<' (not '<<').
        self.i += 1;
        let mut digits = Vec::new();
        while self.i < self.b.len() && self.b[self.i] != b'>' {
            let c = self.b[self.i];
            if c.is_ascii_hexdigit() {
                digits.push(c);
            }
            self.i += 1;
        }
        self.i += 1;
        if digits.len() % 2 == 1 {
            digits.push(b'0');
        }
        digits.chunks(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap_or("00"), 16).unwrap_or(0)).collect()
    }

    fn name(&mut self) -> String {
        // At '/'.
        self.i += 1;
        let w = self.word();
        let mut out = Vec::new();
        let mut k = 0;
        while k < w.len() {
            if w[k] == b'#'
                && k + 3 <= w.len()
                && let Some(v) = w.get(k + 1..k + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                k += 3;
                continue;
            }
            out.push(w[k]);
            k += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Parse one object (references `n g R` resolved syntactically).
    fn obj(&mut self) -> Option<Obj> {
        self.skip();
        let c = *self.b.get(self.i)?;
        Some(match c {
            b'/' => Obj::Name(self.name()),
            b'(' => Obj::Str(self.literal()),
            b'<' if self.b.get(self.i + 1) == Some(&b'<') => {
                self.i += 2;
                let mut d = BTreeMap::new();
                loop {
                    self.skip();
                    if self.b.get(self.i..self.i + 2) == Some(b">>") {
                        self.i += 2;
                        break;
                    }
                    if self.i >= self.b.len() {
                        break;
                    }
                    let Some(Obj::Name(k)) = self.obj() else { return None };
                    let v = self.obj()?;
                    d.insert(k, v);
                }
                Obj::Dict(d)
            }
            b'<' => Obj::Str(self.hex()),
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                loop {
                    self.skip();
                    if self.b.get(self.i) == Some(&b']') {
                        self.i += 1;
                        break;
                    }
                    if self.i >= self.b.len() {
                        break;
                    }
                    a.push(self.obj()?);
                }
                Obj::Array(a)
            }
            _ => {
                let w = self.word();
                if w.is_empty() {
                    self.i += 1;
                    return Some(Obj::Null);
                }
                match w {
                    b"true" => Obj::Bool(true),
                    b"false" => Obj::Bool(false),
                    b"null" => Obj::Null,
                    _ => {
                        let n: f64 = std::str::from_utf8(w).ok()?.parse().ok()?;
                        // `n g R`?
                        let save = self.i;
                        self.skip();
                        let g = self.word();
                        if !g.is_empty() && g.iter().all(u8::is_ascii_digit) {
                            self.skip();
                            if self.b.get(self.i) == Some(&b'R') && self.b.get(self.i + 1).is_none_or(|c| is_ws(*c) || is_delim(*c)) {
                                self.i += 1;
                                return Some(Obj::Ref(n as u32));
                            }
                        }
                        self.i = save;
                        Obj::Num(n)
                    }
                }
            }
        })
    }
}

/// An indirect object: its value and (for streams) the decoded data.
#[derive(Debug, Clone)]
pub struct Indirect {
    pub value: Obj,
    pub stream: Option<Vec<u8>>,
}

/// A parsed file.
#[derive(Debug, Default)]
pub struct ParsedPdf {
    pub header: String,
    pub objects: BTreeMap<u32, Indirect>,
    pub trailer: BTreeMap<String, Obj>,
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

/// Parse a PDF written with a classic xref table.
pub fn parse(pdf: &[u8]) -> Result<ParsedPdf, String> {
    let mut out = ParsedPdf { header: String::from_utf8_lossy(pdf.get(..8).unwrap_or_default()).into_owned(), ..ParsedPdf::default() };
    let sx = rfind(pdf, b"startxref").ok_or("no startxref")?;
    let mut lx = Lexer { b: pdf, i: sx + 9 };
    let xref = lx.obj().and_then(|o| o.num()).ok_or("bad startxref")? as usize;
    if pdf.get(xref..xref + 4) != Some(b"xref") {
        return Err("cross-reference streams are not supported by this checker".into());
    }
    let tpos = find(pdf, b"trailer", xref).ok_or("no trailer")?;
    let mut lx = Lexer { b: pdf, i: tpos + 7 };
    out.trailer = lx.obj().and_then(|o| o.dict().cloned()).ok_or("bad trailer")?;
    // Entries: "nnnnnnnnnn ggggg n" lines after "start count" subsection headers.
    let mut lx = Lexer { b: pdf, i: xref + 4 };
    let mut offsets: Vec<(u32, usize)> = Vec::new();
    loop {
        lx.skip();
        if pdf.get(lx.i..lx.i + 7) == Some(b"trailer") {
            break;
        }
        let start = lx.obj().and_then(|o| o.num()).ok_or("bad xref subsection")? as u32;
        let count = lx.obj().and_then(|o| o.num()).ok_or("bad xref subsection")? as u32;
        for k in 0..count {
            lx.skip();
            let line = pdf.get(lx.i..lx.i + 18).ok_or("truncated xref")?;
            let off: usize = std::str::from_utf8(&line[..10]).ok().and_then(|s| s.parse().ok()).ok_or("bad xref entry")?;
            if line[17] == b'n' {
                offsets.push((start + k, off));
            }
            lx.i += 18;
        }
    }
    for (num, off) in offsets {
        let mut lx = Lexer { b: pdf, i: off };
        let _n = lx.obj();
        let _g = lx.obj();
        lx.skip();
        if pdf.get(lx.i..lx.i + 3) != Some(b"obj") {
            return Err(format!("object {num}: no `obj` at offset {off}"));
        }
        lx.i += 3;
        let value = lx.obj().ok_or_else(|| format!("object {num}: unparsable"))?;
        lx.skip();
        let mut stream = None;
        if pdf.get(lx.i..lx.i + 6) == Some(b"stream") {
            let mut s = lx.i + 6;
            if pdf.get(s) == Some(&b'\r') {
                s += 1;
            }
            if pdf.get(s) == Some(&b'\n') {
                s += 1;
            }
            let d = value.dict().ok_or_else(|| format!("object {num}: stream without dictionary"))?;
            let len = match d.get("Length") {
                Some(Obj::Num(n)) => *n as usize,
                _ => find(pdf, b"endstream", s).ok_or("no endstream")? - s,
            };
            let raw = pdf.get(s..s + len).ok_or_else(|| format!("object {num}: stream past end of file"))?;
            let filters: Vec<String> = match d.get("Filter") {
                Some(Obj::Name(n)) => vec![n.clone()],
                Some(Obj::Array(a)) => a.iter().filter_map(|o| o.name().map(str::to_string)).collect(),
                _ => Vec::new(),
            };
            stream = Some(match filters.first().map(String::as_str) {
                None => raw.to_vec(),
                Some("FlateDecode") if filters.len() == 1 => miniz_oxide::inflate::decompress_to_vec_zlib(raw).map_err(|e| format!("object {num}: inflate: {e:?}"))?,
                // Image codecs (DCT, JPX, …): the data isn't needed.
                Some(_) => Vec::new(),
            });
        }
        out.objects.insert(num, Indirect { value, stream });
    }
    Ok(out)
}

impl ParsedPdf {
    pub fn get<'a>(&'a self, o: &'a Obj) -> Option<&'a Obj> {
        match o {
            Obj::Ref(r) => self.objects.get(r).map(|i| &i.value),
            other => Some(other),
        }
    }

    fn dict_of<'a>(&'a self, o: &'a Obj) -> Option<&'a BTreeMap<String, Obj>> {
        self.get(o).and_then(Obj::dict)
    }

    fn rect(&self, o: Option<&Obj>) -> Option<[f64; 4]> {
        match o.and_then(|o| self.get(o)) {
            Some(Obj::Array(a)) if a.len() == 4 => {
                let v: Vec<f64> = a.iter().filter_map(Obj::num).collect();
                (v.len() == 4).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
            }
            _ => None,
        }
    }

    /// Pages in order (leaves of the page tree), with inherited MediaBox.
    pub fn pages(&self) -> Vec<(u32, BTreeMap<String, Obj>)> {
        let mut out = Vec::new();
        let Some(Obj::Ref(root)) = self.trailer.get("Root") else { return out };
        let Some(cat) = self.objects.get(root).and_then(|o| o.value.dict()) else { return out };
        let mut stack: Vec<(Obj, Option<Obj>)> = cat.get("Pages").map(|p| vec![(p.clone(), None)]).unwrap_or_default();
        while let Some((node, inherited)) = stack.pop() {
            let Obj::Ref(id) = node else { continue };
            let Some(d) = self.objects.get(&id).and_then(|o| o.value.dict()) else { continue };
            let media = d.get("MediaBox").cloned().or(inherited);
            match d.get("Type").and_then(Obj::name) {
                Some("Pages") => {
                    if let Some(Obj::Array(kids)) = d.get("Kids").and_then(|k| self.get(k)) {
                        for k in kids.iter().rev() {
                            stack.push((k.clone(), media.clone()));
                        }
                    }
                }
                _ => {
                    let mut d = d.clone();
                    if let Some(m) = media {
                        d.entry("MediaBox".into()).or_insert(m);
                    }
                    out.push((id, d));
                }
            }
        }
        out
    }
}

/// What the checker found.
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub header: String,
    pub pages: usize,
    /// Problems (empty = passes these checks).
    pub issues: Vec<String>,
    /// Colour space families used by images.
    pub image_spaces: BTreeSet<String>,
    /// Colour space families named anywhere in resources.
    pub color_spaces: BTreeSet<String>,
    /// Content-stream colour operators seen (`rg`, `k`, `scn`, …).
    pub color_ops: BTreeSet<String>,
    pub output_condition: Option<String>,
    pub pdfx_version: Option<String>,
    /// Graphics states with overprint on.
    pub overprint_states: usize,
    pub fonts: usize,
}

const CIE: [&str; 5] = ["ICCBased", "CalRGB", "CalGray", "Lab", "DefaultRGB"];

fn family(pdf: &ParsedPdf, cs: &Obj) -> Vec<String> {
    match pdf.get(cs) {
        Some(Obj::Name(n)) => vec![n.clone()],
        Some(Obj::Array(a)) => {
            let head = a.first().and_then(Obj::name).unwrap_or("?").to_string();
            let mut v = vec![head.clone()];
            match head.as_str() {
                // Alternate / base spaces count too.
                "Separation" => v.extend(a.get(2).map(|alt| family(pdf, alt)).unwrap_or_default()),
                "DeviceN" => v.extend(a.get(2).map(|alt| family(pdf, alt)).unwrap_or_default()),
                "Indexed" => v.extend(a.get(1).map(|b| family(pdf, b)).unwrap_or_default()),
                "ICCBased" => {
                    if let Some(Obj::Ref(r)) = a.get(1)
                        && let Some(n) = pdf.objects.get(r).and_then(|o| o.value.dict()).and_then(|d| d.get("N")).and_then(Obj::num)
                    {
                        v.push(format!("ICCBased/N{n}"));
                    }
                }
                _ => {}
            }
            v
        }
        _ => Vec::new(),
    }
}

/// Content-stream operators (names only), skipping strings, arrays' contents and inline images.
fn operators(content: &[u8]) -> Vec<String> {
    let mut lx = Lexer { b: content, i: 0 };
    let mut ops = Vec::new();
    loop {
        lx.skip();
        let Some(&c) = content.get(lx.i) else { break };
        match c {
            b'(' => {
                lx.literal();
            }
            b'<' if content.get(lx.i + 1) == Some(&b'<') => {
                let _ = lx.obj();
            }
            b'<' => {
                lx.hex();
            }
            b'/' => {
                lx.name();
            }
            b'[' | b']' | b'{' | b'}' | b'>' | b')' => lx.i += 1,
            _ => {
                let w = lx.word();
                if w.is_empty() {
                    lx.i += 1;
                    continue;
                }
                if w[0].is_ascii_alphabetic() || w[0] == b'\'' || w[0] == b'"' {
                    let op = String::from_utf8_lossy(w).into_owned();
                    if op == "BI" {
                        // Skip inline image data up to EI.
                        if let Some(e) = find(content, b"EI", lx.i) {
                            lx.i = e + 2;
                        }
                    }
                    ops.push(op);
                }
            }
        }
    }
    ops
}

/// Check a PDF against the PDF/X rules our writer is responsible for.
pub fn check(pdf: &[u8], standard: PdfStandard) -> Report {
    let mut r = Report { header: String::from_utf8_lossy(pdf.get(..8).unwrap_or_default()).into_owned(), ..Report::default() };
    let parsed = match parse(pdf) {
        Ok(p) => p,
        Err(e) => {
            r.issues.push(format!("unparsable: {e}"));
            return r;
        }
    };
    let x1a = standard == PdfStandard::PdfX1a;
    let max_minor = match standard {
        PdfStandard::PdfX1a => 4,
        PdfStandard::PdfX4 => 6,
        PdfStandard::None => 7,
    };
    let minor = r.header.strip_prefix("%PDF-1.").and_then(|v| v.trim().parse::<u32>().ok());
    if minor.is_none_or(|m| m > max_minor) {
        r.issues.push(format!("header {} is newer than PDF 1.{max_minor}", r.header.trim()));
    }
    if parsed.trailer.contains_key("Encrypt") {
        r.issues.push("encrypted".into());
    }
    if !parsed.trailer.contains_key("ID") {
        r.issues.push("no file /ID in the trailer".into());
    }
    let root = parsed.trailer.get("Root").and_then(|o| parsed.dict_of(o)).cloned().unwrap_or_default();

    // Output intent.
    let mut cmyk_oi = false;
    match root.get("OutputIntents").and_then(|o| parsed.get(o)) {
        Some(Obj::Array(a)) => {
            let ois: Vec<_> = a.iter().filter_map(|o| parsed.dict_of(o)).filter(|d| d.get("S").and_then(Obj::name) == Some("GTS_PDFX")).collect();
            if ois.len() != 1 {
                r.issues.push(format!("{} GTS_PDFX output intents (need exactly 1)", ois.len()));
            }
            if let Some(oi) = ois.first() {
                r.output_condition = oi.get("OutputConditionIdentifier").and_then(Obj::text);
                if r.output_condition.as_deref().is_none_or(str::is_empty) {
                    r.issues.push("output intent without /OutputConditionIdentifier".into());
                }
                match oi.get("DestOutputProfile") {
                    Some(Obj::Ref(p)) => match parsed.objects.get(p) {
                        Some(Indirect { value: Obj::Dict(d), stream: Some(s) }) => {
                            let n = d.get("N").and_then(Obj::num).unwrap_or(0.0);
                            cmyk_oi = n == 4.0;
                            if s.len() < 128 || s.get(36..40) != Some(b"acsp") {
                                r.issues.push("DestOutputProfile is not an ICC profile".into());
                            } else if s.get(12..16) != Some(b"prtr") && n == 4.0 {
                                r.issues.push("DestOutputProfile is not an output (prtr) profile".into());
                            }
                            if x1a && n != 4.0 {
                                r.issues.push("PDF/X-1a needs a CMYK output intent".into());
                            }
                        }
                        _ => r.issues.push("DestOutputProfile is not a stream".into()),
                    },
                    _ => r.issues.push("output intent without an embedded /DestOutputProfile".into()),
                }
            }
        }
        _ if standard != PdfStandard::None => r.issues.push("no /OutputIntents".into()),
        _ => {}
    }

    // Document info + XMP.
    let info = parsed.trailer.get("Info").and_then(|o| parsed.dict_of(o)).cloned().unwrap_or_default();
    r.pdfx_version = info.get("GTS_PDFXVersion").and_then(Obj::text);
    if standard != PdfStandard::None {
        let (want_v, want_c) = if x1a { ("PDF/X-1:2003", Some("PDF/X-1a:2003")) } else { ("PDF/X-4", None) };
        if r.pdfx_version.as_deref() != Some(want_v) {
            r.issues.push(format!("/GTS_PDFXVersion is {:?}, want {want_v}", r.pdfx_version));
        }
        if let Some(c) = want_c
            && info.get("GTS_PDFXConformance").and_then(Obj::text).as_deref() != Some(c)
        {
            r.issues.push(format!("/GTS_PDFXConformance missing or not {c}"));
        }
        if !matches!(info.get("Trapped").and_then(Obj::name), Some("True" | "False")) {
            r.issues.push("/Trapped must be /True or /False".into());
        }
        for k in ["Title", "CreationDate", "ModDate"] {
            if !info.contains_key(k) {
                r.issues.push(format!("document info without /{k}"));
            }
        }
        let xmp = root.get("Metadata").and_then(|m| if let Obj::Ref(id) = m { parsed.objects.get(id) } else { None }).and_then(|i| i.stream.as_ref());
        match xmp {
            Some(x) => {
                let x = String::from_utf8_lossy(x);
                if !x.contains("GTS_PDFXVersion") || !x.contains(r.pdfx_version.as_deref().unwrap_or("?")) {
                    r.issues.push("XMP does not carry the matching pdfxid:GTS_PDFXVersion".into());
                }
            }
            None if !x1a => r.issues.push("no XMP metadata stream (PDF/X-4 requires it)".into()),
            None => {}
        }
    }

    // Pages: boxes and annotations.
    let pages = parsed.pages();
    r.pages = pages.len();
    for (i, (_, p)) in pages.iter().enumerate() {
        let media = parsed.rect(p.get("MediaBox"));
        let trim = parsed.rect(p.get("TrimBox")).or_else(|| parsed.rect(p.get("ArtBox")));
        let bleed = parsed.rect(p.get("BleedBox")).or(media);
        match (media, trim, bleed) {
            (Some(m), Some(t), Some(b)) => {
                let inside = |a: [f64; 4], b: [f64; 4]| a[0] >= b[0] - 0.01 && a[1] >= b[1] - 0.01 && a[2] <= b[2] + 0.01 && a[3] <= b[3] + 0.01;
                if !inside(t, b) || !inside(b, m) {
                    r.issues.push(format!("page {}: TrimBox ⊄ BleedBox ⊄ MediaBox", i + 1));
                }
            }
            (_, None, _) if standard != PdfStandard::None => r.issues.push(format!("page {}: no TrimBox", i + 1)),
            _ => {}
        }
        if let Some(Obj::Array(an)) = p.get("Annots").and_then(|o| parsed.get(o)) {
            for a in an {
                let st = parsed.dict_of(a).and_then(|d| d.get("Subtype")).and_then(Obj::name).unwrap_or("?").to_string();
                if standard != PdfStandard::None && st != "PrinterMark" && st != "TrapNet" {
                    r.issues.push(format!("page {}: /{st} annotation", i + 1));
                }
            }
        }
        if x1a && p.get("Group").and_then(|g| parsed.dict_of(g)).and_then(|g| g.get("S")).and_then(Obj::name) == Some("Transparency") {
            r.issues.push(format!("page {}: transparency group", i + 1));
        }
    }

    // Every object: fonts, images, graphics states, colour spaces, forms, content streams.
    let forbid_rgb = cmyk_oi || x1a;
    for (num, ind) in &parsed.objects {
        let Some(d) = ind.value.dict() else {
            // Indirect colour-space arrays ([/ICCBased …], [/Separation …], …).
            if let Obj::Array(a) = &ind.value
                && a.first().and_then(Obj::name).is_some_and(|n| matches!(n, "ICCBased" | "Separation" | "DeviceN" | "Indexed" | "CalRGB" | "CalGray" | "Lab" | "Pattern"))
            {
                for f in family(&parsed, &Obj::Array(a.clone())) {
                    r.color_spaces.insert(f);
                }
            }
            continue;
        };
        let ty = d.get("Type").and_then(Obj::name);
        let sub = d.get("Subtype").and_then(Obj::name);
        if ty == Some("FontDescriptor") && !(d.contains_key("FontFile") || d.contains_key("FontFile2") || d.contains_key("FontFile3")) {
            r.issues.push(format!("object {num}: font not embedded ({:?})", d.get("FontName")));
        }
        if ty == Some("Font") {
            r.fonts += 1;
            if !matches!(sub, Some("Type0" | "Type3")) && !d.contains_key("FontDescriptor") {
                r.issues.push(format!("object {num}: font without a descriptor (not embedded)"));
            }
        }
        if ty == Some("ExtGState") {
            for k in ["CA", "ca"] {
                if let Some(v) = d.get(k).and_then(Obj::num)
                    && v < 0.999
                    && x1a
                {
                    r.issues.push(format!("object {num}: /{k} {v} (transparency)"));
                }
            }
            if let Some(bm) = d.get("BM").and_then(Obj::name)
                && x1a
                && bm != "Normal"
                && bm != "Compatible"
            {
                r.issues.push(format!("object {num}: blend mode /{bm} (transparency)"));
            }
            if x1a && d.get("SMask").is_some_and(|m| m.name() != Some("None")) {
                r.issues.push(format!("object {num}: soft mask (transparency)"));
            }
            if d.get("op") == Some(&Obj::Bool(true)) || d.get("OP") == Some(&Obj::Bool(true)) {
                r.overprint_states += 1;
            }
        }
        if sub == Some("Image") {
            let fams = d.get("ColorSpace").map(|c| family(&parsed, c)).unwrap_or_else(|| vec![if d.get("ImageMask") == Some(&Obj::Bool(true)) { "ImageMask".into() } else { "?".into() }]);
            for f in &fams {
                r.image_spaces.insert(f.clone());
            }
            if x1a {
                if d.contains_key("SMask") || d.contains_key("SMaskInData") {
                    r.issues.push(format!("object {num}: image with a soft mask (transparency)"));
                }
                if fams.iter().any(|f| f == "DeviceRGB" || CIE.contains(&f.as_str()) || f.starts_with("ICCBased")) {
                    r.issues.push(format!("object {num}: image in {fams:?} (PDF/X-1a needs CMYK/gray/spot)"));
                }
            }
            if forbid_rgb && fams.iter().any(|f| f == "DeviceRGB") {
                r.issues.push(format!("object {num}: DeviceRGB image with a CMYK output intent"));
            }
            if standard != PdfStandard::None && d.get("Interpolate") == Some(&Obj::Bool(true)) {
                r.issues.push(format!("object {num}: image with /Interpolate true"));
            }
        }
        if sub == Some("Form") && x1a && d.get("Group").and_then(|g| parsed.dict_of(g)).and_then(|g| g.get("S")).and_then(Obj::name) == Some("Transparency") {
            r.issues.push(format!("object {num}: transparency group"));
        }
        if let Some(Obj::Dict(cs)) = d.get("ColorSpace").and_then(|c| parsed.get(c)) {
            for v in cs.values() {
                for f in family(&parsed, v) {
                    r.color_spaces.insert(f);
                }
            }
        }
        // Shadings and patterns name their colour space directly.
        if (d.contains_key("ShadingType") || d.get("PatternType").is_some())
            && let Some(cs) = d.get("ColorSpace")
        {
            for f in family(&parsed, cs) {
                r.color_spaces.insert(f);
            }
        }
    }
    for f in r.color_spaces.clone() {
        if forbid_rgb && f == "DeviceRGB" {
            r.issues.push("DeviceRGB colour space with a CMYK output intent".into());
        }
        if x1a && (CIE.contains(&f.as_str()) || f.starts_with("ICCBased")) {
            r.issues.push(format!("{f} colour space (PDF/X-1a allows device CMYK/gray and spot only)"));
        }
    }

    // Content streams: pages and forms (and Type 3 glyph procs).
    let mut streams: Vec<(String, &[u8])> = Vec::new();
    for (i, (_, p)) in pages.iter().enumerate() {
        let refs: Vec<u32> = match p.get("Contents") {
            Some(Obj::Ref(c)) => vec![*c],
            Some(Obj::Array(a)) => a.iter().filter_map(|o| if let Obj::Ref(c) = o { Some(*c) } else { None }).collect(),
            _ => Vec::new(),
        };
        for c in refs {
            if let Some(s) = parsed.objects.get(&c).and_then(|o| o.stream.as_deref()) {
                streams.push((format!("page {}", i + 1), s));
            }
        }
    }
    for (num, ind) in &parsed.objects {
        if ind.value.dict().and_then(|d| d.get("Subtype")).and_then(Obj::name) == Some("Form")
            && let Some(s) = ind.stream.as_deref()
        {
            streams.push((format!("form {num}"), s));
        }
    }
    for (where_, s) in streams {
        for op in operators(s) {
            if matches!(op.as_str(), "rg" | "RG" | "k" | "K" | "g" | "G" | "sc" | "scn" | "SC" | "SCN" | "cs" | "CS") {
                if forbid_rgb && (op == "rg" || op == "RG") {
                    r.issues.push(format!("{where_}: DeviceRGB colour operator `{op}`"));
                }
                r.color_ops.insert(op);
            }
        }
    }
    r.issues.sort();
    r.issues.dedup();
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexer_handles_strings_names_and_refs() {
        let mut lx = Lexer { b: b"<< /A (x\\)y) /B#20C [1 0 R 2.5 /N] /D <48656c6c6f> /E << /F true >> >>", i: 0 };
        let d = lx.obj().unwrap();
        let d = d.dict().unwrap();
        assert_eq!(d.get("A"), Some(&Obj::Str(b"x)y".to_vec())));
        assert_eq!(d.get("B C"), Some(&Obj::Array(vec![Obj::Ref(1), Obj::Num(2.5), Obj::Name("N".into())])));
        assert_eq!(d.get("D").and_then(Obj::text).as_deref(), Some("Hello"));
        assert_eq!(d.get("E").and_then(Obj::dict).and_then(|e| e.get("F")), Some(&Obj::Bool(true)));
    }

    #[test]
    fn operators_skip_strings() {
        let ops = operators(b"q 1 0 0 rg (rg k) Tj [(a) 10 (b)] TJ 0 0 0 1 k /GS0 gs BT ET Q");
        assert_eq!(ops, vec!["q", "rg", "Tj", "TJ", "k", "gs", "BT", "ET", "Q"]);
    }
}
