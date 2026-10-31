// SPDX-License-Identifier: Apache-2.0
//! Plan 11 batch 3: page lock (g96), the stroke eraser (g55) and document resize for "Copy and resize" (g101).

mod common;

use common::*;
use oc_editor_core::{Editor, HitTarget, Mods, Tool, resize_doc};
use oc_schema::elements::Element;
use oc_schema::factory::{MetaInit, empty_design};
use oc_schema::tree::sorted_pages;
use serde_json::{Map, Value, json};

const NO: Mods = Mods::NONE;

fn setup() -> (Editor, String) {
    let (mut ed, page) = setup_with(1000.0, 800.0, 2, None, vp(0.0, 0.0, 1200.0, 900.0));
    ed.set_text_service(Some(Box::new(FakeText)));
    (ed, page)
}

fn strokes(ed: &Editor) -> Vec<oc_schema::elements::DrawEl> {
    ed.doc()
        .els()
        .values()
        .filter_map(|e| match e {
            Element::Draw(d) if !e.is_deleted() => Some(d.clone()),
            _ => None,
        })
        .collect()
}

fn drag(ed: &mut Editor, pts: &[[f64; 2]]) {
    ed.pointer_down(pts[0], NO, HitTarget::Canvas, 0);
    for p in &pts[1..] {
        ed.pointer_move(*p, NO, 0.5);
    }
    ed.pointer_up();
}

fn draw_line(ed: &mut Editor) {
    ed.set_tool(Tool::Draw);
    let pts: Vec<[f64; 2]> = (0..=20).map(|i| [100.0 + i as f64 * 10.0, 100.0]).collect();
    drag(ed, &pts);
}

#[test]
fn the_eraser_splits_a_stroke_it_crosses_in_one_undo_step() {
    let (mut ed, _) = setup();
    draw_line(&mut ed);
    assert_eq!(strokes(&ed).len(), 1);
    ed.set_tool(Tool::Erase);
    assert_eq!(ed.state.tool, Tool::Erase);
    drag(&mut ed, &[[200.0, 60.0], [200.0, 100.0], [200.0, 140.0]]);
    let parts = strokes(&ed);
    assert_eq!(parts.len(), 2, "one stroke on each side of the cut");
    let mut boxes: Vec<(f64, f64)> = parts.iter().map(|d| (d.base.xf.x, d.base.xf.x + d.base.xf.w)).collect();
    boxes.sort_by(|a, b| a.0.total_cmp(&b.0));
    // The default brush is 6 px wide, so the boxes keep a 6 px pad around the remaining points.
    assert!(boxes[0].1 < 200.0 && boxes[0].0 < 110.0, "left piece {:?}", boxes[0]);
    assert!(boxes[1].0 > 200.0 && boxes[1].1 > 290.0, "right piece {:?}", boxes[1]);
    for d in &parts {
        // Points stay inside the fitted box.
        for c in d.points.chunks(3) {
            assert!(c[0] >= 0.0 && c[0] <= d.base_size.w && c[1] >= 0.0 && c[1] <= d.base_size.h);
        }
    }
    ed.undo();
    assert_eq!(strokes(&ed).len(), 1);
}

#[test]
fn the_eraser_deletes_a_stroke_it_covers_and_misses_strokes_elsewhere() {
    let (mut ed, page) = setup();
    draw_line(&mut ed);
    let path: Vec<f64> = (0..=40).flat_map(|i| [100.0 + i as f64 * 5.0, 100.0, 0.5]).collect();
    assert_eq!(ed.erase_strokes(&page, &[500.0, 500.0, 0.5, 600.0, 600.0, 0.5], 10.0), 0);
    assert_eq!(strokes(&ed).len(), 1);
    assert_eq!(ed.erase_strokes(&page, &path, 10.0), 1);
    assert!(strokes(&ed).is_empty());
}

#[test]
fn a_locked_page_keeps_its_elements_out_of_reach() {
    let (mut ed, page) = setup();
    let id = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], Some([150.0, 150.0]), true, false)[0].clone();
    ed.clear_selection();
    let mut lock = Map::new();
    lock.insert("locked".into(), json!(true));
    ed.update_page(&page, &lock);
    assert!(ed.doc().page_locked(&page));
    // Clicks, select-all and programmatic selection all skip it.
    ed.pointer_down([150.0, 150.0], NO, HitTarget::Canvas, 0);
    ed.pointer_up();
    assert!(ed.state.selection.is_empty());
    ed.select_all();
    assert!(ed.state.selection.is_empty());
    ed.select(std::slice::from_ref(&id), false);
    assert!(ed.state.selection.is_empty());
    // Drawing on it does nothing either.
    draw_line(&mut ed);
    assert!(strokes(&ed).is_empty());
    ed.set_tool(Tool::Select);
    let mut unlock = Map::new();
    unlock.insert("locked".into(), Value::Null);
    ed.update_page(&page, &unlock);
    ed.pointer_down([150.0, 150.0], NO, HitTarget::Canvas, 0);
    ed.pointer_up();
    assert_eq!(ed.state.selection, vec![id]);
}

#[test]
fn resizes_a_stored_document_with_or_without_scaling_its_content() {
    let d = empty_design(&MetaInit { w: 1000.0, h: 800.0, pages: Some(1), ..Default::default() });
    let page = sorted_pages(&d.pages)[0].id.clone();
    let mut doc = oc_schema::js::to_value(&d).unwrap();
    let mut el = shape(xf(100.0, 100.0, 200.0, 100.0));
    el.insert("parentId".into(), json!(page));
    el.insert("z".into(), json!("a0"));
    let id = id_of(&el);
    doc["elements"][&id] = Value::Object(el);

    let mut plain = doc.clone();
    resize_doc(&mut plain, 500.0, 500.0, None, false).unwrap();
    assert_eq!(plain["meta"]["dims"]["w"].as_f64(), Some(500.0));
    assert_eq!(plain["elements"][&id]["xf"]["x"].as_f64(), Some(100.0));

    resize_doc(&mut doc, 500.0, 500.0, Some(oc_schema::Unit::Mm), true).unwrap();
    assert_eq!(doc["meta"]["dims"]["h"].as_f64(), Some(500.0));
    assert_eq!(doc["meta"]["dims"]["unit"], json!("mm"));
    // Uniform scale 0.5, centred vertically: (500 - 400) / 2 = 50.
    let x = &doc["elements"][&id]["xf"];
    assert_eq!((x["x"].as_f64(), x["y"].as_f64(), x["w"].as_f64(), x["h"].as_f64()), (Some(50.0), Some(100.0), Some(100.0), Some(50.0)));
}
