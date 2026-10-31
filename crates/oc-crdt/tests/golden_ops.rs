// SPDX-License-Identifier: Apache-2.0
//! DocOp sequences replayed against the TS results: same result objects (errors, created ids — random ids and
//! jittered z keys come from the same mulberry32 stream), same snapshot after every step, and the final states
//! cross-decode (TS final update → yrs, Rust final update → yjs).

mod common;

use common::{assert_same, load, ts_verify, unb64};
use oc_crdt::rng::Mulberry32;
use oc_crdt::{ApplyOptions, apply_ops, apply_update, decode_doc_value, encode_state, from_y_value, new_doc_with_client};
use serde_json::Value;

fn run_case(c: &Value) -> (yrs::Doc, Vec<u8>) {
    let name = c["name"].as_str().unwrap();
    let doc = new_doc_with_client(1);
    apply_update(&doc, &unb64(&c["base"]), None).unwrap();
    let mut rng = Mulberry32::new(c["seed"].as_u64().unwrap() as u32);
    for (i, step) in c["steps"].as_array().unwrap().iter().enumerate() {
        let ops = oc_crdt::ops::parse_ops(&step["ops"]).unwrap_or_else(|e| panic!("{name}#{i}: parse ops: {e}"));
        let o = &step["opts"];
        let opts = ApplyOptions {
            actor: o["actor"].as_str().map(str::to_string),
            now: o["now"].as_f64(),
            partial: o["partial"].as_bool().unwrap_or(false),
            brand_admin: o["brandAdmin"].as_bool(),
            rng: Some(&mut rng),
        };
        let result = apply_ops(&doc, &ops, None, opts);
        assert_same(&serde_json::to_value(&result).unwrap(), &step["result"], &format!("{name}#{i} result"));
        assert_same(&from_y_value(&doc), &step["snapshot"], &format!("{name}#{i} snapshot"));
    }
    let update = encode_state(&doc);
    (doc, update)
}

#[test]
fn docop_sequences_match_ts() {
    let cases = load("ops.json");
    let cases = cases.as_array().unwrap();
    assert!(cases.len() >= 8);
    let mut finals = Vec::new();
    for c in cases {
        let (doc, update) = run_case(c);
        let name = c["name"].as_str().unwrap();
        // TS final state decodes in yrs to what Rust built.
        let ts_final = decode_doc_value(&unb64(&c["final"])).unwrap();
        assert_same(&ts_final, &from_y_value(&doc), &format!("{name}: TS final update decoded by yrs"));
        finals.push((name.to_string(), from_y_value(&doc), update));
    }
    let Some(results) = ts_verify(finals.iter().map(|(_, _, u)| (vec![u.clone()], vec![])).collect()) else { return };
    for ((name, snap, _), r) in finals.iter().zip(results) {
        assert_same(&r["design"], snap, &format!("{name}: Rust final update decoded by yjs"));
    }
}

#[test]
fn ops_serialize_back_to_the_ts_shape() {
    let cases = load("ops.json");
    for c in cases.as_array().unwrap() {
        for step in c["steps"].as_array().unwrap() {
            let ops = oc_crdt::ops::parse_ops(&step["ops"]).unwrap();
            assert_same(&serde_json::to_value(&ops).unwrap(), &step["ops"], c["name"].as_str().unwrap());
        }
    }
}
