// SPDX-License-Identifier: AGPL-3.0-only
//! Formula engine kept in sync with the Y.Doc sheet model (port of `@oc/sheet`'s `SheetEngine`).
//!
//! - Tabs map to workbook sheets by name, so `=Sheet2!A1` / `='My tab'!B3` work across tabs.
//! - Cell edits (local or remote) are applied incrementally; row/column inserts and deletes are replayed
//!   from the Y.Array deltas, so references re-point the way they do in Excel.
//! - Structural edits made here (insert/delete rows/cols, rename/delete tab) let the workbook rewrite every
//!   affected formula and write the rewritten text back in the SAME transaction, so collaborators and the
//!   stored document agree.
//!
//! yrs delivers events from inside the transaction, so a live engine queues them and applies them in
//! [`SheetEngine::sync`] (called by every mutating method here; call it yourself after writing to the doc
//! directly or applying a remote update).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use oc_crdt::rng::{OsRandom, RandomSource};
use oc_crdt::value::out_to_json;
use serde_json::Value;
use yrs::types::{Change as YChange, Event, PathSegment};
use yrs::{DeepObservable, Doc, Map, MapRef, Origin, Transact, TransactionMut};

use crate::a1::Range;
use crate::format::{CellValue, ValueKind};
use crate::formula::workbook::{FixedHost, Host, SheetId, Workbook};
use crate::model::{
    self, Axis, CellFormat, Merge, SHEET_LOCAL, TabSnap, delete_tracks, insert_tracks, sheet_root, snap_tab, snap_tabs, unique_tab_name,
};

/// What the workbook should see for a cell: text-formatted cells are forced to strings (`hfInput`).
pub fn hf_input(raw: Option<&str>, fmt: Option<&CellFormat>) -> Option<String> {
    let raw = raw?;
    if raw.is_empty() {
        return None;
    }
    if fmt.is_some_and(CellFormat::is_text) {
        return Some(format!("'{raw}"));
    }
    Some(raw.to_string())
}

#[derive(Debug, Clone)]
enum Delta {
    Retain(u32),
    Insert(u32),
    Delete(u32),
}

#[derive(Debug, Clone)]
enum EvKind {
    Array(Vec<Delta>),
    Map(Vec<String>),
    Other,
}

#[derive(Debug, Clone)]
struct Ev {
    path: Vec<String>,
    kind: EvKind,
}

#[derive(Debug, Clone)]
struct Batch {
    hf_done: bool,
    events: Vec<Ev>,
}

struct Live {
    key: String,
    queue: Arc<Mutex<Vec<Batch>>>,
    hf_done: Arc<AtomicBool>,
}

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

/// Parts of a tab map the engine tracks.
const PARTS: [&str; 7] = ["rows", "cols", "cells", "fmts", "colW", "rowH", "merges"];

pub struct SheetEngine {
    doc: Doc,
    root: MapRef,
    /// Tab snapshots in display order.
    pub tabs: Vec<TabSnap>,
    /// Bumped on every change (cheap memo key for UIs).
    pub version: u64,
    wb: Workbook,
    hf_sheet: HashMap<String, SheetId>,
    live: Option<Live>,
    rng: Box<dyn RandomSource + Send>,
}

impl std::fmt::Debug for SheetEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SheetEngine").field("tabs", &self.tabs.len()).field("version", &self.version).finish()
    }
}

/// Options for [`SheetEngine::new`].
pub struct EngineOpts {
    /// Observe the document and follow its changes (default true).
    pub live: bool,
    /// Clock / timezone / randomness for TODAY, NOW and RAND.
    pub host: Box<dyn Host>,
    /// Randomness for new row/column ids.
    pub rng: Box<dyn RandomSource + Send>,
}

impl Default for EngineOpts {
    fn default() -> Self {
        EngineOpts { live: true, host: Box::new(FixedHost::default()), rng: Box::new(OsRandom) }
    }
}

/// JS object key order for `buildFromSheets(sheets)`: array-index keys first (ascending), then insertion order.
fn js_key_order(names: &[String]) -> Vec<usize> {
    let index_key = |s: &str| -> Option<u64> {
        if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse::<u64>().ok().filter(|v| *v < 4_294_967_295)
    };
    let mut ints: Vec<(u64, usize)> = names.iter().enumerate().filter_map(|(i, n)| index_key(n).map(|v| (v, i))).collect();
    ints.sort();
    let mut out: Vec<usize> = ints.into_iter().map(|(_, i)| i).collect();
    out.extend((0..names.len()).filter(|i| index_key(&names[*i]).is_none()));
    out
}

impl SheetEngine {
    /// An engine over the `sheet` root of `doc` (no transaction may be open).
    pub fn new(doc: &Doc, opts: EngineOpts) -> SheetEngine {
        let root = sheet_root(doc);
        let mut e = SheetEngine {
            doc: doc.clone(),
            root,
            tabs: Vec::new(),
            version: 0,
            wb: Workbook::new(opts.host),
            hf_sheet: HashMap::new(),
            live: None,
            rng: opts.rng,
        };
        e.rebuild();
        if opts.live {
            let key = format!("oc-sheet-engine-{}", NEXT_KEY.fetch_add(1, Ordering::Relaxed));
            let queue: Arc<Mutex<Vec<Batch>>> = Arc::default();
            let hf_done = Arc::new(AtomicBool::new(false));
            let (q, done) = (queue.clone(), hf_done.clone());
            e.root.observe_deep(key.as_str(), move |txn, events| {
                let mut batch = Batch { hf_done: done.load(Ordering::SeqCst), events: Vec::new() };
                for ev in events.iter() {
                    let path: Vec<String> = ev
                        .path()
                        .into_iter()
                        .map(|s| match s {
                            PathSegment::Key(k) => k.to_string(),
                            PathSegment::Index(i) => i.to_string(),
                        })
                        .collect();
                    let kind = match ev {
                        Event::Array(a) => EvKind::Array(
                            a.delta(txn)
                                .iter()
                                .map(|c| match c {
                                    YChange::Added(v) => Delta::Insert(v.len() as u32),
                                    YChange::Removed(n) => Delta::Delete(*n),
                                    YChange::Retain(n) => Delta::Retain(*n),
                                })
                                .collect(),
                        ),
                        Event::Map(m) => EvKind::Map(m.keys(txn).keys().map(|k| k.to_string()).collect()),
                        _ => EvKind::Other,
                    };
                    batch.events.push(Ev { path, kind });
                }
                q.lock().unwrap_or_else(|p| p.into_inner()).push(batch);
            });
            e.live = Some(Live { key, queue, hf_done });
        }
        e
    }

    /// The document this engine reads.
    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn root(&self) -> &MapRef {
        &self.root
    }

    /// The underlying formula workbook (read-only).
    pub fn workbook(&self) -> &Workbook {
        &self.wb
    }

    pub fn set_rng(&mut self, rng: Box<dyn RandomSource + Send>) {
        self.rng = rng;
    }

    pub fn set_host(&mut self, host: Box<dyn Host>) {
        self.wb.set_host(host);
        self.wb.recompute();
        self.version += 1;
    }

    fn emit(&mut self) {
        self.version += 1;
    }

    /// Rebuild every sheet from scratch (tab added/removed/renamed, or first load).
    pub fn rebuild(&mut self) {
        let host = std::mem::replace(&mut self.wb, Workbook::new(Box::new(FixedHost::default())));
        let host = host.into_host();
        self.tabs = {
            let txn = self.doc.transact();
            snap_tabs(&txn, &self.root)
        };
        let mut names: HashSet<String> = HashSet::new();
        let mut name_of: Vec<String> = Vec::new();
        for t in &self.tabs {
            let mut n = t.name.clone();
            let mut i = 2;
            while names.contains(&n.to_lowercase()) {
                n = format!("{} ({i})", t.name);
                i += 1;
            }
            names.insert(n.to_lowercase());
            name_of.push(n);
        }
        let mut wb = Workbook::new(host);
        self.hf_sheet.clear();
        let order = js_key_order(&name_of);
        let mut sheet_of_tab: Vec<SheetId> = vec![0; self.tabs.len()];
        for i in &order {
            sheet_of_tab[*i] = wb.add_sheet(&name_of[*i]);
        }
        for i in &order {
            let t = &self.tabs[*i];
            let mut cells: Vec<(usize, usize, Option<String>)> = t
                .cells
                .iter()
                .filter_map(|(k, raw)| t.pos_of(k).map(|p| (p.r, p.c, hf_input(Some(raw), t.fmts.get(k)))))
                .collect();
            cells.sort_by_key(|x| (x.0, x.1));
            for (r, c, v) in cells {
                wb.set_contents(sheet_of_tab[*i], r as u32, c as u32, v.as_deref());
            }
        }
        wb.recompute();
        for (i, t) in self.tabs.iter().enumerate() {
            self.hf_sheet.insert(t.id.clone(), sheet_of_tab[i]);
        }
        self.wb = wb;
        self.emit();
    }

    pub fn tab(&self, id: &str) -> Option<&TabSnap> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_index(&self, id: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Apply queued document changes (live engines). Returns true when anything changed.
    pub fn sync(&mut self) -> bool {
        let Some(live) = &self.live else { return false };
        let batches: Vec<Batch> = std::mem::take(&mut *live.queue.lock().unwrap_or_else(|p| p.into_inner()));
        let any = !batches.is_empty();
        for b in batches {
            self.on_events(b);
        }
        any
    }

    fn resnap(&mut self, id: &str) {
        let Some(i) = self.tab_index(id) else { return };
        let map = self.tabs[i].map.clone();
        let txn = self.doc.transact();
        self.tabs[i] = snap_tab(&txn, id, &map);
    }

    /// Refresh changed keys of one map part of a tab snapshot from the live document.
    fn refresh_keys(&mut self, id: &str, part: &str, keys: &[String]) {
        let Some(i) = self.tab_index(id) else { return };
        let t = &mut self.tabs[i];
        let live = match part {
            "cells" => t.cells_map.clone(),
            "fmts" => t.fmts_map.clone(),
            "colW" => t.col_w_map.clone(),
            "rowH" => t.row_h_map.clone(),
            "merges" => t.merges_map.clone(),
            _ => None,
        };
        let Some(m) = live else { return };
        let txn = self.doc.transact();
        for k in keys {
            let v = m.get(&txn, k).map(|o| out_to_json(&txn, &o));
            match part {
                "cells" => match v {
                    Some(Value::String(s)) => {
                        t.cells.insert(k.clone(), s);
                    }
                    Some(Value::Null) | None => {
                        t.cells.remove(k);
                    }
                    Some(o) => {
                        t.cells.insert(k.clone(), oc_schema::js::value_to_js_string(&o));
                    }
                },
                "fmts" => match v {
                    Some(o) if o.is_object() => {
                        t.fmts.insert(k.clone(), CellFormat::from_json(&o));
                    }
                    _ => {
                        t.fmts.remove(k);
                    }
                },
                "colW" | "rowH" => {
                    let target = if part == "colW" { &mut t.col_w } else { &mut t.row_h };
                    match v {
                        Some(o) if o.is_number() => {
                            target.insert(k.clone(), oc_schema::js::value_to_number(&o));
                        }
                        _ => {
                            target.remove(k);
                        }
                    }
                }
                _ => {
                    let m = v.and_then(|o| {
                        Some(Merge {
                            rows: oc_schema::js::value_to_number(o.get("rows")?),
                            cols: oc_schema::js::value_to_number(o.get("cols")?),
                        })
                    });
                    match m {
                        Some(m) => {
                            t.merges.insert(k.clone(), m);
                        }
                        None => {
                            t.merges.remove(k);
                        }
                    }
                }
            }
        }
    }

    fn on_events(&mut self, b: Batch) {
        // 1. Anything touching the tab set or a tab name → full rebuild.
        for e in &b.events {
            let p: Vec<&str> = e.path.iter().map(String::as_str).collect();
            match p.as_slice() {
                [] | ["tabs"] => return self.rebuild(),
                ["order"] => {
                    // Reordering tabs changes nothing the workbook sees.
                    let txn = self.doc.transact();
                    let fresh = snap_tabs(&txn, &self.root);
                    drop(txn);
                    self.tabs = fresh;
                    continue;
                }
                ["tabs", id] => {
                    if self.tab_index(id).is_none() {
                        return self.rebuild();
                    }
                    if let EvKind::Map(keys) = &e.kind
                        && keys.iter().any(|k| k != "frozenRows" && k != "frozenCols")
                    {
                        return self.rebuild();
                    }
                }
                ["tabs", id, part] => {
                    if self.tab_index(id).is_none() || !PARTS.contains(part) {
                        return self.rebuild();
                    }
                }
                _ => return self.rebuild(),
            }
        }
        let mut changed: Vec<String> = Vec::new();
        // 2. Row/column structure: replay the array deltas (unless already applied locally).
        for e in &b.events {
            let [tabs, id, part] = e.path.as_slice() else { continue };
            if tabs != "tabs" || (part != "rows" && part != "cols") {
                continue;
            }
            if !changed.contains(id) {
                changed.push(id.clone());
            }
            if b.hf_done {
                continue;
            }
            let Some(&sheet) = self.hf_sheet.get(id) else { continue };
            let EvKind::Array(delta) = &e.kind else { continue };
            let mut at = 0u32;
            for d in delta {
                match d {
                    Delta::Retain(n) => at += n,
                    Delta::Insert(n) => {
                        let _ = if part == "rows" { self.wb.add_rows(sheet, at, *n) } else { self.wb.add_cols(sheet, at, *n) };
                        at += n;
                    }
                    Delta::Delete(n) => {
                        let _ = if part == "rows" { self.wb.remove_rows(sheet, at, *n) } else { self.wb.remove_cols(sheet, at, *n) };
                    }
                }
            }
        }
        for id in &changed {
            self.resnap(id);
        }
        // 3. Cell contents (and formats, which can switch a cell to text).
        for e in &b.events {
            let p: Vec<&str> = e.path.iter().map(String::as_str).collect();
            let (id, part) = match p.as_slice() {
                ["tabs", id, part] => (id.to_string(), part.to_string()),
                ["tabs", id] => (id.to_string(), "tab".to_string()),
                _ => continue,
            };
            let EvKind::Map(keys) = &e.kind else { continue };
            if part != "cells" && part != "fmts" {
                if part == "tab" {
                    self.resnap(&id);
                } else {
                    self.refresh_keys(&id, &part, keys);
                }
                continue;
            }
            self.refresh_keys(&id, &part, keys);
            let (Some(i), Some(&sheet)) = (self.tab_index(&id), self.hf_sheet.get(&id)) else { continue };
            for k in keys {
                let t = &self.tabs[i];
                let Some(pos) = t.pos_of(k) else { continue };
                let input = hf_input(t.cells.get(k).map(String::as_str), t.fmts.get(k));
                self.wb.set_contents(sheet, pos.r as u32, pos.c as u32, input.as_deref());
            }
        }
        self.wb.recompute();
        self.emit();
    }

    // ------------------------------------------------------------------ reads

    pub fn sheet_id(&self, tab_id: &str) -> Option<SheetId> {
        self.hf_sheet.get(tab_id).copied()
    }

    pub fn value(&self, tab_id: &str, r: usize, c: usize) -> CellValue {
        match self.sheet_id(tab_id) {
            Some(s) if r < u32::MAX as usize && c < u32::MAX as usize => self.wb.value(s, r as u32, c as u32),
            _ => CellValue::Null,
        }
    }

    pub fn kind(&self, tab_id: &str, r: usize, c: usize) -> ValueKind {
        match self.sheet_id(tab_id) {
            Some(s) if r < u32::MAX as usize && c < u32::MAX as usize => self.wb.kind(s, r as u32, c as u32),
            _ => ValueKind::Other,
        }
    }

    /// Computed values of a range (row-major).
    pub fn range_values(&self, tab_id: &str, r: &Range) -> Vec<Vec<CellValue>> {
        (r.r0..=r.r1).map(|i| (r.c0..=r.c1).map(|j| self.value(tab_id, i, j)).collect()).collect()
    }

    // ------------------------------------------------------------------ edits

    /// Run local edits in one transaction (origin `sheet-local`), then apply them.
    pub fn transact<R>(&mut self, f: impl FnOnce(&mut TransactionMut, &[TabSnap], &mut dyn RandomSource) -> R) -> R {
        let out = {
            let mut txn = self.doc.transact_mut_with(Origin::from(SHEET_LOCAL));
            f(&mut txn, &self.tabs, &mut *self.rng)
        };
        self.sync();
        out
    }

    /// Every formula cell's workbook text, keyed (tab id, cell key).
    fn formulas(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for t in &self.tabs {
            let Some(&sheet) = self.hf_sheet.get(&t.id) else { continue };
            for (k, raw) in &t.cells {
                if !raw.starts_with('=') || t.fmts.get(k).is_some_and(CellFormat::is_text) {
                    continue;
                }
                let Some(p) = t.pos_of(k) else { continue };
                if let Some(f) = self.wb.formula(sheet, p.r as u32, p.c as u32) {
                    out.push((t.id.clone(), k.clone(), f));
                }
            }
        }
        out
    }

    /// Run a structural change: `wb_op` applies it to the workbook, `y_op` to the Yjs model. Formulas the
    /// workbook rewrote (shifted references, `#REF!`, renamed sheets) are written back in the same transaction.
    fn structural(
        &mut self,
        wb_op: impl FnOnce(&mut Workbook),
        y_op: impl FnOnce(&mut TransactionMut, &MapRef, &mut dyn RandomSource),
    ) {
        let before = self.formulas();
        if let Some(l) = &self.live {
            l.hf_done.store(true, Ordering::SeqCst);
        }
        {
            let mut txn = self.doc.transact_mut_with(Origin::from(SHEET_LOCAL));
            wb_op(&mut self.wb);
            y_op(&mut txn, &self.root, &mut *self.rng);
            let tabs = snap_tabs(&txn, &self.root);
            for (tab_id, key, f) in before {
                let Some(t) = tabs.iter().find(|x| x.id == tab_id) else { continue };
                let (Some(p), Some(&sheet)) = (t.pos_of(&key), self.hf_sheet.get(&tab_id)) else { continue };
                if let Some(now) = self.wb.formula(sheet, p.r as u32, p.c as u32)
                    && now != f
                    && let Some(cells) = &t.cells_map
                {
                    cells.insert(&mut txn, key, now);
                }
            }
        }
        if let Some(l) = &self.live {
            l.hf_done.store(false, Ordering::SeqCst);
        }
        if self.live.is_some() {
            self.sync();
        } else {
            // A non-live engine still reflects its own edits.
            self.rebuild();
        }
    }

    /// Insert `n` rows/columns before `at`, shifting references.
    pub fn insert(&mut self, tab_id: &str, axis: Axis, at: usize, n: usize) {
        let (Some(t), Some(&sheet)) = (self.tab(tab_id).cloned(), self.hf_sheet.get(tab_id)) else { return };
        self.structural(
            |wb| {
                let _ = if axis == Axis::Rows { wb.add_rows(sheet, at as u32, n as u32) } else { wb.add_cols(sheet, at as u32, n as u32) };
            },
            |txn, _, rng| insert_tracks(txn, &t, axis, at, n, rng),
        );
    }

    /// Delete `n` rows/columns from `at`, shifting references (deleted ones become `#REF!`).
    pub fn remove(&mut self, tab_id: &str, axis: Axis, at: usize, n: usize) {
        let (Some(t), Some(&sheet)) = (self.tab(tab_id).cloned(), self.hf_sheet.get(tab_id)) else { return };
        self.structural(
            |wb| {
                let _ = if axis == Axis::Rows { wb.remove_rows(sheet, at as u32, n as u32) } else { wb.remove_cols(sheet, at as u32, n as u32) };
            },
            |txn, _, _| delete_tracks(txn, &t, axis, at, n),
        );
    }

    /// Rename a tab (unique, cleaned name); formulas referencing it follow. Returns the name used.
    pub fn rename_tab(&mut self, tab_id: &str, name: &str) -> String {
        let Some(t) = self.tab(tab_id).cloned() else { return name.to_string() };
        let next = {
            let txn = self.doc.transact();
            unique_tab_name(&txn, &self.root, name, Some(tab_id))
        };
        if next == t.name {
            return next;
        }
        let sheet = self.hf_sheet.get(tab_id).copied();
        let taken = sheet.is_some_and(|s| self.wb.sheet_id(&next).is_some_and(|o| o != s));
        let n2 = next.clone();
        self.structural(
            |wb| {
                if let Some(s) = sheet
                    && !taken
                {
                    let _ = wb.rename_sheet(s, &n2);
                }
            },
            |txn, _, _| model::set_tab_name(txn, &t, &n2),
        );
        next
    }

    /// Delete a tab (never the last one); formulas referencing it become `#REF!`.
    pub fn delete_tab(&mut self, tab_id: &str) {
        let Some(&sheet) = self.hf_sheet.get(tab_id) else { return };
        if self.tabs.len() <= 1 {
            return;
        }
        let id = tab_id.to_string();
        self.structural(
            |wb| {
                let _ = wb.remove_sheet(sheet);
            },
            |txn, root, _| model::delete_tab(txn, root, &id),
        );
    }
}

impl Drop for SheetEngine {
    fn drop(&mut self) {
        if let Some(l) = &self.live {
            self.root.unobserve_deep(l.key.as_str());
        }
    }
}
