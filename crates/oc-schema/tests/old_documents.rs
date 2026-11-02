// SPDX-License-Identifier: Apache-2.0
//! Stored documents from before sheets, whiteboards, email designs, brand-template locks, beat sync and sticky notes were retired
//! still parse.

use oc_schema::DesignDoc;
use oc_schema::design::Surface;
use oc_schema::elements::Element;
use oc_schema::factory::{MetaInit, empty_design};
use serde_json::{Value, json};

fn base(id: &str, kind: &str) -> Value {
    json!({ "id": id, "type": kind, "parentId": "p", "z": "a0", "opacity": 1, "xf": { "x": 0, "y": 0, "w": 100, "h": 100, "rot": 0 } })
}

fn doc(surface: &str, elements: Value) -> Value {
    let mut d = oc_schema::js::to_value(&empty_design(&MetaInit { w: 100.0, h: 100.0, ..Default::default() })).unwrap();
    d["meta"]["surface"] = json!(surface);
    d["elements"] = elements;
    d
}

#[test]
fn the_retired_surfaces_still_parse() {
    for (name, surface) in [("sheet", Surface::Sheet), ("whiteboard", Surface::Whiteboard), ("email", Surface::Email)] {
        let d: DesignDoc = serde_json::from_value(doc(name, json!({}))).expect(name);
        assert_eq!(d.meta.surface, surface);
    }
}

#[test]
fn a_chart_keeps_its_old_sheet_link_as_an_unknown_field() {
    let mut chart = base("c", "chart");
    let source = json!({ "designId": "sheet0000000000", "sheet": "t1", "range": "A1:B3", "sheetName": "Budget", "refreshedAt": 5 });
    chart["kind"] = json!("column");
    chart["labels"] = json!(["a"]);
    chart["series"] = json!([{ "name": "s", "values": [1], "color": { "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } }]);
    chart["source"] = source.clone();
    let grey = json!({ "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 });
    chart["style"] = json!({ "family": "Inter", "textColor": grey, "gridColor": grey, "showLegend": true, "showValues": false, "showGrid": true, "fontSize": 12 });
    let d: DesignDoc = serde_json::from_value(doc("graphic", json!({ "c": chart }))).expect("old chart");
    let Some(Element::Chart(c)) = d.elements.get("c") else { panic!("chart") };
    assert_eq!(c.labels, vec!["a".to_string()]);
    assert_eq!(serde_json::to_value(&d).unwrap()["elements"]["c"]["source"], source);
}

#[test]
fn elements_keep_their_section_and_mind_map_metadata() {
    let mut section = base("s", "shape");
    section["kind"] = json!("rect");
    section["params"] = json!({});
    section["fills"] = json!([]);
    section["strokes"] = json!([]);
    section["board"] = json!({ "section": { "title": "Ideas", "order": 1, "collapsed": { "h": 400, "hidden": ["n"] } } });
    let mut node = section.clone();
    node["id"] = json!("n");
    node["board"] = json!({ "mind": { "parent": "s" } });
    let d: DesignDoc = serde_json::from_value(doc("whiteboard", json!({ "s": section, "n": node }))).expect("old board");
    let out = serde_json::to_value(&d).unwrap();
    assert_eq!(out["elements"]["s"]["board"]["section"]["title"], "Ideas");
    assert_eq!(out["elements"]["n"]["board"]["mind"]["parent"], "s");
}

#[test]
fn an_element_keeps_its_old_brand_lock() {
    let mut shape = base("s", "shape");
    shape["kind"] = json!("rect");
    shape["params"] = json!({});
    shape["fills"] = json!([]);
    shape["strokes"] = json!([]);
    let lock = json!({ "by": "brand", "move": true, "resize": true, "rotate": true, "delete": true, "content": true, "style": true });
    shape["lock"] = lock.clone();
    let d: DesignDoc = serde_json::from_value(doc("graphic", json!({ "s": shape }))).expect("old brand lock");
    let Some(el) = d.elements.get("s") else { panic!("shape") };
    assert_eq!(el.base().and_then(|b| b.lock.as_ref()).map(|l| l.by.as_str()), Some("brand"));
    assert_eq!(serde_json::to_value(&d).unwrap()["elements"]["s"]["lock"], lock);
}

#[test]
fn a_design_keeps_its_captions_and_old_beat_markers() {
    let mut d = doc("video", json!({}));
    let cue = json!({ "id": "c1", "startMs": 0, "endMs": 900, "text": "Hello" });
    d["meta"]["captions"] = json!({ "visible": true, "cues": [cue], "style": { "family": "Inter", "size": 40, "color": "#ffffff", "background": "#000000", "position": "bottom" } });
    d["meta"]["beats"] = json!([480, 1010]);
    let parsed: DesignDoc = serde_json::from_value(d).expect("old video");
    assert_eq!(parsed.meta.captions.as_ref().map(|c| c.cues.len()), Some(1));
    assert_eq!(serde_json::to_value(&parsed).unwrap()["meta"]["beats"], json!([480, 1010]));
}

#[test]
fn a_sticky_note_stays_an_ordinary_group_with_its_metadata() {
    let mut group = base("g", "group");
    let sticky = json!({ "color": "#fef08a", "author": { "id": "u1", "name": "Ada" } });
    group["name"] = json!("Sticky note");
    group["board"] = json!({ "sticky": sticky });
    let mut square = base("s", "shape");
    square["parentId"] = json!("g");
    square["kind"] = json!("rect");
    square["params"] = json!({ "cornerRadius": 4 });
    square["fills"] = json!([]);
    square["strokes"] = json!([]);
    let d: DesignDoc = serde_json::from_value(doc("graphic", json!({ "g": group, "s": square }))).expect("old sticky note");
    assert!(matches!(d.elements.get("g"), Some(Element::Group(_))));
    assert!(matches!(d.elements.get("s"), Some(Element::Shape(_))));
    assert_eq!(serde_json::to_value(&d).unwrap()["elements"]["g"]["board"]["sticky"], sticky);
}
