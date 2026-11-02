//! Port of services/api/src/mcp/edit.ts — edit operations for AI assistants (MCP `edit_design`). Assistants describe
//! changes in page-space px with hex colours and font names; each operation compiles to DocOps applied with
//! `oc_crdt::apply_ops` (schema checks, locks) — the same write path as every other editor. Operations run in
//! order; an `add_*` operation can carry a `ref` that later operations of the same batch use as `$ref` in place of
//! an id. Processing stops at the first operation that fails (earlier ones stay applied).
//!
//! The operations arrive validated (see [`super::schema`]) as JSON objects. TS applies the whole batch inside one
//! `ydoc.transact`; yrs transactions do not nest, so here every operation is its own transaction (the fallback path
//! merges the updates into one op-log row; live collaborators may receive several updates for one batch).

use std::collections::HashMap;

use oc_crdt::{ApplyOptions, OcOrigin};
use oc_geometry::align::{AlignEdge, AlignItem, align_deltas, distribute_deltas};
use oc_geometry::snap::Axis;
use oc_schema::primitives::Rect;
use serde_json::{Map, Value, json};
use yrs::{Doc, Transact};

use super::layout::{Align, ArrangeOptions, Direction, Justify, LBox, arrange};
use super::view::{self, num, page_pos, truthy};
use crate::js;

pub struct EditContext<'a> {
    pub ydoc: &'a Doc,
    pub actor: String,
    /// Images imported before the run, by operation index (AssetRef JSON).
    pub images: HashMap<usize, Value>,
    pub now: f64,
}

#[derive(Debug, Default)]
pub struct EditResult {
    pub applied: usize,
    pub total: usize,
    /// ref → id for everything created with a `ref`.
    pub refs: Map<String, Value>,
    /// Ids created by each applied operation.
    pub created: Vec<Vec<String>>,
    /// (index, op, message)
    pub error: Option<(usize, String, String)>,
    pub title: Option<String>,
    /// Elements added or changed (shown as the assistant's selection to collaborators).
    pub touched: Vec<String>,
    /// Page the batch ended on.
    pub page_id: Option<String>,
    /// Matches changed by `replace_text` operations.
    pub replaced: usize,
}

struct EditError(String);

fn err<T>(m: impl Into<String>) -> Result<T, EditError> {
    Err(EditError(m.into()))
}

/// `hex(s)` → ColorValue JSON.
pub fn hex(s: &str) -> Value {
    oc_schema::color::hex(s).ok().and_then(|c| serde_json::to_value(c).ok()).unwrap_or(Value::Null)
}

pub fn solid(color: Value) -> Value {
    json!({ "k": "solid", "color": color })
}

/// `paintIn(fill)`: a hex colour or `{ gradient, angle?, radial? }`.
pub fn paint_in(f: &Value) -> Value {
    if let Some(s) = f.as_str() {
        return solid(hex(s));
    }
    let g = f["gradient"].as_array().cloned().unwrap_or_default();
    let n = g.len();
    let stops: Vec<Value> = g.iter().enumerate().map(|(i, c)| json!({ "t": jnum(i as f64 / (n as f64 - 1.0)), "color": hex(c.as_str().unwrap_or("")) })).collect();
    if f.get("radial").is_some_and(|r| r == true) {
        json!({ "k": "radial", "stops": stops })
    } else {
        json!({ "k": "linear", "stops": stops, "angle": f.get("angle").cloned().unwrap_or(json!(0)) })
    }
}

/// A JS number as JSON (integral values without a fraction).
pub fn jnum(f: f64) -> Value {
    crate::http::jsjson::number_value(f).unwrap_or(Value::Null)
}

fn stroke_of(s: &Value) -> Value {
    let mut m = Map::new();
    m.insert("paint".into(), solid(hex(s["color"].as_str().unwrap_or(""))));
    m.insert("width".into(), s["width"].clone());
    m.insert("align".into(), json!("center"));
    m.insert("cap".into(), json!("round"));
    m.insert("join".into(), json!("round"));
    if s["dash"].as_array().is_some_and(|d| !d.is_empty()) {
        m.insert("dash".into(), s["dash"].clone());
    }
    Value::Object(m)
}

fn align_of(a: &str) -> &'static str {
    match a {
        "left" => "start",
        "center" => "center",
        "right" => "end",
        _ => "justify",
    }
}

fn s<'a>(op: &'a Value, k: &str) -> Option<&'a str> {
    op.get(k).and_then(Value::as_str)
}

fn f(op: &Value, k: &str) -> Option<f64> {
    op.get(k).and_then(Value::as_f64)
}

/// `runStyle(o)`.
fn run_style(o: &Value) -> Map<String, Value> {
    let mut m = Map::new();
    if let Some(v) = s(o, "font").filter(|v| !v.is_empty()) {
        m.insert("family".into(), json!(v));
    }
    if let Some(v) = f(o, "size").filter(|v| *v != 0.0) {
        m.insert("size".into(), jnum(v));
    }
    if let Some(v) = f(o, "weight").filter(|v| *v != 0.0) {
        m.insert("weight".into(), jnum(v));
    }
    if let Some(v) = o.get("italic") {
        m.insert("italic".into(), v.clone());
    }
    if let Some(v) = s(o, "color").filter(|v| !v.is_empty()) {
        m.insert("fill".into(), solid(hex(v)));
    }
    if let Some(v) = o.get("letterSpacing") {
        m.insert("tracking".into(), v.clone());
    }
    if let Some(v) = s(o, "case").filter(|v| !v.is_empty()) {
        m.insert("caseTransform".into(), json!(v));
    }
    m
}

/// `textBody(text, style, align)` as JSON.
fn text_body(text: &str, style: &Map<String, Value>, align: &str) -> Value {
    let mut d = json!({ "family": "Inter", "weight": 400, "italic": false, "size": 32, "fill": solid(hex("#1f2328")), "tracking": 0 });
    for (k, v) in style {
        d[k] = v.clone();
    }
    let paragraphs: Vec<Value> = text
        .split('\n')
        .map(|line| json!({ "align": align, "lineHeight": 1.3, "spaceAfter": 0, "runs": if line.is_empty() { json!([]) } else { json!([{ "text": line, "style": {} }]) } }))
        .collect();
    json!({ "defaultRun": d, "paragraphs": paragraphs })
}

/// Rough height of auto-height text before the editor lays it out (the editor and renderer reflow it).
pub fn estimate_text_height(text: &str, w: f64, size: f64, line_height: f64) -> f64 {
    let per_line = (w / (size * 0.52)).floor().max(1.0);
    let lines = text.split('\n').fold(0.0, |n, p| n + (js::len(p) as f64 / per_line).ceil().max(1.0));
    (lines * size * line_height).ceil()
}

/// New text body: optionally new text, with style changes applied to the defaults and cleared from runs.
fn restyle(body: &Value, text: Option<&str>, style: &Map<String, Value>, align: Option<&str>, line_height: Option<&Value>) -> Value {
    let p0 = body["paragraphs"].as_array().and_then(|a| a.first()).cloned();
    let mut paragraphs: Vec<Value> = match text {
        Some(t) => {
            let a = p0.as_ref().and_then(|p| p.get("align")).and_then(Value::as_str).unwrap_or("start").to_owned();
            text_body(t, &Map::new(), &a)["paragraphs"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|mut p| {
                    if let Some(lh) = p0.as_ref().and_then(|p| p.get("lineHeight")).filter(|v| !v.is_null()) {
                        p["lineHeight"] = lh.clone();
                    }
                    p["spaceAfter"] = p0.as_ref().and_then(|p| p.get("spaceAfter")).filter(|v| !v.is_null()).cloned().unwrap_or(json!(0));
                    if let Some(l) = p0.as_ref().and_then(|p| p.get("list")).filter(|l| truthy(Some(l))) {
                        p["list"] = l.clone();
                    }
                    p
                })
                .collect()
        }
        None => body["paragraphs"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut p| {
                if let Some(runs) = p.get_mut("runs").and_then(Value::as_array_mut) {
                    for r in runs {
                        if let Some(st) = r.get_mut("style").and_then(Value::as_object_mut) {
                            st.retain(|k, _| !style.contains_key(k));
                        }
                    }
                }
                p
            })
            .collect(),
    };
    for p in &mut paragraphs {
        if let Some(a) = align {
            p["align"] = json!(align_of(a));
        }
        if let Some(lh) = line_height {
            p["lineHeight"] = lh.clone();
        }
    }
    let mut d = body["defaultRun"].clone();
    if !d.is_object() {
        d = json!({});
    }
    for (k, v) in style {
        d[k] = v.clone();
    }
    json!({ "defaultRun": d, "paragraphs": paragraphs })
}

struct Run<'a> {
    ctx: &'a EditContext<'a>,
    res: EditResult,
    snap: Option<(Value, oc_schema::design::DesignDoc)>,
    last_page: Option<String>,
}

type Compiled = (Vec<Value>, Option<String>, Option<String>);

impl Run<'_> {
    fn doc(&mut self) -> &(Value, oc_schema::design::DesignDoc) {
        if self.snap.is_none() {
            let json = oc_crdt::from_y_value(self.ctx.ydoc);
            let typed = serde_json::from_value(json.clone()).ok().or_else(|| oc_crdt::from_y(self.ctx.ydoc).ok()).unwrap_or_else(empty_doc);
            self.snap = Some((json, typed));
        }
        self.snap.as_ref().expect("snapshot")
    }

    fn sorted_pages(&mut self) -> Vec<String> {
        view::sorted_page_ids(&self.doc().1)
    }

    /// `id(v)`: an element id or `$ref` of a live element.
    fn id(&self, v: &str) -> Result<String, EditError> {
        let real = match v.strip_prefix('$') {
            Some(r) => self.res.refs.get(r).and_then(Value::as_str).map(str::to_owned),
            None => Some(v.to_owned()),
        };
        let Some(real) = real else {
            return err(format!("Unknown reference {v}. Refs must be defined by an earlier operation in the same batch."));
        };
        let r = oc_crdt::roots(self.ctx.ydoc);
        let txn = self.ctx.ydoc.transact();
        let live = oc_crdt::codec::element_map(&txn, &r, &real).is_some_and(|m| !oc_crdt::codec::is_deleted(&txn, &m));
        if !live {
            let extra = if real != v { format!(" ({real})") } else { String::new() };
            return err(format!("No element {v}{extra}. Call get_design for current ids."));
        }
        Ok(real)
    }

    /// `el(v)`: the element as stored now.
    fn el(&self, v: &str) -> Result<Value, EditError> {
        let id = self.id(v)?;
        let r = oc_crdt::roots(self.ctx.ydoc);
        let txn = self.ctx.ydoc.transact();
        let m = oc_crdt::codec::element_map(&txn, &r, &id).expect("live element");
        Ok(Value::Object(oc_crdt::read_element(&txn, &m)))
    }

    fn page(&mut self, v: Option<&Value>) -> Result<String, EditError> {
        match v {
            None | Some(Value::Null) => {
                let p = match &self.last_page {
                    Some(p) => Some(p.clone()),
                    None => self.sorted_pages().first().cloned(),
                };
                p.ok_or(EditError("This design has no pages.".into()))
            }
            Some(Value::Number(n)) => {
                let pages = self.sorted_pages();
                let i = n.as_f64().unwrap_or(0.0) as usize;
                match pages.get(i.wrapping_sub(1)) {
                    Some(p) => Ok(p.clone()),
                    None => err(format!("There is no page {}; the design has {}.", js::num(n.as_f64().unwrap_or(0.0)), pages.len())),
                }
            }
            Some(Value::String(v)) => {
                let real = match v.strip_prefix('$') {
                    Some(r) => self.res.refs.get(r).and_then(Value::as_str).map(str::to_owned),
                    None => Some(v.clone()),
                };
                let exists = real.as_ref().is_some_and(|real| {
                    let r = oc_crdt::roots(self.ctx.ydoc);
                    let txn = self.ctx.ydoc.transact();
                    oc_crdt::codec::page_map(&txn, &r, real).is_some()
                });
                match real {
                    Some(real) if exists => Ok(real),
                    _ => err(format!("No page {v}.")),
                }
            }
            Some(_) => err("No page."),
        }
    }

    /// Parent and page-space → parent-space offset for a new element.
    fn parent_of(&mut self, op: &Value) -> Result<(String, f64, f64), EditError> {
        match s(op, "parent") {
            None => {
                let p = self.page(op.get("page"))?;
                self.last_page = Some(p.clone());
                Ok((p, 0.0, 0.0))
            }
            Some(parent) => {
                let g = self.el(parent)?;
                if g["type"] != "group" {
                    return err(format!("{parent} is a {}, not a group.", g["type"].as_str().unwrap_or("")));
                }
                let id = g["id"].as_str().unwrap_or("").to_owned();
                let doc = &self.doc().0;
                let Some((x, y)) = page_pos(doc, &g) else {
                    return err("Adding into a rotated group is not supported; add to the page and group afterwards.");
                };
                Ok((id, -x, -y))
            }
        }
    }

    /// Page-space box of an existing element.
    fn box_of(&mut self, e: &Value) -> (f64, f64, f64, f64) {
        let id = e["id"].as_str().unwrap_or("").to_owned();
        let (json, typed) = self.doc();
        match page_pos(json, e) {
            Some((x, y)) => (x, y, num(&e["xf"]["w"]), num(&e["xf"]["h"])),
            None => view::world_bounds(typed, &id),
        }
    }

    fn image_asset(&self, i: usize) -> Result<Value, EditError> {
        self.ctx.images.get(&i).cloned().ok_or(EditError("The image could not be imported.".into()))
    }

    fn compile(&mut self, op: &Value, i: usize) -> Result<Compiled, EditError> {
        let kind = s(op, "op").unwrap_or("");
        let r#ref = s(op, "ref").map(str::to_owned);
        match kind {
            "add_text" => {
                let (parent, dx, dy) = self.parent_of(op)?;
                let style = run_style(op);
                let lh = f(op, "lineHeight").unwrap_or(1.3);
                let text = s(op, "text").unwrap_or("");
                let w = f(op, "w").unwrap_or(0.0);
                let h = f(op, "h").unwrap_or_else(|| estimate_text_height(text, w, style.get("size").and_then(Value::as_f64).unwrap_or(32.0), lh));
                let mut t = base("text", op, f(op, "x").unwrap_or(0.0) + dx, f(op, "y").unwrap_or(0.0) + dy, w, h, &parent);
                let mut body = text_body(text, &style, s(op, "align").map_or("start", align_of));
                if (op.get("lineHeight").is_some() || op.get("list").is_some())
                    && let Some(ps) = body["paragraphs"].as_array_mut()
                {
                    {
                        for p in ps {
                            p["lineHeight"] = jnum(lh);
                            if let Some(l) = s(op, "list") {
                                p["list"] = json!({ "kind": l, "level": 0 });
                            }
                        }
                    }
                }
                t.insert("body".into(), body);
                t.insert("frame".into(), if op.get("h").is_some() { json!({ "mode": "fixed" }) } else { json!({ "mode": "autoHeight" }) });
                t.insert("autoFit".into(), json!("none"));
                let id = t["id"].as_str().unwrap_or("").to_owned();
                Ok((vec![json!({ "op": "add", "el": t, "parent": parent })], r#ref, Some(id)))
            }
            "add_shape" => {
                let (parent, dx, dy) = self.parent_of(op)?;
                let mut e = base("shape", op, f(op, "x").unwrap_or(0.0) + dx, f(op, "y").unwrap_or(0.0) + dy, f(op, "w").unwrap_or(0.0), f(op, "h").unwrap_or(0.0), &parent);
                e.insert("kind".into(), op["shape"].clone());
                e.insert("params".into(), match f(op, "radius").filter(|r| *r != 0.0) {
                    Some(r) => json!({ "cornerRadius": jnum(r) }),
                    None => json!({}),
                });
                e.insert("fills".into(), json!([op.get("fill").map_or_else(|| solid(hex("#8b5cf6")), paint_in)]));
                e.insert("strokes".into(), op.get("stroke").map_or(json!([]), |st| json!([stroke_of(st)])));
                let id = e["id"].as_str().unwrap_or("").to_owned();
                Ok((vec![json!({ "op": "add", "el": e, "parent": parent })], r#ref, Some(id)))
            }
            "add_line" => {
                let (parent, dx, dy) = self.parent_of(op)?;
                let (x1, y1, x2, y2) = (f(op, "x1").unwrap_or(0.0), f(op, "y1").unwrap_or(0.0), f(op, "x2").unwrap_or(0.0), f(op, "y2").unwrap_or(0.0));
                let (x, y) = (x1.min(x2), y1.min(y2));
                let mut e = base("line", op, x + dx, y + dy, (x2 - x1).abs().max(1.0), (y2 - y1).abs().max(1.0), &parent);
                e.insert("points".into(), json!([[jnum(x1 - x), jnum(y1 - y)], [jnum(x2 - x), jnum(y2 - y)]]));
                let mut st = Map::new();
                st.insert("paint".into(), solid(hex(s(op, "color").unwrap_or("#1f2328"))));
                st.insert("width".into(), op.get("width").cloned().unwrap_or(json!(4)));
                st.insert("align".into(), json!("center"));
                st.insert("cap".into(), json!("round"));
                st.insert("join".into(), json!("round"));
                if let Some(d) = op.get("dash") {
                    st.insert("dash".into(), d.clone());
                }
                e.insert("stroke".into(), Value::Object(st));
                e.insert("startMarker".into(), op.get("start").cloned().unwrap_or(json!("none")));
                e.insert("endMarker".into(), op.get("end").cloned().unwrap_or(json!("none")));
                let id = e["id"].as_str().unwrap_or("").to_owned();
                Ok((vec![json!({ "op": "add", "el": e, "parent": parent })], r#ref, Some(id)))
            }
            "add_image" => {
                let (parent, dx, dy) = self.parent_of(op)?;
                let a = self.image_asset(i)?;
                let (mut x, mut y, mut w, mut h) = (f(op, "x").unwrap_or(0.0), f(op, "y").unwrap_or(0.0), f(op, "w").unwrap_or(0.0), f(op, "h").unwrap_or(0.0));
                let mut e = base("image", op, 0.0, 0.0, w, h, &parent);
                e.insert("asset".into(), a["id"].clone());
                e.insert("crop".into(), json!({ "x": 0, "y": 0, "w": 1, "h": 1 }));
                let (aw, ah) = (num(&a["w"]), num(&a["h"]));
                let has = aw != 0.0 && !aw.is_nan() && ah != 0.0 && !ah.is_nan();
                if s(op, "fit") == Some("contain") && has {
                    let k = (w / aw).min(h / ah);
                    x += (w - aw * k) / 2.0;
                    y += (h - ah * k) / 2.0;
                    w = aw * k;
                    h = ah * k;
                } else if has {
                    e.insert("crop".into(), crate::routes::v1::cover_crop_for(w / h, aw / ah));
                }
                e["xf"]["x"] = jnum(x + dx);
                e["xf"]["y"] = jnum(y + dy);
                e["xf"]["w"] = jnum(w);
                e["xf"]["h"] = jnum(h);
                if let Some(r) = f(op, "radius").filter(|r| *r != 0.0) {
                    e.insert("cornerRadius".into(), jnum(r));
                }
                if let Some(alt) = s(op, "alt").filter(|a| !a.is_empty()) {
                    e.insert("altText".into(), json!(alt));
                }
                let id = e["id"].as_str().unwrap_or("").to_owned();
                Ok((vec![json!({ "op": "asset", "asset": a }), json!({ "op": "add", "el": e, "parent": parent })], r#ref, Some(id)))
            }
            "add_table" => {
                let (parent, dx, dy) = self.parent_of(op)?;
                let data: Vec<Vec<String>> = op["data"].as_array().map(|rows| rows.iter().map(|r| r.as_array().map(|c| c.iter().map(|x| x.as_str().unwrap_or("").to_owned()).collect()).unwrap_or_default()).collect()).unwrap_or_default();
                let size = f(op, "size").unwrap_or(18.0);
                let h = f(op, "h").unwrap_or(data.len() as f64 * (size * 1.3 + 20.0));
                let mut e = base("table", op, f(op, "x").unwrap_or(0.0) + dx, f(op, "y").unwrap_or(0.0) + dy, f(op, "w").unwrap_or(0.0), h, &parent);
                let r = data.len();
                let c = data.first().map_or(3, Vec::len);
                let mut cells = Map::new();
                for (ri, row) in data.iter().enumerate() {
                    for ci in 0..c {
                        if let Some(t) = row.get(ci).filter(|t| !t.is_empty()) {
                            let mut cell = Map::new();
                            cell.insert("text".into(), json!(t));
                            if ri == 0 {
                                cell.insert("bold".into(), json!(true));
                            }
                            cells.insert(format!("{ri}:{ci}"), Value::Object(cell));
                        }
                    }
                }
                e.insert("rows".into(), Value::Array(vec![jnum(1.0 / r as f64); r]));
                e.insert("cols".into(), Value::Array(vec![jnum(1.0 / c as f64); c]));
                e.insert("cells".into(), Value::Object(cells));
                let mut style = json!({ "family": "Inter", "size": 18, "color": hex("#1f2328"), "borderColor": hex("#d0d7de"), "borderWidth": 1.5, "headerFill": solid(hex("#ede9fe")), "headerColor": hex("#4c1d95"), "padding": 10 });
                if let Some(v) = s(op, "font").filter(|v| !v.is_empty()) {
                    style["family"] = json!(v);
                }
                style["size"] = jnum(size);
                if let Some(v) = s(op, "color").filter(|v| !v.is_empty()) {
                    style["color"] = hex(v);
                }
                if let Some(v) = s(op, "headerFill").filter(|v| !v.is_empty()) {
                    style["headerFill"] = solid(hex(v));
                }
                if let Some(v) = s(op, "headerColor").filter(|v| !v.is_empty()) {
                    style["headerColor"] = hex(v);
                }
                e.insert("style".into(), style);
                let id = e["id"].as_str().unwrap_or("").to_owned();
                Ok((vec![json!({ "op": "add", "el": e, "parent": parent })], r#ref, Some(id)))
            }
            "add_chart" => {
                let (parent, dx, dy) = self.parent_of(op)?;
                let mut e = base("chart", op, f(op, "x").unwrap_or(0.0) + dx, f(op, "y").unwrap_or(0.0) + dy, f(op, "w").unwrap_or(0.0), f(op, "h").unwrap_or(0.0), &parent);
                e.insert("kind".into(), op["chart"].clone());
                e.insert("labels".into(), op["labels"].clone());
                let series: Vec<Value> = op["series"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .enumerate()
                            .map(|(i, x)| {
                                let c = s(x, "color").unwrap_or(oc_schema::factory::CHART_COLORS[i % oc_schema::factory::CHART_COLORS.len()]);
                                json!({ "name": x["name"], "values": x["values"], "color": hex(c) })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                e.insert("series".into(), Value::Array(series));
                let mut style = json!({ "family": "Inter", "textColor": hex("#374151"), "gridColor": hex("#e5e7eb"), "showLegend": true, "showValues": false, "showGrid": true, "fontSize": 16 });
                if let Some(v) = op.get("showLegend") {
                    style["showLegend"] = v.clone();
                }
                if let Some(v) = op.get("showValues") {
                    style["showValues"] = v.clone();
                }
                if let Some(v) = s(op, "font").filter(|v| !v.is_empty()) {
                    style["family"] = json!(v);
                }
                if let Some(v) = s(op, "textColor").filter(|v| !v.is_empty()) {
                    style["textColor"] = hex(v);
                }
                e.insert("style".into(), style);
                let id = e["id"].as_str().unwrap_or("").to_owned();
                Ok((vec![json!({ "op": "add", "el": e, "parent": parent })], r#ref, Some(id)))
            }
            "add_qr" => {
                let (parent, dx, dy) = self.parent_of(op)?;
                let size = f(op, "size").unwrap_or(0.0);
                let mut e = base("qr", op, f(op, "x").unwrap_or(0.0) + dx, f(op, "y").unwrap_or(0.0) + dy, size, size, &parent);
                e.insert("data".into(), op["data"].clone());
                e.insert("ecc".into(), json!("M"));
                e.insert("fg".into(), hex(s(op, "color").filter(|c| !c.is_empty()).unwrap_or("#000000")));
                e.insert("bg".into(), hex(s(op, "background").filter(|c| !c.is_empty()).unwrap_or("#ffffff")));
                let id = e["id"].as_str().unwrap_or("").to_owned();
                Ok((vec![json!({ "op": "add", "el": e, "parent": parent })], r#ref, Some(id)))
            }
            "group" => {
                let ids: Vec<String> = op["ids"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
                let items = ids.iter().map(|v| self.el(v)).collect::<Result<Vec<_>, _>>()?;
                let parent = items[0]["parentId"].as_str().unwrap_or("").to_owned();
                if items.iter().any(|e| e["parentId"].as_str() != Some(parent.as_str())) {
                    return err("Grouped elements must share the same parent (page or group).");
                }
                let boxes: Vec<_> = items.iter().map(|e| self.box_of(e)).collect();
                let gx = boxes.iter().map(|b| b.0).fold(f64::INFINITY, f64::min);
                let gy = boxes.iter().map(|b| b.1).fold(f64::INFINITY, f64::min);
                let gw = boxes.iter().map(|b| b.0 + b.2).fold(f64::NEG_INFINITY, f64::max) - gx;
                let gh = boxes.iter().map(|b| b.1 + b.3).fold(f64::NEG_INFINITY, f64::max) - gy;
                let real: Vec<String> = ids.iter().map(|v| self.id(v)).collect::<Result<_, _>>()?;
                let (json, typed) = self.doc();
                let ppos = json["elements"].get(&parent).and_then(|pe| page_pos(json, pe)).unwrap_or((0.0, 0.0));
                let sorted: Vec<String> = view::children(typed, &parent).into_iter().filter(|c| real.contains(c)).collect();
                let mut g = Map::new();
                let gid = oc_schema::ids::new_id();
                g.insert("id".into(), json!(gid));
                g.insert("type".into(), json!("group"));
                g.insert("parentId".into(), json!(parent));
                g.insert("z".into(), json!("a0"));
                g.insert("opacity".into(), json!(1));
                g.insert("xf".into(), json!({ "x": jnum(gx - ppos.0), "y": jnum(gy - ppos.1), "w": jnum(gw), "h": jnum(gh), "rot": 0 }));
                if let Some(n) = s(op, "name").filter(|n| !n.is_empty()) {
                    g.insert("name".into(), json!(n));
                }
                let mut ops = vec![json!({ "op": "add", "el": g, "parent": parent, "after": sorted.last() })];
                let mut after = "bottom".to_owned();
                for cid in &sorted {
                    let c = &json["elements"][cid.as_str()];
                    let mut xf = c["xf"].clone();
                    xf["x"] = jnum(num(&c["xf"]["x"]) - (gx - ppos.0));
                    xf["y"] = jnum(num(&c["xf"]["y"]) - (gy - ppos.1));
                    ops.push(json!({ "op": "reparent", "id": cid, "parent": gid, "after": after, "xf": xf }));
                    after = cid.clone();
                }
                Ok((ops, r#ref, Some(gid)))
            }
            "ungroup" => {
                let v = s(op, "id").unwrap_or("");
                let g = self.el(v)?;
                if g["type"] != "group" {
                    return err(format!("{v} is not a group."));
                }
                if truthy(g["xf"].get("rot")) {
                    return err("Ungrouping a rotated group is not supported.");
                }
                let gid = g["id"].as_str().unwrap_or("").to_owned();
                let (json, typed) = self.doc();
                let mut ops = Vec::new();
                let mut after = gid.clone();
                for cid in view::children(typed, &gid) {
                    let c = &json["elements"][cid.as_str()];
                    let mut xf = c["xf"].clone();
                    xf["x"] = jnum(num(&c["xf"]["x"]) + num(&g["xf"]["x"]));
                    xf["y"] = jnum(num(&c["xf"]["y"]) + num(&g["xf"]["y"]));
                    ops.push(json!({ "op": "reparent", "id": cid, "parent": g["parentId"], "after": after, "xf": xf }));
                    after = cid;
                }
                ops.push(json!({ "op": "delete", "id": gid }));
                Ok((ops, None, None))
            }
            "update" => Ok((self.compile_update(op, i)?, None, None)),
            "delete" => {
                let ids: Vec<&str> = op["ids"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                let ops = ids.iter().map(|v| self.id(v).map(|id| json!({ "op": "delete", "id": id }))).collect::<Result<_, _>>()?;
                Ok((ops, None, None))
            }
            "reorder" => {
                let e = self.el(s(op, "id").unwrap_or(""))?;
                let id = e["id"].as_str().unwrap_or("").to_owned();
                let sib = view::children(&self.doc().1, e["parentId"].as_str().unwrap_or(""));
                let at = sib.iter().position(|x| *x == id).map_or(-1, |p| p as i64);
                let after = match s(op, "to").unwrap_or("") {
                    "front" => "top".to_owned(),
                    "back" => "bottom".to_owned(),
                    "forward" => sib.get((at + 1) as usize).cloned().unwrap_or_else(|| "top".into()),
                    _ => if at >= 2 { sib[(at - 2) as usize].clone() } else { "bottom".into() },
                };
                Ok((vec![json!({ "op": "reorder", "id": id, "after": after })], None, None))
            }
            "arrange" => {
                let ids: Vec<&str> = op["ids"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                let items = ids.iter().map(|v| self.el(v)).collect::<Result<Vec<_>, _>>()?;
                let boxes: Vec<LBox> = items.iter().map(|e| {
                    let b = self.box_of(e);
                    LBox { id: e["id"].as_str().unwrap_or("").to_owned(), x: b.0, y: b.1, w: b.2, h: b.3 }
                }).collect();
                let pg = (s(op, "within") == Some("page")).then(|| {
                    let d = &self.doc().0["meta"]["dims"];
                    (num(&d["w"]), num(&d["h"]))
                });
                let pad = f(op, "padding").unwrap_or(0.0);
                let align = |a: Option<&str>| match a {
                    Some("start") => Some(Align::Start),
                    Some("center") => Some(Align::Center),
                    Some("end") => Some(Align::End),
                    _ => None,
                };
                let o = ArrangeOptions {
                    direction: match s(op, "direction") { Some("row") => Direction::Row, Some("column") => Direction::Column, _ => Direction::Grid },
                    gap: f(op, "gap"),
                    columns: f(op, "columns"),
                    align: align(s(op, "align")),
                    justify: match s(op, "justify") { Some("space-between") => Some(Justify::SpaceBetween), j => align(j).map(Justify::Align) },
                    stretch: op.get("stretch") == Some(&Value::Bool(true)),
                    x: f(op, "x").or(pg.map(|_| pad)),
                    y: f(op, "y").or(pg.map(|_| pad)),
                    width: f(op, "width").or(pg.map(|p| p.0 - 2.0 * pad)),
                    height: f(op, "height").or(pg.map(|p| p.1 - 2.0 * pad)),
                };
                let out = arrange(&boxes, &o);
                let ops = out.iter().zip(items.iter().zip(boxes.iter())).map(|(b, (e, bx))| move_by(e, b.x - bx.x, b.y - bx.y, if o.stretch { Some((b.w, b.h)) } else { None })).collect();
                Ok((ops, None, None))
            }
            "align" => {
                let ids: Vec<&str> = op["ids"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                let items = ids.iter().map(|v| self.el(v)).collect::<Result<Vec<_>, _>>()?;
                let bounds: Vec<AlignItem> = items.iter().map(|e| {
                    let b = self.box_of(e);
                    AlignItem { id: e["id"].as_str().unwrap_or("").to_owned(), bounds: Rect { x: b.0, y: b.1, w: b.2, h: b.3 } }
                }).collect();
                let d = &self.doc().0["meta"]["dims"];
                let (dw, dh) = (num(&d["w"]), num(&d["h"]));
                let rel = s(op, "relativeTo").unwrap_or(if items.len() == 1 { "page" } else { "selection" });
                let to = if rel == "page" {
                    Rect { x: 0.0, y: 0.0, w: dw, h: dh }
                } else {
                    let x = bounds.iter().map(|b| b.bounds.x).fold(f64::INFINITY, f64::min);
                    let y = bounds.iter().map(|b| b.bounds.y).fold(f64::INFINITY, f64::min);
                    Rect { x, y, w: bounds.iter().map(|b| b.bounds.x + b.bounds.w).fold(f64::NEG_INFINITY, f64::max) - x, h: bounds.iter().map(|b| b.bounds.y + b.bounds.h).fold(f64::NEG_INFINITY, f64::max) - y }
                };
                let edge = match s(op, "to").unwrap_or("") {
                    "left" => AlignEdge::Left,
                    "center" => AlignEdge::Hcenter,
                    "right" => AlignEdge::Right,
                    "top" => AlignEdge::Top,
                    "middle" => AlignEdge::Vcenter,
                    _ => AlignEdge::Bottom,
                };
                let deltas = align_deltas(&bounds, edge, &to);
                let ops = items.iter().map(|e| {
                    let d = deltas.get(e["id"].as_str().unwrap_or("")).copied().unwrap_or([0.0, 0.0]);
                    move_by(e, d[0], d[1], None)
                }).collect();
                Ok((ops, None, None))
            }
            "distribute" => {
                let ids: Vec<&str> = op["ids"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                let items = ids.iter().map(|v| self.el(v)).collect::<Result<Vec<_>, _>>()?;
                let bounds: Vec<AlignItem> = items.iter().map(|e| {
                    let b = self.box_of(e);
                    AlignItem { id: e["id"].as_str().unwrap_or("").to_owned(), bounds: Rect { x: b.0, y: b.1, w: b.2, h: b.3 } }
                }).collect();
                let deltas = distribute_deltas(&bounds, if s(op, "axis") == Some("horizontal") { Axis::X } else { Axis::Y });
                let ops = items.iter().map(|e| {
                    let d = deltas.get(e["id"].as_str().unwrap_or("")).copied().unwrap_or([0.0, 0.0]);
                    move_by(e, d[0], d[1], None)
                }).collect();
                Ok((ops, None, None))
            }
            "add_page" => {
                let pid = oc_schema::ids::new_id();
                self.last_page = Some(pid.clone());
                if op.get("duplicate").is_some() {
                    let src = self.page(op.get("duplicate"))?;
                    return Ok((vec![json!({ "op": "page", "action": "duplicate", "pageId": src, "newPageId": pid })], r#ref, Some(pid)));
                }
                let after = if op.get("after").is_some() { Some(self.page(op.get("after"))?) } else { self.sorted_pages().last().cloned() };
                let mut page = Map::new();
                page.insert("id".into(), json!(pid));
                if let Some(t) = s(op, "title").filter(|t| !t.is_empty()) {
                    page.insert("title".into(), json!(t));
                }
                if let Some(b) = op.get("background").filter(|b| truthy(Some(b))) {
                    page.insert("background".into(), json!([paint_in(b)]));
                }
                Ok((vec![json!({ "op": "page", "action": "add", "after": after, "page": page })], r#ref, Some(pid)))
            }
            "update_page" => {
                let pid = self.page(op.get("page"))?;
                self.last_page = Some(pid.clone());
                let mut patch = Map::new();
                if let Some(t) = op.get("title") {
                    patch.insert("title".into(), t.clone());
                }
                if let Some(b) = op.get("background").filter(|b| truthy(Some(b))) {
                    patch.insert("background".into(), json!([paint_in(b)]));
                }
                if let Some(n) = op.get("notes") {
                    patch.insert("notes".into(), n.clone());
                }
                Ok((vec![json!({ "op": "page", "action": "update", "pageId": pid, "patch": patch })], None, None))
            }
            "delete_page" => {
                if self.sorted_pages().len() <= 1 {
                    return err("A design needs at least one page.");
                }
                let pid = self.page(op.get("page"))?;
                if self.last_page.as_deref() == Some(pid.as_str()) {
                    self.last_page = None;
                }
                Ok((vec![json!({ "op": "page", "action": "delete", "pageId": pid })], None, None))
            }
            "move_page" => {
                let pid = self.page(op.get("page"))?;
                let after = if op.get("after").is_none_or(Value::is_null) { Value::Null } else { json!(self.page(op.get("after"))?) };
                Ok((vec![json!({ "op": "page", "action": "move", "pageId": pid, "after": after })], None, None))
            }
            "replace_text" => {
                let find = s(op, "find").unwrap_or("").to_owned();
                let with = s(op, "replace").unwrap_or("").to_owned();
                let opts = Match { case: op.get("match_case").and_then(Value::as_bool).unwrap_or(false), word: op.get("whole_word").and_then(Value::as_bool).unwrap_or(false) };
                // Where to look: given elements, else one page, else every page.
                let only: Option<Vec<String>> = match op.get("ids").and_then(Value::as_array) {
                    Some(ids) => Some(ids.iter().filter_map(Value::as_str).map(|v| self.id(v)).collect::<Result<_, _>>()?),
                    None => None,
                };
                let page = match (&only, op.get("page")) {
                    (None, Some(p)) => Some(self.page(Some(p))?),
                    _ => None,
                };
                let (json, typed) = self.doc().clone();
                let mut ops = Vec::new();
                let mut n = 0;
                for (id, el) in json["elements"].as_object().into_iter().flatten() {
                    if el["type"] != "text" || el["deleted"] == true {
                        continue;
                    }
                    if only.as_ref().is_some_and(|o| !o.contains(id)) {
                        continue;
                    }
                    if let Some(p) = &page
                        && oc_schema::tree::page_of(&typed.pages, &typed.elements, id) != Some(p.as_str())
                    {
                        continue;
                    }
                    let (body, k) = replace_in_body(&el["body"], &find, &with, opts);
                    if k > 0 {
                        n += k;
                        ops.push(json!({ "op": "setText", "id": id, "body": body }));
                    }
                }
                if n == 0 {
                    return err(format!("No text matching \"{find}\" was found{}.", if page.is_some() { " on that page" } else { "" }));
                }
                self.res.replaced += n;
                Ok((ops, None, None))
            }
            "set_title" => {
                self.res.title = s(op, "title").map(str::to_owned);
                Ok((vec![json!({ "op": "meta", "patch": { "title": op["title"] } })], None, None))
            }
            _ => err("Unknown operation."),
        }
    }

    fn compile_update(&mut self, op: &Value, i: usize) -> Result<Vec<Value>, EditError> {
        let target = s(op, "id").unwrap_or("");
        let e = self.el(target)?;
        let id = e["id"].as_str().unwrap_or("").to_owned();
        let ty = e["type"].as_str().unwrap_or("").to_owned();
        let mut out = Vec::new();
        if ["x", "y", "w", "h", "rotation"].iter().any(|k| op.get(*k).is_some()) {
            let cur = self.box_of(&e);
            let mut xf = e["xf"].clone();
            xf["x"] = jnum(num(&e["xf"]["x"]) + f(op, "x").unwrap_or(cur.0) - cur.0);
            xf["y"] = jnum(num(&e["xf"]["y"]) + f(op, "y").unwrap_or(cur.1) - cur.1);
            if let Some(w) = op.get("w") {
                xf["w"] = w.clone();
            }
            if let Some(h) = op.get("h") {
                xf["h"] = h.clone();
            }
            if let Some(r) = op.get("rotation") {
                xf["rot"] = r.clone();
            }
            out.push(json!({ "op": "xf", "id": id, "xf": xf }));
        }
        let mut patch = Map::new();
        if let Some(v) = op.get("opacity") {
            patch.insert("opacity".into(), v.clone());
        }
        if let Some(v) = op.get("name") {
            patch.insert("name".into(), v.clone());
        }
        if let Some(v) = op.get("role") {
            patch.insert("role".into(), v.clone());
        }
        if let Some(v) = op.get("hidden") {
            patch.insert("visible".into(), json!(v != true));
        }
        if let Some(v) = op.get("alt") {
            patch.insert("altText".into(), v.clone());
        }
        let style = run_style(op);
        let text = s(op, "text");
        let text_change = text.is_some() || !style.is_empty() || op.get("align").is_some() || op.get("lineHeight").is_some();
        let font = s(op, "font").filter(|v| !v.is_empty());
        let size = f(op, "size").filter(|v| *v != 0.0);
        if ty == "text" && text_change {
            out.push(json!({ "op": "setText", "id": id, "body": restyle(&e["body"], text, &style, s(op, "align"), op.get("lineHeight")) }));
        } else if ty != "text" && (text.is_some() || font.is_some() || size.is_some()) {
            if ty == "table" || ty == "chart" {
                if let Some(font) = font {
                    let mut st = e["style"].clone();
                    if !st.is_object() {
                        st = json!({});
                    }
                    st["family"] = json!(font);
                    if let (Some(sz), true) = (size, ty == "table") {
                        st["size"] = jnum(sz);
                    }
                    patch.insert("style".into(), st);
                }
            } else {
                return err(format!("{target} is a {ty}; text and font changes apply to text elements."));
            }
        }
        if let Some(fill) = op.get("fill") {
            match ty.as_str() {
                "shape" => {
                    patch.insert("fills".into(), json!([paint_in(fill)]));
                }
                "frame" => {
                    patch.insert("placeholderFill".into(), paint_in(fill));
                }
                "text" if fill.is_string() => {
                    let mut st = Map::new();
                    st.insert("fill".into(), solid(hex(fill.as_str().unwrap_or(""))));
                    out.push(json!({ "op": "setText", "id": id, "body": restyle(&e["body"], None, &st, None, None) }));
                }
                _ => return err(format!("{target} is a {ty}; fill applies to shapes.")),
            }
        }
        if let Some(stroke) = op.get("stroke") {
            if ty == "shape" {
                patch.insert("strokes".into(), if stroke.is_null() { json!([]) } else { json!([stroke_of(stroke)]) });
            } else if ty == "line" && !stroke.is_null() {
                let mut st = e["stroke"].clone();
                if let (Some(m), Value::Object(new)) = (st.as_object_mut(), stroke_of(stroke)) {
                    for (k, v) in new {
                        m.insert(k, v);
                    }
                }
                patch.insert("stroke".into(), st);
            } else {
                return err(format!("{target} is a {ty}; stroke applies to shapes and lines."));
            }
        }
        if let Some(c) = s(op, "color").filter(|c| !c.is_empty())
            && ty == "line"
        {
            let mut st = e["stroke"].clone();
            st["paint"] = solid(hex(c));
            patch.insert("stroke".into(), st);
        }
        if let Some(r) = op.get("radius") {
            match ty.as_str() {
                "shape" => {
                    let mut p = e["params"].clone();
                    if !p.is_object() {
                        p = json!({});
                    }
                    p["cornerRadius"] = r.clone();
                    patch.insert("params".into(), p);
                }
                "image" => {
                    patch.insert("cornerRadius".into(), r.clone());
                }
                _ => return err(format!("{target} is a {ty}; radius applies to shapes and images.")),
            }
        }
        if op.get("image").is_some() {
            let a = self.image_asset(i)?;
            out.push(json!({ "op": "asset", "asset": a }));
            let (aw, ah) = (num(&a["w"]), num(&a["h"]));
            let crop = if aw != 0.0 && !aw.is_nan() && ah != 0.0 && !ah.is_nan() {
                crate::routes::v1::cover_crop_for(num(&e["xf"]["w"]) / num(&e["xf"]["h"]), aw / ah)
            } else {
                json!({ "x": 0, "y": 0, "w": 1, "h": 1 })
            };
            match ty.as_str() {
                "image" => {
                    patch.insert("asset".into(), a["id"].clone());
                    patch.insert("crop".into(), crop);
                }
                "frame" => {
                    patch.insert("content".into(), json!({ "asset": a["id"], "crop": crop }));
                }
                _ => return err(format!("{target} is a {ty}; image replaces images and fills frames.")),
            }
        }
        if !patch.is_empty() {
            out.push(json!({ "op": "update", "id": id, "patch": patch }));
        }
        if out.is_empty() {
            return err("Nothing to change: give at least one property.");
        }
        Ok(out)
    }
}

/// A new element's base fields (`base(op, x, y, w, h, parent)` + the factory defaults).
fn base(ty: &str, op: &Value, x: f64, y: f64, w: f64, h: f64, parent: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("id".into(), json!(oc_schema::ids::new_id()));
    m.insert("type".into(), json!(ty));
    m.insert("parentId".into(), json!(parent));
    m.insert("z".into(), json!("a0"));
    m.insert("opacity".into(), op.get("opacity").cloned().unwrap_or(json!(1)));
    m.insert("xf".into(), json!({ "x": jnum(x), "y": jnum(y), "w": jnum(w), "h": jnum(h), "rot": op.get("rotation").cloned().unwrap_or(json!(0)) }));
    if let Some(n) = s(op, "name").filter(|n| !n.is_empty()) {
        m.insert("name".into(), json!(n));
    }
    if let Some(r) = s(op, "role") {
        m.insert("role".into(), json!(r));
    }
    m
}

fn move_by(e: &Value, dx: f64, dy: f64, size: Option<(f64, f64)>) -> Value {
    let mut xf = e["xf"].clone();
    xf["x"] = jnum(num(&e["xf"]["x"]) + dx);
    xf["y"] = jnum(num(&e["xf"]["y"]) + dy);
    if let Some((w, h)) = size {
        xf["w"] = jnum(w);
        xf["h"] = jnum(h);
    }
    json!({ "op": "xf", "id": e["id"], "xf": xf })
}

fn empty_doc() -> oc_schema::design::DesignDoc {
    oc_schema::factory::empty_design(&oc_schema::presets::preset_design_init(None, &Default::default()).meta_init())
}

/// How `replace_text` matches.
#[derive(Clone, Copy, Default)]
pub struct Match {
    /// Case-sensitive (default: not).
    pub case: bool,
    /// Only whole words (no letter or digit right before or after).
    pub word: bool,
}

/// Replaces every match of `find` in a text body, paragraph by paragraph, keeping the styling: unmatched text keeps
/// its run, the replacement takes the run where the match starts (a match may span runs). Returns the new body and
/// the number of matches.
pub fn replace_in_body(body: &Value, find: &str, with: &str, opts: Match) -> (Value, usize) {
    let norm = |c: char| if opts.case { c } else { c.to_lowercase().next().unwrap_or(c) };
    let needle: Vec<char> = find.chars().map(norm).collect();
    let mut out = body.clone();
    let mut count = 0;
    if needle.is_empty() {
        return (out, 0);
    }
    let Some(paras) = out.get_mut("paragraphs").and_then(Value::as_array_mut) else { return (out, 0) };
    for p in paras.iter_mut() {
        let Some(runs) = p.get("runs").and_then(Value::as_array).cloned() else { continue };
        // Every character with the index of its run.
        let chars: Vec<(char, usize)> = runs.iter().enumerate().flat_map(|(i, r)| r["text"].as_str().unwrap_or("").chars().map(move |c| (c, i))).collect();
        let word_char = |k: usize| chars.get(k).is_some_and(|(c, _)| c.is_alphanumeric());
        let mut pieces: Vec<(String, usize)> = Vec::new();
        let push = |pieces: &mut Vec<(String, usize)>, text: &str, run: usize| match pieces.last_mut() {
            Some((t, r)) if *r == run => t.push_str(text),
            _ => pieces.push((text.to_owned(), run)),
        };
        let mut k = 0;
        let mut changed = false;
        while k < chars.len() {
            let hit = k + needle.len() <= chars.len()
                && chars[k..k + needle.len()].iter().zip(&needle).all(|((c, _), n)| norm(*c) == *n)
                && (!opts.word || ((k == 0 || !word_char(k - 1)) && !word_char(k + needle.len())));
            if hit {
                push(&mut pieces, with, chars[k].1);
                k += needle.len();
                count += 1;
                changed = true;
            } else {
                let mut b = [0u8; 4];
                push(&mut pieces, chars[k].0.encode_utf8(&mut b), chars[k].1);
                k += 1;
            }
        }
        if changed {
            let new_runs: Vec<Value> = pieces
                .into_iter()
                .filter(|(t, _)| !t.is_empty())
                .map(|(t, i)| {
                    let mut r = runs[i].clone();
                    r["text"] = json!(t);
                    r
                })
                .collect();
            p["runs"] = Value::Array(new_runs);
        }
    }
    (out, count)
}

/// Fills one row of data into a design document (MCP `bulk_create`): a text element bound to a column
/// (`dataBinding.column`, as autofill uses) gets that field's value as its whole text, in its first run's style. A field
/// no element is bound to is replaced literally in all text (case-sensitive, whole words, keeping the styling), so placeholders like
/// "{name}" or "Jane Doe" work in designs without bindings. Returns how many places each field changed.
pub fn fill_row(doc: &mut Value, row: &Map<String, Value>) -> Map<String, Value> {
    let mut counts = Map::new();
    let ids: Vec<String> = doc["elements"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    for (key, value) in row {
        let value = value.as_str().unwrap_or("");
        let bound = ids.iter().any(|id| doc["elements"][id].pointer("/dataBinding/column").and_then(Value::as_str) == Some(key.as_str()));
        let mut n = 0;
        for id in &ids {
            let el = &mut doc["elements"][id];
            if el["type"] != "text" || el["deleted"] == true {
                continue;
            }
            if el.pointer("/dataBinding/column").and_then(Value::as_str) == Some(key.as_str()) {
                let style = el.pointer("/body/paragraphs/0/runs/0/style").cloned().unwrap_or_else(|| json!({}));
                let mut attrs = el.pointer("/body/paragraphs/0").and_then(Value::as_object).cloned().unwrap_or_default();
                attrs.remove("runs");
                let paragraphs: Vec<Value> = value
                    .split('\n')
                    .map(|line| {
                        let mut p = attrs.clone();
                        p.insert("runs".into(), if line.is_empty() { json!([]) } else { json!([{ "text": line, "style": style }]) });
                        Value::Object(p)
                    })
                    .collect();
                el["body"]["paragraphs"] = Value::Array(paragraphs);
                n += 1;
                continue;
            }
            if bound {
                continue;
            }
            let (body, k) = replace_in_body(&el["body"], key, value, Match { case: true, word: true });
            if k > 0 {
                el["body"] = body;
                n += k;
            }
        }
        counts.insert(key.clone(), json!(n));
    }
    counts
}

/// `runEdits(ops, ctx)`.
pub fn run_edits(ops: &[Value], ctx: &EditContext<'_>) -> EditResult {
    let mut run = Run { ctx, res: EditResult { total: ops.len(), ..Default::default() }, snap: None, last_page: None };
    for (i, op) in ops.iter().enumerate() {
        let kind = s(op, "op").unwrap_or("").to_owned();
        let step = (|| {
            let (docops, r#ref, new_id) = run.compile(op, i)?;
            let parsed = oc_crdt::ops::parse_ops(&Value::Array(docops.clone())).map_err(|e| EditError(e.to_string()))?;
            let result = oc_crdt::apply_ops(
                ctx.ydoc,
                &parsed,
                Some(OcOrigin::Agent.origin()),
                ApplyOptions { actor: Some(ctx.actor.clone()), now: Some(ctx.now), ..Default::default() },
            );
            if !result.ok {
                let msg = result.errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ");
                return Err(EditError(if msg.is_empty() { "The change was rejected.".into() } else { msg }));
            }
            Ok((docops, r#ref, new_id))
        })();
        match step {
            Ok((docops, r#ref, new_id)) => {
                run.snap = None;
                if let (Some(r), Some(id)) = (&r#ref, &new_id) {
                    run.res.refs.insert(r.clone(), json!(id));
                }
                run.res.created.push(new_id.into_iter().collect());
                for d in &docops {
                    if let Some(id) = d.get("id").and_then(Value::as_str) {
                        run.res.touched.push(id.to_owned());
                    }
                    if d["op"] == "add"
                        && let Some(id) = d["el"]["id"].as_str()
                    {
                        run.res.touched.push(id.to_owned());
                    }
                }
                run.res.applied += 1;
            }
            Err(EditError(message)) => {
                run.res.error = Some((i, kind, message));
                break;
            }
        }
    }
    let mut seen = Vec::new();
    {
        let r = oc_crdt::roots(ctx.ydoc);
        let txn = ctx.ydoc.transact();
        for t in std::mem::take(&mut run.res.touched) {
            if seen.contains(&t) {
                continue;
            }
            if oc_crdt::codec::element_map(&txn, &r, &t).is_some_and(|m| !oc_crdt::codec::is_deleted(&txn, &m)) {
                seen.push(t);
            }
        }
    }
    run.res.touched = seen;
    run.res.page_id = run.last_page.clone();
    run.res
}

#[cfg(test)]
mod replace_tests {
    use super::*;

    #[test]
    fn fill_row_uses_bindings_then_literal_text() {
        let doc0 = json!({ "elements": {
            "a": { "type": "text", "dataBinding": { "column": "name" }, "body": { "paragraphs": [{ "align": "center", "runs": [{ "text": "Placeholder", "style": { "weight": 700 } }] }] } },
            "b": { "type": "text", "body": { "paragraphs": [{ "runs": [{ "text": "Awarded to {name} for {course}, renamed", "style": {} }] }] } },
            "c": { "type": "shape" }
        } });
        let text_of = |d: &Value, id: &str| d["elements"][id]["body"]["paragraphs"][0]["runs"][0]["text"].clone();
        // A bound key fills only its element, as its whole text, keeping the first run's style and the paragraph.
        let mut doc = doc0.clone();
        let counts = fill_row(&mut doc, json!({ "name": "Ada Lovelace", "missing": "x" }).as_object().unwrap());
        assert_eq!(text_of(&doc, "a"), "Ada Lovelace");
        assert_eq!(doc["elements"]["a"]["body"]["paragraphs"][0]["runs"][0]["style"]["weight"], 700);
        assert_eq!(doc["elements"]["a"]["body"]["paragraphs"][0]["align"], "center");
        assert_eq!(text_of(&doc, "b"), "Awarded to {name} for {course}, renamed");
        assert_eq!((counts["name"].clone(), counts["missing"].clone()), (json!(1), json!(0)));
        // Unbound keys are literal placeholders, matched as whole words.
        let mut doc = doc0.clone();
        let counts = fill_row(&mut doc, json!({ "{name}": "Ada", "{course}": "Engines", "name": "X" }).as_object().unwrap());
        assert_eq!(text_of(&doc, "b"), "Awarded to Ada for Engines, renamed");
        assert_eq!((counts["{name}"].clone(), counts["{course}"].clone(), counts["name"].clone()), (json!(1), json!(1), json!(1)));
        // Without the binding, "name" is a literal whole word: it matches inside "{name}" but not inside "renamed".
        let mut doc = doc0.clone();
        doc["elements"]["a"]["dataBinding"] = Value::Null;
        let counts = fill_row(&mut doc, json!({ "name": "X" }).as_object().unwrap());
        assert_eq!(text_of(&doc, "b"), "Awarded to {X} for {course}, renamed");
        assert_eq!(counts["name"], 1);
    }

    fn body(runs: &[(&str, &str)]) -> Value {
        json!({ "defaultRun": {}, "paragraphs": [{ "runs": runs.iter().map(|(t, w)| json!({ "text": t, "style": { "weight": w } })).collect::<Vec<_>>() }] })
    }

    #[test]
    fn keeps_styles_and_spans_runs() {
        // "Summer Sale" with "Summer" bold: the replacement keeps the bold of the run where the match starts.
        let b = body(&[("Big ", "400"), ("Summer", "700"), (" Sale today", "400")]);
        let (out, n) = replace_in_body(&b, "summer sale", "Autumn deals", Match::default());
        assert_eq!(n, 1);
        let runs: Vec<(String, String)> = out["paragraphs"][0]["runs"].as_array().unwrap().iter().map(|r| (r["text"].as_str().unwrap().into(), r["style"]["weight"].as_str().unwrap().into())).collect();
        assert_eq!(runs, vec![("Big ".into(), "400".into()), ("Autumn deals".into(), "700".into()), (" today".into(), "400".into())]);
    }

    #[test]
    fn case_and_whole_words() {
        let b = body(&[("Cat category CAT", "400")]);
        assert_eq!(replace_in_body(&b, "cat", "dog", Match::default()).1, 3);
        assert_eq!(replace_in_body(&b, "cat", "dog", Match { word: true, ..Default::default() }).1, 2);
        let (out, n) = replace_in_body(&b, "Cat", "Dog", Match { case: true, word: true });
        assert_eq!(n, 1);
        assert_eq!(out["paragraphs"][0]["runs"][0]["text"], "Dog category CAT");
        // Deleting a word drops runs that end up empty.
        let (out, _) = replace_in_body(&body(&[("A", "400"), ("B", "700")]), "B", "", Match::default());
        assert_eq!(out["paragraphs"][0]["runs"].as_array().unwrap().len(), 1);
    }
}
