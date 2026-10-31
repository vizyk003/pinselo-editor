// SPDX-License-Identifier: Apache-2.0
//! Text animated by line, word or letter (MV-005, `packages/render-core/src/textUnits.ts`): the layout never
//! moves; units that haven't appeared yet are drawn transparent, and the current one fades in. Works identically
//! in the editor preview, Present mode and video export because it is just a re-coloured text body.

use oc_schema::elements::AnimUnit;
use oc_schema::primitives::{ColorValue, Paint};
use oc_schema::text::{Run, TextBody, resolve_run_style};

use crate::jsfmt::{max, min, num_to_string, round};
use crate::motion::Units;

/// `{ ...c, a: c.a * k }`.
pub(crate) fn fade_color(c: &ColorValue, k: f64) -> ColorValue {
    let mut c = c.clone();
    match &mut c {
        ColorValue::Srgb(x) | ColorValue::DisplayP3(x) => x.a *= k,
        ColorValue::Cmyk(x) => x.a *= k,
        ColorValue::Spot(x) => x.a *= k,
    }
    c
}

fn fade_paint(p: &Paint, k: f64) -> Paint {
    let mut p = p.clone();
    match &mut p {
        Paint::Solid(s) => s.color = fade_color(&s.color, k),
        Paint::Linear(l) => l.stops.iter_mut().for_each(|s| s.color = fade_color(&s.color, k)),
        Paint::Radial(r) => r.stops.iter_mut().for_each(|s| s.color = fade_color(&s.color, k)),
        Paint::Image(_) => {}
    }
    p
}

/// The body with per-unit opacity applied, plus a cache key for the layout (quantised).
pub fn reveal_units(body: &TextBody, u: &Units) -> (TextBody, String) {
    let alpha = |i: f64| -> f64 {
        if u.fade <= 0.0 {
            return if i < u.shown.floor() { 1.0 } else { 0.0 };
        }
        min(1.0, max(0.0, ((u.shown - i) * u.stagger) / u.fade))
    };
    let mut unit: f64 = -1.0;
    let mut prev_space = true;
    let q = |a: f64| round(a * 8.0) / 8.0;
    let mut out = body.clone();
    for (pi, p) in body.paragraphs.iter().enumerate() {
        if u.by == AnimUnit::Line {
            unit = pi as f64;
        }
        let mut runs: Vec<Run> = Vec::new();
        for r in &p.runs {
            let fill = resolve_run_style(body, r).fill;
            let mut seg = String::new();
            let mut seg_a: f64 = -1.0;
            let flush = |seg: &mut String, seg_a: f64, runs: &mut Vec<Run>| {
                if seg.is_empty() {
                    return;
                }
                let mut run = r.clone();
                run.text = std::mem::take(seg);
                run.style.fill = Some(if seg_a >= 1.0 { fill.clone() } else { fade_paint(&fill, seg_a) });
                runs.push(run);
            };
            for ch in r.text.chars() {
                let space = oc_schema::js::is_js_space(ch);
                if u.by == AnimUnit::Word && !space && prev_space {
                    unit += 1.0;
                }
                if u.by == AnimUnit::Char && !space {
                    unit += 1.0;
                }
                prev_space = space;
                let a = q(alpha(max(0.0, unit)));
                if a != seg_a {
                    flush(&mut seg, seg_a, &mut runs);
                    seg_a = a;
                }
                seg.push(ch);
            }
            flush(&mut seg, seg_a, &mut runs);
        }
        if u.by == AnimUnit::Word {
            prev_space = true;
        }
        if !runs.is_empty() {
            out.paragraphs[pi].runs = runs;
        }
    }
    let key = format!("u{}{}", u.by.as_str(), num_to_string((u.shown * 8.0).floor()));
    (out, key)
}
