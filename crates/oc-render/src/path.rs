// SPDX-License-Identifier: Apache-2.0
//! SVG path data → path commands, with the semantics of Skia's `SkParsePath::FromSVGString` (what CanvasKit's
//! `Path.MakeFromSVGString` used): implicit separators, implicit line-tos after a move, relative coordinates
//! measured from the current point, `S`/`T` reflection only after `C`/`S`, `Q`/`T`, a draw after `Z` starting at
//! the subpath's first point, and the whole path rejected on a parse error (nothing is drawn then). Arcs are
//! converted to cubic Béziers (≤ 90° each) the way `SkPath::arcTo` places them.
//!
//! The raster painter, the SVG writer and the PDF writer all draw from [`PathData`], so their geometry agrees.

/// One path command (absolute coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

/// A parsed path (nonzero winding fill).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PathData {
    pub cmds: Vec<Cmd>,
}

impl PathData {
    pub fn new() -> Self {
        PathData::default()
    }

    pub fn is_empty(&self) -> bool {
        !self.cmds.iter().any(|c| !matches!(c, Cmd::Move(..) | Cmd::Close))
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        self.cmds.push(Cmd::Move(x, y));
    }
    pub fn line_to(&mut self, x: f64, y: f64) {
        self.cmds.push(Cmd::Line(x, y));
    }
    pub fn quad_to(&mut self, x1: f64, y1: f64, x: f64, y: f64) {
        self.cmds.push(Cmd::Quad(x1, y1, x, y));
    }
    pub fn cubic_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) {
        self.cmds.push(Cmd::Cubic(x1, y1, x2, y2, x, y));
    }
    pub fn close(&mut self) {
        self.cmds.push(Cmd::Close);
    }

    /// Axis-aligned rectangle.
    pub fn rect(x: f64, y: f64, w: f64, h: f64) -> PathData {
        PathData { cmds: vec![Cmd::Move(x, y), Cmd::Line(x + w, y), Cmd::Line(x + w, y + h), Cmd::Line(x, y + h), Cmd::Close] }
    }

    /// Rounded rectangle (`RRectXY`), clockwise from the top-left.
    pub fn rrect(x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) -> PathData {
        let rx = rx.max(0.0).min(w / 2.0);
        let ry = ry.max(0.0).min(h / 2.0);
        if rx <= 0.0 || ry <= 0.0 {
            return PathData::rect(x, y, w, h);
        }
        let k = 0.552_284_749_830_793_6;
        let (ox, oy) = (rx * k, ry * k);
        let (r, b) = (x + w, y + h);
        let mut p = PathData::new();
        p.move_to(x + rx, y);
        p.line_to(r - rx, y);
        p.cubic_to(r - rx + ox, y, r, y + ry - oy, r, y + ry);
        p.line_to(r, b - ry);
        p.cubic_to(r, b - ry + oy, r - rx + ox, b, r - rx, b);
        p.line_to(x + rx, b);
        p.cubic_to(x + rx - ox, b, x, b - ry + oy, x, b - ry);
        p.line_to(x, y + ry);
        p.cubic_to(x, y + ry - oy, x + rx - ox, y, x + rx, y);
        p.close();
        p
    }

    /// Circle as four cubics.
    pub fn circle(cx: f64, cy: f64, r: f64) -> PathData {
        PathData::rrect(cx - r, cy - r, r * 2.0, r * 2.0, r, r)
    }

    /// A polyline (open).
    pub fn polyline(pts: &[[f64; 2]]) -> PathData {
        let mut p = PathData::new();
        for (i, [x, y]) in pts.iter().enumerate() {
            if i == 0 { p.move_to(*x, *y) } else { p.line_to(*x, *y) }
        }
        p
    }

    /// Append another path's commands.
    pub fn extend(&mut self, other: &PathData) {
        self.cmds.extend_from_slice(&other.cmds);
    }

    /// Apply an affine `[a,b,c,d,e,f]` to every point.
    pub fn transformed(&self, m: &[f64; 6]) -> PathData {
        let t = |x: f64, y: f64| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        PathData {
            cmds: self
                .cmds
                .iter()
                .map(|c| match *c {
                    Cmd::Move(x, y) => {
                        let (x, y) = t(x, y);
                        Cmd::Move(x, y)
                    }
                    Cmd::Line(x, y) => {
                        let (x, y) = t(x, y);
                        Cmd::Line(x, y)
                    }
                    Cmd::Quad(x1, y1, x, y) => {
                        let (x1, y1) = t(x1, y1);
                        let (x, y) = t(x, y);
                        Cmd::Quad(x1, y1, x, y)
                    }
                    Cmd::Cubic(x1, y1, x2, y2, x, y) => {
                        let (x1, y1) = t(x1, y1);
                        let (x2, y2) = t(x2, y2);
                        let (x, y) = t(x, y);
                        Cmd::Cubic(x1, y1, x2, y2, x, y)
                    }
                    Cmd::Close => Cmd::Close,
                })
                .collect(),
        }
    }

    /// Bounds of all points (control points included): `(x0, y0, x1, y1)`.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut b: Option<(f64, f64, f64, f64)> = None;
        let mut add = |x: f64, y: f64| {
            b = Some(match b {
                None => (x, y, x, y),
                Some((a, c, d, e)) => (a.min(x), c.min(y), d.max(x), e.max(y)),
            });
        };
        for c in &self.cmds {
            match *c {
                Cmd::Move(x, y) | Cmd::Line(x, y) => add(x, y),
                Cmd::Quad(x1, y1, x, y) => {
                    add(x1, y1);
                    add(x, y);
                }
                Cmd::Cubic(x1, y1, x2, y2, x, y) => {
                    add(x1, y1);
                    add(x2, y2);
                    add(x, y);
                }
                Cmd::Close => {}
            }
        }
        b
    }

    /// The path for tiny-skia (`None` when it has no drawable segment).
    pub fn to_skia(&self) -> Option<tiny_skia::Path> {
        let mut pb = tiny_skia::PathBuilder::with_capacity(self.cmds.len(), self.cmds.len() * 3);
        for c in &self.cmds {
            match *c {
                Cmd::Move(x, y) => pb.move_to(x as f32, y as f32),
                Cmd::Line(x, y) => pb.line_to(x as f32, y as f32),
                Cmd::Quad(x1, y1, x, y) => pb.quad_to(x1 as f32, y1 as f32, x as f32, y as f32),
                Cmd::Cubic(x1, y1, x2, y2, x, y) => pb.cubic_to(x1 as f32, y1 as f32, x2 as f32, y2 as f32, x as f32, y as f32),
                Cmd::Close => pb.close(),
            }
        }
        pb.finish()
    }

    /// SVG path data (absolute commands).
    pub fn to_svg(&self, fmt: impl Fn(f64) -> String) -> String {
        let mut s = String::new();
        for c in &self.cmds {
            if !s.is_empty() {
                s.push(' ');
            }
            match *c {
                Cmd::Move(x, y) => s.push_str(&format!("M{} {}", fmt(x), fmt(y))),
                Cmd::Line(x, y) => s.push_str(&format!("L{} {}", fmt(x), fmt(y))),
                Cmd::Quad(x1, y1, x, y) => s.push_str(&format!("Q{} {} {} {}", fmt(x1), fmt(y1), fmt(x), fmt(y))),
                Cmd::Cubic(x1, y1, x2, y2, x, y) => s.push_str(&format!("C{} {} {} {} {} {}", fmt(x1), fmt(y1), fmt(x2), fmt(y2), fmt(x), fmt(y))),
                Cmd::Close => s.push('Z'),
            }
        }
        s
    }
}

// ------------------------------------------------------------------ parser (SkParsePath)

fn is_sep(c: u8) -> bool {
    c.is_ascii_whitespace() || c == b','
}

struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn skip_sep(&mut self) {
        while self.i < self.b.len() && is_sep(self.b[self.i]) {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    /// `SkParse::FindScalar`: optional whitespace, sign, digits, fraction, exponent.
    fn scalar(&mut self) -> Option<f64> {
        self.skip_ws();
        let start = self.i;
        let b = self.b;
        let mut j = self.i;
        if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
            j += 1;
        }
        let ds = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        let mut digits = j - ds;
        if j < b.len() && b[j] == b'.' {
            j += 1;
            let fs = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            digits += j - fs;
        }
        if digits == 0 {
            return None;
        }
        if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
            let mut k = j + 1;
            if k < b.len() && (b[k] == b'-' || b[k] == b'+') {
                k += 1;
            }
            let es = k;
            while k < b.len() && b[k].is_ascii_digit() {
                k += 1;
            }
            if k > es {
                j = k;
            }
        }
        let s = std::str::from_utf8(&b[start..j]).ok()?;
        let v: f64 = s.parse().ok()?;
        self.i = j;
        // Skia parses into an f32.
        Some(f64::from(v as f32))
    }

    fn scalar_rel(&mut self, relative: bool, base: f64) -> Option<f64> {
        let v = self.scalar()?;
        Some(if relative { v + base } else { v })
    }

    /// `find_points`: `n` points, each relative to `c` when `relative`.
    fn points(&mut self, n: usize, relative: bool, c: (f64, f64)) -> Option<Vec<(f64, f64)>> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let x = self.scalar_rel(relative, c.0)?;
            self.skip_sep();
            let y = self.scalar_rel(relative, c.1)?;
            self.skip_sep();
            out.push((x, y));
        }
        Some(out)
    }

    fn flag(&mut self) -> Option<bool> {
        self.skip_ws();
        let v = match self.peek()? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.i += 1;
        Some(v)
    }
}

/// Parse SVG path data like Skia. `None` when the data is malformed (CanvasKit drew nothing then).
pub fn parse(d: &str) -> Option<PathData> {
    let mut cur = Cursor { b: d.as_bytes(), i: 0 };
    let mut p = PathData::new();
    let mut first = (0.0, 0.0);
    let mut c = (0.0, 0.0);
    let mut lastc = (0.0, 0.0);
    let mut op = 0u8;
    let mut prev = 0u8;
    let mut relative = false;
    // Skia's SkPath: drawing after a close (or before any move) starts at the last move-to point.
    let mut last_move = (0.0, 0.0);
    let mut need_move = true;
    let ensure_move = |p: &mut PathData, need_move: &mut bool, last_move: (f64, f64)| {
        if *need_move {
            p.move_to(last_move.0, last_move.1);
            *need_move = false;
        }
    };
    loop {
        cur.skip_ws();
        let Some(ch) = cur.peek() else { break };
        if ch.is_ascii_digit() || ch == b'-' || ch == b'+' || ch == b'.' {
            if op == 0 || op == b'Z' {
                return None;
            }
        } else if is_sep(ch) {
            cur.skip_sep();
        } else {
            op = ch;
            relative = false;
            if ch.is_ascii_lowercase() {
                op = ch.to_ascii_uppercase();
                relative = true;
            }
            cur.i += 1;
            cur.skip_sep();
        }
        match op {
            b'M' => {
                let pts = cur.points(1, relative, c)?;
                p.move_to(pts[0].0, pts[0].1);
                last_move = pts[0];
                need_move = false;
                prev = 0;
                op = b'L';
                c = pts[0];
            }
            b'L' => {
                let pts = cur.points(1, relative, c)?;
                ensure_move(&mut p, &mut need_move, last_move);
                p.line_to(pts[0].0, pts[0].1);
                c = pts[0];
            }
            b'H' => {
                let x = cur.scalar_rel(relative, c.0)?;
                cur.skip_sep();
                ensure_move(&mut p, &mut need_move, last_move);
                p.line_to(x, c.1);
                c.0 = x;
            }
            b'V' => {
                let y = cur.scalar_rel(relative, c.1)?;
                cur.skip_sep();
                ensure_move(&mut p, &mut need_move, last_move);
                p.line_to(c.0, y);
                c.1 = y;
            }
            b'C' | b'S' => {
                let (p0, p1, p2) = if op == b'C' {
                    let pts = cur.points(3, relative, c)?;
                    (pts[0], pts[1], pts[2])
                } else {
                    let pts = cur.points(2, relative, c)?;
                    let mut p0 = c;
                    if prev == b'C' || prev == b'S' {
                        p0.0 -= lastc.0 - c.0;
                        p0.1 -= lastc.1 - c.1;
                    }
                    (p0, pts[0], pts[1])
                };
                ensure_move(&mut p, &mut need_move, last_move);
                p.cubic_to(p0.0, p0.1, p1.0, p1.1, p2.0, p2.1);
                lastc = p1;
                c = p2;
            }
            b'Q' | b'T' => {
                let (p0, p1) = if op == b'Q' {
                    let pts = cur.points(2, relative, c)?;
                    (pts[0], pts[1])
                } else {
                    let pts = cur.points(1, relative, c)?;
                    let mut p0 = c;
                    if prev == b'Q' || prev == b'T' {
                        p0.0 -= lastc.0 - c.0;
                        p0.1 -= lastc.1 - c.1;
                    }
                    (p0, pts[0])
                };
                ensure_move(&mut p, &mut need_move, last_move);
                p.quad_to(p0.0, p0.1, p1.0, p1.1);
                lastc = p0;
                c = p1;
            }
            b'A' => {
                let radii = cur.points(1, false, (0.0, 0.0))?[0];
                cur.skip_sep();
                let angle = cur.scalar()?;
                cur.skip_sep();
                let large = cur.flag()?;
                cur.skip_sep();
                let sweep = cur.flag()?;
                cur.skip_sep();
                let end = cur.points(1, relative, c)?[0];
                ensure_move(&mut p, &mut need_move, last_move);
                arc_to(&mut p, c, radii, angle, large, sweep, end);
                c = end;
            }
            b'Z' => {
                p.close();
                need_move = true;
                c = first;
            }
            _ => return None,
        }
        if prev == 0 {
            first = c;
        }
        prev = op;
    }
    Some(p)
}

/// SVG elliptical arc from `from` to `to` as cubics (`SkPath::arcTo` semantics for degenerate radii).
fn arc_to(p: &mut PathData, from: (f64, f64), radii: (f64, f64), angle: f64, large: bool, sweep: bool, to: (f64, f64)) {
    let (x1, y1) = from;
    let (x2, y2) = to;
    if x1 == x2 && y1 == y2 {
        // Skia: an arc to the current point draws nothing (but the point is still "drawn to").
        p.line_to(x2, y2);
        return;
    }
    let (mut rx, mut ry) = (radii.0.abs(), radii.1.abs());
    if rx == 0.0 || ry == 0.0 {
        p.line_to(x2, y2);
        return;
    }
    let phi = angle.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let dx2 = (x1 - x2) / 2.0;
    let dy2 = (y1 - y2) / 2.0;
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den == 0.0 { 0.0 } else { (num / den).max(0.0).sqrt() };
    if large == sweep {
        coef = -coef;
    }
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * -(ry * x1p / rx);
    let cx = cos_phi * cxp - sin_phi * cyp + (x1 + x2) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (y1 + y2) / 2.0;
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let mut a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = ang(1.0, 0.0, ux, uy);
    let mut dtheta = ang(ux, uy, vx, vy);
    if !sweep && dtheta > 0.0 {
        dtheta -= std::f64::consts::TAU;
    } else if sweep && dtheta < 0.0 {
        dtheta += std::f64::consts::TAU;
    }
    let segs = (dtheta.abs() / std::f64::consts::FRAC_PI_2 - 1e-9).ceil().max(1.0) as usize;
    let delta = dtheta / segs as f64;
    let t = 4.0 / 3.0 * (delta / 4.0).tan();
    let point = |th: f64| {
        let (s, c) = th.sin_cos();
        (cx + rx * c * cos_phi - ry * s * sin_phi, cy + rx * c * sin_phi + ry * s * cos_phi)
    };
    let deriv = |th: f64| {
        let (s, c) = th.sin_cos();
        (-rx * s * cos_phi - ry * c * sin_phi, -rx * s * sin_phi + ry * c * cos_phi)
    };
    let mut th = theta1;
    for i in 0..segs {
        let th2 = th + delta;
        let (px, py) = point(th);
        let (dx1, dy1) = deriv(th);
        let (qx, qy) = if i + 1 == segs { (x2, y2) } else { point(th2) };
        let (dx2, dy2) = deriv(th2);
        p.cubic_to(px + t * dx1, py + t * dy1, qx - t * dx2, qy - t * dy2, qx, qy);
        th = th2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skia_semantics() {
        assert_eq!(parse("not a path"), None);
        assert_eq!(parse(""), Some(PathData::new()));
        assert_eq!(parse("M0 0 1 1").unwrap().cmds, vec![Cmd::Move(0.0, 0.0), Cmd::Line(1.0, 1.0)]);
        assert_eq!(parse("m1 1 2 2").unwrap().cmds, vec![Cmd::Move(1.0, 1.0), Cmd::Line(3.0, 3.0)]);
        assert_eq!(parse("M1-2.5.5.5").unwrap().cmds, vec![Cmd::Move(1.0, -2.5), Cmd::Line(0.5, 0.5)]);
        assert_eq!(parse("M1-2.5.5"), None);
        assert_eq!(parse("M0 0 L10 0 Z L5 5").unwrap().cmds, vec![Cmd::Move(0.0, 0.0), Cmd::Line(10.0, 0.0), Cmd::Close, Cmd::Move(0.0, 0.0), Cmd::Line(5.0, 5.0)]);
        assert_eq!(parse("M0 0 Z 5 5"), None);
        assert_eq!(parse("M0 0h10v5H0z").unwrap().cmds.len(), 5);
        // Arc flags without separators; a half circle is two cubics ending exactly at the end point.
        let a = parse("M0 0a5 5 0 1010 0").unwrap();
        assert_eq!(a.cmds.len(), 3);
        assert_eq!(a.cmds[2], match a.cmds[2] { Cmd::Cubic(x1, y1, x2, y2, _, _) => Cmd::Cubic(x1, y1, x2, y2, 10.0, 0.0), c => c });
    }
}
