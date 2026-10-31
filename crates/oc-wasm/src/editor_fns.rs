// SPDX-License-Identifier: Apache-2.0
//! `@oc/editor-core` module functions used by apps/web (TS name → oc-editor-core).

use oc_crdt::board::{TimerState, VoteEntry};
use oc_editor_core as ec;
use oc_editor_core::{FontSet, VoteTally, Viewport};
use oc_schema::primitives::Vec2;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::convert::{arg, element, js, opt_element, to_js, value};
use crate::doc::DocRef;

#[wasm_bindgen(js_name = editorConstants)]
pub fn editor_constants() -> JsValue {
    js(&json!({
        "STICKY_COLORS": ec::STICKY_COLORS,
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

// ------------------------------------------------------------------ whiteboard

#[wasm_bindgen(js_name = isSticky)]
pub fn is_sticky(el: JsValue) -> Result<bool, JsError> {
    Ok(ec::is_sticky(opt_element(&el)?.as_ref()))
}

#[wasm_bindgen(js_name = isSection)]
pub fn is_section(el: JsValue) -> Result<bool, JsError> {
    Ok(ec::is_section(opt_element(&el)?.as_ref()))
}

#[wasm_bindgen(js_name = isMindNode)]
pub fn is_mind_node(el: JsValue) -> Result<bool, JsError> {
    Ok(ec::is_mind_node(opt_element(&el)?.as_ref()))
}

#[wasm_bindgen(js_name = sectionsOf)]
pub fn sections_of(doc: &DocRef, page_id: &str) -> Vec<String> {
    ec::sections_of(&doc.doc().elements, page_id).iter().map(|e| e.id().unwrap_or("").to_string()).collect()
}

#[wasm_bindgen(js_name = mindNodeOf)]
pub fn mind_node_of(doc: &DocRef, id: &str) -> Option<String> {
    ec::mind_node_of(&doc.doc().elements, id).and_then(|e| e.id()).map(str::to_string)
}

#[wasm_bindgen(js_name = voteScope)]
pub fn vote_scope(doc: &DocRef, page_id: &str, selection: Vec<String>) -> Vec<String> {
    ec::vote_scope(&doc.doc().elements, page_id, &selection)
}

/// `tallyVotes(session: { id, scope }, entries)`.
#[wasm_bindgen(js_name = tallyVotes)]
pub fn tally_votes(session_id: &str, scope: Vec<String>, entries: JsValue) -> Result<JsValue, JsError> {
    let e: Vec<VoteEntry> = arg("entries", &entries)?;
    to_js(&ec::tally_votes(session_id, &scope, &e))
}

#[wasm_bindgen(js_name = topVoted)]
pub fn top_voted(tally: JsValue, n: u32) -> Result<Vec<String>, JsError> {
    let t: Vec<VoteTally> = arg("tally", &tally)?;
    Ok(ec::top_voted(&t, n as usize))
}

/// `voteAllowance(session: { id, perPerson, status }, entries, voter)`.
#[wasm_bindgen(js_name = voteAllowance)]
pub fn vote_allowance(session_id: &str, per_person: f64, status: &str, entries: JsValue, voter: &str) -> Result<f64, JsError> {
    let e: Vec<VoteEntry> = arg("entries", &entries)?;
    Ok(ec::vote_allowance(session_id, per_person, status, &e, voter))
}

#[wasm_bindgen(js_name = formatClock)]
pub fn format_clock(ms: f64) -> String {
    ec::format_clock(ms)
}

#[wasm_bindgen(js_name = makeTimer)]
pub fn make_timer(duration_ms: f64, sound: bool) -> JsValue {
    js(&ec::make_timer(duration_ms, sound))
}

fn timer(t: &JsValue) -> Result<TimerState, JsError> {
    arg("timer", t)
}

#[wasm_bindgen(js_name = timerRemaining)]
pub fn timer_remaining(t: JsValue, now: f64) -> Result<f64, JsError> {
    Ok(ec::timer_remaining(&timer(&t)?, now))
}

#[wasm_bindgen(js_name = timerRunning)]
pub fn timer_running(t: JsValue, now: f64) -> Result<bool, JsError> {
    Ok(ec::timer_running(&timer(&t)?, now))
}

#[wasm_bindgen(js_name = timerStart)]
pub fn timer_start(t: JsValue, now: f64) -> Result<JsValue, JsError> {
    to_js(&ec::timer_start(&timer(&t)?, now))
}

#[wasm_bindgen(js_name = timerPause)]
pub fn timer_pause(t: JsValue, now: f64) -> Result<JsValue, JsError> {
    to_js(&ec::timer_pause(&timer(&t)?, now))
}

#[wasm_bindgen(js_name = timerReset)]
pub fn timer_reset(t: JsValue) -> Result<JsValue, JsError> {
    to_js(&ec::timer_reset(&timer(&t)?))
}

#[wasm_bindgen(js_name = timerAdd)]
pub fn timer_add(t: JsValue, ms: f64, now: f64) -> Result<JsValue, JsError> {
    to_js(&ec::timer_add(&timer(&t)?, ms, now))
}

/// `brandLockError(lock, action)` (`@oc/crdt`): why an action would break a brand lock, `null` when allowed.
#[wasm_bindgen(js_name = brandLockError)]
pub fn brand_lock_error(lock: JsValue, action: JsValue) -> Result<Option<String>, JsError> {
    use oc_crdt::brandlock::BrandAction;
    let lock = value(&lock)?;
    let a = value(&action)?;
    let pair = |v: &Value| -> Option<(Option<Value>, Option<Value>)> { v.is_object().then(|| (v.get("from").cloned(), v.get("to").cloned())) };
    let xf = a.get("xf").and_then(pair);
    let body = a.get("body").and_then(pair);
    let keys: Vec<String> = a.get("keys").and_then(Value::as_array).map(|k| k.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    let (from, to) = (a.get("from").cloned(), a.get("to").cloned());
    let act = match a.get("kind").and_then(Value::as_str) {
        Some("patch") => BrandAction::Patch { keys, xf: xf.as_ref().map(|(f, t)| (f.as_ref(), t.as_ref())), body: body.as_ref().map(|(f, t)| (f.as_ref(), t.as_ref())) },
        Some("xf") => BrandAction::Xf { from: from.as_ref(), to: to.as_ref() },
        Some("move") => BrandAction::Move,
        Some("delete") => BrandAction::Delete,
        Some("content") => BrandAction::Content,
        Some("style") => BrandAction::Style,
        _ => return Err(crate::convert::err("brandLockError: unknown action kind")),
    };
    let l = (!lock.is_null()).then_some(&lock);
    Ok(oc_crdt::brandlock::brand_lock_error(l, &act).map(str::to_owned))
}
