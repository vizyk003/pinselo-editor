//! Layout checks (`lint_page`) against the real renderer with the bundled fonts: each problem is found where it is,
//! and a clean page reports nothing.
mod common;

use std::path::PathBuf;

use oc_gfonts::manifest::{FontFamily, FontManifest};
use oc_render_svc::lint::lint_page;
use oc_render_svc::oc_renderer::OcRenderer;
use oc_render_svc::renderer::{RenderError, RenderFuture, RenderHost};
use oc_schema::color::solid_hex;
use oc_schema::elements::{Element, TextFrameMode};
use oc_schema::factory::{base_init, empty_design, make_shape, make_text, place_all_elements, xf, MetaInit, ShapeOpts, TextOpts};
use oc_schema::text::RunStyleOverride;
use oc_schema::tree::sorted_pages;
use oc_schema::DesignDoc;

struct Host {
    manifest: FontManifest,
    dir: PathBuf,
}

impl RenderHost for Host {
    fn asset_bytes<'a>(&'a self, key: &'a str) -> RenderFuture<'a, Vec<u8>> {
        let e = RenderError::Failed(format!("asset {key} not found"));
        Box::pin(async move { Err(e) })
    }
    fn font_file<'a>(&'a self, file: &'a str) -> RenderFuture<'a, Vec<u8>> {
        let r = common::bundled_font_file(file).map_err(RenderError::Failed);
        Box::pin(async move { r })
    }
    fn custom_families<'a>(&'a self, _: &'a [String]) -> RenderFuture<'a, Vec<FontFamily>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn manifest(&self) -> &FontManifest {
        &self.manifest
    }
    fn fonts_dir(&self) -> PathBuf {
        self.dir.clone()
    }
}

fn text(x: f64, y: f64, w: f64, h: f64, s: &str, size: f64, color: &str) -> Element {
    let style = RunStyleOverride { size: Some(size), fill: Some(solid_hex(color).unwrap()), ..Default::default() };
    Element::Text(make_text(base_init(xf(x, y, w, h, 0.0)), TextOpts { text: Some(s.into()), style: Some(style), ..Default::default() }))
}

/// A 1080×1080 page with a dark band at the top and the given elements (the band is drawn first).
fn page(els: Vec<Element>) -> (DesignDoc, String) {
    let mut doc = empty_design(&MetaInit { w: 1080.0, h: 1080.0, ..Default::default() });
    let page = sorted_pages(&doc.pages)[0].id.to_string();
    let band = Element::Shape(make_shape(base_init(xf(0.0, 0.0, 1080.0, 400.0, 0.0)), ShapeOpts { fill: Some(solid_hex("#0b4a55").unwrap()), ..Default::default() }));
    let mut all = vec![band];
    all.extend(els);
    place_all_elements(&page, &mut all, None);
    for e in all {
        let id = e.base().unwrap().id.to_string();
        doc.elements.insert(id, e);
    }
    (doc, page)
}

async fn kinds(doc: &DesignDoc, page: &str) -> Vec<(&'static str, String)> {
    let host = Host { manifest: common::bundled_manifest(), dir: common::bundled_fonts_dir() };
    let r = OcRenderer::new(&host);
    lint_page(&r, &host, doc, page).await.into_iter().map(|i| (i.kind, i.id)).collect()
}

fn id(doc: &DesignDoc, i: usize) -> String {
    let mut ids: Vec<(String, String)> = doc.elements.values().map(|e| (e.base().unwrap().z.clone(), e.base().unwrap().id.to_string())).collect();
    ids.sort();
    ids[i].1.clone()
}

#[tokio::test]
async fn a_clean_page_has_no_issues() {
    let (doc, p) = page(vec![
        text(80.0, 120.0, 920.0, 0.0, "Autumn sale", 96.0, "#ffffff"),
        text(80.0, 520.0, 920.0, 0.0, "Everything 30% off this weekend only.", 40.0, "#111827"),
    ]);
    assert_eq!(kinds(&doc, &p).await, vec![]);
}

#[tokio::test]
async fn finds_each_problem_on_the_element_that_has_it() {
    let mut fixed = text(80.0, 560.0, 300.0, 60.0, "This long paragraph cannot possibly fit into a box that is only sixty pixels tall at this size.", 32.0, "#111827");
    if let Element::Text(t) = &mut fixed {
        t.frame.mode = TextFrameMode::Fixed;
    }
    let (doc, p) = page(vec![
        // 1: dark text on the dark band.
        text(80.0, 120.0, 920.0, 0.0, "Hard to read", 64.0, "#123c44"),
        // 2: fixed box too small for its text.
        fixed,
        // 3: runs past the right edge.
        text(900.0, 700.0, 400.0, 0.0, "Off the edge", 40.0, "#111827"),
        // 4: entirely below the page.
        text(80.0, 1200.0, 300.0, 0.0, "Lost", 40.0, "#111827"),
        // 5 + 6: two texts on top of each other.
        text(80.0, 860.0, 600.0, 0.0, "First line here", 48.0, "#111827"),
        text(90.0, 870.0, 600.0, 0.0, "Second line here", 48.0, "#111827"),
        // 7: tiny.
        text(700.0, 560.0, 300.0, 0.0, "fine print", 9.0, "#111827"),
    ]);
    let found = kinds(&doc, &p).await;
    let has = |k: &str, i: usize| found.iter().any(|(kk, ii)| *kk == k && *ii == id(&doc, i));
    assert!(has("low_contrast", 1), "{found:?}");
    assert!(has("text_overflow", 2), "{found:?}");
    assert!(has("text_off_page", 3), "{found:?}");
    assert!(has("off_page", 4), "{found:?}");
    assert!(has("text_overlap", 6), "{found:?}");
    assert!(has("tiny_text", 7), "{found:?}");
    // The lower of the two overlapping texts is otherwise fine: it gets no issue of its own.
    assert!(!found.iter().any(|(_, i)| *i == id(&doc, 5)), "{found:?}");
}

