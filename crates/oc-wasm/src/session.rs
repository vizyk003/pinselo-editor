// SPDX-License-Identifier: Apache-2.0
//! The bridge core without wasm-bindgen types, so it is unit-testable natively: an `oc_editor_core::Editor` plus
//! the bookkeeping the TS facade needs to mirror it cheaply.
//!
//! * **Status word.** Every mutating entry point returns [`Session::status`]: `version * 4 + overrides * 2 +
//!   pending`, one `f64` across the boundary. The facade compares the version with the last one it saw (notify
//!   listeners, drop cached snapshots), knows whether the gesture overrides are non-empty (the view differs from
//!   the document) and whether local yrs updates wait to be forwarded to the browser's yjs `Y.Doc`.
//! * **Local updates** are drained from the editor after every call ([`Session::settle`]) and kept until the
//!   facade takes them ([`Session::take_updates`]).
//! * **Document deltas.** The facade keeps a JS mirror of the document with structural sharing. [`Session::doc_delta`]
//!   reports what changed since the last delta by comparing snapshot `Arc`s (the DocStore re-reads only the
//!   elements a transaction touched, so unchanged elements keep their `Arc`): O(n) pointer compares in Rust,
//!   O(changed) serialisation across the boundary.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use oc_crdt::DocSnapshot;
use indexmap::IndexMap;
use oc_editor_core::{Clock, Editor, EditorOptions, EditorState, ManualClock, Mulberry32, OsRandom, RandomSource, Viewport};
use serde::Serialize;
use serde_json::{Map, Value};

/// A random source that counts the bytes drawn (diagnostics; the golden replay compares it with TS).
pub struct CountingRng {
    inner: Box<dyn RandomSource>,
    drawn: Arc<AtomicU64>,
}

impl RandomSource for CountingRng {
    fn fill(&mut self, buf: &mut [u8]) {
        self.inner.fill(buf);
        self.drawn.fetch_add(buf.len() as u64, Ordering::Relaxed);
    }
}

/// Construction options (all optional on the JS side).
pub struct SessionOptions {
    pub viewport: Viewport,
    /// yrs client id of the mirror. Must differ from the browser doc's (random when `None`).
    pub client_id: Option<u64>,
    /// Deterministic ids (`Mulberry32`); `None` = Web Crypto.
    pub seed: Option<u32>,
    /// Wall clock; `None` = a manual clock starting at `start_ms` that the host advances.
    pub clock: Option<Arc<dyn Clock>>,
    pub start_ms: f64,
}

impl Default for SessionOptions {
    fn default() -> Self {
        SessionOptions { viewport: Viewport::default(), client_id: None, seed: None, clock: None, start_ms: 1.0 }
    }
}

pub struct Session {
    pub editor: Editor,
    pending: Vec<Vec<u8>>,
    drawn: Arc<AtomicU64>,
    /// The snapshot the facade's mirror reflects (`None`: nothing sent yet).
    sent: Option<Arc<DocSnapshot>>,
    sent_state: Option<EditorState>,
    sent_overrides: Option<IndexMap<String, Map<String, Value>>>,
    manual: Option<Arc<ManualClock>>,
}

/// `{ full, meta?, assets?, pages?: { id: page | null }, elements?: { id: element | null } }` (`null` = removed).
#[derive(Debug, Serialize)]
pub struct DocDelta<'a> {
    pub full: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<&'a Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<&'a Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<BTreeMap<&'a str, Option<&'a Map<String, Value>>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elements: Option<BTreeMap<&'a str, Option<&'a Map<String, Value>>>>,
}

type Entries = BTreeMap<String, Arc<Map<String, Value>>>;

fn diff_entries<'a>(prev: Option<&'a Entries>, cur: &'a Entries) -> Option<BTreeMap<&'a str, Option<&'a Map<String, Value>>>> {
    let mut out = BTreeMap::new();
    for (k, v) in cur {
        let same = prev.and_then(|p| p.get(k)).is_some_and(|p| Arc::ptr_eq(p, v));
        if !same {
            out.insert(k.as_str(), Some(&**v));
        }
    }
    if let Some(p) = prev {
        for k in p.keys() {
            if !cur.contains_key(k) {
                out.insert(k.as_str(), None);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The changes from `prev` (the facade's mirror) to `cur`; everything when `prev` is `None`.
pub fn doc_delta<'a>(prev: Option<&'a DocSnapshot>, cur: &'a DocSnapshot) -> DocDelta<'a> {
    let changed = |a: Option<&Arc<Map<String, Value>>>, b: &'a Arc<Map<String, Value>>| match a {
        Some(a) if Arc::ptr_eq(a, b) || **a == **b => None,
        _ => Some(&**b),
    };
    DocDelta {
        full: prev.is_none(),
        meta: changed(prev.map(|p| &p.meta), &cur.meta),
        assets: changed(prev.map(|p| &p.assets), &cur.assets),
        pages: diff_entries(prev.map(|p| &p.pages), &cur.pages),
        elements: diff_entries(prev.map(|p| &p.elements), &cur.elements),
    }
}

impl Session {
    /// A mirror of the browser document: `update` is `Y.encodeStateAsUpdate(ydoc)` (lib0 v1; empty = new doc).
    pub fn new(update: &[u8], o: SessionOptions) -> Result<Session, String> {
        let ydoc = match o.client_id {
            Some(id) => oc_crdt::new_doc_with_client(id),
            None => oc_crdt::new_doc(),
        };
        if !update.is_empty() {
            oc_crdt::apply_update(&ydoc, update, None).map_err(|e| format!("initial update: {e}"))?;
        }
        let drawn = Arc::new(AtomicU64::new(0));
        let inner: Box<dyn RandomSource> = match o.seed {
            Some(s) => Box::new(Mulberry32::new(s)),
            None => Box::new(OsRandom),
        };
        let (clock, manual): (Arc<dyn Clock>, _) = match o.clock {
            Some(c) => (c, None),
            None => {
                let m = ManualClock::new(o.start_ms);
                (m.clone(), Some(m))
            }
        };
        let rng = CountingRng { inner, drawn: drawn.clone() };
        let editor = Editor::from_ydoc(ydoc, EditorOptions { viewport: o.viewport, clock, rng: Box::new(rng) });
        Ok(Session { editor, pending: Vec::new(), drawn, sent: None, sent_state: None, sent_overrides: None, manual })
    }

    /// `version * 4 + (overrides non-empty) * 2 + (local updates pending)`.
    pub fn status(&self) -> f64 {
        let v = self.editor.version() as f64;
        v * 4.0 + if self.editor.overrides().is_empty() { 0.0 } else { 2.0 } + if self.pending.is_empty() { 0.0 } else { 1.0 }
    }

    /// Collect the local updates the last call produced; returns the status word.
    pub fn settle(&mut self) -> f64 {
        let ups = self.editor.drain_local_updates();
        if !ups.is_empty() {
            self.pending.extend(ups);
        }
        self.status()
    }

    /// Local updates (lib0 v1) to forward to the browser's yjs doc, oldest first.
    pub fn take_updates(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending)
    }

    /// Apply an update from the browser's yjs doc (local UI writes such as TipTap or the board, or peers via the
    /// provider). The editor's reactions to it may produce local updates of their own.
    pub fn apply_remote_update(&mut self, update: &[u8]) -> Result<f64, String> {
        self.editor.apply_remote_update(update).map_err(|e| e.to_string())?;
        Ok(self.settle())
    }

    /// The changes since the last delta, passed to `f` (`None` when the document is unchanged).
    pub fn doc_delta<R>(&mut self, f: impl FnOnce(&DocDelta<'_>) -> R) -> Option<R> {
        let cur = self.editor.store().doc();
        if self.sent.as_ref().is_some_and(|s| Arc::ptr_eq(s, &cur)) {
            return None;
        }
        let prev = self.sent.take();
        let r = f(&doc_delta(prev.as_deref(), &cur));
        self.sent = Some(cur);
        Some(r)
    }

    /// The editor state if it differs from the one last returned here (`None`: unchanged, keep the old object).
    /// The TS editor replaces `state` only in `set()`, so document-only changes keep its identity; so does this.
    pub fn state_if_changed(&mut self) -> Option<&EditorState> {
        if self.sent_state.as_ref() == Some(&self.editor.state) {
            return None;
        }
        self.sent_state = Some(self.editor.state.clone());
        self.sent_state.as_ref()
    }

    /// The gesture overrides if they differ from the ones last returned here.
    pub fn overrides_if_changed(&mut self) -> Option<&IndexMap<String, Map<String, Value>>> {
        if self.sent_overrides.as_ref() == Some(self.editor.overrides()) {
            return None;
        }
        self.sent_overrides = Some(self.editor.overrides().clone());
        self.sent_overrides.as_ref()
    }

    /// Forget what the facade mirrors (the next delta, state and overrides are sent in full).
    pub fn reset_mirror(&mut self) {
        self.sent = None;
        self.sent_state = None;
        self.sent_overrides = None;
    }

    /// Bytes drawn from the random source so far.
    pub fn random_bytes_drawn(&self) -> u64 {
        self.drawn.load(Ordering::Relaxed)
    }

    /// Set the manual clock (no effect with a host clock). Returns whether the clock is manual.
    pub fn set_now(&self, ms: f64) -> bool {
        self.manual.as_ref().map(|m| m.set(ms)).is_some()
    }

    pub fn now(&self) -> f64 {
        self.editor.now()
    }

    /// `templatePages` drawing ids from the editor's random stream (deterministic with a seed).
    pub fn template_pages(&mut self, tpl: &oc_schema::DesignDoc, w: f64, h: f64) -> Vec<oc_editor_core::ImportedPage> {
        oc_editor_core::template_pages(tpl, w, h, self.editor.rng_mut())
    }
}
