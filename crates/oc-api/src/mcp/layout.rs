//! Port of services/api/src/mcp/layout.ts — layout helpers for AI assistants until the editor has auto layout:
//! stack items in a row or a column, or place them on a grid. Pure: takes boxes in order and returns each item's
//! new page-space box.

#[derive(Debug, Clone, PartialEq)]
pub struct LBox {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Justify {
    Align(Align),
    SpaceBetween,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Row,
    Column,
    Grid,
}

#[derive(Debug, Clone)]
pub struct ArrangeOptions {
    pub direction: Direction,
    pub gap: Option<f64>,
    pub columns: Option<f64>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub align: Option<Align>,
    pub justify: Option<Justify>,
    pub stretch: bool,
}

fn offset(free: f64, a: Option<Align>) -> f64 {
    match a {
        Some(Align::Center) => free / 2.0,
        Some(Align::End) => free,
        _ => 0.0,
    }
}

fn min(v: impl Iterator<Item = f64>) -> f64 {
    v.fold(f64::INFINITY, |a, b| if b < a || b.is_nan() { b } else { a })
}

fn max(v: impl Iterator<Item = f64>) -> f64 {
    v.fold(f64::NEG_INFINITY, |a, b| if b > a || b.is_nan() { b } else { a })
}

pub fn arrange(items: &[LBox], o: &ArrangeOptions) -> Vec<LBox> {
    if items.is_empty() {
        return vec![];
    }
    let gap = o.gap.unwrap_or(0.0);
    let x0 = o.x.unwrap_or_else(|| min(items.iter().map(|b| b.x)));
    let y0 = o.y.unwrap_or_else(|| min(items.iter().map(|b| b.y)));
    let n = items.len() as f64;

    if o.direction == Direction::Grid {
        let cols = o.columns.unwrap_or_else(|| n.sqrt().ceil()).min(n).max(1.0);
        let rows = (n / cols).ceil();
        let cell_w = match o.width {
            Some(w) => (w - gap * (cols - 1.0)) / cols,
            None => max(items.iter().map(|b| b.w)),
        };
        let cell_h = match o.height {
            Some(h) => (h - gap * (rows - 1.0)) / rows,
            None => max(items.iter().map(|b| b.h)),
        };
        return items
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let i = i as f64;
                let c = i % cols;
                let r = (i / cols).floor();
                let w = if o.stretch { cell_w } else { b.w };
                let h = if o.stretch { cell_h } else { b.h };
                LBox { id: b.id.clone(), x: x0 + c * (cell_w + gap) + offset(cell_w - w, o.align), y: y0 + r * (cell_h + gap) + offset(cell_h - h, o.align), w, h }
            })
            .collect();
    }

    let row = o.direction == Direction::Row;
    let main = |b: &LBox| if row { b.w } else { b.h };
    let cross = |b: &LBox| if row { b.h } else { b.w };
    let content = items.iter().fold(0.0, |s, b| s + main(b));
    let main_size = (if row { o.width } else { o.height }).unwrap_or(content + gap * (n - 1.0));
    let cross_size = (if row { o.height } else { o.width }).unwrap_or_else(|| max(items.iter().map(cross)));
    let mut step = gap;
    let mut start = 0.0;
    match o.justify {
        Some(Justify::SpaceBetween) if items.len() > 1 => step = (main_size - content) / (n - 1.0),
        Some(Justify::Align(a)) => start = offset(main_size - content - gap * (n - 1.0), Some(a)),
        _ => {}
    }
    let mut cursor = start;
    items
        .iter()
        .map(|b| {
            let c = if o.stretch { cross_size } else { cross(b) };
            let along = cursor;
            cursor += main(b) + step;
            let across = offset(cross_size - c, o.align);
            if row {
                LBox { id: b.id.clone(), x: x0 + along, y: y0 + across, w: b.w, h: c }
            } else {
                LBox { id: b.id.clone(), x: x0 + across, y: y0 + along, w: c, h: b.h }
            }
        })
        .collect()
}
