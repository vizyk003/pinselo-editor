// SPDX-License-Identifier: Apache-2.0
//! Print PDFs: PDF/X-4 and PDF/X-1a:2003 written from the golden scenes and checked structurally
//! (`oc_render::pdfx_check`: output intent, GTS keys, boxes, embedded fonts, no RGB / no transparency in X-1a),
//! rasterised back with hayro, plus the rich-black rule and marks. The ECI profile comes from `pnpm icc:sync`;
//! without it these tests print a note and pass.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;

use common::*;
use oc_render::pdf::{OutputIntent, PdfOptions, PdfPage, PdfStandard, PrintSetup, write_pdf};
use oc_render::pdfx_check::{Report, check};
use oc_render::{DisplayList, ResolveOptions, resolve_page};
use oc_schema::DesignDoc;

fn icc_dir() -> PathBuf {
    std::env::var_os("OC_ICC_DIR").map(PathBuf::from).unwrap_or_else(|| root().join("fixtures/icc"))
}

fn fogra51() -> Option<OutputIntent> {
    let bytes = match oc_color::registry::read_bundled("pso-coated-v3", &icc_dir()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping: {e}");
            return None;
        }
    };
    let meta = oc_color::registry::find("pso-coated-v3").unwrap();
    Some(OutputIntent {
        profile: oc_color::OutputProfile::new(bytes).unwrap(),
        condition_id: meta.condition_id.clone(),
        condition: Some(meta.condition.clone()),
        registry: Some(oc_color::registry::registry_name().into()),
        info: Some(meta.name.clone()),
    })
}

fn scene(name: &str) -> (DesignDoc, Vec<String>) {
    let scenes = golden("scenes");
    let sc = scenes.as_array().unwrap().iter().find(|s| s["name"] == name).unwrap_or_else(|| panic!("scene {name}")).clone();
    let doc: DesignDoc = de(&sc["doc"]);
    let pages = oc_schema::tree::sorted_pages(&doc.pages).iter().map(|p| p.id.clone()).collect();
    (doc, pages)
}

fn setup(oi: &OutputIntent, standard: PdfStandard) -> PrintSetup {
    PrintSetup { bleed_mm: 3.0, crop_marks: true, registration_marks: true, color_bars: true, standard, output: Some(oi.clone()), ..PrintSetup::default() }
}

fn write(r: &mut oc_render::Renderer, dls: &[DisplayList], assets: &HashMap<String, Vec<u8>>, print: PrintSetup, title: &str) -> Vec<u8> {
    for d in dls {
        ensure_fonts(r, d);
    }
    let pages: Vec<PdfPage<'_>> = dls.iter().map(|dl| PdfPage { dl, links: vec![(oc_schema::primitives::Rect { x: 1.0, y: 1.0, w: 10.0, h: 10.0 }, "https://example.com".into())] }).collect();
    let opts = PdfOptions { title: Some(title.into()), producer: Some("Pinselo".into()), lang: Some("en".into()), print: Some(print), created_unix: Some(1_790_000_000), ..Default::default() };
    write_pdf(r, &pages, assets, &opts).expect("pdf")
}

fn assert_clean(rep: &Report, what: &str) {
    assert!(rep.issues.is_empty(), "{what}: {:#?}", rep.issues);
}

/// Rasterise page 1 with hayro at 1 px per pt, cropped to the trim box (bleed + slug `margin_px` doc px).
fn hayro_trim(pdf: &[u8], margin_px: f64, w: f64, h: f64) -> Vec<u8> {
    let pdf = hayro::hayro_syntax::Pdf::new(pdf.to_vec()).expect("pdf parses");
    let p = pdf.pages().first().expect("page");
    let s = (1.0 / 0.75) as f32;
    let pix = hayro::render(p, &hayro::RenderCache::new(), &hayro::hayro_interpret::InterpreterSettings::default(), &hayro::RenderSettings::default(), &hayro::PixmapSettings { x_scale: s, y_scale: s, ..Default::default() });
    let pw = usize::from(pix.width());
    let mut data: Vec<u8> = pix.data_as_u8_slice().to_vec();
    oc_render::assets::unpremultiply(&mut data);
    let (m, w, h) = (margin_px.round() as usize, w as usize, h as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = (y + m) * pw + m;
        out.extend_from_slice(&data[row * 4..(row + w) * 4]);
    }
    out
}

/// Mean ΔE76 (sRGB → Lab) of two RGBA buffers over white.
fn mean_de(a: &[u8], b: &[u8]) -> f64 {
    let lab = |p: &[u8]| {
        let al = f64::from(p[3]) / 255.0;
        let rgb = [p[0], p[1], p[2]].map(|v| f64::from(v) / 255.0 * al + (1.0 - al));
        let m = oc_color::cie::rgb_to_xyz_d50(oc_color::cie::SRGB_PRIMARIES, oc_color::cie::D65);
        oc_color::cie::xyz_to_lab(oc_color::cie::rgb_to_xyz(&m, rgb))
    };
    let n = a.len() / 4;
    (0..n).map(|i| {
        let (x, y) = (lab(&a[i * 4..i * 4 + 4]), lab(&b[i * 4..i * 4 + 4]));
        ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
    }).sum::<f64>() / n as f64
}

fn hayro_ok(pdf: &[u8]) -> (u32, u32) {
    let pdf = hayro::hayro_syntax::Pdf::new(pdf.to_vec()).expect("pdf parses");
    let p = pdf.pages().first().expect("page");
    let pix = hayro::render(p, &hayro::RenderCache::new(), &hayro::hayro_interpret::InterpreterSettings::default(), &hayro::RenderSettings::default(), &hayro::PixmapSettings { x_scale: 1.0, y_scale: 1.0, ..Default::default() });
    (u32::from(pix.width()), u32::from(pix.height()))
}

const SCENES: [&str; 8] = ["shapes", "lines", "text", "images", "charts", "tables", "media", "photo-shapes"];

#[test]
fn pdfx4_scenes_pass_the_structure_check() {
    let Some(oi) = fogra51() else { return };
    let assets = scene_assets();
    let mut r = renderer();
    for name in SCENES {
        let (doc, pages) = scene(name);
        let dls: Vec<DisplayList> = pages.iter().map(|p| resolve_page(&doc, p, &ResolveOptions::default())).collect();
        let pdf = write(&mut r, &dls, &assets, setup(&oi, PdfStandard::PdfX4), name);
        let rep = check(&pdf, PdfStandard::PdfX4);
        assert_clean(&rep, name);
        assert_eq!(rep.output_condition.as_deref(), Some("FOGRA51"));
        assert_eq!(rep.pdfx_version.as_deref(), Some("PDF/X-4"));
        assert!(rep.header.starts_with("%PDF-1.6"), "{name}: {}", rep.header);
        assert!(!rep.color_spaces.contains("DeviceRGB") && !rep.image_spaces.contains("DeviceRGB"), "{name}: {rep:?}");
        assert!(!pdf.windows(5).any(|w| w == b"/Link"), "{name}: no link annotations in PDF/X");
        hayro_ok(&pdf);
        eprintln!("x4 {name}: images {:?}, spaces {:?}, ops {:?}, overprint gs {}", rep.image_spaces, rep.color_spaces, rep.color_ops, rep.overprint_states);
    }
}

#[test]
fn pdfx1a_scenes_are_cmyk_and_flat() {
    let Some(oi) = fogra51() else { return };
    let assets = scene_assets();
    let mut r = renderer();
    for name in SCENES {
        let (doc, pages) = scene(name);
        let dls: Vec<DisplayList> = pages.iter().map(|p| resolve_page(&doc, p, &ResolveOptions::default())).collect();
        let pdf = write(&mut r, &dls, &assets, setup(&oi, PdfStandard::PdfX1a), name);
        let rep = check(&pdf, PdfStandard::PdfX1a);
        assert_clean(&rep, name);
        assert!(rep.header.starts_with("%PDF-1.4"), "{name}: {}", rep.header);
        assert_eq!(rep.pdfx_version.as_deref(), Some("PDF/X-1:2003"));
        for s in &rep.image_spaces {
            assert!(matches!(s.as_str(), "DeviceCMYK" | "DeviceGray" | "Separation" | "ImageMask"), "{name}: image in {s}");
        }
        for s in &rep.color_spaces {
            assert!(matches!(s.as_str(), "DeviceCMYK" | "DeviceGray" | "Separation" | "DeviceN" | "Pattern" | "Indexed"), "{name}: colour space {s}");
        }
        assert!(!rep.color_ops.contains("rg") && !rep.color_ops.contains("RG"), "{name}: {:?}", rep.color_ops);
        // Geometry survives flattening: the CMYK page, rendered back (hayro's CMYK → RGB), stays close to the
        // RGB painter (a misplaced region would show as a large difference).
        let mm = 96.0 / 25.4;
        let theirs = hayro_trim(&pdf, 13.0 * mm, dls[0].width, dls[0].height);
        ensure_fonts(&mut r, &dls[0]);
        let ours = oc_render::raster::to_rgba8(&r.render(&dls[0], &assets, &oc_render::RenderOptions::default()).expect("render"));
        let de = mean_de(&ours, &theirs);
        eprintln!("x1a {name}: mean ΔE vs painter {de:.2}");
        assert!(de < 12.0, "{name}: mean ΔE {de}");
        eprintln!("x1a {name}: images {:?}, spaces {:?}, ops {:?}", rep.image_spaces, rep.color_spaces, rep.color_ops);
    }
}

#[test]
fn checker_catches_plain_pdfs() {
    // A plain RGB PDF (no output intent, DeviceRGB, transparency, links) fails both PDF/X checks.
    let assets = scene_assets();
    let mut r = renderer();
    let (doc, pages) = scene("photo-shapes");
    let dls: Vec<DisplayList> = pages.iter().map(|p| resolve_page(&doc, p, &ResolveOptions::default())).collect();
    let pdf = write(&mut r, &dls, &assets, PrintSetup::marks(3.0, true), "plain");
    let x4 = check(&pdf, PdfStandard::PdfX4);
    assert!(x4.issues.iter().any(|i| i.contains("OutputIntents")), "{:?}", x4.issues);
    let x1a = check(&pdf, PdfStandard::PdfX1a);
    assert!(x1a.issues.iter().any(|i| i.contains("header")), "{:?}", x1a.issues);
    assert!(x1a.issues.iter().any(|i| i.contains("GTS_PDFXVersion")), "{:?}", x1a.issues);
    assert!(check(&pdf, PdfStandard::None).issues.is_empty(), "plain PDF passes the generic checks");
}

#[test]
fn rich_black_rule_overprints_small_black_text() {
    let Some(oi) = fogra51() else { return };
    let assets = scene_assets();
    let mut r = renderer();
    let (mut doc, pages) = scene("text");
    // Make every run pure black, 10 px.
    let mut v = serde_json::to_value(&doc).unwrap();
    fn walk(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(m) => {
                if m.get("space").and_then(|s| s.as_str()) == Some("srgb") && m.contains_key("r") {
                    m.insert("r".into(), 0.0.into());
                    m.insert("g".into(), 0.0.into());
                    m.insert("b".into(), 0.0.into());
                    m.insert("a".into(), 1.0.into());
                }
                for x in m.values_mut() {
                    walk(x);
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(walk),
            _ => {}
        }
    }
    walk(&mut v);
    doc = serde_json::from_value(v).unwrap();
    let dls: Vec<DisplayList> = pages.iter().map(|p| resolve_page(&doc, p, &ResolveOptions::default())).collect();
    let pdf = write(&mut r, &dls, &assets, setup(&oi, PdfStandard::PdfX4), "black");
    let rep = check(&pdf, PdfStandard::PdfX4);
    assert_clean(&rep, "black text");
    assert!(rep.overprint_states > 0, "K-only black text overprints: {rep:?}");
    assert!(rep.color_ops.contains("k"), "DeviceCMYK K-only fill: {:?}", rep.color_ops);
    // Turned off: no overprint.
    let pdf = write(&mut r, &dls, &assets, PrintSetup { black_overprint: false, ..setup(&oi, PdfStandard::PdfX4) }, "black");
    let rep = check(&pdf, PdfStandard::PdfX4);
    assert_eq!(rep.overprint_states, 0);
}

#[test]
fn marks_widen_the_media_box_and_flatten_rgb_works_without_profile() {
    let assets = scene_assets();
    let mut r = renderer();
    let (doc, pages) = scene("photo-shapes");
    let dls: Vec<DisplayList> = pages.iter().map(|p| resolve_page(&doc, p, &ResolveOptions::default())).collect();
    let flat = PrintSetup { flatten: true, registration_marks: true, ..PrintSetup::marks(5.0, true) };
    let pdf = write(&mut r, &dls, &assets, flat, "flat");
    let rep = check(&pdf, PdfStandard::None);
    assert_clean(&rep, "rgb flatten");
    // Flattened RGB: no transparency left (the X-1a transparency rules hold), but RGB colour.
    let x1a = check(&pdf, PdfStandard::PdfX1a);
    assert!(!x1a.issues.iter().any(|i| i.contains("transparency") || i.contains("soft mask")), "{:?}", x1a.issues);
    let (w, _) = hayro_ok(&pdf);
    let mm = 72.0 / 25.4;
    assert_eq!(w, (dls[0].width * 0.75 + 2.0 * 15.0 * mm).floor() as u32);
}
