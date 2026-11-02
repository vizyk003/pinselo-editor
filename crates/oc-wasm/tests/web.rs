// SPDX-License-Identifier: Apache-2.0
//! wasm-bindgen tests (run in Node by `wasm-bindgen-test-runner`, see the README): what only exists on the JS side
//! of the boundary — JS value shapes (`null` vs `undefined`), the injected JS clock, a JS `TextService` object
//! called from Rust, errors surfacing as JS exceptions, `DocRef` from plain JS objects.
#![cfg(target_arch = "wasm32")]

use js_sys::{Array, Function, Reflect};
use oc_wasm::{DocRef, Editor};
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

fn design() -> serde_json::Value {
    json!({
        "meta": { "schemaVersion": 1, "minClientSchema": 1, "layoutVersion": 1, "surface": "graphic", "dims": { "w": 800, "h": 600, "unit": "px", "dpi": 96 }, "lang": "en", "title": "t", "epoch": 0 },
        "pages": { "p1": { "id": "p1", "order": "a0", "background": [] } },
        "elements": {},
        "assets": {}
    })
}

fn editor(opts: JsValue) -> Editor {
    Editor::new(&oc_crdt::encode_doc_value(&design()), opts).expect("editor")
}

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap()
}

fn js_fn(body: &str) -> Function {
    Function::new_no_args(body)
}

#[wasm_bindgen_test]
fn snapshots_are_plain_js_with_nulls() {
    let mut ed = editor(JsValue::UNDEFINED);
    let st = ed.state().unwrap();
    assert!(get(&st, "hover").is_null(), "None is null, like the TS state");
    assert_eq!(get(&st, "activePageId").as_string().as_deref(), Some("p1"));
    assert!(get(&get(&st, "viewport"), "zoom").as_f64().is_some());
    assert!(ed.overrides().unwrap().is_null());
    let d = ed.doc_delta().unwrap();
    assert_eq!(get(&d, "full").as_bool(), Some(true));
    assert!(get(&get(&d, "pages"), "p1").is_object());
    assert!(ed.doc_delta().unwrap().is_null(), "unchanged");
    assert_eq!(ed.viewport().len(), 5);
}

#[wasm_bindgen_test]
fn the_js_clock_is_injected() {
    let opts = js_sys::Object::new();
    Reflect::set(&opts, &"clock".into(), &js_fn("return 12345.5")).unwrap();
    let ed = editor(opts.into());
    assert_eq!(ed.now(), 12345.5);
    assert!(!ed.set_now(1.0), "no manual clock with a host clock");
}

#[wasm_bindgen_test]
fn a_js_text_service_is_called_from_rust() {
    let mut ed = editor(JsValue::UNDEFINED);
    let svc = js_fn("return { height: () => 42, offsetAt: () => 0, verticalMove: (e, o) => o, wordAt: (e, o) => [o, o], length: (e) => e.body.paragraphs.length - 1 }").call0(&JsValue::NULL).unwrap();
    ed.set_js_text_service(svc);
    assert!(ed.has_text_service());
    let text = json!({
        "id": "t1", "type": "text", "parentId": "", "z": "a0", "opacity": 1, "xf": { "x": 0, "y": 0, "w": 200, "h": 10, "rot": 0 },
        "body": { "defaultRun": { "family": "Inter", "weight": 400, "italic": false, "size": 16, "fill": { "k": "solid", "color": { "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } }, "tracking": 0 },
                  "paragraphs": [{ "runs": [{ "text": "Hi", "style": {} }], "align": "start", "lineHeight": 1.3, "spaceAfter": 0 }] },
        "frame": { "mode": "autoHeight" }, "autoFit": "none"
    });
    let ids = ed.dispatch("insert", &json!([[text]]).to_string()).unwrap();
    let id = Array::from(&ids).get(0).as_string().unwrap();
    let d = ed.doc_delta().unwrap();
    let h = get(&get(&get(&get(&d, "elements"), &id), "xf"), "h").as_f64();
    assert_eq!(h, Some(42.0), "auto-height from the JS service");
}

#[wasm_bindgen_test]
fn errors_surface_as_js_exceptions() {
    let mut ed = editor(JsValue::UNDEFINED);
    assert!(ed.dispatch("noSuchMember", "[]").is_err());
    assert!(ed.dispatch("select", "not json").is_err());
    assert!(oc_wasm::schema::hex("#zz").is_err());
    let e = oc_wasm::geometry::world_bounds(&DocRef::from_js(JsValue::from(js_sys::Object::new())).unwrap(), "missing").unwrap_err();
    assert!(e.is_instance_of::<js_sys::TypeError>(), "a missing element throws a TypeError like the TS");
}

#[wasm_bindgen_test]
fn doc_refs_from_partial_js_documents() {
    let obj = js_fn("return { elements: { a: { id: 'a', type: 'shape', parentId: 'p', z: 'a0', opacity: 1, xf: { x: 1, y: 2, w: 3, h: 4, rot: 0 }, kind: 'rect', params: {}, fills: [], strokes: [] } } }").call0(&JsValue::NULL).unwrap();
    let d = DocRef::from_js(obj).unwrap();
    assert_eq!(d.element_count(), 1);
    assert_eq!(oc_wasm::schema::children_of(&d, "p"), vec!["a".to_string()]);
    assert_eq!(oc_wasm::geometry::world_matrix(&d, "a"), vec![1.0, 0.0, 0.0, 1.0, 1.0, 2.0]);
}

#[wasm_bindgen_test]
fn constants_cross_as_plain_values() {
    let c = oc_wasm::schema::schema_constants();
    assert!(Array::is_array(&get(&c, "SIZE_PRESETS")));
    assert!(get(&c, "GRID_LAYOUTS").is_object());
    assert_eq!(get(&c, "DOC_CONTENT_WIDTH").as_f64(), Some(656.0));
}
