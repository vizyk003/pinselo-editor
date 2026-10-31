// SPDX-License-Identifier: Apache-2.0
//! Scene goldens: hand-built docs (nested rotated/flipped groups, connectors, hidden/locked elements) and docs
//! derived from every fixtures/templates-seed/*.json (with synthetic groups + connectors patched in). Replays every
//! recorded query — world matrices, quads, bounds, hit-testing, marquee, anchors, connector routing, shape and
//! frame outlines, line markers — against the Rust port.

mod common;

use std::collections::HashSet;

use common::*;
use oc_geometry::*;
use oc_schema::elements::{AnchorSide, Element};
use oc_schema::primitives::{Rect, Vec2};
use oc_schema::tree::Elements;
use oc_schema::DesignDoc;
use serde_json::{Value, json};

const GRID: usize = 9;
const LOCAL_SAMPLES: [Vec2; 7] = [[0.5, 0.5], [0.02, 0.02], [0.98, 0.5], [0.3, 0.95], [1.1, 0.5], [0.5, -0.05], [0.15, 0.85]];

fn check_scene(label: &str, els: &Elements, r: &Value, fails: &mut Failures) {
    let ids: Vec<&str> = els.keys().map(String::as_str).collect();
    let idx = |e: Option<&Element>| -> Value {
        json!(e.and_then(Element::id).and_then(|id| ids.iter().position(|x| *x == id)).map_or(-1, |i| i as i64))
    };

    for er in r["elements"].as_array().expect("elements") {
        let id = s(&er["id"]);
        let el = &els[id];
        let l = |what: &str| format!("{label} {id} {what}");
        let m = world_matrix(els, id);
        fails.check(|| l("world"), &j(&m), &er["world"]);
        fails.check(|| l("quad"), &j(&world_quad(els, id)), &er["quad"]);
        fails.check(|| l("bounds"), &j(&world_bounds(els, id)), &er["bounds"]);
        fails.check(|| l("parentInv"), &j(&parent_inverse(els, el)), &er["parentInv"]);
        let xf = el.xf().expect("xf");
        let hits: Vec<Value> = LOCAL_SAMPLES
            .iter()
            .map(|[fx, fy]| {
                let p = apply(&m, [fx * xf.w, fy * xf.h]);
                json!([0.0, 3.0].map(|t| i64::from(hit_element(els, el, p, t))))
            })
            .collect();
        fails.check(|| l("hits"), &Value::Array(hits), &er["hits"]);
        match el {
            Element::Shape(sh) => {
                let d = shape_path(sh.kind.as_str(), xf.w, xf.h, &sh.params, sh.d.as_deref());
                fails.check(|| l("shape"), &j(&d), &er["shape"]);
            }
            Element::Frame(fr) => {
                let got = json!({
                    "outline": frame_el_outline(fr, xf.w, xf.h),
                    "overlays": j(&frame_overlays(fr.overlay.as_deref(), xf.w, xf.h)),
                });
                fails.check(|| l("frame"), &got, &er["frame"]);
            }
            Element::Line(ln) => {
                let got = json!({
                    "page": j(&connector_page_points(els, ln)),
                    "local": j(&connector_local_points(els, ln)),
                    "geometry": j(&connector_geometry(els, ln)),
                    "trimmed": j(&trim_line_for_markers(&ln.points, &ln.start_marker, &ln.end_marker, ln.stroke.width)),
                    "start": j(&line_marker_shape(&ln.start_marker, ln.stroke.width)),
                    "end": j(&line_marker_shape(&ln.end_marker, ln.stroke.width)),
                });
                fails.check(|| l("line"), &got, &er["line"]);
            }
            _ => {}
        }
    }

    for pr in r["pages"].as_array().expect("pages") {
        let pid = s(&pr["id"]);
        let (w, h) = (f(&pr["w"]), f(&pr["h"]));
        let l = |what: &str| format!("{label} page {pid} {what}");
        let mut grid: Vec<Vec2> = Vec::with_capacity(GRID * GRID);
        for i in 0..GRID {
            for jj in 0..GRID {
                grid.push([((i as f64 + 0.5) * w) / GRID as f64, ((jj as f64 + 0.5) * h) / GRID as f64]);
            }
        }
        let hit: Vec<Value> = grid
            .iter()
            .map(|&p| {
                json!([0.0, 4.0].map(|t| idx(hit_test(els, pid, p, HitTestOpts { tolerance: t, enter_group: None }))))
            })
            .collect();
        fails.check(|| l("hitTest"), &Value::Array(hit), &pr["hit"]);
        for er in pr["enter"].as_array().expect("enter") {
            let g = s(&er["group"]);
            let out: Vec<Value> =
                grid.iter().map(|&p| idx(hit_test(els, pid, p, HitTestOpts { tolerance: 0.0, enter_group: Some(g) }))).collect();
            fails.check(|| l(&format!("enter {g}")), &Value::Array(out), &er["out"]);
        }
        for mr in pr["marquee"].as_array().expect("marquee") {
            let rect: Rect = de(&mr["r"]);
            let got: Vec<&str> = marquee_select(els, pid, &rect).into_iter().filter_map(Element::id).collect();
            fails.check(|| l(&format!("marquee {}", mr["r"])), &j(&got), &mr["out"]);
        }
        let sel = &pr["selection"];
        let sel_ids: Vec<String> = de(&sel["ids"]);
        fails.check(|| l("selection"), &j(&selection_bounds(els, &sel_ids)), &sel["out"]);
    }

    for ar in r["anchors"].as_array().expect("anchors") {
        let got = anchor_point(els, s(&ar["id"]), &AnchorSide::parse(s(&ar["anchor"])));
        fails.check(|| format!("{label} anchorPoint {} {}", ar["id"], ar["anchor"]), &j(&got), &ar["out"]);
    }
    for nr in r["nearest"].as_array().expect("nearest") {
        let got = nearest_anchor(els, s(&nr["id"]), de(&nr["p"]), f(&nr["radius"]));
        fails.check(|| format!("{label} nearestAnchor {} {}", nr["id"], nr["radius"]), &j(&got), &nr["out"]);
    }
    for cr in r["connectorsOf"].as_array().expect("connectorsOf") {
        let set: HashSet<String> = de(&cr["ids"]);
        let got: Vec<&str> = connectors_of(els, &set).into_iter().map(|l| l.base.id.as_str()).collect();
        fails.check(|| format!("{label} connectorsOf {}", truncate(&cr["ids"].to_string())), &j(&got), &cr["out"]);
    }
    for sr in r["selection"].as_array().expect("selection") {
        let sel_ids: Vec<String> = de(&sr["ids"]);
        fails.check(|| format!("{label} selectionBounds {}", sr["ids"]), &j(&selection_bounds(els, &sel_ids)), &sr["out"]);
    }
}

#[test]
fn hand_scenes() {
    let g = golden("scenes");
    let mut fails = Failures::default();
    for sc in g.as_array().expect("scenes") {
        let els: Elements = de(&sc["elements"]);
        check_scene(s(&sc["name"]), &els, &sc["results"], &mut fails);
    }
    let n = fails.finish("scenes");
    eprintln!("scenes: {n} checks ok");
}

#[test]
fn template_scenes() {
    let g = golden("templates");
    let cases = g.as_array().expect("templates");
    assert!(cases.len() > 100, "expected the template goldens, found {}", cases.len());
    let mut fails = Failures::default();
    for c in cases {
        let file = s(&c["file"]);
        let mut doc: DesignDoc = de(&fixture(file));
        for (id, el) in c["patch"].as_object().expect("patch") {
            doc.elements.insert(id.clone(), de(el));
        }
        check_scene(file, &doc.elements, &c["results"], &mut fails);
    }
    let n = fails.finish("templates");
    eprintln!("templates: {} docs, {n} checks ok", cases.len());
}
