// SPDX-License-Identifier: Apache-2.0
//! Whiteboard timer + votes: TS updates read by yrs, and the same calls made in Rust give the same reads.

mod common;

use common::{assert_same, load, ts_verify, unb64};
use oc_crdt::board::{
    Voter, cast_vote, clear_votes, read_timer_value, read_vote_session_value, read_votes, write_timer, write_vote_session,
};
use oc_crdt::{apply_update, encode_state, new_doc_with_client};
use serde_json::{Value, json};
use yrs::Doc;

fn votes(d: &Doc, s: &str) -> Value {
    let mut v = serde_json::to_value(read_votes(d, s)).unwrap();
    // yjs returns map order, yrs sorted keys: compare as sets.
    if let Value::Array(a) = &mut v {
        a.sort_by_key(oc_crdt::value::canonical_string);
    }
    v
}

fn sorted(v: &Value) -> Value {
    let mut v = v.clone();
    if let Value::Array(a) = &mut v {
        a.sort_by_key(oc_crdt::value::canonical_string);
    }
    v
}

fn reads(d: &Doc) -> Value {
    json!({
        "timer": read_timer_value(d),
        "vote": read_vote_session_value(d),
        "votesV1": votes(d, "v1"),
        "votesV2": votes(d, "v2"),
    })
}

fn expected(step: &Value) -> Value {
    json!({ "timer": step["timer"], "vote": step["vote"], "votesV1": sorted(&step["votesV1"]), "votesV2": sorted(&step["votesV2"]) })
}

#[test]
fn board_matches_ts() {
    let g = load("board.json");
    let mine = new_doc_with_client(41);
    apply_update(&mine, &unb64(&g["base"]), None).unwrap();
    for (i, s) in g["steps"].as_array().unwrap().iter().enumerate() {
        let args = s["args"].as_array().unwrap();
        let ret: Value = match s["fn"].as_str().unwrap() {
            "writeVoteSession" => {
                write_vote_session(&mine, serde_json::from_value(args[0].clone()).ok().as_ref());
                Value::Null
            }
            "writeTimer" => {
                write_timer(&mine, serde_json::from_value(args[0].clone()).ok().as_ref());
                Value::Null
            }
            "castVote" => {
                let voter: Voter = serde_json::from_value(args[1].clone()).unwrap();
                json!(cast_vote(&mine, args[0].as_str().unwrap(), &voter, args[2].as_str().unwrap(), args[3].as_f64().unwrap()))
            }
            "clearVotes" => {
                clear_votes(&mine, args.first().and_then(Value::as_str));
                Value::Null
            }
            f => panic!("unknown {f}"),
        };
        assert_same(&ret, &s["ret"], &format!("step {i} return"));
        assert_same(&reads(&mine), &expected(s), &format!("step {i} (Rust calls)"));
        // TS state after the same step, read by yrs.
        let ts = new_doc_with_client(42);
        apply_update(&ts, &unb64(&s["update"]), None).unwrap();
        assert_same(&reads(&ts), &expected(s), &format!("step {i} (TS update read by yrs)"));
    }
    // Rust state read by yjs.
    let last = g["steps"].as_array().unwrap().last().unwrap();
    let Some(r) = ts_verify(vec![(vec![encode_state(&mine)], vec![])]) else { return };
    assert_same(&r[0]["board"]["timer"], &last["timer"], "timer read by yjs");
    assert_same(&r[0]["board"]["vote"], &last["vote"], "vote read by yjs");
    let mut cells = serde_json::Map::new();
    for v in last["votesV1"].as_array().unwrap() {
        let mut cell = json!({ "n": v["n"] });
        if let Some(n) = v.get("name") {
            cell["name"] = n.clone();
        }
        cells.insert(format!("v1|{}|{}", v["voter"].as_str().unwrap(), v["element"].as_str().unwrap()), cell);
    }
    assert_same(&r[0]["board"]["votes"], &Value::Object(cells), "votes read by yjs");
}
