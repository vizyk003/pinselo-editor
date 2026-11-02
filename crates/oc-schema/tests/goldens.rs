// SPDX-License-Identifier: Apache-2.0
//! Replays the TS-generated goldens in tests/golden/schema/ against the Rust port.

mod common;

use common::*;
use indexmap::IndexMap;
use oc_schema::color::{self, hex};
use oc_schema::elements::{Element, Keyframe, KeyframeEase};
use oc_schema::primitives::{ColorValue, Paint, Unit, Xf};
use oc_schema::text::{Run, TextAlign, TextBody};
use oc_schema::{fractional, ids, js, keyframes, presets};
use serde_json::{Value, json};

fn report(name: &str, n: usize) {
    eprintln!("{name}: {n} golden cases ok");
}

// ------------------------------------------------------------------ constants

#[test]
fn constants() {
    use oc_schema::{design, doc_blocks, elements, embeds, factory, frames, print, svg, table};
    let g = golden("constants");
    let checked = std::cell::RefCell::new(Vec::<String>::new());
    let check = |k: &str, v: Value| {
        checked.borrow_mut().push(k.to_string());
        if let Some(d) = diff(&v, &g[k], k) {
            panic!("{d}");
        }
    };
    check("ELEMENT_TYPES", j(&elements::ELEMENT_TYPES));
    check("FILTER_PRESETS", j(&elements::FILTER_PRESETS));
    check("ANIM_PRESETS", j(&elements::ANIM_PRESETS));
    check("KEYFRAME_PROPS", j(&elements::KEYFRAME_PROPS));
    check("EMPHASIS_PRESETS", j(&elements::EMPHASIS_PRESETS));
    check("SHAPE_KINDS", j(&elements::SHAPE_KINDS));
    check("LINE_MARKERS", j(&elements::LINE_MARKERS));
    check("CHART_KINDS", j(&elements::CHART_KINDS));
    check("TRANSITION_KINDS", j(&design::TRANSITION_KINDS));
    check("BASE_KEYS", j(&elements::BASE_KEYS));
    check("FRAME_CATEGORIES", j(&frames::FRAME_CATEGORIES));
    check("BORDER_PRESETS", j(&table::BORDER_PRESETS));
    check("TABLE_STYLE_PRESETS", j(&table::TABLE_STYLE_PRESETS));
    check("EMBED_FRAME_HOSTS", j(&embeds::EMBED_FRAME_HOSTS));
    check("CURRENT_SCHEMA", j(&design::CURRENT_SCHEMA));
    check("LAYOUT_VERSION", j(&design::LAYOUT_VERSION));
    check("DEFAULT_PAGE_MS", j(&oc_schema::timeline::DEFAULT_PAGE_MS));
    check("KEYFRAME_NEUTRAL", j(&keyframes::KEYFRAME_NEUTRAL.iter().copied().collect::<IndexMap<_, _>>()));
    let p = oc_schema::text::default_paragraph();
    check("DEFAULT_PARAGRAPH", json!({ "align": p.align, "lineHeight": p.line_height, "spaceAfter": p.space_after }));
    check("DOC_CONTENT_WIDTH", j(&doc_blocks::DOC_CONTENT_WIDTH));
    check("MAX_SVG_COLORS", j(&svg::MAX_SVG_COLORS));
    check("CURRENT_COLOR", j(&svg::CURRENT_COLOR));
    check("DEFAULT_FONT", j(&factory::DEFAULT_FONT));
    check("DEFAULT_RUN", j(&*factory::DEFAULT_RUN));
    check("CHART_COLORS", j(&factory::CHART_COLORS));
    check("GRID_LAYOUTS", j(&*factory::GRID_LAYOUTS));
    check("SIZE_PRESETS", j(&*presets::SIZE_PRESETS));
    let unit_px: IndexMap<&str, f64> = Unit::VARIANTS.iter().map(|u| (u.as_str(), presets::unit_px(u))).collect();
    check("UNIT_PX", j(&unit_px));
    check("CALLOUT_TONES", j(&doc_blocks::CALLOUT_TONES.iter().copied().collect::<IndexMap<_, _>>()));
    use doc_blocks::doc_nodes as n;
    check(
        "DOC_NODES",
        json!({ "designEmbed": n::DESIGN_EMBED, "chart": n::CHART, "callout": n::CALLOUT, "toc": n::TOC, "pageBreak": n::PAGE_BREAK, "videoEmbed": n::VIDEO_EMBED }),
    );
    check("DEFAULT_PRINT_CATALOG", j(&*print::DEFAULT_PRINT_CATALOG));
    check("FRAME_LIBRARY", j(&*frames::FRAME_LIBRARY));
    check("PRESET_CATEGORIES", j(&presets::PRESET_CATEGORIES));
    check("POPULAR_LAYOUTS", j(&presets::POPULAR_LAYOUTS));
    // Every constant the generator records must be checked above (a newly recorded one fails here until ported).
    let unchecked: Vec<&String> = g.as_object().unwrap().keys().filter(|k| !checked.borrow().contains(k)).collect();
    assert!(unchecked.is_empty(), "constants.json keys not checked: {unchecked:?}");
}

// ------------------------------------------------------------------ colour

#[test]
fn color_goldens() {
    let n = replay("color", &golden("color"), |name, a| {
        let c = || de::<ColorValue>(&a[0]);
        match name {
            "rgb" => ok(color::rgb(f(&a[0]), f(&a[1]), f(&a[2]), a.get(3).map_or(1.0, f))),
            "hex" => res(hex(s(&a[0]))),
            "toRgba" => ok(color::to_rgba(&c())),
            "toHex" => ok(color::to_hex(&c(), a[1].as_bool().unwrap())),
            "toCss" => ok(color::to_css(&c())),
            "luminance" => num(color::luminance(&c())).into(),
            "solid" => match &a[0] {
                Value::String(h) => res(color::solid_hex(h)),
                v => ok(color::solid(de(v))),
            },
            "contrastRatio" => num(color::contrast_ratio(&c(), &de(&a[1]))).into(),
            "paintColor" => {
                let p: Option<Paint> = opt(&a[0]);
                opt_out(color::paint_color(p.as_ref()), undef())
            }
            _ => panic!("unknown fn {name}"),
        }
    });
    report("color", n);
}

// ------------------------------------------------------------------ ids / fractional indexing

#[test]
fn ids_goldens() {
    let gkb = |a: Option<&str>, b: Option<&str>| fractional::generate_key_between(a, b).expect("key");
    let n = replay("ids", &golden("ids"), |name, a| {
        let key = |v: &Value| v.as_str().map(str::to_string);
        match name {
            "isId" => ok(ids::is_id(s(&a[0]))),
            "compareOrder" => {
                let o = ids::compare_order(s(&a[0]["key"]), s(&a[0]["id"]), s(&a[1]["key"]), s(&a[1]["id"]));
                ok(o as i32)
            }
            "generateKeyBetween" => res(ids::key_between(key(&a[0]).as_deref(), key(&a[1]).as_deref(), false)),
            "keysBetween" => res(ids::keys_between(key(&a[0]).as_deref(), key(&a[1]).as_deref(), f(&a[2]) as usize)),
            "appendSeq" | "prependSeq" => {
                let mut out: Vec<String> = Vec::new();
                let mut k: Option<String> = None;
                for _ in 0..f(&a[0]) as usize {
                    let next = if name == "appendSeq" { gkb(k.as_deref(), None) } else { gkb(None, k.as_deref()) };
                    out.push(next.clone());
                    k = Some(next);
                }
                ok(out)
            }
            "bisectLow" | "bisectHigh" => {
                let (mut lo, mut hi) = ("a0".to_string(), "a1".to_string());
                let mut out = Vec::new();
                for _ in 0..f(&a[0]) as usize {
                    let m = gkb(Some(&lo), Some(&hi));
                    if name == "bisectLow" {
                        hi = m.clone();
                    } else {
                        lo = m.clone();
                    }
                    out.push(m);
                }
                ok(out)
            }
            "walk" => {
                let mut list: Vec<String> = Vec::new();
                let mut st = f(&a[1]) as u32;
                for _ in 0..f(&a[0]) as usize {
                    st = st.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    let pos = (st % (list.len() as u32 + 1)) as usize;
                    let lo = if pos > 0 { list.get(pos - 1).cloned() } else { None };
                    let hi = list.get(pos).cloned();
                    list.insert(pos, gkb(lo.as_deref(), hi.as_deref()));
                }
                ok(list)
            }
            _ => panic!("unknown fn {name}"),
        }
    });
    report("ids", n);
}

// ------------------------------------------------------------------ text

#[test]
fn text_goldens() {
    use oc_schema::text::{RunStyleOverride, body_plain_text, resolve_run_style};
    let n = replay("text", &golden("text"), |name, a| match name {
        "bodyPlainText" => ok(body_plain_text(&de(&a[0]))),
        "resolveRunStyle" => ok(resolve_run_style(&de::<TextBody>(&a[0]), &de::<Run>(&a[1]))),
        "textBody" => ok(oc_schema::factory::text_body(s(&a[0]), &de::<RunStyleOverride>(&a[1]), de::<TextAlign>(&a[2]))),
        _ => panic!("unknown fn {name}"),
    });
    report("text", n);
}

// ------------------------------------------------------------------ presets + keyframes

#[test]
fn presets_keyframes_goldens() {
    let n = replay("presets_keyframes", &golden("presets_keyframes"), |name, a| match name {
        "pxToUnit" => num(presets::px_to_unit(f(&a[0]), &de(&a[1]))).into(),
        "unitToPx" => num(presets::unit_to_px(f(&a[0]), &de(&a[1]))).into(),
        "formatDims" => ok(presets::format_dims(f(&a[0]), f(&a[1]), &de(&a[2]))),
        "ease" => num(keyframes::ease(&de::<KeyframeEase>(&a[0]), f(&a[1]))).into(),
        "keyframeValue" => {
            let keys: Option<Vec<Keyframe>> = opt(&a[0]);
            num(keyframes::keyframe_value(keys.as_deref(), f(&a[1]), f(&a[2]))).into()
        }
        _ => panic!("unknown fn {name}"),
    });
    report("presets_keyframes", n);
}

// ------------------------------------------------------------------ presets: Create-dialog helpers

#[test]
fn presets_goldens() {
    use presets::{PresetCategoryId, PresetDesignOverrides, SizePreset, SizeSide};
    let g = golden("presets");
    let dicts = g["translations"].as_object().expect("translations");
    let ids = |l: Vec<&SizePreset>| ok(l.iter().map(|p| p.id).collect::<Vec<_>>());
    let usize_arg = |a: &[Value], i: usize, default: usize| a.get(i).map_or(default, |v| f(v) as usize);
    let n = replay("presets", &g["cases"], |name, a| match name {
        "searchPresets" => match a.get(1) {
            None => ids(presets::search_presets(s(&a[0]))),
            Some(tr) => {
                let dict = dicts.get(s(tr));
                ids(presets::search_presets_with(s(&a[0]), |k| dict.and_then(|d| d.get(k)).and_then(Value::as_str).unwrap_or(k).to_string()))
            }
        },
        "findPreset" => opt_out(a[0].as_str().and_then(presets::find_preset), undef()),
        "presetsInCategory" => ids(presets::presets_in_category(&PresetCategoryId::parse(s(&a[0])))),
        "presetPlatforms" => {
            let list: Vec<&SizePreset> = a[0].as_array().unwrap().iter().map(|id| presets::find_preset(s(id)).expect("known id")).collect();
            ok(presets::preset_platforms(&list))
        }
        "popularPresets" => ids(presets::popular_presets()),
        "tryNewPresets" => {
            let recent: Vec<&str> = a[0].as_array().unwrap().iter().map(s).collect();
            ids(presets::try_new_presets(&recent, usize_arg(a, 1, 4)))
        }
        "pushRecentPreset" => ok(presets::push_recent_preset(&a[0], s(&a[1]), usize_arg(a, 2, 8))),
        "presetDesignInit" => {
            let preset = a[0].as_str().map(|id| presets::find_preset(id).expect("known id"));
            let o: PresetDesignOverrides = a.get(1).map(de).unwrap_or_default();
            ok(presets::preset_design_init(preset, &o))
        }
        "lockedSide" => {
            let d = a.get(3).map_or(2, |v| f(v) as i32);
            num(presets::locked_side(&SizeSide::parse(s(&a[0])), f(&a[1]), f(&a[2]), d)).into()
        }
        _ => panic!("unknown fn {name}"),
    });
    report("presets", n);
}

// ------------------------------------------------------------------ migrate

#[test]
fn migrate_goldens() {
    use oc_schema::migrate;
    let n = replay("migrate", &golden("migrate"), |name, a| {
        let doc: oc_schema::DesignDoc = de(&a[0]);
        match name {
            "migrate" => match a.get(1).and_then(Value::as_f64) {
                Some(t) => res(migrate::migrate_to(doc, t as i64)),
                None => res(migrate::migrate(doc)),
            },
            "needsMigration" => ok(migrate::needs_migration(&doc)),
            _ => panic!("unknown fn {name}"),
        }
    });
    report("migrate", n);
}

// ------------------------------------------------------------------ frames

#[test]
fn frames_goldens() {
    let n = replay("frames", &golden("frames"), |name, a| match name {
        "frameLibraryItem" => opt_out(oc_schema::frames::frame_library_item(s(&a[0])), undef()),
        _ => panic!("unknown fn {name}"),
    });
    report("frames", n);
}

// ------------------------------------------------------------------ embeds

#[test]
fn embeds_goldens() {
    let n = replay("embeds", &golden("embeds"), |name, a| match name {
        "parseEmbedUrl" => {
            let r = oc_schema::embeds::parse_embed_url(s(&a[0]));
            if s(&a[0]).contains("bad%E0%A4%A") {
                // Known difference: TS throws `URIError` from decodeURIComponent; Rust rejects the URL.
                assert!(r.is_none());
                return Out::Skip;
            }
            opt_out(r, Value::Null)
        }
        _ => panic!("unknown fn {name}"),
    });
    report("embeds", n);
}

// ------------------------------------------------------------------ svg

#[test]
fn svg_goldens() {
    use oc_schema::svg;
    let n = replay("svg", &golden("svg"), |name, a| {
        let map = |v: &Value| -> IndexMap<String, String> { opt(v).unwrap_or_default() };
        match name {
            "sanitizeSvg" => ok(svg::sanitize_svg(s(&a[0]))),
            "extractSvgColors" => ok(svg::extract_svg_colors(s(&a[0]))),
            "recolorSvg" => ok(svg::recolor_svg(s(&a[0]), &map(&a[1]))),
            "cleanSvgColorMap" => ok(svg::clean_svg_color_map(&map(&a[0]))),
            "recolorKey" => {
                let m: Option<IndexMap<String, String>> = opt(&a[1]);
                ok(svg::recolor_key(s(&a[0]), m.as_ref()))
            }
            "normalizeSvgColor" => opt_out(svg::normalize_svg_color(s(&a[0])), Value::Null),
            "parseRecolorKey" => opt_out(svg::parse_recolor_key(s(&a[0])), Value::Null),
            "svgColorKey" => ok(svg::is_svg_color_key(s(&a[0]))),
            "svgColorValue" => ok(svg::is_svg_color_value(s(&a[0]))),
            _ => panic!("unknown fn {name}"),
        }
    });
    report("svg", n);
}

// ------------------------------------------------------------------ factory

/// TS `BaseInit` JSON → `ElementBase` (`base_init(xf)` overlaid with the given fields).
fn base_of(init: &Value) -> oc_schema::ElementBase {
    let xf: Xf = de(&init["xf"]);
    let mut b = j(&oc_schema::factory::base_init(xf));
    for (k, v) in init.as_object().expect("init") {
        b[k] = v.clone();
    }
    de(&b)
}

fn elj<T: Into<Element>>(x: T) -> Out {
    Out::Val(norm_id(j(&x.into())))
}

fn color_opt(v: &Value) -> Option<ColorValue> {
    v.as_str().map(|h| hex(h).expect("hex"))
}

#[test]
fn factory_goldens() {
    use oc_schema::factory::*;
    let n = replay("factory", &golden("factory"), |name, a| {
        let o = a.get(1).cloned().unwrap_or(Value::Null);
        let el = |e: Value| Out::Val(norm_id(e));
        match name {
            "makeText" => elj(make_text(
                base_of(&a[0]),
                TextOpts { text: opt(&o["text"]), style: opt(&o["style"]), align: opt(&o["align"]), body: None },
            )),
            "makeShape" => elj(make_shape(
                base_of(&a[0]),
                ShapeOpts { kind: opt(&o["kind"]), fill: opt(&o["fill"]), stroke: opt(&o["stroke"]), params: opt(&o["params"]) },
            )),
            "makeLine" => elj(make_line(
                base_of(&a[0]),
                LineOpts {
                    color: color_opt(&o["color"]),
                    width: opt(&o["width"]),
                    dash: opt(&o["dash"]),
                    start_marker: opt(&o["startMarker"]),
                    end_marker: opt(&o["endMarker"]),
                },
            )),
            "makeImage" => elj(make_image(base_of(&a[0]), s(&a[1]))),
            "makeFrame" => elj(make_frame(base_of(&a[0]), FrameOpts { clip: opt(&o["clip"]), asset: opt(&o["asset"]), preset: opt(&o["preset"]) })),
            "makeGrid" => elj(make_grid(base_of(&a[0]), GridOpts { layout: opt(&o["layout"]), cells: opt(&o["cells"]), gap: opt(&o["gap"]) })),
            "makeGroup" => elj(make_group(base_of(&a[0]))),
            "makeQr" => elj(make_qr(base_of(&a[0]), s(&a[1]))),
            "makeDraw" => {
                let o = &a[2];
                elj(make_draw(base_of(&a[0]), de(&a[1]), DrawOpts { brush: opt(&o["brush"]), color: color_opt(&o["color"]), size: opt(&o["size"]) }))
            }
            "makeTable" => elj(make_table(base_of(&a[0]), TableOpts { rows: opt(&o["rows"]), cols: opt(&o["cols"]), data: opt(&o["data"]) })),
            "makeChart" => {
                let series = o["series"].as_array().map(|list| {
                    list.iter()
                        .map(|x| SeriesInit { name: s(&x["name"]).into(), values: de(&x["values"]), color: color_opt(&x["color"]) })
                        .collect()
                });
                elj(make_chart(base_of(&a[0]), ChartOpts { kind: opt(&o["kind"]), labels: opt(&o["labels"]), series }))
            }
            "makeVideo" => elj(make_video(base_of(&a[0]), s(&a[1]), f(&a[2]))),
            "makeForm" => elj(make_form(base_of(&a[0]), FormOpts { title: opt(&o["title"]), fields: opt(&o["fields"]) })),
            "makeEmbed" => elj(make_embed(
                base_of(&a[0]),
                EmbedOpts { url: s(&o["url"]).into(), provider: de(&o["provider"]), src: s(&o["src"]).into(), title: opt(&o["title"]), poster: opt(&o["poster"]) },
            )),
            "makeStroke" => ok(make_stroke(color_opt(&a[0]), opt(&a[1]), opt(&a[2]))),
            "gridCellBox" => ok(grid_cell_box(f(&a[0]["gap"]), &de(&a[0]["xf"]), &de(&a[1]))),
            "clipLengthMs" => num(clip_length_ms(f(&a[0]["trimStartMs"]), f(&a[0]["trimEndMs"]), f(&a[0]["speed"]))).into(),
            "formLayout" => match de::<Element>(&a[0]) {
                Element::Form(fe) => ok(form_layout_of(&fe)),
                _ => panic!("not a form"),
            },
            "makeMeta" | "emptyDesign" => {
                let m = &a[0];
                let init = MetaInit {
                    w: f(&m["w"]),
                    h: f(&m["h"]),
                    unit: opt(&m["unit"]),
                    title: opt(&m["title"]),
                    surface: opt(&m["surface"]),
                    pages: opt(&m["pages"]),
                };
                if name == "makeMeta" {
                    ok(make_meta(&init))
                } else {
                    let d = empty_design(&init);
                    let pages: Vec<Value> = oc_schema::tree::sorted_pages(&d.pages).into_iter().map(|p| norm_id(j(p))).collect();
                    Out::Val(json!({ "meta": j(&d.meta), "pages": pages, "elements": {}, "assets": {} }))
                }
            }
            "makePage" => el(j(&make_page())),
            "xf" => ok(xf(f(&a[0]), f(&a[1]), f(&a[2]), f(&a[3]), a.get(4).map_or(0.0, f))),
            "placeAll" => {
                let mut els: Vec<Element> = de(&a[1]);
                place_all_elements(s(&a[0]), &mut els, a[2].as_str());
                ok(els)
            }
            _ => panic!("unknown fn {name}"),
        }
    });
    report("factory", n);
}

// ------------------------------------------------------------------ tree

fn named_doc(docs: &Value, name: &str) -> oc_schema::DesignDoc {
    match docs.get(name) {
        Some(d) => de(d),
        None => de(&fixture(name)),
    }
}

#[test]
fn tree_goldens() {
    use oc_schema::tree;
    let g = golden("tree");
    let mut cache: IndexMap<String, oc_schema::DesignDoc> = IndexMap::new();
    let ids_of = |v: Vec<&Element>| ok(v.iter().map(|e| e.id().unwrap_or("").to_string()).collect::<Vec<_>>());
    let n = replay("tree", &g["cases"], |name, a| {
        let dn = s(&a[0]).to_string();
        let doc = cache.entry(dn.clone()).or_insert_with(|| named_doc(&g["docs"], &dn)).clone();
        match name {
            "sortedPages" => ok(tree::sorted_pages(&doc.pages).iter().map(|p| p.id.clone()).collect::<Vec<_>>()),
            "liveElements" => ids_of(tree::live_elements(&doc.elements)),
            "childrenOf" => ids_of(tree::children_of(&doc.elements, s(&a[1]))),
            "descendants" => ids_of(tree::descendants(&doc.elements, s(&a[1]))),
            "pageElements" => ids_of(tree::page_elements(&doc.elements, s(&a[1]))),
            "pageOf" => opt_out(tree::page_of(&doc.pages, &doc.elements, s(&a[1])), undef()),
            "ancestors" => ids_of(tree::ancestors(&doc.elements, s(&a[1]))),
            _ => panic!("unknown fn {name}"),
        }
    });
    report("tree", n);
}

// ------------------------------------------------------------------ timeline

#[test]
fn timeline_goldens() {
    use oc_schema::timeline::*;
    let g = golden("timeline");
    let doc: oc_schema::DesignDoc = de(&g["doc"]);
    let span = |sp: &PageSpan| json!({ "pageId": sp.page.id, "index": sp.index, "startMs": sp.start_ms, "durationMs": sp.duration_ms });
    let n = replay("timeline", &g["cases"], |name, a| {
        let el = || &doc.elements[s(&a[0])];
        match name {
            "pageDurationMs" => num(page_duration_ms(&doc.elements, &doc.pages[s(&a[0])])).into(),
            "timelineOf" => {
                let t = timeline_of(&doc, a[0].as_bool().unwrap());
                Out::Val(json!({ "spans": t.spans.iter().map(span).collect::<Vec<_>>(), "totalMs": t.total_ms }))
            }
            "locateTime" => match locate_time(&doc, f(&a[0])) {
                Some(r) => Out::Val(json!({ "span": span(&r.span), "pageTimeMs": r.page_time_ms })),
                None => Out::Val(Value::Null),
            },
            "elementWindow" => ok(element_window(el(), f(&a[1]))),
            "videoSlots" => ok(video_slots(el())),
            "mediaSourceTime" => {
                let slots = video_slots(el());
                let slot = slots.iter().find(|x| x.key == s(&a[1])).expect("slot");
                opt_out(media_source_time(el(), slot.clip, f(&a[2]), f(&a[3])), Value::Null)
            }
            "videoSourceTime" => match el() {
                Element::Video(v) => opt_out(video_source_time(v, f(&a[1]), f(&a[2])), Value::Null),
                _ => panic!("not a video"),
            },
            _ => panic!("unknown fn {name}"),
        }
    });
    report("timeline", n);
}

// ------------------------------------------------------------------ table

#[test]
fn table_goldens() {
    use oc_schema::elements::TableEl;
    use oc_schema::table::*;
    let g = golden("table");
    let tables: IndexMap<String, TableEl> = g["tables"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| match de::<Element>(v) {
            Element::Table(t) => (k.clone(), t),
            _ => panic!("not a table"),
        })
        .collect();
    let measure = |_k: &str, body: &TextBody, width: f64| -> f64 {
        let mut lines = 0.0;
        for p in &body.paragraphs {
            let n = js::utf16_len(&p.runs.iter().map(|r| r.text.as_str()).collect::<String>()) as f64;
            lines += 1f64.max(((n * body.default_run.size * 0.55) / 1f64.max(width)).ceil());
        }
        lines * body.default_run.size * 1.3
    };
    let n = replay("table", &g["cases"], |name, a| {
        let t = || &tables[s(&a[0])];
        let i = |k: usize| f(&a[k]) as i64;
        match name {
            "tableGeometry" => ok(geometry_of(t())),
            "tableAnchors" => ok(table_anchors(t())),
            "tableEdges" => ok(table_edges(t(), None)),
            "distributeColumns" => ok(distribute_columns(&t().cols)),
            "tableFitRows" => opt_out(table_fit_rows(t(), &mut { measure }, a[2].as_str()), Value::Null),
            "anchorOf" => ok(anchor_of(t(), i(1), i(2))),
            "isCovered" => ok(is_covered(t(), i(1), i(2))),
            "cellSpan" => ok(cell_span(t(), i(1), i(2))),
            "effectiveSpan" => ok(effective_span(t(), i(1), i(2))),
            "cellFill" => opt_out(cell_fill(t(), i(1), i(2)), undef()),
            "cellPlainText" => ok(cell_plain_text(t(), i(1), i(2))),
            "cellPadding" => num(cell_padding(t(), i(1), i(2))).into(),
            "cellRect" => ok(cell_rect(t(), i(1), i(2), None)),
            "cellContentBox" => ok(cell_content_box(t(), i(1), i(2), None)),
            "cellBody" => ok(cell_body(t(), i(1), i(2))),
            "migrateCell" => ok(migrate_cell(t(), i(1), i(2))),
            "tableDefaultRun" => {
                let cell: Option<oc_schema::elements::TableCell> = opt(&a[2]);
                ok(table_default_run(&t().style, i(1), cell.as_ref()))
            }
            "cellTextOffset" => num(cell_text_offset(t(), i(1), i(2), f(&a[3]), f(&a[4]))).into(),
            "cellSideBorder" => ok(cell_side_border(t(), i(1), i(2), &de(&a[3]))),
            "slotAt" => opt_out(slot_at(t(), f(&a[1]), f(&a[2]), None), Value::Null),
            "normalizeRange" => ok(normalize_range(t(), de(&a[1]), de(&a[2]))),
            "rangeAnchors" => ok(range_anchors(t(), &de(&a[1]))),
            "mergeCells" => ok(merge_cells(t(), &de(&a[1]))),
            "unmergeCells" => ok(unmerge_cells(t(), &de(&a[1]))),
            "canUnmerge" => ok(can_unmerge(t(), &de(&a[1]))),
            "applyBorderPreset" => ok(apply_border_preset(t(), &de(&a[1]), &de(&a[2]), &de(&a[3]))),
            "tableStructureEdit" => opt_out(table_structure_edit(t(), &de(&a[1]), i(2)), Value::Null),
            "applyTableStyle" => opt_out(apply_table_style(t(), &de(&a[1])), undef()),
            "resizeColumn" => ok(resize_column(t(), i(1), f(&a[2]), a[3].as_f64())),
            "defaultBorder" => ok(default_border(&t().style)),
            "tableStyleSwatch" => opt_out(table_style_swatch(&de(&a[0])), undef()),
            "withBody" => {
                let cell: Option<oc_schema::elements::TableCell> = opt(&a[0]);
                ok(with_body(cell.as_ref(), de(&a[1])))
            }
            "cellKey" => ok(cell_key(i(0), i(1))),
            _ => panic!("unknown fn {name}"),
        }
    });
    report("table", n);
}

// ------------------------------------------------------------------ doc blocks

#[test]
fn doc_blocks_goldens() {
    use oc_schema::doc_blocks::*;
    let arg = |v: &Value| -> Option<Value> { if v.as_str() == Some(UNDEF) { None } else { Some(v.clone()) } };
    let n = replay("doc_blocks", &golden("doc_blocks"), |name, a| match name {
        "defaultDocChart" => ok(default_doc_chart(opt(&a[0]))),
        "parseDocChart" => ok(parse_doc_chart(arg(&a[0]).as_ref())),
        "serializeDocChart" => {
            let text = serialize_doc_chart(&parse_doc_chart(arg(&a[0]).as_ref()));
            Out::Val(serde_json::from_str(&text).expect("json"))
        }
        "docChartElement" => Out::Val(norm_id(j(&Element::from(doc_chart_element(&default_doc_chart(opt(&a[0])), de(&a[1])))))),
        "docBlockWidth" => num(doc_block_width(arg(&a[0]).as_ref(), a[1].as_f64())).into(),
        "calloutTone" => ok(callout_tone(arg(&a[0]).as_ref())),
        _ => panic!("unknown fn {name}"),
    });
    report("doc_blocks", n);
}

// ------------------------------------------------------------------ print

#[test]
fn print_goldens() {
    use oc_schema::print::*;
    let g = golden("print");
    let mut docs: IndexMap<String, oc_schema::DesignDoc> = IndexMap::new();
    let n = replay("print", &g["cases"], |name, a| {
        let mut doc = |v: &Value| {
            let n = s(v).to_string();
            docs.entry(n.clone()).or_insert_with(|| named_doc(&g["docs"], &n)).clone()
        };
        match name {
            "cmykGamutRisk" => ok(cmyk_gamut_risk(&de(&a[0]))),
            "elementBounds" => ok(element_bounds(&de(&a[0]))),
            "bleedPxFor" => num(bleed_px_for(f(&a[0]))).into(),
            "standalonePrintTarget" => ok(standalone_print_target(&doc(&a[0]))),
            "preflightPrint" => {
                let d = doc(&a[0]);
                let fonts: Option<Vec<String>> = opt(&a[2]);
                let has = fonts.map(|list| move |f: &str| list.iter().any(|x| x == f));
                let r = match &has {
                    Some(h) => preflight_print(&d, &de(&a[1]), Some(h as &dyn Fn(&str) -> bool)),
                    None => preflight_print(&d, &de(&a[1]), None),
                };
                ok(r)
            }
            _ => panic!("unknown fn {name}"),
        }
    });
    report("print", n);
}

// ------------------------------------------------------------------ validation

#[test]
fn validation_goldens() {
    let n = replay("validation", &golden("validation"), |name, a| match name {
        "validateDoc" => {
            let r = oc_schema::validate_doc(&a[1]);
            if s(&a[0]) == "pagesString" {
                // The TS throws (`in` on a string); Rust reports issues instead.
                assert!(r.is_err());
                return Out::Skip;
            }
            match r {
                Ok(()) => Out::Val(json!({ "ok": true })),
                Err(issues) => Out::Val(json!({ "ok": false, "issues": issues })),
            }
        }
        _ => panic!("unknown fn {name}"),
    });
    report("validation", n);
}
