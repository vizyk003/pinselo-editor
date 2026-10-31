// SPDX-License-Identifier: Apache-2.0
//! The JS `Editor` handle: an oc-editor-core editor over a yrs mirror of the browser's yjs `Y.Doc`.
//!
//! Boundary design (measured in packages/engine/test/bench.test.ts):
//!
//! * The long tail of commands goes through [`Editor::dispatch`] (`Editor::dispatch(name, args)` in
//!   oc-editor-core, the facade contract): arguments as JSON text (`JSON.stringify` semantics are exactly what
//!   the goldens verify: `undefined` dropped), results as JS values built by serde-wasm-bindgen.
//! * Hot paths take plain numbers and return the status word ([`crate::session::Session::status`]): pointer
//!   moves, hover, pan/zoom, viewport size. Modifier keys are a bit set (`shift 1 | alt 2 | meta 4 | ctrl 8`).
//! * Snapshots are pulled lazily by the facade, at most once per version: `state()`, `overrides()`,
//!   `docDelta()` (only what changed since the last pull), `viewport()` as a `Float64Array`.
//! * No JS callback runs while the editor is borrowed except the injected clock and a JS text service, so a
//!   listener may call back into the editor freely (listeners live in the facade, driven by the version).

use std::sync::Arc;

use oc_editor_core::{Clock, HitTarget, Mods, Viewport};
use oc_schema::primitives::Vec2;
use serde_json::Value;
use wasm_bindgen::prelude::*;

use crate::convert::{arg, err, js, opt_arg, to_js, value};
use crate::doc::{DocRef, design_from_value};
use crate::session::{Session, SessionOptions};
use crate::text::{EngineText, JsText, TextService};

/// A clock backed by a JS function (`() => number`, ms).
struct JsClock(js_sys::Function);

// SAFETY: wasm32-unknown-unknown without the atomics feature is single-threaded: the function is only ever called
// on the thread that created it. `Clock` requires `Send + Sync` because native hosts share clocks across threads.
unsafe impl Send for JsClock {}
// SAFETY: see above.
unsafe impl Sync for JsClock {}

impl Clock for JsClock {
    fn now(&self) -> f64 {
        self.0.call0(&JsValue::NULL).ok().and_then(|v| v.as_f64()).unwrap_or(0.0)
    }
}

fn mods(bits: u32) -> Mods {
    Mods { shift: bits & 1 != 0, alt: bits & 2 != 0, meta: bits & 4 != 0, ctrl: bits & 8 != 0 }
}

fn get(o: &JsValue, k: &str) -> JsValue {
    js_sys::Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

#[wasm_bindgen]
pub struct Editor {
    s: Session,
}

impl Editor {
    pub fn session(&self) -> &Session {
        &self.s
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.s
    }
}

#[wasm_bindgen]
impl Editor {
    /// `new Editor(update, opts?)`: `update` is `Y.encodeStateAsUpdate(ydoc)`; `opts` is
    /// `{ viewport?, clientId?, seed?, clock?: () => number, now? }` (no `clock`: a manual clock at `now`, set with
    /// `setNow`).
    #[wasm_bindgen(constructor)]
    pub fn new(update: &[u8], opts: JsValue) -> Result<Editor, JsError> {
        let mut o = SessionOptions::default();
        if opts.is_object() {
            if let Some(vp) = opt_arg::<Viewport>("viewport", &get(&opts, "viewport"))? {
                o.viewport = vp;
            }
            o.client_id = get(&opts, "clientId").as_f64().map(|x| x as u64);
            o.seed = get(&opts, "seed").as_f64().map(|x| x as u32);
            if let Some(n) = get(&opts, "now").as_f64() {
                o.start_ms = n;
            }
            let clock = get(&opts, "clock");
            if let Some(f) = clock.dyn_ref::<js_sys::Function>() {
                o.clock = Some(Arc::new(JsClock(f.clone())));
            }
        }
        Ok(Editor { s: Session::new(update, o).map_err(err)? })
    }

    /// The status word: `version * 4 + overridesNonEmpty * 2 + updatesPending`.
    pub fn status(&self) -> f64 {
        self.s.status()
    }

    #[wasm_bindgen(getter)]
    pub fn version(&self) -> f64 {
        self.s.editor.version() as f64
    }

    /// Call an editor member by its TS name (`Editor::dispatch`); `args` is the JSON argument array. The result is a
    /// JS value (`null` for `undefined`). Read the status afterwards.
    pub fn dispatch(&mut self, method: &str, args: &str) -> Result<JsValue, JsError> {
        let r = self.call(method, args)?;
        to_js(&r)
    }

    /// [`Editor::dispatch`] returning the result as JSON text (benchmark baseline, debugging).
    #[wasm_bindgen(js_name = dispatchJson)]
    pub fn dispatch_json(&mut self, method: &str, args: &str) -> Result<String, JsError> {
        let r = self.call(method, args)?;
        serde_json::to_string(&r).map_err(err)
    }

    fn call(&mut self, method: &str, args: &str) -> Result<Value, JsError> {
        // Test hook for the facade's panic recovery (`editor.dispatch('$panic')`): a real abort, as any bug would be.
        if method == "$panic" {
            panic!("$panic: test panic requested");
        }
        let a: Value = if args.is_empty() { Value::Null } else { serde_json::from_str(args).map_err(|e| err(format!("{method}: bad arguments: {e}")))? };
        let r = self.s.editor.dispatch(method, &a);
        self.s.settle();
        r.map_err(err)
    }

    // ------------------------------------------------------------------ hot paths (numbers in, status out)

    /// `pointerDown(p, mods, target?, button?)`; `target` is a `HitTarget` object or `undefined` (canvas).
    #[wasm_bindgen(js_name = pointerDown)]
    pub fn pointer_down(&mut self, x: f64, y: f64, mod_bits: u32, target: JsValue, button: f64) -> Result<f64, JsError> {
        let t: HitTarget = opt_arg("target", &target)?.unwrap_or_default();
        if button == 0.0 {
            self.s.editor.pointer_down([x, y], mods(mod_bits), t, 0);
        }
        Ok(self.s.settle())
    }

    #[wasm_bindgen(js_name = pointerMove)]
    pub fn pointer_move(&mut self, x: f64, y: f64, mod_bits: u32, pressure: f64) -> f64 {
        self.s.editor.pointer_move([x, y], mods(mod_bits), pressure);
        self.s.settle()
    }

    #[wasm_bindgen(js_name = pointerUp)]
    pub fn pointer_up(&mut self) -> f64 {
        self.s.editor.pointer_up();
        self.s.settle()
    }

    #[wasm_bindgen(js_name = setHover)]
    pub fn set_hover(&mut self, id: Option<String>) -> f64 {
        self.s.editor.set_hover(id);
        self.s.settle()
    }

    #[wasm_bindgen(js_name = panBy)]
    pub fn pan_by(&mut self, dx: f64, dy: f64) -> f64 {
        self.s.editor.pan_by(dx, dy);
        self.s.settle()
    }

    /// `zoomBy(factor, screenPt?)` (`sx`/`sy` undefined = viewport centre).
    #[wasm_bindgen(js_name = zoomBy)]
    pub fn zoom_by(&mut self, factor: f64, sx: Option<f64>, sy: Option<f64>) -> f64 {
        let pt: Option<Vec2> = sx.zip(sy).map(|(x, y)| [x, y]);
        self.s.editor.zoom_by(factor, pt);
        self.s.settle()
    }

    #[wasm_bindgen(js_name = zoomTo)]
    pub fn zoom_to(&mut self, zoom: f64, sx: Option<f64>, sy: Option<f64>) -> f64 {
        let pt: Option<Vec2> = sx.zip(sy).map(|(x, y)| [x, y]);
        self.s.editor.zoom_to(zoom, pt);
        self.s.settle()
    }

    #[wasm_bindgen(js_name = setViewportSize)]
    pub fn set_viewport_size(&mut self, width: f64, height: f64) -> f64 {
        self.s.editor.set_viewport_size(width, height);
        self.s.settle()
    }

    #[wasm_bindgen(js_name = setViewport)]
    pub fn set_viewport(&mut self, x: f64, y: f64, zoom: f64, width: f64, height: f64) -> f64 {
        self.s.editor.set_viewport(Viewport { x, y, zoom, width, height });
        self.s.settle()
    }

    /// `[x, y, zoom, width, height]`.
    pub fn viewport(&self) -> Vec<f64> {
        let v = &self.s.editor.state.viewport;
        vec![v.x, v.y, v.zoom, v.width, v.height]
    }

    // ------------------------------------------------------------------ snapshots

    /// `editor.state` as a plain object.
    pub fn state(&self) -> Result<JsValue, JsError> {
        to_js(&self.s.editor.state)
    }

    /// `state()` if it changed since the last `stateIfChanged()`, else `undefined` (keep the previous object).
    #[wasm_bindgen(js_name = stateIfChanged)]
    pub fn state_if_changed(&mut self) -> Result<JsValue, JsError> {
        match self.s.state_if_changed() {
            Some(st) => to_js(st),
            None => Ok(JsValue::UNDEFINED),
        }
    }

    /// The overrides if they changed since the last call (`undefined`: unchanged; `null`: none).
    #[wasm_bindgen(js_name = overridesIfChanged)]
    pub fn overrides_if_changed(&mut self) -> Result<JsValue, JsError> {
        match self.s.overrides_if_changed() {
            None => Ok(JsValue::UNDEFINED),
            Some(o) if o.is_empty() => Ok(JsValue::NULL),
            Some(o) => to_js(o),
        }
    }

    /// The gesture overrides `{ [id]: patch }`, or `null` when there are none.
    pub fn overrides(&self) -> Result<JsValue, JsError> {
        let o = self.s.editor.overrides();
        if o.is_empty() { Ok(JsValue::NULL) } else { to_js(o) }
    }

    /// What changed in the document since the last call (`null`: nothing): `{ full, meta?, assets?, pages?,
    /// elements? }` where `pages`/`elements` map ids to the new value or `null` (removed).
    #[wasm_bindgen(js_name = docDelta)]
    pub fn doc_delta(&mut self) -> Result<JsValue, JsError> {
        match self.s.doc_delta(|d| to_js(d)) {
            None => Ok(JsValue::NULL),
            Some(r) => r,
        }
    }

    /// [`Editor::doc_delta`] as JSON text (`undefined`: nothing changed).
    #[wasm_bindgen(js_name = docDeltaJson)]
    pub fn doc_delta_json(&mut self) -> Result<Option<String>, JsError> {
        self.s.doc_delta(|d| serde_json::to_string(d)).transpose().map_err(err)
    }

    /// The next `docDelta` is the whole document.
    #[wasm_bindgen(js_name = resetMirror)]
    pub fn reset_mirror(&mut self) {
        self.s.reset_mirror();
    }

    /// The current document (`kind` 0) or view (`kind` 1) as a [`DocRef`] for the pure helpers (an `Arc` clone).
    #[wasm_bindgen(js_name = docRef)]
    pub fn doc_ref(&self, kind: u8) -> DocRef {
        DocRef::shared(if kind == 1 { self.s.editor.view() } else { self.s.editor.doc() })
    }

    // ------------------------------------------------------------------ sync with the browser's Y.Doc

    /// Apply an update from the yjs doc (lib0 v1). Returns the status word.
    #[wasm_bindgen(js_name = applyRemoteUpdate)]
    pub fn apply_remote_update(&mut self, update: &[u8]) -> Result<f64, JsError> {
        self.s.apply_remote_update(update).map_err(err)
    }

    /// Local updates (lib0 v1) to apply to the yjs doc, oldest first.
    #[wasm_bindgen(js_name = drainLocalUpdates)]
    pub fn drain_local_updates(&mut self) -> js_sys::Array {
        self.s.take_updates().into_iter().map(|u| JsValue::from(js_sys::Uint8Array::from(u.as_slice()))).collect()
    }

    /// `Y.encodeStateAsUpdate` of the mirror (lib0 v1).
    #[wasm_bindgen(js_name = encodeState)]
    pub fn encode_state(&self) -> Vec<u8> {
        oc_crdt::encode_state(self.s.editor.store().ydoc())
    }

    /// `Y.encodeStateVector` of the mirror.
    #[wasm_bindgen(js_name = encodeStateVector)]
    pub fn encode_state_vector(&self) -> Vec<u8> {
        oc_crdt::encode_state_vector(self.s.editor.store().ydoc())
    }

    /// Replace the content (version restore) as an undoable local change: `replaceContent(store.ydoc, doc,
    /// LocalOrigin.I, store.actor)` on the mirror, then the usual change reactions.
    #[wasm_bindgen(js_name = replaceContent)]
    pub fn replace_content(&mut self, doc: JsValue) -> Result<f64, JsError> {
        let d = value(&doc)?;
        let actor = self.s.editor.store().actor.clone();
        let now = self.s.editor.now();
        oc_crdt::replace_content(self.s.editor.store().ydoc(), &d, Some(oc_crdt::OcOrigin::Local.origin()), &actor, now);
        self.s.editor.flush();
        Ok(self.s.settle())
    }

    // ------------------------------------------------------------------ injected services

    /// Measure text with an oc-text [`TextService`] (shared, called from Rust directly).
    #[wasm_bindgen(js_name = setTextService)]
    pub fn set_text_service(&mut self, ts: &TextService) {
        self.s.editor.set_text_service(Some(Box::new(EngineText(ts.engine()))));
    }

    /// Measure text with a JS object implementing the TS `TextService` interface (`null` clears).
    #[wasm_bindgen(js_name = setJsTextService)]
    pub fn set_js_text_service(&mut self, ts: JsValue) {
        let svc: Option<Box<dyn oc_editor_core::TextService>> = if ts.is_object() { Some(Box::new(JsText(ts))) } else { None };
        self.s.editor.set_text_service(svc);
    }

    #[wasm_bindgen(getter, js_name = hasTextService)]
    pub fn has_text_service(&self) -> bool {
        self.s.editor.has_text_service()
    }

    /// Set the manual clock (only without a host clock; returns whether it applied).
    #[wasm_bindgen(js_name = setNow)]
    pub fn set_now(&self, ms: f64) -> bool {
        self.s.set_now(ms)
    }

    pub fn now(&self) -> f64 {
        self.s.now()
    }

    #[wasm_bindgen(js_name = randomBytesDrawn)]
    pub fn random_bytes_drawn(&self) -> f64 {
        self.s.random_bytes_drawn() as f64
    }

    /// `templatePages(tpl, { w, h })` drawing ids from this editor's random stream (deterministic with a seed).
    #[wasm_bindgen(js_name = templatePages)]
    pub fn template_pages(&mut self, tpl: JsValue, w: f64, h: f64) -> Result<JsValue, JsError> {
        let d = design_from_value(value(&tpl)?);
        Ok(js(&self.s.template_pages(&d, w, h)))
    }

    /// `paste()` of the internal clipboard (which then shifts by `offset` for the next paste, like TS).
    #[wasm_bindgen(js_name = pasteClipboard)]
    pub fn paste_clipboard(&mut self, offset: f64) -> f64 {
        self.s.editor.paste(None, offset);
        self.s.settle()
    }

    /// `apply(ops, merge?)` with ops as JS values (the `dispatch('apply', …)` fast path for large op lists).
    pub fn apply(&mut self, ops: JsValue, merge: bool) -> Result<JsValue, JsError> {
        let ops: Vec<oc_crdt::DocOp> = arg("ops", &ops)?;
        let r = self.s.editor.apply(ops, merge);
        self.s.settle();
        to_js(&r)
    }
}
