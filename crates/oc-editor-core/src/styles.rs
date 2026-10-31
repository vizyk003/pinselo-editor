// SPDX-License-Identifier: Apache-2.0
//! Design-wide styles (A17, also used by brand kits): list the colours a design uses, replace colours
//! everywhere, apply a palette (by luminance rank, so light/dark structure and contrast survive) and apply
//! a font pair (by role, else by size rank). Colour mapping is a deep walk over every element's JSON, so
//! new element types are covered automatically. Port of `styles.ts`.
//!
//! Element iteration follows the map order of the given document (id order for snapshots read from yrs, which
//! does not keep insertion order); `design_colors` ties (equal counts) can therefore be ordered differently than
//! in TS for documents whose JS insertion order differs from id order.

use indexmap::IndexMap;
use oc_crdt::DocOp;
use oc_schema::color::{hex, luminance, to_hex};
use oc_schema::elements::{Element, SemanticRole};
use oc_schema::primitives::ColorValue;
use oc_schema::text::{body_plain_text, resolve_run_style};
use oc_schema::tree::sorted_pages;
use oc_schema::{DesignDoc, TextBody};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::util::{jm, jv, max, round};

fn is_color(v: &Map<String, Value>) -> bool {
    v.get("space").is_some_and(|s| !s.is_null()) && (v.contains_key("r") || v.contains_key("c") || v.contains_key("swatch"))
}

fn key(c: &ColorValue) -> String {
    to_hex(c, false).to_lowercase()
}

/// Deep walk; `Some(new)` when something changed.
fn walk(v: &Value, f: &mut dyn FnMut(&ColorValue) -> Option<ColorValue>) -> Option<Value> {
    match v {
        Value::Object(m) if is_color(m) => {
            let c: ColorValue = serde_json::from_value(v.clone()).ok()?;
            let r = f(&c)?;
            let mut out = jm(&r);
            match m.get("a") {
                Some(a) => {
                    out.insert("a".into(), a.clone());
                }
                None => {
                    out.remove("a");
                }
            }
            Some(Value::Object(out))
        }
        Value::Array(a) => {
            let mut changed = false;
            let out: Vec<Value> = a
                .iter()
                .map(|x| match walk(x, f) {
                    Some(n) => {
                        changed = true;
                        n
                    }
                    None => x.clone(),
                })
                .collect();
            changed.then_some(Value::Array(out))
        }
        Value::Object(m) => {
            let mut changed = false;
            let mut out = Map::new();
            for (k, x) in m {
                match walk(x, f) {
                    Some(n) => {
                        changed = true;
                        out.insert(k.clone(), n);
                    }
                    None => {
                        out.insert(k.clone(), x.clone());
                    }
                }
            }
            changed.then_some(Value::Object(out))
        }
        _ => None,
    }
}

const STRUCTURAL: &[&str] = &["id", "type", "parentId", "z", "xf", "deleted", "lock"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorCount {
    pub hex: String,
    pub count: usize,
}

fn lock_flag(el: &Element, content: bool) -> bool {
    el.base().and_then(|b| b.lock.as_ref()).is_some_and(|l| if content { l.content == Some(true) } else { l.style == Some(true) })
}

/// Distinct colours used by the design, most used first.
pub fn design_colors(doc: &DesignDoc) -> Vec<ColorCount> {
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    let mut visit = |c: &ColorValue| {
        *counts.entry(key(c)).or_insert(0) += 1;
        None
    };
    for el in doc.elements.values() {
        if el.is_deleted() {
            continue;
        }
        let mut m = jm(el);
        m.retain(|k, _| !STRUCTURAL.contains(&k.as_str()));
        walk(&Value::Object(m), &mut visit);
    }
    for p in sorted_pages(&doc.pages) {
        walk(&jv(&p.background), &mut visit);
    }
    let mut out: Vec<ColorCount> = counts.into_iter().map(|(hex, count)| ColorCount { hex, count }).collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.count));
    out
}

/// Ops replacing colours (hex → hex) everywhere; alpha is preserved.
pub fn recolor_ops(doc: &DesignDoc, map: &IndexMap<String, String>) -> Vec<DocOp> {
    let m: IndexMap<String, ColorValue> =
        map.iter().filter_map(|(a, b)| hex(b).ok().map(|c| (a.to_lowercase(), c))).collect();
    let mut f = |c: &ColorValue| m.get(&key(c)).cloned();
    let mut ops = Vec::new();
    for el in doc.elements.values() {
        if el.is_deleted() || lock_flag(el, true) {
            continue;
        }
        let mut patch = Map::new();
        for (k, v) in jm(el) {
            if STRUCTURAL.contains(&k.as_str()) {
                continue;
            }
            if let Some(n) = walk(&v, &mut f) {
                patch.insert(k, n);
            }
        }
        if !patch.is_empty() {
            ops.push(DocOp::Update { id: el.id().unwrap_or("").to_string(), patch });
        }
    }
    for p in sorted_pages(&doc.pages) {
        if let Some(n) = walk(&jv(&p.background), &mut f) {
            let mut patch = Map::new();
            patch.insert("background".into(), n);
            ops.push(DocOp::Page(oc_crdt::PageOp::Update { page_id: p.id.clone(), patch }));
        }
    }
    ops
}

fn lum(h: &str) -> f64 {
    hex(h).map(|c| luminance(&c)).unwrap_or(f64::NAN)
}

fn cmp_num(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

/// Map the design's colours onto a palette by luminance rank (darkest → darkest, lightest → lightest).
pub fn palette_map(doc: &DesignDoc, palette: &[String]) -> IndexMap<String, String> {
    let mut pal: Vec<String> = Vec::new();
    for p in palette {
        let p = p.to_lowercase();
        if hex(&p).is_ok() && !pal.contains(&p) {
            pal.push(p);
        }
    }
    pal.sort_by(|a, b| cmp_num(lum(a), lum(b)));
    let used: Vec<String> = design_colors(doc).into_iter().map(|c| c.hex).collect();
    let mut out = IndexMap::new();
    if pal.is_empty() || used.is_empty() {
        return out;
    }
    let mut by_lum = used.clone();
    by_lum.sort_by(|a, b| cmp_num(lum(a), lum(b)));
    let n = by_lum.len();
    for (i, h) in by_lum.iter().enumerate() {
        // Pure black/white text and backgrounds stay neutral when the palette lacks near-neutrals.
        let l = lum(h);
        if (l < 0.01 || l > 0.97) && !pal.iter().any(|p| (lum(p) - l).abs() < 0.08) {
            continue;
        }
        let k = if n == 1 { pal.len() / 2 } else { round((i as f64 / (n - 1) as f64) * (pal.len() - 1) as f64) as usize };
        out.insert(h.clone(), pal[k].clone());
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSet {
    pub heading: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subheading: Option<String>,
    pub body: String,
}

fn body_size(b: &TextBody) -> f64 {
    let mut m = b.default_run.size;
    for p in &b.paragraphs {
        for r in &p.runs {
            m = max(m, resolve_run_style(b, r).size);
        }
    }
    m
}

/// Ops applying a font pair: headline/subhead roles, else the largest text gets the heading font.
pub fn font_set_ops(doc: &DesignDoc, set: &FontSet) -> Vec<DocOp> {
    let texts: Vec<&oc_schema::elements::TextEl> = doc
        .elements
        .values()
        .filter_map(|e| match e {
            Element::Text(t) if t.base.deleted.is_none() && t.base.lock.as_ref().and_then(|l| l.content) != Some(true) => Some(t),
            _ => None,
        })
        .collect();
    let mut sizes: Vec<f64> = texts.iter().map(|t| body_size(&t.body)).collect();
    sizes.sort_by(|a, b| cmp_num(*b, *a));
    let idx = (((sizes.len() as f64) * 0.3).floor() as i64 - 1).max(0) as usize;
    let heading_min = sizes.get(idx).copied().unwrap_or(f64::INFINITY);
    let mut ops = Vec::new();
    for t in texts {
        if oc_schema::js::trim(&body_plain_text(&t.body)).is_empty() {
            continue;
        }
        let s = body_size(&t.body);
        let fam = match &t.base.role {
            Some(SemanticRole::Headline) | Some(SemanticRole::Logo) => set.heading.clone(),
            Some(SemanticRole::Subhead) => set.subheading.clone().unwrap_or_else(|| set.heading.clone()),
            Some(SemanticRole::Body) | Some(SemanticRole::Caption) | Some(SemanticRole::Quote) => set.body.clone(),
            _ => {
                if s >= heading_min && s >= 28.0 {
                    set.heading.clone()
                } else {
                    set.body.clone()
                }
            }
        };
        let mut body = t.body.clone();
        body.default_run.family = fam.clone();
        for p in &mut body.paragraphs {
            for r in &mut p.runs {
                if r.style.family.as_deref().is_some_and(|f| !f.is_empty()) {
                    r.style.family = Some(fam.clone());
                }
            }
        }
        let mut patch = Map::new();
        patch.insert("body".into(), jv(&body));
        ops.push(DocOp::Update { id: t.base.id.clone(), patch });
    }
    ops
}
