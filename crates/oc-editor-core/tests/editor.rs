// SPDX-License-Identifier: Apache-2.0
//! Port of packages/editor-core/src/editor.test.ts.

mod common;

use common::*;
use oc_editor_core::{Arrange, FindOpts, GeometryPatch, HitTarget, Handle, Mods};
use oc_geometry::{AlignEdge, world_bounds};
use oc_schema::design::Surface;
use oc_schema::elements::Element;
use oc_schema::text::RunStyleOverride;
use oc_schema::tree::sorted_pages;
use serde_json::{Value, json};

const NO: Mods = Mods::NONE;

fn setup() -> (oc_editor_core::Editor, String) {
    let (mut ed, page) = setup_with(1000.0, 800.0, 2, None, vp(0.0, 0.0, 1200.0, 900.0));
    ed.set_text_service(Some(Box::new(FakeText)));
    (ed, page)
}

#[test]
fn inserts_centred_moves_with_snapping_commits_once_and_undoes_in_one_step() {
    let (mut ed, _) = setup();
    let id = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], None, true, false)[0].clone();
    assert_eq!(exf(&ed, &id).x, 450.0);
    ed.store_mut().checkpoint();
    ed.pointer_down([500.0, 450.0], NO, HitTarget::Canvas, 0);
    ed.pointer_move([520.0, 460.0], NO, 0.5);
    ed.pointer_move([577.0, 470.0], NO, 0.5);
    assert_eq!(exf(&ed, &id).x, 450.0, "not committed yet");
    assert_eq!(ed.view().el(&id).unwrap().xf().unwrap().x, 527.0);
    ed.pointer_up();
    assert_eq!(exf(&ed, &id).x, 527.0);
    assert_eq!(exf(&ed, &id).y, 370.0);
    ed.undo();
    assert_eq!(exf(&ed, &id).x, 450.0);
}

#[test]
fn snaps_to_the_page_centre() {
    let (mut ed, _) = setup();
    let id = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], Some([100.0, 100.0]), true, false)[0].clone();
    ed.pointer_down([100.0, 100.0], NO, HitTarget::Canvas, 0);
    ed.pointer_move([497.0, 100.0], NO, 0.5);
    ed.pointer_up();
    assert_eq!(exf(&ed, &id).x, 450.0);
}

#[test]
fn resizes_from_a_corner_keeping_the_opposite_corner_fixed() {
    let (mut ed, _) = setup();
    let id = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], Some([150.0, 150.0]), true, false)[0].clone();
    ed.pointer_down([200.0, 200.0], NO, HitTarget::Handle { handle: Handle::Se }, 0);
    ed.pointer_move([300.0, 250.0], NO, 0.5);
    ed.pointer_up();
    let x = exf(&ed, &id);
    assert_eq!([x.x, x.y, x.w, x.h], [100.0, 100.0, 200.0, 150.0]);
}

#[test]
fn groups_and_ungroups_without_moving_anything_on_the_page() {
    let (mut ed, _) = setup();
    let a = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 50.0))], Some([100.0, 100.0]), true, false)[0].clone();
    let mut bx = xf(0.0, 0.0, 80.0, 80.0);
    bx.rot = 30.0;
    let b = ed.insert(vec![shape(bx)], Some([400.0, 300.0]), true, false)[0].clone();
    let wb = |ed: &oc_editor_core::Editor, id: &str| world_bounds(ed.doc().els(), id).unwrap();
    let before = [wb(&ed, &a), wb(&ed, &b)];
    ed.select(&[a.clone(), b.clone()], false);
    ed.group();
    let g = ed.state.selection[0].clone();
    assert!(matches!(el(&ed, &g), Element::Group(_)));
    assert_eq!(el(&ed, &a).parent_id(), Some(g.as_str()));
    let mid = [wb(&ed, &a), wb(&ed, &b)];
    for (r, b0) in mid.iter().zip(&before) {
        assert!((r.x - b0.x).abs() < 1e-6);
    }
    ed.ungroup();
    let after = [wb(&ed, &a), wb(&ed, &b)];
    for (r, b0) in after.iter().zip(&before) {
        assert!((r.x - b0.x).abs() < 1e-6 && (r.y - b0.y).abs() < 1e-6);
    }
    assert!(el(&ed, &g).is_deleted());
}

#[test]
fn duplicates_copies_and_pastes_with_fresh_ids() {
    let (mut ed, _) = setup();
    let a = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 50.0))], None, true, false)[0].clone();
    ed.duplicate();
    let d = ed.state.selection[0].clone();
    assert_ne!(d, a);
    assert_eq!(exf(&ed, &d).x, exf(&ed, &a).x + 20.0);
    ed.select(std::slice::from_ref(&a), false);
    let clip = ed.copy().unwrap();
    ed.paste(Some(&clip.clone()), 20.0);
    let p = ed.state.selection[0].clone();
    assert_ne!(p, a);
    assert_eq!(ed.doc().els().values().filter(|e| !e.is_deleted()).count(), 3);
}

#[test]
fn edits_text_through_the_caret_model() {
    let (mut ed, _) = setup();
    let id = ed.insert(vec![text(xf(0.0, 0.0, 300.0, 20.0), "Hello")], Some([150.0, 10.0]), true, false)[0].clone();
    ed.start_text_edit(&id, None, false);
    let e = ed.state.editing_text.clone().unwrap();
    assert_eq!((e.id.as_str(), e.anchor, e.focus), (id.as_str(), 0, 5));
    ed.move_caret(oc_editor_core::CaretKey::Right, false, false);
    ed.type_text(" world");
    ed.type_text("\nLine 2");
    let t = |ed: &oc_editor_core::Editor| ed.plain_text(&text_el(ed, &id));
    assert_eq!(t(&ed), "Hello world\nLine 2");
    // autoHeight followed the second paragraph
    assert_eq!(exf(&ed, &id).h, 40.0);
    ed.delete_text(-1, true);
    assert_eq!(t(&ed), "Hello world\nLine ");
    ed.select_all();
    ed.set_text_style(&obj(json!({ "weight": 700 })));
    let body = text_el(&ed, &id).body;
    assert!(body.paragraphs.iter().all(|p| p.runs.iter().all(|r| r.style.weight == Some(700.0))));
}

#[test]
fn aligns_to_the_page_and_arranges_z_order() {
    let (mut ed, _) = setup();
    let a = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 50.0))], Some([300.0, 300.0]), true, false)[0].clone();
    let b = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 50.0))], Some([600.0, 300.0]), true, false)[0].clone();
    ed.select(std::slice::from_ref(&a), false);
    ed.align(AlignEdge::Left);
    assert_eq!(exf(&ed, &a).x, 0.0);
    ed.arrange(Arrange::Front);
    assert!(el(&ed, &a).z().unwrap() > el(&ed, &b).z().unwrap());
    ed.select(&[a.clone(), b.clone()], false);
    ed.align(AlignEdge::Top);
    assert_eq!(exf(&ed, &a).y, exf(&ed, &b).y);
}

#[test]
fn manages_pages() {
    let (mut ed, _) = setup();
    let n = |ed: &oc_editor_core::Editor| sorted_pages(&ed.doc().doc.pages).len();
    let before = n(&ed);
    ed.add_page_after_active();
    assert_eq!(n(&ed), before + 1);
    ed.duplicate_page(None);
    assert_eq!(n(&ed), before + 2);
    ed.delete_page(None);
    assert_eq!(n(&ed), before + 1);
}

#[test]
fn handles_keyboard_shortcuts() {
    let (mut ed, _) = setup();
    let a = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 50.0))], None, true, false)[0].clone();
    let x0 = exf(&ed, &a).x;
    ed.handle_key("ArrowRight", Mods::shift(), None);
    assert_eq!(exf(&ed, &a).x, x0 + 10.0);
    ed.handle_key("z", Mods::meta(), None);
    assert_eq!(exf(&ed, &a).x, x0);
    ed.handle_key("Backspace", NO, None);
    assert!(el(&ed, &a).is_deleted());
}

#[test]
fn option_shortcuts_match_by_code_when_macos_changes_the_key() {
    // Z-02: ⌥⌘[ on a Mac arrives as key "“" with code "BracketLeft".
    let (mut ed, _) = setup();
    let a = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 50.0))], None, true, false)[0].clone();
    let b = ed.insert(vec![shape(xf(20.0, 0.0, 100.0, 50.0))], None, true, false)[0].clone();
    let opt_cmd = Mods { alt: true, meta: true, ..NO };
    ed.select(std::slice::from_ref(&b), false);
    let z = |ed: &oc_editor_core::Editor, id: &str| el(ed, id).base().unwrap().z.clone();
    assert!(ed.handle_key("\u{201c}", opt_cmd, Some("BracketLeft")));
    assert!(z(&ed, &b) < z(&ed, &a));
    assert!(ed.handle_key("\u{2018}", opt_cmd, Some("BracketRight")));
    assert!(z(&ed, &b) > z(&ed, &a));
    assert!(!ed.handle_key("\u{201c}", opt_cmd, None));
}

#[test]
fn quick_add_t_r_c_and_l_insert_text_rectangle_circle_and_line() {
    let (mut ed, _) = setup_with(800.0, 600.0, 1, None, oc_editor_core::Viewport::default());
    let before = ed.doc().els().len();
    for k in ["r", "c", "l"] {
        assert!(ed.handle_key(k, NO, None));
    }
    let doc = ed.doc();
    assert_eq!(doc.els().len(), before + 3);
    assert!(doc.els().values().any(|e| matches!(e, Element::Shape(s) if s.kind == oc_schema::elements::ShapeKind::Ellipse)));
    assert!(doc.els().values().any(|e| matches!(e, Element::Line(_))));
    assert!(ed.handle_key("t", NO, None));
    assert!(ed.state.editing_text.is_some());
}

fn at(ed: &oc_editor_core::Editor, page: &str, x: f64, y: f64) -> [f64; 2] {
    let [ox, oy] = ed.page_origin(page);
    [ox + x, oy + y]
}

#[test]
fn alt_drag_duplicates_and_leaves_the_original_in_place() {
    let (mut ed, page) = setup();
    let id = ed.insert(vec![shape(xf(100.0, 100.0, 100.0, 100.0))], Some([150.0, 150.0]), true, false)[0].clone();
    ed.pointer_down(at(&ed, &page, 150.0, 150.0), Mods::alt(), HitTarget::Canvas, 0);
    ed.pointer_move(at(&ed, &page, 450.0, 360.0), Mods::alt(), 0.5);
    ed.pointer_up();
    let doc = ed.doc();
    let shapes: Vec<&Element> = doc.els().values().filter(|e| matches!(e, Element::Shape(_)) && !e.is_deleted()).collect();
    assert_eq!(shapes.len(), 2);
    assert_eq!(exf(&ed, &id).x, 100.0);
    let copy = shapes.iter().find(|s| s.id() != Some(id.as_str())).unwrap();
    assert!(copy.xf().unwrap().x > 300.0);
    assert_eq!(ed.state.selection, vec![copy.id().unwrap().to_string()]);
}

fn require_line() -> serde_json::Map<String, Value> {
    obj(json!({ "id": "line1", "type": "line", "parentId": "", "z": "a0", "opacity": 1, "xf": { "x": 0, "y": 0, "w": 100, "h": 1, "rot": 0 },
        "points": [[0, 0], [100, 0]],
        "stroke": { "paint": { "k": "solid", "color": { "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } }, "width": 2, "align": "center", "cap": "round", "join": "round" },
        "startMarker": "none", "endMarker": "arrow" }))
}

#[test]
fn connectors_stay_glued_when_the_shape_moves_and_copy_style_transfers_fills() {
    let (mut ed, _) = setup();
    let a = ed.insert(vec![shape_fill(xf(0.0, 0.0, 100.0, 100.0), "#ff0000")], Some([100.0, 100.0]), true, false)[0].clone();
    let b = ed.insert(vec![shape_fill(xf(0.0, 0.0, 100.0, 100.0), "#00ff00")], Some([500.0, 100.0]), true, false)[0].clone();
    let mut line = require_line();
    line.insert("startAttach".into(), json!({ "id": a, "anchor": "right" }));
    line.insert("endAttach".into(), json!({ "id": b, "anchor": "left" }));
    let l = ed.insert(vec![line], Some([300.0, 100.0]), true, false)[0].clone();
    // Moving b drags the connector's end with it.
    ed.select(std::slice::from_ref(&b), false);
    ed.nudge(0.0, 200.0);
    let Element::Line(ln) = el(&ed, &l) else { panic!() };
    let end = ln.base.xf.y + ln.points.last().unwrap()[1];
    assert!((end - 300.0).abs() < 0.5, "end {end}");
    // Copy style from a to b.
    ed.select(std::slice::from_ref(&a), false);
    ed.copy_style();
    let st = ed.state.style_brush.clone();
    ed.paste_style(std::slice::from_ref(&b), st.as_ref());
    let fills = |id: &str| match el(&ed, id) {
        Element::Shape(s) => serde_json::to_string(&s.fills).unwrap(),
        _ => panic!(),
    };
    assert_eq!(fills(&b), fills(&a));
}

#[test]
fn finds_and_replaces_across_pages_in_one_undo_step() {
    let (mut ed, _) = setup();
    ed.insert(vec![text(xf(0.0, 0.0, 400.0, 20.0), "Hello world, hello moon")], None, true, false);
    let second = sorted_pages(&ed.doc().doc.pages)[1].id.clone();
    ed.set_active_page(&second, false);
    ed.insert(vec![text(xf(0.0, 0.0, 400.0, 20.0), "HELLO again")], None, true, false);
    assert_eq!(ed.find("hello", &FindOpts::default()).len(), 3);
    assert_eq!(ed.find("hello", &FindOpts { match_case: true, ..Default::default() }).len(), 1);
    assert_eq!(ed.replace("hello", "Goodbye", &FindOpts::default()), 3);
    let doc = ed.doc();
    let mut texts: Vec<String> = doc
        .els()
        .values()
        .filter_map(|e| match e {
            Element::Text(t) => Some(t.body.paragraphs[0].runs.iter().map(|r| r.text.as_str()).collect()),
            _ => None,
        })
        .collect();
    texts.sort();
    assert_eq!(texts, vec!["Goodbye again", "Goodbye world, Goodbye moon"]);
    ed.undo();
    assert_eq!(ed.find("hello", &FindOpts::default()).len(), 3);
}

#[test]
fn sets_exact_geometry_and_fills_grid_cells_by_drop() {
    let (mut ed, page) = setup();
    let id = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], None, true, false)[0].clone();
    ed.set_geometry(GeometryPatch { x: Some(10.0), y: Some(20.0), w: Some(300.0), ..Default::default() });
    let x = exf(&ed, &id);
    assert_eq!([x.x, x.y, x.w], [10.0, 20.0, 300.0]);
    let grid = oc_schema::factory::make_grid(
        oc_schema::factory::base_init(xf(0.0, 0.0, 400.0, 200.0)),
        oc_schema::factory::GridOpts { layout: Some("cols-2".into()), gap: Some(0.0), ..Default::default() },
    );
    let g = ed.insert(vec![to_map(Element::Grid(grid))], Some([200.0, 100.0]), true, false)[0].clone();
    assert!(ed.drop_into_container(&page, [300.0, 100.0], "asset1"));
    let Element::Grid(gr) = el(&ed, &g) else { panic!() };
    let assets: Vec<Option<String>> = gr.cells.iter().map(|c| c.asset.clone()).collect();
    assert_eq!(assets, vec![None, Some("asset1".to_string())]);
}

#[test]
fn an_old_whiteboard_opens_as_a_normal_design() {
    let (mut ed, _) = setup_with(2000.0, 1500.0, 1, Some(Surface::Whiteboard), vp(0.0, 0.0, 1200.0, 900.0));
    ed.set_text_service(Some(Box::new(FakeText)));
    let id = ed.insert(vec![shape(xf(0.0, 0.0, 200.0, 200.0))], Some([1000.0, 750.0]), true, false)[0].clone();
    ed.select(std::slice::from_ref(&id), false);
    ed.nudge(850.0, 0.0);
    assert_eq!(ed.doc().doc.meta.dims.w, 2000.0);
    let before = ed.doc().els().len();
    ed.handle_key("Tab", NO, None);
    assert_eq!(ed.doc().els().len(), before);
}

#[test]
fn double_clicking_a_shape_adds_a_label_grouped_with_it() {
    let (mut ed, page) = setup();
    let id = ed.insert(vec![shape(xf(0.0, 0.0, 300.0, 200.0))], Some([400.0, 300.0]), true, false)[0].clone();
    let [ox, oy] = ed.page_origin(&page);
    ed.double_click([ox + 400.0, oy + 300.0]);
    let shape_el = el(&ed, &id);
    let group = el(&ed, shape_el.parent_id().unwrap());
    assert!(matches!(group, Element::Group(_)));
    assert!(ed.state.editing_text.is_some());
    ed.type_text("Idea");
    let doc = ed.doc();
    let label = doc
        .els()
        .values()
        .find_map(|e| match e {
            Element::Text(t) if t.base.name.as_deref() == Some("Shape label") => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(label.body.paragraphs[0].runs.iter().map(|r| r.text.as_str()).collect::<String>(), "Idea");
    assert_eq!(Some(label.base.parent_id.as_str()), group.id());
}

#[test]
fn bulk_create_clones_the_page_once_per_row_with_bound_text_filled_in() {
    let (mut ed, page) = setup();
    let name = ed.insert(
        vec![text_styled(xf(0.0, 0.0, 300.0, 20.0), "NAME", RunStyleOverride { size: Some(30.0), weight: Some(700.0), ..Default::default() })],
        None,
        true,
        false,
    )[0]
    .clone();
    ed.insert(vec![text(xf(0.0, 40.0, 300.0, 20.0), "static")], None, true, false);
    ed.set_data_binding(&name, Some("Name"));
    let before = sorted_pages(&ed.doc().doc.pages).len();
    let rows: Vec<indexmap::IndexMap<String, String>> = vec![
        [("Name".to_string(), "Ada".to_string())].into_iter().collect(),
        [("Name".to_string(), "Grace\nHopper".to_string())].into_iter().collect(),
    ];
    let pages = ed.bulk_create(&page, &rows, &Default::default());
    assert_eq!(pages.len(), 2);
    assert_eq!(sorted_pages(&ed.doc().doc.pages).len(), before + 2);
    let texts = |pid: &str| {
        let doc = ed.doc();
        let mut v: Vec<String> = doc
            .els()
            .values()
            .filter_map(|e| match e {
                Element::Text(t) if t.base.parent_id == pid => {
                    Some(t.body.paragraphs.iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("|"))
                }
                _ => None,
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(texts(&pages[0]), vec!["Ada", "static"]);
    assert_eq!(texts(&pages[1]), vec!["Grace|Hopper", "static"]);
    let doc = ed.doc();
    let bound = doc
        .els()
        .values()
        .find_map(|e| match e {
            Element::Text(t) if t.base.parent_id == pages[0] && t.body.paragraphs[0].runs[0].text == "Ada" => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(bound.body.paragraphs[0].runs[0].style.weight.unwrap_or(bound.body.default_run.weight), 700.0);
}
