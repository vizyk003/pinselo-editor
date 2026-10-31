// SPDX-License-Identifier: AGPL-3.0-only
//! XLSX parity with `@oc/sheet/xlsx` (exceljs):
//!
//! - workbooks exported by TS read back (Rust `read_xlsx`) exactly as exceljs' `readXlsx` read them;
//! - Rust exports of the same sheets read back identically — by Rust and by exceljs itself (the generator's
//!   `--verify` mode, skipped when `OC_SKIP_TS_VERIFY` is set) — and carry the same cells, number formats,
//!   styles, merges, sizes and panes as the TS exports;
//! - TS and Rust exports import (`import_xlsx`) back to the cells of the source sheet.
//!
//! One deliberate deviation: text holding a literal `_xHHHH_` sequence is escaped by rust_xlsxwriter
//! (`_x005F_xHHHH_`, the OOXML rule) and reads back verbatim; exceljs writes it raw, so every reader —
//! exceljs included — decodes it to the character. Expectations for such cells are the source text.
//!
//! What the format itself loses, for both writers: the content of merge-covered cells (exceljs `mergeCells`),
//! control characters XML 1.0 cannot carry (exceljs `xmlEncode` drops them), HyperFormula's number kinds
//! (Excel keeps a number format instead; displays are preserved), and widths under one character (7 px).

mod common;

use std::collections::HashSet;

use common::*;
use oc_crdt::rng::Mulberry32;
use oc_sheet::a1::{a1, parse_range};
use oc_sheet::engine::SheetEngine;
use oc_sheet::model::{InitOpts, init_sheet, sheet_root};
use oc_sheet::xlsx::{XlsxOptions, import_xlsx, read_workbook, read_xlsx, sheet_to_xlsx, workbook_json};
use serde_json::{Map, Value, json};
use yrs::Transact;

fn golden() -> (f64, Vec<Value>) {
    let g = load("xlsx.json");
    (g["now"].as_f64().unwrap(), g["cases"].as_array().unwrap().clone())
}

fn read_json(bytes: &[u8]) -> Value {
    let m = read_xlsx(bytes).expect("readable workbook");
    Value::Object(m.into_iter().map(|(k, rows)| (k, Value::Array(rows.into_iter().map(Value::Array).collect()))).collect())
}

/// exceljs decodes `_xHHHH_` (upper-case hex) in shared strings.
fn has_x_escape(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(6)).any(|i| {
        b[i] == b'_' && b[i + 1] == b'x' && b[i + 6] == b'_' && b[i + 2..i + 6].iter().all(|c| c.is_ascii_digit() || (b'A'..=b'F').contains(c))
    })
}

/// Cells (tab name, A1) whose source value is plain text with an `_xHHHH_` look-alike.
fn x_escaped_cells(case: &Value) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for (tab, cells) in case["values"].as_object().unwrap() {
        for (a, c) in cells.as_object().unwrap() {
            if c.get("formula").is_none()
                && let Some(s) = c["value"].as_str()
                && has_x_escape(s)
            {
                out.push((tab.clone(), a.clone(), s.to_string()));
            }
        }
    }
    out
}

/// `read` of the TS export with the `_xHHHH_` cells as the source text (what a Rust export reads back as).
fn expected_read(case: &Value) -> Value {
    let mut read = case["read"].clone();
    for (tab, a, s) in x_escaped_cells(case) {
        let p = parse_range(&a).unwrap().range;
        read[&tab][p.r0][p.c0] = Value::String(s);
    }
    read
}

fn rust_export(case: &Value, now: f64) -> (yrs::Doc, SheetEngine, Vec<u8>) {
    let doc = doc_from(&[unb64(&case["update"])]);
    let e = engine(&doc, now, false);
    let bytes = sheet_to_xlsx(&e, &XlsxOptions { creator: None, created_ms: now }).expect("export");
    (doc, e, bytes)
}

#[test]
fn ts_exports_read_like_exceljs() {
    let (_, cases) = golden();
    let mut d = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        diffs(&format!("{name}.read"), &read_json(&unb64(&case["xlsx"])), &case["read"], &mut d);
    }
    assert_no_diffs("read_xlsx(TS export)", &d);
}

#[test]
fn rust_exports_read_back_like_ts_exports() {
    let (now, cases) = golden();
    let mut d = Vec::new();
    let mut exports = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let (_, _, bytes) = rust_export(case, now);
        diffs(&format!("{name}.read"), &read_json(&bytes), &expected_read(case), &mut d);
        // Structure: same cells, number formats, styles, merges, sizes and panes as the TS export.
        let mut ts = without_empty_cells(workbook_json(&read_workbook(&unb64(&case["xlsx"])).unwrap()));
        let rs = without_empty_cells(workbook_json(&read_workbook(&bytes).unwrap()));
        for (tab, a, s) in x_escaped_cells(case) {
            for sheet in ts.as_array_mut().unwrap().iter_mut().filter(|x| x["name"] == tab.as_str()) {
                for c in sheet["cells"].as_array_mut().unwrap().iter_mut().filter(|c| c["a1"] == a.as_str()) {
                    c["value"] = Value::String(s.clone());
                }
            }
        }
        diffs(&format!("{name}.structure"), &rs, &ts, &mut d);
        exports.push(bytes);
    }
    // The same files through exceljs.
    if let Some(out) = ts_verify(&exports, &[]) {
        assert_eq!(out["xlsx"].as_array().map(Vec::len), Some(cases.len()), "exceljs read every workbook");
        for (i, case) in cases.iter().enumerate() {
            let name = case["name"].as_str().unwrap();
            diffs(&format!("{name}.exceljs"), &out["xlsx"][i], &expected_read(case), &mut d);
        }
    }
    assert_no_diffs("Rust export", &d);
}

/// Drop `<c>` elements without value, formula or style (exceljs writes them for merge-covered cells,
/// rust_xlsxwriter omits them; they carry nothing).
fn without_empty_cells(mut wb: Value) -> Value {
    for sheet in wb.as_array_mut().unwrap() {
        sheet["cells"].as_array_mut().unwrap().retain(|c| {
            !(c["value"].is_null() && c["formula"].is_null() && c["numFmt"].is_null() && c["format"] == json!({}))
        });
    }
    wb
}

#[test]
fn rust_exports_are_deterministic() {
    let (now, cases) = golden();
    for case in &cases {
        let (_, _, a) = rust_export(case, now);
        let (_, _, b) = rust_export(case, now);
        assert!(a == b, "{}: two exports differ", case["name"]);
    }
}

/// Merge-covered cells of a tab JSON (`merges: ["A1:B2", …]`), anchors excluded.
fn covered(tab: &Value) -> HashSet<String> {
    let mut out = HashSet::new();
    for m in tab["merges"].as_array().unwrap() {
        let r = parse_range(m.as_str().unwrap()).unwrap().range;
        for row in r.r0..=r.r1 {
            for col in r.c0..=r.c1 {
                if (row, col) != (r.r0, r.c0) {
                    out.insert(a1(row, col));
                }
            }
        }
    }
    out
}

const STYLE_KEYS: [&str; 10] = ["b", "i", "u", "s", "color", "fill", "align", "valign", "wrap", "size"];

/// Import `bytes` into a fresh document and compare every imported tab with the source sheet: values,
/// displays and formulas of all cells (merge-covered cells excepted: exceljs drops their content on export),
/// the explicit style of every format (number formats are compared through the displayed text, since Excel
/// stores a format, not HyperFormula's typed numbers), merges, sizes (exceljs widths never go below one
/// character, 7 px) and frozen panes.
/// Text as an xlsx carries it: without the control characters exceljs drops, and — in files exceljs wrote —
/// with `_xHHHH_` decoded (exceljs writes such text unescaped).
fn as_stored(s: &str, ts_export: bool) -> String {
    let cleaned: String = s.chars().filter(|c| !(*c == '\u{7f}' || (*c <= '\u{1f}' && !matches!(c, '\t' | '\n' | '\r')))).collect();
    if !ts_export || !has_x_escape(&cleaned) {
        return cleaned;
    }
    let u: Vec<char> = cleaned.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < u.len() {
        if u[i] == '_' && i + 6 < u.len() && u[i + 1] == 'x' && u[i + 6] == '_' {
            let hex: String = u[i + 2..i + 6].iter().collect();
            if hex.chars().all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c))
                && let Some(ch) = char::from_u32(u32::from_str_radix(&hex, 16).unwrap())
            {
                out.push(ch);
                i += 7;
                continue;
            }
        }
        out.push(u[i]);
        i += 1;
    }
    out
}

fn check_import(label: &str, case: &Value, bytes: &[u8], now: f64, ts_export: bool, d: &mut Vec<String>) {
    let doc = oc_crdt::codec::new_doc_with_client(77);
    let mut rng = Mulberry32::new(9);
    init_sheet(&doc, InitOpts { name: Some("base".into()), ..InitOpts::default() }, &mut rng);
    let root = sheet_root(&doc);
    let ids = {
        let mut txn = doc.transact_mut();
        import_xlsx(&mut txn, &root, bytes, &mut rng).expect("import")
    };
    let e = engine(&doc, now, false);
    let values = eval_json(&e, &Value::Null);
    let src_tabs = case["tabs"].as_array().unwrap();
    assert_eq!(ids.len(), src_tabs.len(), "{label}: tab count");
    for (k, src) in src_tabs.iter().enumerate() {
        let name = src["name"].as_str().unwrap();
        let t = e.tab(&ids[k]).expect("imported tab");
        let got = oc_sheet::model::tab_to_json(t);
        let path = format!("{label}.{name}");
        if t.name != name {
            d.push(format!("{path}.name: actual {} expected {name}", t.name));
        }
        let skip = covered(src);
        let src_vals = case["values"][name].as_object().unwrap();
        let got_vals = values[&t.name].as_object().cloned().unwrap_or_default();
        for (a, sv) in src_vals {
            if skip.contains(a) {
                continue;
            }
            let gv = got_vals.get(a).cloned().unwrap_or_else(|| json!({ "value": null, "display": "" }));
            let mut sv = sv.clone();
            if sv.get("formula").is_none()
                && let Some(s) = sv["value"].as_str()
            {
                let stored = as_stored(s, ts_export);
                sv["value"] = Value::String(stored.clone());
                sv["display"] = Value::String(stored);
            }
            for key in ["value", "display", "formula"] {
                if !same(&gv.get(key).cloned().unwrap_or(Value::Null), &sv.get(key).cloned().unwrap_or(Value::Null)) {
                    d.push(format!("{path}.{a}.{key}: actual {} expected {}", gv.get(key).unwrap_or(&Value::Null), sv.get(key).unwrap_or(&Value::Null)));
                }
            }
        }
        for (a, f) in src["fmts"].as_object().unwrap() {
            let style = |v: &Value| -> Map<String, Value> { STYLE_KEYS.iter().filter_map(|k| v.get(*k).map(|x| ((*k).to_string(), x.clone()))).collect() };
            diffs(&format!("{path}.fmts.{a}"), &Value::Object(style(&got["fmts"][a])), &Value::Object(style(f)), d);
        }
        diffs(&format!("{path}.merges"), &got["merges"], &src["merges"], d);
        diffs(&format!("{path}.rowH"), &got["rowH"], &src["rowH"], d);
        let col_w: Map<String, Value> =
            src["colW"].as_object().unwrap().iter().map(|(c, w)| (c.clone(), json!(w.as_f64().unwrap().max(7.0)))).collect();
        diffs(&format!("{path}.colW"), &got["colW"], &Value::Object(col_w), d);
        diffs(&format!("{path}.frozen"), &json!([got["frozenRows"], got["frozenCols"]]), &json!([src["frozenRows"], src["frozenCols"]]), d);
    }
}

#[test]
fn ts_exports_import_to_the_source_sheet() {
    let (now, cases) = golden();
    let mut d = Vec::new();
    for case in &cases {
        check_import(case["name"].as_str().unwrap(), case, &unb64(&case["xlsx"]), now, true, &mut d);
    }
    assert_no_diffs("import(TS export)", &d);
}

#[test]
fn rust_exports_import_to_the_source_sheet() {
    let (now, cases) = golden();
    let mut d = Vec::new();
    for case in &cases {
        let (_, _, bytes) = rust_export(case, now);
        check_import(case["name"].as_str().unwrap(), case, &bytes, now, false, &mut d);
    }
    assert_no_diffs("import(Rust export)", &d);
}

/// calamine, an independent reader, sees the exported values and formulas.
#[test]
fn calamine_reads_rust_exports() {
    use calamine::{Data, Reader, Xlsx};
    let (now, cases) = golden();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let (_, e, bytes) = rust_export(case, now);
        let mut wb: Xlsx<_> = calamine::open_workbook_from_rs(std::io::Cursor::new(bytes)).expect("calamine opens the export");
        for t in &e.tabs {
            let range = match wb.worksheet_range(&t.name) {
                Ok(r) => r,
                // calamine rejects a whole sheet on error literals Excel does not know (HyperFormula's #CYCLE!,
                // #SPILL!, #ERROR!); exceljs and Excel read them as error cells.
                Err(err) if err.to_string().contains("error") || err.to_string().contains("Error") => continue,
                Err(err) => panic!("{name}/{}: {err}", t.name),
            };
            let formulas = wb.worksheet_formula(&t.name).expect("formulas");
            for (k, raw) in &t.cells {
                let Some(p) = t.pos_of(k) else { continue };
                let at = (p.r as u32, p.c as u32);
                if let Some(f) = raw.strip_prefix('=') {
                    if !t.fmts.get(k).is_some_and(|x| x.is_text()) {
                        let got = formulas.get_value(at).cloned().unwrap_or_default();
                        assert!(got == f || covered_in(t, p.r, p.c), "{name}/{}!{}: formula {got:?} vs {f:?}", t.name, a1(p.r, p.c));
                    }
                    continue;
                }
                if covered_in(t, p.r, p.c) {
                    continue;
                }
                let got = range.get_value(at).cloned().unwrap_or(Data::Empty);
                let ok = match e.value(&t.id, p.r, p.c) {
                    oc_sheet::format::CellValue::Number(n) => {
                        matches!(got, Data::Float(x) if x == n)
                            || matches!(got, Data::Int(x) if x as f64 == n)
                            || matches!(&got, Data::DateTime(x) if x.as_f64() == n)
                    }
                    oc_sheet::format::CellValue::String(s) => matches!(&got, Data::String(x) if *x == s.chars().filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r')).collect::<String>()),
                    oc_sheet::format::CellValue::Bool(b) => got == Data::Bool(b),
                    oc_sheet::format::CellValue::Error(_) => matches!(got, Data::Error(_)),
                    oc_sheet::format::CellValue::Null => got == Data::Empty,
                };
                assert!(ok, "{name}/{}!{}: calamine read {got:?} for {raw:?}", t.name, a1(p.r, p.c));
            }
        }
    }
}

fn covered_in(t: &oc_sheet::model::TabSnap, r: usize, c: usize) -> bool {
    t.merge_ranges().iter().any(|m| m.contains(r, c) && (r, c) != (m.r0, m.c0))
}
