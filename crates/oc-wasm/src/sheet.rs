// SPDX-License-Identifier: AGPL-3.0-only
//! oc-sheet in the browser: the JS `SheetEngine` handle and the `@oc/sheet` helpers apps/web uses.
//!
//! Same sync model as the editor: the browser's yjs `Y.Doc` (provider, `Y.UndoManager`, direct `Y.Map` writes
//! from the grid) stays the source of truth; this handle keeps a yrs mirror with a live formula engine. Every
//! yjs update not made by the mirror is applied here ([`SheetEngine::apply_remote_update`]); the model writes
//! the TS package did on yjs types (`setRaw`, `patchFormat`, `setMerge`, `writeRows`, `addTab`, `initSheet`,
//! row/column and tab edits with formula rewrites) run on the mirror and come back as update bytes
//! ([`SheetEngine::drain_local_updates`]) that the facade applies inside the caller's yjs transaction, so the
//! yjs undo manager records them under the caller's origin as before.
//!
//! Writers address tabs by id and resolve positions against the mirror's current tab structure.

use std::sync::{Arc, Mutex};

use oc_crdt::OcOrigin;
use oc_crdt::rng::{Mulberry32, OsRandom, RandomSource};
use oc_schema::elements::{ChartEl, ChartSource};
use oc_sheet::a1::{self, CellAddr, Range};
use oc_sheet::chart::{self, ChartData};
use oc_sheet::engine::{EngineOpts, SheetEngine as Engine};
use oc_sheet::format::{self, CellValue, ValueKind};
use oc_sheet::formula::workbook::{FixedHost, Host};
use oc_sheet::model::{self, Axis, CellFormat, InitOpts, TabSnap};
use oc_sheet::{csv, linked, table, xlsx};
use serde_json::{Map, Value, json};
use wasm_bindgen::prelude::*;

use crate::convert::{arg, err, js, opt_arg, to_js, typed, value};

/// The browser clock, timezone and `Math.random` for `NOW()`, `TODAY()`, `RAND()` (wasm32 only).
#[cfg(target_arch = "wasm32")]
struct BrowserHost;

#[cfg(target_arch = "wasm32")]
impl Host for BrowserHost {
    fn now_ms(&self) -> f64 {
        js_sys::Date::now()
    }

    fn tz_offset_minutes(&self) -> f64 {
        js_sys::Date::new_0().get_timezone_offset()
    }

    fn random(&self) -> f64 {
        js_sys::Math::random()
    }
}

fn get(o: &JsValue, k: &str) -> JsValue {
    js_sys::Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

fn axis(s: &str) -> Result<Axis, JsError> {
    match s {
        "rows" => Ok(Axis::Rows),
        "cols" => Ok(Axis::Cols),
        other => Err(err(format!("axis must be 'rows' or 'cols', got {other:?}"))),
    }
}

/// Structure of one tab as the facade's `TabSnap`s depend on it.
#[derive(PartialEq)]
struct Shape {
    id: String,
    name: String,
    rows: Vec<String>,
    cols: Vec<String>,
    frozen: (f64, f64),
}

fn shape_of(t: &TabSnap) -> Shape {
    Shape { id: t.id.clone(), name: t.name.clone(), rows: t.rows.clone(), cols: t.cols.clone(), frozen: (t.frozen_rows, t.frozen_cols) }
}

fn same_shape(a: &[Shape], b: &[TabSnap]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(s, t)| s.id == t.id && s.name == t.name && s.frozen == (t.frozen_rows, t.frozen_cols) && s.rows == t.rows && s.cols == t.cols)
}

/// A yrs mirror of the browser's sheet document with a live formula engine.
#[wasm_bindgen]
pub struct SheetEngine {
    e: Engine,
    outbox: Arc<Mutex<Vec<Vec<u8>>>>,
    pending: Vec<Vec<u8>>,
    key: String,
    shapes: Vec<Shape>,
    struct_rev: u64,
}

impl SheetEngine {
    /// The mirror without JS: `seed` → deterministic ids (else OS randomness), `now` → a fixed clock (else the
    /// browser's on wasm32, the Unix epoch natively).
    pub fn create(update: &[u8], client_id: Option<u64>, seed: Option<u32>, now: Option<f64>, live: bool) -> Result<SheetEngine, String> {
        let doc = match client_id {
            Some(id) => oc_crdt::new_doc_with_client(id),
            None => oc_crdt::new_doc(),
        };
        if !update.is_empty() {
            oc_crdt::apply_update(&doc, update, None).map_err(|e| format!("initial update: {e}"))?;
        }
        let rng: Box<dyn RandomSource + Send> = match seed {
            Some(s) => Box::new(Mulberry32::new(s)),
            None => Box::new(OsRandom),
        };
        let host: Box<dyn Host> = match now {
            Some(ms) => Box::new(FixedHost::new(ms, seed.unwrap_or(7))),
            #[cfg(target_arch = "wasm32")]
            None => Box::new(BrowserHost),
            #[cfg(not(target_arch = "wasm32"))]
            None => Box::new(FixedHost::default()),
        };
        let outbox: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let key = format!("oc-wasm-sheet-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let ob = outbox.clone();
        let _ = doc.observe_update_v1(key.as_str(), move |txn, ev| {
            if txn.origin().and_then(OcOrigin::from_origin) != Some(OcOrigin::Remote) {
                ob.lock().unwrap_or_else(|p| p.into_inner()).push(ev.update.clone());
            }
        });
        let e = Engine::new(&doc, EngineOpts { live, host, rng });
        let shapes = e.tabs.iter().map(shape_of).collect();
        Ok(SheetEngine { e, outbox, pending: Vec::new(), key, shapes, struct_rev: 1 })
    }

    pub fn engine(&self) -> &Engine {
        &self.e
    }

    /// Collect local updates and track the tab structure; returns the status word.
    fn settle(&mut self) -> f64 {
        let ups = std::mem::take(&mut *self.outbox.lock().unwrap_or_else(|p| p.into_inner()));
        self.pending.extend(ups);
        if !same_shape(&self.shapes, &self.e.tabs) {
            self.shapes = self.e.tabs.iter().map(shape_of).collect();
            self.struct_rev += 1;
        }
        self.status()
    }

    fn tab(&self, id: &str) -> Result<&TabSnap, JsError> {
        self.e.tab(id).ok_or_else(|| err(format!("no tab {id}")))
    }

    /// Run a model write on the current snapshot of tab `id`.
    fn write(&mut self, id: &str, f: impl FnOnce(&mut yrs::TransactionMut, &TabSnap, &mut dyn RandomSource)) -> Result<f64, JsError> {
        if self.e.tab(id).is_none() {
            return Err(err(format!("no tab {id}")));
        }
        let id = id.to_string();
        self.e.transact(|txn, tabs, rng| {
            if let Some(t) = tabs.iter().find(|t| t.id == id) {
                f(txn, t, rng);
            }
        });
        Ok(self.settle())
    }

    fn cell_value(&self, tab: &str, r: usize, c: usize) -> CellValue {
        self.e.value(tab, r, c)
    }

    pub fn take_updates(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending)
    }

    pub fn apply_update(&mut self, update: &[u8]) -> Result<f64, String> {
        oc_crdt::apply_update(self.e.doc(), update, Some(OcOrigin::Remote.origin())).map_err(|e| e.to_string())?;
        self.e.sync();
        Ok(self.settle())
    }
}

impl Drop for SheetEngine {
    fn drop(&mut self) {
        let _ = self.e.doc().unobserve_update_v1(self.key.as_str());
    }
}

fn range_arg(v: &JsValue) -> Result<Range, JsError> {
    arg("range", v)
}

fn fmt_arg(v: &JsValue) -> Result<Option<CellFormat>, JsError> {
    Ok(match value(v)? {
        Value::Null => None,
        x => Some(CellFormat::from_json(&x)),
    })
}

fn value_arg(v: &JsValue) -> Result<CellValue, JsError> {
    Ok(CellValue::from_json(&value(v)?))
}

#[wasm_bindgen]
impl SheetEngine {
    /// `new SheetEngine(update, opts?)`: `update` is `Y.encodeStateAsUpdate(ydoc)`; `opts` is
    /// `{ clientId?, seed?, now?, live? }`.
    #[wasm_bindgen(constructor)]
    pub fn new(update: &[u8], opts: JsValue) -> Result<SheetEngine, JsError> {
        let o = |k: &str| if opts.is_object() { get(&opts, k) } else { JsValue::UNDEFINED };
        let live = o("live").as_bool().unwrap_or(true);
        SheetEngine::create(update, o("clientId").as_f64().map(|x| x as u64), o("seed").as_f64().map(|x| x as u32), o("now").as_f64(), live).map_err(err)
    }

    /// `version * 2 + updatesPending`.
    pub fn status(&self) -> f64 {
        self.e.version as f64 * 2.0 + if self.pending.is_empty() { 0.0 } else { 1.0 }
    }

    #[wasm_bindgen(getter)]
    pub fn version(&self) -> f64 {
        self.e.version as f64
    }

    /// Bumped when any tab's id, name, rows, columns or frozen panes change (the facade re-snaps its tabs then).
    #[wasm_bindgen(getter, js_name = structureRev)]
    pub fn structure_rev(&self) -> f64 {
        self.struct_rev as f64
    }

    #[wasm_bindgen(js_name = applyRemoteUpdate)]
    pub fn apply_remote_update(&mut self, update: &[u8]) -> Result<f64, JsError> {
        self.apply_update(update).map_err(err)
    }

    #[wasm_bindgen(js_name = drainLocalUpdates)]
    pub fn drain_local_updates(&mut self) -> js_sys::Array {
        self.take_updates().into_iter().map(|u| JsValue::from(js_sys::Uint8Array::from(u.as_slice()))).collect()
    }

    /// Tab ids in display order.
    #[wasm_bindgen(js_name = tabIds)]
    pub fn tab_ids(&self) -> Vec<String> {
        self.e.tabs.iter().map(|t| t.id.clone()).collect()
    }

    #[wasm_bindgen(js_name = hasSheet)]
    pub fn has_sheet(&self) -> bool {
        model::has_sheet(self.e.doc())
    }

    // ------------------------------------------------------------------ values

    pub fn value(&self, tab: &str, r: usize, c: usize) -> JsValue {
        js(&self.cell_value(tab, r, c))
    }

    pub fn kind(&self, tab: &str, r: usize, c: usize) -> String {
        self.e.kind(tab, r, c).as_str().to_string()
    }

    /// The workbook's (normalised) formula text of a cell, `undefined` for non-formulas (engine addition).
    pub fn formula(&self, tab: &str, r: u32, c: u32) -> Option<String> {
        self.e.sheet_id(tab).and_then(|s| self.e.workbook().formula(s, r, c))
    }

    #[wasm_bindgen(js_name = rangeValues)]
    pub fn range_values(&self, tab: &str, range: JsValue) -> Result<JsValue, JsError> {
        to_js(&self.e.range_values(tab, &range_arg(&range)?))
    }

    /// `displayRows(engine, t, range)`: formatted values of a range (also the batch read for a grid render).
    #[wasm_bindgen(js_name = displayRows)]
    pub fn display_rows(&self, tab: &str, range: JsValue) -> Result<JsValue, JsError> {
        to_js(&csv::display_rows(&self.e, self.tab(tab)?, &range_arg(&range)?))
    }

    #[wasm_bindgen(js_name = rawRows)]
    pub fn raw_rows(&self, tab: &str, range: JsValue) -> Result<JsValue, JsError> {
        to_js(&csv::raw_rows(self.tab(tab)?, &range_arg(&range)?))
    }

    #[wasm_bindgen(js_name = tabToCsv)]
    pub fn tab_to_csv(&self, tab: &str) -> Result<String, JsError> {
        Ok(csv::tab_to_csv(&self.e, self.tab(tab)?))
    }

    #[wasm_bindgen(js_name = usedRange)]
    pub fn used_range(&self, tab: &str) -> Result<JsValue, JsError> {
        to_js(&self.tab(tab)?.used_range())
    }

    #[wasm_bindgen(js_name = mergeRanges)]
    pub fn merge_ranges(&self, tab: &str) -> Result<JsValue, JsError> {
        to_js(&self.tab(tab)?.merge_ranges())
    }

    /// `rangeToTable(engine, t, range, { x?, y?, scale?, family?, rows? })` → a `TableEl`.
    #[wasm_bindgen(js_name = rangeToTable)]
    pub fn range_to_table(&self, tab: &str, range: JsValue, opts: JsValue) -> Result<JsValue, JsError> {
        let o: Map<String, Value> = opt_arg("opts", &opts)?.unwrap_or_default();
        let f = |k: &str| o.get(k).and_then(Value::as_f64);
        let rows: Option<Vec<usize>> = match o.get("rows") {
            Some(v) if !v.is_null() => Some(typed("rows", v.clone())?),
            _ => None,
        };
        let opts = table::TableOptions { x: f("x"), y: f("y"), scale: f("scale"), family: o.get("family").and_then(Value::as_str).map(str::to_string), rows };
        to_js(&oc_schema::Element::from(table::range_to_table(&self.e, self.tab(tab)?, &range_arg(&range)?, &opts)))
    }

    /// `sheetToDesign(engine, { title, w, h, margin?, maxPages?, tabs? })`.
    #[wasm_bindgen(js_name = sheetToDesign)]
    pub fn sheet_to_design(&self, opts: JsValue) -> Result<JsValue, JsError> {
        let o: Map<String, Value> = arg("opts", &opts)?;
        let opts = table::SheetLayoutOpts {
            title: o.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
            w: o.get("w").and_then(Value::as_f64).unwrap_or(f64::NAN),
            h: o.get("h").and_then(Value::as_f64).unwrap_or(f64::NAN),
            margin: o.get("margin").and_then(Value::as_f64),
            max_pages: o.get("maxPages").and_then(Value::as_f64).map(|x| x as usize),
            tabs: match o.get("tabs") {
                Some(v) if !v.is_null() => Some(typed("tabs", v.clone())?),
                _ => None,
            },
        };
        to_js(&table::sheet_to_design(&self.e, &opts))
    }

    /// `readRange(engine, sheet, range)` → `{ tabId, tabName, values }` or `null`.
    #[wasm_bindgen(js_name = readRange)]
    pub fn read_range(&self, sheet: &str, range: &str) -> JsValue {
        match linked::read_range(&self.e, sheet, range) {
            Some(r) => js(&json!({ "tabId": r.tab_id, "tabName": r.tab_name, "values": r.values })),
            None => JsValue::NULL,
        }
    }

    /// `sheetToXlsx(engine, { creator? })` (the creation time is the engine's clock).
    #[wasm_bindgen(js_name = toXlsx)]
    pub fn to_xlsx(&self, creator: Option<String>, created_ms: f64) -> Result<Vec<u8>, JsError> {
        xlsx::sheet_to_xlsx(&self.e, &xlsx::XlsxOptions { creator, created_ms }).map_err(err)
    }

    /// Recompute everything from the document.
    pub fn rebuild(&mut self) -> f64 {
        self.e.rebuild();
        self.settle()
    }

    // ------------------------------------------------------------------ structural edits (formulas follow)

    pub fn insert(&mut self, tab: &str, ax: &str, at: usize, n: usize) -> Result<f64, JsError> {
        self.e.insert(tab, axis(ax)?, at, n);
        Ok(self.settle())
    }

    pub fn remove(&mut self, tab: &str, ax: &str, at: usize, n: usize) -> Result<f64, JsError> {
        self.e.remove(tab, axis(ax)?, at, n);
        Ok(self.settle())
    }

    /// Returns the name used; read the status afterwards.
    #[wasm_bindgen(js_name = renameTab)]
    pub fn rename_tab(&mut self, tab: &str, name: &str) -> String {
        let n = self.e.rename_tab(tab, name);
        self.settle();
        n
    }

    #[wasm_bindgen(js_name = deleteTab)]
    pub fn delete_tab(&mut self, tab: &str) -> f64 {
        self.e.delete_tab(tab);
        self.settle()
    }

    // ------------------------------------------------------------------ model writes (`@oc/sheet` model.ts)

    #[wasm_bindgen(js_name = setRaw)]
    pub fn set_raw(&mut self, tab: &str, r: usize, c: usize, raw: &str) -> Result<f64, JsError> {
        self.write(tab, |txn, t, _| model::set_raw(txn, t, r, c, raw))
    }

    /// `patchFormat(t, r, c, patch | null)`: `null` clears; in a patch `null`/`false`/`""` remove a key.
    #[wasm_bindgen(js_name = patchFormat)]
    pub fn patch_format(&mut self, tab: &str, r: usize, c: usize, patch: JsValue) -> Result<f64, JsError> {
        let p: Option<Map<String, Value>> = opt_arg("patch", &patch)?;
        self.write(tab, |txn, t, _| model::patch_format(txn, t, r, c, p.as_ref()))
    }

    /// `setMerge(t, range | null, anchor?)`.
    #[wasm_bindgen(js_name = setMerge)]
    pub fn set_merge(&mut self, tab: &str, range: JsValue, anchor: JsValue) -> Result<f64, JsError> {
        let r: Option<Range> = opt_arg("range", &range)?;
        let a: Option<CellAddr> = opt_arg("anchor", &anchor)?;
        self.write(tab, |txn, t, _| model::set_merge(txn, t, r, a))
    }

    #[wasm_bindgen(js_name = insertTracks)]
    pub fn insert_tracks(&mut self, tab: &str, ax: &str, at: usize, n: usize) -> Result<f64, JsError> {
        let a = axis(ax)?;
        self.write(tab, |txn, t, rng| model::insert_tracks(txn, t, a, at, n, rng))
    }

    #[wasm_bindgen(js_name = deleteTracks)]
    pub fn delete_tracks(&mut self, tab: &str, ax: &str, at: usize, n: usize) -> Result<f64, JsError> {
        let a = axis(ax)?;
        self.write(tab, |txn, t, _| model::delete_tracks(txn, t, a, at, n))
    }

    /// `writeRows(t, r, c, rows)`, growing the grid when needed.
    #[wasm_bindgen(js_name = writeRows)]
    pub fn write_rows(&mut self, tab: &str, r: usize, c: usize, rows: JsValue) -> Result<f64, JsError> {
        let rows: Vec<Vec<String>> = arg("rows", &rows)?;
        self.write(tab, |txn, t, rng| csv::write_rows(txn, t, r, c, &rows, rng))
    }

    #[wasm_bindgen(js_name = setTrackSize)]
    pub fn set_track_size(&mut self, tab: &str, ax: &str, index: usize, px: Option<f64>) -> Result<f64, JsError> {
        let a = axis(ax)?;
        self.write(tab, |txn, t, _| model::set_track_size(txn, t, a, index, px))
    }

    #[wasm_bindgen(js_name = setFrozen)]
    pub fn set_frozen(&mut self, tab: &str, rows: Option<f64>, cols: Option<f64>) -> Result<f64, JsError> {
        self.write(tab, |txn, t, _| model::set_frozen(txn, t, rows, cols))
    }

    /// `addTab(root, name?, after?)` → the new tab id; read the status afterwards.
    #[wasm_bindgen(js_name = addTab)]
    pub fn add_tab(&mut self, name: Option<String>, after: Option<String>) -> String {
        let root = self.e.root().clone();
        let id = self.e.transact(|txn, _, rng| model::add_tab(txn, &root, name.as_deref(), after.as_deref(), rng));
        self.settle();
        id
    }

    /// `uniqueTabName(root, base, except?)`: tab names are unique (case-insensitive) and cleaned.
    #[wasm_bindgen(js_name = uniqueTabName)]
    pub fn unique_tab_name(&self, base: &str, except: Option<String>) -> String {
        let txn = yrs::Transact::transact(self.e.doc());
        model::unique_tab_name(&txn, self.e.root(), base, except.as_deref())
    }

    /// `deleteTab(root, id)` (the model write: no formula rewrite; the engine's `deleteTab` rewrites).
    #[wasm_bindgen(js_name = deleteTabModel)]
    pub fn delete_tab_model(&mut self, id: &str) -> f64 {
        let root = self.e.root().clone();
        self.e.transact(|txn, _, _| model::delete_tab(txn, &root, id));
        self.settle()
    }

    /// `initSheet(ydoc, { name?, rows?, cols? })` → the first tab id (existing sheets are left alone).
    #[wasm_bindgen(js_name = initSheet)]
    pub fn init_sheet(&mut self, name: Option<String>, rows: Option<usize>, cols: Option<usize>) -> String {
        let root = self.e.root().clone();
        let opts = InitOpts { name, rows, cols, origin: None };
        let id = self.e.transact(|txn, _, rng| model::init_sheet_txn(txn, &root, &opts, rng));
        self.settle();
        id
    }
}

// ------------------------------------------------------------------ pure helpers

#[wasm_bindgen(js_name = sheetConstants)]
pub fn sheet_constants() -> JsValue {
    js(&json!({
        "SHEET_ROOT": model::SHEET_ROOT,
        "DEFAULT_ROWS": model::DEFAULT_ROWS,
        "DEFAULT_COLS": model::DEFAULT_COLS,
        "DEFAULT_COL_W": model::DEFAULT_COL_W,
        "DEFAULT_ROW_H": model::DEFAULT_ROW_H,
        "MAX_ROWS": model::MAX_ROWS,
        "MAX_COLS": model::MAX_COLS,
    }))
}

#[wasm_bindgen(js_name = colName)]
pub fn col_name(c: f64) -> String {
    a1::col_name(c.max(0.0) as usize)
}

#[wasm_bindgen(js_name = colIndex)]
pub fn col_index(name: &str) -> f64 {
    a1::col_index(name).map_or(-1.0, |c| c as f64)
}

#[wasm_bindgen]
pub fn a1(r: f64, c: f64) -> String {
    a1::a1(r.max(0.0) as usize, c.max(0.0) as usize)
}

#[wasm_bindgen(js_name = parseA1)]
pub fn parse_a1(s: &str) -> JsValue {
    a1::parse_a1(s).map_or(JsValue::NULL, |p| js(&p))
}

#[wasm_bindgen(js_name = normRange)]
pub fn norm_range(a: JsValue, b: JsValue) -> Result<JsValue, JsError> {
    let a: CellAddr = arg("a", &a)?;
    let b: CellAddr = opt_arg("b", &b)?.unwrap_or(a);
    to_js(&a1::norm_range(a, b))
}

#[wasm_bindgen(js_name = parseRange)]
pub fn parse_range(s: &str) -> JsValue {
    a1::parse_range(s).map_or(JsValue::NULL, |r| js(&r))
}

#[wasm_bindgen(js_name = formatRange)]
pub fn format_range(r: JsValue) -> Result<String, JsError> {
    Ok(a1::format_range(&range_arg(&r)?))
}

#[wasm_bindgen(js_name = quoteSheet)]
pub fn quote_sheet(name: &str) -> String {
    a1::quote_sheet(name)
}

#[wasm_bindgen(js_name = shiftFormula)]
pub fn shift_formula(src: &str, dr: f64, dc: f64) -> String {
    a1::shift_formula(src, dr as i64, dc as i64)
}

#[wasm_bindgen(js_name = splitKey)]
pub fn split_key(k: &str) -> Vec<String> {
    let (r, c) = model::split_key(k);
    vec![r.to_string(), c.to_string()]
}

#[wasm_bindgen(js_name = generalNumber)]
pub fn general_number(n: f64) -> String {
    format::general_number(n)
}

/// `formatValue(v, fmt?, kind = 'number')`.
#[wasm_bindgen(js_name = formatValue)]
pub fn format_value(v: JsValue, fmt: JsValue, kind: Option<String>) -> Result<String, JsError> {
    let k: ValueKind = match kind {
        Some(k) => typed("kind", Value::String(k))?,
        None => ValueKind::Number,
    };
    Ok(format::format_value(&value_arg(&v)?, fmt_arg(&fmt)?.as_ref(), k))
}

#[wasm_bindgen(js_name = autoAlign)]
pub fn auto_align(v: JsValue, fmt: JsValue) -> Result<String, JsError> {
    let f = fmt_arg(&fmt)?;
    Ok(format::auto_align(&value_arg(&v)?, f.as_ref()).to_string())
}

fn delim(d: Option<String>) -> char {
    d.and_then(|s| s.chars().next()).unwrap_or(',')
}

#[wasm_bindgen(js_name = parseDelimited)]
pub fn parse_delimited(text: &str, d: Option<String>) -> JsValue {
    js(&csv::parse_delimited(text, delim(d)))
}

#[wasm_bindgen(js_name = quoteField)]
pub fn quote_field(s: &str, d: Option<String>) -> String {
    csv::quote_field(s, delim(d))
}

#[wasm_bindgen(js_name = toDelimited)]
pub fn to_delimited(rows: JsValue, d: Option<String>, eol: Option<String>) -> Result<String, JsError> {
    let rows: Vec<Vec<String>> = arg("rows", &rows)?;
    Ok(csv::to_delimited(&rows, delim(d), eol.as_deref().unwrap_or("\r\n")))
}

#[wasm_bindgen(js_name = rangeToChartData)]
pub fn range_to_chart_data(values: JsValue) -> Result<JsValue, JsError> {
    let rows: Vec<Vec<Value>> = arg("values", &values)?;
    let cells: Vec<Vec<CellValue>> = rows.iter().map(|r| r.iter().map(CellValue::from_json).collect()).collect();
    to_js(&chart::range_to_chart_data(&cells))
}

/// A chart element from a `Pick<ChartEl, …>` (the fields a helper does not read get `makeChart` defaults).
fn chart_el(v: &JsValue) -> Result<ChartEl, JsError> {
    use oc_schema::factory::{ChartOpts, base_init, make_chart, xf};
    let mut base = match serde_json::to_value(oc_schema::Element::from(make_chart(base_init(xf(0.0, 0.0, 100.0, 100.0, 0.0)), ChartOpts::default()))) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    if let Value::Object(m) = value(v)? {
        base.extend(m);
    }
    match crate::convert::element(Value::Object(base)) {
        oc_schema::Element::Chart(c) => Ok(c),
        _ => Err(err("not a chart element")),
    }
}

/// `chartPatch(el, data, source?)` → `{ labels, series, source? }`.
#[wasm_bindgen(js_name = chartPatch)]
pub fn chart_patch(el: JsValue, data: JsValue, source: JsValue) -> Result<JsValue, JsError> {
    let d: ChartData = arg("data", &data)?;
    let s: Option<ChartSource> = opt_arg("source", &source)?;
    let p = chart::chart_patch(&chart_el(&el)?, &d, s);
    let mut out = json!({ "labels": p.labels, "series": p.series });
    if let Some(src) = p.source {
        out["source"] = serde_json::to_value(src).map_err(err)?;
    }
    to_js(&out)
}

#[wasm_bindgen(js_name = chartChanged)]
pub fn chart_changed(el: JsValue, data: JsValue) -> Result<bool, JsError> {
    let d: ChartData = arg("data", &data)?;
    Ok(chart::chart_changed(&chart_el(&el)?, &d))
}

/// `readXlsx(bytes)` → `{ [sheet]: rows }`.
#[wasm_bindgen(js_name = readXlsx)]
pub fn read_xlsx(bytes: &[u8]) -> Result<JsValue, JsError> {
    to_js(&xlsx::read_xlsx(bytes).map_err(err)?)
}
