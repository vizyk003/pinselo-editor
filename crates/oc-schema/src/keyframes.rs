// SPDX-License-Identifier: Apache-2.0
//! Keyframe interpolation (MV-100), shared by the renderer (playback/export) and the editor (recording).

use crate::elements::{ColorKey, GradientKey, Keyframe, KeyframeEase};
use crate::primitives::{ColorValue, RgbColor};

/// Easing curve of a keyframe segment (`EASES[ease]`). Unknown eases use `ease` (TS would throw).
pub fn ease(e: &KeyframeEase, p: f64) -> f64 {
    match e {
        KeyframeEase::Linear => p,
        KeyframeEase::EaseIn => p * p * p,
        KeyframeEase::EaseOut => 1.0 - (1.0 - p).powi(3),
        KeyframeEase::Back => 1.0 + 2.70158 * (p - 1.0).powi(3) + 1.70158 * (p - 1.0).powi(2),
        KeyframeEase::Hold => {
            if p < 1.0 { 0.0 } else { 1.0 }
        }
        KeyframeEase::Ease | KeyframeEase::Other(_) => {
            if p < 0.5 { 4.0 * p * p * p } else { 1.0 - (-2.0 * p + 2.0).powi(3) / 2.0 }
        }
    }
}

/// Whether `b` is a usable cubic bezier: finite, with both x control points in 0..1 (as CSS requires, so the
/// curve is a function of time).
pub fn bezier_ok(b: &[f64; 4]) -> bool {
    b.iter().all(|v| v.is_finite()) && (0.0..=1.0).contains(&b[0]) && (0.0..=1.0).contains(&b[2])
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at progress `p` (0..1): solves x(s) = p for the curve parameter s, then
/// returns y(s). Newton steps, falling back to bisection; only + − × ÷, so preview (WASM) and export agree.
pub fn cubic_bezier(b: &[f64; 4], p: f64) -> f64 {
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    let [x1, y1, x2, y2] = *b;
    // Polynomial coefficients of x(s) and y(s) = ((a·s + b)·s + c)·s.
    let (cx, cy) = (3.0 * x1, 3.0 * y1);
    let (bx, by) = (3.0 * (x2 - x1) - cx, 3.0 * (y2 - y1) - cy);
    let (ax, ay) = (1.0 - cx - bx, 1.0 - cy - by);
    let x = |s: f64| ((ax * s + bx) * s + cx) * s;
    let dx = |s: f64| (3.0 * ax * s + 2.0 * bx) * s + cx;
    let mut s = p;
    let mut solved = false;
    for _ in 0..8 {
        let err = x(s) - p;
        if err.abs() < 1e-7 {
            solved = true;
            break;
        }
        let d = dx(s);
        if d.abs() < 1e-6 {
            break;
        }
        s -= err / d;
    }
    if !solved || !(0.0..=1.0).contains(&s) {
        let (mut lo, mut hi) = (0.0, 1.0);
        s = p;
        for _ in 0..40 {
            let v = x(s);
            if (v - p).abs() < 1e-7 {
                break;
            }
            if v < p {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) / 2.0;
        }
    }
    ((ay * s + by) * s + cy) * s
}

/// Neutral value of each keyframed property (`KEYFRAME_NEUTRAL`).
pub const KEYFRAME_NEUTRAL: &[(&str, f64)] =
    &[("x", 0.0), ("y", 0.0), ("scale", 1.0), ("rotate", 0.0), ("opacity", 1.0), ("blur", 0.0)];

/// How a track combines with the element's own value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropMode {
    /// Added to it (stored in the element's rotated box space for `x`/`y`).
    Offset,
    /// Multiplies it.
    Multiply,
    /// Sets it: the track holds the value itself, and the element keeps its own where the track has no keys.
    Replace,
}

/// What a track's keys hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    Number,
    /// [`ColorKey`]s.
    Color,
    /// [`GradientKey`]s: a colour per gradient stop.
    Gradient,
}

/// One keyframed property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropSpec {
    /// Its key in `keyframes`.
    pub name: &'static str,
    pub mode: PropMode,
    pub kind: PropKind,
    /// The value that leaves the element as it is (offset and multiply tracks).
    pub neutral: f64,
    /// The range a value is held to.
    pub min: f64,
    pub max: f64,
    /// Element types it applies to (`type`); empty = all.
    pub types: &'static [&'static str],
}

const fn prop(name: &'static str, mode: PropMode, neutral: f64, min: f64, max: f64, types: &'static [&'static str]) -> PropSpec {
    PropSpec { name, mode, kind: PropKind::Number, neutral, min, max, types }
}
const fn color(name: &'static str, types: &'static [&'static str]) -> PropSpec {
    PropSpec { name, mode: PropMode::Replace, kind: PropKind::Color, neutral: 0.0, min: 0.0, max: 0.0, types }
}
use PropMode::{Multiply, Offset, Replace};
const INF: f64 = f64::INFINITY;
const CROPPED: &[&str] = &["image", "video"];
const STROKED: &[&str] = &["shape", "line"];

/// Every keyframed property, in the order panels list them.
pub const PROPS: &[PropSpec] = &[
    prop("x", Offset, 0.0, -INF, INF, &[]),
    prop("y", Offset, 0.0, -INF, INF, &[]),
    prop("scale", Multiply, 1.0, 0.001, INF, &[]),
    prop("rotate", Offset, 0.0, -INF, INF, &[]),
    prop("opacity", Multiply, 1.0, 0.0, 1.0, &[]),
    prop("blur", Offset, 0.0, 0.0, INF, &[]),
    prop("scaleX", Multiply, 1.0, 0.001, INF, &[]),
    prop("scaleY", Multiply, 1.0, 0.001, INF, &[]),
    prop("anchorX", Replace, 0.5, -INF, INF, &[]),
    prop("anchorY", Replace, 0.5, -INF, INF, &[]),
    color("fill", &["shape"]),
    color("strokeColor", STROKED),
    prop("strokeWidth", Replace, 0.0, 0.0, INF, STROKED),
    prop("radius", Replace, 0.0, 0.0, INF, &["shape", "image", "video"]),
    prop("cropX", Replace, 0.0, 0.0, 1.0, CROPPED),
    prop("cropY", Replace, 0.0, 0.0, 1.0, CROPPED),
    prop("cropW", Replace, 1.0, 0.0001, 1.0, CROPPED),
    prop("cropH", Replace, 1.0, 0.0001, 1.0, CROPPED),
    prop("shadowX", Replace, 0.0, -INF, INF, &[]),
    prop("shadowY", Replace, 0.0, -INF, INF, &[]),
    prop("shadowBlur", Replace, 0.0, 0.0, INF, &[]),
    color("shadowColor", &[]),
    prop("motionBlur", Replace, 0.0, 0.0, 1.0, &[]),
    prop("gradientAngle", Replace, 0.0, -INF, INF, &["shape"]),
    PropSpec { name: "gradientColors", mode: Replace, kind: PropKind::Gradient, neutral: 0.0, min: 0.0, max: 0.0, types: &["shape"] },
];

/// The spec of a keyframed property.
pub fn prop_spec(name: &str) -> Option<&'static PropSpec> {
    PROPS.iter().find(|p| p.name == name)
}

impl PropSpec {
    /// Whether the property does anything on an element of type `ty`.
    pub fn applies_to(&self, ty: &str) -> bool {
        self.types.is_empty() || self.types.contains(&ty)
    }
}

/// The neutral value of a number property (`KEYFRAME_NEUTRAL`, then the later ones).
pub fn keyframe_neutral(prop: &str) -> Option<f64> {
    KEYFRAME_NEUTRAL.iter().find(|(k, _)| *k == prop).map(|(_, v)| *v).or_else(|| prop_spec(prop).filter(|p| p.kind == PropKind::Number).map(|p| p.neutral))
}

/// What a key carries for timing, whatever its value.
trait Timed {
    fn t(&self) -> f64;
    fn shape(&self, p: f64) -> f64;
}
fn shape_of(bezier: Option<&[f64; 4]>, e: Option<&KeyframeEase>, p: f64) -> f64 {
    match bezier.filter(|z| bezier_ok(z)) {
        Some(z) => cubic_bezier(z, p),
        None => ease(e.unwrap_or(&KeyframeEase::Ease), p),
    }
}
impl Timed for Keyframe {
    fn t(&self) -> f64 {
        self.t
    }
    fn shape(&self, p: f64) -> f64 {
        shape_of(self.bezier.as_ref(), self.ease.as_ref(), p)
    }
}
impl Timed for GradientKey {
    fn t(&self) -> f64 {
        self.t
    }
    fn shape(&self, p: f64) -> f64 {
        shape_of(self.bezier.as_ref(), self.ease.as_ref(), p)
    }
}
impl Timed for ColorKey {
    fn t(&self) -> f64 {
        self.t
    }
    fn shape(&self, p: f64) -> f64 {
        shape_of(self.bezier.as_ref(), self.ease.as_ref(), p)
    }
}

/// Where page time `t` falls on a track: on one key (before the first, after the last), or between two with
/// the arriving key's shaped progress.
enum At<'k, K> {
    On(&'k K),
    Between(&'k K, &'k K, f64),
}

fn locate<K: Timed>(keys: &[K], t: f64) -> At<'_, K> {
    let mut ks: Vec<&K> = keys.iter().collect();
    if ks.windows(2).any(|w| w[1].t() < w[0].t()) {
        ks.sort_by(|a, b| a.t().partial_cmp(&b.t()).unwrap_or(std::cmp::Ordering::Equal));
    }
    if t <= ks[0].t() {
        return At::On(ks[0]);
    }
    for i in 1..ks.len() {
        let (a, b) = (ks[i - 1], ks[i]);
        if t <= b.t() {
            return At::Between(a, b, b.shape((t - a.t()) / (b.t() - a.t()).max(1.0)));
        }
    }
    At::On(ks[ks.len() - 1])
}

/// A colour track at page time `t`. Two sRGB (or two Display P3) keys blend channel by channel, alpha included;
/// keys in different spaces, or CMYK and spot colours, switch when the later key is reached.
pub fn color_value(keys: &[ColorKey], t: f64) -> Option<ColorValue> {
    if keys.is_empty() {
        return None;
    }
    Some(match locate(keys, t) {
        At::On(k) => k.c.clone(),
        At::Between(a, b, p) => mix_colors(&a.c, &b.c, p),
    })
}

/// Two colours `p` of the way from `a` to `b`: channel by channel (alpha included) when both are sRGB or both
/// Display P3, else `a` until `b` is reached.
fn mix_colors(a: &ColorValue, b: &ColorValue, p: f64) -> ColorValue {
    let mix = |x: &RgbColor, y: &RgbColor| RgbColor {
        r: x.r + (y.r - x.r) * p,
        g: x.g + (y.g - x.g) * p,
        b: x.b + (y.b - x.b) * p,
        a: x.a + (y.a - x.a) * p,
        extra: Default::default(),
    };
    match (a, b) {
        (ColorValue::Srgb(x), ColorValue::Srgb(y)) => ColorValue::Srgb(mix(x, y)),
        (ColorValue::DisplayP3(x), ColorValue::DisplayP3(y)) => ColorValue::DisplayP3(mix(x, y)),
        _ if p >= 1.0 => b.clone(),
        _ => a.clone(),
    }
}

/// The gradient colour track at page time `t`: stop by stop. Keys with different numbers of stops switch when the
/// later key is reached.
pub fn gradient_value(keys: &[GradientKey], t: f64) -> Option<Vec<ColorValue>> {
    if keys.is_empty() {
        return None;
    }
    Some(match locate(keys, t) {
        At::On(k) => k.colors.clone(),
        At::Between(a, b, p) if a.colors.len() == b.colors.len() => a.colors.iter().zip(&b.colors).map(|(x, y)| mix_colors(x, y, p)).collect(),
        At::Between(a, b, p) => if p >= 1.0 { b.colors.clone() } else { a.colors.clone() },
    })
}

/// A number track at page time `t`, held to the property's range; `None` when the track has no keys.
pub fn number_value(spec: &PropSpec, keys: Option<&[Keyframe]>, t: f64) -> Option<f64> {
    let keys = keys.filter(|k| !k.is_empty())?;
    let v = keyframe_value(Some(keys), t, spec.neutral);
    Some(if v.is_nan() { spec.neutral } else { v.clamp(spec.min, spec.max) })
}

/// Interpolated value of a keyframe track at page time `t` (before the first key → first value, after the
/// last → last).
pub fn keyframe_value(keys: Option<&[Keyframe]>, t: f64, neutral: f64) -> f64 {
    let Some(keys) = keys.filter(|k| !k.is_empty()) else { return neutral };
    match locate(keys, t) {
        At::On(k) => k.v,
        At::Between(a, b, p) => a.v + (b.v - a.v) * p,
    }
}

#[cfg(test)]
mod bezier_tests {
    use super::*;

    fn key(v: serde_json::Value) -> Keyframe {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn matches_css_curves() {
        // CSS `ease` is cubic-bezier(0.25, 0.1, 0.25, 1): 0.8024 at the halfway point (browsers agree to 1e-4).
        assert!((cubic_bezier(&[0.25, 0.1, 0.25, 1.0], 0.5) - 0.8024).abs() < 1e-3);
        for p in [0.0, 0.1, 0.33, 0.5, 0.9, 1.0] {
            assert!((cubic_bezier(&[0.0, 0.0, 1.0, 1.0], p) - p).abs() < 1e-6, "linear at {p}");
        }
        assert!((cubic_bezier(&[0.42, 0.0, 0.58, 1.0], 0.5) - 0.5).abs() < 1e-6);
        // Symmetric curves mirror: y(p) + y(1 − p) = 1.
        let b = [0.65, 0.0, 0.35, 1.0];
        assert!((cubic_bezier(&b, 0.2) + cubic_bezier(&b, 0.8) - 1.0).abs() < 1e-6);
        // An overshooting curve passes 1 before it settles.
        let peak = (1..100).map(|i| cubic_bezier(&[0.34, 1.56, 0.64, 1.0], i as f64 / 100.0)).fold(0.0, f64::max);
        assert!(peak > 1.05, "{peak}");
        // Steep starts still solve (Newton gives up at a flat derivative; bisection finishes).
        let y = cubic_bezier(&[1.0, 0.0, 1.0, 0.0], 0.999);
        assert!((0.0..=1.0).contains(&y));
    }

    #[test]
    fn bezier_wins_over_ease_and_bad_ones_are_ignored() {
        let a = key(serde_json::json!({ "t": 0, "v": 0 }));
        let b = key(serde_json::json!({ "t": 1000, "v": 100, "ease": "linear", "bezier": [0.25, 0.1, 0.25, 1] }));
        assert!((keyframe_value(Some(&[a.clone(), b]), 500.0, 0.0) - 80.24).abs() < 0.1);
        // x outside 0..1: not a function of time, so the ease applies.
        let bad = key(serde_json::json!({ "t": 1000, "v": 100, "ease": "linear", "bezier": [1.5, 0, 0.5, 1] }));
        assert_eq!(keyframe_value(Some(&[a, bad]), 500.0, 0.0), 50.0);
    }

    #[test]
    fn keys_round_trip_unchanged() {
        for v in [serde_json::json!({ "t": 0.0, "v": 1.0, "ease": "back" }), serde_json::json!({ "t": 10.0, "v": 2.0, "bezier": [0.1, 0.2, 0.3, 0.4], "zz": 1 })] {
            assert_eq!(serde_json::to_value(key(v.clone())).unwrap(), v);
        }
    }
}
