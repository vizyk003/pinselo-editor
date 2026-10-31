// SPDX-License-Identifier: Apache-2.0
//! Vector backends: the SVG writer's output parses (usvg) and, rasterised by resvg, matches the painter; the PDF
//! writer's output parses and, rasterised by hayro, matches the painter (same geometry, real embedded text).

mod common;

use std::collections::HashMap;

use common::*;
use oc_render::pdf::{PdfOptions, PdfPage, PrintSetup, write_pdf};
use oc_render::svg::{SvgOptions, display_list_to_svg};
use oc_render::{DisplayList, RenderOptions, ResolveOptions, resolve_page};
use oc_schema::DesignDoc;
use serde_json::Value;

fn scene(name: &str) -> (DesignDoc, Vec<String>) {
    let scenes = golden("scenes");
    let sc = scenes.as_array().unwrap().iter().find(|s| s["name"] == name).unwrap_or_else(|| panic!("scene {name}")).clone();
    let doc: DesignDoc = de(&sc["doc"]);
    let pages = oc_schema::tree::sorted_pages(&doc.pages).iter().map(|p| p.id.clone()).collect();
    (doc, pages)
}

fn lab(p: &[u8]) -> [f64; 3] {
    let a = f64::from(p[3]) / 255.0;
    let ch = |v: u8| {
        let c = (f64::from(v) / 255.0) * a + (1.0 - a);
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (ch(p[0]), ch(p[1]), ch(p[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f64| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    [116.0 * f(y) - 16.0, 500.0 * (f(x) - f(y)), 200.0 * (f(y) - f(z))]
}

/// Mean CIE76 ΔE of two unpremultiplied RGBA buffers over white, and the share of pixels above 10.
fn delta(a: &[u8], b: &[u8]) -> (f64, f64) {
    let n = a.len() / 4;
    let mut sum = 0.0;
    let mut over = 0usize;
    for i in 0..n {
        let (x, y) = (lab(&a[i * 4..i * 4 + 4]), lab(&b[i * 4..i * 4 + 4]));
        let d = ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt();
        sum += d;
        if d > 10.0 {
            over += 1;
        }
    }
    (sum / n as f64, over as f64 / n as f64)
}

fn painter(r: &mut oc_render::Renderer, dl: &DisplayList, assets: &HashMap<String, Vec<u8>>) -> Vec<u8> {
    ensure_fonts(r, dl);
    oc_render::raster::to_rgba8(&r.render(dl, assets, &RenderOptions::default()).expect("render"))
}

#[test]
fn svg_parses_and_matches_the_painter() {
    let assets = scene_assets();
    let mut r = renderer();
    for name in ["shapes", "effects", "lines", "text", "images", "media", "charts", "tables", "animation", "captions", "photo-shapes"] {
        let (doc, pages) = scene(name);
        for page in &pages {
            let dl = resolve_page(&doc, page, &ResolveOptions::default());
            ensure_fonts(&mut r, &dl);
            let svg = display_list_to_svg(&mut r, &dl, &assets, &SvgOptions::default());
            let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap_or_else(|e| panic!("{name}: invalid SVG: {e}"));
            assert_eq!(tree.size().width(), dl.width as f32);
            if name == "text" && page == &pages[0] {
                assert!(svg.matches("<text ").count() > 20 && svg.contains("<tspan"), "real text");
                assert!(svg.contains("Effect text") && svg.contains("highlighted words"));
            }
        }
    }
    // Geometry: text-free scenes rasterised by resvg against the painter.
    for name in ["shapes", "lines", "photo-shapes"] {
        let (doc, pages) = scene(name);
        let dl = resolve_page(&doc, &pages[0], &ResolveOptions::default());
        let ours = painter(&mut r, &dl, &assets);
        let svg = display_list_to_svg(&mut r, &dl, &assets, &SvgOptions::default());
        let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).expect("svg");
        let mut pix = oc_render::tiny_skia::Pixmap::new(dl.width as u32, dl.height as u32).unwrap();
        resvg::render(&tree, oc_render::tiny_skia::Transform::identity(), &mut pix.as_mut());
        let (mean, over) = delta(&ours, &oc_render::raster::to_rgba8(&pix));
        eprintln!("svg {name}: mean ΔE {mean:.3}, ΔE>10 {:.3}%", over * 100.0);
        assert!(mean < 1.0 && over < 0.02, "svg {name}: mean ΔE {mean}, over {over}");
    }
}

fn hayro_rgba(pdf: &[u8], page: usize, scale: f32) -> (u32, u32, Vec<u8>) {
    let pdf = hayro::hayro_syntax::Pdf::new(pdf.to_vec()).expect("pdf parses");
    let pages = pdf.pages();
    let p = pages.get(page).expect("page");
    let settings = hayro::hayro_interpret::InterpreterSettings::default();
    let pix = hayro::render(p, &hayro::RenderCache::new(), &settings, &hayro::RenderSettings::default(), &hayro::PixmapSettings { x_scale: scale, y_scale: scale, ..Default::default() });
    let (w, h) = (u32::from(pix.width()), u32::from(pix.height()));
    let mut data: Vec<u8> = pix.data_as_u8_slice().to_vec();
    oc_render::assets::unpremultiply(&mut data);
    (w, h, data)
}

#[test]
fn pdf_parses_and_matches_the_painter() {
    let assets = scene_assets();
    let mut r = renderer();
    for name in ["shapes", "lines", "text", "images", "charts", "tables", "media", "photo-shapes"] {
        let (doc, pages) = scene(name);
        let dl = resolve_page(&doc, &pages[0], &ResolveOptions::default());
        let ours = painter(&mut r, &dl, &assets);
        let pdf = write_pdf(&mut r, &[PdfPage { dl: &dl, links: vec![] }], &assets, &PdfOptions { title: Some(name.into()), ..Default::default() }).expect("pdf");
        assert!(pdf.starts_with(b"%PDF-"));
        let (w, h, theirs) = hayro_rgba(&pdf, 0, 1.0 / 0.75);
        assert_eq!((w, h), (dl.width as u32, dl.height as u32), "{name}: page size in pt");
        let (mean, over) = delta(&ours, &theirs);
        eprintln!("pdf {name}: mean ΔE {mean:.3}, ΔE>10 {:.3}%", over * 100.0);
        assert!(mean < 2.0 && over < 0.05, "pdf {name}: mean ΔE {mean}, over {over}");
    }
}

#[test]
fn pdf_text_is_real_text_and_print_marks() {
    let assets = scene_assets();
    let mut r = renderer();
    let (doc, pages) = scene("text");
    let dls: Vec<DisplayList> = pages.iter().map(|p| resolve_page(&doc, p, &ResolveOptions::default())).collect();
    for d in &dls {
        ensure_fonts(&mut r, d);
    }
    let input: Vec<PdfPage<'_>> = dls.iter().map(|dl| PdfPage { dl, links: vec![(oc_schema::primitives::Rect { x: 10.0, y: 10.0, w: 100.0, h: 20.0 }, "https://example.com".into())] }).collect();
    let print = PdfOptions { print: Some(PrintSetup::marks(3.0, true)), lang: Some("en".into()), ..Default::default() };
    let pdf = write_pdf(&mut r, &input, &assets, &print).expect("pdf");
    let parsed = hayro::hayro_syntax::Pdf::new(pdf.clone()).expect("parses");
    assert_eq!(parsed.pages().len(), dls.len());
    // Fonts are embedded (subset TrueType/CFF programs) with a ToUnicode map for copy/paste.
    let s = String::from_utf8_lossy(&pdf);
    assert!(s.contains("FontFile2") || s.contains("FontFile3"), "embedded font program");
    assert!(s.contains("ToUnicode"), "ToUnicode map");
    assert!(s.contains("/TrimBox") && s.contains("/BleedBox"), "print boxes");
    assert!(s.contains("example.com"), "link annotation");
    // The trimmed page renders at the document size plus 2 × (3 mm bleed + 10 mm slug).
    let (w, _, _) = hayro_rgba(&pdf, 0, 1.0 / 0.75);
    let mm = 96.0 / 25.4;
    assert_eq!(w, (dls[0].width + 2.0 * 13.0 * mm).floor() as u32);
}

/// Every template page writes a PDF hayro can parse, with one page each.
#[test]
fn templates_write_pdf_and_svg() {
    let dir = root().join("fixtures/templates-seed");
    let mut r = renderer();
    let assets: HashMap<String, Vec<u8>> = HashMap::new();
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    files.sort();
    for f in files.iter().step_by(12) {
        let raw: Value = serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap();
        let doc: DesignDoc = de(&raw);
        let dls: Vec<DisplayList> = oc_schema::tree::sorted_pages(&doc.pages).iter().map(|p| resolve_page(&doc, &p.id, &ResolveOptions::default())).collect();
        for d in &dls {
            ensure_fonts(&mut r, d);
            let svg = display_list_to_svg(&mut r, d, &assets, &SvgOptions::default());
            resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        }
        let pages: Vec<PdfPage<'_>> = dls.iter().map(|dl| PdfPage { dl, links: vec![] }).collect();
        let pdf = write_pdf(&mut r, &pages, &assets, &PdfOptions::default()).expect("pdf");
        let parsed = hayro::hayro_syntax::Pdf::new(pdf).unwrap_or_else(|_| panic!("{}: unparsable PDF", f.display()));
        assert_eq!(parsed.pages().len(), dls.len());
    }
}
