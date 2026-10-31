// SPDX-License-Identifier: Apache-2.0
//! Connectors (WB-010): line ends glued to element anchors. Endpoints are derived from the attached elements, so
//! moving a shape drags its connectors along — in the renderer (live, with drag overrides) and when the editor
//! commits a move (persisted points/xf).

use std::collections::HashSet;

use oc_schema::elements::{AnchorSide, Element, LineEl, LineRoute};
use oc_schema::primitives::{Vec2, Xf};
use oc_schema::tree::Elements;
use serde::{Deserialize, Serialize};

use crate::jsmath::{hypot, max, sign_or, truthy};
use crate::matrix::{apply, bounds_of_points, invert};
use crate::scene::world_matrix;

pub const ANCHORS: [AnchorSide; 5] = [AnchorSide::Top, AnchorSide::Right, AnchorSide::Bottom, AnchorSide::Left, AnchorSide::Center];

/// Anchor position in box space. Unknown anchor values (from newer clients) resolve like the TS lookup would
/// fail; they are treated as the centre here.
fn local(anchor: &AnchorSide, w: f64, h: f64) -> Vec2 {
    match anchor {
        AnchorSide::Top => [w / 2.0, 0.0],
        AnchorSide::Right => [w, h / 2.0],
        AnchorSide::Bottom => [w / 2.0, h],
        AnchorSide::Left => [0.0, h / 2.0],
        AnchorSide::Center | AnchorSide::Other(_) => [w / 2.0, h / 2.0],
    }
}

fn size_of(el: &Element) -> (f64, f64) {
    el.xf().map_or((0.0, 0.0), |xf| (xf.w, xf.h))
}

/// Page-space point of an element anchor (follows rotation); `None` for missing or deleted elements.
pub fn anchor_point(elements: &Elements, id: &str, anchor: &AnchorSide) -> Option<Vec2> {
    let el = elements.get(id)?;
    if el.is_deleted() {
        return None;
    }
    let (w, h) = size_of(el);
    Some(apply(&world_matrix(elements, id), local(anchor, w, h)))
}

/// Outward direction of an anchor in page space (unit vector; zero for centre).
fn anchor_normal(elements: &Elements, id: &str, anchor: &AnchorSide) -> Vec2 {
    if *anchor == AnchorSide::Center {
        return [0.0, 0.0];
    }
    let (w, h) = elements.get(id).map_or((0.0, 0.0), size_of);
    let m = world_matrix(elements, id);
    let c = apply(&m, [w / 2.0, h / 2.0]);
    let p = apply(&m, local(anchor, w, h));
    let d = hypot(p[0] - c[0], p[1] - c[1]);
    let d = if truthy(d) { d } else { 1.0 };
    [(p[0] - c[0]) / d, (p[1] - c[1]) / d]
}

/// Result of [`nearest_anchor`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NearestAnchor {
    pub anchor: AnchorSide,
    pub point: Vec2,
}

/// Nearest anchor of an element to a page-space point, within `radius`.
pub fn nearest_anchor(elements: &Elements, id: &str, p: Vec2, radius: f64) -> Option<NearestAnchor> {
    let mut best: Option<(NearestAnchor, f64)> = None;
    for a in &ANCHORS {
        let Some(q) = anchor_point(elements, id, a) else { continue };
        let d = hypot(q[0] - p[0], q[1] - p[1]);
        if d <= radius && best.as_ref().is_none_or(|(_, bd)| d < *bd) {
            best = Some((NearestAnchor { anchor: a.clone(), point: q }, d));
        }
    }
    best.map(|(b, _)| b)
}

fn cubic(a: Vec2, c1: Vec2, c2: Vec2, b: Vec2, n: u32) -> Vec<Vec2> {
    (0..=n)
        .map(|i| {
            let t = f64::from(i) / f64::from(n);
            let u = 1.0 - t;
            [
                u * u * u * a[0] + 3.0 * u * u * t * c1[0] + 3.0 * u * t * t * c2[0] + t * t * t * b[0],
                u * u * u * a[1] + 3.0 * u * u * t * c1[1] + 3.0 * u * t * t * c2[1] + t * t * t * b[1],
            ]
        })
        .collect()
}

/// Route between two page-space points given the outward normals at each end. `None` (and unknown routes) is
/// `'straight'`.
pub fn route_connector(a: Vec2, na: Vec2, b: Vec2, nb: Vec2, route: Option<&LineRoute>) -> Vec<Vec2> {
    match route {
        Some(LineRoute::Curved) => {
            let k = max(40.0, hypot(b[0] - a[0], b[1] - a[1]) * 0.4);
            let fa = if truthy(na[0]) || truthy(na[1]) { na } else { [sign_or(b[0] - a[0], 1.0), 0.0] };
            let fb = if truthy(nb[0]) || truthy(nb[1]) { nb } else { [sign_or(a[0] - b[0], -1.0), 0.0] };
            cubic(a, [a[0] + fa[0] * k, a[1] + fa[1] * k], [b[0] + fb[0] * k, b[1] + fb[1] * k], b, 24)
        }
        Some(LineRoute::Elbow) => {
            let horiz_a = if na[0] != 0.0 { na[0].abs() >= na[1].abs() } else { (b[0] - a[0]).abs() >= (b[1] - a[1]).abs() };
            let horiz_b = if nb[0] != 0.0 || nb[1] != 0.0 { nb[0].abs() >= nb[1].abs() } else { horiz_a };
            if horiz_a && horiz_b {
                let mx = (a[0] + b[0]) / 2.0;
                return vec![a, [mx, a[1]], [mx, b[1]], b];
            }
            if !horiz_a && !horiz_b {
                let my = (a[1] + b[1]) / 2.0;
                return vec![a, [a[0], my], [b[0], my], b];
            }
            if horiz_a { vec![a, [b[0], a[1]], b] } else { vec![a, [a[0], b[1]], b] }
        }
        _ => vec![a, b],
    }
}

/// Current page-space polyline of a line (attachments resolved), or `None` when it has none.
pub fn connector_page_points(elements: &Elements, line: &LineEl) -> Option<Vec<Vec2>> {
    if line.start_attach.is_none() && line.end_attach.is_none() {
        return None;
    }
    let m = world_matrix(elements, &line.base.id);
    let first = line.points.first().copied().unwrap_or_default();
    let last = line.points.last().copied().unwrap_or_default();
    let own_a = apply(&m, first);
    let own_b = apply(&m, last);
    let a = line.start_attach.as_ref().and_then(|s| anchor_point(elements, &s.id, &s.anchor)).unwrap_or(own_a);
    let b = line.end_attach.as_ref().and_then(|s| anchor_point(elements, &s.id, &s.anchor)).unwrap_or(own_b);
    let normal = |att: &Option<oc_schema::elements::LineAttach>| match att {
        Some(s) if elements.contains_key(&s.id) => anchor_normal(elements, &s.id, &s.anchor),
        _ => [0.0, 0.0],
    };
    let na = normal(&line.start_attach);
    let nb = normal(&line.end_attach);
    Some(route_connector(a, na, b, nb, line.route.as_ref()))
}

/// Connector polyline in the line's own box space (for the renderer).
pub fn connector_local_points(elements: &Elements, line: &LineEl) -> Option<Vec<Vec2>> {
    let pts = connector_page_points(elements, line)?;
    let inv = invert(&world_matrix(elements, &line.base.id));
    Some(pts.into_iter().map(|p| apply(&inv, p)).collect())
}

/// New xf + points for a connector so its box tightly wraps the routed path (persisted on commit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorGeometry {
    pub xf: Xf,
    pub points: Vec<Vec2>,
}

/// New xf + points for a top-level connector so its box tightly wraps the routed path (persisted on commit).
/// Rotation is reset; connectors are defined by their ends.
pub fn connector_geometry(elements: &Elements, line: &LineEl) -> Option<ConnectorGeometry> {
    let pts = connector_page_points(elements, line)?;
    let local: Vec<Vec2> = if elements.contains_key(&line.base.parent_id) {
        let parent_inv = invert(&world_matrix(elements, &line.base.parent_id));
        pts.into_iter().map(|p| apply(&parent_inv, p)).collect()
    } else {
        pts
    };
    let b = bounds_of_points(&local);
    let w = max(1.0, b.w);
    let h = max(1.0, b.h);
    Some(ConnectorGeometry {
        xf: Xf::new(b.x, b.y, w, h, 0.0),
        points: local.into_iter().map(|[x, y]| [x - b.x, y - b.y]).collect(),
    })
}

/// Lines attached to any of the given elements (map order).
pub fn connectors_of<'a>(elements: &'a Elements, ids: &HashSet<String>) -> Vec<&'a LineEl> {
    elements
        .values()
        .filter_map(|e| match e {
            Element::Line(l) if l.base.deleted.is_none() => {
                let hit = l.start_attach.as_ref().is_some_and(|s| ids.contains(&s.id))
                    || l.end_attach.as_ref().is_some_and(|s| ids.contains(&s.id));
                hit.then_some(l)
            }
            _ => None,
        })
        .collect()
}
