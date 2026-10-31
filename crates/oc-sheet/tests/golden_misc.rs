// SPDX-License-Identifier: AGPL-3.0-only
//! Helper parity (`misc.json`): A1 addressing, display formatting, CSV, linked-chart mapping, the Y.Doc model
//! helpers (same ids from the same seeds), sheet → table layout, `readRange` and `resolveLinkedCharts`.

mod common;

use std::collections::HashMap;

use common::*;
use oc_crdt::rng::Mulberry32;
use oc_schema::DesignDoc;
use oc_schema::elements::ChartEl;
use oc_sheet::a1::{CellAddr, Range, col_index, col_name, format_range, parse_a1, parse_range, quote_sheet, shift_formula};
use oc_sheet::chart::{ChartData, chart_changed, chart_patch, range_to_chart_data};
use oc_sheet::csv::{parse_delimited, quote_field, to_delimited, write_rows};
use oc_sheet::format::{CellValue, ValueKind, auto_align, format_value, general_number};
use oc_sheet::linked::{open_sheet_state, read_range, resolve_linked_charts};
use oc_sheet::model::{
    self, Axis, CellFormat, add_tab, delete_tab, delete_tracks, insert_tracks, order_of, patch_format, set_merge, set_raw, sheet_root, snap_tabs,
    tab_ids, tab_to_json, unique_tab_name,
};
use oc_sheet::table::{SheetLayoutOpts, TableOptions, range_to_table, sheet_to_design};
use serde_json::{Map, Value, json};
use yrs::{Any, Array, Out, ReadTxn, Transact};

fn misc() -> Value {
    load("misc.json")
}

/// A number from the golden (`{ num: "NaN" | "Infinity" | "-Infinity" }` for non-finite ones).
fn num(v: &Value) -> f64 {
    match v.get("num").and_then(Value::as_str) {
        Some("NaN") => f64::NAN,
        Some("Infinity") => f64::INFINITY,
        Some("-Infinity") => f64::NEG_INFINITY,
        Some(other) => panic!("num {other}"),
        None => v.as_f64().expect("number"),
    }
}

fn s(v: &Value) -> &str {
    v.as_str().expect("string")
}

fn kind(k: &str) -> ValueKind {
    serde_json::from_value(json!(k)).expect("kind")
}

fn fmt_of(v: &Value) -> Option<CellFormat> {
    (!v.is_null()).then(|| CellFormat::from_json(v))
}

fn values(v: &Value) -> Vec<Vec<CellValue>> {
    v.as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(CellValue::from_json).collect()).collect()
}

fn rows_of(v: &Value) -> Vec<Vec<String>> {
    serde_json::from_value(v.clone()).expect("rows of strings")
}

fn ch(v: &Value) -> char {
    s(v).chars().next().expect("delimiter")
}

#[test]
fn a1_helpers() {
    let g = misc();
    let mut d = Vec::new();
    for c in g["colName"].as_array().unwrap() {
        diffs(&format!("colName({})", c[0]), &json!(col_name(c[0].as_u64().unwrap() as usize)), &c[1], &mut d);
    }
    for c in g["colIndex"].as_array().unwrap() {
        diffs(&format!("colIndex({})", c[0]), &json!(col_index(s(&c[0])).map_or(-1, |i| i as i64)), &c[1], &mut d);
    }
    for c in g["parseA1"].as_array().unwrap() {
        diffs(&format!("parseA1({})", c[0]), &serde_json::to_value(parse_a1(s(&c[0]))).unwrap(), &c[1], &mut d);
    }
    for c in g["parseRange"].as_array().unwrap() {
        diffs(&format!("parseRange({})", c[0]), &serde_json::to_value(parse_range(s(&c[0]))).unwrap(), &c[1], &mut d);
    }
    for c in g["formatRange"].as_array().unwrap() {
        let r: Range = serde_json::from_value(c[0].clone()).unwrap();
        diffs(&format!("formatRange({})", c[0]), &json!(format_range(&r)), &c[1], &mut d);
    }
    for c in g["quoteSheet"].as_array().unwrap() {
        diffs(&format!("quoteSheet({})", c[0]), &json!(quote_sheet(s(&c[0]))), &c[1], &mut d);
    }
    for c in g["shiftFormula"].as_array().unwrap() {
        let got = shift_formula(s(&c[0]), c[1].as_i64().unwrap(), c[2].as_i64().unwrap());
        diffs(&format!("shiftFormula({}, {}, {})", c[0], c[1], c[2]), &json!(got), &c[3], &mut d);
    }
    assert_no_diffs("a1", &d);
}

#[test]
fn display_formatting() {
    let g = misc();
    let mut d = Vec::new();
    for c in g["generalNumber"].as_array().unwrap() {
        diffs(&format!("generalNumber({})", c[0]), &json!(general_number(num(&c[0]))), &c[1], &mut d);
    }
    for c in g["formatValue"].as_array().unwrap() {
        let v = CellValue::from_json(&c[0]);
        let f = fmt_of(&c[1]);
        let got = format_value(&v, f.as_ref(), kind(s(&c[2])));
        diffs(&format!("formatValue({}, {}, {})", c[0], c[1], c[2]), &json!(got), &c[3], &mut d);
    }
    for c in g["autoAlign"].as_array().unwrap() {
        let f = fmt_of(&c[1]);
        diffs(&format!("autoAlign({}, {})", c[0], c[1]), &json!(auto_align(&CellValue::from_json(&c[0]), f.as_ref())), &c[2], &mut d);
    }
    assert_no_diffs("format", &d);
}

#[test]
fn csv_helpers() {
    let g = misc();
    let mut d = Vec::new();
    for c in g["toDelimited"].as_array().unwrap() {
        diffs("toDelimited", &json!(to_delimited(&rows_of(&c[0]), ch(&c[1]), s(&c[2]))), &c[3], &mut d);
    }
    for c in g["parseDelimited"].as_array().unwrap() {
        diffs(&format!("parseDelimited({:?}, {:?})", c[0], c[1]), &json!(parse_delimited(s(&c[0]), ch(&c[1]))), &c[2], &mut d);
    }
    for c in g["quoteField"].as_array().unwrap() {
        diffs(&format!("quoteField({:?}, {:?})", c[0], c[1]), &json!(quote_field(s(&c[0]), ch(&c[1]))), &c[2], &mut d);
    }
    assert_no_diffs("csv", &d);
}

fn patch_json(p: &oc_sheet::chart::ChartPatch) -> Value {
    let mut o = json!({ "labels": p.labels, "series": p.series });
    if let Some(src) = &p.source {
        o["source"] = serde_json::to_value(src).unwrap();
    }
    o
}

#[test]
fn chart_mapping() {
    let g = misc();
    let mut d = Vec::new();
    for (i, c) in g["rangeToChartData"].as_array().unwrap().iter().enumerate() {
        diffs(&format!("rangeToChartData[{i}]"), &serde_json::to_value(range_to_chart_data(&values(&c[0]))).unwrap(), &c[1], &mut d);
    }
    let cp = &g["chartPatch"];
    let el: ChartEl = serde_json::from_value(cp[0].clone()).unwrap();
    let data: ChartData = serde_json::from_value(cp[1].clone()).unwrap();
    diffs("chartPatch", &patch_json(&chart_patch(&el, &data, None)), &cp[2], &mut d);
    for (i, c) in g["chartChanged"].as_array().unwrap().iter().enumerate() {
        let el: ChartEl = serde_json::from_value(c[0].clone()).unwrap();
        let data: ChartData = serde_json::from_value(c[1].clone()).unwrap();
        diffs(&format!("chartChanged[{i}]"), &json!(chart_changed(&el, &data)), &c[2], &mut d);
    }
    assert_no_diffs("chart", &d);
}

fn order_array<T: ReadTxn>(txn: &T, root: &yrs::MapRef) -> Vec<String> {
    order_of(txn, root)
        .map(|o| {
            o.iter(txn)
                .map(|x| match x {
                    Out::Any(Any::String(s)) => s.to_string(),
                    other => panic!("order entry {other:?}"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The generator's `snap(label, extra)`. `merges` (from `tabJson`) are sorted formatted ranges.
fn model_snap<T: ReadTxn>(txn: &T, root: &yrs::MapRef, label: &str, extra: Option<(&str, Value)>) -> Value {
    let tabs: Vec<Value> = snap_tabs(txn, root)
        .iter()
        .map(|t| {
            let mut o = Map::new();
            o.insert("id".into(), json!(t.id));
            o.insert("rowIds".into(), json!(t.rows));
            o.insert("colIds".into(), json!(t.cols));
            o.insert("used".into(), serde_json::to_value(t.used_range()).unwrap());
            if let Value::Object(tj) = tab_to_json(t) {
                o.extend(tj);
            }
            Value::Object(o)
        })
        .collect();
    let mut v = json!({ "label": label, "ids": tab_ids(txn, root), "order": order_array(txn, root), "tabs": tabs });
    if let Some((k, x)) = extra {
        v[k] = x;
    }
    v
}

#[test]
fn model_helpers_with_the_generator_seeds() {
    let g = misc();
    let m = &g["model"];
    let (doc, mut rng) = sheet_doc(m["seed"].as_u64().unwrap() as u32, m["rows"].as_u64().unwrap() as usize, m["cols"].as_u64().unwrap() as usize, 1);
    let root = sheet_root(&doc);
    let mut steps: Vec<Value> = Vec::new();
    let tab = |txn: &yrs::TransactionMut| snap_tabs(txn, &root).remove(0);
    let snap = |steps: &mut Vec<Value>, label: &str, extra: Option<(&str, Value)>| {
        let txn = doc.transact();
        steps.push(model_snap(&txn, &root, label, extra));
    };
    let pf = |v: Value| v.as_object().unwrap().clone();
    snap(&mut steps, "init", None);
    {
        let mut txn = doc.transact_mut();
        {
            let t = tab(&txn);
            set_raw(&mut txn, &t, 1, 1, "x");
        }
        {
            let t = tab(&txn);
            set_raw(&mut txn, &t, 3, 2, "");
        }
        {
            let t = tab(&txn);
            set_raw(&mut txn, &t, 5, 3, "=1");
        }
        {
            let t = tab(&txn);
            set_raw(&mut txn, &t, 9, 9, "outside");
        }
    }
    snap(&mut steps, "setRaw", None);
    {
        let mut txn = doc.transact_mut();
        {
            let t = tab(&txn);
            patch_format(&mut txn, &t, 1, 1, Some(&pf(json!({ "b": true, "color": "#ff0000", "nf": "number" }))));
        }
        {
            let t = tab(&txn);
            patch_format(&mut txn, &t, 1, 1, Some(&pf(json!({ "b": false, "i": true, "color": "" }))));
        }
        {
            let t = tab(&txn);
            patch_format(&mut txn, &t, 2, 2, Some(&pf(json!({ "b": false }))));
        }
        {
            let t = tab(&txn);
            patch_format(&mut txn, &t, 0, 0, Some(&pf(json!({ "size": 0, "dp": 0, "cur": "" }))));
        }
    }
    snap(&mut steps, "patchFormat", None);
    let rg = |r0, c0, r1, c1| Some(Range { r0, c0, r1, c1 });
    {
        let mut txn = doc.transact_mut();
        {
            let t = tab(&txn);
            set_merge(&mut txn, &t, rg(0, 0, 1, 1), None);
        }
        {
            let t = tab(&txn);
            set_merge(&mut txn, &t, rg(4, 2, 9, 9), None);
        }
        {
            let t = tab(&txn);
            set_merge(&mut txn, &t, rg(3, 3, 3, 3), None);
        }
    }
    snap(&mut steps, "setMerge", None);
    {
        let mut txn = doc.transact_mut();
        {
            let t = tab(&txn);
            set_merge(&mut txn, &t, rg(1, 1, 2, 2), None);
        }
    }
    snap(&mut steps, "setMerge-overlap", None);
    {
        let mut txn = doc.transact_mut();
        {
            let t = tab(&txn);
            set_merge(&mut txn, &t, None, Some(CellAddr { r: 5, c: 3 }));
        }
    }
    snap(&mut steps, "unmerge", None);
    {
        let mut txn = doc.transact_mut();
        {
            let t = tab(&txn);
            insert_tracks(&mut txn, &t, Axis::Rows, 2, 2, &mut rng);
        }
        {
            let t = tab(&txn);
            insert_tracks(&mut txn, &t, Axis::Cols, 100, 1, &mut rng);
        }
    }
    snap(&mut steps, "insertTracks", None);
    {
        let mut txn = doc.transact_mut();
        {
            let t = tab(&txn);
            delete_tracks(&mut txn, &t, Axis::Rows, 1, 2);
        }
    }
    snap(&mut steps, "deleteTracks-rows", None);
    {
        let mut txn = doc.transact_mut();
        {
            let t = tab(&txn);
            delete_tracks(&mut txn, &t, Axis::Cols, 0, 100);
        }
    }
    snap(&mut steps, "deleteTracks-all-cols", None);
    let mut names = Vec::new();
    {
        let mut txn = doc.transact_mut();
        let first = tab_ids(&txn, &root)[0].clone();
        for (base, except) in [("Sheet1", None), ("sheet1", None), ("  a!b'c[d]*e?f/g\\h:i  ", None), ("!!!", None), ("Sheet1", Some(first.as_str()))] {
            names.push(unique_tab_name(&txn, &root, base, except));
        }
        add_tab(&mut txn, &root, None, None, &mut rng);
        add_tab(&mut txn, &root, Some("Sheet1"), None, &mut rng);
        add_tab(&mut txn, &root, Some("After first"), Some(&first), &mut rng);
        names.push(unique_tab_name(&txn, &root, "Sheet1", None));
    }
    snap(&mut steps, "addTab", Some(("names", json!(names))));
    {
        let mut txn = doc.transact_mut();
        let id = tab_ids(&txn, &root)[1].clone();
        delete_tab(&mut txn, &root, &id);
    }
    snap(&mut steps, "deleteTab", None);
    {
        let mut txn = doc.transact_mut();
        for id in tab_ids(&txn, &root)[1..].to_vec() {
            delete_tab(&mut txn, &root, &id);
        }
        let id = tab_ids(&txn, &root)[0].clone();
        delete_tab(&mut txn, &root, &id);
    }
    snap(&mut steps, "deleteTab-last", None);
    let mut d = Vec::new();
    diffs("model", &json!(steps), &m["steps"], &mut d);
    let mut rng = Mulberry32::new(601);
    let ids: Vec<String> = (0..5).map(|_| model::new_track_id(&mut rng)).collect();
    diffs("newTrackId", &json!(ids), &g["newTrackId"], &mut d);
    assert_no_diffs("model", &d);
}

// ------------------------------------------------------------------ layout, readRange, linked charts

/// The generator's layout sheet (seed 700, 200 × 5): 100 rows of items, styles, merges and sizes.
fn layout_sheet() -> (yrs::Doc, oc_sheet::engine::SheetEngine) {
    let (doc, rng) = sheet_doc(700, 200, 5, 1);
    let mut e = engine_with(&doc, 0.0, true, rng);
    let mut rows = vec![vec!["Item".to_string(), "Qty".into(), "Price".into()]];
    rows.extend((0..99).map(|i| vec![format!("Item {i}"), i.to_string(), format!("=B{}*2", i + 2)]));
    let pf = |v: Value| v.as_object().unwrap().clone();
    e.transact(|txn, tabs, rng| {
        let t = &tabs[0];
        write_rows(txn, t, 0, 0, &rows, rng);
        model::set_frozen(txn, t, Some(1.0), None);
        patch_format(txn, t, 0, 0, Some(&pf(json!({ "b": true, "fill": "#eeeeee" }))));
        patch_format(txn, t, 1, 2, Some(&pf(json!({ "nf": "currency", "color": "#ff0000", "i": true, "align": "center", "valign": "top", "size": 20 }))));
        patch_format(txn, t, 2, 1, Some(&pf(json!({ "align": "justify", "valign": "weird", "u": true }))));
        set_merge(txn, t, Some(Range { r0: 0, c0: 1, r1: 0, c1: 2 }), None);
        set_merge(txn, t, Some(Range { r0: 3, c0: 0, r1: 5, c1: 1 }), None);
        model::set_track_size(txn, t, Axis::Cols, 0, Some(300.0));
        model::set_track_size(txn, t, Axis::Rows, 4, Some(60.0));
    });
    (doc, e)
}

/// Replace every occurrence (keys and string values) of the ids in `map`.
fn rename_ids(v: &Value, map: &HashMap<String, String>) -> Value {
    match v {
        Value::String(s) => Value::String(map.get(s).cloned().unwrap_or_else(|| s.clone())),
        Value::Array(a) => Value::Array(a.iter().map(|x| rename_ids(x, map)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (map.get(k).cloned().unwrap_or_else(|| k.clone()), rename_ids(x, map))).collect()),
        other => other.clone(),
    }
}

/// Random page / element ids replaced by structural names: pages by sort order, elements by page and z.
fn normalize_design(v: &Value) -> Value {
    let mut map = HashMap::new();
    let mut pages: Vec<(&String, &str)> = v["pages"].as_object().unwrap().iter().map(|(id, p)| (id, p["order"].as_str().unwrap())).collect();
    pages.sort_by(|a, b| a.1.cmp(b.1));
    for (i, (id, _)) in pages.iter().enumerate() {
        map.insert((*id).clone(), format!("page{i}"));
    }
    for (id, e) in v["elements"].as_object().unwrap() {
        let parent = e["parentId"].as_str().unwrap();
        map.insert(id.clone(), format!("{}/{}", map.get(parent).cloned().unwrap_or_else(|| parent.to_string()), e["z"].as_str().unwrap()));
    }
    rename_ids(v, &map)
}

#[test]
fn sheet_layout() {
    let g = misc();
    let (doc, mut e) = layout_sheet();
    let mut d = Vec::new();
    for (i, c) in g["rangeToTable"].as_array().unwrap().iter().enumerate() {
        let range: Range = serde_json::from_value(c[0].clone()).unwrap();
        let o = &c[1];
        let opts = TableOptions {
            x: o["x"].as_f64(),
            y: o["y"].as_f64(),
            scale: o["scale"].as_f64(),
            family: o["family"].as_str().map(str::to_string),
            rows: o["rows"].as_array().map(|r| r.iter().map(|x| x.as_u64().unwrap() as usize).collect()),
        };
        let mut got = serde_json::to_value(oc_schema::Element::Table(range_to_table(&e, &e.tabs[0], &range, &opts))).unwrap();
        let mut want = c[2].clone();
        got["id"] = json!("table");
        want["id"] = json!("table");
        diffs(&format!("rangeToTable[{i}]"), &got, &want, &mut d);
    }
    let layouts = g["sheetToDesign"].as_array().unwrap();
    let layout = |e: &oc_sheet::engine::SheetEngine, o: &Value| -> Value {
        let opts = SheetLayoutOpts {
            title: o["title"].as_str().unwrap().to_string(),
            w: o["w"].as_f64().unwrap(),
            h: o["h"].as_f64().unwrap(),
            margin: o["margin"].as_f64(),
            max_pages: o["maxPages"].as_u64().map(|x| x as usize),
            tabs: o["tabs"].as_array().map(|t| t.iter().map(|x| x.as_str().unwrap().to_string()).collect()),
        };
        normalize_design(&serde_json::to_value(sheet_to_design(e, &opts)).unwrap())
    };
    for (i, c) in layouts[..2].iter().enumerate() {
        diffs(&format!("sheetToDesign[{i}]"), &layout(&e, &c[0]), &normalize_design(&c[1]), &mut d);
    }
    let root = sheet_root(&doc);
    e.transact(|txn, _, rng| {
        add_tab(txn, &root, Some("Empty"), None, rng);
        add_tab(txn, &root, Some("Other"), None, rng);
    });
    put(&mut e, 2, json!({ "A1": "solo", "B2": "=1/0" }).as_object().unwrap());
    for (i, c) in layouts.iter().enumerate().skip(2) {
        // The generator passes the "Other" tab by id.
        let mut o = c[0].clone();
        if o.get("tabs").is_some_and(|t| t[0] != "nope") {
            o["tabs"] = json!([e.tabs[2].id]);
        }
        diffs(&format!("sheetToDesign[{i}]"), &layout(&e, &o), &normalize_design(&c[1]), &mut d);
    }
    // readRange over the state the generator stored, and over the same sheet built here. The tabs added
    // after the layout calls get other ids here (TS drew the layout's element ids from the seeded generator,
    // Rust from the OS), so ids are mapped by tab position.
    let rr = &g["readRange"];
    let ts_ro = open_sheet_state(&unb64(&rr["state"]), host(0.0));
    let ro = open_sheet_state(&oc_crdt::codec::encode_state(&doc), host(0.0));
    diffs("readRange.tabs", &json!(ts_ro.tabs.iter().map(|t| &t.id).collect::<Vec<_>>()), &rr["tabs"], &mut d);
    diffs("readRange.state", &tabs_json(&ro), &tabs_json(&ts_ro), &mut d);
    let to_ts: HashMap<String, String> = ro.tabs.iter().zip(&ts_ro.tabs).map(|(a, b)| (a.id.clone(), b.id.clone())).collect();
    let from_ts: HashMap<&str, &str> = to_ts.iter().map(|(a, b)| (b.as_str(), a.as_str())).collect();
    for c in rr["cases"].as_array().unwrap() {
        let got = serde_json::to_value(read_range(&ts_ro, s(&c[0]), s(&c[1]))).unwrap();
        diffs(&format!("readRange[ts]({}, {})", c[0], c[1]), &got, &c[2], &mut d);
        let sheet = from_ts.get(s(&c[0])).copied().unwrap_or(s(&c[0]));
        let got = rename_ids(&serde_json::to_value(read_range(&ro, sheet, s(&c[1]))).unwrap(), &to_ts);
        diffs(&format!("readRange[rust]({}, {})", c[0], c[1]), &got, &c[2], &mut d);
    }
    assert_no_diffs("layout", &d);
}

#[test]
fn linked_charts_refresh_from_the_sheet_state() {
    let g = misc();
    let r = &g["resolveLinkedCharts"];
    let state = unb64(&r["state"]);
    let mut design: DesignDoc = serde_json::from_value(r["before"].clone()).unwrap();
    let n = resolve_linked_charts(&mut design, |id| (id == "sheet-1").then(|| state.clone()), || host(0.0));
    let mut d = Vec::new();
    diffs("n", &json!(n), &r["n"], &mut d);
    diffs("after", &serde_json::to_value(&design).unwrap(), &r["after"], &mut d);
    // Missing sources keep the cached values.
    let mut again: DesignDoc = serde_json::from_value(r["after"].clone()).unwrap();
    assert_eq!(resolve_linked_charts(&mut again, |_| None, || host(0.0)), 0);
    assert_no_diffs("resolveLinkedCharts", &d);
}
