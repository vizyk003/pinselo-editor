// SPDX-License-Identifier: Apache-2.0
//! Line breaking and justification with SkParagraph's rules (`TextWrapper::lookAhead/moveForward`,
//! `TextLine::justify`), over clusters shaped by parley. Break opportunities are ICU4X UAX #14 (CSS
//! `line-break: normal`, dictionaries for Thai/Lao/Khmer/Myanmar), as ICU gave SkParagraph.
//!
//! The differences from CSS that matter: a word wider than the whole line is broken greedily at cluster
//! boundaries and *fills the current line* when at least two of its glyphs fit there (CSS would move it to a new
//! line first); widths are compared to the line width after rounding to 0.01 px when within ±0.25 px
//! (`LineBreakerWithLittleRounding`).

use std::ops::Range;

/// One grapheme cluster in logical order, as the wrapper sees it.
#[derive(Debug, Clone)]
pub(crate) struct WrapCluster {
    pub width: f32,
    /// Breakable whitespace (`isWhitespaceBreak`).
    pub ws_break: bool,
    /// Non-breaking whitespace inside a word (`isIntraWordBreak`, e.g. NBSP).
    pub intra_ws: bool,
    /// A line break opportunity follows this cluster (`isSoftBreak`).
    pub soft_break: bool,
    /// Mandatory break after this cluster.
    pub hard_break: bool,
    pub glyphs: usize,
    /// Glyph index range of the cluster inside its SkParagraph run (visual order, so decreasing along the text
    /// in RTL runs). Only used for Skia's "is there room for part of a too-long word" test, which subtracts these
    /// positions even across runs.
    pub start_pos: usize,
    pub end_pos: usize,
}

struct LittleRound {
    lower: f32,
    max: f32,
    upper: f32,
}

impl LittleRound {
    fn new(max: f32) -> Self {
        LittleRound { lower: max - 0.25, max, upper: max + 0.25 }
    }

    fn break_line(&self, width: f32) -> bool {
        if width < self.lower {
            return false;
        }
        if width > self.upper {
            return true;
        }
        let v = width.abs();
        let rounded = if v < 10000.0 {
            (width * 100.0).round() * (1.0 / 100.0)
        } else if v < 100000.0 {
            (width * 10.0).round() * (1.0 / 10.0)
        } else {
            width.floor()
        };
        rounded > self.max
    }
}

/// Cluster index ranges of the lines (each includes its trailing whitespace).
pub(crate) fn break_lines(clusters: &[WrapCluster], max_width: f32) -> Vec<Range<usize>> {
    let n = clusters.len();
    let breaker = LittleRound::new(max_width);
    let mut lines = Vec::new();
    let mut start = 0usize;
    while start < n {
        // fWords = [start, words_end), fClusters = [cl_start, cl_end).
        let mut words_end = start;
        let mut words_w = 0.0f32;
        let mut cl_start = start;
        let mut cl_end = start;
        let mut cl_w = 0.0f32;
        // fClusters' first cluster (None while empty).
        let mut cl_first: Option<usize> = None;
        let mut too_long_word = false;
        let mut i = start;
        while i < n {
            let c = &clusters[i];
            if !c.hard_break && breaker.break_line(words_w + cl_w + c.width) {
                if c.ws_break {
                    // Whitespace hangs: it ends the word.
                    cl_w += c.width;
                    words_end = i + 1;
                    words_w += cl_w;
                    cl_w = 0.0;
                    cl_start = i + 1;
                    cl_end = i + 1;
                    cl_first = None;
                    i += 1;
                    continue;
                }
                // Is the rest of this word too long for any line?
                let mut next_len = cl_w;
                let mut next_short = next_len;
                let mut nbsp: Option<usize> = None;
                for (f, fc) in clusters.iter().enumerate().skip(i) {
                    if fc.soft_break || fc.hard_break || fc.ws_break {
                        break;
                    }
                    if next_len > 0.0 && next_len <= max_width && fc.intra_ws {
                        nbsp = Some(f);
                        next_short = next_len;
                    }
                    next_len += fc.width;
                }
                if next_len > max_width {
                    if let Some(nb) = nbsp {
                        if !breaker.break_line(words_w + next_short) {
                            // The part up to the no-break space fits on this line.
                            words_end = nb + 1;
                        }
                        cl_start = words_end;
                        cl_end = words_end;
                        break;
                    }
                    // `fClusters.endPos() - fClusters.startPos() > 1` (size_t arithmetic).
                    let span = match cl_first {
                        Some(f) if cl_end > cl_start => clusters[cl_end - 1].end_pos.wrapping_sub(clusters[f].start_pos),
                        _ => 0,
                    };
                    if span > 1 || words_end == start {
                        too_long_word = true;
                    }
                }
                if c.width > max_width {
                    // A single cluster wider than the line.
                    cl_end = i + 1;
                    too_long_word = true;
                }
                break;
            }
            cl_w += c.width;
            cl_first.get_or_insert(i);
            i += 1;
            cl_end = i;
            if c.soft_break || c.hard_break {
                words_end = i;
                words_w += cl_w;
                cl_w = 0.0;
                cl_start = i;
                cl_first = None;
            }
            if c.hard_break {
                break;
            }
        }
        if i >= n {
            words_end = words_end.max(cl_end);
            cl_start = cl_end;
        }
        // moveForward.
        let mut end = start;
        if words_end > start {
            end = words_end;
        }
        if (words_end == start || too_long_word) && cl_end > cl_start {
            end = cl_end;
        }
        if end <= start {
            end = start + 1;
        }
        // Whitespace after a break belongs to the line it follows (SkParagraph never starts a line with it).
        while end < n && clusters[end].ws_break {
            end += 1;
        }
        lines.push(start..end);
        start = end;
    }
    lines
}

/// Per-cluster x shifts for a justified line (`TextLine::justify`), clusters in visual order. `ghost` marks the
/// trailing whitespace, `ideographic` CJK ideographs (which get gaps on both sides). Returns `None` when there is
/// nothing to stretch.
pub(crate) fn justify_shifts(widths: &[f32], ws_break: &[bool], ideographic: &[bool], ghost: &[bool], max_width: f32) -> Option<Vec<f32>> {
    let mut patches = 0i32;
    let mut text_len = 0.0f32;
    let mut patch = false;
    let mut leading = false;
    let mut index = 0usize;
    for k in 0..widths.len() {
        if ghost[k] {
            continue;
        }
        if ws_break[k] {
            if index == 0 {
                leading = true;
            } else if !patch && !leading {
                patches += 1;
            }
            patch = !leading;
        } else if ideographic[k] {
            if !patch && index != 0 {
                patches += 1;
            }
            patch = true;
            leading = false;
            patches += 1;
        } else {
            patch = false;
            leading = false;
        }
        text_len += widths[k];
        index += 1;
    }
    if patch {
        patches -= 1;
    }
    if patches <= 0 {
        return None;
    }
    let step = (max_width - text_len) / patches as f32;
    let mut shifts = vec![0.0f32; widths.len()];
    let mut shift = 0.0f32;
    let mut patch = false;
    let mut leading = false;
    let mut index = 0usize;
    for k in 0..widths.len() {
        if ghost[k] {
            shifts[k] = shift;
            continue;
        }
        if ws_break[k] {
            // The gap opens after the first space of a patch: the space keeps its place and widens.
            shifts[k] = shift;
            if index == 0 {
                leading = true;
            } else if !patch && !leading {
                shift += step;
                patch = true;
            }
        } else if ideographic[k] {
            if !patch && index != 0 {
                shift += step;
            }
            shifts[k] = shift;
            shift += step;
            patch = true;
            leading = false;
        } else {
            patch = false;
            leading = false;
            shifts[k] = shift;
        }
        index += 1;
    }
    Some(shifts)
}
