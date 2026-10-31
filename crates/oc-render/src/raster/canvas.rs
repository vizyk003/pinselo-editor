// SPDX-License-Identifier: Apache-2.0
//! A Skia-style canvas over tiny-skia: a save stack of transforms and anti-aliased clip masks, and offscreen
//! layers (`saveLayer`) composited on restore with opacity, a blend mode and image filters (blur, drop shadow).
//!
//! Layers are full-device pixmaps; each tracks the device rectangle drawn into it so restoring (filtering and
//! compositing) only touches that region.

use std::rc::Rc;

use tiny_skia::{BlendMode, Color, FillRule, IntRect, Mask, Paint, Path, Pixmap, PixmapPaint, Rect, Stroke, Transform};

use super::blur::{self, Edge};

/// An image filter of a layer, in device units (resolved from the layer's transform when it was pushed).
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// Gaussian blur, transparent outside (`ImageFilter.MakeBlur(σx, σy, Decal)`).
    Blur { sx: f32, sy: f32 },
    /// Gaussian blur that clamps at the layer content's bounds (`TileMode.Clamp`, used by image edits).
    BlurClamp { sx: f32, sy: f32, bounds: IntRect },
    /// `ImageFilter.MakeDropShadow`: the content's alpha, blurred, tinted and offset, under the content.
    DropShadow { dx: f32, dy: f32, sx: f32, sy: f32, color: Color },
}

impl Filter {
    /// How far the filter moves content outward, in pixels.
    fn reach(&self) -> i32 {
        match self {
            Filter::Blur { sx, sy } | Filter::BlurClamp { sx, sy, .. } => blur::extent(sx.max(*sy)),
            Filter::DropShadow { dx, dy, sx, sy, .. } => blur::extent(sx.max(*sy)) + dx.abs().ceil() as i32 + dy.abs().ceil() as i32,
        }
    }
}

/// How a layer is composited onto what is below it.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerPaint {
    pub opacity: f32,
    pub blend: BlendMode,
    pub filters: Vec<Filter>,
}

impl Default for LayerPaint {
    fn default() -> Self {
        LayerPaint { opacity: 1.0, blend: BlendMode::SourceOver, filters: Vec::new() }
    }
}

#[derive(Clone)]
struct State {
    m: Transform,
    clip: Option<Rc<Mask>>,
    clip_bounds: IntRect,
    layer: bool,
}

struct Layer {
    pixmap: Pixmap,
    dirty: Option<IntRect>,
    paint: LayerPaint,
    /// Clip in effect when the layer was pushed (applied when compositing it).
    clip: Option<Rc<Mask>>,
    clip_bounds: IntRect,
    role: Role,
}

/// What a layer is for (DS-107 masks).
#[derive(Clone)]
enum Role {
    Normal,
    /// Collects a mask's pixels; never composited.
    MaskSource { luminance: bool },
    /// Content composited through `mask` (its drawn area is `bounds`; nothing outside it shows).
    Masked { mask: Rc<Mask>, bounds: Option<IntRect> },
}

/// Mask coverage of one premultiplied pixel: its alpha, or its luminance (which premultiplication already scaled
/// by alpha).
fn mask_value(px: &[u8], luminance: bool) -> u8 {
    if luminance {
        let l = 0.2126 * f32::from(px[0]) + 0.7152 * f32::from(px[1]) + 0.0722 * f32::from(px[2]);
        l.round().clamp(0.0, 255.0) as u8
    } else {
        px[3]
    }
}

/// Union of two device rectangles.
fn union(a: Option<IntRect>, b: IntRect) -> IntRect {
    match a {
        None => b,
        Some(a) => {
            let l = a.left().min(b.left());
            let t = a.top().min(b.top());
            let r = a.right().max(b.right());
            let bo = a.bottom().max(b.bottom());
            IntRect::from_ltrb(l, t, r, bo).unwrap_or(b)
        }
    }
}

fn intersect(a: IntRect, b: IntRect) -> Option<IntRect> {
    let l = a.left().max(b.left());
    let t = a.top().max(b.top());
    let r = a.right().min(b.right());
    let bo = a.bottom().min(b.bottom());
    if r > l && bo > t { IntRect::from_ltrb(l, t, r, bo) } else { None }
}

fn outset(r: IntRect, d: i32) -> IntRect {
    IntRect::from_ltrb(r.left() - d, r.top() - d, r.right() + d, r.bottom() + d).unwrap_or(r)
}

/// Device bounds of a local rectangle under a transform (rounded outward, plus `pad` pixels).
pub fn device_bounds(r: Rect, m: &Transform, pad: f32) -> Option<IntRect> {
    let pts = [(r.left(), r.top()), (r.right(), r.top()), (r.right(), r.bottom()), (r.left(), r.bottom())];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for (x, y) in pts {
        let px = m.sx * x + m.kx * y + m.tx;
        let py = m.ky * x + m.sy * y + m.ty;
        x0 = x0.min(px);
        y0 = y0.min(py);
        x1 = x1.max(px);
        y1 = y1.max(py);
    }
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return None;
    }
    let clampf = |v: f32| v.clamp(-1.0e7, 1.0e7);
    IntRect::from_ltrb(clampf(x0 - pad).floor() as i32, clampf(y0 - pad).floor() as i32, clampf(x1 + pad).ceil() as i32 + 1, clampf(y1 + pad).ceil() as i32 + 1)
}

/// The canvas. Draws into its own device pixmap; take it with [`Canvas::into_pixmap`].
///
/// A canvas made with [`Canvas::new_region`] holds only a rectangle of a larger device: transforms, clips and
/// device rectangles are all in the larger device's coordinates, and only rasterisation is shifted into the
/// pixmap. Repainting part of a frame that way rounds every device coordinate exactly as the full frame did
/// (glyph origins land on the same quantised positions), so the patch matches the full repaint.
pub struct Canvas {
    width: u32,
    height: u32,
    /// Device position of the pixmap's top-left pixel.
    origin: (i32, i32),
    /// `layers[0]` is the device; the last one is the current target.
    layers: Vec<Layer>,
    stack: Vec<State>,
    cur: State,
}

impl Canvas {
    /// A transparent canvas. `None` for a zero or too large size.
    pub fn new(width: u32, height: u32) -> Option<Canvas> {
        let pixmap = Pixmap::new(width, height)?;
        let full = IntRect::from_xywh(0, 0, width, height)?;
        let base = Layer { pixmap, dirty: None, paint: LayerPaint::default(), clip: None, clip_bounds: full, role: Role::Normal };
        Some(Canvas { width, height, origin: (0, 0), layers: vec![base], stack: Vec::new(), cur: State { m: Transform::identity(), clip: None, clip_bounds: full, layer: false } })
    }

    /// A transparent canvas covering the device rectangle `(x, y, width, height)` of a larger device.
    pub fn new_region(x: i32, y: i32, width: u32, height: u32) -> Option<Canvas> {
        let mut c = Canvas::new(width, height)?;
        c.origin = (x, y);
        let full = IntRect::from_xywh(x, y, width, height)?;
        c.layers[0].clip_bounds = full;
        c.cur.clip_bounds = full;
        Some(c)
    }

    /// Device position of the pixmap's top-left pixel.
    pub fn origin(&self) -> (i32, i32) {
        self.origin
    }

    /// The device rectangle this canvas covers.
    pub fn device_rect(&self) -> IntRect {
        self.device()
    }

    /// A device transform as applied to the pixmap.
    fn px(&self, m: Transform) -> Transform {
        if self.origin == (0, 0) { m } else { m.post_translate(-self.origin.0 as f32, -self.origin.1 as f32) }
    }

    /// A device rectangle in pixmap coordinates.
    fn local(&self, r: IntRect) -> Option<IntRect> {
        IntRect::from_xywh(r.x() - self.origin.0, r.y() - self.origin.1, r.width(), r.height())
    }

    /// A canvas drawing over an existing pixmap.
    pub fn from_pixmap(pixmap: Pixmap) -> Canvas {
        let (width, height) = (pixmap.width(), pixmap.height());
        let full = IntRect::from_xywh(0, 0, width, height).expect("non-empty pixmap");
        let base = Layer { pixmap, dirty: None, paint: LayerPaint::default(), clip: None, clip_bounds: full, role: Role::Normal };
        Canvas { width, height, origin: (0, 0), layers: vec![base], stack: Vec::new(), cur: State { m: Transform::identity(), clip: None, clip_bounds: full, layer: false } }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Finish: pops every open save/layer and returns the device pixmap.
    pub fn into_pixmap(mut self) -> Pixmap {
        while !self.stack.is_empty() {
            self.restore();
        }
        self.layers.swap_remove(0).pixmap
    }

    pub fn transform(&self) -> Transform {
        self.cur.m
    }

    pub fn set_transform(&mut self, m: Transform) {
        self.cur.m = m;
    }

    /// Run `f` with the current transform replaced by `m` (clips made inside `f` stay in effect).
    pub fn with_transform<R>(&mut self, m: Transform, f: impl FnOnce(&mut Canvas) -> R) -> R {
        let saved = self.cur.m;
        self.cur.m = m;
        let r = f(self);
        self.cur.m = saved;
        r
    }

    pub fn concat(&mut self, m: Transform) {
        self.cur.m = self.cur.m.pre_concat(m);
    }

    pub fn translate(&mut self, x: f32, y: f32) {
        self.concat(Transform::from_translate(x, y));
    }

    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.concat(Transform::from_scale(sx, sy));
    }

    /// Rotate by `deg` degrees about the origin.
    pub fn rotate(&mut self, deg: f32) {
        self.concat(Transform::from_rotate(deg));
    }

    pub fn save(&mut self) {
        self.stack.push(self.cur.clone());
        self.cur.layer = false;
    }

    /// Push an offscreen layer composited with `paint` on the matching restore.
    pub fn save_layer(&mut self, paint: LayerPaint) {
        let pixmap = Pixmap::new(self.width, self.height).expect("canvas size is valid");
        self.layers.push(Layer { pixmap, dirty: None, paint, clip: self.cur.clip.clone(), clip_bounds: self.cur.clip_bounds, role: Role::Normal });
        self.stack.push(self.cur.clone());
        self.cur.layer = true;
    }

    pub fn restore(&mut self) {
        let Some(prev) = self.stack.pop() else { return };
        if self.cur.layer {
            let layer = self.layers.pop().expect("layer");
            if !matches!(layer.role, Role::MaskSource { .. }) {
                self.composite(layer);
            }
        }
        self.cur = prev;
    }

    /// Open a layer that collects a mask (DS-107). Draw the mask into it, then call
    /// [`Canvas::begin_masked_content`]; the `restore` after the content composites it through the mask.
    pub fn save_mask_layer(&mut self, luminance: bool) {
        self.save_layer(LayerPaint::default());
        if let Some(l) = self.layers.last_mut() {
            l.role = Role::MaskSource { luminance };
        }
    }

    /// Turn the mask layer opened by [`Canvas::save_mask_layer`] into a mask (alpha, or luminance × alpha) and
    /// open the content layer it gates. Without an open mask layer this opens a plain layer.
    pub fn begin_masked_content(&mut self) {
        let source = match self.layers.last() {
            Some(Layer { role: Role::MaskSource { luminance }, .. }) if self.cur.layer => Some(*luminance),
            _ => None,
        };
        let Some(luminance) = source else {
            self.save_layer(LayerPaint::default());
            return;
        };
        let src = self.layers.pop().expect("mask layer");
        if let Some(prev) = self.stack.pop() {
            self.cur = prev;
        }
        let mut mask = Mask::new(self.width, self.height).expect("canvas size is valid");
        let bounds = src.dirty.and_then(|d| self.local(d).map(|l| (d, l)));
        if let Some((_, l)) = bounds {
            let w = self.width as usize;
            let data = src.pixmap.data();
            let md = mask.data_mut();
            let (x0, x1) = (l.left().max(0), l.right().min(self.width as i32));
            let (y0, y1) = (l.top().max(0), l.bottom().min(self.height as i32));
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = y as usize * w + x as usize;
                    md[i] = mask_value(&data[i * 4..i * 4 + 4], luminance);
                }
            }
        }
        self.save_layer(LayerPaint::default());
        if let Some(l) = self.layers.last_mut() {
            l.role = Role::Masked { mask: Rc::new(mask), bounds: bounds.map(|(d, _)| d) };
        }
    }

    /// Nesting depth (saves and layers).
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Pop until the depth is `n`.
    pub fn restore_to(&mut self, n: usize) {
        while self.stack.len() > n {
            self.restore();
        }
    }

    fn device(&self) -> IntRect {
        IntRect::from_xywh(self.origin.0, self.origin.1, self.width, self.height).expect("size")
    }

    /// Narrow the clip to `path` (anti-aliased), or to everything outside it when `difference`.
    pub fn clip_path(&mut self, path: &Path, difference: bool) {
        let Some(mut mask) = Mask::new(self.width, self.height) else { return };
        mask.fill_path(path, FillRule::Winding, true, self.px(self.cur.m));
        let bounds = if difference {
            mask.invert();
            Some(self.cur.clip_bounds)
        } else {
            device_bounds(path.bounds(), &self.cur.m, 1.0).and_then(|b| intersect(b, self.cur.clip_bounds))
        };
        if let Some(old) = &self.cur.clip {
            for (a, b) in mask.data_mut().iter_mut().zip(old.data()) {
                *a = ((u16::from(*a) * u16::from(*b) + 127) / 255) as u8;
            }
        }
        self.cur.clip_bounds = bounds.unwrap_or_else(|| IntRect::from_xywh(self.origin.0, self.origin.1, 1, 1).expect("rect"));
        if bounds.is_none() {
            mask.clear();
        }
        self.cur.clip = Some(Rc::new(mask));
    }

    /// Clip to an empty region (nothing draws until restore).
    pub fn clip_empty(&mut self) {
        if let Some(mut mask) = Mask::new(self.width, self.height) {
            mask.clear();
            self.cur.clip = Some(Rc::new(mask));
        }
    }

    pub fn clip_rect(&mut self, r: Rect) {
        let p = tiny_skia::PathBuilder::from_rect(r);
        self.clip_path(&p, false);
    }

    fn target(&mut self) -> &mut Layer {
        self.layers.last_mut().expect("device layer")
    }

    fn mark(&mut self, b: Option<IntRect>) {
        let clip = self.cur.clip_bounds;
        let dev = self.device();
        if let Some(b) = b.and_then(|b| intersect(b, clip)).and_then(|b| intersect(b, dev)) {
            let t = self.target();
            t.dirty = Some(union(t.dirty, b));
        }
    }

    /// Whether anything drawn now could be visible (the clip is not empty).
    pub fn clip_is_empty(&self) -> bool {
        intersect(self.cur.clip_bounds, self.device()).is_none()
    }

    pub fn fill_path(&mut self, path: &Path, paint: &Paint) {
        if self.clip_is_empty() {
            return;
        }
        let m = self.cur.m;
        let b = device_bounds(path.bounds(), &m, 1.0);
        let clip = self.cur.clip.clone();
        let pm = self.px(m);
        self.target().pixmap.fill_path(path, paint, FillRule::Winding, pm, clip.as_deref());
        self.mark(b);
    }

    pub fn stroke_path(&mut self, path: &Path, paint: &Paint, stroke: &Stroke) {
        if self.clip_is_empty() {
            return;
        }
        let m = self.cur.m;
        let scale = (m.sx.hypot(m.ky)).max(m.kx.hypot(m.sy));
        let pad = (stroke.width.max(1.0) * scale) * stroke.miter_limit.max(1.0) + 2.0;
        let b = device_bounds(path.bounds(), &m, pad);
        let clip = self.cur.clip.clone();
        let pm = self.px(m);
        self.target().pixmap.stroke_path(path, paint, stroke, pm, clip.as_deref());
        self.mark(b);
    }

    pub fn fill_rect(&mut self, r: Rect, paint: &Paint) {
        let p = tiny_skia::PathBuilder::from_rect(r);
        self.fill_path(&p, paint);
    }

    /// Fill the whole clip with a paint (`drawPaint`).
    pub fn fill_all(&mut self, paint: &Paint) {
        let saved = self.cur.m;
        self.cur.m = Transform::identity();
        if let Some(r) = Rect::from_xywh(self.origin.0 as f32, self.origin.1 as f32, self.width as f32, self.height as f32) {
            self.fill_rect(r, paint);
        }
        self.cur.m = saved;
    }

    /// Composite a device-space premultiplied pixmap at `(x, y)` through the current clip.
    pub fn draw_device_pixmap(&mut self, x: i32, y: i32, src: &Pixmap, opacity: f32, blend: BlendMode) {
        if self.clip_is_empty() {
            return;
        }
        let clip = self.cur.clip.clone();
        let (ox, oy) = self.origin;
        if clip.is_none() && opacity >= 1.0 && matches!(blend, BlendMode::SourceOver | BlendMode::Source) {
            // Unclipped, opaque-paint blits (glyph masks, shadows, cached layers): integer copy / source-over on
            // the rows (tiny-skia's draw_pixmap runs a float pattern pipeline per pixel for these too).
            blit(&mut self.target().pixmap, x - ox, y - oy, src, blend == BlendMode::Source);
        } else {
            let paint = PixmapPaint { opacity, blend_mode: blend, quality: tiny_skia::FilterQuality::Nearest };
            self.target().pixmap.draw_pixmap(x - ox, y - oy, src.as_ref(), &paint, Transform::identity(), clip.as_deref());
        }
        self.mark(IntRect::from_xywh(x, y, src.width(), src.height()));
    }

    /// Read back a device rectangle of the current target (premultiplied).
    pub fn read_target(&self, r: IntRect) -> Option<Pixmap> {
        self.layers.last()?.pixmap.clone_rect(self.local(r)?)
    }

    /// The current clip mask (pixmap space), if any.
    pub fn clip_mask(&self) -> Option<&Mask> {
        self.cur.clip.as_deref()
    }

    pub fn clip_bounds(&self) -> IntRect {
        self.cur.clip_bounds
    }

    fn composite(&mut self, layer: Layer) {
        let Some(dirty) = layer.dirty else { return };
        let reach: i32 = layer.paint.filters.iter().map(Filter::reach).sum();
        let dev = self.device();
        let Some(mut region) = intersect(outset(dirty, reach), dev) else { return };
        // A masked layer shows only where its mask was drawn, through the mask (times the layer's clip).
        let gate: Option<Rc<Mask>> = match &layer.role {
            Role::Masked { mask, bounds } => {
                let Some(b) = bounds.and_then(|b| intersect(b, region)) else { return };
                region = b;
                let mut m = (**mask).clone();
                if let Some(c) = &layer.clip {
                    for (a, b) in m.data_mut().iter_mut().zip(c.data()) {
                        *a = ((u16::from(*a) * u16::from(*b) + 127) / 255) as u8;
                    }
                }
                Some(Rc::new(m))
            }
            _ => layer.clip.clone(),
        };
        let Some(local) = self.local(region) else { return };
        let Some(mut img) = layer.pixmap.clone_rect(local) else { return };
        for f in &layer.paint.filters {
            apply_filter(&mut img, region, f);
        }
        let paint = PixmapPaint { opacity: layer.paint.opacity.clamp(0.0, 1.0), blend_mode: layer.paint.blend, quality: tiny_skia::FilterQuality::Nearest };
        let target = self.layers.last_mut().expect("parent layer");
        target.pixmap.draw_pixmap(local.x(), local.y(), img.as_ref(), &paint, Transform::identity(), gate.as_deref());
        if let Some(b) = intersect(region, layer.clip_bounds) {
            target.dirty = Some(union(target.dirty, b));
        }
    }
}

/// Composite premultiplied `src` onto `dst` at `(x, y)` (pixmap coordinates, clipped to `dst`): a copy when
/// `copy`, else source-over (`d = s + d·(255 − sa)/255`, rounded).
fn blit(dst: &mut Pixmap, x: i32, y: i32, src: &Pixmap, copy: bool) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (src.width() as i32, src.height() as i32);
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + sw).min(dw), (y + sh).min(dh));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let n = ((x1 - x0) * 4) as usize;
    let s = src.data();
    let d = dst.data_mut();
    for yy in y0..y1 {
        let so = (((yy - y) * sw + (x0 - x)) * 4) as usize;
        let doff = ((yy * dw + x0) * 4) as usize;
        let (sr, dr) = (&s[so..so + n], &mut d[doff..doff + n]);
        if copy {
            dr.copy_from_slice(sr);
            continue;
        }
        for (dp, sp) in dr.as_chunks_mut::<4>().0.iter_mut().zip(sr.as_chunks::<4>().0) {
            let sa = u32::from(sp[3]);
            if sa == 0 {
                continue;
            }
            if sa == 255 {
                *dp = *sp;
                continue;
            }
            let inv = 255 - sa;
            for c in 0..4 {
                let v = u32::from(dp[c]) * inv + 127;
                dp[c] = (u32::from(sp[c]) + ((v + (v >> 8)) >> 8)).min(255) as u8;
            }
        }
    }
}

/// Apply one image filter to a premultiplied region whose top-left is `origin` in device space.
fn apply_filter(img: &mut Pixmap, origin: IntRect, f: &Filter) {
    let (w, h) = (img.width() as usize, img.height() as usize);
    match *f {
        Filter::Blur { sx, sy } => blur::blur_rgba8(img.data_mut(), w, h, sx, sy, Edge::Transparent),
        Filter::BlurClamp { sx, sy, bounds } => {
            // Skia's Clamp tiling: the content's edge pixels extend outward as far as the blur reaches, the region is
            // blurred with clamped edges, and the result covers that whole (hard-edged) region.
            let Some(local) = IntRect::from_xywh(bounds.x() - origin.x(), bounds.y() - origin.y(), bounds.width(), bounds.height()) else { return };
            let Some(r) = intersect(local, IntRect::from_xywh(0, 0, w as u32, h as u32).expect("size")) else { return };
            let src = img.data().to_vec();
            let data = img.data_mut();
            for y in 0..h as i32 {
                let cy = y.clamp(r.top(), r.bottom() - 1);
                for x in 0..w as i32 {
                    if x >= r.left() && x < r.right() && y >= r.top() && y < r.bottom() {
                        continue;
                    }
                    let cx = x.clamp(r.left(), r.right() - 1);
                    let si = (cy as usize * w + cx as usize) * 4;
                    let di = (y as usize * w + x as usize) * 4;
                    data[di..di + 4].copy_from_slice(&src[si..si + 4]);
                }
            }
            blur::blur_rgba8(img.data_mut(), w, h, sx, sy, Edge::Clamp);
        }
        Filter::DropShadow { dx, dy, sx, sy, color } => {
            let mut alpha: Vec<f32> = img.data().as_chunks::<4>().0.iter().map(|p| f32::from(p[3]) / 255.0).collect();
            blur::blur_alpha(&mut alpha, w, h, sx, sy, Edge::Transparent);
            let (ox, oy) = (dx.round() as isize, dy.round() as isize);
            let c = color.premultiply();
            let (cr, cg, cb, ca) = (c.red(), c.green(), c.blue(), c.alpha());
            let data = img.data_mut();
            for y in 0..h as isize {
                for x in 0..w as isize {
                    let (sxp, syp) = (x - ox, y - oy);
                    let a = if sxp >= 0 && syp >= 0 && (sxp as usize) < w && (syp as usize) < h { alpha[syp as usize * w + sxp as usize] } else { 0.0 };
                    if a <= 0.0 {
                        continue;
                    }
                    let i = (y as usize * w + x as usize) * 4;
                    let inv = 1.0 - f32::from(data[i + 3]) / 255.0;
                    if inv <= 0.0 {
                        continue;
                    }
                    let k = a * inv * 255.0;
                    for (ch, cv) in [cr, cg, cb, ca].into_iter().enumerate() {
                        let v = f32::from(data[i + ch]) + cv * k;
                        data[i + ch] = v.round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red() -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color_rgba8(255, 0, 0, 255);
        p
    }

    #[test]
    fn layer_opacity_and_clip() {
        let mut c = Canvas::new(20, 20).unwrap();
        c.save();
        c.clip_rect(Rect::from_xywh(0.0, 0.0, 10.0, 20.0).unwrap());
        c.save_layer(LayerPaint { opacity: 0.5, ..Default::default() });
        c.fill_rect(Rect::from_xywh(0.0, 0.0, 20.0, 20.0).unwrap(), &red());
        c.restore();
        c.restore();
        let px = c.into_pixmap();
        let at = |x: u32, y: u32| px.pixel(x, y).unwrap();
        assert_eq!((at(5, 5).red(), at(5, 5).alpha()), (128, 128));
        assert_eq!(at(15, 5).alpha(), 0);
    }

    #[test]
    fn drop_shadow_under_content() {
        let mut c = Canvas::new(40, 40).unwrap();
        let shadow = Filter::DropShadow { dx: 10.0, dy: 0.0, sx: 0.0, sy: 0.0, color: Color::from_rgba8(0, 0, 255, 255) };
        c.save_layer(LayerPaint { filters: vec![shadow], ..Default::default() });
        c.fill_rect(Rect::from_xywh(5.0, 5.0, 10.0, 10.0).unwrap(), &red());
        c.restore();
        let px = c.into_pixmap();
        assert_eq!(px.pixel(10, 10).unwrap().red(), 255);
        assert_eq!(px.pixel(22, 10).unwrap().blue(), 255);
    }
}
