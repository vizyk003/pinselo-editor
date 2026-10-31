// SPDX-License-Identifier: Apache-2.0
//! site.ts parity: `buildSite` with the renderer's answers replayed (`tests/golden/render-svc/site/*.json`); the
//! inner content of every `<svg>` is masked on both sides (drawing is the painter's job), everything else —
//! head, fonts, CSS, sections, overlay links/videos/forms/embeds, text mirror, phone layout, script — must match.

use std::sync::{Arc, LazyLock};

use oc_render_svc::renderer::{FaceRequest, PickedFace};
use oc_render_svc::site::{SiteOptions, build_site, describe, scope_svg_ids};
use regex::Regex;
use serde_json::Value;

use crate::common::*;
use crate::replay::{Replay, TestHost};

static SVG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(<svg\b[^>]*>)[\s\S]*?</svg>").unwrap());

fn mask(html: &str) -> String {
    SVG.replace_all(html, "${1}</svg>").into_owned()
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn sites_match_ts() {
    let mut n = 0;
    for (file, g) in golden_dir("site") {
        let name = g["name"].as_str().unwrap();
        let doc = design(name);
        let mut r = Replay::default();
        for p in g["pages"].as_array().unwrap() {
            let id = p["page"].as_str().unwrap().to_string();
            r.heads
                .insert(id.clone(), p["head"].as_str().unwrap().to_string());
            r.faces.insert(
                id,
                serde_json::from_value::<Vec<FaceRequest>>(p["faces"].clone()).unwrap(),
            );
        }
        for (k, v) in g["picks"].as_object().unwrap() {
            r.picks.insert(
                k.clone(),
                serde_json::from_value::<PickedFace>(v.clone()).unwrap(),
            );
        }
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
        let html = block_on(build_site(&r, &TestHost::new(), &doc, &opts)).unwrap();
        if let Some(d) = str_diff(&mask(&html), g["html"].as_str().unwrap()) {
            panic!("{file}: {d}");
        }
        n += 1;
    }
    assert!(n >= 15);
}

#[test]
fn site_units_match_ts() {
    let g = golden("site-units.json");
    for c in g["scope"].as_array().unwrap() {
        assert_eq!(
            scope_svg_ids(c["svg"].as_str().unwrap(), c["prefix"].as_str().unwrap()),
            c["out"].as_str().unwrap()
        );
    }
    for c in g["describe"].as_array().unwrap() {
        let texts: Vec<String> = c["texts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            describe(&texts).as_deref(),
            c["out"].as_str(),
            "describe {texts:?}"
        );
    }
}
