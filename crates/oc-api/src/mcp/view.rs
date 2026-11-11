//! Port of services/api/src/mcp/view.ts — a compact, model-friendly view of a design for AI assistants (MCP
//! `get_design`): a short page → element tree with page-space boxes (px at 96 dpi), plain text, hex colours and the
//! few style fields an assistant needs to edit.
//!
//! Works on the JSON snapshot (`from_y_value`) for element fields and on the typed document for tree order and
//! world bounds (oc-schema / oc-geometry, the ports of the TS helpers).

use oc_schema::design::DesignDoc;
use serde_json::{Map, Value, json};

use crate::js;

pub fn r1(n: f64) -> f64 {
    js::round(n * 10.0) / 10.0
}

pub fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

/// JS truthiness of an optional JSON value.
pub fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(oc_schema::tree::truthy)
}

fn clip(s: &str, n: usize) -> String {
    let len = js::len(s);
    if len > n { format!("{}… ({len} chars)", js::take(s, n)) } else { s.to_owned() }
}

/// `toHex(c, c.a < 1)` and the cmyk/spot forms.
pub fn color_out(c: &Value) -> Value {
    match c.get("space").and_then(Value::as_str) {
        Some("cmyk") => Value::String(format!("cmyk({},{},{},{})", js::num(num(&c["c"])), js::num(num(&c["m"])), js::num(num(&c["y"])), js::num(num(&c["k"])))),
        Some("spot") => Value::String(format!("spot({})", js_string(c.get("swatch")))),
        _ => match serde_json::from_value::<oc_schema::primitives::ColorValue>(c.clone()) {
            Ok(cv) => Value::String(oc_schema::color::to_hex(&cv, num(&c["a"]) < 1.0)),
            Err(_) => Value::Null,
        },
    }
}

fn js_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) => "null".into(),
        Some(Value::Number(n)) => js::num(n.as_f64().unwrap_or(f64::NAN)),
        Some(other) => other.to_string(),
    }
}

/// A paint as an assistant writes it: `'#rrggbb[aa]'`, or a short description for gradients and images.
pub fn paint_out(p: Option<&Value>) -> Option<Value> {
    let p = p.filter(|p| !p.is_null())?;
    let stops = || Value::Array(p["stops"].as_array().map(|a| a.iter().map(|s| color_out(&s["color"])).collect()).unwrap_or_default());
    Some(match p.get("k").and_then(Value::as_str) {
        Some("solid") => color_out(&p["color"]),
        Some("linear") => {
            let mut m = Map::new();
            m.insert("gradient".into(), stops());
            if let Some(a) = p.get("angle") {
                m.insert("angle".into(), a.clone());
            }
            Value::Object(m)
        }
        Some("radial") => json!({ "gradient": stops(), "radial": true }),
        _ => {
            let mut m = Map::new();
            if let Some(a) = p.get("asset") {
                m.insert("image".into(), a.clone());
            }
            if let Some(f) = p.get("fit") {
                m.insert("fit".into(), f.clone());
            }
            Value::Object(m)
        }
    })
}

fn put(m: &mut Map<String, Value>, k: &str, v: Option<Value>) {
    if let Some(v) = v {
        m.insert(k.into(), v);
    }
}

pub fn body_plain_text(body: &Value) -> String {
    body["paragraphs"]
        .as_array()
        .map(|ps| {
            ps.iter()
                .map(|p| p["runs"].as_array().map(|rs| rs.iter().map(|r| r["text"].as_str().unwrap_or("")).collect::<String>()).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

pub fn text_out(body: &Value, max_text: usize) -> Map<String, Value> {
    let d = &body["defaultRun"];
    let paragraphs = body["paragraphs"].as_array().cloned().unwrap_or_default();
    let p0 = paragraphs.first();
    let mut out = Map::new();
    out.insert("text".into(), json!(clip(&body_plain_text(body), max_text)));
    put(&mut out, "font", d.get("family").cloned());
    put(&mut out, "size", d.get("size").map(|s| json!(r1(num(s)))));
    put(&mut out, "weight", d.get("weight").cloned());
    put(&mut out, "color", paint_out(d.get("fill")));
    if truthy(d.get("italic")) {
        out.insert("italic".into(), json!(true));
    }
    if truthy(d.get("tracking")) {
        out.insert("letterSpacing".into(), d["tracking"].clone());
    }
    if let Some(c) = d.get("caseTransform").filter(|c| truthy(Some(c)) && *c != "none") {
        out.insert("case".into(), c.clone());
    }
    if let Some(p0) = p0 {
        if p0.get("align").is_none_or(|a| a != "start") {
            let a = p0.get("align").cloned();
            put(&mut out, "align", if a.as_ref().is_some_and(|a| a == "end") { Some(json!("right")) } else { a });
        }
        if p0.get("lineHeight").is_none_or(|l| num(l) != 1.3) {
            put(&mut out, "lineHeight", p0.get("lineHeight").cloned());
        }
    }
    if let Some(l) = paragraphs.iter().find(|p| truthy(p.get("list"))) {
        put(&mut out, "list", l["list"].get("kind").cloned());
    }
    let mixed = paragraphs.iter().any(|p| p["runs"].as_array().is_some_and(|rs| rs.iter().any(|r| r["style"].as_object().is_some_and(|s| !s.is_empty()))));
    if mixed {
        out.insert("mixedStyles".into(), json!(true));
    }
    out
}

fn detail(el: &Value, max_text: usize) -> Map<String, Value> {
    let mut o = Map::new();
    match el["type"].as_str().unwrap_or("") {
        "text" => {
            o = text_out(&el["body"], max_text);
            if el["frame"]["mode"] == "fixed" {
                o.insert("fixedHeight".into(), json!(true));
            }
            if el["autoFit"] == "shrink" {
                o.insert("shrinkToFit".into(), json!(true));
            }
            if truthy(el.get("effect")) {
                put(&mut o, "effect", el["effect"].get("k").cloned());
            }
        }
        "shape" => {
            put(&mut o, "shape", el.get("kind").cloned());
            let fills = el["fills"].as_array().cloned().unwrap_or_default();
            o.insert("fill".into(), if fills.is_empty() { json!("none") } else { paint_out(fills.first()).unwrap_or(Value::Null) });
            if let Some(st) = el["strokes"].as_array().and_then(|a| a.first()).filter(|s| truthy(Some(s))) {
                let mut sm = Map::new();
                put(&mut sm, "color", paint_out(st.get("paint")));
                put(&mut sm, "width", st.get("width").cloned());
                if st["dash"].as_array().is_some_and(|d| !d.is_empty()) {
                    sm.insert("dash".into(), st["dash"].clone());
                }
                o.insert("stroke".into(), Value::Object(sm));
            }
            if truthy(el["params"].get("cornerRadius")) {
                o.insert("radius".into(), el["params"]["cornerRadius"].clone());
            }
        }
        "line" => {
            put(&mut o, "color", paint_out(el["stroke"].get("paint")));
            put(&mut o, "width", el["stroke"].get("width").cloned());
            let (x, y) = (num(&el["xf"]["x"]), num(&el["xf"]["y"]));
            let pts: Vec<Value> = el["points"].as_array().map(|a| a.iter().map(|p| json!([r1(x + num(&p[0])), r1(y + num(&p[1]))])).collect()).unwrap_or_default();
            o.insert("points".into(), Value::Array(pts));
            if let Some(m) = el.get("startMarker").filter(|m| *m != "none") {
                o.insert("start".into(), m.clone());
            }
            if let Some(m) = el.get("endMarker").filter(|m| *m != "none") {
                o.insert("end".into(), m.clone());
            }
        }
        "image" => {
            put(&mut o, "asset", el.get("asset").cloned());
            if truthy(el.get("cornerRadius")) {
                o.insert("radius".into(), el["cornerRadius"].clone());
            }
            if truthy(el.get("altText")) {
                o.insert("alt".into(), el["altText"].clone());
            }
        }
        "frame" => {
            put(&mut o, "clip", el.get("clip").cloned());
            if truthy(el.get("content")) {
                put(&mut o, "image", el["content"].get("asset").cloned());
            } else {
                o.insert("empty".into(), json!(true));
            }
        }
        "table" => {
            let rows = el["rows"].as_array().map_or(0, Vec::len);
            let cols = el["cols"].as_array().map_or(0, Vec::len);
            o.insert("rows".into(), json!(rows));
            o.insert("cols".into(), json!(cols));
            let cells: Vec<Value> = (0..rows.min(8))
                .map(|r| Value::Array((0..cols).map(|c| json!(clip(el["cells"][format!("{r}:{c}")]["text"].as_str().unwrap_or(""), 60))).collect()))
                .collect();
            o.insert("cells".into(), Value::Array(cells));
        }
        "chart" => {
            put(&mut o, "chart", el.get("kind").cloned());
            let labels: Vec<Value> = el["labels"].as_array().map(|a| a.iter().take(24).cloned().collect()).unwrap_or_default();
            o.insert("labels".into(), Value::Array(labels));
            let series: Vec<Value> = el["series"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .take(8)
                        .map(|s| {
                            let mut m = Map::new();
                            put(&mut m, "name", s.get("name").cloned());
                            m.insert("values".into(), Value::Array(s["values"].as_array().map(|v| v.iter().take(24).cloned().collect()).unwrap_or_default()));
                            m.insert("color".into(), color_out(&s["color"]));
                            Value::Object(m)
                        })
                        .collect()
                })
                .unwrap_or_default();
            o.insert("series".into(), Value::Array(series));
        }
        "qr" => put(&mut o, "data", el.get("data").cloned()),
        "video" => put(&mut o, "asset", el.get("asset").cloned()),
        "embed" => put(&mut o, "url", el.get("url").cloned()),
        _ => {}
    }
    o
}

/// Page-space top-left of an element whose own box and every parent group are unrotated, else `None`.
pub fn page_pos(doc: &Value, el: &Value) -> Option<(f64, f64)> {
    let (mut x, mut y) = (0.0, 0.0);
    let mut e = Some(el);
    let mut guard = 0;
    while let Some(cur) = e {
        if truthy(cur["xf"].get("rot")) {
            return None;
        }
        x += num(&cur["xf"]["x"]);
        y += num(&cur["xf"]["y"]);
        e = cur.get("parentId").and_then(Value::as_str).and_then(|p| doc["elements"].get(p));
        guard += 1;
        if guard > 1000 {
            break;
        }
    }
    Some((x, y))
}

/// The ids of the live children of `parent`, bottom to top.
pub fn children(typed: &DesignDoc, parent: &str) -> Vec<String> {
    oc_schema::tree::children_of(&typed.elements, parent).into_iter().filter_map(|e| e.id().map(str::to_owned)).collect()
}

/// Page ids in page order.
pub fn sorted_page_ids(typed: &DesignDoc) -> Vec<String> {
    oc_schema::tree::sorted_pages(&typed.pages).into_iter().map(|p| p.id.to_string()).collect()
}

/// `worldBounds(doc, id)` as (x, y, w, h).
pub fn world_bounds(typed: &DesignDoc, id: &str) -> (f64, f64, f64, f64) {
    match oc_geometry::scene::world_bounds(&typed.elements, id) {
        Some(r) => (r.x, r.y, r.w, r.h),
        None => (f64::NAN, f64::NAN, f64::NAN, f64::NAN),
    }
}

fn count_live(typed: &DesignDoc, page: &str) -> usize {
    let mut n = 0;
    let mut stack = vec![page.to_owned()];
    while let Some(p) = stack.pop() {
        for c in children(typed, &p) {
            n += 1;
            stack.push(c);
        }
    }
    n
}

/// The compact tree. Positions are page-space bounds, so nested elements read the same as top-level ones.
pub fn design_view(doc: &Value, typed: &DesignDoc, page_id: Option<&str>) -> Map<String, Value> {
    let max_elements = 400;
    let max_text = 2000;
    let order = sorted_page_ids(typed);
    let mut listed = 0usize;
    fn node(doc: &Value, typed: &DesignDoc, id: &str, listed: &mut usize, max_text: usize) -> Value {
        *listed += 1;
        let el = &doc["elements"][id];
        let mut n = Map::new();
        n.insert("id".into(), json!(id));
        n.insert("type".into(), el["type"].clone());
        if truthy(el.get("name")) {
            n.insert("name".into(), el["name"].clone());
        }
        if let Some(r) = el.get("role").filter(|r| r.is_string()) {
            n.insert("role".into(), r.clone());
        }
        let (x, y) = match page_pos(doc, el) {
            Some(p) => p,
            None => {
                let b = world_bounds(typed, id);
                (b.0, b.1)
            }
        };
        n.insert("x".into(), json!(r1(x)));
        n.insert("y".into(), json!(r1(y)));
        n.insert("w".into(), json!(r1(num(&el["xf"]["w"]))));
        n.insert("h".into(), json!(r1(num(&el["xf"]["h"]))));
        if truthy(el["xf"].get("rot")) {
            n.insert("rotation".into(), json!(r1(num(&el["xf"]["rot"]))));
        }
        if el.get("opacity").is_some_and(|o| num(o) < 1.0) {
            n.insert("opacity".into(), json!(r1(num(&el["opacity"]))));
        }
        if el.get("visible") == Some(&Value::Bool(false)) {
            n.insert("hidden".into(), json!(true));
        }
        if let Some(lock) = el.get("lock").filter(|l| truthy(Some(l)) && l["by"] != "brand")
            && lock.as_object().is_some_and(|m| m.values().any(oc_schema::tree::truthy))
        {
            n.insert("locked".into(), json!(true));
        }
        for (k, v) in detail(el, max_text) {
            n.insert(k, v);
        }
        for (k, v) in super::motion::element_view(el) {
            n.insert(k, v);
        }
        let kids = children(typed, id);
        if !kids.is_empty() {
            n.insert("children".into(), Value::Array(kids.iter().map(|k| node(doc, typed, k, listed, max_text)).collect()));
        }
        Value::Object(n)
    }
    let meta = &doc["meta"];
    let mut size = Map::new();
    size.insert("w".into(), meta["dims"]["w"].clone());
    size.insert("h".into(), meta["dims"]["h"].clone());
    size.insert("unit".into(), json!("px"));
    if meta["dims"]["unit"] != "px" {
        size.insert("displayUnit".into(), meta["dims"]["unit"].clone());
    }
    // Page lengths and transitions once the design uses time (a video, animations, sound).
    let timed = super::motion::is_timed(doc);
    let lengths: std::collections::HashMap<String, f64> = oc_schema::timeline::timeline_of(typed, true).spans.iter().map(|s| (s.page.id.to_string(), s.duration_ms)).collect();
    let mut pages = Vec::new();
    for pid in order.iter().filter(|p| page_id.is_none_or(|q| q == p.as_str())) {
        let p = &doc["pages"][pid.as_str()];
        let top = children(typed, pid);
        let before = listed;
        let mut elements = Vec::new();
        for id in &top {
            if listed - before >= max_elements {
                break;
            }
            elements.push(node(doc, typed, id, &mut listed, max_text));
        }
        let total = count_live(typed, pid);
        let mut pm = Map::new();
        pm.insert("id".into(), json!(pid));
        pm.insert("index".into(), json!(order.iter().position(|x| x == pid).map_or(0, |i| i + 1)));
        if truthy(p.get("title")) {
            pm.insert("title".into(), p["title"].clone());
        }
        let bg = p["background"].as_array().cloned().unwrap_or_default();
        pm.insert("background".into(), if bg.is_empty() { json!("#ffffff") } else { paint_out(bg.last()).unwrap_or(Value::Null) });
        if let Some(n) = p.get("notes").and_then(Value::as_str).filter(|n| !n.is_empty()) {
            pm.insert("notes".into(), json!(clip(n, 500)));
        }
        if truthy(p.get("hidden")) {
            pm.insert("hidden".into(), json!(true));
        }
        if timed {
            for (k, v) in super::motion::page_view(p, lengths.get(pid.as_str()).copied().unwrap_or(oc_schema::timeline::DEFAULT_PAGE_MS)) {
                pm.insert(k, v);
            }
        }
        pm.insert("elements".into(), Value::Array(elements));
        if total > listed - before {
            pm.insert("omitted".into(), json!(total - (listed - before)));
        }
        pages.push(Value::Object(pm));
    }
    let mut out = Map::new();
    out.insert("title".into(), meta["title"].clone());
    out.insert("type".into(), meta["surface"].clone());
    out.insert("size".into(), Value::Object(size));
    out.insert("pageCount".into(), json!(order.len()));
    if timed {
        out.insert("timeline".into(), super::motion::timeline_view(doc, typed));
    }
    out.insert("pages".into(), Value::Array(pages));
    out
}

/// One line per design for listings.
pub fn design_summary(d: &Value, web_origin: &str) -> Value {
    let id = d["id"].as_str().unwrap_or("");
    json!({
        "id": id,
        "title": d["title"],
        "type": d["surface"],
        "size": format!("{}×{} px", js::num(js::round(num(&d["width"]))), js::num(js::round(num(&d["height"])))),
        "pages": d["page_count"],
        "updated": d["updated_at"],
        "editUrl": format!("{web_origin}/design/{id}"),
    })
}
