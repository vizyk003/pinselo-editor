// SPDX-License-Identifier: Apache-2.0
//! PS-1 pixel and layout baselines (AD-18, plan 13 stage 4.3): the renderer against stored fingerprints of its own
//! output, now that oc-render is the only engine (the CanvasKit comparison it replaced is `PARITY.md`).
//!
//! Corpus: every page of the hand-made scenes (`tests/golden/render/scenes.json`, with the committed test assets)
//! and of the seed templates (`fixtures/templates-seed`, drawn without their library photos: the missing-image
//! placeholder is part of the picture), at `min(1, 480 / long side)`.
//!
//! Per page `tests/golden/ps1/baselines.json` stores:
//! - `sha256` of the unpremultiplied RGBA8 pixels — equal means pixel-identical (the normal case on one platform);
//! - `cells`: the mean CIELAB (D50, from sRGB over white) of a 16 × 16 grid of cells, quantised (base64 of
//!   `L·2, a+128, b+128` as bytes) — when the hash differs, PS-1/A's tile rule is applied to these cells: the
//!   page fails if any cell's CIEDE2000 against the baseline exceeds 3.0;
//! - `layout`: sha256 of every text box's caret geometry (all UTF-16 offsets, positions quantised to 1/64 px) —
//!   PS-1/L: line breaks and glyph positions must be identical.
//!
//! The WebAssembly build is checked against the same file by `packages/engine/test/ps1.test.ts` (PS-1/A wasm vs
//! native: identical pixels; PS-1/L: identical layout). `OC_PS1_UPDATE=1 cargo test -p oc-render --test ps1`
//! rewrites the file after an intended rendering change; review the pages it reports first.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use base64::Engine as _;
use common::*;
use oc_render::displaylist::DLItem;
use oc_render::{AssetProvider, NoAssets, RenderOptions, ResolveOptions, resolve_page};
use oc_schema::DesignDoc;
use oc_schema::tree::sorted_pages;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const GRID: u32 = 16;
const TILE_DE00: f64 = 3.0;

struct Page {
    name: String,
    /// Where the document comes from: `scenes:<name>` or a fixture path.
    doc: String,
    page: String,
    scale: f64,
}

fn hex(b: &[u8]) -> String {
    b.iter().fold(String::with_capacity(b.len() * 2), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

/// sRGB (0..1, over white) → CIELAB D50 (Bradford-adapted sRGB matrix).
fn lab(r: f64, g: f64, b: f64) -> [f64; 3] {
    let lin = |c: f64| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let (r, g, b) = (lin(r), lin(g), lin(b));
    let x = (0.436_074_7 * r + 0.385_064_9 * g + 0.143_080_4 * b) / 0.964_22;
    let y = 0.222_504_5 * r + 0.716_878_6 * g + 0.060_616_9 * b;
    let z = (0.013_932_2 * r + 0.097_104_5 * g + 0.714_173_3 * b) / 0.825_21;
    let f = |t: f64| if t > 216.0 / 24389.0 { t.cbrt() } else { (24389.0 / 27.0 * t + 16.0) / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 (kL = kC = kH = 1).
fn de00(p: [f64; 3], q: [f64; 3]) -> f64 {
    let (l1, a1, b1) = (p[0], p[1], p[2]);
    let (l2, a2, b2) = (q[0], q[1], q[2]);
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let cm = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = (a1p.hypot(b1), a2p.hypot(b2));
    let hue = |b: f64, a: f64| {
        if b == 0.0 && a == 0.0 {
            0.0
        } else {
            let h = b.atan2(a).to_degrees();
            if h < 0.0 { h + 360.0 } else { h }
        }
    };
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();
    let lm = (l1 + l2) / 2.0;
    let cmp = (c1p + c2p) / 2.0;
    let hm = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hm - 30.0).to_radians().cos() + 0.24 * (2.0 * hm).to_radians().cos() + 0.32 * (3.0 * hm + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hm - 63.0).to_radians().cos();
    let dtheta = 30.0 * (-((hm - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cmp.powi(7) / (cmp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lm - 50.0).powi(2) / (20.0 + (lm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cmp;
    let sh = 1.0 + 0.015 * cmp * t;
    let rt = -(2.0 * dtheta.to_radians()).sin() * rc;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh)).sqrt()
}

/// Mean Lab of a GRID × GRID grid of cells (pixels composited over white), quantised to bytes.
fn cells(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut sum = vec![[0f64; 4]; (GRID * GRID) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let a = f64::from(rgba[i + 3]) / 255.0;
            let c = |v: u8| (f64::from(v) / 255.0) * a + (1.0 - a);
            let l = lab(c(rgba[i]), c(rgba[i + 1]), c(rgba[i + 2]));
            let cell = &mut sum[((y * GRID / h) * GRID + x * GRID / w) as usize];
            cell[0] += l[0];
            cell[1] += l[1];
            cell[2] += l[2];
            cell[3] += 1.0;
        }
    }
    let q = |v: f64, lo: f64, k: f64| ((v - lo) * k).round().clamp(0.0, 255.0) as u8;
    sum.iter()
        .flat_map(|c| {
            let n = c[3].max(1.0);
            [q(c[0] / n, 0.0, 2.0), q(c[1] / n, -128.0, 1.0), q(c[2] / n, -128.0, 1.0)]
        })
        .collect()
}

fn cell_lab(b: &[u8], i: usize) -> [f64; 3] {
    [f64::from(b[i * 3]) / 2.0, f64::from(b[i * 3 + 1]) - 128.0, f64::from(b[i * 3 + 2]) - 128.0]
}

/// sha256 over every text box's caret rects (each UTF-16 offset, 1/64 px), in display-list order.
fn layout_hash(r: &mut oc_render::Renderer, items: &[DLItem]) -> String {
    let mut s = String::new();
    for it in items {
        if let DLItem::Text { key, body, bbox, effect, .. } = it {
            let l = r.text.layout(key, body, bbox.w, effect.as_ref());
            let _ = write!(s, "{key}:{}:", l.length);
            for o in 0..=l.length {
                let c = l.caret_rect(o);
                let q = |v: f64| (v * 64.0).round() as i64;
                let _ = write!(s, "{},{},{},{};", q(c.x), q(c.y), q(c.w), q(c.h));
            }
            s.push('\n');
        }
    }
    hex(&Sha256::digest(s.as_bytes()))
}

fn corpus() -> (Vec<Page>, BTreeMap<String, DesignDoc>) {
    let mut pages = Vec::new();
    let mut docs = BTreeMap::new();
    let scale = |d: &DesignDoc| 1f64.min(480.0 / d.meta.dims.w.max(d.meta.dims.h));
    for sc in golden("scenes").as_array().expect("scenes") {
        let key = format!("scenes:{}", s(&sc["name"]));
        let doc: DesignDoc = de(&sc["doc"]);
        for (i, p) in sorted_pages(&doc.pages).iter().enumerate() {
            pages.push(Page { name: format!("scene-{}-p{}", s(&sc["name"]), i + 1), doc: key.clone(), page: p.id.to_string(), scale: scale(&doc) });
        }
        docs.insert(key, doc);
    }
    let dir = root().join("fixtures/templates-seed");
    let mut files: Vec<String> = std::fs::read_dir(&dir).expect("templates-seed").filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|f| f.ends_with(".json")).collect();
    files.sort();
    for f in files {
        let key = format!("fixtures/templates-seed/{f}");
        let doc: DesignDoc = de(&fixture(&f));
        for (i, p) in sorted_pages(&doc.pages).iter().enumerate() {
            pages.push(Page { name: format!("{}-p{}", f.trim_end_matches(".json"), i + 1), doc: key.clone(), page: p.id.to_string(), scale: scale(&doc) });
        }
        docs.insert(key, doc);
    }
    (pages, docs)
}

#[test]
fn ps1_baselines() {
    let (pages, docs) = corpus();
    let path = root().join("tests/golden/ps1/baselines.json");
    let update = std::env::var("OC_PS1_UPDATE").is_ok_and(|v| v == "1");
    let stored: Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let by_name: BTreeMap<&str, &Value> = stored["pages"].as_array().map(|a| a.iter().map(|p| (s(&p["name"]), p)).collect()).unwrap_or_default();
    assert!(update || !by_name.is_empty(), "no baselines in {} — run with OC_PS1_UPDATE=1", path.display());

    let scene_assets = scene_assets();
    let assets_for = |doc: &str| -> &dyn AssetProvider { if doc.starts_with("scenes:") { &scene_assets } else { &NoAssets } };
    let b64 = &base64::engine::general_purpose::STANDARD;
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let chunks: Vec<&[Page]> = pages.chunks(pages.len().div_ceil(workers)).collect();
    let results: Vec<(Value, Option<String>, bool)> = std::thread::scope(|sc| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                let (docs, by_name, assets_for) = (&docs, &by_name, &assets_for);
                sc.spawn(move || {
                    let mut r = renderer();
                    let mut out = Vec::new();
                    for p in *chunk {
                        let doc = &docs[&p.doc];
                        let dl = resolve_page(doc, &p.page, &ResolveOptions::default());
                        ensure_fonts(&mut r, &dl);
                        let pix = r.render(&dl, assets_for(&p.doc), &RenderOptions { scale: p.scale, background: None }).expect("render");
                        let (w, h) = (pix.width(), pix.height());
                        let rgba = oc_render::raster::to_rgba8(&pix);
                        let sha = hex(&Sha256::digest(&rgba));
                        let cl = cells(&rgba, w, h);
                        let layout = layout_hash(&mut r, &dl.items);
                        let entry = json!({ "name": p.name, "doc": p.doc, "page": p.page, "scale": p.scale, "w": w, "h": h, "sha256": sha, "layout": layout, "cells": b64.encode(&cl) });
                        let mut problem = None;
                        let mut exact = false;
                        match by_name.get(p.name.as_str()) {
                            None => problem = Some(format!("{}: no baseline", p.name)),
                            Some(b) => {
                                if (f(&b["w"]) as u32, f(&b["h"]) as u32) != (w, h) {
                                    problem = Some(format!("{}: size {w}×{h}, baseline {}×{}", p.name, b["w"], b["h"]));
                                } else if s(&b["layout"]) != layout {
                                    problem = Some(format!("{}: PS-1/L text layout differs", p.name));
                                } else if s(&b["sha256"]) == sha {
                                    exact = true;
                                } else {
                                    let want = b64.decode(s(&b["cells"])).expect("cells");
                                    let worst = (0..(GRID * GRID) as usize).map(|i| de00(cell_lab(&want, i), cell_lab(&cl, i))).fold(0.0, f64::max);
                                    if worst > TILE_DE00 {
                                        problem = Some(format!("{}: PS-1/A cell ΔE00 {worst:.2} > {TILE_DE00}", p.name));
                                    }
                                }
                            }
                        }
                        out.push((entry, problem, exact));
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("worker")).collect()
    });

    let exact = results.iter().filter(|r| r.2).count();
    let problems: Vec<&String> = results.iter().filter_map(|r| r.1.as_ref()).collect();
    eprintln!("PS-1: {} pages, {exact} pixel-identical, {} outside the thresholds", results.len(), problems.len());
    if update {
        std::fs::create_dir_all(path.parent().expect("dir")).expect("mkdir");
        let pages: Vec<Value> = results.into_iter().map(|r| r.0).collect();
        let file = json!({
            "about": "PS-1 baselines of oc-render (crates/oc-render/tests/ps1.rs); regenerate with OC_PS1_UPDATE=1",
            "grid": GRID,
            "pages": pages,
        });
        let mut text = serde_json::to_string_pretty(&file).expect("json");
        text.push('\n');
        std::fs::write(&path, text).expect("write baselines");
        eprintln!("PS-1: wrote {}", path.display());
        return;
    }
    assert!(problems.is_empty(), "PS-1 failures:\n{}", problems.iter().map(|p| format!("  {p}")).collect::<Vec<_>>().join("\n"));
}

#[test]
fn ciede2000_reference_pairs() {
    // Sharma, Wu, Dalal (2005), test data pairs 1, 7, 17 and 25.
    let cases = [
        ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
        ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
        ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
        ([60.2574, -34.0099, 36.2677], [60.4626, -34.1751, 39.4387], 1.2644),
    ];
    for (a, b, want) in cases {
        let got = de00(a, b);
        assert!((got - want).abs() < 1e-4, "ΔE00 {a:?} {b:?} = {got}, want {want}");
    }
}
