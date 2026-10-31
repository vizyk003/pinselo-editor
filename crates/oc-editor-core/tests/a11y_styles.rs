// SPDX-License-Identifier: Apache-2.0
//! Ports of packages/editor-core/src/a11y.test.ts and styles.test.ts.

use indexmap::IndexMap;
use oc_crdt::DocOp;
use oc_editor_core::{A11yIssueCode, ElementKindLabel, FontSet, audit_design, describe_element, design_colors, font_set_ops, palette_map, recolor_ops};
use oc_schema::color::{solid_hex, to_hex};
use oc_schema::elements::{Element, ImageEl};
use oc_schema::factory::{MetaInit, ShapeOpts, TextOpts, base_init, empty_design, make_shape, make_text};
use oc_schema::primitives::{StoredRect, Xf};
use oc_schema::text::RunStyleOverride;
use oc_schema::tree::sorted_pages;
use oc_schema::DesignDoc;
use serde_json::Value;

fn text_el(page: &str, z: &str, text: &str, size: f64, fill: &str, xf: Xf) -> oc_schema::elements::TextEl {
    let mut b = base_init(xf);
    b.parent_id = page.into();
    b.z = z.into();
    make_text(b, TextOpts {
        text: Some(text.into()),
        style: Some(RunStyleOverride { size: Some(size), fill: solid_hex(fill).ok(), ..Default::default() }),
        ..Default::default()
    })
}

fn a11y_doc() -> DesignDoc {
    let mut d = empty_design(&MetaInit { w: 1080.0, h: 1080.0, ..Default::default() });
    d.meta.lang = "en".into();
    d
}

#[test]
fn flags_missing_language_alt_text_low_contrast_and_small_text() {
    let mut d = a11y_doc();
    let page = sorted_pages(&d.pages)[0].id.clone();
    d.meta.lang = String::new();
    let txt = text_el(&page, "a", "Faint", 8.0, "#eeeeee", Xf::new(10.0, 10.0, 300.0, 40.0, 0.0));
    let mut ib = txt.base.clone();
    ib.id = "img00000000000001".into();
    ib.z = "b".into();
    let img = ImageEl { base: ib, asset: "a".into(), crop: StoredRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0, extra: Default::default() }, edits: None, corner_radius: None, svg_colors: None };
    d.elements.insert(txt.base.id.clone(), Element::Text(txt));
    d.elements.insert("img00000000000001".into(), Element::Image(img));
    let mut codes: Vec<A11yIssueCode> = audit_design(&d).into_iter().map(|i| i.code).collect();
    codes.sort_by_key(|c| serde_json::to_string(c).unwrap());
    assert_eq!(codes, vec![A11yIssueCode::AltMissing, A11yIssueCode::LowContrast, A11yIssueCode::NoLanguage, A11yIssueCode::SmallText]);
}

#[test]
fn passes_readable_text_and_decorative_images() {
    let mut d = a11y_doc();
    let page = sorted_pages(&d.pages)[0].id.clone();
    let txt = text_el(&page, "a", "Readable", 32.0, "#111111", Xf::new(10.0, 10.0, 300.0, 40.0, 0.0));
    d.elements.insert(txt.base.id.clone(), Element::Text(txt.clone()));
    assert!(audit_design(&d).is_empty());
    let desc = describe_element(&d, &Element::Text(txt));
    assert_eq!(desc.kind, ElementKindLabel::Text);
    assert_eq!(desc.label, "Readable");
    assert_eq!(desc.w, 300.0);
}

struct StyleDoc {
    d: DesignDoc,
    sh: String,
    big: String,
    small: String,
}

fn style_doc() -> StyleDoc {
    let mut d = empty_design(&MetaInit { w: 1000.0, h: 1000.0, ..Default::default() });
    let page = sorted_pages(&d.pages)[0].id.clone();
    let mut sb = base_init(Xf::new(0.0, 0.0, 100.0, 100.0, 0.0));
    sb.parent_id = page.clone();
    sb.z = "a".into();
    let sh = make_shape(sb, ShapeOpts { fill: solid_hex("#ff0000").ok(), ..Default::default() });
    let big = text_el(&page, "b", "Big title", 80.0, "#111111", Xf::new(0.0, 200.0, 500.0, 100.0, 0.0));
    let small = text_el(&page, "c", "Body copy", 20.0, "#ff0000", Xf::new(0.0, 400.0, 500.0, 40.0, 0.0));
    let ids = (sh.base.id.clone(), big.base.id.clone(), small.base.id.clone());
    d.elements.insert(ids.0.clone(), Element::Shape(sh));
    d.elements.insert(ids.1.clone(), Element::Text(big));
    d.elements.insert(ids.2.clone(), Element::Text(small));
    StyleDoc { d, sh: ids.0, big: ids.1, small: ids.2 }
}

#[test]
fn lists_and_replaces_colours_everywhere() {
    let s = style_doc();
    assert_eq!(design_colors(&s.d)[0].hex, "#ff0000");
    let map: IndexMap<String, String> = [("#ff0000".to_string(), "#00aa55".to_string())].into_iter().collect();
    let ops = recolor_ops(&s.d, &map);
    let patch = |id: &str| {
        ops.iter()
            .find_map(|o| match o {
                DocOp::Update { id: i, patch } if i == id => Some(patch.clone()),
                _ => None,
            })
            .unwrap()
    };
    let fills = patch(&s.sh)["fills"].clone();
    let c: oc_schema::primitives::ColorValue = serde_json::from_value(fills[0]["color"].clone()).unwrap();
    assert_eq!(to_hex(&c, false), "#00aa55");
    assert!(serde_json::to_string(&Value::Object(patch(&s.small))).unwrap().contains("\"g\":0.666"));
}

#[test]
fn applies_a_palette_by_luminance_rank() {
    let s = style_doc();
    let m = palette_map(&s.d, &["#fef3c7".into(), "#1e3a8a".into(), "#f59e0b".into()]);
    assert_eq!(m.get("#111111").map(String::as_str), Some("#1e3a8a"));
    assert!(m.get("#ffffff").is_none());
    assert_eq!(m.get("#ff0000").map(String::as_str), Some("#f59e0b"));
}

#[test]
fn applies_fonts_by_size_rank() {
    let s = style_doc();
    let ops = font_set_ops(&s.d, &FontSet { heading: "Fraunces".into(), subheading: None, body: "Inter".into() });
    let fam = |id: &str| {
        ops.iter()
            .find_map(|o| match o {
                DocOp::Update { id: i, patch } if i == id => Some(patch["body"]["defaultRun"]["family"].as_str().unwrap().to_string()),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(fam(&s.big), "Fraunces");
    assert_eq!(fam(&s.small), "Inter");
}
