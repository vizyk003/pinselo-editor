// SPDX-License-Identifier: Apache-2.0
//! Resolve: document page → [`DisplayList`] (`packages/render-core/src/resolve.ts`). Pure; no drawing.

use std::collections::HashSet;

use indexmap::IndexMap;
use oc_geometry::{connector_local_points, frame_el_outline, frame_overlays, local_matrix, shape_path};
use oc_schema::color::hex;
use oc_schema::design::DesignDoc;
use oc_schema::elements::{AutoFit, BlurEffect, CellVAlign, Effect, Element, EmbedProvider, ShapeEl, TextFrameMode};
use oc_schema::primitives::BlendMode;
use oc_schema::factory::{grid_cell_box, text_body};
use oc_schema::primitives::{ColorValue, Extra, GradientStop, ImageFit, ImagePaint, LinearPaint, Paint, Rect, StoredRect, Stroke, StrokeAlign, StrokeCap, StrokeJoin, Xf};
use oc_schema::svg::recolor_key;
use oc_schema::text::{BackgroundTextEffect, RunStyleOverride, TextAlign, TextBody, TextEffect};
use oc_schema::timeline::{element_window, media_source_time, page_duration_ms, video_source_time};
use oc_schema::tree::{Elements, children_of, sorted_pages, truthy};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::builders::{chart_items, draw_items, form_items, table_items};
use crate::displaylist::{DLItem, DisplayList, PathStroke, ResolvedPaint, TextFit};
use crate::jsfmt::{max, min, num_to_string};
use crate::motion::{IDENTITY_MOTION, MotionState, motion_at_time};
use crate::qr::{self, Ecc};
use crate::text_units::reveal_units;

/// Options of [`resolve_page`] (`ResolveOptions`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResolveOptions {
    /// Transient per-element overrides (drag previews) merged over the document (`{ ...el, ...override }`).
    pub overrides: IndexMap<String, Map<String, Value>>,
    /// Elements to skip (e.g. hidden while being cropped).
    pub skip: HashSet<String>,
    /// Skip the page background (transparent PNG export).
    pub transparent: bool,
    /// Page time in ms for animated output (Present, video export, preview). `None` = settled state.
    pub time_ms: Option<f64>,
    /// Design time (ms) for the caption overlay; `None` = no captions.
    pub caption_time_ms: Option<f64>,
    /// Present mode: page time at which each click build step happened (steps not yet clicked stay hidden).
    pub click_steps: Option<Vec<f64>>,
    /// Text being edited: drawn flat (not curved) so the caret lines up.
    pub editing_text: Option<String>,
}

/// Cover crop (normalized) for an image of aspect `ia` in a box of aspect `ba`.
pub fn cover_crop_for(ba: f64, ia: f64) -> Rect {
    if !crate::jsfmt::truthy(ia) || !crate::jsfmt::truthy(ba) {
        return Rect::UNIT;
    }
    if ia > ba {
        let w = ba / ia;
        return Rect { x: (1.0 - w) / 2.0, y: 0.0, w, h: 1.0 };
    }
    let h = ia / ba;
    Rect { x: 0.0, y: (1.0 - h) / 2.0, w: 1.0, h }
}

fn valid_crop(c: &StoredRect) -> bool {
    [c.x, c.y, c.w, c.h].iter().all(|v| v.is_finite()) && c.w > 0.0 && c.h > 0.0
}

/// Copy of oc-editor-core `refit_crop` (oc-render doesn't depend on the editor; keep them identical): `crop` made
/// for a box shaped like `from`, shown in a box shaped like `to` (`(w, h)`) without stretching — centre kept, the
/// visible part grows along the axis that grew relatively more and zooms in once it reaches the edge of the source.
/// Port of `refitCropFor`.
pub fn refit_crop_for(crop: &StoredRect, from: (f64, f64), to: (f64, f64)) -> StoredRect {
    let f = (to.0 / to.1) / (from.0 / from.1);
    if !f.is_finite() || f <= 0.0 || (f - 1.0).abs() < 1e-6 {
        return crop.clone();
    }
    let mut w = if f > 1.0 { crop.w * f } else { crop.w };
    let mut h = if f > 1.0 { crop.h } else { crop.h / f };
    let m = max(max(w, h), 1.0);
    w /= m;
    h /= m;
    let cx = crop.x + crop.w / 2.0;
    let cy = crop.y + crop.h / 2.0;
    StoredRect::from(Rect { x: min(1.0 - w, max(0.0, cx - w / 2.0)), y: min(1.0 - h, max(0.0, cy - h / 2.0)), w, h })
}

/// The photo filling a shape (topmost image fill, by index into `fills`) and the part of it shown in the box: its
/// stored crop refit to the box's aspect (as oc-editor-core `Editor::media_crop`), else a cover crop when it is
/// `fit: 'fill'` and the asset size is known. `None` when the shape has no such photo (`fit`/`tile` image fills keep
/// drawing as paint). Port of `shapePhoto`.
pub fn shape_photo<'s>(el: &'s ShapeEl, doc: &DesignDoc) -> Option<(usize, &'s ImagePaint, StoredRect)> {
    let (i, paint) = el.fills.iter().enumerate().rev().find_map(|(i, p)| match p {
        Paint::Image(ip) => Some((i, ip)),
        _ => None,
    })?;
    let a = doc.assets.get(&paint.asset);
    let t = crate::jsfmt::truthy;
    let xf = &el.base.xf;
    let sized = a.is_some_and(|a| t(a.w) && t(a.h)) && t(xf.w) && t(xf.h);
    if let Some(c) = paint.crop.as_ref().filter(|c| valid_crop(c)) {
        let crop = match a {
            Some(a) if sized => refit_crop_for(c, (c.w * a.w, c.h * a.h), (xf.w, xf.h)),
            _ => c.clone(),
        };
        return Some((i, paint, crop));
    }
    match a {
        Some(a) if sized && paint.fit == ImageFit::Fill => Some((i, paint, cover_crop_for(xf.w / xf.h, a.w / a.h).into())),
        _ => None,
    }
}

pub fn resolve_paint(p: &Paint, bbox: Rect) -> ResolvedPaint {
    match p {
        Paint::Solid(s) => ResolvedPaint::Solid { color: s.color.clone() },
        Paint::Linear(l) => ResolvedPaint::Linear { stops: l.stops.clone(), angle: l.angle, bbox },
        Paint::Radial(r) => ResolvedPaint::Radial { stops: r.stops.clone(), bbox },
        Paint::Image(i) => ResolvedPaint::Image { asset: i.asset.clone(), fit: i.fit.clone(), bbox },
    }
}

fn embed_color(p: &EmbedProvider) -> &'static str {
    match p {
        EmbedProvider::Youtube => "#ff0000",
        EmbedProvider::Vimeo => "#1ab7ea",
        EmbedProvider::Spotify => "#1db954",
        EmbedProvider::Soundcloud => "#ff5500",
        EmbedProvider::Maps => "#34a853",
        EmbedProvider::Loom => "#625df5",
        EmbedProvider::Figma => "#a259ff",
        EmbedProvider::Codepen => "#111111",
        EmbedProvider::Other(_) => "#111827",
    }
}

fn embed_label(p: &EmbedProvider) -> &'static str {
    match p {
        EmbedProvider::Youtube => "YouTube",
        EmbedProvider::Vimeo => "Vimeo",
        EmbedProvider::Spotify => "Spotify",
        EmbedProvider::Soundcloud => "SoundCloud",
        EmbedProvider::Maps => "Google Maps",
        EmbedProvider::Loom => "Loom",
        EmbedProvider::Figma => "Figma",
        EmbedProvider::Codepen => "CodePen",
        // `EMBED_LABELS[provider]` is undefined for unknown providers.
        EmbedProvider::Other(_) => "undefined",
    }
}

fn hexc(s: &str) -> ColorValue {
    hex(s).unwrap_or_else(|_| ColorValue::srgb(0.0, 0.0, 0.0, 1.0))
}

fn stop(t: f64, color: ColorValue) -> GradientStop {
    GradientStop { t, color, extra: Extra::new() }
}

fn rect_params(r: f64) -> oc_geometry::Params {
    let mut p = oc_geometry::Params::new();
    p.insert("cornerRadius".into(), r);
    p
}

/// `{ ...el, ...override }` (falls back to the element when the merge does not form a valid element).
fn merge(el: &Element, o: &Map<String, Value>) -> Element {
    let Ok(Value::Object(mut m)) = serde_json::to_value(el) else { return el.clone() };
    for (k, v) in o {
        m.insert(k.clone(), v.clone());
    }
    serde_json::from_value(Value::Object(m)).unwrap_or_else(|_| el.clone())
}

/// The fields every element has, read the same way for unknown element types.
struct Common {
    id: String,
    xf: Xf,
    opacity: f64,
    blend: Option<BlendMode>,
    effects: Vec<Effect>,
    hidden: bool,
}

fn common(el: &Element) -> Common {
    match el {
        Element::Unknown(m) => Common {
            id: m.get("id").and_then(Value::as_str).unwrap_or("").to_owned(),
            xf: m.get("xf").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_else(|| Xf::new(f64::NAN, f64::NAN, f64::NAN, f64::NAN, 0.0)),
            opacity: m.get("opacity").and_then(Value::as_f64).unwrap_or(f64::NAN),
            blend: m.get("blend").and_then(Value::as_str).map(BlendMode::parse),
            effects: m.get("effects").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
            hidden: m.get("visible") == Some(&Value::Bool(false)) || m.get("deleted").is_some_and(truthy),
        },
        _ => {
            let b = el.base().expect("known element");
            Common {
                id: b.id.clone(),
                xf: b.xf.clone(),
                opacity: b.opacity,
                blend: b.blend.clone(),
                effects: b.effects.clone().unwrap_or_default(),
                hidden: b.visible == Some(false) || b.deleted.is_some(),
            }
        }
    }
}

struct Resolver<'a> {
    doc: &'a DesignDoc,
    opts: &'a ResolveOptions,
    dl: DisplayList,
    view: Option<Elements>,
    page_ms: f64,
    page_no: usize,
    page_total: usize,
}

impl<'a> Resolver<'a> {
    fn get(&self, el: &'a Element) -> std::borrow::Cow<'a, Element> {
        match el.id().and_then(|id| self.opts.overrides.get(id)) {
            Some(o) => std::borrow::Cow::Owned(merge(el, o)),
            None => std::borrow::Cow::Borrowed(el),
        }
    }

    /// The document's elements with the overrides applied (connectors follow live drag previews).
    fn view(&mut self) -> &Elements {
        if self.view.is_none() {
            let mut els = self.doc.elements.clone();
            for (id, o) in &self.opts.overrides {
                if let Some(e) = els.get_mut(id) {
                    *e = merge(e, o);
                }
            }
            self.view = Some(els);
        }
        self.view.as_ref().expect("view")
    }

    /// `{{page}}` / `{{pages}}` tokens (ED-095). `None` when the body has none.
    fn page_tokens(&self, body: &TextBody) -> Option<TextBody> {
        if !body.paragraphs.iter().any(|p| p.runs.iter().any(|r| r.text.contains("{{page"))) {
            return None;
        }
        let mut out = body.clone();
        for p in &mut out.paragraphs {
            for r in &mut p.runs {
                r.text = r.text.replace("{{page}}", &self.page_no.to_string()).replace("{{pages}}", &self.page_total.to_string());
            }
        }
        Some(out)
    }

    fn push(&mut self, it: DLItem) {
        self.dl.items.push(it);
    }

    fn asset(&mut self, a: impl Into<String>) {
        self.dl.assets.insert(a.into());
    }

    fn asset_aspect(&self, id: &str, fallback: f64) -> f64 {
        match self.doc.assets.get(id) {
            Some(a) if crate::jsfmt::truthy(a.w) && crate::jsfmt::truthy(a.h) => a.w / a.h,
            _ => fallback,
        }
    }

    fn emit(&mut self, raw: &'a Element, depth: usize, motion: &MotionState) {
        if depth > 64 || raw.id().is_some_and(|id| self.opts.skip.contains(id)) {
            return;
        }
        let el = self.get(raw);
        let c = common(&el);
        if c.hidden {
            return;
        }
        self.push(DLItem::Save { matrix: local_matrix(&c.xf) });
        let bbox = Rect { x: 0.0, y: 0.0, w: c.xf.w, h: c.xf.h };
        let moving = !motion.is_identity_ref();
        if moving {
            // Animate about the box centre: translate, rotate, scale (element box space).
            let cx = bbox.w / 2.0;
            let cy = bbox.h / 2.0;
            let r = (motion.rot * std::f64::consts::PI) / 180.0;
            // Rotation · scale (with flip's horizontal squash), about the centre.
            let sx = motion.scale * motion.sx.unwrap_or(1.0);
            let sy = motion.scale;
            let (a, b, cc, d) = (libm::cos(r) * sx, libm::sin(r) * sx, -libm::sin(r) * sy, libm::cos(r) * sy);
            self.push(DLItem::Save { matrix: [a, b, cc, d, cx + motion.dx - a * cx - cc * cy, cy + motion.dy - b * cx - d * cy] });
            if motion.reveal < 1.0 {
                let k = motion.reveal;
                let (x0, y0, x1, y1) = match motion.reveal_from.as_ref().map(|d| d.as_str()) {
                    Some("left") => (bbox.w * (1.0 - k), 0.0, bbox.w, bbox.h),
                    Some("up") => (0.0, bbox.h * (1.0 - k), bbox.w, bbox.h),
                    Some("down") => (0.0, 0.0, bbox.w, bbox.h * k),
                    _ => (0.0, 0.0, bbox.w * k, bbox.h),
                };
                let s = num_to_string;
                self.push(DLItem::Clip { path: format!("M{} {} L{} {} L{} {} L{} {} Z", s(x0), s(y0), s(x1), s(y0), s(x1), s(y1), s(x0), s(y1)) });
            }
        }
        let opacity = c.opacity * motion.opacity;
        let mut effects = c.effects.clone();
        if motion.blur > 0.1 {
            effects.push(Effect::Blur(BlurEffect { radius: motion.blur, extra: Extra::new() }));
        }
        let blend_set = c.blend.as_ref().is_some_and(|b| !b.as_str().is_empty() && *b != BlendMode::Normal);
        let needs_layer = opacity < 1.0 || blend_set || !effects.is_empty();
        if needs_layer {
            self.push(DLItem::Layer { opacity, blend: c.blend.clone().unwrap_or(BlendMode::Normal), effects, src_el: c.id.clone() });
        }
        let id = c.id.clone();
        match &*el {
            Element::Shape(sh) => {
                let path = shape_path(sh.kind.as_str(), bbox.w, bbox.h, &sh.params, sh.d.as_deref());
                let stroke = sh.strokes.first().filter(|s| s.width > 0.0).map(|s| PathStroke { stroke: s.clone(), paint: resolve_paint(&s.paint, bbox) });
                // A photo filling the shape (dropped onto it) draws like a frame's photo: cropped and clipped to the
                // outline, between the paints below and above it, under the border.
                if let Some((i, photo, crop)) = shape_photo(sh, self.doc) {
                    let under = &sh.fills[..i];
                    let over = &sh.fills[i + 1..];
                    if !under.is_empty() {
                        self.push(DLItem::Path { path: path.clone(), fills: under.iter().map(|p| resolve_paint(p, bbox)).collect(), stroke: None, src_el: id.clone() });
                    }
                    self.push(DLItem::Image { asset: photo.asset.clone(), dst: bbox, crop, edits: None, clip: Some(path.clone()), src_el: id.clone() });
                    if !over.is_empty() || stroke.is_some() {
                        self.push(DLItem::Path { path, fills: over.iter().map(|p| resolve_paint(p, bbox)).collect(), stroke, src_el: id });
                    }
                } else {
                    self.push(DLItem::Path { path, fills: sh.fills.iter().map(|p| resolve_paint(p, bbox)).collect(), stroke, src_el: id });
                }
                for p in &sh.fills {
                    if let Paint::Image(i) = p {
                        self.asset(i.asset.clone());
                    }
                }
            }
            Element::Line(ln) => {
                let routed = if ln.start_attach.is_some() || ln.end_attach.is_some() { connector_local_points(self.view(), ln) } else { None };
                self.push(DLItem::Line {
                    points: routed.unwrap_or_else(|| ln.points.clone()),
                    stroke: ln.stroke.clone(),
                    paint: resolve_paint(&ln.stroke.paint, bbox),
                    start_marker: ln.start_marker.clone(),
                    end_marker: ln.end_marker.clone(),
                    src_el: id,
                });
            }
            Element::Grid(g) => {
                for (ci, cell) in g.cells.iter().enumerate() {
                    let cb = grid_cell_box(g.gap, &g.base.xf, &Rect::from(&cell.rect));
                    let local = Rect { x: 0.0, y: 0.0, w: cb.w, h: cb.h };
                    let params = if crate::jsfmt::truthy(g.radius) { rect_params(g.radius) } else { oc_geometry::Params::new() };
                    let clip = shape_path("rect", cb.w, cb.h, &params, None);
                    self.push(DLItem::Save { matrix: [1.0, 0.0, 0.0, 1.0, cb.x, cb.y] });
                    let asset = cell.asset.as_deref().filter(|a| !a.is_empty());
                    match (asset, &cell.video) {
                        (Some(a), Some(video)) => {
                            let crop = cell.crop.clone().unwrap_or_else(|| cover_crop_for(cb.w / cb.h, self.asset_aspect(a, cb.w / cb.h)).into());
                            let src = self.opts.time_ms.and_then(|t| media_source_time(&el, video, t, self.page_ms));
                            self.push(DLItem::Video {
                                asset: a.to_owned(),
                                dst: local,
                                crop,
                                source_time_ms: src,
                                clip: Some(clip),
                                key: Some(format!("{id}#{ci}")),
                                src_el: id.clone(),
                                edits: None,
                            });
                            self.asset(format!("{a}/poster"));
                        }
                        (Some(a), None) => {
                            let crop = cell.crop.clone().unwrap_or_else(|| cover_crop_for(cb.w / cb.h, self.asset_aspect(a, cb.w / cb.h)).into());
                            self.push(DLItem::Image { asset: a.to_owned(), dst: local, crop, edits: cell.edits.clone(), clip: Some(clip), src_el: id.clone() });
                            self.asset(a.to_owned());
                        }
                        _ => self.push(DLItem::Path { path: clip, fills: vec![resolve_paint(&g.placeholder_fill, local)], stroke: None, src_el: id.clone() }),
                    }
                    self.push(DLItem::Restore);
                }
            }
            Element::Image(im) => {
                let clip = im.corner_radius.filter(|r| crate::jsfmt::truthy(*r)).map(|r| shape_path("rect", bbox.w, bbox.h, &rect_params(r), None));
                // Recoloured SVGs (A4) draw a derived asset ("<id>/recolor/<pairs>") rasterised from the SVG source.
                let asset = match &im.svg_colors {
                    Some(m) => recolor_key(&im.asset, Some(m)),
                    None => im.asset.clone(),
                };
                // Image border (g63): the `outline` effect drawn as an inside stroke along the (rounded) edge.
                let border = im.base.effects.iter().flatten().find_map(|e| match e {
                    Effect::Outline(o) if o.width > 0.0 => Some(o.clone()),
                    _ => None,
                });
                let edge = border.as_ref().map(|_| clip.clone().unwrap_or_else(|| shape_path("rect", bbox.w, bbox.h, &oc_geometry::Params::new(), None)));
                self.push(DLItem::Image { asset: asset.clone(), dst: bbox, crop: im.crop.clone(), edits: im.edits.clone(), clip, src_el: id.clone() });
                self.asset(asset);
                if let (Some(o), Some(path)) = (border, edge) {
                    let paint = Paint::Solid(oc_schema::primitives::SolidPaint { color: o.color.clone(), extra: Extra::new() });
                    let stroke = Stroke { paint: paint.clone(), width: o.width, align: StrokeAlign::Inside, cap: StrokeCap::Butt, join: StrokeJoin::Miter, dash: None, extra: Extra::new() };
                    self.push(DLItem::Path { path, fills: vec![], stroke: Some(PathStroke { stroke, paint: resolve_paint(&paint, bbox) }), src_el: id });
                }
            }
            Element::Frame(fr) => {
                let clip = frame_el_outline(fr, bbox.w, bbox.h);
                match &fr.content {
                    Some(c) if c.video.is_some() => {
                        let video = c.video.as_ref().expect("video");
                        let src = self.opts.time_ms.and_then(|t| media_source_time(&el, video, t, self.page_ms));
                        self.push(DLItem::Video {
                            asset: c.asset.clone(),
                            dst: bbox,
                            crop: c.crop.clone(),
                            source_time_ms: src,
                            clip: Some(clip),
                            key: Some(id.clone()),
                            src_el: id.clone(),
                            edits: None,
                        });
                        self.asset(format!("{}/poster", c.asset));
                    }
                    Some(c) => {
                        self.push(DLItem::Image { asset: c.asset.clone(), dst: bbox, crop: c.crop.clone(), edits: c.edits.clone(), clip: Some(clip), src_el: id.clone() });
                        self.asset(c.asset.clone());
                    }
                    None => self.push(DLItem::Path { path: clip, fills: vec![resolve_paint(&fr.placeholder_fill, bbox)], stroke: None, src_el: id.clone() }),
                }
                for o in frame_overlays(fr.overlay.as_deref(), bbox.w, bbox.h) {
                    self.push(DLItem::Path { path: o.path, fills: vec![resolve_paint(&o.overlay.fill, bbox)], stroke: None, src_el: id.clone() });
                    if let Paint::Image(i) = &o.overlay.fill {
                        self.asset(i.asset.clone());
                    }
                }
            }
            Element::Text(t) => {
                // Page-number tokens (ED-095): {{page}} and {{pages}}, shown raw only while being edited.
                let editing = self.opts.editing_text.as_deref() == Some(id.as_str());
                let numbered = if editing { None } else { self.page_tokens(&t.body) };
                let shown: Option<(TextBody, String)> = match (&motion.units, numbered) {
                    (Some(u), numbered) => Some(reveal_units(numbered.as_ref().unwrap_or(&t.body), u)),
                    (None, Some(b)) => Some((b, format!("pg{}/{}", self.page_no, self.page_total))),
                    (None, None) => None,
                };
                let curve = t.curve.filter(|c| crate::jsfmt::truthy(*c) && !editing);
                let fit = (t.auto_fit == AutoFit::Shrink && t.frame.mode == TextFrameMode::Fixed && !t.curve.is_some_and(crate::jsfmt::truthy)).then_some(TextFit::Shrink);
                let (key, body) = match shown {
                    Some((b, k)) => (format!("{id}:{k}"), b),
                    None => (id.clone(), t.body.clone()),
                };
                // Vertical anchor: only a fixed box can be taller than its text (a curved line is centred anyway).
                let valign = t.valign.clone().filter(|v| *v != CellVAlign::Top && t.frame.mode == TextFrameMode::Fixed && curve.is_none());
                self.push(DLItem::Text { key, body, bbox, effect: t.effect.clone(), valign, curve, fit, src_el: id });
            }
            Element::Qr(q) => {
                let data = if q.data.is_empty() { " " } else { q.data.as_str() };
                match qr::create(data, Ecc::parse(q.ecc.as_str())) {
                    Ok(code) => self.push(DLItem::Qr { modules: code.modules, size: code.size, bbox, fg: q.fg.clone(), bg: q.bg.clone(), src_el: id }),
                    // The TS throws here (the whole page fails to resolve); draw a placeholder instead.
                    Err(e) => self.push(DLItem::Placeholder { bbox, label: format!("QR code: {e}"), src_el: id }),
                }
            }
            Element::Group(g) => {
                let kids = children_of(&self.doc.elements, &id);
                // A mask child that exists and is shown gates the others (DS-107); without it they draw unmasked.
                let mask = g.mask.as_ref().and_then(|m| {
                    kids.iter().copied().find(|k| k.id() == Some(m.id.as_str()) && !common(&self.get(k)).hidden).map(|k| (k, m.mode.clone()))
                });
                match mask {
                    Some((mk, mode)) => {
                        self.push(DLItem::Mask { mode, src_el: id.clone() });
                        self.emit(mk, depth + 1, &IDENTITY_MOTION);
                        self.push(DLItem::MaskContent);
                        for ch in kids.iter().copied().filter(|k| !std::ptr::eq(*k, mk)) {
                            self.emit(ch, depth + 1, &IDENTITY_MOTION);
                        }
                        self.push(DLItem::Restore);
                    }
                    None => {
                        let mid = g.mask.as_ref().map(|m| m.id.as_str());
                        for ch in kids {
                            if mid.is_some() && ch.id() == mid {
                                continue;
                            }
                            self.emit(ch, depth + 1, &IDENTITY_MOTION);
                        }
                    }
                }
            }
            Element::Draw(d) => {
                for it in draw_items(d) {
                    self.push(it);
                }
            }
            Element::Table(tb) => {
                for it in table_items(tb) {
                    self.push(it);
                }
            }
            Element::Chart(ch) => {
                for it in chart_items(ch) {
                    self.push(it);
                }
            }
            Element::Form(fm) => {
                for it in form_items(fm) {
                    self.push(it);
                }
            }
            Element::Embed(em) => {
                // Poster + play badge + title; Present/view/sites overlay the live iframe on top.
                let r = min(bbox.w, bbox.h) * 0.04;
                let clip = shape_path("rect", bbox.w, bbox.h, &rect_params(r), None);
                let brand = embed_color(&em.provider);
                match em.poster.as_deref().filter(|p| !p.is_empty()) {
                    Some(poster) => {
                        let crop = cover_crop_for(bbox.w / bbox.h, self.asset_aspect(poster, bbox.w / bbox.h));
                        self.push(DLItem::Image { asset: poster.to_owned(), dst: bbox, crop: crop.into(), edits: None, clip: Some(clip), src_el: id.clone() });
                        self.asset(poster.to_owned());
                    }
                    None => {
                        let p = Paint::Linear(LinearPaint { stops: vec![stop(0.0, hexc(brand)), stop(1.0, hexc("#0b0b10"))], angle: 135.0, extra: Extra::new() });
                        self.push(DLItem::Path { path: clip, fills: vec![resolve_paint(&p, bbox)], stroke: None, src_el: id.clone() });
                    }
                }
                let band = min(bbox.h * 0.38, max(36.0, bbox.h * 0.22));
                let s = num_to_string;
                let shade = Paint::Linear(LinearPaint { stops: vec![stop(0.0, hexc("#00000000")), stop(1.0, hexc("#000000b3"))], angle: 90.0, extra: Extra::new() });
                self.push(DLItem::Path {
                    path: format!("M0 {} L{} {} L{} {} L0 {} Z", s(bbox.h - band), s(bbox.w), s(bbox.h - band), s(bbox.w), s(bbox.h), s(bbox.h)),
                    fills: vec![resolve_paint(&shade, Rect { x: 0.0, y: bbox.h - band, w: bbox.w, h: band })],
                    stroke: None,
                    src_el: id.clone(),
                });
                let sz = min(bbox.w, bbox.h);
                if matches!(em.provider, EmbedProvider::Youtube | EmbedProvider::Vimeo | EmbedProvider::Loom | EmbedProvider::Spotify | EmbedProvider::Soundcloud) {
                    let pr = max(14.0, sz * 0.12);
                    let (cx, cy) = (bbox.w / 2.0, bbox.h / 2.0);
                    self.push(DLItem::Path {
                        path: format!("M{} {} a{} {} 0 1 0 {} 0 a{} {} 0 1 0 {} 0", s(cx - pr), s(cy), s(pr), s(pr), s(pr * 2.0), s(pr), s(pr), s(-pr * 2.0)),
                        fills: vec![ResolvedPaint::Solid { color: hexc(brand) }],
                        stroke: None,
                        src_el: id.clone(),
                    });
                    self.push(DLItem::Path {
                        path: format!("M{} {} L{} {} L{} {} Z", s(cx - pr * 0.32), s(cy - pr * 0.45), s(cx + pr * 0.5), s(cy), s(cx - pr * 0.32), s(cy + pr * 0.45)),
                        fills: vec![ResolvedPaint::Solid { color: hexc("#ffffff") }],
                        stroke: None,
                        src_el: id.clone(),
                    });
                }
                let fs = max(10.0, min(28.0, sz * 0.07));
                let pad = fs * 0.8;
                let title = if em.title.is_empty() { em.url.as_str() } else { em.title.as_str() };
                let style = RunStyleOverride {
                    family: Some("Inter".into()),
                    size: Some(fs),
                    weight: Some(600.0),
                    fill: Some(Paint::solid_color(hexc("#ffffff"))),
                    ..Default::default()
                };
                self.push(DLItem::Text {
                    key: format!("{id}:t"),
                    body: text_body(&format!("{title}\n{}", embed_label(&em.provider)), &style, TextAlign::Start),
                    bbox: Rect { x: pad, y: bbox.h - band, w: bbox.w - pad * 2.0, h: band - pad },
                    effect: None,
                    valign: Some(CellVAlign::Bottom),
                    curve: None,
                    fit: None,
                    src_el: id,
                });
            }
            Element::Video(v) => {
                let clip = v.corner_radius.filter(|r| crate::jsfmt::truthy(*r)).map(|r| shape_path("rect", bbox.w, bbox.h, &rect_params(r), None));
                let src = self.opts.time_ms.and_then(|t| video_source_time(v, t, self.page_ms));
                self.push(DLItem::Video { asset: v.asset.clone(), dst: bbox, crop: v.crop.clone(), source_time_ms: src, clip, key: None, src_el: id, edits: v.edits.clone() });
                self.asset(format!("{}/poster", v.asset));
            }
            Element::Unknown(_) => {
                let label = format!("Update to edit ({})", el.type_name());
                self.push(DLItem::Placeholder { bbox, label, src_el: id });
            }
        }
        if needs_layer {
            self.push(DLItem::Restore);
        }
        if moving {
            self.push(DLItem::Restore);
        }
        self.push(DLItem::Restore);
    }
}

/// Resolve a page of a design into a display list (`resolvePage`).
pub fn resolve_page(doc: &DesignDoc, page_id: &str, opts: &ResolveOptions) -> DisplayList {
    let (w, h) = (doc.meta.dims.w, doc.meta.dims.h);
    let mut dl = DisplayList::new(w, h);
    let Some(page) = doc.pages.get(page_id) else { return dl };
    let page_box = Rect { x: 0.0, y: 0.0, w, h };
    let page_ms = page_duration_ms(&doc.elements, page);
    if !opts.transparent {
        dl.background = page.background.iter().map(|p| resolve_paint(p, page_box)).collect();
    }
    for p in &page.background {
        if let Paint::Image(i) = p {
            dl.assets.insert(i.asset.clone());
        }
    }
    let sorted = sorted_pages(&doc.pages);
    let page_no = sorted.iter().position(|p| p.id == page_id).map_or(0, |i| i + 1);
    let mut r = Resolver { doc, opts, dl, view: None, page_ms, page_no, page_total: sorted.len() };
    let top = children_of(&doc.elements, page_id);
    for (i, raw) in top.into_iter().enumerate() {
        match opts.time_ms {
            None => r.emit(raw, 0, &IDENTITY_MOTION),
            Some(t) => {
                let el = r.get(raw);
                let window = element_window(&el, page_ms);
                if let Some(m) = motion_at_time(&el, page, i, t, window, opts.click_steps.as_deref()) {
                    r.emit(raw, 0, &m);
                }
            }
        }
    }
    let mut dl = r.dl;

    // Caption overlay (design-level track), drawn above everything.
    if let (Some(cap), Some(ct)) = (&doc.meta.captions, opts.caption_time_ms)
        && cap.visible
        && let Some(cue) = cap.cues.iter().find(|c| ct >= c.start_ms && ct < c.end_ms)
        && !oc_schema::js::trim(&cue.text).is_empty()
    {
        let size = cap.style.size;
        let bw = w * 0.86;
        let y = match cap.style.position.as_str() {
            "top" => h * 0.06,
            "middle" => h / 2.0 - size,
            _ => h * 0.94 - size * 2.6,
        };
        let style = RunStyleOverride {
            family: Some(cap.style.family.clone()),
            size: Some(size),
            weight: Some(700.0),
            fill: Some(Paint::solid_color(hexc(&cap.style.color))),
            ..Default::default()
        };
        dl.items.push(DLItem::Text {
            key: format!("caption:{}", cue.id),
            body: text_body(&cue.text, &style, TextAlign::Center),
            bbox: Rect { x: (w - bw) / 2.0, y, w: bw, h: size * 2.6 },
            effect: Some(TextEffect::Background(BackgroundTextEffect {
                color: hexc(&cap.style.background),
                padding: size * 0.35,
                radius: size * 0.25,
                extra: Extra::new(),
            })),
            valign: Some(CellVAlign::Middle),
            curve: None,
            fit: None,
            src_el: "caption".into(),
        });
    }
    dl
}
