// SPDX-License-Identifier: AGPL-3.0-only
//! Linked charts (A15): a sheet range → chart labels + series, and the refresh mapping applied to a ChartEl.
//!
//! Layout convention (what Excel/Sheets/Canva infer): series run down columns. When the first column holds
//! text it is the category labels; when the first row holds text it is the series names.

use oc_schema::elements::{ChartEl, ChartSeries, ChartSource};
use oc_schema::factory::CHART_COLORS;
use oc_schema::primitives::Extra;
use serde::{Deserialize, Serialize};

use crate::format::{CellValue, ValueKind, format_value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesData {
    pub name: String,
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ChartData {
    pub labels: Vec<String>,
    pub series: Vec<SeriesData>,
}

fn js_number(s: &str) -> f64 {
    oc_schema::js::string_to_number(s)
}

fn is_num(v: &CellValue) -> bool {
    match v {
        CellValue::Number(_) => true,
        CellValue::String(s) => !oc_schema::js::trim(s).is_empty() && js_number(s).is_finite(),
        _ => false,
    }
}

fn is_text(v: &CellValue) -> bool {
    matches!(v, CellValue::String(s) if !oc_schema::js::trim(s).is_empty()) && !is_num(v)
}

fn num(v: &CellValue) -> f64 {
    match v {
        CellValue::Number(n) => *n,
        CellValue::Bool(b) => f64::from(u8::from(*b)),
        CellValue::String(s) if is_num(v) => js_number(s),
        _ => 0.0,
    }
}

fn label(v: &CellValue) -> String {
    match v {
        CellValue::Number(_) => format_value(v, None, ValueKind::Number),
        CellValue::Null => String::new(),
        o => format_value(o, None, ValueKind::Number),
    }
}

/// A range of computed values → chart labels and series.
pub fn range_to_chart_data(values: &[Vec<CellValue>]) -> ChartData {
    let rows: Vec<&Vec<CellValue>> =
        values.iter().filter(|r| r.iter().any(|v| !matches!(v, CellValue::Null) && *v != CellValue::String(String::new()))).collect();
    if rows.is_empty() || rows[0].is_empty() {
        return ChartData::default();
    }
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let null = CellValue::Null;
    let at = |r: usize, c: usize| -> &CellValue { rows.get(r).and_then(|x| x.get(c)).unwrap_or(&null) };
    let body: Vec<&&Vec<CellValue>> = if rows.len() > 1 { rows[1..].iter().collect() } else { rows.iter().collect() };
    let label_col = width > 1 && body.iter().any(|r| is_text(r.first().unwrap_or(&null)));
    let first_data = usize::from(label_col);
    let head: Vec<&CellValue> = rows[0].iter().skip(first_data).collect();
    let header = rows.len() > 1 && head.iter().any(|v| is_text(v)) && head.iter().all(|v| !is_num(v));
    let r0 = usize::from(header);
    let labels: Vec<String> =
        (r0..rows.len()).map(|r| if label_col { label(at(r, 0)) } else { (r - r0 + 1).to_string() }).collect();
    let mut series = Vec::new();
    for c in first_data..width {
        let fallback = format!("Series {}", c - first_data + 1);
        let name = if header {
            let l = label(at(0, c));
            if l.is_empty() { fallback } else { l }
        } else {
            fallback
        };
        let vals = (r0..rows.len()).map(|r| num(at(r, c))).collect();
        series.push(SeriesData { name, values: vals });
    }
    ChartData { labels, series }
}

/// `hex(CHART_COLORS[i % n])`.
pub fn palette(i: usize) -> oc_schema::ColorValue {
    oc_schema::color::hex(CHART_COLORS[i % CHART_COLORS.len()]).expect("palette colours are valid")
}

/// The fields a refresh writes into a chart element.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartPatch {
    pub labels: Vec<String>,
    pub series: Vec<ChartSeries>,
    pub source: Option<ChartSource>,
}

/// Patch for a chart element with fresh data; existing series colours are kept (by position).
pub fn chart_patch(el: &ChartEl, data: &ChartData, source: Option<ChartSource>) -> ChartPatch {
    let series = data
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| ChartSeries {
            name: s.name.clone(),
            values: s.values.clone(),
            color: el.series.get(i).map_or_else(|| palette(i), |x| x.color.clone()),
            extra: Extra::new(),
        })
        .collect();
    ChartPatch { labels: data.labels.clone(), series, source }
}

impl ChartPatch {
    /// `{ ...el, ...patch }`.
    pub fn apply(&self, el: &mut ChartEl) {
        el.labels = self.labels.clone();
        el.series = self.series.clone();
        if let Some(s) = &self.source {
            el.source = Some(s.clone());
        }
    }
}

/// True when the patch would change what the chart shows.
pub fn chart_changed(el: &ChartEl, data: &ChartData) -> bool {
    if el.labels.len() != data.labels.len() || el.labels.iter().zip(&data.labels).any(|(a, b)| a != b) {
        return true;
    }
    if el.series.len() != data.series.len() {
        return true;
    }
    el.series.iter().zip(&data.series).any(|(s, d)| s.name != d.name || s.values.len() != d.values.len() || s.values.iter().zip(&d.values).any(|(a, b)| a != b))
}
