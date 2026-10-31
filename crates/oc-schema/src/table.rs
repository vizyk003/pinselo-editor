// SPDX-License-Identifier: Apache-2.0
//! Table model helpers (ED-028, A12): geometry, merged cells, rich cell bodies, per-side borders, table style
//! presets, structure edits and row auto-height. Pure functions over `TableEl`.
//!
//! Cell positions are `i64` because the TS helpers accept (and do arithmetic on) out-of-range and negative
//! indices; the results for those match the JS arithmetic.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::color::{hexc, solid, solidc, to_hex};
use crate::elements::{CellBorder, CellBorders, CellSpan, CellVAlign, TableCell, TableEl, TableStyle};
use crate::factory::DEFAULT_RUN;
use crate::js;
use crate::macros::string_enum;
use crate::primitives::{ColorValue, Extra, Paint, Rect, Xf};
use crate::text::{Paragraph, Run, RunStyle, TextAlign, TextBody, body_plain_text, default_paragraph};

pub type Cells = IndexMap<String, TableCell>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellPos {
    pub r: i64,
    pub c: i64,
}

/// Inclusive cell rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellRange {
    pub r0: i64,
    pub c0: i64,
    pub r1: i64,
    pub c1: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub rows: i64,
    pub cols: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorSpan {
    pub r: i64,
    pub c: i64,
    pub rows: i64,
    pub cols: i64,
}

string_enum! {
    pub enum CellSide { T = "t", R = "r", B = "b", L = "l" }
}
string_enum! {
    pub enum BorderPreset { All = "all", Outer = "outer", Inner = "inner", Horizontal = "horizontal", None = "none" }
}
pub const BORDER_PRESETS: &[&str] = &["all", "outer", "inner", "horizontal", "none"];

pub fn cell_key(r: i64, c: i64) -> String {
    format!("{r}:{c}")
}

fn cell(cells: &Cells, r: i64, c: i64) -> Option<&TableCell> {
    cells.get(&cell_key(r, c))
}

fn side_of<'a>(b: &'a CellBorders, s: &CellSide) -> Option<&'a CellBorder> {
    match s {
        CellSide::T => b.t.as_ref(),
        CellSide::R => b.r.as_ref(),
        CellSide::B => b.b.as_ref(),
        CellSide::L => b.l.as_ref(),
        CellSide::Other(_) => None,
    }
}

fn set_side_of(b: &mut CellBorders, s: &CellSide, v: Option<CellBorder>) {
    match s {
        CellSide::T => b.t = v,
        CellSide::R => b.r = v,
        CellSide::B => b.b = v,
        CellSide::L => b.l = v,
        CellSide::Other(_) => {}
    }
}

// ------------------------------------------------------------------ geometry

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub ys: Vec<f64>,
    pub xs: Vec<f64>,
}

fn sum(v: &[f64]) -> f64 {
    v.iter().fold(0.0, |a, b| a + b)
}

/// Track boundaries in table-local px: `xs` has cols+1 entries, `ys` rows+1. Track sizes are relative.
pub fn table_geometry(rows: &[f64], cols: &[f64], xf: &Xf) -> Geometry {
    let or1 = |v: f64| if v == 0.0 || v.is_nan() { 1.0 } else { v };
    let (sr, sc) = (or1(sum(rows)), or1(sum(cols)));
    let mut ys = vec![0.0];
    for r in rows {
        ys.push(ys[ys.len() - 1] + (r / sr) * xf.h);
    }
    let mut xs = vec![0.0];
    for c in cols {
        xs.push(xs[xs.len() - 1] + (c / sc) * xf.w);
    }
    Geometry { ys, xs }
}

pub fn geometry_of(el: &TableEl) -> Geometry {
    table_geometry(&el.rows, &el.cols, &el.base.xf)
}

/// Span of an anchor cell, clipped to the table (1×1 when not merged).
pub fn cell_span(el: &TableEl, r: i64, c: i64) -> Span {
    let Some(s) = cell(&el.cells, r, c).and_then(|x| x.span.as_ref()) else { return Span { rows: 1, cols: 1 } };
    let (nr, nc) = (el.rows.len() as i64, el.cols.len() as i64);
    let clip = |v: f64, lim: i64| -> i64 {
        let f = v.floor();
        let f = if f.is_finite() { f as i64 } else { 1 };
        1.max(f.min(lim))
    };
    Span { rows: clip(s.rows, nr - r), cols: clip(s.cols, nc - c) }
}

/// Anchor index (r * cols + c) for every grid slot; overlapping later spans are treated as 1×1.
fn coverage(el: &TableEl) -> Vec<i64> {
    let (nr, nc) = (el.rows.len() as i64, el.cols.len() as i64);
    let mut map = vec![-1i64; (nr * nc).max(0) as usize];
    for r in 0..nr {
        for c in 0..nc {
            if map[(r * nc + c) as usize] != -1 {
                continue;
            }
            let s = cell_span(el, r, c);
            let mut free = true;
            'outer: for i in r..r + s.rows {
                for j in c..c + s.cols {
                    if map[(i * nc + j) as usize] != -1 {
                        free = false;
                        break 'outer;
                    }
                }
            }
            let (rs, cs) = if free { (s.rows, s.cols) } else { (1, 1) };
            for i in r..r + rs {
                for j in c..c + cs {
                    map[(i * nc + j) as usize] = r * nc + c;
                }
            }
        }
    }
    map
}

fn cov_at(map: &[i64], idx: i64) -> Option<i64> {
    if idx < 0 { None } else { map.get(idx as usize).copied() }
}

fn anchor_in(map: &[i64], nc: i64, r: i64, c: i64) -> CellPos {
    if nc == 0 {
        return CellPos { r, c };
    }
    let i = cov_at(map, r * nc + c).unwrap_or(r * nc + c);
    CellPos { r: i.div_euclid(nc), c: i % nc }
}

/// The anchor cell whose (merged) area contains (r, c).
pub fn anchor_of(el: &TableEl, r: i64, c: i64) -> CellPos {
    anchor_in(&coverage(el), el.cols.len() as i64, r, c)
}

/// True when (r, c) is hidden under another cell's merge.
pub fn is_covered(el: &TableEl, r: i64, c: i64) -> bool {
    let a = anchor_of(el, r, c);
    a.r != r || a.c != c
}

fn effective_in(el: &TableEl, map: &[i64], r: i64, c: i64) -> Span {
    let s = cell_span(el, r, c);
    let nc = el.cols.len() as i64;
    let me = r * nc + c;
    let mut rows = 0;
    while rows < s.rows && cov_at(map, (r + rows) * nc + c) == Some(me) {
        rows += 1;
    }
    let mut cols = 0;
    while cols < s.cols && cov_at(map, r * nc + c + cols) == Some(me) {
        cols += 1;
    }
    Span { rows: rows.max(1), cols: cols.max(1) }
}

/// Effective span honouring overlap resolution (what is actually drawn).
pub fn effective_span(el: &TableEl, r: i64, c: i64) -> Span {
    effective_in(el, &coverage(el), r, c)
}

/// Visible cells (anchors) in row-major order.
pub fn table_anchors(el: &TableEl) -> Vec<AnchorSpan> {
    let map = coverage(el);
    let nc = el.cols.len() as i64;
    let mut out = Vec::new();
    for r in 0..el.rows.len() as i64 {
        for c in 0..nc {
            let a = anchor_in(&map, nc, r, c);
            if a.r == r && a.c == c {
                let s = effective_in(el, &map, r, c);
                out.push(AnchorSpan { r, c, rows: s.rows, cols: s.cols });
            }
        }
    }
    out
}

fn at(v: &[f64], i: i64) -> f64 {
    if i < 0 { f64::NAN } else { v.get(i as usize).copied().unwrap_or(f64::NAN) }
}

/// Box of a (possibly merged) cell in table-local px.
pub fn cell_rect(el: &TableEl, r: i64, c: i64, geo: Option<&Geometry>) -> Rect {
    let own;
    let geo = match geo {
        Some(g) => g,
        None => {
            own = geometry_of(el);
            &own
        }
    };
    let a = anchor_of(el, r, c);
    let s = effective_span(el, a.r, a.c);
    let x = at(&geo.xs, a.c);
    let y = at(&geo.ys, a.r);
    Rect { x, y, w: at(&geo.xs, a.c + s.cols) - x, h: at(&geo.ys, a.r + s.rows) - y }
}

/// Grid slot under a table-local point (not resolved to its anchor), or `None` outside.
pub fn slot_at(el: &TableEl, x: f64, y: f64, geo: Option<&Geometry>) -> Option<CellPos> {
    let own;
    let geo = match geo {
        Some(g) => g,
        None => {
            own = geometry_of(el);
            &own
        }
    };
    if x < 0.0 || y < 0.0 || x > el.base.xf.w || y > el.base.xf.h {
        return None;
    }
    let nr = el.rows.len() as i64;
    let nc = el.cols.len() as i64;
    let mut r = nr - 1;
    for i in 0..nr {
        if y <= at(&geo.ys, i + 1) {
            r = i;
            break;
        }
    }
    let mut c = nc - 1;
    for j in 0..nc {
        if x <= at(&geo.xs, j + 1) {
            c = j;
            break;
        }
    }
    Some(CellPos { r, c })
}

/// Rectangle spanned by two cells, grown until no merged cell straddles its edge.
pub fn normalize_range(el: &TableEl, a: CellPos, b: CellPos) -> CellRange {
    let map = coverage(el);
    let nc = el.cols.len() as i64;
    let (mut r0, mut r1, mut c0, mut c1) = (a.r.min(b.r), a.r.max(b.r), a.c.min(b.c), a.c.max(b.c));
    let mut changed = true;
    while changed {
        changed = false;
        let mut r = r0;
        while r <= r1 {
            let mut c = c0;
            while c <= c1 {
                let an = anchor_in(&map, nc, r, c);
                let s = effective_in(el, &map, an.r, an.c);
                let (nr0, nc0, nr1, nc1) = (r0.min(an.r), c0.min(an.c), r1.max(an.r + s.rows - 1), c1.max(an.c + s.cols - 1));
                if nr0 != r0 || nc0 != c0 || nr1 != r1 || nc1 != c1 {
                    (r0, c0, r1, c1) = (nr0, nc0, nr1, nc1);
                    changed = true;
                }
                c += 1;
            }
            r += 1;
        }
    }
    CellRange { r0, c0, r1, c1 }
}

/// Anchors whose cell lies inside the range.
pub fn range_anchors(el: &TableEl, range: &CellRange) -> Vec<CellPos> {
    let map = coverage(el);
    let nc = el.cols.len() as i64;
    let mut out = Vec::new();
    for r in range.r0..=range.r1 {
        for c in range.c0..=range.c1 {
            let a = anchor_in(&map, nc, r, c);
            if a.r == r && a.c == c {
                out.push(CellPos { r, c });
            }
        }
    }
    out
}

// ------------------------------------------------------------------ rich cell content

/// Run style every cell inherits from the table style (header row colour / weight included).
pub fn table_default_run(style: &TableStyle, r: i64, c: Option<&TableCell>) -> RunStyle {
    let header = r == 0;
    let color: ColorValue = match (&style.header_color, header) {
        (Some(hc), true) => hc.clone(),
        _ => style.color.clone(),
    };
    let bold = c.and_then(|c| c.bold) == Some(true) || (header && style.header_bold == Some(true));
    let mut out = DEFAULT_RUN.clone();
    out.family = style.family.clone();
    out.size = style.size;
    out.weight = if bold { 700.0 } else { 400.0 };
    out.fill = solid(color);
    out
}

fn legacy_paragraphs(c: Option<&TableCell>) -> Vec<Paragraph> {
    let text = c.map_or("", |c| c.text.as_str());
    let align = c.and_then(|c| c.align.as_ref()).map_or(TextAlign::Start, |a| a.to_text_align());
    let attrs = crate::text::ParagraphAttrs { align, ..default_paragraph() };
    text.split('\n').map(|line| attrs.with_runs(if line.is_empty() { vec![] } else { vec![Run::plain(line)] })).collect()
}

/// The cell's rich body (rich cells use `body`, legacy cells `text`/`bold`/`align`).
pub fn cell_body(el: &TableEl, r: i64, c: i64) -> TextBody {
    let x = cell(&el.cells, r, c);
    let paragraphs = match x.and_then(|x| x.body.as_ref()) {
        Some(b) if !b.paragraphs.is_empty() => b.paragraphs.clone(),
        _ => legacy_paragraphs(x),
    };
    TextBody { default_run: table_default_run(&el.style, r, x), paragraphs, extra: Extra::new() }
}

/// Normalise a legacy cell to the rich shape.
pub fn migrate_cell(el: &TableEl, r: i64, c: i64) -> TableCell {
    let x = cell(&el.cells, r, c).cloned().unwrap_or_else(|| TableCell::text(""));
    if x.body.is_some() {
        return x;
    }
    with_body(Some(&x), cell_body(el, r, c))
}

/// Cell with a new body and its plain-text mirror kept in sync.
pub fn with_body(c: Option<&TableCell>, body: TextBody) -> TableCell {
    let mut out = c.cloned().unwrap_or_default();
    out.text = body_plain_text(&body);
    out.body = Some(body);
    out
}

pub fn cell_plain_text(el: &TableEl, r: i64, c: i64) -> String {
    match cell(&el.cells, r, c) {
        Some(x) => match &x.body {
            Some(b) => body_plain_text(b),
            None => x.text.clone(),
        },
        None => String::new(),
    }
}

pub fn cell_padding(el: &TableEl, r: i64, c: i64) -> f64 {
    cell(&el.cells, r, c).and_then(|x| x.padding).unwrap_or(el.style.padding)
}

/// Text box of a cell (inside its padding), table-local px.
pub fn cell_content_box(el: &TableEl, r: i64, c: i64, geo: Option<&Geometry>) -> Rect {
    let a = anchor_of(el, r, c);
    let bx = cell_rect(el, a.r, a.c, geo);
    let pad = cell_padding(el, a.r, a.c).min(bx.w / 2.0 - 0.5).min(bx.h / 2.0 - 0.5);
    let p = 0f64.max(pad);
    Rect { x: bx.x + p, y: bx.y + p, w: 1f64.max(bx.w - p * 2.0), h: 1f64.max(bx.h - p * 2.0) }
}

/// Vertical offset of laid-out text inside the content box for the cell's vertical alignment.
pub fn cell_text_offset(el: &TableEl, r: i64, c: i64, box_h: f64, text_h: f64) -> f64 {
    match cell(&el.cells, r, c).and_then(|x| x.valign.clone()).unwrap_or(CellVAlign::Middle) {
        CellVAlign::Top => 0.0,
        CellVAlign::Bottom => box_h - text_h,
        _ => (box_h - text_h) / 2.0,
    }
}

// ------------------------------------------------------------------ merge / unmerge

/// Merge a range into its top-left cell. Contents of the other non-empty cells are appended as paragraphs.
pub fn merge_cells(el: &TableEl, range: &CellRange) -> Cells {
    let rg = normalize_range(el, CellPos { r: range.r0, c: range.c0 }, CellPos { r: range.r1, c: range.c1 });
    let mut cells = el.cells.clone();
    let mut paras: Vec<Paragraph> = Vec::new();
    let first = cell(&el.cells, rg.r0, rg.c0);
    let mut others = false;
    for p in range_anchors(el, &rg) {
        if cell_plain_text(el, p.r, p.c).is_empty() {
            continue;
        }
        paras.extend(cell_body(el, p.r, p.c).paragraphs);
        if p.r != rg.r0 || p.c != rg.c0 {
            others = true;
        }
    }
    for r in rg.r0..=rg.r1 {
        for c in rg.c0..=rg.c1 {
            if r != rg.r0 || c != rg.c0 {
                cells.shift_remove(&cell_key(r, c));
            }
        }
    }
    let span = Span { rows: rg.r1 - rg.r0 + 1, cols: rg.c1 - rg.c0 + 1 };
    let mut base = first.cloned().unwrap_or_else(|| TableCell::text(""));
    base.span = None;
    let body = TextBody {
        default_run: table_default_run(&el.style, rg.r0, first),
        paragraphs: if paras.is_empty() { vec![default_paragraph().with_runs(vec![])] } else { paras },
        extra: Extra::new(),
    };
    let mut merged = if others { with_body(Some(&base), body) } else { base };
    if span.rows * span.cols > 1 {
        merged.span = Some(CellSpan { rows: span.rows as f64, cols: span.cols as f64, extra: Extra::new() });
    }
    cells.insert(cell_key(rg.r0, rg.c0), merged);
    cells
}

/// Split every merged cell intersecting the range (content stays in the top-left cell).
pub fn unmerge_cells(el: &TableEl, range: &CellRange) -> Cells {
    let map = coverage(el);
    let nc = el.cols.len() as i64;
    let mut cells = el.cells.clone();
    for r in range.r0..=range.r1 {
        for c in range.c0..=range.c1 {
            let a = anchor_in(&map, nc, r, c);
            if let Some(x) = cells.get_mut(&cell_key(a.r, a.c)) {
                x.span = None;
            }
        }
    }
    cells
}

pub fn can_unmerge(el: &TableEl, range: &CellRange) -> bool {
    let map = coverage(el);
    let nc = el.cols.len() as i64;
    for r in range.r0..=range.r1 {
        for c in range.c0..=range.c1 {
            let a = anchor_in(&map, nc, r, c);
            let s = effective_in(el, &map, a.r, a.c);
            if s.rows > 1 || s.cols > 1 {
                return true;
            }
        }
    }
    false
}

// ------------------------------------------------------------------ structure

string_enum! {
    pub enum StructureAction { AddRow = "addRow", AddCol = "addCol", DelRow = "delRow", DelCol = "delCol" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructureEdit {
    pub rows: Vec<f64>,
    pub cols: Vec<f64>,
    pub cells: Cells,
    pub xf: Xf,
}

/// `Array#splice` start index normalisation.
fn splice_index(start: i64, len: usize) -> usize {
    let len = len as i64;
    (if start < 0 { (len + start).max(0) } else { start.min(len) }) as usize
}

/// Insert a row/column after index `at` (-1 = before the first) or delete the one at `at`.
pub fn table_structure_edit(el: &TableEl, action: &StructureAction, at: i64) -> Option<StructureEdit> {
    let is_row = matches!(action, StructureAction::AddRow | StructureAction::DelRow);
    let add = matches!(action, StructureAction::AddRow | StructureAction::AddCol);
    let mut tracks = if is_row { el.rows.clone() } else { el.cols.clone() };
    let len = tracks.len() as i64;
    if !add && (len < 2 || at < 0 || at >= len) {
        return None;
    }
    let s0 = sum(&tracks);
    let total = if s0 == 0.0 || s0.is_nan() { 1.0 } else { s0 };
    let extent = if is_row { el.base.xf.h } else { el.base.xf.w };
    let mut size = extent;
    if add {
        let idx = 0.max((len - 1).min(at));
        let rf = if idx < len { tracks[idx as usize] } else { total / len as f64 };
        let pos = splice_index(at + 1, tracks.len());
        tracks.insert(pos, rf);
        size += (rf / total) * extent;
    } else {
        size -= (tracks[at as usize] / total) * extent;
        tracks.remove(at as usize);
    }
    let map = coverage(el);
    let nc = el.cols.len() as i64;
    let mut cells = Cells::new();
    for (k, x) in &el.cells {
        let parts: Vec<&str> = k.split(':').collect();
        if parts.len() < 2 {
            continue;
        }
        let (rf, cf) = (js::string_to_number(parts[0]), js::string_to_number(parts[1]));
        if !js::is_integer(rf) || !js::is_integer(cf) {
            continue;
        }
        let (r, c) = (rf as i64, cf as i64);
        let a = anchor_in(&map, nc, r, c);
        if a.r != r || a.c != c {
            continue;
        }
        let i = if is_row { r } else { c };
        let s = effective_in(el, &map, r, c);
        let n = if is_row { s.rows } else { s.cols };
        let (mut ni, mut nn) = (i, n);
        if add {
            if i > at {
                ni = i + 1;
            } else if i + n - 1 > at {
                nn = n + 1;
            }
        } else if i > at {
            ni = i - 1;
        } else if i == at {
            if n == 1 {
                continue;
            }
            nn = n - 1;
        } else if i + n > at {
            nn = n - 1;
        }
        let mut next = x.clone();
        let span = if is_row { Span { rows: nn, cols: s.cols } } else { Span { rows: s.rows, cols: nn } };
        next.span = if span.rows > 1 || span.cols > 1 {
            Some(CellSpan { rows: span.rows as f64, cols: span.cols as f64, extra: Extra::new() })
        } else {
            None
        };
        cells.insert(if is_row { cell_key(ni, c) } else { cell_key(r, ni) }, next);
    }
    let mut xf = el.base.xf.clone();
    if is_row {
        xf.h = 20f64.max(size);
    } else {
        xf.w = 20f64.max(size);
    }
    Some(StructureEdit {
        rows: if is_row { tracks.clone() } else { el.rows.clone() },
        cols: if is_row { el.cols.clone() } else { tracks },
        cells,
        xf,
    })
}

// ------------------------------------------------------------------ borders

pub fn default_border(style: &TableStyle) -> CellBorder {
    CellBorder::new(style.border_width, style.border_color.clone())
}

fn same_border(a: &CellBorder, b: &CellBorder) -> bool {
    a.width == b.width && a.color == b.color
}

/// Border of a grid edge shared by two cells: an explicit side beats the table default; wider wins a tie.
fn resolve_edge(a: Option<&CellBorder>, b: Option<&CellBorder>, def: &CellBorder) -> CellBorder {
    match (a, b) {
        (None, None) => def.clone(),
        (None, Some(b)) => b.clone(),
        (Some(a), None) => a.clone(),
        (Some(a), Some(b)) => {
            if b.width > a.width {
                b.clone()
            } else {
                a.clone()
            }
        }
    }
}

fn border_at<'a>(cells: &'a Cells, p: Option<CellPos>, s: &CellSide) -> Option<&'a CellBorder> {
    let p = p?;
    cell(cells, p.r, p.c).and_then(|x| x.borders.as_ref()).and_then(|b| side_of(b, s))
}

/// Resolved border on one side of an anchor cell.
pub fn cell_side_border(el: &TableEl, r: i64, c: i64, side: &CellSide) -> CellBorder {
    let map = coverage(el);
    let nc = el.cols.len() as i64;
    let s = effective_in(el, &map, r, c);
    let own = border_at(&el.cells, Some(CellPos { r, c }), side);
    let (nr, ncol, opp) = match side {
        CellSide::T => (r - 1, c, CellSide::B),
        CellSide::B => (r + s.rows, c, CellSide::T),
        CellSide::L => (r, c - 1, CellSide::R),
        _ => (r, c + s.cols, CellSide::L),
    };
    let nb = if nr >= 0 && ncol >= 0 && nr < el.rows.len() as i64 && ncol < nc {
        Some(anchor_in(&map, nc, nr, ncol))
    } else {
        None
    };
    resolve_edge(own, border_at(&el.cells, nb, &opp), &default_border(&el.style))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableEdge {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub border: CellBorder,
}

/// Visible border segments (merged interiors skipped, collinear runs joined), table-local px.
pub fn table_edges(el: &TableEl, geo: Option<&Geometry>) -> Vec<TableEdge> {
    let own;
    let geo = match geo {
        Some(g) => g,
        None => {
            own = geometry_of(el);
            &own
        }
    };
    let (nr, nc) = (el.rows.len() as i64, el.cols.len() as i64);
    let map = coverage(el);
    let def = default_border(&el.style);
    let mut out: Vec<TableEdge> = Vec::new();
    let mut push = |e: TableEdge| {
        if let Some(last) = out.last_mut()
            && same_border(&last.border, &e.border)
            && last.x2 == e.x1
            && last.y2 == e.y1
            && (last.x1 == last.x2) == (e.x1 == e.x2)
        {
            last.x2 = e.x2;
            last.y2 = e.y2;
            return;
        }
        out.push(e);
    };
    for i in 0..=nr {
        for j in 0..nc {
            let above = (i > 0).then(|| anchor_in(&map, nc, i - 1, j));
            let below = (i < nr).then(|| anchor_in(&map, nc, i, j));
            if let (Some(a), Some(b)) = (above, below)
                && a == b
            {
                continue;
            }
            let b = resolve_edge(border_at(&el.cells, above, &CellSide::B), border_at(&el.cells, below, &CellSide::T), &def);
            if b.width > 0.0 {
                push(TableEdge { x1: at(&geo.xs, j), y1: at(&geo.ys, i), x2: at(&geo.xs, j + 1), y2: at(&geo.ys, i), border: b });
            }
        }
    }
    for j in 0..=nc {
        for i in 0..nr {
            let left = (j > 0).then(|| anchor_in(&map, nc, i, j - 1));
            let right = (j < nc).then(|| anchor_in(&map, nc, i, j));
            if let (Some(a), Some(b)) = (left, right)
                && a == b
            {
                continue;
            }
            let b = resolve_edge(border_at(&el.cells, left, &CellSide::R), border_at(&el.cells, right, &CellSide::L), &def);
            if b.width > 0.0 {
                push(TableEdge { x1: at(&geo.xs, j), y1: at(&geo.ys, i), x2: at(&geo.xs, j), y2: at(&geo.ys, i + 1), border: b });
            }
        }
    }
    out
}

/// Apply a border preset to a range (`all`, `outer`, `inner`, `horizontal`, `none`). Neighbours outside the range
/// get the matching facing side. An unknown preset clears the sides (TS sets them to `undefined`).
pub fn apply_border_preset(el: &TableEl, range: &CellRange, preset: &BorderPreset, border: &CellBorder) -> Cells {
    let rg = normalize_range(el, CellPos { r: range.r0, c: range.c0 }, CellPos { r: range.r1, c: range.c1 });
    let off = CellBorder::new(0.0, border.color.clone());
    let mut cells = el.cells.clone();
    let set_side = |cells: &mut Cells, p: CellPos, s: &CellSide, v: Option<CellBorder>| {
        let k = cell_key(p.r, p.c);
        let mut x = cells.get(&k).cloned().unwrap_or_else(|| TableCell::text(""));
        let mut b = x.borders.take().unwrap_or_default();
        set_side_of(&mut b, s, v);
        x.borders = Some(b);
        cells.insert(k, x);
    };
    let pick = |outer: bool, horizontal: bool| -> Option<CellBorder> {
        match preset {
            BorderPreset::All => Some(border.clone()),
            BorderPreset::Outer => Some(if outer { border.clone() } else { off.clone() }),
            BorderPreset::Inner => Some(if outer { off.clone() } else { border.clone() }),
            BorderPreset::Horizontal => Some(if horizontal { border.clone() } else { off.clone() }),
            BorderPreset::None => Some(off.clone()),
            BorderPreset::Other(_) => None,
        }
    };
    let map = coverage(el);
    let nc = el.cols.len() as i64;
    for a in range_anchors(el, &rg) {
        let s = effective_in(el, &map, a.r, a.c);
        set_side(&mut cells, a, &CellSide::T, pick(a.r == rg.r0, true));
        set_side(&mut cells, a, &CellSide::B, pick(a.r + s.rows - 1 == rg.r1, true));
        set_side(&mut cells, a, &CellSide::L, pick(a.c == rg.c0, false));
        set_side(&mut cells, a, &CellSide::R, pick(a.c + s.cols - 1 == rg.c1, false));
    }
    let mut seen: HashSet<String> = HashSet::new();
    let nr = el.rows.len() as i64;
    let mut neighbour = |cells: &mut Cells, r: i64, c: i64, s: CellSide, v: Option<CellBorder>| {
        if r < 0 || c < 0 || r >= nr || c >= nc {
            return;
        }
        let a = anchor_in(&map, nc, r, c);
        let k = format!("{}{}", cell_key(a.r, a.c), s.as_str());
        if !seen.insert(k) {
            return;
        }
        set_side(cells, a, &s, v);
    };
    for c in rg.c0..=rg.c1 {
        neighbour(&mut cells, rg.r0 - 1, c, CellSide::B, pick(true, true));
        neighbour(&mut cells, rg.r1 + 1, c, CellSide::T, pick(true, true));
    }
    for r in rg.r0..=rg.r1 {
        neighbour(&mut cells, r, rg.c0 - 1, CellSide::R, pick(true, false));
        neighbour(&mut cells, r, rg.c1 + 1, CellSide::L, pick(true, false));
    }
    cells
}

// ------------------------------------------------------------------ style presets

string_enum! {
    pub enum TableStylePreset {
        Minimal = "minimal", Striped = "striped", HeaderDark = "header-dark", Bordered = "bordered", Card = "card",
        Colourful = "colourful",
    }
}
pub const TABLE_STYLE_PRESETS: &[&str] = &["minimal", "striped", "header-dark", "bordered", "card", "colourful"];

struct PresetDef {
    color: &'static str,
    border_color: &'static str,
    border_width: f64,
    header_fill: Option<&'static str>,
    header_color: &'static str,
    stripe_fill: Option<&'static str>,
    borders: &'static [(BorderPresetConst, f64, &'static str)],
}

#[derive(Clone, Copy)]
enum BorderPresetConst {
    Horizontal,
    Outer,
}

fn preset_def(p: &TableStylePreset) -> Option<PresetDef> {
    use BorderPresetConst as B;
    Some(match p {
        TableStylePreset::Minimal => PresetDef {
            color: "#1f2328",
            border_color: "#e5e7eb",
            border_width: 1.0,
            header_fill: None,
            header_color: "#111827",
            stripe_fill: None,
            borders: &[(B::Horizontal, 1.0, "#e5e7eb")],
        },
        TableStylePreset::Striped => PresetDef {
            color: "#1f2328",
            border_color: "#ddd6fe",
            border_width: 1.0,
            header_fill: Some("#7c3aed"),
            header_color: "#ffffff",
            stripe_fill: Some("#f5f3ff"),
            borders: &[(B::Horizontal, 1.0, "#ddd6fe")],
        },
        TableStylePreset::HeaderDark => PresetDef {
            color: "#1f2328",
            border_color: "#d1d5db",
            border_width: 1.0,
            header_fill: Some("#111827"),
            header_color: "#ffffff",
            stripe_fill: None,
            borders: &[],
        },
        TableStylePreset::Bordered => PresetDef {
            color: "#111827",
            border_color: "#111827",
            border_width: 2.0,
            header_fill: None,
            header_color: "#111827",
            stripe_fill: None,
            borders: &[],
        },
        TableStylePreset::Card => PresetDef {
            color: "#334155",
            border_color: "#e2e8f0",
            border_width: 1.0,
            header_fill: Some("#f1f5f9"),
            header_color: "#0f172a",
            stripe_fill: None,
            borders: &[(B::Horizontal, 1.0, "#e2e8f0"), (B::Outer, 2.0, "#cbd5e1")],
        },
        TableStylePreset::Colourful => PresetDef {
            color: "#431407",
            border_color: "#ffffff",
            border_width: 2.0,
            header_fill: Some("#f97316"),
            header_color: "#ffffff",
            stripe_fill: Some("#ffedd5"),
            borders: &[],
        },
        TableStylePreset::Other(_) => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleSwatch {
    pub header: String,
    pub body: String,
    pub stripe: String,
    pub border: String,
}

/// Per-preset swatch colours for the picker (header, stripe/body, border).
pub fn table_style_swatch(preset: &TableStylePreset) -> Option<StyleSwatch> {
    let p = preset_def(preset)?;
    let hex_of = |x: Option<&str>| x.map_or_else(|| "#ffffff".to_string(), |h| to_hex(&hexc(h), false));
    let border = p.borders.last().map_or_else(|| to_hex(&hexc(p.border_color), false), |b| b.2.to_string());
    Some(StyleSwatch { header: hex_of(p.header_fill), body: "#ffffff".into(), stripe: hex_of(p.stripe_fill), border })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyledTable {
    pub style: TableStyle,
    pub cells: Cells,
}

/// Apply a table style preset. Keeps content, merges, fonts, padding and per-cell alignment; resets per-cell
/// fills and borders. `None` for an unknown preset (TS throws).
pub fn apply_table_style(el: &TableEl, preset: &TableStylePreset) -> Option<StyledTable> {
    let def = preset_def(preset)?;
    let mut style = el.style.clone();
    style.color = hexc(def.color);
    style.border_color = hexc(def.border_color);
    style.border_width = def.border_width;
    style.header_fill = def.header_fill.map(solidc);
    style.header_color = Some(hexc(def.header_color));
    style.header_bold = Some(true);
    style.stripe_fill = def.stripe_fill.map(solidc);
    style.preset = Some(preset.as_str().to_string());
    let mut next = el.clone();
    next.style = style.clone();
    next.cells = el
        .cells
        .iter()
        .map(|(k, x)| {
            let mut x = x.clone();
            x.fill = None;
            x.borders = None;
            (k.clone(), x)
        })
        .collect();
    let all = CellRange { r0: 0, c0: 0, r1: el.rows.len() as i64 - 1, c1: el.cols.len() as i64 - 1 };
    for (bp, width, color) in def.borders {
        let preset = match bp {
            BorderPresetConst::Horizontal => BorderPreset::Horizontal,
            BorderPresetConst::Outer => BorderPreset::Outer,
        };
        next.cells = apply_border_preset(&next, &all, &preset, &CellBorder::new(*width, hexc(color)));
    }
    Some(StyledTable { style, cells: next.cells })
}

/// Fill a cell shows: its own, else header / stripe from the table style.
pub fn cell_fill(el: &TableEl, r: i64, c: i64) -> Option<Paint> {
    if let Some(f) = cell(&el.cells, r, c).and_then(|x| x.fill.clone()) {
        return Some(f);
    }
    if r == 0 {
        el.style.header_fill.clone()
    } else if r % 2 == 0 {
        el.style.stripe_fill.clone()
    } else {
        None
    }
}

// ------------------------------------------------------------------ auto height

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FitRows {
    pub rows: Vec<f64>,
    pub h: f64,
}

/// Rows grow to fit their content (A12). `measure(key, body, width)` returns the laid-out text height.
/// Returns the new track sizes (px) or `None` when every row already fits. Rows never shrink.
pub fn table_fit_rows(el: &TableEl, measure: &mut dyn FnMut(&str, &TextBody, f64) -> f64, id: Option<&str>) -> Option<FitRows> {
    let id = id.unwrap_or("table");
    let geo = geometry_of(el);
    let mut hs: Vec<f64> = (0..el.rows.len()).map(|i| geo.ys[i + 1] - geo.ys[i]).collect();
    let mut anchors = table_anchors(el);
    anchors.sort_by_key(|a| a.rows);
    let mut grew = false;
    for a in anchors {
        if cell_plain_text(el, a.r, a.c).is_empty() {
            continue;
        }
        let pad = cell_padding(el, a.r, a.c);
        let bx = cell_content_box(el, a.r, a.c, Some(&geo));
        let need = measure(&format!("{id}:{}:{}", a.r, a.c), &cell_body(el, a.r, a.c), bx.w) + pad * 2.0;
        let mut have = 0.0;
        for i in a.r..a.r + a.rows {
            have += hs[i as usize];
        }
        if need > have + 0.5 {
            hs[(a.r + a.rows - 1) as usize] += need - have;
            grew = true;
        }
    }
    if !grew {
        return None;
    }
    let rows: Vec<f64> = hs.iter().map(|h| js::to_fixed_num(*h, 2)).collect();
    let h = sum(&rows);
    Some(FitRows { rows, h })
}

/// Even column widths (keeps the box width).
pub fn distribute_columns(cols: &[f64]) -> Vec<f64> {
    cols.iter().map(|_| 1.0 / cols.len() as f64).collect()
}

/// Move the boundary after column `i` to table-local x, keeping both neighbours at least `min` px wide.
pub fn resize_column(el: &TableEl, i: i64, x: f64, min: Option<f64>) -> Vec<f64> {
    let min = min.unwrap_or(16.0);
    let xs = geometry_of(el).xs;
    if i < 0 || i >= el.cols.len() as i64 - 1 {
        return el.cols.clone();
    }
    let iu = i as usize;
    let lo = xs[iu] + min;
    let hi = xs[iu + 2] - min;
    let nx = lo.max(hi.min(x));
    let mut widths: Vec<f64> = (0..el.cols.len()).map(|j| xs[j + 1] - xs[j]).collect();
    widths[iu] = nx - xs[iu];
    widths[iu + 1] = xs[iu + 2] - nx;
    widths.iter().map(|w| js::to_fixed_num(w / el.base.xf.w, 5)).collect()
}
