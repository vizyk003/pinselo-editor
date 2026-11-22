// SPDX-License-Identifier: Apache-2.0
//! Keyframe interpolation (MV-100), shared by the renderer (playback/export) and the editor (recording).

use crate::elements::{Keyframe, KeyframeEase};

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

pub fn keyframe_neutral(prop: &str) -> Option<f64> {
    KEYFRAME_NEUTRAL.iter().find(|(k, _)| *k == prop).map(|(_, v)| *v)
}

/// Interpolated value of a keyframe track at page time `t` (before the first key → first value, after the
/// last → last).
pub fn keyframe_value(keys: Option<&[Keyframe]>, t: f64, neutral: f64) -> f64 {
    let Some(keys) = keys.filter(|k| !k.is_empty()) else { return neutral };
    let unsorted = keys.len() > 1 && keys.windows(2).any(|w| w[1].t < w[0].t);
    let sorted: Vec<&Keyframe>;
    let ks: Vec<&Keyframe> = if unsorted {
        sorted = {
            let mut v: Vec<&Keyframe> = keys.iter().collect();
            v.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
            v
        };
        sorted
    } else {
        keys.iter().collect()
    };
    if t <= ks[0].t {
        return ks[0].v;
    }
    for i in 1..ks.len() {
        let (a, b) = (ks[i - 1], ks[i]);
        if t <= b.t {
            let p = (t - a.t) / (b.t - a.t).max(1.0);
            let shaped = match b.bezier.as_ref().filter(|z| bezier_ok(z)) {
                Some(z) => cubic_bezier(z, p),
                None => ease(&b.ease.clone().unwrap_or(KeyframeEase::Ease), p),
            };
            return a.v + (b.v - a.v) * shaped;
        }
    }
    ks[ks.len() - 1].v
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
