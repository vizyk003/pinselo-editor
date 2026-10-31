// SPDX-License-Identifier: Apache-2.0
//! Motion core (MV-002/003, `packages/render-core/src/motion.ts`). Pure and deterministic: the state of an
//! animated element is a function of (preset, element time) only, so editor preview, Present mode and server
//! MP4/GIF export agree frame-for-frame (ADR-0035: time is an explicit evaluator input; no wall clock).

use oc_schema::design::{Page, TransitionKind};
use oc_schema::elements::{AnimDirection, AnimPreset, AnimUnit, Element, ElementAnim, ElementEmphasis, EmphasisPreset, Keyframes, MotionEasing};
use oc_schema::keyframes::keyframe_value;
use oc_schema::primitives::Extra;
use oc_schema::timeline::Window;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::jsfmt::{cos, hypot, max, min, pow, sin};

const PI: f64 = std::f64::consts::PI;

/// Text revealed by line, word or letter: `shown` units (fractional = the current unit fading in).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Units {
    pub by: AnimUnit,
    pub shown: f64,
    pub stagger: f64,
    pub fade: f64,
}

/// The animated state of an element (multiplied over its own transform and opacity).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionState {
    pub opacity: f64,
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub rot: f64,
    pub blur: f64,
    /// 0..1 reveal for 'wipe' (from the side given by `reveal_from`).
    pub reveal: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reveal_from: Option<AnimDirection>,
    /// Horizontal squash (flip), multiplies `scale` on x.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sx: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<Units>,
    /// Whether this is the shared identity object of the TS (`motion === IDENTITY_MOTION`): the resolver adds
    /// the animation transform only for any other state, even one with identity values.
    #[serde(skip)]
    pub identity: bool,
}

/// `IDENTITY_MOTION` (the shared object: `identity` is set).
pub const IDENTITY_MOTION: MotionState = MotionState {
    opacity: 1.0,
    dx: 0.0,
    dy: 0.0,
    scale: 1.0,
    rot: 0.0,
    blur: 0.0,
    reveal: 1.0,
    reveal_from: None,
    sx: None,
    units: None,
    identity: true,
};

/// `{ ...IDENTITY_MOTION }`: identity values in a new object.
fn fresh() -> MotionState {
    MotionState { identity: false, ..IDENTITY_MOTION }
}

impl MotionState {
    pub fn is_identity_ref(&self) -> bool {
        self.identity
    }
}

fn ease_out_bounce(t: f64) -> f64 {
    let n = 7.5625;
    let d = 2.75;
    if t < 1.0 / d {
        return n * t * t;
    }
    if t < 2.0 / d {
        let t = t - 1.5 / d;
        return n * t * t + 0.75;
    }
    if t < 2.5 / d {
        let t = t - 2.25 / d;
        return n * t * t + 0.9375;
    }
    let t = t - 2.625 / d;
    n * t * t + 0.984375
}

fn dir_vec(d: Option<&AnimDirection>, fallback: AnimDirection) -> (f64, f64) {
    let d = match d {
        Some(d) if d.is_known() => d.clone(),
        _ => fallback,
    };
    match d {
        AnimDirection::Up => (0.0, 1.0),
        AnimDirection::Down => (0.0, -1.0),
        AnimDirection::Left => (1.0, 0.0),
        AnimDirection::Right | AnimDirection::Other(_) => (-1.0, 0.0),
    }
}

/// `1 - (1 - t) ** 3`.
pub fn ease_out_cubic(t: f64) -> f64 {
    1.0 - pow(1.0 - t, 3.0)
}

fn ease_out_back(t: f64) -> f64 {
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * pow(t - 1.0, 3.0) + c1 * pow(t - 1.0, 2.0)
}

/// Direction and intensity of a preset evaluation (`{ direction?, intensity? }`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PresetOpts {
    pub direction: Option<AnimDirection>,
    pub intensity: Option<f64>,
}

impl From<&ElementAnim> for PresetOpts {
    fn from(a: &ElementAnim) -> Self {
        PresetOpts { direction: a.direction.clone(), intensity: a.intensity }
    }
}

/// JS truthiness of an optional direction string.
fn has_dir(d: &Option<AnimDirection>) -> bool {
    d.as_ref().is_some_and(|d| !d.as_str().is_empty())
}

/// State of an element `t` ms into its entrance animation (t ≥ duration → settled). `direction` is where the
/// element comes FROM travelling towards (e.g. 'up' = moves up into place).
pub fn evaluate_preset(preset: &AnimPreset, t: f64, duration_ms: f64, size: (f64, f64), o: &PresetOpts) -> MotionState {
    if t <= 0.0 {
        return MotionState { opacity: 0.0, reveal: 0.0, ..fresh() };
    }
    let p = min(1.0, t / max(1.0, duration_ms));
    if p >= 1.0 {
        return IDENTITY_MOTION;
    }
    let k = min(2.0, max(0.25, o.intensity.unwrap_or(1.0)));
    let e = ease_out_cubic(p);
    let (w, h) = size;
    let travel = max(40.0, min(w, h) * 0.5) * k;
    let (vx, vy) = dir_vec(o.direction.as_ref(), AnimDirection::Up);
    let id = fresh();
    match preset {
        AnimPreset::Fade => MotionState { opacity: e, ..id },
        AnimPreset::Rise => {
            if has_dir(&o.direction) {
                MotionState { opacity: e, dx: (1.0 - e) * travel * vx, dy: (1.0 - e) * travel * vy, ..id }
            } else {
                MotionState { opacity: e, dy: (1.0 - e) * travel, ..id }
            }
        }
        AnimPreset::Drop => MotionState { opacity: e, dy: -(1.0 - e) * travel, ..id },
        AnimPreset::Pan => MotionState { opacity: e, dx: -(1.0 - e) * travel * 1.5, ..id },
        AnimPreset::Slide => {
            // Travels a full box length, no fade: reads as entering from off the element's edge.
            let (sx, sy) = dir_vec(o.direction.as_ref(), AnimDirection::Left);
            MotionState { dx: (1.0 - e) * w * 1.2 * k * sx, dy: (1.0 - e) * h * 1.2 * k * sy, opacity: min(1.0, p * 4.0), ..id }
        }
        AnimPreset::Pop => MotionState { opacity: min(1.0, p * 3.0), scale: max(0.01, 1.0 + (ease_out_back(p) - 1.0) * k), ..id },
        AnimPreset::Zoom => MotionState { opacity: e, scale: 1.0 + 0.6 * k * (1.0 - e), ..id },
        AnimPreset::Shrink => MotionState { opacity: e, scale: max(0.05, 1.0 - 0.5 * k * (1.0 - e)), ..id },
        AnimPreset::Stomp => {
            // Slams down from large with a small squash at impact.
            let s = if p < 0.6 {
                1.0 + 1.4 * k * (1.0 - ease_out_cubic(p / 0.6))
            } else {
                1.0 - 0.06 * k * sin(((p - 0.6) / 0.4) * PI)
            };
            MotionState { opacity: min(1.0, p * 4.0), scale: s, ..id }
        }
        AnimPreset::Bounce => MotionState {
            opacity: min(1.0, p * 3.0),
            dx: (1.0 - ease_out_bounce(p)) * travel * vx,
            dy: (1.0 - ease_out_bounce(p)) * travel * if has_dir(&o.direction) { vy } else { -1.0 },
            ..id
        },
        AnimPreset::Float => MotionState { opacity: e, dy: (1.0 - e) * travel * 0.3, ..id },
        AnimPreset::Wipe => MotionState { reveal: e, reveal_from: Some(o.direction.clone().unwrap_or(AnimDirection::Right)), ..id },
        AnimPreset::Blur => MotionState { opacity: e, blur: (1.0 - e) * 24.0 * k, ..id },
        AnimPreset::Flip => MotionState { opacity: min(1.0, p * 2.0), sx: Some(max(0.02, cos((1.0 - e) * PI / 2.0 * k).abs())), ..id },
        AnimPreset::Spin => MotionState { opacity: e, rot: (1.0 - e) * -360.0 * k, scale: 0.3 + 0.7 * e, ..id },
        AnimPreset::Tumble => MotionState { opacity: e, rot: (1.0 - e) * -90.0 * k, scale: 0.6 + 0.4 * e, ..id },
        AnimPreset::Breathe => MotionState { opacity: e, scale: 1.0 - 0.15 * k * (1.0 - e), ..id },
        // Text elements reveal letter by letter (see `units_at`); other elements just appear.
        AnimPreset::Typewriter => MotionState { opacity: if p > 0.0 { 1.0 } else { 0.0 }, ..id },
        // A preset this build does not know: settled.
        AnimPreset::Other(_) => IDENTITY_MOTION,
    }
}

/// Looping emphasis state (multiplied over the entrance), `t` ms after the emphasis starts.
pub fn evaluate_emphasis(em: &ElementEmphasis, t: f64, size: (f64, f64)) -> MotionState {
    if t <= 0.0 {
        return IDENTITY_MOTION;
    }
    let k = min(2.0, max(0.25, em.intensity.unwrap_or(1.0)));
    let period = max(200.0, em.period_ms);
    let ph = ((t % period) / period) * PI * 2.0;
    let amp = max(6.0, min(size.0, size.1) * 0.06) * k;
    let id = fresh();
    match em.preset {
        EmphasisPreset::Pulse => MotionState { scale: 1.0 + 0.06 * k * (0.5 - 0.5 * cos(ph)), ..id },
        EmphasisPreset::Heartbeat => {
            let x = ph / (PI * 2.0);
            let beat = |c: f64| max(0.0, 1.0 - (x - c).abs() / 0.08);
            MotionState { scale: 1.0 + 0.1 * k * max(beat(0.1), beat(0.3) * 0.7), ..id }
        }
        EmphasisPreset::Wiggle => MotionState { rot: sin(ph * 2.0) * 6.0 * k * max(0.0, sin(ph / 2.0)), ..id },
        EmphasisPreset::Swing => MotionState { rot: sin(ph) * 10.0 * k, ..id },
        EmphasisPreset::Bounce => MotionState { dy: -sin(ph / 2.0).abs() * amp, ..id },
        EmphasisPreset::Float => MotionState { dy: sin(ph) * amp * 0.6, ..id },
        EmphasisPreset::Flicker => MotionState {
            opacity: 0.55 + 0.45 * (if sin(ph * 3.0) > -0.6 { 1.0 } else { 0.0 }) * (0.85 + 0.15 * sin(ph * 7.0)),
            ..id
        },
        EmphasisPreset::Spin => MotionState { rot: (ph / (PI * 2.0)) * 360.0, ..id },
        EmphasisPreset::Other(_) => IDENTITY_MOTION,
    }
}

fn compose(a: &MotionState, b: &MotionState) -> MotionState {
    MotionState {
        opacity: a.opacity * b.opacity,
        dx: a.dx + b.dx,
        dy: a.dy + b.dy,
        scale: a.scale * b.scale,
        rot: a.rot + b.rot,
        blur: a.blur + b.blur,
        reveal: min(a.reveal, b.reveal),
        reveal_from: if a.reveal < 1.0 { a.reveal_from.clone() } else { b.reveal_from.clone() },
        sx: if a.sx.is_some() || b.sx.is_some() { Some(a.sx.unwrap_or(1.0) * b.sx.unwrap_or(1.0)) } else { None },
        units: a.units.clone().or_else(|| b.units.clone()),
        identity: false,
    }
}

/// Motion state from keyframes at page time `t`.
pub fn keyframe_motion(k: &Keyframes, t: f64) -> MotionState {
    MotionState {
        dx: keyframe_value(k.x.as_deref(), t, 0.0),
        dy: keyframe_value(k.y.as_deref(), t, 0.0),
        scale: max(0.001, keyframe_value(k.scale.as_deref(), t, 1.0)),
        rot: keyframe_value(k.rotate.as_deref(), t, 0.0),
        opacity: max(0.0, min(1.0, keyframe_value(k.opacity.as_deref(), t, 1.0))),
        blur: max(0.0, keyframe_value(k.blur.as_deref(), t, 0.0)),
        ..fresh()
    }
}

/// `Object.values(el.keyframes).some((k) => k?.length)`.
fn has_keys(k: &Keyframes) -> bool {
    let known = [&k.x, &k.y, &k.scale, &k.rotate, &k.opacity, &k.blur].iter().any(|v| v.as_ref().is_some_and(|v| !v.is_empty()));
    known || extra_has_length(&k.extra)
}

fn extra_has_length(extra: &Extra) -> bool {
    extra.values().any(|v| match v {
        Value::Array(a) => !a.is_empty(),
        Value::String(s) => !s.is_empty(),
        _ => false,
    })
}

/// Text by unit: how many units are visible `t` ms into the entrance (fractional = the current unit fading in).
fn units_at(a: &ElementAnim, t: f64) -> Option<Units> {
    let by = if a.preset == AnimPreset::Typewriter {
        match &a.by {
            Some(b) if !b.as_str().is_empty() && *b != AnimUnit::Element => b.clone(),
            _ => AnimUnit::Char,
        }
    } else {
        a.by.clone()?
    };
    if by.as_str().is_empty() || by == AnimUnit::Element {
        return None;
    }
    let default = match by {
        AnimUnit::Char => 45.0,
        AnimUnit::Word => 140.0,
        _ => 320.0,
    };
    let stagger = max(10.0, a.stagger_ms.unwrap_or(default));
    let fade = if a.preset == AnimPreset::Typewriter { 0.0 } else { max(1.0, min(a.duration_ms, stagger * 3.0)) };
    Some(Units { by, shown: max(0.0, t / stagger), stagger, fade })
}

/// The animation that applies to an element: its own, else the page animation staggered by paint order.
pub fn animation_for(el: &Element, page: &Page, index: usize) -> Option<ElementAnim> {
    if let Some(a) = el.base().and_then(|b| b.anim.as_ref()) {
        return Some(a.clone());
    }
    page.anim.as_ref().map(|pa| ElementAnim {
        preset: pa.preset.clone(),
        duration_ms: pa.duration_ms,
        delay_ms: index as f64 * 120.0,
        direction: None,
        intensity: None,
        by: None,
        stagger_ms: None,
        on_click: None,
        extra: Extra::new(),
    })
}

fn size_of(el: &Element) -> (f64, f64) {
    el.xf().map_or((f64::NAN, f64::NAN), |x| (x.w, x.h))
}

pub fn motion_at(el: &Element, page: &Page, index: usize, page_time_ms: f64) -> MotionState {
    match animation_for(el, page, index) {
        Some(a) => entrance(el, &a, page_time_ms - a.delay_ms),
        None => IDENTITY_MOTION,
    }
}

/// Entrance state; text animated by unit keeps the box still and reveals units instead.
fn entrance(el: &Element, a: &ElementAnim, t: f64) -> MotionState {
    let units = if matches!(el, Element::Text(_)) { units_at(a, t) } else { None };
    if let Some(units) = units {
        return if t <= 0.0 { MotionState { opacity: 0.0, ..fresh() } } else { MotionState { units: Some(units), ..fresh() } };
    }
    evaluate_preset(&a.preset, t, a.duration_ms, size_of(el), &PresetOpts::from(a))
}

/// How long an entrance takes to finish (text by unit: every unit plus its fade).
pub fn entrance_ms(el: &Element, a: &ElementAnim, unit_count: impl FnOnce() -> f64) -> f64 {
    let u = if matches!(el, Element::Text(_)) { units_at(a, 0.0) } else { None };
    match u {
        Some(u) => a.delay_ms + unit_count() * u.stagger + u.fade,
        None => a.delay_ms + a.duration_ms,
    }
}

/// Full motion state at page time `t`, honouring the element's visibility window, entrance, exit and motion
/// path. `None` when the element is not visible at `t`.
pub fn motion_at_time(el: &Element, page: &Page, index: usize, t: f64, window: Window, click_steps: Option<&[f64]>) -> Option<MotionState> {
    if t < window.start || t >= window.end {
        return None;
    }
    let mut local = t - window.start;
    let mut m = IDENTITY_MOTION;
    let a = animation_for(el, page, index);
    if let Some(on_click) = a.as_ref().and_then(|a| a.on_click).filter(|c| crate::jsfmt::truthy(*c)) {
        // Build steps: Present passes when each click happened; exports play steps 1.2 s apart.
        let at = match click_steps {
            Some(steps) => {
                let i = on_click - 1.0;
                if i >= 0.0 && i.fract() == 0.0 && (i as usize) < steps.len() { Some(steps[i as usize]) } else { None }
            }
            None => Some(on_click * 1200.0),
        };
        let at = at?;
        local -= at;
        if local < 0.0 {
            return None;
        }
    }
    if let Some(a) = &a {
        m = entrance(el, a, local - a.delay_ms);
    }
    let base = el.base();
    if let Some(k) = base.and_then(|b| b.keyframes.as_ref())
        && has_keys(k)
    {
        m = compose(&m, &keyframe_motion(k, t));
    }
    if let Some(em) = base.and_then(|b| b.emphasis.as_ref()) {
        // Emphasis loops once the entrance has finished.
        let start = a.as_ref().map_or(0.0, |a| a.delay_ms + a.duration_ms);
        if local > start {
            m = compose(&m, &evaluate_emphasis(em, local - start, size_of(el)));
        }
    }
    if let Some(out) = base.and_then(|b| b.anim_out.as_ref()) {
        let left = window.end - t;
        if left < out.duration_ms {
            let o = evaluate_preset(&out.preset, left, out.duration_ms, size_of(el), &PresetOpts::default());
            let units = m.units.clone();
            m = MotionState { units, ..compose(&m, &MotionState { dx: -o.dx, dy: -o.dy, ..o }) };
        }
    }
    if let Some(mp) = base.and_then(|b| b.motion_path.as_ref())
        && mp.points.len() >= 4
    {
        let mut p = min(1.0, max(0.0, (local - mp.delay_ms) / max(1.0, mp.duration_ms)));
        if mp.easing == MotionEasing::Ease {
            p = if p < 0.5 { 2.0 * p * p } else { 1.0 - pow(-2.0 * p + 2.0, 2.0) / 2.0 };
        }
        let (dx, dy) = point_along(&mp.points, p);
        m = MotionState { dx: m.dx + dx, dy: m.dy + dy, identity: false, ..m };
    }
    Some(m)
}

/// Point at fraction p (0..1) along a flat [x0,y0,x1,y1,…] polyline, by arc length.
pub fn point_along(pts: &[f64], p: f64) -> (f64, f64) {
    let mut segs: Vec<f64> = Vec::new();
    let mut total = 0.0;
    let mut i = 2;
    while i + 1 < pts.len() {
        let l = hypot(pts[i] - pts[i - 2], pts[i + 1] - pts[i - 1]);
        segs.push(l);
        total += l;
        i += 2;
    }
    if total == 0.0 {
        return (pts.first().copied().unwrap_or(0.0), pts.get(1).copied().unwrap_or(0.0));
    }
    let mut d = p * total;
    for (k, &l) in segs.iter().enumerate() {
        if d <= l || k == segs.len() - 1 {
            let f = if l != 0.0 && !l.is_nan() { min(1.0, d / l) } else { 0.0 };
            let i = k * 2;
            return (pts[i] + (pts[i + 2] - pts[i]) * f, pts[i + 1] + (pts[i + 3] - pts[i + 1]) * f);
        }
        d -= l;
    }
    (pts[pts.len() - 2], pts[pts.len() - 1])
}

/// When every entrance on the page has settled (used by Present mode and video export).
pub fn page_settle_ms(page: &Page, els: &[&Element]) -> f64 {
    let mut end: f64 = 0.0;
    for (i, el) in els.iter().enumerate() {
        if let Some(a) = animation_for(el, page, i) {
            let click = a.on_click.filter(|c| crate::jsfmt::truthy(*c)).map_or(0.0, |c| c * 1200.0);
            end = max(end, entrance_ms(el, &a, || text_unit_count(el, &a)) + click);
        }
    }
    end
}

/// Number of animation units in a text element (lines = paragraphs).
pub fn text_unit_count(el: &Element, a: &ElementAnim) -> f64 {
    let Element::Text(t) = el else { return 1.0 };
    let by = if a.preset == AnimPreset::Typewriter && a.by.as_ref().is_none_or(|b| b.as_str().is_empty() || *b == AnimUnit::Element) {
        Some(AnimUnit::Char)
    } else {
        a.by.clone()
    };
    let text: Vec<String> = t.body.paragraphs.iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect()).collect();
    match by {
        Some(AnimUnit::Line) => text.len() as f64,
        Some(AnimUnit::Word) => {
            let joined = text.join(" ");
            joined.split(oc_schema::js::is_js_space).filter(|s| !s.is_empty()).count() as f64
        }
        Some(AnimUnit::Char) => {
            let joined = text.concat();
            joined.chars().filter(|c| !oc_schema::js::is_js_space(*c)).map(char::len_utf16).sum::<usize>() as f64
        }
        _ => 1.0,
    }
}

/// Transition overlay progress for a page that started `t` ms ago (1 = fully visible).
pub fn transition_progress(page: &Page, t: f64) -> f64 {
    match &page.transition {
        None => 1.0,
        Some(tr) => ease_out_cubic(min(1.0, max(0.0, t / max(1.0, tr.duration_ms)))),
    }
}

/// Visible region of a transition layer, in page fractions.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TransitionClip {
    Rect { x: f64, y: f64, w: f64, h: f64 },
    Circle { r: f64 },
}

/// How to draw one side of a page transition, in fractions of the page size. Present mode and video export both
/// draw from this, so they always agree.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransitionLayer {
    pub alpha: f64,
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub rot: f64,
    pub sx: f64,
    pub blur: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<TransitionClip>,
}

impl Default for TransitionLayer {
    fn default() -> Self {
        TransitionLayer { alpha: 1.0, dx: 0.0, dy: 0.0, scale: 1.0, rot: 0.0, sx: 1.0, blur: 0.0, clip: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionFrame {
    pub prev: Option<TransitionLayer>,
    pub next: TransitionLayer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_first: Option<bool>,
}

/// Layers at eased progress `p` (0..1) of a transition.
pub fn transition_frame(kind: &TransitionKind, p: f64, direction: Option<&AnimDirection>) -> TransitionFrame {
    let (vx, vy) = dir_vec(direction, AnimDirection::Left);
    let l = TransitionLayer::default;
    let fr = |prev: Option<TransitionLayer>, next: TransitionLayer| TransitionFrame { prev, next, next_first: None };
    let rect = |x: f64, y: f64, w: f64, h: f64| Some(TransitionClip::Rect { x, y, w, h });
    match kind {
        TransitionKind::Fade | TransitionKind::Other(_) => fr(None, TransitionLayer { alpha: p, ..l() }),
        TransitionKind::Dissolve => fr(Some(l()), TransitionLayer { alpha: p, ..l() }),
        TransitionKind::Slide | TransitionKind::Push => {
            fr(Some(TransitionLayer { dx: -p * vx, dy: -p * vy, ..l() }), TransitionLayer { dx: (1.0 - p) * vx, dy: (1.0 - p) * vy, ..l() })
        }
        TransitionKind::Cover => fr(Some(l()), TransitionLayer { dx: (1.0 - p) * vx, dy: (1.0 - p) * vy, ..l() }),
        TransitionKind::Reveal => TransitionFrame { prev: Some(TransitionLayer { dx: -p * vx, dy: -p * vy, ..l() }), next: l(), next_first: Some(true) },
        TransitionKind::Zoom => fr(None, TransitionLayer { alpha: p, scale: 0.85 + 0.15 * p, ..l() }),
        TransitionKind::Wipe => {
            let clip = if vx > 0.0 {
                rect(1.0 - p, 0.0, p, 1.0)
            } else if vx < 0.0 {
                rect(0.0, 0.0, p, 1.0)
            } else if vy > 0.0 {
                rect(0.0, 1.0 - p, 1.0, p)
            } else {
                rect(0.0, 0.0, 1.0, p)
            };
            fr(Some(l()), TransitionLayer { clip, ..l() })
        }
        TransitionKind::Iris => fr(Some(l()), TransitionLayer { clip: Some(TransitionClip::Circle { r: p * 0.75 }), ..l() }),
        TransitionKind::Split => {
            let clip = if vy != 0.0 { rect(0.0, 0.5 - p / 2.0, 1.0, p) } else { rect(0.5 - p / 2.0, 0.0, p, 1.0) };
            fr(Some(l()), TransitionLayer { clip, ..l() })
        }
        TransitionKind::Flip => {
            if p < 0.5 {
                fr(Some(TransitionLayer { sx: max(0.01, 1.0 - p * 2.0), ..l() }), TransitionLayer { alpha: 0.0, ..l() })
            } else {
                fr(None, TransitionLayer { sx: max(0.01, p * 2.0 - 1.0), ..l() })
            }
        }
        TransitionKind::Blur => {
            fr(Some(TransitionLayer { blur: p * 24.0, alpha: 1.0 - p, ..l() }), TransitionLayer { alpha: p, blur: (1.0 - p) * 24.0, ..l() })
        }
        TransitionKind::Spin => fr(
            Some(TransitionLayer { alpha: 1.0 - p, ..l() }),
            TransitionLayer { alpha: p, rot: (1.0 - p) * -180.0, scale: 0.4 + 0.6 * p, ..l() },
        ),
        TransitionKind::Drop => fr(Some(l()), TransitionLayer { dy: -(1.0 - ease_out_bounce(p)), ..l() }),
    }
}

/// Number of click build steps on a page (0 = none).
pub fn click_step_count(els: &[&Element]) -> f64 {
    els.iter().fold(0.0, |n, e| max(n, e.base().and_then(|b| b.anim.as_ref()).and_then(|a| a.on_click).unwrap_or(0.0)))
}
