// SPDX-License-Identifier: Apache-2.0
//! The parts of pdf.js font loading (`PartialEvaluator.translateFont`, `extractDataStructures`,
//! `extractWidths`, `buildToUnicode`, `Font._charToGlyph`) that text extraction depends on: the font's name,
//! glyph widths, Unicode values and how a string is split into character codes.

use std::collections::HashMap;

use hayro::hayro_interpret::hayro_cmap::{CMap, CMapName};
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, Stream};

use super::tables::{glyph_names, mac_expert, mac_roman, metrics, standard, symbol, win_ansi, zapf_dings};

pub(crate) const FONT_IDENTITY_MATRIX: [f64; 6] = [0.001, 0.0, 0.0, 0.001, 0.0, 0.0];

/// A loaded font, as far as getTextContent needs it.
#[derive(Debug)]
pub(crate) struct PdfFont {
    /// pdf.js `font.name` (what the TS importer reads from `commonObjs`).
    pub name: String,
    pub is_type3: bool,
    pub font_matrix: [f64; 6],
    pub bbox: [f64; 4],
    widths: HashMap<u32, f64>,
    default_width: f64,
    to_unicode: HashMap<u32, String>,
    cmap: Option<CMap>,
    cid_to_unicode_identity: bool,
}

pub(crate) struct Glyph {
    pub unicode: String,
    pub width: f64,
}

/// pdf.js `getStdFontMap` (the common aliases) → the standard 14 metrics.
fn std_font_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "Helvetica" | "Arial" | "ArialMT" | "Arial-Regular" | "Helvetica-Regular" | "ArialNarrow" | "Helvetica-Narrow" => "Helvetica",
        "Helvetica-Bold" | "Arial-Bold" | "Arial-BoldMT" | "Arial,Bold" | "Arial_Bold" | "Helvetica,Bold" => "Helvetica-Bold",
        "Helvetica-Oblique" | "Helvetica-Italic" | "Arial-Italic" | "Arial-ItalicMT" | "Arial,Italic" | "Helvetica,Italic" => "Helvetica-Oblique",
        "Helvetica-BoldOblique" | "Helvetica-BoldItalic" | "Arial-BoldItalic" | "Arial-BoldItalicMT" | "Arial,BoldItalic" => "Helvetica-BoldOblique",
        "Times-Roman" | "Times" | "TimesNewRoman" | "TimesNewRomanPS" | "TimesNewRomanPSMT" | "Times-Regular" | "TimesNewRoman-Regular" => "Times-Roman",
        "Times-Bold" | "TimesNewRoman-Bold" | "TimesNewRomanPS-Bold" | "TimesNewRomanPS-BoldMT" | "TimesNewRoman,Bold" => "Times-Bold",
        "Times-Italic" | "TimesNewRoman-Italic" | "TimesNewRomanPS-Italic" | "TimesNewRomanPS-ItalicMT" | "TimesNewRoman,Italic" => "Times-Italic",
        "Times-BoldItalic" | "TimesNewRoman-BoldItalic" | "TimesNewRomanPS-BoldItalic" | "TimesNewRomanPS-BoldItalicMT" | "TimesNewRoman,BoldItalic" => {
            "Times-BoldItalic"
        }
        "Courier" | "CourierNew" | "CourierNewPSMT" | "Courier-Regular" => "Courier",
        "Courier-Bold" | "CourierNew-Bold" | "CourierNewPS-BoldMT" | "CourierNew,Bold" => "Courier-Bold",
        "Courier-Oblique" | "Courier-Italic" | "CourierNew-Italic" | "CourierNewPS-ItalicMT" | "CourierNew,Italic" => "Courier-Oblique",
        "Courier-BoldOblique" | "Courier-BoldItalic" | "CourierNew-BoldItalic" | "CourierNewPS-BoldItalicMT" | "CourierNew,BoldItalic" => {
            "Courier-BoldOblique"
        }
        "Symbol" | "Symbol-Regular" => "Symbol",
        "ZapfDingbats" | "ZapfDingbatsITC" | "Dingbats" => "ZapfDingbats",
        _ => return None,
    })
}

fn is_serif_font(name: &str) -> bool {
    let n = name.split(['-', ',', '+']).next().unwrap_or("").to_lowercase();
    ["times", "georgia", "garamond", "palatino", "cambria", "bookman", "century", "serif"].iter().any(|s| n.contains(s))
}

/// pdf.js `getBaseFontMetrics`: widths by glyph name, or a fixed width (Courier).
enum Metrics {
    Named(&'static phf::Map<&'static str, f32>),
    Fixed(f64),
}

fn base_font_metrics(name: &str) -> Metrics {
    let lookup = std_font_name(name).unwrap_or(if is_serif_font(name) { "Times-Roman" } else { "Helvetica" });
    match lookup {
        "Courier" | "Courier-Bold" | "Courier-Oblique" | "Courier-BoldOblique" => Metrics::Fixed(600.0),
        "Helvetica-Bold" => Metrics::Named(&metrics::HELVETICA_BOLD),
        "Helvetica-Oblique" => Metrics::Named(&metrics::HELVETICA_OBLIQUE),
        "Helvetica-BoldOblique" => Metrics::Named(&metrics::HELVETICA_BOLD_OBLIQUE),
        "Times-Roman" => Metrics::Named(&metrics::TIMES_ROMAN),
        "Times-Bold" => Metrics::Named(&metrics::TIMES_BOLD),
        "Times-Italic" => Metrics::Named(&metrics::TIMES_ITALIC),
        "Times-BoldItalic" => Metrics::Named(&metrics::TIMES_BOLD_ITALIC),
        "Symbol" => Metrics::Named(&metrics::SYMBOL),
        "ZapfDingbats" => Metrics::Named(&metrics::ZAPF_DING_BATS),
        _ => Metrics::Named(&metrics::HELVETICA),
    }
}

#[derive(Clone, Copy)]
enum Enc {
    Standard,
    WinAnsi,
    MacRoman,
    MacExpert,
    Symbol,
    ZapfDingbats,
}

fn enc_name(e: Enc, code: u8) -> Option<&'static str> {
    match e {
        Enc::Standard => standard::get(code),
        // pdf.js' WinAnsiEncoding has "bullet" only at 127, 129, 141, 143, 144 and 157.
        Enc::WinAnsi => match code {
            127 | 129 | 141 | 143 | 144 | 157 => Some("bullet"),
            c if c < 32 => None,
            c => win_ansi::get(c),
        },
        Enc::MacRoman => mac_roman::get(code),
        Enc::MacExpert => mac_expert::get(code),
        Enc::Symbol => symbol::get(code),
        Enc::ZapfDingbats => zapf_dings::get(code),
    }
}

fn encoding_by_name(n: &str) -> Option<Enc> {
    Some(match n {
        "WinAnsiEncoding" => Enc::WinAnsi,
        "StandardEncoding" => Enc::Standard,
        "MacRomanEncoding" => Enc::MacRoman,
        "MacExpertEncoding" => Enc::MacExpert,
        "SymbolSetEncoding" => Enc::Symbol,
        "ZapfDingbatsEncoding" => Enc::ZapfDingbats,
        _ => return None,
    })
}

/// pdf.js `getUnicodeForGlyph` (for `uniXXXX` / `uXXXX[XX]` names).
fn unicode_for_glyph(name: &str) -> Option<u32> {
    if let Some(u) = glyph_names::get(name) {
        return u.chars().next().map(|c| c as u32);
    }
    let n = name.len();
    if n == 7 && name.starts_with("uni") && name[3..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return u32::from_str_radix(&name[3..], 16).ok();
    }
    if (5..=7).contains(&n) && name.starts_with('u') && name[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return u32::from_str_radix(&name[1..], 16).ok();
    }
    None
}

fn js_parse_int_hex(s: &str) -> Option<u32> {
    let digits: String = s.chars().take_while(char::is_ascii_hexdigit).collect();
    if digits.is_empty() { None } else { u32::from_str_radix(&digits, 16).ok() }
}

fn name_str(n: &Name<'_>) -> String {
    n.as_str().to_owned()
}

fn num_of(o: &Object<'_>) -> Option<f64> {
    match o {
        Object::Number(n) => Some(n.as_f64()),
        _ => None,
    }
}

/// pdf.js `isKnownFontName` (approximation: the standard 14 and their aliases).
fn is_known_font_name(n: &str) -> bool {
    std_font_name(n).is_some()
}

impl PdfFont {
    pub(crate) fn load(dict: &Dict<'_>) -> PdfFont {
        let subtype = dict.get::<Name<'_>>(b"Subtype").map(|n| name_str(&n)).unwrap_or_default();
        let composite = subtype == "Type0";
        let is_type3 = subtype == "Type3";
        let base_dict = dict.clone();
        let fdict = if composite {
            dict.get::<Array<'_>>(b"DescendantFonts").and_then(|a| a.iter::<Dict<'_>>().next()).unwrap_or_else(|| dict.clone())
        } else {
            dict.clone()
        };
        let descriptor = fdict.get::<Dict<'_>>(b"FontDescriptor");
        let base_font = fdict.get::<Name<'_>>(b"BaseFont").map(|n| name_str(&n));

        // ---- name
        let name = match &descriptor {
            None if is_type3 => "Type3".to_owned(),
            None => base_font.clone().unwrap_or_default().replace([',', '_'], "-"),
            Some(d) => {
                let font_name = d.get::<Name<'_>>(b"FontName").map(|n| name_str(&n));
                let mut chosen = font_name.clone();
                if !is_type3 && font_name != base_font {
                    if let (Some(f), Some(b)) = (&font_name, &base_font)
                        && (b.starts_with(f.as_str()) || (!is_known_font_name(f) && is_known_font_name(b)))
                    {
                        chosen = None;
                    }
                    if chosen.is_none() {
                        chosen = base_font.clone();
                    }
                }
                chosen.unwrap_or_default()
            }
        };

        // ---- encoding (simple fonts)
        let flags = descriptor.as_ref().and_then(|d| d.get::<i64>(b"Flags")).unwrap_or(0);
        let embedded = descriptor.as_ref().is_some_and(|d| d.contains_key(b"FontFile") || d.contains_key(b"FontFile2") || d.contains_key(b"FontFile3"));
        let mut base_encoding: Option<Enc> = None;
        let mut differences: HashMap<u32, String> = HashMap::new();
        if let Some(n) = base_dict.get::<Name<'_>>(b"Encoding") {
            base_encoding = encoding_by_name(n.as_str());
        } else if let Some(e) = base_dict.get::<Dict<'_>>(b"Encoding") {
            base_encoding = e.get::<Name<'_>>(b"BaseEncoding").and_then(|n| encoding_by_name(n.as_str()));
            if let Some(diff) = e.get::<Array<'_>>(b"Differences") {
                let mut code = 0u32;
                for o in diff.iter::<Object<'_>>() {
                    match o {
                        Object::Number(n) => code = n.as_f64() as u32,
                        Object::Name(n) => {
                            differences.insert(code, name_str(&n));
                            code += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
        let has_base_encoding = base_encoding.is_some();
        let default_encoding = base_encoding.unwrap_or({
            let symbolic = flags & 4 != 0;
            let nonsymbolic = flags & 32 != 0;
            if subtype == "TrueType" && !nonsymbolic {
                Enc::WinAnsi
            } else if symbolic && !nonsymbolic {
                if !embedded && name.to_lowercase().contains("symbol") {
                    Enc::Symbol
                } else if !embedded && name.to_lowercase().contains("dingbats") {
                    Enc::ZapfDingbats
                } else {
                    Enc::MacRoman
                }
            } else if descriptor.is_none() && std_font_name(&name) == Some("Symbol") {
                Enc::Symbol
            } else if descriptor.is_none() && std_font_name(&name) == Some("ZapfDingbats") {
                Enc::ZapfDingbats
            } else {
                Enc::Standard
            }
        });
        let glyph_name = |code: u32| -> Option<String> {
            differences.get(&code).filter(|n| n.as_str() != ".notdef").cloned().or_else(|| u8::try_from(code).ok().and_then(|c| enc_name(default_encoding, c)).map(str::to_owned))
        };

        // ---- font matrix / bbox (Type3)
        let mut font_matrix = FONT_IDENTITY_MATRIX;
        let mut bbox = [0.0; 4];
        if is_type3 {
            if let Some(m) = fdict.get::<Array<'_>>(b"FontMatrix") {
                let v: Vec<f64> = m.iter::<f64>().collect();
                if v.len() == 6 {
                    font_matrix.copy_from_slice(&v);
                }
            }
            if let Some(b) = fdict.get::<Array<'_>>(b"FontBBox") {
                let v: Vec<f64> = b.iter::<f64>().collect();
                if v.len() == 4 {
                    bbox.copy_from_slice(&v);
                }
            }
        }

        // ---- widths
        let mut widths: HashMap<u32, f64> = HashMap::new();
        let mut default_width = 0.0;
        if composite {
            default_width = fdict.get::<f64>(b"DW").unwrap_or(1000.0);
            if let Some(w) = fdict.get::<Array<'_>>(b"W") {
                let items: Vec<Object<'_>> = w.iter::<Object<'_>>().collect();
                let mut i = 0;
                while i < items.len() {
                    let Some(start) = num_of(&items[i]) else { break };
                    match items.get(i + 1) {
                        Some(Object::Array(a)) => {
                            for (k, o) in a.iter::<Object<'_>>().enumerate() {
                                if let Some(v) = num_of(&o) {
                                    widths.insert(start as u32 + k as u32, v);
                                }
                            }
                            i += 2;
                        }
                        Some(o) => {
                            let end = num_of(o).unwrap_or(start);
                            let v = items.get(i + 2).and_then(num_of).unwrap_or(0.0);
                            for c in start as u32..=end as u32 {
                                widths.insert(c, v);
                            }
                            i += 3;
                        }
                        None => break,
                    }
                }
            }
        } else if let Some(w) = fdict.get::<Array<'_>>(b"Widths") {
            let first = fdict.get::<f64>(b"FirstChar").unwrap_or(0.0) as u32;
            for (k, o) in w.iter::<Object<'_>>().enumerate() {
                if let Some(v) = num_of(&o) {
                    widths.insert(first + k as u32, v);
                }
            }
            default_width = descriptor.as_ref().and_then(|d| d.get::<f64>(b"MissingWidth")).unwrap_or(0.0);
        } else if !is_type3 {
            // Standard 14 (or unknown) font without /Widths: AFM metrics by glyph name (`buildCharCodeToWidth`).
            match base_font_metrics(&name) {
                Metrics::Fixed(v) => default_width = v,
                Metrics::Named(m) => {
                    for code in 0u32..256 {
                        if let Some(g) = differences.get(&code)
                            && let Some(v) = m.get(g.as_str())
                        {
                            widths.insert(code, f64::from(*v));
                            continue;
                        }
                        if let Some(g) = u8::try_from(code).ok().and_then(|c| enc_name(default_encoding, c))
                            && let Some(v) = m.get(g)
                        {
                            widths.insert(code, f64::from(*v));
                        }
                    }
                }
            }
        }

        // ---- unicode
        let mut to_unicode: HashMap<u32, String> = HashMap::new();
        let included = base_dict.get::<Stream<'_>>(b"ToUnicode").and_then(|s| s.decoded().ok().map(|d| d.to_vec()));
        let mut cid_to_unicode_identity = false;
        let mut have_included = false;
        if let Some(data) = included {
            to_unicode = parse_to_unicode(&data);
            have_included = !to_unicode.is_empty();
        }
        if !have_included {
            if !composite {
                to_unicode = simple_font_to_unicode(&glyph_name, has_base_encoding.then_some(default_encoding));
            } else {
                // `IdentityToUnicodeMap`: the code itself.
                cid_to_unicode_identity = true;
            }
        }

        // ---- code reading (composite)
        let cmap = if composite {
            match base_dict.get::<Name<'_>>(b"Encoding") {
                Some(n) if n.as_str() == "Identity-V" => Some(CMap::identity_v()),
                Some(_) => Some(CMap::identity_h()),
                None => base_dict.get::<Stream<'_>>(b"Encoding").and_then(|s| s.decoded().ok().and_then(|d| CMap::parse(&d, |_: CMapName<'_>| None))).or_else(|| Some(CMap::identity_h())),
            }
        } else {
            None
        };

        PdfFont { name, is_type3, font_matrix, bbox, widths, default_width, to_unicode, cmap, cid_to_unicode_identity }
    }

    /// Split a shown string into character codes (`charsToGlyphs`).
    pub(crate) fn codes(&self, bytes: &[u8]) -> Vec<(u32, u8)> {
        let Some(cmap) = &self.cmap else { return bytes.iter().map(|b| (u32::from(*b), 1)).collect() };
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let mut code = 0u32;
            let mut used = 0;
            for n in 0..4.min(bytes.len() - i) {
                code = (code << 8) | u32::from(bytes[i + n]);
                if cmap.lookup_cid_code(code, (n + 1) as u8).is_some() {
                    used = n + 1;
                    break;
                }
            }
            if used == 0 {
                // pdf.js reads the longest code space; fall back to 2-byte codes like Identity-H.
                let n = 2.min(bytes.len() - i);
                code = bytes[i..i + n].iter().fold(0, |a, b| (a << 8) | u32::from(*b));
                used = n;
            }
            out.push((code, used as u8));
            i += used;
        }
        out
    }

    /// `_charToGlyph`: width (glyph space) and Unicode value of a character code.
    pub(crate) fn glyph(&self, code: u32, len: u8) -> Glyph {
        let width_code = match &self.cmap {
            Some(cm) => cm.lookup_cid_code(code, len).unwrap_or(code),
            None => code,
        };
        let width = self.widths.get(&width_code).copied().unwrap_or(self.default_width);
        let unicode = match self.to_unicode.get(&code) {
            Some(s) => s.clone(),
            None => {
                // `String.fromCharCode(charcode)` (one UTF-16 code unit; also what `IdentityToUnicodeMap` answers).
                let _ = self.cid_to_unicode_identity;
                char::from_u32(code & 0xffff).map(String::from).unwrap_or_else(|| "\u{fffd}".into())
            }
        };
        Glyph { unicode, width }
    }
}

/// `_simpleFontToUnicode`: Unicode values from the encoding's glyph names.
fn simple_font_to_unicode(glyph_name: &dyn Fn(u32) -> Option<String>, base: Option<Enc>) -> HashMap<u32, String> {
    let names: Vec<(u32, String)> = (0u32..256).filter_map(|c| glyph_name(c).filter(|n| n != ".notdef" || c == 0).map(|n| (c, n))).collect();
    let mut force = false;
    'again: loop {
        let mut out = HashMap::new();
        for (code, gname) in &names {
            if gname.is_empty() {
                continue;
            }
            if let Some(u) = glyph_names::get(gname) {
                out.insert(*code, u.chars().next().map(String::from).unwrap_or_default());
                continue;
            }
            let mut cp: u32 = 0;
            let first = gname.as_bytes()[0];
            match first {
                b'G' if gname.len() == 3 => cp = js_parse_int_hex(&gname[1..]).unwrap_or(0),
                b'g' if gname.len() == 5 => cp = js_parse_int_hex(&gname[1..]).unwrap_or(0),
                b'C' | b'c' if (3..=4).contains(&gname.len()) => {
                    let s = &gname[1..];
                    if force {
                        cp = js_parse_int_hex(s).unwrap_or(0);
                    } else if let Ok(v) = s.parse::<u32>() {
                        cp = v;
                    } else if js_parse_int_hex(s).is_some() {
                        force = true;
                        continue 'again;
                    }
                }
                b'u' => cp = unicode_for_glyph(gname).unwrap_or(0),
                _ => {
                    if matches!(gname.as_str(), "f_h" | "f_t" | "T_h") {
                        out.insert(*code, gname.replace('_', ""));
                        continue;
                    }
                }
            }
            if cp > 0 && cp <= 0x10ffff {
                if let Some(b) = base
                    && cp == *code
                    && let Some(n) = u8::try_from(*code).ok().and_then(|c| enc_name(b, c))
                {
                    if let Some(u) = glyph_names::get(n) {
                        out.insert(*code, u.chars().next().map(String::from).unwrap_or_default());
                    }
                    continue;
                }
                if let Some(c) = char::from_u32(cp) {
                    out.insert(*code, c.to_string());
                }
            }
        }
        return out;
    }
}

/// A ToUnicode CMap (`bfchar` / `bfrange`), like pdf.js `readToUnicode`: destination strings are UTF-16BE; an
/// empty destination maps to "" (which `_charToGlyph` treats as unmapped).
fn parse_to_unicode(data: &[u8]) -> HashMap<u32, String> {
    #[derive(Debug)]
    enum Tok {
        Hex(Vec<u8>),
        Word(Vec<u8>),
        Open,
        Close,
    }
    let mut toks = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        match c {
            b'%' => {
                while i < data.len() && data[i] != b'\n' && data[i] != b'\r' {
                    i += 1;
                }
            }
            b'<' if data.get(i + 1) == Some(&b'<') => i += 2,
            b'>' if data.get(i + 1) == Some(&b'>') => i += 2,
            b'<' => {
                let mut hex = Vec::new();
                i += 1;
                while i < data.len() && data[i] != b'>' {
                    if data[i].is_ascii_hexdigit() {
                        hex.push(data[i]);
                    }
                    i += 1;
                }
                i += 1;
                if hex.len() % 2 == 1 {
                    hex.push(b'0');
                }
                toks.push(Tok::Hex(hex.chunks(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap_or("0"), 16).unwrap_or(0)).collect()));
            }
            b'[' => {
                toks.push(Tok::Open);
                i += 1;
            }
            b']' => {
                toks.push(Tok::Close);
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                let s = i;
                while i < data.len() && !data[i].is_ascii_whitespace() && !b"<>[]%/()".contains(&data[i]) {
                    i += 1;
                }
                if i == s {
                    i += 1;
                }
                toks.push(Tok::Word(data[s..i].to_vec()));
            }
        }
    }
    let code_of = |b: &[u8]| b.iter().fold(0u32, |a, x| (a << 8) | u32::from(*x));
    let text_of = |b: &[u8]| -> String {
        if b.len() == 1 {
            return char::from(b[0]).to_string();
        }
        let units: Vec<u16> = b.chunks(2).map(|p| (u16::from(p[0]) << 8) | u16::from(*p.get(1).unwrap_or(&0))).collect();
        String::from_utf16_lossy(&units)
    };
    let mut out = HashMap::new();
    let mut k = 0;
    let mut mode = 0;
    while k < toks.len() {
        match &toks[k] {
            Tok::Word(w) if w == b"beginbfchar" => mode = 1,
            Tok::Word(w) if w == b"beginbfrange" => mode = 2,
            Tok::Word(w) if w == b"endbfchar" || w == b"endbfrange" => mode = 0,
            Tok::Hex(src) if mode == 1 => {
                if let Some(Tok::Hex(dst)) = toks.get(k + 1) {
                    out.insert(code_of(src), text_of(dst));
                    k += 1;
                }
            }
            Tok::Hex(lo) if mode == 2 => {
                let (Some(Tok::Hex(hi)), Some(next)) = (toks.get(k + 1), toks.get(k + 2)) else { break };
                let (lo, hi) = (code_of(lo), code_of(hi));
                match next {
                    Tok::Hex(dst) => {
                        let mut d = dst.clone();
                        let mut c = lo;
                        while c <= hi && c - lo < 0x10000 {
                            out.insert(c, text_of(&d));
                            let n = d.len();
                            if n >= 2 && d[n - 1] == 0xff {
                                d[n - 2] = d[n - 2].wrapping_add(1);
                                d[n - 1] = 0;
                            } else if n >= 1 {
                                d[n - 1] = d[n - 1].wrapping_add(1);
                            }
                            c += 1;
                        }
                        k += 2;
                    }
                    Tok::Open => {
                        let mut j = k + 3;
                        let mut c = lo;
                        while let Some(Tok::Hex(dst)) = toks.get(j) {
                            if c <= hi {
                                out.insert(c, text_of(dst));
                            }
                            c += 1;
                            j += 1;
                        }
                        k = j;
                    }
                    _ => k += 2,
                }
            }
            _ => {}
        }
        k += 1;
    }
    out.retain(|_, v| !v.is_empty());
    out
}
