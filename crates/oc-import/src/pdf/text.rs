// SPDX-License-Identifier: Apache-2.0
//! Text items of a page: a port of pdf.js' `PartialEvaluator.getTextContent` (pdfjs-dist 5.4, horizontal
//! writing), which is what the TS importer reads. The content stream is walked with hayro-syntax; fonts come from
//! [`super::font`]. Items keep pdf.js' chunking: glyphs are merged while they continue on the same line, gaps of
//! 0.102–0.6 em become a space inside an item, larger gaps a separate `" "` item, and font changes, form
//! XObjects and marked content end an item.

use std::collections::HashMap;
use std::rc::Rc;

use hayro::hayro_syntax::content::TypedIter;
use hayro::hayro_syntax::content::ops::TypedInstruction;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier, Stream};
use hayro::hayro_syntax::page::Resources;

use super::font::{FONT_IDENTITY_MATRIX, PdfFont};

pub(crate) type M = [f64; 6];
pub(crate) const IDENTITY: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// pdf.js `Util.transform(m1, m2)`.
pub(crate) fn mul(m1: &M, m2: &M) -> M {
    [
        m1[0] * m2[0] + m1[2] * m2[1],
        m1[1] * m2[0] + m1[3] * m2[1],
        m1[0] * m2[2] + m1[2] * m2[3],
        m1[1] * m2[2] + m1[3] * m2[3],
        m1[0] * m2[4] + m1[2] * m2[5] + m1[4],
        m1[1] * m2[4] + m1[3] * m2[5] + m1[5],
    ]
}

/// The CTM is a `Float32Array` in pdf.js.
fn f32m(m: M) -> M {
    m.map(|v| f64::from(v as f32))
}

/// One pdf.js text content item.
#[derive(Debug, Clone)]
pub(crate) struct TextItem {
    pub str: String,
    pub width: f64,
    pub transform: M,
    pub font_name: String,
}

#[derive(Clone)]
struct TextState {
    ctm: M,
    font_res: Option<Vec<u8>>,
    font_size: f64,
    font: Option<Rc<PdfFont>>,
    font_matrix: M,
    text_matrix: M,
    text_line_matrix: M,
    char_spacing: f64,
    word_spacing: f64,
    leading: f64,
    h_scale: f64,
    rise: f64,
}

impl TextState {
    fn new() -> Self {
        TextState {
            ctm: IDENTITY,
            font_res: None,
            font_size: 0.0,
            font: None,
            font_matrix: FONT_IDENTITY_MATRIX,
            text_matrix: IDENTITY,
            text_line_matrix: IDENTITY,
            char_spacing: 0.0,
            word_spacing: 0.0,
            leading: 0.0,
            h_scale: 1.0,
            rise: 0.0,
        }
    }
    fn translate_text_matrix(&mut self, x: f64, y: f64) {
        let m = &mut self.text_matrix;
        m[4] += m[0] * x + m[2] * y;
        m[5] += m[1] * x + m[3] * y;
    }
    fn translate_text_line_matrix(&mut self, x: f64, y: f64) {
        let m = &mut self.text_line_matrix;
        m[4] += m[0] * x + m[2] * y;
        m[5] += m[1] * x + m[3] * y;
    }
    fn carriage_return(&mut self) {
        self.translate_text_line_matrix(0.0, -self.leading);
        self.text_matrix = self.text_line_matrix;
    }
    fn current_transform(&self) -> M {
        let mut tsm = [self.font_size * self.h_scale, 0.0, 0.0, self.font_size, 0.0, self.rise];
        if let Some(f) = &self.font
            && f.is_type3
            && self.font_size <= 1.0
            && self.font_matrix != FONT_IDENTITY_MATRIX
        {
            let glyph_height = f.bbox[3] - f.bbox[1];
            if glyph_height > 0.0 {
                tsm[3] *= glyph_height * self.font_matrix[3];
            }
        }
        mul(&self.ctm, &mul(&self.text_matrix, &tsm))
    }
    fn same_font(&self, o: &TextState) -> bool {
        let a = self.font.as_ref().map(Rc::as_ptr);
        let b = o.font.as_ref().map(Rc::as_ptr);
        a == b && self.font_size == o.font_size && self.font_res == o.font_res
    }
}

/// `textContentItem`.
struct Chunk {
    initialized: bool,
    str: Vec<String>,
    total_width: f64,
    width: f64,
    prev_transform: Option<M>,
    text_advance_scale: f64,
    space_in_flow_min: f64,
    space_in_flow_max: f64,
    tracking_space_min: f64,
    negative_space_max: f64,
    not_a_space: f64,
    transform: M,
    font_name: String,
    height: f64,
}

/// Whitespace / zero-width diacritic / invisible format mark (`getCharUnicodeCategory`).
fn category(s: &str) -> (bool, bool, bool) {
    let first = s.chars().next();
    let ws = first.is_some_and(is_js_space);
    let mn = s.chars().any(is_mn);
    let cf = s.chars().last().is_some_and(is_cf);
    (ws, mn, cf)
}

fn is_js_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// `\p{Mn}` (approximated by a non-zero canonical combining class).
fn is_mn(c: char) -> bool {
    unicode_normalization::char::canonical_combining_class(c) != 0
}

/// `\p{Cf}`.
fn is_cf(c: char) -> bool {
    matches!(c as u32, 0xad | 0x600..=0x605 | 0x61c | 0x6dd | 0x70f | 0x890 | 0x891 | 0x8e2 | 0x180e | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x2064 | 0x2066..=0x206f | 0xfeff | 0xfff9..=0xfffb | 0x110bd | 0x110cd | 0x13430..=0x1343f | 0x1bca0..=0x1bca3 | 0x1d173..=0x1d17a | 0xe0001 | 0xe0020..=0xe007f)
}

/// pdf.js `normalizeUnicode`: NFKC for a fixed set of compatibility characters (ligatures, spaces, …).
fn normalize_unicode(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let special = |c: char| {
        let u = c as u32;
        matches!(u, 0xa0 | 0xb5 | 0x37e | 0xeb3 | 0x2000..=0x200a | 0x202f | 0x2126 | 0xfb00..=0xfb04 | 0xfb06 | 0xfb20..=0xfb36 | 0xfb38..=0xfb3c | 0xfb3e | 0xfb40..=0xfb41 | 0xfb43..=0xfb44 | 0xfb46..=0xfba1 | 0xfba4..=0xfba9 | 0xfbae..=0xfbb1 | 0xfbd3..=0xfbdc | 0xfbde..=0xfbe7 | 0xfbea..=0xfbf8 | 0xfbfc..=0xfbfd | 0xfc00..=0xfc5d | 0xfc64..=0xfcf1 | 0xfcf5..=0xfd3d | 0xfd88 | 0xfdf4 | 0xfdfa..=0xfdfb | 0xfe71 | 0xfe77 | 0xfe79 | 0xfe7b | 0xfe7d)
    };
    if !s.chars().any(|c| special(c) || c == '\u{fb05}') {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut run = String::new();
    for c in s.chars() {
        if special(c) {
            run.push(c);
            continue;
        }
        if !run.is_empty() {
            out.extend(run.nfkc());
            run.clear();
        }
        if c == '\u{fb05}' {
            out.push_str("\u{17f}t");
        } else {
            out.push(c);
        }
    }
    out.extend(run.nfkc());
    out
}

const TRACKING_SPACE_FACTOR: f64 = 0.102;
const NOT_A_SPACE_FACTOR: f64 = 0.03;
const NEGATIVE_SPACE_FACTOR: f64 = -0.2;
const SPACE_IN_FLOW_MIN_FACTOR: f64 = 0.102;
const SPACE_IN_FLOW_MAX_FACTOR: f64 = 0.6;
const VERTICAL_SHIFT_RATIO: f64 = 0.25;

/// Fonts loaded per font dictionary (pdf.js caches by reference).
#[derive(Default)]
pub(crate) struct FontCache {
    by_id: HashMap<ObjectIdentifier, Rc<PdfFont>>,
}

impl FontCache {
    fn load(&mut self, dict: &Dict<'_>) -> Rc<PdfFont> {
        if let Some(id) = dict.obj_id()
            && let Some(f) = self.by_id.get(&id)
        {
            return f.clone();
        }
        let f = Rc::new(PdfFont::load(dict));
        if let Some(id) = dict.obj_id() {
            self.by_id.insert(id, f.clone());
        }
        f
    }
}

struct Ctx<'c> {
    items: &'c mut Vec<TextItem>,
    fonts: &'c mut FontCache,
    view: [f64; 4],
}

/// One `getTextContent` call (the page, or a form XObject with its own item builder).
struct Builder {
    chunk: Chunk,
    two_last: [char; 2],
    two_last_pos: usize,
}

impl Builder {
    fn new() -> Self {
        Builder {
            chunk: Chunk {
                initialized: false,
                str: Vec::new(),
                total_width: 0.0,
                width: 0.0,
                prev_transform: None,
                text_advance_scale: 0.0,
                space_in_flow_min: 0.0,
                space_in_flow_max: 0.0,
                tracking_space_min: f64::INFINITY,
                negative_space_max: f64::NEG_INFINITY,
                not_a_space: f64::NEG_INFINITY,
                transform: IDENTITY,
                font_name: String::new(),
                height: 0.0,
            },
            two_last: [' ', ' '],
            two_last_pos: 0,
        }
    }

    fn save_last_char(&mut self, ch: char) -> bool {
        let next = (self.two_last_pos + 1) % 2;
        let ret = self.two_last[self.two_last_pos] != ' ' && self.two_last[next] == ' ';
        self.two_last[self.two_last_pos] = ch;
        self.two_last_pos = next;
        ret
    }
    fn should_add_whitespace(&self) -> bool {
        self.two_last[self.two_last_pos] != ' ' && self.two_last[(self.two_last_pos + 1) % 2] == ' '
    }
    fn reset_last_chars(&mut self) {
        self.two_last = [' ', ' '];
        self.two_last_pos = 0;
    }

    fn push_whitespace(&self, ctx: &mut Ctx<'_>, width: f64, transform: Option<M>, font_name: Option<String>, ts: &TextState) {
        ctx.items.push(TextItem {
            str: " ".into(),
            width,
            transform: transform.or(self.chunk.prev_transform).unwrap_or_else(|| ts.current_transform()),
            font_name: font_name.unwrap_or_else(|| self.chunk.font_name.clone()),
        });
    }

    fn ensure(&mut self, ts: &TextState) {
        if self.chunk.initialized {
            return;
        }
        let c = &mut self.chunk;
        c.font_name = ts.font.as_ref().map(|f| f.name.clone()).unwrap_or_default();
        let trm = ts.current_transform();
        c.transform = trm;
        c.width = 0.0;
        c.total_width = 0.0;
        c.height = trm[2].hypot(trm[3]);
        let scale_line_x = ts.text_line_matrix[0].hypot(ts.text_line_matrix[1]);
        let scale_ctm_x = ts.ctm[0].hypot(ts.ctm[1]);
        c.text_advance_scale = scale_ctm_x * scale_line_x;
        let fs = ts.font_size;
        c.tracking_space_min = fs * TRACKING_SPACE_FACTOR;
        c.not_a_space = fs * NOT_A_SPACE_FACTOR;
        c.negative_space_max = fs * NEGATIVE_SPACE_FACTOR;
        c.space_in_flow_min = fs * SPACE_IN_FLOW_MIN_FACTOR;
        c.space_in_flow_max = fs * SPACE_IN_FLOW_MAX_FACTOR;
        c.initialized = true;
    }

    fn update_advance_scale(&mut self, ts: &TextState) {
        if !self.chunk.initialized {
            return;
        }
        let scale = ts.ctm[0].hypot(ts.ctm[1]) * ts.text_line_matrix[0].hypot(ts.text_line_matrix[1]);
        if scale == self.chunk.text_advance_scale {
            return;
        }
        self.chunk.total_width += self.chunk.width * self.chunk.text_advance_scale;
        self.chunk.width = 0.0;
        self.chunk.text_advance_scale = scale;
    }

    fn flush(&mut self, ctx: &mut Ctx<'_>) {
        if !self.chunk.initialized {
            return;
        }
        let c = &mut self.chunk;
        c.total_width += c.width * c.text_advance_scale;
        ctx.items.push(TextItem {
            str: normalize_unicode(&c.str.concat()),
            width: c.total_width.abs(),
            transform: c.transform,
            font_name: c.font_name.clone(),
        });
        c.initialized = false;
        c.str.clear();
    }

    fn append_eol(&mut self, ctx: &mut Ctx<'_>) {
        self.reset_last_chars();
        if self.chunk.initialized {
            self.flush(ctx);
        }
        // (An empty item with `hasEOL` otherwise; the importer skips empty strings.)
    }

    /// `addFakeSpaces`: true when a separate whitespace item was pushed.
    fn add_fake_spaces(&mut self, ctx: &mut Ctx<'_>, width: f64, transf: Option<M>, orient: f64, ts: &TextState) -> bool {
        if orient * self.chunk.space_in_flow_min <= width && width <= orient * self.chunk.space_in_flow_max {
            if self.chunk.initialized {
                self.reset_last_chars();
                self.chunk.str.push(" ".into());
            }
            return false;
        }
        let font_name = self.chunk.font_name.clone();
        self.flush(ctx);
        self.reset_last_chars();
        let t = transf.unwrap_or_else(|| ts.current_transform());
        self.push_whitespace(ctx, width.abs(), Some(t), Some(font_name), ts);
        true
    }

    fn compare_with_last_position(&mut self, ctx: &mut Ctx<'_>, glyph_width: f64, ts: &TextState) -> bool {
        let cur = ts.current_transform();
        let mut pos_x = cur[4];
        let mut pos_y = cur[5];
        let vb = ctx.view;
        if pos_x + glyph_width < vb[0] || pos_x > vb[2] || pos_y < vb[1] || pos_y > vb[3] {
            return false;
        }
        if ts.font.is_none() {
            return true;
        }
        let Some(prev) = self.chunk.prev_transform else { return true };
        let mut last_x = prev[4];
        let mut last_y = prev[5];
        if last_x == pos_x && last_y == pos_y {
            return true;
        }
        let mut rotate = -1;
        if cur[0] != 0.0 && cur[1] == 0.0 && cur[2] == 0.0 {
            rotate = if cur[0] > 0.0 { 0 } else { 180 };
        } else if cur[1] != 0.0 && cur[0] == 0.0 && cur[3] == 0.0 {
            rotate = if cur[1] > 0.0 { 90 } else { 270 };
        }
        match rotate {
            0 => {}
            90 => {
                std::mem::swap(&mut pos_x, &mut pos_y);
                std::mem::swap(&mut last_x, &mut last_y);
            }
            180 => {
                (pos_x, pos_y, last_x, last_y) = (-pos_x, -pos_y, -last_x, -last_y);
            }
            270 => {
                (pos_x, pos_y) = (-pos_y, -pos_x);
                (last_x, last_y) = (-last_y, -last_x);
            }
            _ => {
                let inv = |x: f64, y: f64, m: &M| {
                    let s = m[0].hypot(m[1]);
                    ((m[0] * x + m[1] * y) / s, (m[2] * x + m[3] * y) / s)
                };
                (pos_x, pos_y) = inv(pos_x, pos_y, &cur);
                (last_x, last_y) = inv(last_x, last_y, &prev);
            }
        }
        let advance_x = (pos_x - last_x) / self.chunk.text_advance_scale;
        let advance_y = pos_y - last_y;
        let orient = js_sign(self.chunk.width);
        if advance_x < orient * self.chunk.negative_space_max {
            if advance_y.abs() > 0.5 * self.chunk.height {
                self.append_eol(ctx);
                return true;
            }
            self.reset_last_chars();
            self.flush(ctx);
            return true;
        }
        if advance_y.abs() > self.chunk.height {
            self.append_eol(ctx);
            return true;
        }
        if advance_x <= orient * self.chunk.not_a_space {
            self.reset_last_chars();
        }
        if advance_x <= orient * self.chunk.tracking_space_min {
            if self.should_add_whitespace() {
                self.reset_last_chars();
                self.flush(ctx);
                self.push_whitespace(ctx, advance_x.abs(), None, None, ts);
            } else {
                self.chunk.width += advance_x;
            }
        } else if !self.add_fake_spaces(ctx, advance_x, self.chunk.prev_transform, orient, ts) {
            if self.chunk.str.is_empty() {
                self.reset_last_chars();
                self.push_whitespace(ctx, advance_x.abs(), None, None, ts);
            } else {
                self.chunk.width += advance_x;
            }
        }
        if advance_y.abs() > self.chunk.height * VERTICAL_SHIFT_RATIO {
            self.flush(ctx);
        }
        true
    }

    fn build(&mut self, ctx: &mut Ctx<'_>, ts: &mut TextState, chars: &[u8], extra_spacing: f64) {
        let Some(font) = ts.font.clone() else { return };
        if chars.is_empty() {
            let cs = ts.char_spacing + extra_spacing;
            if cs != 0.0 {
                ts.translate_text_matrix(cs * ts.h_scale, 0.0);
            }
            return;
        }
        let codes = font.codes(chars);
        let scale = ts.font_matrix[0] * ts.font_size;
        let n = codes.len();
        for (i, (code, len)) in codes.into_iter().enumerate() {
            let g = font.glyph(code, len);
            let (ws, mn, cf) = category(&g.unicode);
            if cf {
                continue;
            }
            let mut char_spacing = ts.char_spacing + if i + 1 == n { extra_spacing } else { 0.0 };
            let mut scaled = g.width * scale;
            if ws {
                char_spacing += scaled + ts.word_spacing;
                ts.translate_text_matrix(char_spacing * ts.h_scale, 0.0);
                self.save_last_char(' ');
                continue;
            }
            if !mn && !self.compare_with_last_position(ctx, scaled, ts) {
                ts.translate_text_matrix(scaled * ts.h_scale, 0.0);
                continue;
            }
            self.ensure(ts);
            if mn {
                scaled = 0.0;
            }
            scaled *= ts.h_scale;
            ts.translate_text_matrix(scaled, 0.0);
            self.chunk.width += scaled;
            if scaled != 0.0 {
                self.chunk.prev_transform = Some(ts.current_transform());
            }
            // `saveLastChar(glyphUnicode)` compares one-character strings; a longer value never equals " ".
            let key = if g.unicode.chars().count() == 1 { g.unicode.chars().next().unwrap_or('x') } else { '\u{0}' };
            if self.save_last_char(key) {
                self.chunk.str.push(" ".into());
            }
            self.chunk.str.push(g.unicode);
            if char_spacing != 0.0 {
                ts.translate_text_matrix(char_spacing * ts.h_scale, 0.0);
            }
        }
    }
}

/// `Math.sign` (0 for ±0).
fn js_sign(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v
    }
}


#[allow(clippy::too_many_lines)]
fn run(ctx: &mut Ctx<'_>, content: &[u8], resources: &Resources<'_>, initial: TextState, seen: &[ObjectIdentifier]) {
    let mut b = Builder::new();
    let mut stack: Vec<TextState> = Vec::new();
    let mut ts = initial;
    let mut ops = TypedIter::new(content);
    let mut spaced: Vec<u8> = Vec::new();
    while let Some(op) = ops.next() {
        match op {
            TypedInstruction::SaveState(_) => stack.push(ts.clone()),
            TypedInstruction::RestoreState(_) => {
                if let Some(prev) = stack.pop() {
                    let changed = !prev.same_font(&ts);
                    ts = prev;
                    if changed {
                        b.flush(ctx);
                    }
                }
            }
            TypedInstruction::Transform(t) => {
                let m = [t.0.as_f64(), t.1.as_f64(), t.2.as_f64(), t.3.as_f64(), t.4.as_f64(), t.5.as_f64()];
                ts.ctm = f32m(mul(&ts.ctm, &m));
            }
            TypedInstruction::TextFont(f) => {
                let name = f.0.as_ref().to_vec();
                let size = f.1.as_f64();
                if ts.font.is_some() && ts.font_res.as_deref() == Some(&name[..]) && ts.font_size == size {
                    continue;
                }
                b.flush(ctx);
                ts.font_res = Some(name.clone());
                ts.font_size = size;
                match resources.fonts.get::<Dict<'_>>(&name[..]) {
                    Some(d) => {
                        let font = ctx.fonts.load(&d);
                        ts.font_matrix = font.font_matrix;
                        ts.font = Some(font);
                    }
                    None => {
                        ts.font = None;
                        ts.font_matrix = FONT_IDENTITY_MATRIX;
                    }
                }
            }
            TypedInstruction::TextRise(v) => ts.rise = v.0.as_f64(),
            TypedInstruction::HorizontalScaling(v) => ts.h_scale = v.0.as_f64() / 100.0,
            TypedInstruction::TextLeading(v) => ts.leading = v.0.as_f64(),
            TypedInstruction::NextLine(v) => {
                ts.translate_text_line_matrix(v.0.as_f64(), v.1.as_f64());
                ts.text_matrix = ts.text_line_matrix;
            }
            TypedInstruction::NextLineAndSetLeading(v) => {
                ts.leading = -v.1.as_f64();
                ts.translate_text_line_matrix(v.0.as_f64(), v.1.as_f64());
                ts.text_matrix = ts.text_line_matrix;
            }
            TypedInstruction::NextLineUsingLeading(_) => ts.carriage_return(),
            TypedInstruction::SetTextMatrix(t) => {
                let m = [t.0.as_f64(), t.1.as_f64(), t.2.as_f64(), t.3.as_f64(), t.4.as_f64(), t.5.as_f64()];
                ts.text_matrix = m;
                ts.text_line_matrix = m;
                b.update_advance_scale(&ts);
            }
            TypedInstruction::CharacterSpacing(v) => ts.char_spacing = v.0.as_f64(),
            TypedInstruction::WordSpacing(v) => ts.word_spacing = v.0.as_f64(),
            TypedInstruction::BeginText(_) => {
                ts.text_matrix = IDENTITY;
                ts.text_line_matrix = IDENTITY;
            }
            TypedInstruction::ShowTexts(arr) => {
                if ts.font.is_none() {
                    continue;
                }
                let space_factor = -ts.font_size / 1000.0;
                for item in arr.0.iter::<Object<'_>>() {
                    match item {
                        Object::String(s) => spaced.extend_from_slice(s.as_bytes()),
                        Object::Number(n) if n.as_f64() != 0.0 => {
                            let chars = std::mem::take(&mut spaced);
                            b.build(ctx, &mut ts, &chars, n.as_f64() * space_factor);
                        }
                        _ => {}
                    }
                }
                if !spaced.is_empty() {
                    let chars = std::mem::take(&mut spaced);
                    b.build(ctx, &mut ts, &chars, 0.0);
                }
            }
            TypedInstruction::ShowText(s) => {
                if ts.font.is_some() {
                    b.build(ctx, &mut ts, s.0.as_bytes(), 0.0);
                }
            }
            TypedInstruction::NextLineAndShowText(s) => {
                if ts.font.is_some() {
                    ts.carriage_return();
                    b.build(ctx, &mut ts, s.0.as_bytes(), 0.0);
                }
            }
            TypedInstruction::ShowTextWithParameters(p) => {
                if ts.font.is_some() {
                    ts.word_spacing = p.0.as_f64();
                    ts.char_spacing = p.1.as_f64();
                    ts.carriage_return();
                    b.build(ctx, &mut ts, p.2.as_bytes(), 0.0);
                }
            }
            TypedInstruction::XObject(x) => {
                b.flush(ctx);
                let Some(stream) = resources.x_objects.get::<Stream<'_>>(x.0.as_ref()) else { continue };
                let dict = stream.dict();
                if dict.get::<Name<'_>>(b"Subtype").is_none_or(|n| n.as_str() != "Form") {
                    continue;
                }
                let id = stream.obj_id();
                if seen.contains(&id) || seen.len() > 24 {
                    continue;
                }
                let mut state = ts.clone();
                if let Some(m) = dict.get::<Array<'_>>(b"Matrix") {
                    let v: Vec<f64> = m.iter::<f64>().collect();
                    if v.len() == 6 {
                        state.ctm = f32m(mul(&state.ctm, &[v[0], v[1], v[2], v[3], v[4], v[5]]));
                    }
                }
                let local = dict.get::<Dict<'_>>(b"Resources").map(Resources::new);
                let res = local.as_ref().unwrap_or(resources);
                let Ok(data) = stream.decoded() else { continue };
                let mut seen2 = seen.to_vec();
                seen2.push(id);
                run(ctx, &data, res, state, &seen2);
            }
            TypedInstruction::SetGraphicsState(g) => {
                let Some(gs) = resources.ext_g_states.get::<Dict<'_>>(g.0.as_ref()) else { continue };
                let Some(arr) = gs.get::<Array<'_>>(b"Font") else { continue };
                let mut it = arr.iter::<Object<'_>>();
                let (Some(Object::Dict(fd)), Some(Object::Number(size))) = (it.next(), it.next()) else { continue };
                b.flush(ctx);
                ts.font_res = None;
                ts.font_size = size.as_f64();
                let font = ctx.fonts.load(&fd);
                ts.font_matrix = font.font_matrix;
                ts.font = Some(font);
            }
            TypedInstruction::BeginMarkedContent(_) | TypedInstruction::BeginMarkedContentWithProperties(_) | TypedInstruction::EndMarkedContent(_) => {
                b.flush(ctx);
            }
            _ => {}
        }
    }
    b.flush(ctx);
}

/// The text content items of a page (`page.getTextContent()`), in user space; `view` is the page's view box.
pub(crate) fn text_content(content: &[u8], resources: &Resources<'_>, view: [f64; 4], fonts: &mut FontCache) -> Vec<TextItem> {
    let mut items = Vec::new();
    let mut ctx = Ctx { items: &mut items, fonts, view };
    run(&mut ctx, content, resources, TextState::new(), &[]);
    items
}
