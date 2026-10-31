// SPDX-License-Identifier: Apache-2.0
//! World transforms as the TS computes them: like `oc_geometry::{world_matrix, world_bounds}`, except that an
//! element of a type this build does not know still contributes its stored `xf` (the TS reads `el.xf` whatever
//! the type; oc-geometry gives untyped elements no transform).

use std::collections::HashSet;

use oc_geometry::{IDENTITY, apply, bounds_of_points, local_matrix, multiply, rect_corners};
use oc_schema::elements::Element;
use oc_schema::primitives::{Mat2D, Rect, Xf};
use oc_schema::tree::Elements;

/// The element's `xf`, typed or read from an unknown element's JSON.
pub fn xf_of(el: &Element) -> Option<Xf> {
    match el {
        Element::Unknown(m) => m
            .get("xf")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
        _ => el.xf().cloned(),
    }
}

/// `worldMatrix(doc, id)`: element box space → page space (groups compose); identity for unknown ids.
pub fn world_matrix(elements: &Elements, id: &str) -> Mat2D {
    let mut chain: Vec<&Element> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut cur = elements.get(id);
    while let Some(el) = cur {
        let eid = el.id().unwrap_or("");
        if !seen.insert(eid) {
            break;
        }
        chain.push(el);
        cur = el.parent_id().and_then(|p| elements.get(p));
    }
    let mut m = IDENTITY;
    for el in chain.iter().rev() {
        if let Some(xf) = xf_of(el) {
            m = multiply(&m, &local_matrix(&xf));
        }
    }
    m
}

/// `worldBounds(doc, id)`: axis-aligned page-space bounds (zero box at the origin for unknown ids).
pub fn world_bounds(elements: &Elements, id: &str) -> Rect {
    let Some(el) = elements.get(id) else {
        return Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
    };
    let m = world_matrix(elements, id);
    let (w, h) = xf_of(el).map_or((0.0, 0.0), |xf| (xf.w, xf.h));
    let q = rect_corners(&Rect {
        x: 0.0,
        y: 0.0,
        w,
        h,
    });
    bounds_of_points(&q.map(|p| apply(&m, p)))
}
