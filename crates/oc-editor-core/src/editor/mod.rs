// SPDX-License-Identifier: Apache-2.0
//! Editor engine (docs/plan/03 §8). DOM-free: the host feeds it pointer/keyboard input in scene coordinates
//! and renders `state` + `view()`. All persistent changes go through `DocStore::apply` (DocOps). Gestures write
//! transient overrides (previews) and commit once on pointer-up, so a drag is one undo step and peers never see
//! half-finished moves. Port of `editor.ts`.
//!
//! Change handling: the TS editor reacts to every DocStore transaction (sanitise the selection, fit auto-height
//! text, re-route glued connectors, grow whiteboards) from inside the store listener. A transaction opened by
//! such a reaction writes to the Y.Doc at once, but yjs defers its cleanup (observers, the store's snapshot
//! rebuild, listeners) until the current one finishes, in FIFO order; the rebuild then re-reads the keys that
//! transaction touched from the Y.Doc *as it is at that moment* (later writes included) over the previous
//! snapshot. yrs forbids opening a transaction from a listener, so the Rust store listener only queues the
//! change; the editor drains the queue after each store operation in the same order and rebuilds the same
//! intermediate snapshots (previous one ⊕ the change's keys read from the current store), which `doc()` reports
//! while a reaction runs.

mod board;
pub mod commands;
mod keys;
mod pointer;
mod table;
pub mod text;
pub mod timeline;
pub mod vector;

use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use oc_crdt::{ApplyOptions, Change, DocOp, DocOpError, DocOpErrorCode, DocOpResult, DocSnapshot, DocStore, OcOrigin, PageOp};
use oc_geometry::{
    connector_geometry, connectors_of, selection_bounds, world_bounds, world_matrix,
};
use oc_schema::design::Surface;
use oc_schema::elements::{Element, TextEl};
use oc_schema::factory::form_layout_of;
use oc_schema::primitives::{Mat2D, Rect, Vec2, Xf};
use oc_schema::table::table_fit_rows;
use oc_schema::tree::{ancestors, children_of, sorted_pages};
use serde_json::{Map, Value};

use crate::doc::{DocView, ViewCache, with_overrides};
use crate::env::{Clock, RandomSource, undo_clock};
use crate::state::*;
use crate::text_service::TextService;
use crate::transform::Handle;
use crate::util::{jv, max, min};
use crate::viewport::{Viewport, fit_rect, page_at_scene, slot_of, zoom_at};

pub use table::CellTextFrame;
pub use text::EditingTextFrame;

pub(crate) type Doc = Arc<DocView>;

/// Construction options.
pub struct EditorOptions {
    pub viewport: Viewport,
    pub clock: Arc<dyn Clock>,
    pub rng: Box<dyn RandomSource>,
}

impl Default for EditorOptions {
    fn default() -> Self {
        EditorOptions { viewport: Viewport::default(), clock: Arc::new(|| 1.0), rng: Box::new(crate::env::OsRandom) }
    }
}

/// A gesture in progress (pointer down → up).
#[derive(Debug, Clone)]
pub(crate) enum Gesture {
    Move { start: Vec2, origin: IndexMap<String, Xf>, start_bounds: Rect, page_id: String, moved: bool, duplicate: bool },
    Endpoint { id: String, end: u8, page_id: String, other: Vec2 },
    Resize { handle: Handle, origin: IndexMap<String, Element>, bounds: Rect, single: bool, page_id: String },
    Rotate { origin: IndexMap<String, Xf>, pivot: Vec2, start_angle: f64, page_id: String },
    Marquee { start: Vec2, page_id: String, additive: Vec<String> },
    TextSelect { id: String },
    CropPan { id: String, start: Vec2, crop: oc_schema::primitives::StoredRect },
    CellSelect { id: String, page_id: String, start: oc_schema::table::CellPos, moved: bool },
    ColResize { id: String, page_id: String, index: i64 },
    /// Node or handle drag while editing a path (DS-100), in the element's own pixel space.
    PathEdit {
        id: String,
        start: Vec2,
        origin: Box<Element>,
        path: oc_geometry::VectorPath,
        node: oc_geometry::NodeRef,
        part: oc_geometry::NodePart,
        nodes: Vec<oc_geometry::NodeRef>,
        moved: bool,
    },
}

type Queue = Arc<Mutex<VecDeque<Change>>>;

static NEXT_EDITOR: AtomicU64 = AtomicU64::new(1);

pub struct Editor {
    store: DocStore,
    pub state: EditorState,
    pub(crate) text_service: Option<Box<dyn TextService>>,
    /// Brand-template locks (A8): false for members working on a design started from a brand template —
    /// `lock.by === 'brand'` masks are then enforced on every write. Writing the field does not notify
    /// (like the TS field); [`Editor::set_brand_admin`] does.
    pub brand_admin: bool,
    /// The local user (sticky-note authors, mind-map nodes).
    pub user: Option<User>,
    /// Transient overrides merged over the document while a gesture is running (insertion ordered).
    pub(crate) overrides: IndexMap<String, Map<String, Value>>,
    overrides_rev: u64,
    pub(crate) gesture: Option<Gesture>,
    listeners: Vec<(u64, Box<dyn FnMut()>)>,
    next_listener: u64,
    pub(crate) clipboard: Option<ClipboardPayload>,
    /// Styles to apply to the next typed characters (toggled with no selection).
    pub(crate) pending_marks: Option<Map<String, Value>>,
    version: u64,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) rng: Box<dyn RandomSource>,
    queue: Queue,
    outbox: Arc<Mutex<Vec<Vec<u8>>>>,
    key: String,
    store_listener: u64,
    cache: RefCell<ViewCache>,
    view_cache: RefCell<Option<(Arc<DocSnapshot>, u64, Doc)>>,
    handling: Option<Arc<DocSnapshot>>,
    /// The snapshot the last processed change left (TS `store.snap`); the base the next one is rebuilt over.
    settled: Arc<DocSnapshot>,
    processing: bool,
    /// Set while the changes of an undo/redo are being reacted to (see [`Editor::store_apply`]).
    pub(crate) in_history: bool,
}

pub(crate) fn op_update(id: &str, patch: Map<String, Value>) -> DocOp {
    DocOp::Update { id: id.to_string(), patch }
}

pub(crate) fn op_xf(id: &str, xf: &Xf) -> DocOp {
    DocOp::Xf { id: id.to_string(), xf: jv(xf) }
}

pub(crate) fn op_add(el: Map<String, Value>, parent: &str, after: Option<&str>) -> DocOp {
    DocOp::Add { el: Value::Object(el), parent: parent.to_string(), after: after.map(str::to_string) }
}

pub(crate) fn op_page_update(page_id: &str, patch: Map<String, Value>) -> DocOp {
    DocOp::Page(PageOp::Update { page_id: page_id.to_string(), patch })
}

pub(crate) fn op_meta(patch: Map<String, Value>) -> DocOp {
    DocOp::Meta { patch }
}

pub(crate) fn one(k: &str, v: Value) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(k.to_string(), v);
    m
}

pub(crate) fn el_xf(el: &Element) -> Xf {
    el.xf().cloned().unwrap_or_default()
}

pub(crate) fn el_id(el: &Element) -> String {
    el.id().unwrap_or("").to_string()
}

pub(crate) fn lock_of(el: &Element) -> Option<&oc_schema::elements::LockMask> {
    el.base().and_then(|b| b.lock.as_ref())
}

pub(crate) fn locked_move(el: &Element) -> bool {
    lock_of(el).is_some_and(|l| l.move_ == Some(true))
}

pub(crate) fn locked_content(el: &Element) -> bool {
    lock_of(el).is_some_and(|l| l.content == Some(true))
}

pub(crate) fn visible(el: &Element) -> bool {
    el.base().is_none_or(|b| b.visible != Some(false))
}

impl Editor {
    /// Wrap a store. The store's undo clock should be the same clock (see [`Editor::from_ydoc`]).
    pub fn new(store: DocStore, opts: EditorOptions) -> Self {
        let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));
        let q = queue.clone();
        let store_listener = store.subscribe(Arc::new(move |_: &Arc<DocSnapshot>, c: &Change| {
            q.lock().unwrap_or_else(|e| e.into_inner()).push_back(c.clone());
        }));
        let outbox: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let key = format!("oc-editor-{}", NEXT_EDITOR.fetch_add(1, Ordering::Relaxed));
        let ob = outbox.clone();
        let _ = store.ydoc().observe_update_v1(key.as_str(), move |txn, e| {
            let remote = txn.origin().and_then(OcOrigin::from_origin) == Some(OcOrigin::Remote);
            if !remote {
                ob.lock().unwrap_or_else(|e| e.into_inner()).push(e.update.clone());
            }
        });
        let settled = store.doc();
        let first = {
            let snap = store.doc();
            let mut cache = ViewCache::default();
            let v = cache.view(&snap);
            sorted_pages(&v.doc.pages).first().map(|p| p.id.clone()).unwrap_or_default()
        };
        Editor {
            store,
            state: EditorState {
                active_page_id: first,
                selection: vec![],
                hover: None,
                entered_group: None,
                editing_text: None,
                cropping_id: None,
                viewport: opts.viewport,
                guides: vec![],
                marquee: None,
                gesturing: false,
                presenting: false,
                tool: Tool::Select,
                brush: BrushState { kind: oc_schema::elements::Brush::Pen, color: "#1f2328".into(), size: 6.0 },
                drawing: None,
                editing_cell: None,
                cell_range: None,
                preview: None,
                playhead: None,
                playing: false,
                play_anchor: None,
                spacing: vec![],
                connect_target: None,
                style_brush: None,
                record_keyframes: false,
                pen: None,
                path_edit: None,
            },
            text_service: None,
            brand_admin: true,
            user: None,
            overrides: IndexMap::new(),
            overrides_rev: 0,
            gesture: None,
            listeners: Vec::new(),
            next_listener: 1,
            clipboard: None,
            pending_marks: None,
            version: 0,
            clock: opts.clock,
            rng: opts.rng,
            queue,
            outbox,
            key,
            store_listener,
            cache: RefCell::new(ViewCache::default()),
            view_cache: RefCell::new(None),
            handling: None,
            settled,
            processing: false,
            in_history: false,
        }
    }

    /// Create the store for a yrs document (made with `oc_crdt::new_doc`) and wrap it.
    pub fn from_ydoc(ydoc: yrs::Doc, opts: EditorOptions) -> Self {
        let store = DocStore::new(ydoc, undo_clock(opts.clock.clone()));
        Editor::new(store, opts)
    }

    // ------------------------------------------------------------------ plumbing

    pub fn store(&self) -> &DocStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut DocStore {
        &mut self.store
    }

    pub fn set_text_service(&mut self, ts: Option<Box<dyn TextService>>) {
        self.text_service = ts;
    }

    pub fn has_text_service(&self) -> bool {
        self.text_service.is_some()
    }

    pub fn now(&self) -> f64 {
        self.clock.now()
    }

    /// The editor's random source (ids for host-built elements, e.g. `template_pages`, come from the same
    /// stream as the editor's own, like `crypto.getRandomValues` in TS).
    pub fn rng_mut(&mut self) -> &mut dyn RandomSource {
        self.rng.as_mut()
    }

    /// The document (during a change reaction: the snapshot of the transaction being handled).
    pub fn doc(&self) -> Doc {
        let snap = match &self.handling {
            Some(s) => s.clone(),
            None => self.store.doc(),
        };
        self.cache.borrow_mut().view(&snap)
    }

    /// Document view including transient overrides (what the renderer and overlay should show).
    pub fn view(&self) -> Doc {
        let d = self.doc();
        if self.overrides.is_empty() {
            return d;
        }
        if let Some((s, rev, v)) = &*self.view_cache.borrow()
            && Arc::ptr_eq(s, &d.snap)
            && *rev == self.overrides_rev
        {
            return v.clone();
        }
        let v = Arc::new(with_overrides(&d, self.overrides.iter()));
        *self.view_cache.borrow_mut() = Some((d.snap.clone(), self.overrides_rev, v.clone()));
        v
    }

    /// The design as JSON (`store.doc` in TS).
    pub fn doc_json(&self) -> Value {
        self.doc().to_value()
    }

    /// `view` as JSON.
    pub fn view_json(&self) -> Value {
        self.view().to_value()
    }

    pub fn overrides(&self) -> &IndexMap<String, Map<String, Value>> {
        &self.overrides
    }

    pub(crate) fn set_override(&mut self, id: &str, patch: Map<String, Value>) {
        self.overrides.insert(id.to_string(), patch);
        self.overrides_rev += 1;
    }

    pub(crate) fn clear_overrides(&mut self) {
        if !self.overrides.is_empty() {
            self.overrides.clear();
            self.overrides_rev += 1;
        }
    }

    /// Register a change listener (called after every state or document change). Returns its id.
    pub fn subscribe(&mut self, f: Box<dyn FnMut()>) -> u64 {
        let id = self.next_listener;
        self.next_listener += 1;
        self.listeners.push((id, f));
        id
    }

    pub fn unsubscribe(&mut self, id: u64) -> bool {
        let n = self.listeners.len();
        self.listeners.retain(|(i, _)| *i != id);
        n != self.listeners.len()
    }

    /// Bumped on every notification; the UI caches derived values on it.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn emit(&mut self) {
        self.version += 1;
        for (_, l) in self.listeners.iter_mut() {
            l();
        }
    }

    /// `this.set(patch)`: update, normalise the table-cell editing state, notify.
    pub(crate) fn set(&mut self, f: impl FnOnce(&mut EditorState)) {
        let mut next = self.state.clone();
        f(&mut next);
        // Cell editing state only lives while its table is selected and either the caret or a cell range is active.
        let cell_id = next.editing_cell.as_ref().map(|c| c.id.clone()).or_else(|| next.cell_range.as_ref().map(|c| c.id.clone()));
        if let Some(cell_id) = cell_id {
            let typing = next.editing_text.as_ref().is_some_and(|e| e.id == cell_id);
            let ranged = next.cell_range.as_ref().is_some_and(|c| c.id == cell_id) && next.editing_text.is_none();
            if !(typing || ranged)
                || !next.selection.contains(&cell_id)
                || next.editing_cell.as_ref().is_some_and(|c| c.id != cell_id)
            {
                next.editing_cell = None;
                next.cell_range = None;
                if typing {
                    next.editing_text = None;
                }
            }
        }
        self.state = next;
        self.emit();
    }


    /// `store.apply(ops, LocalOrigin.I, { brandAdmin })` + change reactions.
    ///
    /// Deviation from TS: change reactions (auto-height text, glued connectors, whiteboard growth) to an undo or
    /// redo are written with an origin the undo manager does not track. In yjs they run inside the undo
    /// transaction's cleanup, while `UndoManager.undoing` is still set, so they land on the redo (undo) stack as
    /// an extra step; yrs resets its undo/redo flags before observers can write, so a tracked write would clear
    /// the redo stack instead. Untracked, the redo stack keeps exactly the undone steps.
    pub(crate) fn store_apply(&mut self, ops: &[DocOp]) -> DocOpResult {
        let opts = ApplyOptions {
            actor: None,
            now: Some(self.clock.now()),
            partial: false,
            brand_admin: Some(self.brand_admin),
            rng: Some(self.rng.as_mut()),
        };
        let origin = if self.in_history { OcOrigin::Repair } else { OcOrigin::Local };
        let r = self.store.apply(ops, Some(origin), opts);
        self.process_changes();
        r
    }

    /// Apply ops as the local user. Discrete commands are their own undo step; `merge` lets rapid repeats
    /// (typing, nudges, colour-picker drags) coalesce within the undo capture window.
    pub fn apply(&mut self, ops: Vec<DocOp>, merge: bool) -> DocOpResult {
        let mut ops = ops;
        // Brand-template locks (A8): batch commands skip the locked elements instead of failing as a whole;
        // a batch made only of locked edits is reported as rejected.
        if !self.brand_admin {
            let allowed: Vec<DocOp> = ops.iter().filter(|op| self.brand_violation(op).is_none()).cloned().collect();
            if allowed.is_empty() && !ops.is_empty() {
                return DocOpResult {
                    ok: false,
                    applied: 0,
                    created: vec![],
                    errors: vec![DocOpError { index: 0, code: DocOpErrorCode::Locked, message: "locked by the brand template".into() }],
                };
            }
            ops = allowed;
        }
        if !merge {
            self.store.checkpoint();
        }
        let r = self.store_apply(&ops);
        if !merge {
            self.store.checkpoint();
        }
        r
    }

    /// Apply a yjs/yrs update received from the network (the browser's authoritative Y.Doc or a peer).
    ///
    /// The text caret and selection are carried across it as yrs sticky indexes (COL-02): a peer typing in front of
    /// the caret shifts it, so the next local keystroke still lands after the user's own text.
    pub fn apply_remote_update(&mut self, update: &[u8]) -> Result<(), oc_crdt::CrdtError> {
        let carets = self.sticky_caret();
        let r = self.store.apply_update(update, None);
        self.restore_caret(carets);
        self.process_changes();
        r
    }

    /// The text body fragment behind the caret (a text element; table cells are laid out from the table).
    fn caret_fragment<T: yrs::ReadTxn>(&self, txn: &T, id: &str) -> Option<yrs::XmlFragmentRef> {
        if self.state.editing_cell.as_ref().is_some_and(|c| c.id == id) {
            return None;
        }
        let m = oc_crdt::codec::element_map(txn, self.store.roots(), id)?;
        oc_crdt::codec::text_of(txn, &m)
    }

    fn sticky_caret(&self) -> Option<(String, yrs::StickyIndex, yrs::StickyIndex)> {
        use yrs::Transact;
        let e = self.state.editing_text.as_ref()?;
        let txn = self.store.ydoc().transact();
        let frag = self.caret_fragment(&txn, &e.id)?;
        let a = oc_crdt::text::sticky_offset(&txn, &frag, e.anchor)?;
        let f = if e.focus == e.anchor { a.clone() } else { oc_crdt::text::sticky_offset(&txn, &frag, e.focus)? };
        Some((e.id.clone(), a, f))
    }

    fn restore_caret(&mut self, carets: Option<(String, yrs::StickyIndex, yrs::StickyIndex)>) {
        use yrs::Transact;
        let Some((id, a, f)) = carets else { return };
        let moved = {
            let txn = self.store.ydoc().transact();
            let Some(frag) = self.caret_fragment(&txn, &id) else { return };
            (oc_crdt::text::resolve_sticky(&txn, &frag, &a), oc_crdt::text::resolve_sticky(&txn, &frag, &f))
        };
        if let Some(e) = self.state.editing_text.as_mut()
            && e.id == id
        {
            if let Some(a) = moved.0 {
                e.anchor = a;
            }
            if let Some(f) = moved.1 {
                e.focus = f;
            }
        }
    }

    /// Local updates (lib0 v1) produced since the last call, to forward to the authoritative Y.Doc.
    pub fn drain_local_updates(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.outbox.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// React to writes made directly on the yrs document by the host (e.g. `oc_crdt::replace_content` for a version
    /// restore, or `store_mut().apply(..)`): runs the queued change reactions and notifies. Editor commands do this
    /// themselves.
    pub fn flush(&mut self) {
        self.process_changes();
    }

    /// `editor.store.apply(ops, origin)`: a store write with an explicit origin (e.g. `Migration`, which undo does
    /// not track), followed by the usual change reactions. Like the TS store call it is not held to brand locks.
    pub fn store_apply_with(&mut self, ops: &[DocOp], origin: OcOrigin) -> DocOpResult {
        let opts = ApplyOptions {
            actor: None,
            now: Some(self.clock.now()),
            partial: false,
            brand_admin: None,
            rng: Some(self.rng.as_mut()),
        };
        let r = self.store.apply(ops, Some(origin), opts);
        self.process_changes();
        r
    }

    /// Re-read the whole document (after an initial sync replaced the contents).
    pub fn reload(&mut self) {
        self.store.reload();
        self.process_changes();
    }

    /// Run queued change reactions (FIFO, like yjs transaction cleanups).
    pub(crate) fn process_changes(&mut self) {
        if self.processing {
            return;
        }
        self.processing = true;
        loop {
            let next = self.queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
            let Some(change) = next else { break };
            let snap = rebuild(&self.settled, &self.store.doc(), &change);
            self.settled = snap.clone();
            self.handling = Some(snap);
            self.on_change(&change);
        }
        self.handling = None;
        // Every key a processed change touched was re-read, so the settled snapshot now equals the store's; keep
        // the store's `Arc` (the view cache keys on it).
        self.settled = self.store.doc();
        self.processing = false;
    }

    fn on_change(&mut self, change: &Change) {
        let doc = self.doc();
        self.sanitize(&doc);
        if change.local {
            self.fix_text_heights(&doc, &change.elements);
            self.fix_connectors(&doc, &change.elements);
            if doc.doc.meta.surface == Surface::Whiteboard && !change.elements.is_empty() {
                self.expand_whiteboard(600.0);
            }
        }
        self.emit();
    }

    /// Drop selection entries that no longer exist (deleted remotely) and keep the active page valid.
    fn sanitize(&mut self, doc: &DocView) {
        let live = |id: &str| doc.live(id).is_some();
        let s = &mut self.state;
        s.selection.retain(|id| live(id));
        if s.editing_text.as_ref().is_some_and(|e| !live(&e.id)) {
            s.editing_text = None;
        }
        if s.entered_group.as_ref().is_some_and(|g| !live(g)) {
            s.entered_group = None;
        }
        // A remote edit may remove the row/column holding the caret cell.
        if let Some(ec) = s.editing_cell.clone() {
            let bad = match doc.el(&ec.id) {
                Some(Element::Table(t)) => t.base.deleted.is_some() || ec.r >= t.rows.len() as i64 || ec.c >= t.cols.len() as i64,
                _ => true,
            };
            if bad {
                s.editing_cell = None;
                s.cell_range = None;
                if s.editing_text.as_ref().is_some_and(|e| e.id == ec.id) {
                    s.editing_text = None;
                }
            }
        }
        if !doc.has_page(&s.active_page_id)
            && let Some(p) = sorted_pages(&doc.doc.pages).first() {
                s.active_page_id = p.id.clone();
            }
    }

    /// Auto-height text: keep xf.h equal to the laid-out height after local edits.
    fn fix_text_heights(&mut self, doc: &DocView, ids: &BTreeSet<String>) {
        let mut ops = Vec::new();
        for id in ids {
            let Some(el) = doc.el(id) else { continue };
            match el {
                Element::Table(t) if t.base.deleted.is_none() => {
                    if let Some(ts) = &self.text_service {
                        let mut measure =
                            |key: &str, body: &oc_schema::TextBody, width: f64| ts.height(&virtual_text(key, body, width, 1.0), width);
                        if let Some(fit) = table_fit_rows(t, &mut measure, Some(&t.base.id)) {
                            let mut patch = one("rows", jv(&fit.rows));
                            patch.insert("xf".into(), jv(&Xf { h: fit.h, ..t.base.xf.clone() }));
                            ops.push(op_update(id, patch));
                        }
                    }
                    continue;
                }
                Element::Form(f) if f.base.deleted.is_none() => {
                    let h = form_layout_of(f).height;
                    if (h - f.base.xf.h).abs() > 0.5 {
                        ops.push(op_xf(id, &Xf { h, ..f.base.xf.clone() }));
                    }
                    continue;
                }
                _ => {}
            }
            let Element::Text(t) = el else { continue };
            let Some(ts) = &self.text_service else { continue };
            if t.base.deleted.is_some() {
                continue;
            }
            if t.curve.is_some_and(|c| c != 0.0 && !c.is_nan()) {
                // Curved text sizes its box to the arc (scaled by any resize via the font size).
                if let Some((w, h)) = ts.curved_size(t)
                    && ((w - t.base.xf.w).abs() > 0.5 || (h - t.base.xf.h).abs() > 0.5)
                {
                    let cx = t.base.xf.x + t.base.xf.w / 2.0;
                    let cy = t.base.xf.y + t.base.xf.h / 2.0;
                    ops.push(op_xf(id, &Xf { x: cx - w / 2.0, y: cy - h / 2.0, w, h, ..t.base.xf.clone() }));
                }
                continue;
            }
            if t.frame.mode != oc_schema::elements::TextFrameMode::AutoHeight {
                continue;
            }
            let h = ts.height(t, t.base.xf.w);
            if (h - t.base.xf.h).abs() > 0.5 {
                // Text inside a shape stays vertically centred on the shape as it grows or shrinks.
                let shape = if t.base.name.as_deref() == Some("Shape label") {
                    children_of(doc.els(), &t.base.parent_id).into_iter().find(|e| matches!(e, Element::Shape(_)))
                } else {
                    None
                };
                let y = match shape {
                    Some(s) if el_xf(s).rot == 0.0 || el_xf(s).rot.is_nan() => el_xf(s).y + el_xf(s).h / 2.0 - h / 2.0,
                    _ => t.base.xf.y,
                };
                ops.push(op_xf(id, &Xf { y, h, ..t.base.xf.clone() }));
            }
        }
        if !ops.is_empty() {
            self.store_apply(&ops);
        }
    }

    /// Connectors follow the elements they are glued to (persisted with the same local change).
    fn fix_connectors(&mut self, doc: &DocView, ids: &BTreeSet<String>) {
        if ids.is_empty() {
            return;
        }
        let set: std::collections::HashSet<String> = ids.iter().cloned().collect();
        let mut lines: Vec<&oc_schema::elements::LineEl> = connectors_of(doc.els(), &set);
        for id in ids {
            if let Some(Element::Line(l)) = doc.el(id)
                && l.base.deleted.is_none()
                && (l.start_attach.is_some() || l.end_attach.is_some())
                && !lines.iter().any(|x| x.base.id == l.base.id)
            {
                lines.push(l);
            }
        }
        let mut ops = Vec::new();
        for line in lines {
            let Some(g) = connector_geometry(doc.els(), line) else { continue };
            let lx = &line.base.xf;
            let same = (g.xf.x - lx.x).abs() < 0.5
                && (g.xf.y - lx.y).abs() < 0.5
                && (g.xf.w - lx.w).abs() < 0.5
                && (g.xf.h - lx.h).abs() < 0.5
                && g.points.len() == line.points.len()
                && g.points.iter().zip(&line.points).all(|(p, q)| (p[0] - q[0]).abs() < 0.5 && (p[1] - q[1]).abs() < 0.5);
            if same {
                continue;
            }
            ops.push(op_xf(&line.base.id, &g.xf));
            ops.push(op_update(&line.base.id, one("points", jv(&g.points))));
        }
        if !ops.is_empty() {
            self.store_apply(&ops);
        }
    }

    /// Infinite whiteboard (WB-001): the board grows whenever content comes within a margin of an edge.
    /// Growing up/left shifts everything (and the viewport) so nothing moves on screen. It never shrinks.
    /// TS default `margin = 600`.
    pub fn expand_whiteboard(&mut self, margin: f64) {
        let doc = self.doc();
        let (w, h) = (doc.doc.meta.dims.w, doc.doc.meta.dims.h);
        let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in sorted_pages(&doc.doc.pages) {
            for el in children_of(doc.els(), &p.id) {
                let b = world_bounds(doc.els(), el.id().unwrap_or("")).unwrap_or_default();
                x0 = min(x0, b.x);
                y0 = min(y0, b.y);
                x1 = max(x1, b.x + b.w);
                y1 = max(y1, b.y + b.h);
            }
        }
        if !x0.is_finite() {
            return;
        }
        let dx = if x0 < margin { (margin - x0).ceil() } else { 0.0 };
        let dy = if y0 < margin { (margin - y0).ceil() } else { 0.0 };
        let nw = max(w + dx, (x1 + dx + margin).ceil());
        let nh = max(h + dy, (y1 + dy + margin).ceil());
        if dx == 0.0 && dy == 0.0 && nw == w && nh == h {
            return;
        }
        let mut dims = doc.doc.meta.dims.clone();
        dims.w = nw;
        dims.h = nh;
        let mut ops = vec![op_meta(one("dims", jv(&dims)))];
        if dx != 0.0 || dy != 0.0 {
            for p in sorted_pages(&doc.doc.pages) {
                for el in children_of(doc.els(), &p.id) {
                    let x = el_xf(el);
                    ops.push(op_xf(el.id().unwrap_or(""), &Xf { x: x.x + dx, y: x.y + dy, ..x.clone() }));
                }
                if let Some(guides) = p.guides.as_ref().filter(|g| !g.is_empty()) {
                    let next: Vec<oc_schema::design::Guide> = guides
                        .iter()
                        .map(|g| {
                            let mut g = g.clone();
                            g.pos += if g.axis == oc_schema::design::GuideAxis::X { dx } else { dy };
                            g
                        })
                        .collect();
                    ops.push(op_page_update(&p.id, one("guides", jv(&next))));
                }
            }
        }
        self.store_apply(&ops);
        if dx != 0.0 || dy != 0.0 {
            self.state.viewport.x += dx;
            self.state.viewport.y += dy;
            self.emit();
        }
    }

    /// Re-measure every text element (after fonts load).
    pub fn remeasure_text(&mut self) {
        let doc = self.doc();
        let ids: BTreeSet<String> =
            doc.els().values().filter(|e| matches!(e, Element::Text(_))).map(el_id).collect();
        self.fix_text_heights(&doc, &ids);
    }

    // ------------------------------------------------------------------ geometry helpers

    pub fn page_origin(&self, page_id: &str) -> Vec2 {
        slot_of(&self.doc().doc, page_id).map_or([0.0, 0.0], |s| [s.rect.x, s.rect.y])
    }

    /// Scene point → page-local point.
    pub fn to_page(&self, page_id: &str, p: Vec2) -> Vec2 {
        let [ox, oy] = self.page_origin(page_id);
        [p[0] - ox, p[1] - oy]
    }

    /// Selected live elements (from the view, overrides included).
    pub fn selected(&self) -> Vec<Element> {
        let d = self.view();
        self.state.selection.iter().filter_map(|id| d.live(id).cloned()).collect()
    }

    pub fn selection_box(&self) -> Option<Rect> {
        selection_bounds(self.view().els(), &self.state.selection)
    }

    /// The single selected element's xf + world matrix (for rotated selection chrome).
    pub fn selection_frame(&self) -> Option<(Xf, Mat2D)> {
        let sel = self.selected();
        if sel.len() != 1 {
            return None;
        }
        let el = &sel[0];
        Some((el_xf(el), world_matrix(self.view().els(), el.id().unwrap_or(""))))
    }

    fn top_level_siblings(&self, page_id: &str, exclude: &std::collections::HashSet<String>) -> Vec<Rect> {
        let v = self.view();
        let parent = self.state.entered_group.clone().unwrap_or_else(|| page_id.to_string());
        children_of(v.els(), &parent)
            .into_iter()
            .filter(|e| !exclude.contains(e.id().unwrap_or("")) && visible(e))
            .map(|e| world_bounds(v.els(), e.id().unwrap_or("")).unwrap_or_default())
            .collect()
    }

    // ------------------------------------------------------------------ selection

    pub fn select(&mut self, ids: &[String], keep_text: bool) {
        let doc = self.doc();
        // Elements on a locked page (g96) stay unselectable until the page is unlocked.
        let valid: Vec<String> = ids.iter().filter(|id| doc.live(id).is_some() && !doc.on_locked_page(id)).cloned().collect();
        let page_id = valid.first().and_then(|v| doc.page_of(v));
        let entered = match (&valid.first(), &self.state.entered_group) {
            (Some(v0), Some(g)) if ancestors(doc.els(), v0).iter().any(|a| a.id() == Some(g.as_str())) => Some(g.clone()),
            _ => None,
        };
        self.set(|s| {
            s.selection = valid;
            if s.path_edit.as_ref().is_some_and(|pe| s.selection.len() != 1 || s.selection[0] != pe.id) {
                s.path_edit = None;
            }
            if let Some(p) = page_id {
                s.active_page_id = p;
            }
            if !keep_text {
                s.editing_text = None;
            }
            s.cropping_id = None;
            s.entered_group = entered;
        });
        self.pending_marks = None;
    }

    pub fn clear_selection(&mut self) {
        self.set(|s| {
            s.selection = vec![];
            s.path_edit = None;
            s.editing_text = None;
            s.entered_group = None;
            s.cropping_id = None;
        });
    }

    pub fn select_all(&mut self) {
        if let Some(e) = self.state.editing_text.clone() {
            let len = self.edit_target().map_or(0, |t| self.text_len(&t.el));
            self.set(|s| s.editing_text = Some(TextEditState { anchor: 0, focus: len, ..e }));
            return;
        }
        let cr = self.state.cell_range.as_ref().map(|c| c.id.clone()).or_else(|| self.state.editing_cell.as_ref().map(|c| c.id.clone()));
        if let Some(id) = cr
            && let Some(Element::Table(t)) = self.doc().el(&id)
        {
            let (r1, c1) = (t.rows.len() as i64 - 1, t.cols.len() as i64 - 1);
            let id = t.base.id.clone();
            self.select_cell_range(&id, oc_schema::table::CellPos { r: 0, c: 0 }, oc_schema::table::CellPos { r: r1, c: c1 });
            return;
        }
        let parent = self.state.entered_group.clone().unwrap_or_else(|| self.state.active_page_id.clone());
        let doc = self.doc();
        let ids: Vec<String> = children_of(doc.els(), &parent).into_iter().filter(|e| visible(e)).map(el_id).collect();
        self.select(&ids, false);
    }

    pub fn set_active_page(&mut self, page_id: &str, scroll: bool) {
        if !self.doc().has_page(page_id) {
            return;
        }
        let same = page_id == self.state.active_page_id;
        self.set(|s| {
            s.active_page_id = page_id.to_string();
            if !same {
                s.selection = vec![];
            }
            s.editing_text = None;
        });
        if scroll {
            self.scroll_to_page(page_id);
        }
    }

    pub fn set_hover(&mut self, id: Option<String>) {
        if id != self.state.hover {
            self.set(|s| s.hover = id);
        }
    }

    // ------------------------------------------------------------------ viewport

    pub fn set_viewport_size(&mut self, width: f64, height: f64) {
        self.set(|s| {
            s.viewport.width = width;
            s.viewport.height = height;
        });
    }

    pub fn set_viewport(&mut self, vp: Viewport) {
        self.set(|s| s.viewport = vp);
    }

    pub fn zoom_to(&mut self, zoom: f64, screen_pt: Option<Vec2>) {
        let vp = self.state.viewport;
        let next = zoom_at(&vp, zoom, screen_pt.unwrap_or([vp.width / 2.0, vp.height / 2.0]));
        self.set(|s| s.viewport = next);
    }

    pub fn zoom_by(&mut self, factor: f64, screen_pt: Option<Vec2>) {
        self.zoom_to(self.state.viewport.zoom * factor, screen_pt);
    }

    pub fn pan_by(&mut self, dx: f64, dy: f64) {
        self.set(|s| {
            let z = s.viewport.zoom;
            s.viewport.x += dx / z;
            s.viewport.y += dy / z;
        });
    }

    pub fn zoom_to_fit(&mut self) {
        if let Some(sl) = slot_of(&self.doc().doc, &self.state.active_page_id) {
            let vp = fit_rect(&self.state.viewport, &sl.rect, 48.0);
            self.set(|s| s.viewport = vp);
        }
    }

    pub fn scroll_to_page(&mut self, page_id: &str) {
        let Some(sl) = slot_of(&self.doc().doc, page_id) else { return };
        self.set(|s| s.viewport.y = sl.rect.y + sl.rect.h / 2.0 - s.viewport.height / 2.0 / s.viewport.zoom);
    }

    /// Page whose centre is closest to the viewport centre (tracks scrolling).
    pub fn update_active_page_from_scroll(&mut self) {
        let vp = self.state.viewport;
        let cy = vp.y + vp.height / 2.0 / vp.zoom;
        if let Some(sl) = page_at_scene(&self.doc().doc, [0.0, cy])
            && sl.page.id != self.state.active_page_id
            && self.state.selection.is_empty()
        {
            self.set(|s| s.active_page_id = sl.page.id.clone());
        }
    }

    // ------------------------------------------------------------------ misc shared helpers

    pub(crate) fn text_len(&self, el: &TextEl) -> i64 {
        match &self.text_service {
            Some(ts) => ts.length(el),
            None => crate::util::len16(&oc_schema::text::body_plain_text(&el.body)),
        }
    }

}

impl Drop for Editor {
    fn drop(&mut self) {
        self.store.unsubscribe(self.store_listener);
        let _ = self.store.ydoc().unobserve_update_v1(self.key.as_str());
    }
}

/// `DocStore.rebuild` at cleanup time: `prev` with the keys `c` touched re-read from the current document. A
/// reload (no keys, meta and assets, not local) replaces everything, like `DocStore.reload`.
fn rebuild(prev: &Arc<DocSnapshot>, current: &Arc<DocSnapshot>, c: &Change) -> Arc<DocSnapshot> {
    if c.elements.is_empty() && c.pages.is_empty() && c.meta && c.assets && !c.local {
        return current.clone();
    }
    let mut next = (**prev).clone();
    for id in &c.elements {
        match current.elements.get(id) {
            Some(e) => next.elements.insert(id.clone(), e.clone()),
            None => next.elements.remove(id),
        };
    }
    for id in &c.pages {
        match current.pages.get(id) {
            Some(p) => next.pages.insert(id.clone(), p.clone()),
            None => next.pages.remove(id),
        };
    }
    if c.meta {
        next.meta = current.meta.clone();
    }
    if c.assets {
        next.assets = current.assets.clone();
    }
    if next == **current { current.clone() } else { Arc::new(next) }
}

/// Virtual text element the text engine lays out for a body at a width (id doubles as the layout cache key).
pub(crate) fn virtual_text(key: &str, body: &oc_schema::TextBody, width: f64, h: f64) -> TextEl {
    TextEl {
        base: oc_schema::elements::ElementBase {
            id: key.to_string(),
            parent_id: String::new(),
            z: "a0".into(),
            opacity: 1.0,
            xf: Xf::new(0.0, 0.0, width, h, 0.0),
            ..Default::default()
        },
        body: body.clone(),
        frame: oc_schema::elements::TextFrame { mode: oc_schema::elements::TextFrameMode::Fixed, extra: Default::default() },
        auto_fit: oc_schema::elements::AutoFit::None,
        curve: None,
        effect: None,
        valign: None,
    }
}
