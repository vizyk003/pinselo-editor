// SPDX-License-Identifier: Apache-2.0
//! Rich text on Y.XmlFragment: `<paragraph align lineHeight spaceAfter list>` → Y.XmlText with run marks.
//! The shape matches the y-prosemirror "oc-text-v1" schema (ADR-0009), identical to `@oc/crdt`'s `text.ts`, so
//! the browser (yjs, TipTap) and the server (yrs) edit the same structure. Editing uses flat offsets where
//! paragraphs are joined by `'\n'`; offsets and lengths are UTF-16 code units like JS strings (documents must
//! be created with [`crate::new_doc`], which sets `OffsetKind::Utf16`).

use oc_schema::js::{str_cmp, utf16_len, value_to_js_string};
use serde_json::{Map, Value};
use yrs::types::text::YChange;
use yrs::types::Attrs;
use yrs::branch::BranchPtr;
use yrs::{
    Any, Assoc, IndexedSequence, Out, ReadTxn, StickyIndex, Text, TransactionMut, Xml, XmlElementPrelim, XmlElementRef,
    XmlFragment, XmlFragmentRef, XmlOut, XmlTextPrelim, XmlTextRef,
};

use crate::value::{any_to_json, attrs_to_json, json_to_any, json_to_attrs, out_to_json};

/// Paragraph node name.
pub const PARA: &str = "paragraph";

/// `DEFAULT_PARAGRAPH` as JSON.
pub fn default_paragraph() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("align".into(), Value::from("start"));
    m.insert("lineHeight".into(), Value::from(1.3));
    m.insert("spaceAfter".into(), Value::from(0));
    m
}

/// A paragraph element and its text node.
#[derive(Debug, Clone)]
pub struct ParaRef {
    pub el: XmlElementRef,
    pub text: XmlTextRef,
}

/// One delta op of a text node (string inserts only; embeds are skipped like the TS filters do).
#[derive(Debug, Clone, PartialEq)]
pub struct DeltaOp {
    pub insert: String,
    pub attributes: Option<Attrs>,
}

/// `text.toDelta()` restricted to string inserts.
pub fn to_delta<T: ReadTxn>(txn: &T, text: &XmlTextRef) -> Vec<DeltaOp> {
    text.diff(txn, YChange::identity)
        .into_iter()
        .filter_map(|d| match d.insert {
            Out::Any(Any::String(s)) => Some(DeltaOp { insert: s.to_string(), attributes: d.attributes.map(|a| *a) }),
            _ => None,
        })
        .collect()
}

fn children<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef) -> Vec<XmlOut> {
    frag.children(txn).collect()
}

fn index_of<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef, el: &XmlElementRef) -> Option<u32> {
    children(txn, frag).iter().position(|c| matches!(c, XmlOut::Element(e) if e == el)).map(|i| i as u32)
}

/// Paragraph elements of a fragment with their text nodes; a paragraph without a text node gets one
/// (`paragraphsOf`, write path).
pub fn paragraphs_of(txn: &mut TransactionMut, frag: &XmlFragmentRef) -> Vec<ParaRef> {
    let mut out = Vec::new();
    for node in children(txn, frag) {
        let XmlOut::Element(el) = node else { continue };
        if el.tag().as_ref() != PARA {
            continue;
        }
        let text = match el.get(txn, 0) {
            Some(XmlOut::Text(t)) => t,
            _ => el.insert(txn, 0, XmlTextPrelim::new("")),
        };
        out.push(ParaRef { el, text });
    }
    out
}

/// Paragraph elements of a fragment (read path: a missing text node reads as empty, nothing is written).
fn paragraphs_read<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef) -> Vec<(XmlElementRef, Option<XmlTextRef>)> {
    let mut out = Vec::new();
    for node in children(txn, frag) {
        let XmlOut::Element(el) = node else { continue };
        if el.tag().as_ref() != PARA {
            continue;
        }
        let text = match el.get(txn, 0) {
            Some(XmlOut::Text(t)) => Some(t),
            _ => None,
        };
        out.push((el, text));
    }
    out
}

fn set_para_attrs(txn: &mut TransactionMut, el: &XmlElementRef, attrs: &Map<String, Value>) {
    for (k, v) in attrs {
        el.insert_attribute(txn, k.as_str(), json_to_any(v));
    }
}

/// `{ ...DEFAULT_PARAGRAPH, ...el.getAttributes() }`.
pub fn para_attrs<T: ReadTxn>(txn: &T, el: &XmlElementRef) -> Map<String, Value> {
    let mut m = default_paragraph();
    let mut attrs: Vec<(String, Out)> = el.attributes(txn).map(|(k, v)| (k.to_string(), v)).collect();
    attrs.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in attrs {
        if crate::value::is_undefined(&v) {
            continue;
        }
        m.insert(k, out_to_json(txn, &v));
    }
    m
}

fn insert_with(txn: &mut TransactionMut, text: &XmlTextRef, pos: u32, s: &str, attrs: Attrs) {
    if !s.is_empty() {
        text.insert_with_attributes(txn, pos, s, attrs);
    }
}

fn insert_paragraph(
    txn: &mut TransactionMut,
    frag: &XmlFragmentRef,
    index: u32,
    attrs: &Map<String, Value>,
    delta: &[DeltaOp],
) -> ParaRef {
    let el = frag.insert(txn, index, XmlElementPrelim::empty(PARA));
    let text = el.insert(txn, 0, XmlTextPrelim::new(""));
    set_para_attrs(txn, &el, attrs);
    let mut pos = 0u32;
    for op in delta {
        insert_with(txn, &text, pos, &op.insert, op.attributes.clone().unwrap_or_default());
        pos += utf16_len(&op.insert) as u32;
    }
    ParaRef { el, text }
}

/// Write a whole TextBody (JSON) into an (empty or existing) fragment.
pub fn write_body(txn: &mut TransactionMut, frag: &XmlFragmentRef, body: &Value) {
    let len = frag.len(txn);
    if len > 0 {
        frag.remove_range(txn, 0, len);
    }
    let mut paras: Vec<Map<String, Value>> = match body.get("paragraphs") {
        Some(Value::Array(a)) => a.iter().map(|p| crate::value::obj(Some(p))).collect(),
        _ => vec![],
    };
    if paras.is_empty() {
        let mut p = default_paragraph();
        p.insert("runs".into(), Value::Array(vec![]));
        paras.push(p);
    }
    for (i, p) in paras.iter().enumerate() {
        let mut attrs = p.clone();
        let runs = attrs.remove("runs");
        let delta: Vec<DeltaOp> = match runs {
            Some(Value::Array(rs)) => rs
                .iter()
                .filter_map(|r| {
                    let text = r.get("text")?;
                    if !crate::value::truthy(text) {
                        return None;
                    }
                    let style = match r.get("style") {
                        Some(Value::Object(m)) => json_to_attrs(m),
                        _ => Attrs::new(),
                    };
                    Some(DeltaOp { insert: value_to_js_string(text), attributes: Some(style) })
                })
                .collect(),
            _ => vec![],
        };
        insert_paragraph(txn, frag, i as u32, &attrs, &delta);
    }
}

fn runs_of(delta: &[DeltaOp]) -> Vec<Value> {
    delta
        .iter()
        .filter(|d| !d.insert.is_empty())
        .map(|d| {
            let mut r = Map::new();
            r.insert("text".into(), Value::String(d.insert.clone()));
            r.insert("style".into(), Value::Object(d.attributes.as_ref().map(attrs_to_json).unwrap_or_default()));
            Value::Object(r)
        })
        .collect()
}

/// The paragraphs of a fragment as TextBody JSON (`readParagraphs`).
pub fn read_paragraphs<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef) -> Vec<Value> {
    let paras: Vec<Value> = paragraphs_read(txn, frag)
        .into_iter()
        .map(|(el, text)| {
            let mut p = para_attrs(txn, &el);
            let runs = text.map(|t| runs_of(&to_delta(txn, &t))).unwrap_or_default();
            p.insert("runs".into(), Value::Array(runs));
            Value::Object(p)
        })
        .collect();
    if paras.is_empty() {
        let mut p = default_paragraph();
        p.insert("runs".into(), Value::Array(vec![]));
        return vec![Value::Object(p)];
    }
    paras
}

/// `XmlText#toString()` of yjs: the text with each formatted run wrapped in `<mark key="value">` tags.
fn xml_text_to_string<T: ReadTxn>(txn: &T, text: &XmlTextRef) -> String {
    let mut out = String::new();
    for d in to_delta(txn, text) {
        let mut nodes: Vec<(String, Vec<(String, String)>)> = Vec::new();
        if let Some(attrs) = &d.attributes {
            for (name, v) in attrs.iter() {
                let mut kv: Vec<(String, String)> = for_in(&any_to_json(v));
                kv.sort_by(|a, b| if str_cmp(&a.0, &b.0).is_lt() { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater });
                nodes.push((name.to_string(), kv));
            }
        }
        nodes.sort_by(|a, b| if str_cmp(&a.0, &b.0).is_lt() { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater });
        for (name, kv) in &nodes {
            out.push('<');
            out.push_str(name);
            for (k, v) in kv {
                out.push_str(&format!(" {k}=\"{v}\""));
            }
            out.push('>');
        }
        out.push_str(&d.insert);
        for (name, _) in nodes.iter().rev() {
            out.push_str(&format!("</{name}>"));
        }
    }
    out
}

/// JS `for (const key in v)` pairs `(key, String(v[key]))`.
fn for_in(v: &Value) -> Vec<(String, String)> {
    match v {
        Value::Object(m) => m.iter().map(|(k, x)| (k.clone(), value_to_js_string(x))).collect(),
        Value::Array(a) => a.iter().enumerate().map(|(i, x)| (i.to_string(), elem_string(x))).collect(),
        Value::String(s) => s
            .encode_utf16()
            .enumerate()
            .map(|(i, u)| (i.to_string(), String::from_utf16_lossy(&[u])))
            .collect(),
        _ => vec![],
    }
}

/// `String(x)` of an array element (`null`/`undefined` render empty inside `Array#join`).
fn elem_string(x: &Value) -> String {
    match x {
        Value::Null => String::new(),
        other => value_to_js_string(other),
    }
}

/// `plainText(frag)`: paragraphs joined by `'\n'`, each rendered with yjs `XmlText#toString()` (so formatted
/// runs carry their mark tags, exactly like the TS function).
pub fn plain_text<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef) -> String {
    paragraphs_read(txn, frag)
        .into_iter()
        .map(|(_, t)| t.map(|t| xml_text_to_string(txn, &t)).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The characters only (no mark tags), paragraphs joined by `'\n'`.
pub fn text_content<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef) -> String {
    paragraphs_read(txn, frag)
        .into_iter()
        .map(|(_, t)| t.map(|t| to_delta(txn, &t).into_iter().map(|d| d.insert).collect::<String>()).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

fn ensure_one(txn: &mut TransactionMut, frag: &XmlFragmentRef) -> Vec<ParaRef> {
    let ps = paragraphs_of(txn, frag);
    if !ps.is_empty() {
        return ps;
    }
    vec![insert_paragraph(txn, frag, 0, &default_paragraph(), &[])]
}

fn text_len<T: ReadTxn>(txn: &T, t: &XmlTextRef) -> i64 {
    t.len(txn) as i64
}

/// Map a flat offset to (paragraph index, offset within paragraph). Clamps to the end.
pub fn locate<T: ReadTxn>(txn: &T, paras: &[ParaRef], offset: i64) -> (usize, i64) {
    let mut rem = offset.max(0);
    for (i, p) in paras.iter().enumerate() {
        let len = text_len(txn, &p.text);
        if rem <= len {
            return (i, rem);
        }
        rem -= len + 1;
    }
    let last = paras.len() - 1;
    (last, text_len(txn, &paras[last].text))
}

/// `s.slice(from)` in UTF-16 units.
fn slice_utf16(s: &str, from: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    if from >= units.len() {
        return String::new();
    }
    String::from_utf16_lossy(&units[from..])
}

fn slice_delta<T: ReadTxn>(txn: &T, text: &XmlTextRef, from: i64) -> Vec<DeltaOp> {
    let mut out = Vec::new();
    let mut pos = 0i64;
    for d in to_delta(txn, text) {
        let len = utf16_len(&d.insert) as i64;
        if pos + len > from {
            let s = slice_utf16(&d.insert, (from - pos).max(0) as usize);
            out.push(DeltaOp { insert: s, attributes: d.attributes.clone() });
        }
        pos += len;
    }
    out
}

/// Marks in effect at an offset (the run before the caret), used to continue typing in the same style.
pub fn marks_at(txn: &mut TransactionMut, frag: &XmlFragmentRef, offset: i64) -> Attrs {
    let paras = ensure_one(txn, frag);
    let (p, o) = locate(txn, &paras, offset);
    let target = if o > 0 { o - 1 } else { 0 };
    let mut pos = 0i64;
    for d in to_delta(txn, &paras[p].text) {
        let len = utf16_len(&d.insert) as i64;
        if target < pos + len {
            return d.attributes.unwrap_or_default();
        }
        pos += len;
    }
    Attrs::new()
}

/// A flat text offset as a yrs sticky index (COL-02): anchored to the character before it (`Assoc::Before`), so
/// text inserted concurrently in front of it, or at the same spot by someone else, does not move it into the
/// middle of the local user's own typing. `None` when the body has no paragraphs.
pub fn sticky_offset<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef, offset: i64) -> Option<StickyIndex> {
    let paras = paragraphs_read(txn, frag);
    let n = paras.len();
    let mut rem = offset.max(0);
    for (i, (_, t)) in paras.iter().enumerate() {
        let len = t.as_ref().map_or(0, |t| text_len(txn, t));
        if rem <= len || i + 1 == n {
            return t.as_ref()?.sticky_index(txn, rem.min(len) as u32, Assoc::Before);
        }
        rem -= len + 1;
    }
    None
}

/// The flat offset a [`sticky_offset`] points at now (`None` when its paragraph is gone, e.g. joined remotely).
pub fn resolve_sticky<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef, s: &StickyIndex) -> Option<i64> {
    let off = s.get_offset(txn)?;
    let mut base = 0i64;
    for (_, t) in paragraphs_read(txn, frag) {
        let Some(t) = t else {
            base += 1;
            continue;
        };
        let len = text_len(txn, &t);
        if BranchPtr::from(AsRef::<yrs::branch::Branch>::as_ref(&t)) == off.branch {
            return Some(base + (off.index as i64).min(len));
        }
        base += len + 1;
    }
    None
}

/// Insert `s` (may contain `'\n'`, which splits paragraphs) at a flat offset. `attrs` = explicit marks, or
/// `None` to continue the marks at the caret.
pub fn insert_text(txn: &mut TransactionMut, frag: &XmlFragmentRef, offset: i64, s: &str, attrs: Option<&Map<String, Value>>) {
    let mut paras = ensure_one(txn, frag);
    let (mut p, mut o) = locate(txn, &paras, offset);
    let marks = match attrs {
        Some(m) => json_to_attrs(m),
        None => marks_at(txn, frag, offset),
    };
    for (i, part) in s.split('\n').enumerate() {
        if i > 0 {
            // Split paragraph p at o: the tail moves into a new paragraph after it, with the same attributes.
            let cur = paras[p].clone();
            let tail = slice_delta(txn, &cur.text, o);
            let tail_len = text_len(txn, &cur.text) - o;
            if tail_len > 0 {
                cur.text.remove_range(txn, o as u32, tail_len as u32);
            }
            let attrs = para_attrs(txn, &cur.el);
            let idx = index_of(txn, frag, &cur.el).map(|i| i + 1).unwrap_or(0);
            insert_paragraph(txn, frag, idx, &attrs, &tail);
            paras = paragraphs_of(txn, frag);
            p += 1;
            o = 0;
        }
        if !part.is_empty() {
            insert_with(txn, &paras[p].text, o as u32, part, marks.clone());
            o += utf16_len(part) as i64;
        }
    }
}

/// Delete `len` UTF-16 units at a flat offset (joining paragraphs when the range spans them).
/// True when `o` falls between the two halves of a surrogate pair in `t`.
fn splits_pair<T: ReadTxn>(txn: &T, t: &XmlTextRef, o: i64) -> bool {
    if o <= 0 || o >= text_len(txn, t) {
        return false;
    }
    let u: Vec<u16> = to_delta(txn, t).iter().flat_map(|d| d.insert.encode_utf16().collect::<Vec<_>>()).collect();
    let (hi, lo) = (u.get(o as usize - 1).copied().unwrap_or(0), u.get(o as usize).copied().unwrap_or(0));
    (0xd800..=0xdbff).contains(&hi) && (0xdc00..=0xdfff).contains(&lo)
}

pub fn delete_text(txn: &mut TransactionMut, frag: &XmlFragmentRef, offset: i64, len: i64) {
    if len <= 0 {
        return;
    }
    let paras = ensure_one(txn, frag);
    let mut a = locate(txn, &paras, offset);
    let mut b = locate(txn, &paras, offset + len);
    // Never delete half a surrogate pair (yrs refuses and panics): widen the range to whole code points.
    if splits_pair(txn, &paras[a.0].text, a.1) {
        a.1 -= 1;
    }
    if splits_pair(txn, &paras[b.0].text, b.1) {
        b.1 += 1;
    }
    if a.0 == b.0 {
        if b.1 > a.1 {
            paras[a.0].text.remove_range(txn, a.1 as u32, (b.1 - a.1) as u32);
        }
        return;
    }
    let first = paras[a.0].clone();
    let last = paras[b.0].clone();
    let rest = slice_delta(txn, &last.text, b.1);
    let first_len = text_len(txn, &first.text);
    if first_len - a.1 > 0 {
        first.text.remove_range(txn, a.1 as u32, (first_len - a.1) as u32);
    }
    let start = index_of(txn, frag, &first.el).map(|i| i + 1).unwrap_or(0);
    let end = index_of(txn, frag, &last.el).unwrap_or(0);
    if end + 1 > start {
        frag.remove_range(txn, start, end - start + 1);
    }
    let mut pos = a.1 as u32;
    for d in rest {
        insert_with(txn, &first.text, pos, &d.insert, d.attributes.clone().unwrap_or_default());
        pos += utf16_len(&d.insert) as u32;
    }
}

/// Apply marks to a flat range. A `null` value removes that mark.
pub fn format_text(txn: &mut TransactionMut, frag: &XmlFragmentRef, offset: i64, len: i64, attrs: &Map<String, Value>) {
    let paras = ensure_one(txn, frag);
    let a = locate(txn, &paras, offset);
    let b = locate(txn, &paras, offset + len);
    let marks = json_to_attrs(attrs);
    for (i, para) in paras.iter().enumerate().take(b.0 + 1).skip(a.0) {
        let t = &para.text;
        let from = if i == a.0 { a.1 } else { 0 };
        let to = if i == b.0 { b.1 } else { text_len(txn, t) };
        if to > from {
            t.format(txn, from as u32, (to - from) as u32, marks.clone());
        }
    }
}

/// Apply marks to every run in the body (whole-element styling from the toolbar).
pub fn format_all(txn: &mut TransactionMut, frag: &XmlFragmentRef, attrs: &Map<String, Value>) {
    let marks = json_to_attrs(attrs);
    for p in ensure_one(txn, frag) {
        let len = p.text.len(txn);
        if len > 0 {
            p.text.format(txn, 0, len, marks.clone());
        }
    }
}

/// Set paragraph attributes on every paragraph touched by the range (or all when the range is omitted).
pub fn set_paragraphs(txn: &mut TransactionMut, frag: &XmlFragmentRef, attrs: &Map<String, Value>, range: Option<(i64, i64)>) {
    let paras = ensure_one(txn, frag);
    let (mut from, mut to) = (0usize, paras.len() - 1);
    if let Some((offset, len)) = range {
        from = locate(txn, &paras, offset).0;
        to = locate(txn, &paras, offset + len).0;
    }
    for p in paras.iter().take(to + 1).skip(from) {
        set_para_attrs(txn, &p.el, attrs);
    }
}
