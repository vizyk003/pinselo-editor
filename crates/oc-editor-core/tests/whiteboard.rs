// SPDX-License-Identifier: Apache-2.0
//! Port of packages/editor-core/src/whiteboard.test.ts.

mod common;

use common::*;
use oc_crdt::board::VoteEntry;
use oc_editor_core::{
    Editor, HitTarget, Mods, User, cluster_layout, make_timer, mind_children, mind_layout, section_members, sections_of, tally_votes, timer_add,
    timer_pause, timer_remaining, timer_reset, timer_start, top_voted, vote_allowance, vote_scope,
};
use oc_schema::design::Surface;
use oc_schema::elements::{AnchorSide, Element, LineRoute};

const NO: Mods = Mods::NONE;

/// A big whiteboard with the view centred on (2000, 1500), so nothing triggers auto-expansion.
fn board() -> (Editor, String) {
    let (mut ed, page) = setup_with(4000.0, 3000.0, 1, Some(Surface::Whiteboard), vp(1400.0, 1050.0, 1200.0, 900.0));
    ed.set_text_service(Some(Box::new(BoardText)));
    ed.user = Some(User { id: "u1".into(), name: "Ada".into() });
    (ed, page)
}

fn mind_parent(e: &Element) -> Option<Option<String>> {
    e.base().and_then(|b| b.board.as_ref()).and_then(|b| b.mind.as_ref()).and_then(|m| m.parent.clone())
}

#[test]
fn tab_adds_a_child_to_the_right_glued_with_a_curved_connector() {
    let (mut ed, _) = board();
    let root = ed.insert(vec![shape(xf(0.0, 0.0, 200.0, 100.0))], Some([1500.0, 1500.0]), true, false)[0].clone();
    ed.select(std::slice::from_ref(&root), false);
    assert!(ed.handle_key("Tab", NO, None));
    let doc = ed.doc();
    let kids = mind_children(doc.els(), &root);
    assert_eq!(kids.len(), 1);
    let r = doc.el(&root).unwrap();
    let c = kids[0];
    assert_eq!(mind_parent(r), Some(None));
    let (rx, cx) = (r.xf().unwrap(), c.xf().unwrap());
    assert!(cx.x > rx.x + rx.w);
    assert!(((cx.y + cx.h / 2.0) - (rx.y + rx.h / 2.0)).abs() < 0.005);
    let link = doc
        .els()
        .values()
        .find_map(|e| match e {
            Element::Line(l) if l.end_attach.as_ref().map(|a| a.id.as_str()) == c.id() => Some(l.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(link.route, Some(LineRoute::Curved));
    let sa = link.start_attach.clone().unwrap();
    assert_eq!((sa.id.as_str(), sa.anchor), (root.as_str(), AnchorSide::Right));
    assert_eq!(link.end_attach.unwrap().anchor, AnchorSide::Left);
    assert!(ed.state.editing_text.is_some());
}

#[test]
fn enter_adds_a_sibling_below_layout_balances_the_subtree_around_the_root() {
    let (mut ed, _) = board();
    let root = ed.insert(vec![shape(xf(0.0, 0.0, 200.0, 100.0))], Some([1500.0, 1500.0]), true, false)[0].clone();
    ed.select(std::slice::from_ref(&root), false);
    ed.mind_map_add(true).unwrap();
    ed.stop_text_edit();
    let first_child = |ed: &Editor| mind_children(ed.doc().els(), &root)[0].id().unwrap().to_string();
    let c0 = first_child(&ed);
    ed.select(&[c0], false);
    assert!(ed.handle_key("Enter", NO, None));
    ed.stop_text_edit();
    let c0 = first_child(&ed);
    ed.select(&[c0], false);
    ed.handle_key("Enter", NO, None);
    ed.stop_text_edit();
    let doc = ed.doc();
    let kids = mind_children(doc.els(), &root);
    assert_eq!(kids.len(), 3);
    assert!(kids[1].xf().unwrap().y > kids[0].xf().unwrap().y + kids[0].xf().unwrap().h - 0.5);

    ed.layout_mind_map(Some(&root));
    let doc = ed.doc();
    let r = doc.el(&root).unwrap().xf().unwrap().clone();
    let laid = mind_children(doc.els(), &root);
    let top = laid.iter().map(|k| k.xf().unwrap().y).fold(f64::INFINITY, f64::min);
    let bottom = laid.iter().map(|k| k.xf().unwrap().y + k.xf().unwrap().h).fold(f64::NEG_INFINITY, f64::max);
    assert!(((top + bottom) / 2.0 - (r.y + r.h / 2.0)).abs() < 0.0005);
    let xs: std::collections::HashSet<i64> = laid.iter().map(|k| k.xf().unwrap().x.round() as i64).collect();
    assert_eq!(xs.len(), 1);
    for i in 1..laid.len() {
        assert!(laid[i].xf().unwrap().y >= laid[i - 1].xf().unwrap().y + laid[i - 1].xf().unwrap().h);
    }
}

#[test]
fn mind_layout_gives_each_subtree_a_band_as_tall_as_its_leaves() {
    let (mut ed, _) = board();
    let root = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 50.0))], Some([1000.0, 1500.0]), true, false)[0].clone();
    ed.select(std::slice::from_ref(&root), false);
    let a = ed.mind_map_add(true).unwrap();
    ed.stop_text_edit();
    ed.select(std::slice::from_ref(&root), false);
    let b = ed.mind_map_add(true).unwrap();
    ed.stop_text_edit();
    for _ in 0..3 {
        ed.select(std::slice::from_ref(&a), false);
        ed.mind_map_add(true);
        ed.stop_text_edit();
    }
    let doc = ed.doc();
    let pos = mind_layout(doc.els(), &root, Some(50.0), Some(10.0));
    let h = doc.el(&a).unwrap().xf().unwrap().h;
    let a_band = 3.0 * h + 2.0 * 10.0;
    let pa = pos[&a];
    let pb = pos[&b];
    assert!((pb.1 - (pa.1 + h / 2.0 - a_band / 2.0) - (a_band + 10.0)).abs() < 0.0005);
    assert_eq!(pos.len(), 5);
}

fn e(voter: &str, element: &str, n: f64, s: &str) -> VoteEntry {
    VoteEntry { session: s.into(), voter: voter.into(), element: element.into(), n, name: None }
}

fn scope() -> Vec<String> {
    ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect()
}

#[test]
fn tallies_dots_per_element_most_first_ignoring_other_sessions_and_out_of_scope_elements() {
    let entries = vec![
        e("u1", "b", 2.0, "s1"),
        e("u2", "b", 1.0, "s1"),
        e("u1", "c", 1.0, "s1"),
        e("u2", "a", 1.0, "s1"),
        e("u3", "c", 1.0, "s1"),
        e("u3", "zzz", 5.0, "s1"),
        e("u1", "d", 9.0, "old"),
    ];
    let t = tally_votes("s1", &scope(), &entries);
    let got: Vec<(String, f64)> = t.iter().map(|x| (x.element.clone(), x.count)).collect();
    assert_eq!(got, vec![("b".into(), 3.0), ("c".into(), 2.0), ("a".into(), 1.0), ("d".into(), 0.0)]);
    let mut v: Vec<String> = t[0].voters.iter().map(|v| v.id.clone()).collect();
    v.sort();
    assert_eq!(v, vec!["u1", "u2"]);
    assert_eq!(top_voted(&t, 3), vec!["b", "c", "a"]);
    assert_eq!(top_voted(&t, 1), vec!["b"]);
}

#[test]
fn includes_ties_at_the_cut_off_and_enforces_the_per_person_budget() {
    let entries = vec![e("u1", "a", 1.0, "s1"), e("u1", "b", 1.0, "s1"), e("u2", "c", 1.0, "s1"), e("u2", "d", 1.0, "s1")];
    assert_eq!(top_voted(&tally_votes("s1", &scope(), &entries), 2), vec!["a", "b", "c", "d"]);
    assert_eq!(vote_allowance("s1", 3.0, "open", &entries, "u1"), 1.0);
    let mut more = entries.clone();
    more.push(e("u1", "c", 1.0, "s1"));
    assert_eq!(vote_allowance("s1", 3.0, "open", &more, "u1"), 0.0);
    assert_eq!(vote_allowance("s1", 3.0, "ended", &[], "u1"), 0.0);
}

#[test]
fn defaults_the_scope_to_stickies_and_shapes_on_the_page() {
    let (mut ed, page) = board();
    let sticky = ed.add_sticky("#fef08a", Some("Idea"), None, None, None).unwrap();
    let shape_id = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], Some([1200.0, 1200.0]), true, false)[0].clone();
    ed.create_section("Ideas");
    let doc = ed.doc();
    let mut sc = vote_scope(doc.els(), &page, &[]);
    sc.sort();
    let mut want = vec![sticky, shape_id.clone()];
    want.sort();
    assert_eq!(sc, want);
    assert_eq!(vote_scope(doc.els(), &page, std::slice::from_ref(&shape_id)), vec![shape_id]);
}

#[test]
fn contains_the_elements_inside_its_box_moves_them_along_and_collapses_them() {
    let (mut ed, page) = board();
    let a = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], Some([1800.0, 1500.0]), true, false)[0].clone();
    let b = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], Some([2100.0, 1500.0]), true, false)[0].clone();
    let outside = ed.insert(vec![shape(xf(0.0, 0.0, 100.0, 100.0))], Some([3000.0, 2500.0]), true, false)[0].clone();
    ed.select(&[a.clone(), b.clone()], false);
    let s = ed.create_section("Group 1").unwrap();
    let doc = ed.doc();
    assert_eq!(sections_of(doc.els(), &page).iter().map(|e| e.id().unwrap().to_string()).collect::<Vec<_>>(), vec![s.clone()]);
    let mut m = section_members(doc.els(), &s);
    m.sort();
    let mut ab = vec![a.clone(), b.clone()];
    ab.sort();
    assert_eq!(m, ab);
    let kids = oc_schema::tree::children_of(doc.els(), &page);
    assert_eq!(kids[0].id(), Some(s.as_str()));

    let before = exf(&ed, &a).x;
    let sec = exf(&ed, &s);
    let [ox, oy] = ed.page_origin(&page);
    let grab = [ox + sec.x + 10.0, oy + sec.y + 10.0];
    ed.pointer_down(grab, NO, HitTarget::Canvas, 0);
    ed.pointer_move([grab[0] + 50.0, grab[1] + 30.0], Mods::meta(), 0.5);
    ed.pointer_up();
    assert!((exf(&ed, &a).x - (before + 50.0)).abs() < 0.005);
    assert_eq!(exf(&ed, &outside).x, 2950.0);

    ed.toggle_section(&s);
    assert_eq!(el(&ed, &a).base().unwrap().visible, Some(false));
    assert!(exf(&ed, &s).h <= 64.0);
    let mut m = section_members(ed.doc().els(), &s);
    m.sort();
    assert_eq!(m, ab);
    ed.toggle_section(&s);
    assert_eq!(el(&ed, &a).base().unwrap().visible, None);
    assert!(el(&ed, &s).base().unwrap().board.as_ref().unwrap().section.as_ref().unwrap().collapsed.is_none());
}

#[test]
fn reorders_sections_in_the_navigator() {
    let (mut ed, page) = board();
    let one = ed.create_section("One").unwrap();
    ed.clear_selection();
    let two = ed.create_section("Two").unwrap();
    let names = |ed: &Editor| sections_of(ed.doc().els(), &page).iter().map(|e| e.base().unwrap().name.clone().unwrap()).collect::<Vec<_>>();
    assert_eq!(names(&ed), vec!["One", "Two"]);
    ed.move_section(&two, -1);
    assert_eq!(sections_of(ed.doc().els(), &page).iter().map(|e| e.id().unwrap().to_string()).collect::<Vec<_>>(), vec![two.clone(), one.clone()]);
    ed.rename_section(&one, "Uno");
    assert_eq!(el(&ed, &one).base().unwrap().board.as_ref().unwrap().section.as_ref().unwrap().title, "Uno");
}

#[test]
fn records_the_author_recolours_and_clusters_by_colour() {
    let (mut ed, _) = board();
    let ids = [
        ed.add_sticky("#fef08a", None, Some([1000.0, 1000.0]), None, None).unwrap(),
        ed.add_sticky("#bbf7d0", None, Some([1600.0, 1300.0]), None, None).unwrap(),
        ed.add_sticky("#fef08a", None, Some([2200.0, 1100.0]), None, None).unwrap(),
        ed.add_sticky("#bbf7d0", None, Some([1300.0, 1900.0]), None, None).unwrap(),
    ];
    let author = el(&ed, &ids[0]).base().unwrap().board.clone().unwrap().sticky.unwrap().author.unwrap();
    assert_eq!((author.id.as_str(), author.name.as_str()), ("u1", "Ada"));
    let doc = ed.doc();
    let t = doc
        .els()
        .values()
        .find_map(|e| match e {
            Element::Text(t) if t.base.parent_id == ids[0] => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(t.body.default_run.family, "Caveat");

    ed.set_sticky_color("#fecaca", Some(std::slice::from_ref(&ids[3])));
    assert_eq!(el(&ed, &ids[3]).base().unwrap().board.clone().unwrap().sticky.unwrap().color, "#fecaca");

    let layout = cluster_layout(ed.doc().els(), &ids, 20.0);
    assert_eq!(layout.len(), 4);
    ed.cluster_stickies(Some(&ids));
    let y0 = exf(&ed, &ids[0]);
    let y2 = exf(&ed, &ids[2]);
    assert_eq!(y0.y, y2.y);
    assert_eq!((y2.x - y0.x).abs(), y0.w + 24.0);
}

#[test]
fn starts_pauses_adds_a_minute_and_resets() {
    let mut t = make_timer(5.0 * 60_000.0, true);
    assert_eq!(timer_remaining(&t, 0.0), 300_000.0);
    t = timer_start(&t, 1000.0);
    assert_eq!(timer_remaining(&t, 61_000.0), 240_000.0);
    t = timer_pause(&t, 61_000.0);
    assert_eq!(timer_remaining(&t, 999_999.0), 240_000.0);
    t = timer_add(&t, 60_000.0, 70_000.0);
    assert_eq!(timer_remaining(&t, 70_000.0), 300_000.0);
    t = timer_start(&t, 100_000.0);
    t = timer_add(&t, 60_000.0, 100_000.0);
    assert_eq!(timer_remaining(&t, 100_000.0), 360_000.0);
    assert_eq!(timer_remaining(&t, 10_000_000.0), 0.0);
    assert_eq!(timer_remaining(&timer_reset(&t), 0.0), t.duration_ms);
}
