// SPDX-License-Identifier: Apache-2.0
//! Port of packages/editor-core/src/brandlock.test.ts.

mod common;

use common::*;
use oc_crdt::{DocOp, brand_lock_violations};
use oc_editor_core::{BrandLockLevels, Editor};
use oc_schema::elements::Element;
use oc_schema::factory::{TextOpts, base_init, make_text, text_body};
use oc_schema::text::{RunStyleOverride, TextAlign};
use serde_json::{Map, Value, json};

struct Ids {
    logo: String,
    head: String,
    boxx: String,
    free: String,
}

fn setup(admin: bool) -> (Editor, Ids) {
    let (mut ed, page) = setup_with(800.0, 600.0, 1, None, oc_editor_core::Viewport::default());
    let mut logo = shape(xf(10.0, 10.0, 100.0, 100.0));
    let body = text_body("Brand headline", &RunStyleOverride { family: Some("Inter".into()), size: Some(40.0), ..Default::default() }, TextAlign::Start);
    let mut head = to_map(Element::Text(make_text(base_init(xf(10.0, 200.0, 300.0, 60.0)), TextOpts { body: Some(body), ..Default::default() })));
    let mut boxx = shape_fill(xf(300.0, 300.0, 100.0, 100.0), "#7c3aed");
    let mut free = shape(xf(500.0, 300.0, 50.0, 50.0));
    for m in [&mut logo, &mut head, &mut boxx, &mut free] {
        m.insert("parentId".into(), Value::String(page.clone()));
    }
    let ids = Ids { logo: id_of(&logo), head: id_of(&head), boxx: id_of(&boxx), free: id_of(&free) };
    ed.apply([logo, head, boxx, free].into_iter().map(|el| DocOp::Add { el: Value::Object(el), parent: page.clone(), after: None }).collect(), false);
    ed.select(std::slice::from_ref(&ids.logo), false);
    ed.set_brand_lock(Some(BrandLockLevels { position: true, ..Default::default() }));
    ed.select(std::slice::from_ref(&ids.head), false);
    ed.set_brand_lock(Some(BrandLockLevels { content: true, ..Default::default() }));
    ed.select(std::slice::from_ref(&ids.boxx), false);
    ed.set_brand_lock(Some(BrandLockLevels { style: true, ..Default::default() }));
    ed.clear_selection();
    ed.set_brand_admin(admin);
    (ed, ids)
}

fn xfv(x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({ "x": x, "y": y, "w": w, "h": h, "rot": 0 })
}

fn upd(id: &str, patch: Value) -> DocOp {
    DocOp::Update { id: id.into(), patch: obj(patch) }
}

fn solid(hex: &str) -> Value {
    oc_schema::js::to_value(&oc_schema::color::solid_hex(hex).unwrap()).unwrap()
}

fn elements_json(ed: &Editor) -> Map<String, Value> {
    obj(ed.doc_json()["elements"].clone())
}

#[test]
fn holds_members_to_the_lock_mask() {
    let (mut ed, ids) = setup(false);
    assert!(!ed.apply(vec![DocOp::Xf { id: ids.logo.clone(), xf: xfv(50.0, 50.0, 100.0, 100.0) }], false).ok);
    assert!(!ed.apply(vec![DocOp::Delete { id: ids.logo.clone() }], false).ok);
    assert!(ed.apply(vec![upd(&ids.logo, json!({ "fills": [solid("#ff0000")] }))], false).ok);
    assert!(!ed.apply(vec![DocOp::TextInsert { id: ids.head.clone(), offset: 0.0, text: "x".into(), marks: None }], false).ok);
    assert!(!ed.apply(vec![DocOp::SetText { id: ids.head.clone(), body: json!("Other") }], false).ok);
    assert!(ed.apply(vec![DocOp::Xf { id: ids.head.clone(), xf: xfv(20.0, 200.0, 300.0, 60.0) }], false).ok);
    assert!(ed.apply(vec![DocOp::TextFormat { id: ids.head.clone(), marks: obj(json!({ "weight": 700 })), offset: None, len: None }], false).ok);
    assert!(!ed.apply(vec![upd(&ids.boxx, json!({ "fills": [solid("#00ff00")] }))], false).ok);
    assert!(!ed.apply(vec![upd(&ids.boxx, json!({ "opacity": 0.5 }))], false).ok);
    assert!(ed.apply(vec![upd(&ids.boxx, json!({ "name": "Box" }))], false).ok);
    assert!(!ed.apply(vec![upd(&ids.boxx, json!({ "lock": null }))], false).ok);
    ed.select(std::slice::from_ref(&ids.boxx), false);
    ed.toggle_lock();
    ed.set_brand_lock(None);
    let b = el(&ed, &ids.boxx);
    assert_eq!(b.base().unwrap().lock.as_ref().unwrap().by.as_str(), "brand");
    let Element::Shape(s) = b else { panic!() };
    assert_eq!(s.fills.len(), 1);
}

#[test]
fn skips_brand_locked_elements_in_a_mixed_selection_instead_of_failing_the_batch() {
    let (mut ed, ids) = setup(false);
    ed.select(&[ids.boxx.clone(), ids.free.clone()], false);
    ed.update_selected(&obj(json!({ "opacity": 0.4 })), false, true);
    assert_eq!(el(&ed, &ids.free).base().unwrap().opacity, 0.4);
    assert_eq!(el(&ed, &ids.boxx).base().unwrap().opacity, 1.0);
}

#[test]
fn separates_words_from_styling_in_rich_text_and_filters_batch_commands() {
    let (mut ed, ids) = setup(true);
    ed.select(std::slice::from_ref(&ids.head), false);
    ed.set_brand_lock(Some(BrandLockLevels { style: true, ..Default::default() }));
    ed.set_brand_admin(false);
    let head = elements_json(&ed)[&ids.head].clone();
    assert!(ed.apply(vec![DocOp::TextInsert { id: ids.head.clone(), offset: 0.0, text: "Hi ".into(), marks: None }], false).ok);
    let mut restyled = head["body"].clone();
    restyled["defaultRun"]["family"] = json!("Lora");
    assert!(!ed.apply(vec![upd(&ids.head, json!({ "body": restyled }))], false).ok);
    let mut after = head.clone();
    after["body"] = restyled;
    let before_m: Map<String, Value> = [(ids.head.clone(), head.clone())].into_iter().collect();
    let after_m: Map<String, Value> = [(ids.head.clone(), after)].into_iter().collect();
    assert_eq!(brand_lock_violations(&before_m, &after_m).len(), 1);
    let mut typed = head.clone();
    typed["body"]["paragraphs"][0]["runs"][0]["text"] = json!("Other words");
    let typed_m: Map<String, Value> = [(ids.head.clone(), typed)].into_iter().collect();
    assert_eq!(brand_lock_violations(&before_m, &typed_m).len(), 0);
    let r = ed.apply(
        vec![upd(&ids.boxx, json!({ "fills": [solid("#123456")] })), upd(&ids.free, json!({ "fills": [solid("#123456")] }))],
        false,
    );
    assert!(r.ok);
    let fill0 = |id: &str| match el(&ed, id) {
        Element::Shape(s) => oc_schema::js::to_value(&s.fills[0]).unwrap(),
        _ => panic!(),
    };
    let want = solid("#123456");
    let close = |a: &Value, b: &Value| {
        ["r", "g", "b", "a"].iter().all(|k| (a["color"][k].as_f64().unwrap() - b["color"][k].as_f64().unwrap()).abs() < 1e-12)
    };
    assert!(close(&fill0(&ids.free), &want));
    assert!(!close(&fill0(&ids.boxx), &want));
}

#[test]
fn lets_brand_admins_edit_and_unlock() {
    let (mut ed, ids) = setup(true);
    assert!(ed.apply(vec![upd(&ids.boxx, json!({ "fills": [solid("#00ff00")] }))], false).ok);
    assert!(ed.apply(vec![DocOp::TextInsert { id: ids.head.clone(), offset: 0.0, text: "New ".into(), marks: None }], false).ok);
    ed.select(std::slice::from_ref(&ids.head), false);
    ed.set_brand_lock(None);
    assert!(el(&ed, &ids.head).base().unwrap().lock.is_none());
    assert!(text_el(&ed, &ids.head).body.paragraphs[0].runs[0].text.starts_with("New"));
}

#[test]
fn detects_violations_in_raw_document_changes() {
    let (ed, ids) = setup(true);
    let before = elements_json(&ed);
    let mut after = before.clone();
    after[&ids.boxx]["fills"] = json!([solid("#000000")]);
    after[&ids.logo]["xf"] = xfv(0.0, 0.0, 100.0, 100.0);
    after[&ids.free]["fills"] = json!([solid("#000000")]);
    let mut v: Vec<String> = brand_lock_violations(&before, &after).into_iter().map(|x| x.id).collect();
    v.sort();
    let mut want = vec![ids.boxx.clone(), ids.logo.clone()];
    want.sort();
    assert_eq!(v, want);
    after.remove(&ids.head);
    assert!(!brand_lock_violations(&before, &after).iter().any(|x| x.id == ids.head));
}
