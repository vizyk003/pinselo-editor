// SPDX-License-Identifier: Apache-2.0
//! `@oc/schema` helpers used by apps/web (TS name → oc-schema). Arguments arrive as JS values in the TS call
//! shape; the facade (packages/engine/src/schema.ts) supplies defaults the TS signatures declare and maps id
//! results back to the caller's objects.

use indexmap::IndexMap;
use oc_schema::color::{hex as hex_color, solid_hex, to_css as css, to_hex as hex_of};
use oc_schema::design::Page;
use oc_schema::doc_blocks::{self, DocChartData};
use oc_schema::elements::{ChartKind, Element, FormField, Keyframe, TableEl};
use oc_schema::factory::*;
use oc_schema::presets::{self, PresetCategoryId, SizeSide};
use oc_schema::primitives::{ColorValue, Paint, Unit, Xf};
use oc_schema::print::{self, Destination, PreflightTarget, PrintCatalog, PrintProduct, PrintSelection};
use oc_schema::table::{CellRange, Geometry, TableStylePreset};
use oc_schema::text::{RunStyleOverride, TextAlign, TextBody};
use oc_schema::timeline::PageSpan;
use oc_schema::{ElementBase, embeds, frames, keyframes, svg, timeline, tree};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use wasm_bindgen::prelude::*;

use crate::convert::{arg, element, err, js, opt_arg, to_js, type_error, typed, value};
use crate::doc::DocRef;

fn jv<T: serde::Serialize + ?Sized>(t: &T) -> Value {
    oc_schema::js::to_value(t).unwrap_or(Value::Null)
}

fn ids<'a>(els: impl IntoIterator<Item = &'a Element>) -> Vec<String> {
    els.into_iter().map(|e| e.id().unwrap_or("").to_string()).collect()
}

fn field<T: DeserializeOwned>(m: &Map<String, Value>, k: &str) -> Result<Option<T>, JsError> {
    match m.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => typed(k, v.clone()).map(Some),
    }
}

fn color_field(m: &Map<String, Value>, k: &str) -> Result<Option<ColorValue>, JsError> {
    match m.get(k) {
        Some(Value::String(s)) => hex_color(s).map(Some).map_err(err),
        _ => Ok(None),
    }
}

/// TS `BaseInit & opts` → `(ElementBase, opts)`: `base_init(xf)` overlaid with every non-option field, like
/// `base(type, rest)` (`{ id: newId(), parentId: '', z: 'a0', opacity: 1, ...rest }`).
fn split_init(init: &JsValue, opt_keys: &[&str]) -> Result<(ElementBase, Map<String, Value>), JsError> {
    let Value::Object(mut m) = value(init)? else { return Err(err("init must be an object")) };
    let mut o = Map::new();
    for k in opt_keys {
        if let Some(v) = m.remove(*k)
            && !v.is_null()
        {
            o.insert((*k).to_string(), v);
        }
    }
    let xf: Xf = typed("xf", m.get("xf").cloned().unwrap_or(Value::Null))?;
    let mut b = match jv(&base_init(xf)) {
        Value::Object(b) => b,
        _ => Map::new(),
    };
    for (k, v) in m {
        if !v.is_null() {
            b.insert(k, v);
        }
    }
    Ok((typed("init", Value::Object(b))?, o))
}

/// A table from a full element or a `Pick<TableEl, …>` (missing base fields filled in).
fn table_el(v: &JsValue) -> Result<TableEl, JsError> {
    let Value::Object(mut m) = value(v)? else { return Err(err("table must be an object")) };
    for (k, d) in [("id", json!("")), ("type", json!("table")), ("parentId", json!("")), ("z", json!("a0")), ("opacity", json!(1))] {
        m.entry(k).or_insert(d);
    }
    if !m.contains_key("xf") {
        m.insert("xf".into(), json!({ "x": 0, "y": 0, "w": 0, "h": 0, "rot": 0 }));
    }
    match element(Value::Object(m)) {
        Element::Table(t) => Ok(t),
        _ => Err(err("not a table element")),
    }
}

fn span_json(s: &PageSpan<'_>) -> Value {
    json!({ "pageId": s.page.id, "index": s.index, "startMs": s.start_ms, "durationMs": s.duration_ms })
}

/// Every `@oc/schema` constant apps/web imports, in one object (read once at init).
#[wasm_bindgen(js_name = schemaConstants)]
pub fn schema_constants() -> JsValue {
    use oc_schema::{design, elements, embeds, table};
    js(&json!({
        "ANIM_PRESETS": elements::ANIM_PRESETS,
        "BORDER_PRESETS": table::BORDER_PRESETS,
        "CALLOUT_TONES": jv(&doc_blocks::CALLOUT_TONES.iter().copied().collect::<IndexMap<_, _>>()),
        "CHART_COLORS": CHART_COLORS,
        "CHART_KINDS": elements::CHART_KINDS,
        "CURRENT_COLOR": svg::CURRENT_COLOR,
        "DEFAULT_PRINT_CATALOG": jv(&*print::DEFAULT_PRINT_CATALOG),
        "DOC_CONTENT_WIDTH": doc_blocks::DOC_CONTENT_WIDTH,
        "EMBED_FRAME_HOSTS": embeds::EMBED_FRAME_HOSTS,
        "EMPHASIS_PRESETS": elements::EMPHASIS_PRESETS,
        "FILTER_PRESETS": elements::FILTER_PRESETS,
        "FRAME_CATEGORIES": frames::FRAME_CATEGORIES,
        "FRAME_LIBRARY": jv(&*frames::FRAME_LIBRARY),
        "GRID_LAYOUTS": jv(&*GRID_LAYOUTS),
        "KEYFRAME_NEUTRAL": jv(&keyframes::KEYFRAME_NEUTRAL.iter().copied().collect::<IndexMap<_, _>>()),
        "KEYFRAME_PROPS": elements::KEYFRAME_PROPS,
        "LINE_MARKERS": elements::LINE_MARKERS,
        "POPULAR_LAYOUTS": jv(&presets::POPULAR_LAYOUTS),
        "PRESET_CATEGORIES": jv(&presets::PRESET_CATEGORIES),
        "SHAPE_KINDS": elements::SHAPE_KINDS,
        "SIZE_PRESETS": jv(&*presets::SIZE_PRESETS),
        "TABLE_STYLE_PRESETS": table::TABLE_STYLE_PRESETS,
        "TRANSITION_KINDS": design::TRANSITION_KINDS,
    }))
}

// ------------------------------------------------------------------ text, colour

#[wasm_bindgen(js_name = bodyPlainText)]
pub fn body_plain_text(body: JsValue) -> Result<String, JsError> {
    Ok(oc_schema::text::body_plain_text(&arg::<TextBody>("body", &body)?))
}

#[wasm_bindgen(js_name = textBody)]
pub fn text_body_js(text: &str, style: JsValue, align: Option<String>) -> Result<JsValue, JsError> {
    let style: RunStyleOverride = opt_arg("style", &style)?.unwrap_or_default();
    let align: TextAlign = match align {
        Some(a) => typed("align", Value::String(a))?,
        None => TextAlign::Start,
    };
    to_js(&text_body(text, &style, align))
}

#[wasm_bindgen]
pub fn hex(s: &str) -> Result<JsValue, JsError> {
    to_js(&hex_color(s).map_err(err)?)
}

/// `solid(c: ColorValue | string)`.
#[wasm_bindgen]
pub fn solid(c: JsValue) -> Result<JsValue, JsError> {
    let p: Paint = match c.as_string() {
        Some(s) => solid_hex(&s).map_err(err)?,
        None => oc_schema::color::solid(arg("color", &c)?),
    };
    to_js(&p)
}

#[wasm_bindgen(js_name = toHex)]
pub fn to_hex(c: JsValue, with_alpha: bool) -> Result<String, JsError> {
    Ok(hex_of(&arg("color", &c)?, with_alpha))
}

#[wasm_bindgen(js_name = toCss)]
pub fn to_css(c: JsValue) -> Result<String, JsError> {
    Ok(css(&arg("color", &c)?))
}

#[wasm_bindgen(js_name = newId)]
pub fn new_id() -> String {
    oc_schema::ids::new_id()
}

// ------------------------------------------------------------------ factory

#[wasm_bindgen(js_name = makeText)]
pub fn make_text_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["text", "style", "align", "body"])?;
    let opts = TextOpts { text: field(&o, "text")?, style: field(&o, "style")?, align: field(&o, "align")?, body: field(&o, "body")? };
    to_js(&Element::from(make_text(b, opts)))
}

#[wasm_bindgen(js_name = makeShape)]
pub fn make_shape_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["kind", "fill", "stroke", "params"])?;
    let fill = match o.get("fill") {
        Some(Value::String(s)) => Some(solid_hex(s).map_err(err)?),
        _ => field(&o, "fill")?,
    };
    let opts = ShapeOpts { kind: field(&o, "kind")?, fill, stroke: field(&o, "stroke")?, params: field(&o, "params")? };
    to_js(&Element::from(make_shape(b, opts)))
}

#[wasm_bindgen(js_name = makeLine)]
pub fn make_line_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["color", "width", "dash", "startMarker", "endMarker"])?;
    let opts = LineOpts {
        color: color_field(&o, "color")?,
        width: field(&o, "width")?,
        dash: field(&o, "dash")?,
        start_marker: field(&o, "startMarker")?,
        end_marker: field(&o, "endMarker")?,
    };
    to_js(&Element::from(make_line(b, opts)))
}

#[wasm_bindgen(js_name = makeStroke)]
pub fn make_stroke_js(color: Option<String>, width: Option<f64>, dash: JsValue) -> Result<JsValue, JsError> {
    let c = color.map(|s| hex_color(&s)).transpose().map_err(err)?;
    to_js(&make_stroke(c, width, opt_arg("dash", &dash)?))
}

#[wasm_bindgen(js_name = makeImage)]
pub fn make_image_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["asset"])?;
    let asset: String = field(&o, "asset")?.unwrap_or_default();
    to_js(&Element::from(make_image(b, &asset)))
}

/// `gridCellBox({ gap, xf }, cell)`.
#[wasm_bindgen(js_name = gridCellBox)]
pub fn grid_cell_box_js(el: JsValue, cell: JsValue) -> Result<JsValue, JsError> {
    let v = value(&el)?;
    let gap = v.get("gap").and_then(Value::as_f64).unwrap_or(f64::NAN);
    let xf: Xf = typed("xf", v.get("xf").cloned().unwrap_or(Value::Null))?;
    let cell: oc_schema::primitives::Rect = arg("cell", &cell)?;
    to_js(&grid_cell_box(gap, &xf, &cell))
}

#[wasm_bindgen(js_name = makeFrame)]
pub fn make_frame_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["clip", "asset", "preset"])?;
    let opts = FrameOpts { clip: field(&o, "clip")?, asset: field(&o, "asset")?, preset: field(&o, "preset")? };
    to_js(&Element::from(make_frame(b, opts)))
}

#[wasm_bindgen(js_name = makeGrid)]
pub fn make_grid_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["layout", "cells", "gap"])?;
    let opts = GridOpts { layout: field(&o, "layout")?, cells: field(&o, "cells")?, gap: field(&o, "gap")? };
    to_js(&Element::from(make_grid(b, opts)))
}

#[wasm_bindgen(js_name = makeQr)]
pub fn make_qr_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["data"])?;
    let data: String = field(&o, "data")?.unwrap_or_default();
    to_js(&Element::from(make_qr(b, &data)))
}

#[wasm_bindgen(js_name = makeTable)]
pub fn make_table_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["rows", "cols", "data"])?;
    let opts = TableOpts { rows: field(&o, "rows")?, cols: field(&o, "cols")?, data: field(&o, "data")? };
    to_js(&Element::from(make_table(b, opts)))
}

#[wasm_bindgen(js_name = makeChart)]
pub fn make_chart_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["kind", "labels", "series"])?;
    let series = match o.get("series") {
        Some(Value::Array(list)) => Some(
            list.iter()
                .map(|s| {
                    let m = s.as_object().cloned().unwrap_or_default();
                    Ok(SeriesInit {
                        name: field(&m, "name")?.unwrap_or_default(),
                        values: field(&m, "values")?.unwrap_or_default(),
                        color: color_field(&m, "color")?,
                    })
                })
                .collect::<Result<Vec<_>, JsError>>()?,
        ),
        _ => None,
    };
    let kind: Option<ChartKind> = field(&o, "kind")?;
    to_js(&Element::from(make_chart(b, ChartOpts { kind, labels: field(&o, "labels")?, series })))
}

#[wasm_bindgen(js_name = makeVideo)]
pub fn make_video_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["asset", "durationMs"])?;
    let asset: String = field(&o, "asset")?.unwrap_or_default();
    let ms: f64 = field(&o, "durationMs")?.unwrap_or(f64::NAN);
    to_js(&Element::from(make_video(b, &asset, ms)))
}

#[wasm_bindgen(js_name = makeForm)]
pub fn make_form_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["title", "fields"])?;
    let fields: Option<Vec<FormField>> = field(&o, "fields")?;
    to_js(&Element::from(make_form(b, FormOpts { title: field(&o, "title")?, fields })))
}

#[wasm_bindgen(js_name = makeEmbed)]
pub fn make_embed_js(init: JsValue) -> Result<JsValue, JsError> {
    let (b, o) = split_init(&init, &["url", "provider", "src", "title", "poster"])?;
    let opts = EmbedOpts {
        url: field(&o, "url")?.unwrap_or_default(),
        provider: typed("provider", o.get("provider").cloned().unwrap_or(Value::Null))?,
        src: field(&o, "src")?.unwrap_or_default(),
        title: field(&o, "title")?,
        poster: field(&o, "poster")?,
    };
    to_js(&Element::from(make_embed(b, opts)))
}

/// `emptyDesign({ w, h, unit?, title?, surface?, pages? })`.
#[wasm_bindgen(js_name = emptyDesign)]
pub fn empty_design_js(init: JsValue) -> Result<JsValue, JsError> {
    let Value::Object(m) = value(&init)? else { return Err(err("init must be an object")) };
    let mi = MetaInit {
        w: field(&m, "w")?.unwrap_or(f64::NAN),
        h: field(&m, "h")?.unwrap_or(f64::NAN),
        unit: field(&m, "unit")?,
        title: field(&m, "title")?,
        surface: field(&m, "surface")?,
        pages: field(&m, "pages")?,
    };
    to_js(&empty_design(&mi))
}

/// The z keys `placeAll` assigns (`keyBetween(prev, null)` from `after`); the facade spreads them over the
/// caller's elements (`{ ...e, parentId, z }`) so extra fields survive like in TS.
#[wasm_bindgen(js_name = placeAllKeys)]
pub fn place_all_keys(after: Option<String>, n: u32) -> Vec<String> {
    let mut bases: Vec<ElementBase> = (0..n).map(|_| ElementBase::default()).collect();
    let mut refs: Vec<&mut ElementBase> = bases.iter_mut().collect();
    place_all("", &mut refs, after.as_deref());
    bases.into_iter().map(|b| b.z).collect()
}

/// `clipLengthMs({ trimStartMs, trimEndMs, speed })`.
#[wasm_bindgen(js_name = clipLengthMs)]
pub fn clip_length_ms_js(c: JsValue) -> Result<f64, JsError> {
    let Value::Object(m) = value(&c)? else { return Err(err("clip must be an object")) };
    let n = |k: &str| m.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
    // `c.speed || 1` is applied by the Rust port as well (0/NaN speed → 1).
    Ok(clip_length_ms(n("trimStartMs"), n("trimEndMs"), m.get("speed").and_then(Value::as_f64).unwrap_or(0.0)))
}

/// `formLayout(el: Pick<FormEl, 'title' | 'fields' | 'submitLabel' | 'style' | 'xf'>)`.
#[wasm_bindgen(js_name = formLayout)]
pub fn form_layout_js(el: JsValue) -> Result<JsValue, JsError> {
    let Value::Object(m) = value(&el)? else { return Err(err("form must be an object")) };
    let title: String = field(&m, "title")?.unwrap_or_default();
    let fields: Vec<FormField> = field(&m, "fields")?.unwrap_or_default();
    let submit: String = field(&m, "submitLabel")?.unwrap_or_default();
    let size = m.get("style").and_then(|s| s.get("size")).and_then(Value::as_f64).unwrap_or(f64::NAN);
    let xf: Xf = typed("xf", m.get("xf").cloned().unwrap_or(Value::Null))?;
    to_js(&form_layout(&title, &fields, &submit, size, &xf))
}

// ------------------------------------------------------------------ tree, timeline (document helpers)

#[wasm_bindgen(js_name = sortedPages)]
pub fn sorted_pages(doc: &DocRef) -> Vec<String> {
    tree::sorted_pages(&doc.doc().pages).iter().map(|p| p.id.clone()).collect()
}

#[wasm_bindgen(js_name = childrenOf)]
pub fn children_of(doc: &DocRef, parent_id: &str) -> Vec<String> {
    ids(tree::children_of(&doc.doc().elements, parent_id))
}

#[wasm_bindgen]
pub fn descendants(doc: &DocRef, id: &str) -> Vec<String> {
    ids(tree::descendants(&doc.doc().elements, id))
}

#[wasm_bindgen(js_name = pageOf)]
pub fn page_of(doc: &DocRef, id: &str) -> Option<String> {
    let d = doc.doc();
    tree::page_of(&d.pages, &d.elements, id).map(str::to_string)
}

#[wasm_bindgen(js_name = pageDurationMs)]
pub fn page_duration_ms(doc: &DocRef, page: JsValue) -> Result<f64, JsError> {
    let p: Page = arg("page", &page)?;
    Ok(timeline::page_duration_ms(&doc.doc().elements, &p))
}

/// `timelineOf(doc, includeHidden)` with spans as `{ pageId, index, startMs, durationMs }`.
#[wasm_bindgen(js_name = timelineOf)]
pub fn timeline_of(doc: &DocRef, include_hidden: bool) -> JsValue {
    let t = timeline::timeline_of(doc.doc(), include_hidden);
    js(&json!({ "spans": t.spans.iter().map(span_json).collect::<Vec<_>>(), "totalMs": t.total_ms }))
}

#[wasm_bindgen(js_name = locateTime)]
pub fn locate_time(doc: &DocRef, t: f64) -> JsValue {
    match timeline::locate_time(doc.doc(), t) {
        Some(r) => js(&json!({ "span": span_json(&r.span), "pageTimeMs": r.page_time_ms })),
        None => JsValue::NULL,
    }
}

#[wasm_bindgen(js_name = elementWindow)]
pub fn element_window(el: JsValue, page_ms: f64) -> Result<JsValue, JsError> {
    to_js(&timeline::element_window(&element(value(&el)?), page_ms))
}

#[wasm_bindgen(js_name = mediaSourceTime)]
pub fn media_source_time(el: JsValue, clip: JsValue, page_time_ms: f64, page_ms: f64) -> Result<Option<f64>, JsError> {
    Ok(timeline::media_source_time(&element(value(&el)?), &arg("clip", &clip)?, page_time_ms, page_ms))
}

#[wasm_bindgen(js_name = videoSlots)]
pub fn video_slots(el: JsValue) -> Result<JsValue, JsError> {
    to_js(&timeline::video_slots(&element(value(&el)?)))
}

// ------------------------------------------------------------------ tables

#[wasm_bindgen(js_name = tableGeometry)]
pub fn table_geometry(el: JsValue) -> Result<JsValue, JsError> {
    let Value::Object(m) = value(&el)? else { return Err(err("table must be an object")) };
    let rows: Vec<f64> = field(&m, "rows")?.unwrap_or_default();
    let cols: Vec<f64> = field(&m, "cols")?.unwrap_or_default();
    let xf: Xf = typed("xf", m.get("xf").cloned().unwrap_or(Value::Null))?;
    to_js(&oc_schema::table::table_geometry(&rows, &cols, &xf))
}

#[wasm_bindgen(js_name = cellRect)]
pub fn cell_rect(el: JsValue, r: f64, c: f64, geo: JsValue) -> Result<JsValue, JsError> {
    let t = table_el(&el)?;
    let g: Option<Geometry> = opt_arg("geo", &geo)?;
    to_js(&oc_schema::table::cell_rect(&t, r as i64, c as i64, g.as_ref()))
}

#[wasm_bindgen(js_name = canUnmerge)]
pub fn can_unmerge(el: JsValue, range: JsValue) -> Result<bool, JsError> {
    let range: CellRange = arg("range", &range)?;
    Ok(oc_schema::table::can_unmerge(&table_el(&el)?, &range))
}

/// `tableStyleSwatch(preset)`; an unknown preset throws a `TypeError` like the TS (`PRESETS[preset].style`).
#[wasm_bindgen(js_name = tableStyleSwatch)]
pub fn table_style_swatch(preset: JsValue) -> Result<JsValue, JsValue> {
    let swatch = opt_arg::<TableStylePreset>("preset", &preset).ok().flatten().and_then(|p| oc_schema::table::table_style_swatch(&p));
    match swatch {
        Some(s) => Ok(js(&s)),
        None => Err(type_error("Cannot read properties of undefined (reading 'style')")),
    }
}

// ------------------------------------------------------------------ presets, units

#[wasm_bindgen(js_name = findPreset)]
pub fn find_preset(id: Option<String>) -> JsValue {
    match id.as_deref().filter(|s| !s.is_empty()).and_then(presets::find_preset) {
        Some(p) => js(p),
        None => JsValue::UNDEFINED,
    }
}

#[wasm_bindgen(js_name = presetsInCategory)]
pub fn presets_in_category(cat: JsValue) -> Result<JsValue, JsError> {
    // An unknown category yields no presets (TS: `PRESET_CATEGORIES.find(…)?.group` is undefined).
    match opt_arg::<PresetCategoryId>("cat", &cat) {
        Ok(Some(c)) => to_js(&presets::presets_in_category(&c)),
        _ => Ok(js_sys::Array::new().into()),
    }
}

/// `presetPlatforms(list)`: distinct truthy `platform`s in list order.
#[wasm_bindgen(js_name = presetPlatforms)]
pub fn preset_platforms(list: JsValue) -> Result<Vec<String>, JsError> {
    let list: Vec<Value> = arg("list", &list)?;
    let mut out: Vec<String> = Vec::new();
    for p in &list {
        if let Some(s) = p.get("platform").and_then(Value::as_str).filter(|s| !s.is_empty())
            && !out.iter().any(|x| x == s)
        {
            out.push(s.to_string());
        }
    }
    Ok(out)
}

#[wasm_bindgen(js_name = popularPresets)]
pub fn popular_presets() -> JsValue {
    js(&presets::popular_presets())
}

#[wasm_bindgen(js_name = tryNewPresets)]
pub fn try_new_presets(recent_ids: Vec<String>, max: u32) -> JsValue {
    let r: Vec<&str> = recent_ids.iter().map(String::as_str).collect();
    js(&presets::try_new_presets(&r, max as usize))
}

#[wasm_bindgen(js_name = pushRecentPreset)]
pub fn push_recent_preset(list: JsValue, id: &str, max: u32) -> Result<Vec<String>, JsError> {
    Ok(presets::push_recent_preset(&value(&list)?, id, max as usize))
}

/// `searchPresets(q, translate?)`.
#[wasm_bindgen(js_name = searchPresets)]
pub fn search_presets(q: &str, translate: Option<js_sys::Function>) -> JsValue {
    match translate {
        None => js(&presets::search_presets(q)),
        Some(f) => js(&presets::search_presets_with(q, |s| {
            f.call1(&JsValue::NULL, &JsValue::from_str(s)).ok().and_then(|v| v.as_string()).unwrap_or_else(|| s.to_string())
        })),
    }
}

#[wasm_bindgen(js_name = lockedSide)]
pub fn locked_side(changed: &str, v: f64, ratio: f64, decimals: f64) -> Result<f64, JsError> {
    let side: SizeSide = typed("changed", Value::String(changed.into()))?;
    Ok(presets::locked_side(&side, v, ratio, decimals as i32))
}

#[wasm_bindgen(js_name = pxToUnit)]
pub fn px_to_unit(px: f64, unit: JsValue) -> Result<f64, JsError> {
    Ok(presets::px_to_unit(px, &arg::<Unit>("unit", &unit)?))
}

#[wasm_bindgen(js_name = unitToPx)]
pub fn unit_to_px(v: f64, unit: JsValue) -> Result<f64, JsError> {
    Ok(presets::unit_to_px(v, &arg::<Unit>("unit", &unit)?))
}

#[wasm_bindgen(js_name = formatDims)]
pub fn format_dims(w: f64, h: f64, unit: JsValue) -> Result<String, JsError> {
    Ok(presets::format_dims(w, h, &arg::<Unit>("unit", &unit)?))
}

// ------------------------------------------------------------------ print

#[wasm_bindgen(js_name = expectedPages)]
pub fn expected_pages(p: JsValue, picked: JsValue) -> Result<f64, JsError> {
    let p: PrintProduct = arg("product", &p)?;
    let picked: IndexMap<String, String> = opt_arg("picked", &picked)?.unwrap_or_default();
    print::expected_pages(&p, &picked).map_err(err)
}

#[wasm_bindgen(js_name = preflightTargetFor)]
pub fn preflight_target_for(p: JsValue, size_id: &str, options: JsValue) -> Result<JsValue, JsError> {
    let p: PrintProduct = arg("product", &p)?;
    let o: IndexMap<String, String> = opt_arg("options", &options)?.unwrap_or_default();
    to_js(&print::preflight_target_for(&p, size_id, &o).map_err(err)?)
}

#[wasm_bindgen(js_name = standalonePrintTarget)]
pub fn standalone_print_target(doc: &DocRef) -> JsValue {
    js(&print::standalone_print_target(doc.doc()))
}

/// `preflightPrint(doc, target, { availableFonts?, outOfGamut? })`; `available_fonts` is the `{ has(family) }`
/// object, `out_of_gamut` the `(c: ColorValue) => boolean | undefined` override of the gamut heuristic.
#[wasm_bindgen(js_name = preflightPrint)]
pub fn preflight_print(doc: &DocRef, target: JsValue, available_fonts: JsValue, out_of_gamut: JsValue) -> Result<JsValue, JsError> {
    let t: PreflightTarget = arg("target", &target)?;
    let has = if available_fonts.is_object() {
        js_sys::Reflect::get(&available_fonts, &"has".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok())
    } else {
        None
    };
    let check = has.map(|f| move |family: &str| f.call1(&available_fonts, &JsValue::from_str(family)).ok().is_some_and(|v| v.is_truthy()));
    let gamut = out_of_gamut.dyn_into::<js_sys::Function>().ok().map(|f| {
        move |c: &ColorValue| match f.call1(&JsValue::UNDEFINED, &js(c)) {
            Ok(v) if !v.is_undefined() => Some(v.is_truthy()),
            _ => None,
        }
    });
    let r = print::preflight_print_with(
        doc.doc(),
        &t,
        check.as_ref().map(|f| f as &dyn Fn(&str) -> bool),
        gamut.as_ref().map(|f| f as &dyn Fn(&ColorValue) -> Option<bool>),
    );
    to_js(&r)
}

/// `printColors(doc, max = 512)`.
#[wasm_bindgen(js_name = printColors)]
pub fn print_colors(doc: &DocRef, max: Option<f64>) -> JsValue {
    let max = max.map_or(512, |m| if m.is_finite() && m > 0.0 { m.ceil() as usize } else { 0 });
    js(&print::print_colors(doc.doc(), max))
}

/// `printColorKey(c)`.
#[wasm_bindgen(js_name = printColorKey)]
pub fn print_color_key(c: JsValue) -> Result<String, JsError> {
    let c: ColorValue = arg("color", &c)?;
    Ok(print::print_color_key(&c))
}

/// `quotePrint(catalog, sel, dest?, shippingMethod?, currency?)`.
#[wasm_bindgen(js_name = quotePrint)]
pub fn quote_print(catalog: JsValue, sel: JsValue, dest: JsValue, shipping_method: Option<String>, currency: Option<String>) -> Result<JsValue, JsError> {
    let c: PrintCatalog = arg("catalog", &catalog)?;
    let s: PrintSelection = arg("selection", &sel)?;
    let d: Destination = opt_arg("dest", &dest)?.unwrap_or_default();
    to_js(&print::quote_print(&c, &s, &d, shipping_method.as_deref(), currency.as_deref()).map_err(err)?)
}

#[wasm_bindgen(js_name = validateAddress)]
pub fn validate_address(a: JsValue, countries: Option<Vec<String>>) -> Result<JsValue, JsError> {
    let m: Map<String, Value> = match value(&a)? {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    to_js(&print::validate_address(&m, countries.as_deref()))
}

// ------------------------------------------------------------------ docs, embeds, svg, keyframes

#[wasm_bindgen(js_name = calloutTone)]
pub fn callout_tone(v: JsValue) -> Result<JsValue, JsError> {
    let x = value(&v)?;
    to_js(&doc_blocks::callout_tone(if v.is_undefined() { None } else { Some(&x) }))
}

#[wasm_bindgen(js_name = docBlockWidth)]
pub fn doc_block_width(w: JsValue, fallback: Option<f64>) -> Result<f64, JsError> {
    let x = value(&w)?;
    Ok(doc_blocks::doc_block_width(if w.is_undefined() { None } else { Some(&x) }, fallback))
}

#[wasm_bindgen(js_name = parseDocChart)]
pub fn parse_doc_chart(raw: JsValue) -> Result<JsValue, JsError> {
    let x = value(&raw)?;
    to_js(&doc_blocks::parse_doc_chart(if raw.is_undefined() { None } else { Some(&x) }))
}

#[wasm_bindgen(js_name = serializeDocChart)]
pub fn serialize_doc_chart(d: JsValue) -> Result<String, JsError> {
    Ok(doc_blocks::serialize_doc_chart(&arg::<DocChartData>("chart", &d)?))
}

#[wasm_bindgen(js_name = docChartElement)]
pub fn doc_chart_element(d: JsValue, bx: JsValue) -> Result<JsValue, JsError> {
    let d: DocChartData = arg("chart", &d)?;
    to_js(&Element::from(doc_blocks::doc_chart_element(&d, arg("box", &bx)?)))
}

#[wasm_bindgen(js_name = parseEmbedUrl)]
pub fn parse_embed_url(raw: &str) -> JsValue {
    match embeds::parse_embed_url(raw) {
        Some(t) => js(&t),
        None => JsValue::NULL,
    }
}

#[wasm_bindgen(js_name = extractSvgColors)]
pub fn extract_svg_colors(svg_text: &str) -> JsValue {
    js(&svg::extract_svg_colors(svg_text))
}

/// `sanitizeSvg`: strip active content and external references from an SVG.
#[wasm_bindgen(js_name = sanitizeSvg)]
pub fn sanitize_svg(svg_text: &str) -> String {
    svg::sanitize_svg(svg_text)
}

#[wasm_bindgen(js_name = keyframeValue)]
pub fn keyframe_value(keys: JsValue, t: f64, neutral: f64) -> Result<f64, JsError> {
    let k: Option<Vec<Keyframe>> = opt_arg("keys", &keys)?;
    Ok(keyframes::keyframe_value(k.as_deref(), t, neutral))
}
