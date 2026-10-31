// SPDX-License-Identifier: Apache-2.0
//! Caret and selection geometry in box space (`caretRect`, `offsetAt`, `selectionRects`, `wordAt`,
//! `verticalMove`). Caret geometry always comes from the layout, never from browser text layout. Offsets are
//! flat UTF-16 offsets (paragraph separators count as one), like the TS.

use icu_properties::CodePointMapData;
use icu_properties::props::Script;
use icu_segmenter::WordSegmenter;
use icu_segmenter::options::WordBreakInvariantOptions;
use oc_schema::primitives::Rect;
use oc_schema::text::TextAlign;

use crate::layout::{LaidLine, LaidParagraph, TextLayout};
use crate::utf16::{Utf16Map, byte_at_utf16};

/// A rectangle with the direction of the text it covers (SkParagraph `TextBox`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct TextBox {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    rtl: bool,
}

/// `getRectsForRange(start, end, RectHeightStyle.Max, RectWidthStyle.Tight)` over UTF-16 offsets.
fn rects_for_range(p: &LaidParagraph, start: usize, end: usize) -> Vec<TextBox> {
    let mut out: Vec<TextBox> = Vec::new();
    // A range edge inside a grapheme (e.g. half a surrogate pair) yields no boxes, as in SkParagraph.
    let inside = |o: usize| p.lines.iter().flat_map(|l| &l.clusters).any(|c| c.start < o && o < c.end);
    if inside(start) || inside(end) {
        return out;
    }
    for line in &p.lines {
        let mut cur: Option<TextBox> = None;
        for c in &line.clusters {
            if c.end <= start || c.start >= end || c.end <= c.start {
                if let Some(b) = cur.take() {
                    out.push(b);
                }
                continue;
            }
            // RectHeightStyle::Max: the runs' ascent and descent around the line's (Flutter) baseline.
            let b = TextBox { left: c.x, top: line.baseline - line.ascent, right: c.x + c.advance, bottom: line.baseline + line.descent, rtl: c.rtl };
            cur = match cur {
                Some(prev) if prev.rtl == b.rtl && (prev.right - b.left).abs() < 1e-3 => Some(TextBox { right: b.right, ..prev }),
                Some(prev) => {
                    out.push(prev);
                    Some(b)
                }
                None => Some(b),
            };
        }
        if let Some(b) = cur {
            out.push(b);
        }
    }
    out
}

fn line_at_y(p: &LaidParagraph, y: f64) -> Option<&LaidLine> {
    p.lines.iter().find(|l| y < l.top + l.height).or(p.lines.last())
}

/// `getGlyphPositionAtCoordinate(x, y).pos` (paragraph space), a UTF-16 offset into the paragraph text.
fn position_at(p: &LaidParagraph, x: f64, y: f64) -> usize {
    let Some(line) = line_at_y(p, y) else { return 0 };
    let hit: Vec<&crate::layout::ClusterBox> = line.clusters.iter().collect();
    let Some(&first) = hit.first() else { return line.start };
    if x < first.x {
        return if first.rtl { first.end } else { first.start };
    }
    for &c in &hit {
        if x < c.x + c.advance {
            let leading = x < c.x + c.advance / 2.0;
            return if leading != c.rtl { c.start } else { c.end };
        }
    }
    let last = hit.last().copied().unwrap_or(first);
    if last.rtl { last.start } else { last.end }
}

impl TextLayout {
    fn para_at(&self, offset: usize) -> Option<(&LaidParagraph, usize)> {
        for p in &self.paras {
            if offset <= p.start + p.len {
                return Some((p, offset.saturating_sub(p.start)));
            }
        }
        self.paras.last().map(|p| (p, p.len))
    }

    /// Caret rectangle (1.5 px wide) at a flat offset.
    pub fn caret_rect(&self, offset: usize) -> Rect {
        let Some((p, o)) = self.para_at(offset) else { return Rect { x: 0.0, y: 0.0, w: 1.5, h: self.height } };
        if p.len == 0 {
            let x = match p.align {
                TextAlign::Center => p.width / 2.0,
                TextAlign::End => p.width,
                _ => 0.0,
            };
            return Rect { x: p.x + x, y: p.y, w: 1.5, h: p.height };
        }
        let forward = o < p.len;
        let rects = if forward { rects_for_range(p, o, o + 1) } else { rects_for_range(p, o - 1, o) };
        let Some(r) = rects.first() else { return Rect { x: p.x, y: p.y, w: 1.5, h: p.height } };
        let x = if forward != r.rtl { r.left } else { r.right };
        Rect { x: p.x + x, y: p.y + r.top, w: 1.5, h: r.bottom - r.top }
    }

    /// Flat offset nearest to a box-space point.
    pub fn offset_at(&self, x: f64, y: f64) -> usize {
        let Some(mut target) = self.paras.first() else { return 0 };
        for p in &self.paras {
            target = p;
            if y < p.y + p.height {
                break;
            }
        }
        if target.len == 0 {
            return target.start;
        }
        let pos = position_at(target, x - target.x, y - target.y);
        target.start + pos.min(target.len)
    }

    /// Selection rectangles for a flat range (either order).
    pub fn selection_rects(&self, from: usize, to: usize) -> Vec<Rect> {
        let (a, b) = if from <= to { (from, to) } else { (to, from) };
        let mut out = Vec::new();
        for p in &self.paras {
            let s = a.max(p.start) as isize - p.start as isize;
            let e = b.min(p.start + p.len) as isize - p.start as isize;
            if e > s {
                for r in rects_for_range(p, s as usize, e as usize) {
                    out.push(Rect { x: p.x + r.left, y: p.y + r.top, w: r.right - r.left, h: r.bottom - r.top });
                }
            } else if p.len == 0 && a <= p.start && b > p.start {
                out.push(Rect { x: p.x, y: p.y, w: 6.0, h: p.height });
            }
        }
        out
    }

    /// Word boundaries around an offset (double-click selection), as flat offsets.
    pub fn word_at(&self, offset: usize) -> (usize, usize) {
        let Some((p, o)) = self.para_at(offset) else { return (0, 0) };
        let o = o.min(p.len.saturating_sub(1));
        let text = &p.text;
        if text.is_empty() {
            return (p.start, p.start);
        }
        let map = Utf16Map::new(text);
        let byte = byte_at_utf16(text, o);
        let seg = WordSegmenter::new_for_non_complex_scripts(WordBreakInvariantOptions::default());
        let breaks: Vec<usize> = seg.segment_str(text).collect();
        let mut start = 0usize;
        let mut end = text.len();
        for w in breaks.windows(2) {
            if w[0] <= byte && byte < w[1] {
                start = w[0];
                end = w[1];
                break;
            }
        }
        // SkParagraph's ICU has no CJK dictionary: every Han ideograph and Hiragana character is its own word.
        if let Some(c) = text[byte..].chars().next() {
            let script = CodePointMapData::<Script>::new().get(c);
            if script == Script::Han || script == Script::Hiragana {
                return (p.start + map.at(byte), p.start + map.at(byte + c.len_utf8()));
            }
        }
        (p.start + map.at(start), p.start + map.at(end))
    }

    /// Vertical caret movement: the offset one line above (`dir < 0`) or below at the same x.
    pub fn vertical_move(&self, offset: usize, dir: i32) -> usize {
        let c = self.caret_rect(offset);
        let y = if dir < 0 { c.y - 2.0 } else { c.y + c.h + 2.0 };
        if y < 0.0 {
            return 0;
        }
        if y > self.height {
            return self.length;
        }
        self.offset_at(c.x, y)
    }
}
