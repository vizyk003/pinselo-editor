// SPDX-License-Identifier: Apache-2.0
//! Golden replay (ADR-0001): every session in tests/golden/editor/*.json (written by tools/golden/editor.ts from
//! the TS `Editor`) is replayed through `Editor::dispatch`; after each step the return value, the editor state,
//! the notification counter, the document, the gesture overrides and the undo/redo availability must match
//! (floats within 1e-9), and so must the number of random bytes drawn (ids, z keys).
//!
//! Debugging: `OC_GOLDEN_CASE=<name>` replays one session; `OC_GOLDEN_TRACE=<step>[,<step>…]` prints the selection
//! box, the selected elements and the overrides before those steps, plus the element `OC_GOLDEN_EL=<id>`.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use common::{GoldText, golden, json_diff, root, short};
use oc_crdt::DocOp;
use oc_editor_core::{Editor, EditorOptions, ManualClock, Mulberry32, RandomSource, Viewport, template_pages};
use oc_schema::DesignDoc;
use serde_json::{Map, Value, json};

/// Mulberry32 that counts the bytes drawn (the goldens record the TS count per step).
struct Counting(Mulberry32, Arc<AtomicU64>);

impl RandomSource for Counting {
    fn fill(&mut self, buf: &mut [u8]) {
        self.0.fill(buf);
        self.1.fetch_add(buf.len() as u64, Ordering::SeqCst);
    }
}

fn unb64(v: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(v.as_str().expect("base64 string")).expect("base64")
}

/// Documented deviation (see the crate docs): turning a list off writes `list: null` where TS removes the paragraph
/// attribute (`setParagraphStyle({ list: undefined })`); JSON DocOps cannot carry `undefined`. Readers treat both
/// alike, so paragraph objects compare with a `null` list equal to none.
fn without_null_list(v: &mut Value) {
    match v {
        Value::Object(m) => {
            let paragraph = m.contains_key("runs") || (m.contains_key("align") && m.contains_key("lineHeight") && m.contains_key("spaceAfter"));
            if paragraph && m.get("list") == Some(&Value::Null) {
                m.remove("list");
            }
            m.values_mut().for_each(without_null_list);
        }
        Value::Array(a) => a.iter_mut().for_each(without_null_list),
        _ => {}
    }
}

fn same(actual: &Value, expected: &Value, path: &str) -> Option<String> {
    let (mut a, mut e) = (actual.clone(), expected.clone());
    without_null_list(&mut a);
    without_null_list(&mut e);
    json_diff(&a, &e, path)
}

/// `same` for the (large) documents: normalises in place, the expected one once per delta.
fn same_doc(mut actual: Value, expected: &Value) -> Option<String> {
    without_null_list(&mut actual);
    json_diff(&actual, expected, "doc")
}

/// `{ ...doc }` with a recorded delta applied (`null` removes a key).
fn apply_delta(doc: &mut Value, d: &Value) {
    for (k, field) in [("e", "elements"), ("p", "pages"), ("a", "assets")] {
        if let Some(Value::Object(m)) = d.get(k) {
            let target = doc[field].as_object_mut().expect("doc map");
            for (id, v) in m {
                if v.is_null() {
                    target.remove(id);
                } else {
                    target.insert(id.clone(), v.clone());
                }
            }
        }
    }
    if let Some(m) = d.get("m") {
        doc["meta"] = m.clone();
    }
}

/// The web TemplatesPanel apply flow (tools/golden/editor.ts `applyTemplate`).
fn apply_template(ed: &mut Editor, name: &str) -> Value {
    let raw = std::fs::read_to_string(root().join("fixtures/templates-seed").join(name)).expect("fixture");
    let tpl: DesignDoc = serde_json::from_str(&raw).expect("fixture design");
    let tpl_json: Value = serde_json::from_str(&raw).expect("fixture json");
    let (w, h) = {
        let d = ed.doc();
        (d.doc.meta.dims.w, d.doc.meta.dims.h)
    };
    let imported = template_pages(&tpl, w, h, ed.rng_mut());
    let active = ed.state.active_page_id.clone();
    let (pages, active_empty) = {
        let d = ed.doc();
        (d.doc.pages.len(), oc_schema::tree::children_of(d.els(), &active).is_empty())
    };
    let mut ops: Vec<Value> = Vec::new();
    let mut after = active.clone();
    for ip in &imported {
        ops.push(json!({ "op": "page", "action": "add", "page": ip.page, "after": after }));
        after = ip.page["id"].as_str().unwrap().to_string();
        for el in &ip.elements {
            ops.push(json!({ "op": "add", "el": el, "parent": el.get("parentId").cloned().unwrap_or(Value::Null) }));
        }
    }
    if active_empty && pages >= 1 {
        ops.push(json!({ "op": "page", "action": "delete", "pageId": active }));
    }
    if let Some(Value::Object(assets)) = tpl_json.get("assets") {
        for a in assets.values() {
            ops.insert(0, json!({ "op": "asset", "asset": a }));
        }
    }
    let ops: Vec<DocOp> = serde_json::from_value(Value::Array(ops)).expect("template ops");
    let r = ed.apply(ops, false);
    let first = imported[0].page["id"].as_str().unwrap().to_string();
    if r.ok {
        ed.set_active_page(&first, true);
    }
    json!({ "ok": r.ok, "pages": imported.iter().map(|p| p.page["id"].clone()).collect::<Vec<_>>() })
}

/// Replay one session; returns (steps passed, total, first failure).
fn replay(file: &str) -> (usize, usize, Option<String>) {
    let g = golden(file);
    let ydoc = oc_crdt::new_doc_with_client(2);
    oc_crdt::codec::apply_update(&ydoc, &unb64(&g["update"]), None).expect("initial update");
    let clock = ManualClock::new(g["clock"].as_f64().unwrap());
    let vp: Viewport = serde_json::from_value(g["viewport"].clone()).unwrap();
    let seed = g["seed"].as_u64().unwrap() as u32;
    let drawn = Arc::new(AtomicU64::new(0));
    let rng = Counting(Mulberry32::new(seed), drawn.clone());
    let mut ed = Editor::from_ydoc(ydoc, EditorOptions { viewport: vp, clock: clock.clone(), rng: Box::new(rng) });
    if g["text"].as_str() == Some("gold") {
        ed.set_text_service(Some(Box::new(GoldText)));
    }
    ed.brand_admin = g["brandAdmin"].as_bool().unwrap();
    ed.user = serde_json::from_value(g["user"].clone()).unwrap();
    let mut expected = g["doc"].clone();
    let steps = g["steps"].as_array().unwrap();
    if let Some(d) = json_diff(&ed.doc_json(), &expected, "doc") {
        return (0, steps.len(), Some(format!("{file}: initial document differs: {d}")));
    }
    let clock: Arc<ManualClock> = clock;
    for (i, st) in steps.iter().enumerate() {
        let m = st["m"].as_str().unwrap();
        let a = &st["a"];
        let fail = |what: &str, d: String| Some(format!("{file} step {i} {m}({}): {what}: {d}", short(a)));
        if let Ok(t) = std::env::var("OC_GOLDEN_TRACE")
            && t.split(',').any(|x| x.parse::<usize>().ok() == Some(i))
        {
            for q in ["selectionBox", "selected", "overrides"] {
                eprintln!("trace step {i} before {m}: {q} = {}", short(&ed.dispatch(q, &Value::Null).unwrap_or(Value::Null)));
            }
            if let Ok(id) = std::env::var("OC_GOLDEN_EL") {
                eprintln!("trace step {i} before {m}: {id} = {}", ed.doc_json()["elements"][&id]);
            }
        }
        let r = match m {
            "$clock" => {
                clock.advance(a[0].as_f64().unwrap());
                Value::Null
            }
            "$text" => {
                ed.set_text_service(if a[0].as_str() == Some("gold") { Some(Box::new(GoldText)) } else { None });
                Value::Null
            }
            "$remote" => {
                ed.apply_remote_update(&unb64(&a[0])).expect("remote update");
                Value::Null
            }
            "$template" => apply_template(&mut ed, a[0].as_str().unwrap()),
            _ => match ed.dispatch(m, a) {
                Ok(v) => v,
                Err(e) => return (i, steps.len(), fail("dispatch error", e.to_string())),
            },
        };
        if let Some(d) = json_diff(&json!(drawn.load(Ordering::SeqCst)), &st["n"], "random bytes drawn") {
            return (i, steps.len(), fail("random draws", d));
        }
        if let Some(d) = same(&r, &st["r"], "return") {
            return (i, steps.len(), fail("return value", d));
        }
        if let Some(d) = same(&serde_json::to_value(&ed.state).unwrap(), &st["s"], "state") {
            return (i, steps.len(), fail("state", d));
        }
        if let Some(dd) = st.get("d") {
            let mut dd = dd.clone();
            without_null_list(&mut dd);
            apply_delta(&mut expected, &dd);
        }
        let actual_doc = ed.doc_json();
        if let Some(d) = same_doc(actual_doc.clone(), &expected) {
            // Show the whole differing element / page.
            let mut parts = d.splitn(4, '.').skip(1);
            let (map, id) = (parts.next().unwrap_or(""), parts.next().unwrap_or("").split(':').next().unwrap_or(""));
            let ctx = format!("\n  actual {map}.{id}:   {}\n  expected {map}.{id}: {}", short(&actual_doc[map][id]), short(&expected[map][id]));
            return (i, steps.len(), fail("document", d + &ctx));
        }
        let o = ed.dispatch("overrides", &Value::Null).unwrap();
        if let Some(d) = same(&o, st.get("o").unwrap_or(&Value::Object(Map::new())), "overrides") {
            return (i, steps.len(), fail("overrides", d));
        }
        let u = json!([ed.can_undo(), ed.can_redo()]);
        if let Some(d) = json_diff(&u, &st["u"], "undo") {
            return (i, steps.len(), fail("undo/redo", d));
        }
        if let Some(d) = json_diff(&json!(ed.version()), &st["v"], "version") {
            return (i, steps.len(), fail("version", d));
        }
    }
    (steps.len(), steps.len(), None)
}

#[test]
fn replays_every_golden_session() {
    let dir = root().join("tests/golden/editor");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("tests/golden/editor (run `pnpm tsx tools/golden/editor.ts`)")
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
        .filter(|f| f.ends_with(".json") && f != "modules.json")
        .collect();
    files.sort();
    if let Ok(only) = std::env::var("OC_GOLDEN_CASE") {
        files.retain(|f| f.trim_end_matches(".json") == only);
    }
    assert!(!files.is_empty(), "no golden sessions");
    let (mut passed, mut total, mut failures) = (0, 0, Vec::new());
    for f in &files {
        let (ok, n, err) = replay(f);
        passed += ok;
        total += n;
        eprintln!("{f}: {ok}/{n} steps");
        if let Some(e) = err {
            failures.push(e);
        }
    }
    eprintln!("golden sessions: {passed}/{total} steps in {} files", files.len());
    assert!(failures.is_empty(), "{} session(s) diverged:\n{}", failures.len(), failures.join("\n\n"));
}
