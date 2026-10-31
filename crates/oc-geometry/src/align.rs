// SPDX-License-Identifier: Apache-2.0
//! Align and distribute. Pure: returns per-id translation deltas in page space.

use indexmap::IndexMap;
use oc_schema::primitives::{Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::snap::{Axis, cmp_diff};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlignEdge {
    Left,
    Hcenter,
    Right,
    Top,
    Vcenter,
    Bottom,
}

/// An item to align: its id and page-space bounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlignItem {
    pub id: String,
    pub bounds: Rect,
}

/// Per-id deltas in insertion order (like the TS `Map`; a repeated id keeps its first position, last value).
pub type Deltas = IndexMap<String, Vec2>;

pub fn align_deltas(items: &[AlignItem], edge: AlignEdge, to: &Rect) -> Deltas {
    let mut out = Deltas::new();
    for AlignItem { id, bounds: b } in items {
        let (mut dx, mut dy) = (0.0, 0.0);
        match edge {
            AlignEdge::Left => dx = to.x - b.x,
            AlignEdge::Hcenter => dx = to.x + to.w / 2.0 - (b.x + b.w / 2.0),
            AlignEdge::Right => dx = to.x + to.w - (b.x + b.w),
            AlignEdge::Top => dy = to.y - b.y,
            AlignEdge::Vcenter => dy = to.y + to.h / 2.0 - (b.y + b.h / 2.0),
            AlignEdge::Bottom => dy = to.y + to.h - (b.y + b.h),
        }
        out.insert(id.clone(), [dx, dy]);
    }
    out
}

/// Equal gaps between items along an axis; first and last stay put.
pub fn distribute_deltas(items: &[AlignItem], axis: Axis) -> Deltas {
    let mut out = Deltas::new();
    if items.len() < 3 {
        return out;
    }
    let pos = |r: &Rect| if axis == Axis::X { r.x } else { r.y };
    let size = |r: &Rect| if axis == Axis::X { r.w } else { r.h };
    let mut sorted: Vec<&AlignItem> = items.iter().collect();
    sorted.sort_by(|a, b| cmp_diff(pos(&a.bounds) - pos(&b.bounds)));
    let first = &sorted[0].bounds;
    let last = &sorted[sorted.len() - 1].bounds;
    let span = pos(last) + size(last) - pos(first);
    let total = sorted.iter().fold(0.0, |s, i| s + size(&i.bounds));
    let gap = (span - total) / (sorted.len() - 1) as f64;
    let mut cursor = pos(first);
    for it in sorted {
        let d = cursor - pos(&it.bounds);
        out.insert(it.id.clone(), if axis == Axis::X { [d, 0.0] } else { [0.0, d] });
        cursor += size(&it.bounds) + gap;
    }
    out
}
