// SPDX-License-Identifier: Apache-2.0
//! Constructors with sane defaults. Every editor/AI/template path creates elements through these.
//!
//! TS `BaseInit` (`Partial<ElementBase> & { xf }`) is an `ElementBase` here: start from [`base_init`] and set any
//! field; an empty `id` is replaced by a fresh id. Colours are typed (`ColorValue` / `Paint`) instead of hex
//! strings; the defaults are the TS literals.

use std::sync::LazyLock;

use indexmap::IndexMap;
use serde::Serialize;

use crate::color::{hexc, solidc};
use crate::design::{CURRENT_SCHEMA, DesignDoc, DesignMeta, Dims, LAYOUT_VERSION, Page, Surface};
use crate::elements::{
    AutoFit, Brush, ChartEl, ChartKind, ChartSeries, ChartStyle, DrawEl, ElementBase, EmbedEl, EmbedProvider,
    FormEl, FormField, FormFieldKind, FormStyle, FrameClip, FrameContent, FrameEl, GridCell, GridEl, GroupEl,
    ImageEl, LineEl, LineMarker, MediaClip, QrEcc, QrEl, ShapeEl, ShapeKind, Size, TableCell, TableEl, TableStyle,
    TextEl, TextFrame, TextFrameMode, VideoEl,
};
use crate::frames::frame_library_item;
use crate::ids::{key_between, new_id};
use crate::js;
use crate::primitives::{ColorValue, Extra, LinkTarget, Paint, Rect, StoredRect, Stroke, StrokeAlign, StrokeCap, StrokeJoin, Unit, Xf};
use crate::text::{RunStyle, RunStyleOverride, TextAlign, TextBody, apply_override, default_paragraph, Run};

pub const DEFAULT_FONT: &str = "Inter";

pub static DEFAULT_RUN: LazyLock<RunStyle> = LazyLock::new(|| RunStyle {
    family: DEFAULT_FONT.into(),
    weight: 400.0,
    italic: false,
    size: 32.0,
    fill: solidc("#1f2328"),
    tracking: 0.0,
    underline: None,
    strike: None,
    case_transform: None,
    highlight: None,
    baseline: None,
    extra: Extra::new(),
});

/// The next key after `prev` (`keyBetween(prev, null, false)`); keys this crate generates are always valid.
fn next_key(prev: Option<&str>) -> String {
    key_between(prev, None, false).expect("valid order key")
}

/// `BaseInit` with only `xf` set (parent `''`, z `'a0'`, opacity 1).
pub fn base_init(xf: Xf) -> ElementBase {
    ElementBase { parent_id: String::new(), z: next_key(None), xf, opacity: 1.0, ..Default::default() }
}

fn base(mut init: ElementBase) -> ElementBase {
    if init.id.is_empty() {
        init.id = new_id();
    }
    init
}

pub fn xf(x: f64, y: f64, w: f64, h: f64, rot: f64) -> Xf {
    Xf::new(x, y, w, h, rot)
}

pub fn text_body(text: &str, style: &RunStyleOverride, align: TextAlign) -> TextBody {
    let p = default_paragraph();
    let attrs = crate::text::ParagraphAttrs { align, ..p };
    TextBody {
        default_run: apply_override(&DEFAULT_RUN, style),
        paragraphs: text
            .split('\n')
            .map(|line| attrs.with_runs(if line.is_empty() { vec![] } else { vec![Run::plain(line)] }))
            .collect(),
        extra: Extra::new(),
    }
}

#[derive(Debug, Clone, Default)]
pub struct TextOpts {
    pub text: Option<String>,
    pub style: Option<RunStyleOverride>,
    pub align: Option<TextAlign>,
    pub body: Option<TextBody>,
}

pub fn make_text(init: ElementBase, o: TextOpts) -> TextEl {
    let body = o.body.unwrap_or_else(|| {
        text_body(
            o.text.as_deref().unwrap_or("Add a heading"),
            &o.style.unwrap_or_default(),
            o.align.unwrap_or(TextAlign::Start),
        )
    });
    TextEl {
        base: base(init),
        body,
        frame: TextFrame { mode: TextFrameMode::AutoHeight, extra: Extra::new() },
        auto_fit: AutoFit::None,
        curve: None,
        effect: None,
        valign: None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct ShapeOpts {
    pub kind: Option<ShapeKind>,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
    pub params: Option<IndexMap<String, f64>>,
}

pub fn make_shape(init: ElementBase, o: ShapeOpts) -> ShapeEl {
    ShapeEl {
        base: base(init),
        kind: o.kind.unwrap_or(ShapeKind::Rect),
        params: o.params.unwrap_or_default(),
        d: None,
        fills: vec![o.fill.unwrap_or_else(|| solidc("#8b5cf6"))],
        strokes: o.stroke.into_iter().collect(),
    }
}

/// `makeStroke(color = '#1f2328', width = 4, dash?)`.
pub fn make_stroke(color: Option<ColorValue>, width: Option<f64>, dash: Option<Vec<f64>>) -> Stroke {
    Stroke {
        paint: Paint::solid_color(color.unwrap_or_else(|| hexc("#1f2328"))),
        width: width.unwrap_or(4.0),
        align: StrokeAlign::Center,
        cap: StrokeCap::Round,
        join: StrokeJoin::Round,
        dash,
        extra: Extra::new(),
    }
}

#[derive(Debug, Clone, Default)]
pub struct LineOpts {
    pub color: Option<ColorValue>,
    pub width: Option<f64>,
    pub dash: Option<Vec<f64>>,
    pub start_marker: Option<LineMarker>,
    pub end_marker: Option<LineMarker>,
}

pub fn make_line(init: ElementBase, o: LineOpts) -> LineEl {
    let e = base(init);
    let (w, h) = (e.xf.w, e.xf.h);
    LineEl {
        base: e,
        points: vec![[0.0, h / 2.0], [w, h / 2.0]],
        stroke: make_stroke(o.color, o.width, o.dash),
        start_marker: o.start_marker.unwrap_or(LineMarker::None),
        end_marker: o.end_marker.unwrap_or(LineMarker::None),
        start_attach: None,
        end_attach: None,
        route: None,
    }
}

pub fn make_image(init: ElementBase, asset: &str) -> ImageEl {
    ImageEl {
        base: base(init),
        asset: asset.into(),
        crop: Rect::UNIT.into(),
        edits: None,
        corner_radius: None,
        svg_colors: None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct FrameOpts {
    pub clip: Option<FrameClip>,
    pub asset: Option<String>,
    pub preset: Option<String>,
}

/// A frame with a built-in `clip`, or a frame library `preset` (its outline and chrome are copied in).
pub fn make_frame(init: ElementBase, o: FrameOpts) -> FrameEl {
    let lib = o.preset.as_deref().filter(|p| !p.is_empty()).and_then(frame_library_item);
    FrameEl {
        base: base(init),
        clip: o.clip.or_else(|| lib.map(|l| l.fallback.clone())).unwrap_or(FrameClip::Rect),
        clip_path: lib.map(|l| l.d.clone()),
        overlay: lib.and_then(|l| l.overlay.clone()),
        preset: lib.map(|l| l.id.clone()),
        content: o.asset.filter(|a| !a.is_empty()).map(|asset| FrameContent {
            asset,
            crop: Rect::UNIT.into(),
            edits: None,
            video: None,
            extra: Extra::new(),
        }),
        placeholder_fill: solidc("#d0d7de"),
    }
}

fn even(rows: usize, cols: usize) -> Vec<Rect> {
    (0..rows * cols)
        .map(|i| Rect {
            x: (i % cols) as f64 / cols as f64,
            y: (i / cols) as f64 / rows as f64,
            w: 1.0 / cols as f64,
            h: 1.0 / rows as f64,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GridLayout {
    pub label: &'static str,
    pub cells: Vec<Rect>,
}

/// Photo-grid layouts (IM-030), as normalized cell rects (insertion order = TS record order).
pub static GRID_LAYOUTS: LazyLock<IndexMap<&'static str, GridLayout>> = LazyLock::new(|| {
    let r = Rect::new;
    let third = 1.0 / 3.0;
    let mut m = IndexMap::new();
    let mut add = |k, label, cells| {
        m.insert(k, GridLayout { label, cells });
    };
    add("cols-2", "2 columns", even(1, 2));
    add("rows-2", "2 rows", even(2, 1));
    add("cols-3", "3 columns", even(1, 3));
    add("grid-2x2", "2 × 2", even(2, 2));
    add("grid-3x3", "3 × 3", even(3, 3));
    add("grid-2x3", "2 × 3", even(2, 3));
    add("big-left", "Feature left", vec![r(0.0, 0.0, 0.6, 1.0), r(0.6, 0.0, 0.4, 0.5), r(0.6, 0.5, 0.4, 0.5)]);
    add(
        "big-top",
        "Feature top",
        vec![r(0.0, 0.0, 1.0, 0.6), r(0.0, 0.6, third, 0.4), r(third, 0.6, third, 0.4), r(2.0 / 3.0, 0.6, third, 0.4)],
    );
    add(
        "mosaic",
        "Mosaic",
        vec![r(0.0, 0.0, 0.5, 0.6), r(0.5, 0.0, 0.5, 0.4), r(0.0, 0.6, 0.5, 0.4), r(0.5, 0.4, 0.5, 0.6)],
    );
    add("strip-4", "Film strip", even(1, 4));
    m
});

#[derive(Debug, Clone, Default)]
pub struct GridOpts {
    pub layout: Option<String>,
    pub cells: Option<Vec<Rect>>,
    pub gap: Option<f64>,
}

pub fn make_grid(init: ElementBase, o: GridOpts) -> GridEl {
    let layout = o.layout.unwrap_or_else(|| "grid-2x2".into());
    let rects = o.cells.unwrap_or_else(|| {
        GRID_LAYOUTS.get(layout.as_str()).unwrap_or_else(|| &GRID_LAYOUTS["grid-2x2"]).cells.clone()
    });
    GridEl {
        base: base(init),
        cells: rects
            .into_iter()
            .map(|rect| GridCell { rect: rect.into(), asset: None, crop: None, edits: None, video: None, extra: Extra::new() })
            .collect(),
        gap: o.gap.unwrap_or(8.0),
        radius: 0.0,
        placeholder_fill: solidc("#d0d7de"),
    }
}

/// Pixel rect of a grid cell inside the element box, after the gap.
pub fn grid_cell_box(gap: f64, xf: &Xf, cell: &Rect) -> Rect {
    let g = gap;
    let (w, h) = (xf.w, xf.h);
    let x0 = cell.x * w + if cell.x == 0.0 { g } else { g / 2.0 };
    let y0 = cell.y * h + if cell.y == 0.0 { g } else { g / 2.0 };
    let x1 = (cell.x + cell.w) * w - if cell.x + cell.w >= 0.9999 { g } else { g / 2.0 };
    let y1 = (cell.y + cell.h) * h - if cell.y + cell.h >= 0.9999 { g } else { g / 2.0 };
    Rect { x: x0, y: y0, w: 1f64.max(x1 - x0), h: 1f64.max(y1 - y0) }
}

pub fn make_group(init: ElementBase) -> GroupEl {
    GroupEl { base: base(init), mask: None }
}

pub fn make_qr(init: ElementBase, data: &str) -> QrEl {
    QrEl { base: base(init), data: data.into(), ecc: QrEcc::M, fg: hexc("#000000"), bg: hexc("#ffffff") }
}

/// `makePage()` — a fresh page; set fields on the result for `makePage(init)`.
pub fn make_page() -> Page {
    Page {
        id: new_id(),
        order: next_key(None),
        title: None,
        hidden: None,
        locked: None,
        background: vec![solidc("#ffffff")],
        notes: None,
        duration_ms: None,
        guides: None,
        transition: None,
        anim: None,
        mobile_layout: None,
        extra: Extra::new(),
    }
}

#[derive(Debug, Clone, Default)]
pub struct MetaInit {
    pub w: f64,
    pub h: f64,
    pub unit: Option<Unit>,
    pub title: Option<String>,
    pub surface: Option<Surface>,
    /// `emptyDesign` only: number of pages (default 1).
    pub pages: Option<usize>,
}

pub fn make_meta(init: &MetaInit) -> DesignMeta {
    DesignMeta {
        schema_version: CURRENT_SCHEMA,
        min_client_schema: CURRENT_SCHEMA,
        layout_version: LAYOUT_VERSION,
        surface: init.surface.clone().unwrap_or(Surface::Graphic),
        dims: Dims { w: init.w, h: init.h, unit: init.unit.clone().unwrap_or(Unit::Px), dpi: 96.0, extra: Extra::new() },
        lang: "en".into(),
        title: init.title.clone().unwrap_or_else(|| "Untitled design".into()),
        epoch: 0,
        template_origin: None,
        audio: None,
        captions: None,
        extra: Extra::new(),
    }
}

pub fn empty_design(init: &MetaInit) -> DesignDoc {
    let meta = make_meta(init);
    let mut pages = IndexMap::new();
    let mut prev: Option<String> = None;
    for _ in 0..init.pages.unwrap_or(1) {
        let order = next_key(prev.as_deref());
        let mut p = make_page();
        p.order = order.clone();
        pages.insert(p.id.clone(), p);
        prev = Some(order);
    }
    DesignDoc { meta, pages, elements: IndexMap::new(), assets: IndexMap::new(), extra: Extra::new() }
}

/// Assign `parentId` + an ordered z key to elements added to the same parent (`placeAll`).
pub fn place_all(parent_id: &str, els: &mut [&mut ElementBase], after: Option<&str>) {
    let mut prev = after.map(str::to_string);
    for e in els.iter_mut() {
        let z = next_key(prev.as_deref());
        prev = Some(z.clone());
        e.parent_id = parent_id.into();
        e.z = z;
    }
}

/// `placeAll` over whole elements (unknown elements are skipped).
pub fn place_all_elements(parent_id: &str, els: &mut [crate::elements::Element], after: Option<&str>) {
    let mut bases: Vec<&mut ElementBase> = els.iter_mut().filter_map(|e| e.base_mut()).collect();
    place_all(parent_id, &mut bases, after);
}

#[derive(Debug, Clone, Default)]
pub struct DrawOpts {
    pub brush: Option<Brush>,
    pub color: Option<ColorValue>,
    pub size: Option<f64>,
}

pub fn make_draw(init: ElementBase, points: Vec<f64>, o: DrawOpts) -> DrawEl {
    let (w, h) = (init.xf.w, init.xf.h);
    DrawEl {
        base: base(init),
        points,
        base_size: Size { w, h, extra: Extra::new() },
        brush: o.brush.unwrap_or(Brush::Pen),
        color: o.color.unwrap_or_else(|| hexc("#1f2328")),
        size: o.size.unwrap_or(6.0),
    }
}

#[derive(Debug, Clone, Default)]
pub struct TableOpts {
    pub rows: Option<usize>,
    pub cols: Option<usize>,
    pub data: Option<Vec<Vec<String>>>,
}

pub fn make_table(init: ElementBase, o: TableOpts) -> TableEl {
    let r = o.data.as_ref().map_or(o.rows.unwrap_or(3), Vec::len);
    let c = o.data.as_ref().and_then(|d| d.first()).map_or(o.cols.unwrap_or(3), Vec::len);
    let mut cells = IndexMap::new();
    for i in 0..r {
        for j in 0..c {
            if let Some(t) = o.data.as_ref().and_then(|d| d.get(i)).and_then(|row| row.get(j))
                && !t.is_empty()
            {
                let mut cell = TableCell::text(t.clone());
                if i == 0 {
                    cell.bold = Some(true);
                }
                cells.insert(format!("{i}:{j}"), cell);
            }
        }
    }
    TableEl {
        base: base(init),
        rows: vec![1.0 / r as f64; r],
        cols: vec![1.0 / c as f64; c],
        cells,
        style: TableStyle {
            family: DEFAULT_FONT.into(),
            size: 18.0,
            color: hexc("#1f2328"),
            border_color: hexc("#d0d7de"),
            border_width: 1.5,
            header_fill: Some(solidc("#ede9fe")),
            header_color: Some(hexc("#4c1d95")),
            stripe_fill: None,
            padding: 10.0,
            header_bold: None,
            preset: None,
            extra: Extra::new(),
        },
    }
}

pub const CHART_COLORS: &[&str] = &["#7c3aed", "#06b6d4", "#f59e0b", "#ec4899", "#10b981", "#3b82f6", "#ef4444", "#84cc16"];

/// A chart series in `makeChart` input (`color` optional).
#[derive(Debug, Clone)]
pub struct SeriesInit {
    pub name: String,
    pub values: Vec<f64>,
    pub color: Option<ColorValue>,
}

#[derive(Debug, Clone, Default)]
pub struct ChartOpts {
    pub kind: Option<ChartKind>,
    pub labels: Option<Vec<String>>,
    pub series: Option<Vec<SeriesInit>>,
}

pub fn make_chart(init: ElementBase, o: ChartOpts) -> ChartEl {
    let labels = o.labels.unwrap_or_else(|| ["Q1", "Q2", "Q3", "Q4"].map(String::from).to_vec());
    let series = o.series.unwrap_or_else(|| {
        vec![
            SeriesInit { name: "Revenue".into(), values: vec![12.0, 19.0, 15.0, 24.0], color: None },
            SeriesInit { name: "Costs".into(), values: vec![8.0, 11.0, 13.0, 14.0], color: None },
        ]
    });
    ChartEl {
        base: base(init),
        kind: o.kind.unwrap_or(ChartKind::Column),
        labels,
        series: series
            .into_iter()
            .enumerate()
            .map(|(i, s)| ChartSeries {
                name: s.name,
                values: s.values,
                color: s.color.unwrap_or_else(|| hexc(CHART_COLORS[i % CHART_COLORS.len()])),
                extra: Extra::new(),
            })
            .collect(),
        source: None,
        style: ChartStyle {
            family: DEFAULT_FONT.into(),
            text_color: hexc("#374151"),
            grid_color: hexc("#e5e7eb"),
            show_legend: true,
            show_values: false,
            show_grid: true,
            font_size: 16.0,
            format: None,
            max: None,
            slice_colors: None,
            extra: Extra::new(),
        },
    }
}

pub fn make_video(init: ElementBase, asset: &str, duration_ms: f64) -> VideoEl {
    VideoEl {
        base: base(init),
        asset: asset.into(),
        crop: Rect::UNIT.into(),
        clip: MediaClip {
            trim_start_ms: 0.0,
            trim_end_ms: duration_ms,
            speed: 1.0,
            volume: 1.0,
            muted: false,
            loop_: false,
            extra: Extra::new(),
        },
        corner_radius: None,
        edits: None,
    }
}

/// Played length of a clip in page time.
pub fn clip_length_ms(trim_start_ms: f64, trim_end_ms: f64, speed: f64) -> f64 {
    let sp = if speed == 0.0 || speed.is_nan() { 1.0 } else { speed };
    0f64.max((trim_end_ms - trim_start_ms) / sp)
}

pub fn media_clip_length_ms(c: &MediaClip) -> f64 {
    clip_length_ms(c.trim_start_ms, c.trim_end_ms, c.speed)
}

/// Form layout row (FM-001).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormRow {
    /// 'title' | 'label' | 'input' | 'option' | 'submit'
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FormLayout {
    pub rows: Vec<FormRow>,
    pub height: f64,
}

/// Vertical, deterministic form layout shared by the renderer, the editor and the live HTML overlay.
pub fn form_layout(title: &str, fields: &[FormField], submit_label: &str, size: f64, xf: &Xf) -> FormLayout {
    let s = size;
    let pad = s;
    let w = xf.w - pad * 2.0;
    let mut rows = Vec::new();
    let mut y = pad;
    let row = |kind, field_id: Option<&str>, option: Option<&str>, y, w, h, text: String| FormRow {
        kind,
        field_id: field_id.map(str::to_string),
        option: option.map(str::to_string),
        x: pad,
        y,
        w,
        h,
        text: Some(text),
    };
    if !title.is_empty() {
        rows.push(row("title", None, None, y, w, s * 1.6, title.to_string()));
        y += s * 1.6 + s * 0.8;
    }
    for f in fields {
        let label = format!("{}{}", f.label, if f.required == Some(true) { " *" } else { "" });
        rows.push(row("label", Some(&f.id), None, y, w, s * 1.35, label));
        y += s * 1.35 + s * 0.35;
        if f.kind == FormFieldKind::Choice || f.kind == FormFieldKind::Checkboxes {
            for o in f.options.iter().flatten() {
                rows.push(row("option", Some(&f.id), Some(o), y, w, s * 1.5, o.clone()));
                y += s * 1.5 + s * 0.25;
            }
        } else {
            let h = if f.kind == FormFieldKind::Longtext { s * 5.0 } else { s * 2.4 };
            rows.push(row("input", Some(&f.id), None, y, w, h, f.placeholder.clone().unwrap_or_default()));
            y += h;
        }
        y += s * 0.9;
    }
    let sw = w.min((s * 8.0).max(js::utf16_len(submit_label) as f64 * s * 0.7 + s * 2.0));
    rows.push(row("submit", None, None, y, sw, s * 2.6, submit_label.to_string()));
    y += s * 2.6 + pad;
    FormLayout { rows, height: y }
}

pub fn form_layout_of(el: &FormEl) -> FormLayout {
    form_layout(&el.title, &el.fields, &el.submit_label, el.style.size, &el.base.xf)
}

#[derive(Debug, Clone, Default)]
pub struct FormOpts {
    pub title: Option<String>,
    pub fields: Option<Vec<FormField>>,
}

pub fn make_form(init: ElementBase, o: FormOpts) -> FormEl {
    let mut el = FormEl {
        base: base(init),
        title: o.title.unwrap_or_else(|| "Get in touch".into()),
        fields: o.fields.unwrap_or_else(|| {
            vec![
                FormField::new("name", FormFieldKind::Text, "Name", Some(true)),
                FormField::new("email", FormFieldKind::Email, "Email", Some(true)),
                FormField::new("message", FormFieldKind::Longtext, "Message", None),
            ]
        }),
        submit_label: "Send".into(),
        thanks: "Thanks! We got your response.".into(),
        style: FormStyle {
            family: DEFAULT_FONT.into(),
            size: 18.0,
            color: hexc("#1f2328"),
            accent: hexc("#7c3aed"),
            field_fill: hexc("#ffffff"),
            border_color: hexc("#cbd5e1"),
            radius: 8.0,
            extra: Extra::new(),
        },
    };
    el.base.xf.h = form_layout_of(&el).height;
    el
}

#[derive(Debug, Clone)]
pub struct EmbedOpts {
    pub url: String,
    pub provider: EmbedProvider,
    pub src: String,
    pub title: Option<String>,
    pub poster: Option<String>,
}

/// Embedded media (A11); `link` makes static exports open the original URL.
pub fn make_embed(init: ElementBase, o: EmbedOpts) -> EmbedEl {
    let title = o.title.unwrap_or_default();
    let mut b = base(init);
    b.link = Some(LinkTarget::url(o.url.clone()));
    b.alt_text = if title.is_empty() { None } else { Some(title.clone()) };
    EmbedEl {
        base: b,
        url: o.url,
        provider: o.provider,
        src: o.src,
        title,
        poster: o.poster.filter(|p| !p.is_empty()),
    }
}

/// Stored rect helper for callers building cells by hand.
pub fn stored(r: Rect) -> StoredRect {
    r.into()
}
