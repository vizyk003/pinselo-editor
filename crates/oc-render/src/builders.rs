// SPDX-License-Identifier: Apache-2.0
//! Display-list builders for compound elements: freehand strokes, tables, charts and forms
//! (`packages/render-core/src/builders.ts`, all pure).

use oc_schema::color::{hex, to_rgba};
use oc_schema::elements::{Brush, CellBorder, CellVAlign, ChartEl, ChartKind, DrawEl, FormEl, FormFieldKind, TableEl};
use oc_schema::factory::{form_layout_of, text_body};
use oc_schema::primitives::{ColorValue, Extra, Paint, Rect, Stroke, StrokeAlign, StrokeCap, StrokeJoin};
use oc_schema::table::{cell_body, cell_content_box, cell_fill, cell_key, cell_plain_text, cell_rect, geometry_of, table_anchors, table_edges};
use oc_schema::text::{RunStyleOverride, TextAlign};

use crate::displaylist::{DLItem, PathStroke, ResolvedPaint};
use crate::jsfmt::{cos, f, log10, max, max_of, min, num_to_string, pow, round, sin, sqrt, to_fixed, to_locale_fixed};
use crate::resolve::resolve_paint;

const PI: f64 = std::f64::consts::PI;

fn solid_p(c: ColorValue) -> Paint {
    Paint::solid_color(c)
}

fn stroke(color: &ColorValue, width: f64, align: StrokeAlign, cap: StrokeCap, join: StrokeJoin) -> Stroke {
    Stroke { paint: solid_p(color.clone()), width, align, cap, join, dash: None, extra: Extra::new() }
}

/// `{ ...c, a }`.
fn with_alpha(c: &ColorValue, a: f64) -> ColorValue {
    let mut c = c.clone();
    match &mut c {
        ColorValue::Srgb(x) | ColorValue::DisplayP3(x) => x.a = a,
        ColorValue::Cmyk(x) => x.a = a,
        ColorValue::Spot(x) => x.a = a,
    }
    c
}

fn hexc(s: &str) -> ColorValue {
    hex(s).expect("valid literal colour")
}

// ------------------------------------------------------------------ freehand

/// Smooth a polyline through midpoints with quadratic segments (stable, cheap, deterministic).
pub fn smooth_path(pts: &[[f64; 2]]) -> String {
    let Some(first) = pts.first() else { return String::new() };
    if pts.len() == 1 {
        let [x, y] = *first;
        return format!("M{} {} L{} {}", f(x), f(y), f(x + 0.01), f(y));
    }
    let mut d = format!("M{} {}", f(first[0]), f(first[1]));
    for i in 1..pts.len() - 1 {
        let [x, y] = pts[i];
        let [nx, ny] = pts[i + 1];
        d.push_str(&format!(" Q{} {} {} {}", f(x), f(y), f((x + nx) / 2.0), f((y + ny) / 2.0)));
    }
    let last = pts[pts.len() - 1];
    format!("{d} L{} {}", f(last[0]), f(last[1]))
}

fn or1(v: f64) -> f64 {
    if v == 0.0 || v.is_nan() { 1.0 } else { v }
}

pub fn draw_items(el: &DrawEl) -> Vec<DLItem> {
    let xf = &el.base.xf;
    let sx = xf.w / or1(el.base_size.w);
    let sy = xf.h / or1(el.base_size.h);
    let mut pts: Vec<[f64; 2]> = Vec::new();
    let mut i = 0;
    while i + 1 < el.points.len() {
        pts.push([el.points[i] * sx, el.points[i + 1] * sy]);
        i += 3;
    }
    let k = (sx + sy) / 2.0;
    let c = to_rgba(&el.color);
    let hl = el.brush == Brush::Highlighter;
    let color = ColorValue::srgb(c.r, c.g, c.b, if hl { 0.38 } else { 1.0 });
    let width = el.size * k * if hl { 3.0 } else if el.brush == Brush::Marker { 1.6 } else { 1.0 };
    let s = stroke(&color, width, StrokeAlign::Center, if hl { StrokeCap::Square } else { StrokeCap::Round }, StrokeJoin::Round);
    vec![DLItem::Path {
        path: smooth_path(&pts),
        fills: vec![],
        stroke: Some(PathStroke { stroke: s, paint: ResolvedPaint::Solid { color } }),
        src_el: el.base.id.clone(),
    }]
}

// ------------------------------------------------------------------ tables

/// Table display list: cell fills (merged cells as one box), rich cell text with per-cell padding and vertical
/// alignment, then border segments grouped by width/colour. Covered cells draw nothing.
pub fn table_items(el: &TableEl) -> Vec<DLItem> {
    let mut out = Vec::new();
    let geo = geometry_of(el);
    let rect = |x: f64, y: f64, w: f64, h: f64| format!("M{} {} L{} {} L{} {} L{} {} Z", f(x), f(y), f(x + w), f(y), f(x + w), f(y + h), f(x), f(y + h));
    let anchors = table_anchors(el);
    let id = &el.base.id;
    for a in &anchors {
        let b = cell_rect(el, a.r, a.c, Some(&geo));
        if let Some(fill) = cell_fill(el, a.r, a.c) {
            out.push(DLItem::Path { path: rect(b.x, b.y, b.w, b.h), fills: vec![resolve_paint(&fill, b)], stroke: None, src_el: id.clone() });
        }
    }
    for a in &anchors {
        if cell_plain_text(el, a.r, a.c).is_empty() {
            continue;
        }
        let valign = el.cells.get(&cell_key(a.r, a.c)).and_then(|c| c.valign.clone()).unwrap_or(CellVAlign::Middle);
        out.push(DLItem::Text {
            key: format!("{id}:{}:{}", a.r, a.c),
            body: cell_body(el, a.r, a.c),
            bbox: cell_content_box(el, a.r, a.c, Some(&geo)),
            effect: None,
            valign: Some(valign),
            curve: None,
            fit: None,
            src_el: id.clone(),
        });
    }
    let mut groups: Vec<(String, CellBorder, String)> = Vec::new();
    for e in table_edges(el, Some(&geo)) {
        let k = format!("{}|{}", num_to_string(e.border.width), oc_schema::js::to_string(&e.border.color).unwrap_or_default());
        let seg = format!("M{} {} L{} {} ", f(e.x1), f(e.y1), f(e.x2), f(e.y2));
        match groups.iter_mut().find(|g| g.0 == k) {
            Some(g) => g.2.push_str(&seg),
            None => groups.push((k, e.border.clone(), seg)),
        }
    }
    for (_, border, d) in groups {
        let s = stroke(&border.color, border.width, StrokeAlign::Center, StrokeCap::Square, StrokeJoin::Miter);
        out.push(DLItem::Path {
            path: d.trim_end().to_string(),
            fills: vec![],
            stroke: Some(PathStroke { stroke: s, paint: ResolvedPaint::Solid { color: border.color } }),
            src_el: id.clone(),
        });
    }
    out
}

// ------------------------------------------------------------------ charts

/// "Nice" axis maximum and step (1, 2, 2.5, 5 × 10^n).
pub fn nice_scale(m: f64, ticks: f64) -> (f64, f64) {
    if m.is_nan() || m <= 0.0 {
        return (1.0, 0.2);
    }
    let raw = m / ticks;
    let mag = pow(10.0, log10(raw).floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|x| x * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
    ((m / step).ceil() * step, step)
}

/// `fmt`: compact axis numbers (1.2M, 3.4k, 0.25).
fn fmt(v: f64) -> String {
    if v.abs() >= 1e6 {
        return format!("{}M", num_to_string(oc_schema::js::to_fixed_num(v / 1e6, 1)));
    }
    if v.abs() >= 1e3 {
        return format!("{}k", num_to_string(oc_schema::js::to_fixed_num(v / 1e3, 1)));
    }
    num_to_string(oc_schema::js::to_fixed_num(v, 2))
}

const PIE: [&str; 10] = ["#7c3aed", "#06b6d4", "#f59e0b", "#ec4899", "#10b981", "#3b82f6", "#ef4444", "#84cc16", "#a855f7", "#14b8a6"];

pub fn slice_color(el: &ChartEl, i: usize) -> ColorValue {
    if let Some(own) = el.style.slice_colors.as_ref().and_then(|v| v.get(i)) {
        return own.clone();
    }
    if i == 0
        && let Some(s) = el.series.first()
    {
        return s.color.clone();
    }
    hexc(PIE[i % PIE.len()])
}

/// Digits argument of `toFixed`/`toLocaleString` (`ToIntegerOrInfinity`, clamped to the allowed range).
fn digits(d: f64) -> usize {
    if d.is_nan() { 0 } else { d.trunc().clamp(0.0, 100.0) as usize }
}

struct Chart<'a> {
    el: &'a ChartEl,
    out: Vec<DLItem>,
}

impl Chart<'_> {
    #[allow(clippy::too_many_arguments)]
    fn label(&mut self, key: &str, text: &str, b: Rect, align: TextAlign, color: Option<&ColorValue>, weight: f64, size: Option<f64>) {
        let st = &self.el.style;
        let style = RunStyleOverride {
            family: Some(st.family.clone()),
            size: Some(size.unwrap_or(st.font_size)),
            weight: Some(weight),
            fill: Some(solid_p(color.unwrap_or(&st.text_color).clone())),
            ..Default::default()
        };
        self.out.push(DLItem::Text {
            key: format!("{}:{key}", self.el.base.id),
            body: text_body(text, &style, align),
            bbox: b,
            effect: None,
            valign: Some(CellVAlign::Middle),
            curve: None,
            fit: None,
            src_el: self.el.base.id.clone(),
        });
    }

    fn fill_path(&mut self, path: String, color: &ColorValue, alpha: f64) {
        let c = to_rgba(color);
        self.out.push(DLItem::Path {
            path,
            fills: vec![ResolvedPaint::Solid { color: ColorValue::srgb(c.r, c.g, c.b, c.a * alpha) }],
            stroke: None,
            src_el: self.el.base.id.clone(),
        });
    }

    fn stroke_path(&mut self, path: String, color: &ColorValue, width: f64, cap: StrokeCap) {
        let s = stroke(color, width, StrokeAlign::Center, cap, StrokeJoin::Round);
        self.out.push(DLItem::Path {
            path,
            fills: vec![],
            stroke: Some(PathStroke { stroke: s, paint: ResolvedPaint::Solid { color: color.clone() } }),
            src_el: self.el.base.id.clone(),
        });
    }

    /// Value labels: `vf`.
    fn vf(&self, v: f64) -> String {
        let Some(fm) = &self.el.style.format else { return fmt(v) };
        let d = fm.decimals.unwrap_or(if oc_schema::js::is_integer(v) { 0.0 } else { 1.0 });
        let n = if fm.thousands == Some(false) { to_fixed(v, digits(d)) } else { to_locale_fixed(v, digits(d)) };
        format!("{}{n}{}", fm.prefix.as_deref().unwrap_or(""), fm.suffix.as_deref().unwrap_or(""))
    }
}

fn dot(x: f64, y: f64, r: f64) -> String {
    format!("M{} {} a{} {} 0 1 0 {} 0 a{} {} 0 1 0 {} 0", f(x - r), f(y), f(r), f(r), f(r * 2.0), f(r), f(r), f(-r * 2.0))
}

fn arc(cx: f64, cy: f64, r: f64, a0: f64, a1: f64) -> String {
    let large = if (a1 - a0).abs() > PI { 1 } else { 0 };
    format!(
        "M{} {} A{} {} 0 {large} 1 {} {}",
        f(cx + cos(a0) * r),
        f(cy + sin(a0) * r),
        f(r),
        f(r),
        f(cx + cos(a1) * r),
        f(cy + sin(a1) * r)
    )
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

pub fn chart_items(el: &ChartEl) -> Vec<DLItem> {
    let mut c = Chart { el, out: Vec::new() };
    let (w, h) = (el.base.xf.w, el.base.xf.h);
    let st = &el.style;
    let fs = st.font_size;
    let text_color = &st.text_color;
    let grid_color = &st.grid_color;
    let white = hexc("#ffffff");
    let has_fm = st.format.is_some();
    let kind = &el.kind;
    let first_label = el.labels.first().filter(|l| !l.is_empty());

    // ---- single-value kinds: progress ring / bar, gauge
    if matches!(kind, ChartKind::ProgressRing | ChartKind::ProgressBar | ChartKind::Gauge) {
        let v = max(0.0, el.series.first().and_then(|s| s.values.first().copied()).unwrap_or(0.0));
        let mx = st.max.unwrap_or(100.0);
        let k = min(1.0, v / max(1e-9, mx));
        let color = el.series.first().map_or_else(|| hexc("#7c3aed"), |s| s.color.clone());
        let text = if has_fm { c.vf(v) } else { format!("{}%", num_to_string(round(k * 100.0))) };
        if *kind == ChartKind::ProgressBar {
            let bh = max(6.0, min(h * 0.5, fs * 1.4));
            let y = h - bh;
            let rr = |x: f64, ww: f64| {
                format!(
                    "M{} {} h{} a{} {} 0 0 1 0 {} h{} a{} {} 0 0 1 0 {} Z",
                    f(x + bh / 2.0),
                    f(y),
                    f(max(0.0, ww - bh)),
                    f(bh / 2.0),
                    f(bh / 2.0),
                    f(bh),
                    f(-max(0.0, ww - bh)),
                    f(bh / 2.0),
                    f(bh / 2.0),
                    f(-bh)
                )
            };
            c.fill_path(rr(0.0, w), grid_color, 1.0);
            if k > 0.0 {
                c.fill_path(rr(0.0, max(bh, w * k)), &color, 1.0);
            }
            let l0 = el.labels.first().cloned().unwrap_or_default();
            c.label("pl", &l0, rect(0.0, 0.0, w * 0.7, y - 4.0), TextAlign::Start, Some(text_color), 600.0, None);
            c.label("pv", &text, rect(w * 0.5, 0.0, w * 0.5, y - 4.0), TextAlign::End, Some(text_color), 700.0, None);
            return c.out;
        }
        let cx = w / 2.0;
        if *kind == ChartKind::ProgressRing {
            let cy = h / 2.0;
            let r = max(4.0, min(w, h) / 2.0 - min(w, h) * 0.07);
            let sw = min(w, h) * 0.12;
            c.stroke_path(dot(cx, cy, r), grid_color, sw, StrokeCap::Round);
            if k > 0.001 {
                let p = if k >= 0.9999 { dot(cx, cy, r) } else { arc(cx, cy, r, -PI / 2.0, -PI / 2.0 + k * PI * 2.0) };
                c.stroke_path(p, &color, sw, StrokeCap::Round);
            }
            let lh = r * if first_label.is_some() { 0.75 } else { 1.0 };
            c.label("pv", &text, rect(cx - r, cy - r * 0.5, r * 2.0, lh), TextAlign::Center, Some(text_color), 800.0, Some(max(fs, r * 0.45)));
            if let Some(l) = first_label {
                c.label("pl", l, rect(cx - r * 0.8, cy + r * 0.25, r * 1.6, r * 0.4), TextAlign::Center, None, 400.0, None);
            }
            return c.out;
        }
        // gauge: a 240° dial with a needle.
        let r = max(4.0, min(w / 2.0, h * 0.62) - min(w, h) * 0.06);
        let cy = min(h - fs * 2.0, r + min(w, h) * 0.06 + 4.0);
        let sw = r * 0.18;
        let a0 = PI * (5.0 / 6.0);
        let a1 = PI * (13.0 / 6.0);
        c.stroke_path(arc(cx, cy, r, a0, a1), grid_color, sw, StrokeCap::Butt);
        if k > 0.001 {
            c.stroke_path(arc(cx, cy, r, a0, a0 + (a1 - a0) * k), &color, sw, StrokeCap::Butt);
        }
        let an = a0 + (a1 - a0) * k;
        c.stroke_path(
            format!("M{} {} L{} {}", f(cx), f(cy), f(cx + cos(an) * r * 0.78), f(cy + sin(an) * r * 0.78)),
            text_color,
            max(2.0, r * 0.04),
            StrokeCap::Round,
        );
        c.fill_path(dot(cx, cy, max(3.0, r * 0.07)), text_color, 1.0);
        c.label("pv", &text, rect(cx - r, cy + r * 0.18, r * 2.0, max(fs * 1.6, r * 0.35)), TextAlign::Center, Some(text_color), 800.0, Some(max(fs, r * 0.24)));
        if let Some(l) = first_label {
            c.label("pl", l, rect(0.0, h - fs * 1.6, w, fs * 1.5), TextAlign::Center, None, 400.0, None);
        }
        return c.out;
    }

    let pie = matches!(kind, ChartKind::Pie | ChartKind::Donut | ChartKind::Funnel);
    // Legend (top).
    let mut top = 0.0;
    let legend: Vec<(String, ColorValue)> = if pie {
        el.labels.iter().enumerate().map(|(i, l)| (l.clone(), slice_color(el, i))).collect()
    } else {
        let ser = if *kind == ChartKind::Bubble { &el.series[..el.series.len().min(1)] } else { &el.series[..] };
        ser.iter().map(|s| (s.name.clone(), s.color.clone())).collect()
    };
    if st.show_legend && !legend.is_empty() && *kind != ChartKind::Funnel {
        let n = legend.len() as f64;
        let item_w = min(w / n, fs * 9.0);
        let start_x = (w - item_w * n) / 2.0;
        for (i, (name, color)) in legend.iter().enumerate() {
            let x = start_x + i as f64 * item_w;
            c.fill_path(format!("M{} {} h{} v{} h{} Z", f(x), f(fs * 0.3), f(fs * 0.9), f(fs * 0.9), f(-fs * 0.9)), color, 1.0);
            c.label(&format!("lg{i}"), name, rect(x + fs * 1.3, 0.0, item_w - fs * 1.5, fs * 1.5), TextAlign::Start, None, 400.0, None);
        }
        top = fs * 2.2;
    }

    if *kind == ChartKind::Funnel {
        let vals: Vec<f64> = el.series.first().map_or(&[][..], |s| &s.values[..]).iter().take(el.labels.len()).map(|v| max(0.0, *v)).collect();
        let mx = max(1e-9, max_of(vals.iter().copied()));
        let gap = max(2.0, h * 0.015);
        let bh = (h - gap * (vals.len() as f64 - 1.0)) / max(1.0, vals.len() as f64);
        for (i, &v) in vals.iter().enumerate() {
            let bw = max(fs * 3.0, (v / mx) * w);
            let nw = max(fs * 3.0, (vals.get(i + 1).copied().unwrap_or(v * 0.8) / mx) * w);
            let y = i as f64 * (bh + gap);
            let x0 = (w - bw) / 2.0;
            let x1 = (w - nw) / 2.0;
            c.fill_path(
                format!("M{} {} L{} {} L{} {} L{} {} Z", f(x0), f(y), f(x0 + bw), f(y), f(x1 + nw), f(y + bh), f(x1), f(y + bh)),
                &slice_color(el, i),
                1.0,
            );
            let txt = format!("{}{}", el.labels.get(i).map_or("", String::as_str), if st.show_values { format!(" · {}", c.vf(v)) } else { String::new() });
            c.label(&format!("fl{i}"), &txt, rect(0.0, y, w, bh), TextAlign::Center, Some(&white), 700.0, None);
        }
        return c.out;
    }

    if pie {
        let vals: Vec<f64> = el.series.first().map_or(&[][..], |s| &s.values[..]).iter().map(|v| max(0.0, *v)).collect();
        let sum = vals.iter().fold(0.0, |a, b| a + b);
        let total = if sum == 0.0 || sum.is_nan() { 1.0 } else { sum };
        let cx = w / 2.0;
        let cy = top + (h - top) / 2.0;
        let r = max(1.0, min(w, h - top) / 2.0 - 4.0);
        let r0 = if *kind == ChartKind::Donut { r * 0.55 } else { 0.0 };
        let mut a0 = -PI / 2.0;
        for (i, &v) in vals.iter().enumerate() {
            let a1 = a0 + (v / total) * PI * 2.0;
            let large = if a1 - a0 > PI { 1 } else { 0 };
            let p = |a: f64, rr: f64| format!("{} {}", f(cx + cos(a) * rr), f(cy + sin(a) * rr));
            let full = v / total > 0.9999;
            let path = if full {
                let mut s = format!("M{} {} A{} {} 0 1 1 {} {} A{} {} 0 1 1 {} {} Z", f(cx - r), f(cy), f(r), f(r), f(cx + r), f(cy), f(r), f(r), f(cx - r), f(cy));
                if r0 != 0.0 {
                    s.push_str(&format!(
                        " M{} {} A{} {} 0 1 0 {} {} A{} {} 0 1 0 {} {} Z",
                        f(cx - r0),
                        f(cy),
                        f(r0),
                        f(r0),
                        f(cx + r0),
                        f(cy),
                        f(r0),
                        f(r0),
                        f(cx - r0),
                        f(cy)
                    ));
                }
                s
            } else if r0 != 0.0 {
                format!("M{} A{} {} 0 {large} 1 {} L{} A{} {} 0 {large} 0 {} Z", p(a0, r), f(r), f(r), p(a1, r), p(a1, r0), f(r0), f(r0), p(a0, r0))
            } else {
                format!("M{} {} L{} A{} {} 0 {large} 1 {} Z", f(cx), f(cy), p(a0, r), f(r), f(r), p(a1, r))
            };
            if v > 0.0 {
                c.fill_path(path, &slice_color(el, i), 1.0);
            }
            if st.show_values && v / total > 0.04 {
                let am = (a0 + a1) / 2.0;
                let half = (r + r0) / 2.0;
                let rr = if half == 0.0 || half.is_nan() { r * 0.6 } else { half };
                let txt = if has_fm { c.vf(v) } else { format!("{}%", num_to_string(round((v / total) * 100.0))) };
                c.label(
                    &format!("pv{i}"),
                    &txt,
                    rect(cx + cos(am) * rr - fs * 2.5, cy + sin(am) * rr - fs * 0.8, fs * 5.0, fs * 1.6),
                    TextAlign::Center,
                    Some(&white),
                    700.0,
                    None,
                );
            }
            a0 = a1;
        }
        return c.out;
    }

    let n = el.labels.len();
    let nf = n as f64;

    if *kind == ChartKind::Radar {
        let cx = w / 2.0;
        let cy = top + (h - top) / 2.0;
        let r = max(4.0, min(w, h - top) / 2.0 - fs * 2.2);
        let (smax, sstep) = nice_scale(max(0.0, max_of(el.series.iter().flat_map(|s| s.values.iter().take(n).copied()))), 4.0);
        let ang = |i: usize| -PI / 2.0 + (i as f64 / max(1.0, nf)) * PI * 2.0;
        let ticks = round(smax / sstep);
        let mut t = 1.0;
        while t <= ticks {
            let rr = (t / ticks) * r;
            let mut path: String = (0..n).map(|i| format!("{}{} {}", if i > 0 { "L" } else { "M" }, f(cx + cos(ang(i)) * rr), f(cy + sin(ang(i)) * rr))).collect::<Vec<_>>().join(" ");
            path.push_str(" Z");
            c.stroke_path(path, grid_color, 1.0, StrokeCap::Round);
            t += 1.0;
        }
        for (i, l) in el.labels.iter().enumerate() {
            c.stroke_path(format!("M{} {} L{} {}", f(cx), f(cy), f(cx + cos(ang(i)) * r), f(cy + sin(ang(i)) * r)), grid_color, 1.0, StrokeCap::Round);
            let lx = cx + cos(ang(i)) * (r + fs * 1.1);
            let ly = cy + sin(ang(i)) * (r + fs * 1.1);
            c.label(&format!("rl{i}"), l, rect(lx - fs * 4.0, ly - fs * 0.8, fs * 8.0, fs * 1.6), TextAlign::Center, None, 400.0, None);
        }
        for s in &el.series {
            let pts: Vec<(f64, f64)> = s
                .values
                .iter()
                .take(n)
                .enumerate()
                .map(|(i, v)| (cx + cos(ang(i)) * (max(0.0, *v) / smax) * r, cy + sin(ang(i)) * (max(0.0, *v) / smax) * r))
                .collect();
            let mut path: String = pts.iter().enumerate().map(|(i, (x, y))| format!("{}{} {}", if i > 0 { "L" } else { "M" }, f(*x), f(*y))).collect::<Vec<_>>().join(" ");
            path.push_str(" Z");
            c.fill_path(path.clone(), &s.color, 0.25);
            c.stroke_path(path, &s.color, max(2.0, fs * 0.16), StrokeCap::Round);
        }
        return c.out;
    }

    // ---- cartesian charts
    let stacked = matches!(kind, ChartKind::StackedColumn | ChartKind::StackedBar);
    let xy = matches!(kind, ChartKind::Scatter | ChartKind::Bubble);
    let ser = if *kind == ChartKind::Bubble { &el.series[..el.series.len().min(1)] } else { &el.series[..] };
    let all: Vec<f64> = if stacked {
        (0..n).map(|i| el.series.iter().fold(0.0, |a, s| a + max(0.0, s.values.get(i).copied().unwrap_or(0.0)))).collect()
    } else {
        ser.iter().flat_map(|s| s.values.iter().take(n).copied()).collect()
    };
    let (smax, sstep) = nice_scale(max(0.0, max_of(all.iter().copied())), 5.0);
    let horizontal = matches!(kind, ChartKind::Bar | ChartKind::StackedBar);
    let axis_w = if horizontal { min(w * 0.3, fs * 6.0) } else { fs * 3.2 };
    let axis_h = fs * 2.0;
    let plot = rect(axis_w, top + fs * 0.5, max(1.0, w - axis_w - fs * 0.5), max(1.0, h - top - axis_h - fs * 0.5));
    let ticks = round(smax / sstep);
    let mut i = 0.0;
    while i <= ticks {
        let v = i * sstep;
        let ii = num_to_string(i);
        if horizontal {
            let x = plot.x + (v / smax) * plot.w;
            if st.show_grid {
                c.stroke_path(format!("M{} {} L{} {}", f(x), f(plot.y), f(x), f(plot.y + plot.h)), grid_color, 1.0, StrokeCap::Round);
            }
            let t = c.vf(v);
            c.label(&format!("ax{ii}"), &t, rect(x - fs * 2.5, plot.y + plot.h + fs * 0.3, fs * 5.0, fs * 1.4), TextAlign::Center, None, 400.0, None);
        } else {
            let y = plot.y + plot.h - (v / smax) * plot.h;
            if st.show_grid {
                c.stroke_path(format!("M{} {} L{} {}", f(plot.x), f(y), f(plot.x + plot.w), f(y)), grid_color, 1.0, StrokeCap::Round);
            }
            let t = c.vf(v);
            c.label(&format!("ay{ii}"), &t, rect(0.0, y - fs * 0.75, axis_w - fs * 0.5, fs * 1.5), TextAlign::End, None, 400.0, None);
        }
        i += 1.0;
    }

    if xy {
        // x from the labels when they are numbers (else 1…n); bubble size from the second series.
        let xs: Vec<f64> = el
            .labels
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let v = crate::jsfmt::parse_float(l);
                if v.is_finite() { v } else { (i + 1) as f64 }
            })
            .collect();
        let (xmax, xstep) = nice_scale(max(0.0, max_of(xs.iter().copied())), 5.0);
        let xt = round(xmax / xstep);
        let mut i = 0.0;
        while i <= xt {
            let x = plot.x + ((i * xstep) / xmax) * plot.w;
            if st.show_grid && i != 0.0 {
                c.stroke_path(format!("M{} {} L{} {}", f(x), f(plot.y), f(x), f(plot.y + plot.h)), grid_color, 1.0, StrokeCap::Round);
            }
            c.label(&format!("xx{}", num_to_string(i)), &fmt(i * xstep), rect(x - fs * 2.5, plot.y + plot.h + fs * 0.3, fs * 5.0, fs * 1.4), TextAlign::Center, None, 400.0, None);
            i += 1.0;
        }
        let sizes: Vec<f64> = if *kind == ChartKind::Bubble { el.series.get(1).map_or_else(Vec::new, |s| s.values.clone()) } else { Vec::new() };
        let smx = max(1e-9, max_of(sizes.iter().map(|v| v.abs())));
        for (si, s) in ser.iter().enumerate() {
            for (i, &v) in s.values.iter().take(n).enumerate() {
                let x = plot.x + (xs[i] / xmax) * plot.w;
                let y = plot.y + plot.h - (max(0.0, v) / smax) * plot.h;
                let r = if *kind == ChartKind::Bubble {
                    max(fs * 0.3, sqrt(sizes.get(i).copied().unwrap_or(0.0).abs() / smx) * min(plot.w, plot.h) * 0.12)
                } else {
                    fs * 0.35
                };
                c.fill_path(dot(x, y, r), &s.color, if *kind == ChartKind::Bubble { 0.7 } else { 1.0 });
                if st.show_values {
                    let t = c.vf(v);
                    c.label(&format!("v{si}:{i}"), &t, rect(x - fs * 2.0, y - r - fs * 1.5, fs * 4.0, fs * 1.4), TextAlign::Center, Some(text_color), 600.0, Some(fs * 0.85));
                }
            }
        }
        return c.out;
    }

    let band = (if horizontal { plot.h } else { plot.w }) / max(1.0, nf);
    for (i, l) in el.labels.iter().enumerate() {
        let fi = i as f64;
        if horizontal {
            c.label(&format!("cl{i}"), l, rect(0.0, plot.y + fi * band, axis_w - fs * 0.6, band), TextAlign::End, None, 400.0, None);
        } else {
            c.label(&format!("cl{i}"), l, rect(plot.x + fi * band, plot.y + plot.h + fs * 0.3, band, fs * 1.4), TextAlign::Center, None, 400.0, None);
        }
    }
    let bar_path = |off: f64, bw: f64, from: f64, len: f64, round_: bool| -> String {
        let rad = if round_ { min(min(bw * 0.25, 6.0), len) } else { 0.0 };
        if horizontal {
            format!(
                "M{} {} h{} q{} 0 {} {} v{} q0 {} {} {} h{} Z",
                f(plot.x + from),
                f(plot.y + off),
                f(max(0.0, len - rad)),
                f(rad),
                f(rad),
                f(rad),
                f(max(0.0, bw - 2.0 * rad)),
                f(rad),
                f(-rad),
                f(rad),
                f(-max(0.0, len - rad))
            )
        } else {
            format!(
                "M{} {} v{} q0 {} {} {} h{} q{} 0 {} {} v{} Z",
                f(plot.x + off),
                f(plot.y + plot.h - from),
                f(-max(0.0, len - rad)),
                f(-rad),
                f(rad),
                f(-rad),
                f(max(0.0, bw - 2.0 * rad)),
                f(rad),
                f(rad),
                f(rad),
                f(max(0.0, len - rad))
            )
        }
    };
    let span = if horizontal { plot.w } else { plot.h };

    if stacked {
        let bw = band * 0.62;
        for i in 0..n {
            let fi = i as f64;
            let mut acc = 0.0;
            let last = el.series.iter().rposition(|s| s.values.get(i).copied().unwrap_or(0.0) > 0.0);
            for (si, s) in el.series.iter().enumerate() {
                let v = max(0.0, s.values.get(i).copied().unwrap_or(0.0));
                let len = (v / smax) * span;
                let off = fi * band + (band - bw) / 2.0;
                if len > 0.5 {
                    c.fill_path(bar_path(off, bw, (acc / smax) * span, len, Some(si) == last), &s.color, 1.0);
                }
                if st.show_values && len > fs * 1.4 {
                    let mid = ((acc + v / 2.0) / smax) * span;
                    let t = c.vf(v);
                    let b = if horizontal {
                        rect(plot.x + mid - fs * 2.0, plot.y + off, fs * 4.0, bw)
                    } else {
                        rect(plot.x + off, plot.y + plot.h - mid - fs * 0.7, bw, fs * 1.4)
                    };
                    c.label(&format!("v{si}:{i}"), &t, b, TextAlign::Center, Some(&white), 700.0, Some(fs * 0.85));
                }
                acc += v;
            }
        }
        return c.out;
    }

    let columns: &[_] = match kind {
        ChartKind::Column | ChartKind::Bar => &el.series[..],
        ChartKind::Combo => &el.series[..el.series.len().min(1)],
        _ => &[],
    };
    let lines: &[_] = match kind {
        ChartKind::Line | ChartKind::Area => &el.series[..],
        ChartKind::Combo => &el.series[el.series.len().min(1)..],
        _ => &[],
    };
    if !columns.is_empty() {
        let k = columns.len() as f64;
        let inner = band * 0.72;
        let bw = inner / k;
        for (si, s) in columns.iter().enumerate() {
            for (i, &v) in s.values.iter().take(n).enumerate() {
                let len = (max(0.0, v) / smax) * span;
                let off = i as f64 * band + (band - inner) / 2.0 + si as f64 * bw;
                if len > 0.5 {
                    c.fill_path(bar_path(off, bw, 0.0, len, true), &s.color, 1.0);
                }
                if st.show_values {
                    let t = c.vf(v);
                    if horizontal {
                        c.label(&format!("v{si}:{i}"), &t, rect(plot.x + len + 4.0, plot.y + off, fs * 4.0, bw), TextAlign::Start, Some(text_color), 600.0, Some(fs * 0.85));
                    } else {
                        c.label(
                            &format!("v{si}:{i}"),
                            &t,
                            rect(plot.x + off - fs, plot.y + plot.h - len - fs * 1.5, bw + fs * 2.0, fs * 1.4),
                            TextAlign::Center,
                            Some(text_color),
                            600.0,
                            Some(fs * 0.85),
                        );
                    }
                }
            }
        }
    }
    for (li, s) in lines.iter().enumerate() {
        let si = columns.len() + li;
        let pts: Vec<(f64, f64)> =
            s.values.iter().take(n).enumerate().map(|(i, v)| (plot.x + i as f64 * band + band / 2.0, plot.y + plot.h - (max(0.0, *v) / smax) * plot.h)).collect();
        if pts.is_empty() {
            continue;
        }
        let line = pts.iter().enumerate().map(|(i, (x, y))| format!("{}{} {}", if i > 0 { "L" } else { "M" }, f(*x), f(*y))).collect::<Vec<_>>().join(" ");
        if *kind == ChartKind::Area {
            let last = pts[pts.len() - 1];
            c.fill_path(format!("{line} L{} {} L{} {} Z", f(last.0), f(plot.y + plot.h), f(pts[0].0), f(plot.y + plot.h)), &s.color, 0.28);
        }
        c.stroke_path(line, &s.color, max(2.0, fs * 0.2), StrokeCap::Round);
        for (i, &(x, y)) in pts.iter().enumerate() {
            c.fill_path(dot(x, y, fs * 0.3), &s.color, 1.0);
            if st.show_values {
                let t = c.vf(s.values[i]);
                c.label(&format!("v{si}:{i}"), &t, rect(x - fs * 2.0, y - fs * 1.8, fs * 4.0, fs * 1.4), TextAlign::Center, Some(text_color), 600.0, Some(fs * 0.85));
            }
        }
    }
    c.out
}

// ------------------------------------------------------------------ forms

pub fn form_items(el: &FormEl) -> Vec<DLItem> {
    let mut out = Vec::new();
    let st = &el.style;
    let id = &el.base.id;
    let rows = form_layout_of(el).rows;
    let rr = |x: f64, y: f64, w: f64, h: f64, r: f64| {
        let k = min(min(r, w / 2.0), h / 2.0);
        format!(
            "M{} {} L{} {} Q{} {} {} {} L{} {} Q{} {} {} {} L{} {} Q{} {} {} {} L{} {} Q{} {} {} {} Z",
            f(x + k),
            f(y),
            f(x + w - k),
            f(y),
            f(x + w),
            f(y),
            f(x + w),
            f(y + k),
            f(x + w),
            f(y + h - k),
            f(x + w),
            f(y + h),
            f(x + w - k),
            f(y + h),
            f(x + k),
            f(y + h),
            f(x),
            f(y + h),
            f(x),
            f(y + h - k),
            f(x),
            f(y + k),
            f(x),
            f(y),
            f(x + k),
            f(y)
        )
    };
    let border = stroke(&st.border_color, 1.5, StrokeAlign::Inside, StrokeCap::Butt, StrokeJoin::Miter);
    let border_stroke = || Some(PathStroke { stroke: border.clone(), paint: ResolvedPaint::Solid { color: st.border_color.clone() } });
    let muted = with_alpha(&st.color, 0.5);
    let style = |size: f64, weight: Option<f64>, color: &ColorValue| RunStyleOverride {
        family: Some(st.family.clone()),
        size: Some(size),
        weight,
        fill: Some(solid_p(color.clone())),
        ..Default::default()
    };
    let text = |key: String, b: Rect, valign: Option<CellVAlign>, body| DLItem::Text { key, body, bbox: b, effect: None, valign, curve: None, fit: None, src_el: id.clone() };
    for r in &rows {
        let t = r.text.clone().unwrap_or_default();
        let fid = r.field_id.as_deref().unwrap_or("undefined");
        let b = rect(r.x, r.y, r.w, r.h);
        match r.kind {
            "title" => out.push(text(format!("{id}:title"), b, None, text_body(&t, &style(st.size * 1.25, Some(700.0), &st.color), TextAlign::Start))),
            "label" => out.push(text(format!("{id}:l:{fid}"), b, None, text_body(&t, &style(st.size * 0.85, Some(600.0), &st.color), TextAlign::Start))),
            "input" => {
                out.push(DLItem::Path {
                    path: rr(r.x, r.y, r.w, r.h, st.radius),
                    fills: vec![ResolvedPaint::Solid { color: st.field_fill.clone() }],
                    stroke: border_stroke(),
                    src_el: id.clone(),
                });
                if !t.is_empty() {
                    out.push(text(
                        format!("{id}:p:{fid}"),
                        rect(r.x + st.size * 0.6, r.y, r.w - st.size * 1.2, min(r.h, st.size * 2.4)),
                        Some(CellVAlign::Middle),
                        text_body(&t, &style(st.size * 0.85, None, &muted), TextAlign::Start),
                    ));
                }
            }
            "option" => {
                let field = el.fields.iter().find(|x| Some(x.id.as_str()) == r.field_id.as_deref());
                let bs = st.size * 1.1;
                let by = r.y + (r.h - bs) / 2.0;
                let path = if field.is_some_and(|x| x.kind == FormFieldKind::Choice) { rr(r.x, by, bs, bs, bs / 2.0) } else { rr(r.x, by, bs, bs, bs * 0.2) };
                out.push(DLItem::Path { path, fills: vec![ResolvedPaint::Solid { color: st.field_fill.clone() }], stroke: border_stroke(), src_el: id.clone() });
                let opt = r.option.as_deref().unwrap_or("undefined");
                out.push(text(
                    format!("{id}:o:{fid}:{opt}"),
                    rect(r.x + bs + st.size * 0.6, r.y, r.w - bs - st.size * 0.6, r.h),
                    Some(CellVAlign::Middle),
                    text_body(&t, &style(st.size * 0.9, None, &st.color), TextAlign::Start),
                ));
            }
            "submit" => {
                out.push(DLItem::Path { path: rr(r.x, r.y, r.w, r.h, st.radius), fills: vec![ResolvedPaint::Solid { color: st.accent.clone() }], stroke: None, src_el: id.clone() });
                out.push(text(
                    format!("{id}:submit"),
                    b,
                    Some(CellVAlign::Middle),
                    text_body(&t, &style(st.size, Some(700.0), &ColorValue::srgb(1.0, 1.0, 1.0, 1.0)), TextAlign::Center),
                ));
            }
            _ => {}
        }
    }
    out
}
