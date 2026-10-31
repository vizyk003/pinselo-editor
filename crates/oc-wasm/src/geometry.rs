// SPDX-License-Identifier: Apache-2.0
//! `@oc/geometry` helpers used by apps/web (TS name → oc-geometry). Element results are ids (the facade maps them
//! to the caller's objects).

use oc_geometry::{HitTestOpts, Params};
use oc_schema::elements::{AnchorSide, FrameClip, LineMarker};
use oc_schema::primitives::{Mat2D, Vec2};
use wasm_bindgen::prelude::*;

use crate::convert::{arg, element, js, opt_arg, type_error, value};
use crate::doc::DocRef;

fn missing(id: &str) -> JsValue {
    type_error(&format!("Cannot read properties of undefined (element {id})"))
}

#[wasm_bindgen(js_name = geometryConstants)]
pub fn geometry_constants() -> JsValue {
    js(&serde_json::json!({ "ANCHORS": oc_geometry::ANCHORS }))
}

#[wasm_bindgen(js_name = worldMatrix)]
pub fn world_matrix(doc: &DocRef, id: &str) -> Vec<f64> {
    oc_geometry::world_matrix(&doc.doc().elements, id).to_vec()
}

/// `worldBounds(doc, id)`; a missing element throws a `TypeError` like the TS.
#[wasm_bindgen(js_name = worldBounds)]
pub fn world_bounds(doc: &DocRef, id: &str) -> Result<JsValue, JsValue> {
    oc_geometry::world_bounds(&doc.doc().elements, id).map(|r| js(&r)).ok_or_else(|| missing(id))
}

/// `selectionBounds(doc, ids)`: the world bounds of the given elements, null when none of them exist.
#[wasm_bindgen(js_name = selectionBounds)]
pub fn selection_bounds(doc: &DocRef, ids: Vec<String>) -> JsValue {
    oc_geometry::selection_bounds(&doc.doc().elements, &ids).map_or(JsValue::NULL, |r| js(&r))
}

/// `worldQuad(doc, id)` as `[x0, y0, x1, y1, x2, y2, x3, y3]`.
#[wasm_bindgen(js_name = worldQuad)]
pub fn world_quad(doc: &DocRef, id: &str) -> Result<Vec<f64>, JsValue> {
    let q = oc_geometry::world_quad(&doc.doc().elements, id).ok_or_else(|| missing(id))?;
    Ok(q.iter().flat_map(|p| [p[0], p[1]]).collect())
}

/// `anchorPoint(doc, id, anchor)` (`undefined` → `null`).
#[wasm_bindgen(js_name = anchorPoint)]
pub fn anchor_point(doc: &DocRef, id: &str, anchor: JsValue) -> Result<Option<Vec<f64>>, JsError> {
    let a: AnchorSide = arg("anchor", &anchor)?;
    Ok(oc_geometry::anchor_point(&doc.doc().elements, id, &a).map(|p| p.to_vec()))
}

/// `hitTest(doc, pageId, p, { tolerance?, enterGroup? })` → the hit element's id.
#[wasm_bindgen(js_name = hitTest)]
pub fn hit_test(doc: &DocRef, page_id: &str, x: f64, y: f64, tolerance: f64, enter_group: Option<String>) -> Option<String> {
    let opts = HitTestOpts { tolerance, enter_group: enter_group.as_deref() };
    oc_geometry::hit_test(&doc.doc().elements, page_id, [x, y], opts).and_then(|e| e.id()).map(str::to_string)
}

fn mat(v: &[f64]) -> Mat2D {
    let mut m = [0.0; 6];
    for (i, x) in v.iter().take(6).enumerate() {
        m[i] = *x;
    }
    m
}

#[wasm_bindgen]
pub fn apply(m: &[f64], x: f64, y: f64) -> Vec<f64> {
    oc_geometry::apply(&mat(m), [x, y]).to_vec()
}

#[wasm_bindgen]
pub fn multiply(m: &[f64], n: &[f64]) -> Vec<f64> {
    oc_geometry::multiply(&mat(m), &mat(n)).to_vec()
}

#[wasm_bindgen]
pub fn invert(m: &[f64]) -> Vec<f64> {
    oc_geometry::invert(&mat(m)).to_vec()
}

/// `hitElement(doc, el, p, tolerance)`: whether `p` (scene space) is on the element.
#[wasm_bindgen(js_name = hitElement)]
pub fn hit_element(doc: &DocRef, el: JsValue, x: f64, y: f64, tolerance: f64) -> Result<bool, JsError> {
    let el = element(value(&el)?);
    Ok(oc_geometry::hit_element(&doc.doc().elements, &el, [x, y], tolerance))
}

/// `frameOutline({ clip, clipPath? }, w, h)`.
#[wasm_bindgen(js_name = frameOutline)]
pub fn frame_outline(el: JsValue, w: f64, h: f64) -> Result<String, JsError> {
    let v = value(&el)?;
    let clip: FrameClip = arg("clip", &JsValue::from_str(v.get("clip").and_then(serde_json::Value::as_str).unwrap_or("")))?;
    let clip_path = v.get("clipPath").and_then(serde_json::Value::as_str);
    Ok(oc_geometry::frame_outline(&clip, clip_path, w, h))
}

#[wasm_bindgen(js_name = shapePath)]
pub fn shape_path(kind: &str, w: f64, h: f64, params: JsValue, d: Option<String>) -> Result<String, JsError> {
    let p: Params = opt_arg("params", &params)?.unwrap_or_default();
    Ok(oc_geometry::shape_path(kind, w, h, &p, d.as_deref()))
}

#[wasm_bindgen(js_name = scaleUnitPath)]
pub fn scale_unit_path(d: &str, w: f64, h: f64) -> String {
    oc_geometry::scale_unit_path(d, w, h)
}

#[wasm_bindgen(js_name = lineMarkerShape)]
pub fn line_marker_shape(kind: JsValue, width: f64) -> Result<JsValue, JsError> {
    let k: LineMarker = arg("kind", &kind)?;
    Ok(oc_geometry::line_marker_shape(&k, width).map_or(JsValue::NULL, |s| js(&s)))
}

/// `trimLineForMarkers(points, start, end, width)`; points as a flat `[x0, y0, x1, y1, …]`.
#[wasm_bindgen(js_name = trimLineForMarkers)]
pub fn trim_line_for_markers(points: &[f64], start: JsValue, end: JsValue, width: f64) -> Result<Vec<f64>, JsError> {
    let (pairs, _) = points.as_chunks::<2>();
    let pts: Vec<Vec2> = pairs.to_vec();
    let (s, e): (LineMarker, LineMarker) = (arg("start", &start)?, arg("end", &end)?);
    Ok(oc_geometry::trim_line_for_markers(&pts, &s, &e, width).into_iter().flat_map(|p| [p[0], p[1]]).collect())
}
