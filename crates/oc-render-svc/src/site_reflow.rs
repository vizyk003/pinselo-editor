// SPDX-License-Identifier: Apache-2.0
//! Mobile reflow planning for published websites (pure, no rendering) — port of
//! `services/render/src/siteReflow.ts`.
//!
//! A desktop page is a free-form canvas. On phones each page is re-laid out as one column:
//!  1. Elements that cover (almost) the whole page become the section background.
//!  2. "Containers" (shapes, photos, frames) take the elements that sit on top of them as children, so a button
//!     label stays on its button and a headline stays on its hero photo.
//!  3. The remaining elements are grouped into rows by vertical overlap; within a row they are split into columns
//!     by horizontal overlap; columns are read left → right, each top → bottom.
//!
//! Item identity is by reference (`===` in the TS), parents are keyed by item id like the TS maps (ids are unique
//! within a design).

use std::collections::HashMap;

use oc_schema::primitives::Rect;

/// What the planner needs to know about an element.
pub trait ReflowItem {
    fn id(&self) -> &str;
    fn bx(&self) -> Rect;
    /// Paint order (higher = on top).
    fn z(&self) -> f64;
    /// Can hold the elements drawn on top of it (shapes, photos, frames).
    fn container(&self) -> bool;
    /// Visual (not text/form): can be a full-page background.
    fn visual(&self) -> bool;
}

/// Anything with a box (items and plan nodes).
pub trait Boxed {
    fn bx(&self) -> Rect;
}

impl<T: ReflowItem> Boxed for T {
    fn bx(&self) -> Rect {
        ReflowItem::bx(self)
    }
}

/// A plain item (tests, the TS `ReflowItem` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub bx: Rect,
    pub z: f64,
    pub container: bool,
    pub visual: bool,
}

impl ReflowItem for Item {
    fn id(&self) -> &str {
        &self.id
    }
    fn bx(&self) -> Rect {
        self.bx
    }
    fn z(&self) -> f64 {
        self.z
    }
    fn container(&self) -> bool {
        self.container
    }
    fn visual(&self) -> bool {
        self.visual
    }
}

/// `ReflowNode`: an item and the rows of elements on top of it (when it is a container).
#[derive(Debug, Clone)]
pub struct ReflowNode<'a, T> {
    pub item: &'a T,
    pub bx: Rect,
    pub rows: Vec<Vec<ReflowNode<'a, T>>>,
}

#[derive(Debug, Clone)]
pub struct ReflowPlan<'a, T> {
    pub background: Vec<&'a T>,
    pub rows: Vec<Vec<ReflowNode<'a, T>>>,
}

fn area(b: &Rect) -> f64 {
    b.w.max(0.0) * b.h.max(0.0)
}

fn intersection(a: &Rect, b: &Rect) -> f64 {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

fn overlap1d(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// `a - b || c - d` as an ordering (finite values).
fn by(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

/// Does `outer` contain most (≥ 80 %) of `inner`, with `inner` clearly smaller?
pub fn contains(outer: &Rect, inner: &Rect) -> bool {
    let ia = area(inner);
    if ia <= 0.0 || area(outer) <= ia * 1.05 {
        return false;
    }
    intersection(outer, inner) >= ia * 0.8
}

/// Group boxes into rows: an item joins the current row when it overlaps the row's vertical band by at least
/// half of the smaller height. Rows come out top → bottom.
pub fn group_rows<'a, T: Boxed>(items: &[&'a T]) -> Vec<Vec<&'a T>> {
    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| by(a.bx().y, b.bx().y).then(by(a.bx().x, b.bx().x)));
    let mut rows: Vec<(f64, f64, Vec<&'a T>)> = Vec::new();
    for it in sorted {
        let b = it.bx();
        let (top, bottom) = (b.y, b.y + b.h);
        if let Some(row) = rows.last_mut() {
            let ov = overlap1d(row.0, row.1, top, bottom);
            let min_h = (row.1 - row.0).min(bottom - top);
            if ov > 0.0 && ov >= min_h * 0.5 - 0.001 {
                row.2.push(it);
                row.1 = row.1.max(bottom);
                continue;
            }
        }
        rows.push((top, bottom, vec![it]));
    }
    rows.into_iter().map(|r| r.2).collect()
}

/// Reading order inside one row: split into columns (items that overlap horizontally share a column), columns
/// left → right, items in a column top → bottom.
pub fn row_reading_order<'a, T: Boxed>(row: &[&'a T]) -> Vec<&'a T> {
    let mut by_x = row.to_vec();
    by_x.sort_by(|a, b| by(a.bx().x, b.bx().x).then(by(a.bx().y, b.bx().y)));
    let mut cols: Vec<(f64, f64, Vec<&'a T>)> = Vec::new();
    for it in by_x {
        let b = it.bx();
        let (l, r) = (b.x, b.x + b.w);
        // Merge with any column it overlaps horizontally (by ≥ 30 % of the narrower of the two).
        if let Some(col) = cols
            .iter_mut()
            .find(|c| overlap1d(c.0, c.1, l, r) >= (c.1 - c.0).min(r - l) * 0.3)
        {
            col.2.push(it);
            col.0 = col.0.min(l);
            col.1 = col.1.max(r);
        } else {
            cols.push((l, r, vec![it]));
        }
    }
    cols.sort_by(|a, b| by(a.0, b.0));
    cols.into_iter()
        .flat_map(|mut c| {
            c.2.sort_by(|a, b| by(a.bx().y, b.bx().y).then(by(a.bx().x, b.bx().x)));
            c.2
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Axis {
    X,
    Y,
}

/// Split boxes at whitespace gaps (≥ `min_gap`) along one axis; groups come out in order.
fn cut_at<'a, T: Boxed>(
    items: &[&'a T],
    axis: Axis,
    min_gap: f64,
    widest_only: bool,
) -> Vec<Vec<&'a T>> {
    let lo = |b: &Rect| if axis == Axis::X { b.x } else { b.y };
    let hi = |b: &Rect| {
        if axis == Axis::X {
            b.x + b.w
        } else {
            b.y + b.h
        }
    };
    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| by(lo(&a.bx()), lo(&b.bx())));
    // Gaps before each item (−∞ where it overlaps what came before).
    let mut gaps: Vec<f64> = Vec::with_capacity(sorted.len());
    let mut end = f64::NEG_INFINITY;
    for it in &sorted {
        let b = it.bx();
        gaps.push(lo(&b) - end);
        end = end.max(hi(&b));
    }
    let widest = gaps
        .iter()
        .skip(1)
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let cut = if widest_only {
        min_gap.max(widest * 0.8)
    } else {
        min_gap
    };
    let mut groups: Vec<Vec<&'a T>> = Vec::new();
    for (i, it) in sorted.into_iter().enumerate() {
        if groups.is_empty() || gaps[i] >= cut {
            groups.push(vec![it]);
        } else if let Some(g) = groups.last_mut() {
            g.push(it);
        }
    }
    groups
}

/// Rows (top → bottom), each in reading order — a recursive XY-cut. A region splits at clean whitespace: into
/// columns when every column holds 3+ items (menus, feature lists: read each column top to bottom), otherwise
/// into horizontal bands first. Regions with no clean cut fall back to row grouping by vertical overlap.
pub fn reading_rows<'a, T: Boxed>(items: &[&'a T], min_gap: f64, depth: u32) -> Vec<Vec<&'a T>> {
    if items.len() <= 1 {
        return if items.is_empty() {
            vec![]
        } else {
            vec![items.to_vec()]
        };
    }
    if depth > 24 {
        return group_rows(items)
            .iter()
            .map(|r| row_reading_order(r))
            .collect();
    }
    // Wide items alone in their band (section titles, footers) separate the content between them.
    let left = items.iter().map(|i| i.bx().x).fold(f64::INFINITY, f64::min);
    let right = items
        .iter()
        .map(|i| i.bx().x + i.bx().w)
        .fold(f64::NEG_INFINITY, f64::max);
    let wide = |it: &T| it.bx().w >= (right - left) * 0.6;
    let is_sep = |it: &T| {
        let b = it.bx();
        wide(it)
            && !items.iter().any(|o| {
                !std::ptr::eq(*o, it)
                    && !wide(o)
                    && overlap1d(o.bx().y, o.bx().y + o.bx().h, b.y, b.y + b.h) > 0.0
            })
    };
    let seps: Vec<&'a T> = items.iter().copied().filter(|it| is_sep(it)).collect();
    if !seps.is_empty() && seps.len() < items.len() {
        let mut out: Vec<Vec<&'a T>> = Vec::new();
        let mut seg: Vec<&'a T> = Vec::new();
        let mut sorted = items.to_vec();
        sorted.sort_by(|a, b| by(a.bx().y, b.bx().y).then(by(a.bx().x, b.bx().x)));
        for it in sorted {
            if seps.iter().any(|s| std::ptr::eq(*s, it)) {
                if !seg.is_empty() {
                    out.extend(reading_rows(&seg, min_gap, depth + 1));
                }
                seg.clear();
                out.push(vec![it]);
            } else {
                seg.push(it);
            }
        }
        if !seg.is_empty() {
            out.extend(reading_rows(&seg, min_gap, depth + 1));
        }
        return out;
    }
    let cols = cut_at(items, Axis::X, min_gap, false);
    let bands = cut_at(items, Axis::Y, min_gap, true);
    let use_cols = cols.len() > 1 && (bands.len() <= 1 || cols.iter().all(|c| c.len() >= 3));
    if use_cols {
        // Side-by-side single items (icons, logo + nav, image + caption block) stay one row.
        if cols.iter().all(|c| c.len() == 1) {
            return vec![cols.into_iter().map(|c| c[0]).collect()];
        }
        return cols
            .iter()
            .flat_map(|c| reading_rows(c, min_gap, depth + 1))
            .collect();
    }
    if bands.len() > 1 {
        return bands
            .iter()
            .flat_map(|b| reading_rows(b, min_gap, depth + 1))
            .collect();
    }
    group_rows(items)
        .iter()
        .map(|r| row_reading_order(r))
        .collect()
}

/// `planReflow(items, page)`.
pub fn plan_reflow<'a, T: ReflowItem>(
    items: &'a [T],
    page_w: f64,
    page_h: f64,
) -> ReflowPlan<'a, T> {
    let page = Rect {
        x: 0.0,
        y: 0.0,
        w: page_w,
        h: page_h,
    };
    let page_area = area(&page);
    let mut sorted: Vec<&'a T> = items.iter().collect();
    sorted.sort_by(|a, b| by(a.z(), b.z()));
    let mut background = Vec::new();
    let mut rest: Vec<&'a T> = Vec::new();
    for it in sorted {
        if it.visual() && intersection(&ReflowItem::bx(it), &page) >= page_area * 0.85 {
            background.push(it);
        } else {
            rest.push(it);
        }
    }
    // Each item's parent: the smallest container below it (lower z) that contains it.
    let mut parent: HashMap<&str, &'a T> = HashMap::new();
    for it in &rest {
        let mut best: Option<&'a T> = None;
        for c in &rest {
            if std::ptr::eq(*c, *it)
                || !c.container()
                || c.z() >= it.z()
                || !contains(&ReflowItem::bx(*c), &ReflowItem::bx(*it))
            {
                continue;
            }
            if best.is_none_or(|b| area(&ReflowItem::bx(*c)) < area(&ReflowItem::bx(b))) {
                best = Some(c);
            }
        }
        if let Some(b) = best {
            parent.insert(it.id(), b);
        }
    }
    let mut kids: HashMap<&str, Vec<&'a T>> = HashMap::new();
    let mut roots: Vec<&'a T> = Vec::new();
    for it in &rest {
        match parent.get(it.id()) {
            Some(p) => kids.entry(p.id()).or_default().push(it),
            None => roots.push(it),
        }
    }
    let kid_rows: HashMap<&str, Vec<Vec<&'a T>>> = kids
        .iter()
        .map(|(id, list)| (*id, reading_rows(list, 8.0, 0)))
        .collect();
    fn node<'a, T: ReflowItem>(
        it: &'a T,
        kid_rows: &HashMap<&str, Vec<Vec<&'a T>>>,
    ) -> ReflowNode<'a, T> {
        let rows = kid_rows.get(it.id()).map_or_else(Vec::new, |rows| {
            rows.iter()
                .map(|r| r.iter().map(|k| node(*k, kid_rows)).collect())
                .collect()
        });
        ReflowNode {
            item: it,
            bx: ReflowItem::bx(it),
            rows,
        }
    }
    let rows = reading_rows(&roots, 8.0, 0)
        .iter()
        .map(|r| r.iter().map(|it| node(*it, &kid_rows)).collect())
        .collect();
    ReflowPlan { background, rows }
}

/// Flatten a plan to reading order, depth first — the order screen readers and the mobile layout use.
pub fn reading_order<'a, T>(plan: &ReflowPlan<'a, T>) -> Vec<&'a T> {
    fn walk<'a, T>(rows: &[Vec<ReflowNode<'a, T>>], out: &mut Vec<&'a T>) {
        for row in rows {
            for n in row {
                out.push(n.item);
                walk(&n.rows, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&plan.rows, &mut out);
    out
}

/// Readable mobile font size (px) for a desktop size on a page `page_w` wide: never below 14 px, headings capped.
pub fn mobile_font_size(size: f64, page_w: f64) -> f64 {
    let f = 1f64.min(0.36f64.max((390.0 / 1f64.max(page_w)).sqrt()));
    oc_schema::js::round(44f64.min(14f64.max(size * f)) * 10.0) / 10.0
}

/// Desktop-width pages reflow by default; narrow or phone-shaped pages (link in bio, stories) keep their layout.
pub fn should_reflow(mode: Option<&str>, w: f64, h: f64) -> bool {
    if mode == Some("desktop") {
        return false;
    }
    let phone_shaped = w <= 1242.0 && h / w >= 1.5;
    w > 700.0 && !phone_shaped
}
