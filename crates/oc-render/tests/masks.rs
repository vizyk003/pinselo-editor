// SPDX-License-Identifier: Apache-2.0
//! Masks (DS-107) and vector paths (DS-100): a masked group draws its content only where the mask is (alpha) or
//! as bright as the mask (luminance), the same in the painter, the SVG writer (resvg) and the PDF writer (hayro);
//! a missing or hidden mask leaves the content unmasked; custom path shapes draw their outline.

mod common;

use std::collections::HashMap;

use common::*;
use oc_render::pdf::{PdfOptions, PdfPage, write_pdf};
use oc_render::svg::{SvgOptions, display_list_to_svg};
use oc_render::{DisplayList, RenderOptions, ResolveOptions, ops_of, resolve_page};
use oc_schema::DesignDoc;
use serde_json::{Value, json};

fn solid(r: f64, g: f64, b: f64) -> Value {
    json!({ "k": "solid", "color": { "space": "srgb", "r": r, "g": g, "b": b, "a": 1 } })
}

#[allow(clippy::too_many_arguments)]
fn shape(id: &str, parent: &str, z: &str, x: f64, y: f64, w: f64, h: f64, kind: &str, fill: Value) -> Value {
    json!({ "id": id, "type": "shape", "parentId": parent, "z": z, "opacity": 1, "xf": { "x": x, "y": y, "w": w, "h": h, "rot": 0 },
            "kind": kind, "params": {}, "fills": [fill], "strokes": [] })
}

/// A 200×100 white page with a group (0,0,200,100): a red rect under `mask_el` (child `m`) masked in `mode`.
fn doc(mode: Option<&str>, mask_el: Value) -> DesignDoc {
    let mut els = serde_json::Map::new();
    let mut g = json!({ "id": "g", "type": "group", "parentId": "p", "z": "a0", "opacity": 1, "xf": { "x": 0, "y": 0, "w": 200, "h": 100, "rot": 0 } });
    if let Some(m) = mode {
        g["mask"] = json!({ "id": "m", "mode": m });
    }
    els.insert("g".into(), g);
    els.insert("c".into(), shape("c", "g", "a0", 0.0, 0.0, 200.0, 100.0, "rect", solid(1.0, 0.0, 0.0)));
    els.insert("m".into(), mask_el);
    de(&json!({
        "meta": { "schemaVersion": 1, "minClientSchema": 1, "layoutVersion": 1, "surface": "graphic",
                  "dims": { "w": 200, "h": 100, "unit": "px", "dpi": 96 }, "lang": "en", "title": "m", "epoch": 0 },
        "pages": { "p": { "id": "p", "order": "a0", "background": [solid(1.0, 1.0, 1.0)] } },
        "elements": els,
        "assets": {}
    }))
}

fn ellipse_mask() -> Value {
    shape("m", "g", "a1", 50.0, 0.0, 100.0, 100.0, "ellipse", solid(1.0, 1.0, 1.0))
}

fn gradient_mask() -> Value {
    // Black (left) to white (right) over the whole group.
    let stops = json!([{ "t": 0, "color": { "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } },
                       { "t": 1, "color": { "space": "srgb", "r": 1, "g": 1, "b": 1, "a": 1 } }]);
    shape("m", "g", "a1", 0.0, 0.0, 200.0, 100.0, "rect", json!({ "k": "linear", "angle": 0, "stops": stops }))
}

fn paint(dl: &DisplayList) -> Vec<u8> {
    let mut r = renderer();
    oc_render::raster::to_rgba8(&r.render(dl, &HashMap::new(), &RenderOptions::default()).expect("render"))
}

fn px(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * w + x) * 4;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

fn near(a: [u8; 4], b: [u8; 3], tol: i32) -> bool {
    (0..3).all(|k| (i32::from(a[k]) - i32::from(b[k])).abs() <= tol)
}

#[test]
fn alpha_mask_shows_content_inside_the_mask_only() {
    let d = doc(Some("alpha"), ellipse_mask());
    let dl = resolve_page(&d, "p", &ResolveOptions::default());
    let ops = ops_of(&dl);
    assert!(ops.contains(&"mask") && ops.contains(&"maskContent"), "{ops:?}");
    let img = paint(&dl);
    assert!(near(px(&img, 200, 100, 50), [255, 0, 0], 2), "centre is red: {:?}", px(&img, 200, 100, 50));
    assert!(near(px(&img, 200, 10, 50), [255, 255, 255], 2), "outside the ellipse the page shows");
    assert!(near(px(&img, 200, 190, 90), [255, 255, 255], 2));
    // The white mask itself is never drawn.
    assert!(near(px(&img, 200, 52, 5), [255, 255, 255], 2));
}

#[test]
fn luminance_mask_follows_brightness() {
    let d = doc(Some("luminance"), gradient_mask());
    let img = paint(&resolve_page(&d, "p", &ResolveOptions::default()));
    let left = px(&img, 200, 3, 50);
    let mid = px(&img, 200, 100, 50);
    let right = px(&img, 200, 196, 50);
    assert!(near(left, [255, 255, 255], 8), "black hides: {left:?}");
    assert!(near(right, [255, 10, 10], 12), "white shows: {right:?}");
    assert!(mid[1] > 60 && mid[1] < 200, "grey shows partly: {mid:?}");
    // The same gradient as an alpha mask (opaque everywhere) shows the content everywhere.
    let a = paint(&resolve_page(&doc(Some("alpha"), gradient_mask()), "p", &ResolveOptions::default()));
    assert!(near(px(&a, 200, 3, 50), [255, 0, 0], 2));
}

#[test]
fn missing_or_hidden_mask_leaves_content_unmasked() {
    // Mask id pointing at nothing: the group draws its children as usual.
    let mut d = doc(Some("alpha"), ellipse_mask());
    d.elements.shift_remove("m");
    let img = paint(&resolve_page(&d, "p", &ResolveOptions::default()));
    assert!(near(px(&img, 200, 10, 50), [255, 0, 0], 2));
    // A hidden mask element: unmasked, and the mask stays hidden.
    let mut m = ellipse_mask();
    m["visible"] = json!(false);
    let img = paint(&resolve_page(&doc(Some("alpha"), m), "p", &ResolveOptions::default()));
    assert!(near(px(&img, 200, 10, 50), [255, 0, 0], 2));
    // No mask field: a plain group draws every child (the white ellipse over the red rect).
    let img = paint(&resolve_page(&doc(None, ellipse_mask()), "p", &ResolveOptions::default()));
    assert!(near(px(&img, 200, 100, 50), [255, 255, 255], 2));
    assert!(near(px(&img, 200, 10, 50), [255, 0, 0], 2));
}

#[test]
fn masked_group_without_mask_field_is_unchanged_json() {
    // Older documents (no `mask`) serialise exactly as before.
    let d = doc(None, ellipse_mask());
    let v = serde_json::to_value(&d.elements["g"]).unwrap();
    assert!(v.get("mask").is_none());
    let d = doc(Some("luminance"), ellipse_mask());
    let v = serde_json::to_value(&d.elements["g"]).unwrap();
    assert_eq!(v["mask"], json!({ "id": "m", "mode": "luminance" }));
}

fn delta(a: &[u8], b: &[u8]) -> f64 {
    let n = a.len() / 4;
    let mut sum = 0.0;
    for i in 0..n {
        for k in 0..3 {
            sum += (f64::from(a[i * 4 + k]) - f64::from(b[i * 4 + k])).abs();
        }
    }
    sum / (n as f64 * 3.0)
}

#[test]
fn svg_and_pdf_match_the_painter() {
    let mut r = renderer();
    let assets: HashMap<String, Vec<u8>> = HashMap::new();
    for (mode, mask) in [("alpha", ellipse_mask()), ("luminance", gradient_mask())] {
        let d = doc(Some(mode), mask);
        let dl = resolve_page(&d, "p", &ResolveOptions::default());
        let ours = paint(&dl);
        let svg = display_list_to_svg(&mut r, &dl, &assets, &SvgOptions::default());
        assert!(svg.contains("<mask ") && svg.contains(&format!("mask-type:{mode}")), "{svg}");
        let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).expect("svg parses");
        let mut pix = oc_render::tiny_skia::Pixmap::new(200, 100).unwrap();
        resvg::render(&tree, oc_render::tiny_skia::Transform::identity(), &mut pix.as_mut());
        let theirs = oc_render::raster::to_rgba8(&pix);
        let dsvg = delta(&ours, &theirs);
        assert!(dsvg < 2.0, "svg {mode}: mean channel delta {dsvg}");

        let pdf = write_pdf(&mut r, &[PdfPage { dl: &dl, links: vec![] }], &assets, &PdfOptions::default()).expect("pdf");
        let p = hayro::hayro_syntax::Pdf::new(pdf).expect("pdf parses");
        let pages = p.pages();
        let page = pages.first().expect("page");
        let settings = hayro::hayro_interpret::InterpreterSettings::default();
        let pm = hayro::render(page, &hayro::RenderCache::new(), &settings, &hayro::RenderSettings::default(), &hayro::PixmapSettings { x_scale: 1.0 / 0.75, y_scale: 1.0 / 0.75, ..Default::default() });
        let mut data: Vec<u8> = pm.data_as_u8_slice().to_vec();
        oc_render::assets::unpremultiply(&mut data);
        assert_eq!((u32::from(pm.width()), u32::from(pm.height())), (200, 100));
        let dpdf = delta(&ours, &data);
        assert!(dpdf < 3.0, "pdf {mode}: mean channel delta {dpdf}");
        assert!(near(px(&data, 200, 2, 50), [255, 255, 255], 12), "pdf {mode} left: {:?}", px(&data, 200, 2, 50));
        assert!(near(px(&data, 200, if mode == "alpha" { 100 } else { 197 }, 50), [255, 0, 0], 16), "pdf {mode} shows: {:?}", px(&data, 200, 100, 50));
    }
}

#[test]
fn custom_path_shape_draws_its_outline() {
    // A triangle path in the unit box, open at the bottom-left corner: filled as closed, nothing outside.
    let mut d = doc(None, shape("m", "g", "a1", 0.0, 0.0, 1.0, 1.0, "rect", solid(1.0, 1.0, 1.0)));
    d.elements.shift_remove("m");
    let mut tri = shape("c", "g", "a0", 0.0, 0.0, 200.0, 100.0, "custom", solid(0.0, 0.0, 1.0));
    tri["d"] = json!("M0 1 L0.5 0 C0.75 0.5 1 0.75 1 1 Z");
    d.elements.insert("c".into(), de(&tri));
    let img = paint(&resolve_page(&d, "p", &ResolveOptions::default()));
    assert!(near(px(&img, 200, 100, 80), [0, 0, 255], 2));
    assert!(near(px(&img, 200, 10, 10), [255, 255, 255], 2));
}

#[test]
fn outlined_text_covers_the_same_pixels_as_the_text() {
    // A text element drawn by the painter, and the same text as a filled path: nearly the same ink.
    let mut r = renderer();
    let mut d = doc(None, ellipse_mask());
    d.elements.shift_remove("m");
    d.elements.shift_remove("c");
    let t = oc_schema::factory::make_text(
        oc_schema::factory::base_init(oc_schema::primitives::Xf::new(10.0, 10.0, 180.0, 80.0, 0.0)),
        oc_schema::factory::TextOpts {
            text: Some("Hg".into()),
            style: Some(oc_schema::text::RunStyleOverride { size: Some(60.0), ..Default::default() }),
            ..Default::default()
        },
    );
    let mut tv = serde_json::to_value(oc_schema::elements::Element::Text(t)).unwrap();
    tv["id"] = json!("t");
    tv["parentId"] = json!("g");
    d.elements.insert("t".into(), de(&tv));
    let dl = resolve_page(&d, "p", &ResolveOptions::default());
    ensure_fonts(&mut r, &dl);
    let text_img = oc_render::raster::to_rgba8(&r.render(&dl, &HashMap::new(), &RenderOptions::default()).unwrap());
    let oc_schema::elements::Element::Text(te) = &d.elements["t"] else { panic!("text") };
    let (path_d, fill) = r.text_outline(te).expect("outline");
    assert!(path_d.starts_with('M') && path_d.contains('Z'));
    assert!(matches!(fill, oc_schema::primitives::Paint::Solid(_)));
    // Draw the outline as a custom shape over the same box.
    let vp = oc_geometry::VectorPath::parse(&path_d);
    let b = vp.bounds().unwrap();
    assert!(b.x >= 0.0 && b.y >= 0.0 && b.x + b.w <= 180.0 && b.y + b.h <= 80.0, "{b:?}");
    let (bx, unit) = vp.to_unit(1.0).unwrap();
    let mut sh = shape("t", "g", "a0", 10.0 + bx.x, 10.0 + bx.y, bx.w, bx.h, "custom", serde_json::to_value(&fill).unwrap());
    sh["d"] = json!(unit.to_d());
    d.elements.insert("t".into(), de(&sh));
    let path_img = paint(&resolve_page(&d, "p", &ResolveOptions::default()));
    let ink = |img: &[u8]| img.chunks(4).filter(|p| p[0] < 128).count() as f64;
    let (a, bb) = (ink(&text_img), ink(&path_img));
    assert!(a > 500.0, "text drew ink: {a}");
    // The painter hints glyphs at this size; the outline is the exact (unhinted) shape.
    assert!((a - bb).abs() / a < 0.06, "ink {a} vs {bb}");
}

/// `doc` with extra fields merged into the content rect `c` and, when given, the mask `m`.
fn timed_doc(mode: Option<&str>, c_extra: Value, mask_extra: Option<Value>) -> DesignDoc {
    let mut els = serde_json::Map::new();
    let mut g = json!({ "id": "g", "type": "group", "parentId": "p", "z": "a0", "opacity": 1, "xf": { "x": 0, "y": 0, "w": 200, "h": 100, "rot": 0 } });
    if let Some(m) = mode {
        g["mask"] = json!({ "id": "m", "mode": m });
    }
    els.insert("g".into(), g);
    let mut c = shape("c", "g", "a0", 0.0, 0.0, 50.0, 100.0, "rect", solid(1.0, 0.0, 0.0));
    for (k, v) in c_extra.as_object().unwrap() {
        c[k.as_str()] = v.clone();
    }
    els.insert("c".into(), c);
    if let Some(extra) = mask_extra {
        let mut m = shape("m", "g", "a1", 0.0, 0.0, 200.0, 100.0, "rect", solid(1.0, 1.0, 1.0));
        for (k, v) in extra.as_object().unwrap() {
            m[k.as_str()] = v.clone();
        }
        els.insert("m".into(), m);
    }
    de(&json!({
        "meta": { "schemaVersion": 1, "minClientSchema": 1, "layoutVersion": 1, "surface": "video",
                  "dims": { "w": 200, "h": 100, "unit": "px", "dpi": 96 }, "lang": "en", "title": "m", "epoch": 0 },
        "pages": { "p": { "id": "p", "order": "a0", "background": [solid(1.0, 1.0, 1.0)], "durationMs": 3000 } },
        "elements": els,
        "assets": {}
    }))
}

fn at(d: &DesignDoc, t: Option<f64>) -> Vec<u8> {
    paint(&resolve_page(d, "p", &ResolveOptions { time_ms: t, ..Default::default() }))
}

const RED: [u8; 3] = [255, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

#[test]
fn elements_inside_a_group_follow_their_own_keyframes() {
    let d = timed_doc(None, json!({ "keyframes": { "x": [{ "t": 0, "v": 0 }, { "t": 1000, "v": 150 }] } }), None);
    // Without a time the page is drawn at rest.
    let still = at(&d, None);
    assert!(near(px(&still, 200, 25, 50), RED, 2) && near(px(&still, 200, 175, 50), WHITE, 2));
    let start = at(&d, Some(0.0));
    assert!(near(px(&start, 200, 25, 50), RED, 2) && near(px(&start, 200, 175, 50), WHITE, 2));
    let end = at(&d, Some(2000.0));
    assert!(near(px(&end, 200, 25, 50), WHITE, 2), "it has moved away: {:?}", px(&end, 200, 25, 50));
    assert!(near(px(&end, 200, 175, 50), RED, 2), "to the right: {:?}", px(&end, 200, 175, 50));
}

#[test]
fn elements_inside_a_group_show_only_in_their_window() {
    let d = timed_doc(None, json!({ "timing": { "startMs": 1000 } }), None);
    assert!(near(px(&at(&d, Some(500.0)), 200, 25, 50), WHITE, 2), "not yet shown");
    assert!(near(px(&at(&d, Some(1500.0)), 200, 25, 50), RED, 2), "shown");
    assert!(near(px(&at(&d, None), 200, 25, 50), RED, 2), "drawn at rest without a time");
}

#[test]
fn a_mask_outside_its_window_hides_what_it_gates() {
    let d = timed_doc(Some("alpha"), json!({}), Some(json!({ "timing": { "startMs": 1000 } })));
    assert!(near(px(&at(&d, Some(500.0)), 200, 25, 50), WHITE, 2), "mask not shown yet, so neither is the content");
    assert!(near(px(&at(&d, Some(1500.0)), 200, 25, 50), RED, 2));
}

#[test]
fn a_moving_mask_moves_the_window_onto_the_content() {
    // A 200 px mask slid 150 px right by keyframes: at the end only its left 50 px sit over the page from x 150.
    let d = timed_doc(Some("alpha"), json!({}), Some(json!({ "xf": { "x": 0, "y": 0, "w": 50, "h": 100, "rot": 0 }, "keyframes": { "x": [{ "t": 0, "v": 150 }, { "t": 1000, "v": 0 }] } })));
    assert!(near(px(&at(&d, Some(0.0)), 200, 25, 50), WHITE, 2), "mask away from the content");
    assert!(near(px(&at(&d, Some(2000.0)), 200, 25, 50), RED, 2), "mask over the content");
}

#[test]
fn keyed_width_height_and_anchor_scale_about_the_pivot() {
    // Half height, pivoting on the bottom edge: the rect keeps its bottom and loses its top half.
    let d = timed_doc(None, json!({ "keyframes": { "scaleY": [{ "t": 0, "v": 0.5 }], "anchorY": [{ "t": 0, "v": 1 }] } }), None);
    let img = at(&d, Some(0.0));
    assert!(near(px(&img, 200, 25, 25), WHITE, 2) && near(px(&img, 200, 25, 75), RED, 2));
    // About the centre (no anchor) it keeps the middle instead.
    let d = timed_doc(None, json!({ "keyframes": { "scaleY": [{ "t": 0, "v": 0.5 }] } }), None);
    let img = at(&d, Some(0.0));
    assert!(near(px(&img, 200, 25, 10), WHITE, 2) && near(px(&img, 200, 25, 50), RED, 2) && near(px(&img, 200, 25, 90), WHITE, 2));
    // Half width from the left edge; the height is untouched.
    let d = timed_doc(None, json!({ "keyframes": { "scaleX": [{ "t": 0, "v": 0.5 }], "anchorX": [{ "t": 0, "v": 0 }] } }), None);
    let img = at(&d, Some(0.0));
    assert!(near(px(&img, 200, 10, 50), RED, 2) && near(px(&img, 200, 40, 50), WHITE, 2) && near(px(&img, 200, 10, 95), RED, 2));
    // Drawn without a time, keys do nothing.
    assert!(near(px(&at(&d, None), 200, 40, 50), RED, 2));
}

fn srgb(r: f64, g: f64, b: f64, a: f64) -> Value {
    json!({ "space": "srgb", "r": r, "g": g, "b": b, "a": a })
}

#[test]
fn fill_keys_blend_between_colours() {
    let k = json!({ "fill": [{ "t": 0, "c": srgb(1.0, 0.0, 0.0, 1.0) }, { "t": 1000, "c": srgb(0.0, 0.0, 1.0, 1.0), "ease": "linear" }] });
    let d = timed_doc(None, json!({ "keyframes": k }), None);
    assert!(near(px(&at(&d, None), 200, 25, 50), RED, 2), "its own fill at rest");
    assert!(near(px(&at(&d, Some(500.0)), 200, 25, 50), [128, 0, 128], 3), "halfway: {:?}", px(&at(&d, Some(500.0)), 200, 25, 50));
    assert!(near(px(&at(&d, Some(2000.0)), 200, 25, 50), [0, 0, 255], 2));
}

#[test]
fn stroke_radius_and_shadow_keys_set_the_element_values() {
    let black = srgb(0.0, 0.0, 0.0, 1.0);
    // A 20 px centred stroke reaches 10 px past the right edge (x = 50).
    let d = timed_doc(None, json!({ "keyframes": { "strokeWidth": [{ "t": 0, "v": 20 }], "strokeColor": [{ "t": 0, "c": black }] } }), None);
    assert!(near(px(&at(&d, Some(0.0)), 200, 55, 50), [0, 0, 0], 2));
    assert!(near(px(&at(&d, None), 200, 55, 50), WHITE, 2), "no stroke at rest");
    // A rounded corner leaves the very corner white.
    let d = timed_doc(None, json!({ "keyframes": { "radius": [{ "t": 0, "v": 0 }, { "t": 1000, "v": 25 }] } }), None);
    assert!(near(px(&at(&d, Some(0.0)), 200, 1, 1), RED, 2));
    assert!(near(px(&at(&d, Some(1000.0)), 200, 1, 1), WHITE, 2));
    // A hard shadow 20 px to the right.
    let d = timed_doc(None, json!({ "keyframes": { "shadowX": [{ "t": 0, "v": 20 }], "shadowColor": [{ "t": 0, "c": black }] } }), None);
    assert!(near(px(&at(&d, Some(0.0)), 200, 60, 50), [0, 0, 0], 3), "{:?}", px(&at(&d, Some(0.0)), 200, 60, 50));
    assert!(near(px(&at(&d, None), 200, 60, 50), WHITE, 2));
}

#[test]
fn a_colour_track_that_does_not_read_is_left_out() {
    // Numbers where colours belong: the element still loads and draws with its own fill.
    let d = timed_doc(None, json!({ "keyframes": { "fill": [{ "t": 0, "v": 3 }], "x": [{ "t": 0, "v": 100 }] } }), None);
    let img = at(&d, Some(0.0));
    assert!(near(px(&img, 200, 125, 50), RED, 2), "{:?}", px(&img, 200, 125, 50));
}

/// A 200×100 white page holding the given elements.
fn page_of(els: Vec<Value>) -> DesignDoc {
    let mut m = serde_json::Map::new();
    for e in els {
        m.insert(e["id"].as_str().unwrap().to_owned(), e);
    }
    de(&json!({
        "meta": { "schemaVersion": 1, "minClientSchema": 1, "layoutVersion": 1, "surface": "video",
                  "dims": { "w": 200, "h": 100, "unit": "px", "dpi": 96 }, "lang": "en", "title": "f", "epoch": 0 },
        "pages": { "p": { "id": "p", "order": "a0", "background": [solid(1.0, 1.0, 1.0)], "durationMs": 3000 } },
        "elements": m,
        "assets": {}
    }))
}

const BLUE: [u8; 3] = [0, 0, 255];

#[test]
fn a_follower_takes_on_its_leaders_movement() {
    let mut a = shape("a", "p", "a0", 0.0, 0.0, 20.0, 100.0, "rect", solid(0.0, 0.0, 1.0));
    a["keyframes"] = json!({ "x": [{ "t": 0, "v": 0 }, { "t": 1000, "v": 100 }] });
    let mut b = shape("b", "p", "a1", 30.0, 0.0, 20.0, 100.0, "rect", solid(1.0, 0.0, 0.0));
    b["follow"] = json!({ "id": "a" });
    let d = page_of(vec![a, b.clone()]);
    assert!(near(px(&at(&d, None), 200, 40, 50), RED, 2), "at rest it stays put");
    assert!(near(px(&at(&d, Some(0.0)), 200, 40, 50), RED, 2));
    let end = at(&d, Some(1500.0));
    assert!(near(px(&end, 200, 140, 50), RED, 2) && near(px(&end, 200, 110, 50), BLUE, 2) && near(px(&end, 200, 40, 50), WHITE, 2));
    // A follower inside a group follows a leader on the page just the same.
    let g = json!({ "id": "g", "type": "group", "parentId": "p", "z": "a1", "opacity": 1, "xf": { "x": 30, "y": 0, "w": 20, "h": 100, "rot": 0 } });
    let mut bg = b;
    bg["parentId"] = json!("g");
    bg["xf"]["x"] = json!(0);
    let mut a = shape("a", "p", "a0", 0.0, 0.0, 20.0, 100.0, "rect", solid(0.0, 0.0, 1.0));
    a["keyframes"] = json!({ "x": [{ "t": 0, "v": 0 }, { "t": 1000, "v": 100 }] });
    let d = page_of(vec![a, g, bg]);
    assert!(near(px(&at(&d, Some(1500.0)), 200, 140, 50), RED, 2));
}

#[test]
fn a_follower_turns_around_its_leader() {
    let mut a = shape("a", "p", "a0", 90.0, 40.0, 20.0, 20.0, "rect", solid(0.0, 0.0, 1.0));
    a["keyframes"] = json!({ "rotate": [{ "t": 0, "v": 180 }] });
    let mut b = shape("b", "p", "a1", 120.0, 40.0, 20.0, 20.0, "rect", solid(1.0, 0.0, 0.0));
    b["follow"] = json!({ "id": "a" });
    let img = at(&page_of(vec![a, b]), Some(0.0));
    // Half a turn about the leader's centre (100, 50) carries it to the other side.
    assert!(near(px(&img, 200, 70, 50), RED, 2) && near(px(&img, 200, 130, 50), WHITE, 2));
}

#[test]
fn a_follow_cycle_or_a_missing_leader_moves_nobody() {
    let mut a = shape("a", "p", "a0", 0.0, 0.0, 20.0, 100.0, "rect", solid(0.0, 0.0, 1.0));
    a["keyframes"] = json!({ "x": [{ "t": 0, "v": 50 }] });
    a["follow"] = json!({ "id": "b" });
    let mut b = shape("b", "p", "a1", 150.0, 0.0, 20.0, 100.0, "rect", solid(1.0, 0.0, 0.0));
    b["follow"] = json!({ "id": "a" });
    let img = at(&page_of(vec![a, b]), Some(0.0));
    assert!(near(px(&img, 200, 60, 50), BLUE, 2), "the leader moves by its own keys only");
    assert!(near(px(&img, 200, 160, 50), RED, 2), "the follower stays put");
    let mut c = shape("c", "p", "a0", 0.0, 0.0, 20.0, 100.0, "rect", solid(1.0, 0.0, 0.0));
    c["follow"] = json!({ "id": "gone" });
    assert!(near(px(&at(&page_of(vec![c]), Some(0.0)), 200, 10, 50), RED, 2));
}
