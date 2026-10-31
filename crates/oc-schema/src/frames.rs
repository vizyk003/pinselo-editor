// SPDX-License-Identifier: Apache-2.0
//! Frame library (A13): named photo/video frames. Each entry is a unit-box clip path (x, y in 0..1, scaled to the
//! frame box) plus optional decorative chrome drawn above the content. Letters and numbers come from a bundled
//! OFL font (`frame_glyphs`). Generated deterministically (seeded PRNG) so the library is identical everywhere.

use std::f64::consts::PI;
use std::sync::LazyLock;

use fancy_regex::{Captures, Regex};
use serde::Serialize;

use crate::color::solidc;
use crate::elements::{FrameClip, FrameOverlay};
use crate::frame_glyphs::frame_glyph;
use crate::js;
use crate::macros::string_enum;
use crate::primitives::Extra;

string_enum! {
    pub enum FrameCategory {
        Basic = "basic", Shapes = "shapes", Organic = "organic", Photo = "photo", Devices = "devices",
        Letters = "letters", Numbers = "numbers",
    }
}
pub const FRAME_CATEGORIES: &[&str] = &["basic", "shapes", "organic", "photo", "devices", "letters", "numbers"];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FrameLibraryItem {
    pub id: String,
    /// English display name (the web UI translates it).
    pub name: String,
    pub category: FrameCategory,
    /// Clip outline in the unit box.
    pub d: String,
    /// Default width / height.
    pub aspect: f64,
    /// Built-in clip that old clients fall back to when they do not understand `clipPath`.
    pub fallback: FrameClip,
    /// Decorative chrome above the content, unit box, nonzero fill.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay: Option<Vec<FrameOverlay>>,
    /// Extra search words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
}

// ------------------------------------------------------------------ path building (design space → unit box)

fn r4(n: f64) -> String {
    js::num_to_string(js::round(n * 10000.0) / 10000.0)
}

type Pt = (f64, f64);
const KAPPA: f64 = 0.5522847498;

/// Collects path commands in a W × H design space and emits a unit-box path.
struct P {
    w: f64,
    h: f64,
    parts: Vec<String>,
}

static NUM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-?[0-9]*\.?[0-9]+").expect("regex"));

impl P {
    fn new(w: f64, h: f64) -> Self {
        P { w, h, parts: Vec::new() }
    }
    fn x(&self, v: f64) -> String {
        r4(v / self.w)
    }
    fn y(&self, v: f64) -> String {
        r4(v / self.h)
    }
    fn m(&mut self, x: f64, y: f64) -> &mut Self {
        let s = format!("M{} {}", self.x(x), self.y(y));
        self.parts.push(s);
        self
    }
    fn l(&mut self, x: f64, y: f64) -> &mut Self {
        let s = format!("L{} {}", self.x(x), self.y(y));
        self.parts.push(s);
        self
    }
    #[allow(clippy::too_many_arguments)]
    fn c(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) -> &mut Self {
        let s = format!(
            "C{} {} {} {} {} {}",
            self.x(x1),
            self.y(y1),
            self.x(x2),
            self.y(y2),
            self.x(x),
            self.y(y)
        );
        self.parts.push(s);
        self
    }
    fn z(&mut self) -> &mut Self {
        self.parts.push("Z".into());
        self
    }
    fn poly(&mut self, pts: &[Pt], reverse: bool) -> &mut Self {
        let mut p: Vec<Pt> = pts.to_vec();
        if reverse {
            p.reverse();
        }
        for (i, (x, y)) in p.iter().enumerate() {
            if i > 0 {
                self.l(*x, *y);
            } else {
                self.m(*x, *y);
            }
        }
        self.z()
    }
    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, reverse: bool) -> &mut Self {
        self.poly(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)], reverse)
    }
    /// Rounded rectangle, clockwise (or counter-clockwise when `reverse`).
    #[allow(clippy::too_many_arguments)]
    fn rrect(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64, reverse: bool) -> &mut Self {
        let rr = 0f64.max(r.min(w / 2.0).min(h / 2.0));
        if rr == 0.0 || rr.is_nan() {
            return self.rect(x, y, w, h, reverse);
        }
        let k = KAPPA * rr;
        let (x1, y1) = (x + w, y + h);
        if !reverse {
            return self
                .m(x + rr, y)
                .l(x1 - rr, y)
                .c(x1 - rr + k, y, x1, y + rr - k, x1, y + rr)
                .l(x1, y1 - rr)
                .c(x1, y1 - rr + k, x1 - rr + k, y1, x1 - rr, y1)
                .l(x + rr, y1)
                .c(x + rr - k, y1, x, y1 - rr + k, x, y1 - rr)
                .l(x, y + rr)
                .c(x, y + rr - k, x + rr - k, y, x + rr, y)
                .z();
        }
        self.m(x + rr, y)
            .c(x + rr - k, y, x, y + rr - k, x, y + rr)
            .l(x, y1 - rr)
            .c(x, y1 - rr + k, x + rr - k, y1, x + rr, y1)
            .l(x1 - rr, y1)
            .c(x1 - rr + k, y1, x1, y1 - rr + k, x1, y1 - rr)
            .l(x1, y + rr)
            .c(x1, y + rr - k, x1 - rr + k, y, x1 - rr, y)
            .l(x + rr, y)
            .z()
    }
    fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, reverse: bool) -> &mut Self {
        let (kx, ky) = (KAPPA * rx, KAPPA * ry);
        if !reverse {
            return self
                .m(cx, cy - ry)
                .c(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy)
                .c(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry)
                .c(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy)
                .c(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry)
                .z();
        }
        self.m(cx, cy - ry)
            .c(cx - kx, cy - ry, cx - rx, cy - ky, cx - rx, cy)
            .c(cx - rx, cy + ky, cx - kx, cy + ry, cx, cy + ry)
            .c(cx + kx, cy + ry, cx + rx, cy + ky, cx + rx, cy)
            .c(cx + rx, cy - ky, cx + kx, cy - ry, cx, cy - ry)
            .z()
    }
    /// Smooth closed curve through points (Catmull-Rom → cubic Béziers).
    fn smooth(&mut self, pts: &[Pt], tension: f64) -> &mut Self {
        let n = pts.len() as i64;
        let at = |i: i64| pts[(((i % n) + n) % n) as usize];
        self.m(pts[0].0, pts[0].1);
        for i in 0..n {
            let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
            let t = tension / 6.0;
            self.c(
                p1.0 + (p2.0 - p0.0) * t,
                p1.1 + (p2.1 - p0.1) * t,
                p2.0 - (p3.0 - p1.0) * t,
                p2.1 - (p3.1 - p1.1) * t,
                p2.0,
                p2.1,
            );
        }
        self.z()
    }
    fn d(&self) -> String {
        self.parts.concat()
    }
    /// The path rescaled so its control-point hull spans the unit box.
    fn fitted(&self) -> String {
        let d = self.d();
        let ns: Vec<f64> = NUM_RE
            .find_iter(&d)
            .filter_map(Result::ok)
            .map(|m| js::string_to_number(m.as_str()))
            .collect();
        let xs: Vec<f64> = ns.iter().step_by(2).copied().collect();
        let ys: Vec<f64> = ns.iter().skip(1).step_by(2).copied().collect();
        let (x0, x1) = (min(&xs), max(&xs));
        let (y0, y1) = (min(&ys), max(&ys));
        let mut i = 0usize;
        NUM_RE
            .replace_all(&d, |c: &Captures| {
                let n = js::string_to_number(&c[0]);
                let out = if i.is_multiple_of(2) { r4((n - x0) / (x1 - x0)) } else { r4((n - y0) / (y1 - y0)) };
                i += 1;
                out
            })
            .into_owned()
    }
}

fn min(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::INFINITY, f64::min)
}
fn max(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

/// Deterministic pseudo-random sequence (LCG; same as the TS `rng`).
fn rng(seed: u32) -> impl FnMut() -> f64 {
    let mut s: u64 = u64::from(seed);
    move || {
        s = (s * 1_664_525 + 1_013_904_223) & 0xffff_ffff;
        s as f64 / 4_294_967_296.0
    }
}

/// Points on a closed radial outline r(θ) around (cx, cy), clockwise from the top.
fn radial(n: usize, cx: f64, cy: f64, rx: f64, ry: f64, r: impl Fn(f64, usize) -> f64) -> Vec<Pt> {
    (0..n)
        .map(|i| {
            let a = -PI / 2.0 + (i as f64 * 2.0 * PI) / n as f64;
            let k = r(a, i);
            (cx + a.cos() * rx * k, cy + a.sin() * ry * k)
        })
        .collect()
}

fn blob(seed: u32, n: usize, spread: f64) -> String {
    let mut rand = rng(seed);
    let radii: Vec<f64> = (0..n).map(|_| 1.0 - spread + rand() * spread).collect();
    let pts = radial(n, 0.0, 0.0, 1.0, 1.0, |_, i| radii[i]);
    let xs: Vec<f64> = pts.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
    let (x0, x1, y0, y1) = (min(&xs), max(&xs), min(&ys), max(&ys));
    let fit: Vec<Pt> = pts
        .iter()
        .map(|(x, y)| (6.0 + ((x - x0) / (x1 - x0)) * 88.0, 6.0 + ((y - y0) / (y1 - y0)) * 88.0))
        .collect();
    P::new(100.0, 100.0).smooth(&fit, 1.0).fitted()
}

fn solid_overlay(d: String, color: &str) -> FrameOverlay {
    FrameOverlay { d, fill: solidc(color), extra: Extra::new() }
}

const BEZEL: &str = "#1f2328";
const SILVER: &str = "#a3a9b3";

fn clamp_pts(pts: &[Pt], w: f64, h: f64) -> Vec<Pt> {
    pts.iter().map(|(x, y)| (0f64.max(w.min(*x)), 0f64.max(h.min(*y)))).collect()
}

struct Item<'a> {
    id: &'a str,
    name: &'a str,
    category: FrameCategory,
    aspect: f64,
    fallback: FrameClip,
    d: String,
    overlay: Option<Vec<FrameOverlay>>,
    tags: Option<&'a str>,
}

fn build() -> Vec<FrameLibraryItem> {
    use FrameCategory as Cat;
    use FrameClip as Fb;
    let mut out: Vec<FrameLibraryItem> = Vec::new();
    let mut add = |it: Item| {
        out.push(FrameLibraryItem {
            id: it.id.to_string(),
            name: it.name.to_string(),
            category: it.category,
            d: it.d,
            aspect: it.aspect,
            fallback: it.fallback,
            overlay: it.overlay,
            tags: it.tags.map(str::to_string),
        })
    };
    let s = || P::new(100.0, 100.0);
    let item = |id, name, category, aspect, fallback, d: String, tags: Option<&'static str>| Item {
        id,
        name,
        category,
        aspect,
        fallback,
        d,
        overlay: None,
        tags,
    };

    // Basic
    add(item("square", "Square", Cat::Basic, 1.0, Fb::Rect, s().rect(0.0, 0.0, 100.0, 100.0, false).d(), None));
    add(item(
        "portrait",
        "Portrait",
        Cat::Basic,
        4.0 / 5.0,
        Fb::Rect,
        s().rect(0.0, 0.0, 100.0, 100.0, false).d(),
        Some("rectangle vertical"),
    ));
    add(item("rounded", "Rounded square", Cat::Basic, 1.0, Fb::Rounded, s().rrect(0.0, 0.0, 100.0, 100.0, 16.0, false).d(), None));
    add(item("circle", "Circle", Cat::Basic, 1.0, Fb::Ellipse, s().ellipse(50.0, 50.0, 50.0, 50.0, false).d(), Some("round")));
    add(item("oval", "Oval", Cat::Basic, 3.0 / 4.0, Fb::Ellipse, s().ellipse(50.0, 50.0, 50.0, 50.0, false).d(), Some("ellipse")));
    {
        // Superellipse |x|^4 + |y|^4 = 1.
        let pts = radial(64, 50.0, 50.0, 50.0, 50.0, |a, _| {
            1.0 / (a.cos().abs().powf(4.0) + a.sin().abs().powf(4.0)).powf(1.0 / 4.0)
        });
        add(item("squircle", "Squircle", Cat::Basic, 1.0, Fb::Rounded, s().smooth(&pts, 1.0).fitted(), None));
    }
    {
        let mut p = P::new(80.0, 100.0);
        let k = KAPPA * 40.0;
        p.m(0.0, 100.0)
            .l(0.0, 40.0)
            .c(0.0, 40.0 - k, 40.0 - k, 0.0, 40.0, 0.0)
            .c(40.0 + k, 0.0, 80.0, 40.0 - k, 80.0, 40.0)
            .l(80.0, 100.0)
            .z();
        add(item("arch", "Arch", Cat::Basic, 0.8, Fb::Rounded, p.d(), Some("window door")));
    }
    add(item(
        "pill",
        "Pill",
        Cat::Basic,
        2.0,
        Fb::Rounded,
        P::new(200.0, 100.0).rrect(0.0, 0.0, 200.0, 100.0, 50.0, false).d(),
        Some("capsule"),
    ));

    // Shapes
    {
        let mut p = s();
        p.m(50.0, 26.0)
            .c(50.0, 12.0, 38.0, 2.0, 26.0, 2.0)
            .c(10.0, 2.0, 0.0, 15.0, 0.0, 30.0)
            .c(0.0, 56.0, 30.0, 74.0, 50.0, 98.0)
            .c(70.0, 74.0, 100.0, 56.0, 100.0, 30.0)
            .c(100.0, 15.0, 90.0, 2.0, 74.0, 2.0)
            .c(62.0, 2.0, 50.0, 12.0, 50.0, 26.0)
            .z();
        add(item("heart", "Heart", Cat::Shapes, 1.0, Fb::Heart, p.d(), Some("love")));
    }
    let star = |points: usize, inner: f64| {
        s().poly(&radial(points * 2, 50.0, 50.0, 50.0, 50.0, |_, i| if i % 2 == 1 { inner } else { 1.0 }), false).d()
    };
    add(item("star", "Star", Cat::Shapes, 1.0, Fb::Star, star(5, 0.45), None));
    add(item("burst", "Starburst", Cat::Shapes, 1.0, Fb::Star, star(14, 0.8), Some("badge sale")));
    add(item(
        "hexagon",
        "Hexagon",
        Cat::Shapes,
        1.1547,
        Fb::Hexagon,
        s().poly(&[(25.0, 0.0), (75.0, 0.0), (100.0, 50.0), (75.0, 100.0), (25.0, 100.0), (0.0, 50.0)], false).d(),
        None,
    ));
    add(item(
        "octagon",
        "Octagon",
        Cat::Shapes,
        1.0,
        Fb::Rect,
        s().poly(
            &[
                (29.29, 0.0),
                (70.71, 0.0),
                (100.0, 29.29),
                (100.0, 70.71),
                (70.71, 100.0),
                (29.29, 100.0),
                (0.0, 70.71),
                (0.0, 29.29),
            ],
            false,
        )
        .d(),
        None,
    ));
    add(item(
        "diamond",
        "Diamond",
        Cat::Shapes,
        1.0,
        Fb::Diamond,
        s().poly(&[(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)], false).d(),
        Some("rhombus"),
    ));
    add(item(
        "triangle",
        "Triangle",
        Cat::Shapes,
        1.1547,
        Fb::Triangle,
        s().poly(&[(50.0, 0.0), (100.0, 100.0), (0.0, 100.0)], false).d(),
        None,
    ));
    {
        let mut p = s();
        p.ellipse(50.0, 41.0, 50.0, 41.0, false);
        p.poly(&[(22.0, 70.0), (42.0, 80.0), (8.0, 100.0)], false);
        add(item("speech", "Speech bubble", Cat::Shapes, 1.15, Fb::Ellipse, p.d(), Some("chat talk quote")));
    }
    {
        let mut p = s();
        p.rrect(0.0, 0.0, 100.0, 80.0, 12.0, false);
        p.poly(&[(62.0, 78.0), (84.0, 78.0), (86.0, 100.0)], false);
        add(item("speech-rect", "Chat bubble", Cat::Shapes, 1.25, Fb::Rounded, p.d(), Some("speech message talk")));
    }
    add(item(
        "flower",
        "Flower",
        Cat::Shapes,
        1.0,
        Fb::Ellipse,
        s().smooth(&radial(64, 50.0, 50.0, 50.0, 50.0, |a, _| 0.84 + 0.16 * (8.0 * (a + PI / 2.0)).cos()), 1.0)
            .fitted(),
        Some("scalloped badge"),
    ));
    {
        // Postage stamp: scalloped (perforated) edges.
        let (w, h, r, n, m) = (100.0, 120.0, 3.4, 10usize, 12usize);
        let mut p = P::new(w, h);
        let k = KAPPA * r;
        p.m(0.0, 0.0);
        let (sx, sy) = (w / n as f64, h / m as f64);
        for i in 0..n {
            let c = sx * (i as f64 + 0.5);
            p.l(c - r, 0.0).c(c - r, k, c - k, r, c, r).c(c + k, r, c + r, k, c + r, 0.0);
        }
        p.l(w, 0.0);
        for i in 0..m {
            let c = sy * (i as f64 + 0.5);
            p.l(w, c - r).c(w - k, c - r, w - r, c - k, w - r, c).c(w - r, c + k, w - k, c + r, w, c + r);
        }
        p.l(w, h);
        for i in (0..n).rev() {
            let c = sx * (i as f64 + 0.5);
            p.l(c + r, h).c(c + r, h - k, c + k, h - r, c, h - r).c(c - k, h - r, c - r, h - k, c - r, h);
        }
        p.l(0.0, h);
        for i in (0..m).rev() {
            let c = sy * (i as f64 + 0.5);
            p.l(0.0, c + r).c(k, c + r, r, c + k, r, c).c(r, c - k, k, c - r, 0.0, c - r);
        }
        p.z();
        add(item("stamp", "Stamp", Cat::Shapes, w / h, Fb::Rect, p.d(), Some("postage perforated scalloped")));
    }
    add(item(
        "cross",
        "Cross",
        Cat::Shapes,
        1.0,
        Fb::Rect,
        s().poly(
            &[
                (32.0, 0.0),
                (68.0, 0.0),
                (68.0, 32.0),
                (100.0, 32.0),
                (100.0, 68.0),
                (68.0, 68.0),
                (68.0, 100.0),
                (32.0, 100.0),
                (32.0, 68.0),
                (0.0, 68.0),
                (0.0, 32.0),
                (32.0, 32.0),
            ],
            false,
        )
        .d(),
        Some("plus"),
    ));

    // Organic
    for (i, seed) in [11u32, 23, 37, 58, 71, 94].into_iter().enumerate() {
        let id = format!("blob-{}", i + 1);
        let name = format!("Blob {}", i + 1);
        let d = blob(seed, 6 + (i % 3), 0.24 + (i % 2) as f64 * 0.08);
        add(Item {
            id: &id,
            name: &name,
            category: Cat::Organic,
            aspect: 1.0,
            fallback: Fb::Ellipse,
            d,
            overlay: None,
            tags: Some("organic"),
        });
    }
    add(item(
        "wavy",
        "Wavy circle",
        Cat::Organic,
        1.0,
        Fb::Ellipse,
        s().smooth(&radial(72, 50.0, 50.0, 50.0, 50.0, |a, _| 0.93 + 0.07 * (12.0 * a).sin()), 1.0).fitted(),
        Some("organic wave"),
    ));
    {
        let mut rand = rng(4242);
        let mut pts: Vec<Pt> = Vec::new();
        for i in 0..18 {
            let a = -PI / 2.0 + (f64::from(i) * 2.0 * PI) / 18.0;
            let k = 0.82 + rand() * 0.18;
            pts.push((50.0 + a.cos() * 47.0 * k, 50.0 + a.sin() * 47.0 * k));
        }
        add(item("pebble", "Pebble", Cat::Organic, 1.25, Fb::Ellipse, s().smooth(&pts, 0.8).fitted(), Some("stone rock organic")));
    }
    {
        // Brush stroke: wavy long edges, bristly ends.
        let mut rand = rng(777);
        let (w, h) = (200.0, 100.0);
        let mut pts: Vec<Pt> = Vec::new();
        let mut x = 14.0;
        while x <= 186.0 {
            pts.push((x, 7.0 + (x / 23.0).sin() * 3.0 + rand() * 3.0));
            x += 8.0;
        }
        for i in 0..9 {
            let y = 12.0 + (f64::from(i) * 76.0) / 8.0;
            let dx = if i % 2 == 1 { -6.0 - rand() * 6.0 } else { rand() * 8.0 };
            pts.push((190.0 + dx, y));
        }
        let mut x = 186.0;
        while x >= 14.0 {
            pts.push((x, 93.0 + (x / 19.0).sin() * 3.0 - rand() * 3.0));
            x -= 8.0;
        }
        for i in (0..=8).rev() {
            let y = 12.0 + (f64::from(i) * 76.0) / 8.0;
            let dx = if i % 2 == 1 { -6.0 - rand() * 6.0 } else { rand() * 8.0 };
            pts.push((10.0 - dx, y));
        }
        let d = P::new(w, h).poly(&clamp_pts(&pts, w, h), false).d();
        add(item("brush", "Brush stroke", Cat::Organic, 2.0, Fb::Rect, d, Some("paint")));
    }
    {
        // Torn paper: irregular jagged edge all round.
        let mut rand = rng(1313);
        let (w, h, m) = (120.0, 100.0, 4.0);
        let mut j = || (rand() - 0.5) * 2.0 * 3.2;
        let mut pts: Vec<Pt> = Vec::new();
        let mut x = m;
        while x < w - m {
            pts.push((x, m + j()));
            x += 3.0;
        }
        let mut y = m;
        while y < h - m {
            pts.push((w - m + j(), y));
            y += 3.0;
        }
        let mut x = w - m;
        while x > m {
            pts.push((x, h - m + j()));
            x -= 3.0;
        }
        let mut y = h - m;
        while y > m {
            pts.push((m + j(), y));
            y -= 3.0;
        }
        let d = P::new(w, h).poly(&clamp_pts(&pts, w, h), false).d();
        add(item("torn", "Torn paper", Cat::Organic, w / h, Fb::Rect, d, Some("ripped edge")));
    }

    // Photo
    {
        let (w, h) = (88.0, 107.0);
        let win = (6.0, 6.0, 76.0, 77.0);
        let card = P::new(w, h).rect(0.0, 0.0, w, h, false).rect(win.0, win.1, win.2, win.3, true).d();
        let edge = P::new(w, h).rect(0.0, 0.0, w, h, false).rect(0.5, 0.5, w - 1.0, h - 1.0, true).d();
        add(Item {
            overlay: Some(vec![solid_overlay(card, "#ffffff"), solid_overlay(edge, "#d9dce1")]),
            ..item("polaroid", "Polaroid", Cat::Photo, w / h, Fb::Rect, P::new(w, h).rect(win.0, win.1, win.2, win.3, false).d(), Some("instant snapshot"))
        });
    }
    {
        let (w, h) = (54.0, 86.0);
        let win = (4.0, 6.0, 46.0, 62.0);
        let card = P::new(w, h).rrect(0.0, 0.0, w, h, 2.0, false).rect(win.0, win.1, win.2, win.3, true).d();
        let edge = P::new(w, h).rrect(0.0, 0.0, w, h, 2.0, false).rrect(0.5, 0.5, w - 1.0, h - 1.0, 1.6, true).d();
        add(Item {
            overlay: Some(vec![solid_overlay(card, "#fdfcf8"), solid_overlay(edge, "#d9dce1")]),
            ..item("instant-mini", "Instant mini", Cat::Photo, w / h, Fb::Rect, P::new(w, h).rect(win.0, win.1, win.2, win.3, false).d(), Some("polaroid snapshot"))
        });
    }
    {
        let (w, h) = (160.0, 100.0);
        let win = (10.0, 16.0, 140.0, 68.0);
        let mut strip = P::new(w, h);
        strip.rect(0.0, 0.0, w, h, false).rect(win.0, win.1, win.2, win.3, true);
        let mut x = 5.0;
        while x < w - 6.0 {
            strip.rrect(x, 4.0, 6.0, 7.0, 1.2, true);
            strip.rrect(x, h - 11.0, 6.0, 7.0, 1.2, true);
            x += 12.0;
        }
        add(Item {
            overlay: Some(vec![solid_overlay(strip.d(), "#16181d")]),
            ..item("film", "Film strip", Cat::Photo, w / h, Fb::Rect, P::new(w, h).rect(win.0, win.1, win.2, win.3, false).d(), Some("movie cinema negative"))
        });
    }
    {
        let (w, h) = (100.0, 100.0);
        let win = P::new(w, h).rrect(18.0, 25.0, 64.0, 50.0, 2.0, false).d();
        let mount = P::new(w, h).rrect(0.0, 0.0, w, h, 7.0, false).rrect(18.0, 25.0, 64.0, 50.0, 2.0, true).d();
        let rim = P::new(w, h).rrect(0.0, 0.0, w, h, 7.0, false).rrect(0.6, 0.6, w - 1.2, h - 1.2, 6.4, true).d();
        add(Item {
            overlay: Some(vec![solid_overlay(mount, "#f1f2f4"), solid_overlay(rim, "#c9cdd3")]),
            ..item("slide", "Photo slide", Cat::Photo, 1.0, Fb::Rect, win, Some("35mm mount"))
        });
    }

    // Devices (clip = screen; chrome = bezel, body, stand)
    {
        let (w, h) = (90.0, 190.0);
        let sc = (5.0, 5.0, 80.0, 180.0, 10.0);
        let bezel = P::new(w, h).rrect(0.0, 0.0, w, h, 14.0, false).rrect(sc.0, sc.1, sc.2, sc.3, sc.4, true).d();
        let notch = P::new(w, h).rrect(33.0, 10.0, 24.0, 7.0, 3.5, false).d();
        add(Item {
            overlay: Some(vec![solid_overlay(bezel, BEZEL), solid_overlay(notch, "#0b0c0f")]),
            ..item(
                "phone",
                "Phone",
                Cat::Devices,
                w / h,
                Fb::Rounded,
                P::new(w, h).rrect(sc.0, sc.1, sc.2, sc.3, sc.4, false).d(),
                Some("mobile smartphone iphone android mockup"),
            )
        });
    }
    {
        let (w, h) = (150.0, 200.0);
        let sc = (9.0, 9.0, 132.0, 182.0, 4.0);
        let bezel = P::new(w, h)
            .rrect(0.0, 0.0, w, h, 13.0, false)
            .rrect(sc.0, sc.1, sc.2, sc.3, sc.4, true)
            .ellipse(75.0, 4.6, 1.6, 1.6, true)
            .d();
        add(Item {
            overlay: Some(vec![solid_overlay(bezel, BEZEL)]),
            ..item("tablet", "Tablet", Cat::Devices, w / h, Fb::Rounded, P::new(w, h).rrect(sc.0, sc.1, sc.2, sc.3, sc.4, false).d(), Some("ipad mockup"))
        });
    }
    {
        let (w, h) = (160.0, 104.0);
        let sc = (20.0, 5.0, 120.0, 77.0);
        let lid = P::new(w, h).rrect(14.0, 0.0, 132.0, 90.0, 6.0, false).rect(sc.0, sc.1, sc.2, sc.3, true).d();
        let base = P::new(w, h)
            .m(0.0, 90.0)
            .l(160.0, 90.0)
            .l(160.0, 96.0)
            .c(160.0, 100.0, 154.0, 104.0, 146.0, 104.0)
            .l(14.0, 104.0)
            .c(6.0, 104.0, 0.0, 100.0, 0.0, 96.0)
            .z()
            .d();
        let notch = P::new(w, h).rrect(66.0, 90.0, 28.0, 3.6, 1.8, false).d();
        add(Item {
            overlay: Some(vec![solid_overlay(lid, BEZEL), solid_overlay(base, SILVER), solid_overlay(notch, "#7b818c")]),
            ..item(
                "laptop",
                "Laptop",
                Cat::Devices,
                w / h,
                Fb::Rect,
                P::new(w, h).rect(sc.0, sc.1, sc.2, sc.3, false).d(),
                Some("computer macbook notebook mockup"),
            )
        });
    }
    {
        let (w, h) = (160.0, 132.0);
        let sc = (5.0, 5.0, 150.0, 84.0);
        let display = P::new(w, h).rrect(0.0, 0.0, w, 100.0, 5.0, false).rect(sc.0, sc.1, sc.2, sc.3, true).d();
        let stand = P::new(w, h)
            .poly(&[(70.0, 100.0), (90.0, 100.0), (93.0, 122.0), (67.0, 122.0)], false)
            .rrect(42.0, 121.0, 76.0, 9.0, 3.0, false)
            .d();
        add(Item {
            overlay: Some(vec![solid_overlay(stand, SILVER), solid_overlay(display, BEZEL)]),
            ..item(
                "monitor",
                "Desktop monitor",
                Cat::Devices,
                w / h,
                Fb::Rect,
                P::new(w, h).rect(sc.0, sc.1, sc.2, sc.3, false).d(),
                Some("computer screen imac display mockup"),
            )
        });
    }
    {
        let (w, h) = (100.0, 150.0);
        let sc = (16.0, 37.0, 68.0, 76.0, 15.0);
        let straps = P::new(w, h).rrect(24.0, 0.0, 52.0, 34.0, 5.0, false).rrect(24.0, 116.0, 52.0, 34.0, 5.0, false).d();
        let body = P::new(w, h).rrect(8.0, 28.0, 84.0, 94.0, 24.0, false).rrect(sc.0, sc.1, sc.2, sc.3, sc.4, true).d();
        let crown = P::new(w, h).rrect(91.0, 58.0, 7.0, 18.0, 2.5, false).d();
        add(Item {
            overlay: Some(vec![solid_overlay(straps, "#3b4250"), solid_overlay(crown, "#4b5260"), solid_overlay(body, BEZEL)]),
            ..item(
                "watch",
                "Smartwatch",
                Cat::Devices,
                w / h,
                Fb::Rounded,
                P::new(w, h).rrect(sc.0, sc.1, sc.2, sc.3, sc.4, false).d(),
                Some("watch wearable mockup"),
            )
        });
    }

    // Letters, then numbers.
    for ch in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".chars() {
        let Some(g) = frame_glyph(ch) else { continue };
        let digit = ch.is_ascii_digit();
        let id = format!("{}-{}", if digit { "number" } else { "letter" }, ch);
        let name = if digit { format!("Number {ch}") } else { format!("Letter {ch}") };
        let tags = ch.to_ascii_lowercase().to_string();
        add(Item {
            id: &id,
            name: &name,
            category: if digit { Cat::Numbers } else { Cat::Letters },
            aspect: g.aspect,
            fallback: Fb::Rect,
            d: g.d.to_string(),
            overlay: None,
            tags: Some(&tags),
        });
    }
    out
}

pub static FRAME_LIBRARY: LazyLock<Vec<FrameLibraryItem>> = LazyLock::new(build);

pub fn frame_library_item(id: &str) -> Option<&'static FrameLibraryItem> {
    FRAME_LIBRARY.iter().find(|f| f.id == id)
}
