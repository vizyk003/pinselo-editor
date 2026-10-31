// SPDX-License-Identifier: Apache-2.0
//! Docs body (DC-001): the Tiptap/ProseMirror tree lives in the XmlFragment `docBody` (y-prosemirror encoding:
//! one XmlElement per node, marks as XmlText formatting attributes). Shared by the render service (layout,
//! DOCX/Markdown export) and the API (embedded-design access checks).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use yrs::{
    ReadTxn, Text, Transact, TransactionMut, WriteTxn, Xml, XmlElementPrelim, XmlElementRef, XmlFragment,
    XmlFragmentRef, XmlOut, XmlTextPrelim,
};

use crate::codec::{apply_update, new_doc};
use crate::error::CrdtError;
use crate::text::to_delta;
use crate::value::{any_to_json, is_undefined, json_to_any, json_to_attrs, out_to_json};

/// The root fragment name.
pub const DOC_FIELD: &str = "docBody";

/// A ProseMirror mark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PmMark {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Map<String, Value>>,
}

/// A ProseMirror node (`{ type, attrs?, content?, text?, marks? }`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PmNode {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<PmNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marks: Option<Vec<PmMark>>,
}

fn element_attrs<T: ReadTxn>(txn: &T, el: &XmlElementRef) -> Map<String, Value> {
    let mut attrs: Vec<(String, Value)> =
        el.attributes(txn).filter(|(_, v)| !is_undefined(v)).map(|(k, v)| (k.to_string(), out_to_json(txn, &v))).collect();
    attrs.sort_by(|a, b| a.0.cmp(&b.0));
    attrs.into_iter().collect()
}

fn conv<T: ReadTxn>(txn: &T, n: &XmlOut) -> Vec<PmNode> {
    match n {
        XmlOut::Text(t) => to_delta(txn, t)
            .into_iter()
            .map(|d| {
                let mut marks: Vec<PmMark> = d
                    .attributes
                    .iter()
                    .flat_map(|a| a.iter())
                    .map(|(k, v)| {
                        let attrs = match any_to_json(v) {
                            Value::Object(m) => m,
                            // Arrays are objects in JS: `typeof [] === 'object'`, so they pass through as attrs.
                            Value::Array(a) => a.into_iter().enumerate().map(|(i, x)| (i.to_string(), x)).collect(),
                            _ => Map::new(),
                        };
                        PmMark { kind: k.to_string(), attrs: Some(attrs) }
                    })
                    .collect();
                // yrs keeps marks in a hash map; sort for a stable order (yjs uses insertion order).
                marks.sort_by(|a, b| a.kind.cmp(&b.kind));
                PmNode { kind: "text".into(), text: Some(d.insert), marks: Some(marks), ..Default::default() }
            })
            .collect(),
        XmlOut::Element(el) => {
            let content: Vec<PmNode> = el.children(txn).flat_map(|c| conv(txn, &c)).collect();
            vec![PmNode { kind: el.tag().to_string(), attrs: Some(element_attrs(txn, el)), content: Some(content), ..Default::default() }]
        }
        XmlOut::Fragment(f) => f.children(txn).flat_map(|c| conv(txn, &c)).collect(),
    }
}

/// XmlFragment (y-prosemirror encoding) → ProseMirror-style JSON. `fragmentToJson`.
pub fn fragment_to_json<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef) -> PmNode {
    let content: Vec<PmNode> = frag.children(txn).flat_map(|c| conv(txn, &c)).collect();
    PmNode { kind: "doc".into(), content: Some(content), ..Default::default() }
}

fn build_into(txn: &mut TransactionMut, parent: &XmlElementRef, n: &PmNode) {
    let mut index = 0u32;
    let mut text: Option<(yrs::XmlTextRef, u32)> = None;
    for c in n.content.iter().flatten() {
        if c.kind == "text" {
            let (t, len) = match text.take() {
                Some(t) => t,
                None => {
                    let t = parent.insert(txn, index, XmlTextPrelim::new(""));
                    index += 1;
                    (t, 0)
                }
            };
            let mut attrs = Map::new();
            for m in c.marks.iter().flatten() {
                attrs.insert(m.kind.clone(), Value::Object(m.attrs.clone().unwrap_or_default()));
            }
            let s = c.text.clone().unwrap_or_default();
            if !s.is_empty() {
                t.insert_with_attributes(txn, len, &s, json_to_attrs(&attrs));
            }
            let len = len + oc_schema::js::utf16_len(&s) as u32;
            text = Some((t, len));
        } else {
            text = None;
            let el = parent.insert(txn, index, XmlElementPrelim::empty(c.kind.as_str()));
            index += 1;
            set_attrs(txn, &el, c);
            build_into(txn, &el, c);
        }
    }
}

fn set_attrs(txn: &mut TransactionMut, el: &XmlElementRef, n: &PmNode) {
    for (k, v) in n.attrs.iter().flatten() {
        if !v.is_null() {
            el.insert_attribute(txn, k.as_str(), json_to_any(v));
        }
    }
}

/// ProseMirror-style JSON → children appended to a fragment (tests, server-side imports). `jsonToFragment`.
pub fn json_to_fragment(txn: &mut TransactionMut, frag: &XmlFragmentRef, root: &PmNode) {
    let start = frag.len(txn);
    for (index, c) in (start..).zip(root.content.iter().flatten()) {
        let el = frag.insert(txn, index, XmlElementPrelim::empty(c.kind.as_str()));
        set_attrs(txn, &el, c);
        build_into(txn, &el, c);
    }
}

/// Walk every node (depth-first). `walkDoc`.
pub fn walk_doc<F: FnMut(&PmNode)>(root: &PmNode, f: &mut F) {
    f(root);
    for c in root.content.iter().flatten() {
        walk_doc(c, f);
    }
}

/// Design ids embedded in a doc body (`designEmbed` blocks), in first-seen order. `docEmbeddedDesignIds`.
pub fn doc_embedded_design_ids(root: &PmNode) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    walk_doc(root, &mut |n| {
        if n.kind == "designEmbed"
            && let Some(Value::String(id)) = n.attrs.as_ref().and_then(|a| a.get("designId"))
            && !id.is_empty()
            && !out.contains(id)
        {
            out.push(id.clone());
        }
    });
    out
}

/// Embedded design ids straight from a persisted state (no full design decode). `embeddedDesignIdsOfState`.
pub fn embedded_design_ids_of_state(state: &[u8]) -> Result<Vec<String>, CrdtError> {
    let ydoc = new_doc();
    apply_update(&ydoc, state, None)?;
    let mut txn = ydoc.transact_mut();
    let frag = txn.get_or_insert_xml_fragment(DOC_FIELD);
    Ok(doc_embedded_design_ids(&fragment_to_json(&txn, &frag)))
}

/// Plain text of a node (headings for a table of contents, alt text…). `nodeText`.
pub fn node_text(n: &PmNode) -> String {
    match n.kind.as_str() {
        "text" => n.text.clone().unwrap_or_default(),
        "hardBreak" => " ".into(),
        _ => n.content.iter().flatten().map(node_text).collect(),
    }
}
