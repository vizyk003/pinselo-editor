// SPDX-License-Identifier: Apache-2.0
//! Boolean operations on vector paths (DS-100): union, subtract, intersect and exclude.
//!
//! Outlines are flattened (fine tolerance, short pieces), combined with `i_overlay` (robust against
//! self-intersections, holes, coincident edges and touching shapes; nonzero fill per input), and the resulting
//! polygons are turned back into Béziers: runs between corners are fitted with cubics (Schneider's algorithm) so
//! curves stay editable with few nodes, and long straight edges stay lines.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use serde::{Deserialize, Serialize};

use crate::vector::{P, PathNode, SubPath, VectorPath};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BoolOp {
    Union,
    /// The first path minus all the others.
    Subtract,
    Intersect,
    /// Areas covered by an odd number of the paths.
    Exclude,
}

type Contours = Vec<Vec<P>>;
type Shapes = Vec<Vec<Vec<P>>>;

/// Flatten settings for a set of paths: tolerance and maximum piece length, relative to their size.
fn flatten_params(paths: &[&VectorPath]) -> (f64, f64) {
    let size = paths.iter().filter_map(|p| p.bounds()).map(|b| b.w.max(b.h)).fold(0.0, f64::max).max(1.0);
    (size * 2e-4, size / 64.0)
}

fn contours(p: &VectorPath, tol: f64, max_seg: f64) -> Contours {
    // Every subpath counts as closed (fill semantics); fewer than three points enclose nothing.
    p.flatten(tol, max_seg).into_iter().map(|(pts, _)| pts).filter(|c| c.len() >= 3).collect()
}

/// One path as clean shapes (nonzero fill resolved, consistent orientation).
fn normalize(c: &Contours, fill: FillRule) -> Shapes {
    if c.is_empty() {
        return vec![];
    }
    let empty: Contours = vec![];
    c.overlay(&empty, OverlayRule::Subject, fill)
}

fn flat(s: &Shapes) -> Contours {
    s.iter().flat_map(|sh| sh.iter().cloned()).collect()
}

/// Combine paths (in the same coordinate space). Inputs use the nonzero fill rule.
pub fn boolean(paths: &[&VectorPath], op: BoolOp) -> VectorPath {
    let (tol, max_seg) = flatten_params(paths);
    let norm: Vec<Shapes> = paths.iter().map(|p| normalize(&contours(p, tol, max_seg), FillRule::NonZero)).collect();
    let Some(first) = norm.first() else { return VectorPath::default() };
    let mut acc: Shapes = first.clone();
    match op {
        BoolOp::Subtract => {
            let rest: Contours = norm[1..].iter().flat_map(flat).collect();
            if !rest.is_empty() && !acc.is_empty() {
                let others = normalize(&rest, FillRule::NonZero);
                acc = flat(&acc).overlay(&flat(&others), OverlayRule::Difference, FillRule::NonZero);
            }
        }
        _ => {
            let rule = match op {
                BoolOp::Union => OverlayRule::Union,
                BoolOp::Intersect => OverlayRule::Intersect,
                _ => OverlayRule::Xor,
            };
            for s in &norm[1..] {
                acc = flat(&acc).overlay(&flat(s), rule, FillRule::NonZero);
            }
        }
    }
    from_shapes(&acc, tol, max_seg)
}

/// Resolve a path drawn with the even-odd rule into one that fills the same area with nonzero (SVG import).
pub fn even_odd_to_nonzero(p: &VectorPath) -> VectorPath {
    let (tol, max_seg) = flatten_params(&[p]);
    from_shapes(&normalize(&contours(p, tol, max_seg), FillRule::EvenOdd), tol, max_seg)
}

fn from_shapes(s: &Shapes, tol: f64, max_seg: f64) -> VectorPath {
    let mut out = VectorPath::default();
    for shape in s {
        for c in shape {
            if c.len() >= 3 {
                out.subpaths.push(fit_closed(c, tol, max_seg));
            }
        }
    }
    out
}

fn d2(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

fn norm(v: P) -> P {
    let l = v[0].hypot(v[1]);
    if l < 1e-15 { [0.0, 0.0] } else { [v[0] / l, v[1] / l] }
}

/// Turn a closed polygon back into a Bézier subpath.
fn fit_closed(c: &[P], tol: f64, max_seg: f64) -> SubPath {
    let n = c.len();
    // A vertex is a break when the outline turns sharply there or an edge next to it is long (a straight edge of
    // an input, or a clipped piece of one).
    let long = max_seg * 1.5;
    let is_long = |i: usize| d2(c[i], c[(i + 1) % n]) > long;
    let brk: Vec<bool> = (0..n)
        .map(|i| {
            let a = c[(i + n - 1) % n];
            let b = c[i];
            let cc = c[(i + 1) % n];
            let u = norm(sub(b, a));
            let v = norm(sub(cc, b));
            let cos = u[0] * v[0] + u[1] * v[1];
            cos < 0.82 || is_long(i) || is_long((i + n - 1) % n)
        })
        .collect();
    let breaks: Vec<usize> = (0..n).filter(|i| brk[*i]).collect();
    if breaks.is_empty() {
        // A smooth closed loop (circle-like): split in two runs at opposite points.
        let mut run1: Vec<P> = c[..=n / 2].to_vec();
        let mut run2: Vec<P> = c[n / 2..].to_vec();
        run2.push(c[0]);
        let err = tol * 5.0;
        let mut nodes: Vec<PathNode> = vec![PathNode::corner(c[0])];
        append_fit(&mut nodes, &mut run1, err);
        append_fit(&mut nodes, &mut run2, err);
        let last = nodes.pop().expect("node");
        nodes[0].cin = last.cin;
        for nd in &mut nodes {
            nd.smooth = nd.cin.is_some() && nd.cout.is_some();
        }
        return SubPath { nodes, closed: true };
    }
    let start = breaks[0];
    let mut nodes: Vec<PathNode> = vec![PathNode::corner(c[start])];
    let mut i = start;
    let err = tol * 5.0;
    loop {
        // Walk to the next break.
        let mut run = vec![c[i]];
        let mut j = (i + 1) % n;
        loop {
            run.push(c[j]);
            if brk[j] {
                break;
            }
            j = (j + 1) % n;
        }
        if run.len() == 2 {
            nodes.push(PathNode::corner(c[j]));
        } else {
            append_fit(&mut nodes, &mut run, err);
        }
        i = j;
        if i == start {
            break;
        }
    }
    // The walk ended on the start node: fold it into the first one.
    let last = nodes.pop().expect("node");
    nodes[0].cin = last.cin;
    SubPath { nodes, closed: true }
}

/// Fit cubics through `pts` (first point already in `nodes`), appending nodes up to the last point.
fn append_fit(nodes: &mut Vec<PathNode>, pts: &mut [P], err: f64) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    let t1 = norm(sub(pts[1], pts[0]));
    let t2 = norm(sub(pts[n - 2], pts[n - 1]));
    let mut out: Vec<[P; 4]> = Vec::new();
    fit_cubic(pts, t1, t2, err, &mut out, 0);
    for c in out {
        if let Some(last) = nodes.last_mut() {
            last.cout = Some(c[1]);
        }
        nodes.push(PathNode { p: c[3], cin: Some(c[2]), cout: None, smooth: false });
    }
}

// --- Schneider, "An Algorithm for Automatically Fitting Digitized Curves" (Graphics Gems, 1990).

fn bez(c: &[P; 4], t: f64) -> P {
    crate::vector::cubic_point(c, t)
}

fn chord_params(pts: &[P]) -> Vec<f64> {
    let mut u = vec![0.0];
    for i in 1..pts.len() {
        let l = u[i - 1] + d2(pts[i], pts[i - 1]);
        u.push(l);
    }
    let total = *u.last().expect("params");
    if total > 0.0 {
        for v in &mut u {
            *v /= total;
        }
    }
    u
}

fn generate(pts: &[P], u: &[f64], t1: P, t2: P) -> [P; 4] {
    let (p0, p3) = (pts[0], pts[pts.len() - 1]);
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &t) in u.iter().enumerate() {
        let mt = 1.0 - t;
        let b0 = mt * mt * mt;
        let b1 = 3.0 * mt * mt * t;
        let b2 = 3.0 * mt * t * t;
        let b3 = t * t * t;
        let a1 = [t1[0] * b1, t1[1] * b1];
        let a2 = [t2[0] * b2, t2[1] * b2];
        c00 += a1[0] * a1[0] + a1[1] * a1[1];
        c01 += a1[0] * a2[0] + a1[1] * a2[1];
        c11 += a2[0] * a2[0] + a2[1] * a2[1];
        let tmp = [pts[i][0] - (p0[0] * (b0 + b1) + p3[0] * (b2 + b3)), pts[i][1] - (p0[1] * (b0 + b1) + p3[1] * (b2 + b3))];
        x0 += a1[0] * tmp[0] + a1[1] * tmp[1];
        x1 += a2[0] * tmp[0] + a2[1] * tmp[1];
    }
    let det = c00 * c11 - c01 * c01;
    let seg = d2(p0, p3);
    let (mut al, mut ar) = if det.abs() > 1e-12 { ((x0 * c11 - c01 * x1) / det, (c00 * x1 - c01 * x0) / det) } else { (0.0, 0.0) };
    let eps = 1e-6 * seg;
    if al < eps || ar < eps || !al.is_finite() || !ar.is_finite() {
        al = seg / 3.0;
        ar = seg / 3.0;
    }
    [p0, [p0[0] + t1[0] * al, p0[1] + t1[1] * al], [p3[0] + t2[0] * ar, p3[1] + t2[1] * ar], p3]
}

fn max_error(pts: &[P], c: &[P; 4], u: &[f64]) -> (f64, usize) {
    let mut worst = (0.0, pts.len() / 2);
    for i in 1..pts.len() - 1 {
        let d = d2(bez(c, u[i]), pts[i]);
        if d > worst.0 {
            worst = (d, i);
        }
    }
    worst
}

fn reparam(pts: &[P], c: &[P; 4], u: &mut [f64]) {
    let q1 = [sub(c[1], c[0]), sub(c[2], c[1]), sub(c[3], c[2])].map(|v| [v[0] * 3.0, v[1] * 3.0]);
    let q2 = [sub(q1[1], q1[0]), sub(q1[2], q1[1])].map(|v| [v[0] * 2.0, v[1] * 2.0]);
    for (i, t) in u.iter_mut().enumerate() {
        let p = bez(c, *t);
        let mt = 1.0 - *t;
        let d1 = [q1[0][0] * mt * mt + q1[1][0] * 2.0 * mt * *t + q1[2][0] * *t * *t, q1[0][1] * mt * mt + q1[1][1] * 2.0 * mt * *t + q1[2][1] * *t * *t];
        let dd = [q2[0][0] * mt + q2[1][0] * *t, q2[0][1] * mt + q2[1][1] * *t];
        let diff = sub(p, pts[i]);
        let num = diff[0] * d1[0] + diff[1] * d1[1];
        let den = d1[0] * d1[0] + d1[1] * d1[1] + diff[0] * dd[0] + diff[1] * dd[1];
        if den.abs() > 1e-12 {
            *t = (*t - num / den).clamp(0.0, 1.0);
        }
    }
}

fn fit_cubic(pts: &[P], t1: P, t2: P, err: f64, out: &mut Vec<[P; 4]>, depth: usize) {
    let n = pts.len();
    if n == 2 {
        let l = d2(pts[0], pts[1]) / 3.0;
        out.push([pts[0], [pts[0][0] + t1[0] * l, pts[0][1] + t1[1] * l], [pts[1][0] + t2[0] * l, pts[1][1] + t2[1] * l], pts[1]]);
        return;
    }
    let mut u = chord_params(pts);
    let mut c = generate(pts, &u, t1, t2);
    let (mut e, mut split) = max_error(pts, &c, &u);
    if e <= err {
        out.push(c);
        return;
    }
    if e < err * 4.0 {
        for _ in 0..8 {
            reparam(pts, &c, &mut u);
            c = generate(pts, &u, t1, t2);
            let (e2, s2) = max_error(pts, &c, &u);
            e = e2;
            split = s2;
            if e <= err {
                out.push(c);
                return;
            }
        }
    }
    if depth > 24 {
        out.push(c);
        return;
    }
    let split = split.clamp(1, n - 2);
    let tc = norm(sub(pts[split - 1], pts[split + 1]));
    fit_cubic(&pts[..=split], t1, tc, err, out, depth + 1);
    fit_cubic(&pts[split..], [-tc[0], -tc[1]], t2, err, out, depth + 1);
}

/// Signed area of a flattened path (positive = clockwise in y-down space), for tests and checks.
pub fn area(p: &VectorPath) -> f64 {
    let (tol, _) = flatten_params(&[p]);
    p.flatten(tol, 0.0)
        .iter()
        .map(|(pts, _)| {
            let n = pts.len();
            (0..n).map(|i| pts[i][0] * pts[(i + 1) % n][1] - pts[(i + 1) % n][0] * pts[i][1]).sum::<f64>() / 2.0
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> VectorPath {
        VectorPath::parse(&format!("M{x} {y} L{} {y} L{} {} L{x} {} Z", x + w, x + w, y + h, y + h))
    }

    fn circle(cx: f64, cy: f64, r: f64) -> VectorPath {
        let k = 0.5522847498 * r;
        VectorPath::parse(&format!(
            "M{} {cy} C{} {} {} {} {cx} {} C{} {} {} {} {} {cy} C{} {} {} {} {cx} {} C{} {} {} {} {} {cy} Z",
            cx + r, cx + r, cy + k, cx + k, cy + r, cy + r,
            cx - k, cy + r, cx - r, cy + k, cx - r,
            cx - r, cy - k, cx - k, cy - r, cy - r,
            cx + k, cy - r, cx + r, cy - k, cx + r
        ))
    }

    fn abs_area(p: &VectorPath) -> f64 {
        area(p).abs()
    }

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs().max(1.0)
    }

    #[test]
    fn rect_ops() {
        let a = rect(0.0, 0.0, 100.0, 100.0);
        let b = rect(50.0, 50.0, 100.0, 100.0);
        let u = boolean(&[&a, &b], BoolOp::Union);
        assert!(close(abs_area(&u), 17500.0, 1e-6), "{}", abs_area(&u));
        assert_eq!(u.subpaths.len(), 1);
        assert_eq!(u.subpaths[0].nodes.len(), 8, "{}", u.to_d());
        assert!(u.subpaths[0].nodes.iter().all(|n| n.cin.is_none() && n.cout.is_none()), "straight edges stay lines");
        let s = boolean(&[&a, &b], BoolOp::Subtract);
        assert!(close(abs_area(&s), 7500.0, 1e-6));
        let i = boolean(&[&a, &b], BoolOp::Intersect);
        assert!(close(abs_area(&i), 2500.0, 1e-6));
        let x = boolean(&[&a, &b], BoolOp::Exclude);
        assert!(close(abs_area(&x), 15000.0, 1e-6));
    }

    #[test]
    fn coincident_edges_and_touching() {
        // Two squares sharing an edge union into one rectangle with four corners.
        let a = rect(0.0, 0.0, 50.0, 50.0);
        let b = rect(50.0, 0.0, 50.0, 50.0);
        let u = boolean(&[&a, &b], BoolOp::Union);
        assert_eq!(u.subpaths.len(), 1);
        assert!(close(abs_area(&u), 5000.0, 1e-6));
        assert_eq!(u.subpaths[0].nodes.len(), 4, "{}", u.to_d());
        // Identical shapes: subtract leaves nothing, intersect leaves the shape.
        assert!(boolean(&[&a, &a], BoolOp::Subtract).is_empty());
        assert!(close(abs_area(&boolean(&[&a, &a.clone()], BoolOp::Intersect)), 2500.0, 1e-6));
        // Disjoint: intersect is empty, union keeps both.
        let far = rect(500.0, 500.0, 10.0, 10.0);
        assert!(boolean(&[&a, &far], BoolOp::Intersect).is_empty());
        assert_eq!(boolean(&[&a, &far], BoolOp::Union).subpaths.len(), 2);
    }

    #[test]
    fn holes_survive() {
        // Subtracting a smaller square from the middle leaves an outer contour and a hole wound the other way.
        let a = rect(0.0, 0.0, 100.0, 100.0);
        let b = rect(25.0, 25.0, 50.0, 50.0);
        let s = boolean(&[&a, &b], BoolOp::Subtract);
        assert_eq!(s.subpaths.len(), 2);
        assert!(close(abs_area(&s), 7500.0, 1e-6));
        let areas: Vec<f64> = s.subpaths.iter().map(|sp| area(&VectorPath { subpaths: vec![sp.clone()] })).collect();
        assert!(areas[0] * areas[1] < 0.0, "hole is counter-wound: {areas:?}");
        // A ring input (outer + reversed inner) keeps its hole through a union with a far shape.
        let ring = VectorPath::parse("M0 0 L100 0 L100 100 L0 100 Z M25 25 L25 75 L75 75 L75 25 Z");
        let u = boolean(&[&ring, &rect(300.0, 0.0, 10.0, 10.0)], BoolOp::Union);
        assert!(close(abs_area(&u), 7500.0 + 100.0, 1e-6));
    }

    #[test]
    fn self_intersection_nonzero() {
        // A bow tie crosses itself: nonzero fills both lobes.
        let bow = VectorPath::parse("M0 0 L100 100 L100 0 L0 100 Z");
        let u = boolean(&[&bow, &rect(1000.0, 0.0, 1.0, 1.0)], BoolOp::Union);
        assert!(close(abs_area(&u), 5000.0 + 1.0, 1e-6), "{}", abs_area(&u));
        // A pentagram drawn in one stroke: nonzero also fills the centre.
        let star = VectorPath::parse("M50 0 L79 90 L2 35 L98 35 L21 90 Z");
        let n = boolean(&[&star], BoolOp::Union);
        let eo = even_odd_to_nonzero(&star);
        assert!(abs_area(&n) > abs_area(&eo) + 100.0, "{} vs {}", abs_area(&n), abs_area(&eo));
    }

    #[test]
    fn curves_come_back_as_few_curves() {
        let a = circle(50.0, 50.0, 50.0);
        let b = circle(100.0, 50.0, 50.0);
        let u = boolean(&[&a, &b], BoolOp::Union);
        let expect = 2.0 * std::f64::consts::PI * 2500.0 - 2.0 * (2500.0 * (std::f64::consts::PI / 3.0) - 0.5 * 2500.0 * (2.0 * std::f64::consts::PI / 3.0).sin());
        assert!(close(abs_area(&u), expect, 2e-3), "{} vs {expect}", abs_area(&u));
        let nodes = u.subpaths[0].nodes.len();
        assert!(nodes <= 16, "fitted to {nodes} nodes: {}", u.to_d());
        assert!(u.subpaths[0].nodes.iter().any(|n| n.cout.is_some()), "curves are curves");
        // A lone circle round-trips with a handful of smooth nodes.
        let c = boolean(&[&circle(0.0, 0.0, 40.0)], BoolOp::Union);
        assert!(c.subpaths[0].nodes.len() <= 12, "{}", c.to_d());
        assert!(close(abs_area(&c), std::f64::consts::PI * 1600.0, 2e-3), "{} {}", abs_area(&c), c.to_d());
    }

    #[test]
    fn open_and_empty_inputs() {
        assert!(boolean(&[], BoolOp::Union).is_empty());
        let line = VectorPath::parse("M0 0 L10 10");
        assert!(boolean(&[&line], BoolOp::Union).is_empty(), "a line encloses nothing");
        // An open curve is filled as if closed.
        let open = VectorPath::parse("M0 0 L100 0 L100 100");
        assert!(close(abs_area(&boolean(&[&open], BoolOp::Union)), 5000.0, 1e-6));
    }
}
