// SPDX-License-Identifier: Apache-2.0
//! Smart guides: snap a moving box's edges/centre to page and sibling edges/centres. docs/plan/03 §7.3.

use std::cmp::Ordering;

use oc_schema::primitives::Rect;
use serde::{Deserialize, Serialize};

use crate::jsmath::{max, min, round};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuideKind {
    Page,
    Element,
    Ruler,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub axis: Axis,
    /// Page-space coordinate of the guide line.
    pub pos: f64,
    /// Extent of the guide along the other axis.
    pub from: f64,
    pub to: f64,
    pub kind: GuideKind,
}

/// Equal-spacing marker (ED-060): a measured gap drawn between two boxes. `axis` is the measuring direction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpacingMark {
    pub axis: Axis,
    pub from: f64,
    pub to: f64,
    pub at: f64,
    pub gap: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SnapResult {
    pub dx: f64,
    pub dy: f64,
    pub guides: Vec<Guide>,
    pub spacing: Vec<SpacingMark>,
}

/// A custom ruler guide (page px).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RulerGuide {
    pub axis: Axis,
    pub pos: f64,
}

#[derive(Debug, Clone, Copy)]
struct Line {
    pos: f64,
    from: f64,
    to: f64,
    kind: GuideKind,
}

fn lines_of(r: &Rect, axis: Axis, kind: GuideKind) -> [Line; 3] {
    match axis {
        Axis::X => [r.x, r.x + r.w / 2.0, r.x + r.w].map(|pos| Line { pos, from: r.y, to: r.y + r.h, kind }),
        Axis::Y => [r.y, r.y + r.h / 2.0, r.y + r.h].map(|pos| Line { pos, from: r.x, to: r.x + r.w, kind }),
    }
}

fn shifted(r: &Rect, axis: Axis, delta: f64) -> Rect {
    match axis {
        Axis::X => Rect { x: r.x + delta, ..*r },
        Axis::Y => Rect { y: r.y + delta, ..*r },
    }
}

/// `Array#sort` comparator semantics for `a - b` (NaN compares equal).
pub(crate) fn cmp_diff(d: f64) -> Ordering {
    if d < 0.0 {
        Ordering::Less
    } else if d > 0.0 {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// * `moving` — the moving selection's bounds (already offset by the raw drag delta)
/// * `page` — page rect (0,0,w,h)
/// * `targets` — bounds of the other elements on the page
/// * `threshold` — snap distance in page px (callers pass screenPx / zoom)
pub fn snap_move(moving: &Rect, page: &Rect, targets: &[Rect], threshold: f64, rulers: &[RulerGuide]) -> SnapResult {
    let mut res = SnapResult::default();
    for axis in [Axis::X, Axis::Y] {
        let mine = lines_of(moving, axis, GuideKind::Element);
        let mut cands: Vec<Line> = lines_of(page, axis, GuideKind::Page).to_vec();
        cands.extend(targets.iter().flat_map(|t| lines_of(t, axis, GuideKind::Element)));
        cands.extend(rulers.iter().filter(|r| r.axis == axis).map(|r| match axis {
            Axis::X => Line { pos: r.pos, from: page.y, to: page.y + page.h, kind: GuideKind::Ruler },
            Axis::Y => Line { pos: r.pos, from: page.x, to: page.x + page.w, kind: GuideKind::Ruler },
        }));
        let mut best: Option<f64> = None;
        for m in &mine {
            for c in &cands {
                let d = c.pos - m.pos;
                if d.abs() <= threshold && best.is_none_or(|b| d.abs() < b.abs()) {
                    best = Some(d);
                }
            }
        }
        let Some(delta) = best else {
            if let Some((d, marks)) = snap_spacing(moving, targets, axis, threshold) {
                match axis {
                    Axis::X => res.dx = d,
                    Axis::Y => res.dy = d,
                }
                res.spacing.extend(marks);
            }
            continue;
        };
        match axis {
            Axis::X => res.dx = delta,
            Axis::Y => res.dy = delta,
        }
        // Collect every line that coincides after snapping, for the guide overlay.
        let snapped = lines_of(&shifted(moving, axis, delta), axis, GuideKind::Element);
        for m in &snapped {
            for c in &cands {
                if (c.pos - m.pos).abs() < 0.5 {
                    res.guides.push(Guide { axis, pos: c.pos, from: min(m.from, c.from), to: max(m.to, c.to), kind: c.kind });
                }
            }
        }
    }
    res
}

/// Result of [`snap_value`]: the snapped value and the candidate it snapped to (if any).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapValue {
    pub value: f64,
    pub snapped: Option<f64>,
}

/// Snap a single value (used for resize edges).
pub fn snap_value(v: f64, candidates: &[f64], threshold: f64) -> SnapValue {
    let mut best: Option<f64> = None;
    for &c in candidates {
        if (c - v).abs() <= threshold && best.is_none_or(|b| (c - v).abs() < (b - v).abs()) {
            best = Some(c);
        }
    }
    SnapValue { value: best.unwrap_or(v), snapped: best }
}

/// Rotation snapping: magnetic to multiples of 45° within ±3°, or hard 15° steps with Shift.
pub fn snap_angle(deg: f64, shift: bool) -> f64 {
    if shift {
        return round(deg / 15.0) * 15.0;
    }
    let nearest = round(deg / 45.0) * 45.0;
    if (nearest - deg).abs() <= 3.0 { nearest } else { deg }
}

/// Equal-spacing snap along one axis: centre the box between its two neighbours, or match a gap that already
/// exists between other elements in the same row/column. Returns the delta and the marks to draw.
pub fn snap_spacing(moving: &Rect, targets: &[Rect], axis: Axis, threshold: f64) -> Option<(f64, Vec<SpacingMark>)> {
    let x = axis == Axis::X;
    let lo = |r: &Rect| if x { r.x } else { r.y };
    let hi = |r: &Rect| if x { r.x + r.w } else { r.y + r.h };
    let olo = |r: &Rect| if x { r.y } else { r.x };
    let ohi = |r: &Rect| if x { r.y + r.h } else { r.x + r.w };
    // Same row (for x) / column (for y): overlapping on the other axis.
    let mut row: Vec<&Rect> = targets.iter().filter(|t| ohi(t) > olo(moving) && olo(t) < ohi(moving)).collect();
    row.sort_by(|a, b| cmp_diff(lo(a) - lo(b)));
    if row.is_empty() {
        return None;
    }
    let before = row.iter().copied().rfind(|t| hi(t) <= lo(moving) + threshold);
    let after = row.iter().copied().find(|t| lo(t) >= hi(moving) - threshold);
    let mid = |a: &Rect, b: &Rect| (max(olo(a), olo(b)) + min(ohi(a), ohi(b))) / 2.0;
    let mark = |a: f64, b: f64, other: &Rect, me: &Rect| SpacingMark { axis, from: a, to: b, at: mid(other, me), gap: round(b - a) };

    // 1. Centred between the two neighbours.
    if let (Some(bf), Some(af)) = (before, after) {
        let gl = lo(moving) - hi(bf);
        let gr = lo(af) - hi(moving);
        let delta = (gr - gl) / 2.0;
        if delta.abs() <= threshold && gl + delta > 0.0 {
            let m = shifted(moving, axis, delta);
            return Some((delta, vec![mark(hi(bf), lo(&m), bf, &m), mark(hi(&m), lo(af), af, &m)]));
        }
    }
    // 2. Match an existing gap between consecutive neighbours in the row.
    let mut gaps: Vec<(f64, &Rect, &Rect)> = Vec::new();
    for i in 1..row.len() {
        let g = lo(row[i]) - hi(row[i - 1]);
        if g > 0.0 {
            gaps.push((g, row[i - 1], row[i]));
        }
    }
    // (delta, gap index, side is before)
    let mut best: Option<(f64, usize, bool)> = None;
    for (gi, &(g, _, _)) in gaps.iter().enumerate() {
        if let Some(bf) = before {
            let d = hi(bf) + g - lo(moving);
            if d.abs() <= threshold && best.is_none_or(|b| d.abs() < b.0.abs()) {
                best = Some((d, gi, true));
            }
        }
        if let Some(af) = after {
            let d = lo(af) - g - hi(moving);
            if d.abs() <= threshold && best.is_none_or(|b| d.abs() < b.0.abs()) {
                best = Some((d, gi, false));
            }
        }
    }
    let (delta, gi, side_before) = best?;
    let (_, ra, rb) = gaps[gi];
    let m = shifted(moving, axis, delta);
    let mut marks = vec![mark(hi(ra), lo(rb), ra, rb)];
    if side_before {
        let bf = before.expect("before side");
        marks.push(mark(hi(bf), lo(&m), bf, &m));
    } else {
        let af = after.expect("after side");
        marks.push(mark(hi(&m), lo(af), af, &m));
    }
    Some((delta, marks))
}
