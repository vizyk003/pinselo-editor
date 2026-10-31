// SPDX-License-Identifier: Apache-2.0
//! DocOp: the single write contract (ADR-0041). Humans, the AI agent, templates and imports all call
//! [`apply_ops`], which validates, checks locks and writes inside ONE transaction (all-or-nothing).
//! Text edits use flat offsets (paragraphs joined by `'\n'`, UTF-16 units) instead of ProseMirror steps.
//!
//! Ops are the same JSON objects the TS `DocOp` union describes (`{ "op": "add", "el": …, "parent": … }`), so
//! they can be passed between the TS shell, the WASM core and the server unchanged.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use indexmap::IndexMap;
use oc_schema::elements::{BASE_KEYS, ELEMENT_TYPES, Element};
use oc_schema::js::str_cmp;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use yrs::{Doc, Map as _, MapRef, Origin, Out, ReadTxn, Transact, TransactionMut, XmlFragmentRef};

use crate::brandlock::{BrandAction, brand_lock_error, is_brand_lock};
use crate::codec::{
    Roots, apply_update, element_map, encode_state, is_deleted, new_doc_with_client, page_map, props_of, read_element,
    read_page, roots, text_of, tombstone, write_element, write_page,
};
use crate::rng::{OsRandom, RandomSource, key_between, new_id};
use crate::text::{delete_text, format_all, format_text, insert_text, set_paragraphs, write_body};
use crate::value::{json_to_any, obj, out_to_json, truthy_opt};

/// Where to place an element among its siblings: after an element id, or `"top"` / `"bottom"`.
pub type After = String;

fn double_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

/// One write. Serialises to / from the TS `DocOp` JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum DocOp {
    #[serde(rename = "add")]
    Add {
        el: Value,
        parent: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<After>,
    },
    #[serde(rename = "update")]
    Update { id: String, patch: Map<String, Value> },
    #[serde(rename = "xf")]
    Xf { id: String, xf: Value },
    #[serde(rename = "reparent")]
    Reparent {
        id: String,
        parent: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<After>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        xf: Option<Value>,
    },
    #[serde(rename = "reorder")]
    Reorder { id: String, after: After },
    #[serde(rename = "delete")]
    Delete { id: String },
    /// `body`: a TextBody object or a plain string.
    #[serde(rename = "setText")]
    SetText { id: String, body: Value },
    #[serde(rename = "textInsert")]
    TextInsert {
        id: String,
        offset: f64,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        marks: Option<Map<String, Value>>,
    },
    #[serde(rename = "textDelete")]
    TextDelete { id: String, offset: f64, len: f64 },
    #[serde(rename = "textFormat")]
    TextFormat {
        id: String,
        marks: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        len: Option<f64>,
    },
    #[serde(rename = "textParagraph")]
    TextParagraph {
        id: String,
        attrs: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        len: Option<f64>,
    },
    #[serde(rename = "page")]
    Page(PageOp),
    #[serde(rename = "meta")]
    Meta { patch: Map<String, Value> },
    #[serde(rename = "asset")]
    Asset { asset: Value },
}

/// `{ op: 'page', action: … }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum PageOp {
    /// `after`: absent = after the last page, `null` = first, id = after that page.
    #[serde(rename = "add")]
    Add {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page: Option<Map<String, Value>>,
        #[serde(default, deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
        after: Option<Option<String>>,
    },
    #[serde(rename = "delete")]
    Delete {
        #[serde(rename = "pageId")]
        page_id: String,
    },
    #[serde(rename = "duplicate")]
    Duplicate {
        #[serde(rename = "pageId")]
        page_id: String,
        #[serde(rename = "newPageId", default, skip_serializing_if = "Option::is_none")]
        new_page_id: Option<String>,
    },
    #[serde(rename = "move")]
    Move {
        #[serde(rename = "pageId")]
        page_id: String,
        after: Option<String>,
    },
    #[serde(rename = "update")]
    Update {
        #[serde(rename = "pageId")]
        page_id: String,
        patch: Map<String, Value>,
    },
}

/// `DocOpErrorCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocOpErrorCode {
    #[serde(rename = "SCHEMA")]
    Schema,
    #[serde(rename = "NOT_FOUND")]
    NotFound,
    #[serde(rename = "LOCKED")]
    Locked,
    #[serde(rename = "CYCLE")]
    Cycle,
    #[serde(rename = "INVALID")]
    Invalid,
}

/// One failed op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocOpError {
    pub index: usize,
    pub code: DocOpErrorCode,
    pub message: String,
}

/// `DocOpResult`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DocOpResult {
    pub ok: bool,
    pub applied: usize,
    pub created: Vec<String>,
    pub errors: Vec<DocOpError>,
}

/// `ApplyOptions`.
#[derive(Default)]
pub struct ApplyOptions<'a> {
    /// Tombstone author (`'unknown'` when unset).
    pub actor: Option<String>,
    /// Tombstone time, ms since the epoch. The TS default is `Date.now()`; the crate never reads a clock (it must
    /// run in WASM and deterministic tests), so callers pass it. Unset = 0.
    pub now: Option<f64>,
    /// Apply what can be applied instead of all-or-nothing.
    pub partial: bool,
    /// Brand-template locks (A8): `Some(false)` holds the writer to `lock.by === 'brand'` masks (team members /
    /// designers); `Some(true)` or unset = brand admin (team owner/admin, server-side tools).
    pub brand_admin: Option<bool>,
    /// Random source for new ids and jittered z keys (default: the OS RNG).
    pub rng: Option<&'a mut dyn RandomSource>,
}

struct OpError(DocOpErrorCode, String);

fn err<T>(code: DocOpErrorCode, msg: impl Into<String>) -> Result<T, OpError> {
    Err(OpError(code, msg.into()))
}

/// Keys an `update` may change on a locked element.
const LOCK_EXEMPT: &[&str] = &["lock", "name", "altText", "decorative"];

fn is_base(k: &str) -> bool {
    BASE_KEYS.contains(&k)
}

/// Link marks and run links written by text ops (SEC-01; elements are covered by the schema check). `checkLinks`.
fn check_links(marks: Option<&Map<String, Value>>, path: &str) -> Result<(), OpError> {
    match marks {
        Some(m) if m.contains_key("link") && !oc_schema::links::is_safe_link(m.get("link")) => {
            err(DocOpErrorCode::Schema, format!("{path}link.url: unsafe link URL"))
        }
        _ => Ok(()),
    }
}

/// `checkBodyLinks`: run links and run-style links of a body.
fn check_body_links(body: &Value) -> Result<(), OpError> {
    let Some(ps) = body.get("paragraphs").and_then(Value::as_array) else { return Ok(()) };
    for (i, p) in ps.iter().enumerate() {
        let Some(runs) = p.get("runs").and_then(Value::as_array) else { continue };
        for (j, r) in runs.iter().enumerate() {
            check_links(r.as_object(), &format!("paragraphs.{i}.runs.{j}."))?;
            check_links(r.get("style").and_then(Value::as_object), &format!("paragraphs.{i}.runs.{j}.style."))?;
        }
    }
    Ok(())
}

/// A minimal valid design used to run the element schema check through `oc_schema::validate_doc`.
static CHECK_WRAPPER: LazyLock<Map<String, Value>> = LazyLock::new(|| {
    let d = oc_schema::factory::empty_design(&oc_schema::factory::MetaInit { w: 100.0, h: 100.0, ..Default::default() });
    obj(Some(&oc_schema::js::to_value(&d).unwrap_or(Value::Null)))
});

/// `zElement.safeParse(el)`: issues formatted as `path: message`, joined by `'; '`.
pub fn check_element(el: &Value) -> Result<(), String> {
    let t = el.get("type").and_then(Value::as_str);
    if !t.is_some_and(|t| ELEMENT_TYPES.contains(&t)) {
        // zod's discriminated union with no matching discriminator (option order of `zElement`).
        return Err(
            "type: Invalid discriminator value. Expected 'shape' | 'line' | 'image' | 'frame' | 'group' | 'text' | 'qr' | \
             'video' | 'draw' | 'table' | 'embed' | 'form' | 'grid' | 'chart'"
                .into(),
        );
    }
    let key = match el.get("id") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => "__oc_check__".into(),
    };
    let mut doc = CHECK_WRAPPER.clone();
    let mut elements = Map::new();
    elements.insert(key.clone(), el.clone());
    doc.insert("elements".into(), Value::Object(elements));
    let Err(issues) = oc_schema::validate_doc(&Value::Object(doc)) else { return Ok(()) };
    let prefix = format!("elements.{key}");
    let msgs: Vec<String> = issues
        .into_iter()
        .filter(|i| i.path == prefix || i.path.starts_with(&format!("{prefix}.")))
        .filter(|i| !(i.path == format!("{prefix}.parentId") && i.message.starts_with("parent ") && i.message.ends_with(" does not exist")))
        .filter(|i| i.message != "id does not match key")
        .map(|i| {
            let path = i.path.strip_prefix(&prefix).unwrap_or("").trim_start_matches('.');
            format!("{path}: {}", i.message)
        })
        .collect();
    if msgs.is_empty() { Ok(()) } else { Err(msgs.join("; ")) }
}

struct Ctx<'a, 'r> {
    r: Roots,
    result: DocOpResult,
    now: f64,
    actor: String,
    brand_admin: Option<bool>,
    rng: &'r mut (dyn RandomSource + 'a),
}

fn str_of<T: ReadTxn>(txn: &T, m: &MapRef, k: &str) -> Option<String> {
    match m.get(txn, k) {
        Some(Out::Any(yrs::Any::String(s))) => Some(s.to_string()),
        _ => None,
    }
}

fn json_of<T: ReadTxn>(txn: &T, m: &MapRef, k: &str) -> Option<Value> {
    m.get(txn, k).filter(|v| !crate::value::is_undefined(v)).map(|v| out_to_json(txn, &v))
}

/// `compareOrder({ key, id }, …)`: an `undefined` key is neither less nor greater, so ids decide.
fn cmp_key_id(ak: Option<&str>, ai: &str, bk: Option<&str>, bi: &str) -> std::cmp::Ordering {
    if let (Some(a), Some(b)) = (ak, bk) {
        let c = str_cmp(a, b);
        if c.is_ne() {
            return c;
        }
    }
    str_cmp(ai, bi)
}

fn cmp_sib(a: &(String, Option<String>), b: &(String, Option<String>)) -> std::cmp::Ordering {
    cmp_key_id(a.1.as_deref(), &a.0, b.1.as_deref(), &b.0)
}

impl Ctx<'_, '_> {
    fn el_map(&self, txn: &TransactionMut, id: &str) -> Result<MapRef, OpError> {
        match element_map(txn, &self.r, id) {
            Some(m) if !is_deleted(txn, &m) => Ok(m),
            _ => err(DocOpErrorCode::NotFound, format!("element {id} not found")),
        }
    }

    fn text_frag(&self, txn: &TransactionMut, id: &str) -> Result<XmlFragmentRef, OpError> {
        let m = self.el_map(txn, id)?;
        text_of(txn, &m).map_or_else(|| err(DocOpErrorCode::Invalid, format!("element {id} has no text")), Ok)
    }

    fn lock_of(txn: &TransactionMut, m: &MapRef) -> Option<Value> {
        json_of(txn, m, "lock")
    }

    /// User locks pin position; brand locks are checked separately (by mask) and only for non-admins.
    fn is_locked(txn: &TransactionMut, m: &MapRef) -> bool {
        let l = Self::lock_of(txn, m);
        truthy_opt(l.as_ref().and_then(|l| l.get("move"))) && !is_brand_lock(l.as_ref())
    }

    fn brand_check(&self, txn: &TransactionMut, id: &str, m: &MapRef, action: &BrandAction<'_>) -> Result<(), OpError> {
        if self.brand_admin != Some(false) {
            return Ok(());
        }
        let lock = Self::lock_of(txn, m);
        match brand_lock_error(lock.as_ref(), action) {
            Some(e) => err(DocOpErrorCode::Locked, format!("element {id} can’t be changed: {e}")),
            None => Ok(()),
        }
    }

    /// Text ops target the element itself (or, for a table/shape label, still the element id).
    fn brand_text(&self, txn: &TransactionMut, id: &str, action: BrandAction<'_>) -> Result<(), OpError> {
        match element_map(txn, &self.r, id) {
            Some(m) => self.brand_check(txn, id, &m, &action),
            None => Ok(()),
        }
    }

    fn parent_exists(&self, txn: &TransactionMut, id: &str) -> bool {
        self.r.pages.contains_key(txn, id) || element_map(txn, &self.r, id).is_some_and(|m| !is_deleted(txn, &m))
    }

    /// Live siblings under a parent, ordered by (z, id).
    fn siblings(&self, txn: &TransactionMut, parent: &str, exclude: Option<&str>) -> Vec<(String, Option<String>)> {
        let mut out: Vec<(String, Option<String>)> = Vec::new();
        for (id, m) in self.r.elements.iter(txn) {
            let Out::YMap(m) = m else { continue };
            if Some(id) == exclude || is_deleted(txn, &m) || str_of(txn, &m, "parentId").as_deref() != Some(parent) {
                continue;
            }
            out.push((id.to_string(), str_of(txn, &m, "z")));
        }
        out.sort_by(cmp_sib);
        out
    }

    fn key(&mut self, a: Option<&str>, b: Option<&str>) -> Result<String, OpError> {
        key_between(self.rng, a, b).map_err(|e| OpError(DocOpErrorCode::Invalid, e.0))
    }

    fn z_for(&mut self, txn: &TransactionMut, parent: &str, after: Option<&str>, exclude: Option<&str>) -> Result<String, OpError> {
        let sib = self.siblings(txn, parent, exclude);
        let last = sib.last().and_then(|s| s.1.clone());
        match after {
            None | Some("top") => self.key(last.as_deref(), None),
            Some("bottom") => {
                let first = sib.first().and_then(|s| s.1.clone());
                self.key(None, first.as_deref())
            }
            Some(a) => match sib.iter().position(|s| s.0 == a) {
                None => self.key(last.as_deref(), None),
                Some(i) => {
                    let lo = sib[i].1.clone();
                    let hi = sib.get(i + 1).and_then(|s| s.1.clone());
                    if lo == hi { self.key(lo.as_deref(), None) } else { self.key(lo.as_deref(), hi.as_deref()) }
                }
            },
        }
    }

    fn would_cycle(&self, txn: &TransactionMut, id: &str, parent: &str) -> bool {
        let mut cur = Some(parent.to_string());
        let mut seen = HashSet::new();
        while let Some(c) = cur {
            if c.is_empty() || !seen.insert(c.clone()) {
                break;
            }
            if c == id {
                return true;
            }
            cur = element_map(txn, &self.r, &c).and_then(|m| str_of(txn, &m, "parentId"));
        }
        false
    }

    fn pages_sorted(&self, txn: &TransactionMut) -> Vec<Map<String, Value>> {
        let mut ps: Vec<Map<String, Value>> = Vec::new();
        for (_, m) in self.r.pages.iter(txn) {
            if let Out::YMap(m) = m {
                ps.push(read_page(txn, &m));
            }
        }
        ps.sort_by(|a, b| {
            let s = |m: &Map<String, Value>, k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
            cmp_key_id(s(a, "order").as_deref(), &s(a, "id").unwrap_or_default(), s(b, "order").as_deref(), &s(b, "id").unwrap_or_default())
        });
        ps
    }

    /// `pageOrderAfter`: `after` = `None` (undefined: after the last page), `Some(None)` (null: first), id.
    fn page_order_after(&mut self, txn: &TransactionMut, after: Option<Option<&str>>, exclude: Option<&str>) -> Result<String, OpError> {
        let ps: Vec<Map<String, Value>> =
            self.pages_sorted(txn).into_iter().filter(|p| p.get("id").and_then(Value::as_str) != exclude).collect();
        let order = |i: Option<usize>| -> Option<String> {
            i.and_then(|i| ps.get(i)).and_then(|p| p.get("order").and_then(Value::as_str)).map(str::to_string)
        };
        match after {
            Some(None) => {
                let hi = order(Some(0));
                self.key(None, hi.as_deref())
            }
            _ => {
                let i: Option<usize> = match after {
                    None => ps.len().checked_sub(1),
                    Some(Some(a)) => ps.iter().position(|p| p.get("id").and_then(Value::as_str) == Some(a)),
                    Some(None) => unreachable!(),
                };
                // `ps[-1]` is undefined in JS; `ps[i + 1]` with i = -1 is `ps[0]`.
                let lo = order(i);
                let hi = match i {
                    Some(i) => order(Some(i + 1)),
                    None => order(Some(0)),
                };
                self.key(lo.as_deref(), hi.as_deref())
            }
        }
    }

    /// Cheap elements-only snapshot used for tree walks inside a transaction.
    fn elements_only(&self, txn: &TransactionMut) -> IndexMap<String, Element> {
        let mut ids: Vec<(String, MapRef)> = self
            .r
            .elements
            .iter(txn)
            .filter_map(|(k, v)| match v {
                Out::YMap(m) => Some((k.to_string(), m)),
                _ => None,
            })
            .collect();
        ids.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out = IndexMap::new();
        for (id, m) in ids {
            if is_deleted(txn, &m) {
                continue;
            }
            let mut e = Map::new();
            e.insert("id".into(), Value::String(id.clone()));
            if let Some(p) = json_of(txn, &m, "parentId") {
                e.insert("parentId".into(), p);
            }
            if let Some(z) = json_of(txn, &m, "z") {
                e.insert("z".into(), z);
            }
            out.insert(id, Element::Unknown(e));
        }
        out
    }

    fn descendants(&self, txn: &TransactionMut, id: &str) -> Vec<String> {
        let snap = self.elements_only(txn);
        oc_schema::tree::descendants(&snap, id).into_iter().filter_map(|e| e.id().map(str::to_string)).collect()
    }

    fn run(&mut self, txn: &mut TransactionMut, op: &DocOp) -> Result<(), OpError> {
        use DocOpErrorCode::*;
        match op {
            DocOp::Add { el, parent, after } => {
                let id = el.get("id").and_then(Value::as_str).map(str::to_string);
                if let Some(id) = &id
                    && element_map(txn, &self.r, id).is_some_and(|m| !is_deleted(txn, &m))
                {
                    return err(Invalid, format!("element {id} already exists"));
                }
                if !self.parent_exists(txn, parent) {
                    return err(NotFound, format!("parent {parent} not found"));
                }
                let z = self.z_for(txn, parent, after.as_deref(), None)?;
                let mut e = obj(Some(el));
                e.insert("parentId".into(), Value::String(parent.clone()));
                e.insert("z".into(), Value::String(z));
                let ev = Value::Object(e);
                check_element(&ev).or_else(|m| err(Schema, m))?;
                let e = obj(Some(&ev));
                write_element(txn, &self.r.elements, &e);
                self.result.created.push(id.unwrap_or_default());
                Ok(())
            }
            DocOp::Update { id, patch } => {
                let m = self.el_map(txn, id)?;
                if Self::is_locked(txn, &m) && patch.keys().any(|k| !LOCK_EXEMPT.contains(&k.as_str())) {
                    return err(Locked, format!("element {id} is locked"));
                }
                let keys: Vec<String> = patch.keys().cloned().collect();
                let cur_xf = json_of(txn, &m, "xf");
                let cur_body = if patch.contains_key("body") && self.brand_admin == Some(false) {
                    Some(read_element(txn, &m).get("body").cloned())
                } else {
                    None
                };
                let xf_pair = patch.get("xf").filter(|v| crate::value::truthy(v)).map(|to| (cur_xf.as_ref(), Some(to)));
                let body_pair = cur_body.as_ref().map(|from| (from.as_ref(), patch.get("body")));
                self.brand_check(txn, id, &m, &BrandAction::Patch { keys, xf: xf_pair, body: body_pair })?;
                for k in patch.keys() {
                    if matches!(k.as_str(), "id" | "type" | "parentId" | "z") {
                        return err(Invalid, format!("use reparent/reorder to change {k}"));
                    }
                }
                // Validate the would-be element before writing anything.
                let mut next = read_element(txn, &m);
                for (k, v) in patch {
                    if v.is_null() {
                        next.remove(k);
                    } else {
                        next.insert(k.clone(), v.clone());
                    }
                }
                check_element(&Value::Object(next)).or_else(|e| err(Schema, e))?;
                let props = props_of(txn, &m);
                let is_text = str_of(txn, &m, "type").as_deref() == Some("text");
                for (k, v) in patch {
                    if k == "body" && is_text {
                        let frag = text_of(txn, &m).ok_or_else(|| OpError(Invalid, format!("element {id} has no text")))?;
                        write_body(txn, &frag, v);
                        if let Some(p) = &props
                            && let Some(dr) = v.get("defaultRun")
                        {
                            p.insert(txn, "defaultRun", json_to_any(dr));
                        }
                        continue;
                    }
                    let target = if is_base(k) { Some(m.clone()) } else { props.clone() };
                    let Some(target) = target else {
                        return err(Invalid, "Cannot read properties of undefined (reading 'set')");
                    };
                    if v.is_null() {
                        target.remove(txn, k);
                    } else {
                        target.insert(txn, k.as_str(), json_to_any(v));
                    }
                }
                Ok(())
            }
            DocOp::Xf { id, xf } => {
                let m = self.el_map(txn, id)?;
                if Self::is_locked(txn, &m) {
                    return err(Locked, format!("element {id} is locked"));
                }
                let from = json_of(txn, &m, "xf");
                self.brand_check(txn, id, &m, &BrandAction::Xf { from: from.as_ref(), to: Some(xf) })?;
                let mut next = read_element(txn, &m);
                next.insert("xf".into(), xf.clone());
                check_element(&Value::Object(next)).or_else(|e| err(Schema, e))?;
                m.insert(txn, "xf", json_to_any(xf));
                Ok(())
            }
            DocOp::Reparent { id, parent, after, xf } => {
                let m = self.el_map(txn, id)?;
                if !self.parent_exists(txn, parent) {
                    return err(NotFound, format!("parent {parent} not found"));
                }
                if self.would_cycle(txn, id, parent) {
                    return err(Cycle, format!("cannot move {id} into its own descendant"));
                }
                self.brand_check(txn, id, &m, &BrandAction::Move)?;
                let z = self.z_for(txn, parent, after.as_deref(), Some(id))?;
                m.insert(txn, "z", z);
                m.insert(txn, "parentId", parent.clone());
                if let Some(xf) = xf.as_ref().filter(|v| crate::value::truthy(v)) {
                    m.insert(txn, "xf", json_to_any(xf));
                }
                Ok(())
            }
            DocOp::Reorder { id, after } => {
                let m = self.el_map(txn, id)?;
                self.brand_check(txn, id, &m, &BrandAction::Move)?;
                let parent = str_of(txn, &m, "parentId").unwrap_or_default();
                let z = self.z_for(txn, &parent, Some(after), Some(id))?;
                m.insert(txn, "z", z);
                Ok(())
            }
            DocOp::Delete { id } => {
                let m = self.el_map(txn, id)?;
                let lock = Self::lock_of(txn, &m);
                if truthy_opt(lock.as_ref().and_then(|l| l.get("delete"))) && (!is_brand_lock(lock.as_ref()) || self.brand_admin == Some(false)) {
                    return err(Locked, format!("element {id} cannot be deleted"));
                }
                self.brand_check(txn, id, &m, &BrandAction::Delete)?;
                let desc = self.descendants(txn, id);
                for d in &desc {
                    if let Some(dm) = element_map(txn, &self.r, d) {
                        self.brand_check(txn, d, &dm, &BrandAction::Delete)?;
                    }
                }
                let stamp = json_to_any(&tombstone(self.now, &self.actor));
                m.insert(txn, "deleted", stamp.clone());
                // Tombstone descendants too, so a later undo restores the whole subtree.
                for d in &desc {
                    if let Some(dm) = element_map(txn, &self.r, d) {
                        dm.insert(txn, "deleted", stamp.clone());
                    }
                }
                Ok(())
            }
            DocOp::SetText { id, body } => {
                let m = self.el_map(txn, id)?;
                self.brand_check(txn, id, &m, &BrandAction::Content)?;
                let frag = self.text_frag(txn, id)?;
                let props = props_of(txn, &m);
                let body = match body {
                    Value::String(s) => {
                        let dr = props.as_ref().and_then(|p| json_of(txn, p, "defaultRun"));
                        text_body_json(s, dr)
                    }
                    other => other.clone(),
                };
                check_body_links(&body)?;
                write_body(txn, &frag, &body);
                if let Some(p) = &props
                    && let Some(dr) = body.get("defaultRun")
                {
                    p.insert(txn, "defaultRun", json_to_any(dr));
                }
                Ok(())
            }
            DocOp::TextInsert { id, offset, text, marks } => {
                self.brand_text(txn, id, BrandAction::Content)?;
                check_links(marks.as_ref(), "marks.")?;
                let frag = self.text_frag(txn, id)?;
                insert_text(txn, &frag, *offset as i64, text, marks.as_ref());
                Ok(())
            }
            DocOp::TextDelete { id, offset, len } => {
                self.brand_text(txn, id, BrandAction::Content)?;
                let frag = self.text_frag(txn, id)?;
                delete_text(txn, &frag, *offset as i64, *len as i64);
                Ok(())
            }
            DocOp::TextFormat { id, marks, offset, len } => {
                self.brand_text(txn, id, BrandAction::Style)?;
                check_links(Some(marks), "marks.")?;
                let frag = self.text_frag(txn, id)?;
                match (offset, len) {
                    (Some(o), Some(l)) => format_text(txn, &frag, *o as i64, *l as i64, marks),
                    _ => {
                        format_all(txn, &frag, marks);
                        // Whole-element styling also updates the default run so new text matches.
                        let m = self.el_map(txn, id)?;
                        if let Some(props) = props_of(txn, &m) {
                            let mut dr = obj(json_of(txn, &props, "defaultRun").as_ref());
                            for (k, v) in marks {
                                if v.is_null() {
                                    dr.remove(k);
                                } else {
                                    dr.insert(k.clone(), v.clone());
                                }
                            }
                            props.insert(txn, "defaultRun", json_to_any(&Value::Object(dr)));
                        }
                    }
                }
                Ok(())
            }
            DocOp::TextParagraph { id, attrs, offset, len } => {
                self.brand_text(txn, id, BrandAction::Style)?;
                let frag = self.text_frag(txn, id)?;
                let range = offset.map(|o| (o as i64, len.unwrap_or(0.0) as i64));
                set_paragraphs(txn, &frag, attrs, range);
                Ok(())
            }
            DocOp::Page(p) => self.run_page(txn, p),
            DocOp::Meta { patch } => {
                for (k, v) in patch {
                    self.r.meta.insert(txn, k.as_str(), json_to_any(v));
                }
                Ok(())
            }
            DocOp::Asset { asset } => {
                let id = asset.get("id").map(oc_schema::js::value_to_js_string).unwrap_or_else(|| "undefined".into());
                self.r.assets.insert(txn, id, json_to_any(asset));
                Ok(())
            }
        }
    }

    fn run_page(&mut self, txn: &mut TransactionMut, op: &PageOp) -> Result<(), OpError> {
        use DocOpErrorCode::*;
        match op {
            PageOp::Add { page, after } => {
                let id = match page.as_ref().and_then(|p| p.get("id")).filter(|v| !v.is_null()) {
                    Some(v) => oc_schema::js::value_to_js_string(v),
                    None => new_id(self.rng),
                };
                let mut p = Map::new();
                p.insert("background".into(), serde_json::json!([{ "k": "solid", "color": { "space": "srgb", "r": 1, "g": 1, "b": 1, "a": 1 } }]));
                for (k, v) in page.iter().flatten() {
                    p.insert(k.clone(), v.clone());
                }
                p.insert("id".into(), Value::String(id.clone()));
                let order = self.page_order_after(txn, after.as_ref().map(|a| a.as_deref()), None)?;
                p.insert("order".into(), Value::String(order));
                write_page(txn, &self.r.pages, &p);
                self.result.created.push(id);
                Ok(())
            }
            PageOp::Delete { page_id } => {
                if !self.r.pages.contains_key(txn, page_id) {
                    return err(NotFound, format!("page {page_id} not found"));
                }
                if self.r.pages.len(txn) <= 1 {
                    return err(Invalid, "a design needs at least one page");
                }
                let desc = self.descendants(txn, page_id);
                for d in &desc {
                    if let Some(dm) = element_map(txn, &self.r, d) {
                        self.brand_check(txn, d, &dm, &BrandAction::Delete)?;
                    }
                }
                let stamp = json_to_any(&tombstone(self.now, &self.actor));
                for d in &desc {
                    if let Some(dm) = element_map(txn, &self.r, d) {
                        dm.insert(txn, "deleted", stamp.clone());
                    }
                }
                self.r.pages.remove(txn, page_id);
                Ok(())
            }
            PageOp::Duplicate { page_id, new_page_id } => {
                let Some(src) = page_map(txn, &self.r, page_id) else {
                    return err(NotFound, format!("page {page_id} not found"));
                };
                let mut page = read_page(txn, &src);
                let id = match new_page_id {
                    Some(i) => i.clone(),
                    None => new_id(self.rng),
                };
                let order = self.page_order_after(txn, Some(Some(page_id)), None)?;
                page.insert("id".into(), Value::String(id.clone()));
                page.insert("order".into(), Value::String(order));
                write_page(txn, &self.r.pages, &page);
                let els = self.descendants(txn, page_id);
                let mut remap: HashMap<String, String> = HashMap::from([(page_id.clone(), id.clone())]);
                for e in &els {
                    let n = new_id(self.rng);
                    remap.insert(e.clone(), n);
                }
                for e in &els {
                    let Some(em) = element_map(txn, &self.r, e) else { continue };
                    let mut full = read_element(txn, &em);
                    let parent = full.get("parentId").and_then(Value::as_str).and_then(|p| remap.get(p)).cloned();
                    full.insert("id".into(), Value::String(remap[e].clone()));
                    match parent {
                        Some(p) => full.insert("parentId".into(), Value::String(p)),
                        // `remap.get(e.parentId)` is undefined: the key disappears from the written element.
                        None => full.remove("parentId"),
                    };
                    write_element(txn, &self.r.elements, &full);
                }
                self.result.created.push(id);
                Ok(())
            }
            PageOp::Move { page_id, after } => {
                let Some(m) = page_map(txn, &self.r, page_id) else {
                    return err(NotFound, format!("page {page_id} not found"));
                };
                let order = self.page_order_after(txn, Some(after.as_deref()), Some(page_id))?;
                m.insert(txn, "order", order);
                Ok(())
            }
            PageOp::Update { page_id, patch } => {
                let Some(m) = page_map(txn, &self.r, page_id) else {
                    return err(NotFound, format!("page {page_id} not found"));
                };
                for (k, v) in patch {
                    if k == "id" || k == "order" {
                        continue;
                    }
                    if v.is_null() {
                        m.remove(txn, k);
                    } else {
                        m.insert(txn, k.as_str(), json_to_any(v));
                    }
                }
                Ok(())
            }
        }
    }
}

/// `textBody(text, defaultRun)` as JSON: one paragraph per line with `DEFAULT_PARAGRAPH` attributes.
pub fn text_body_json(text: &str, default_run: Option<Value>) -> Value {
    let paragraphs: Vec<Value> = text
        .split('\n')
        .map(|line| {
            let mut p = crate::text::default_paragraph();
            let runs = if line.is_empty() { vec![] } else { vec![serde_json::json!({ "text": line, "style": {} })] };
            p.insert("runs".into(), Value::Array(runs));
            Value::Object(p)
        })
        .collect();
    let mut b = Map::new();
    b.insert("paragraphs".into(), Value::Array(paragraphs));
    if let Some(dr) = default_run {
        b.insert("defaultRun".into(), dr);
    }
    Value::Object(b)
}

fn run_all(ydoc: &Doc, ops: &[DocOp], origin: Option<Origin>, ctx: &mut Ctx<'_, '_>) {
    let mut txn = match origin {
        Some(o) => ydoc.transact_mut_with(o),
        None => ydoc.transact_mut(),
    };
    for (index, op) in ops.iter().enumerate() {
        match ctx.run(&mut txn, op) {
            Ok(()) => ctx.result.applied += 1,
            Err(OpError(code, message)) => {
                ctx.result.ok = false;
                ctx.result.errors.push(DocOpError { index, code, message });
            }
        }
    }
}

/// Apply ops in one transaction. Without `partial`, a multi-op batch is dry-run on a scratch copy first and
/// nothing is written if any op fails.
pub fn apply_ops(ydoc: &Doc, ops: &[DocOp], origin: Option<Origin>, opts: ApplyOptions<'_>) -> DocOpResult {
    let mut os = OsRandom;
    let ApplyOptions { actor, now, partial, brand_admin, rng } = opts;
    let rng: &mut dyn RandomSource = match rng {
        Some(r) => r,
        None => &mut os,
    };
    let actor = actor.unwrap_or_else(|| "unknown".into());
    let now = now.unwrap_or(0.0);

    // All-or-nothing: each op validates before it writes, so a single op is atomic on its own.
    // Multi-op batches are dry-run against a scratch clone first.
    if !partial && ops.len() > 1 {
        let scratch = new_doc_with_client(ydoc.client_id().get());
        let _ = apply_update(&scratch, &encode_state(ydoc), None);
        let mut ctx = Ctx {
            r: roots(&scratch),
            result: DocOpResult { ok: true, ..Default::default() },
            now,
            actor: actor.clone(),
            brand_admin,
            rng: &mut *rng,
        };
        run_all(&scratch, ops, None, &mut ctx);
        if !ctx.result.errors.is_empty() {
            return DocOpResult { ok: false, applied: 0, created: vec![], errors: ctx.result.errors };
        }
    }

    let mut ctx = Ctx { r: roots(ydoc), result: DocOpResult { ok: true, ..Default::default() }, now, actor, brand_admin, rng };
    run_all(ydoc, ops, origin, &mut ctx);
    ctx.result
}

/// Parse ops from JSON (the TS `DocOp[]` shape).
pub fn parse_ops(v: &Value) -> Result<Vec<DocOp>, serde_json::Error> {
    serde_json::from_value(v.clone())
}
