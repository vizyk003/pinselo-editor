// SPDX-License-Identifier: Apache-2.0
//! Read-side helpers over a design snapshot. Normalizes invariants at read time (cycles, orphans, ties).

use std::cmp::Ordering;
use std::collections::HashSet;

use indexmap::IndexMap;
use serde_json::Value;

use crate::design::Page;
use crate::elements::Element;
use crate::js;
use crate::primitives::{ElementId, PageId};

pub type Pages = IndexMap<PageId, Page>;
pub type Elements = IndexMap<ElementId, Element>;

/// JS truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `compareOrder` where a missing key or id compares like `undefined` in JS (neither less nor greater).
fn cmp_opt(a: Option<&str>, b: Option<&str>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => js::str_cmp(a, b),
        _ => Ordering::Equal,
    }
}

fn order_of(a_key: Option<&str>, a_id: Option<&str>, b_key: Option<&str>, b_id: Option<&str>) -> Ordering {
    cmp_opt(a_key, b_key).then_with(|| cmp_opt(a_id, b_id))
}

pub fn sorted_pages(pages: &Pages) -> Vec<&Page> {
    let mut v: Vec<&Page> = pages.values().collect();
    v.sort_by(|a, b| order_of(Some(&a.order), Some(&a.id), Some(&b.order), Some(&b.id)));
    v
}

pub fn live_elements(elements: &Elements) -> Vec<&Element> {
    elements.values().filter(|e| !e.is_deleted()).collect()
}

/// Children of a parent (page or group), bottom-most first.
pub fn children_of<'a>(elements: &'a Elements, parent_id: &str) -> Vec<&'a Element> {
    let mut v: Vec<&Element> = elements
        .values()
        .filter(|e| !e.is_deleted() && e.parent_id() == Some(parent_id))
        .collect();
    v.sort_by(|a, b| order_of(a.z(), a.id(), b.z(), b.id()));
    v
}

/// The page an element ultimately belongs to, or `None` for orphans / cycles.
pub fn page_of<'a>(pages: &'a Pages, elements: &'a Elements, id: &str) -> Option<&'a str> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut cur: Option<&str> = Some(id);
    while let Some(c) = cur {
        if c.is_empty() || !seen.insert(c) {
            break;
        }
        if let Some((k, _)) = pages.get_key_value(c) {
            return Some(k.as_str());
        }
        let el = elements.get(c)?;
        cur = el.parent_id();
    }
    None
}

/// All live descendants of an element or page (depth-first, paint order).
pub fn descendants<'a>(elements: &'a Elements, id: &str) -> Vec<&'a Element> {
    fn visit<'a>(elements: &'a Elements, pid: &str, depth: usize, out: &mut Vec<&'a Element>) {
        if depth > 64 {
            return;
        }
        for c in children_of(elements, pid) {
            out.push(c);
            if let Some(cid) = c.id() {
                visit(elements, cid, depth + 1, out);
            } else {
                // `visit(undefined)`: children whose parentId is undefined never match a string parent.
            }
        }
    }
    let mut out = Vec::new();
    visit(elements, id, 0, &mut out);
    out
}

/// Elements on a page in paint order (flattened, groups expanded).
pub fn page_elements<'a>(elements: &'a Elements, page_id: &str) -> Vec<&'a Element> {
    descendants(elements, page_id)
}

/// Ancestors of an element up to (not including) its page, nearest first.
pub fn ancestors<'a>(elements: &'a Elements, id: &str) -> Vec<&'a Element> {
    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::from([id]);
    let mut cur = elements.get(id).and_then(Element::parent_id);
    while let Some(c) = cur {
        if c.is_empty() || seen.contains(c) {
            break;
        }
        let Some(el) = elements.get(c) else { break };
        out.push(el);
        seen.insert(c);
        cur = el.parent_id();
    }
    out
}
