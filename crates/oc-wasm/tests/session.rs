// SPDX-License-Identifier: Apache-2.0
//! Native tests of the bridge core (no JS needed): the status word, local-update forwarding, document deltas
//! (applied to a JSON mirror they rebuild the editor's document; untouched elements are not re-sent), changed-only
//! state snapshots, two mirrors converging through updates, deterministic ids, tolerant document parsing and the
//! oc-text service fed with font bytes.

use std::path::PathBuf;

use oc_wasm::{Session, SessionOptions, design_from_value};
use serde_json::{Map, Value, json};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(root().join("fixtures/templates-seed").join(name)).expect("fixture")).expect("json")
}

fn session(design: &Value, seed: u32) -> Session {
    session_from(&oc_crdt::encode_doc_value(design), seed)
}

/// A mirror of a browser doc holding `update` (peers must start from the same bytes: every encode uses a fresh client).
fn session_from(update: &[u8], seed: u32) -> Session {
    Session::new(update, SessionOptions { seed: Some(seed), client_id: Some(seed as u64 + 10), ..Default::default() }).expect("session")
}

fn call(s: &mut Session, m: &str, a: Value) -> Value {
    let r = s.editor.dispatch(m, &a).unwrap_or_else(|e| panic!("{m}: {e}"));
    s.settle();
    r
}

fn first_page(s: &Session) -> String {
    s.editor.state.active_page_id.clone()
}

/// Apply a serialised delta to a JSON mirror (what the facade does in JS).
fn apply_delta(mirror: &mut Value, d: &Value) {
    if d["full"] == json!(true) {
        *mirror = json!({ "meta": d["meta"], "pages": {}, "elements": {}, "assets": d.get("assets").cloned().unwrap_or(json!({})) });
    }
    for k in ["meta", "assets"] {
        if let Some(v) = d.get(k) {
            mirror[k] = v.clone();
        }
    }
    for k in ["pages", "elements"] {
        if let Some(Value::Object(m)) = d.get(k) {
            for (id, v) in m {
                if v.is_null() {
                    mirror[k].as_object_mut().unwrap().remove(id);
                } else {
                    mirror[k][id] = v.clone();
                }
            }
        }
    }
}

fn delta(s: &mut Session) -> Option<Value> {
    s.doc_delta(|d| serde_json::to_value(d).unwrap())
}

#[test]
fn starts_from_the_browser_state() {
    let design = fixture("card-business-clean.json");
    let s = session(&design, 1);
    let got = s.editor.doc_json();
    let want = oc_crdt::decode_doc_value(&oc_crdt::encode_doc_value(&design)).unwrap();
    assert_eq!(got, want);
    assert_eq!(s.status(), 0.0, "version 0, no overrides, nothing pending");
}

#[test]
fn status_word_reports_version_overrides_and_pending_updates() {
    let mut s = session(&fixture("card-business-clean.json"), 2);
    let shape = oc_schema::factory::make_shape(oc_schema::factory::base_init(oc_schema::factory::xf(0.0, 0.0, 50.0, 50.0, 0.0)), Default::default());
    let shape = serde_json::to_value(oc_schema::Element::from(shape)).unwrap();
    let ids = call(&mut s, "insert", json!([[shape], [100, 100]]));
    let id = ids[0].as_str().unwrap().to_string();
    let st = s.status();
    assert_eq!(st % 2.0, 1.0, "local updates pending after a command");
    assert!(st >= 4.0, "version moved");
    let ups = s.take_updates();
    assert!(!ups.is_empty());
    assert_eq!(s.status() % 2.0, 0.0, "taken");
    // A drag: overrides while moving, no document writes until pointer-up.
    let origin = s.editor.page_origin(&first_page(&s));
    let b = s.editor.doc().el(&id).and_then(|e| e.xf().cloned()).unwrap();
    let c = [origin[0] + b.x + b.w / 2.0, origin[1] + b.y + b.h / 2.0];
    s.editor.pointer_down(c, oc_editor_core::Mods::NONE, Default::default(), 0);
    s.settle();
    s.editor.pointer_move([c[0] + 40.0, c[1] + 10.0], oc_editor_core::Mods::NONE, 0.5);
    let st = s.settle();
    assert_eq!((st / 2.0).floor() % 2.0, 1.0, "overrides bit set while dragging");
    assert_eq!(st % 2.0, 0.0, "no document writes while dragging");
    s.editor.pointer_up();
    let st = s.settle();
    assert_eq!((st / 2.0).floor() % 2.0, 0.0, "overrides cleared");
    assert_eq!(st % 2.0, 1.0, "the move is committed once");
}

#[test]
fn deltas_rebuild_the_document_and_skip_untouched_elements() {
    let mut s = session(&fixture("ig-sale-violet.json"), 3);
    let mut mirror = Value::Null;
    apply_delta(&mut mirror, &delta(&mut s).expect("first delta is full"));
    assert_eq!(mirror, s.editor.doc_json());
    assert!(delta(&mut s).is_none(), "nothing changed");

    let id = s.editor.doc_json()["elements"].as_object().unwrap().keys().next().unwrap().clone();
    call(&mut s, "select", json!([[id]]));
    assert!(delta(&mut s).is_none(), "selection is state, not document");
    call(&mut s, "updateSelected", json!([{ "opacity": 0.25 }]));
    let d = delta(&mut s).expect("changed");
    assert_eq!(d["full"], json!(false));
    assert_eq!(d["elements"].as_object().unwrap().keys().collect::<Vec<_>>(), vec![&id], "only the touched element");
    assert!(d.get("pages").is_none() && d.get("meta").is_none());
    apply_delta(&mut mirror, &d);
    assert_eq!(mirror, s.editor.doc_json());

    call(&mut s, "deleteSelection", json!([]));
    call(&mut s, "addPage", json!([null]));
    call(&mut s, "setLanguage", json!(["de"]));
    apply_delta(&mut mirror, &delta(&mut s).unwrap());
    assert_eq!(mirror, s.editor.doc_json());
    call(&mut s, "undo", json!([]));
    apply_delta(&mut mirror, &delta(&mut s).unwrap());
    assert_eq!(mirror, s.editor.doc_json());
}

#[test]
fn state_snapshots_are_sent_only_when_they_change() {
    let mut s = session(&fixture("card-business-clean.json"), 4);
    assert!(s.state_if_changed().is_some());
    assert!(s.state_if_changed().is_none());
    call(&mut s, "setLanguage", json!(["fr"]));
    assert!(s.state_if_changed().is_none(), "a document-only change keeps the state");
    call(&mut s, "zoomBy", json!([2]));
    assert!(s.state_if_changed().is_some());
    assert!(s.overrides_if_changed().is_some(), "first call reports the (empty) overrides");
    assert!(s.overrides_if_changed().is_none());
}

#[test]
fn two_mirrors_converge_through_updates() {
    let design = fixture("card-business-clean.json");
    let shared = oc_crdt::encode_doc_value(&design);
    let mut a = session_from(&shared, 5);
    let mut b = session_from(&shared, 6);
    let sync = |from: &mut Session, to: &mut Session| {
        for u in from.take_updates() {
            to.apply_remote_update(&u).expect("update");
        }
    };
    call(&mut a, "addPage", json!([null]));
    call(&mut b, "setLanguage", json!(["es"]));
    sync(&mut a, &mut b);
    sync(&mut b, &mut a);
    assert_eq!(a.editor.doc_json(), b.editor.doc_json());
    // Remote changes are not undoable locally.
    assert!(b.editor.can_undo());
    call(&mut b, "undo", json!([]));
    sync(&mut b, &mut a);
    assert_eq!(a.editor.doc_json(), b.editor.doc_json());
    assert_eq!(a.editor.doc_json()["meta"]["lang"], design["meta"]["lang"]);
    assert_eq!(a.editor.doc_json()["pages"].as_object().unwrap().len(), design["pages"].as_object().unwrap().len() + 1);
}

#[test]
fn seeded_ids_are_deterministic() {
    let tpl: oc_schema::DesignDoc = serde_json::from_value(fixture("ig-sale-violet.json")).unwrap();
    let design = fixture("card-business-clean.json");
    let mut a = session(&design, 7);
    let mut b = session(&design, 7);
    let pa = serde_json::to_value(a.template_pages(&tpl, 1080.0, 1080.0)).unwrap();
    let pb = serde_json::to_value(b.template_pages(&tpl, 1080.0, 1080.0)).unwrap();
    assert_eq!(pa, pb);
    assert!(a.random_bytes_drawn() > 0);
    assert_eq!(a.random_bytes_drawn(), b.random_bytes_drawn());
}

#[test]
fn foreign_documents_parse_tolerantly() {
    let mut d = fixture("card-business-clean.json");
    d["elements"]["broken"] = json!({ "id": "broken", "type": "shape", "lock": {} });
    let doc = design_from_value(d.clone());
    assert!(matches!(doc.elements.get("broken"), Some(oc_schema::Element::Unknown(_))));
    assert_eq!(doc.elements.len(), d["elements"].as_object().unwrap().len());
    // Element order is the JS object order (TS iteration order).
    let keys: Vec<&String> = doc.elements.keys().collect();
    let want: Vec<&String> = d["elements"].as_object().unwrap().keys().collect();
    assert_eq!(keys, want);
    // `Pick<DesignDoc, 'elements'>`: the rest is defaulted.
    let mut only = Map::new();
    only.insert("elements".into(), d["elements"].clone());
    let doc = design_from_value(Value::Object(only));
    assert!(doc.pages.is_empty());
    assert_eq!(doc.meta.dims.w, 100.0);
}

#[test]
fn text_service_takes_font_bytes_by_hash() {
    let files = root().join("fixtures/fonts/files");
    if !files.exists() {
        eprintln!("fixtures/fonts/files missing (pnpm fonts:sync): skipping");
        return;
    }
    let manifest: oc_text::FontManifest = serde_json::from_str(&std::fs::read_to_string(root().join("fixtures/fonts/manifest.json")).unwrap()).unwrap();
    let ts = oc_wasm::TextService::from_manifest(manifest);
    let file = "inter/Inter_400Regular.ttf";
    assert_eq!(ts.register_bytes(std::fs::read(files.join(file)).unwrap()).unwrap(), file);
    assert!(ts.register_bytes(vec![0, 1, 2]).is_err());
    // The editor-facing adapter measures like apps/web's TS wiring: height grows when the box narrows.
    let el: oc_schema::elements::TextEl = match oc_wasm::convert::element(json!({
        "id": "t", "type": "text", "parentId": "", "z": "a0", "opacity": 1, "xf": { "x": 0, "y": 0, "w": 400, "h": 10, "rot": 0 },
        "body": { "defaultRun": { "family": "Inter", "weight": 400, "italic": false, "size": 16, "fill": { "k": "solid", "color": { "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } }, "tracking": 0 },
                  "paragraphs": [{ "runs": [{ "text": "Hello wonderful world of text", "style": {} }], "align": "start", "lineHeight": 1.3, "spaceAfter": 0 }] },
        "frame": { "mode": "autoHeight" }, "autoFit": "none"
    })) {
        oc_schema::Element::Text(t) => t,
        other => panic!("not text: {other:?}"),
    };
    let svc = oc_wasm::text::EngineText(ts.engine());
    let wide = oc_editor_core::TextService::height(&svc, &el, 400.0);
    let narrow = oc_editor_core::TextService::height(&svc, &el, 60.0);
    assert!(wide > 0.0 && narrow > wide * 2.0, "{wide} {narrow}");
    assert_eq!(oc_editor_core::TextService::length(&svc, &el), 29);
    assert_eq!(oc_editor_core::TextService::word_at(&svc, &el, 7), (6, 15));
}
