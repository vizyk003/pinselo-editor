// SPDX-License-Identifier: Apache-2.0
//! 2D affine matrices `[a, b, c, d, e, f]` (x' = a x + c y + e; y' = b x + d y + f) and rect helpers.

use std::f64::consts::PI;

use oc_schema::primitives::{Mat2D, Rect, Vec2, Xf};

use crate::jsmath::{atan2, cos, hypot, round, sin};

pub const IDENTITY: Mat2D = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

pub fn multiply(m: &Mat2D, n: &Mat2D) -> Mat2D {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

pub fn translate(tx: f64, ty: f64) -> Mat2D {
    [1.0, 0.0, 0.0, 1.0, tx, ty]
}

/// `scale(sx, sy = sx)`; pass the same value twice for a uniform scale.
pub fn scale(sx: f64, sy: f64) -> Mat2D {
    [sx, 0.0, 0.0, sy, 0.0, 0.0]
}

pub fn rotate(deg: f64) -> Mat2D {
    let r = (deg * PI) / 180.0;
    let c = cos(r);
    let s = sin(r);
    [c, s, -s, c, 0.0, 0.0]
}

/// Inverse, or the identity for (near-)singular matrices.
pub fn invert(m: &Mat2D) -> Mat2D {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 {
        return IDENTITY;
    }
    let id = 1.0 / det;
    [
        m[3] * id,
        -m[1] * id,
        -m[2] * id,
        m[0] * id,
        (m[2] * m[5] - m[3] * m[4]) * id,
        (m[1] * m[4] - m[0] * m[5]) * id,
    ]
}

pub fn apply(m: &Mat2D, p: Vec2) -> Vec2 {
    [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]
}

pub fn apply_vector(m: &Mat2D, v: Vec2) -> Vec2 {
    [m[0] * v[0] + m[2] * v[1], m[1] * v[0] + m[3] * v[1]]
}

/// Matrix mapping an element's own box space (0..w, 0..h) to its parent's space.
/// Rotation and flips are about the box centre.
pub fn local_matrix(xf: &Xf) -> Mat2D {
    let cx = xf.x + xf.w / 2.0;
    let cy = xf.y + xf.h / 2.0;
    let mut m = translate(cx, cy);
    if xf.rot != 0.0 && !xf.rot.is_nan() {
        m = multiply(&m, &rotate(xf.rot));
    }
    let fx = xf.flip_x == Some(true);
    let fy = xf.flip_y == Some(true);
    if fx || fy {
        m = multiply(&m, &scale(if fx { -1.0 } else { 1.0 }, if fy { -1.0 } else { 1.0 }));
    }
    multiply(&m, &translate(-xf.w / 2.0, -xf.h / 2.0))
}

/// Normalises degrees into (-180, 180], rounded to 1e-6.
pub fn normalize_angle(deg: f64) -> f64 {
    let mut a = deg % 360.0;
    if a <= -180.0 {
        a += 360.0;
    }
    if a > 180.0 {
        a -= 360.0;
    }
    round(a * 1e6) / 1e6
}

pub fn rect_corners(r: &Rect) -> [Vec2; 4] {
    [[r.x, r.y], [r.x + r.w, r.y], [r.x + r.w, r.y + r.h], [r.x, r.y + r.h]]
}

pub fn bounds_of_points(pts: &[Vec2]) -> Rect {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for &[x, y] in pts {
        if x < min_x {
            min_x = x;
        }
        if y < min_y {
            min_y = y;
        }
        if x > max_x {
            max_x = x;
        }
        if y > max_y {
            max_y = y;
        }
    }
    if !min_x.is_finite() {
        return Rect::default();
    }
    Rect { x: min_x, y: min_y, w: max_x - min_x, h: max_y - min_y }
}

pub fn union_rects(rs: &[Rect]) -> Rect {
    let pts: Vec<Vec2> = rs.iter().flat_map(|r| [[r.x, r.y], [r.x + r.w, r.y + r.h]]).collect();
    bounds_of_points(&pts)
}

pub fn rects_intersect(a: &Rect, b: &Rect) -> bool {
    a.x <= b.x + b.w && b.x <= a.x + a.w && a.y <= b.y + b.h && b.y <= a.y + a.h
}

pub fn rect_contains(outer: &Rect, inner: &Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.w <= outer.x + outer.w
        && inner.y + inner.h <= outer.y + outer.h
}

/// Decompose a (rotation + translation + uniform-ish scale) matrix applied to a w×h box back into an Xf.
pub fn xf_from_matrix(m: &Mat2D, w: f64, h: f64) -> Xf {
    let sx = hypot(m[0], m[1]);
    let det = m[0] * m[3] - m[1] * m[2];
    let sy = det / if sx != 0.0 && !sx.is_nan() { sx } else { 1.0 };
    let rot = normalize_angle((atan2(m[1], m[0]) * 180.0) / PI);
    let big_w = w * sx;
    let big_h = h * sy.abs();
    let [cx, cy] = apply(m, [w / 2.0, h / 2.0]);
    let mut xf = Xf::new(cx - big_w / 2.0, cy - big_h / 2.0, big_w, big_h, rot);
    if sy < 0.0 {
        xf.flip_y = Some(true);
    }
    xf
}
