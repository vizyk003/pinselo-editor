// SPDX-License-Identifier: Apache-2.0
//! Text measurement for the editor.
//!
//! * [`TextService`] (JS class `TextService`): oc-text's `TextEngine` with a font registry fed from JS.
//!   `registerFont(bytes)` looks the bytes up by their SHA-256 in the manifest (fixtures/fonts/manifest.json), so the
//!   host only fetches files and never names them. Attached to an editor it is called directly from Rust (no
//!   boundary crossing per measurement). The future renderer module shares the same engine ([`TextService::engine`]).
//! * [`JsText`]: any JS object implementing the TS `TextService` interface (`height`, `offsetAt`, `verticalMove`,
//!   `wordAt`, `length`, `curvedSize?`), for hosts that keep their own text engine during the migration. Each call
//!   serialises the text element to JS. The object must not call back into the editor (the editor is borrowed
//!   while it measures).
//!
//! The adapter reproduces the apps/web wiring of the TS editor (EditorShell.tsx) exactly: `height` lays out under
//! the element id at its own width and under `<id>:probe` at any other width, caret queries use the layout at the
//! element's width, `length` is the UTF-16 length of the plain text, `curvedSize` is the curved layout's box for a
//! truthy `curve`.

use std::cell::RefCell;
use std::rc::Rc;

use oc_schema::elements::{Element, TextEl};
use oc_schema::js::utf16_len;
use oc_schema::text::{TextBody, body_plain_text};
use oc_text::{FontFamily, FontManifest, FontRegistry, TextEngine, TextLayout};
use serde_json::json;
use sha2::{Digest, Sha256};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::convert::{arg, err, js, to_js, value};

pub type SharedEngine = Rc<RefCell<TextEngine>>;

/// The editor-facing adapter over a shared engine.
pub struct EngineText(pub SharedEngine);

impl EngineText {
    fn layout(&self, el: &TextEl) -> std::sync::Arc<TextLayout> {
        self.0.borrow_mut().layout(&el.base.id, &el.body, el.base.xf.w, el.effect.as_ref())
    }
}

fn offset(o: i64) -> usize {
    o.max(0) as usize
}

impl oc_editor_core::TextService for EngineText {
    fn height(&self, el: &TextEl, width: f64) -> f64 {
        let key = if width == el.base.xf.w { el.base.id.clone() } else { format!("{}:probe", el.base.id) };
        self.0.borrow_mut().layout(&key, &el.body, width, el.effect.as_ref()).height
    }

    fn offset_at(&self, el: &TextEl, x: f64, y: f64) -> i64 {
        self.layout(el).offset_at(x, y) as i64
    }

    fn vertical_move(&self, el: &TextEl, o: i64, dir: i32) -> i64 {
        self.layout(el).vertical_move(offset(o), dir) as i64
    }

    fn word_at(&self, el: &TextEl, o: i64) -> (i64, i64) {
        let (a, b) = self.layout(el).word_at(offset(o));
        (a as i64, b as i64)
    }

    fn length(&self, el: &TextEl) -> i64 {
        utf16_len(&body_plain_text(&el.body)) as i64
    }

    fn curved_size(&self, el: &TextEl) -> Option<(f64, f64)> {
        let curve = el.curve.filter(|c| *c != 0.0 && !c.is_nan())?;
        self.0.borrow_mut().curved(&el.base.id, &el.body, curve).map(|l| (l.width, l.height))
    }
}

/// A JS object implementing the TS `TextService` interface.
pub struct JsText(pub JsValue);

impl JsText {
    fn call(&self, name: &str, args: &[JsValue]) -> JsValue {
        let Ok(f) = js_sys::Reflect::get(&self.0, &JsValue::from_str(name)) else { return JsValue::UNDEFINED };
        let Ok(f) = f.dyn_into::<js_sys::Function>() else { return JsValue::UNDEFINED };
        let list: js_sys::Array = args.iter().collect();
        f.apply(&self.0, &list).unwrap_or(JsValue::UNDEFINED)
    }

    fn el(el: &TextEl) -> JsValue {
        js(&Element::Text(el.clone()))
    }

    fn num(v: JsValue) -> f64 {
        v.as_f64().unwrap_or(0.0)
    }
}

impl oc_editor_core::TextService for JsText {
    fn height(&self, el: &TextEl, width: f64) -> f64 {
        Self::num(self.call("height", &[Self::el(el), width.into()]))
    }

    fn offset_at(&self, el: &TextEl, x: f64, y: f64) -> i64 {
        Self::num(self.call("offsetAt", &[Self::el(el), x.into(), y.into()])) as i64
    }

    fn vertical_move(&self, el: &TextEl, o: i64, dir: i32) -> i64 {
        Self::num(self.call("verticalMove", &[Self::el(el), (o as f64).into(), f64::from(dir).into()])) as i64
    }

    fn word_at(&self, el: &TextEl, o: i64) -> (i64, i64) {
        let r = self.call("wordAt", &[Self::el(el), (o as f64).into()]);
        let a = js_sys::Reflect::get_u32(&r, 0).ok().and_then(|v| v.as_f64()).unwrap_or(o as f64);
        let b = js_sys::Reflect::get_u32(&r, 1).ok().and_then(|v| v.as_f64()).unwrap_or(o as f64);
        (a as i64, b as i64)
    }

    fn length(&self, el: &TextEl) -> i64 {
        Self::num(self.call("length", &[Self::el(el)])) as i64
    }

    fn curved_size(&self, el: &TextEl) -> Option<(f64, f64)> {
        let r = self.call("curvedSize", &[Self::el(el)]);
        if r.is_null() || r.is_undefined() {
            return None;
        }
        let w = js_sys::Reflect::get(&r, &"w".into()).ok()?.as_f64()?;
        let h = js_sys::Reflect::get(&r, &"h".into()).ok()?.as_f64()?;
        Some((w, h))
    }
}

/// oc-text behind the TS `TextService` interface, with a font registry fed from JS.
#[wasm_bindgen]
pub struct TextService {
    engine: SharedEngine,
}

impl TextService {
    pub fn engine(&self) -> SharedEngine {
        self.engine.clone()
    }

    pub fn from_manifest(manifest: FontManifest) -> TextService {
        TextService { engine: Rc::new(RefCell::new(TextEngine::new(FontRegistry::new(manifest)))) }
    }

    /// Register font bytes by content hash. Returns the manifest file they belong to.
    pub fn register_bytes(&self, bytes: Vec<u8>) -> Result<String, String> {
        let hash = hex::encode(Sha256::digest(&bytes));
        let mut e = self.engine.borrow_mut();
        let file = e.fonts.face_by_sha256(&hash).map(|f| f.file.clone()).ok_or_else(|| format!("no manifest face has sha256 {hash}"))?;
        // The lookup already verified the hash.
        e.fonts.insert_unchecked(&file, bytes).map_err(|x| x.to_string())?;
        Ok(file)
    }
}

fn text_el(v: &JsValue) -> Result<TextEl, JsError> {
    match crate::convert::element(value(v)?) {
        Element::Text(t) => Ok(t),
        _ => Err(err("not a text element")),
    }
}

#[wasm_bindgen]
impl TextService {
    /// `new TextService(manifest)` with the font manifest (`fixtures/fonts/manifest.json`, or the merged catalogue
    /// served at `/api/fonts/manifest.json`).
    #[wasm_bindgen(constructor)]
    pub fn new(manifest: JsValue) -> Result<TextService, JsError> {
        Ok(TextService::from_manifest(arg("manifest", &manifest)?))
    }

    /// Register a font file's bytes; the face is found by the SHA-256 the manifest pins. Returns its manifest path.
    #[wasm_bindgen(js_name = registerFont)]
    pub fn register_font(&self, bytes: &[u8]) -> Result<String, JsError> {
        self.register_bytes(bytes.to_vec()).map_err(err)
    }

    /// Register bytes for a manifest path (faces without a pinned hash: Google catalogue, uploads). Checked
    /// against the manifest hash when there is one.
    #[wasm_bindgen(js_name = registerFontFile)]
    pub fn register_font_file(&self, file: &str, bytes: &[u8]) -> Result<(), JsError> {
        self.engine.borrow_mut().fonts.insert_face(file, bytes.to_vec()).map_err(err)
    }

    /// Add (or replace) a family at runtime (uploaded brand fonts).
    #[wasm_bindgen(js_name = addFamily)]
    pub fn add_family(&self, family: JsValue) -> Result<(), JsError> {
        let f: FontFamily = arg("family", &family)?;
        self.engine.borrow_mut().fonts.add_family(f);
        Ok(())
    }

    #[wasm_bindgen(js_name = hasFamily)]
    pub fn has_family(&self, family: &str) -> bool {
        self.engine.borrow().fonts.has(family)
    }

    /// Bumped whenever a face finishes loading (layout caches key on it).
    #[wasm_bindgen(getter)]
    pub fn generation(&self) -> f64 {
        self.engine.borrow().fonts.generation() as f64
    }

    #[wasm_bindgen(getter, js_name = loadedCount)]
    pub fn loaded_count(&self) -> u32 {
        self.engine.borrow().fonts.loaded_count() as u32
    }

    /// Manifest files still needed to lay out a text body exactly (empty when ready).
    #[wasm_bindgen(js_name = missingFonts)]
    pub fn missing_fonts(&self, body: JsValue) -> Result<Vec<String>, JsError> {
        let b: TextBody = arg("body", &body)?;
        Ok(self.engine.borrow().missing_fonts(&b))
    }

    pub fn height(&self, el: JsValue, width: f64) -> Result<f64, JsError> {
        Ok(oc_editor_core::TextService::height(&EngineText(self.engine.clone()), &text_el(&el)?, width))
    }

    #[wasm_bindgen(js_name = offsetAt)]
    pub fn offset_at(&self, el: JsValue, x: f64, y: f64) -> Result<f64, JsError> {
        Ok(oc_editor_core::TextService::offset_at(&EngineText(self.engine.clone()), &text_el(&el)?, x, y) as f64)
    }

    #[wasm_bindgen(js_name = verticalMove)]
    pub fn vertical_move(&self, el: JsValue, offset: f64, dir: i32) -> Result<f64, JsError> {
        Ok(oc_editor_core::TextService::vertical_move(&EngineText(self.engine.clone()), &text_el(&el)?, offset as i64, dir) as f64)
    }

    #[wasm_bindgen(js_name = wordAt)]
    pub fn word_at(&self, el: JsValue, offset: f64) -> Result<Vec<f64>, JsError> {
        let (a, b) = oc_editor_core::TextService::word_at(&EngineText(self.engine.clone()), &text_el(&el)?, offset as i64);
        Ok(vec![a as f64, b as f64])
    }

    pub fn length(&self, el: JsValue) -> Result<f64, JsError> {
        Ok(oc_editor_core::TextService::length(&EngineText(self.engine.clone()), &text_el(&el)?) as f64)
    }

    /// `{ w, h }` of curved text, `null` when the element is not curved or no face is loaded.
    #[wasm_bindgen(js_name = curvedSize)]
    pub fn curved_size(&self, el: JsValue) -> Result<JsValue, JsError> {
        let s = oc_editor_core::TextService::curved_size(&EngineText(self.engine.clone()), &text_el(&el)?);
        to_js(&s.map(|(w, h)| json!({ "w": w, "h": h })))
    }

    /// Record the faces layouts (measuring and drawing) need but do not have, for the host to load
    /// (`takeMissingFaces`). The TS engine loaded them itself whenever it laid text out.
    #[wasm_bindgen(js_name = reportMissingFaces)]
    pub fn report_missing_faces(&self, on: bool) {
        self.engine.borrow_mut().report_missing_faces(on);
    }

    /// `[{ family, weight, italic, file }]` recorded since the last call (each file once).
    #[wasm_bindgen(js_name = takeMissingFaces)]
    pub fn take_missing_faces(&self) -> JsValue {
        js(&self.engine.borrow_mut().take_missing_faces())
    }

    /// Number of recorded missing faces not taken yet (cheap to poll).
    #[wasm_bindgen(getter, js_name = pendingMissingFaces)]
    pub fn pending_missing_faces(&self) -> u32 {
        self.engine.borrow().pending_missing_faces() as u32
    }

    /// Let `file` be reported again (its load failed).
    #[wasm_bindgen(js_name = forgetMissingFace)]
    pub fn forget_missing_face(&self, file: &str) {
        self.engine.borrow_mut().forget_missing_face(file);
    }

    /// Whether a manifest file's bytes are registered.
    #[wasm_bindgen(js_name = isFileLoaded)]
    pub fn is_file_loaded(&self, file: &str) -> bool {
        self.engine.borrow().fonts.face_by_file(file).is_some()
    }

    /// The manifest file used for `(family, weight, italic)` (`pickFace`), `undefined` for an empty manifest.
    #[wasm_bindgen(js_name = pickFile)]
    pub fn pick_file(&self, family: &str, weight: f64, italic: bool) -> Option<String> {
        self.engine.borrow().fonts.pick_face(family, weight, italic).map(|(_, f)| f.file.clone())
    }

    /// Lay a body out under `key` (cached like the drawing and the editor's measurements, which share it).
    pub fn layout(&self, key: &str, body: JsValue, width: f64, effect: JsValue) -> Result<TextLayoutRef, JsError> {
        let b: TextBody = arg("body", &body)?;
        let e: Option<oc_schema::text::TextEffect> = crate::convert::opt_arg("effect", &effect)?;
        Ok(TextLayoutRef(self.engine.borrow_mut().layout(key, &b, width, e.as_ref())))
    }

    /// Drop cached layouts of one key (an element id) or all.
    pub fn dispose(&self, key: Option<String>) {
        let mut e = self.engine.borrow_mut();
        match key {
            Some(k) => e.dispose(&k),
            None => e.dispose_all(),
        }
    }
}

/// A laid-out text body (`@oc/text` `TextLayout`) with its caret and selection geometry.
#[wasm_bindgen]
pub struct TextLayoutRef(std::sync::Arc<TextLayout>);

#[wasm_bindgen]
impl TextLayoutRef {
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> f64 {
        self.0.width
    }

    #[wasm_bindgen(getter)]
    pub fn height(&self) -> f64 {
        self.0.height
    }

    /// Flat (UTF-16) length of the laid-out text.
    #[wasm_bindgen(getter)]
    pub fn length(&self) -> u32 {
        self.0.length as u32
    }

    /// `{ x, y, w, h }` of the caret at a flat offset (box space).
    #[wasm_bindgen(js_name = caretRect)]
    pub fn caret_rect(&self, offset: f64) -> JsValue {
        js(&self.0.caret_rect(offset.max(0.0) as usize))
    }

    #[wasm_bindgen(js_name = selectionRects)]
    pub fn selection_rects(&self, from: f64, to: f64) -> JsValue {
        js(&self.0.selection_rects(from.max(0.0) as usize, to.max(0.0) as usize))
    }

    #[wasm_bindgen(js_name = offsetAt)]
    pub fn offset_at(&self, x: f64, y: f64) -> f64 {
        self.0.offset_at(x, y) as f64
    }

    #[wasm_bindgen(js_name = wordAt)]
    pub fn word_at(&self, offset: f64) -> Vec<f64> {
        let (a, b) = self.0.word_at(offset.max(0.0) as usize);
        vec![a as f64, b as f64]
    }

    #[wasm_bindgen(js_name = verticalMove)]
    pub fn vertical_move(&self, offset: f64, dir: i32) -> f64 {
        self.0.vertical_move(offset.max(0.0) as usize, dir) as f64
    }
}
