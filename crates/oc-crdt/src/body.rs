// SPDX-License-Identifier: Apache-2.0
//! Text ops on a detached TextBody (table cells, A12). Runs the exact Y.XmlFragment code text elements use on a
//! scratch doc, so typing, deleting, formatting and paragraph attributes behave identically in a cell.

use oc_schema::TextBody;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use yrs::{Transact, WriteTxn};

use crate::codec::new_doc;
use crate::error::CrdtError;
use crate::text::{delete_text, format_all, format_text, insert_text, read_paragraphs, set_paragraphs, write_body};

/// `BodyOp` (the text subset of `DocOp`, without an element id).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum BodyOp {
    #[serde(rename = "textInsert")]
    TextInsert {
        offset: f64,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        marks: Option<Map<String, Value>>,
    },
    #[serde(rename = "textDelete")]
    TextDelete { offset: f64, len: f64 },
    #[serde(rename = "textFormat")]
    TextFormat {
        marks: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        len: Option<f64>,
    },
    #[serde(rename = "textParagraph")]
    TextParagraph {
        attrs: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        len: Option<f64>,
    },
}

/// Apply body ops to a TextBody (JSON); returns `{ defaultRun, paragraphs }`. The input is not modified.
pub fn edit_body_value(body: &Value, ops: &[BodyOp]) -> Value {
    let ydoc = new_doc();
    let paragraphs = {
        let mut txn = ydoc.transact_mut();
        let frag = txn.get_or_insert_xml_fragment("body");
        write_body(&mut txn, &frag, body);
        for op in ops {
            match op {
                BodyOp::TextInsert { offset, text, marks } => insert_text(&mut txn, &frag, *offset as i64, text, marks.as_ref()),
                BodyOp::TextDelete { offset, len } => delete_text(&mut txn, &frag, *offset as i64, *len as i64),
                BodyOp::TextFormat { marks, offset, len } => match (offset, len) {
                    (Some(o), Some(l)) => format_text(&mut txn, &frag, *o as i64, *l as i64, marks),
                    _ => format_all(&mut txn, &frag, marks),
                },
                BodyOp::TextParagraph { attrs, offset, len } => {
                    set_paragraphs(&mut txn, &frag, attrs, offset.map(|o| (o as i64, len.unwrap_or(0.0) as i64)))
                }
            }
        }
        read_paragraphs(&txn, &frag)
    };
    let mut out = Map::new();
    if let Some(dr) = body.get("defaultRun") {
        out.insert("defaultRun".into(), dr.clone());
    }
    out.insert("paragraphs".into(), Value::Array(paragraphs));
    Value::Object(out)
}

/// Typed [`edit_body_value`]. `editBody`.
pub fn edit_body(body: &TextBody, ops: &[BodyOp]) -> Result<TextBody, CrdtError> {
    let v = oc_schema::js::to_value(body)?;
    Ok(serde_json::from_value(edit_body_value(&v, ops))?)
}
