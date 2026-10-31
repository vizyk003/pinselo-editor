// SPDX-License-Identifier: Apache-2.0
//! Cross-language convergence: two yjs peers (TS, recorded in converge.json) and two yrs peers edit the same
//! base concurrently; every merge order converges, and yjs reading the merged Rust state agrees.

mod common;

use common::{assert_same, load, ts_verify, unb64};
use oc_crdt::board::{Voter, cast_vote, read_timer_value, read_votes};
use oc_crdt::rng::Mulberry32;
use oc_crdt::text::plain_text;
use oc_crdt::{
    ApplyOptions, DocOp, apply_ops, apply_update, codec, encode_diff, encode_state, encode_state_vector, from_y_value,
    new_doc_with_client,
};
use serde_json::{Value, json};
use yrs::{Doc, Transact};

fn ops(v: Value) -> Vec<DocOp> {
    serde_json::from_value(v).unwrap()
}

fn apply(d: &Doc, rng: &mut Mulberry32, v: Value) {
    let r = apply_ops(d, &ops(v), None, ApplyOptions { now: Some(1_700_000_000_000.0), rng: Some(rng), ..Default::default() });
    assert!(r.ok, "{r:?}");
}

fn text_of(d: &Doc, id: &str) -> String {
    let r = codec::roots(d);
    let txn = d.transact();
    let m = codec::element_map(&txn, &r, id).unwrap();
    plain_text(&txn, &codec::text_of(&txn, &m).unwrap())
}

#[test]
fn ts_peers_merge_in_rust() {
    let g = load("converge.json");
    let d = new_doc_with_client(100);
    for k in ["base", "a", "b"] {
        apply_update(&d, &unb64(&g[k]), None).unwrap();
    }
    assert_same(&from_y_value(&d), &g["merged"]["design"], "TS peers merged by yrs");
    assert_eq!(json!(text_of(&d, g["textId"].as_str().unwrap())), g["merged"]["plain"]);
    assert_same(&json!(read_timer_value(&d)), &g["merged"]["timer"], "timer");
    assert_same(&json!(read_votes(&d, "v")), &g["merged"]["votes"], "votes");
    // Reverse order converges to the same state.
    let e = new_doc_with_client(101);
    for k in ["b", "a", "base"] {
        apply_update(&e, &unb64(&g[k]), None).unwrap();
    }
    assert_same(&from_y_value(&e), &from_y_value(&d), "reverse merge order");
}

#[test]
fn ts_and_rust_peers_converge() {
    let g = load("converge.json");
    let text = g["textId"].as_str().unwrap();
    let s1 = g["shapeIds"][0].as_str().unwrap();
    let s2 = g["shapeIds"][1].as_str().unwrap();
    let base = unb64(&g["base"]);
    let peer = |id: u64| {
        let d = new_doc_with_client(id);
        apply_update(&d, &base, None).unwrap();
        d
    };
    let (r1, r2) = (peer(20), peer(21));
    let mut rng = Mulberry32::new(5);
    // Concurrent with the TS peers: same paragraph, same shapes.
    apply(&r1, &mut rng, json!([{ "op": "textInsert", "id": text, "offset": 1, "text": "R" }]));
    apply(&r1, &mut rng, json!([{ "op": "xf", "id": s1, "xf": { "x": 3, "y": 3, "w": 10, "h": 10, "rot": 0 } }]));
    apply(&r1, &mut rng, json!([{ "op": "textParagraph", "id": text, "attrs": { "align": "center" } }]));
    cast_vote(&r1, "v", &Voter { id: "ur".into(), name: Some("Rust".into()) }, s1, 3.0);
    apply(&r2, &mut rng, json!([{ "op": "textDelete", "id": text, "offset": 0, "len": 1 }]));
    apply(&r2, &mut rng, json!([{ "op": "update", "id": s2, "patch": { "opacity": 0.25 } }, { "op": "reorder", "id": s2, "after": "bottom" }]));
    apply(&r2, &mut rng, json!([{ "op": "page", "action": "add", "page": { "id": "convPage00000000" } }]));

    let sv = {
        let b = peer(22);
        encode_state_vector(&b)
    };
    let ur1 = encode_diff(&r1, &sv).unwrap();
    let ur2 = encode_diff(&r2, &sv).unwrap();
    let (ua, ub) = (unb64(&g["a"]), unb64(&g["b"]));

    // Every peer gets everything, in different orders.
    let orders: [&[&[u8]]; 3] = [&[&ua, &ub, &ur1, &ur2], &[&ur2, &ub, &ur1, &ua], &[&ur1, &ua, &ur2, &ub]];
    let mut snaps = Vec::new();
    for (i, order) in orders.iter().enumerate() {
        let d = peer(30 + i as u64);
        for u in order.iter() {
            apply_update(&d, u, None).unwrap();
        }
        snaps.push((from_y_value(&d), text_of(&d, text), encode_state(&d)));
    }
    for (s, t, _) in &snaps[1..] {
        assert_same(s, &snaps[0].0, "merge orders converge");
        assert_eq!(t, &snaps[0].1);
    }
    // The text kept every peer's intent (TS "b", "d\nnew line" and bold; Rust "R", delete of "a", centring).
    assert!(snaps[0].1.contains('R') && snaps[0].1.contains("new line"), "{}", snaps[0].1);

    // yjs merging the same updates reaches the same state.
    let all: Vec<Vec<u8>> = vec![base.clone(), ua.clone(), ub.clone(), ur1.clone(), ur2.clone()];
    let Some(r) = ts_verify(vec![(all, vec![text.to_string()]), (vec![snaps[0].2.clone()], vec![text.to_string()])]) else { return };
    for (i, res) in r.iter().enumerate() {
        assert_same(&res["design"], &snaps[0].0, &format!("yjs merge #{i}"));
        assert_eq!(res["plain"][text], json!(snaps[0].1), "yjs plainText #{i}");
        let mut votes: Vec<Value> = read_votes(&peer_with(&snaps[0].2), "v").into_iter().map(|v| serde_json::to_value(v).unwrap()).collect();
        votes.sort_by_key(|v| v.to_string());
        assert_eq!(res["board"]["votes"].as_object().unwrap().len(), votes.len());
    }
}

fn peer_with(update: &[u8]) -> Doc {
    let d = new_doc_with_client(99);
    apply_update(&d, update, None).unwrap();
    d
}
