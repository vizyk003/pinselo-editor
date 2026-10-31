// SPDX-License-Identifier: Apache-2.0
//! Sync model round trip with a **yjs** peer (`pnpm tsx tools/golden/editor.ts --verify`): the Rust editor's
//! `drain_local_updates` must rebuild the same design in yjs, and a yjs peer's update fed through
//! `apply_remote_update` must give the same design in Rust (and sanitise the editor state). Skipped when the TS
//! side is unavailable (`OC_SKIP_TS_VERIFY`, no pnpm, TS packages removed); the yrs-only checks always run.

mod common;

use std::process::Command;

use base64::Engine;
use common::{GoldText, golden, json_diff, root, short};
use oc_editor_core::{Editor, EditorOptions, ManualClock, Mulberry32, Viewport};
use serde_json::{Value, json};

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn unb64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(s).expect("base64")
}

/// Run the yjs side; `None` when it is unavailable.
fn ts_verify(cases: Value) -> Option<Vec<Value>> {
    if std::env::var_os("OC_SKIP_TS_VERIFY").is_some() {
        eprintln!("OC_SKIP_TS_VERIFY set: skipping the yjs side");
        return None;
    }
    let root = root();
    if !root.join("tools/golden/editor.ts").exists() || !root.join("packages/crdt/src/index.ts").exists() {
        eprintln!("TS packages not present: skipping the yjs side");
        return None;
    }
    let dir = std::env::temp_dir();
    let tag = format!("{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], "");
    let input = dir.join(format!("oc-editor-sync-{tag}.json"));
    let output = dir.join(format!("oc-editor-sync-{tag}.out.json"));
    std::fs::write(&input, serde_json::to_vec(&json!({ "cases": cases })).unwrap()).expect("write verify input");
    let out = Command::new("pnpm").args(["tsx", "tools/golden/editor.ts", "--verify"]).arg(&input).arg("--out").arg(&output).current_dir(&root).output();
    let _ = std::fs::remove_file(&input);
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cannot run pnpm ({e}): skipping the yjs side");
            return None;
        }
    };
    assert!(out.status.success(), "TS verify failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(&output).expect("verify output");
    let _ = std::fs::remove_file(&output);
    let v: Value = serde_json::from_slice(&bytes).expect("verify output json");
    Some(v["results"].as_array().expect("results").clone())
}

#[track_caller]
fn assert_same(actual: &Value, expected: &Value, what: &str) {
    if let Some(d) = json_diff(actual, expected, what) {
        panic!("{d}");
    }
}

fn editor() -> Editor {
    let clock = ManualClock::new(1_700_000_000_000.0);
    let mut ed = Editor::from_ydoc(
        oc_crdt::new_doc_with_client(2),
        EditorOptions { viewport: Viewport { x: 0.0, y: 0.0, zoom: 1.0, width: 1200.0, height: 900.0 }, clock, rng: Box::new(Mulberry32::new(4242)) },
    );
    ed.set_text_service(Some(Box::new(GoldText)));
    ed
}

fn call(ed: &mut Editor, m: &str, a: Value) -> Value {
    ed.dispatch(m, &a).unwrap_or_else(|e| panic!("{m}: {e}"))
}

/// A replica built from updates only must read the same design as the editor.
fn yrs_replica(updates: &[Vec<u8>]) -> Value {
    let d = oc_crdt::new_doc_with_client(77);
    for u in updates {
        oc_crdt::codec::apply_update(&d, u, None).expect("apply update");
    }
    oc_crdt::DocStore::new(d, std::sync::Arc::new(|| 1u64)).doc().to_value()
}

#[test]
fn local_updates_and_remote_updates_round_trip_with_yjs() {
    // Start from a TS-encoded design (the select session's initial state), received like an initial sync.
    let g = golden("select.json");
    let initial = unb64(g["update"].as_str().unwrap());
    let mut ed = editor();
    ed.apply_remote_update(&initial).expect("initial sync");
    assert!(ed.drain_local_updates().is_empty(), "remote updates are not echoed back");
    assert_same(&ed.doc_json(), &g["doc"], "doc after initial sync");
    let pages: Vec<String> = {
        let d = ed.doc();
        oc_schema::tree::sorted_pages(&d.doc.pages).iter().map(|p| p.id.clone()).collect()
    };
    assert_eq!(ed.state.active_page_id, pages[0], "the active page follows the first synced page");

    // Local edits of every kind: element adds, text CRDT edits, moves, grouping, deletes, pages, meta, tables.
    let shape = json!({ "id": "SyncShape0000001", "type": "shape", "parentId": "", "z": "a0", "opacity": 1,
        "xf": { "x": 0, "y": 0, "w": 120, "h": 80, "rot": 0 }, "kind": "rect", "params": {}, "fills": [], "strokes": [] });
    let ids = call(&mut ed, "insert", json!([[shape], [300, 300]]));
    let sid = ids[0].as_str().unwrap().to_string();
    call(&mut ed, "handleKey", json!(["t", { "shift": false, "alt": false, "meta": false }]));
    call(&mut ed, "typeText", json!(["Hello "]));
    call(&mut ed, "typeText", json!(["yjs"]));
    call(&mut ed, "selectAll", json!([]));
    call(&mut ed, "setTextStyle", json!([{ "weight": 700 }]));
    call(&mut ed, "moveCaret", json!(["end", false]));
    call(&mut ed, "deleteText", json!([-1]));
    call(&mut ed, "stopTextEdit", json!([]));
    call(&mut ed, "select", json!([[sid]]));
    call(&mut ed, "pointerDown", json!([[360, 340], { "shift": false, "alt": false, "meta": true }]));
    call(&mut ed, "pointerMove", json!([[420, 390], { "shift": false, "alt": false, "meta": true }]));
    call(&mut ed, "pointerUp", json!([]));
    call(&mut ed, "selectAll", json!([]));
    call(&mut ed, "group", json!([]));
    call(&mut ed, "ungroup", json!([]));
    call(&mut ed, "addPage", json!([]));
    let mut base = oc_schema::factory::base_init(oc_schema::primitives::Xf::new(0.0, 0.0, 300.0, 120.0, 0.0));
    base.id = "SyncTable0000001".into();
    let t = oc_schema::factory::make_table(base, oc_schema::factory::TableOpts { rows: Some(3), cols: Some(2), data: None });
    let table = oc_schema::js::to_value(&oc_schema::elements::Element::Table(t)).unwrap();
    let tid = call(&mut ed, "insert", json!([[table]]))[0].as_str().unwrap().to_string();
    call(&mut ed, "startCellEdit", json!([tid, 0, 1]));
    call(&mut ed, "typeText", json!(["cell"]));
    call(&mut ed, "stopCellEdit", json!([]));
    call(&mut ed, "setLanguage", json!(["fr"]));
    call(&mut ed, "setActivePage", json!([pages[0]]));
    call(&mut ed, "selectAll", json!([]));
    call(&mut ed, "undo", json!([]));
    call(&mut ed, "redo", json!([]));
    let local = ed.drain_local_updates();
    assert!(!local.is_empty(), "local edits produce updates");
    assert!(ed.drain_local_updates().is_empty(), "drain empties the outbox");

    let mut all = vec![initial.clone()];
    all.extend(local.iter().cloned());
    assert_same(&yrs_replica(&all), &ed.doc_json(), "yrs replica of the drained updates");

    // The yjs peer reads the same design, then edits it concurrently-free and sends its update back.
    let a_id = g["doc"]["elements"].as_object().unwrap().keys().next().unwrap().clone();
    let selected: Vec<String> = ed.state.selection.clone();
    let victim = selected.iter().find(|i| **i != a_id).cloned().unwrap_or_else(|| selected[0].clone());
    let peer_ops = json!([
        { "op": "delete", "id": victim },
        { "op": "xf", "id": sid, "xf": { "x": 10, "y": 20, "w": 30, "h": 40, "rot": 15 } },
        { "op": "meta", "patch": { "title": "From yjs" } },
        { "op": "page", "action": "add" },
    ]);
    let updates: Vec<String> = all.iter().map(|u| b64(u)).collect();
    let Some(r) = ts_verify(json!([{ "updates": updates, "peerOps": peer_ops, "seed": 7 }])) else { return };
    let r = &r[0];
    assert_same(&ed.doc_json(), &r["design"], "yjs reads what the Rust editor wrote");
    assert_eq!(r["result"]["ok"], json!(true), "peer ops: {}", short(&r["result"]));

    let before = ed.version();
    ed.apply_remote_update(&unb64(r["update"].as_str().unwrap())).expect("peer update");
    assert!(ed.version() > before, "remote changes notify");
    assert_same(&ed.doc_json(), &r["after"], "Rust reads what the yjs peer wrote");
    assert!(!ed.state.selection.contains(&victim), "a remotely deleted element leaves the selection");
    assert!(ed.drain_local_updates().is_empty(), "remote updates are not echoed back");

    // Keep editing on top of the peer's change; yjs must still agree.
    call(&mut ed, "select", json!([[sid]]));
    call(&mut ed, "nudge", json!([5, 5]));
    call(&mut ed, "duplicate", json!([]));
    let more = ed.drain_local_updates();
    let mut all2: Vec<String> = updates.clone();
    all2.push(r["update"].as_str().unwrap().to_string());
    all2.extend(more.iter().map(|u| b64(u)));
    let r2 = ts_verify(json!([{ "updates": all2 }])).expect("TS side available");
    assert_same(&ed.doc_json(), &r2[0]["design"], "yjs agrees after further local edits");
}

#[test]
fn a_remote_deletion_ends_cell_editing_and_clears_the_selection() {
    let mut ed = editor();
    let g = golden("table.json");
    ed.apply_remote_update(&unb64(g["update"].as_str().unwrap())).unwrap();
    let tid = g["doc"]["elements"].as_object().unwrap().iter().find(|(_, e)| e["type"] == "table").unwrap().0.clone();
    call(&mut ed, "startCellEdit", json!([tid, 1, 1]));
    assert!(ed.state.editing_cell.is_some());
    // A yrs peer (same wire format) deletes the table.
    let peer = oc_crdt::new_doc_with_client(91);
    oc_crdt::codec::apply_update(&peer, &oc_crdt::codec::encode_state(ed.store().ydoc()), None).unwrap();
    let sv = oc_crdt::codec::encode_state_vector(&peer);
    let ops: Vec<oc_crdt::DocOp> = serde_json::from_value(json!([{ "op": "delete", "id": tid }])).unwrap();
    let r = oc_crdt::apply_ops(&peer, &ops, None, oc_crdt::ApplyOptions { now: Some(1.0), ..Default::default() });
    assert!(r.ok);
    let upd = oc_crdt::codec::encode_diff(&peer, &sv).unwrap();
    ed.apply_remote_update(&upd).unwrap();
    assert!(ed.state.editing_cell.is_none() && ed.state.editing_text.is_none() && ed.state.selection.is_empty());
}

fn peer_editor(client: u64) -> Editor {
    let clock = ManualClock::new(1_700_000_000_000.0);
    let mut ed = Editor::from_ydoc(
        oc_crdt::new_doc_with_client(client),
        EditorOptions { viewport: Viewport { x: 0.0, y: 0.0, zoom: 1.0, width: 1200.0, height: 900.0 }, clock, rng: Box::new(Mulberry32::new(client as u32)) },
    );
    ed.set_text_service(Some(Box::new(GoldText)));
    ed
}

/// Two editors on the same design (A seeded, B synced from A's full state), with different yrs clients.
fn pair() -> (Editor, Editor) {
    let d = oc_schema::factory::empty_design(&oc_schema::factory::MetaInit { w: 800.0, h: 600.0, ..Default::default() });
    let mut a = common::editor_for(&d, Viewport { x: 0.0, y: 0.0, zoom: 1.0, width: 1200.0, height: 900.0 });
    a.set_text_service(Some(Box::new(GoldText)));
    let mut b = peer_editor(12);
    b.apply_remote_update(&oc_crdt::codec::encode_state(a.store().ydoc())).unwrap();
    (a, b)
}

fn exchange(a: &mut Editor, b: &mut Editor) {
    for u in a.drain_local_updates() {
        b.apply_remote_update(&u).unwrap();
    }
    for u in b.drain_local_updates() {
        a.apply_remote_update(&u).unwrap();
    }
}

fn text_of(ed: &Editor, id: &str) -> String {
    let doc = ed.doc_json();
    doc["elements"][id]["body"]["paragraphs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["runs"].as_array().unwrap().iter().map(|r| r["text"].as_str().unwrap()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// COL-02: two people typing in the same text box, one at the start and one at the end. The local caret follows
/// the peer's inserts (sticky index), so both runs stay intact and in order, with per-key delivery and in bursts.
#[test]
fn concurrent_typing_in_one_text_box_keeps_both_inserts_intact() {
    for burst in [1usize, 3] {
        let (mut a, mut b) = pair();
        let ids = call(&mut a, "insert", json!([[common::text(common::xf(0.0, 0.0, 400.0, 40.0), "Hello world")], null, {}]));
        let id = ids[0].as_str().unwrap().to_string();
        exchange(&mut a, &mut b);
        call(&mut a, "startTextEdit", json!([id]));
        call(&mut a, "moveCaret", json!(["end", false]));
        call(&mut b, "startTextEdit", json!([id]));
        call(&mut b, "moveCaret", json!(["home", false]));
        let (ta, tb): (Vec<char>, Vec<char>) = ("12345".chars().collect(), "ab".chars().collect());
        let mut i = 0;
        while i < ta.len().max(tb.len()) {
            for k in i..(i + burst) {
                if let Some(c) = ta.get(k) {
                    call(&mut a, "typeText", json!([c.to_string()]));
                }
                if let Some(c) = tb.get(k) {
                    call(&mut b, "typeText", json!([c.to_string()]));
                }
            }
            exchange(&mut a, &mut b);
            i += burst;
        }
        assert_eq!(text_of(&a, &id), "abHello world12345", "burst {burst}");
        assert_eq!(text_of(&b, &id), "abHello world12345", "burst {burst}");
        // Both carets sit right after their own text.
        assert_eq!(a.state.editing_text.as_ref().unwrap().focus, 18);
        assert_eq!(b.state.editing_text.as_ref().unwrap().focus, 2);
    }
}

/// COL-02: typing at the same spot concurrently keeps each person's run contiguous (no interleaving).
#[test]
fn concurrent_typing_at_the_same_spot_keeps_runs_contiguous() {
    let (mut a, mut b) = pair();
    let ids = call(&mut a, "insert", json!([[common::text(common::xf(0.0, 0.0, 400.0, 40.0), "[]")], null, {}]));
    let id = ids[0].as_str().unwrap().to_string();
    exchange(&mut a, &mut b);
    for ed in [&mut a, &mut b] {
        call(ed, "startTextEdit", json!([id]));
        call(ed, "moveCaret", json!(["home", false]));
        call(ed, "moveCaret", json!(["right", false]));
    }
    for (x, y) in ["a1", "a2", "a3"].iter().zip(["b1", "b2", "b3"]) {
        call(&mut a, "typeText", json!([x]));
        call(&mut b, "typeText", json!([y]));
        exchange(&mut a, &mut b);
    }
    let t = text_of(&a, &id);
    assert_eq!(t, text_of(&b, &id));
    assert!(t.contains("a1a2a3") && t.contains("b1b2b3"), "runs interleaved: {t}");
}
