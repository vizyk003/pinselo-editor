// SPDX-License-Identifier: AGPL-3.0-only
//! XLSX export and import (A15).
//!
//! Export (rust_xlsxwriter) mirrors `@oc/sheet/xlsx`'s exceljs writer: formulas with cached results, number
//! formats, fonts, fills, alignment, merges, column widths, row heights and frozen panes. rust_xlsxwriter
//! writes the package, styles and plain cells; the cells it cannot express the way exceljs does are patched
//! into the worksheet XML afterwards (`patch_sheets`): error values (it has no error cells), formula results
//! typed from the computed value rather than guessed from their text (a text result "10" stays text,
//! `#CYCLE!` / `#SPILL!` / `#ERROR!` stay errors, an empty result writes no `<v>`), formula text as entered
//! (no `_xlfn.` prefixes, no dynamic-array metadata) and strings over Excel's 32,767-character limit (inline,
//! where rust_xlsxwriter would refuse the whole export). Text drops the control characters XML 1.0 cannot
//! carry, as exceljs' `xmlEncode` does.
//!
//! Import reads the OOXML package directly (zip + quick-xml): values, formulas (shared formulas expanded,
//! `_xlfn.` prefixes dropped), number formats, fonts, fills, alignment, merges, column widths, row heights
//! and frozen panes, mapped back onto the sheet model the way the export maps them out. calamine is not
//! used here because it exposes neither cell styles nor column/row sizes or panes, and it rejects the whole
//! sheet on error literals HyperFormula writes (`#CYCLE!`, `#ERROR!`, `#SPILL!`); it is still used by the
//! tests as an independent reader of the exported files.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use indexmap::IndexMap;
use oc_crdt::rng::RandomSource;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use rust_xlsxwriter::{DocProperties, ExcelDateTime, Format, FormatAlign, FormatPattern, FormatUnderline};
use serde_json::{Value, json};
use yrs::{Map, MapRef, TransactionMut};

use crate::a1::{Range, col_index, shift_formula};
use crate::engine::SheetEngine;
use crate::format::{CellValue, ValueKind};
use crate::formula::workbook::{RawContent, parse_content};
use crate::jsnum::num_to_string;
use crate::model::{
    self, Axis, CellFormat, DEFAULT_COL_W, DEFAULT_ROW_H, NumberFormat, TabSnap, add_tab, insert_tracks, set_frozen, set_merge, set_raw,
    set_track_size, snap_tab, tabs_of,
};

#[derive(Debug, thiserror::Error)]
pub enum XlsxError {
    #[error("xlsx write: {0}")]
    Write(String),
    #[error("not an xlsx package: {0}")]
    Zip(String),
    #[error("malformed xlsx xml: {0}")]
    Xml(String),
    #[error("missing part: {0}")]
    Missing(String),
}

impl From<rust_xlsxwriter::XlsxError> for XlsxError {
    fn from(e: rust_xlsxwriter::XlsxError) -> Self {
        XlsxError::Write(e.to_string())
    }
}

// ================================================================== export

/// Options of [`sheet_to_xlsx`].
#[derive(Debug, Clone)]
pub struct XlsxOptions {
    /// Workbook creator (default "Pinselo").
    pub creator: Option<String>,
    /// Creation time, ms since the Unix epoch (injected clock).
    pub created_ms: f64,
}

/// The number format exported for a cell (`numFmt`).
pub fn num_fmt(f: Option<&CellFormat>, kind: ValueKind) -> Option<String> {
    let dp = f.and_then(|x| x.dp);
    let dec = |n: f64| if n > 0.0 { format!(".{}", "0".repeat(n.trunc() as usize)) } else { String::new() };
    match f.and_then(|x| x.nf) {
        Some(NumberFormat::Number) => Some(format!("#,##0{}", dec(dp.unwrap_or(2.0)))),
        Some(NumberFormat::Percent) => Some(format!("0{}%", dec(dp.unwrap_or(0.0)))),
        Some(NumberFormat::Currency) => {
            let cur = f.and_then(|x| x.cur.clone()).unwrap_or_else(|| "$".into());
            Some(format!("\"{cur}\"#,##0{}", dec(dp.unwrap_or(2.0))))
        }
        Some(NumberFormat::Date) => Some("yyyy-mm-dd".into()),
        Some(NumberFormat::Datetime) => Some("yyyy-mm-dd hh:mm".into()),
        Some(NumberFormat::Text) => Some("@".into()),
        _ => match kind {
            ValueKind::Date => Some("yyyy-mm-dd".into()),
            ValueKind::Datetime => Some("yyyy-mm-dd hh:mm".into()),
            ValueKind::Time => Some("hh:mm".into()),
            ValueKind::Percent => Some("0%".into()),
            ValueKind::Currency => Some("#,##0.00".into()),
            _ => dp.map(|d| format!("#,##0{}", dec(d))),
        },
    }
}

/// `argb(hex)`: the first six hex digits after '#' (exceljs writes whatever is there; invalid colours are
/// skipped here).
fn rgb_of(hex: &str) -> Option<u32> {
    let h: String = hex.trim_start_matches('#').chars().take(6).collect();
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(&h, 16).ok()
}

fn cell_format(f: Option<&CellFormat>, kind: ValueKind) -> Format {
    let mut fm = Format::new();
    if let Some(nf) = num_fmt(f, kind) {
        fm = fm.set_num_format(nf);
    }
    let Some(f) = f else { return fm };
    if f.b == Some(true) {
        fm = fm.set_bold();
    }
    if f.i == Some(true) {
        fm = fm.set_italic();
    }
    if f.u == Some(true) {
        fm = fm.set_underline(FormatUnderline::Single);
    }
    if f.s == Some(true) {
        fm = fm.set_font_strikethrough();
    }
    if let Some(c) = f.color.as_deref().filter(|c| !c.is_empty()).and_then(rgb_of) {
        fm = fm.set_font_color(c);
    }
    if let Some(sz) = f.size.filter(|s| *s != 0.0 && !s.is_nan()) {
        fm = fm.set_font_size(oc_schema::js::round(sz * 0.75));
    }
    if let Some(c) = f.fill.as_deref().filter(|c| !c.is_empty()).and_then(rgb_of) {
        fm = fm.set_pattern(FormatPattern::Solid).set_background_color(c);
    }
    match f.align.as_deref() {
        Some("left") => fm = fm.set_align(FormatAlign::Left),
        Some("center") => fm = fm.set_align(FormatAlign::Center),
        Some("right") => fm = fm.set_align(FormatAlign::Right),
        _ => {}
    }
    match f.valign.as_deref() {
        Some("top") => fm = fm.set_align(FormatAlign::Top),
        Some("middle") => fm = fm.set_align(FormatAlign::VerticalCenter),
        Some("bottom") => fm = fm.set_align(FormatAlign::Bottom),
        _ => {}
    }
    if f.wrap == Some(true) {
        fm = fm.set_text_wrap();
    }
    fm
}

/// Excel's cell text limit (rust_xlsxwriter refuses longer strings).
const MAX_STRING_CHARS: usize = 32_767;

/// exceljs `xmlEncode`: the five XML specials escaped, the control characters XML 1.0 cannot carry (all of
/// U+0000–U+001F except tab, LF and CR, and U+007F) dropped.
fn xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&apos;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if is_dropped_control(c) => {}
            c => out.push(c),
        }
    }
    out
}

fn is_dropped_control(c: char) -> bool {
    c == '\u{7f}' || (c <= '\u{1f}' && !matches!(c, '\t' | '\n' | '\r'))
}

/// A string as exceljs stores it (control characters dropped).
fn clean_text(s: &str) -> std::borrow::Cow<'_, str> {
    if s.chars().any(is_dropped_control) { s.chars().filter(|c| !is_dropped_control(*c)).collect::<String>().into() } else { s.into() }
}

/// The `t` attribute and children of a cell rendered the way exceljs' `CellXform` renders it.
fn exceljs_cell(w: &CellWrite) -> String {
    let v = |x: &str| format!("<v>{}</v>", xml_text(x));
    match w {
        CellWrite::Formula(f, res) => {
            let f = format!("<f>{}</f>", xml_text(f));
            match res {
                CellValue::Null => format!(">{f}"),
                CellValue::String(s) => format!(" t=\"str\">{f}{}", v(s)),
                CellValue::Number(n) => format!(">{f}{}", v(&num_to_string(*n))),
                CellValue::Bool(b) => format!(" t=\"b\">{f}{}", v(if *b { "1" } else { "0" })),
                CellValue::Error(e) => format!(" t=\"e\">{f}{}", v(e)),
            }
        }
        CellWrite::Error(e) => format!(" t=\"e\">{}", v(e)),
        CellWrite::Str(s) => {
            let space = if s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace) || s.contains('\n') { " xml:space=\"preserve\"" } else { "" };
            format!(" t=\"inlineStr\"><is><t{space}>{}</t></is>", xml_text(s))
        }
        CellWrite::Number(n) => format!(">{}", v(&num_to_string(*n))),
        CellWrite::Bool(b) => format!(" t=\"b\">{}", v(if *b { "1" } else { "0" })),
    }
}

/// Whether rust_xlsxwriter can write the cell itself.
fn native(w: &CellWrite) -> bool {
    match w {
        CellWrite::Formula(..) | CellWrite::Error(_) => false,
        CellWrite::Str(s) => s.chars().count() <= MAX_STRING_CHARS,
        CellWrite::Number(_) | CellWrite::Bool(_) => true,
    }
}

fn utc_datetime(ms: f64) -> ExcelDateTime {
    let p = crate::format::time_parts(ms).unwrap_or(crate::format::DateParts { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0, ms: 0 });
    ExcelDateTime::from_ymd(p.year.clamp(1900, 9999) as u16, p.month as u8, p.day as u8)
        .and_then(|d| d.and_hms(p.hour as u16, p.minute as u8, f64::from(p.second)))
        .unwrap_or_else(|_| ExcelDateTime::from_ymd(1970, 1, 1).expect("valid date"))
}

/// Write the workbook: one worksheet per tab, formulas with cached results, formats, merges, sizes, panes.
pub fn sheet_to_xlsx(engine: &SheetEngine, opts: &XlsxOptions) -> Result<Vec<u8>, XlsxError> {
    let mut wb = rust_xlsxwriter::Workbook::new();
    let props = DocProperties::new()
        .set_author(opts.creator.clone().unwrap_or_else(|| "Pinselo".into()))
        .set_creation_datetime(&utc_datetime(opts.created_ms));
    wb.set_properties(&props);
    let mut fixes: Vec<HashMap<String, String>> = Vec::new();
    for t in &engine.tabs {
        let ws = wb.add_worksheet();
        let name: String = {
            let u: Vec<u16> = t.name.encode_utf16().take(31).collect();
            String::from_utf16_lossy(&u)
        };
        ws.set_name(name)?;
        if t.frozen_rows != 0.0 || t.frozen_cols != 0.0 {
            ws.set_freeze_panes(t.frozen_rows.max(0.0) as u32, t.frozen_cols.max(0.0) as u16)?;
        }
        let used = t.used_range();
        let last_c = t.col_w.keys().filter_map(|id| t.col_ix.get(id).copied()).chain(used.map(|u| u.c1)).max().unwrap_or(0);
        for c in 0..=last_c {
            if let Some(w) = t.cols.get(c).and_then(|id| t.col_w.get(id))
                && *w != DEFAULT_COL_W
            {
                // exceljs `width = max(1, w / 7)` characters; rust_xlsxwriter stores pixels and writes px / 7.
                let px = (w / 7.0).max(1.0) * 7.0;
                ws.set_column_width_pixels(c as u16, px.round().clamp(0.0, 65_535.0) as u32)?;
            }
        }
        let mut heights: Vec<(usize, f64)> = t.row_h.iter().filter_map(|(id, h)| t.row_ix.get(id).map(|r| (*r, *h))).collect();
        heights.sort_by_key(|x| x.0);
        for (r, h) in heights {
            if h != DEFAULT_ROW_H {
                ws.set_row_height(r as u32, h * 0.75)?;
            }
        }
        let mut keys: Vec<&String> = t.cells.keys().chain(t.fmts.keys().filter(|k| !t.cells.contains_key(*k))).collect();
        keys.sort();
        let mut anchors: HashMap<(usize, usize), (Format, Option<CellWrite>)> = HashMap::new();
        let mut fix: HashMap<String, String> = HashMap::new();
        for k in keys {
            let Some(p) = t.pos_of(k) else { continue };
            let raw = t.cells.get(k).map_or("", String::as_str);
            let f = t.fmts.get(k);
            let v = engine.value(&t.id, p.r, p.c);
            let kind = engine.kind(&t.id, p.r, p.c);
            let fm = cell_format(f, kind);
            let write = if raw.starts_with('=') && !f.is_some_and(CellFormat::is_text) {
                Some(CellWrite::Formula(raw[1..].to_string(), v))
            } else if !raw.is_empty() {
                match v {
                    CellValue::Null => None,
                    CellValue::Number(n) => Some(CellWrite::Number(n)),
                    CellValue::String(s) => Some(CellWrite::Str(clean_text(&s).into_owned())),
                    CellValue::Bool(b) => Some(CellWrite::Bool(b)),
                    CellValue::Error(e) => Some(CellWrite::Error(e)),
                }
            } else {
                None
            };
            write_cell(ws, p.r as u32, p.c as u16, write.as_ref(), &fm)?;
            if let Some(w) = write.as_ref().filter(|w| !native(w)) {
                fix.insert(crate::a1::a1(p.r, p.c), exceljs_cell(w));
            }
            anchors.insert((p.r, p.c), (fm, write));
        }
        for m in t.merge_ranges() {
            if m.r1 == m.r0 && m.c1 == m.c0 {
                continue;
            }
            // Covered cells take the anchor's style and lose their values (exceljs `mergeCells`).
            let (fm, write) = anchors.get(&(m.r0, m.c0)).cloned().unwrap_or_else(|| (Format::new(), None));
            if ws.merge_range(m.r0 as u32, m.c0 as u16, m.r1 as u32, m.c1 as u16, "", &fm).is_ok() {
                write_cell(ws, m.r0 as u32, m.c0 as u16, write.as_ref(), &fm)?;
                for r in m.r0..=m.r1 {
                    for c in m.c0..=m.c1 {
                        if (r, c) != (m.r0, m.c0) {
                            fix.remove(&crate::a1::a1(r, c));
                        }
                    }
                }
            }
        }
        fixes.push(fix);
    }
    patch_sheets(wb.save_to_buffer()?, &fixes)
}

/// A cell's content as exported.
#[derive(Debug, Clone)]
enum CellWrite {
    /// Formula text (without '=') and its computed value.
    Formula(String, CellValue),
    Number(f64),
    Str(String),
    Bool(bool),
    Error(String),
}

fn write_cell(ws: &mut rust_xlsxwriter::Worksheet, r: u32, c: u16, w: Option<&CellWrite>, fm: &Format) -> Result<(), XlsxError> {
    match w {
        None => {
            ws.write_blank(r, c, fm)?;
        }
        Some(CellWrite::Number(n)) => {
            ws.write_number_with_format(r, c, *n, fm)?;
        }
        Some(CellWrite::Str(s)) if s.chars().count() <= MAX_STRING_CHARS => {
            ws.write_string_with_format(r, c, s, fm)?;
        }
        Some(CellWrite::Bool(b)) => {
            ws.write_boolean_with_format(r, c, *b, fm)?;
        }
        // Patched afterwards: a placeholder keeps the cell (and its style) in the sheet XML.
        Some(_) => {
            ws.write_number_with_format(r, c, 0.0, fm)?;
        }
    }
    Ok(())
}

/// Replace the `<c>` elements named in `fixes[i]` (A1 reference → `t` attribute and children) in worksheet
/// `i + 1`, keeping each cell's style index; every other part of the package is copied unchanged.
fn patch_sheets(bytes: Vec<u8>, fixes: &[HashMap<String, String>]) -> Result<Vec<u8>, XlsxError> {
    use std::io::Write;
    if fixes.iter().all(HashMap::is_empty) {
        return Ok(bytes);
    }
    let zerr = |e: zip::result::ZipError| XlsxError::Zip(e.to_string());
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(zerr)?;
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(zerr)?;
        let name = f.name().to_string();
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(f.compression())
            .last_modified_time(zip::DateTime::default());
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(|e| XlsxError::Zip(e.to_string()))?;
        let sheet = name.strip_prefix("xl/worksheets/sheet").and_then(|x| x.strip_suffix(".xml")).and_then(|n| n.parse::<usize>().ok());
        if let Some(fx) = sheet.and_then(|n| n.checked_sub(1)).and_then(|i| fixes.get(i)).filter(|fx| !fx.is_empty()) {
            data = patch_sheet_xml(&data, fx)?;
        }
        w.start_file(name, opts).map_err(zerr)?;
        w.write_all(&data).map_err(|e| XlsxError::Zip(e.to_string()))?;
    }
    Ok(w.finish().map_err(zerr)?.into_inner())
}

fn patch_sheet_xml(xml: &[u8], fixes: &HashMap<String, String>) -> Result<Vec<u8>, XlsxError> {
    let xerr = |e: quick_xml::Error| XlsxError::Xml(e.to_string());
    let mut rd = Reader::from_reader(xml);
    let mut out = quick_xml::Writer::new(Vec::with_capacity(xml.len() + 64 * fixes.len()));
    let mut buf = Vec::new();
    let mut skip = false;
    loop {
        buf.clear();
        let ev = rd.read_event_into(&mut buf).map_err(xerr)?;
        if skip {
            if matches!(&ev, Event::End(e) if e.local_name().as_ref() == b"c") {
                skip = false;
            }
            continue;
        }
        let replaced = match &ev {
            Event::Start(e) | Event::Empty(e) if local(e) == b"c" => attr(e, "r").and_then(|r| fixes.get(&r).map(|fx| (r, attr(e, "s"), fx))),
            _ => None,
        };
        if let Some((r, style, fx)) = replaced {
            let style = style.map_or_else(String::new, |s| format!(" s=\"{s}\""));
            out.get_mut().extend_from_slice(format!("<c r=\"{r}\"{style}{fx}</c>").as_bytes());
            skip = matches!(ev, Event::Start(_));
            continue;
        }
        if matches!(ev, Event::Eof) {
            break;
        }
        out.write_event(ev).map_err(|e| XlsxError::Xml(e.to_string()))?;
    }
    Ok(out.into_inner())
}

// ================================================================== OOXML reading

fn unzip(bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>, XlsxError> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| XlsxError::Zip(e.to_string()))?;
    let mut out = HashMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| XlsxError::Zip(e.to_string()))?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().trim_start_matches('/').to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| XlsxError::Zip(e.to_string()))?;
        out.insert(name, buf);
    }
    Ok(out)
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    for a in e.attributes().flatten() {
        let key = a.key.as_ref();
        let local = key.rsplit(|b| *b == b':').next().unwrap_or(key);
        if local == name.as_bytes() || key == name.as_bytes() {
            return a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned());
        }
    }
    None
}

fn local(e: &BytesStart) -> Vec<u8> {
    e.local_name().as_ref().to_vec()
}

/// Decode Excel's `_xHHHH_` escapes.
fn decode_x(s: &str) -> String {
    if !s.contains("_x") {
        return s.to_string();
    }
    let b: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == '_' && i + 6 < b.len() && b[i + 1] == 'x' && b[i + 6] == '_' {
            let hex: String = b[i + 2..i + 6].iter().collect();
            if let Ok(v) = u32::from_str_radix(&hex, 16)
                && let Some(ch) = char::from_u32(v)
            {
                out.push(ch);
                i += 7;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Collect the text content until the end of the current element (`depth` counts nested elements).
fn read_text(rd: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> Result<String, XlsxError> {
    let mut out = String::new();
    let mut depth = 1;
    loop {
        buf.clear();
        match rd.read_event_into(buf).map_err(|e| XlsxError::Xml(e.to_string()))? {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(out);
                }
            }
            Event::Text(t) => out.push_str(&t.xml10_content().map_err(|e| XlsxError::Xml(e.to_string()))?),
            Event::CData(t) => out.push_str(&String::from_utf8_lossy(&t)),
            Event::GeneralRef(r) => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    out.push(c);
                } else {
                    let name = String::from_utf8_lossy(&r);
                    out.push_str(match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Event::Eof => return Ok(out),
            _ => {}
        }
    }
}

/// Text of `<t>` elements under the current element (rich text runs concatenated, phonetic runs skipped).
fn read_si(rd: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> Result<String, XlsxError> {
    let mut out = String::new();
    let mut depth = 1;
    let mut skip = 0;
    loop {
        buf.clear();
        let ev = rd.read_event_into(buf).map_err(|e| XlsxError::Xml(e.to_string()))?;
        match ev {
            Event::Start(e) => {
                let n = local(&e);
                if n == b"rPh" || n == b"phoneticPr" {
                    skip += 1;
                    depth += 1;
                } else if n == b"t" && skip == 0 {
                    let mut b2 = Vec::new();
                    out.push_str(&read_text(rd, &mut b2)?);
                } else {
                    depth += 1;
                }
            }
            Event::End(e) => {
                let n = e.local_name().as_ref().to_vec();
                if n == b"rPh" || n == b"phoneticPr" {
                    skip -= 1;
                }
                depth -= 1;
                if depth == 0 {
                    return Ok(decode_x(&out));
                }
            }
            Event::Eof => return Ok(decode_x(&out)),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Default)]
struct FontInfo {
    b: bool,
    i: bool,
    u: bool,
    s: bool,
    size: Option<f64>,
    rgb: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Xf {
    num_fmt: u32,
    font: usize,
    fill: usize,
    h: Option<String>,
    v: Option<String>,
    wrap: bool,
}

#[derive(Debug, Clone, Default)]
struct Styles {
    num_fmts: HashMap<u32, String>,
    fonts: Vec<FontInfo>,
    fills: Vec<Option<String>>,
    xfs: Vec<Xf>,
}

fn rgb_attr(e: &BytesStart) -> Option<String> {
    let v = attr(e, "rgb")?;
    let h = if v.len() == 8 { &v[2..] } else { v.as_str() };
    (h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| format!("#{}", h.to_lowercase()))
}

fn parse_styles(xml: &[u8]) -> Result<Styles, XlsxError> {
    let mut st = Styles::default();
    let mut rd = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut section: Vec<u8> = Vec::new();
    let mut font: Option<FontInfo> = None;
    let mut fill: Option<Option<String>> = None;
    let mut xf: Option<Xf> = None;
    let mut in_pattern = false;
    loop {
        buf.clear();
        let ev = rd.read_event_into(&mut buf).map_err(|e| XlsxError::Xml(e.to_string()))?;
        let (e, empty) = match &ev {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            Event::End(e) => {
                let n = e.local_name().as_ref().to_vec();
                match n.as_slice() {
                    b"numFmts" | b"fonts" | b"fills" | b"cellXfs" | b"cellStyleXfs" | b"dxfs" => section.clear(),
                    b"font" if section == b"fonts" => {
                        if let Some(f) = font.take() {
                            st.fonts.push(f);
                        }
                    }
                    b"fill" if section == b"fills" => {
                        st.fills.push(fill.take().unwrap_or(None));
                    }
                    b"patternFill" => in_pattern = false,
                    b"xf" if section == b"cellXfs" => {
                        if let Some(x) = xf.take() {
                            st.xfs.push(x);
                        }
                    }
                    _ => {}
                }
                (None, false)
            }
            Event::Eof => break,
            _ => (None, false),
        };
        let Some(e) = e else { continue };
        let n = local(&e);
        match n.as_slice() {
            b"numFmts" | b"fonts" | b"fills" | b"cellXfs" | b"cellStyleXfs" | b"dxfs" => {
                if !empty {
                    section = n.clone();
                }
            }
            b"numFmt" => {
                if let (Some(id), Some(code)) = (attr(&e, "numFmtId"), attr(&e, "formatCode"))
                    && let Ok(id) = id.parse()
                {
                    st.num_fmts.insert(id, code);
                }
            }
            b"font" if section == b"fonts" => {
                if empty {
                    st.fonts.push(FontInfo::default());
                } else {
                    font = Some(FontInfo::default());
                }
            }
            b"b" | b"i" | b"u" | b"strike" | b"sz" | b"color" if font.is_some() => {
                let f = font.as_mut().expect("font");
                let on = attr(&e, "val").is_none_or(|v| v != "0" && v != "false" && v != "none");
                match n.as_slice() {
                    b"b" => f.b = on,
                    b"i" => f.i = on,
                    b"u" => f.u = on,
                    b"strike" => f.s = on,
                    b"sz" => f.size = attr(&e, "val").and_then(|v| v.parse().ok()),
                    _ => f.rgb = rgb_attr(&e),
                }
            }
            b"fill" if section == b"fills" => {
                if empty {
                    st.fills.push(None);
                } else {
                    fill = Some(None);
                }
            }
            b"patternFill" if fill.is_some() => {
                in_pattern = attr(&e, "patternType").as_deref() == Some("solid");
                if empty {
                    in_pattern = false;
                }
            }
            b"fgColor" if fill.is_some() && in_pattern => {
                fill = Some(rgb_attr(&e));
            }
            b"xf" if section == b"cellXfs" => {
                let x = Xf {
                    num_fmt: attr(&e, "numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0),
                    font: attr(&e, "fontId").and_then(|v| v.parse().ok()).unwrap_or(0),
                    fill: attr(&e, "fillId").and_then(|v| v.parse().ok()).unwrap_or(0),
                    ..Xf::default()
                };
                if empty {
                    st.xfs.push(x);
                } else {
                    xf = Some(x);
                }
            }
            b"alignment" if xf.is_some() => {
                let x = xf.as_mut().expect("xf");
                x.h = attr(&e, "horizontal");
                x.v = attr(&e, "vertical");
                x.wrap = attr(&e, "wrapText").is_some_and(|v| v == "1" || v == "true");
            }
            _ => {}
        }
    }
    Ok(st)
}

fn builtin_fmt(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "mm-dd-yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        49 => "@",
        _ => return None,
    })
}

impl Styles {
    fn code(&self, xf: Option<&Xf>) -> Option<String> {
        let id = xf?.num_fmt;
        self.num_fmts.get(&id).cloned().or_else(|| builtin_fmt(id).map(str::to_string))
    }
}

/// Date/time class of a number format: (has date part, has time part).
fn date_class(code: &str) -> (bool, bool) {
    // Strip quoted text, escapes and colour/condition brackets (but keep elapsed-time [h]).
    let mut s = String::new();
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                for d in chars.by_ref() {
                    if d == '"' {
                        break;
                    }
                }
            }
            '\\' => {
                chars.next();
            }
            '[' => {
                let mut inner = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    inner.push(d);
                }
                if matches!(inner.to_lowercase().as_str(), "h" | "hh" | "m" | "mm" | "s" | "ss") {
                    s.push_str(&inner);
                }
            }
            _ => s.push(c.to_ascii_lowercase()),
        }
    }
    let s = s.split(';').next().unwrap_or("").to_string();
    let date = s.contains('y') || s.contains('d');
    let time = s.contains('h') || s.contains('s') || (!date && s.contains('m') && !s.contains('0') && !s.contains('#'));
    (date, time)
}

#[derive(Debug, Clone, Default)]
struct RawCell {
    r: u32,
    c: u32,
    style: usize,
    t: Option<String>,
    v: Option<String>,
    f: Option<String>,
    f_shared: Option<(String, Option<String>)>,
    inline: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RawSheet {
    cells: Vec<RawCell>,
    cols: Vec<(u32, u32, f64)>,
    rows: Vec<(u32, f64)>,
    merges: Vec<String>,
    pane: Option<(f64, f64)>,
    /// Highest row index with a `<row>` element.
    last_row: Option<u32>,
}

/// "B3" → (row, col), 0-based.
fn cell_ref(s: &str) -> Option<(u32, u32)> {
    let s = s.replace('$', "");
    let split = s.find(|c: char| c.is_ascii_digit())?;
    let c = col_index(&s[..split])? as u32;
    let r: u32 = s[split..].parse().ok()?;
    (r > 0).then_some((r - 1, c))
}

fn parse_sheet(xml: &[u8]) -> Result<RawSheet, XlsxError> {
    let mut sh = RawSheet::default();
    let mut rd = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut cur: Option<RawCell> = None;
    let mut next_row: u32 = 0;
    let mut next_col: u32 = 0;
    loop {
        buf.clear();
        let ev = rd.read_event_into(&mut buf).map_err(|e| XlsxError::Xml(e.to_string()))?;
        match ev {
            Event::Start(e) | Event::Empty(e) if local(&e) == b"row" => {
                let r = attr(&e, "r").and_then(|v| v.parse::<u32>().ok()).map_or(next_row, |r| r.saturating_sub(1));
                next_row = r + 1;
                next_col = 0;
                sh.last_row = sh.last_row.max(Some(r));
                if let Some(ht) = attr(&e, "ht").and_then(|v| v.parse::<f64>().ok())
                    && attr(&e, "customHeight").is_some_and(|v| v == "1" || v == "true")
                {
                    sh.rows.push((r, ht));
                }
            }
            Event::Start(e) if local(&e) == b"c" => {
                let (r, c) = attr(&e, "r").and_then(|x| cell_ref(&x)).unwrap_or((next_row.saturating_sub(1), next_col));
                next_col = c + 1;
                cur = Some(RawCell {
                    r,
                    c,
                    style: attr(&e, "s").and_then(|v| v.parse().ok()).unwrap_or(0),
                    t: attr(&e, "t"),
                    ..RawCell::default()
                });
            }
            Event::Empty(e) if local(&e) == b"c" => {
                let (r, c) = attr(&e, "r").and_then(|x| cell_ref(&x)).unwrap_or((next_row.saturating_sub(1), next_col));
                next_col = c + 1;
                sh.cells.push(RawCell { r, c, style: attr(&e, "s").and_then(|v| v.parse().ok()).unwrap_or(0), t: attr(&e, "t"), ..RawCell::default() });
            }
            Event::End(e) if e.local_name().as_ref() == b"c" => {
                if let Some(c) = cur.take() {
                    sh.cells.push(c);
                }
            }
            Event::Start(e) if local(&e) == b"v" && cur.is_some() => {
                let mut b2 = Vec::new();
                let t = read_text(&mut rd, &mut b2)?;
                cur.as_mut().expect("cell").v = Some(t);
            }
            Event::Start(e) if local(&e) == b"f" && cur.is_some() => {
                let shared = attr(&e, "t").as_deref() == Some("shared");
                let si = attr(&e, "si");
                let mut b2 = Vec::new();
                let t = read_text(&mut rd, &mut b2)?;
                let c = cur.as_mut().expect("cell");
                if shared {
                    c.f_shared = Some((si.unwrap_or_default(), (!t.is_empty()).then_some(t)));
                } else {
                    c.f = Some(t);
                }
            }
            Event::Empty(e) if local(&e) == b"f" && cur.is_some() => {
                if attr(&e, "t").as_deref() == Some("shared") {
                    cur.as_mut().expect("cell").f_shared = Some((attr(&e, "si").unwrap_or_default(), None));
                }
            }
            Event::Start(e) if local(&e) == b"is" && cur.is_some() => {
                let mut b2 = Vec::new();
                let t = read_si(&mut rd, &mut b2)?;
                cur.as_mut().expect("cell").inline = Some(t);
            }
            Event::Start(e) | Event::Empty(e) if local(&e) == b"col" => {
                let min = attr(&e, "min").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
                let max = attr(&e, "max").and_then(|v| v.parse::<u32>().ok()).unwrap_or(min);
                if let Some(w) = attr(&e, "width").and_then(|v| v.parse::<f64>().ok())
                    && attr(&e, "customWidth").is_none_or(|v| v == "1" || v == "true")
                {
                    sh.cols.push((min.saturating_sub(1), max.saturating_sub(1).min(16_383), w));
                }
            }
            Event::Start(e) | Event::Empty(e) if local(&e) == b"mergeCell" => {
                if let Some(r) = attr(&e, "ref") {
                    sh.merges.push(r);
                }
            }
            Event::Start(e) | Event::Empty(e) if local(&e) == b"pane" => {
                if matches!(attr(&e, "state").as_deref(), Some("frozen") | Some("frozenSplit")) {
                    let x = attr(&e, "xSplit").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let y = attr(&e, "ySplit").and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    sh.pane = Some((x, y));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(sh)
}

fn resolve_target(base: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn parse_rels(xml: &[u8]) -> Result<HashMap<String, (String, String)>, XlsxError> {
    let mut out = HashMap::new();
    let mut rd = Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match rd.read_event_into(&mut buf).map_err(|e| XlsxError::Xml(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if local(&e) == b"Relationship" => {
                if let (Some(id), Some(t)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    out.insert(id, (t, attr(&e, "Type").unwrap_or_default()));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// One worksheet as read from the package.
#[derive(Debug, Clone, Default)]
pub struct XlsxSheet {
    pub name: String,
    /// (row, col) → cell
    pub cells: Vec<XlsxCell>,
    /// (col, width in characters)
    pub col_widths: Vec<(u32, f64)>,
    /// (row, height in points)
    pub row_heights: Vec<(u32, f64)>,
    pub merges: Vec<Range>,
    pub frozen_rows: f64,
    pub frozen_cols: f64,
    /// Highest row index that has a `<row>` element (rows can exist without cells, e.g. for a height).
    pub last_row: Option<u32>,
}

/// A cell: its value (formula result for formula cells — `Null` when the file has none), formula text
/// (without `=`; shared formulas expanded, `_xlfn.` prefixes dropped), the formula text exactly as stored
/// (`None` for a shared-formula child), and style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct XlsxCell {
    pub r: u32,
    pub c: u32,
    pub value: CellValue,
    pub formula: Option<String>,
    pub formula_text: Option<String>,
    pub num_fmt: Option<String>,
    pub format: CellFormat,
}

/// Drop `_xlfn.` / `_xlws.` future-function prefixes outside string literals.
pub fn strip_future_prefixes(f: &str) -> String {
    let mut out = String::with_capacity(f.len());
    let mut in_str = false;
    let mut i = 0;
    let b = f.as_bytes();
    while i < b.len() {
        if b[i] == b'"' {
            in_str = !in_str;
        }
        if !in_str {
            if f[i..].starts_with("_xlfn.") {
                i += 6;
                continue;
            }
            if f[i..].starts_with("_xlws.") {
                i += 6;
                continue;
            }
        }
        let ch = f[i..].chars().next().expect("char");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn style_format(st: &Styles, xf: Option<&Xf>, code: Option<&str>) -> CellFormat {
    let mut f = CellFormat::default();
    if let Some(code) = code {
        let lower = code.to_lowercase();
        let zeros = |s: &str| -> Option<f64> {
            match s.strip_prefix('.') {
                None if s.is_empty() => Some(0.0),
                Some(z) if !z.is_empty() && z.bytes().all(|b| b == b'0') => Some(z.len() as f64),
                _ => None,
            }
        };
        if code == "@" {
            f.nf = Some(NumberFormat::Text);
        } else if let Some(rest) = lower.strip_prefix('0').and_then(|r| r.strip_suffix('%')) {
            if let Some(dp) = zeros(rest) {
                f.nf = Some(NumberFormat::Percent);
                f.dp = Some(dp);
            }
        } else if let Some(rest) = code.strip_prefix('"').and_then(|r| r.split_once("\"#,##0")) {
            if let Some(dp) = zeros(rest.1) {
                f.nf = Some(NumberFormat::Currency);
                f.dp = Some(dp);
                f.cur = Some(rest.0.to_string());
            }
        } else if let Some(rest) = code.strip_prefix("#,##0")
            && let Some(dp) = zeros(rest)
        {
            f.nf = Some(NumberFormat::Number);
            f.dp = Some(dp);
        } else {
            // Dates (`yyyy-mm-dd`, what the export writes for `nf: "date"`) and date-times; time-only formats
            // are left to the typed input ("10:30" reads back as a time).
            match date_class(code) {
                (true, false) => f.nf = Some(NumberFormat::Date),
                (true, true) => f.nf = Some(NumberFormat::Datetime),
                _ => {}
            }
        }
    }
    let Some(xf) = xf else { return f };
    if let Some(font) = st.fonts.get(xf.font).filter(|_| xf.font > 0 || st.fonts.len() == 1) {
        if font.b {
            f.b = Some(true);
        }
        if font.i {
            f.i = Some(true);
        }
        if font.u {
            f.u = Some(true);
        }
        if font.s {
            f.s = Some(true);
        }
        if let Some(c) = &font.rgb
            && c != "#000000"
        {
            f.color = Some(c.clone());
        }
        // 11pt is the default font size of every writer: treat it as "not set".
        if let Some(sz) = font.size
            && sz != 11.0
        {
            f.size = Some(oc_schema::js::round(sz / 0.75));
        }
    }
    if let Some(Some(fill)) = st.fills.get(xf.fill) {
        f.fill = Some(fill.clone());
    }
    match xf.h.as_deref() {
        Some("left") => f.align = Some("left".into()),
        Some("center") | Some("centerContinuous") => f.align = Some("center".into()),
        Some("right") => f.align = Some("right".into()),
        _ => {}
    }
    match xf.v.as_deref() {
        Some("top") => f.valign = Some("top".into()),
        Some("center") => f.valign = Some("middle".into()),
        Some("bottom") => f.valign = Some("bottom".into()),
        _ => {}
    }
    if xf.wrap {
        f.wrap = Some(true);
    }
    f
}

fn parse_range_ref(s: &str) -> Option<Range> {
    let (a, b) = s.split_once(':').unwrap_or((s, s));
    let (r0, c0) = cell_ref(a)?;
    let (r1, c1) = cell_ref(b)?;
    Some(Range { r0: r0.min(r1) as usize, c0: c0.min(c1) as usize, r1: r0.max(r1) as usize, c1: c0.max(c1) as usize })
}

/// Read every worksheet of an XLSX package.
pub fn read_workbook(bytes: &[u8]) -> Result<Vec<XlsxSheet>, XlsxError> {
    let files = unzip(bytes)?;
    let wb_path = {
        let rels = files.get("_rels/.rels").map(|x| parse_rels(x)).transpose()?.unwrap_or_default();
        rels.values().find(|(_, ty)| ty.ends_with("/officeDocument")).map_or_else(|| "xl/workbook.xml".to_string(), |(t, _)| resolve_target("", t))
    };
    let wb_xml = files.get(&wb_path).ok_or_else(|| XlsxError::Missing(wb_path.clone()))?;
    let rels_path = {
        let (dir, file) = wb_path.rsplit_once('/').unwrap_or(("", wb_path.as_str()));
        if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") }
    };
    let rels = files.get(&rels_path).map(|x| parse_rels(x)).transpose()?.unwrap_or_default();
    // Sheets + 1904 flag.
    let mut sheets: Vec<(String, String)> = Vec::new();
    let mut date1904 = false;
    {
        let mut rd = Reader::from_reader(wb_xml.as_slice());
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match rd.read_event_into(&mut buf).map_err(|e| XlsxError::Xml(e.to_string()))? {
                Event::Start(e) | Event::Empty(e) if local(&e) == b"sheet" => {
                    if let (Some(n), Some(id)) = (attr(&e, "name"), attr(&e, "id")) {
                        sheets.push((n, id));
                    }
                }
                Event::Start(e) | Event::Empty(e) if local(&e) == b"workbookPr" => {
                    date1904 = attr(&e, "date1904").is_some_and(|v| v == "1" || v == "true");
                }
                Event::Eof => break,
                _ => {}
            }
        }
    }
    let target_of = |ty: &str, default: &str| -> String {
        rels.values().find(|(_, t)| t.ends_with(ty)).map_or_else(|| default.to_string(), |(t, _)| resolve_target(&wb_path, t))
    };
    let mut shared: Vec<String> = Vec::new();
    if let Some(xml) = files.get(&target_of("/sharedStrings", "xl/sharedStrings.xml")) {
        let mut rd = Reader::from_reader(xml.as_slice());
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match rd.read_event_into(&mut buf).map_err(|e| XlsxError::Xml(e.to_string()))? {
                Event::Start(e) if local(&e) == b"si" => {
                    let mut b2 = Vec::new();
                    shared.push(read_si(&mut rd, &mut b2)?);
                }
                Event::Empty(e) if local(&e) == b"si" => shared.push(String::new()),
                Event::Eof => break,
                _ => {}
            }
        }
    }
    let styles = match files.get(&target_of("/styles", "xl/styles.xml")) {
        Some(x) => parse_styles(x)?,
        None => Styles::default(),
    };
    let mut out = Vec::new();
    for (name, rid) in sheets {
        let Some((target, _)) = rels.get(&rid) else { continue };
        let path = resolve_target(&wb_path, target);
        let Some(xml) = files.get(&path) else { continue };
        let raw = parse_sheet(xml)?;
        let mut shared_f: HashMap<String, (u32, u32, String)> = HashMap::new();
        let mut cells = Vec::with_capacity(raw.cells.len());
        for rc in raw.cells {
            let xf = styles.xfs.get(rc.style);
            let code = styles.code(xf);
            let formula_text = rc.f.clone().or_else(|| rc.f_shared.as_ref().and_then(|x| x.1.clone()));
            let formula = match (&rc.f, &rc.f_shared) {
                (Some(f), _) => Some(f.clone()),
                (None, Some((si, Some(f)))) => {
                    shared_f.insert(si.clone(), (rc.r, rc.c, f.clone()));
                    Some(f.clone())
                }
                (None, Some((si, None))) => shared_f.get(si).map(|(r0, c0, f)| {
                    let shifted = shift_formula(&format!("={f}"), i64::from(rc.r) - i64::from(*r0), i64::from(rc.c) - i64::from(*c0));
                    shifted[1..].to_string()
                }),
                _ => None,
            }
            .map(|f| strip_future_prefixes(&f));
            let (is_date, _) = code.as_deref().map_or((false, false), date_class);
            let value = match rc.t.as_deref() {
                Some("s") => rc.v.as_deref().and_then(|v| v.trim().parse::<usize>().ok()).and_then(|i| shared.get(i)).map_or(CellValue::Null, |s| CellValue::String(s.clone())),
                Some("inlineStr") => rc.inline.clone().map_or(CellValue::Null, CellValue::String),
                // `_xHHHH_` escapes belong to shared and inline strings only.
                Some("str") => CellValue::String(rc.v.clone().unwrap_or_default()),
                // `parseInt(v, 10) !== 0`
                Some("b") => match rc.v.as_deref() {
                    Some(v) => CellValue::Bool(oc_schema::js::parse_int10(v) != 0.0),
                    None => CellValue::Null,
                },
                Some("e") => rc.v.clone().map_or(CellValue::Null, CellValue::Error),
                Some("d") => rc.v.as_deref().map_or(CellValue::Null, |v| iso_to_serial(v).map_or(CellValue::String(v.to_string()), CellValue::Number)),
                _ => match rc.v.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                    Some(v) => {
                        let n = crate::jsnum::parse_float(v);
                        CellValue::Number(if is_date && date1904 { n + 1462.0 } else { n })
                    }
                    None => CellValue::Null,
                },
            };
            // A formula's cached result counts only when `<v>` has text (exceljs: `if (model.value)`).
            let value = if formula.is_some() && rc.v.as_deref().is_none_or(str::is_empty) { CellValue::Null } else { value };
            let format = style_format(&styles, xf, code.as_deref());
            cells.push(XlsxCell { r: rc.r, c: rc.c, value, formula, formula_text, num_fmt: code, format });
        }
        let mut col_widths = Vec::new();
        for (a, b, w) in raw.cols {
            for c in a..=b.min(a + 1024) {
                col_widths.push((c, w));
            }
        }
        out.push(XlsxSheet {
            name,
            cells,
            col_widths,
            row_heights: raw.rows,
            merges: raw.merges.iter().filter_map(|m| parse_range_ref(m)).collect(),
            frozen_rows: raw.pane.map_or(0.0, |p| p.1),
            frozen_cols: raw.pane.map_or(0.0, |p| p.0),
            last_row: raw.last_row,
        });
    }
    Ok(out)
}

fn iso_to_serial(s: &str) -> Option<f64> {
    let (d, t) = s.split_once('T').unwrap_or((s, ""));
    let mut it = d.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let day: u32 = it.next()?.parse().ok()?;
    let days = crate::format::days_from_civil(y, m, day);
    let mut frac = 0.0;
    if !t.is_empty() {
        let t = t.trim_end_matches('Z');
        let mut p = t.split(':');
        let h: f64 = p.next()?.parse().ok()?;
        let mi: f64 = p.next().unwrap_or("0").parse().ok()?;
        let sec: f64 = p.next().unwrap_or("0").parse().ok()?;
        frac = (h * 3600.0 + mi * 60.0 + sec) / 86400.0;
    }
    Some(days as f64 + 25569.0 + frac)
}

/// exceljs `isDateFmt`: after removing `[…]` sections and quoted text, any of `y m d h M s b` remains.
pub fn exceljs_is_date_fmt(code: &str) -> bool {
    let mut rest = String::with_capacity(code.len());
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        let close = match c {
            '[' => ']',
            '"' => '"',
            _ => {
                rest.push(c);
                continue;
            }
        };
        // `/\[[^\]]*]/g` and `/"[^"]*"/g` only remove closed sections.
        let section: String = chars.clone().take_while(|d| *d != close).collect();
        if chars.clone().nth(section.chars().count()) == Some(close) {
            for _ in 0..=section.chars().count() {
                chars.next();
            }
        } else {
            rest.push(c);
        }
    }
    rest.contains(['y', 'm', 'd', 'h', 'M', 's', 'b'])
}

/// exceljs `excelToDate(v)` as the generator's JSON writes a `Date`: `{ date: ms }`, `{ date: null }` when
/// invalid. `v` goes through JS `Number()` first (`v - 25569`).
fn exceljs_date(v: &CellValue) -> Value {
    let n = match v {
        CellValue::Number(n) => *n,
        CellValue::String(s) => oc_schema::js::string_to_number(s),
        CellValue::Bool(b) => f64::from(u8::from(*b)),
        CellValue::Null => 0.0,
        CellValue::Error(_) => f64::NAN,
    };
    let ms = oc_schema::js::round((n - 25569.0) * 86_400_000.0);
    let mut out = if ms.is_finite() && ms.abs() <= 8.64e15 { json!({ "date": ms }) } else { json!({ "date": null }) };
    oc_schema::js::normalize_numbers(&mut out);
    out
}

/// A cell's `cell.value` as exceljs reads it.
fn exceljs_value(c: &XlsxCell) -> Value {
    let date = c.num_fmt.as_deref().is_some_and(exceljs_is_date_fmt);
    if let Some(f) = c.formula_text.as_ref().or(c.formula.as_ref()) {
        let mut o = serde_json::Map::new();
        o.insert("formula".into(), Value::String(f.clone()));
        // `cell.value` copies the result only when truthy (`_copyModel`): 0, "" and FALSE are dropped; a
        // result under a date format is a `Date` by then (always kept).
        let truthy = match &c.value {
            CellValue::Null => false,
            CellValue::Number(n) => *n != 0.0 && !n.is_nan(),
            CellValue::String(s) => !s.is_empty(),
            CellValue::Bool(b) => *b,
            CellValue::Error(_) => true,
        };
        if !matches!(c.value, CellValue::Null) && date {
            o.insert("result".into(), exceljs_date(&c.value));
        } else if truthy {
            o.insert("result".into(), c.value.to_json());
        }
        return Value::Object(o);
    }
    match &c.value {
        CellValue::Number(_) if date => exceljs_date(&c.value),
        v => v.to_json(),
    }
}

/// The values of every worksheet as `@oc/sheet/xlsx`'s `readXlsx` (exceljs) returns them: tab name → rows of
/// cell values — numbers, strings, booleans, `null`, `{ formula, result }`, `{ error }`, and `{ date }` for
/// numbers (and formula results) under a date format. Cells covered by a merge show the merge's value; every
/// row up to the last `<row>` / cell / merge is present.
///
/// Deviations: rich text reads as its plain text (exceljs: `{ richText }`) and a shared-formula child as its
/// expanded formula (exceljs: `{ sharedFormula }`); neither writer here produces them.
pub fn read_xlsx(bytes: &[u8]) -> Result<IndexMap<String, Vec<Vec<Value>>>, XlsxError> {
    use std::collections::BTreeMap;
    let mut out = IndexMap::new();
    for s in read_workbook(bytes)? {
        let mut grid: BTreeMap<u32, BTreeMap<u32, Value>> = BTreeMap::new();
        for c in &s.cells {
            grid.entry(c.r).or_default().insert(c.c, exceljs_value(c));
        }
        for m in &s.merges {
            let (r0, c0) = (m.r0 as u32, m.c0 as u32);
            let master = grid.get(&r0).and_then(|row| row.get(&c0)).cloned().unwrap_or(Value::Null);
            for r in r0..=m.r1 as u32 {
                for c in c0..=m.c1 as u32 {
                    grid.entry(r).or_default().insert(c, master.clone());
                }
            }
        }
        let n_rows = grid.keys().next_back().map(|r| r + 1).max(s.last_row.map(|r| r + 1)).unwrap_or(0);
        let rows = (0..n_rows)
            .map(|r| match grid.get(&r) {
                None => Vec::new(),
                Some(row) => {
                    let w = row.keys().next_back().map_or(0, |c| c + 1);
                    (0..w).map(|c| row.get(&c).cloned().unwrap_or(Value::Null)).collect()
                }
            })
            .collect();
        out.insert(s.name, rows);
    }
    Ok(out)
}

// ================================================================== import into the sheet model

fn pad2(n: u32) -> String {
    format!("{n:02}")
}

/// A date/time serial as an input string the content parser reads back as the same kind.
fn serial_input(n: f64, date: bool, time: bool) -> String {
    let ms = crate::format::serial_to_time(n);
    let Some(p) = crate::format::time_parts(ms) else { return num_to_string(n) };
    let d = format!("{}-{}-{}", p.year, pad2(p.month), pad2(p.day));
    let t = if p.second != 0 {
        format!("{}:{}:{}", pad2(p.hour), pad2(p.minute), pad2(p.second))
    } else {
        format!("{}:{}", pad2(p.hour), pad2(p.minute))
    };
    let whole = n.fract() == 0.0;
    match (date, time) {
        (true, false) if whole => d,
        (false, true) if (0.0..1.0).contains(&n) => t,
        _ => format!("{d} {t}"),
    }
}

/// The raw input a cell value is imported as (formulas as `=…`, dates as date strings, strings that would
/// otherwise parse as something else behind an apostrophe).
pub fn import_raw(cell: &XlsxCell) -> String {
    if let Some(f) = &cell.formula {
        return format!("={f}");
    }
    let text = cell.format.is_text();
    match &cell.value {
        CellValue::Null => String::new(),
        CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        CellValue::Error(e) => e.clone(),
        CellValue::Number(n) => {
            let (date, time) = cell.num_fmt.as_deref().map_or((false, false), date_class);
            if (date || time) && !text && n.is_finite() && *n >= 0.0 {
                serial_input(*n, date, time)
            } else {
                num_to_string(*n)
            }
        }
        CellValue::String(s) => {
            if text || s.is_empty() {
                return s.clone();
            }
            let same = matches!(parse_content(s), Some(RawContent::Value(crate::formula::value::Scalar::Str(ref x))) if &**x == s.as_str());
            if same { s.clone() } else { format!("'{s}") }
        }
    }
}

/// Import every worksheet of an XLSX package as new tabs (after the existing ones). Returns the new tab ids.
pub fn import_xlsx(txn: &mut TransactionMut, root: &MapRef, bytes: &[u8], rng: &mut dyn RandomSource) -> Result<Vec<String>, XlsxError> {
    let sheets = read_workbook(bytes)?;
    if tabs_of(txn, root).is_none() {
        model::init_sheet_txn(txn, root, &model::InitOpts::default(), rng);
    }
    let mut ids = Vec::new();
    for s in sheets {
        let id = add_tab(txn, root, Some(&s.name), None, rng);
        let tabs = tabs_of(txn, root).expect("tabs");
        let Some(yrs::Out::YMap(map)) = tabs.get(txn, &id) else { continue };
        let mut t: TabSnap = snap_tab(txn, &id, &map);
        let max_r = s.cells.iter().map(|c| c.r as usize + 1).chain(s.merges.iter().map(|m| m.r1 + 1)).chain(s.row_heights.iter().map(|x| x.0 as usize + 1)).max().unwrap_or(0);
        let max_c = s.cells.iter().map(|c| c.c as usize + 1).chain(s.merges.iter().map(|m| m.c1 + 1)).chain(s.col_widths.iter().map(|x| x.0 as usize + 1)).max().unwrap_or(0);
        let max_r = max_r.min(model::MAX_ROWS);
        let max_c = max_c.min(model::MAX_COLS);
        if max_r > t.rows.len() {
            insert_tracks(txn, &t, Axis::Rows, t.rows.len(), max_r - t.rows.len(), rng);
        }
        if max_c > t.cols.len() {
            insert_tracks(txn, &t, Axis::Cols, t.cols.len(), max_c - t.cols.len(), rng);
        }
        t = snap_tab(txn, &id, &map);
        for c in &s.cells {
            let (r, col) = (c.r as usize, c.c as usize);
            let raw = import_raw(c);
            if !raw.is_empty() {
                set_raw(txn, &t, r, col, &raw);
            }
            if c.format != CellFormat::default() {
                model::patch_format_typed(txn, &t, r, col, &c.format);
            }
        }
        for (c, w) in &s.col_widths {
            let px = oc_schema::js::round(w * 7.0);
            if (*c as usize) < t.cols.len() && px != DEFAULT_COL_W {
                set_track_size(txn, &t, Axis::Cols, *c as usize, Some(px));
            }
        }
        for (r, h) in &s.row_heights {
            let px = oc_schema::js::round(h / 0.75);
            if (*r as usize) < t.rows.len() && px != DEFAULT_ROW_H {
                set_track_size(txn, &t, Axis::Rows, *r as usize, Some(px));
            }
        }
        for m in &s.merges {
            if m.r1 > m.r0 || m.c1 > m.c0 {
                set_merge(txn, &t, Some(*m), None);
            }
        }
        if s.frozen_rows != 0.0 || s.frozen_cols != 0.0 {
            set_frozen(txn, &t, Some(s.frozen_rows), Some(s.frozen_cols));
        }
        ids.push(id);
    }
    Ok(ids)
}

/// A plain JSON summary of a read workbook (tests, debugging): cells in row-major order with value, formula,
/// number format and mapped format; merges; column widths and row heights converted back to pixels the way
/// [`import_xlsx`] does; frozen panes.
pub fn workbook_json(sheets: &[XlsxSheet]) -> Value {
    Value::Array(
        sheets
            .iter()
            .map(|s| {
                let mut cells: Vec<&XlsxCell> = s.cells.iter().collect();
                cells.sort_by_key(|c| (c.r, c.c));
                let mut cols: Vec<(u32, f64)> = s.col_widths.iter().map(|(c, w)| (*c, oc_schema::js::round(w * 7.0))).collect();
                cols.sort_by_key(|x| x.0);
                let mut rows: Vec<(u32, f64)> = s.row_heights.iter().map(|(r, h)| (*r, oc_schema::js::round(h / 0.75))).collect();
                rows.sort_by_key(|x| x.0);
                let mut v = json!({
                    "name": s.name,
                    "cells": cells.iter().map(|c| json!({
                        "a1": crate::a1::a1(c.r as usize, c.c as usize),
                        "value": c.value.to_json(),
                        "formula": c.formula,
                        "numFmt": c.num_fmt,
                        "format": c.format.to_json(),
                    })).collect::<Vec<_>>(),
                    "merges": s.merges.iter().map(crate::a1::format_range).collect::<Vec<_>>(),
                    "colW": cols.iter().map(|(c, w)| json!([c, w])).collect::<Vec<_>>(),
                    "rowH": rows.iter().map(|(r, h)| json!([r, h])).collect::<Vec<_>>(),
                    "frozen": [s.frozen_rows, s.frozen_cols],
                });
                oc_schema::js::normalize_numbers(&mut v);
                v
            })
            .collect(),
    )
}
