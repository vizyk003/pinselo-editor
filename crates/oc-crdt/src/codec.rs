// SPDX-License-Identifier: Apache-2.0
//! JSON snapshot ⇄ Y.Doc. Root layout (docs/plan/03 §4.1, storage 'single'), frozen and shared with yjs:
//!
//! ```text
//! meta: Y.Map (scalars)   pages: Y.Map<PageId, Y.Map>   elements: Y.Map<ElementId, Y.Map>   assets: Y.Map<AssetId, plain>
//! ```
//!
//! Element map: base fields as plain (atomic) values, type-specific fields in `props: Y.Map`, rich text in
//! `text: Y.XmlFragment` (with the body's `defaultRun` in `props`).
//!
//! The core functions work on JSON (`serde_json::Value`) exactly like the TS code works on plain objects; the
//! typed wrappers ([`to_y`], [`from_y`], [`encode_doc`], [`decode_doc`]) convert through `oc_schema::DesignDoc`.

use oc_schema::DesignDoc;
use oc_schema::elements::BASE_KEYS;
use serde_json::{Map, Value};
use yrs::updates::decoder::Decode;
use yrs::{
    Doc, Map as _, MapPrelim, MapRef, OffsetKind, Options, Origin, Out, ReadTxn, StateVector, Transact, TransactionMut,
    Update, WriteTxn, XmlFragmentPrelim, XmlFragmentRef,
};

use crate::error::CrdtError;
use crate::text::{read_paragraphs, write_body};
use crate::value::{is_undefined, json_to_any, out_to_json, truthy_out};

/// Root map names (frozen; shared with the browser's yjs doc).
pub const META: &str = "meta";
pub const PAGES: &str = "pages";
pub const ELEMENTS: &str = "elements";
pub const ASSETS: &str = "assets";
/// Fields of a text element that live in the fragment rather than in props.
pub const TEXT_BODY_KEY: &str = "body";

/// The four root maps.
#[derive(Debug, Clone)]
pub struct Roots {
    pub meta: MapRef,
    pub pages: MapRef,
    pub elements: MapRef,
    pub assets: MapRef,
}

/// A new document configured for Pinselo: UTF-16 text offsets (the JS string semantics yjs uses).
pub fn new_doc() -> Doc {
    Doc::with_options(Options { offset_kind: OffsetKind::Utf16, ..Options::default() })
}

/// [`new_doc`] with a fixed client id (deterministic updates in tests and goldens).
pub fn new_doc_with_client(client_id: u64) -> Doc {
    Doc::with_options(Options { client_id: yrs::block::ClientID::new(client_id), offset_kind: OffsetKind::Utf16, ..Options::default() })
}

/// The root maps of a document (must not be called while a transaction is open).
pub fn roots(ydoc: &Doc) -> Roots {
    Roots {
        meta: ydoc.get_or_insert_map(META),
        pages: ydoc.get_or_insert_map(PAGES),
        elements: ydoc.get_or_insert_map(ELEMENTS),
        assets: ydoc.get_or_insert_map(ASSETS),
    }
}

/// The root maps, from inside a write transaction.
pub fn roots_txn(txn: &mut TransactionMut) -> Roots {
    Roots {
        meta: txn.get_or_insert_map(META),
        pages: txn.get_or_insert_map(PAGES),
        elements: txn.get_or_insert_map(ELEMENTS),
        assets: txn.get_or_insert_map(ASSETS),
    }
}

/// `Object.fromEntries(map.entries())` as JSON (keys sorted; `undefined` values dropped).
pub fn map_to_json<T: ReadTxn>(txn: &T, map: &MapRef) -> Map<String, Value> {
    let mut entries: Vec<(String, Out)> = map.iter(txn).map(|(k, v)| (k.to_string(), v)).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.into_iter().filter(|(_, v)| !is_undefined(v)).map(|(k, v)| (k, out_to_json(txn, &v))).collect()
}

fn is_base(k: &str) -> bool {
    BASE_KEYS.contains(&k)
}

/// Write an element (JSON object) as a new Y.Map under `elements[el.id]`.
pub fn write_element(txn: &mut TransactionMut, elements: &MapRef, el: &Map<String, Value>) -> MapRef {
    let id = el.get("id").map(oc_schema::js::value_to_js_string).unwrap_or_default();
    let map = elements.insert(txn, id, MapPrelim::default());
    let props = map.insert(txn, "props", MapPrelim::default());
    let is_text = el.get("type").and_then(Value::as_str) == Some("text");
    for (k, v) in el {
        if is_base(k) {
            map.insert(txn, k.as_str(), json_to_any(v));
        } else if is_text && k == TEXT_BODY_KEY {
            continue;
        } else {
            props.insert(txn, k.as_str(), json_to_any(v));
        }
    }
    if is_text {
        let frag = map.insert(txn, "text", XmlFragmentPrelim::default());
        let body = el.get(TEXT_BODY_KEY).cloned().unwrap_or(Value::Null);
        write_body(txn, &frag, &body);
        if let Some(dr) = body.get("defaultRun") {
            props.insert(txn, "defaultRun", json_to_any(dr));
        }
    }
    map
}

/// The `props` sub-map of an element map.
pub fn props_of<T: ReadTxn>(txn: &T, map: &MapRef) -> Option<MapRef> {
    match map.get(txn, "props") {
        Some(Out::YMap(m)) => Some(m),
        _ => None,
    }
}

/// The `text` fragment of an element map.
pub fn text_of<T: ReadTxn>(txn: &T, map: &MapRef) -> Option<XmlFragmentRef> {
    match map.get(txn, "text") {
        Some(Out::YXmlFragment(f)) => Some(f),
        _ => None,
    }
}

/// Read an element map back to JSON.
pub fn read_element<T: ReadTxn>(txn: &T, map: &MapRef) -> Map<String, Value> {
    let mut out = Map::new();
    let mut entries: Vec<(String, Out)> = map.iter(txn).map(|(k, v)| (k.to_string(), v)).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in entries {
        if k == "props" || k == "text" || is_undefined(&v) {
            continue;
        }
        out.insert(k, out_to_json(txn, &v));
    }
    let props = props_of(txn, map);
    if let Some(props) = &props {
        for (k, v) in map_to_json(txn, props) {
            if k != "defaultRun" {
                out.insert(k, v);
            }
        }
    }
    if out.get("type").and_then(Value::as_str) == Some("text")
        && let Some(frag) = text_of(txn, map)
    {
        let mut body = Map::new();
        body.insert("paragraphs".into(), Value::Array(read_paragraphs(txn, &frag)));
        if let Some(dr) = props.as_ref().and_then(|p| p.get(txn, "defaultRun"))
            && !is_undefined(&dr)
        {
            body.insert("defaultRun".into(), out_to_json(txn, &dr));
        }
        out.insert(TEXT_BODY_KEY.into(), Value::Object(body));
    }
    out
}

/// Write a page (JSON object) as a new Y.Map under `pages[page.id]`.
pub fn write_page(txn: &mut TransactionMut, pages: &MapRef, page: &Map<String, Value>) -> MapRef {
    let id = page.get("id").map(oc_schema::js::value_to_js_string).unwrap_or_default();
    let map = pages.insert(txn, id, MapPrelim::default());
    for (k, v) in page {
        map.insert(txn, k.as_str(), json_to_any(v));
    }
    map
}

pub fn read_page<T: ReadTxn>(txn: &T, map: &MapRef) -> Map<String, Value> {
    map_to_json(txn, map)
}

pub fn read_meta<T: ReadTxn>(txn: &T, meta: &MapRef) -> Map<String, Value> {
    map_to_json(txn, meta)
}

fn sub_map<T: ReadTxn>(txn: &T, parent: &MapRef, key: &str) -> Option<MapRef> {
    match parent.get(txn, key) {
        Some(Out::YMap(m)) => Some(m),
        _ => None,
    }
}

/// `elements.get(id)` as a Y.Map.
pub fn element_map<T: ReadTxn>(txn: &T, r: &Roots, id: &str) -> Option<MapRef> {
    sub_map(txn, &r.elements, id)
}

/// `pages.get(id)` as a Y.Map.
pub fn page_map<T: ReadTxn>(txn: &T, r: &Roots, id: &str) -> Option<MapRef> {
    sub_map(txn, &r.pages, id)
}

/// Truthiness of `m.get('deleted')`.
pub fn is_deleted<T: ReadTxn>(txn: &T, m: &MapRef) -> bool {
    truthy_out(m.get(txn, "deleted").as_ref())
}

fn obj_entries(v: Option<&Value>) -> Vec<(&String, &Value)> {
    match v {
        Some(Value::Object(m)) => m.iter().collect(),
        _ => vec![],
    }
}

/// Populate (inside `txn`) from a JSON snapshot.
pub fn write_snapshot(txn: &mut TransactionMut, r: &Roots, doc: &Value) {
    for (k, v) in obj_entries(doc.get("meta")) {
        r.meta.insert(txn, k.as_str(), json_to_any(v));
    }
    for (_, p) in obj_entries(doc.get("pages")) {
        write_page(txn, &r.pages, &crate::value::obj(Some(p)));
    }
    for (_, e) in obj_entries(doc.get("elements")) {
        write_element(txn, &r.elements, &crate::value::obj(Some(e)));
    }
    for (_, a) in obj_entries(doc.get("assets")) {
        let id = a.get("id").map(oc_schema::js::value_to_js_string).unwrap_or_default();
        r.assets.insert(txn, id, json_to_any(a));
    }
}

/// Populate an empty Y.Doc from a JSON snapshot (one transaction). `toY`.
pub fn to_y_value(doc: &Value, ydoc: &Doc, origin: Option<Origin>) {
    let r = roots(ydoc);
    let mut txn = match origin {
        Some(o) => ydoc.transact_mut_with(o),
        None => ydoc.transact_mut(),
    };
    write_snapshot(&mut txn, &r, doc);
}

/// Populate an empty Y.Doc from a typed snapshot. `toY`.
pub fn to_y(doc: &DesignDoc, ydoc: &Doc, origin: Option<Origin>) -> Result<(), CrdtError> {
    to_y_value(&oc_schema::js::to_value(doc)?, ydoc, origin);
    Ok(())
}

/// Read a JSON snapshot from inside a transaction.
pub fn read_snapshot<T: ReadTxn>(txn: &T, r: &Roots) -> Value {
    let mut pages = Map::new();
    let mut ps: Vec<(String, Out)> = r.pages.iter(txn).map(|(k, v)| (k.to_string(), v)).collect();
    ps.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, m) in ps {
        if let Out::YMap(m) = m {
            pages.insert(id, Value::Object(read_page(txn, &m)));
        }
    }
    let mut elements = Map::new();
    let mut es: Vec<(String, Out)> = r.elements.iter(txn).map(|(k, v)| (k.to_string(), v)).collect();
    es.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, m) in es {
        if let Out::YMap(m) = m {
            elements.insert(id, Value::Object(read_element(txn, &m)));
        }
    }
    let mut out = Map::new();
    out.insert("meta".into(), Value::Object(read_meta(txn, &r.meta)));
    out.insert("pages".into(), Value::Object(pages));
    out.insert("elements".into(), Value::Object(elements));
    out.insert("assets".into(), Value::Object(map_to_json(txn, &r.assets)));
    Value::Object(out)
}

/// The document as a JSON snapshot. `fromY`.
pub fn from_y_value(ydoc: &Doc) -> Value {
    let r = roots(ydoc);
    let txn = ydoc.transact();
    read_snapshot(&txn, &r)
}

/// The document as a typed snapshot. `fromY`.
pub fn from_y(ydoc: &Doc) -> Result<DesignDoc, CrdtError> {
    Ok(serde_json::from_value(from_y_value(ydoc))?)
}

/// `Y.encodeStateAsUpdate(ydoc)` (lib0 v1, the encoding yjs and Hocuspocus use).
pub fn encode_state(ydoc: &Doc) -> Vec<u8> {
    ydoc.transact().encode_state_as_update_v1(&StateVector::default())
}

/// `Y.encodeStateVector(ydoc)`.
pub fn encode_state_vector(ydoc: &Doc) -> Vec<u8> {
    use yrs::updates::encoder::Encode;
    ydoc.transact().state_vector().encode_v1()
}

/// `Y.encodeStateAsUpdate(ydoc, stateVector)`.
pub fn encode_diff(ydoc: &Doc, state_vector: &[u8]) -> Result<Vec<u8>, CrdtError> {
    let sv = StateVector::decode_v1(state_vector).map_err(|e| CrdtError::Decode(e.to_string()))?;
    Ok(ydoc.transact().encode_state_as_update_v1(&sv))
}

/// `Y.applyUpdate(ydoc, update, origin)`.
pub fn apply_update(ydoc: &Doc, update: &[u8], origin: Option<Origin>) -> Result<(), CrdtError> {
    let u = Update::decode_v1(update).map_err(|e| CrdtError::Decode(e.to_string()))?;
    let mut txn = match origin {
        Some(o) => ydoc.transact_mut_with(o),
        None => ydoc.transact_mut(),
    };
    txn.apply_update(u).map_err(|e| CrdtError::Decode(e.to_string()))
}

/// Encode a JSON snapshot as a yjs update. `encodeDoc`.
pub fn encode_doc_value(doc: &Value) -> Vec<u8> {
    let ydoc = new_doc();
    to_y_value(doc, &ydoc, None);
    encode_state(&ydoc)
}

/// Encode a typed snapshot as a yjs update. `encodeDoc`.
pub fn encode_doc(doc: &DesignDoc) -> Result<Vec<u8>, CrdtError> {
    Ok(encode_doc_value(&oc_schema::js::to_value(doc)?))
}

/// Decode a yjs update into a JSON snapshot. `decodeDoc`.
pub fn decode_doc_value(update: &[u8]) -> Result<Value, CrdtError> {
    let ydoc = new_doc();
    apply_update(&ydoc, update, None)?;
    Ok(from_y_value(&ydoc))
}

/// Decode a yjs update into a typed snapshot. `decodeDoc`.
pub fn decode_doc(update: &[u8]) -> Result<DesignDoc, CrdtError> {
    Ok(serde_json::from_value(decode_doc_value(update)?)?)
}

/// Replace the live document's content with a snapshot (version restore) as ONE transaction.
/// Elements missing from the snapshot are tombstoned with `{ at: now, by: actor }` (so undo can bring them
/// back); the rest are rewritten. `now` is ms since the epoch (passed in: the crate never reads a clock).
pub fn replace_content(ydoc: &Doc, doc: &Value, origin: Option<Origin>, actor: &str, now: f64) {
    let r = roots(ydoc);
    let mut txn = match origin {
        Some(o) => ydoc.transact_mut_with(o),
        None => ydoc.transact_mut(),
    };
    replace_content_txn(&mut txn, &r, doc, actor, now);
}

/// [`replace_content`] inside an open transaction.
pub fn replace_content_txn(txn: &mut TransactionMut, r: &Roots, doc: &Value, actor: &str, now: f64) {
    for (k, v) in obj_entries(doc.get("meta")) {
        if k != "epoch" {
            r.meta.insert(txn, k.as_str(), json_to_any(v));
        }
    }
    let snap_pages = crate::value::obj(doc.get("pages"));
    let snap_elements = crate::value::obj(doc.get("elements"));
    let page_ids: Vec<String> = r.pages.keys(txn).map(str::to_string).collect();
    for id in page_ids {
        if !crate::value::truthy_opt(snap_pages.get(&id)) {
            r.pages.remove(txn, &id);
        }
    }
    for p in snap_pages.values() {
        write_page(txn, &r.pages, &crate::value::obj(Some(p)));
    }
    let stamp = tombstone(now, actor);
    let live: Vec<(String, MapRef)> = r
        .elements
        .iter(txn)
        .filter_map(|(k, v)| match v {
            Out::YMap(m) => Some((k.to_string(), m)),
            _ => None,
        })
        .collect();
    for (id, m) in live {
        if !crate::value::truthy_opt(snap_elements.get(&id)) && !is_deleted(txn, &m) {
            m.insert(txn, "deleted", json_to_any(&stamp));
        }
    }
    for e in snap_elements.values() {
        write_element(txn, &r.elements, &crate::value::obj(Some(e)));
    }
    for a in crate::value::obj(doc.get("assets")).values() {
        let id = a.get("id").map(oc_schema::js::value_to_js_string).unwrap_or_default();
        r.assets.insert(txn, id, json_to_any(a));
    }
}

/// `{ at, by }`.
pub fn tombstone(now: f64, by: &str) -> Value {
    let mut m = Map::new();
    m.insert("at".into(), crate::value::f64_to_json(now));
    m.insert("by".into(), Value::String(by.to_string()));
    Value::Object(m)
}
