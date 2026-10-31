// SPDX-License-Identifier: Apache-2.0
//! Every template fixture: yjs update (TS codec) → yrs → identical snapshot; Rust codec → yrs update → yjs decodes
//! the identical snapshot.

mod common;

use common::{assert_same, load, ts_verify, unb64};
use oc_crdt::{decode_doc, decode_doc_value, encode_doc_value};
use serde_json::Value;

fn fixtures() -> Vec<(String, Value)> {
    let dir = common::golden_dir().join("templates");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("templates goldens")
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    names.into_iter().map(|n| (n.clone(), load(&format!("templates/{n}")))).collect()
}

fn fixture_json(name: &str) -> Value {
    let p = common::root().join("fixtures/templates-seed").join(name);
    serde_json::from_str(&std::fs::read_to_string(p).expect("fixture")).expect("fixture json")
}

#[test]
fn yjs_updates_decode_to_the_same_snapshot() {
    let fx = fixtures();
    assert!(fx.len() >= 100, "expected the template goldens, found {}", fx.len());
    for (name, g) in &fx {
        let snap = decode_doc_value(&unb64(&g["update"])).expect("decode");
        assert_same(&snap, &g["snapshot"], name);
        // The typed model parses what yrs reads back.
        decode_doc(&unb64(&g["update"])).unwrap_or_else(|e| panic!("{name}: typed decode: {e}"));
    }
}

#[test]
fn rust_encoded_docs_round_trip_in_rust_and_in_yjs() {
    let fx = fixtures();
    let mut updates = Vec::new();
    for (name, g) in &fx {
        let update = encode_doc_value(&fixture_json(name));
        let back = decode_doc_value(&update).expect("decode own update");
        assert_same(&back, &g["snapshot"], name);
        updates.push((vec![update], vec![]));
    }
    let Some(results) = ts_verify(updates) else { return };
    for ((name, g), r) in fx.iter().zip(results) {
        assert_same(&r["design"], &g["snapshot"], &format!("yjs decode of the Rust update for {name}"));
    }
}
