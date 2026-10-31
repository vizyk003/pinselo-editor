// SPDX-License-Identifier: Apache-2.0
//! siteMobile.ts parity (`tests/golden/render-svc/mobile.json`): the phone layout of every website page with the
//! generator's deterministic ctx doubles, the text mirror, text/paint helpers, and site.ts's semantic tags.

use std::collections::HashMap;

use futures::future::BoxFuture;
use oc_render_svc::renderer::RenderError;
use oc_render_svc::site::text_tags;
use oc_render_svc::site_mobile::{
    Fit, MobileCtx, TextTag, max_run_size, mirror_html, mobile_page, page_items, paint_css,
    text_html, text_kind,
};
use oc_schema::DesignDoc;
use oc_schema::elements::{Element, TextEl};
use oc_schema::js::num_to_string as n;
use oc_schema::primitives::{Paint, Rect};
use oc_schema::tree::sorted_pages;
use serde_json::Value;

use crate::common::*;

struct Ctx<'a> {
    doc: &'a DesignDoc,
    pages: Vec<String>,
    tags: &'a HashMap<String, TextTag>,
}

impl MobileCtx for Ctx<'_> {
    fn doc(&self) -> &DesignDoc {
        self.doc
    }
    fn page_w(&self) -> f64 {
        self.doc.meta.dims.w
    }
    fn page_h(&self) -> f64 {
        self.doc.meta.dims.h
    }
    fn design_id(&self) -> &str {
        "site-design"
    }
    fn asset_url(&self, id: &str) -> String {
        format!("https://assets.example/a/{id}")
    }
    fn family(&self, family: &str, weight: f64, italic: bool) -> String {
        format!("{family}~{}{}", n(weight), if italic { "i" } else { "" })
    }
    fn svg_of<'b>(
        &'b self,
        ids: Vec<String>,
        bx: Rect,
        fit: Fit,
    ) -> BoxFuture<'b, Result<String, RenderError>> {
        let s = format!(
            "<svg data-ids=\"{}\" data-box=\"{} {} {} {}\" data-fit=\"{}\"></svg>",
            ids.join(","),
            n(bx.x),
            n(bx.y),
            n(bx.w),
            n(bx.h),
            fit.as_str()
        );
        Box::pin(async move { Ok(s) })
    }
    fn page_href(&self, page_id: &str) -> String {
        format!(
            "#p{}",
            self.pages
                .iter()
                .position(|p| p == page_id)
                .map_or(0, |i| i + 1)
        )
    }
    fn tag_of(&self, el: &TextEl) -> TextTag {
        self.tags.get(&el.base.id).copied().unwrap_or(TextTag::P)
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn phone_layouts_match_ts() {
    let g = golden("mobile.json");
    let mut pages_checked = 0;
    for d in g["docs"].as_array().unwrap() {
        let name = d["name"].as_str().unwrap();
        let doc = design(name);
        let (tags, _) = text_tags(&doc);
        let want_tags: HashMap<String, String> = d["tags"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect();
        let got_tags: HashMap<String, String> = tags
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().to_string()))
            .collect();
        assert_eq!(got_tags, want_tags, "{name}: semantic tags");
        let pages: Vec<String> = sorted_pages(&doc.pages)
            .into_iter()
            .filter(|p| !p.hidden.unwrap_or(false))
            .map(|p| p.id.to_string())
            .collect();
        let ctx = Ctx {
            doc: &doc,
            pages,
            tags: &tags,
        };
        for p in d["pages"].as_array().unwrap() {
            let pid = p["page"].as_str().unwrap();
            let label = format!("{name}/{pid}");
            let items = page_items(&doc, pid);
            let m = block_on(mobile_page(&items, &ctx, doc.pages[pid].background.first())).unwrap();
            if let Some(e) = str_diff(&m.html, p["html"].as_str().unwrap()) {
                panic!("{label}: mobile html {e}");
            }
            let order: Vec<&str> = m.order.iter().map(|it| it.id.as_str()).collect();
            let want: Vec<&str> = p["order"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap())
                .collect();
            assert_eq!(order, want, "{label}: reading order");
            if let Some(e) = str_diff(&mirror_html(&m.order, &ctx), p["mirror"].as_str().unwrap()) {
                panic!("{label}: mirror {e}");
            }
            let texts: Vec<&Value> = p["texts"].as_array().unwrap().iter().collect();
            let ours: Vec<&TextEl> = items
                .iter()
                .filter_map(|it| {
                    if let Element::Text(t) = it.el {
                        Some(t)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(ours.len(), texts.len(), "{label}: text count");
            for (t, w) in ours.iter().zip(texts) {
                assert_eq!(t.base.id, w["id"].as_str().unwrap());
                assert_eq!(
                    text_kind(t, doc.meta.dims.w),
                    w["kind"].as_str().unwrap(),
                    "{label} {}: kind",
                    t.base.id
                );
                assert!((max_run_size(t) - w["maxRunSize"].as_f64().unwrap()).abs() < 1e-9);
                if let Some(e) = str_diff(&text_html(t, &ctx, false), w["html"].as_str().unwrap()) {
                    panic!("{label} {}: textHtml {e}", t.base.id);
                }
                if let Some(e) =
                    str_diff(&text_html(t, &ctx, true), w["htmlInLink"].as_str().unwrap())
                {
                    panic!("{label} {}: textHtml(inLink) {e}", t.base.id);
                }
            }
            for c in p["paints"].as_array().unwrap() {
                let paint: Paint = serde_json::from_value(c["paint"].clone()).unwrap();
                let url = |id: &str| ctx.asset_url(id);
                assert_eq!(
                    paint_css(Some(&paint), &url).as_deref(),
                    c["css"].as_str(),
                    "{label}: paintCss {}",
                    c["paint"]
                );
            }
            pages_checked += 1;
        }
    }
    assert!(pages_checked >= 30, "{pages_checked} pages");
}
