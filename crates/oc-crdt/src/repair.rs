// SPDX-License-Identifier: Apache-2.0
//! Load-time repair (CO-004): concurrent edits can leave structurally invalid states that no single client
//! produced — an element whose parent was deleted, a parent cycle from two concurrent re-groupings, a NaN
//! transform. The server fixes them in one transaction so every client converges on a valid document.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use oc_schema::CURRENT_SCHEMA;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use yrs::any::Number;
use yrs::{Any, Doc, Map as _, MapRef, Out, Transact};

use crate::error::CrdtError;
use crate::codec::{from_y_value, is_deleted, replace_content, roots, tombstone};
use crate::origins::REPAIR_ORIGIN;
use crate::value::json_to_any;

/// What a repair pass changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairReport {
    pub orphans: u32,
    pub cycles: u32,
    pub transforms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrated_from: Option<i64>,
}

fn num(a: Option<&Any>) -> Option<f64> {
    match a {
        Some(Any::Number(Number::Int(i))) => Some(*i as f64),
        Some(Any::Number(Number::Float(f))) => Some(*f),
        _ => None,
    }
}

/// `Number.isFinite(v) ? v : d`.
fn finite(a: Option<&Any>, d: f64) -> f64 {
    num(a).filter(|f| f.is_finite()).unwrap_or(d)
}

fn str_of(m: &MapRef, txn: &impl yrs::ReadTxn, k: &str) -> Option<String> {
    match m.get(txn, k) {
        Some(Out::Any(Any::String(s))) => Some(s.to_string()),
        _ => None,
    }
}

fn float(f: f64) -> Any {
    json_to_any(&crate::value::f64_to_json(f))
}

/// Repair orphans, parent cycles and non-finite transforms in one transaction (origin `REPAIR_ORIGIN`).
///
/// `now` is the tombstone time (ms since the epoch). A cycle is broken by re-parenting onto the first page in
/// page order (`sortedPages`); the TS code takes the first key of the yjs pages map, an order yrs cannot
/// reproduce (and that differs between yjs replicas too).
pub fn repair_y(ydoc: &Doc, now: f64) -> RepairReport {
    let r = roots(ydoc);
    let mut report = RepairReport::default();
    let mut txn = ydoc.transact_mut_with(REPAIR_ORIGIN.origin());
    let stamp = json_to_any(&tombstone(now, "repair"));
    let first_page: Option<String> = {
        let mut ps: Vec<(String, Option<String>)> = r
            .pages
            .iter(&txn)
            .map(|(k, v)| {
                let order = match v {
                    Out::YMap(m) => str_of(&m, &txn, "order"),
                    _ => None,
                };
                (k.to_string(), order)
            })
            .collect();
        ps.sort_by(|a, b| match (&a.1, &b.1) {
            (Some(x), Some(y)) => oc_schema::ids::compare_order(x, &a.0, y, &b.0),
            _ => oc_schema::js::str_cmp(&a.0, &b.0),
        });
        ps.first().map(|p| p.0.clone())
    };
    let mut ids: Vec<(String, MapRef)> = r
        .elements
        .iter(&txn)
        .filter_map(|(k, v)| match v {
            Out::YMap(m) => Some((k.to_string(), m)),
            _ => None,
        })
        .collect();
    ids.sort_by(|a, b| a.0.cmp(&b.0));
    let el_maps: HashMap<String, MapRef> = ids.iter().cloned().collect();
    for (id, m) in &ids {
        if is_deleted(&txn, m) {
            continue;
        }
        // Parent must be a page or a live element, and following parents must reach a page without looping.
        let mut seen: HashSet<String> = HashSet::from([id.clone()]);
        let mut p = str_of(m, &txn, "parentId");
        let mut broken: Option<&str> = None;
        while let Some(pid) = p.clone() {
            if r.pages.contains_key(&txn, &pid) {
                break;
            }
            let pm = el_maps.get(&pid);
            if !pm.is_some_and(|pm| !is_deleted(&txn, pm)) {
                broken = Some("orphan");
                break;
            }
            if seen.contains(&pid) {
                broken = Some("cycle");
                break;
            }
            seen.insert(pid);
            p = pm.and_then(|pm| str_of(pm, &txn, "parentId"));
        }
        if p.is_none() && broken.is_none() {
            broken = Some("orphan");
        }
        match broken {
            Some("orphan") => {
                m.insert(&mut txn, "deleted", stamp.clone());
                report.orphans += 1;
            }
            Some("cycle") => {
                // Break the loop by re-parenting this element onto the first page.
                match &first_page {
                    Some(fp) => {
                        m.insert(&mut txn, "parentId", fp.clone());
                    }
                    None => {
                        m.insert(&mut txn, "deleted", stamp.clone());
                    }
                }
                report.cycles += 1;
            }
            _ => {}
        }
        let xf = match m.get(&txn, "xf") {
            Some(Out::Any(a)) if !matches!(a, Any::Null | Any::Undefined | Any::Bool(false)) => a,
            _ => continue,
        };
        let fields: HashMap<String, Any> = match &xf {
            Any::Map(f) => f.as_ref().clone(),
            _ => HashMap::new(),
        };
        let all_finite = ["x", "y", "w", "h", "rot"].iter().all(|k| num(fields.get(*k)).is_some_and(f64::is_finite));
        if !all_finite {
            let mut next = fields.clone();
            next.insert("x".into(), float(finite(fields.get("x"), 0.0)));
            next.insert("y".into(), float(finite(fields.get("y"), 0.0)));
            next.insert("w".into(), float(finite(fields.get("w"), 100.0).max(1.0)));
            next.insert("h".into(), float(finite(fields.get("h"), 100.0).max(1.0)));
            next.insert("rot".into(), float(finite(fields.get("rot"), 0.0)));
            m.insert(&mut txn, "xf", Any::Map(Arc::new(next)));
            report.transforms += 1;
        }
    }
    report
}

/// Run registered schema migrators on an older document (ADR-0011), then repair. Fails like the TS function
/// when no migrator chain reaches `CURRENT_SCHEMA` (nothing is written then).
pub fn upgrade_y(ydoc: &Doc, now: f64) -> Result<RepairReport, CrdtError> {
    let doc = from_y_value(ydoc);
    let mut migrated_from = None;
    if let Some(v) = doc.get("meta").and_then(|m| m.get("schemaVersion")).and_then(Value::as_f64)
        && v < CURRENT_SCHEMA as f64
    {
        let typed: oc_schema::DesignDoc = serde_json::from_value(doc)?;
        let migrated = oc_schema::migrate::migrate(typed)?;
        let mv = oc_schema::js::to_value(&migrated)?;
        migrated_from = Some(v as i64);
        replace_content(ydoc, &mv, Some(REPAIR_ORIGIN.origin()), "migrate", now);
        let r = roots(ydoc);
        let mut txn = ydoc.transact_mut_with(REPAIR_ORIGIN.origin());
        r.meta.insert(&mut txn, "schemaVersion", Any::Number(Number::Int(CURRENT_SCHEMA)));
    }
    Ok(RepairReport { migrated_from, ..repair_y(ydoc, now) })
}
