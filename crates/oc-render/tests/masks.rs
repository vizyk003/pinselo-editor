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
