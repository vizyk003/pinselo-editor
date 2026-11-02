// SPDX-License-Identifier: Apache-2.0
//! `@oc/editor-core` module functions used by apps/web (TS name → oc-editor-core).

use oc_editor_core as ec;
use oc_editor_core::{FontSet, Viewport};
use oc_schema::primitives::Vec2;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::convert::{arg, element, js, to_js, value};
use crate::doc::DocRef;

#[wasm_bindgen(js_name = editorConstants)]
pub fn editor_constants() -> JsValue {
    js(&json!({
        "MIN_ZOOM": ec::MIN_ZOOM,
        "MAX_ZOOM": ec::MAX_ZOOM,
        "CORNERS": ec::CORNERS,
        "EDGES": ec::EDGES,
        "CLIPBOARD_MIME": ec::CLIPBOARD_MIME,
    }))
}

// ------------------------------------------------------------------ crops

/// `coverCrop(boxAspect, imgAspect)`.
#[wasm_bindgen(js_name = coverCrop)]
pub fn cover_crop(box_aspect: f64, img_aspect: f64) -> JsValue {
    js(&ec::cover_crop(box_aspect, img_aspect))
}

/// `refitCrop(crop, from, to)` (`from`/`to`: `{ w, h }`).
#[wasm_bindgen(js_name = refitCrop)]
pub fn refit_crop(crop: JsValue, from: JsValue, to: JsValue) -> Result<JsValue, JsError> {
    let c: oc_schema::primitives::StoredRect = arg("crop", &crop)?;
    let wh = |v: &JsValue| -> Result<(f64, f64), JsError> {
        let v = value(v)?;
        let n = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
        Ok((n("w"), n("h")))
    };
    to_js(&ec::refit_crop(&c, wh(&from)?, wh(&to)?))
}

// ------------------------------------------------------------------ viewport, styles, a11y

/// `pageSlots(doc)` as `[{ id, index, rect }]` (the facade puts the page objects back).
#[wasm_bindgen(js_name = pageSlots)]
pub fn page_slots(doc: &DocRef) -> JsValue {
    let slots: Vec<Value> = ec::page_slots(doc.doc()).iter().map(|s| json!({ "id": s.page.id, "index": s.index, "rect": s.rect })).collect();
    js(&slots)
}

#[wasm_bindgen(js_name = screenToScene)]
pub fn screen_to_scene(vp: JsValue, x: f64, y: f64) -> Result<Vec<f64>, JsError> {
    let v: Viewport = arg("viewport", &vp)?;
    let p: Vec2 = ec::screen_to_scene(&v, [x, y]);
    Ok(p.to_vec())
}

#[wasm_bindgen(js_name = designColors)]
pub fn design_colors(doc: &DocRef) -> JsValue {
    js(&ec::design_colors(doc.doc()))
}

#[wasm_bindgen(js_name = fontSetOps)]
pub fn font_set_ops(doc: &DocRef, set: JsValue) -> Result<JsValue, JsError> {
    let s: FontSet = arg("set", &set)?;
    to_js(&ec::font_set_ops(doc.doc(), &s))
}

#[wasm_bindgen(js_name = auditDesign)]
pub fn audit_design(doc: &DocRef) -> JsValue {
    js(&ec::audit_design(doc.doc()))
}

#[wasm_bindgen(js_name = describeElement)]
pub fn describe_element(doc: &DocRef, el: JsValue) -> Result<JsValue, JsError> {
    to_js(&ec::describe_element(doc.doc(), &element(value(&el)?)))
}

/// `scalePatches(doc, el, sx, sy)` as a plain object `{ [id]: patch }` (the facade turns it into a `Map`).
#[wasm_bindgen(js_name = scalePatches)]
pub fn scale_patches(doc: &DocRef, el: JsValue, sx: f64, sy: f64) -> Result<JsValue, JsError> {
    Ok(js(&ec::scale_patches(&doc.doc().elements, &element(value(&el)?), sx, sy)))
}

/// `templatePages(tpl, { w, h })` with Web Crypto ids (an editor's `templatePages` draws from its own stream).
#[wasm_bindgen(js_name = templatePages)]
pub fn template_pages(tpl: &DocRef, w: f64, h: f64) -> JsValue {
    js(&ec::template_pages(tpl.doc(), w, h, &mut oc_editor_core::OsRandom))
}

