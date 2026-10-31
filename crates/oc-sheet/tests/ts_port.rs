// SPDX-License-Identifier: AGPL-3.0-only
//! Port of `packages/sheet/src/sheet.test.ts`, test for test (same names, same assertions).

mod common;

use common::{cell, host};
use oc_crdt::codec::{apply_update, encode_diff, encode_state, encode_state_vector, new_doc};
use oc_crdt::rng::OsRandom;
use oc_schema::elements::{ChartEl, ChartSource};
use oc_schema::factory::{ChartOpts, MetaInit, SeriesInit, base_init, empty_design, make_chart, place_all_elements};
use oc_schema::tree::sorted_pages;
use oc_schema::{Element, Extra, Surface, Xf};
use oc_sheet::a1::{Range, parse_range, shift_formula};
use oc_sheet::chart::{ChartData, SeriesData, chart_patch, range_to_chart_data};
use oc_sheet::csv::{parse_delimited, tab_to_csv, to_delimited, write_rows};
use oc_sheet::engine::{EngineOpts, SheetEngine};
use oc_sheet::format::{CellValue, ValueKind, format_value};
use oc_sheet::linked::resolve_linked_charts;
use oc_sheet::model::{
    Axis, InitOpts, add_tab, find_tab, init_sheet_default, patch_format, set_frozen, set_merge, set_raw, sheet_root, snap_doc, snap_tabs,
};
use oc_sheet::state::encode_sheet_design;
use oc_sheet::table::{SheetLayoutOpts, sheet_to_design};
use oc_sheet::xlsx::{XlsxOptions, read_xlsx, sheet_to_xlsx};
use serde_json::{Value, json};
use yrs::Transact;

/// 2024-05-17 13:45:30 UTC.
const NOW: f64 = 1_715_953_530_000.0;

struct Sheet {
    doc: yrs::Doc,
    engine: SheetEngine,
}

impl Sheet {
    /// `make(rows, cols)`: a doc with one tab and a live engine.
    fn new(rows: usize, cols: usize) -> Sheet {
        let doc = new_doc();
        init_sheet_default(&doc, InitOpts { rows: Some(rows), cols: Some(cols), ..InitOpts::default() });
        let engine = SheetEngine::new(&doc, EngineOpts { host: host(NOW), ..EngineOpts::default() });
        Sheet { doc, engine }
    }
    fn tab_id(&self, i: usize) -> String {
        self.engine.tabs[i].id.clone()
    }
    fn put_in(&mut self, tab: usize, cells: &[(&str, &str)]) {
        self.engine.transact(|txn, tabs, _| {
            for (a, v) in cells {
                let p = cell(a);
                set_raw(txn, &tabs[tab], p.r, p.c, v);
            }
        });
    }
    fn put(&mut self, cells: &[(&str, &str)]) {
        self.put_in(0, cells);
    }
    fn val_in(&self, tab: usize, a: &str) -> CellValue {
        let p = cell(a);
        self.engine.value(&self.engine.tabs[tab].id, p.r, p.c)
    }
    fn val(&self, a: &str) -> CellValue {
        self.val_in(0, a)
    }
    fn raw(&self, tab: usize, r: usize, c: usize) -> String {
        self.engine.tabs[tab].raw_at(r, c).to_string()
    }
}

fn n(v: f64) -> CellValue {
    CellValue::Number(v)
}

fn st(v: &str) -> CellValue {
    CellValue::String(v.into())
}

fn err(v: &str) -> CellValue {
    CellValue::Error(v.into())
}

// ------------------------------------------------------------------ formula engine

#[test]
fn evaluates_common_functions() {
    let mut s = Sheet::new(50, 10);
    s.put(&[
        ("A1", "10"),
        ("A2", "20"),
        ("A3", "30"),
        ("A4", "x"),
        ("B1", "=SUM(A1:A3)"),
        ("B2", "=AVERAGE(A1:A3)"),
        ("B3", "=MIN(A1:A3)"),
        ("B4", "=MAX(A1:A3)"),
        ("B5", "=COUNT(A1:A4)"),
        ("B6", "=IF(A1>5,\"big\",\"small\")"),
        ("B7", "=ROUND(2/3,2)"),
        ("B8", "=CONCAT(\"a\",A1,\"b\")"),
        ("B9", "=CONCATENATE(A4,\"!\")"),
        ("C1", "apple"),
        ("D1", "1.5"),
        ("C2", "pear"),
        ("D2", "2.5"),
        ("B10", "=VLOOKUP(\"pear\",C1:D2,2,FALSE)"),
        ("B11", "=XLOOKUP(\"apple\",C1:C2,D1:D2)"),
        ("B12", "=TODAY()"),
        ("B13", "=IFERROR(1/0,\"oops\")"),
    ]);
    assert_eq!([s.val("B1"), s.val("B2"), s.val("B3"), s.val("B4"), s.val("B5")], [n(60.0), n(20.0), n(10.0), n(30.0), n(3.0)]);
    assert_eq!(s.val("B6"), st("big"));
    assert_eq!(s.val("B7"), n(0.67));
    assert_eq!(s.val("B8"), st("a10b"));
    assert_eq!(s.val("B9"), st("x!"));
    assert_eq!(s.val("B10"), n(2.5));
    assert_eq!(s.val("B11"), n(1.5));
    assert_eq!(s.engine.kind(&s.tab_id(0), 11, 1), ValueKind::Date);
    let today = format_value(&s.val("B12"), None, ValueKind::Date);
    assert!(today.len() == 10 && today.as_bytes()[4] == b'-' && today.as_bytes()[7] == b'-' && today.replace('-', "").bytes().all(|b| b.is_ascii_digit()), "{today}");
    assert_eq!(s.val("B13"), st("oops"));
}

#[test]
fn reports_errors() {
    let mut s = Sheet::new(50, 10);
    s.put(&[("A1", "=1/0"), ("A2", "=NOSUCH(1)"), ("A3", "=\"a\"+1"), ("A4", "=A4")]);
    assert_eq!(s.val("A1"), err("#DIV/0!"));
    assert_eq!(s.val("A2"), err("#NAME?"));
    assert_eq!(s.val("A3"), err("#VALUE!"));
    assert_eq!(s.val("A4"), err("#CYCLE!"));
}

#[test]
fn resolves_cross_tab_references_and_keeps_them_through_renames() {
    let mut s = Sheet::new(50, 10);
    let root = sheet_root(&s.doc);
    let t2 = s.engine.transact(|txn, _, rng| add_tab(txn, &root, Some("Data sheet"), None, rng));
    let data = s.engine.tabs.iter().position(|t| t.id == t2).unwrap();
    s.put_in(data, &[("A1", "7")]);
    s.put(&[("A1", "='Data sheet'!A1*2")]);
    assert_eq!(s.val("A1"), n(14.0));
    let name = s.engine.rename_tab(&t2, "Inputs");
    assert_eq!(name, "Inputs");
    assert_eq!(s.raw(0, 0, 0), "=Inputs!A1*2");
    assert_eq!(s.val("A1"), n(14.0));
    s.engine.delete_tab(&t2);
    assert_eq!(s.val("A1"), err("#REF!"));
}

#[test]
fn shifts_references_on_row_column_insert_and_delete_and_writes_the_rewritten_formulas_back() {
    let mut s = Sheet::new(50, 10);
    s.put(&[("A1", "1"), ("A2", "2"), ("A3", "3"), ("B1", "=SUM(A1:A3)"), ("C1", "=A3")]);
    let id = s.tab_id(0);
    s.engine.insert(&id, Axis::Rows, 1, 2); // above A2
    assert_eq!(s.raw(0, 0, 1), "=SUM(A1:A5)");
    assert_eq!(s.raw(0, 0, 2), "=A5");
    assert_eq!(s.val("B1"), n(6.0));
    s.engine.remove(&id, Axis::Rows, 4, 1); // old A3
    assert_eq!(s.raw(0, 0, 2), "=#REF!");
    assert_eq!(s.val("B1"), n(3.0));
    s.engine.insert(&id, Axis::Cols, 0, 1);
    assert_eq!(s.raw(0, 0, 2), "=SUM(B1:B4)");
    assert_eq!(s.val("C1"), n(3.0));
}

#[test]
fn treats_text_formatted_cells_as_strings() {
    let mut s = Sheet::new(50, 10);
    s.put(&[("A1", "007")]);
    assert_eq!(s.val("A1"), n(7.0));
    s.engine.transact(|txn, tabs, _| patch_format(txn, &tabs[0], 0, 0, json!({ "nf": "text" }).as_object()));
    assert_eq!(s.val("A1"), st("007"));
}

// ------------------------------------------------------------------ CRDT merge

fn sync(a: &yrs::Doc, b: &yrs::Doc) {
    apply_update(b, &encode_diff(a, &encode_state_vector(b)).unwrap(), None).unwrap();
    apply_update(a, &encode_diff(b, &encode_state_vector(a)).unwrap(), None).unwrap();
}

#[test]
fn merges_concurrent_edits_cell_by_cell_even_across_a_concurrent_row_insert() {
    let a = new_doc();
    init_sheet_default(&a, InitOpts { rows: Some(20), cols: Some(5), ..InitOpts::default() });
    let b = new_doc();
    sync(&a, &b);
    let mut ea = SheetEngine::new(&a, EngineOpts { host: host(NOW), ..EngineOpts::default() });
    let mut eb = SheetEngine::new(&b, EngineOpts { host: host(NOW), ..EngineOpts::default() });
    ea.transact(|txn, tabs, _| {
        set_raw(txn, &tabs[0], 0, 0, "alice");
        set_raw(txn, &tabs[0], 4, 1, "5");
    });
    // Bob, offline: inserts two rows at the top and edits the cell that used to be row 5 (now row 7).
    let id = eb.tabs[0].id.clone();
    eb.insert(&id, Axis::Rows, 0, 2);
    eb.transact(|txn, tabs, _| {
        set_raw(txn, &tabs[0], 6, 1, "=1+1");
        set_raw(txn, &tabs[0], 9, 2, "bob");
    });
    sync(&a, &b);
    ea.sync();
    eb.sync();
    for e in [&ea, &eb] {
        let t = &e.tabs[0];
        assert_eq!(t.rows.len(), 22);
        assert_eq!(t.raw_at(2, 0), "alice"); // Alice's A1 moved down with its row
        assert!(["5", "=1+1"].contains(&t.raw_at(6, 1))); // same cell as Alice's "5": one deterministic winner
        assert_eq!(t.raw_at(9, 2), "bob");
    }
    assert_eq!(ea.tabs[0].raw_at(6, 1), eb.tabs[0].raw_at(6, 1));
    let va = ea.value(&ea.tabs[0].id, 6, 1);
    assert_eq!(va, eb.value(&eb.tabs[0].id, 6, 1));
    assert!(va == n(2.0) || va == n(5.0), "{va:?}");
}

#[test]
fn converges_on_concurrent_writes_to_the_same_cell_and_keeps_formats_independent_of_values() {
    let a = new_doc();
    init_sheet_default(&a, InitOpts { rows: Some(5), cols: Some(5), ..InitOpts::default() });
    let b = new_doc();
    sync(&a, &b);
    {
        let root = sheet_root(&a);
        let mut txn = a.transact_mut();
        let ta = snap_tabs(&txn, &root).remove(0);
        set_raw(&mut txn, &ta, 1, 1, "from a");
    }
    {
        let root = sheet_root(&b);
        let mut txn = b.transact_mut();
        let tb = snap_tabs(&txn, &root).remove(0);
        set_raw(&mut txn, &tb, 1, 1, "from b");
        patch_format(&mut txn, &tb, 1, 1, json!({ "b": true }).as_object());
    }
    sync(&a, &b);
    assert_eq!(snap_doc(&a)[0].raw_at(1, 1), snap_doc(&b)[0].raw_at(1, 1));
    assert_eq!(snap_doc(&a)[0].fmts.len(), 1);
}

#[test]
fn a_new_design_state_carries_exactly_one_tab() {
    let design = empty_design(&MetaInit { w: 100.0, h: 100.0, surface: Some(Surface::Sheet), ..MetaInit::default() });
    let state = encode_sheet_design(&design, &mut OsRandom).unwrap();
    let d = new_doc();
    apply_update(&d, &state, None).unwrap();
    assert_eq!(snap_doc(&d).len(), 1);
    assert_eq!(snap_doc(&d)[0].rows.len(), 1000);
}

// ------------------------------------------------------------------ CSV

fn strs(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter().map(|r| r.iter().map(|x| x.to_string()).collect()).collect()
}

#[test]
fn round_trips_quoting_embedded_delimiters_and_newlines() {
    let rows = strs(&[&["name", "note", "n"], &["Smith, J", "said \"hi\"", "3"], &["multi", "line\none", ""], &["", "", "-1.5"]]);
    let csv = to_delimited(&rows, ',', "\r\n");
    assert_eq!(parse_delimited(&csv, ','), rows);
    let tsv = to_delimited(&rows, '\t', "\n");
    assert_eq!(parse_delimited(&tsv, '\t'), rows);
}

#[test]
fn imports_into_a_tab_and_exports_the_same_csv() {
    let mut s = Sheet::new(3, 2);
    let csv = "city,pop\r\n\"Paris, FR\",2100000\r\nRome,2800000\r\nTotal,=SUM(B2:B3)";
    s.engine.transact(|txn, tabs, rng| write_rows(txn, &tabs[0], 0, 0, &parse_delimited(csv, ','), rng));
    assert_eq!(s.engine.tabs[0].rows.len(), 4);
    assert_eq!(tab_to_csv(&s.engine, &s.engine.tabs[0]), "city,pop\r\n\"Paris, FR\",2100000\r\nRome,2800000\r\nTotal,4900000");
}

// ------------------------------------------------------------------ references

#[test]
fn shifts_relative_references_only() {
    assert_eq!(shift_formula("=A1+$B$2+C$3+$D4", 1, 1), "=B2+$B$2+D$3+$D5");
    assert_eq!(shift_formula("=SUM(A1:B2)&\"A1\"", 2, 0), "=SUM(A3:B4)&\"A1\"");
    assert_eq!(shift_formula("='Q1 data'!A1+Sheet2!B1+LOG10(A1)", 1, 0), "='Q1 data'!A2+Sheet2!B2+LOG10(A2)");
    assert_eq!(shift_formula("=A1", -1, 0), "=#REF!");
    let p = parse_range("'My tab'!B2:A1").unwrap();
    assert_eq!(p.sheet.as_deref(), Some("My tab"));
    assert_eq!(p.range, Range { r0: 0, c0: 0, r1: 1, c1: 1 });
}

// ------------------------------------------------------------------ linked charts

fn data(labels: &[&str], series: &[(&str, &[f64])]) -> ChartData {
    ChartData {
        labels: labels.iter().map(|x| x.to_string()).collect(),
        series: series.iter().map(|(n, v)| SeriesData { name: n.to_string(), values: v.to_vec() }).collect(),
    }
}

#[test]
fn maps_a_range_with_a_header_row_and_label_column() {
    let d = range_to_chart_data(&[
        vec![st("Quarter"), st("Revenue"), st("Costs")],
        vec![st("Q1"), n(10.0), n(4.0)],
        vec![st("Q2"), n(12.0), st("5")],
        vec![st("Q3"), CellValue::Null, n(6.0)],
    ]);
    assert_eq!(d, data(&["Q1", "Q2", "Q3"], &[("Revenue", &[10.0, 12.0, 0.0]), ("Costs", &[4.0, 5.0, 6.0])]));
}

#[test]
fn maps_bare_numbers_with_generated_labels_and_names() {
    let d = range_to_chart_data(&[vec![n(1.0), n(2.0)], vec![n(3.0), n(4.0)]]);
    assert_eq!(d, data(&["1", "2"], &[("Series 1", &[1.0, 3.0]), ("Series 2", &[2.0, 4.0])]));
}

fn chart(series: Option<Vec<SeriesInit>>) -> ChartEl {
    make_chart(base_init(Xf::new(0.0, 0.0, 100.0, 100.0, 0.0)), ChartOpts { series, ..ChartOpts::default() })
}

#[test]
fn keeps_series_colours_on_refresh() {
    let red = oc_schema::color::hex("#ff0000").unwrap();
    let el = chart(Some(vec![SeriesInit { name: "A".into(), values: vec![1.0], color: Some(red) }]));
    let p = chart_patch(&el, &data(&["x", "y"], &[("B", &[1.0, 2.0]), ("C", &[3.0, 4.0])]), None);
    assert_eq!(p.series[0].color, el.series[0].color);
    assert!(serde_json::to_value(&p.series[1].color).unwrap().is_object());
}

#[test]
fn refreshes_linked_charts_in_a_design_from_the_sheet_state() {
    let mut s = Sheet::new(10, 4);
    s.put(&[("A1", "Month"), ("B1", "Sales"), ("A2", "Jan"), ("B2", "5"), ("A3", "Feb"), ("B3", "=B2*3")]);
    let state = encode_state(&s.doc);
    let mut design = empty_design(&MetaInit { w: 800.0, h: 600.0, ..MetaInit::default() });
    let page = sorted_pages(&design.pages)[0].id.to_string();
    let mut el = chart(None);
    el.source = Some(ChartSource { design_id: "sheet-1".into(), sheet: s.tab_id(0), range: "A1:B3".into(), sheet_name: None, refreshed_at: None, extra: Extra::new() });
    let mut els = vec![Element::Chart(el)];
    place_all_elements(&page, &mut els, None);
    let placed = els.remove(0);
    let id = placed.id().unwrap().to_string();
    design.elements.insert(id.clone(), placed);
    let n_refreshed = resolve_linked_charts(&mut design, |d| (d == "sheet-1").then(|| state.clone()), || host(NOW));
    assert_eq!(n_refreshed, 1);
    let Some(Element::Chart(got)) = design.elements.get(id.as_str()) else { panic!("chart") };
    assert_eq!(got.labels, ["Jan", "Feb"]);
    assert_eq!(got.series.len(), 1);
    assert_eq!(got.series[0].name, "Sales");
    assert_eq!(got.series[0].values, [5.0, 15.0]);
    assert_eq!(got.source.as_ref().unwrap().range, "A1:B3");
    // Missing source: cached values stay.
    if let Some(Element::Chart(c)) = design.elements.get_mut(id.as_str()) {
        c.source = Some(ChartSource { design_id: "gone".into(), sheet: "x".into(), range: "A1:B3".into(), sheet_name: None, refreshed_at: None, extra: Extra::new() });
    }
    assert_eq!(resolve_linked_charts(&mut design, |_| None, || host(NOW)), 0);
}

// ------------------------------------------------------------------ layout and export

#[test]
fn paginates_a_sheet_into_table_pages_with_repeated_frozen_rows() {
    let mut s = Sheet::new(200, 3);
    let mut rows = vec![vec!["Item".to_string(), "Qty".into(), "Price".into()]];
    rows.extend((0..99).map(|i| vec![format!("Item {i}"), i.to_string(), format!("=B{}*2", i + 2)]));
    s.engine.transact(|txn, tabs, rng| {
        write_rows(txn, &tabs[0], 0, 0, &rows, rng);
        set_frozen(txn, &tabs[0], Some(1.0), None);
        patch_format(txn, &tabs[0], 0, 0, json!({ "b": true, "fill": "#eeeeee" }).as_object());
        set_merge(txn, &tabs[0], Some(Range { r0: 0, c0: 1, r1: 0, c1: 2 }), None);
    });
    let d = sheet_to_design(&s.engine, &SheetLayoutOpts { title: "Stock".into(), w: 1123.0, h: 794.0, ..SheetLayoutOpts::default() });
    let pages: Vec<String> = sorted_pages(&d.pages).iter().map(|p| p.id.to_string()).collect();
    assert!(pages.len() > 2);
    let tables: Vec<Value> = d
        .elements
        .values()
        .filter(|e| matches!(e, Element::Table(_)))
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();
    assert_eq!(tables.len(), pages.len());
    let first = tables.iter().find(|t| t["parentId"] == pages[0].as_str()).unwrap();
    assert_eq!(first["cells"]["0:0"]["text"], "Item");
    assert_eq!(first["cells"]["0:0"]["bold"], true);
    let span = &first["cells"]["0:1"]["span"];
    assert_eq!((span["rows"].as_f64(), span["cols"].as_f64()), (Some(1.0), Some(2.0)));
    assert_eq!(first["cells"]["1:2"]["text"], "0");
    let second = tables.iter().find(|t| t["parentId"] == pages[1].as_str()).unwrap();
    assert_eq!(second["cells"]["0:0"]["text"], "Item");
}

#[test]
fn writes_xlsx_with_formulas_cached_results_and_merges() {
    let mut s = Sheet::new(5, 3);
    s.put(&[("A1", "2"), ("A2", "3"), ("A3", "=SUM(A1:A2)"), ("B1", "label")]);
    s.engine.transact(|txn, tabs, _| {
        patch_format(txn, &tabs[0], 0, 1, json!({ "b": true, "nf": "text" }).as_object());
        set_merge(txn, &tabs[0], Some(Range { r0: 0, c0: 1, r1: 1, c1: 2 }), None);
    });
    let bytes = sheet_to_xlsx(&s.engine, &XlsxOptions { creator: None, created_ms: NOW }).unwrap();
    assert!(bytes.len() > 1000);
    let back = read_xlsx(&bytes).unwrap();
    let rows = &back["Sheet1"];
    assert_eq!(rows[0][0], json!(2));
    assert_eq!(rows[2][0]["formula"], "SUM(A1:A2)");
    assert_eq!(rows[2][0]["result"], json!(5));
    assert_eq!(rows[0][1], "label");
}

#[test]
fn finds_tabs_by_id_or_name() {
    let s = Sheet::new(2, 2);
    assert_eq!(find_tab(&s.engine.tabs, "sheet1").map(|t| t.id.clone()), Some(s.tab_id(0)));
}
