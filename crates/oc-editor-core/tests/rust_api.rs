// SPDX-License-Identifier: Apache-2.0
//! Rust-side API contract: the dispatcher's argument conventions and errors, host writes + `flush`, and the
//! documented deviations from TS (see the crate docs).

mod common;

use common::*;
use oc_crdt::{ApplyOptions, DocOp, OcOrigin};
use oc_editor_core::{DispatchError, Editor};
use oc_schema::tree::sorted_pages;
use serde_json::{Value, json};

fn setup() -> (Editor, String) {
    let (mut ed, page) = setup_with(1000.0, 800.0, 2, None, vp(0.0, 0.0, 1200.0, 900.0));
    ed.set_text_service(Some(Box::new(FakeText)));
    (ed, page)
}

fn call(ed: &mut Editor, m: &str, a: Value) -> Value {
    ed.dispatch(m, &a).unwrap_or_else(|e| panic!("{m}: {e}"))
}

fn page_ids(ed: &Editor) -> Vec<String> {
    sorted_pages(&ed.doc().doc.pages).iter().map(|p| p.id.clone()).collect()
}

#[test]
fn an_old_brand_lock_holds_nobody() {
    let (mut ed, page) = setup();
    let el = json!({ "id": "oldbrandlock0000", "type": "shape", "parentId": page, "z": "a0", "opacity": 1, "xf": { "x": 10, "y": 10, "w": 50, "h": 50, "rot": 0 }, "kind": "rect", "params": {}, "fills": [], "strokes": [],
        "lock": { "by": "brand", "move": true, "resize": true, "rotate": true, "delete": true, "content": true, "style": true } });
    let r = call(&mut ed, "apply", json!([[{ "op": "add", "el": el, "parent": page }]]));
    assert_eq!(r["ok"], json!(true));
    call(&mut ed, "select", json!([["oldbrandlock0000"]]));
    call(&mut ed, "nudge", json!([5, 0]));
    assert_eq!(ed.doc_json()["elements"]["oldbrandlock0000"]["xf"]["x"], json!(15));
    // The lock button sees an unlocked element and sets a real lock.
    call(&mut ed, "toggleLock", json!([]));
    assert_eq!(ed.doc_json()["elements"]["oldbrandlock0000"]["lock"]["by"], json!("user"));
    call(&mut ed, "toggleLock", json!([]));
    call(&mut ed, "deleteSelection", json!([]));
    assert!(ed.doc().live("oldbrandlock0000").is_none());
}

#[test]
fn dispatch_reports_unknown_methods_and_bad_arguments() {
    let (mut ed, _) = setup();
    assert_eq!(ed.dispatch("teleport", &json!([])), Err(DispatchError::Unknown("teleport".into())));
    assert!(matches!(ed.dispatch("select", &json!(["not a list"])), Err(DispatchError::Args(m, _)) if m == "select"));
    assert!(matches!(ed.dispatch("select", &json!({ "ids": [] })), Err(DispatchError::Args(..))));
    assert!(matches!(ed.dispatch("pointerDown", &json!([])), Err(DispatchError::Args(..))));
    assert!(matches!(ed.dispatch("setTool", &json!(["laser"])), Err(DispatchError::Args(..))));
    assert!(matches!(ed.dispatch("store.apply", &json!([[], "nowhere"])), Err(DispatchError::Args(..))));
    // `null` arguments are accepted where TS takes no arguments.
    assert_eq!(ed.dispatch("selectAll", &Value::Null), Ok(Value::Null));
}

#[test]
fn dispatch_keeps_the_ts_difference_between_undefined_and_null() {
    let (mut ed, first) = setup();
    // addPage() → after the active page; addPage(null) → at the end.
    let a = call(&mut ed, "addPage", json!([null])).as_str().unwrap().to_string();
    assert_eq!(page_ids(&ed).last(), Some(&a));
    call(&mut ed, "setActivePage", json!([first]));
    let b = call(&mut ed, "addPage", json!([])).as_str().unwrap().to_string();
    assert_eq!(page_ids(&ed)[1], b);

    // pasteStyle(ids) uses the armed brush; pasteStyle(ids, null) applies nothing.
    let red = call(&mut ed, "insert", json!([[shape_fill(xf(0.0, 0.0, 50.0, 50.0), "#ff0000")]]))[0].as_str().unwrap().to_string();
    let blue = call(&mut ed, "insert", json!([[shape_fill(xf(0.0, 0.0, 50.0, 50.0), "#0000ff")]]))[0].as_str().unwrap().to_string();
    let fills = |ed: &Editor, id: &str| ed.doc_json()["elements"][id]["fills"].clone();
    call(&mut ed, "select", json!([[red]]));
    call(&mut ed, "copyStyle", json!([]));
    call(&mut ed, "pasteStyle", json!([[blue], null]));
    assert_ne!(fills(&ed, &blue), fills(&ed, &red));
    call(&mut ed, "pasteStyle", json!([[blue]]));
    assert_eq!(fills(&ed, &blue), fills(&ed, &red));
}

#[test]
fn flush_runs_the_reactions_to_host_writes() {
    let (mut ed, page) = setup();
    let mut t = text(xf(10.0, 10.0, 300.0, 99.0), "Two\nlines");
    t.insert("parentId".into(), json!(page));
    let ops: Vec<DocOp> = serde_json::from_value(json!([{ "op": "add", "el": t, "parent": page }])).unwrap();
    let id = t["id"].as_str().unwrap().to_string();
    let v = ed.version();
    let r = ed.store_mut().apply(&ops, Some(OcOrigin::Local), ApplyOptions { now: Some(1.0), ..Default::default() });
    assert!(r.ok);
    assert_eq!(ed.version(), v, "nothing is processed until the editor runs");
    ed.flush();
    assert!(ed.version() > v);
    assert_eq!(exf(&ed, &id).h, 40.0, "auto-height applied to the host-written text");
}

#[test]
fn store_apply_with_an_untracked_origin_is_not_undoable() {
    let (mut ed, _) = setup();
    let id = call(&mut ed, "insert", json!([[shape(xf(0.0, 0.0, 50.0, 50.0))]]))[0].as_str().unwrap().to_string();
    let x0 = exf(&ed, &id).x;
    call(&mut ed, "checkpoint", json!([]));
    let r = call(&mut ed, "store.apply", json!([[{ "op": "update", "id": id, "patch": { "name": "Migrated" } }], "migration"]));
    assert_eq!(r["ok"], json!(true));
    call(&mut ed, "select", json!([[id]]));
    call(&mut ed, "nudge", json!([7, 0]));
    call(&mut ed, "undo", json!([]));
    // The nudge is undone; the migration write is not part of any undo step.
    assert_eq!(exf(&ed, &id).x, x0);
    assert_eq!(ed.doc_json()["elements"][&id]["name"], json!("Migrated"));
}

/// Deviation: a fix-up caused by an undo is untracked, so the redo stack keeps exactly the undone step (yjs adds
/// the fix-up as an extra redo step that reverts it).
#[test]
fn reactions_to_an_undo_keep_the_redo_stack() {
    let (mut ed, page) = setup();
    // A text whose stored height (99) disagrees with its layout (20): every restore of it gets fixed up.
    let mut t = text(xf(10.0, 10.0, 300.0, 99.0), "One line");
    t.insert("parentId".into(), json!(page));
    let ops: Vec<DocOp> = serde_json::from_value(json!([{ "op": "add", "el": t, "parent": page }])).unwrap();
    let id = t["id"].as_str().unwrap().to_string();
    ed.store_mut().apply(&ops, Some(OcOrigin::Migration), ApplyOptions { now: Some(1.0), ..Default::default() });
    ed.flush();
    assert_eq!(exf(&ed, &id).h, 20.0);
    ed.store_mut().checkpoint();
    call(&mut ed, "select", json!([[id]]));
    call(&mut ed, "nudge", json!([30, 0]));
    assert_eq!(exf(&ed, &id).x, 40.0);
    call(&mut ed, "undo", json!([]));
    assert_eq!((exf(&ed, &id).x, exf(&ed, &id).h), (10.0, 20.0));
    assert!(ed.can_redo(), "the undone step is still redoable");
    call(&mut ed, "redo", json!([]));
    assert_eq!(exf(&ed, &id).x, 40.0);
    assert!(ed.can_undo());
}

/// Deviation: toggling a list off writes `list: null` (TS removes the attribute; JSON DocOps cannot say
/// `undefined`). The paragraph reads as "no list" either way.
#[test]
fn turning_a_list_off_leaves_a_null_list() {
    let (mut ed, _) = setup();
    let id = call(&mut ed, "insert", json!([[text(xf(0.0, 0.0, 300.0, 20.0), "Item")]]))[0].as_str().unwrap().to_string();
    call(&mut ed, "select", json!([[id]]));
    let cmd = json!({ "shift": true, "alt": false, "meta": true });
    call(&mut ed, "textShortcut", json!(["*", cmd, "Digit8"]));
    assert_eq!(ed.doc_json()["elements"][&id]["body"]["paragraphs"][0]["list"], json!({ "kind": "bullet", "level": 0 }));
    call(&mut ed, "textShortcut", json!(["*", cmd, "Digit8"]));
    assert_eq!(ed.doc_json()["elements"][&id]["body"]["paragraphs"][0]["list"], Value::Null);
    assert!(text_el(&ed, &id).body.paragraphs[0].list.is_none());
}
