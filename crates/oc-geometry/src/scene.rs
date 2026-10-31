// SPDX-License-Identifier: Apache-2.0
//! Element-level geometry over a design snapshot: world matrices, bounds, hit-testing.
//!
//! The TS functions take `Pick<DesignDoc, 'elements' | 'pages'>`; only `elements` is ever read, so these take the
//! element map (`&doc.elements`).

use std::collections::HashSet;

use oc_schema::elements::Element;
use oc_schema::primitives::{Mat2D, Rect, Vec2};
use oc_schema::tree::{Elements, children_of};

use crate::jsmath::{hypot, max, min};
use crate::matrix::{IDENTITY, apply, bounds_of_points, invert, local_matrix, multiply, rect_corners, rects_intersect};
use crate::shapes::hit_shape;

/// Matrix from the element's box space to page space (groups compose). Identity for unknown ids.
pub fn world_matrix(elements: &Elements, id: &str) -> Mat2D {
    let mut chain: Vec<&Element> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut cur = elements.get(id);
    while let Some(el) = cur {
        let eid = el.id().unwrap_or("");
        if seen.contains(eid) {
            break;
        }
        chain.push(el);
        seen.insert(eid);
        cur = el.parent_id().and_then(|p| elements.get(p));
    }
    let mut m = IDENTITY;
    for el in chain.iter().rev() {
        // Unknown element types carry no typed `xf`; they contribute no transform.
        if let Some(xf) = el.xf() {
            m = multiply(&m, &local_matrix(xf));
        }
    }
    m
}

/// Matrix from page space into the element's parent space (identity for top-level elements).
pub fn parent_inverse(elements: &Elements, el: &Element) -> Mat2D {
    match el.parent_id().and_then(|p| elements.get(p)) {
        Some(parent) => invert(&world_matrix(elements, parent.id().unwrap_or(""))),
        None => IDENTITY,
    }
}

fn box_size(el: &Element) -> (f64, f64) {
    el.xf().map_or((0.0, 0.0), |xf| (xf.w, xf.h))
}

/// Corners of the element box in page space (rotated quad); `None` for unknown ids.
pub fn world_quad(elements: &Elements, id: &str) -> Option<[Vec2; 4]> {
    let el = elements.get(id)?;
    let m = world_matrix(elements, id);
    let (w, h) = box_size(el);
    let [a, b, c, d] = rect_corners(&Rect::new(0.0, 0.0, w, h));
    Some([apply(&m, a), apply(&m, b), apply(&m, c), apply(&m, d)])
}

/// Axis-aligned page-space bounds of the element; `None` for unknown ids.
pub fn world_bounds(elements: &Elements, id: &str) -> Option<Rect> {
    world_quad(elements, id).map(|q| bounds_of_points(&q))
}

/// Union of the world quads of the existing ids, or `None` when none exists.
pub fn selection_bounds<S: AsRef<str>>(elements: &Elements, ids: &[S]) -> Option<Rect> {
    let quads: Vec<Vec2> = ids.iter().filter_map(|id| world_quad(elements, id.as_ref())).flatten().collect();
    if quads.is_empty() { None } else { Some(bounds_of_points(&quads)) }
}

fn dist_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { max(0.0, min(1.0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2)) };
    hypot(p[0] - (a[0] + t * dx), p[1] - (a[1] + t * dy))
}

/// Does page-space point p hit the element (shape-accurate for shapes; box for everything else)?
pub fn hit_element(elements: &Elements, el: &Element, p: Vec2, tolerance: f64) -> bool {
    let inv = invert(&world_matrix(elements, el.id().unwrap_or("")));
    let [x, y] = apply(&inv, p);
    let (w, h) = box_size(el);
    let t = tolerance;
    if let Element::Line(line) = el {
        // Every segment counts (elbow and curved connectors are polylines).
        let reach = max(line.stroke.width / 2.0 + t, 6.0);
        return line.points.windows(2).any(|s| dist_to_segment([x, y], s[0], s[1]) <= reach);
    }
    if x < -t || y < -t || x > w + t || y > h + t {
        return false;
    }
    if let Element::Shape(s) = el
        && t == 0.0
    {
        return hit_shape(s.kind.as_str(), w, h, x, y, &s.params);
    }
    true
}

/// Options for [`hit_test`].
#[derive(Debug, Clone, Copy, Default)]
pub struct HitTestOpts<'a> {
    pub tolerance: f64,
    /// A group whose children are tested (clicking inside any other group selects the group).
    pub enter_group: Option<&'a str>,
}

fn is_hidden(el: &Element) -> bool {
    match el {
        Element::Unknown(m) => m.get("visible") == Some(&serde_json::Value::Bool(false)),
        _ => el.base().is_some_and(|b| b.visible == Some(false)),
    }
}

/// Topmost selectable element under p on a page. Clicking inside a group selects the group
/// (unless `enter_group` names that group, in which case its children are tested).
pub fn hit_test<'a>(elements: &'a Elements, page_id: &str, p: Vec2, opts: HitTestOpts<'_>) -> Option<&'a Element> {
    fn test<'a>(elements: &'a Elements, parent_id: &str, p: Vec2, opts: &HitTestOpts<'_>) -> Option<&'a Element> {
        let kids = children_of(elements, parent_id);
        for el in kids.into_iter().rev() {
            if is_hidden(el) {
                continue;
            }
            if let Element::Group(_) = el {
                let id = el.id().unwrap_or("");
                if let Some(inner) = test(elements, id, p, opts) {
                    return Some(if opts.enter_group == Some(id) { inner } else { el });
                }
                continue;
            }
            if hit_element(elements, el, p, opts.tolerance) {
                return Some(el);
            }
        }
        None
    }
    test(elements, page_id, p, &opts)
}

/// Top-level elements of a page whose world bounds intersect (or are contained by) the marquee.
pub fn marquee_select<'a>(elements: &'a Elements, page_id: &str, r: &Rect) -> Vec<&'a Element> {
    children_of(elements, page_id)
        .into_iter()
        .filter(|el| {
            let locked = el.base().and_then(|b| b.lock.as_ref()).is_some_and(|l| l.move_ == Some(true));
            !is_hidden(el)
                && !locked
                && world_bounds(elements, el.id().unwrap_or("")).is_some_and(|b| rects_intersect(&b, r))
        })
        .collect()
}
