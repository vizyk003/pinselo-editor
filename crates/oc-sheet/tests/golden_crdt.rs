// SPDX-License-Identifier: AGPL-3.0-only
//! CRDT parity: sheets built with yjs decode in yrs to the same tabs and values; concurrent edits merge and
//! re-evaluate the same way in live engines (remote row inserts replayed into the formulas); sheets built by
//! the Rust model with the generator's seeds get the same ids, and their updates decode in yjs (the
//! generator's `--verify` mode) to the same state as the yjs-built ones.

mod common;

use common::*;
use oc_crdt::codec::{apply_update, encode_diff, encode_state, encode_state_vector, new_doc_with_client};
use oc_crdt::rng::Mulberry32;
use oc_sheet::engine::SheetEngine;
use oc_sheet::model::{Axis, order_of, patch_format, set_frozen, set_merge, set_raw, set_track_size, sheet_root};
use serde_json::{Value, json};
use yrs::{Any, Array, Out, Transact};

fn case(name: &str) -> Value {
    load("crdt.json")["cases"].as_array().unwrap().iter().find(|c| c["name"] == name).cloned().unwrap_or_else(|| panic!("case {name}"))
}

fn updates(c: &Value) -> Vec<Vec<u8>> {
    c["updates"].as_array().unwrap().iter().map(unb64).collect()
}

fn order_ids(doc: &yrs::Doc) -> Vec<String> {
    let root = sheet_root(doc);
    let txn = doc.transact();
    order_of(&txn, &root)
        .map(|o| {
            o.iter(&txn)
                .map(|x| match x {
                    Out::Any(Any::String(s)) => s.to_string(),
                    other => panic!("order entry {other:?}"),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sync(a: &yrs::Doc, b: &yrs::Doc) {
    apply_update(b, &encode_diff(a, &encode_state_vector(b)).unwrap(), None).unwrap();
    apply_update(a, &encode_diff(b, &encode_state_vector(a)).unwrap(), None).unwrap();
}

/// `Y.encodeStateVectorFromUpdate(update)`.
fn state_vector_of(update: &[u8]) -> Vec<u8> {
    let d = new_doc_with_client(4242);
    apply_update(&d, update, None).unwrap();
    encode_state_vector(&d)
}

#[test]
fn yjs_built_sheets_decode_to_the_same_state() {
    let mut d = Vec::new();
    for name in ["full", "concurrent", "new-design"] {
        let c = case(name);
        let doc = doc_from(&updates(&c));
        let e = engine(&doc, 0.0, false);
        diffs(&format!("{name}.tabs"), &tabs_json(&e), &c["tabs"], &mut d);
        if name == "full" {
            diffs(&format!("{name}.values"), &eval_json(&e, &Value::Null), &c["values"], &mut d);
            let t = &e.tabs[0];
            let ids = json!({ "order": order_ids(&doc), "rows": t.rows[..3], "cols": t.cols[..3], "t2": e.tabs[1].id });
            diffs(&format!("{name}.ids"), &ids, &c["ids"], &mut d);
        }
    }
    assert_no_diffs("decode", &d);
}

/// The generator's merge, replayed into live engines: Alice's side sees Bob's row insert as a remote change
/// (her formulas shift inside the engine without their text changing), Bob's side sees Alice's cells.
#[test]
fn concurrent_updates_reevaluate_like_live_yjs_engines() {
    let c = case("concurrent");
    let u = updates(&c);
    let (base, upd_a, upd_b) = (&u[0], &u[1], &u[2]);
    let mut d = Vec::new();
    for (label, first, second, expected) in [("alice", upd_a, upd_b, &c["values"]), ("bob", upd_b, upd_a, &c["valuesB"])] {
        let doc = doc_from(std::slice::from_ref(base));
        let mut e = engine(&doc, 0.0, true);
        apply_update(&doc, first, None).unwrap();
        e.sync();
        apply_update(&doc, second, None).unwrap();
        e.sync();
        diffs(&format!("{label}.tabs"), &tabs_json(&e), &c["tabs"], &mut d);
        diffs(&format!("{label}.values"), &eval_json(&e, &Value::Null), expected, &mut d);
    }
    assert_no_diffs("concurrent", &d);
}

/// The generator's "full" sheet, built by the Rust model.
fn build_full() -> (yrs::Doc, SheetEngine, String) {
    let (doc, rng) = sheet_doc(400, 30, 8, 1);
    let mut e = engine_with(&doc, 0.0, true, rng);
    let root = e.root().clone();
    let t2 = e.transact(|txn, _, rng| oc_sheet::model::add_tab(txn, &root, Some("Second"), None, rng));
    let cells = |v: Value| v.as_object().unwrap().clone();
    put(&mut e, 0, &cells(json!({ "A1": "Item", "B1": "Qty", "C1": "Price", "A2": "Pen", "B2": "3", "C2": "1.5", "D2": "=B2*C2", "A3": "Ink", "B3": "2", "C3": "$4", "D3": "=B3*C3", "D4": "=SUM(D2:D3)", "E1": "=Second!A1" })));
    put(&mut e, 1, &cells(json!({ "A1": "42", "B1": "hello \"world\"", "C3": "=A1/7" })));
    put_fmt(&mut e, 0, &cells(json!({
        "A1": { "b": true, "fill": "#eeeeee", "color": "#112233", "size": 16 },
        "C2": { "nf": "currency", "dp": 2, "cur": "€" },
        "D4": { "nf": "number", "dp": 1, "align": "right", "valign": "middle", "wrap": true, "i": true, "u": true, "s": true },
    })));
    e.transact(|txn, tabs, _| {
        let t = &tabs[0];
        set_merge(txn, t, Some(oc_sheet::a1::parse_range("A5:C6").unwrap().range), None);
        set_track_size(txn, t, Axis::Cols, 0, Some(150.0));
        set_track_size(txn, t, Axis::Rows, 1, Some(30.0));
        set_frozen(txn, t, Some(1.0), Some(2.0));
    });
    let id = e.tabs[0].id.clone();
    e.insert(&id, Axis::Rows, 1, 1);
    (doc, e, t2)
}

/// The generator's concurrent edit, by the Rust model: (base, Alice's update, Bob's update, merged A, merged B).
fn build_concurrent() -> (Vec<u8>, Vec<u8>, Vec<u8>, SheetEngine, SheetEngine) {
    let (a, rng_a) = sheet_doc(401, 20, 5, 10);
    let b = new_doc_with_client(11);
    sync(&a, &b);
    let base = encode_state(&a);
    let mut ea = engine_with(&a, 0.0, true, rng_a);
    let mut eb = engine_with(&b, 0.0, true, Mulberry32::new(402));
    ea.transact(|txn, tabs, _| {
        set_raw(txn, &tabs[0], 0, 0, "alice");
        set_raw(txn, &tabs[0], 4, 1, "5");
        set_raw(txn, &tabs[0], 5, 1, "=B5*2");
    });
    let id = eb.tabs[0].id.clone();
    eb.insert(&id, Axis::Rows, 0, 2);
    eb.transact(|txn, tabs, _| {
        set_raw(txn, &tabs[0], 6, 1, "=1+1");
        set_raw(txn, &tabs[0], 9, 2, "bob");
        patch_format(txn, &tabs[0], 6, 1, json!({ "b": true }).as_object());
    });
    let sv = state_vector_of(&base);
    let upd_a = encode_diff(&a, &sv).unwrap();
    let upd_b = encode_diff(&b, &sv).unwrap();
    sync(&a, &b);
    ea.sync();
    eb.sync();
    (base, upd_a, upd_b, ea, eb)
}

#[test]
fn rust_built_sheets_match_the_yjs_built_ones() {
    let mut d = Vec::new();
    let full = case("full");
    let (doc, e, t2) = build_full();
    let t = &e.tabs[0];
    let ids = json!({ "order": order_ids(&doc), "rows": t.rows[..3], "cols": t.cols[..3], "t2": t2 });
    diffs("full.ids", &ids, &full["ids"], &mut d);
    diffs("full.tabs", &tabs_json(&e), &full["tabs"], &mut d);
    diffs("full.values", &eval_json(&e, &Value::Null), &full["values"], &mut d);

    let conc = case("concurrent");
    let (_, _, _, ea, eb) = build_concurrent();
    diffs("concurrent.tabs", &tabs_json(&ea), &conc["tabs"], &mut d);
    diffs("concurrent.tabsB", &tabs_json(&eb), &conc["tabs"], &mut d);
    diffs("concurrent.values", &eval_json(&ea, &Value::Null), &conc["values"], &mut d);
    diffs("concurrent.valuesB", &eval_json(&eb, &Value::Null), &conc["valuesB"], &mut d);
    assert_no_diffs("Rust-built", &d);
}

/// yjs decodes the Rust-written updates to exactly what it decodes the yjs-written ones to.
#[test]
fn yjs_decodes_rust_updates_like_its_own() {
    let (doc, _, _) = build_full();
    let (base, upd_a, upd_b, _, _) = build_concurrent();
    let nd = case("new-design");
    let design: oc_schema::DesignDoc = serde_json::from_value(nd["design"].clone()).expect("design");
    let rust_new = oc_sheet::state::encode_sheet_design(&design, &mut Mulberry32::new(403)).expect("encode");
    // The design half of the new state is the design itself.
    let decoded = oc_crdt::codec::decode_doc(&rust_new).expect("design roots");
    assert_eq!(serde_json::to_value(&decoded).unwrap(), serde_json::to_value(&design).unwrap(), "new-design roots");
    // (case, yjs-written updates, Rust-written updates)
    type Pair = (&'static str, Vec<Vec<u8>>, Vec<Vec<u8>>);
    let pairs: Vec<Pair> = vec![
        ("full", updates(&case("full")), vec![encode_state(&doc)]),
        ("concurrent", updates(&case("concurrent")), vec![base, upd_a, upd_b]),
        ("new-design", updates(&nd), vec![rust_new]),
    ];
    let all: Vec<Vec<Vec<u8>>> = pairs.iter().flat_map(|(_, ts, rs)| [ts.clone(), rs.clone()]).collect();
    let Some(out) = ts_verify(&[], &all) else { return };
    let docs = out["docs"].as_array().expect("docs");
    assert_eq!(docs.len(), all.len(), "yjs decoded every document");
    let mut d = Vec::new();
    for (i, (name, _, _)) in pairs.iter().enumerate() {
        diffs(name, &docs[2 * i + 1], &docs[2 * i], &mut d);
        // …and to what yrs decodes them to.
        let ts_doc = doc_from(&pairs[i].1);
        let e = engine(&ts_doc, 0.0, false);
        diffs(&format!("{name}.yrs"), &json!({ "tabs": tabs_json(&e), "values": eval_json(&e, &Value::Null) }), &docs[2 * i], &mut d);
    }
    assert_no_diffs("yjs decode", &d);
}
