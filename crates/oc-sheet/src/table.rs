// SPDX-License-Identifier: AGPL-3.0-only
//! Sheet → table elements (A15): a range becomes an ordinary `TableEl` (so it can be pasted into any design),
//! and a whole sheet is paginated onto fixed pages of tables for PDF/PNG export and thumbnails through the
//! existing render pipeline.

use indexmap::IndexMap;
use oc_schema::elements::{CellAlign, CellSpan, CellVAlign, TableCell, TableEl, TableStyle};
use oc_schema::factory::{MetaInit, TableOpts, TextOpts, base_init, empty_design, make_table, make_text, place_all_elements, text_body};
use oc_schema::text::RunStyleOverride;
use oc_schema::tree::sorted_pages;
use oc_schema::{DesignDoc, Element, Extra, Paint, Surface, Xf};

use crate::a1::Range;
use crate::engine::SheetEngine;
use crate::format::{CellValue, format_value};
use crate::model::{CellFormat, TabSnap};

const BASE_SIZE: f64 = 13.0;

/// Options of [`range_to_table`].
#[derive(Debug, Clone, Default)]
pub struct TableOptions {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub scale: Option<f64>,
    pub family: Option<String>,
    /// Explicit row indexes (repeated frozen header rows); merges are not applied then.
    pub rows: Option<Vec<usize>>,
}

fn solid(s: &str) -> Option<Paint> {
    oc_schema::color::hex(s).ok().map(oc_schema::color::solid)
}

fn hexc(s: &str) -> oc_schema::ColorValue {
    oc_schema::color::hex(s).expect("literal colour")
}

/// `ALIGN[autoAlign(v, fmt)]`: an unknown stored alignment maps to no alignment.
fn cell_align(v: &CellValue, fmt: Option<&CellFormat>) -> Option<CellAlign> {
    match crate::format::auto_align(v, fmt) {
        "left" => Some(CellAlign::Start),
        "center" => Some(CellAlign::Center),
        "right" => Some(CellAlign::End),
        _ => None,
    }
}

fn cell_valign(fmt: Option<&CellFormat>) -> Option<CellVAlign> {
    match fmt.and_then(|f| f.valign.as_deref()).unwrap_or("bottom") {
        "top" => Some(CellVAlign::Top),
        "middle" => Some(CellVAlign::Middle),
        "bottom" => Some(CellVAlign::Bottom),
        _ => None,
    }
}

fn truthy_str(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|x| !x.is_empty())
}

fn truthy_num(n: Option<f64>) -> bool {
    n.is_some_and(|x| x != 0.0 && !x.is_nan())
}

/// A table element for `range` (or for an explicit list of row indexes via `opts.rows`, used to repeat frozen
/// header rows on every page). Sizes follow the sheet's column widths and row heights times `scale`.
pub fn range_to_table(engine: &SheetEngine, t: &TabSnap, range: &Range, opts: &TableOptions) -> TableEl {
    let scale = opts.scale.unwrap_or(1.0);
    let row_list: Vec<usize> = opts.rows.clone().unwrap_or_else(|| (range.r0..=range.r1).collect());
    let cols: Vec<usize> = (range.c0..=range.c1).collect();
    let hs: Vec<f64> = row_list.iter().map(|r| t.row_height(*r) * scale).collect();
    let ws: Vec<f64> = cols.iter().map(|c| t.col_width(*c) * scale).collect();
    let big_h: f64 = hs.iter().sum();
    let big_w: f64 = ws.iter().sum();
    let mut el = make_table(
        base_init(Xf::new(opts.x.unwrap_or(0.0), opts.y.unwrap_or(0.0), big_w, big_h, 0.0)),
        TableOpts { rows: Some(row_list.len()), cols: Some(cols.len()), data: None },
    );
    el.rows = hs.iter().map(|h| h / big_h).collect();
    el.cols = ws.iter().map(|w| w / big_w).collect();
    el.base.name = Some(t.name.clone());
    let family = opts.family.clone().unwrap_or_else(|| "Inter".into());
    el.style = TableStyle {
        family: family.clone(),
        size: BASE_SIZE * scale,
        color: hexc("#1f2328"),
        border_color: hexc("#dadce0"),
        border_width: scale.max(0.5),
        header_fill: None,
        header_color: None,
        stripe_fill: None,
        padding: 4.0 * scale,
        header_bold: None,
        preset: None,
        extra: Extra::new(),
    };
    let mut cells: IndexMap<String, TableCell> = IndexMap::new();
    let merges = t.merge_ranges();
    for (i, r) in row_list.iter().enumerate() {
        for (j, c) in cols.iter().enumerate() {
            let (r, c) = (*r, *c);
            let v = engine.value(&t.id, r, c);
            let fmt = t.fmt_at(r, c);
            let text = format_value(&v, fmt, engine.kind(&t.id, r, c));
            let mut cell = TableCell { text: text.clone(), align: cell_align(&v, fmt), valign: cell_valign(fmt), ..TableCell::default() };
            if let Some(f) = fmt {
                if f.b == Some(true) {
                    cell.bold = Some(true);
                }
                if truthy_str(&f.fill) {
                    cell.fill = solid(f.fill.as_deref().unwrap_or(""));
                }
                let styled = truthy_str(&f.color)
                    || f.i == Some(true)
                    || f.u == Some(true)
                    || f.s == Some(true)
                    || truthy_num(f.size)
                    || f.b == Some(true);
                if !text.is_empty() && styled {
                    let mut style = RunStyleOverride {
                        family: Some(family.clone()),
                        size: Some(f.size.unwrap_or(BASE_SIZE) * scale),
                        ..RunStyleOverride::default()
                    };
                    if truthy_str(&f.color) {
                        style.fill = solid(f.color.as_deref().unwrap_or(""));
                    }
                    if f.b == Some(true) {
                        style.weight = Some(700.0);
                    }
                    if f.i == Some(true) {
                        style.italic = Some(true);
                    }
                    if f.u == Some(true) {
                        style.underline = Some(true);
                    }
                    if f.s == Some(true) {
                        style.strike = Some(true);
                    }
                    let align = cell.align.as_ref().map_or(oc_schema::text::TextAlign::Start, CellAlign::to_text_align);
                    cell.body = Some(text_body(&text, &style, align));
                }
            }
            // Merges anchored here, clipped to the rows/cols this table shows (only with contiguous rows).
            if opts.rows.is_none()
                && let Some(m) = merges.iter().find(|x| x.r0 == r && x.c0 == c)
            {
                cell.span = Some(CellSpan {
                    rows: (m.r1.min(range.r1) as f64) - r as f64 + 1.0,
                    cols: (m.c1.min(range.c1) as f64) - c as f64 + 1.0,
                    extra: Extra::new(),
                });
            }
            if !cell.text.is_empty() || cell.fill.is_some() || cell.span.is_some() || cell.bold.is_some() {
                cells.insert(format!("{i}:{j}"), cell);
            }
        }
    }
    el.cells = cells;
    el
}

/// Split [0, n) into consecutive groups whose summed size fits `max` (each group has at least one item).
fn chunks(sizes: &[f64], max: f64) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut acc = 0.0;
    for (i, s) in sizes.iter().enumerate() {
        if i > start && acc + s > max {
            out.push((start, i - 1));
            start = i;
            acc = 0.0;
        }
        acc += s;
    }
    if !sizes.is_empty() {
        out.push((start, sizes.len() - 1));
    }
    out
}

/// Options of [`sheet_to_design`].
#[derive(Debug, Clone, Default)]
pub struct SheetLayoutOpts {
    pub title: String,
    pub w: f64,
    pub h: f64,
    pub margin: Option<f64>,
    pub max_pages: Option<usize>,
    /// Tab ids to include (all when `None`).
    pub tabs: Option<Vec<String>>,
}

fn or_default(v: f64, d: f64) -> f64 {
    if v == 0.0 || v.is_nan() { d } else { v }
}

/// Lay a sheet out on pages: every tab's used range, scaled to fit the page width (down to 50%, after which
/// columns continue on further pages), rows split across pages with frozen header rows repeated.
pub fn sheet_to_design(engine: &SheetEngine, opts: &SheetLayoutOpts) -> DesignDoc {
    let big_w = or_default(opts.w, 1123.0);
    let big_h = or_default(opts.h, 794.0);
    let m = opts.margin.unwrap_or(36.0);
    let max_pages = opts.max_pages.unwrap_or(300);
    let tabs: Vec<&TabSnap> = engine.tabs.iter().filter(|t| opts.tabs.as_ref().is_none_or(|ids| ids.contains(&t.id))).collect();
    let multi = tabs.len() > 1;
    let mut pages: Vec<Vec<Element>> = Vec::new();
    for t in tabs {
        let Some(used) = t.used_range() else { continue };
        let head = if multi { 30.0 } else { 0.0 };
        let avail_w = big_w - 2.0 * m;
        let avail_h = big_h - 2.0 * m - head;
        let widths: Vec<f64> = (0..=used.c1).map(|c| t.col_width(c)).collect();
        let total: f64 = widths.iter().sum();
        // `Math.max(0.5, Math.min(1, w))`, NaN included.
        let scale = (avail_w / total).clamp(0.5, 1.0);
        // `Math.min(frozenRows, r1 + 1, 5)` used as an array length (NaN / negatives → none).
        let frozen = if t.frozen_rows.is_nan() { 0.0 } else { t.frozen_rows.min((used.r1 + 1) as f64).min(5.0) };
        let frozen_n = frozen.max(0.0).floor() as usize;
        let frozen_h: f64 = (0..frozen_n).map(|r| t.row_height(r) * scale).sum();
        let scaled: Vec<f64> = widths.iter().map(|w| w * scale).collect();
        for (c0, c1) in chunks(&scaled, avail_w) {
            let mut r = 0usize;
            let mut first = true;
            while r <= used.r1 && pages.len() < max_pages {
                let mut rows: Vec<usize> = if first || frozen_n == 0 { Vec::new() } else { (0..frozen_n).collect() };
                let mut h = if first { 0.0 } else { frozen_h };
                loop {
                    rows.push(r);
                    h += t.row_height(r) * scale;
                    r += 1;
                    if !(r <= used.r1 && h + t.row_height(r) * scale <= avail_h) {
                        break;
                    }
                }
                let contiguous = rows.windows(2).all(|w| w[1] == w[0] + 1);
                let range = Range { r0: rows[0], c0, r1: rows[rows.len() - 1], c1 };
                let mut els: Vec<Element> = Vec::new();
                if multi {
                    let style = RunStyleOverride { size: Some(16.0), weight: Some(700.0), fill: solid("#374151"), ..RunStyleOverride::default() };
                    els.push(Element::Text(make_text(
                        base_init(Xf::new(m, m - 4.0, avail_w, 24.0, 0.0)),
                        TextOpts { text: Some(t.name.clone()), style: Some(style), align: None, body: None },
                    )));
                }
                let topts = TableOptions {
                    x: Some(m),
                    y: Some(m + head),
                    scale: Some(scale),
                    family: None,
                    rows: if contiguous { None } else { Some(rows.clone()) },
                };
                els.push(Element::Table(range_to_table(engine, t, &range, &topts)));
                pages.push(els);
                first = false;
            }
        }
    }
    if pages.is_empty() {
        pages.push(Vec::new());
    }
    let mut design = empty_design(&MetaInit {
        w: big_w,
        h: big_h,
        unit: None,
        title: Some(opts.title.clone()),
        surface: Some(Surface::Sheet),
        pages: Some(pages.len()),
    });
    let page_ids: Vec<String> = sorted_pages(&design.pages).iter().map(|p| p.id.to_string()).collect();
    for (i, mut els) in pages.into_iter().enumerate() {
        place_all_elements(&page_ids[i], &mut els, None);
        for e in els {
            let id = e.id().unwrap_or_default().to_string();
            design.elements.insert(id, e);
        }
    }
    design
}
