// SPDX-License-Identifier: Apache-2.0
//! Port of packages/editor-core/src/table.test.ts.

mod common;

use common::*;
use oc_editor_core::{Editor, HitTarget, Mods, TextEditState};
use oc_schema::elements::{Element, TableEl};
use oc_schema::factory::{TableOpts, base_init, make_table};
use oc_schema::table::{BorderPreset, TableStylePreset, cell_body};
use serde_json::json;

const NO: Mods = Mods::NONE;

fn setup() -> (Editor, String) {
    let (mut ed, page) = setup_with(1000.0, 800.0, 1, None, vp(0.0, 0.0, 1200.0, 900.0));
    ed.set_text_service(Some(Box::new(WrapText)));
    // 3×3 table at (100,100), 300×150 → 100×50 cells, padding 10.
    let data = vec![vec!["A".into(), "B".into(), "C".into()], vec![String::new(); 3], vec![String::new(); 3]];
    let t = make_table(base_init(xf(100.0, 100.0, 300.0, 150.0)), TableOpts { data: Some(data), ..Default::default() });
    let id = t.base.id.clone();
    ed.apply(vec![oc_crdt::DocOp::Add { el: serde_json::Value::Object(to_map(Element::Table(t))), parent: page, after: None }], false);
    (ed, id)
}

fn table(ed: &Editor, id: &str) -> TableEl {
    match el(ed, id) {
        Element::Table(t) => t,
        _ => panic!(),
    }
}

#[test]
fn types_rich_text_into_a_cell_through_the_shared_text_engine() {
    let (mut ed, id) = setup();
    ed.start_cell_edit(&id, 1, 1, None, false);
    assert_eq!(ed.state.editing_text.as_ref().unwrap().id, id);
    assert!(ed.editing_table_text());
    ed.type_text("Hello world");
    ed.state.editing_text = Some(TextEditState { id: id.clone(), anchor: 0, focus: 5 });
    ed.set_text_style(&obj(json!({ "weight": 700, "italic": true })));
    let cell = table(&ed, &id).cells["1:1"].clone();
    assert_eq!(cell.text, "Hello world");
    let run0 = serde_json::to_value(&cell.body.unwrap().paragraphs[0].runs[0]).unwrap();
    assert_eq!(run0, json!({ "text": "Hello", "style": { "weight": 700.0, "italic": true } }));
    assert_eq!(ed.current_text_style().unwrap()["weight"].as_f64(), Some(700.0));
    let fr = ed.editing_text_frame().unwrap();
    assert_eq!(fr.host_id, id);
    assert_eq!(fr.ox, 110.0);
    ed.state.editing_text = Some(TextEditState { id: id.clone(), anchor: 11, focus: 11 });
    ed.delete_text(-1, false);
    assert_eq!(table(&ed, &id).cells["1:1"].text, "Hello worl");
}

#[test]
fn grows_the_row_as_text_wraps() {
    let (mut ed, id) = setup();
    ed.start_cell_edit(&id, 1, 0, None, false);
    let before = table(&ed, &id).base.xf.h;
    ed.type_text("a long line that wraps over many lines");
    assert!(table(&ed, &id).base.xf.h > before);
    let t = table(&ed, &id);
    let sum: f64 = t.rows.iter().sum();
    assert!((t.rows[1] / sum) * t.base.xf.h > 50.0);
    assert!(((t.rows[0] / sum) * t.base.xf.h - 50.0).abs() < 0.005);
}

#[test]
fn selects_a_cell_range_with_shift_click_merges_and_unmerges_it() {
    let (mut ed, id) = setup();
    ed.double_click([150.0, 125.0]);
    let ec = ed.state.editing_cell.clone().unwrap();
    assert_eq!((ec.id.as_str(), ec.r, ec.c), (id.as_str(), 0, 0));
    ed.pointer_down([250.0, 175.0], Mods::shift(), HitTarget::Canvas, 0);
    ed.pointer_up();
    let cr = ed.state.cell_range.clone().unwrap();
    assert_eq!((cr.id.as_str(), cr.r0, cr.c0, cr.r1, cr.c1), (id.as_str(), 0, 0, 1, 1));
    assert!(ed.state.editing_text.is_none());
    ed.set_text_style(&obj(json!({ "weight": 300 })));
    assert_eq!(cell_body(&table(&ed, &id), 0, 1).paragraphs[0].runs[0].style.weight, Some(300.0));
    ed.merge_cell_range(&id);
    let span = table(&ed, &id).cells["0:0"].span.clone().unwrap();
    assert_eq!((span.rows, span.cols), (2.0, 2.0));
    assert_eq!(table(&ed, &id).cells["0:0"].text, "A\nB");
    let ec = ed.state.editing_cell.clone().unwrap();
    assert_eq!((ec.r, ec.c), (0, 0));
    let c = ed.cell_at(&id, [250.0, 175.0]).unwrap();
    assert_eq!((c.r, c.c), (0, 0));
    ed.unmerge_cell_range(&id);
    assert!(table(&ed, &id).cells["0:0"].span.is_none());
}

#[test]
fn drags_across_cells_applies_cell_props_borders_and_styles_escape_leaves() {
    let (mut ed, id) = setup();
    ed.select(std::slice::from_ref(&id), false);
    ed.start_cell_edit(&id, 2, 0, None, false);
    ed.pointer_down([150.0, 225.0], NO, HitTarget::Canvas, 0);
    ed.pointer_move([350.0, 225.0], NO, 0.5);
    ed.pointer_up();
    let cr = ed.state.cell_range.clone().unwrap();
    assert_eq!((cr.r0, cr.c0, cr.r1, cr.c1), (2, 0, 2, 2));
    ed.set_cells(&id, &obj(json!({ "valign": "bottom", "padding": 4 })));
    let c21 = table(&ed, &id).cells["2:1"].clone();
    assert_eq!(c21.valign.unwrap().as_str(), "bottom");
    assert_eq!(c21.padding, Some(4.0));
    assert!(table(&ed, &id).cells.get("1:1").and_then(|c| c.valign.clone()).is_none());
    ed.set_cell_align(&id, "center");
    assert_eq!(table(&ed, &id).cells["2:2"].align.clone().unwrap().as_str(), "center");
    ed.set_cell_borders(&id, BorderPreset::None, None, None);
    assert_eq!(table(&ed, &id).cells["2:0"].borders.clone().unwrap().t.unwrap().width, 0.0);
    ed.set_table_style(&id, TableStylePreset::parse("header-dark"));
    assert_eq!(table(&ed, &id).style.preset.as_deref(), Some("header-dark"));
    ed.handle_key("Escape", NO, None);
    assert!(ed.state.cell_range.is_none());
    assert!(ed.state.editing_cell.is_none());
}

#[test]
fn resizes_a_column_by_dragging_its_border_and_tabs_between_cells() {
    let (mut ed, id) = setup();
    ed.select(std::slice::from_ref(&id), false);
    ed.pointer_down([200.0, 120.0], NO, HitTarget::TableCol { id: id.clone(), index: 0 }, 0);
    ed.pointer_move([250.0, 120.0], NO, 0.5);
    ed.pointer_up();
    let t = table(&ed, &id);
    let sum: f64 = t.cols.iter().sum();
    assert!(((t.cols[0] / sum) * t.base.xf.w - 150.0).abs() < 0.005);
    ed.start_cell_edit(&id, 2, 2, None, false);
    ed.cell_tab(1);
    assert_eq!(table(&ed, &id).rows.len(), 4);
    let ec = ed.state.editing_cell.clone().unwrap();
    assert_eq!((ec.id.as_str(), ec.r, ec.c), (id.as_str(), 3, 0));
}
