// SPDX-License-Identifier: Apache-2.0
//! Detached text bodies (editBody) and the Docs body XmlFragment (y-prosemirror encoding).

mod common;

use common::{assert_same, load, ts_verify, unb64};
use oc_crdt::body::{BodyOp, edit_body_value};
use oc_crdt::doc_body::{DOC_FIELD, PmNode, embedded_design_ids_of_state, fragment_to_json, json_to_fragment, node_text};
use oc_crdt::{apply_update, encode_state, new_doc, new_doc_with_client};
use serde_json::Value;
use yrs::{Transact, WriteTxn};

#[test]
fn edit_body_matches_ts() {
    let g = load("body.json");
    for (i, c) in g["bodies"].as_array().unwrap().iter().enumerate() {
        let ops: Vec<BodyOp> = serde_json::from_value(c["ops"].clone()).unwrap();
        let out = edit_body_value(&c["body"], &ops);
        assert_same(&out, &c["out"], &format!("body case {i}"));
    }
}

fn doc_json(update: &[u8]) -> Value {
    let d = new_doc();
    apply_update(&d, update, None).unwrap();
    let mut txn = d.transact_mut();
    let frag = txn.get_or_insert_xml_fragment(DOC_FIELD);
    serde_json::to_value(fragment_to_json(&txn, &frag)).unwrap()
}

/// yrs sorts marks by type (yjs keeps insertion order); compare mark lists order-insensitively.
fn sort_marks(v: &mut Value) {
    match v {
        Value::Object(m) => {
            if let Some(Value::Array(marks)) = m.get_mut("marks") {
                marks.sort_by_key(|x| x["type"].as_str().unwrap_or("").to_string());
            }
            m.values_mut().for_each(sort_marks);
        }
        Value::Array(a) => a.iter_mut().for_each(sort_marks),
        _ => {}
    }
}

#[test]
fn doc_body_fragments_cross_decode() {
    let g = load("body.json");
    let mut rust_updates = Vec::new();
    for (i, c) in g["docBodies"].as_array().unwrap().iter().enumerate() {
        let what = format!("docBody case {i}");
        let mut expected = c["json"].clone();
        sort_marks(&mut expected);
        // TS update → yrs.
        let ts_update = unb64(&c["update"]);
        assert_same(&doc_json(&ts_update), &expected, &format!("{what}: yjs update read by yrs"));
        assert_eq!(serde_json::to_value(embedded_design_ids_of_state(&ts_update).unwrap()).unwrap(), c["embedded"], "{what}: embedded ids");
        // Rust jsonToFragment → yrs read-back + yjs read-back.
        let pm: PmNode = serde_json::from_value(c["pm"].clone()).unwrap();
        let d = new_doc_with_client(70 + i as u64);
        {
            let mut txn = d.transact_mut();
            let frag = txn.get_or_insert_xml_fragment(DOC_FIELD);
            json_to_fragment(&mut txn, &frag, &pm);
        }
        let update = encode_state(&d);
        assert_same(&doc_json(&update), &expected, &format!("{what}: Rust fragment read by yrs"));
        let back: PmNode = serde_json::from_value(expected.clone()).unwrap();
        let texts: Vec<String> = back.content.iter().flatten().map(node_text).collect();
        assert_eq!(serde_json::to_value(texts).unwrap(), c["text"], "{what}: nodeText");
        rust_updates.push((vec![update], vec![]));
    }
    let Some(results) = ts_verify(rust_updates) else { return };
    for (i, (r, c)) in results.iter().zip(g["docBodies"].as_array().unwrap()).enumerate() {
        let mut a = r["docBody"].clone();
        sort_marks(&mut a);
        let mut e = c["json"].clone();
        sort_marks(&mut e);
        assert_same(&a, &e, &format!("docBody case {i}: Rust update read by yjs"));
    }
}
