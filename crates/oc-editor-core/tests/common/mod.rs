// SPDX-License-Identifier: Apache-2.0
//! Shared helpers for the ported TS tests and the golden replay.
#![allow(dead_code)]

use oc_editor_core::{Editor, EditorOptions, ManualClock, Mulberry32, TextService, Viewport};
use oc_schema::design::Surface;
use oc_schema::elements::{Element, TextEl};
use oc_schema::factory::{MetaInit, ShapeOpts, TextOpts, base_init, empty_design, make_shape, make_text};
use oc_schema::primitives::{Paint, Xf};
use oc_schema::text::RunStyleOverride;
use oc_schema::tree::sorted_pages;
use serde_json::{Map, Value};

pub fn lines(el: &TextEl) -> Vec<String> {
    el.body.paragraphs.iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect()).collect()
}

fn u16len(s: &str) -> i64 {
    s.encode_utf16().count() as i64
}

fn is_word(c: u16) -> bool {
    c < 128 && ((c as u8).is_ascii_alphanumeric() || c == b'_' as u16)
}

/// editor.test.ts: monospace fake — 10px per char, 20px per line, no wrapping.
pub struct FakeText;

impl TextService for FakeText {
    fn height(&self, el: &TextEl, _w: f64) -> f64 {
        el.body.paragraphs.len() as f64 * 20.0
    }
    fn offset_at(&self, el: &TextEl, x: f64, y: f64) -> i64 {
        let ls = lines(el);
        let li = ((y / 20.0).floor() as i64).clamp(0, ls.len() as i64 - 1) as usize;
        let mut off = 0;
        for l in ls.iter().take(li) {
            off += u16len(l) + 1;
        }
        off + (oc_schema::js::round(x / 10.0) as i64).clamp(0, u16len(&ls[li]))
    }
    fn vertical_move(&self, _el: &TextEl, o: i64, _dir: i32) -> i64 {
        o
    }
    fn word_at(&self, el: &TextEl, o: i64) -> (i64, i64) {
        let t: Vec<u16> = lines(el).join("\n").encode_utf16().collect();
        let (mut a, mut b) = (o, o);
        while a > 0 && is_word(t[(a - 1) as usize]) {
            a -= 1;
        }
        while b < t.len() as i64 && is_word(t[b as usize]) {
            b += 1;
        }
        (a, b)
    }
    fn length(&self, el: &TextEl) -> i64 {
        el.body.paragraphs.iter().map(|p| p.runs.iter().map(|r| u16len(&r.text)).sum::<i64>()).sum::<i64>() + el.body.paragraphs.len() as i64 - 1
    }
}

/// table.test.ts: monospace fake that wraps at the box width.
pub struct WrapText;

impl TextService for WrapText {
    fn height(&self, el: &TextEl, w: f64) -> f64 {
        lines(el).iter().map(|l| ((u16len(l) as f64 * 10.0) / w.max(1.0)).ceil().max(1.0) * 20.0).sum()
    }
    fn offset_at(&self, el: &TextEl, x: f64, _y: f64) -> i64 {
        (oc_schema::js::round(x / 10.0) as i64).clamp(0, u16len(&lines(el).join("\n")))
    }
    fn vertical_move(&self, _el: &TextEl, o: i64, _dir: i32) -> i64 {
        o
    }
    fn word_at(&self, _el: &TextEl, o: i64) -> (i64, i64) {
        (o, o)
    }
    fn length(&self, el: &TextEl) -> i64 {
        u16len(&lines(el).join("\n"))
    }
}

pub fn editor_for(design: &oc_schema::DesignDoc, vp: Viewport) -> Editor {
    let ydoc = oc_crdt::new_doc_with_client(1);
    oc_crdt::to_y(design, &ydoc, None).expect("to_y");
    let clock = ManualClock::new(1_700_000_000_000.0);
    Editor::from_ydoc(ydoc, EditorOptions { viewport: vp, clock, rng: Box::new(Mulberry32::new(7)) })
}

pub fn vp(x: f64, y: f64, w: f64, h: f64) -> Viewport {
    Viewport { x, y, zoom: 1.0, width: w, height: h }
}

/// `emptyDesign({ w, h, pages, surface })` in an editor with the given viewport.
pub fn setup_with(w: f64, h: f64, pages: usize, surface: Option<Surface>, viewport: Viewport) -> (Editor, String) {
    let d = empty_design(&MetaInit { w, h, pages: Some(pages), surface, ..Default::default() });
    let ed = editor_for(&d, viewport);
    let page = sorted_pages(&d.pages)[0].id.clone();
    (ed, page)
}

pub fn xf(x: f64, y: f64, w: f64, h: f64) -> Xf {
    Xf::new(x, y, w, h, 0.0)
}

pub fn to_map(el: Element) -> Map<String, Value> {
    match oc_schema::js::to_value(&el).unwrap() {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

pub fn shape(x: Xf) -> Map<String, Value> {
    to_map(Element::Shape(make_shape(base_init(x), ShapeOpts::default())))
}

pub fn shape_fill(x: Xf, fill: &str) -> Map<String, Value> {
    let p: Paint = oc_schema::color::solid_hex(fill).unwrap();
    to_map(Element::Shape(make_shape(base_init(x), ShapeOpts { fill: Some(p), ..Default::default() })))
}

pub fn text(x: Xf, s: &str) -> Map<String, Value> {
    to_map(Element::Text(make_text(base_init(x), TextOpts { text: Some(s.into()), ..Default::default() })))
}

pub fn text_styled(x: Xf, s: &str, style: RunStyleOverride) -> Map<String, Value> {
    to_map(Element::Text(make_text(base_init(x), TextOpts { text: Some(s.into()), style: Some(style), ..Default::default() })))
}

pub fn id_of(m: &Map<String, Value>) -> String {
    m["id"].as_str().unwrap().to_string()
}

pub fn el(ed: &Editor, id: &str) -> Element {
    ed.doc().el(id).cloned().expect("element")
}

pub fn exf(ed: &Editor, id: &str) -> Xf {
    el(ed, id).xf().cloned().unwrap()
}

pub fn text_el(ed: &Editor, id: &str) -> TextEl {
    match el(ed, id) {
        Element::Text(t) => t,
        _ => panic!("not text"),
    }
}

pub fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => panic!("not an object"),
    }
}

/// tools/golden/editor.ts `goldText`: monospace 10px per UTF-16 unit, 20px lines, wrapping at the box width,
/// paragraph-wise caret moves, ASCII `\w` words, and a curved-text box from the text length and the curve.
pub struct GoldText;

fn gold_lines(el: &TextEl) -> Vec<Vec<u16>> {
    lines(el).iter().map(|l| l.encode_utf16().collect()).collect()
}

impl TextService for GoldText {
    fn height(&self, el: &TextEl, w: f64) -> f64 {
        gold_lines(el).iter().map(|l| ((l.len() as f64 * 10.0) / w.max(1.0)).ceil().max(1.0) * 20.0).sum()
    }
    fn offset_at(&self, el: &TextEl, x: f64, y: f64) -> i64 {
        let ls = gold_lines(el);
        if ls.is_empty() {
            return 0;
        }
        let li = ((y / 20.0).floor().clamp(0.0, (ls.len() - 1) as f64)) as usize;
        let off: i64 = ls.iter().take(li).map(|l| l.len() as i64 + 1).sum();
        off + (oc_schema::js::round(x / 10.0)).clamp(0.0, ls[li].len() as f64) as i64
    }
    fn vertical_move(&self, el: &TextEl, o: i64, dir: i32) -> i64 {
        let ls = gold_lines(el);
        if ls.is_empty() {
            return 0;
        }
        let (mut li, mut start) = (0usize, 0i64);
        while li < ls.len() - 1 && o > start + ls[li].len() as i64 {
            start += ls[li].len() as i64 + 1;
            li += 1;
        }
        let col = (o - start).max(0);
        let ti = li as i64 + dir as i64;
        if ti < 0 {
            return 0;
        }
        if ti >= ls.len() as i64 {
            return ls.iter().map(|l| l.len() as i64).sum::<i64>() + ls.len() as i64 - 1;
        }
        let ts: i64 = ls.iter().take(ti as usize).map(|l| l.len() as i64 + 1).sum();
        ts + col.min(ls[ti as usize].len() as i64)
    }
    fn word_at(&self, el: &TextEl, o: i64) -> (i64, i64) {
        let t: Vec<u16> = lines(el).join("\n").encode_utf16().collect();
        let c = o.clamp(0, t.len() as i64);
        let (mut a, mut b) = (c, c);
        while a > 0 && is_word(t[(a - 1) as usize]) {
            a -= 1;
        }
        while b < t.len() as i64 && is_word(t[b as usize]) {
            b += 1;
        }
        (a, b)
    }
    fn length(&self, el: &TextEl) -> i64 {
        u16len(&lines(el).join("\n"))
    }
    fn curved_size(&self, el: &TextEl) -> Option<(f64, f64)> {
        let n = u16len(&lines(el).join("\n")) as f64;
        Some(((n * 12.0).max(40.0), 60.0 + oc_schema::js::round(el.curve.unwrap_or(0.0).abs() / 5.0)))
    }
}

pub fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

/// A golden file from tests/golden/editor/.
pub fn golden(name: &str) -> Value {
    let p = root().join("tests/golden/editor").join(name);
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e} (run `pnpm tsx tools/golden/editor.ts`)", p.display()));
    serde_json::from_str(&s).expect("golden json")
}

/// Structural JSON comparison: objects order-insensitive, numbers within 1e-9 (relative above 1). Returns the
/// path and both values of the first difference.
pub fn json_diff(a: &Value, e: &Value, path: &str) -> Option<String> {
    match (a, e) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
            let tol = 1e-9 * x.abs().max(y.abs()).max(1.0);
            if (x - y).abs() <= tol { None } else { Some(format!("{path}: {x} != {y} (expected)")) }
        }
        (Value::Object(x), Value::Object(y)) => {
            for k in y.keys() {
                if !x.contains_key(k) {
                    return Some(format!("{path}.{k}: missing (expected {})", short(&y[k])));
                }
            }
            for k in x.keys() {
                if !y.contains_key(k) {
                    return Some(format!("{path}.{k}: unexpected {}", short(&x[k])));
                }
            }
            x.iter().find_map(|(k, v)| json_diff(v, &y[k], &format!("{path}.{k}")))
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: length {} != {} (expected)\n  actual:   {}\n  expected: {}", x.len(), y.len(), short(a), short(e)));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (p, q))| json_diff(p, q, &format!("{path}[{i}]")))
        }
        _ if a == e => None,
        _ => Some(format!("{path}: {} != {} (expected)", short(a), short(e))),
    }
}

pub fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 600 { format!("{}…", &s[..s.char_indices().take_while(|(i, _)| *i < 600).last().map_or(0, |(i, _)| i)]) } else { s }
}
