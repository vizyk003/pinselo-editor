// SPDX-License-Identifier: Apache-2.0
//! `DocRef`: a parsed design held on the Rust side, passed to the pure helpers instead of the JS object.
//!
//! The TS helpers take a `DesignDoc` (or a `Pick` of it) on every call. Re-serialising a whole document per call
//! (`sortedPages(doc)` runs on every render) would cost O(document) at the boundary, so the facade caches one
//! `DocRef` per JS document object (a `WeakMap`):
//!
//! * documents the editor handed out (`editor.doc` / `editor.view`) map to the editor's own parsed snapshot
//!   ([`DocRef::shared`], an `Arc` clone: free);
//! * any other object (a template, a fetched design, `{ elements }`) is parsed once ([`DocRef::from_js`]).
//!
//! Helpers return ids for element/page results; the facade maps them back to the objects of the JS document, so
//! results keep their identity like the TS ones.

use std::sync::Arc;

use oc_editor_core::DocView;
use oc_schema::DesignDoc;
use oc_schema::factory::{MetaInit, make_meta};
use serde_json::Value;
use wasm_bindgen::prelude::*;

use crate::convert::{element, value};

enum Inner {
    Shared(Arc<DocView>),
    Owned(Box<DesignDoc>),
}

#[wasm_bindgen]
pub struct DocRef {
    inner: Inner,
}

impl DocRef {
    pub fn shared(v: Arc<DocView>) -> DocRef {
        DocRef { inner: Inner::Shared(v) }
    }

    pub fn owned(d: DesignDoc) -> DocRef {
        DocRef { inner: Inner::Owned(Box::new(d)) }
    }

    pub fn doc(&self) -> &DesignDoc {
        match &self.inner {
            Inner::Shared(v) => &v.doc,
            Inner::Owned(d) => d,
        }
    }
}

#[wasm_bindgen]
impl DocRef {
    /// Parse a JS design (or a `Pick<DesignDoc, …>`: missing parts are empty, a missing `meta` is a 100×100 default).
    #[wasm_bindgen(js_name = fromJs)]
    pub fn from_js(doc: JsValue) -> Result<DocRef, JsError> {
        Ok(DocRef::owned(design_from_value(value(&doc)?)))
    }

    /// Number of elements (diagnostics).
    #[wasm_bindgen(getter, js_name = elementCount)]
    pub fn element_count(&self) -> u32 {
        self.doc().elements.len() as u32
    }
}

/// A design from JSON, tolerant like the editor's snapshot views: elements that do not match the schema are kept
/// as `Element::Unknown`, unparsable pages/assets are skipped. Object key order is kept (JS insertion order).
pub fn design_from_value(v: Value) -> DesignDoc {
    let Value::Object(mut m) = v else {
        return DesignDoc { meta: default_meta(), pages: Default::default(), elements: Default::default(), assets: Default::default(), extra: Default::default() };
    };
    let meta = m.remove("meta").and_then(|x| serde_json::from_value(x).ok()).unwrap_or_else(default_meta);
    let pages = match m.remove("pages") {
        Some(Value::Object(p)) => p.into_iter().filter_map(|(k, v)| serde_json::from_value(v).ok().map(|p| (k, p))).collect(),
        _ => Default::default(),
    };
    let elements = match m.remove("elements") {
        Some(Value::Object(e)) => e.into_iter().map(|(k, v)| (k, element(v))).collect(),
        _ => Default::default(),
    };
    let assets = match m.remove("assets") {
        Some(Value::Object(a)) => a.into_iter().filter_map(|(k, v)| serde_json::from_value(v).ok().map(|a| (k, a))).collect(),
        _ => Default::default(),
    };
    DesignDoc { meta, pages, elements, assets, extra: Default::default() }
}

fn default_meta() -> oc_schema::design::DesignMeta {
    make_meta(&MetaInit { w: 100.0, h: 100.0, ..Default::default() })
}
