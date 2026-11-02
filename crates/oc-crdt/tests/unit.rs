// SPDX-License-Identifier: Apache-2.0
//! Ports of the @oc/crdt unit tests (crdt.test.ts, body.test.ts) plus Rust-specific checks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use oc_crdt::body::{BodyOp, edit_body};
use oc_crdt::repair::{RepairReport, repair_y};
use oc_crdt::text::plain_text;
use oc_crdt::value::canonical;
use oc_crdt::{
    ApplyOptions, DocOp, DocOpErrorCode, DocStore, OcOrigin, apply_ops, apply_update, codec, encode_diff, encode_state,
    encode_state_vector, from_y, from_y_value, new_doc, new_doc_with_client, to_y, to_y_value,
};
use oc_schema::elements::Element;
use oc_schema::factory::{MetaInit, ShapeOpts, TextOpts, base_init, empty_design, make_shape, make_text, text_body, xf};
use oc_schema::text::{RunStyleOverride, TextAlign};
use oc_schema::tree::sorted_pages;
use oc_schema::{DesignDoc, Surface};
use serde_json::{Value, json};
use yrs::{Any, Doc, Map as _, MapPrelim, Transact};

fn design() -> DesignDoc {
    empty_design(&MetaInit { w: 800.0, h: 600.0, ..Default::default() })
}

fn setup() -> (Doc, String) {
    let d = design();
    let page = sorted_pages(&d.pages)[0].id.clone();
    let ydoc = new_doc();
    to_y(&d, &ydoc, None).unwrap();
    (ydoc, page)
}

fn shape(x: f64) -> Value {
    oc_schema::js::to_value(&Element::Shape(make_shape(base_init(xf(x, 0.0, 10.0, 10.0, 0.0)), ShapeOpts::default()))).unwrap()
}

fn text(s: &str) -> Value {
    let t = make_text(base_init(xf(0.0, 0.0, 100.0, 20.0, 0.0)), TextOpts { text: Some(s.into()), ..Default::default() });
    oc_schema::js::to_value(&Element::Text(t)).unwrap()
}

fn id(el: &Value) -> String {
    el["id"].as_str().unwrap().to_string()
}

fn ops(v: Value) -> Vec<DocOp> {
    serde_json::from_value(v).unwrap()
}

fn run(d: &Doc, v: Value) -> oc_crdt::DocOpResult {
    apply_ops(d, &ops(v), None, ApplyOptions::default())
}

fn snap(d: &Doc) -> Value {
    from_y_value(d)
}

fn peers(n: usize) -> (Vec<Doc>, String) {
    let (ydoc, page) = setup();
    let base = encode_state(&ydoc);
    let docs = (0..n)
        .map(|i| {
            let d = new_doc_with_client(1000 + i as u64);
            apply_update(&d, &base, None).unwrap();
            d
        })
        .collect();
    (docs, page)
}

fn sync(docs: &[Doc]) {
    for a in docs {
        for b in docs {
            if !std::ptr::eq(a, b) {
                let u = encode_diff(a, &encode_state_vector(b)).unwrap();
                apply_update(b, &u, None).unwrap();
            }
        }
    }
}

fn text_plain(d: &Doc, id: &str) -> String {
    let r = codec::roots(d);
    let txn = d.transact();
    let m = codec::element_map(&txn, &r, id).unwrap();
    plain_text(&txn, &codec::text_of(&txn, &m).unwrap())
}

// ------------------------------------------------------------------ codec

#[test]
fn codec_round_trips_a_snapshot_through_ydoc() {
    let mut d = design();
    let p = sorted_pages(&d.pages)[0].id.clone();
    let mut t = make_text(base_init(xf(1.0, 2.0, 300.0, 40.0, 0.0)), TextOpts { text: Some("One\nTwo".into()), ..Default::default() });
    t.base.parent_id = p;
    t.base.z = "a0".into();
    let tid = t.base.id.clone();
    d.elements.insert(tid.clone(), Element::Text(t.clone()));
    let ydoc = new_doc();
    to_y(&d, &ydoc, None).unwrap();
    let back = from_y(&ydoc).unwrap();
    assert_eq!(back.elements[&tid], Element::Text(t));
    assert_eq!(back.pages, d.pages);
    assert_eq!(back.meta, d.meta);
}

#[test]
fn codec_layout_is_the_frozen_yjs_layout() {
    let (ydoc, page) = setup();
    let t = text("Hi");
    run(&ydoc, json!([{ "op": "add", "el": t, "parent": page }]));
    // Roots first: `roots` opens a write transaction, which must not overlap a read transaction.
    let r = codec::roots(&ydoc);
    let txn = ydoc.transact();
    let m = codec::element_map(&txn, &r, &id(&t)).unwrap();
    // Base keys on the element map, the rest in props, the body in an XmlFragment.
    assert!(matches!(m.get(&txn, "xf"), Some(yrs::Out::Any(Any::Map(_)))));
    assert!(matches!(m.get(&txn, "props"), Some(yrs::Out::YMap(_))));
    assert!(matches!(m.get(&txn, "text"), Some(yrs::Out::YXmlFragment(_))));
    let props = codec::props_of(&txn, &m).unwrap();
    assert!(props.get(&txn, "frame").is_some() && props.get(&txn, "defaultRun").is_some());
    assert!(props.get(&txn, "body").is_none() && m.get(&txn, "frame").is_none());
}

// ------------------------------------------------------------------ DocOps

#[test]
fn adds_updates_reorders_and_tombstones() {
    let (ydoc, page) = setup();
    let (a, b) = (shape(0.0), shape(0.0));
    assert!(run(&ydoc, json!([{ "op": "add", "el": a, "parent": page }, { "op": "add", "el": b, "parent": page }])).ok);
    let z = |d: &Doc, e: &Value| snap(d)["elements"][id(e)]["z"].as_str().unwrap().to_string();
    assert!(z(&ydoc, &b) > z(&ydoc, &a));
    run(&ydoc, json!([{ "op": "reorder", "id": id(&b), "after": "bottom" }]));
    assert!(z(&ydoc, &b) < z(&ydoc, &a));
    run(&ydoc, json!([{ "op": "update", "id": id(&a), "patch": { "opacity": 0.5, "fills": [] } }]));
    assert_eq!(snap(&ydoc)["elements"][id(&a)]["opacity"], json!(0.5));
    run(&ydoc, json!([{ "op": "delete", "id": id(&a) }]));
    assert!(snap(&ydoc)["elements"][id(&a)]["deleted"].is_object());
}

#[test]
fn is_all_or_nothing_for_a_batch() {
    let (ydoc, page) = setup();
    let a = shape(0.0);
    let r = run(&ydoc, json!([
        { "op": "add", "el": a, "parent": page },
        { "op": "xf", "id": id(&a), "xf": { "x": 0, "y": 0, "w": -5, "h": 10, "rot": 0 } },
    ]));
    assert!(!r.ok);
    assert_eq!(r.errors[0].code, DocOpErrorCode::Schema);
    assert!(snap(&ydoc)["elements"].get(id(&a)).is_none());
}

#[test]
fn rejects_cycles_and_edits_to_locked_elements() {
    let (ydoc, page) = setup();
    let a = shape(0.0);
    run(&ydoc, json!([{ "op": "add", "el": a, "parent": page }]));
    assert_eq!(run(&ydoc, json!([{ "op": "reparent", "id": id(&a), "parent": id(&a) }])).errors[0].code, DocOpErrorCode::Cycle);
    run(&ydoc, json!([{ "op": "update", "id": id(&a), "patch": { "lock": { "move": true, "by": "user" } } }]));
    assert_eq!(
        run(&ydoc, json!([{ "op": "xf", "id": id(&a), "xf": { "x": 5, "y": 5, "w": 10, "h": 10, "rot": 0 } }])).errors[0].code,
        DocOpErrorCode::Locked
    );
    assert!(run(&ydoc, json!([{ "op": "update", "id": id(&a), "patch": { "lock": null } }])).ok);
}

#[test]
fn an_old_brand_lock_holds_nobody() {
    let (ydoc, page) = setup();
    let a = shape(0.0);
    run(&ydoc, json!([{ "op": "add", "el": a, "parent": page }]));
    let lock = json!({ "by": "brand", "move": true, "resize": true, "rotate": true, "delete": true, "content": true, "style": true });
    assert!(run(&ydoc, json!([{ "op": "update", "id": id(&a), "patch": { "lock": lock } }])).ok);
    assert!(run(&ydoc, json!([{ "op": "xf", "id": id(&a), "xf": { "x": 5, "y": 5, "w": 10, "h": 10, "rot": 0 } }])).ok);
    assert!(run(&ydoc, json!([{ "op": "update", "id": id(&a), "patch": { "opacity": 0.5 } }])).ok);
    assert!(run(&ydoc, json!([{ "op": "delete", "id": id(&a) }])).ok);
}

#[test]
fn duplicates_and_moves_pages() {
    let (ydoc, page) = setup();
    let a = shape(0.0);
    run(&ydoc, json!([{ "op": "add", "el": a, "parent": page }]));
    let r = run(&ydoc, json!([{ "op": "page", "action": "duplicate", "pageId": page }]));
    let copy = r.created[0].clone();
    let order = |d: &Doc| {
        let s: DesignDoc = from_y(d).unwrap();
        sorted_pages(&s.pages).into_iter().map(|p| p.id.clone()).collect::<Vec<_>>()
    };
    assert_eq!(order(&ydoc), vec![page.clone(), copy.clone()]);
    let s = snap(&ydoc);
    let copies: Vec<&Value> = s["elements"].as_object().unwrap().values().filter(|e| e["parentId"] == json!(copy)).collect();
    assert_eq!(copies.len(), 1);
    // The copy is a complete element (type, transform, fills), not just a tree stub.
    let mut c = copies[0].clone();
    c["id"] = json!(id(&a));
    c["parentId"] = json!(page);
    assert_eq!(canonical(&c), canonical(&s["elements"][id(&a)]));
    run(&ydoc, json!([{ "op": "page", "action": "move", "pageId": copy, "after": null }]));
    assert_eq!(order(&ydoc), vec![copy, page]);
}

// ------------------------------------------------------------------ rich text

#[test]
fn splits_and_merges_paragraphs_with_flat_offsets() {
    let (ydoc, page) = setup();
    let t = text("Hello");
    let tid = id(&t);
    run(&ydoc, json!([{ "op": "add", "el": t, "parent": page }]));
    run(&ydoc, json!([{ "op": "textInsert", "id": tid, "offset": 5, "text": " big\nworld" }]));
    assert_eq!(text_plain(&ydoc, &tid), "Hello big\nworld");
    run(&ydoc, json!([{ "op": "textDelete", "id": tid, "offset": 8, "len": 3 }]));
    assert_eq!(text_plain(&ydoc, &tid), "Hello biorld");
    run(&ydoc, json!([{ "op": "textFormat", "id": tid, "offset": 0, "len": 5, "marks": { "weight": 700 } }]));
    let body = &snap(&ydoc)["elements"][&tid]["body"];
    assert_eq!(body["paragraphs"][0]["runs"][0], json!({ "text": "Hello", "style": { "weight": 700 } }));
}

#[test]
fn offsets_are_utf16_like_js_strings() {
    let (ydoc, page) = setup();
    let t = text("a😀b");
    let tid = id(&t);
    run(&ydoc, json!([{ "op": "add", "el": t, "parent": page }]));
    // "😀" is two UTF-16 units: offset 3 is between the emoji and "b".
    run(&ydoc, json!([{ "op": "textInsert", "id": tid, "offset": 3, "text": "|" }]));
    {
        let r = codec::roots(&ydoc);
        let txn = ydoc.transact();
        let frag = codec::text_of(&txn, &codec::element_map(&txn, &r, &tid).unwrap()).unwrap();
        assert_eq!(oc_crdt::text::text_content(&txn, &frag), "a😀|b");
    }
    run(&ydoc, json!([{ "op": "textDelete", "id": tid, "offset": 1, "len": 2 }]));
    assert_eq!(text_plain(&ydoc, &tid), "a|b");
}

// ------------------------------------------------------------------ convergence

#[test]
fn concurrent_edits_from_4_peers_converge() {
    let (docs, page) = peers(4);
    let mut ids = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        let el = shape(i as f64 * 10.0);
        ids.push(id(&el));
        run(d, json!([{ "op": "add", "el": el, "parent": page }]));
    }
    sync(&docs);
    // Everyone moves every element and reorders concurrently.
    for (i, d) in docs.iter().enumerate() {
        for e in &ids {
            let i = i as f64;
            run(d, json!([{ "op": "xf", "id": e, "xf": { "x": i, "y": i, "w": 20, "h": 20, "rot": 0 } }, { "op": "reorder", "id": e, "after": "top" }]));
        }
    }
    sync(&docs);
    let snaps: Vec<String> = docs.iter().map(|d| oc_crdt::value::canonical_string(&snap(d))).collect();
    assert!(snaps.iter().all(|s| s == &snaps[0]));
}

#[test]
fn concurrent_typing_in_the_same_paragraph_merges_character_wise() {
    let (docs, page) = peers(2);
    let t = text("ac");
    let tid = id(&t);
    run(&docs[0], json!([{ "op": "add", "el": t, "parent": page }]));
    sync(&docs);
    run(&docs[0], json!([{ "op": "textInsert", "id": tid, "offset": 1, "text": "b" }]));
    run(&docs[1], json!([{ "op": "textInsert", "id": tid, "offset": 2, "text": "d" }]));
    sync(&docs);
    assert_eq!(text_plain(&docs[0], &tid), "abcd");
    assert_eq!(text_plain(&docs[1], &tid), "abcd");
}

// ------------------------------------------------------------------ DocStore

#[test]
fn docstore_keeps_an_incremental_snapshot_and_undoes_only_local_changes() {
    let (ydoc, page) = setup();
    let clock = Arc::new(AtomicU64::new(1_000));
    let c = clock.clone();
    let mut store = DocStore::new(ydoc.clone(), Arc::new(move || c.load(Ordering::SeqCst)));
    let a = shape(0.0);
    let aid = id(&a);
    let calls = Arc::new(AtomicUsize::new(0));
    let cc = calls.clone();
    store.subscribe(Arc::new(move |_, _| {
        cc.fetch_add(1, Ordering::SeqCst);
    }));
    store.apply(&ops(json!([{ "op": "add", "el": a, "parent": page }])), None, ApplyOptions::default());
    assert!(store.doc().elements.contains_key(&aid));
    // A remote change lands between two local steps; undo must not revert it.
    let remote = new_doc();
    apply_update(&remote, &encode_state(&ydoc), None).unwrap();
    run(&remote, json!([{ "op": "update", "id": aid, "patch": { "opacity": 0.3 } }]));
    store.apply_update(&encode_diff(&remote, &encode_state_vector(&ydoc)).unwrap(), None).unwrap();
    store.checkpoint();
    clock.fetch_add(10_000, Ordering::SeqCst);
    store.apply(&ops(json!([{ "op": "xf", "id": aid, "xf": { "x": 50, "y": 50, "w": 10, "h": 10, "rot": 0 } }])), Some(OcOrigin::Local), ApplyOptions::default());
    assert_eq!(store.doc().elements[&aid]["xf"]["x"], json!(50));
    assert!(store.undo());
    assert_eq!(store.doc().elements[&aid]["xf"]["x"], json!(0));
    assert_eq!(store.doc().elements[&aid]["opacity"], json!(0.3));
    assert!(calls.load(Ordering::SeqCst) >= 4);
    // The incremental snapshot equals a full re-read.
    assert_eq!(canonical(&store.doc().to_value()), canonical(&snap(&ydoc)));
    assert!(store.redo());
    assert_eq!(store.doc().elements[&aid]["xf"]["x"], json!(50));
}

#[test]
fn docstore_reports_what_changed() {
    let (ydoc, page) = setup();
    let store = DocStore::new(ydoc.clone(), Arc::new(|| 0));
    let seen: Arc<std::sync::Mutex<Vec<oc_crdt::Change>>> = Arc::default();
    let s = seen.clone();
    store.subscribe(Arc::new(move |_, c| s.lock().unwrap().push(c.clone())));
    let t = text("x");
    let tid = id(&t);
    store.apply(&ops(json!([{ "op": "add", "el": t, "parent": page }])), None, ApplyOptions::default());
    store.apply(&ops(json!([{ "op": "textInsert", "id": tid, "offset": 1, "text": "y" }, { "op": "meta", "patch": { "title": "T" } }])), None, ApplyOptions::default());
    let remote = new_doc();
    apply_update(&remote, &encode_state(&ydoc), None).unwrap();
    run(&remote, json!([{ "op": "page", "action": "update", "pageId": page, "patch": { "title": "P" } }]));
    store.apply_update(&encode_diff(&remote, &encode_state_vector(&ydoc)).unwrap(), None).unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen[0].elements.contains(&tid) && seen[0].local);
    assert!(seen[1].elements.contains(&tid) && seen[1].meta && !seen[1].assets);
    assert!(seen[2].pages.contains(&page) && !seen[2].local);
    assert_eq!(seen[0].origin.as_ref().and_then(OcOrigin::from_origin), Some(OcOrigin::Local));
}

// ------------------------------------------------------------------ repair

#[test]
fn repair_tombstones_orphans_breaks_cycles_and_fixes_nan_transforms() {
    let doc = empty_design(&MetaInit { w: 100.0, h: 100.0, ..Default::default() });
    let page = doc.pages.keys().next().unwrap().clone();
    let ydoc = new_doc();
    to_y(&doc, &ydoc, None).unwrap();
    let r = codec::roots(&ydoc);
    let mk = |id: &str, parent: &str, x: f64| {
        let mut txn = ydoc.transact_mut();
        let m = r.elements.insert(&mut txn, id, MapPrelim::default());
        m.insert(&mut txn, "id", id);
        m.insert(&mut txn, "type", "group");
        m.insert(&mut txn, "parentId", parent);
        m.insert(&mut txn, "z", "a0");
        let xf = oc_crdt::value::json_to_any(&json!({ "y": 0, "w": 10, "h": 10, "rot": 0 }));
        let Any::Map(mut fields) = xf else { unreachable!() };
        Arc::make_mut(&mut fields).insert("x".into(), Any::Number(yrs::any::Number::Float(x)));
        m.insert(&mut txn, "xf", Any::Map(fields));
        m.insert(&mut txn, "opacity", 1);
        m.insert(&mut txn, "props", MapPrelim::default());
    };
    mk("orphan", "missing-parent", 0.0);
    mk("loopA", "loopB", 0.0);
    mk("loopB", "loopA", 0.0);
    mk("nan", &page, f64::NAN);
    let rep = repair_y(&ydoc, 0.0);
    assert_eq!(rep.orphans, 1);
    assert!(rep.cycles >= 1);
    assert_eq!(rep.transforms, 1);
    let s = snap(&ydoc);
    assert!(s["elements"]["orphan"]["deleted"].is_object());
    assert_eq!(s["elements"]["nan"]["xf"]["x"], json!(0));
    // Second pass finds nothing.
    assert_eq!(repair_y(&ydoc, 0.0), RepairReport::default());
}

// ------------------------------------------------------------------ retired whiteboard state

#[test]
fn documents_with_whiteboard_timer_and_vote_roots_still_decode() {
    use yrs::{Map, Transact};
    let ydoc = new_doc();
    to_y(&empty_design(&MetaInit { w: 800.0, h: 600.0, surface: Some(Surface::Whiteboard), ..Default::default() }), &ydoc, None).unwrap();
    {
        let board = ydoc.get_or_insert_map("board");
        let votes = ydoc.get_or_insert_map("votes");
        let mut txn = ydoc.transact_mut();
        board.insert(&mut txn, "timer", "old");
        votes.insert(&mut txn, "v|u|e", 1.0);
    }
    let copy = new_doc();
    apply_update(&copy, &encode_state(&ydoc), None).unwrap();
    assert_eq!(snap(&copy)["meta"]["surface"], json!("whiteboard"));
    assert_eq!(snap(&copy)["elements"].as_object().unwrap().len(), 0);
}

// ------------------------------------------------------------------ editBody

#[test]
fn edit_body_inserts_splits_formats_and_deletes_like_text_elements_do() {
    let b0 = text_body("Hello", &RunStyleOverride::default(), TextAlign::Start);
    let ed = |b: &oc_schema::TextBody, v: Value| edit_body(b, &serde_json::from_value::<Vec<BodyOp>>(v).unwrap()).unwrap();
    let words = |b: &oc_schema::TextBody| b.paragraphs.iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>();
    let b1 = ed(&b0, json!([{ "op": "textInsert", "offset": 5, "text": " world\nnext" }]));
    assert_eq!(words(&b1), vec!["Hello world", "next"]);
    let b2 = ed(&b1, json!([{ "op": "textFormat", "offset": 0, "len": 5, "marks": { "weight": 700 } }]));
    assert_eq!(b2.paragraphs[0].runs[0].text, "Hello");
    assert_eq!(b2.paragraphs[0].runs[0].style.weight, Some(700.0));
    assert_eq!(b2.paragraphs[0].runs[1].text, " world");
    let b3 = ed(&b2, json!([{ "op": "textDelete", "offset": 5, "len": 7 }, { "op": "textParagraph", "attrs": { "align": "center" } }]));
    assert_eq!(b3.paragraphs.len(), 1);
    assert_eq!(b3.paragraphs[0].align, TextAlign::Center);
    assert_eq!(words(&b3), vec!["Hellonext"]);
    assert_eq!(b3.default_run, b0.default_run);
    // The input is never mutated.
    assert_eq!(b0.paragraphs[0].runs[0].text, "Hello");
}

// ------------------------------------------------------------------ misc

#[test]
fn json_snapshot_and_typed_snapshot_agree() {
    let d = design();
    let ydoc = new_doc();
    to_y_value(&oc_schema::js::to_value(&d).unwrap(), &ydoc, Some(OcOrigin::Template.origin()));
    assert_eq!(from_y(&ydoc).unwrap(), d);
    assert_eq!(oc_crdt::decode_doc(&oc_crdt::encode_doc(&d).unwrap()).unwrap(), d);
}

#[test]
fn mulberry32_matches_the_js_reference() {
    // mulberry32(1) first outputs, from the JS function in tools/golden/crdt.ts.
    let mut r = oc_crdt::rng::Mulberry32::new(1);
    let got: Vec<u32> = (0..3).map(|_| r.next_u32()).collect();
    assert_eq!(got, vec![2_693_262_067, 11_749_833, 2_265_367_787]);
}

/// TXT-01: a delete that starts or ends between the halves of a surrogate pair widens to the whole code point
/// (yrs panics on half a pair: "Couldn't remove 1 elements from an array").
#[test]
fn text_delete_never_splits_a_surrogate_pair() {
    let body = text_body("ab😀c", &RunStyleOverride::default(), TextAlign::Start);
    let plain = |b: &oc_schema::TextBody| b.paragraphs.iter().flat_map(|p| p.runs.iter().map(|r| r.text.clone())).collect::<String>();
    // Backspace as an old client sent it: one unit before the caret after the emoji.
    let b = edit_body(&body, &[BodyOp::TextDelete { offset: 3.0, len: 1.0 }]).unwrap();
    assert_eq!(plain(&b), "abc");
    let b = edit_body(&body, &[BodyOp::TextDelete { offset: 2.0, len: 1.0 }]).unwrap();
    assert_eq!(plain(&b), "abc");
    let b = edit_body(&body, &[BodyOp::TextDelete { offset: 1.0, len: 2.0 }]).unwrap();
    assert_eq!(plain(&b), "ac");
}
