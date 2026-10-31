// SPDX-License-Identifier: Apache-2.0
//! Path elements (DS-100) and masks (DS-107) round-trip through the yrs document and converge between two peers
//! editing them concurrently.

use oc_crdt::rng::Mulberry32;
use oc_crdt::{ApplyOptions, DocOp, apply_ops, apply_update, encode_diff, encode_state_vector, from_y_value, new_doc_with_client};
use oc_schema::elements::{Element, MaskMode};
use serde_json::{Value, json};
use yrs::Doc;

fn apply(d: &Doc, rng: &mut Mulberry32, v: Value) {
    let ops: Vec<DocOp> = serde_json::from_value(v).unwrap();
    let r = apply_ops(d, &ops, None, ApplyOptions { now: Some(1_700_000_000_000.0), rng: Some(rng), ..Default::default() });
    assert!(r.ok, "{r:?}");
}

struct Els(serde_json::Map<String, Value>);

impl Els {
    fn get(&self, id: &str) -> Option<Element> {
        self.0.get(id).map(|v| serde_json::from_value(v.clone()).expect("element parses"))
    }
}

struct Design {
    elements: Els,
}

fn design(d: &Doc) -> Design {
    let v = from_y_value(d);
    Design { elements: Els(v["elements"].as_object().cloned().unwrap_or_default()) }
}

fn shape(id: &str, parent: &str, kind: &str, d: Option<&str>) -> Value {
    let mut v = json!({ "id": id, "type": "shape", "parentId": parent, "z": "a0", "opacity": 1,
        "xf": { "x": 10, "y": 10, "w": 100, "h": 80, "rot": 0 }, "kind": kind, "params": {},
        "fills": [{ "k": "solid", "color": { "space": "srgb", "r": 1, "g": 0, "b": 0, "a": 1 } }], "strokes": [] });
    if let Some(d) = d {
        v["d"] = json!(d);
    }
    v
}

#[test]
fn paths_and_masks_round_trip_and_converge() {
    let base = new_doc_with_client(1);
    let mut rng = Mulberry32::new(7);
    apply(&base, &mut rng, json!([{ "op": "page", "action": "add", "page": { "id": "page000000000001" } }]));
    let group = json!({ "id": "grp0000000000001", "type": "group", "parentId": "page000000000001", "z": "a0", "opacity": 1,
        "xf": { "x": 0, "y": 0, "w": 200, "h": 200, "rot": 0 }, "mask": { "id": "msk0000000000001", "mode": "alpha" } });
    apply(&base, &mut rng, json!([
        { "op": "add", "el": group, "parent": "page000000000001" },
        { "op": "add", "el": shape("img0000000000001", "grp0000000000001", "rect", None), "parent": "grp0000000000001" },
        { "op": "add", "el": shape("msk0000000000001", "grp0000000000001", "custom", Some("M0 1 L0.5 0 C0.75 0.5 1 0.75 1 1 Z")), "parent": "grp0000000000001" },
    ]));
    let d = design(&base);
    let Some(Element::Group(g)) = d.elements.get("grp0000000000001") else { panic!("group") };
    let m = g.mask.as_ref().expect("mask survives the yrs document");
    assert_eq!((m.id.as_str(), &m.mode), ("msk0000000000001", &MaskMode::Alpha));
    let Some(Element::Shape(s)) = d.elements.get("msk0000000000001") else { panic!("path") };
    assert_eq!(s.d.as_deref(), Some("M0 1 L0.5 0 C0.75 0.5 1 0.75 1 1 Z"));

    // Two peers from the same state: one switches the mask to luminance, the other edits the path's nodes.
    let st = oc_crdt::encode_state(&base);
    let peer = |id: u64| {
        let p = new_doc_with_client(id);
        apply_update(&p, &st, None).unwrap();
        p
    };
    let (a, b) = (peer(2), peer(3));
    apply(&a, &mut rng, json!([{ "op": "update", "id": "grp0000000000001", "patch": { "mask": { "id": "msk0000000000001", "mode": "luminance" } } }]));
    apply(&b, &mut rng, json!([{ "op": "update", "id": "msk0000000000001", "patch": { "d": "M0 0 L1 0 L1 1 Z" } }]));
    let sv = encode_state_vector(&base);
    let (ua, ub) = (encode_diff(&a, &sv).unwrap(), encode_diff(&b, &sv).unwrap());
    apply_update(&a, &ub, None).unwrap();
    apply_update(&b, &ua, None).unwrap();
    assert_eq!(from_y_value(&a), from_y_value(&b), "peers converge");
    let d = design(&a);
    let Some(Element::Group(g)) = d.elements.get("grp0000000000001") else { panic!("group") };
    assert_eq!(g.mask.as_ref().map(|m| &m.mode), Some(&MaskMode::Luminance));
    let Some(Element::Shape(s)) = d.elements.get("msk0000000000001") else { panic!("path") };
    assert_eq!(s.d.as_deref(), Some("M0 0 L1 0 L1 1 Z"));

    // Releasing the mask (null) removes the field.
    apply(&a, &mut rng, json!([{ "op": "update", "id": "grp0000000000001", "patch": { "mask": null } }]));
    let Some(Element::Group(g)) = design(&a).elements.get("grp0000000000001") else { panic!("group") };
    assert!(g.mask.is_none());
}
