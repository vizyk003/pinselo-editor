// SPDX-License-Identifier: Apache-2.0
//! oc-render in the browser (feature `render`): the `@oc/render-core` resolver and the painter that replaces
//! `@oc/render-canvaskit`.
//!
//! * [`DisplayList`] (JS `DisplayList`): a resolved page held on the Rust side. `resolvePage(docRef, pageId,
//!   opts)` returns one; JS reads `width`/`height` directly and the op list only when it asks (`toJs`), so a
//!   page resolved per drag frame never crosses the boundary as data.
//! * [`RenderTarget`]: a tile (or any rectangle) of a device being painted — a tiny-skia region canvas
//!   (`Canvas::new_region`) with the Skia-style save/restore/transform/clip/layer calls the editor's scene needs,
//!   plus the scene primitives (workspace fill, page shadow, whiteboard dots). `end()` hands back unpremultiplied
//!   RGBA for `putImageData`.
//! * [`Painter`]: oc-render's `Renderer` sharing the editor's oc-text engine (one font registry and one layout
//!   cache for measuring, caret geometry and drawing), the asset bytes fed from JS, video frames fed from JS, and
//!   the group/dirty-region queries for partial repaints.
//! * The pure helpers apps/web imports from `@oc/render-core`: `smoothPath`, `sliceColor`, `pageSettleMs`,
//!   `clickStepCount`, `transitionProgress`, `transitionFrame`.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use oc_render::bounds::{Dirty, Group};
use oc_render::raster::canvas::{Filter, LayerPaint};
use oc_render::tiny_skia::{self, BlendMode, Color, IntRect, Paint, PathBuilder, Pixmap, Rect as SkRect};
use oc_render::{AssetProvider, Canvas, Renderer, ResolveOptions, VideoFrames};
use oc_schema::design::{Page, TransitionKind};
use oc_schema::elements::{AnimDirection, ChartEl, Element};
use oc_text::{FontManifest, FontRegistry, TextEngine};
use serde_json::{Value, json};
use wasm_bindgen::Clamped;
use wasm_bindgen::prelude::*;

use crate::convert::{arg, element, err, js, opt_arg, to_js, value};
use crate::doc::DocRef;
use crate::text::{SharedEngine, TextService};

// ------------------------------------------------------------------------------------------------ display lists

/// A resolved page (`@oc/render-core` `DisplayList`), held in Rust.
#[wasm_bindgen]
pub struct DisplayList {
    dl: Rc<oc_render::DisplayList>,
    /// Groups with bounds, computed on first use by a painter (text bounds need layouts).
    groups: RefCell<Option<Rc<Vec<Group>>>>,
}

impl DisplayList {
    fn new(dl: oc_render::DisplayList) -> DisplayList {
        DisplayList { dl: Rc::new(dl), groups: RefCell::new(None) }
    }

    pub fn inner(&self) -> &oc_render::DisplayList {
        &self.dl
    }
}

#[wasm_bindgen]
impl DisplayList {
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> f64 {
        self.dl.width
    }

    #[wasm_bindgen(getter)]
    pub fn height(&self) -> f64 {
        self.dl.height
    }

    /// The whole list as the TS object (`{ width, height, background, items, assets }`).
    #[wasm_bindgen(js_name = toJs)]
    pub fn to_js_value(&self) -> Result<JsValue, JsError> {
        to_js(&*self.dl)
    }

    /// Assets referenced, in first-use order.
    #[wasm_bindgen(getter)]
    pub fn assets(&self) -> Vec<String> {
        self.dl.assets.iter().map(|a| a.to_string()).collect()
    }

    /// Video frames the list draws: `[{ asset, key, sourceTimeMs }]` for every video op with a source time
    /// (posters are plain assets).
    pub fn videos(&self) -> JsValue {
        let mut out = Vec::new();
        for it in &self.dl.items {
            if let oc_render::DLItem::Video { asset, source_time_ms: Some(t), key, src_el, .. } = it {
                out.push(json!({ "asset": asset, "key": key.as_deref().unwrap_or(src_el.as_str()), "sourceTimeMs": t }));
            }
        }
        js(&out)
    }

    /// Whether the background paints every page pixel opaquely (a solid colour or gradient without
    /// transparency): a page shadow under it then only shows around the page.
    #[wasm_bindgen(getter)]
    pub fn opaque(&self) -> bool {
        use oc_render::ResolvedPaint as P;
        let alpha = |c: &oc_schema::primitives::ColorValue| oc_schema::color::to_rgba(c).a >= 1.0;
        self.dl.background.iter().any(|p| match p {
            P::Solid { color } => alpha(color),
            P::Linear { stops, .. } | P::Radial { stops, .. } => !stops.is_empty() && stops.iter().all(|s| alpha(&s.color)),
            P::Image { .. } => false,
        })
    }

    /// Another handle to the same list (for calls that take a list by value, like `drawTransition`'s `prev`).
    pub fn share(&self) -> DisplayList {
        DisplayList { dl: self.dl.clone(), groups: RefCell::new(self.groups.borrow().clone()) }
    }

    /// Number of ops (diagnostics).
    #[wasm_bindgen(getter, js_name = itemCount)]
    pub fn item_count(&self) -> u32 {
        self.dl.items.len() as u32
    }
}

/// `resolvePage(doc, pageId, opts?)`.
#[wasm_bindgen(js_name = resolvePage)]
pub fn resolve_page(doc: &DocRef, page_id: &str, opts: JsValue) -> Result<DisplayList, JsError> {
    let o: ResolveOptions = opt_arg("opts", &opts)?.unwrap_or_default();
    Ok(DisplayList::new(oc_render::resolve_page(doc.doc(), page_id, &o)))
}

// ------------------------------------------------------------------------------------------------ pure helpers

/// `svgToElements(svg, w, h, crop?)` (DS-100): an SVG's paths as path elements (JSON, `parentId` `''`, bottom
/// first) in the pixel space of a `w × h` image element showing it through `crop` (`{ x, y, w, h }` in 0..1),
/// or `null` when nothing converts.
#[wasm_bindgen(js_name = svgToElements)]
pub fn svg_to_elements(svg: &str, w: f64, h: f64, crop: JsValue) -> Result<JsValue, JsError> {
    let c: Option<oc_schema::primitives::StoredRect> = opt_arg("crop", &crop)?;
    let c = c.map_or((0.0, 0.0, 1.0, 1.0), |c| (c.x, c.y, c.w, c.h));
    match oc_render::svg_import::svg_to_elements(svg, |sw, sh| oc_render::svg_import::Placement::image_box(sw, sh, w, h, c)) {
        Some(els) => to_js(&els),
        None => Ok(JsValue::NULL),
    }
}

/// `smoothPath(points)`: an SVG path through freehand points.
#[wasm_bindgen(js_name = smoothPath)]
pub fn smooth_path(points: JsValue) -> Result<String, JsError> {
    let pts: Vec<[f64; 2]> = arg("points", &points)?;
    Ok(oc_render::builders::smooth_path(&pts))
}

/// `sliceColor(chart, i)`.
#[wasm_bindgen(js_name = sliceColor)]
pub fn slice_color(el: JsValue, i: u32) -> Result<JsValue, JsError> {
    let c: ChartEl = arg("el", &el)?;
    to_js(&oc_render::builders::slice_color(&c, i as usize))
}

fn elements(els: &JsValue) -> Result<Vec<Element>, JsError> {
    match value(els)? {
        Value::Array(v) => Ok(v.into_iter().map(element).collect()),
        Value::Null => Ok(Vec::new()),
        _ => Err(err("els: expected an array of elements")),
    }
}

/// `pageSettleMs(page, els)`.
#[wasm_bindgen(js_name = pageSettleMs)]
pub fn page_settle_ms(page: JsValue, els: JsValue) -> Result<f64, JsError> {
    let p: Page = arg("page", &page)?;
    let e = elements(&els)?;
    let refs: Vec<&Element> = e.iter().collect();
    Ok(oc_render::motion::page_settle_ms(&p, &refs))
}

/// `clickStepCount(els)`.
#[wasm_bindgen(js_name = clickStepCount)]
pub fn click_step_count(els: JsValue) -> Result<f64, JsError> {
    let e = elements(&els)?;
    let refs: Vec<&Element> = e.iter().collect();
    Ok(oc_render::motion::click_step_count(&refs))
}

/// `transitionProgress(page, t)`.
#[wasm_bindgen(js_name = transitionProgress)]
pub fn transition_progress(page: JsValue, t: f64) -> Result<f64, JsError> {
    let p: Page = arg("page", &page)?;
    Ok(oc_render::motion::transition_progress(&p, t))
}

/// `transitionFrame(kind, p, direction?)`.
#[wasm_bindgen(js_name = transitionFrame)]
pub fn transition_frame(kind: JsValue, p: f64, direction: JsValue) -> Result<JsValue, JsError> {
    let k: TransitionKind = arg("kind", &kind)?;
    let d: Option<AnimDirection> = opt_arg("direction", &direction)?;
    to_js(&oc_render::motion::transition_frame(&k, p, d.as_ref()))
}

// ------------------------------------------------------------------------------------------------ render target

fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color::from_rgba(r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a.clamp(0.0, 1.0)).unwrap_or(Color::BLACK)
}

fn solid(c: Color) -> Paint<'static> {
    let mut p = Paint { anti_alias: true, ..Default::default() };
    p.set_color(c);
    p
}

/// `erf` (Abramowitz–Stegun 7.1.26, |error| < 1.5e-7).
fn erf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let y = 1.0 - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t + 0.254_829_592) * t * (-x * x).exp();
    if x < 0.0 { -y } else { y }
}

/// Coverage of a Gaussian-blurred [lo, hi) interval at pixel `i` (centre `i + 0.5`).
fn blurred_span(i: i32, lo: f64, hi: f64, sigma: f64) -> f64 {
    let c = f64::from(i) + 0.5;
    if sigma <= 0.01 {
        return if c >= lo && c < hi { 1.0 } else { 0.0 };
    }
    let k = 1.0 / (sigma * std::f64::consts::SQRT_2);
    0.5 * (erf((c - lo) * k) - erf((c - hi) * k))
}

/// One tile (or any rectangle) of a device being painted. Coordinates are the device's: a tile at (512, 256)
/// starts with the identity transform, and drawing at device (520, 260) lands at its pixel (8, 4).
#[wasm_bindgen]
pub struct RenderTarget {
    canvas: Option<Canvas>,
}

impl Default for RenderTarget {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderTarget {
    fn c(&mut self) -> Result<&mut Canvas, JsError> {
        self.canvas.as_mut().ok_or_else(|| err("RenderTarget: begin() first"))
    }

    pub fn canvas_mut(&mut self) -> Option<&mut Canvas> {
        self.canvas.as_mut()
    }
}

#[wasm_bindgen]
impl RenderTarget {
    #[wasm_bindgen(constructor)]
    pub fn new() -> RenderTarget {
        RenderTarget { canvas: None }
    }

    /// Start painting the device rectangle `(x, y, w, h)` (transparent, identity transform).
    pub fn begin(&mut self, x: i32, y: i32, w: u32, h: u32) -> Result<(), JsError> {
        self.canvas = Some(Canvas::new_region(x, y, w.max(1), h.max(1)).ok_or_else(|| err("RenderTarget: bad size"))?);
        Ok(())
    }

    /// Finish and return the pixels: unpremultiplied RGBA rows of the rectangle (for `new ImageData`).
    pub fn end(&mut self) -> Result<Clamped<Vec<u8>>, JsError> {
        let c = self.canvas.take().ok_or_else(|| err("RenderTarget: begin() first"))?;
        let mut v = c.into_pixmap().take();
        oc_render::assets::unpremultiply(&mut v);
        Ok(Clamped(v))
    }

    /// Finish, keeping only the inner `(x, y, w, h)` (pixmap coordinates) of the painted rectangle — a tile
    /// painted with a margin around it so filters see their whole input.
    #[wasm_bindgen(js_name = endCrop)]
    pub fn end_crop(&mut self, x: u32, y: u32, w: u32, h: u32) -> Result<Clamped<Vec<u8>>, JsError> {
        let c = self.canvas.take().ok_or_else(|| err("RenderTarget: begin() first"))?;
        let p = c.into_pixmap();
        let r = IntRect::from_xywh(x as i32, y as i32, w, h).ok_or_else(|| err("endCrop: bad rect"))?;
        let mut v = p.clone_rect(r).ok_or_else(|| err("endCrop: outside"))?.take();
        oc_render::assets::unpremultiply(&mut v);
        Ok(Clamped(v))
    }

    #[wasm_bindgen(getter)]
    pub fn active(&self) -> bool {
        self.canvas.is_some()
    }

    /// A copy of the painted pixels `(x, y, w, h)` (pixmap coordinates, premultiplied), to composite later.
    pub fn snapshot(&mut self, x: u32, y: u32, w: u32, h: u32) -> Result<Snapshot, JsError> {
        let c = self.c()?;
        let r = IntRect::from_xywh(c.origin().0 + x as i32, c.origin().1 + y as i32, w, h).ok_or_else(|| err("snapshot: bad rect"))?;
        let pix = c.read_target(r).ok_or_else(|| err("snapshot: outside"))?;
        let alpha = pix.data().as_chunks::<4>().0.iter().fold((true, true), |(o, e), p| (o && p[3] == 255, e && p[3] == 0));
        Ok(Snapshot { pix, opaque: alpha.0, empty: alpha.1 })
    }

    /// Composite a snapshot over the pixels at `(x, y)` (pixmap coordinates), ignoring the transform. Call it
    /// with no clip in effect (outside any `save`).
    pub fn compose(&mut self, snap: &Snapshot, x: i32, y: i32) -> Result<(), JsError> {
        let c = self.c()?;
        if c.depth() != 0 {
            return Err(err("compose: restore first (no save or clip may be in effect)"));
        }
        if snap.empty {
            return Ok(());
        }
        let (ox, oy) = c.origin();
        // An opaque snapshot replaces what is there: a plain copy, no blending.
        let blend = if snap.opaque { BlendMode::Source } else { BlendMode::SourceOver };
        c.draw_device_pixmap(ox + x, oy + y, &snap.pix, 1.0, blend);
        Ok(())
    }

    pub fn save(&mut self) -> Result<(), JsError> {
        self.c()?.save();
        Ok(())
    }

    pub fn restore(&mut self) -> Result<(), JsError> {
        self.c()?.restore();
        Ok(())
    }

    pub fn translate(&mut self, x: f32, y: f32) -> Result<(), JsError> {
        self.c()?.translate(x, y);
        Ok(())
    }

    pub fn scale(&mut self, sx: f32, sy: f32) -> Result<(), JsError> {
        self.c()?.scale(sx, sy);
        Ok(())
    }

    /// Rotate by `deg` degrees.
    pub fn rotate(&mut self, deg: f32) -> Result<(), JsError> {
        self.c()?.rotate(deg);
        Ok(())
    }

    /// The current transform `[sx, ky, kx, sy, tx, ty]`.
    pub fn transform(&mut self) -> Result<Vec<f32>, JsError> {
        let m = self.c()?.transform();
        Ok(vec![m.sx, m.ky, m.kx, m.sy, m.tx, m.ty])
    }

    #[wasm_bindgen(js_name = clipRect)]
    pub fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> Result<(), JsError> {
        let c = self.c()?;
        match SkRect::from_xywh(x, y, w, h) {
            Some(r) => c.clip_rect(r),
            None => c.clip_empty(),
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = clipCircle)]
    pub fn clip_circle(&mut self, cx: f32, cy: f32, r: f32) -> Result<(), JsError> {
        let c = self.c()?;
        match PathBuilder::from_circle(cx, cy, r) {
            Some(p) => c.clip_path(&p, false),
            None => c.clip_empty(),
        }
        Ok(())
    }

    /// Push a layer composited with `alpha` and (when > 0) a Gaussian blur of σ = `blur` local units.
    #[wasm_bindgen(js_name = saveLayer)]
    pub fn save_layer(&mut self, alpha: f32, blur: f32) -> Result<(), JsError> {
        let c = self.c()?;
        let m = c.transform();
        let (sx, sy) = (m.sx.hypot(m.ky), m.kx.hypot(m.sy));
        let filters = if blur > 0.0 { vec![Filter::Blur { sx: blur * sx, sy: blur * sy }] } else { Vec::new() };
        c.save_layer(LayerPaint { opacity: alpha.clamp(0.0, 1.0), filters, ..Default::default() });
        Ok(())
    }

    /// Fill the clip with a colour, replacing what is there (`clear`).
    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) -> Result<(), JsError> {
        let mut p = solid(rgba(r, g, b, a));
        p.blend_mode = BlendMode::Source;
        self.c()?.fill_all(&p);
        Ok(())
    }

    #[wasm_bindgen(js_name = fillRect)]
    #[allow(clippy::too_many_arguments)]
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, g: f32, b: f32, a: f32) -> Result<(), JsError> {
        if let Some(rr) = SkRect::from_xywh(x, y, w, h) {
            self.c()?.fill_rect(rr, &solid(rgba(r, g, b, a)));
        }
        Ok(())
    }

    /// The soft shadow of a rectangle (`MaskFilter.MakeBlur(Normal, sigma)` on `drawRect`), computed analytically
    /// in device space (the transform must be axis-aligned; the scene's is a scale and a translation). Pixels inside
    /// `hole` (local units; skipped when its width is not positive) are left alone: the opaque page drawn over the
    /// shadow covers them, so only the band around the page costs anything.
    #[wasm_bindgen(js_name = shadowRect)]
    #[allow(clippy::too_many_arguments)]
    pub fn shadow_rect(&mut self, x: f32, y: f32, w: f32, h: f32, sigma: f32, r: f32, g: f32, b: f32, a: f32, hx: f32, hy: f32, hw: f32, hh: f32) -> Result<(), JsError> {
        let c = self.c()?;
        let m = c.transform();
        // The hole in device pixels, shrunk by one so anti-aliased page edges keep the shadow under them.
        let hole = (hw > 0.0 && hh > 0.0).then(|| {
            let (a0, b0) = (m.sx * hx + m.tx, m.sy * hy + m.ty);
            let (a1, b1) = (m.sx * (hx + hw) + m.tx, m.sy * (hy + hh) + m.ty);
            ((a0.min(a1) + 1.0).ceil() as i32, (b0.min(b1) + 1.0).ceil() as i32, (a0.max(a1) - 1.0).floor() as i32, (b0.max(b1) - 1.0).floor() as i32)
        });
        let (x0, y0) = (f64::from(m.sx * x + m.tx), f64::from(m.sy * y + m.ty));
        let (x1, y1) = (f64::from(m.sx * (x + w) + m.tx), f64::from(m.sy * (y + h) + m.ty));
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let (y0, y1) = (y0.min(y1), y0.max(y1));
        let sd = f64::from(sigma) * f64::from(m.sx.abs().max(m.sy.abs()));
        let reach = (sd * 3.0).ceil() + 1.0;
        let dev = c.clip_bounds();
        let l = ((x0 - reach).floor() as i32).max(dev.left());
        let t = ((y0 - reach).floor() as i32).max(dev.top());
        let rr = ((x1 + reach).ceil() as i32).min(dev.right());
        let bb = ((y1 + reach).ceil() as i32).min(dev.bottom());
        if rr <= l || bb <= t {
            return Ok(());
        }
        // Covered entirely by the hole: nothing to draw.
        if let Some((h0, v0, h1, v1)) = hole
            && l >= h0
            && t >= v0
            && rr <= h1
            && bb <= v1
        {
            return Ok(());
        }
        let gx: Vec<f32> = (l..rr).map(|i| blurred_span(i, x0, x1, sd) as f32).collect();
        let gy: Vec<f32> = (t..bb).map(|i| blurred_span(i, y0, y1, sd) as f32).collect();
        let col = rgba(r, g, b, a).premultiply();
        let ch = [col.red() * 255.0, col.green() * 255.0, col.blue() * 255.0, col.alpha() * 255.0];
        // Rows of the band: whole rows above/below the hole, the left and right parts beside it.
        let mut spans: Vec<(i32, i32, i32)> = Vec::new();
        for y in t..bb {
            match hole {
                Some((h0, v0, h1, v1)) if y >= v0 && y < v1 => {
                    if l < h0 {
                        spans.push((y, l, h0.min(rr)));
                    }
                    if rr > h1 {
                        spans.push((y, h1.max(l), rr));
                    }
                }
                _ => spans.push((y, l, rr)),
            }
        }
        // Each span as a one-row pixmap (premultiplied colour × coverage), composited through the clip.
        for (y, x0s, x1s) in spans {
            if x1s <= x0s {
                continue;
            }
            let vy = gy[(y - t) as usize];
            if vy <= 0.0 {
                continue;
            }
            let Some(mut row) = Pixmap::new((x1s - x0s) as u32, 1) else { continue };
            for (i, px) in row.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let k = gx[(x0s - l) as usize + i] * vy;
                for (c, v) in px.iter_mut().zip(ch) {
                    *c = (v * k).round() as u8;
                }
            }
            c.draw_device_pixmap(x0s, y, &row, 1.0, BlendMode::SourceOver);
        }
        Ok(())
    }

    /// Round dots of `radius` (local units) at `points` (`[x0, y0, x1, y1, …]`), only those in the clip.
    #[allow(clippy::too_many_arguments)]
    pub fn dots(&mut self, points: &[f32], radius: f32, r: f32, g: f32, b: f32, a: f32) -> Result<(), JsError> {
        let c = self.c()?;
        let m = c.transform();
        let inv = m.invert().unwrap_or_default();
        let clip = c.clip_bounds();
        // The clip in local units (axis-aligned scene transform), widened by the radius.
        let mut lo = tiny_skia::Point::from_xy(clip.left() as f32, clip.top() as f32);
        let mut hi = tiny_skia::Point::from_xy(clip.right() as f32, clip.bottom() as f32);
        inv.map_point(&mut lo);
        inv.map_point(&mut hi);
        let (lx, hx) = (lo.x.min(hi.x) - radius, lo.x.max(hi.x) + radius);
        let (ly, hy) = (lo.y.min(hi.y) - radius, lo.y.max(hi.y) + radius);
        let mut pb = PathBuilder::new();
        for &[x, y] in points.as_chunks::<2>().0 {
            if x >= lx && x <= hx && y >= ly && y <= hy {
                pb.push_circle(x, y, radius);
            }
        }
        if let Some(path) = pb.finish() {
            c.fill_path(&path, &solid(rgba(r, g, b, a)));
        }
        Ok(())
    }
}

/// Premultiplied pixels kept for compositing (a cached tile layer).
#[wasm_bindgen]
pub struct Snapshot {
    pix: Pixmap,
    opaque: bool,
    empty: bool,
}

#[wasm_bindgen]
impl Snapshot {
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 {
        self.pix.width()
    }

    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.pix.height()
    }

    /// Every pixel opaque (composited as a copy).
    #[wasm_bindgen(getter)]
    pub fn opaque(&self) -> bool {
        self.opaque
    }

    /// Every pixel transparent (composited as nothing).
    #[wasm_bindgen(getter)]
    pub fn empty(&self) -> bool {
        self.empty
    }
}

// ------------------------------------------------------------------------------------------------ painter

/// Asset bytes fed from JS; keys asked for and not available are queued for the host to fetch (once).
#[derive(Default)]
struct Store {
    bytes: HashMap<String, Vec<u8>>,
    requested: RefCell<HashSet<String>>,
    missing: RefCell<Vec<String>>,
}

impl AssetProvider for Store {
    fn asset(&self, key: &str) -> Option<Cow<'_, [u8]>> {
        if let Some(b) = self.bytes.get(key) {
            return Some(Cow::Borrowed(b.as_slice()));
        }
        if self.requested.borrow_mut().insert(key.to_owned()) {
            self.missing.borrow_mut().push(key.to_owned());
        }
        None
    }
}

type Frames = Arc<Mutex<HashMap<String, Arc<Pixmap>>>>;

/// Video frames fed from JS by playback key.
struct JsFrames(Frames);

impl VideoFrames for JsFrames {
    fn frame(&self, _asset: &str, _t: f64, key: &str) -> Option<Arc<Pixmap>> {
        self.0.lock().ok()?.get(key).cloned()
    }
}

/// oc-render's painter over the shared text engine.
#[wasm_bindgen]
pub struct Painter {
    renderer: Renderer,
    text: SharedEngine,
    store: Store,
    frames: Frames,
}

impl Painter {
    /// Run `f` with the shared text engine inside the renderer (swapped in and back: the renderer owns a
    /// `TextEngine`, the editor shares one through `Rc<RefCell<…>>`; no copy, no second cache).
    fn with_text<R>(&mut self, f: impl FnOnce(&mut Renderer, &Store) -> R) -> R {
        let Painter { renderer, text, store, .. } = self;
        let mut shared = text.borrow_mut();
        std::mem::swap(&mut *shared, &mut renderer.text);
        let r = f(renderer, store);
        std::mem::swap(&mut *shared, &mut renderer.text);
        r
    }

    fn groups_of(&mut self, dl: &DisplayList) -> Rc<Vec<Group>> {
        if let Some(g) = dl.groups.borrow().as_ref() {
            return g.clone();
        }
        let g = Rc::new(self.with_text(|r, _| r.groups(&dl.dl)));
        *dl.groups.borrow_mut() = Some(g.clone());
        g
    }

    pub fn from_engine(text: SharedEngine) -> Painter {
        let placeholder = TextEngine::new(FontRegistry::new(FontManifest { version: 1.into(), families: Vec::new() }));
        let frames: Frames = Arc::new(Mutex::new(HashMap::new()));
        let mut renderer = Renderer::new(placeholder);
        renderer.video_frames = Some(Box::new(JsFrames(frames.clone())));
        Painter { renderer, text, store: Store::default(), frames }
    }
}

#[wasm_bindgen]
impl Painter {
    /// `new Painter(textService)`: draws with the service's fonts and layouts.
    #[wasm_bindgen(constructor)]
    pub fn new(text: &TextService) -> Painter {
        Painter::from_engine(text.engine())
    }

    /// Draw a display list onto a target (its current transform maps page space to device pixels). `background`
    /// / `items` choose the parts (the whiteboard grid goes between them). Groups outside the clip are skipped.
    pub fn draw(&mut self, target: &mut RenderTarget, dl: &DisplayList, background: bool, items: bool) -> Result<(), JsError> {
        let groups = self.groups_of(dl);
        let c = target.c()?;
        self.with_text(|r, store| r.draw_culled(c, &dl.dl, &groups, store, background, items));
        Ok(())
    }

    /// Draw the background (when `background`) and the groups `from..to` of a list (see `layering`).
    #[wasm_bindgen(js_name = drawGroups)]
    pub fn draw_groups(&mut self, target: &mut RenderTarget, dl: &DisplayList, from: u32, to: u32, background: bool) -> Result<(), JsError> {
        let groups = self.groups_of(dl);
        let c = target.c()?;
        self.with_text(|r, store| r.draw_groups(c, &dl.dl, &groups, from as usize..to as usize, store, background, true));
        Ok(())
    }

    /// Number of element groups of a list.
    #[wasm_bindgen(js_name = groupCount)]
    pub fn group_count(&mut self, dl: &DisplayList) -> u32 {
        self.groups_of(dl).len() as u32
    }

    /// How `next` can be painted in layers against `prev` while its groups `from..to` change: `{ from, to, below,
    /// above, reach }` (`below`/`above`: content hashes of the unchanged layers; `reach`: the filter reach of the
    /// changing groups), or `null`.
    pub fn layering(&mut self, prev: &DisplayList, next: &DisplayList) -> JsValue {
        let (og, ng) = (self.groups_of(prev), self.groups_of(next));
        match oc_render::layering(&prev.dl, &og, &next.dl, &ng) {
            Some(l) => {
                let reach = ng[l.from..l.to].iter().map(|g| g.reach).fold(0.0, f64::max);
                js(&json!({ "from": l.from, "to": l.to, "below": format!("{:016x}", l.below), "above": format!("{:016x}", l.above), "reach": reach }))
            }
            None => JsValue::NULL,
        }
    }

    /// The page-space offset `[dx, dy]` by which groups `nfrom..nto` of `next` are groups `pfrom..pto` of `prev`
    /// moved (a drag: the same ops, translated), `undefined` otherwise.
    pub fn translation(&mut self, prev: &DisplayList, pfrom: u32, pto: u32, next: &DisplayList, nfrom: u32, nto: u32) -> Option<Vec<f64>> {
        let (og, ng) = (self.groups_of(prev), self.groups_of(next));
        oc_render::translation(&prev.dl, &og, pfrom as usize..pto as usize, &next.dl, &ng, nfrom as usize..nto as usize).map(|(x, y)| vec![x, y])
    }

    /// Page-space bounds `[x, y, w, h, …]` of the groups that draw video (their pixels change without the list
    /// changing when a paused video seeks or loads a frame).
    #[wasm_bindgen(js_name = videoBounds)]
    pub fn video_bounds(&mut self, dl: &DisplayList) -> Vec<f64> {
        let g = self.groups_of(dl);
        let mut out = Vec::new();
        for gr in g.iter() {
            if dl.dl.items[gr.range.clone()].iter().any(|i| matches!(i, oc_render::DLItem::Video { .. }))
                && let Some(b) = &gr.bounds
            {
                out.extend([b.x, b.y, b.w, b.h]);
            }
        }
        out
    }

    /// `[x, y, w, h]` (page units) covering groups `from..to`, `undefined` when they draw nothing.
    #[wasm_bindgen(js_name = groupsBounds)]
    pub fn groups_bounds(&mut self, dl: &DisplayList, from: u32, to: u32) -> Option<Vec<f64>> {
        let g = self.groups_of(dl);
        oc_render::groups_bounds(&g, from as usize..to as usize).map(|r| vec![r.x, r.y, r.w, r.h])
    }

    /// Draw a page transition frame (`transitionFrame` result) of a `w × h` page: `prev` and `next` drawn with
    /// oc-render's `draw_transition` (Present and video export draw exactly this).
    #[wasm_bindgen(js_name = drawTransition)]
    pub fn draw_transition(&mut self, target: &mut RenderTarget, w: f64, h: f64, frame: JsValue, prev: Option<DisplayList>, next: &DisplayList) -> Result<(), JsError> {
        let f = transition_frame_arg(&frame)?;
        let c = target.c()?;
        let prev_dl = prev.map(|p| p.dl);
        let next_dl = next.dl.clone();
        self.with_text(|r, store| {
            oc_render::raster::transition::draw_transition(c, w, h, &f, |side, cv| match side {
                oc_render::raster::transition::Side::Next => r.draw(cv, &next_dl, store),
                oc_render::raster::transition::Side::Prev => {
                    if let Some(p) = &prev_dl {
                        r.draw(cv, p, store);
                    }
                }
            });
        });
        Ok(())
    }

    /// Page-space area that differs between two lists of a page: `null` (nothing), `"full"`, or `{ x, y, w, h }`.
    #[wasm_bindgen(js_name = dirtyRegion)]
    pub fn dirty_region(&mut self, old: &DisplayList, new: &DisplayList) -> JsValue {
        if Rc::ptr_eq(&old.dl, &new.dl) {
            return JsValue::NULL;
        }
        let (og, ng) = (self.groups_of(old), self.groups_of(new));
        let d = self.with_text(|r, _| r.dirty_region_fine(&old.dl, &og, &new.dl, &ng));
        match d {
            Dirty::None => JsValue::NULL,
            Dirty::Full => JsValue::from_str("full"),
            Dirty::Area(a) => js(&json!({ "x": a.x, "y": a.y, "w": a.w, "h": a.h })),
        }
    }

    /// How far (page units) filters of the list reach: tiles are painted with this much margin.
    #[wasm_bindgen(js_name = filterReach)]
    pub fn filter_reach(&mut self, dl: &DisplayList) -> f64 {
        self.groups_of(dl).iter().map(|g| g.reach).fold(0.0, f64::max)
    }

    /// Hand over an asset's encoded bytes (image, `<id>/poster`, `<id>/recolor/…`); what was decoded for the key
    /// (a placeholder while it loaded) is forgotten.
    /// `textOutline(doc, id)` (DS-100): a text element's glyphs as path data in its own box, with the paint of
    /// its first run — `{ d, fill }`, or `null` when it has no outlines (empty, or fonts not loaded yet).
    #[wasm_bindgen(js_name = textOutline)]
    pub fn text_outline(&mut self, doc: &DocRef, id: &str) -> Result<JsValue, JsError> {
        let Some(Element::Text(t)) = doc.doc().elements.get(id) else { return Ok(JsValue::NULL) };
        let t = t.clone();
        match self.with_text(|r, _| r.text_outline(&t)) {
            Some((d, fill)) => to_js(&json!({ "d": d, "fill": fill })),
            None => Ok(JsValue::NULL),
        }
    }

    #[wasm_bindgen(js_name = addAsset)]
    pub fn add_asset(&mut self, key: &str, bytes: Vec<u8>) {
        self.store.bytes.insert(key.to_owned(), bytes);
        self.store.requested.borrow_mut().insert(key.to_owned());
        self.renderer.invalidate_asset(key);
    }

    /// The asset could not be loaded: keep drawing its placeholder, do not ask again.
    #[wasm_bindgen(js_name = assetFailed)]
    pub fn asset_failed(&mut self, key: &str) {
        self.store.requested.borrow_mut().insert(key.to_owned());
    }

    /// Asset keys drawing needed and does not have (each once): fetch them and `addAsset`.
    #[wasm_bindgen(js_name = takeMissingAssets)]
    pub fn take_missing_assets(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.store.missing.borrow_mut())
    }

    #[wasm_bindgen(js_name = hasAsset)]
    pub fn has_asset(&self, key: &str) -> bool {
        self.store.bytes.contains_key(key)
    }

    /// Decoded size `[w, h]` of an asset whose bytes are in (natural size for SVG).
    #[wasm_bindgen(js_name = imageSize)]
    pub fn image_size(&mut self, key: &str) -> Option<Vec<f64>> {
        let Painter { renderer, store, .. } = self;
        renderer.image_size(key, store).map(|(w, h)| vec![w, h])
    }

    /// Forget every decoded image (bytes are kept).
    #[wasm_bindgen(js_name = clearImages)]
    pub fn clear_images(&mut self) {
        self.renderer.clear_images();
    }

    /// The current frame of a playback (`key`): unpremultiplied RGBA (`getImageData` of a video frame).
    #[wasm_bindgen(js_name = setVideoFrame)]
    pub fn set_video_frame(&mut self, key: &str, width: u32, height: u32, rgba: Vec<u8>) -> Result<(), JsError> {
        let mut data = rgba;
        if data.len() != (width as usize) * (height as usize) * 4 {
            return Err(err("setVideoFrame: data length is not width × height × 4"));
        }
        oc_render::assets::premultiply(&mut data);
        let size = tiny_skia::IntSize::from_wh(width, height).ok_or_else(|| err("setVideoFrame: empty frame"))?;
        let pix = Pixmap::from_vec(data, size).ok_or_else(|| err("setVideoFrame: bad frame"))?;
        if let Ok(mut f) = self.frames.lock() {
            f.insert(key.to_owned(), Arc::new(pix));
        }
        Ok(())
    }

    /// Drop a playback's frame (or all): the video shows its poster again.
    #[wasm_bindgen(js_name = clearVideoFrame)]
    pub fn clear_video_frame(&mut self, key: Option<String>) {
        if let Ok(mut f) = self.frames.lock() {
            match key {
                Some(k) => {
                    f.remove(&k);
                }
                None => f.clear(),
            }
        }
    }
}

fn transition_frame_arg(v: &JsValue) -> Result<oc_render::motion::TransitionFrame, JsError> {
    use oc_render::motion::{TransitionClip, TransitionFrame, TransitionLayer};
    let j = value(v)?;
    let layer = |l: &Value| -> Option<TransitionLayer> {
        if !l.is_object() {
            return None;
        }
        let n = |k: &str, d: f64| l.get(k).and_then(Value::as_f64).unwrap_or(d);
        let clip = l.get("clip").and_then(|c| match c.get("kind").and_then(Value::as_str) {
            Some("rect") => Some(TransitionClip::Rect { x: c["x"].as_f64().unwrap_or(0.0), y: c["y"].as_f64().unwrap_or(0.0), w: c["w"].as_f64().unwrap_or(1.0), h: c["h"].as_f64().unwrap_or(1.0) }),
            Some("circle") => Some(TransitionClip::Circle { r: c["r"].as_f64().unwrap_or(1.0) }),
            _ => None,
        });
        Some(TransitionLayer { alpha: n("alpha", 1.0), dx: n("dx", 0.0), dy: n("dy", 0.0), scale: n("scale", 1.0), rot: n("rot", 0.0), sx: n("sx", 1.0), blur: n("blur", 0.0), clip })
    };
    let next = layer(&j["next"]).ok_or_else(|| err("frame.next missing"))?;
    Ok(TransitionFrame { prev: layer(&j["prev"]), next, next_first: j.get("nextFirst").and_then(Value::as_bool) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erf_and_blurred_spans() {
        assert!((erf(0.5) - 0.520_499_877_8).abs() < 1e-6);
        assert!((erf(-1.0) + 0.842_700_792_9).abs() < 1e-6);
        // Far inside a wide span: full coverage; at its edge: half.
        assert!((blurred_span(50, 0.0, 100.0, 4.0) - 1.0).abs() < 1e-6);
        assert!((blurred_span(-1, -0.5, 100.0, 4.0) - 0.5).abs() < 1e-6);
        assert_eq!(blurred_span(3, 0.0, 10.0, 0.0), 1.0);
    }
}
