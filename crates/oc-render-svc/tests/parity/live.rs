// SPDX-License-Identifier: Apache-2.0
//! The writers over the real painter (crates/oc-render, bundled fonts, test assets): where the TS output is
//! structural it must match the goldens exactly — the website HTML (inner SVG content masked, as in the replay
//! test) and the email HTML (its slices are numbered, so pixels don't enter it).

use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use futures::future::BoxFuture;
use oc_gfonts::manifest::{FontFamily, FontManifest};
use oc_render_svc::email::{EmailDeps, export_email};
use oc_render_svc::oc_renderer::OcRenderer;
use oc_render_svc::renderer::{RenderError, RenderFuture, RenderHost};
use oc_render_svc::site::{SiteOptions, build_site};
use regex::Regex;
use serde_json::Value;

use crate::common::*;

/// Bundled fonts only (no Google catalogue, no network, no database), the generator's assets.
struct LiveHost {
    manifest: FontManifest,
    dir: PathBuf,
}

impl LiveHost {
    fn new() -> LiveHost {
        LiveHost {
            manifest: bundled_manifest(),
            dir: bundled_fonts_dir(),
        }
    }
}

fn asset_for(key: &str) -> Option<&'static str> {
    if key.ends_with("/poster") {
        return Some("poster.png");
    }
    if key.contains("/recolor/") {
        return Some("tile.png");
    }
    match key.split('/').next().unwrap_or("") {
        "photo" => Some("photo.jpg"),
        "tile" => Some("tile.png"),
        "anim" => Some("anim.gif"),
        "pic" => Some("pic.webp"),
        "icon" => Some("icon.svg"),
        "clip" => Some("poster.png"),
        b if b.starts_with("nope") || b.starts_with("missing") => None,
        _ => Some("photo.jpg"),
    }
}

impl RenderHost for LiveHost {
    fn asset_bytes<'a>(&'a self, key: &'a str) -> RenderFuture<'a, Vec<u8>> {
        let r = asset_for(key)
            .map(asset)
            .ok_or_else(|| RenderError::Failed(format!("asset {key} not found")));
        Box::pin(async move { r })
    }
    fn font_file<'a>(&'a self, file: &'a str) -> RenderFuture<'a, Vec<u8>> {
        let r = bundled_font_file(file).map_err(RenderError::Failed);
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

static SVG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(<svg\b[^>]*>)[\s\S]*?</svg>").unwrap());
/// The `@font-face` lines: their face list comes from oc-text's `fallback_families`, which differs from
/// @oc/text for a few symbols (V8 counts ★ U+2605 as `Extended_Pictographic`, ICU4X's data does not, so the TS
/// also declares Noto Color Emoji). The replay test pins the `@font-face` logic itself; this test pins the rest.
static FACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^@font-face\{.*\}\n").unwrap());

fn structural(html: &str) -> String {
    FACES
        .replace_all(&SVG.replace_all(html, "${1}</svg>"), "")
        .into_owned()
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn sites_match_ts_with_the_real_painter() {
    let host = LiveHost::new();
    let r = OcRenderer::new(&host);
    let mut n = 0;
    for (file, g) in golden_dir("site") {
        let doc = design(g["name"].as_str().unwrap());
        let o = &g["opts"];
        let prefix = g["assetUrl"].as_str().unwrap().to_string();
        let opts = SiteOptions {
            design_id: o["designId"].as_str().unwrap().into(),
            title: o["title"].as_str().unwrap().into(),
            description: o["description"].as_str().map(str::to_string),
            noindex: o["noindex"].as_bool().unwrap_or(false),
            asset_url: Arc::new(move |id: &str| format!("{prefix}{id}")),
            og_image: o["ogImage"].as_str().map(str::to_string),
            badge: o.get("badge").map(|b: &Value| {
                (
                    b["name"].as_str().unwrap().to_string(),
                    b["url"].as_str().unwrap().to_string(),
                )
            }),
        };
        let html =
            block_on(build_site(&r, &host, &doc, &opts)).unwrap_or_else(|e| panic!("{file}: {e}"));
        if let Some(d) = str_diff(&structural(&html), &structural(g["html"].as_str().unwrap())) {
            panic!("{file}: {d}");
        }
        n += 1;
    }
    assert!(n >= 15);
}

struct Numbered(Mutex<usize>);

impl EmailDeps for Numbered {
    fn host<'a>(&'a self, png: Vec<u8>) -> BoxFuture<'a, Result<String, RenderError>> {
        assert_eq!(&png[..4], b"\x89PNG");
        let k = {
            let mut n = self.0.lock().unwrap();
            *n += 1;
            *n
        };
        Box::pin(async move { Ok(format!("https://img.example/e/{k}.png")) })
    }
}

#[test]
fn emails_match_ts_with_the_real_painter() {
    let host = LiveHost::new();
    let r = OcRenderer::new(&host);
    for (file, g) in golden_dir("email") {
        let doc = design(g["name"].as_str().unwrap());
        let pages: Vec<String> = g["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        let out = block_on(export_email(
            &r,
            &host,
            &doc,
            &pages,
            &Numbered(Mutex::new(0)),
        ))
        .unwrap();
        if let Some(d) = str_diff(&out.html, g["html"].as_str().unwrap()) {
            panic!("{file}: {d}");
        }
        assert_eq!(out.text, g["text"].as_str().unwrap(), "{file}");
    }
}

struct LayoutRefs(serde_json::Map<String, Value>);

fn asset_ref(id: &str, w: f64, h: f64, name: &str) -> oc_schema::design::AssetRef {
    serde_json::from_value(serde_json::json!({ "id": id, "kind": "image", "sha256": "0".repeat(64), "mime": "image/png", "w": w, "h": h, "name": name })).unwrap()
}

impl oc_render_svc::doc_layout::DocLayoutDeps for LayoutRefs {
    fn asset<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<oc_schema::design::AssetRef>> {
        let r = self
            .0
            .get(id)
            .filter(|v| v.is_object())
            .map(|v| asset_ref(id, v["w"].as_f64().unwrap(), v["h"].as_f64().unwrap(), id));
        Box::pin(async move { r })
    }
    fn embed<'a>(
        &'a self,
        design_id: &'a str,
        _page: Option<&'a str>,
        width_px: f64,
    ) -> BoxFuture<'a, Option<oc_schema::design::AssetRef>> {
        let r = design_id.starts_with("allowed").then(|| {
            asset_ref(
                &format!("embed-{design_id}"),
                width_px,
                oc_schema::js::round(width_px * 0.6),
                "Embedded design.png",
            )
        });
        Box::pin(async move { r })
    }
}

/// Text whose characters need a Noto fallback the generator (offline) could not load: CanvasKit measured those
/// glyphs with whatever face it had, oc-text with the notdef metrics.
fn needs_missing_fallback(text: &str) -> bool {
    text.chars()
        .any(|c| !oc_text::fallback_families(&c.to_string()).is_empty())
}

/// Exact comparison, except that a line needing a missing fallback face may differ in height by ≤ 2 px; later
/// elements on the page then shift by the accumulated difference.
fn compare_layout(name: &str, ours: &[Value], theirs: &[Value]) {
    assert_eq!(ours.len(), theirs.len(), "{name}: page count");
    for (pi, (a, b)) in ours.iter().zip(theirs).enumerate() {
        let (a, b) = (a.as_array().unwrap(), b.as_array().unwrap());
        assert_eq!(a.len(), b.len(), "{name} page {pi}: element count");
        let mut drift = 0.0;
        for (ei, (x, y)) in a.iter().zip(b).enumerate() {
            let at = format!("{name} page {pi} element {ei}");
            for k in ["type", "text", "asset", "altText"] {
                assert_eq!(x.get(k), y.get(k), "{at}: {k}");
            }
            let f = |v: &Value, k: &str| v["xf"][k].as_f64().unwrap();
            for k in ["x", "w"] {
                assert!(
                    (f(x, k) - f(y, k)).abs() < 1e-6,
                    "{at}: {k} {} != {}",
                    f(x, k),
                    f(y, k)
                );
            }
            assert!(
                (f(x, "y") + drift - f(y, "y")).abs() < 1e-6,
                "{at}: y {} (+{drift}) != {}",
                f(x, "y"),
                f(y, "y")
            );
            let dh = f(y, "h") - f(x, "h");
            if x.get("text")
                .and_then(Value::as_str)
                .is_some_and(needs_missing_fallback)
            {
                assert!(dh.abs() <= 2.0, "{at}: h {} != {}", f(x, "h"), f(y, "h"));
                drift += dh;
            } else {
                assert!(dh.abs() < 1e-6, "{at}: h {} != {}", f(x, "h"), f(y, "h"));
            }
        }
    }
}

/// docLayout.ts pagination with oc-text measuring: the same pages, elements, texts and boxes as with SkParagraph.
#[test]
fn doc_layouts_match_ts_with_the_real_painter() {
    let host = LiveHost::new();
    let r = OcRenderer::new(&host);
    let g = golden("doc-layout.json");
    let docs = golden("docs.json");
    let refs = LayoutRefs(g["images"].as_object().unwrap().clone());
    for c in g["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let case = docs["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == *name)
            .unwrap();
        let pm: oc_crdt::doc_body::PmNode = serde_json::from_value(case["pm"].clone()).unwrap();
        let res = block_on(oc_render_svc::doc_layout::doc_to_design(
            &r,
            &host,
            &pm,
            case["title"].as_str().unwrap(),
            816.0,
            1056.0,
            &refs,
        ));
        if let Some(e) = c.get("error") {
            assert_eq!(
                res.err().map(|e| e.to_string()).as_deref(),
                e.as_str(),
                "{name}: error"
            );
            continue;
        }
        let d = res.unwrap_or_else(|e| panic!("{name}: {e}"));
        let pages: Vec<Value> = oc_schema::tree::sorted_pages(&d.pages)
            .iter()
            .map(|p| {
                Value::Array(
                    oc_schema::tree::children_of(&d.elements, &p.id)
                        .iter()
                        .map(|e| {
                            let mut v = serde_json::json!({ "type": e.type_name(), "xf": { "x": e.xf().unwrap().x, "y": e.xf().unwrap().y, "w": e.xf().unwrap().w, "h": e.xf().unwrap().h } });
                            match e {
                                oc_schema::Element::Text(t) => v["text"] = Value::String(t.body.paragraphs.iter().map(|q| q.runs.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")),
                                oc_schema::Element::Image(i) => {
                                    v["asset"] = Value::String(i.asset.to_string());
                                    v["altText"] = i.base.alt_text.clone().map_or(Value::Null, Value::String);
                                }
                                _ => {}
                            }
                            v
                        })
                        .collect(),
                )
            })
            .collect();
        compare_layout(name, &pages, c["pages"].as_array().unwrap());
        let assets: Vec<String> = d.assets.keys().map(|k| k.to_string()).collect();
        let want: Vec<String> = c["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect();
        assert_eq!(assets, want, "{name}: assets");
    }
}

/// oc-render's SVG path parser against CanvasKit's (`Path.MakeFromSVGString(d).toCmds()`) for every path the
/// PowerPoint goldens parsed: identical commands, except that CanvasKit keeps arcs as conics where oc-render
/// emits cubics (same curve); for those only the move/line/close skeleton and end points are compared.
#[test]
fn svg_path_commands_match_canvaskit() {
    use oc_render_svc::renderer::{PathCmd, Renderer};
    let host = LiveHost::new();
    let r = OcRenderer::new(&host);
    let (mut exact, mut arcs) = (0, 0);
    for (file, g) in golden_dir("pptx") {
        for (d, c) in g["paths"].as_object().unwrap() {
            let ours = r.path_commands(d).unwrap();
            let Some(c) = c.as_array() else {
                assert!(ours.is_none(), "{file}: {d:?} is unparsable in CanvasKit");
                continue;
            };
            let theirs =
                PathCmd::from_flat(&c.iter().map(|x| x.as_f64().unwrap()).collect::<Vec<_>>());
            let ours = ours.unwrap_or_else(|| panic!("{file}: {d:?} parses in CanvasKit"));
            if theirs.iter().any(|c| matches!(c, PathCmd::Conic(..))) {
                let end = |v: &[PathCmd]| {
                    v.iter().rev().find_map(|c| match *c {
                        PathCmd::Move(x, y)
                        | PathCmd::Line(x, y)
                        | PathCmd::Quad(_, _, x, y)
                        | PathCmd::Conic(_, _, x, y, _)
                        | PathCmd::Cubic(_, _, _, _, x, y) => Some((x, y)),
                        PathCmd::Close => None,
                    })
                };
                let (a, b) = (end(&ours).unwrap(), end(&theirs).unwrap());
                assert!(
                    (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3,
                    "{file}: {d:?} ends at {a:?} vs {b:?}"
                );
                arcs += 1;
                continue;
            }
            assert_eq!(ours.len(), theirs.len(), "{file}: {d:?}");
            for (x, y) in ours.iter().zip(&theirs) {
                let (a, b) = (
                    serde_json::to_value(x).unwrap(),
                    serde_json::to_value(y).unwrap(),
                );
                if let Some(e) = diff(&a, &b, d) {
                    panic!("{file}: {e}");
                }
            }
            exact += 1;
        }
    }
    assert!(exact > 50, "{exact} exact, {arcs} with arcs");
}

/// pptx.ts over the real painter: every XML part identical to the TS (ids renumbered as in the replay test),
/// the same media (element pictures drawn by oc-render).
#[test]
fn decks_match_ts_with_the_real_painter() {
    let host = LiveHost::new();
    let r = OcRenderer::new(&host);
    for (file, g) in golden_dir("pptx") {
        let doc = design(g["name"].as_str().unwrap());
        let assets = crate::office::Assets(
            g["assets"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().map(str::to_string)))
                .collect(),
        );
        let pages: Vec<String> = g["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        let pkg = block_on(oc_render_svc::pptx::pptx_package(
            &r,
            &host,
            &doc,
            &pages,
            &assets,
            chrono::Utc::now(),
        ))
        .unwrap_or_else(|e| panic!("{file}: {e}"));
        let mut ours = std::collections::BTreeMap::new();
        let mut media = 0;
        for (part, bytes) in &pkg.parts {
            if part.starts_with("ppt/media/") {
                assert_eq!(
                    &bytes[..2],
                    if part.ends_with(".jpeg") {
                        &[0xff, 0xd8][..]
                    } else if part.ends_with(".gif") {
                        &b"GI"[..]
                    } else {
                        &b"\x89P"[..]
                    },
                    "{file} {part}"
                );
                media += 1;
            } else {
                ours.insert(part.clone(), String::from_utf8(bytes.clone()).unwrap());
            }
        }
        let theirs: std::collections::BTreeMap<String, String> = g["parts"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect();
        let (a, _) = crate::office::canonical(&ours, pages.len());
        let (b, _) = crate::office::canonical(&theirs, pages.len());
        assert_eq!(
            a.keys().collect::<Vec<_>>(),
            b.keys().collect::<Vec<_>>(),
            "{file}: parts"
        );
        for k in b.keys() {
            if let Some(d) = str_diff(&a[k], &b[k]) {
                panic!("{file} {k}: {d}");
            }
        }
        assert_eq!(media, g["media"].as_array().unwrap().len(), "{file}: media");
    }
}

/// `pdf-flat`: each page is one picture with the page's text laid over it invisibly (selectable and searchable,
/// like the TS's invisible Helvetica layer), plus the link annotations.
#[test]
fn flat_pdf_has_an_invisible_text_layer() {
    use oc_render_svc::renderer::{RasterPdfOptions, Renderer};
    let host = LiveHost::new();
    let r = OcRenderer::new(&host);
    let doc = design("fixture:vd-43-sales");
    let pages: Vec<String> = oc_schema::tree::sorted_pages(&doc.pages)
        .iter()
        .take(2)
        .map(|p| p.id.to_string())
        .collect();
    let pdf = block_on(r.raster_pdf(&host, &doc, &pages, &RasterPdfOptions::default())).unwrap();
    assert_eq!(&pdf[..5], b"%PDF-");
    let have_pdftotext = std::process::Command::new("pdftotext")
        .arg("-v")
        .output()
        .is_ok();
    if !have_pdftotext {
        eprintln!("pdftotext not installed: only the PDF header was checked");
        return;
    }
    let dir = std::env::temp_dir().join(format!("oc-render-svc-flat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("flat.pdf");
    std::fs::write(&file, &pdf).unwrap();
    let out = std::process::Command::new("pdftotext")
        .arg(&file)
        .arg("-")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    // Every text element of those pages is in the extracted text.
    let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let extracted = squash(&text);
    let mut n = 0;
    for p in &pages {
        for el in oc_schema::tree::descendants(&doc.elements, p) {
            if let oc_schema::elements::Element::Text(t) = el {
                for para in &t.body.paragraphs {
                    let s: String = para.runs.iter().map(|r| r.text.as_str()).collect();
                    let s = squash(&s);
                    if s.is_empty() {
                        continue;
                    }
                    let first = s.split(' ').next().unwrap();
                    assert!(
                        extracted.contains(first),
                        "{first:?} not in the text layer:\n{text}"
                    );
                    n += 1;
                }
            }
        }
    }
    assert!(n > 3, "{n} paragraphs checked");
}
