// SPDX-License-Identifier: Apache-2.0
//! Pure transform math for resize/rotate gestures. All inputs/outputs in the element's parent space.
//! Port of `transform.ts`.

use indexmap::IndexMap;
use oc_geometry::normalize_angle;
use oc_schema::elements::Element;
use oc_schema::primitives::{StoredRect, Vec2, Xf};
use oc_schema::tree::{Elements, descendants};
use oc_schema::TextBody;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::util::{cos, jv, max, min, round, sin};

/// A resize handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Handle {
    N,
    S,
    E,
    W,
    Ne,
    Nw,
    Se,
    Sw,
}

impl Handle {
    pub fn as_str(self) -> &'static str {
        match self {
            Handle::N => "n",
            Handle::S => "s",
            Handle::E => "e",
            Handle::W => "w",
            Handle::Ne => "ne",
            Handle::Nw => "nw",
            Handle::Se => "se",
            Handle::Sw => "sw",
        }
    }

    /// `handle.includes(c)`.
    pub fn has(self, c: char) -> bool {
        self.as_str().contains(c)
    }

    pub fn is_corner(self) -> bool {
        self.as_str().len() == 2
    }
}

pub const CORNERS: [Handle; 4] = [Handle::Nw, Handle::Ne, Handle::Se, Handle::Sw];
pub const EDGES: [Handle; 4] = [Handle::N, Handle::E, Handle::S, Handle::W];

fn rad(d: f64) -> f64 {
    (d * std::f64::consts::PI) / 180.0
}

pub(crate) fn rot(v: Vec2, deg: f64) -> Vec2 {
    let c = cos(rad(deg));
    let s = sin(rad(deg));
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResizeResult {
    pub xf: Xf,
    pub sx: f64,
    pub sy: f64,
    pub anchor: Vec2,
}

/// Resize an (optionally rotated) box by dragging `handle` to point `p` (parent space).
/// The opposite edge/corner stays fixed. `keep_aspect` forces uniform scaling. TS default `min = 4`.
pub fn resize_box(xf: &Xf, handle: Handle, p: Vec2, keep_aspect: bool, min_size: f64) -> ResizeResult {
    let c0 = [xf.x + xf.w / 2.0, xf.y + xf.h / 2.0];
    let q = rot([p[0] - c0[0], p[1] - c0[1]], -xf.rot);
    let mut l = -xf.w / 2.0;
    let mut r = xf.w / 2.0;
    let mut t = -xf.h / 2.0;
    let mut b = xf.h / 2.0;
    if handle.has('e') {
        r = max(l + min_size, q[0]);
    }
    if handle.has('w') {
        l = min(r - min_size, q[0]);
    }
    if handle.has('s') {
        b = max(t + min_size, q[1]);
    }
    if handle.has('n') {
        t = min(b - min_size, q[1]);
    }
    let mut ww = r - l;
    let mut hh = b - t;
    if keep_aspect {
        let mut s = if handle.is_corner() {
            max(ww / xf.w, hh / xf.h)
        } else if handle == Handle::E || handle == Handle::W {
            ww / xf.w
        } else {
            hh / xf.h
        };
        s = max(s, min_size / min(xf.w, xf.h));
        ww = xf.w * s;
        hh = xf.h * s;
        l = if handle.has('w') {
            xf.w / 2.0 - ww
        } else if handle.has('e') {
            -xf.w / 2.0
        } else {
            -ww / 2.0
        };
        t = if handle.has('n') {
            xf.h / 2.0 - hh
        } else if handle.has('s') {
            -xf.h / 2.0
        } else {
            -hh / 2.0
        };
        r = l + ww;
        b = t + hh;
    }
    let c_local = [(l + r) / 2.0, (t + b) / 2.0];
    let d = rot(c_local, xf.rot);
    let c = [c0[0] + d[0], c0[1] + d[1]];
    let anchor_local = [
        if handle.has('w') {
            r
        } else if handle.has('e') {
            l
        } else {
            0.0
        },
        if handle.has('n') {
            b
        } else if handle.has('s') {
            t
        } else {
            0.0
        },
    ];
    let a = rot(anchor_local, xf.rot);
    ResizeResult {
        xf: Xf { x: c[0] - ww / 2.0, y: c[1] - hh / 2.0, w: ww, h: hh, ..xf.clone() },
        sx: ww / xf.w,
        sy: hh / xf.h,
        anchor: [c0[0] + a[0], c0[1] + a[1]],
    }
}

/// Scale a box uniformly about an anchor point (multi-selection resize).
pub fn scale_about(xf: &Xf, anchor: Vec2, s: f64) -> Xf {
    let cx = xf.x + xf.w / 2.0;
    let cy = xf.y + xf.h / 2.0;
    let nx = anchor[0] + (cx - anchor[0]) * s;
    let ny = anchor[1] + (cy - anchor[1]) * s;
    let w = xf.w * s;
    let h = xf.h * s;
    Xf { x: nx - w / 2.0, y: ny - h / 2.0, w, h, ..xf.clone() }
}

/// Rotate a box's centre about a pivot and add to its rotation.
pub fn rotate_about(xf: &Xf, pivot: Vec2, delta: f64) -> Xf {
    let cx = xf.x + xf.w / 2.0;
    let cy = xf.y + xf.h / 2.0;
    let [dx, dy] = rot([cx - pivot[0], cy - pivot[1]], delta);
    Xf { x: pivot[0] + dx - xf.w / 2.0, y: pivot[1] + dy - xf.h / 2.0, rot: normalize_angle(xf.rot + delta), ..xf.clone() }
}

pub fn scale_body(body: &TextBody, s: f64) -> TextBody {
    let r = |v: f64| max(1.0, round(v * s * 100.0) / 100.0);
    let mut out = body.clone();
    // `{ defaultRun, paragraphs }`: other body keys are not carried over.
    out.extra.clear();
    out.default_run.size = r(body.default_run.size);
    for p in &mut out.paragraphs {
        p.space_after *= s;
        for run in &mut p.runs {
            if let Some(size) = run.style.size
                && size != 0.0
                && !size.is_nan()
            {
                run.style.size = Some(r(size));
            }
        }
    }
    out
}

/// Patches keyed by element id, in insertion order (a JS `Map`).
pub type Patches = IndexMap<String, Map<String, Value>>;

/// Element-specific consequences of a box scale (sx, sy) on an element whose xf was already updated:
/// text font sizes (uniform only), line points, and the same for every descendant of a group.
/// Returns patches keyed by element id (the root's patch excludes xf).
pub fn scale_patches(elements: &Elements, el: &Element, sx: f64, sy: f64) -> Patches {
    let mut out = Patches::new();
    let mut own = |e: &Element, scale_xf: bool| {
        let mut patch = Map::new();
        if scale_xf && let Some(xf) = e.xf() {
            patch.insert("xf".into(), jv(&Xf { x: xf.x * sx, y: xf.y * sy, w: xf.w * sx, h: xf.h * sy, ..xf.clone() }));
        }
        match e {
            Element::Text(t) if (sx - sy).abs() < 1e-6 && (sx - 1.0).abs() > 1e-6 => {
                patch.insert("body".into(), jv(&scale_body(&t.body, sx)));
            }
            Element::Line(l) => {
                let pts: Vec<Vec2> = l.points.iter().map(|[x, y]| [x * sx, y * sy]).collect();
                patch.insert("points".into(), jv(&pts));
            }
            _ => {}
        }
        if !patch.is_empty()
            && let Some(id) = e.id()
        {
            out.insert(id.to_string(), patch);
        }
    };
    own(el, false);
    if let Element::Group(g) = el {
        for d in descendants(elements, &g.base.id) {
            own(d, true);
        }
    }
    out
}

/// Image edge-drag crops instead of stretching: keep the image scale, reveal more or less of the source.
pub fn crop_resize(xf: &Xf, crop: &StoredRect, next: &Xf, handle: Handle) -> (Xf, StoredRect) {
    if handle == Handle::E || handle == Handle::W {
        let px_per_crop = xf.w / crop.w;
        let max_cw = if handle == Handle::E { 1.0 - crop.x } else { crop.x + crop.w };
        let cw = max(4.0 / px_per_crop, min(next.w / px_per_crop, max_cw));
        let cx = if handle == Handle::W { crop.x + crop.w - cw } else { crop.x };
        let w = cw * px_per_crop;
        let [dx, dy] = rot([if handle == Handle::W { xf.w - w } else { 0.0 }, 0.0], xf.rot);
        let [ax, ay] = adjust_center(xf, w, xf.h, dx, dy);
        return (Xf { w, x: xf.x + ax, y: xf.y + ay, ..xf.clone() }, StoredRect { x: cx, w: cw, ..crop.clone() });
    }
    let px_per_crop = xf.h / crop.h;
    let max_ch = if handle == Handle::S { 1.0 - crop.y } else { crop.y + crop.h };
    let ch = max(4.0 / px_per_crop, min(next.h / px_per_crop, max_ch));
    let cy = if handle == Handle::N { crop.y + crop.h - ch } else { crop.y };
    let h = ch * px_per_crop;
    let [dx, dy] = rot([0.0, if handle == Handle::N { xf.h - h } else { 0.0 }], xf.rot);
    let [ax, ay] = adjust_center(xf, xf.w, h, dx, dy);
    (Xf { h, x: xf.x + ax, y: xf.y + ay, ..xf.clone() }, StoredRect { y: cy, h: ch, ..crop.clone() })
}

/// Crop for a photo shown in a box of another aspect without stretching (photo-filled shapes): `from` is the shape
/// the crop was made for (e.g. the crop in source pixels), `to` the box `(w, h)`. The visible part grows along the
/// axis that grew relatively more and zooms in once it reaches the edge of the source. The same aspect (within 1e-6)
/// keeps the crop as is. Centre kept, clamped inside the source. oc-render `refit_crop_for` is a copy (keep them
/// identical). Port of `refitCrop`.
pub fn refit_crop(crop: &StoredRect, from: (f64, f64), to: (f64, f64)) -> StoredRect {
    let f = (to.0 / to.1) / (from.0 / from.1);
    if !f.is_finite() || f <= 0.0 || (f - 1.0).abs() < 1e-6 {
        return crop.clone();
    }
    let mut w = if f > 1.0 { crop.w * f } else { crop.w };
    let mut h = if f > 1.0 { crop.h } else { crop.h / f };
    let m = max(max(w, h), 1.0);
    w /= m;
    h /= m;
    let cx = crop.x + crop.w / 2.0;
    let cy = crop.y + crop.h / 2.0;
    StoredRect::from(oc_schema::primitives::Rect::new(min(1.0 - w, max(0.0, cx - w / 2.0)), min(1.0 - h, max(0.0, cy - h / 2.0)), w, h))
}

/// When a rotated box shrinks from its top-left (local) side by a shift already rotated into parent space as
/// (dx, dy), the change to the unrotated top-left so the opposite edge stays put.
fn adjust_center(xf: &Xf, w: f64, h: f64, dx: f64, dy: f64) -> Vec2 {
    let old_c = [xf.x + xf.w / 2.0, xf.y + xf.h / 2.0];
    let tl_old = rot([-xf.w / 2.0, -xf.h / 2.0], xf.rot);
    let tl_new = [old_c[0] + tl_old[0] + dx, old_c[1] + tl_old[1] + dy];
    let half = rot([w / 2.0, h / 2.0], xf.rot);
    let c_new = [tl_new[0] + half[0], tl_new[1] + half[1]];
    [c_new[0] - w / 2.0 - xf.x, c_new[1] - h / 2.0 - xf.y]
}
