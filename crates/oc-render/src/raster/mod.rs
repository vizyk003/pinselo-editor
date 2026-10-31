// SPDX-License-Identifier: Apache-2.0
//! Raster backend (`packages/render-canvaskit/src/painter.ts`): draws a [`DisplayList`] with tiny-skia into an
//! RGBA pixmap. Text is laid out by oc-text and drawn from skrifa outlines; images are decoded by the `image`
//! crate (SVG by resvg). The same code runs natively and in WASM.

pub mod blur;
pub mod canvas;
mod color_glyph;
mod image;
mod text;
pub mod transition;

use std::collections::HashMap;
use std::sync::Arc;

use oc_geometry::{MarkerMode, line_marker_shape, trim_line_for_markers};
use oc_schema::color::to_rgba as doc_rgba;
use oc_schema::elements::{Effect, LineMarker};
use oc_schema::primitives::{BlendMode as DocBlend, ColorValue, ImageFit, Stroke as DocStroke, StrokeAlign, StrokeCap, StrokeJoin};
use oc_text::TextEngine;
use tiny_skia::{BlendMode, Color, FilterQuality, GradientStop, LinearGradient, Paint, Pattern, Point, RadialGradient, Rect, Shader, SpreadMode, Stroke, StrokeDash, Transform};

pub use canvas::{Canvas, Filter, LayerPaint};
pub use image::fit_scale as image_fit_scale;

use crate::assets::{AssetProvider, Decoded, ImageCache, RasterImage, VideoFrames};
use crate::displaylist::{DLItem, DisplayList, ResolvedPaint};
use crate::glyphs::GlyphCache;
use crate::path::{self, PathData};
use crate::resolve::{ResolveOptions, resolve_page};

/// `Color4f` of a document colour (display sRGB, clamped).
pub fn color(c: &ColorValue) -> Color {
    let x = doc_rgba(c);
    let f = |v: f64| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) as f32 };
    Color::from_rgba(f(x.r), f(x.g), f(x.b), f(x.a)).unwrap_or(Color::BLACK)
}

pub(crate) fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color::from_rgba(r, g, b, a).unwrap_or(Color::BLACK)
}

/// Skia blend mode of a document blend mode.
pub fn blend(b: &DocBlend) -> BlendMode {
    match b {
        DocBlend::Multiply => BlendMode::Multiply,
        DocBlend::Screen => BlendMode::Screen,
        DocBlend::Overlay => BlendMode::Overlay,
        DocBlend::Darken => BlendMode::Darken,
        DocBlend::Lighten => BlendMode::Lighten,
        DocBlend::ColorDodge => BlendMode::ColorDodge,
        DocBlend::ColorBurn => BlendMode::ColorBurn,
        DocBlend::HardLight => BlendMode::HardLight,
        DocBlend::SoftLight => BlendMode::SoftLight,
        DocBlend::Difference => BlendMode::Difference,
        DocBlend::Exclusion => BlendMode::Exclusion,
        DocBlend::Hue => BlendMode::Hue,
        DocBlend::Saturation => BlendMode::Saturation,
        DocBlend::Color => BlendMode::Color,
        DocBlend::Luminosity => BlendMode::Luminosity,
        DocBlend::Normal | DocBlend::Other(_) => BlendMode::SourceOver,
    }
}

pub(crate) fn tr(m: &[f64; 6]) -> Transform {
    Transform::from_row(m[0] as f32, m[1] as f32, m[2] as f32, m[3] as f32, m[4] as f32, m[5] as f32)
}

pub(crate) fn rect(x: f64, y: f64, w: f64, h: f64) -> Option<Rect> {
    Rect::from_xywh(x as f32, y as f32, w as f32, h as f32)
}

/// Device scale of a transform along each axis.
pub(crate) fn axis_scales(m: &Transform) -> (f32, f32) {
    (m.sx.hypot(m.ky), m.kx.hypot(m.sy))
}

/// Linear gradient end points for `angle` (degrees) across a box, as the TS computes them.
pub fn linear_points(angle: f64, b: &oc_schema::primitives::Rect) -> ((f64, f64), (f64, f64)) {
    let a = (angle * std::f64::consts::PI) / 180.0;
    let (dx, dy) = (a.cos(), a.sin());
    let half = ((b.w * dx).abs() + (b.h * dy).abs()) / 2.0;
    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    ((cx - dx * half, cy - dy * half), (cx + dx * half, cy + dy * half))
}

fn stops(s: &[oc_schema::primitives::GradientStop]) -> Vec<GradientStop> {
    s.iter().map(|g| GradientStop::new(if g.t.is_nan() { 0.0 } else { g.t as f32 }, color(&g.color))).collect()
}

/// Options of a page render.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    /// Output pixels per document px.
    pub scale: f64,
    /// Fill under the page (JPEG export: white); `None` = transparent.
    pub background: Option<Color>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions { scale: 1.0, background: None }
    }
}

/// The renderer: a text engine (fonts), decoded images and glyph outlines, kept across frames.
pub struct Renderer {
    pub text: TextEngine,
    pub(crate) images: ImageCache,
    pub(crate) glyphs: GlyphCache,
    /// Edited (tone/colour-matrix) images by key.
    pub(crate) edited: HashMap<String, Arc<RasterImage>>,
    /// Decoded video frames supplied by the host; without it videos show their poster.
    pub video_frames: Option<Box<dyn VideoFrames + Send>>,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer").field("text", &self.text).field("images", &self.images).finish_non_exhaustive()
    }
}

/// The source of a resolved paint: a colour or gradient shader, or an image pattern (which borrows its pixels,
/// so the tiny-skia paint is made on use).
enum PaintSrc {
    Shader(Shader<'static>),
    Image { pix: Arc<tiny_skia::Pixmap>, t: Transform, spread: SpreadMode },
}

/// A resolved paint ready to fill with, plus (for non-tiling image fills) the image rectangle the fill is clipped
/// to (Skia's `Decal` tiling).
pub(crate) struct Fill {
    src: PaintSrc,
    decal: Option<Rect>,
    aa: bool,
}

impl Fill {
    pub(crate) fn paint(&self) -> Paint<'_> {
        let shader = match &self.src {
            PaintSrc::Shader(s) => s.clone(),
            PaintSrc::Image { pix, t, spread } => Pattern::new(pix.as_ref().as_ref(), *spread, FilterQuality::Bicubic, 1.0, *t),
        };
        Paint { shader, anti_alias: self.aa, ..Default::default() }
    }
}

impl Renderer {
    pub fn new(text: TextEngine) -> Renderer {
        Renderer { text, images: ImageCache::default(), glyphs: GlyphCache::default(), edited: HashMap::new(), video_frames: None }
    }

    /// Manifest font files the display list needs that are not loaded yet (load them, then draw).
    pub fn missing_fonts(&self, dl: &DisplayList) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for it in &dl.items {
            if let DLItem::Text { body, .. } = it {
                for f in self.text.missing_fonts(body) {
                    if !out.contains(&f) {
                        out.push(f);
                    }
                }
            }
        }
        out
    }

    /// Forget what was decoded from one asset key (its bytes arrived or changed): the key itself, the variants
    /// derived from it (`<key>/poster`, `<key>/recolor/…`) and its edited versions.
    pub fn invalidate_asset(&mut self, key: &str) {
        let derived = format!("{key}/");
        self.images.retain(|k| k != key && !k.starts_with(&derived));
        self.edited.retain(|k, _| !(k.starts_with(key) && k[key.len()..].starts_with(['|', '/'])));
    }

    /// Forget decoded images (e.g. when asset bytes may have changed).
    pub fn clear_images(&mut self) {
        self.images.clear();
        self.edited.clear();
    }

    /// Resolve and draw a page into a new pixmap of `dims × scale` pixels.
    pub fn render_page(&mut self, doc: &oc_schema::DesignDoc, page_id: &str, resolve: &ResolveOptions, assets: &dyn AssetProvider, opts: &RenderOptions) -> Option<tiny_skia::Pixmap> {
        let dl = resolve_page(doc, page_id, resolve);
        self.render(&dl, assets, opts)
    }

    /// Draw a display list into a new pixmap of `width × height × scale` pixels.
    pub fn render(&mut self, dl: &DisplayList, assets: &dyn AssetProvider, opts: &RenderOptions) -> Option<tiny_skia::Pixmap> {
        let w = (dl.width * opts.scale).round().max(1.0) as u32;
        let h = (dl.height * opts.scale).round().max(1.0) as u32;
        let mut canvas = Canvas::new(w, h)?;
        if let Some(bg) = opts.background {
            let mut p = Paint::default();
            p.set_color(bg);
            canvas.fill_all(&p);
        }
        canvas.scale(opts.scale as f32, opts.scale as f32);
        self.draw(&mut canvas, dl, assets);
        Some(canvas.into_pixmap())
    }

    /// Draw a whole display list. The canvas must already be scaled for DPR/zoom by the caller.
    pub fn draw(&mut self, canvas: &mut Canvas, dl: &DisplayList, assets: &dyn AssetProvider) {
        let depth = canvas.depth();
        if let Some(r) = rect(0.0, 0.0, dl.width, dl.height) {
            let page_path = tiny_skia::PathBuilder::from_rect(r);
            for bg in &dl.background {
                self.fill_path_with(canvas, &page_path, bg, assets);
            }
        }
        for it in &dl.items {
            self.item(canvas, it, assets);
        }
        canvas.restore_to(depth);
    }

    pub(crate) fn item(&mut self, canvas: &mut Canvas, it: &DLItem, assets: &dyn AssetProvider) {
        match it {
            DLItem::Save { matrix } => {
                canvas.save();
                canvas.concat(tr(matrix));
            }
            DLItem::Restore => canvas.restore(),
            DLItem::Layer { opacity, blend: b, effects, .. } => {
                let m = canvas.transform();
                let (sx, sy) = axis_scales(&m);
                let mut filters = Vec::new();
                for e in effects {
                    match e {
                        Effect::DropShadow(d) => filters.push(Filter::DropShadow {
                            dx: m.sx * d.dx as f32 + m.kx * d.dy as f32,
                            dy: m.ky * d.dx as f32 + m.sy * d.dy as f32,
                            sx: (d.blur / 2.0) as f32 * sx,
                            sy: (d.blur / 2.0) as f32 * sy,
                            color: color(&d.color),
                        }),
                        Effect::Blur(b) => filters.push(Filter::Blur { sx: (b.radius / 2.0) as f32 * sx, sy: (b.radius / 2.0) as f32 * sy }),
                        // The TS painter draws no outline effect (no CanvasKit image filter is attached for it).
                        Effect::Outline(_) => {}
                    }
                }
                let opacity = if opacity.is_nan() { 0.0 } else { *opacity as f32 };
                canvas.save_layer(LayerPaint { opacity, blend: blend(b), filters });
            }
            DLItem::Mask { mode, .. } => canvas.save_mask_layer(*mode == oc_schema::elements::MaskMode::Luminance),
            DLItem::MaskContent => canvas.begin_masked_content(),
            DLItem::Clip { path: d } => {
                if let Some(p) = path::parse(d).and_then(|p| p.to_skia()) {
                    canvas.clip_path(&p, false);
                } else if path::parse(d).is_some() {
                    // A valid but empty path clips everything away (Skia: empty clip).
                    canvas.clip_empty();
                }
            }
            DLItem::Path { path: d, fills, stroke, .. } => {
                let Some(pd) = path::parse(d) else { return };
                let Some(p) = pd.to_skia() else { return };
                for f in fills {
                    self.fill_path_with(canvas, &p, f, assets);
                }
                if let Some(s) = stroke {
                    let Some((fill, st)) = self.stroke_paint(&s.stroke, &s.paint, assets, canvas) else { return };
                    let paint = fill.paint();
                    match s.stroke.align {
                        StrokeAlign::Inside => {
                            canvas.save();
                            canvas.clip_path(&p, false);
                            canvas.stroke_path(&p, &paint, &st);
                            canvas.restore();
                        }
                        StrokeAlign::Outside => {
                            canvas.save();
                            canvas.clip_path(&p, true);
                            canvas.stroke_path(&p, &paint, &st);
                            canvas.restore();
                        }
                        _ => canvas.stroke_path(&p, &paint, &st),
                    }
                }
            }
            DLItem::Line { points, stroke, paint, start_marker, end_marker, .. } => self.line(canvas, points, stroke, paint, start_marker, end_marker, assets),
            DLItem::Image { asset, dst, crop, edits, clip, .. } => {
                self.draw_image(canvas, image::ImageOp { asset, dst, crop: crop.into(), edits: edits.as_ref(), clip: clip.as_deref(), video: None }, assets);
            }
            DLItem::Video { asset, dst, crop, source_time_ms, clip, key, src_el, edits } => {
                let frame = match (source_time_ms, &self.video_frames) {
                    (Some(t), Some(v)) => v.frame(asset, *t, key.as_deref().unwrap_or(src_el)),
                    _ => None,
                };
                let poster = format!("{asset}/poster");
                self.draw_image(canvas, image::ImageOp { asset: &poster, dst, crop: crop.into(), edits: edits.as_ref(), clip: clip.as_deref(), video: Some(frame) }, assets);
            }
            DLItem::Text { key, body, bbox, effect, valign, curve, fit, .. } => {
                self.draw_text(canvas, key, body, bbox, effect.as_ref(), valign.as_ref(), *curve, fit.is_some());
            }
            DLItem::Qr { modules, size, bbox, fg, bg, .. } => {
                let mut bgp = Paint { anti_alias: false, ..Default::default() };
                bgp.set_color(color(bg));
                if let Some(r) = rect(bbox.x, bbox.y, bbox.w, bbox.h) {
                    canvas.fill_rect(r, &bgp);
                }
                let quiet = 2.0;
                let n = *size as f64;
                let cell = bbox.w.min(bbox.h) / (n + quiet * 2.0);
                let ox = bbox.x + (bbox.w - cell * n) / 2.0;
                let oy = bbox.y + (bbox.h - cell * n) / 2.0;
                let mut pb = tiny_skia::PathBuilder::new();
                for r in 0..*size {
                    for c in 0..*size {
                        if modules.get(r * size + c).copied().unwrap_or(0) != 0
                            && let Some(rr) = rect(ox + c as f64 * cell, oy + r as f64 * cell, cell + 0.02, cell + 0.02)
                        {
                            pb.push_rect(rr);
                        }
                    }
                }
                if let Some(p) = pb.finish() {
                    let mut fgp = Paint { anti_alias: false, ..Default::default() };
                    fgp.set_color(color(fg));
                    canvas.fill_path(&p, &fgp);
                }
            }
            DLItem::Placeholder { bbox, .. } => {
                let mut p = Paint { anti_alias: false, ..Default::default() };
                p.set_color(rgba(0.6, 0.6, 0.65, 0.25));
                if let Some(r) = rect(bbox.x, bbox.y, bbox.w, bbox.h) {
                    canvas.fill_rect(r, &p);
                }
            }
        }
    }

    /// Fill `path` with a resolved paint (anti-aliased).
    pub(crate) fn fill_path_with(&mut self, canvas: &mut Canvas, path: &tiny_skia::Path, p: &ResolvedPaint, assets: &dyn AssetProvider) {
        let m = canvas.transform();
        let Some(fill) = self.paint(p, assets, &m) else { return };
        let paint = fill.paint();
        if let Some(d) = fill.decal {
            canvas.save();
            canvas.clip_rect(d);
            canvas.fill_path(path, &paint);
            canvas.restore();
        } else {
            canvas.fill_path(path, &paint);
        }
    }

    /// A paint for a resolved paint (`painter.paint`); a missing image fill becomes light grey.
    pub(crate) fn paint(&mut self, p: &ResolvedPaint, assets: &dyn AssetProvider, m: &Transform) -> Option<Fill> {
        let mut paint = Paint { anti_alias: true, ..Default::default() };
        let fallback = |paint: &mut Paint| paint.set_color(rgba(0.85, 0.87, 0.9, 1.0));
        let mut decal = None;
        let mut image_src: Option<PaintSrc> = None;
        match p {
            ResolvedPaint::Solid { color: c } => paint.set_color(color(c)),
            ResolvedPaint::Linear { stops: s, angle, bbox } => {
                let ((x0, y0), (x1, y1)) = linear_points(*angle, bbox);
                let st = stops(s);
                match LinearGradient::new(Point::from_xy(x0 as f32, y0 as f32), Point::from_xy(x1 as f32, y1 as f32), st.clone(), SpreadMode::Pad, Transform::identity()) {
                    Some(sh) => paint.shader = sh,
                    None => match st.last() {
                        Some(_) => paint.set_color(last_color(s)),
                        None => fallback(&mut paint),
                    },
                }
            }
            ResolvedPaint::Radial { stops: s, bbox } => {
                let c = Point::from_xy((bbox.x + bbox.w / 2.0) as f32, (bbox.y + bbox.h / 2.0) as f32);
                let r = (bbox.w.max(bbox.h) / 2.0) as f32;
                match RadialGradient::new(c, 0.0, c, r, stops(s), SpreadMode::Pad, Transform::identity()) {
                    Some(sh) => paint.shader = sh,
                    None if !s.is_empty() => paint.set_color(last_color(s)),
                    None => fallback(&mut paint),
                }
            }
            ResolvedPaint::Image { asset, fit, bbox } => {
                let (sx, sy) = axis_scales(m);
                let img = match self.images.get(asset, assets) {
                    Some(Decoded::Raster(r)) => Some(r),
                    Some(Decoded::Svg(s)) => {
                        let (w, h) = s.size();
                        let k = image::fit_scale(fit, bbox, f64::from(w), f64::from(h)) * f64::from(sx.max(sy));
                        s.raster((f64::from(w) * k).ceil() as u32, (f64::from(h) * k).ceil() as u32)
                    }
                    None => None,
                };
                match img {
                    None => fallback(&mut paint),
                    Some(img) => {
                        let (iw, ih) = (f64::from(img.width()), f64::from(img.height()));
                        let natural = match self.images.get(asset, assets) {
                            Some(Decoded::Svg(s)) => {
                                let (w, h) = s.size();
                                (f64::from(w), f64::from(h))
                            }
                            _ => (iw, ih),
                        };
                        let s = image::fit_scale(fit, bbox, natural.0, natural.1);
                        let (dw, dh) = (natural.0 * s, natural.1 * s);
                        let tx = bbox.x + (bbox.w - dw) / 2.0;
                        let ty = bbox.y + (bbox.h - dh) / 2.0;
                        let t = Transform::from_row((dw / iw) as f32, 0.0, 0.0, (dh / ih) as f32, tx as f32, ty as f32);
                        let tile = *fit == ImageFit::Tile;
                        image_src = Some(PaintSrc::Image { pix: img.pixmap.clone(), t, spread: if tile { SpreadMode::Repeat } else { SpreadMode::Pad } });
                        if !tile {
                            decal = rect(tx, ty, dw, dh);
                        }
                    }
                }
            }
        }
        let src = image_src.unwrap_or(PaintSrc::Shader(paint.shader));
        Some(Fill { src, decal, aa: true })
    }

    /// Stroke paint and stroke (`strokePaint`): inside/outside strokes are drawn twice as wide and clipped.
    fn stroke_paint(&mut self, s: &DocStroke, p: &ResolvedPaint, assets: &dyn AssetProvider, canvas: &Canvas) -> Option<(Fill, Stroke)> {
        let m = canvas.transform();
        let fill = self.paint(p, assets, &m)?;
        let width = if s.align == StrokeAlign::Center || matches!(s.align, StrokeAlign::Other(_)) { s.width } else { s.width * 2.0 };
        let mut st = Stroke {
            width: width.max(0.0) as f32,
            miter_limit: 4.0,
            line_cap: match s.cap {
                StrokeCap::Round => tiny_skia::LineCap::Round,
                StrokeCap::Square => tiny_skia::LineCap::Square,
                _ => tiny_skia::LineCap::Butt,
            },
            line_join: match s.join {
                StrokeJoin::Round => tiny_skia::LineJoin::Round,
                StrokeJoin::Bevel => tiny_skia::LineJoin::Bevel,
                _ => tiny_skia::LineJoin::Miter,
            },
            dash: None,
        };
        if let Some(d) = s.dash.as_ref().filter(|d| !d.is_empty()) {
            st.dash = StrokeDash::new(d.iter().map(|v| (v * s.width.max(1.0)) as f32).collect(), 0.0);
        }
        Some((fill, st))
    }

    #[allow(clippy::too_many_arguments)]
    fn line(&mut self, canvas: &mut Canvas, pts: &[[f64; 2]], s: &DocStroke, p: &ResolvedPaint, start: &LineMarker, end: &LineMarker, assets: &dyn AssetProvider) {
        if pts.len() < 2 {
            return;
        }
        let body = trim_line_for_markers(pts, start, end, s.width);
        if let Some(path) = PathData::polyline(&body).to_skia()
            && let Some((fill, st)) = self.stroke_paint(s, p, assets, canvas)
        {
            canvas.stroke_path(&path, &fill.paint(), &st);
        }
        let m = canvas.transform();
        let Some(fill) = self.paint(p, assets, &m) else { return };
        let fpaint = fill.paint();
        let undashed = DocStroke { dash: None, ..s.clone() };
        let Some((sfill, sst)) = self.stroke_paint(&undashed, p, assets, canvas) else { return };
        let spaint = sfill.paint();
        let n = pts.len();
        for (kind, from, at) in [(start, pts[1], pts[0]), (end, pts[n - 2], pts[n - 1])] {
            let Some(shape) = line_marker_shape(kind, s.width) else { continue };
            let Some(mp) = path::parse(&shape.d).and_then(|p| p.to_skia()) else { continue };
            let ang = (at[1] - from[1]).atan2(at[0] - from[0]);
            canvas.save();
            canvas.translate(at[0] as f32, at[1] as f32);
            canvas.rotate(ang.to_degrees() as f32);
            match shape.mode {
                MarkerMode::Fill => canvas.fill_path(&mp, &fpaint),
                MarkerMode::Stroke => canvas.stroke_path(&mp, &spaint, &sst),
            }
            canvas.restore();
        }
    }
}

fn last_color(s: &[oc_schema::primitives::GradientStop]) -> Color {
    s.last().map_or(Color::BLACK, |g| color(&g.color))
}

/// An anti-aliased solid-colour paint.
pub(crate) fn solid(c: Color) -> Paint<'static> {
    Paint { shader: Shader::SolidColor(c), anti_alias: true, ..Default::default() }
}

/// Unpremultiplied RGBA bytes of a pixmap (what PNG/JPEG encoders and `readPixels(Unpremul)` give).
pub fn to_rgba8(p: &tiny_skia::Pixmap) -> Vec<u8> {
    let mut v = p.data().to_vec();
    crate::assets::unpremultiply(&mut v);
    v
}

/// PNG of a pixmap (unpremultiplied RGBA).
pub fn encode_png(p: &tiny_skia::Pixmap) -> Vec<u8> {
    let mut out = Vec::new();
    let enc = ::image::codecs::png::PngEncoder::new(&mut out);
    let _ = ::image::ImageEncoder::write_image(enc, &to_rgba8(p), p.width(), p.height(), ::image::ExtendedColorType::Rgba8);
    out
}

/// JPEG of a pixmap composited over white, `quality` 1..=100.
pub fn encode_jpeg(p: &tiny_skia::Pixmap, quality: u8) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((p.width() * p.height() * 3) as usize);
    for px in p.pixels() {
        let inv = 255 - u16::from(px.alpha());
        for c in [px.red(), px.green(), px.blue()] {
            rgb.push((u16::from(c) + inv).min(255) as u8);
        }
    }
    let mut out = Vec::new();
    let enc = ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100));
    let _ = ::image::ImageEncoder::write_image(enc, &rgb, p.width(), p.height(), ::image::ExtendedColorType::Rgb8);
    out
}

impl Renderer {
    /// Rasterise display-list items (page space through `ctm`) into a transparent pixmap of the page-space
    /// rectangle `(x0, y0, w, h)` at `scale` pixels per px. Used by the vector writers for what has no vector
    /// equivalent (blur and shadow effects, soft text effects).
    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_items(&mut self, items: &[DLItem], ctm: &[f64; 6], x0: f64, y0: f64, w: f64, h: f64, scale: f64, assets: &dyn AssetProvider) -> Option<tiny_skia::Pixmap> {
        let pw = (w * scale).ceil().max(1.0) as u32;
        let ph = (h * scale).ceil().max(1.0) as u32;
        let mut canvas = Canvas::new(pw, ph)?;
        canvas.scale(scale as f32, scale as f32);
        canvas.translate(-x0 as f32, -y0 as f32);
        canvas.concat(tr(ctm));
        let depth = canvas.depth();
        for it in items {
            self.item(&mut canvas, it, assets);
        }
        canvas.restore_to(depth);
        Some(canvas.into_pixmap())
    }

    /// An image with its edits baked in, at the native resolution of its crop (`None` when the asset is missing).
    pub fn bake_image(&mut self, asset: &str, crop: &oc_schema::primitives::Rect, edits: Option<&oc_schema::elements::ImageEditStack>, assets: &dyn AssetProvider) -> Option<tiny_skia::Pixmap> {
        let (iw, ih) = match self.images.get(asset, assets)? {
            Decoded::Raster(r) => (f64::from(r.width()), f64::from(r.height())),
            Decoded::Svg(s) => {
                let (w, h) = s.size();
                (f64::from(w).max(1.0) * 2.0, f64::from(h).max(1.0) * 2.0)
            }
        };
        let w = (crop.w * iw).round().clamp(1.0, 8192.0);
        let h = (crop.h * ih).round().clamp(1.0, 8192.0);
        let mut canvas = Canvas::new(w as u32, h as u32)?;
        let dst = oc_schema::primitives::Rect { x: 0.0, y: 0.0, w, h };
        self.draw_image(&mut canvas, image::ImageOp { asset, dst: &dst, crop: *crop, edits, clip: None, video: None }, assets);
        Some(canvas.into_pixmap())
    }

    /// The decoded asset for a key (cached; the PDF writer's image source).
    #[cfg(feature = "pdf")]
    pub(crate) fn images_get(&mut self, key: &str, assets: &dyn AssetProvider) -> Option<Decoded> {
        self.images.get(key, assets)
    }

    /// Decoded size of an asset (natural size for SVG), if available.
    pub fn image_size(&mut self, asset: &str, assets: &dyn AssetProvider) -> Option<(f64, f64)> {
        Some(match self.images.get(asset, assets)? {
            Decoded::Raster(r) => (f64::from(r.width()), f64::from(r.height())),
            Decoded::Svg(s) => {
                let (w, h) = s.size();
                (f64::from(w), f64::from(h))
            }
        })
    }
}
