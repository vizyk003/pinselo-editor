// SPDX-License-Identifier: Apache-2.0
//! DocStore: a Y.Doc plus an incrementally maintained immutable JSON snapshot.
//! Only elements touched by a transaction are re-read, so a keystroke in one text box costs O(1) element reads.
//!
//! Differences from the TS class, forced by Rust/yrs: the undo manager's clock is passed in (no clock in lib
//! code), listeners must not open a transaction on the same document from inside the callback (yrs
//! transactions are exclusive), and `local` is false only for updates applied through [`DocStore::apply_update`]
//! (yrs does not expose the transaction's `local` flag).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Map, Value};
use yrs::sync::Clock;
use yrs::types::{Event, PathSegment};
use yrs::undo::Options as UndoOptions;
use yrs::{DeepObservable, Doc, Map as _, Observable, Origin, Out, ReadTxn, Transact, TransactionMut, UndoManager};

use crate::codec::{Roots, apply_update, map_to_json, read_element, read_meta, read_page, roots};
use crate::error::CrdtError;
use crate::ops::{ApplyOptions, DocOp, DocOpResult, apply_ops};
use crate::origins::OcOrigin;

/// What a transaction touched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Change {
    pub elements: BTreeSet<String>,
    pub pages: BTreeSet<String>,
    pub meta: bool,
    pub assets: bool,
    pub local: bool,
    pub origin: Option<Origin>,
}

/// An immutable design snapshot with structural sharing (unchanged elements/pages keep their `Arc`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocSnapshot {
    pub meta: Arc<Map<String, Value>>,
    pub pages: BTreeMap<String, Arc<Map<String, Value>>>,
    pub elements: BTreeMap<String, Arc<Map<String, Value>>>,
    pub assets: Arc<Map<String, Value>>,
}

impl DocSnapshot {
    fn read<T: ReadTxn>(txn: &T, r: &Roots) -> Self {
        let mut s = DocSnapshot {
            meta: Arc::new(read_meta(txn, &r.meta)),
            assets: Arc::new(map_to_json(txn, &r.assets)),
            ..Default::default()
        };
        for (id, m) in r.pages.iter(txn) {
            if let Out::YMap(m) = m {
                s.pages.insert(id.to_string(), Arc::new(read_page(txn, &m)));
            }
        }
        for (id, m) in r.elements.iter(txn) {
            if let Out::YMap(m) = m {
                s.elements.insert(id.to_string(), Arc::new(read_element(txn, &m)));
            }
        }
        s
    }

    /// The full JSON snapshot (`{ meta, pages, elements, assets }`).
    pub fn to_value(&self) -> Value {
        let mut out = Map::new();
        out.insert("meta".into(), Value::Object((*self.meta).clone()));
        out.insert("pages".into(), Value::Object(self.pages.iter().map(|(k, v)| (k.clone(), Value::Object((**v).clone()))).collect()));
        out.insert(
            "elements".into(),
            Value::Object(self.elements.iter().map(|(k, v)| (k.clone(), Value::Object((**v).clone()))).collect()),
        );
        out.insert("assets".into(), Value::Object((*self.assets).clone()));
        Value::Object(out)
    }
}

/// A snapshot listener.
pub type Listener = Arc<dyn Fn(&Arc<DocSnapshot>, &Change) + Send + Sync>;

#[derive(Default)]
struct Inner {
    snap: Arc<DocSnapshot>,
    pending: Option<Change>,
    listeners: Vec<(u64, Listener)>,
}

fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

/// The yjs `Y.Doc` + snapshot + undo manager used by editors and services.
pub struct DocStore {
    ydoc: Doc,
    roots: Roots,
    undo: UndoManager,
    inner: Arc<Mutex<Inner>>,
    remote: Arc<AtomicBool>,
    key: String,
    next_listener: AtomicU64,
    /// Tombstone author for [`DocStore::apply`].
    pub actor: String,
}

fn change(inner: &Mutex<Inner>, f: impl FnOnce(&mut Change)) {
    let mut g = lock(inner);
    let c = g.pending.get_or_insert_with(|| Change { local: true, ..Default::default() });
    f(c);
}

fn first_key(ev: &Event) -> Option<String> {
    match ev.path().front() {
        Some(PathSegment::Key(k)) => Some(k.to_string()),
        _ => None,
    }
}

fn changed_keys(txn: &TransactionMut, ev: &Event) -> Vec<String> {
    match ev {
        Event::Map(e) => e.keys(txn).keys().map(|k| k.to_string()).collect(),
        _ => vec![],
    }
}

impl DocStore {
    /// Wrap a document (create it with [`crate::new_doc`]). `clock` drives the undo manager's 500 ms capture
    /// window (`Arc::new(|| now_ms())`).
    pub fn new(ydoc: Doc, clock: Arc<dyn Clock>) -> Self {
        let r = roots(&ydoc);
        let snap = {
            let txn = ydoc.transact();
            Arc::new(DocSnapshot::read(&txn, &r))
        };
        let inner = Arc::new(Mutex::new(Inner { snap, ..Default::default() }));
        let remote = Arc::new(AtomicBool::new(false));
        let key = format!("oc-docstore-{}", NEXT_KEY.fetch_add(1, Ordering::Relaxed));

        let mut undo = UndoManager::with_options(UndoOptions {
            capture_timeout_millis: 500,
            tracked_origins: Default::default(),
            capture_transaction: None,
            timestamp: clock,
            init_undo_stack: Vec::new(),
            init_redo_stack: Vec::new(),
        });
        undo.include_origin(OcOrigin::Local.origin());
        for scope in [&r.meta, &r.pages, &r.elements, &r.assets] {
            undo.expand_scope(&ydoc, scope);
        }

        let i = inner.clone();
        r.elements.observe_deep(key.as_str(), move |txn, events| {
            change(&i, |c| {
                for ev in events.iter() {
                    if ev.path().is_empty() {
                        c.elements.extend(changed_keys(txn, ev));
                    } else if let Some(k) = first_key(ev) {
                        c.elements.insert(k);
                    }
                }
            });
        });
        let i = inner.clone();
        r.pages.observe_deep(key.as_str(), move |txn, events| {
            change(&i, |c| {
                for ev in events.iter() {
                    if ev.path().is_empty() {
                        c.pages.extend(changed_keys(txn, ev));
                    } else if let Some(k) = first_key(ev) {
                        c.pages.insert(k);
                    }
                }
            });
        });
        let i = inner.clone();
        r.meta.observe(key.as_str(), move |_, _| change(&i, |c| c.meta = true));
        let i = inner.clone();
        r.assets.observe(key.as_str(), move |_, _| change(&i, |c| c.assets = true));

        let i = inner.clone();
        let rr = r.clone();
        let rem = remote.clone();
        ydoc.observe_after_transaction(key.as_str(), move |txn| {
            let (snap, c, listeners) = {
                let mut g = lock(&i);
                let Some(mut c) = g.pending.take() else { return };
                c.origin = txn.origin().cloned();
                c.local = !rem.load(Ordering::SeqCst) && c.origin.as_ref().and_then(OcOrigin::from_origin) != Some(OcOrigin::Remote);
                let next = Arc::new(rebuild(&g.snap, &c, txn, &rr));
                g.snap = next.clone();
                (next, c, g.listeners.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>())
            };
            for l in listeners {
                l(&snap, &c);
            }
        })
        .expect("no transaction is open while the store is created");

        DocStore { ydoc, roots: r, undo, inner, remote, key, next_listener: AtomicU64::new(1), actor: "local".into() }
    }

    pub fn ydoc(&self) -> &Doc {
        &self.ydoc
    }

    pub fn roots(&self) -> &Roots {
        &self.roots
    }

    /// The current snapshot.
    pub fn doc(&self) -> Arc<DocSnapshot> {
        lock(&self.inner).snap.clone()
    }

    /// Apply ops (default origin: `Local`, tracked by undo). `actor` defaults to the store's.
    pub fn apply(&self, ops: &[DocOp], origin: Option<OcOrigin>, mut opts: ApplyOptions<'_>) -> DocOpResult {
        if opts.actor.is_none() {
            opts.actor = Some(self.actor.clone());
        }
        apply_ops(&self.ydoc, ops, Some(origin.unwrap_or(OcOrigin::Local).origin()), opts)
    }

    /// Apply a remote update (the change is reported with `local: false`).
    pub fn apply_update(&self, update: &[u8], origin: Option<OcOrigin>) -> Result<(), CrdtError> {
        self.remote.store(true, Ordering::SeqCst);
        let r = apply_update(&self.ydoc, update, Some(origin.unwrap_or(OcOrigin::Remote).origin()));
        self.remote.store(false, Ordering::SeqCst);
        r
    }

    /// Register a listener; returns its id for [`DocStore::unsubscribe`].
    pub fn subscribe(&self, f: Listener) -> u64 {
        let id = self.next_listener.fetch_add(1, Ordering::Relaxed);
        lock(&self.inner).listeners.push((id, f));
        id
    }

    pub fn unsubscribe(&self, id: u64) -> bool {
        let mut g = lock(&self.inner);
        let n = g.listeners.len();
        g.listeners.retain(|(i, _)| *i != id);
        g.listeners.len() != n
    }

    /// Start a fresh undo step (call at the start of a gesture so it is not merged with the previous one).
    pub fn checkpoint(&mut self) {
        self.undo.reset();
    }

    pub fn undo(&mut self) -> bool {
        self.undo.undo_blocking()
    }

    pub fn redo(&mut self) -> bool {
        self.undo.redo_blocking()
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// Re-read everything (after the provider's initial sync replaces the doc contents).
    pub fn reload(&self) {
        let snap = {
            let txn = self.ydoc.transact();
            Arc::new(DocSnapshot::read(&txn, &self.roots))
        };
        let c = Change { meta: true, assets: true, local: false, ..Default::default() };
        let listeners: Vec<Listener> = {
            let mut g = lock(&self.inner);
            g.snap = snap.clone();
            g.listeners.iter().map(|(_, l)| l.clone()).collect()
        };
        for l in listeners {
            l(&snap, &c);
        }
    }

    /// Drop listeners and observers.
    pub fn destroy(&mut self) {
        lock(&self.inner).listeners.clear();
        self.roots.elements.unobserve_deep(self.key.as_str());
        self.roots.pages.unobserve_deep(self.key.as_str());
        self.roots.meta.unobserve(self.key.as_str());
        self.roots.assets.unobserve(self.key.as_str());
        let _ = self.ydoc.unobserve_after_transaction(self.key.as_str());
    }
}

impl Drop for DocStore {
    fn drop(&mut self) {
        self.destroy();
    }
}

fn rebuild(prev: &DocSnapshot, c: &Change, txn: &TransactionMut, r: &Roots) -> DocSnapshot {
    let mut next = prev.clone();
    for id in &c.elements {
        match crate::codec::element_map(txn, r, id) {
            Some(m) => {
                next.elements.insert(id.clone(), Arc::new(read_element(txn, &m)));
            }
            None => {
                next.elements.remove(id);
            }
        }
    }
    for id in &c.pages {
        match crate::codec::page_map(txn, r, id) {
            Some(m) => {
                next.pages.insert(id.clone(), Arc::new(read_page(txn, &m)));
            }
            None => {
                next.pages.remove(id);
            }
        }
    }
    if c.meta {
        next.meta = Arc::new(read_meta(txn, &r.meta));
    }
    if c.assets {
        next.assets = Arc::new(map_to_json(txn, &r.assets));
    }
    next
}
