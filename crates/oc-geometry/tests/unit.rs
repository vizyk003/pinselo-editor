// SPDX-License-Identifier: Apache-2.0
//! Ports of the TS unit tests (packages/geometry/src/geometry.test.ts and shapes.test.ts).

use std::collections::HashSet;

use oc_geometry::*;
use oc_schema::elements::{Element, SHAPE_KINDS, ShapeKind};
use oc_schema::factory::{MetaInit, ShapeOpts, base_init, empty_design, make_group, make_shape, xf};
use oc_schema::primitives::{Rect, Xf};
use oc_schema::tree::sorted_pages;
use oc_schema::DesignDoc;

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

// ------------------------------------------------------------------ geometry.test.ts

#[test]
fn matrices_invert() {
    let m = local_matrix(&Xf { flip_x: Some(true), ..Xf::new(10.0, 20.0, 100.0, 50.0, 33.0) });
    let id = multiply(&m, &invert(&m));
    for (i, v) in [1.0, 0.0, 0.0, 1.0, 0.0, 0.0].iter().enumerate() {
        close(id[i], *v);
    }
}

#[test]
fn matrices_rotate_about_centre() {
    let m = local_matrix(&Xf::new(0.0, 0.0, 100.0, 100.0, 90.0));
    let [x, y] = apply(&m, [50.0, 50.0]);
    close(x, 50.0);
    close(y, 50.0);
    let [x2, y2] = apply(&m, [0.0, 0.0]);
    close(x2, 100.0);
    close(y2, 0.0);
}

#[test]
fn matrices_decompose_back_into_xf() {
    let src = Xf::new(12.0, 34.0, 200.0, 80.0, -30.0);
    let back = xf_from_matrix(&local_matrix(&src), src.w, src.h);
    close(back.x, src.x);
    close(back.y, src.y);
    close(back.rot, src.rot);
}

const G: &str = "GGGGGGGGGGGGGGGG";
const I: &str = "IIIIIIIIIIIIIIII";
const T: &str = "TTTTTTTTTTTTTTTT";

fn scene_doc() -> (DesignDoc, String) {
    let mut d = empty_design(&MetaInit { w: 500.0, h: 500.0, ..Default::default() });
    let p = sorted_pages(&d.pages)[0].id.clone();
    let mut g = make_group(base_init(xf(100.0, 100.0, 200.0, 200.0, 90.0)));
    g.base.parent_id = p.clone();
    g.base.z = "a1".into();
    g.base.id = G.into();
    let mut inner = make_shape(base_init(xf(0.0, 0.0, 100.0, 100.0, 0.0)), ShapeOpts::default());
    inner.base.parent_id = G.into();
    inner.base.z = "a0".into();
    inner.base.id = I.into();
    let mut tri =
        make_shape(base_init(xf(350.0, 350.0, 100.0, 100.0, 0.0)), ShapeOpts { kind: Some(ShapeKind::Triangle), ..Default::default() });
    tri.base.parent_id = p.clone();
    tri.base.z = "a2".into();
    tri.base.id = T.into();
    d.elements.insert(G.into(), Element::Group(g));
    d.elements.insert(I.into(), Element::Shape(inner));
    d.elements.insert(T.into(), Element::Shape(tri));
    (d, p)
}

#[test]
fn scene_composes_group_transforms_into_world_bounds() {
    let (d, _) = scene_doc();
    let b = world_bounds(&d.elements, I).expect("bounds");
    // group rotated 90° about (200,200): child box (100..200, 100..200) maps to (200..300, 100..200)
    close(b.x, 200.0);
    close(b.y, 100.0);
    close(b.w, 100.0);
}

#[test]
fn scene_hit_tests_shapes_and_selects_the_group() {
    let (d, p) = scene_doc();
    let hit = |pt: [f64; 2], enter: Option<&str>| {
        hit_test(&d.elements, &p, pt, HitTestOpts { tolerance: 0.0, enter_group: enter }).and_then(Element::id).map(str::to_string)
    };
    assert_eq!(hit([250.0, 150.0], None).as_deref(), Some(G));
    assert_eq!(hit([400.0, 445.0], None).as_deref(), Some(T));
    assert_eq!(hit([355.0, 355.0], None), None); // outside the triangle, inside its box
    assert_eq!(hit([250.0, 150.0], Some(G)).as_deref(), Some(I));
}

#[test]
fn snaps_box_centre_to_page_centre() {
    let r = snap_move(&Rect::new(197.0, 10.0, 100.0, 50.0), &Rect::new(0.0, 0.0, 500.0, 500.0), &[], 5.0, &[]);
    close(r.dx, 3.0);
    assert!(r.guides.iter().any(|g| g.axis == Axis::X && g.pos == 250.0));
}

#[test]
fn snaps_rotation() {
    assert_eq!(snap_angle(43.0, false), 45.0);
    assert_eq!(snap_angle(40.0, false), 40.0);
    assert_eq!(snap_angle(40.0, true), 45.0);
}

fn items() -> Vec<AlignItem> {
    [("a", Rect::new(0.0, 0.0, 10.0, 10.0)), ("b", Rect::new(50.0, 20.0, 10.0, 10.0)), ("c", Rect::new(100.0, 40.0, 30.0, 10.0))]
        .into_iter()
        .map(|(id, bounds)| AlignItem { id: id.into(), bounds })
        .collect()
}

#[test]
fn aligns_left() {
    let d = align_deltas(&items(), AlignEdge::Left, &Rect::new(0.0, 0.0, 130.0, 50.0));
    assert_eq!(d["c"], [-100.0, 0.0]);
}

#[test]
fn distributes_with_equal_gaps() {
    let d = distribute_deltas(&items(), Axis::X);
    // span 130, sizes 50, gap 40 → b at x=50 already
    assert_eq!(d["b"], [0.0, 0.0]);
}

#[test]
fn centres_a_box_between_two_neighbours() {
    let page = Rect::new(0.0, 0.0, 1000.0, 1000.0);
    let left = Rect::new(100.0, 100.0, 100.0, 100.0);
    let right = Rect::new(500.0, 100.0, 100.0, 100.0);
    // gap left = 103, gap right = 97 → centre needs dx = -3
    let r = snap_move(&Rect::new(303.0, 400.0, 100.0, 100.0), &page, &[left, right, Rect::new(0.0, 700.0, 1.0, 1.0)], 5.0, &[]);
    assert_eq!(r.dx, 0.0); // not in the same row (y 400) → no spacing snap
    assert!(r.spacing.is_empty());
    let r2 = snap_move(&Rect::new(303.0, 120.0, 100.0, 60.0), &page, &[left, right], 5.0, &[]);
    assert!((r2.dx + 3.0).abs() < 0.005);
    assert_eq!(r2.spacing.iter().map(|m| m.gap).collect::<Vec<_>>(), vec![100.0, 100.0]);
}

#[test]
fn matches_an_existing_gap() {
    let a = Rect::new(0.0, 0.0, 50.0, 50.0);
    let b = Rect::new(80.0, 0.0, 50.0, 50.0); // gap 30
    let (delta, _) = snap_spacing(&Rect::new(158.0, 10.0, 40.0, 40.0), &[a, b], Axis::X, 4.0).expect("snaps");
    assert!((delta - 2.0).abs() < 0.005); // 130 + 30 = 160
}

#[test]
fn routes_elbow_connectors_orthogonally() {
    use oc_schema::elements::LineRoute;
    let pts = route_connector([0.0, 0.0], [1.0, 0.0], [100.0, 50.0], [-1.0, 0.0], Some(&LineRoute::Elbow));
    assert_eq!(pts, vec![[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [100.0, 50.0]]);
    for w in pts.windows(2) {
        assert!(w[1][0] == w[0][0] || w[1][1] == w[0][1]);
    }
}

#[test]
fn connectors_of_finds_nothing_without_attachments() {
    let (d, _) = scene_doc();
    let ids: HashSet<String> = [G, I, T].map(String::from).into();
    assert!(connectors_of(&d.elements, &ids).is_empty());
}

// ------------------------------------------------------------------ shapes.test.ts

/// Sampled extents of an absolute M/L/C/Q/Z path (curves at 24 steps).
struct Extents {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    subpaths: usize,
}

fn extents(d: &str) -> Extents {
    let re = regex::Regex::new(r"(?i)[MLCQZ]|-?[0-9]*\.?[0-9]+(?:e-?[0-9]+)?").expect("regex");
    let toks: Vec<&str> = re.find_iter(d).map(|m| m.as_str()).collect();
    let mut e = Extents { x0: f64::INFINITY, y0: f64::INFINITY, x1: f64::NEG_INFINITY, y1: f64::NEG_INFINITY, subpaths: 0 };
    let add = |e: &mut Extents, x: f64, y: f64| {
        e.x0 = e.x0.min(x);
        e.y0 = e.y0.min(y);
        e.x1 = e.x1.max(x);
        e.y1 = e.y1.max(y);
    };
    let is_cmd = |t: &str| t.len() == 1 && "MLCQZmlcqz".contains(t);
    let mut cmd = String::new();
    let (mut px, mut py) = (0.0, 0.0);
    let mut i = 0;
    while i < toks.len() {
        if is_cmd(toks[i]) {
            cmd = toks[i].to_uppercase();
            i += 1;
        }
        if cmd == "Z" {
            continue;
        }
        let n = |k: usize| -> Vec<f64> { toks[i..i + k].iter().map(|t| t.parse::<f64>().expect("number")).collect() };
        match cmd.as_str() {
            "M" | "L" => {
                let v = n(2);
                (px, py) = (v[0], v[1]);
                add(&mut e, px, py);
                if cmd == "M" {
                    e.subpaths += 1;
                }
                i += 2;
            }
            "C" | "Q" => {
                let k = if cmd == "C" { 6 } else { 4 };
                let c = n(k);
                let mut pts: Vec<[f64; 2]> = vec![[px, py]];
                pts.extend(c.chunks(2).map(|p| [p[0], p[1]]));
                for st in 1..=24 {
                    let t = f64::from(st) / 24.0;
                    let mut p = pts.clone();
                    while p.len() > 1 {
                        p = p.windows(2).map(|w| [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]).collect();
                    }
                    add(&mut e, p[0][0], p[0][1]);
                }
                [px, py] = pts[pts.len() - 1];
                i += k;
            }
            _ => panic!("unexpected token {}", toks[i]),
        }
    }
    e
}

#[test]
fn shape_library_has_60_plus_unique_kinds() {
    let set: HashSet<&str> = SHAPE_KINDS.iter().copied().collect();
    assert_eq!(set.len(), SHAPE_KINDS.len());
    assert!(SHAPE_KINDS.len() >= 60);
}

#[test]
fn every_kind_produces_a_closed_in_box_path() {
    for (w, h) in [(100.0, 100.0), (320.0, 140.0), (60.0, 200.0)] {
        for kind in SHAPE_KINDS {
            let d = shape_path(kind, w, h, &Params::new(), None);
            assert!(d.len() > 5, "{kind}");
            assert!(!d.contains("NaN") && !d.contains("Infinity"), "{kind}");
            assert!(d.trim().ends_with('Z'), "{kind}");
            let e = extents(&d);
            assert!(e.subpaths > 0, "{kind}");
            assert!(e.x0 >= -1.0 && e.y0 >= -1.0, "{kind} at {w}×{h}");
            assert!(e.x1 <= w + 1.0 && e.y1 <= h + 1.0, "{kind} at {w}×{h}");
            // Uses a meaningful part of the box.
            assert!(e.x1 - e.x0 > w * 0.5, "{kind}");
            assert!(e.y1 - e.y0 > h * 0.4, "{kind}");
        }
    }
}
