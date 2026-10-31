// SPDX-License-Identifier: Apache-2.0
//! Editable vector paths (DS-100): the node model behind the pen tool, node editing and Boolean operations.
//!
//! A path element is a `custom` shape whose `d` is an absolute path (M L C Q Z) in its unit box. Editing parses
//! that into [`VectorPath`] — subpaths of Bézier nodes, each with an optional incoming and outgoing handle —
//! scales it to the element box, edits it, and writes it back normalised to the new tight bounds
//! ([`VectorPath::to_unit`]).
//!
//! A segment from node `a` to node `b` is a cubic when `a.out` or `b.in` is set (a missing handle sits on its
//! node) and a straight line otherwise. Quadratics are raised to cubics on parse.

use serde::{Deserialize, Serialize};

use crate::matrix::apply;
use oc_schema::primitives::{Mat2D, Rect};

pub type P = [f64; 2];

/// One anchor point with its handles (absolute positions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathNode {
    pub p: P,
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub cin: Option<P>,
    #[serde(rename = "out", default, skip_serializing_if = "Option::is_none")]
    pub cout: Option<P>,
    /// Smooth nodes keep their two handles collinear while one is dragged.
    #[serde(default)]
    pub smooth: bool,
}

impl PathNode {
    pub fn corner(p: P) -> Self {
        PathNode { p, cin: None, cout: None, smooth: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SubPath {
    pub nodes: Vec<PathNode>,
    pub closed: bool,
}

/// Which part of a node is addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodePart {
    Anchor,
    In,
    Out,
}

/// A node address: subpath `s`, node `i`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeRef {
    pub s: usize,
    pub i: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VectorPath {
    pub subpaths: Vec<SubPath>,
}

/// A cubic segment `[p0, c1, c2, p3]` (lines have c1 = p0, c2 = p3).
pub type Cubic = [P; 4];

fn lerp(a: P, b: P, t: f64) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn dist(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}

fn scl(a: P, k: f64) -> P {
    [a[0] * k, a[1] * k]
}

/// Point on a cubic at `t`.
pub fn cubic_point(c: &Cubic, t: f64) -> P {
    let mt = 1.0 - t;
    let a = mt * mt * mt;
    let b = 3.0 * mt * mt * t;
    let cc = 3.0 * mt * t * t;
    let d = t * t * t;
    [a * c[0][0] + b * c[1][0] + cc * c[2][0] + d * c[3][0], a * c[0][1] + b * c[1][1] + cc * c[2][1] + d * c[3][1]]
}

/// de Casteljau split at `t`: (left, right).
pub fn split_cubic(c: &Cubic, t: f64) -> (Cubic, Cubic) {
    let p01 = lerp(c[0], c[1], t);
    let p12 = lerp(c[1], c[2], t);
    let p23 = lerp(c[2], c[3], t);
    let p012 = lerp(p01, p12, t);
    let p123 = lerp(p12, p23, t);
    let m = lerp(p012, p123, t);
    ([c[0], p01, p012, m], [m, p123, p23, c[3]])
}

fn is_line(c: &Cubic) -> bool {
    c[1] == c[0] && c[2] == c[3]
}

/// Parameter values in (0, 1) where one coordinate of the cubic has an extremum.
fn extrema(c: &Cubic) -> Vec<f64> {
    let mut out = Vec::new();
    for (p0, p1, p2, p3) in [(c[0][0], c[1][0], c[2][0], c[3][0]), (c[0][1], c[1][1], c[2][1], c[3][1])] {
        // Derivative / 3: a t² + b t + c
        let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
        let b = 2.0 * (p0 - 2.0 * p1 + p2);
        let cc = p1 - p0;
        if a.abs() < 1e-12 {
            if b.abs() > 1e-12 {
                out.push(-cc / b);
            }
        } else {
            let disc = b * b - 4.0 * a * cc;
            if disc >= 0.0 {
                let s = disc.sqrt();
                out.push((-b + s) / (2.0 * a));
                out.push((-b - s) / (2.0 * a));
            }
        }
    }
    out.retain(|t| *t > 0.0 && *t < 1.0);
    out
}

fn fmt(v: f64) -> String {
    let r = (v * 1e6).round() / 1e6;
    let r = if r == 0.0 { 0.0 } else { r };
    oc_schema::js::num_to_string(r)
}

struct Tok<'a> {
    b: &'a [u8],
    i: usize,
}

impl Tok<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i].is_ascii_whitespace() || self.b[self.i] == b',') {
            self.i += 1;
        }
    }
    fn num(&mut self) -> Option<f64> {
        self.ws();
        let st = self.i;
        let b = self.b;
        if self.i < b.len() && (b[self.i] == b'-' || b[self.i] == b'+') {
            self.i += 1;
        }
        let mut dot = false;
        while self.i < b.len() && (b[self.i].is_ascii_digit() || (b[self.i] == b'.' && !dot)) {
            dot |= b[self.i] == b'.';
            self.i += 1;
        }
        if self.i < b.len() && (b[self.i] == b'e' || b[self.i] == b'E') {
            let save = self.i;
            self.i += 1;
            if self.i < b.len() && (b[self.i] == b'-' || b[self.i] == b'+') {
                self.i += 1;
            }
            let ds = self.i;
            while self.i < b.len() && b[self.i].is_ascii_digit() {
                self.i += 1;
            }
            if self.i == ds {
                self.i = save;
            }
        }
        std::str::from_utf8(&b[st..self.i]).ok()?.parse().ok()
    }
    fn has_num(&mut self) -> bool {
        self.ws();
        self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.')
    }
}

impl VectorPath {
    /// Parse SVG path data (M L H V C S Q T Z, absolute or relative; an arc is replaced by a line to its end).
    /// Malformed data stops the parse at the error (what was read so far is kept).
    pub fn parse(d: &str) -> VectorPath {
        let mut t = Tok { b: d.as_bytes(), i: 0 };
        let mut out = VectorPath::default();
        let mut cur: Option<SubPath> = None;
        let mut c: P = [0.0, 0.0];
        let mut start: P = [0.0, 0.0];
        let mut last_ctrl: Option<(u8, P)> = None;
        let mut cmd = 0u8;
        loop {
            t.ws();
            if t.i >= t.b.len() {
                break;
            }
            let ch = t.b[t.i];
            if ch.is_ascii_alphabetic() {
                cmd = ch;
                t.i += 1;
            } else if !t.has_num() || cmd == 0 || cmd.eq_ignore_ascii_case(&b'z') {
                break;
            }
            let rel = cmd.is_ascii_lowercase();
            let up = cmd.to_ascii_uppercase();
            let base = if rel { c } else { [0.0, 0.0] };
            macro_rules! pt {
                () => {{
                    let (Some(x), Some(y)) = (t.num(), t.num()) else { break };
                    [x + base[0], y + base[1]]
                }};
            }
            let ensure = |cur: &mut Option<SubPath>, at: P| {
                if cur.is_none() {
                    *cur = Some(SubPath { nodes: vec![PathNode::corner(at)], closed: false });
                }
            };
            match up {
                b'M' => {
                    let p = pt!();
                    if let Some(sp) = cur.take() {
                        out.subpaths.push(sp);
                    }
                    cur = Some(SubPath { nodes: vec![PathNode::corner(p)], closed: false });
                    c = p;
                    start = p;
                    cmd = if rel { b'l' } else { b'L' };
                    last_ctrl = None;
                }
                b'L' | b'H' | b'V' | b'A' => {
                    let p = match up {
                        b'L' => pt!(),
                        b'H' => {
                            let Some(x) = t.num() else { break };
                            [x + base[0], c[1]]
                        }
                        b'V' => {
                            let Some(y) = t.num() else { break };
                            [c[0], y + base[1]]
                        }
                        _ => {
                            for _ in 0..5 {
                                if t.num().is_none() {
                                    break;
                                }
                            }
                            pt!()
                        }
                    };
                    ensure(&mut cur, c);
                    cur.as_mut().expect("subpath").nodes.push(PathNode::corner(p));
                    c = p;
                    last_ctrl = None;
                }
                b'C' | b'S' => {
                    let c1 = if up == b'C' {
                        pt!()
                    } else {
                        match last_ctrl {
                            Some((b'C', lc)) => sub(scl(c, 2.0), lc),
                            _ => c,
                        }
                    };
                    let c2 = pt!();
                    let p = pt!();
                    ensure(&mut cur, c);
                    let sp = cur.as_mut().expect("subpath");
                    sp.nodes.last_mut().expect("node").cout = Some(c1);
                    sp.nodes.push(PathNode { p, cin: Some(c2), cout: None, smooth: false });
                    c = p;
                    last_ctrl = Some((b'C', c2));
                }
                b'Q' | b'T' => {
                    let q = if up == b'Q' {
                        pt!()
                    } else {
                        match last_ctrl {
                            Some((b'Q', lc)) => sub(scl(c, 2.0), lc),
                            _ => c,
                        }
                    };
                    let p = pt!();
                    ensure(&mut cur, c);
                    let sp = cur.as_mut().expect("subpath");
                    sp.nodes.last_mut().expect("node").cout = Some(lerp(c, q, 2.0 / 3.0));
                    sp.nodes.push(PathNode { p, cin: Some(lerp(p, q, 2.0 / 3.0)), cout: None, smooth: false });
                    c = p;
                    last_ctrl = Some((b'Q', q));
                }
                b'Z' => {
                    if let Some(mut sp) = cur.take() {
                        sp.closed = true;
                        out.subpaths.push(sp);
                    }
                    c = start;
                    last_ctrl = None;
                }
                _ => break,
            }
        }
        if let Some(sp) = cur.take() {
            out.subpaths.push(sp);
        }
        for sp in &mut out.subpaths {
            // `… L p0 Z` / `… C … p0 Z`: the explicit closing node is the first node.
            if sp.closed && sp.nodes.len() > 1 {
                let (f, l) = (sp.nodes[0].p, sp.nodes[sp.nodes.len() - 1].p);
                if dist(f, l) < 1e-9 {
                    let last = sp.nodes.pop().expect("node");
                    sp.nodes[0].cin = last.cin;
                }
            }
            for n in &mut sp.nodes {
                // A handle on its anchor is no handle (rounded corners of radius 0, `C` segments that are lines).
                let p = n.p;
                n.cin = n.cin.filter(|c| dist(*c, p) > 1e-9);
                n.cout = n.cout.filter(|c| dist(*c, p) > 1e-9);
                n.smooth = is_smooth(n);
            }
        }
        out.subpaths.retain(|sp| !sp.nodes.is_empty());
        out
    }

    /// Absolute path data (M L C Z).
    pub fn to_d(&self) -> String {
        let mut s = String::new();
        for sp in &self.subpaths {
            let Some(first) = sp.nodes.first() else { continue };
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(&format!("M{} {}", fmt(first.p[0]), fmt(first.p[1])));
            let n = sp.nodes.len();
            let segs = if sp.closed { n } else { n.saturating_sub(1) };
            for k in 0..segs {
                let a = &sp.nodes[k];
                let b = &sp.nodes[(k + 1) % n];
                if a.cout.is_none() && b.cin.is_none() {
                    if sp.closed && k + 1 == n {
                        break;
                    }
                    s.push_str(&format!(" L{} {}", fmt(b.p[0]), fmt(b.p[1])));
                } else {
                    let c1 = a.cout.unwrap_or(a.p);
                    let c2 = b.cin.unwrap_or(b.p);
                    s.push_str(&format!(" C{} {} {} {} {} {}", fmt(c1[0]), fmt(c1[1]), fmt(c2[0]), fmt(c2[1]), fmt(b.p[0]), fmt(b.p[1])));
                }
            }
            if sp.closed {
                s.push_str(" Z");
            }
        }
        s
    }

    pub fn is_empty(&self) -> bool {
        self.subpaths.iter().all(|s| s.nodes.is_empty())
    }

    pub fn node_count(&self) -> usize {
        self.subpaths.iter().map(|s| s.nodes.len()).sum()
    }

    /// Every node address in order.
    pub fn refs(&self) -> Vec<NodeRef> {
        self.subpaths.iter().enumerate().flat_map(|(s, sp)| (0..sp.nodes.len()).map(move |i| NodeRef { s, i })).collect()
    }

    pub fn node(&self, r: NodeRef) -> Option<&PathNode> {
        self.subpaths.get(r.s).and_then(|sp| sp.nodes.get(r.i))
    }

    pub fn node_mut(&mut self, r: NodeRef) -> Option<&mut PathNode> {
        self.subpaths.get_mut(r.s).and_then(|sp| sp.nodes.get_mut(r.i))
    }

    /// Map every point (anchors and handles).
    pub fn map(&self, f: impl Fn(P) -> P) -> VectorPath {
        let mut out = self.clone();
        for sp in &mut out.subpaths {
            for n in &mut sp.nodes {
                n.p = f(n.p);
                n.cin = n.cin.map(&f);
                n.cout = n.cout.map(&f);
            }
        }
        out
    }

    pub fn transformed(&self, m: &Mat2D) -> VectorPath {
        self.map(|p| apply(m, p))
    }

    pub fn scaled(&self, sx: f64, sy: f64) -> VectorPath {
        self.map(|p| [p[0] * sx, p[1] * sy])
    }

    /// The segments of subpath `s` as cubics, with the index of their start node.
    pub fn segments(&self, s: usize) -> Vec<(usize, Cubic)> {
        let Some(sp) = self.subpaths.get(s) else { return vec![] };
        let n = sp.nodes.len();
        let segs = if sp.closed && n > 1 { n } else { n.saturating_sub(1) };
        (0..segs)
            .map(|k| {
                let a = &sp.nodes[k];
                let b = &sp.nodes[(k + 1) % n];
                (k, [a.p, a.cout.unwrap_or(a.p), b.cin.unwrap_or(b.p), b.p])
            })
            .collect()
    }

    /// Tight bounds (curve extrema, not control points). `None` for an empty path.
    pub fn bounds(&self) -> Option<Rect> {
        let mut pts: Vec<P> = Vec::new();
        for (s, sp) in self.subpaths.iter().enumerate() {
            for nd in &sp.nodes {
                pts.push(nd.p);
            }
            for (_, c) in self.segments(s) {
                for t in extrema(&c) {
                    pts.push(cubic_point(&c, t));
                }
            }
        }
        let first = pts.first()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[0], first[1]);
        for p in &pts {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }

    /// Normalise to a unit box: returns the box the path occupies (each side at least `min_side`, centred on the
    /// path when it is thinner) and the path mapped into 0..1.
    pub fn to_unit(&self, min_side: f64) -> Option<(Rect, VectorPath)> {
        let b = self.bounds()?;
        let (mut x, mut y, mut w, mut h) = (b.x, b.y, b.w, b.h);
        if w < min_side {
            x -= (min_side - w) / 2.0;
            w = min_side;
        }
        if h < min_side {
            y -= (min_side - h) / 2.0;
            h = min_side;
        }
        let unit = self.map(|p| [(p[0] - x) / w, (p[1] - y) / h]);
        Some((Rect { x, y, w, h }, unit))
    }

    /// Insert a node on segment `seg` (from node `seg` to the next) of subpath `s` at parameter `t`, keeping the
    /// curve's shape. Returns the new node's address.
    pub fn split_segment(&mut self, s: usize, seg: usize, t: f64) -> Option<NodeRef> {
        let segs = self.segments(s);
        let (_, c) = *segs.iter().find(|(k, _)| *k == seg)?;
        let t = t.clamp(0.01, 0.99);
        let sp = &mut self.subpaths[s];
        let n = sp.nodes.len();
        let next = (seg + 1) % n;
        if is_line(&c) {
            let p = lerp(c[0], c[3], t);
            sp.nodes.insert(seg + 1, PathNode::corner(p));
        } else {
            let (l, r) = split_cubic(&c, t);
            sp.nodes[seg].cout = Some(l[1]);
            sp.nodes[next].cin = Some(r[2]);
            sp.nodes.insert(seg + 1, PathNode { p: l[3], cin: Some(l[2]), cout: Some(r[1]), smooth: true });
        }
        Some(NodeRef { s, i: seg + 1 })
    }

    /// Delete nodes. A subpath left with fewer than two nodes (one for a single dot) is removed.
    pub fn delete_nodes(&mut self, refs: &[NodeRef]) {
        let mut sorted = refs.to_vec();
        sorted.sort();
        sorted.dedup();
        for r in sorted.iter().rev() {
            if let Some(sp) = self.subpaths.get_mut(r.s)
                && r.i < sp.nodes.len()
            {
                sp.nodes.remove(r.i);
            }
        }
        for sp in &mut self.subpaths {
            if sp.nodes.len() < 3 {
                sp.closed = false;
            }
        }
        self.subpaths.retain(|sp| sp.nodes.len() >= 2);
    }

    /// Move anchors (with their handles) by `d`.
    pub fn move_nodes(&mut self, refs: &[NodeRef], d: P) {
        for r in refs {
            if let Some(n) = self.node_mut(*r) {
                n.p = add(n.p, d);
                n.cin = n.cin.map(|c| add(c, d));
                n.cout = n.cout.map(|c| add(c, d));
            }
        }
    }

    /// Put a handle at `to`. On a smooth node the opposite handle turns to stay collinear (keeping its length;
    /// `mirror` also matches the length). Dragging a handle onto its anchor removes it.
    pub fn move_handle(&mut self, r: NodeRef, part: NodePart, to: P, mirror: bool) {
        let Some(n) = self.node_mut(r) else { return };
        let p = n.p;
        let h = if dist(to, p) < 1e-9 { None } else { Some(to) };
        let (mine, other) = match part {
            NodePart::In => (&mut n.cin, &mut n.cout),
            NodePart::Out => (&mut n.cout, &mut n.cin),
            NodePart::Anchor => return,
        };
        *mine = h;
        if (n.smooth || mirror)
            && let Some(hh) = h
        {
            let v = sub(p, hh);
            let len = dist(hh, p);
            let olen = if mirror { len } else { other.map_or(len, |o| dist(o, p)) };
            if len > 1e-12 {
                *other = Some(add(p, scl(v, olen / len)));
            }
        }
    }

    /// Make a node smooth (handles collinear, created from its neighbours when missing) or a corner (handles
    /// removed).
    pub fn set_smooth(&mut self, r: NodeRef, smooth: bool) {
        let Some(sp) = self.subpaths.get(r.s) else { return };
        let n = sp.nodes.len();
        if r.i >= n {
            return;
        }
        let prev = if r.i > 0 { Some(sp.nodes[r.i - 1].p) } else if sp.closed { Some(sp.nodes[n - 1].p) } else { None };
        let next = if r.i + 1 < n { Some(sp.nodes[r.i + 1].p) } else if sp.closed { Some(sp.nodes[0].p) } else { None };
        let node = &mut self.subpaths[r.s].nodes[r.i];
        node.smooth = smooth;
        if !smooth {
            node.cin = None;
            node.cout = None;
            return;
        }
        let p = node.p;
        // Tangent along prev → next (Catmull-Rom), handles a third of the way to each neighbour.
        let (tan, lin, lout) = match (prev, next) {
            (Some(a), Some(b)) => (sub(b, a), dist(a, p) / 3.0, dist(b, p) / 3.0),
            (Some(a), None) => (sub(p, a), dist(a, p) / 3.0, dist(a, p) / 3.0),
            (None, Some(b)) => (sub(b, p), dist(b, p) / 3.0, dist(b, p) / 3.0),
            (None, None) => return,
        };
        let tl = tan[0].hypot(tan[1]);
        if tl < 1e-12 {
            return;
        }
        let u = scl(tan, 1.0 / tl);
        let existing_in = node.cin.map(|c| dist(c, p)).filter(|l| *l > 1e-9);
        let existing_out = node.cout.map(|c| dist(c, p)).filter(|l| *l > 1e-9);
        let lin = existing_in.unwrap_or(lin);
        let lout = existing_out.unwrap_or(lout);
        node.cin = Some(sub(p, scl(u, lin)));
        node.cout = Some(add(p, scl(u, lout)));
    }

    /// Closest point on any segment to `p`: (subpath, segment start node, t, distance).
    pub fn nearest_segment(&self, p: P) -> Option<(usize, usize, f64, f64)> {
        let mut best: Option<(usize, usize, f64, f64)> = None;
        for s in 0..self.subpaths.len() {
            for (k, c) in self.segments(s) {
                let steps = if is_line(&c) { 1 } else { 48 };
                let mut bt = 0.0;
                let mut bd = f64::INFINITY;
                if steps == 1 {
                    let (a, b) = (c[0], c[3]);
                    let ab = sub(b, a);
                    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
                    let t = if l2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0) };
                    bt = t;
                    bd = dist(p, lerp(a, b, t));
                } else {
                    for j in 0..=steps {
                        let t = j as f64 / steps as f64;
                        let d = dist(p, cubic_point(&c, t));
                        if d < bd {
                            bd = d;
                            bt = t;
                        }
                    }
                    // Refine around the best sample.
                    let mut lo = (bt - 1.0 / steps as f64).max(0.0);
                    let mut hi = (bt + 1.0 / steps as f64).min(1.0);
                    for _ in 0..24 {
                        let m1 = lo + (hi - lo) / 3.0;
                        let m2 = hi - (hi - lo) / 3.0;
                        if dist(p, cubic_point(&c, m1)) < dist(p, cubic_point(&c, m2)) {
                            hi = m2;
                        } else {
                            lo = m1;
                        }
                    }
                    bt = (lo + hi) / 2.0;
                    bd = dist(p, cubic_point(&c, bt));
                }
                if best.is_none_or(|b| bd < b.3) {
                    best = Some((s, k, bt, bd));
                }
            }
        }
        best
    }

    /// Flatten to polylines within `tol` (each piece no longer than `max_seg` when that is > 0). Closed
    /// subpaths do not repeat their first point.
    pub fn flatten(&self, tol: f64, max_seg: f64) -> Vec<(Vec<P>, bool)> {
        let mut out = Vec::new();
        for (s, sp) in self.subpaths.iter().enumerate() {
            let Some(first) = sp.nodes.first() else { continue };
            let mut pts = vec![first.p];
            for (_, c) in self.segments(s) {
                if is_line(&c) {
                    pts.push(c[3]);
                    continue;
                }
                // Subdivision count from the control polygon's deviation (Wang's formula).
                let dd = |a: P, b: P, cc: P| {
                    let v = [a[0] - 2.0 * b[0] + cc[0], a[1] - 2.0 * b[1] + cc[1]];
                    v[0].hypot(v[1])
                };
                let m = dd(c[0], c[1], c[2]).max(dd(c[1], c[2], c[3]));
                let mut k = ((0.75 * m / tol.max(1e-9)).sqrt()).ceil().max(1.0);
                if max_seg > 0.0 {
                    let len = dist(c[0], c[1]) + dist(c[1], c[2]) + dist(c[2], c[3]);
                    k = k.max((len / max_seg).ceil());
                }
                let k = (k as usize).min(2048);
                for j in 1..=k {
                    pts.push(cubic_point(&c, j as f64 / k as f64));
                }
            }
            if sp.closed && pts.len() > 1 && dist(pts[0], pts[pts.len() - 1]) < 1e-9 {
                pts.pop();
            }
            out.push((pts, sp.closed));
        }
        out
    }
}

fn is_smooth(n: &PathNode) -> bool {
    match (n.cin, n.cout) {
        (Some(a), Some(b)) => {
            let u = sub(n.p, a);
            let v = sub(b, n.p);
            let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
            if lu < 1e-9 || lv < 1e-9 {
                return false;
            }
            let cross = (u[0] * v[1] - u[1] * v[0]) / (lu * lv);
            let dot = (u[0] * v[0] + u[1] * v[1]) / (lu * lv);
            cross.abs() < 0.02 && dot > 0.0
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_write_round_trip() {
        let d = "M0 0 L1 0 C1 0.5 0.5 1 0 1 Z";
        let vp = VectorPath::parse(d);
        assert_eq!(vp.subpaths.len(), 1);
        assert_eq!(vp.subpaths[0].nodes.len(), 3);
        assert!(vp.subpaths[0].closed);
        assert_eq!(vp.to_d(), d);
        // An explicit closing node merges into the first node (its incoming handle moves there).
        let vp = VectorPath::parse("M0 0 C0 -1 1 -1 1 0 C1 1 0 1 0 0 Z");
        assert_eq!(vp.subpaths[0].nodes.len(), 2);
        assert_eq!(vp.subpaths[0].nodes[0].cin, Some([0.0, 1.0]));
        assert_eq!(vp.to_d(), "M0 0 C0 -1 1 -1 1 0 C1 1 0 1 0 0 Z");
    }

    #[test]
    fn parse_relative_and_shorthands() {
        let vp = VectorPath::parse("m10 10 h10 v10 h-10 z m30 0 l5 5");
        assert_eq!(vp.subpaths.len(), 2);
        assert_eq!(vp.subpaths[0].nodes.iter().map(|n| n.p).collect::<Vec<_>>(), vec![[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 20.0]]);
        assert_eq!(vp.subpaths[1].nodes[0].p, [40.0, 10.0]);
        assert_eq!(vp.subpaths[1].nodes[1].p, [45.0, 15.0]);
        // Quadratics become exact cubics; S reflects the previous cubic handle.
        let q = VectorPath::parse("M0 0 Q3 3 6 0");
        assert_eq!(q.subpaths[0].nodes[0].cout, Some([2.0, 2.0]));
        assert_eq!(q.subpaths[0].nodes[1].cin, Some([4.0, 2.0]));
        let s = VectorPath::parse("M0 0 C0 1 1 1 1 0 S2 -1 2 0");
        assert_eq!(s.subpaths[0].nodes[1].cout, Some([1.0, -1.0]));
        assert!(s.subpaths[0].nodes[1].smooth);
        // Garbage stops the parse but keeps what was read.
        assert_eq!(VectorPath::parse("M0 0 L5 5 X 3").subpaths[0].nodes.len(), 2);
        assert!(VectorPath::parse("").is_empty());
    }

    #[test]
    fn tight_bounds_and_unit_box() {
        // A cubic bulging above its end points.
        let vp = VectorPath::parse("M0 0 C0 -40 100 -40 100 0");
        let b = vp.bounds().unwrap();
        assert!((b.y + 30.0).abs() < 1e-9, "{b:?}");
        assert_eq!(b.w, 100.0);
        let (bx, unit) = vp.to_unit(1.0).unwrap();
        assert_eq!(bx.x, 0.0);
        let ub = unit.bounds().unwrap();
        assert!((ub.w - 1.0).abs() < 1e-9 && (ub.h - 1.0).abs() < 1e-9);
        // A horizontal line gets a minimum height, centred.
        let (lb, _) = VectorPath::parse("M0 5 L10 5").to_unit(1.0).unwrap();
        assert_eq!((lb.y, lb.h), (4.5, 1.0));
    }

    #[test]
    fn split_keeps_shape() {
        let mut vp = VectorPath::parse("M0 0 C0 -40 100 -40 100 0");
        let before = cubic_point(&vp.segments(0)[0].1, 0.75);
        let r = vp.split_segment(0, 0, 0.5).unwrap();
        assert_eq!(r, NodeRef { s: 0, i: 1 });
        assert_eq!(vp.subpaths[0].nodes.len(), 3);
        let after = cubic_point(&vp.segments(0)[1].1, 0.5);
        assert!(dist(before, after) < 1e-9);
        // On a line: a corner node at the point.
        let mut l = VectorPath::parse("M0 0 L10 0 L10 10 Z");
        l.split_segment(0, 2, 0.5);
        assert_eq!(l.subpaths[0].nodes[3].p, [5.0, 5.0]);
        assert!(l.subpaths[0].closed);
    }

    #[test]
    fn delete_nodes_and_degenerate_subpaths() {
        let mut vp = VectorPath::parse("M0 0 L10 0 L10 10 Z M20 20 L30 30");
        vp.delete_nodes(&[NodeRef { s: 0, i: 1 }]);
        assert_eq!(vp.subpaths[0].nodes.len(), 2);
        assert!(!vp.subpaths[0].closed, "two nodes cannot enclose an area");
        vp.delete_nodes(&[NodeRef { s: 1, i: 0 }]);
        assert_eq!(vp.subpaths.len(), 1);
    }

    #[test]
    fn smooth_toggle_and_handle_drag() {
        let mut vp = VectorPath::parse("M0 0 L10 10 L20 0");
        let r = NodeRef { s: 0, i: 1 };
        vp.set_smooth(r, true);
        let n = vp.node(r).unwrap().clone();
        assert!(n.smooth && n.cin.is_some() && n.cout.is_some());
        assert!(is_smooth(&n));
        // Dragging one handle of a smooth node turns the other one.
        vp.move_handle(r, NodePart::Out, [15.0, 15.0], false);
        let n = vp.node(r).unwrap();
        assert!(is_smooth(n), "{n:?}");
        vp.set_smooth(r, false);
        assert_eq!(vp.node(r).unwrap().cin, None);
        assert_eq!(vp.to_d(), "M0 0 L10 10 L20 0");
        // Mirror makes both handles the same length.
        vp.move_handle(r, NodePart::In, [5.0, 10.0], true);
        assert_eq!(vp.node(r).unwrap().cout, Some([15.0, 10.0]));
    }

    #[test]
    fn nearest_segment_finds_curve_point() {
        let vp = VectorPath::parse("M0 0 C0 -40 100 -40 100 0 L100 50");
        let (s, k, t, d) = vp.nearest_segment([50.0, -31.0]).unwrap();
        assert_eq!((s, k), (0, 0));
        assert!((t - 0.5).abs() < 0.01 && d < 1.5, "{t} {d}");
        let (_, k, t, _) = vp.nearest_segment([101.0, 25.0]).unwrap();
        assert_eq!(k, 1);
        assert!((t - 0.5).abs() < 1e-9);
    }

    #[test]
    fn flatten_is_within_tolerance() {
        let vp = VectorPath::parse("M0 0 C0 -40 100 -40 100 0 Z");
        let f = vp.flatten(0.1, 0.0);
        assert_eq!(f.len(), 1);
        assert!(f[0].1);
        let pts = &f[0].0;
        assert!(pts.len() > 8);
        assert_ne!(pts.last(), pts.first());
    }
}
