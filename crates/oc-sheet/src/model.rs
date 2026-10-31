// SPDX-License-Identifier: AGPL-3.0-only
//! Sheet data model in the design's Y.Doc (A15) — the same layout as `@oc/sheet`'s yjs model:
//!
//! ```text
//! sheet: Y.Map
//!   tabs:  Y.Map<tabId, Y.Map>     one per sheet tab
//!   order: Y.Array<tabId>          tab order
//! tab: Y.Map
//!   name: string
//!   rows / cols: Y.Array<string>   stable row / column ids, in display order
//!   cells: Y.Map<"rowId:colId", string>        raw input ("12", "hello", "=SUM(A1:A3)")
//!   fmts:  Y.Map<"rowId:colId", CellFormat>    per-cell format (atomic per cell, last writer wins)
//!   colW / rowH: Y.Map<id, number>             custom track sizes in px
//!   merges: Y.Map<"rowId:colId", {rows, cols}> merged range anchored at that cell
//!   frozenRows / frozenCols: number
//! ```
//!
//! Reads go through [`TabSnap`], a plain snapshot of one tab (rebuild it after writing). Writes take an open
//! transaction, like the TS helpers that must be called inside `ydoc.transact`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use oc_crdt::rng::{OsRandom, RandomSource};
use oc_crdt::value::{json_to_any, out_to_json};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value};
use yrs::types::ToJson;
use yrs::{Any, Array, ArrayPrelim, ArrayRef, Doc, Map, MapPrelim, MapRef, Origin, Out, ReadTxn, Transact, TransactionMut};

use crate::a1::{CellAddr, Range};

pub const SHEET_ROOT: &str = "sheet";
/// Transaction origin for local edits (tracked by the undo manager in the browser).
pub const SHEET_LOCAL: &str = "sheet-local";

pub const DEFAULT_ROWS: usize = 1000;
pub const DEFAULT_COLS: usize = 26;
pub const DEFAULT_COL_W: f64 = 100.0;
pub const DEFAULT_ROW_H: f64 = 24.0;
pub const MAX_ROWS: usize = 100_000;
/// A..ZZ
pub const MAX_COLS: usize = 702;

/// Number format of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NumberFormat {
    Auto,
    Number,
    Percent,
    Currency,
    Date,
    Datetime,
    Text,
}

/// Per-cell format. Unknown keys are kept (`extra`) so patches never drop data written by newer clients.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CellFormat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nf: Option<NumberFormat>,
    /// Decimal places for number/percent/currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dp: Option<f64>,
    /// Currency symbol (default "$").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cur: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub u: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s: Option<bool>,
    /// Text colour, "#rrggbb".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Fill colour, "#rrggbb".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// "left" | "center" | "right"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    /// "top" | "middle" | "bottom"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valign: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<bool>,
    /// Font size in px (default 13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(flatten)]
    pub extra: JsonMap<String, Value>,
}

impl CellFormat {
    pub fn is_text(&self) -> bool {
        self.nf == Some(NumberFormat::Text)
    }
    pub fn from_json(v: &Value) -> CellFormat {
        // A malformed value (wrong field type written by some client) degrades field by field.
        serde_json::from_value(v.clone()).unwrap_or_else(|_| {
            let mut f = CellFormat::default();
            if let Value::Object(m) = v {
                for (k, x) in m {
                    let one = Value::Object(JsonMap::from_iter([(k.clone(), x.clone())]));
                    if let Ok(p) = serde_json::from_value::<CellFormat>(one) {
                        f = merge_formats(&f, &p);
                    }
                }
            }
            f
        })
    }
    pub fn to_json(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        oc_schema::js::normalize_numbers(&mut v);
        v
    }
}

fn merge_formats(a: &CellFormat, b: &CellFormat) -> CellFormat {
    let mut m = match a.to_json() {
        Value::Object(m) => m,
        _ => JsonMap::new(),
    };
    if let Value::Object(bm) = b.to_json() {
        m.extend(bm);
    }
    serde_json::from_value(Value::Object(m)).unwrap_or_default()
}

/// A merged range anchored at a cell.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Merge {
    pub rows: f64,
    pub cols: f64,
}

pub fn ckey(row_id: &str, col_id: &str) -> String {
    format!("{row_id}:{col_id}")
}

/// Split "rowId:colId" at the first ':'.
pub fn split_key(k: &str) -> (&str, &str) {
    match k.find(':') {
        Some(i) => (&k[..i], &k[i + 1..]),
        // `k.slice(0, -1)` / `k.slice(0)` for a key without ':' (indexOf → -1).
        None => {
            let cut = k.char_indices().last().map_or(0, |(i, _)| i);
            (&k[..cut], k)
        }
    }
}

const ALPHA: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// 8 random base-62 characters (~47 bits).
pub fn new_track_id(rng: &mut dyn RandomSource) -> String {
    let mut b = [0u8; 8];
    rng.fill(&mut b);
    b.iter().map(|x| ALPHA[(*x % 62) as usize] as char).collect()
}

fn ids(rng: &mut dyn RandomSource, n: usize) -> Vec<String> {
    (0..n).map(|_| new_track_id(rng)).collect()
}

/// The root map `sheet` (must not be called while a transaction is open).
pub fn sheet_root(ydoc: &Doc) -> MapRef {
    ydoc.get_or_insert_map(SHEET_ROOT)
}

fn sub_map<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<MapRef> {
    match m.get(txn, key) {
        Some(Out::YMap(x)) => Some(x),
        _ => None,
    }
}

fn sub_array<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<ArrayRef> {
    match m.get(txn, key) {
        Some(Out::YArray(x)) => Some(x),
        _ => None,
    }
}

pub fn tabs_of<T: ReadTxn>(txn: &T, root: &MapRef) -> Option<MapRef> {
    sub_map(txn, root, "tabs")
}

pub fn order_of<T: ReadTxn>(txn: &T, root: &MapRef) -> Option<ArrayRef> {
    sub_array(txn, root, "order")
}

fn strings<T: ReadTxn>(txn: &T, a: &ArrayRef) -> Vec<String> {
    a.iter(txn)
        .map(|o| match o {
            Out::Any(Any::String(s)) => s.to_string(),
            other => oc_schema::js::value_to_js_string(&out_to_json(txn, &other)),
        })
        .collect()
}

/// Insert a new tab map (`createTab`) under `tabs[id]`.
pub fn create_tab(txn: &mut TransactionMut, tabs: &MapRef, id: &str, name: &str, rows: usize, cols: usize, rng: &mut dyn RandomSource) -> MapRef {
    let tab = tabs.insert(txn, id.to_string(), MapPrelim::default());
    tab.insert(txn, "name", name.to_string());
    let r = ids(rng, rows);
    let c = ids(rng, cols);
    tab.insert(txn, "rows", ArrayPrelim::from(r));
    tab.insert(txn, "cols", ArrayPrelim::from(c));
    for k in ["cells", "fmts", "colW", "rowH", "merges"] {
        tab.insert(txn, k, MapPrelim::default());
    }
    tab.insert(txn, "frozenRows", Any::Number(yrs::any::Number::Int(0)));
    tab.insert(txn, "frozenCols", Any::Number(yrs::any::Number::Int(0)));
    tab
}

/// True when the doc already has a sheet (at least one tab).
pub fn has_sheet(ydoc: &Doc) -> bool {
    let root = sheet_root(ydoc);
    let txn = ydoc.transact();
    tabs_of(&txn, &root).is_some_and(|t| t.len(&txn) > 0)
}

/// Options for [`init_sheet`].
#[derive(Debug, Clone, Default)]
pub struct InitOpts {
    pub name: Option<String>,
    pub rows: Option<usize>,
    pub cols: Option<usize>,
    pub origin: Option<Origin>,
}

/// Create the sheet structure with one empty tab, unless it exists. Returns the first tab id.
pub fn init_sheet(ydoc: &Doc, opts: InitOpts, rng: &mut dyn RandomSource) -> String {
    let root = sheet_root(ydoc);
    let mut txn = match opts.origin.clone() {
        Some(o) => ydoc.transact_mut_with(o),
        None => ydoc.transact_mut(),
    };
    init_sheet_txn(&mut txn, &root, &opts, rng)
}

/// [`init_sheet`] inside an open transaction.
pub fn init_sheet_txn(txn: &mut TransactionMut, root: &MapRef, opts: &InitOpts, rng: &mut dyn RandomSource) -> String {
    if tabs_of(txn, root).is_none() {
        root.insert(txn, "tabs", MapPrelim::default());
    }
    if order_of(txn, root).is_none() {
        root.insert(txn, "order", ArrayPrelim::default());
    }
    let tabs = tabs_of(txn, root).expect("tabs");
    let order = order_of(txn, root).expect("order");
    if tabs.len(txn) == 0 {
        let id = new_track_id(rng);
        create_tab(
            txn,
            &tabs,
            &id,
            opts.name.as_deref().unwrap_or("Sheet1"),
            opts.rows.unwrap_or(DEFAULT_ROWS),
            opts.cols.unwrap_or(DEFAULT_COLS),
            rng,
        );
        let n = order.len(txn);
        order.insert(txn, n, id);
    }
    let first = order.get(txn, 0).map(|o| oc_schema::js::value_to_js_string(&out_to_json(txn, &o)));
    first.unwrap_or_else(|| {
        let mut keys: Vec<String> = tabs.keys(txn).map(str::to_string).collect();
        keys.sort();
        keys.into_iter().next().unwrap_or_default()
    })
}

/// [`init_sheet`] with the OS random source.
pub fn init_sheet_default(ydoc: &Doc, opts: InitOpts) -> String {
    init_sheet(ydoc, opts, &mut OsRandom)
}

/// Tab ids in display order (order array first, then any tab missing from it — e.g. after a race).
pub fn tab_ids<T: ReadTxn>(txn: &T, root: &MapRef) -> Vec<String> {
    let Some(tabs) = tabs_of(txn, root) else { return vec![] };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    if let Some(order) = order_of(txn, root) {
        for id in strings(txn, &order) {
            if tabs.get(txn, &id).is_some() && !seen.contains(&id) {
                seen.insert(id.clone());
                out.push(id);
            }
        }
    }
    let mut keys: Vec<String> = tabs.keys(txn).map(str::to_string).collect();
    keys.sort_by(|a, b| oc_schema::js::str_cmp(a, b));
    for id in keys {
        if !seen.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Plain, read-only view of one tab with position lookups. Rebuild it when the tab changes.
#[derive(Debug, Clone)]
pub struct TabSnap {
    pub id: String,
    pub name: String,
    pub map: MapRef,
    pub rows: Vec<String>,
    pub cols: Vec<String>,
    pub row_ix: HashMap<String, usize>,
    pub col_ix: HashMap<String, usize>,
    pub cells: HashMap<String, String>,
    pub fmts: HashMap<String, CellFormat>,
    pub col_w: HashMap<String, f64>,
    pub row_h: HashMap<String, f64>,
    pub merges: HashMap<String, Merge>,
    pub frozen_rows: f64,
    pub frozen_cols: f64,
    /// Handles of the shared maps (for writes).
    pub cells_map: Option<MapRef>,
    pub fmts_map: Option<MapRef>,
    pub col_w_map: Option<MapRef>,
    pub row_h_map: Option<MapRef>,
    pub merges_map: Option<MapRef>,
}

fn js_number(v: &Value) -> f64 {
    oc_schema::js::value_to_number(v)
}

fn read_map<T: ReadTxn, V>(txn: &T, m: Option<&MapRef>, f: impl Fn(Value) -> Option<V>) -> HashMap<String, V> {
    let mut out = HashMap::new();
    if let Some(m) = m {
        for (k, v) in m.iter(txn) {
            if let Some(x) = f(out_to_json(txn, &v)) {
                out.insert(k.to_string(), x);
            }
        }
    }
    out
}

pub fn snap_tab<T: ReadTxn>(txn: &T, id: &str, map: &MapRef) -> TabSnap {
    let rows = sub_array(txn, map, "rows").map(|a| strings(txn, &a)).unwrap_or_default();
    let cols = sub_array(txn, map, "cols").map(|a| strings(txn, &a)).unwrap_or_default();
    let cells_map = sub_map(txn, map, "cells");
    let fmts_map = sub_map(txn, map, "fmts");
    let col_w_map = sub_map(txn, map, "colW");
    let row_h_map = sub_map(txn, map, "rowH");
    let merges_map = sub_map(txn, map, "merges");
    let name = match map.get(txn, "name") {
        None => id.to_string(),
        Some(o) => match out_to_json(txn, &o) {
            Value::Null => id.to_string(),
            v => oc_schema::js::value_to_js_string(&v),
        },
    };
    let num = |k: &str| map.get(txn, k).map_or(0.0, |o| js_number(&out_to_json(txn, &o)));
    let mut row_ix = HashMap::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        row_ix.insert(r.clone(), i);
    }
    let mut col_ix = HashMap::with_capacity(cols.len());
    for (i, c) in cols.iter().enumerate() {
        col_ix.insert(c.clone(), i);
    }
    TabSnap {
        id: id.to_string(),
        name,
        map: map.clone(),
        cells: read_map(txn, cells_map.as_ref(), |v| match v {
            Value::String(s) => Some(s),
            Value::Null => None,
            other => Some(oc_schema::js::value_to_js_string(&other)),
        }),
        fmts: read_map(txn, fmts_map.as_ref(), |v| v.is_object().then(|| CellFormat::from_json(&v))),
        col_w: read_map(txn, col_w_map.as_ref(), |v| v.is_number().then(|| js_number(&v))),
        row_h: read_map(txn, row_h_map.as_ref(), |v| v.is_number().then(|| js_number(&v))),
        merges: read_map(txn, merges_map.as_ref(), |v| {
            let r = v.get("rows").map(js_number)?;
            let c = v.get("cols").map(js_number)?;
            Some(Merge { rows: r, cols: c })
        }),
        frozen_rows: num("frozenRows"),
        frozen_cols: num("frozenCols"),
        rows,
        cols,
        row_ix,
        col_ix,
        cells_map,
        fmts_map,
        col_w_map,
        row_h_map,
        merges_map,
    }
}

pub fn snap_tabs<T: ReadTxn>(txn: &T, root: &MapRef) -> Vec<TabSnap> {
    let Some(tabs) = tabs_of(txn, root) else { return vec![] };
    tab_ids(txn, root)
        .into_iter()
        .filter_map(|id| sub_map(txn, &tabs, &id).map(|m| snap_tab(txn, &id, &m)))
        .collect()
}

/// Snapshot every tab of a document.
pub fn snap_doc(ydoc: &Doc) -> Vec<TabSnap> {
    let root = sheet_root(ydoc);
    let txn = ydoc.transact();
    snap_tabs(&txn, &root)
}

/// Find a tab by id, then by (case-insensitive) name.
pub fn find_tab<'a>(tabs: &'a [TabSnap], r: &str) -> Option<&'a TabSnap> {
    tabs.iter().find(|t| t.id == r).or_else(|| {
        let l = r.to_lowercase();
        tabs.iter().find(|t| t.name.to_lowercase() == l)
    })
}

impl TabSnap {
    /// Position of a cell key, or `None` when its row/column was deleted.
    pub fn pos_of(&self, key: &str) -> Option<CellAddr> {
        let (rid, cid) = split_key(key);
        Some(CellAddr { r: *self.row_ix.get(rid)?, c: *self.col_ix.get(cid)? })
    }

    pub fn key_at(&self, r: usize, c: usize) -> Option<String> {
        let rid = self.rows.get(r)?;
        let cid = self.cols.get(c)?;
        if rid.is_empty() || cid.is_empty() {
            return None;
        }
        Some(ckey(rid, cid))
    }

    pub fn raw_at(&self, r: usize, c: usize) -> &str {
        self.key_at(r, c).and_then(|k| self.cells.get(&k)).map_or("", String::as_str)
    }

    pub fn fmt_at(&self, r: usize, c: usize) -> Option<&CellFormat> {
        self.key_at(r, c).and_then(|k| self.fmts.get(&k))
    }

    pub fn col_width(&self, c: usize) -> f64 {
        self.cols.get(c).and_then(|id| self.col_w.get(id)).copied().unwrap_or(DEFAULT_COL_W)
    }

    pub fn row_height(&self, r: usize) -> f64 {
        self.rows.get(r).and_then(|id| self.row_h.get(id)).copied().unwrap_or(DEFAULT_ROW_H)
    }

    /// Smallest range holding every non-empty cell (`None` when the tab is empty).
    pub fn used_range(&self) -> Option<Range> {
        let mut r1: i64 = -1;
        let mut c1: i64 = -1;
        for (k, v) in &self.cells {
            if v.is_empty() {
                continue;
            }
            let Some(p) = self.pos_of(k) else { continue };
            r1 = r1.max(p.r as i64);
            c1 = c1.max(p.c as i64);
        }
        for (k, m) in &self.merges {
            let Some(p) = self.pos_of(k) else { continue };
            r1 = r1.max((p.r as f64 + m.rows - 1.0) as i64);
            c1 = c1.max((p.c as f64 + m.cols - 1.0) as i64);
        }
        if r1 < 0 {
            return None;
        }
        Some(Range { r0: 0, c0: 0, r1: r1 as usize, c1: c1.max(0) as usize })
    }

    /// Merged ranges in positions (anchors whose tracks were deleted are skipped; spans clipped to the grid).
    pub fn merge_ranges(&self) -> Vec<Range> {
        let mut keys: Vec<&String> = self.merges.keys().collect();
        keys.sort();
        let mut out = Vec::new();
        for k in keys {
            let m = self.merges[k];
            let Some(p) = self.pos_of(k) else { continue };
            if m.rows <= 1.0 && m.cols <= 1.0 {
                continue;
            }
            let r1 = (p.r as f64 + m.rows - 1.0).min(self.rows.len() as f64 - 1.0).max(0.0) as usize;
            let c1 = (p.c as f64 + m.cols - 1.0).min(self.cols.len() as f64 - 1.0).max(0.0) as usize;
            out.push(Range { r0: p.r, c0: p.c, r1, c1 });
        }
        out
    }
}

// ------------------------------------------------------------------ writes (inside a transaction)

fn live_map(txn: &TransactionMut, t: &TabSnap, part: &str) -> Option<MapRef> {
    sub_map(txn, &t.map, part)
}

fn live_array(txn: &TransactionMut, t: &TabSnap, part: &str) -> Option<ArrayRef> {
    sub_array(txn, &t.map, part)
}

pub fn set_raw(txn: &mut TransactionMut, t: &TabSnap, r: usize, c: usize, raw: &str) {
    let Some(k) = t.key_at(r, c) else { return };
    let Some(cells) = live_map(txn, t, "cells") else { return };
    if raw.is_empty() {
        cells.remove(txn, &k);
    } else {
        let same = matches!(cells.get(txn, &k), Some(Out::Any(Any::String(s))) if &*s == raw);
        if !same {
            cells.insert(txn, k, raw.to_string());
        }
    }
}

/// Merge `patch` into the cell's format (`None` clears it). Keys set to `null` are removed, as are
/// `false` / `""` values (TS: `undefined`, `false` and `''` are dropped).
pub fn patch_format(txn: &mut TransactionMut, t: &TabSnap, r: usize, c: usize, patch: Option<&JsonMap<String, Value>>) {
    let Some(k) = t.key_at(r, c) else { return };
    let Some(fmts) = live_map(txn, t, "fmts") else { return };
    let Some(patch) = patch else {
        fmts.remove(txn, &k);
        return;
    };
    let mut next = match fmts.get(txn, &k).map(|o| out_to_json(txn, &o)) {
        Some(Value::Object(m)) => m,
        _ => JsonMap::new(),
    };
    for (key, v) in patch {
        next.insert(key.clone(), v.clone());
    }
    next.retain(|_, v| !matches!(v, Value::Null | Value::Bool(false)) && v != &Value::String(String::new()));
    if next.is_empty() {
        fmts.remove(txn, &k);
    } else {
        fmts.insert(txn, k, json_to_any(&Value::Object(next)));
    }
}

/// Typed convenience over [`patch_format`].
pub fn patch_format_typed(txn: &mut TransactionMut, t: &TabSnap, r: usize, c: usize, patch: &CellFormat) {
    if let Value::Object(m) = patch.to_json() {
        patch_format(txn, t, r, c, Some(&m));
    }
}

/// "rows" or "cols".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    Rows,
    Cols,
}

impl Axis {
    pub fn key(self) -> &'static str {
        match self {
            Axis::Rows => "rows",
            Axis::Cols => "cols",
        }
    }
}

/// Insert `n` new tracks before index `at`.
pub fn insert_tracks(txn: &mut TransactionMut, t: &TabSnap, axis: Axis, at: usize, n: usize, rng: &mut dyn RandomSource) {
    let Some(arr) = live_array(txn, t, axis.key()) else { return };
    let len = arr.len(txn) as usize;
    let max = if axis == Axis::Rows { MAX_ROWS } else { MAX_COLS };
    let k = n.min(max.saturating_sub(len));
    if k > 0 {
        let new = ids(rng, k);
        arr.insert_range(txn, at.min(len) as u32, new.into_iter().map(|s| Any::String(Arc::from(s))));
    }
}

/// Delete tracks [at, at+n) and every cell, format, size and merge anchored on them.
pub fn delete_tracks(txn: &mut TransactionMut, t: &TabSnap, axis: Axis, at: usize, n: usize) {
    let Some(arr) = live_array(txn, t, axis.key()) else { return };
    let len = arr.len(txn) as i64;
    let k = (n as i64).min(len - at as i64).min(len - 1).max(0);
    if k <= 0 {
        return;
    }
    let k = k as usize;
    let snap_ids = if axis == Axis::Rows { &t.rows } else { &t.cols };
    let gone: HashSet<String> = snap_ids.iter().skip(at).take(k).cloned().collect();
    arr.remove_range(txn, at as u32, k as u32);
    let side = usize::from(axis == Axis::Cols);
    for part in ["cells", "fmts", "merges"] {
        let Some(m) = live_map(txn, t, part) else { continue };
        let keys: Vec<String> = m.keys(txn).map(str::to_string).collect();
        for key in keys {
            let (a, b) = split_key(&key);
            if gone.contains(if side == 0 { a } else { b }) {
                m.remove(txn, &key);
            }
        }
    }
    if let Some(sizes) = live_map(txn, t, if axis == Axis::Rows { "rowH" } else { "colW" }) {
        for id in &gone {
            sizes.remove(txn, id);
        }
    }
}

/// Merge `r` (dropping overlapping merges), or with `None` unmerge whatever covers `anchor`.
pub fn set_merge(txn: &mut TransactionMut, t: &TabSnap, r: Option<Range>, anchor: Option<CellAddr>) {
    let Some(merges) = live_map(txn, t, "merges") else { return };
    let current = {
        // Merge ranges from the live map (the snapshot may be stale inside the transaction).
        let mut snap = t.clone();
        snap.merges = read_map(txn, Some(&merges), |v| {
            Some(Merge { rows: v.get("rows").map(js_number)?, cols: v.get("cols").map(js_number)? })
        });
        snap.merge_ranges()
    };
    let Some(r) = r else {
        if let Some(a) = anchor {
            for m in current {
                if m.contains(a.r, a.c)
                    && let Some(k) = t.key_at(m.r0, m.c0)
                {
                    merges.remove(txn, &k);
                }
            }
        }
        return;
    };
    for m in current {
        if !(m.r1 < r.r0 || m.r0 > r.r1 || m.c1 < r.c0 || m.c0 > r.c1)
            && let Some(k) = t.key_at(m.r0, m.c0)
        {
            merges.remove(txn, &k);
        }
    }
    if let Some(k) = t.key_at(r.r0, r.c0)
        && (r.r1 > r.r0 || r.c1 > r.c0)
    {
        let v = serde_json::json!({ "rows": r.r1 - r.r0 + 1, "cols": r.c1 - r.c0 + 1 });
        merges.insert(txn, k, json_to_any(&v));
    }
}

/// Set a track size (px), or reset it with `None`.
pub fn set_track_size(txn: &mut TransactionMut, t: &TabSnap, axis: Axis, index: usize, px: Option<f64>) {
    let (part, ids) = if axis == Axis::Rows { ("rowH", &t.rows) } else { ("colW", &t.cols) };
    let (Some(m), Some(id)) = (live_map(txn, t, part), ids.get(index)) else { return };
    match px {
        Some(v) => {
            m.insert(txn, id.clone(), json_to_any(&oc_crdt::value::f64_to_json(v)));
        }
        None => {
            m.remove(txn, id);
        }
    }
}

/// Set the frozen row/column counts.
pub fn set_frozen(txn: &mut TransactionMut, t: &TabSnap, rows: Option<f64>, cols: Option<f64>) {
    if let Some(r) = rows {
        t.map.insert(txn, "frozenRows", json_to_any(&oc_crdt::value::f64_to_json(r)));
    }
    if let Some(c) = cols {
        t.map.insert(txn, "frozenCols", json_to_any(&oc_crdt::value::f64_to_json(c)));
    }
}

/// Tab names must be unique (case-insensitive) — formulas reference tabs by name.
pub fn unique_tab_name<T: ReadTxn>(txn: &T, root: &MapRef, base: &str, except: Option<&str>) -> String {
    let names: HashSet<String> =
        snap_tabs(txn, root).into_iter().filter(|t| Some(t.id.as_str()) != except).map(|t| t.name.to_lowercase()).collect();
    let cleaned: String = base.chars().filter(|c| !matches!(c, '!' | '\'' | '[' | ']' | '*' | '?' | '/' | '\\' | ':')).collect();
    let trimmed = oc_schema::js::trim(&cleaned);
    let clean = if trimmed.is_empty() { "Sheet" } else { trimmed };
    if !names.contains(&clean.to_lowercase()) {
        return clean.to_string();
    }
    let mut i = 2;
    loop {
        let n = format!("{clean} {i}");
        if !names.contains(&n.to_lowercase()) {
            return n;
        }
        i += 1;
    }
}

/// Add a tab (after `after`, else at the end). Returns its id.
pub fn add_tab(txn: &mut TransactionMut, root: &MapRef, name: Option<&str>, after: Option<&str>, rng: &mut dyn RandomSource) -> String {
    let id = new_track_id(rng);
    let tabs = tabs_of(txn, root).expect("sheet initialised");
    let order = order_of(txn, root).expect("sheet initialised");
    let base = name.map_or_else(|| format!("Sheet{}", tabs.len(txn) + 1), str::to_string);
    let unique = unique_tab_name(txn, root, &base, None);
    create_tab(txn, &tabs, &id, &unique, DEFAULT_ROWS, DEFAULT_COLS, rng);
    let arr = strings(txn, &order);
    let at = after.and_then(|a| arr.iter().position(|x| x == a));
    let idx = at.map_or(arr.len(), |i| i + 1);
    order.insert(txn, idx as u32, id.clone());
    id
}

/// Delete a tab (never the last one).
pub fn delete_tab(txn: &mut TransactionMut, root: &MapRef, id: &str) {
    let Some(tabs) = tabs_of(txn, root) else { return };
    if tabs.len(txn) <= 1 {
        return;
    }
    tabs.remove(txn, id);
    if let Some(order) = order_of(txn, root) {
        let arr = strings(txn, &order);
        for i in (0..arr.len()).rev() {
            if arr[i] == id {
                order.remove_range(txn, i as u32, 1);
            }
        }
    }
}

/// Rename a tab map (no formula rewriting — use the engine's `rename_tab` for that).
pub fn set_tab_name(txn: &mut TransactionMut, t: &TabSnap, name: &str) {
    t.map.insert(txn, "name", name.to_string());
}

/// Plain JSON view of a tab by positions (tests, debugging, goldens):
/// `{ name, rows, cols, cells: {A1: raw}, fmts: {A1: fmt}, colW: {index: px}, rowH, merges: [A1:B2], frozenRows, frozenCols }`.
pub fn tab_to_json(t: &TabSnap) -> Value {
    use crate::a1::{a1, format_range};
    let mut cells: Vec<(usize, usize, &String)> = t.cells.iter().filter_map(|(k, v)| t.pos_of(k).map(|p| (p.r, p.c, v))).collect();
    cells.sort();
    let mut fmts: Vec<(usize, usize, &CellFormat)> = t.fmts.iter().filter_map(|(k, v)| t.pos_of(k).map(|p| (p.r, p.c, v))).collect();
    fmts.sort_by_key(|x| (x.0, x.1));
    let mut col_w: Vec<(usize, f64)> = t.col_w.iter().filter_map(|(k, v)| t.col_ix.get(k).map(|i| (*i, *v))).collect();
    col_w.sort_by_key(|x| x.0);
    let mut row_h: Vec<(usize, f64)> = t.row_h.iter().filter_map(|(k, v)| t.row_ix.get(k).map(|i| (*i, *v))).collect();
    row_h.sort_by_key(|x| x.0);
    let mut merges: Vec<String> = t.merge_ranges().iter().map(format_range).collect();
    merges.sort();
    let num = oc_crdt::value::f64_to_json;
    serde_json::json!({
        "name": t.name,
        "rows": t.rows.len(),
        "cols": t.cols.len(),
        "cells": JsonMap::from_iter(cells.into_iter().map(|(r, c, v)| (a1(r, c), Value::String(v.clone())))),
        "fmts": JsonMap::from_iter(fmts.into_iter().map(|(r, c, f)| (a1(r, c), f.to_json()))),
        "colW": JsonMap::from_iter(col_w.into_iter().map(|(i, v)| (i.to_string(), num(v)))),
        "rowH": JsonMap::from_iter(row_h.into_iter().map(|(i, v)| (i.to_string(), num(v)))),
        "merges": merges,
        "frozenRows": num(t.frozen_rows),
        "frozenCols": num(t.frozen_cols),
    })
}

/// Any value read through `ToJson` (kept for API symmetry with the TS `map.toJSON()`).
pub fn map_json<T: ReadTxn>(txn: &T, m: &MapRef) -> Value {
    oc_crdt::value::any_to_json(&m.to_json(txn))
}
