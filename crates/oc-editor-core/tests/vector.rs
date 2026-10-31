// SPDX-License-Identifier: Apache-2.0
//! Pen tool, node editing, Boolean operations, outline conversion and masks (DS-100, DS-107): mouse and
//! keyboard paths, undo/redo, and two editors kept in sync through yrs updates.

mod common;

use common::*;
use oc_editor_core::{Editor, EditorOptions, HitTarget, ManualClock, Mods, Mulberry32, Tool};
use oc_geometry::{BoolOp, NodePart, NodeRef, VectorPath, world_bounds};
use oc_schema::elements::{Element, MaskMode, ShapeKind};
use serde_json::{Value, json};

const NO: Mods = Mods::NONE;

fn setup() -> (Editor, String) {
    let (mut ed, page) = setup_with(1000.0, 800.0, 1, None, vp(0.0, 0.0, 1200.0, 900.0));
    ed.set_text_service(Some(Box::new(FakeText)));
    (ed, page)
}

fn click(ed: &mut Editor, p: [f64; 2]) {
    ed.pointer_down(p, NO, HitTarget::Canvas, 0);
    ed.pointer_up();
}

fn shapes(ed: &Editor) -> Vec<String> {
    let d = ed.doc();
    d.els().iter().filter(|(_, e)| !e.is_deleted() && matches!(e, Element::Shape(_))).map(|(k, _)| k.clone()).collect()
}

fn shape_of(ed: &Editor, id: &str) -> oc_schema::elements::ShapeEl {
    match el(ed, id) {
        Element::Shape(s) => s,
        e => panic!("not a shape: {e:?}"),
    }
}

/// The element's outline in page space.
fn outline(ed: &Editor, id: &str) -> VectorPath {
    let e = el(ed, id);
    let p = oc_editor_core::editor::vector::shape_outline(&e).unwrap();
    p.transformed(&oc_geometry::world_matrix(ed.doc().els(), id))
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05
}

#[test]
fn pen_draws_corners_and_curves_and_closes_on_the_first_node() {
    let (mut ed, _) = setup();
    ed.set_tool(Tool::Pen);
    click(&mut ed, [100.0, 100.0]);
    // Drag out a smooth node.
    ed.pointer_down([300.0, 100.0], NO, HitTarget::Canvas, 0);
    ed.pointer_move([350.0, 150.0], NO, 0.5);
    ed.pointer_up();
    click(&mut ed, [300.0, 300.0]);
    let pen = ed.state.pen.clone().unwrap();
    assert_eq!(pen.nodes.len(), 3);
    assert_eq!(pen.nodes[1].cout, Some([350.0, 150.0]));
    assert_eq!(pen.nodes[1].cin, Some([250.0, 50.0]));
    assert!(pen.nodes[1].smooth);
    assert!(shapes(&ed).is_empty(), "nothing is written while drawing");
    // Clicking near the first node closes the path.
    click(&mut ed, [103.0, 98.0]);
    assert!(ed.state.pen.is_none());
    assert_eq!(ed.state.tool, Tool::Select);
    let ids = shapes(&ed);
    assert_eq!(ids.len(), 1);
    assert_eq!(ed.state.selection, ids);
    let s = shape_of(&ed, &ids[0]);
    assert_eq!(s.kind, ShapeKind::Custom);
    assert!(!s.fills.is_empty(), "closed paths are filled");
    let o = outline(&ed, &ids[0]);
    assert!(o.subpaths[0].closed);
    assert_eq!(o.subpaths[0].nodes.len(), 3);
    assert!(near(o.subpaths[0].nodes[0].p, [100.0, 100.0]));
    assert!(near(o.subpaths[0].nodes[1].cout.unwrap(), [350.0, 150.0]));
    // One undo step removes the path.
    ed.undo();
    assert!(shapes(&ed).is_empty());
    ed.redo();
    assert_eq!(shapes(&ed).len(), 1);
}

#[test]
fn pen_by_keyboard_and_open_paths() {
    let (mut ed, _) = setup();
    assert!(ed.handle_key("p", NO, Some("KeyP")));
    assert_eq!(ed.state.tool, Tool::Pen);
    // The cursor starts at the page centre (500, 400).
    assert!(ed.handle_key(" ", NO, Some("Space")));
    for _ in 0..5 {
        ed.handle_key("ArrowRight", Mods::shift(), None);
    }
    ed.handle_key(" ", NO, None);
    ed.handle_key("ArrowDown", Mods::shift(), None);
    ed.handle_key(" ", NO, None);
    // Backspace drops the last node, Space puts it back.
    ed.handle_key("Backspace", NO, None);
    assert_eq!(ed.state.pen.as_ref().unwrap().nodes.len(), 2);
    ed.handle_key(" ", NO, None);
    // "c" does not quick-add a circle while drawing; Enter finishes an open path.
    assert!(ed.handle_key("Enter", NO, None));
    let ids = shapes(&ed);
    assert_eq!(ids.len(), 1);
    let s = shape_of(&ed, &ids[0]);
    assert!(s.fills.is_empty() && !s.strokes.is_empty(), "open paths are stroked");
    let o = outline(&ed, &ids[0]);
    assert!(!o.subpaths[0].closed);
    let pts: Vec<[f64; 2]> = o.subpaths[0].nodes.iter().map(|n| n.p).collect();
    assert!(near(pts[0], [500.0, 400.0]) && near(pts[1], [550.0, 400.0]) && near(pts[2], [550.0, 410.0]), "{pts:?}");
    // Escape cancels a path in progress, a second Escape leaves the pen.
    ed.handle_key("p", NO, None);
    ed.handle_key(" ", NO, None);
    ed.handle_key("Escape", NO, None);
    assert!(ed.state.pen.is_none() && ed.state.tool == Tool::Pen);
    ed.handle_key("Escape", NO, None);
    assert_eq!(ed.state.tool, Tool::Select);
    assert_eq!(shapes(&ed).len(), 1);
}

#[test]
fn double_click_finishes_an_open_path() {
    let (mut ed, _) = setup();
    ed.set_tool(Tool::Pen);
    click(&mut ed, [100.0, 100.0]);
    click(&mut ed, [200.0, 150.0]);
    click(&mut ed, [200.0, 150.0]);
    ed.double_click([200.0, 150.0]);
    let ids = shapes(&ed);
    assert_eq!(ids.len(), 1);
    assert_eq!(outline(&ed, &ids[0]).subpaths[0].nodes.len(), 2);
}

#[test]
fn node_editing_by_mouse_and_keyboard() {
    let (mut ed, _) = setup();
    let id = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], Some([150.0, 150.0]), true, false)[0].clone();
    assert!(ed.start_path_edit(&id));
    let view = ed.path_edit_view().unwrap();
    assert_eq!(view.subpaths[0].nodes.len(), 4);
    assert!(near(view.subpaths[0].nodes[0].p, [100.0, 100.0]));
    // Drag the top-left node out by (-20, -30): previewed, then committed as one step.
    ed.pointer_down([100.0, 100.0], NO, HitTarget::PathNode { s: 0, i: 0, part: NodePart::Anchor }, 0);
    ed.pointer_move([80.0, 70.0], NO, 0.5);
    assert_eq!(shape_of(&ed, &id).kind, ShapeKind::Rect, "not committed yet");
    ed.pointer_up();
    let s = shape_of(&ed, &id);
    assert_eq!(s.kind, ShapeKind::Custom);
    let x = exf(&ed, &id);
    assert_eq!([x.x, x.y, x.w, x.h], [80.0, 70.0, 120.0, 130.0]);
    let o = outline(&ed, &id);
    assert!(near(o.subpaths[0].nodes[0].p, [80.0, 70.0]) && near(o.subpaths[0].nodes[2].p, [200.0, 200.0]));
    assert_eq!(ed.state.path_edit.as_ref().unwrap().nodes, vec![NodeRef { s: 0, i: 0 }]);
    // Arrow keys nudge the selected node.
    assert!(ed.handle_key("ArrowRight", Mods::shift(), None));
    assert!(near(outline(&ed, &id).subpaths[0].nodes[0].p, [90.0, 70.0]));
    // Tab picks the next node; S makes it smooth (handles appear), S again a corner.
    ed.handle_key("Tab", NO, None);
    assert_eq!(ed.state.path_edit.as_ref().unwrap().nodes, vec![NodeRef { s: 0, i: 1 }]);
    ed.handle_key("s", NO, None);
    let n = outline(&ed, &id).subpaths[0].nodes[1].clone();
    assert!(n.cin.is_some() && n.cout.is_some());
    ed.handle_key("s", NO, None);
    assert!(outline(&ed, &id).subpaths[0].nodes[1].cout.is_none());
    // + adds a node after the selection, Delete removes it.
    ed.handle_key("+", NO, None);
    assert_eq!(outline(&ed, &id).subpaths[0].nodes.len(), 5);
    ed.handle_key("Delete", NO, None);
    assert_eq!(outline(&ed, &id).subpaths[0].nodes.len(), 4);
    // Clicking on the outline adds a node there.
    ed.pointer_down([200.0, 150.0], NO, HitTarget::Canvas, 0);
    ed.pointer_up();
    assert_eq!(outline(&ed, &id).subpaths[0].nodes.len(), 5);
    // Handles drag symmetric on smooth nodes.
    let sel = ed.state.path_edit.as_ref().unwrap().nodes[0];
    ed.set_path_nodes_smooth(Some(true));
    let n = outline(&ed, &id).subpaths[0].nodes[sel.i].clone();
    ed.pointer_down(n.cout.unwrap(), NO, HitTarget::PathNode { s: 0, i: sel.i, part: NodePart::Out }, 0);
    ed.pointer_move([n.p[0] + 30.0, n.p[1] + 30.0], NO, 0.5);
    ed.pointer_up();
    let n2 = outline(&ed, &id).subpaths[0].nodes[sel.i].clone();
    assert!(near(n2.cout.unwrap(), [n.p[0] + 30.0, n.p[1] + 30.0]));
    let (a, b) = (n2.cin.unwrap(), n2.cout.unwrap());
    let cross = (n2.p[0] - a[0]) * (b[1] - n2.p[1]) - (n2.p[1] - a[1]) * (b[0] - n2.p[0]);
    assert!(cross.abs() < 1e-3, "smooth node handles stay collinear");
    // Escape stops editing; clicking away does too.
    ed.handle_key("Escape", NO, None);
    assert!(ed.state.path_edit.is_none());
    assert_eq!(ed.state.selection, vec![id.clone()]);
    // Undo walks back each edit; the first one restores the rectangle.
    let mut steps = 0;
    while shape_of(&ed, &id).kind == ShapeKind::Custom {
        ed.undo();
        steps += 1;
        assert!(steps <= 9, "every edit is its own undo step");
    }
    assert!(steps >= 8, "{steps} undo steps");
    assert_eq!(shape_of(&ed, &id).kind, ShapeKind::Rect);
    assert_eq!(exf(&ed, &id).x, 100.0);
}

#[test]
fn rotated_paths_keep_their_place_while_editing() {
    let (mut ed, _) = setup();
    let mut x = xf(0.0, 0.0, 100.0, 50.0);
    x.rot = 30.0;
    let id = ed.insert(vec![shape(x)], Some([300.0, 300.0]), true, false)[0].clone();
    let before = outline(&ed, &id);
    ed.start_path_edit(&id);
    ed.select_path_nodes(vec![NodeRef { s: 0, i: 2 }]);
    ed.nudge_path_nodes(5.0, 0.0);
    let after = outline(&ed, &id);
    for i in [0, 1, 3] {
        assert!(near(before.subpaths[0].nodes[i].p, after.subpaths[0].nodes[i].p), "node {i} stays");
    }
    let (b, a) = (before.subpaths[0].nodes[2].p, after.subpaths[0].nodes[2].p);
    assert!(near(a, [b[0] + 5.0, b[1]]), "{b:?} → {a:?}");
    assert_eq!(exf(&ed, &id).rot, 30.0);
}

fn area(p: &VectorPath) -> f64 {
    oc_geometry::boolean::area(p).abs()
}

#[test]
fn booleans_replace_the_selection_with_one_path() {
    let (mut ed, _) = setup();
    let a = ed.insert(vec![shape_fill(xf(0.0, 0.0, 100.0, 100.0), "#ff0000")], Some([150.0, 150.0]), true, false)[0].clone();
    let b = ed.insert(vec![shape_fill(xf(0.0, 0.0, 100.0, 100.0), "#0000ff")], Some([200.0, 200.0]), true, false)[0].clone();
    for (op, expect) in [(BoolOp::Union, 17500.0), (BoolOp::Subtract, 7500.0), (BoolOp::Intersect, 2500.0), (BoolOp::Exclude, 15000.0)] {
        ed.select(&[a.clone(), b.clone()], false);
        let id = ed.boolean_op(op).unwrap_or_else(|| panic!("{op:?}"));
        assert_eq!(shapes(&ed), vec![id.clone()]);
        assert!((area(&outline(&ed, &id)) - expect).abs() < 1.0, "{op:?}: {}", area(&outline(&ed, &id)));
        // Styled like the bottom shape.
        assert_eq!(serde_json::to_value(&shape_of(&ed, &id).fills).unwrap()[0]["color"]["r"], json!(1.0));
        ed.undo();
        assert_eq!(shapes(&ed).len(), 2);
    }
    // Shapes that do not meet have no intersection: nothing changes.
    let c = ed.insert(vec![shape(xf(0.0, 0.0, 10.0, 10.0))], Some([900.0, 700.0]), true, false)[0].clone();
    ed.select(&[a.clone(), c.clone()], false);
    assert!(ed.boolean_op(BoolOp::Intersect).is_none());
    assert_eq!(shapes(&ed).len(), 3);
    // Text is not a shape: outline it first.
    let t = ed.insert(vec![text(xf(0.0, 0.0, 100.0, 40.0), "Hi")], Some([100.0, 600.0]), true, false)[0].clone();
    ed.select(&[a.clone(), t.clone()], false);
    assert!(ed.boolean_op(BoolOp::Union).is_none());
}

#[test]
fn outline_and_svg_conversion_replace_the_element() {
    let (mut ed, page) = setup();
    let t = ed.insert(vec![text(xf(0.0, 0.0, 200.0, 50.0), "Hi")], Some([300.0, 300.0]), true, false)[0].clone();
    let x0 = exf(&ed, &t);
    // Outlines in the element's own pixel space: a box at (10..60, 5..45).
    let id = ed.replace_with_path(&t, "M10 5 L60 5 L60 45 L10 45 Z M20 15 L20 35 L50 35 L50 15 Z", None).unwrap();
    assert!(el(&ed, &t).is_deleted());
    let x = exf(&ed, &id);
    assert_eq!([x.x, x.y, x.w, x.h], [x0.x + 10.0, x0.y + 5.0, 50.0, 40.0]);
    assert_eq!(el(&ed, &id).parent_id(), Some(page.as_str()));
    ed.undo();
    assert!(!el(&ed, &t).is_deleted());
    // SVG import: an image replaced by a group of paths placed in its box.
    let img = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], Some([600.0, 300.0]), true, false)[0].clone();
    let child = json!({ "id": "svgpath000000001", "type": "shape", "parentId": "", "z": "a0", "opacity": 1,
        "xf": { "x": 10, "y": 10, "w": 50, "h": 50, "rot": 0 }, "kind": "custom", "d": "M0 0 L1 0 L1 1 Z", "params": {},
        "fills": [], "strokes": [] });
    let g = ed.replace_with_elements(&img, vec![child.as_object().unwrap().clone()]).unwrap();
    assert!(matches!(el(&ed, &g), Element::Group(_)));
    assert_eq!(el(&ed, "svgpath000000001").parent_id(), Some(g.as_str()));
    let wb = world_bounds(ed.doc().els(), "svgpath000000001").unwrap();
    assert_eq!([wb.x, wb.y], [560.0, 260.0]);
}

#[test]
fn masks_group_edit_and_release() {
    let (mut ed, _) = setup();
    let photo = ed.insert(vec![shape(xf(0.0, 0.0, 200.0, 100.0))], Some([300.0, 300.0]), true, false)[0].clone();
    let mut e = xf(0.0, 0.0, 80.0, 80.0);
    e.rot = 10.0;
    let mask = ed.insert(vec![shape(e)], Some([320.0, 300.0]), true, false)[0].clone();
    let wb = |ed: &Editor, id: &str| world_bounds(ed.doc().els(), id).unwrap();
    let before = [wb(&ed, &photo), wb(&ed, &mask)];
    ed.select(&[photo.clone(), mask.clone()], false);
    let g = ed.mask_selection(MaskMode::Alpha).unwrap();
    let Element::Group(grp) = el(&ed, &g) else { panic!("group") };
    let m = grp.mask.unwrap();
    assert_eq!((m.id.as_str(), m.mode), (mask.as_str(), MaskMode::Alpha), "the top element masks");
    // The mask and the content stay editable inside the group; the mode can switch from either.
    ed.select(std::slice::from_ref(&photo), false);
    assert_eq!(ed.masked_group(), Some(g.clone()));
    assert!(ed.set_mask_mode(MaskMode::Luminance));
    let Element::Group(grp) = el(&ed, &g) else { panic!("group") };
    assert_eq!(grp.mask.unwrap().mode, MaskMode::Luminance);
    // Release puts both back exactly where they were.
    ed.select(std::slice::from_ref(&g), false);
    assert!(ed.release_mask());
    assert!(el(&ed, &g).is_deleted());
    let after = [wb(&ed, &photo), wb(&ed, &mask)];
    for (a, b) in after.iter().zip(&before) {
        assert!((a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.w - b.w).abs() < 1e-6);
    }
    ed.undo();
    assert!(!el(&ed, &g).is_deleted());
    // A single element cannot mask itself.
    ed.select(std::slice::from_ref(&photo), false);
    assert!(ed.mask_selection(MaskMode::Alpha).is_none());
}

#[test]
fn two_editors_see_each_others_paths_and_masks() {
    let (mut a, _) = setup();
    let mk = |upd: &[u8]| {
        let ydoc = oc_crdt::new_doc_with_client(2);
        oc_crdt::apply_update(&ydoc, upd, None).unwrap();
        Editor::from_ydoc(ydoc, EditorOptions { viewport: vp(0.0, 0.0, 1200.0, 900.0), clock: ManualClock::new(1_700_000_000_000.0), rng: Box::new(Mulberry32::new(9)) })
    };
    a.drain_local_updates();
    let mut b = mk(&oc_crdt::encode_state(a.store().ydoc()));
    let sync = |from: &mut Editor, to: &mut Editor| {
        for u in from.drain_local_updates() {
            to.apply_remote_update(&u).unwrap();
        }
    };
    // A draws a path; B edits its nodes; A masks a shape with it.
    a.set_tool(Tool::Pen);
    for p in [[100.0, 100.0], [300.0, 100.0], [200.0, 250.0], [100.0, 100.0]] {
        click(&mut a, p);
    }
    let path = shapes(&a)[0].clone();
    sync(&mut a, &mut b);
    assert_eq!(shapes(&b), vec![path.clone()]);
    assert!(b.start_path_edit(&path));
    b.select_path_nodes(vec![NodeRef { s: 0, i: 2 }]);
    b.nudge_path_nodes(0.0, 50.0);
    sync(&mut b, &mut a);
    assert!(near(outline(&a, &path).subpaths[0].nodes[2].p, [200.0, 300.0]));
    let photo = a.insert(vec![shape(xf(0.0, 0.0, 300.0, 300.0))], Some([200.0, 200.0]), true, false)[0].clone();
    // The path is the top element: move the photo below it.
    a.select(std::slice::from_ref(&photo), false);
    a.arrange(oc_editor_core::Arrange::Back);
    a.select(&[photo.clone(), path.clone()], false);
    let g = a.mask_selection(MaskMode::Alpha).unwrap();
    sync(&mut a, &mut b);
    let Element::Group(grp) = el(&b, &g) else { panic!("group on b") };
    assert_eq!(grp.mask.unwrap().id, path);
    assert_eq!(a.doc_json()["elements"], b.doc_json()["elements"]);
    // B's editing state was on the path, which is now inside the group: still consistent.
    let _: Value = b.doc_json();
}
