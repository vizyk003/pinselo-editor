// SPDX-License-Identifier: Apache-2.0
//! Brand-template locks (A8). Elements locked `by: 'brand'` hold non-admins to their mask:
//!  - position & size (`move`/`resize`/`rotate`, plus `delete`): no xf change, reparent, reorder or delete;
//!  - `content`: no text edits, image/video swaps, table cell or chart data changes;
//!  - `style`: no fills, strokes, fonts, effects, opacity, … (every other property).
//!
//! Brand admins (team owners/admins) are not restricted. The lock itself can only be changed by an admin.
//! Used by `apply_ops` (editor + API writes) and by the collab server to vet raw Yjs updates.
//!
//! Locks, transforms and bodies are JSON values (the TS code works on plain objects).

use serde_json::{Map, Value};

use crate::value::{canonical_string, same, strict_eq, truthy_opt};

/// Properties that are an element's content rather than its style.
pub const CONTENT_KEYS: &[&str] = &[
    "body", "text", "asset", "crop", "content", "cells", "rows", "cols", "data", "series", "value", "url", "src", "html",
    "fields", "embed", "clip", "poster", "dataBinding", "link", "points", "paths",
];
/// Properties anyone may change, even on a fully brand-locked element.
pub const BRAND_EXEMPT: &[&str] = &["name", "altText", "decorative"];
const IDENTITY: &[&str] = &["id", "type", "parentId", "z", "deleted"];

/// What a writer is about to do to a brand-locked element.
#[derive(Debug, Clone)]
pub enum BrandAction<'a> {
    Patch { keys: Vec<String>, xf: Option<(Option<&'a Value>, Option<&'a Value>)>, body: Option<(Option<&'a Value>, Option<&'a Value>)> },
    Xf { from: Option<&'a Value>, to: Option<&'a Value> },
    Move,
    Delete,
    Content,
    Style,
}

fn flag(lock: &Value, k: &str) -> bool {
    truthy_opt(lock.get(k))
}

/// `lock.by === 'brand'`.
pub fn is_brand_lock(lock: Option<&Value>) -> bool {
    matches!(lock, Some(l) if l.get("by").and_then(Value::as_str) == Some("brand"))
}

fn field<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a Value> {
    v.and_then(|v| v.get(k))
}

/// `(v ?? 0)`.
fn or_zero(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => Value::from(0),
        Some(x) => x.clone(),
    }
}

fn xf_error(lock: &Value, from: Option<&Value>, to: Option<&Value>) -> Option<&'static str> {
    let ne = |k: &str| !strict_eq(field(from, k), field(to, k));
    if flag(lock, "move") && (ne("x") || ne("y")) {
        return Some("its position is locked by the brand template");
    }
    if flag(lock, "resize")
        && (ne("w") || ne("h") || truthy_opt(field(from, "flipX")) != truthy_opt(field(to, "flipX")) || truthy_opt(field(from, "flipY")) != truthy_opt(field(to, "flipY")))
    {
        return Some("its size is locked by the brand template");
    }
    if flag(lock, "rotate") && !strict_eq(Some(&or_zero(field(from, "rot"))), Some(&or_zero(field(to, "rot")))) {
        return Some("its rotation is locked by the brand template");
    }
    None
}

/// Why `action` would break the brand lock (`None` = allowed).
pub fn brand_lock_error(lock: Option<&Value>, action: &BrandAction<'_>) -> Option<&'static str> {
    if !is_brand_lock(lock) {
        return None;
    }
    let lock = lock?;
    match action {
        BrandAction::Xf { from, to } => xf_error(lock, *from, *to),
        BrandAction::Move => flag(lock, "move").then_some("its position is locked by the brand template"),
        BrandAction::Delete => (flag(lock, "delete") || flag(lock, "move")).then_some("it is locked by the brand template"),
        BrandAction::Content => flag(lock, "content").then_some("its content is locked by the brand template"),
        BrandAction::Style => flag(lock, "style").then_some("its style is locked by the brand template"),
        BrandAction::Patch { keys, xf, body } => {
            for k in keys {
                let k = k.as_str();
                if BRAND_EXEMPT.contains(&k) || IDENTITY.contains(&k) {
                    continue;
                }
                if k == "lock" {
                    return Some("only brand admins can change brand-template locks");
                }
                if k == "xf" {
                    let e = match xf {
                        Some((from, to)) => xf_error(lock, *from, *to),
                        None => (flag(lock, "move") || flag(lock, "resize") || flag(lock, "rotate"))
                            .then_some("its position is locked by the brand template"),
                    };
                    if e.is_some() {
                        return e;
                    }
                    continue;
                }
                if k == "body" {
                    // Rich text carries both: compare the words and the styling separately when both are known.
                    let ch = match body {
                        Some((from, to)) => body_change(*from, *to),
                        None => BodyChange { content: true, style: true },
                    };
                    if ch.content && flag(lock, "content") {
                        return Some("its content is locked by the brand template");
                    }
                    if ch.style && flag(lock, "style") {
                        return Some("its style is locked by the brand template");
                    }
                    continue;
                }
                if CONTENT_KEYS.contains(&k) {
                    if flag(lock, "content") {
                        return Some("its content is locked by the brand template");
                    }
                } else if flag(lock, "style") {
                    return Some("its style is locked by the brand template");
                }
            }
            None
        }
    }
}

/// What changed between two text bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BodyChange {
    pub content: bool,
    pub style: bool,
}

fn body_text(t: &Value) -> String {
    let paras = t.get("paragraphs").and_then(Value::as_array).cloned().unwrap_or_default();
    paras
        .iter()
        .map(|p| {
            p.get("runs")
                .and_then(Value::as_array)
                .map(|rs| rs.iter().map(|r| r.get("text").map(oc_schema::js::value_to_js_string).unwrap_or_default()).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn body_look(t: &Value) -> String {
    let paras = t.get("paragraphs").and_then(Value::as_array).cloned().unwrap_or_default();
    let attrs: Vec<Value> = paras
        .iter()
        .map(|p| {
            let mut m: Map<String, Value> = crate::value::obj(Some(p));
            m.remove("runs");
            Value::Object(m)
        })
        .collect();
    let mut styles: Vec<String> = paras
        .iter()
        .flat_map(|p| p.get("runs").and_then(Value::as_array).cloned().unwrap_or_default())
        .map(|r| r.get("style").map(canonical_string).unwrap_or_else(|| "undefined".into()))
        .collect();
    styles.sort_by(|a, b| oc_schema::js::str_cmp(a, b));
    styles.dedup();
    canonical_string(&Value::Array(vec![
        t.get("defaultRun").cloned().unwrap_or(Value::Null),
        Value::Array(attrs),
        Value::Array(styles.into_iter().map(Value::String).collect()),
    ]))
}

/// What changed between two text bodies: the words, the styling, or both. Comparisons are by value (key
/// order does not matter).
pub fn body_change(a: Option<&Value>, b: Option<&Value>) -> BodyChange {
    let has_paras = |v: Option<&Value>| truthy_opt(field(v, "paragraphs"));
    match (a, b) {
        (Some(ta), Some(tb)) if has_paras(a) && has_paras(b) => {
            BodyChange { content: body_text(ta) != body_text(tb), style: body_look(ta) != body_look(tb) }
        }
        _ => {
            let s = same(a, b);
            BodyChange { content: !s, style: !s }
        }
    }
}

/// A brand-lock violation found by [`brand_lock_violations`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Violation {
    pub id: String,
    pub reason: String,
}

/// Compare two versions of the elements map and list brand-lock violations (collab server check).
/// Only elements that were brand-locked before the change are inspected.
pub fn brand_lock_violations(before: &Map<String, Value>, after: &Map<String, Value>) -> Vec<Violation> {
    let mut out = Vec::new();
    for (id, a) in before {
        if truthy_opt(a.get("deleted")) || !is_brand_lock(a.get("lock")) {
            continue;
        }
        let lock = a.get("lock");
        let b = match after.get(id) {
            Some(b) if !truthy_opt(b.get("deleted")) => b,
            _ => {
                if let Some(e) = brand_lock_error(lock, &BrandAction::Delete) {
                    out.push(Violation { id: id.clone(), reason: e.into() });
                }
                continue;
            }
        };
        let mut keys: Vec<String> = crate::value::obj(Some(a)).keys().cloned().collect();
        for k in crate::value::obj(Some(b)).keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        let changed: Vec<String> = keys.into_iter().filter(|k| !same(a.get(k), b.get(k))).collect();
        if changed.is_empty() {
            continue;
        }
        if changed.iter().any(|k| k == "parentId" || k == "z")
            && let Some(e) = brand_lock_error(lock, &BrandAction::Move)
        {
            out.push(Violation { id: id.clone(), reason: e.into() });
            continue;
        }
        let action = BrandAction::Patch {
            keys: changed,
            xf: Some((a.get("xf"), b.get("xf"))),
            body: Some((a.get("body"), b.get("body"))),
        };
        if let Some(e) = brand_lock_error(lock, &action) {
            out.push(Violation { id: id.clone(), reason: e.into() });
        }
    }
    out
}
