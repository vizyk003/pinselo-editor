// SPDX-License-Identifier: Apache-2.0
//! Docs blocks (DC-030): the custom Tiptap nodes a doc can hold besides text — shared by the editor, the
//! layout/export pipeline and tests, so every surface agrees on attribute names and defaults.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::color::hexc;
use crate::elements::{CHART_KINDS, ChartEl, ChartKind, ChartSeries, ChartStyle};
use crate::factory::{ChartOpts, base_init, make_chart};
use crate::js;
use crate::macros::string_enum;
use crate::primitives::{ColorValue, Extra, Xf};

/// Node names (`DOC_NODES`).
pub mod doc_nodes {
    pub const DESIGN_EMBED: &str = "designEmbed";
    pub const CHART: &str = "chart";
    pub const CALLOUT: &str = "callout";
    pub const TOC: &str = "tableOfContents";
    pub const PAGE_BREAK: &str = "pageBreak";
    pub const VIDEO_EMBED: &str = "videoEmbed";
}

/// Content width of the doc page (816 px page − 2 × 80 px margins). Block widths are ≤ this.
pub const DOC_CONTENT_WIDTH: f64 = 656.0;

string_enum! {
    pub enum CalloutTone { Info = "info", Success = "success", Warning = "warning", Danger = "danger", Note = "note" }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CalloutStyle {
    pub bg: &'static str,
    pub border: &'static str,
    pub icon: &'static str,
}

pub const CALLOUT_TONES: &[(&str, CalloutStyle)] = &[
    ("info", CalloutStyle { bg: "#eff6ff", border: "#3b82f6", icon: "ℹ️" }),
    ("success", CalloutStyle { bg: "#ecfdf5", border: "#10b981", icon: "✅" }),
    ("warning", CalloutStyle { bg: "#fffbeb", border: "#f59e0b", icon: "⚠️" }),
    ("danger", CalloutStyle { bg: "#fef2f2", border: "#ef4444", icon: "⛔" }),
    ("note", CalloutStyle { bg: "#f5f3ff", border: "#8b5cf6", icon: "💡" }),
];

pub fn callout_style(tone: &CalloutTone) -> CalloutStyle {
    CALLOUT_TONES.iter().find(|(k, _)| *k == tone.as_str()).map_or(CALLOUT_TONES[0].1, |(_, s)| *s)
}

/// Lenient tone attribute: a known tone name, else `info`.
pub fn callout_tone(v: Option<&Value>) -> CalloutTone {
    match v {
        Some(Value::String(s)) if CALLOUT_TONES.iter().any(|(k, _)| k == s) => CalloutTone::parse(s),
        _ => CalloutTone::Info,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocChartData {
    pub kind: ChartKind,
    pub labels: Vec<String>,
    pub series: Vec<ChartSeries>,
    pub style: ChartStyle,
}

pub fn default_doc_chart(kind: Option<ChartKind>) -> DocChartData {
    let el = make_chart(
        base_init(Xf::new(0.0, 0.0, 1.0, 1.0, 0.0)),
        ChartOpts { kind: Some(kind.unwrap_or(ChartKind::Column)), ..Default::default() },
    );
    let mut style = el.style;
    style.font_size = 14.0;
    DocChartData { kind: el.kind, labels: el.labels, series: el.series, style }
}

fn is_color(c: Option<&Value>) -> bool {
    matches!(c, Some(Value::Object(m)) if m.contains_key("space"))
}

/// `s?.prop` for an arbitrary JSON value.
fn prop<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m.get(k).filter(|x| !x.is_null()),
        _ => None,
    }
}

/// Lenient parse of the `data` attribute (a JSON string or object); falls back to defaults field by field.
///
/// Differences from TS (typed model): a series colour or style field that does not fit the typed shape falls back
/// to the default instead of being kept verbatim.
pub fn parse_doc_chart(raw: Option<&Value>) -> DocChartData {
    let parsed: Option<Value> = match raw {
        Some(Value::String(s)) => serde_json::from_str(s).ok(),
        Some(v) => Some(v.clone()),
        None => None,
    };
    let d = default_doc_chart(None);
    let o: Map<String, Value> = match parsed {
        Some(Value::Object(m)) => m,
        Some(Value::Array(_)) => Map::new(),
        _ => return d,
    };
    let kind = match o.get("kind") {
        Some(Value::String(k)) if CHART_KINDS.contains(&k.as_str()) => ChartKind::parse(k),
        _ => d.kind.clone(),
    };
    let labels = match o.get("labels") {
        Some(Value::Array(a)) => a
            .iter()
            .map(|l| if l.is_null() { String::new() } else { js::value_to_js_string(l) })
            .take(500)
            .collect(),
        _ => d.labels.clone(),
    };
    let series = match o.get("series") {
        Some(Value::Array(a)) => a
            .iter()
            .take(50)
            .enumerate()
            .map(|(i, s)| {
                let name = match prop(s, "name") {
                    Some(n) => js::value_to_js_string(n),
                    None => format!("Series {}", i + 1),
                };
                let values = match prop(s, "values") {
                    Some(Value::Array(vs)) => vs
                        .iter()
                        .take(500)
                        .map(|x| {
                            let n = js::value_to_number(x);
                            if n.is_finite() { n } else { 0.0 }
                        })
                        .collect(),
                    _ => vec![],
                };
                let fallback = || d.series.get(i % d.series.len()).map_or_else(|| hexc("#7c3aed"), |s| s.color.clone());
                let color = if is_color(prop(s, "color")) {
                    serde_json::from_value::<ColorValue>(prop(s, "color").cloned().unwrap_or(Value::Null))
                        .unwrap_or_else(|_| fallback())
                } else {
                    fallback()
                };
                ChartSeries { name, values, color, extra: Extra::new() }
            })
            .collect(),
        _ => d.series.clone(),
    };
    let style = match o.get("style") {
        Some(Value::Object(over)) => merge_style(&d.style, over),
        Some(Value::Array(_)) => d.style.clone(),
        _ => d.style.clone(),
    };
    DocChartData { kind, labels, series, style }
}

/// `{ ...base, ...over }` for chart styles; keys that would not fit the typed style are skipped.
fn merge_style(base: &ChartStyle, over: &Map<String, Value>) -> ChartStyle {
    let Ok(Value::Object(mut cur)) = serde_json::to_value(base) else { return base.clone() };
    for (k, v) in over {
        let mut next = cur.clone();
        next.insert(k.clone(), v.clone());
        if serde_json::from_value::<ChartStyle>(Value::Object(next.clone())).is_ok() {
            cur = next;
        }
    }
    serde_json::from_value(Value::Object(cur)).unwrap_or_else(|_| base.clone())
}

/// `JSON.stringify(d)`.
pub fn serialize_doc_chart(d: &DocChartData) -> String {
    js::to_string(d).unwrap_or_default()
}

/// A chart element for a doc chart block.
pub fn doc_chart_element(d: &DocChartData, bx: Xf) -> ChartEl {
    let mut el = make_chart(base_init(bx), ChartOpts { kind: Some(d.kind.clone()), ..Default::default() });
    el.labels = d.labels.clone();
    el.series = d.series.clone();
    el.style = d.style.clone();
    el
}

/// Clamp a block width (px at 100 % zoom) to the doc content width (`None` = `undefined`).
pub fn doc_block_width(w: Option<&Value>, fallback: Option<f64>) -> f64 {
    let n = w.map_or(f64::NAN, js::value_to_number);
    if n.is_finite() && n > 0.0 { 80f64.max(DOC_CONTENT_WIDTH.min(n)) } else { fallback.unwrap_or(DOC_CONTENT_WIDTH) }
}
