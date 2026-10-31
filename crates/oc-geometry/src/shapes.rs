// SPDX-License-Identifier: Apache-2.0
//! Parametric shape outlines as SVG path data in the element's box space (0..w, 0..h).
//! Shared by the renderer, hit-testing and exporters, so every surface agrees.
//!
//! Strings are byte-identical to the TS output: coordinates print as `${+n.toFixed(3)}`.

use std::f64::consts::PI;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;

use crate::jsmath::{cos, f3, max, min, num_to_string, round, sin, tan};

/// Shape parameters (`cornerRadius`, `sides`, `points`, `innerRatio`, …), as stored on `ShapeEl.params`.
pub type Params = IndexMap<String, f64>;

const TAU: f64 = PI * 2.0;
const KAPPA: f64 = 0.5522847498;

fn poly(pts: &[[f64; 2]]) -> String {
    let body: Vec<String> =
        pts.iter().enumerate().map(|(i, [x, y])| format!("{}{} {}", if i > 0 { 'L' } else { 'M' }, f3(*x), f3(*y))).collect();
    format!("{} Z", body.join(" "))
}

fn rounded_rect(w: f64, h: f64, r: f64) -> String {
    let rr = max(0.0, min(min(r, w / 2.0), h / 2.0));
    if rr == 0.0 {
        return poly(&[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]);
    }
    let k = KAPPA * rr;
    [
        format!("M{} 0", f3(rr)),
        format!("L{} 0", f3(w - rr)),
        format!("C{} 0 {} {} {} {}", f3(w - rr + k), f3(w), f3(rr - k), f3(w), f3(rr)),
        format!("L{} {}", f3(w), f3(h - rr)),
        format!("C{} {} {} {} {} {}", f3(w), f3(h - rr + k), f3(w - rr + k), f3(h), f3(w - rr), f3(h)),
        format!("L{} {}", f3(rr), f3(h)),
        format!("C{} {} 0 {} 0 {}", f3(rr - k), f3(h), f3(h - rr + k), f3(h - rr)),
        format!("L0 {}", f3(rr)),
        format!("C0 {} {} 0 {} 0", f3(rr - k), f3(rr - k), f3(rr)),
        "Z".into(),
    ]
    .join(" ")
}

fn ellipse(w: f64, h: f64) -> String {
    let rx = w / 2.0;
    let ry = h / 2.0;
    let kx = KAPPA * rx;
    let ky = KAPPA * ry;
    [
        format!("M{} 0", f3(rx)),
        format!("C{} 0 {} {} {} {}", f3(rx + kx), f3(w), f3(ry - ky), f3(w), f3(ry)),
        format!("C{} {} {} {} {} {}", f3(w), f3(ry + ky), f3(rx + kx), f3(h), f3(rx), f3(h)),
        format!("C{} {} 0 {} 0 {}", f3(rx - kx), f3(h), f3(ry + ky), f3(ry)),
        format!("C0 {} {} 0 {} 0", f3(ry - ky), f3(rx - kx), f3(rx)),
        "Z".into(),
    ]
    .join(" ")
}

fn ellipse_at(x: f64, y: f64, w: f64, h: f64, reverse: bool) -> String {
    let rx = w / 2.0;
    let ry = h / 2.0;
    let cx = x + rx;
    let cy = y + ry;
    let kx = KAPPA * rx;
    let ky = KAPPA * ry;
    if !reverse {
        return format!(
            "M{} {} C{} {} {} {} {} {} C{} {} {} {} {} {} C{} {} {} {} {} {} C{} {} {} {} {} {} Z",
            f3(cx), f3(y),
            f3(cx + kx), f3(y), f3(x + w), f3(cy - ky), f3(x + w), f3(cy),
            f3(x + w), f3(cy + ky), f3(cx + kx), f3(y + h), f3(cx), f3(y + h),
            f3(cx - kx), f3(y + h), f3(x), f3(cy + ky), f3(x), f3(cy),
            f3(x), f3(cy - ky), f3(cx - kx), f3(y), f3(cx), f3(y),
        );
    }
    format!(
        "M{} {} C{} {} {} {} {} {} C{} {} {} {} {} {} C{} {} {} {} {} {} C{} {} {} {} {} {} Z",
        f3(cx), f3(y),
        f3(cx - kx), f3(y), f3(x), f3(cy - ky), f3(x), f3(cy),
        f3(x), f3(cy + ky), f3(cx - kx), f3(y + h), f3(cx), f3(y + h),
        f3(cx + kx), f3(y + h), f3(x + w), f3(cy + ky), f3(x + w), f3(cy),
        f3(x + w), f3(cy - ky), f3(cx + kx), f3(y), f3(cx), f3(y),
    )
}

fn regular(w: f64, h: f64, sides: f64, inner: Option<f64>) -> String {
    let n = if inner.is_none() { sides } else { sides * 2.0 };
    let mut pts = Vec::new();
    let mut i = 0.0;
    while i < n {
        let a = -PI / 2.0 + (i * 2.0 * PI) / n;
        let r = match inner {
            Some(v) if i % 2.0 == 1.0 => v,
            _ => 1.0,
        };
        pts.push([w / 2.0 + (cos(a) * r * w) / 2.0, h / 2.0 + (sin(a) * r * h) / 2.0]);
        i += 1.0;
    }
    poly(&pts)
}

fn heart(w: f64, h: f64) -> String {
    [
        format!("M{} {}", f3(w / 2.0), f3(h * 0.28)),
        format!("C{} {} {} {} {} {}", f3(w * 0.5), f3(h * 0.1), f3(w * 0.05), f3(-h * 0.02), f3(0.0), f3(h * 0.3)),
        format!("C{} {} {} {} {} {}", f3(-w * 0.02), f3(h * 0.6), f3(w * 0.35), f3(h * 0.75), f3(w / 2.0), f3(h)),
        format!("C{} {} {} {} {} {}", f3(w * 0.65), f3(h * 0.75), f3(w * 1.02), f3(h * 0.6), f3(w), f3(h * 0.3)),
        format!("C{} {} {} {} {} {}", f3(w * 0.95), f3(-h * 0.02), f3(w * 0.5), f3(h * 0.1), f3(w / 2.0), f3(h * 0.28)),
        "Z".into(),
    ]
    .join(" ")
}

/// Path command with absolute coordinates (arcs become cubics).
#[derive(Debug, Clone, Copy)]
enum Cmd {
    M(f64, f64),
    L(f64, f64),
    C(f64, f64, f64, f64, f64, f64),
    Z,
}

/// Path builder with absolute M/L/C/Z only. Coordinates stay numeric so radial shapes can be drawn in any unit
/// space and then `fit` to the box using their true curve extents.
#[derive(Debug, Default, Clone)]
struct Pb {
    cmds: Vec<Cmd>,
    cx: f64,
    cy: f64,
}

impl Pb {
    fn new() -> Self {
        Self::default()
    }
    fn m(mut self, x: f64, y: f64) -> Self {
        self.cmds.push(Cmd::M(x, y));
        self.cx = x;
        self.cy = y;
        self
    }
    fn l(mut self, x: f64, y: f64) -> Self {
        self.cmds.push(Cmd::L(x, y));
        self.cx = x;
        self.cy = y;
        self
    }
    fn lp(self, p: [f64; 2]) -> Self {
        self.l(p[0], p[1])
    }
    fn c(mut self, a: f64, b: f64, c: f64, d: f64, x: f64, y: f64) -> Self {
        self.cmds.push(Cmd::C(a, b, c, d, x, y));
        self.cx = x;
        self.cy = y;
        self
    }
    /// Quadratic, emitted as the equivalent cubic.
    fn q(self, qx: f64, qy: f64, x: f64, y: f64) -> Self {
        let (cx, cy) = (self.cx, self.cy);
        self.c(
            cx + (2.0 / 3.0) * (qx - cx),
            cy + (2.0 / 3.0) * (qy - cy),
            x + (2.0 / 3.0) * (qx - x),
            y + (2.0 / 3.0) * (qy - y),
            x,
            y,
        )
    }
    /// Elliptical arc around (cx, cy) from angle a0 to a1 (radians, y down). Lines to (or, with `mv`, starts at)
    /// the arc start.
    #[allow(clippy::too_many_arguments)]
    fn a(mut self, cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, a1: f64, mv: bool) -> Self {
        let sx = cx + cos(a0) * rx;
        let sy = cy + sin(a0) * ry;
        if mv || self.cmds.is_empty() {
            self = self.m(sx, sy);
        } else if (sx - self.cx).abs() > 1e-9 || (sy - self.cy).abs() > 1e-9 {
            self = self.l(sx, sy);
        }
        let n = max(1.0, ((a1 - a0).abs() / (PI / 2.0) - 1e-9).ceil());
        let da = (a1 - a0) / n;
        let k = (4.0 / 3.0) * tan(da / 4.0);
        let mut i = 0.0;
        while i < n {
            let t0 = a0 + i * da;
            let t1 = t0 + da;
            let (c0, s0, c1, s1) = (cos(t0), sin(t0), cos(t1), sin(t1));
            self = self.c(
                cx + rx * (c0 - k * s0),
                cy + ry * (s0 + k * c0),
                cx + rx * (c1 + k * s1),
                cy + ry * (s1 - k * c1),
                cx + rx * c1,
                cy + ry * s1,
            );
            i += 1.0;
        }
        self
    }
    fn poly(mut self, pts: &[[f64; 2]]) -> Self {
        for (i, &[x, y]) in pts.iter().enumerate() {
            self = if i > 0 { self.l(x, y) } else { self.m(x, y) };
        }
        self.z()
    }
    fn z(mut self) -> Self {
        self.cmds.push(Cmd::Z);
        self
    }
    /// Scale + translate so the sampled outline exactly fills (x, y, w, h).
    fn fit(mut self, w: f64, h: f64, x: f64, y: f64) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut add = |px: f64, py: f64| {
            x0 = min(x0, px);
            y0 = min(y0, py);
            x1 = max(x1, px);
            y1 = max(y1, py);
        };
        let (mut px, mut py) = (0.0, 0.0);
        for c in &self.cmds {
            match *c {
                Cmd::M(a, b) | Cmd::L(a, b) => {
                    add(a, b);
                    px = a;
                    py = b;
                }
                Cmd::C(c1, c2, c3, c4, c5, c6) => {
                    for i in 1..=16 {
                        let t = f64::from(i) / 16.0;
                        let u = 1.0 - t;
                        add(
                            u * u * u * px + 3.0 * u * u * t * c1 + 3.0 * u * t * t * c3 + t * t * t * c5,
                            u * u * u * py + 3.0 * u * u * t * c2 + 3.0 * u * t * t * c4 + t * t * t * c6,
                        );
                    }
                    px = c5;
                    py = c6;
                }
                Cmd::Z => {}
            }
        }
        let or1 = |v: f64| if v != 0.0 && !v.is_nan() { v } else { 1.0 };
        let sx = w / or1(x1 - x0);
        let sy = h / or1(y1 - y0);
        let fx = |v: f64| x + (v - x0) * sx;
        let fy = |v: f64| y + (v - y0) * sy;
        for c in &mut self.cmds {
            *c = match *c {
                Cmd::Z => Cmd::Z,
                Cmd::C(a, b, cc, d, e, f) => Cmd::C(fx(a), fy(b), fx(cc), fy(d), fx(e), fy(f)),
                Cmd::M(a, b) => Cmd::M(fx(a), fy(b)),
                Cmd::L(a, b) => Cmd::L(fx(a), fy(b)),
            };
        }
        self
    }
    fn fit_box(self, w: f64, h: f64) -> Self {
        self.fit(w, h, 0.0, 0.0)
    }
}

impl std::fmt::Display for Pb {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, c) in self.cmds.iter().enumerate() {
            if i > 0 {
                out.write_str(" ")?;
            }
            match *c {
                Cmd::M(x, y) => write!(out, "M{} {}", f3(x), f3(y))?,
                Cmd::L(x, y) => write!(out, "L{} {}", f3(x), f3(y))?,
                Cmd::C(a, b, c, d, x, y) => write!(out, "C{} {} {} {} {} {}", f3(a), f3(b), f3(c), f3(d), f3(x), f3(y))?,
                Cmd::Z => out.write_str("Z")?,
            }
        }
        Ok(())
    }
}

fn polar(r: f64, a: f64) -> [f64; 2] {
    [cos(a) * r, sin(a) * r]
}

/// Circle with `n` outward bumps (badge, thought bubble), in unit space; caller fits it.
fn scallops(mut pb: Pb, n: f64, rin: f64) -> Pb {
    let da = TAU / n;
    let mut i = 0.0;
    while i < n {
        let a = -PI / 2.0 + i * da;
        let [x0, y0] = polar(rin, a);
        let [x1, y1] = polar(rin, a + da);
        let rc = 2.0 - rin * cos(da / 2.0);
        let [qx, qy] = polar(rc, a + da / 2.0);
        if i == 0.0 {
            pb = pb.m(x0, y0);
        }
        pb = pb.q(qx, qy, x1, y1);
        i += 1.0;
    }
    pb.z()
}

static UNIT_NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)-?[0-9]*\.?[0-9]+(?:e-?[0-9]+)?").expect("regex"));

/// Scale a unit-box path (absolute M/L/C/Q/Z, coordinates in 0..1) to w × h.
pub fn scale_unit_path(d: &str, w: f64, h: f64) -> String {
    let mut i = 0usize;
    UNIT_NUM
        .replace_all(d, |c: &regex::Captures<'_>| {
            let v = oc_schema::js::string_to_number(&c[0]) * if i.is_multiple_of(2) { w } else { h };
            i += 1;
            num_to_string(round(v * 100.0) / 100.0)
        })
        .into_owned()
}

fn param(params: &Params, key: &str, default: f64) -> f64 {
    params.get(key).copied().unwrap_or(default)
}

/// Outline of a shape kind (or frame clip) as SVG path data in box space.
///
/// `kind` is a `ShapeKind` / `FrameClip` string. Unknown kinds yield an empty string (the TS returns
/// `undefined`). `d` is the unit-box path of `custom` shapes.
pub fn shape_path(kind: &str, w: f64, h: f64, params: &Params, d: Option<&str>) -> String {
    let p = |k: &str, def: f64| param(params, k, def);
    let mmin = min(w, h);
    match kind {
        "custom" => match d {
            Some(d) if !d.is_empty() => scale_unit_path(d, w, h),
            _ => format!("M0 0 L{w} 0 L{w} {h} L0 {h} Z", w = num_to_string(w), h = num_to_string(h)),
        },
        "rect" => rounded_rect(w, h, p("cornerRadius", 0.0)),
        "rounded" => rounded_rect(w, h, mmin * 0.15),
        "ellipse" => ellipse(w, h),
        "triangle" => poly(&[[w / 2.0, 0.0], [w, h], [0.0, h]]),
        "diamond" => poly(&[[w / 2.0, 0.0], [w, h / 2.0], [w / 2.0, h], [0.0, h / 2.0]]),
        "hexagon" => poly(&[[w * 0.25, 0.0], [w * 0.75, 0.0], [w, h / 2.0], [w * 0.75, h], [w * 0.25, h], [0.0, h / 2.0]]),
        "polygon" => regular(w, h, max(3.0, round(p("sides", 5.0))), None),
        "star" => regular(w, h, max(3.0, round(p("points", 5.0))), Some(p("innerRatio", 0.45))),
        "heart" => heart(w, h),
        "arrow" => {
            let t = p("shaft", 0.4);
            let hd = p("head", 0.4);
            let y0 = (h * (1.0 - t)) / 2.0;
            let y1 = h - y0;
            let x = w * (1.0 - hd);
            poly(&[[0.0, y0], [x, y0], [x, 0.0], [w, h / 2.0], [x, h], [x, y1], [0.0, y1]])
        }
        "chevron" => {
            let d = w * p("depth", 0.3);
            poly(&[[0.0, 0.0], [w - d, 0.0], [w, h / 2.0], [w - d, h], [0.0, h], [d, h / 2.0]])
        }
        "cross" | "plus" => {
            let t = p("thickness", if kind == "cross" { 0.33 } else { 0.22 });
            let ax = (w * (1.0 - t)) / 2.0;
            let ay = (h * (1.0 - t)) / 2.0;
            poly(&[
                [ax, 0.0], [w - ax, 0.0], [w - ax, ay], [w, ay], [w, h - ay], [w - ax, h - ay], [w - ax, h], [ax, h],
                [ax, h - ay], [0.0, h - ay], [0.0, ay], [ax, ay],
            ])
        }
        "pentagon" => regular(w, h, 5.0, None),
        "octagon" => {
            let a = w * 0.2929;
            let b = h * 0.2929;
            poly(&[[a, 0.0], [w - a, 0.0], [w, b], [w, h - b], [w - a, h], [a, h], [0.0, h - b], [0.0, b]])
        }
        "right-triangle" => poly(&[[0.0, 0.0], [w, h], [0.0, h]]),
        "parallelogram" => {
            let k = w * p("skew", 0.25);
            poly(&[[k, 0.0], [w, 0.0], [w - k, h], [0.0, h]])
        }
        "trapezoid" => {
            let k = w * p("inset", 0.2);
            poly(&[[k, 0.0], [w - k, 0.0], [w, h], [0.0, h]])
        }
        "double-arrow" => {
            let t = p("shaft", 0.4);
            let hd = p("head", 0.25) * w;
            let y0 = (h * (1.0 - t)) / 2.0;
            let y1 = h - y0;
            poly(&[
                [0.0, h / 2.0], [hd, 0.0], [hd, y0], [w - hd, y0], [w - hd, 0.0], [w, h / 2.0], [w - hd, h],
                [w - hd, y1], [hd, y1], [hd, h],
            ])
        }
        "lightning" => poly(&[
            [w * 0.55, 0.0], [w * 0.1, h * 0.58], [w * 0.45, h * 0.58], [w * 0.3, h], [w * 0.9, h * 0.38],
            [w * 0.55, h * 0.38], [w * 0.75, 0.0],
        ]),
        "burst" => regular(w, h, max(6.0, round(p("points", 16.0))), Some(p("innerRatio", 0.72))),
        "seal" => {
            let n = max(8.0, round(p("points", 24.0)));
            let mut pts = Vec::new();
            let mut i = 0.0;
            while i < n * 2.0 {
                let a = -PI / 2.0 + (i * PI) / n;
                let r = if i % 2.0 != 0.0 { 0.9 } else { 1.0 };
                pts.push([w / 2.0 + (cos(a) * r * w) / 2.0, h / 2.0 + (sin(a) * r * h) / 2.0]);
                i += 1.0;
            }
            poly(&pts)
        }
        "tag" => poly(&[[0.0, h * 0.5], [w * 0.22, 0.0], [w, 0.0], [w, h], [w * 0.22, h]]),
        "flag" => poly(&[[0.0, 0.0], [w, 0.0], [w * 0.78, h / 2.0], [w, h], [0.0, h]]),
        "ribbon" => poly(&[[0.0, 0.0], [w, 0.0], [w * 0.88, h / 2.0], [w, h], [0.0, h], [w * 0.12, h / 2.0]]),
        "ring" => {
            let t = p("thickness", 0.28);
            let ix = (w * t) / 2.0;
            let iy = (h * t) / 2.0;
            format!("{} {}", ellipse(w, h), ellipse_at(ix, iy, w - ix * 2.0, h - iy * 2.0, true))
        }
        "half-circle" => {
            let rx = w / 2.0;
            let k = KAPPA;
            [
                format!("M0 {}", f3(h)),
                format!("C0 {} {} 0 {} 0", f3(h - h * k), f3(rx - rx * k), f3(rx)),
                format!("C{} 0 {} {} {} {}", f3(rx + rx * k), f3(w), f3(h - h * k), f3(w), f3(h)),
                "Z".into(),
            ]
            .join(" ")
        }
        "arch" => {
            let rx = w / 2.0;
            let ry = min(h, w / 2.0);
            let k = KAPPA;
            [
                format!("M0 {}", f3(h)),
                format!("L0 {}", f3(ry)),
                format!("C0 {} {} 0 {} 0", f3(ry - ry * k), f3(rx - rx * k), f3(rx)),
                format!("C{} 0 {} {} {} {}", f3(rx + rx * k), f3(w), f3(ry - ry * k), f3(w), f3(ry)),
                format!("L{} {}", f3(w), f3(h)),
                "Z".into(),
            ]
            .join(" ")
        }
        "moon" => [
            format!("M{} 0", f3(w * 0.62)),
            format!("C{} {} {} {} {} {}", f3(w * 0.05), f3(h * 0.02), f3(-w * 0.1), f3(h * 0.85), f3(w * 0.55), f3(h)),
            format!("C{} {} {} {} {} {}", f3(w * 0.85), f3(h), f3(w), f3(h * 0.9), f3(w), f3(h * 0.82)),
            format!("C{} {} {} {} {} 0", f3(w * 0.45), f3(h * 0.9), f3(w * 0.25), f3(h * 0.25), f3(w * 0.62)),
            "Z".into(),
        ]
        .join(" "),
        "drop" => [
            format!("M{} 0", f3(w / 2.0)),
            format!("C{} {} {} {} {} {}", f3(w * 0.7), f3(h * 0.3), f3(w), f3(h * 0.5), f3(w), f3(h * 0.68)),
            format!("C{} {} {} {} {} {}", f3(w), f3(h * 0.88), f3(w * 0.78), f3(h), f3(w / 2.0), f3(h)),
            format!("C{} {} 0 {} 0 {}", f3(w * 0.22), f3(h), f3(h * 0.88), f3(h * 0.68)),
            format!("C0 {} {} {} {} 0", f3(h * 0.5), f3(w * 0.3), f3(h * 0.3), f3(w / 2.0)),
            "Z".into(),
        ]
        .join(" "),
        "cloud" => [
            format!("M{} {}", f3(w * 0.25), f3(h * 0.85)),
            format!("C{} {} {} {} {} {}", f3(w * 0.05), f3(h * 0.85), f3(0.0), f3(h * 0.6), f3(w * 0.15), f3(h * 0.5)),
            format!("C{} {} {} {} {} {}", f3(w * 0.08), f3(h * 0.25), f3(w * 0.35), f3(h * 0.1), f3(w * 0.48), f3(h * 0.25)),
            format!("C{} {} {} {} {} {}", f3(w * 0.58), f3(h * 0.02), f3(w * 0.88), f3(h * 0.1), f3(w * 0.83), f3(h * 0.4)),
            format!("C{} {} {} {} {} {}", f3(w), f3(h * 0.45), f3(w), f3(h * 0.85), f3(w * 0.78), f3(h * 0.85)),
            "Z".into(),
        ]
        .join(" "),
        "blob" => [
            format!("M{} {}", f3(w * 0.5), f3(h * 0.02)),
            format!("C{} {} {} {} {} {}", f3(w * 0.82), f3(h * 0.0), f3(w * 1.0), f3(h * 0.3), f3(w * 0.95), f3(h * 0.55)),
            format!("C{} {} {} {} {} {}", f3(w * 0.9), f3(h * 0.85), f3(w * 0.6), f3(h * 1.0), f3(w * 0.38), f3(h * 0.95)),
            format!("C{} {} {} {} {} {}", f3(w * 0.1), f3(h * 0.9), f3(0.0), f3(h * 0.6), f3(w * 0.05), f3(h * 0.38)),
            format!("C{} {} {} {} {} {}", f3(w * 0.1), f3(h * 0.12), f3(w * 0.25), f3(h * 0.04), f3(w * 0.5), f3(h * 0.02)),
            "Z".into(),
        ]
        .join(" "),
        "squircle" => {
            let k = 0.92;
            let (cx, cy) = (w / 2.0, h / 2.0);
            [
                format!("M{} 0", f3(cx)),
                format!("C{} 0 {} {} {} {}", f3(cx + cx * k), f3(w), f3(cy - cy * k), f3(w), f3(cy)),
                format!("C{} {} {} {} {} {}", f3(w), f3(cy + cy * k), f3(cx + cx * k), f3(h), f3(cx), f3(h)),
                format!("C{} {} 0 {} 0 {}", f3(cx - cx * k), f3(h), f3(cy + cy * k), f3(cy)),
                format!("C0 {} {} 0 {} 0", f3(cy - cy * k), f3(cx - cx * k), f3(cx)),
                "Z".into(),
            ]
            .join(" ")
        }
        "pill" => rounded_rect(w, h, mmin / 2.0),
        "snip-corner" => {
            let c = mmin * p("snip", 0.25);
            poly(&[[0.0, 0.0], [w - c, 0.0], [w, c], [w, h], [0.0, h]])
        }
        "snip-two" => {
            let c = mmin * p("snip", 0.25);
            poly(&[[c, 0.0], [w - c, 0.0], [w, c], [w, h], [0.0, h], [0.0, c]])
        }
        "round-one" => {
            let r = mmin * p("radius", 0.35);
            Pb::new().m(0.0, 0.0).l(w - r, 0.0).a(w - r, r, r, r, -PI / 2.0, 0.0, false).l(w, h).l(0.0, h).z().to_string()
        }
        "folded-corner" => {
            let c = mmin * p("fold", 0.28);
            let g = mmin * 0.03;
            Pb::new()
                .poly(&[[0.0, 0.0], [w - c - g, 0.0], [w - c - g, c + g], [w, c + g], [w, h], [0.0, h]])
                .poly(&[[w - c, 0.0], [w, c], [w - c, c]])
                .to_string()
        }
        "plaque" => {
            let r = mmin * p("radius", 0.18);
            Pb::new()
                .m(r, 0.0)
                .l(w - r, 0.0)
                .a(w, 0.0, r, r, PI, PI / 2.0, false)
                .l(w, h - r)
                .a(w, h, r, r, -PI / 2.0, -PI, false)
                .l(r, h)
                .a(0.0, h, r, r, 0.0, -PI / 2.0, false)
                .l(0.0, r)
                .a(0.0, 0.0, r, r, PI / 2.0, 0.0, false)
                .z()
                .to_string()
        }
        "frame" => {
            let t = mmin * p("thickness", 0.14);
            format!(
                "{} {}",
                poly(&[[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]),
                poly(&[[t, t], [t, h - t], [w - t, h - t], [w - t, t]])
            )
        }
        "ticket" => {
            let r = min(h * 0.16, w * 0.12);
            Pb::new()
                .m(0.0, 0.0)
                .l(w, 0.0)
                .l(w, h / 2.0 - r)
                .a(w, h / 2.0, r, r, -PI / 2.0, -1.5 * PI, false)
                .l(w, h)
                .l(0.0, h)
                .l(0.0, h / 2.0 + r)
                .a(0.0, h / 2.0, r, r, PI / 2.0, -PI / 2.0, false)
                .z()
                .to_string()
        }
        "shield" => [
            format!("M{} 0", f3(w / 2.0)),
            format!("C{} {} {} {} {} {}", f3(w * 0.68), f3(h * 0.08), f3(w * 0.85), f3(h * 0.1), f3(w), f3(h * 0.1)),
            format!("L{} {}", f3(w), f3(h * 0.45)),
            format!("C{} {} {} {} {} {}", f3(w), f3(h * 0.75), f3(w * 0.72), f3(h * 0.9), f3(w / 2.0), f3(h)),
            format!("C{} {} 0 {} 0 {}", f3(w * 0.28), f3(h * 0.9), f3(h * 0.75), f3(h * 0.45)),
            format!("L0 {}", f3(h * 0.1)),
            format!("C{} {} {} {} {} 0", f3(w * 0.15), f3(h * 0.1), f3(w * 0.32), f3(h * 0.08), f3(w / 2.0)),
            "Z".into(),
        ]
        .join(" "),
        "cylinder" => {
            let ry = min(h * 0.14, w * 0.3);
            let g = mmin * 0.03;
            let body = Pb::new()
                .m(0.0, ry + g)
                .l(0.0, h - ry)
                .a(w / 2.0, h - ry, w / 2.0, ry, PI, 0.0, false)
                .l(w, ry + g)
                .a(w / 2.0, ry + g, w / 2.0, ry, 0.0, PI, false)
                .z();
            format!("{} {}", ellipse_at(0.0, 0.0, w, ry * 2.0, false), body)
        }
        "cube" => {
            let d = mmin * p("depth", 0.28);
            let g = mmin * 0.025;
            Pb::new()
                .poly(&[[d, 0.0], [w, 0.0], [w - d, d], [0.0, d]])
                .poly(&[[0.0, d + g], [w - d - g, d + g], [w - d - g, h], [0.0, h]])
                .poly(&[[w - d, d + g * 1.4], [w, g * 1.4], [w, h - d], [w - d, h]])
                .to_string()
        }
        "block-arc" => {
            let t = p("thickness", 0.3);
            let (rx, ry) = (w / 2.0, h);
            Pb::new()
                .a(rx, h, rx, ry, PI, TAU, true)
                .a(rx, h, rx * (1.0 - t), ry * (1.0 - t), TAU, PI, false)
                .z()
                .to_string()
        }
        "pie" => {
            let sweep = (p("angle", 270.0) * PI) / 180.0;
            Pb::new()
                .m(w / 2.0, h / 2.0)
                .a(w / 2.0, h / 2.0, w / 2.0, h / 2.0, 0.0, max(0.01, min(TAU - 0.001, sweep)), false)
                .z()
                .to_string()
        }
        "star-6" => regular(w, h, 6.0, Some(p("innerRatio", 0.58))),
        "star-8" => regular(w, h, 8.0, Some(p("innerRatio", 0.62))),
        "starburst-12" => regular(w, h, 12.0, Some(p("innerRatio", 0.68))),
        "sparkle" => Pb::new()
            .m(0.0, -1.0)
            .c(0.08, -0.32, 0.32, -0.08, 1.0, 0.0)
            .c(0.32, 0.08, 0.08, 0.32, 0.0, 1.0)
            .c(-0.08, 0.32, -0.32, 0.08, -1.0, 0.0)
            .c(-0.32, -0.08, -0.08, -0.32, 0.0, -1.0)
            .z()
            .fit_box(w, h)
            .to_string(),
        "badge" => scallops(Pb::new(), max(6.0, round(p("points", 14.0))), 0.9).fit_box(w, h).to_string(),
        "gear" => {
            let n = max(5.0, round(p("teeth", 9.0)));
            let step = TAU / n;
            let mut pts = Vec::new();
            let mut i = 0.0;
            while i < n {
                let a = -PI / 2.0 + i * step;
                pts.push(polar(0.76, a - step * 0.3));
                pts.push(polar(1.0, a - step * 0.16));
                pts.push(polar(1.0, a + step * 0.16));
                pts.push(polar(0.76, a + step * 0.3));
                i += 1.0;
            }
            let hole = p("hole", 0.3);
            Pb::new().poly(&pts).a(0.0, 0.0, hole, hole, 0.0, -TAU, true).z().fit_box(w, h).to_string()
        }
        "flower" => {
            let n = max(3.0, round(p("petals", 6.0)));
            let mut pb = Pb::new();
            let da = TAU / n;
            let rin = 0.3;
            let mut i = 0.0;
            while i < n {
                let a = -PI / 2.0 - da / 2.0 + i * da;
                let [x0, y0] = polar(rin, a);
                if i == 0.0 {
                    pb = pb.m(x0, y0);
                }
                let [c1x, c1y] = polar(1.32, a - da * 0.06);
                let [c2x, c2y] = polar(1.32, a + da * 1.06);
                let [x1, y1] = polar(rin, a + da);
                pb = pb.c(c1x, c1y, c2x, c2y, x1, y1);
                i += 1.0;
            }
            pb.z().fit_box(w, h).to_string()
        }
        "sun" => {
            let n = max(6.0, round(p("rays", 10.0)));
            let mut pb = Pb::new().a(0.0, 0.0, 0.56, 0.56, -PI / 2.0, 1.5 * PI, true).z();
            let mut i = 0.0;
            while i < n {
                let a = -PI / 2.0 + (i * TAU) / n;
                let b = PI / n * 0.5;
                pb = pb.poly(&[polar(0.68, a - b), polar(1.0, a), polar(0.68, a + b)]);
                i += 1.0;
            }
            pb.fit_box(w, h).to_string()
        }
        "leaf" => [
            format!("M0 {}", f3(h)),
            format!("C0 {} {} 0 {} 0", f3(h * 0.35), f3(w * 0.35), f3(w)),
            format!("C{} {} {} {} 0 {}", f3(w), f3(h * 0.65), f3(w * 0.65), f3(h), f3(h)),
            "Z".into(),
        ]
        .join(" "),
        "cross-x" => {
            let d = p("thickness", 0.2);
            let pts: [[f64; 2]; 12] = [
                [0.0, d], [d, 0.0], [0.5, 0.5 - d], [1.0 - d, 0.0], [1.0, d], [0.5 + d, 0.5], [1.0, 1.0 - d],
                [1.0 - d, 1.0], [0.5, 0.5 + d], [d, 1.0], [0.0, 1.0 - d], [0.5 - d, 0.5],
            ];
            poly(&pts.map(|[x, y]| [x * w, y * h]))
        }
        "check" => Pb::new()
            .poly(&[[0.0, 0.56], [0.15, 0.41], [0.37, 0.63], [0.85, 0.15], [1.0, 0.3], [0.37, 0.93]])
            .fit_box(w, h)
            .to_string(),
        "chevron-double" => {
            let d = w * p("depth", 0.2);
            let cw = w * 0.56;
            let one = |x: f64| [[x, 0.0], [x + cw - d, 0.0], [x + cw, h / 2.0], [x + cw - d, h], [x, h], [x + d, h / 2.0]];
            format!("{} {}", poly(&one(0.0)), poly(&one(w - cw)))
        }
        "curved-arrow" => {
            let (ro, ri, e, hl) = (1.0, 0.6, 0.24, 0.5);
            let mid = (ro + ri) / 2.0;
            Pb::new()
                .a(0.0, 0.0, ro, ro, PI, 1.5 * PI, true)
                .l(0.0, -ro - e)
                .l(hl, -mid)
                .l(0.0, -ri + e)
                .l(0.0, -ri)
                .a(0.0, 0.0, ri, ri, 1.5 * PI, PI, false)
                .z()
                .fit_box(w, h)
                .to_string()
        }
        "u-turn-arrow" => {
            let (r, t, yh, e, hl) = (0.5, 0.24, 0.6, 0.16, 0.34);
            Pb::new()
                .m(0.0, 1.1)
                .l(0.0, r)
                .a(r, r, r, r, PI, TAU, false)
                .l(2.0 * r, yh)
                .l(2.0 * r + e, yh)
                .l(2.0 * r - t / 2.0, yh + hl)
                .l(2.0 * r - t - e, yh)
                .l(2.0 * r - t, yh)
                .l(2.0 * r - t, r)
                .a(r, r, r - t, r - t, TAU, PI, false)
                .l(t, 1.1)
                .z()
                .fit_box(w, h)
                .to_string()
        }
        "circular-arrow" => {
            let (ro, ri, e) = (1.0, 0.7, 0.2);
            let a0 = -PI / 2.0 + 0.5;
            let a1 = a0 + TAU * 0.76;
            let [bx, by] = polar((ro + ri) / 2.0, a1);
            let (mx, my) = (bx - sin(a1) * 0.5, by + cos(a1) * 0.5);
            Pb::new()
                .a(0.0, 0.0, ro, ro, a0, a1, true)
                .lp(polar(ro + e, a1))
                .l(mx, my)
                .lp(polar(ri - e, a1))
                .lp(polar(ri, a1))
                .a(0.0, 0.0, ri, ri, a1, a0, false)
                .z()
                .fit_box(w, h)
                .to_string()
        }
        "banner" => {
            let (bx, bh, ty, tx, g) = (0.13, 0.72, 0.28, 0.24, 0.025);
            let tail: [[f64; 2]; 7] =
                [[0.0, ty], [bx - g, ty], [bx - g, bh + g], [tx, bh + g], [tx, 1.0], [0.0, 1.0], [0.08, (ty + 1.0) / 2.0]];
            let mut mirrored: Vec<[f64; 2]> = tail.iter().map(|&[x, y]| [1.0 - x, y]).collect();
            mirrored.reverse();
            Pb::new()
                .poly(&[[bx, 0.0], [1.0 - bx, 0.0], [1.0 - bx, bh], [bx, bh]])
                .poly(&tail)
                .poly(&mirrored)
                .fit_box(w, h)
                .to_string()
        }
        "wave" => {
            let a = h * p("amplitude", 0.1);
            let k = a * 3.46;
            [
                format!("M0 {}", f3(a)),
                format!("C{} {} {} {} {} {}", f3(w / 3.0), f3(a - k), f3((2.0 * w) / 3.0), f3(a + k), f3(w), f3(a)),
                format!("L{} {}", f3(w), f3(h - a)),
                format!("C{} {} {} {} 0 {}", f3((2.0 * w) / 3.0), f3(h - a + k), f3(w / 3.0), f3(h - a - k), f3(h - a)),
                "Z".into(),
            ]
            .join(" ")
        }
        "callout-rect" => {
            let bh = h * 0.78;
            let r = min(w, bh) * 0.14;
            Pb::new()
                .m(r, 0.0)
                .l(w - r, 0.0)
                .a(w - r, r, r, r, -PI / 2.0, 0.0, false)
                .l(w, bh - r)
                .a(w - r, bh - r, r, r, 0.0, PI / 2.0, false)
                .l(w * 0.42, bh)
                .l(w * 0.2, h)
                .l(w * 0.24, bh)
                .l(r, bh)
                .a(r, bh - r, r, r, PI / 2.0, PI, false)
                .l(0.0, r)
                .a(r, r, r, r, PI, 1.5 * PI, false)
                .z()
                .to_string()
        }
        "callout-round" => {
            let bh = h * 0.82;
            let a0 = (128.0 * PI) / 180.0;
            let a1 = (100.0 * PI) / 180.0 + TAU;
            Pb::new().a(w / 2.0, bh / 2.0, w / 2.0, bh / 2.0, a0, a1, true).l(w * 0.14, h).z().to_string()
        }
        "thought-bubble" => {
            let body = scallops(Pb::new(), 9.0, 0.86).fit_box(w, h * 0.76);
            format!(
                "{} {} {}",
                body,
                ellipse_at(w * 0.13, h * 0.78, w * 0.13, h * 0.12, false),
                ellipse_at(w * 0.02, h * 0.92, w * 0.08, h * 0.08, false)
            )
        }
        "speech" => {
            let bh = h * 0.78;
            poly(&[[0.0, 0.0], [w, 0.0], [w, bh], [w * 0.42, bh], [w * 0.22, h], [w * 0.26, bh], [0.0, bh]])
        }
        // Flowchart symbols (gaps of `g` separate the parts that read as lines).
        "document" => Pb::new().m(0.0, 0.0).l(w, 0.0).l(w, h * 0.86).c(w * 0.7, h * 0.72, w * 0.3, h, 0.0, h * 0.86).z().to_string(),
        "multi-document" => {
            let (dx, dy) = (w * 0.06, h * 0.06);
            let g = mmin * 0.025;
            let band = |x0: f64, y0: f64| -> [[f64; 2]; 6] {
                let (x1, y1) = (x0 + w * 0.88, y0 + h * 0.74);
                [[x0, y0], [x1, y0], [x1, y1], [x1 - dx + g, y1], [x1 - dx + g, y0 + dy - g], [x0, y0 + dy - g]]
            };
            let (fy, fw, fh) = (dy * 2.0, w * 0.88, h - dy * 2.0);
            Pb::new()
                .poly(&band(dx * 2.0, 0.0))
                .poly(&band(dx, dy))
                .m(0.0, fy)
                .l(fw, fy)
                .l(fw, fy + fh * 0.86)
                .c(fw * 0.7, fy + fh * 0.72, fw * 0.3, fy + fh, 0.0, fy + fh * 0.86)
                .z()
                .to_string()
        }
        "predefined-process" => {
            let g = mmin * 0.04;
            let (a, b) = (w * 0.1, w * 0.9);
            Pb::new()
                .poly(&[[0.0, 0.0], [a - g / 2.0, 0.0], [a - g / 2.0, h], [0.0, h]])
                .poly(&[[a + g / 2.0, 0.0], [b - g / 2.0, 0.0], [b - g / 2.0, h], [a + g / 2.0, h]])
                .poly(&[[b + g / 2.0, 0.0], [w, 0.0], [w, h], [b + g / 2.0, h]])
                .to_string()
        }
        "manual-input" => poly(&[[0.0, h * 0.25], [w, 0.0], [w, h], [0.0, h]]),
        "delay" => {
            let r = min(w / 2.0, h / 2.0);
            Pb::new().m(0.0, 0.0).l(w - r, 0.0).a(w - r, h / 2.0, r, h / 2.0, -PI / 2.0, PI / 2.0, false).l(0.0, h).z().to_string()
        }
        "off-page" => poly(&[[0.0, 0.0], [w, 0.0], [w, h * 0.6], [w / 2.0, h], [0.0, h * 0.6]]),
        "internal-storage" => {
            let g = mmin * 0.04;
            let (a, b) = (w * 0.2, h * 0.2);
            Pb::new()
                .poly(&[[0.0, 0.0], [w, 0.0], [w, b - g / 2.0], [0.0, b - g / 2.0]])
                .poly(&[[0.0, b + g / 2.0], [a - g / 2.0, b + g / 2.0], [a - g / 2.0, h], [0.0, h]])
                .poly(&[[a + g / 2.0, b + g / 2.0], [w, b + g / 2.0], [w, h], [a + g / 2.0, h]])
                .to_string()
        }
        "display" => {
            let r = min(w * 0.2, h / 2.0);
            Pb::new()
                .m(0.0, h / 2.0)
                .l(w * 0.2, 0.0)
                .l(w - r, 0.0)
                .a(w - r, h / 2.0, r, h / 2.0, -PI / 2.0, PI / 2.0, false)
                .l(w * 0.2, h)
                .z()
                .to_string()
        }
        _ => String::new(),
    }
}

static POLY_PT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ML](-?[0-9.]+) (-?[0-9.]+)").expect("regex"));

fn polygon_points(d: &str) -> Vec<[f64; 2]> {
    if d.contains('C') || d.matches('M').count() > 1 {
        return Vec::new();
    }
    POLY_PT
        .captures_iter(d)
        .map(|m| [oc_schema::js::string_to_number(&m[1]), oc_schema::js::string_to_number(&m[2])])
        .collect()
}

/// Point-in-polygon for straight-edged shapes; ellipses use an analytical check and curved outlines count as
/// their whole box.
pub fn hit_shape(kind: &str, w: f64, h: f64, x: f64, y: f64, params: &Params) -> bool {
    if x < 0.0 || y < 0.0 || x > w || y > h {
        return false;
    }
    if kind == "ellipse" {
        let dx = (x - w / 2.0) / (w / 2.0);
        let dy = (y - h / 2.0) / (h / 2.0);
        return dx * dx + dy * dy <= 1.0;
    }
    if matches!(kind, "rect" | "rounded" | "speech" | "custom") {
        return true;
    }
    let d = shape_path(kind, w, h, params, None);
    if d.contains('C') {
        return true;
    }
    let pts = polygon_points(&d);
    if pts.is_empty() {
        return true;
    }
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let [xi, yi] = pts[i];
        let [xj, yj] = pts[j];
        if (yi > y) != (yj > y) && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}
