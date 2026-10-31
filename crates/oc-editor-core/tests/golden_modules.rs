// SPDX-License-Identifier: Apache-2.0
//! Golden replay of the pure module functions (tests/golden/editor/modules.json from tools/golden/editor.ts):
//! transform, viewport, styles, a11y, templates and the whiteboard helpers, over hand-made inputs and every
//! fixtures/templates-seed design. Floats within 1e-9.

mod common;

use std::collections::{BTreeMap, HashMap};

use common::{golden, json_diff, root, short};
use indexmap::IndexMap;
use oc_crdt::board::VoteEntry;
use oc_editor_core::*;
use oc_schema::DesignDoc;
use oc_schema::primitives::{Rect, StoredRect, Xf};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

fn p<T: DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("bad golden input {}: {e}", short(v)))
}

fn j<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap()
}

/// `Map` results are recorded as `[key, value]` entries.
fn entries<V: serde::Serialize>(m: &IndexMap<String, V>) -> Value {
    Value::Array(m.iter().map(|(k, v)| json!([k, v])).collect())
}

fn xy(m: &IndexMap<String, (f64, f64)>) -> Value {
    Value::Array(m.iter().map(|(k, (x, y))| json!([k, { "x": x, "y": y }])).collect())
}

struct Fixtures(BTreeMap<String, (DesignDoc, Value)>);

impl Fixtures {
    fn load() -> Self {
        let dir = root().join("fixtures/templates-seed");
        let mut out = BTreeMap::new();
        for e in std::fs::read_dir(dir).expect("fixtures") {
            let path = e.unwrap().path();
            if path.extension().is_some_and(|x| x == "json") {
                let raw = std::fs::read_to_string(&path).unwrap();
                let doc: DesignDoc = serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                out.insert(path.file_name().unwrap().to_string_lossy().to_string(), (doc, serde_json::from_str(&raw).unwrap()));
            }
        }
        Fixtures(out)
    }

    fn doc(&self, name: &Value) -> &DesignDoc {
        &self.0[name.as_str().unwrap()].0
    }
}

fn run(fx: &Fixtures, fname: &str, a: &[Value]) -> Value {
    match fname {
        "resizeBox" => {
            let min = a.get(4).and_then(Value::as_f64).unwrap_or(4.0);
            j(&resize_box(&p::<Xf>(&a[0]), p(&a[1]), p(&a[2]), a[3].as_bool().unwrap(), min))
        }
        "scaleAbout" => j(&scale_about(&p::<Xf>(&a[0]), p(&a[1]), a[2].as_f64().unwrap())),
        "rotateAbout" => j(&rotate_about(&p::<Xf>(&a[0]), p(&a[1]), a[2].as_f64().unwrap())),
        "cropResize" => {
            let (xf, crop) = crop_resize(&p::<Xf>(&a[0]), &p::<StoredRect>(&a[1]), &p::<Xf>(&a[2]), p(&a[3]));
            json!({ "xf": xf, "crop": crop })
        }
        "designColors" => j(&design_colors(fx.doc(&a[0]))),
        "auditDesign" => j(&audit_design(fx.doc(&a[0]))),
        "minTextPx" => j(&min_text_px(fx.doc(&a[0]))),
        "describeElement" => {
            let d = fx.doc(&a[0]);
            let mut ids: Vec<&String> = d.elements.keys().collect();
            ids.sort();
            Value::Array(ids.into_iter().map(|id| j(&describe_element(d, &d.elements[id]))).collect())
        }
        "paletteMap" => j(&palette_map(fx.doc(&a[0]), &p::<Vec<String>>(&a[1]))),
        "fontSetOps" => j(&font_set_ops(fx.doc(&a[0]), &p::<FontSet>(&a[1]))),
        "recolorOps" => j(&recolor_ops(fx.doc(&a[0]), &p::<IndexMap<String, String>>(&a[1]))),
        "pageSlots" => j(&page_slots(fx.doc(&a[0]))),
        "pageAtScene" => j(&page_at_scene(fx.doc(&a[0]), p(&a[1]))),
        "templatePages" => {
            let target = &a[1];
            let mut rng = Mulberry32::new(a[2].as_u64().unwrap() as u32);
            j(&template_pages(fx.doc(&a[0]), target["w"].as_f64().unwrap(), target["h"].as_f64().unwrap(), &mut rng))
        }
        "scalePatches" => {
            let d = fx.doc(&a[0]);
            let el = &d.elements[a[1].as_str().unwrap()];
            entries(&scale_patches(&d.elements, el, a[2].as_f64().unwrap(), a[3].as_f64().unwrap()))
        }
        "zoomAt" => j(&zoom_at(&p::<Viewport>(&a[0]), a[1].as_f64().unwrap(), p(&a[2]))),
        "fitRect" => j(&fit_rect(&p::<Viewport>(&a[0]), &p::<Rect>(&a[1]), a[2].as_f64().unwrap())),
        "screenToScene" => j(&screen_to_scene(&p::<Viewport>(&a[0]), p(&a[1]))),
        "sceneToScreen" => j(&scene_to_screen(&p::<Viewport>(&a[0]), p(&a[1]))),
        "clampZoom" => j(&clamp_zoom(a[0].as_f64().unwrap())),
        "sectionsOf" => {
            let d = fx.doc(&a[0]);
            Value::Array(sections_of(&d.elements, a[1].as_str().unwrap()).iter().map(|e| json!(e.id())).collect())
        }
        "sectionMembers" => j(&section_members(&fx.doc(&a[0]).elements, a[1].as_str().unwrap())),
        "withSectionMembers" => j(&with_section_members(&fx.doc(&a[0]).elements, &p::<Vec<String>>(&a[1]))),
        "voteScope" => j(&vote_scope(&fx.doc(&a[0]).elements, a[1].as_str().unwrap(), &p::<Vec<String>>(&a[2]))),
        "mind" => {
            let els = &fx.doc(&a[0]).elements;
            let id = a[1].as_str().unwrap();
            let e = els.get(id);
            let (x, y) = mind_child_position(els, id, 60.0);
            json!({
                "isSticky": is_sticky(e), "isSection": is_section(e), "isMindNode": is_mind_node(e),
                "node": mind_node_of(els, id).and_then(|n| n.id()),
                "children": mind_children(els, id).iter().map(|c| c.id()).collect::<Vec<_>>(),
                "root": mind_root(els, id),
                "childPos": { "x": x, "y": y },
            })
        }
        "mindLayout" => {
            let els = &fx.doc(&a[0]).elements;
            let sp = a.get(2).cloned().unwrap_or(Value::Null);
            xy(&mind_layout(els, a[1].as_str().unwrap(), sp["gapX"].as_f64(), sp["gapY"].as_f64()))
        }
        "clusterLayout" => xy(&cluster_layout(&fx.doc(&a[0]).elements, &p::<Vec<String>>(&a[1]), 24.0)),
        "makeTimer" => j(&make_timer(a[0].as_f64().unwrap(), a.get(1).and_then(Value::as_bool).unwrap_or(true))),
        "formatClock" => json!(format_clock(a[0].as_f64().unwrap())),
        "tallyVotes" => {
            let s = &a[0];
            j(&tally_votes(s["id"].as_str().unwrap(), &p::<Vec<String>>(&s["scope"]), &p::<Vec<VoteEntry>>(&a[1])))
        }
        "topVoted" => j(&top_voted(&p::<Vec<VoteTally>>(&a[0]), a[1].as_u64().unwrap() as usize)),
        "votesUsed" => json!(votes_used(a[0]["id"].as_str().unwrap(), &p::<Vec<VoteEntry>>(&a[1]), a[2].as_str().unwrap())),
        "voteAllowance" => {
            let s = &a[0];
            json!(vote_allowance(
                s["id"].as_str().unwrap(),
                s["perPerson"].as_f64().unwrap(),
                s["status"].as_str().unwrap(),
                &p::<Vec<VoteEntry>>(&a[1]),
                a[2].as_str().unwrap()
            ))
        }
        "STICKY_COLORS" => j(&STICKY_COLORS),
        "constants" => json!({ "MIN_ZOOM": MIN_ZOOM, "MAX_ZOOM": MAX_ZOOM, "CORNERS": CORNERS, "EDGES": EDGES, "CLIPBOARD_MIME": CLIPBOARD_MIME }),
        "refitCrop" => {
            let wh = |v: &Value| (v["w"].as_f64().unwrap_or(f64::NAN), v["h"].as_f64().unwrap_or(f64::NAN));
            j(&refit_crop(&p::<StoredRect>(&a[0]), wh(&a[1]), wh(&a[2])))
        }
        "coverCrop" => j(&[(1.0, 2.0), (2.0, 1.0), (1.0, 1.0), (0.5, 0.25)].map(|(x, y)| cover_crop(x, y))),
        "withKeyframe" => {
            let k = |v: Value| -> Map<String, Value> { p(&v) };
            json!([
                with_keyframe(&Map::new(), "x", 0.0, 10.0, None),
                with_keyframe(&Map::new(), "x", 500.0, 10.12345, Some("easeIn")),
                with_keyframe(&k(json!({ "x": [{ "t": 0, "v": 0 }, { "t": 500, "v": 4 }] })), "x", 510.0, 7.0, None),
                with_keyframe(&k(json!({ "opacity": [{ "t": 100, "v": 0.5 }] })), "opacity", 50.0, 1.0, None),
            ])
        }
        other => panic!("modules.json: no Rust replay for {other}"),
    }
}

/// Documented deviation: `designColors` sorts by count; colours with equal counts keep their first-seen order,
/// which follows the input's element and key iteration order. TS sees JS insertion order; the typed `DesignDoc`
/// (serde field order) and yrs snapshots (unordered maps, read sorted) do not keep it. So the counts must match
/// position by position and each run of equal counts must hold the same colours, in any order.
fn same_colors(actual: &Value, expected: &Value) -> Option<String> {
    let runs = |v: &Value| -> Vec<(u64, Vec<String>)> {
        let mut out: Vec<(u64, Vec<String>)> = Vec::new();
        for c in v.as_array().into_iter().flatten() {
            let (n, hex) = (c["count"].as_u64().unwrap_or(0), c["hex"].as_str().unwrap_or("").to_string());
            match out.last_mut() {
                Some((m, hs)) if *m == n => hs.push(hex),
                _ => out.push((n, vec![hex])),
            }
        }
        for (_, hs) in out.iter_mut() {
            hs.sort();
        }
        out
    };
    let (a, e) = (runs(actual), runs(expected));
    (a != e).then(|| format!("colour runs differ:\n  actual:   {a:?}\n  expected: {e:?}"))
}

#[test]
fn replays_module_functions() {
    let g = golden("modules.json");
    let fx = Fixtures::load();
    let cases = g["cases"].as_array().unwrap();
    let mut timer: Option<oc_crdt::board::TimerState> = None;
    let mut by_fn: HashMap<String, (usize, usize)> = HashMap::new();
    let mut failures = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let fname = c["fn"].as_str().unwrap();
        let a = c["args"].as_array().unwrap();
        let out = if fname == "timer" {
            // A running sequence: start / remaining / add / pause / reset over one timer.
            let mut t = timer.take().unwrap_or_else(|| make_timer(5.0 * 60_000.0, true));
            let (op, at, arg) = (a[0].as_str().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap());
            t = match op {
                "start" => timer_start(&t, at),
                "pause" => timer_pause(&t, at),
                "add" => timer_add(&t, arg, at),
                "reset" => timer_reset(&t),
                _ => t,
            };
            let out = json!({ "t": t, "remaining": timer_remaining(&t, at), "running": timer_running(&t, at) });
            timer = Some(t);
            out
        } else {
            run(&fx, fname, a)
        };
        let e = by_fn.entry(fname.to_string()).or_default();
        e.1 += 1;
        let diff = if fname == "designColors" { same_colors(&out, &c["out"]) } else { json_diff(&out, &c["out"], "out") };
        match diff {
            None => e.0 += 1,
            Some(d) => failures.push(format!("case {i} {fname}({}): {d}", short(&c["args"]))),
        }
    }
    let mut names: Vec<_> = by_fn.iter().collect();
    names.sort();
    for (f, (ok, n)) in names {
        eprintln!("{f}: {ok}/{n}");
    }
    eprintln!("module functions: {}/{} cases", cases.len() - failures.len(), cases.len());
    assert!(failures.is_empty(), "{} module case(s) diverged:\n{}", failures.len(), failures.iter().take(25).cloned().collect::<Vec<_>>().join("\n"));
}
