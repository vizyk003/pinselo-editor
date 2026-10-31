// SPDX-License-Identifier: Apache-2.0
//! Pinselo browser bridge (ADR-0001 phase 4): the Rust core compiled to WebAssembly with wasm-bindgen, wrapped
//! by the TS facade `@oc/engine` (packages/engine) that keeps the signatures of `@oc/editor-core`,
//! `@oc/geometry` and `@oc/schema`.
//!
//! The TS shell keeps the browser input stack and the accessibility tree; the browser's yjs `Y.Doc` stays the
//! network/TipTap source of truth and the [`Editor`] holds a yrs mirror, synced by exchanging update bytes.
//!
//! Modules (JS exports):
//! * [`editor`]: the `Editor` handle (dispatch, hot-path pointer/viewport methods, snapshots, sync, injected
//!   clock/RNG/text service). The bridge bookkeeping lives in [`session`], which is plain Rust (tested natively).
//! * [`text`]: the oc-text `TextService` (font registry fed from JS) and the JS text-service adapter.
//! * [`doc`]: `DocRef`, a design parsed once and passed to the document helpers.
//! * [`schema`], [`geometry`], [`editor_fns`]: the pure helpers apps/web imports.
//! * `sheet` (feature `sheet`): the oc-sheet `SheetEngine` handle (yrs mirror + formula engine, model writes)
//!   and the `@oc/sheet` helpers.
//!
//! * `render` (feature `render`): oc-render — `resolvePage` → `DisplayList`, the tile `RenderTarget`, the
//!   `Painter` sharing [`text::TextService::engine`] (one font registry and layout cache for measuring, caret
//!   geometry and drawing), and the `@oc/render-core` helpers.

pub mod convert;
pub mod doc;
pub mod editor;
pub mod editor_fns;
pub mod geometry;
#[cfg(feature = "render")]
pub mod render;
pub mod schema;
pub mod session;
#[cfg(feature = "sheet")]
pub mod sheet;
pub mod text;

pub use doc::{DocRef, design_from_value};
pub use editor::Editor;
pub use session::{DocDelta, Session, SessionOptions, doc_delta};
pub use text::TextService;

use wasm_bindgen::prelude::*;

/// Runs when the module is instantiated.
#[wasm_bindgen(start)]
pub fn start() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// `{ version, features }` of this build.
#[wasm_bindgen(js_name = engineInfo)]
pub fn engine_info() -> JsValue {
    let features: Vec<&str> = [
        cfg!(feature = "console_error_panic_hook").then_some("console_error_panic_hook"),
        cfg!(feature = "complex-scripts").then_some("complex-scripts"),
        cfg!(feature = "sheet").then_some("sheet"),
        cfg!(feature = "render").then_some("render"),
    ]
    .into_iter()
    .flatten()
    .collect();
    convert::js(&serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "features": features }))
}

/// A design as a yjs update (`toY` into a fresh doc + `Y.encodeStateAsUpdate`), for seeding a browser `Y.Doc`.
#[wasm_bindgen(js_name = encodeDesign)]
pub fn encode_design(doc: JsValue) -> Result<Vec<u8>, JsError> {
    Ok(oc_crdt::encode_doc_value(&convert::value(&doc)?))
}

/// The design held by a yjs update (`fromY` of a doc with only that update applied).
#[wasm_bindgen(js_name = decodeDesign)]
pub fn decode_design(update: &[u8]) -> Result<JsValue, JsError> {
    convert::to_js(&oc_crdt::decode_doc_value(update).map_err(convert::err)?)
}
