// SPDX-License-Identifier: AGPL-3.0-only
//! Shared helpers for the golden / cross-language tests of oc-sheet.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use base64::Engine;
use oc_crdt::value::canonical;
use oc_sheet::a1::{a1, parse_range};
use oc_sheet::engine::{EngineOpts, SheetEngine};
use oc_sheet::format::{CellValue, format_value};
use oc_sheet::formula::workbook::FixedHost;
use oc_sheet::model::tab_to_json;
use serde_json::{Map, Value, json};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

pub fn load(name: &str) -> Value {
    let p = root().join("tests/golden/sheet").join(name);
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e} (run `TZ=UTC pnpm tsx tools/golden/sheet.ts`)", p.display()));
    serde_json::from_str(&s).expect("golden json")
}

pub fn unb64(s: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(s.as_str().expect("base64 string")).expect("base64")
}

pub fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// The deterministic clock the generator pins (`Date.now()`), UTC.
pub fn host(now: f64) -> Box<FixedHost> {
    Box::new(FixedHost::new(now, 7))
}

/// A doc decoded from yjs updates.
pub fn doc_from(updates: &[Vec<u8>]) -> yrs::Doc {
    let d = oc_crdt::codec::new_doc_with_client(999);
    for u in updates {
        oc_crdt::codec::apply_update(&d, u, None).expect("valid update");
    }
    d
}

pub fn engine(doc: &yrs::Doc, now: f64, live: bool) -> SheetEngine {
    SheetEngine::new(doc, EngineOpts { live, host: host(now), ..EngineOpts::default() })
}

/// Golden value encoding: non-finite numbers as `{ num }`.
pub fn value_json(v: &CellValue) -> Value {
    match v {
        CellValue::Number(n) if !n.is_finite() => json!({ "num": oc_sheet::jsnum::num_to_string(*n) }),
        o => o.to_json(),
    }
}

/// `evalJson` of the generator: value, kind, display and formula text of every non-empty (or listed) cell.
pub fn eval_json(e: &SheetEngine, extra: &Value) -> Value {
    let mut out = Map::new();
    for t in &e.tabs {
        let mut addrs: BTreeSet<(usize, usize)> = BTreeSet::new();
        for k in t.cells.keys() {
            if let Some(p) = t.pos_of(k) {
                addrs.insert((p.r, p.c));
            }
        }
        if let Some(list) = extra.get(&t.name).and_then(Value::as_array) {
            for a in list {
                let r = parse_range(a.as_str().unwrap()).unwrap().range;
                addrs.insert((r.r0, r.c0));
            }
        }
        let mut cells = Map::new();
        for (r, c) in addrs {
            let v = e.value(&t.id, r, c);
            let kind = e.kind(&t.id, r, c);
            let display = format_value(&v, t.fmt_at(r, c), kind);
            let mut o = Map::new();
            o.insert("value".into(), value_json(&v));
            o.insert("kind".into(), Value::String(kind.as_str().into()));
            o.insert("display".into(), Value::String(display));
            if let Some(s) = e.sheet_id(&t.id)
                && let Some(f) = e.workbook().formula(s, r as u32, c as u32)
            {
                o.insert("formula".into(), Value::String(f));
            }
            cells.insert(a1(r, c), Value::Object(o));
        }
        out.insert(t.name.clone(), Value::Object(cells));
    }
    Value::Object(out)
}

pub fn tabs_json(e: &SheetEngine) -> Value {
    Value::Array(e.tabs.iter().map(tab_to_json).collect())
}

/// `{ utf16: [...] }` (a JS string with unpaired surrogates) as Rust reads it: surrogates become U+FFFD.
fn lossy(v: &Value) -> Option<String> {
    let units: Vec<u16> = v.get("utf16")?.as_array()?.iter().map(|x| x.as_u64().unwrap_or(0) as u16).collect();
    Some(String::from_utf16_lossy(&units))
}

/// Numbers within 1e-9 (relative above 1), everything else exactly.
pub fn same(a: &Value, b: &Value) -> bool {
    if let (Value::String(x), Some(y)) = (a, lossy(b)) {
        return *x == y;
    }
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            x == y || (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0)
        }
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w))),
        _ => a == b,
    }
}

/// Differences between two golden-shaped objects (path → (actual, expected)).
pub fn diffs(path: &str, actual: &Value, expected: &Value, out: &mut Vec<String>) {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) if !e.contains_key("utf16") => {
            let keys: BTreeSet<&String> = a.keys().chain(e.keys()).collect();
            for k in keys {
                let p = format!("{path}.{k}");
                match (a.get(k), e.get(k)) {
                    (Some(x), Some(y)) => diffs(&p, x, y, out),
                    (x, y) => out.push(format!("{p}: actual {} expected {}", fmt(x), fmt(y))),
                }
            }
        }
        (Value::Array(a), Value::Array(e)) if a.len() == e.len() => {
            for (i, (x, y)) in a.iter().zip(e).enumerate() {
                diffs(&format!("{path}[{i}]"), x, y, out);
            }
        }
        _ => {
            if !same(&canonical(actual), &canonical(expected)) {
                out.push(format!("{path}: actual {} expected {}", actual, expected));
            }
        }
    }
}

fn fmt(v: Option<&Value>) -> String {
    v.map_or("<missing>".into(), Value::to_string)
}

#[track_caller]
pub fn assert_no_diffs(what: &str, d: &[String]) {
    if !d.is_empty() {
        let limit = if std::env::var_os("OC_SHEET_ALL_DIFFS").is_some() { usize::MAX } else { 60 };
        let shown: Vec<&String> = d.iter().take(limit).collect();
        panic!("{what}: {} difference(s)\n{}", d.len(), shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
    }
}

/// Re-read Rust output with the TS stack (`tools/golden/sheet.ts --verify`). `None` when the TS side is not
/// available (TS package deleted after cutover, no pnpm, or `OC_SKIP_TS_VERIFY` set).
pub fn ts_verify(xlsx: &[Vec<u8>], updates: &[Vec<Vec<u8>>]) -> Option<Value> {
    if std::env::var_os("OC_SKIP_TS_VERIFY").is_some() {
        eprintln!("OC_SKIP_TS_VERIFY set: skipping the TS side of the cross-check");
        return None;
    }
    let root = root();
    if !root.join("tools/golden/sheet.ts").exists() || !root.join("packages/sheet/src/index.ts").exists() {
        eprintln!("TS @oc/sheet not present: skipping the TS side of the cross-check");
        return None;
    }
    let input = json!({
        "xlsx": xlsx.iter().map(|b| b64(b)).collect::<Vec<_>>(),
        "updates": updates.iter().map(|us| us.iter().map(|u| b64(u)).collect::<Vec<_>>()).collect::<Vec<_>>(),
    });
    let tag = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let path = std::env::temp_dir().join(format!("oc-sheet-verify-{tag}.json"));
    let out_path = std::env::temp_dir().join(format!("oc-sheet-verify-{tag}.out.json"));
    std::fs::write(&path, serde_json::to_vec(&input).unwrap()).expect("write verify input");
    let out = Command::new("pnpm")
        .args(["tsx", "tools/golden/sheet.ts", "--verify"])
        .arg(&path)
        .arg("--out")
        .arg(&out_path)
        .env("TZ", "UTC")
        .current_dir(&root)
        .output();
    let _ = std::fs::remove_file(&path);
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cannot run pnpm ({e}): skipping the TS side of the cross-check");
            return None;
        }
    };
    assert!(out.status.success(), "TS verify failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(&out_path).expect("verify output file");
    let _ = std::fs::remove_file(&out_path);
    Some(serde_json::from_slice(&bytes).expect("verify output json"))
}

// ------------------------------------------------------------------ building sheets like the generator

use oc_crdt::rng::Mulberry32;
use oc_sheet::a1::CellAddr;
use oc_sheet::model::{Axis, InitOpts, add_tab, init_sheet, order_of, patch_format, set_frozen, set_merge, set_raw, set_track_size};
use yrs::{Any, Array, Out};

/// `sheetDoc(seed, rows, cols, clientID)`: one tab, track ids from mulberry32(seed). Returns the doc and the
/// generator, positioned where the TS one is after `initSheet` (later ids continue the same stream).
pub fn sheet_doc(seed: u32, rows: usize, cols: usize, client: u64) -> (yrs::Doc, Mulberry32) {
    let d = oc_crdt::codec::new_doc_with_client(client);
    let mut rng = Mulberry32::new(seed);
    init_sheet(&d, InitOpts { rows: Some(rows), cols: Some(cols), ..InitOpts::default() }, &mut rng);
    (d, rng)
}

/// A live engine over `doc` whose new row/column/tab ids continue `rng`.
pub fn engine_with(doc: &yrs::Doc, now: f64, live: bool, rng: Mulberry32) -> SheetEngine {
    SheetEngine::new(doc, EngineOpts { live, host: host(now), rng: Box::new(rng) })
}

pub fn cell(a: &str) -> CellAddr {
    let r = parse_range(a).unwrap_or_else(|| panic!("bad address {a}")).range;
    CellAddr { r: r.r0, c: r.c0 }
}

/// `put(ydoc, engine, tabIx, cells)`.
pub fn put(e: &mut SheetEngine, tab: usize, cells: &Map<String, Value>) {
    e.transact(|txn, tabs, _| {
        for (a, v) in cells {
            let p = cell(a);
            set_raw(txn, &tabs[tab], p.r, p.c, v.as_str().expect("raw string"));
        }
    });
}

/// `fmt(ydoc, engine, tabIx, fmts)` (`null` patch → clear the format).
pub fn put_fmt(e: &mut SheetEngine, tab: usize, fmts: &Map<String, Value>) {
    e.transact(|txn, tabs, _| {
        for (a, f) in fmts {
            let p = cell(a);
            patch_format(txn, &tabs[tab], p.r, p.c, f.as_object());
        }
    });
}

pub fn axis(v: &Value) -> Axis {
    if v.as_str() == Some("rows") { Axis::Rows } else { Axis::Cols }
}

fn uz(v: &Value) -> usize {
    v.as_u64().unwrap_or_else(|| panic!("expected an index, got {v}")) as usize
}

/// `applyStep` of the generator's recalc cases.
pub fn apply_step(e: &mut SheetEngine, s: &Value) {
    let tab_id = |e: &SheetEngine| e.tabs[uz(&s["tab"])].id.clone();
    let root = e.root().clone();
    match s["op"].as_str().expect("op") {
        "set" => put(e, uz(&s["tab"]), s["cells"].as_object().expect("cells")),
        "fmt" => put_fmt(e, uz(&s["tab"]), s["fmts"].as_object().expect("fmts")),
        "insert" => {
            let id = tab_id(e);
            e.insert(&id, axis(&s["axis"]), uz(&s["at"]), uz(&s["n"]));
        }
        "remove" => {
            let id = tab_id(e);
            e.remove(&id, axis(&s["axis"]), uz(&s["at"]), uz(&s["n"]));
        }
        "rename" => {
            let id = tab_id(e);
            e.rename_tab(&id, s["name"].as_str().expect("name"));
        }
        "deleteTab" => {
            let id = tab_id(e);
            e.delete_tab(&id);
        }
        "addTab" => {
            let name = s["name"].as_str().expect("name").to_string();
            e.transact(|txn, _, rng| {
                add_tab(txn, &root, Some(&name), None, rng);
            });
        }
        "merge" => {
            let ix = uz(&s["tab"]);
            let range = s["range"].as_str().map(|r| parse_range(r).expect("range").range);
            let anchor = s.get("anchor").and_then(Value::as_str).map(cell);
            e.transact(|txn, tabs, _| set_merge(txn, &tabs[ix], range, anchor));
        }
        "frozen" => {
            let ix = uz(&s["tab"]);
            let (r, c) = (s["rows"].as_f64(), s["cols"].as_f64());
            e.transact(|txn, tabs, _| set_frozen(txn, &tabs[ix], r, c));
        }
        "size" => {
            let ix = uz(&s["tab"]);
            let (ax, index, px) = (axis(&s["axis"]), uz(&s["index"]), s["px"].as_f64());
            e.transact(|txn, tabs, _| set_track_size(txn, &tabs[ix], ax, index, px));
        }
        "writeRows" => {
            let ix = uz(&s["tab"]);
            let at = cell(s["at"].as_str().expect("at"));
            let rows: Vec<Vec<String>> = serde_json::from_value(s["rows"].clone()).expect("rows");
            e.transact(|txn, tabs, rng| oc_sheet::csv::write_rows(txn, &tabs[ix], at.r, at.c, &rows, rng));
        }
        "order" => {
            let perm: Vec<usize> = s["ids"].as_array().expect("ids").iter().map(uz).collect();
            e.transact(|txn, _, _| {
                let order = order_of(txn, &root).expect("order");
                let ids: Vec<String> = order
                    .iter(txn)
                    .map(|o| match o {
                        Out::Any(Any::String(s)) => s.to_string(),
                        other => panic!("unexpected order entry {other:?}"),
                    })
                    .collect();
                order.remove_range(txn, 0, ids.len() as u32);
                order.insert_range(txn, 0, perm.iter().map(|i| Any::String(ids[*i].clone().into())));
            });
        }
        other => panic!("unknown step op {other}"),
    }
}
