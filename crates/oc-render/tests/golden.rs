// SPDX-License-Identifier: Apache-2.0
//! render-core parity: replays tests/golden/render/*.json (generated from the TS by tools/golden/render.ts) against
//! the Rust port — display lists of every template page (settled and animated), the hand-made scenes under every
//! recorded option set, and the pure helpers. Structure and strings must match exactly, floats within 1e-9.

mod common;

use std::collections::HashMap;

use common::*;
use oc_render::color_matrix::{adjust_matrix, duotone_matrix, edit_matrix, needs_tone_shader};
use oc_render::motion::{self, PresetOpts, Units};
use oc_render::{ResolveOptions, resolve_page};
use oc_schema::DesignDoc;
use oc_schema::design::{Page, TransitionKind};
use oc_schema::elements::{AnimDirection, AnimPreset, Element, ElementAnim};
use oc_schema::timeline::Window;
use serde_json::{Value, json};

fn doc_of(v: &Value) -> DesignDoc {
    de(v)
}

#[test]
fn templates_settled() {
    let cases = golden("templates");
    let mut docs: HashMap<String, DesignDoc> = HashMap::new();
    let mut fails = Failures::default();
    for c in cases.as_array().expect("cases") {
        let file = s(&c["file"]);
        let doc = docs.entry(file.to_owned()).or_insert_with(|| doc_of(&fixture(file)));
        let page = s(&c["page"]);
        let dl = resolve_page(doc, page, &ResolveOptions::default());
        fails.check(|| format!("{file} {page}"), &j(&dl), &c["dl"]);
    }
    let n = fails.finish("templates");
    assert!(n >= 296, "expected every template page, got {n}");
}

#[test]
fn templates_animated() {
    let cases = golden("animated");
    let mut fails = Failures::default();
    for c in cases.as_array().expect("cases") {
        let file = s(&c["file"]);
        let page = s(&c["page"]);
        let mut raw = fixture(file);
        merge(&mut raw["pages"][page], &c["patch"]["page"]);
        for (id, p) in c["patch"]["elements"].as_object().expect("elements") {
            merge(&mut raw["elements"][id], p);
        }
        let doc = doc_of(&raw);
        let steps: Option<Vec<f64>> = c["clickSteps"].as_array().map(|a| a.iter().map(f).collect());
        for k in c["cases"].as_array().expect("times") {
            let t = f(&k["timeMs"]);
            let opts = ResolveOptions { time_ms: Some(t), click_steps: steps.clone(), ..Default::default() };
            let dl = resolve_page(&doc, page, &opts);
            fails.check(|| format!("{file} {page} t={t}"), &j(&dl), &k["dl"]);
        }
    }
    fails.finish("animated templates");
}

#[test]
fn scenes() {
    let scenes = golden("scenes");
    let mut fails = Failures::default();
    let mut names = Vec::new();
    for sc in scenes.as_array().expect("scenes") {
        let name = s(&sc["name"]);
        names.push(name.to_owned());
        let doc = doc_of(&sc["doc"]);
        for c in sc["cases"].as_array().expect("cases") {
            let opts: ResolveOptions = de(&c["opts"]);
            let page = s(&c["page"]);
            let dl = resolve_page(&doc, page, &opts);
            fails.check(|| format!("{name} {page} {}", c["opts"]), &j(&dl), &c["dl"]);
        }
    }
    fails.finish("scenes");
    for want in ["shapes", "effects", "lines", "text", "images", "media", "charts", "tables", "animation", "captions"] {
        assert!(names.iter().any(|n| n == want), "scene {want} missing");
    }
}

fn size(v: &Value) -> (f64, f64) {
    (f(&v["w"]), f(&v["h"]))
}

fn popts(v: &Value) -> PresetOpts {
    PresetOpts { direction: v.get("direction").map(|d| AnimDirection::parse(s(d))), intensity: v.get("intensity").map(f), ..Default::default() }
}

fn opt_dir(v: &Value) -> Option<AnimDirection> {
    if is_undef(v) { None } else { Some(AnimDirection::parse(s(v))) }
}

#[test]
fn units() {
    let cases = golden("units");
    let mut fails = Failures::default();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for c in cases.as_array().expect("cases") {
        let name = s(&c["fn"]);
        *seen.entry(name.to_owned()).or_default() += 1;
        let a = c["args"].as_array().expect("args");
        let expected = c.get("out").cloned().unwrap_or_else(|| json!({ "error": c["error"] }));
        let got: Value = match name {
            "evaluatePreset" => j(&motion::evaluate_preset(&AnimPreset::parse(s(&a[0])), f(&a[1]), f(&a[2]), size(&a[3]), &popts(&a[4]))),
            "evaluateEmphasis" => j(&motion::evaluate_emphasis(&de(&a[0]), f(&a[1]), size(&a[2]))),
            "keyframeMotion" => j(&motion::keyframe_motion(&de(&a[0]), f(&a[1]))),
            "transitionFrame" => j(&motion::transition_frame(&TransitionKind::parse(s(&a[0])), f(&a[1]), opt_dir(&a[2]).as_ref())),
            "transitionProgress" => j(&motion::transition_progress(&de::<Page>(&a[0]), f(&a[1]))),
            "pointAlong" => {
                let pts: Vec<f64> = a[0].as_array().expect("pts").iter().map(f).collect();
                let (x, y) = motion::point_along(&pts, f(&a[1]));
                j(&[x, y])
            }
            "niceScale" => {
                let (max, step) = oc_render::builders::nice_scale(f(&a[0]), f(&a[1]));
                json!({ "max": num(max), "step": num(step) })
            }
            "smoothPath" => {
                let pts: Vec<[f64; 2]> = de(&a[0]);
                json!(oc_render::builders::smooth_path(&pts))
            }
            "coverCropFor" => j(&oc_render::cover_crop_for(f(&a[0]), f(&a[1]))),
            "refitCropFor" => {
                let wh = |v: &Value| (f(&v["w"]), f(&v["h"]));
                j(&oc_render::refit_crop_for(&de(&a[0]), wh(&a[1]), wh(&a[2])))
            }
            "textUnitCount" => num(motion::text_unit_count(&de::<Element>(&a[0]), &de::<ElementAnim>(&a[1]))),
            "entranceMs" => {
                let el: Element = de(&a[0]);
                let an: ElementAnim = de(&a[1]);
                num(motion::entrance_ms(&el, &an, || motion::text_unit_count(&el, &an)))
            }
            "pageSettleMs" => {
                let els: Vec<Element> = de(&a[1]);
                num(motion::page_settle_ms(&de::<Page>(&a[0]), &els.iter().collect::<Vec<_>>()))
            }
            "clickStepCount" => {
                let els: Vec<Element> = de(&a[0]);
                num(motion::click_step_count(&els.iter().collect::<Vec<_>>()))
            }
            "revealUnits" => {
                let u: Units = de(&a[1]);
                let (body, key) = oc_render::text_units::reveal_units(&de(&a[0]), &u);
                json!({ "body": j(&body), "key": key })
            }
            "motionAt" => {
                let el: Element = de(&a[0]);
                let page: Page = de(&a[1]);
                j(&motion::motion_at(&el, &page, a[2].as_u64().expect("index") as usize, f(&a[3])))
            }
            "motionAtTime" => {
                let el: Element = de(&a[0]);
                let page: Page = de(&a[1]);
                let w = Window { start: f(&a[4]["start"]), end: f(&a[4]["end"]) };
                let steps: Option<Vec<f64>> = if is_undef(&a[5]) { None } else { Some(de(&a[5])) };
                let i = a[2].as_u64().expect("index") as usize;
                j(&motion::motion_at_time(&el, &page, i, f(&a[3]), w, steps.as_deref()))
            }
            "qr" => {
                let ecc = oc_render::qr::Ecc::parse(s(&a[1]));
                match oc_render::qr::create(s(&a[0]), ecc) {
                    Ok(q) if c.get("out").is_some_and(|o| o.is_object()) => json!({
                        "size": q.size,
                        "modules": q.modules.iter().map(|m| char::from(b'0' + m)).collect::<String>(),
                        "version": q.version,
                        "mask": q.mask,
                    }),
                    Ok(q) => json!(q.version),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "adjustMatrix" => j(&adjust_matrix(if is_undef(&a[0]) { None } else { Some(de(&a[0])) }.as_ref()).to_vec()),
            "editMatrix" => {
                let e: Option<oc_schema::elements::ImageEditStack> = if is_undef(&a[0]) { None } else { Some(de(&a[0])) };
                j(&edit_matrix(e.as_ref()).map(|m| m.to_vec()))
            }
            "duotoneMatrix" => j(&duotone_matrix(&de(&a[0])).to_vec()),
            "needsToneShader" => {
                let adj: Option<oc_schema::elements::ImageAdjust> = if is_undef(&a[0]) { None } else { Some(de(&a[0])) };
                json!(needs_tone_shader(adj.as_ref()))
            }
            "sliceColor" => j(&oc_render::builders::slice_color(&de(&a[0]), a[1].as_u64().expect("i") as usize)),
            "toLocaleString" => json!(oc_render::jsfmt::to_locale_fixed(f(&a[0]), a[1].as_u64().expect("d") as usize)),
            "parseFloat" => num(oc_render::jsfmt::parse_float(s(&a[0]))),
            other => panic!("unknown golden fn {other}"),
        };
        fails.check(|| format!("{name}({})", truncate(&Value::Array(a.clone()).to_string())), &got, &expected);
    }
    fails.finish("units");
    for want in ["evaluatePreset", "evaluateEmphasis", "transitionFrame", "motionAt", "motionAtTime", "qr", "editMatrix", "revealUnits", "toLocaleString"] {
        assert!(seen.get(want).copied().unwrap_or(0) > 0, "{want} not covered");
    }
}

#[test]
fn path_data_tolerance() {
    // The goldens round path coordinates to 1e-6 (tools/golden/render.ts `canonPath`); the port prints them unrounded.
    assert!(path_data_eq("M0 0 L815.43168 0 Z", "M0 0 L815.4316799999998 0 Z"));
    assert!(path_data_eq("M0 0 L0 2", "M-0 1e-9 L0 2"));
    assert!(!path_data_eq("M0 0 L815.4317 0 Z", "M0 0 L815.43168 0 Z"));
    assert!(!path_data_eq("M0 0 L1 0 Z", "M0 0 C1 0 Z"));
    assert!(!path_data_eq("M0 0 L1 0", "M0 0 L1 0 Z"));
    assert!(!path_data_eq("Monday 1", "Monday 1.0000001"));
}
