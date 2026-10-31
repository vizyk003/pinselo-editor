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
            let e = b.ease.clone().unwrap_or(KeyframeEase::Ease);
            return a.v + (b.v - a.v) * ease(&e, p);
        }
    }
    ks[ks.len() - 1].v
}
