// SPDX-License-Identifier: Apache-2.0
//! Bring template pages into an existing design: fresh ids, scaled to fit the design's page size.
//! Port of `templates.ts`; ids come from the injected random source.

use std::collections::HashMap;

use oc_crdt::rng::{RandomSource, new_id};
use oc_schema::primitives::Xf;
use oc_schema::tree::{descendants, sorted_pages};
use oc_schema::DesignDoc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::transform::scale_patches;
use crate::util::{jm, jv, min};

/// A template page with its elements, re-keyed for insertion (plain JSON, like the TS objects).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportedPage {
    pub page: Map<String, Value>,
    pub elements: Vec<Map<String, Value>>,
}

pub fn template_pages(tpl: &DesignDoc, target_w: f64, target_h: f64, rng: &mut dyn RandomSource) -> Vec<ImportedPage> {
    let (tw, th) = (tpl.meta.dims.w, tpl.meta.dims.h);
    let s = min(target_w / tw, target_h / th);
    let dx = (target_w - tw * s) / 2.0;
    let dy = (target_h - th * s) / 2.0;
    let mut out = Vec::new();
    for p in sorted_pages(&tpl.pages) {
        let page_id = new_id(rng);
        let els = descendants(&tpl.elements, &p.id);
        let mut remap: HashMap<String, String> = HashMap::new();
        remap.insert(p.id.clone(), page_id.clone());
        for e in &els {
            remap.insert(e.id().unwrap_or("").to_string(), new_id(rng));
        }
        let mut elements = Vec::new();
        for e in &els {
            let mut el = jm(*e);
            let id = e.id().unwrap_or("");
            el.insert("id".into(), Value::String(remap.get(id).cloned().unwrap_or_default()));
            let parent = e.parent_id().unwrap_or("");
            match remap.get(parent) {
                Some(p) => el.insert("parentId".into(), Value::String(p.clone())),
                None => el.remove("parentId"),
            };
            if s != 1.0 {
                let xf = e.xf().cloned().unwrap_or_default();
                let next = if parent == p.id {
                    // Top-level elements: scale about the page origin, then centre.
                    Xf { x: xf.x * s + dx, y: xf.y * s + dy, w: xf.w * s, h: xf.h * s, ..xf }
                } else {
                    Xf { x: xf.x * s, y: xf.y * s, w: xf.w * s, h: xf.h * s, ..xf }
                };
                el.insert("xf".into(), jv(&next));
                if let Some(patch) = scale_patches(&tpl.elements, e, s, s).get(id) {
                    for (k, v) in patch {
                        el.insert(k.clone(), v.clone());
                    }
                }
            }
            elements.push(el);
        }
        let mut page = jm(p);
        page.insert("id".into(), Value::String(page_id));
        out.push(ImportedPage { page, elements });
    }
    out
}
