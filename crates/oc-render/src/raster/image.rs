// SPDX-License-Identifier: Apache-2.0
//! Images and video frames (`drawImage` / the `video` op of painter.ts): crop to the destination box, colour matrix
//! (filters, adjustments, duotone), tone/detail (highlights, shadows, sharpen, clarity), blur and vignette.

use std::sync::Arc;

use oc_schema::elements::{ImageAdjust, ImageEditStack};
use oc_schema::primitives::{ImageFit, Rect as SRect};
use tiny_skia::{FilterQuality, GradientStop, Paint, Pattern, Point, RadialGradient, SpreadMode, Transform};

use super::canvas::{Canvas, Filter, LayerPaint, device_bounds};
use super::{Renderer, axis_scales, rect, rgba};
use crate::assets::{AssetProvider, Decoded, RasterImage, premultiply, unpremultiply};
use crate::color_matrix::{Cm, edit_matrix, needs_tone_shader};
use crate::path;

/// One image (or video frame) to draw.
pub(crate) struct ImageOp<'a> {
    pub asset: &'a str,
    pub dst: &'a SRect,
    pub crop: SRect,
    pub edits: Option<&'a ImageEditStack>,
    pub clip: Option<&'a str>,
    /// `Some` for a video op: the decoded frame, if the host supplied one (else the poster in `asset`).
    pub video: Option<Option<Arc<tiny_skia::Pixmap>>>,
}

/// Scale of an image of natural size `iw × ih` in a box for a fit mode (`fit` = contain, `fill` = cover).
pub fn fit_scale(fit: &ImageFit, b: &SRect, iw: f64, ih: f64) -> f64 {
    let sx = b.w / iw;
    let sy = b.h / ih;
    match fit {
        ImageFit::Fit => sx.min(sy),
        ImageFit::Fill => sx.max(sy),
        _ => 1.0,
    }
}

impl Renderer {
    pub(crate) fn draw_image(&mut self, canvas: &mut Canvas, op: ImageOp<'_>, assets: &dyn AssetProvider) {
        let depth = canvas.depth();
        canvas.save();
        if let Some(c) = op.clip
            && let Some(p) = path::parse(c).and_then(|p| p.to_skia())
        {
            canvas.clip_path(&p, false);
        }
        self.draw_image_inner(canvas, &op, assets);
        canvas.restore_to(depth);
    }

    fn draw_image_inner(&mut self, canvas: &mut Canvas, op: &ImageOp<'_>, assets: &dyn AssetProvider) {
        let is_video = op.video.is_some();
        let m = canvas.transform();
        let (dsx, dsy) = axis_scales(&m);
        let d = op.dst;
        let c = &op.crop;
        let Some(dst) = rect(d.x, d.y, d.w, d.h) else { return };
        let img: Option<Arc<RasterImage>> = match &op.video {
            Some(Some(frame)) => Some(Arc::new(RasterImage::shared(frame.clone()))),
            _ => match self.images.get(op.asset, assets) {
                Some(Decoded::Raster(r)) => Some(r),
                Some(Decoded::Svg(s)) if c.w > 0.0 && c.h > 0.0 => {
                    let wpx = (d.w / c.w * f64::from(dsx)).ceil();
                    let hpx = (d.h / c.h * f64::from(dsy)).ceil();
                    s.raster(wpx.clamp(1.0, 4096.0) as u32, hpx.clamp(1.0, 4096.0) as u32)
                }
                _ => None,
            },
        };
        let Some(img) = img else {
            let mut p = Paint { anti_alias: false, ..Default::default() };
            p.set_color(if is_video { rgba(0.1, 0.1, 0.12, 1.0) } else { rgba(0.88, 0.9, 0.93, 1.0) });
            canvas.fill_rect(dst, &p);
            return;
        };
        if !(c.w > 0.0 && c.h > 0.0) {
            return;
        }
        let (iw, ih) = (f64::from(img.width()), f64::from(img.height()));
        let sxi = d.w / (c.w * iw);
        let syi = d.h / (c.h * ih);
        let adjust = op.edits.and_then(|e| e.adjust.as_ref());
        let cm = edit_matrix(op.edits);
        let tone = !is_video && needs_tone_shader(adjust);
        let img = if cm.is_some() || tone {
            let key = format!("{}|{}x{}|{:.3}x{:.3}|{}", op.asset, iw, ih, sxi.max(1.0), syi.max(1.0), oc_schema::js::to_string(&op.edits).unwrap_or_default());
            match self.edited.get(&key) {
                Some(e) if !is_video => e.clone(),
                _ => {
                    let e = Arc::new(apply_edits(&img, cm.as_ref(), if tone { adjust } else { None }, sxi, syi));
                    if !is_video {
                        if self.edited.len() > 64 {
                            self.edited.clear();
                        }
                        self.edited.insert(key, e.clone());
                    }
                    e
                }
            }
        } else {
            img
        };
        // Image px → destination: the crop rectangle maps onto `dst`.
        let t = Transform::from_row(sxi as f32, 0.0, 0.0, syi as f32, (d.x - c.x * iw * sxi) as f32, (d.y - c.y * ih * syi) as f32);
        // Mipmaps (MipmapMode.Linear for images; videos use none): trilinear between the two levels around Skia's
        // `SkMipmap::ComputeLevel` — log2 of the more minified axis' minification, with its −0.5 "sharpen" bias.
        let s = (sxi * f64::from(dsx)).min(syi * f64::from(dsy));
        let level = if !is_video && s < 1.0 && s > 0.0 { ((1.0 / s).log2() - 0.5).clamp(0.0, 16.0) as f32 } else { 0.0 };
        let (pix, ls) = img.trilinear(level);
        let t = t.pre_scale(1.0 / ls, 1.0 / ls);
        let blur = adjust.and_then(|a| a.blur).filter(|b| *b > 0.0);
        if let Some(b) = blur {
            let sigma = (b / 8.0) as f32;
            // The content's pixel bounds (rounded, so the anti-aliased edge pixels are not what gets clamped).
            let bounds = device_bounds(dst, &m, 0.0)
                .and_then(|_| {
                    let pts = [(dst.left(), dst.top()), (dst.right(), dst.top()), (dst.right(), dst.bottom()), (dst.left(), dst.bottom())].map(|(x, y)| (m.sx * x + m.kx * y + m.tx, m.ky * x + m.sy * y + m.ty));
                    let x0 = pts.iter().map(|p| p.0).fold(f32::INFINITY, f32::min).round() as i32;
                    let y0 = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min).round() as i32;
                    let x1 = pts.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max).round() as i32;
                    let y1 = pts.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max).round() as i32;
                    tiny_skia::IntRect::from_ltrb(x0, y0, x1.max(x0 + 1), y1.max(y0 + 1))
                })
                .unwrap_or_else(|| canvas.clip_bounds());
            canvas.save_layer(LayerPaint { filters: vec![Filter::BlurClamp { sx: sigma * dsx, sy: sigma * dsy, bounds }], ..Default::default() });
        }
        let paint = Paint { shader: Pattern::new(pix.as_ref().as_ref(), SpreadMode::Pad, FilterQuality::Bilinear, 1.0, t), anti_alias: true, ..Default::default() };
        canvas.fill_rect(dst, &paint);
        if blur.is_some() {
            canvas.restore();
        }
        if let Some(v) = adjust.and_then(|a| a.vignette).filter(|v| *v > 0.0) {
            vignette(canvas, d, v / 100.0);
        }
    }
}

fn vignette(canvas: &mut Canvas, r: &SRect, amount: f64) {
    let c = Point::from_xy((r.x + r.w / 2.0) as f32, (r.y + r.h / 2.0) as f32);
    let radius = (r.w.hypot(r.h) / 2.0) as f32;
    let stops = vec![
        GradientStop::new(0.0, rgba(0.0, 0.0, 0.0, 0.0)),
        GradientStop::new(0.55, rgba(0.0, 0.0, 0.0, 0.0)),
        GradientStop::new(1.0, rgba(0.0, 0.0, 0.0, (0.85 * amount).clamp(0.0, 1.0) as f32)),
    ];
    let Some(shader) = RadialGradient::new(c, 0.0, c, radius, stops, SpreadMode::Pad, Transform::identity()) else { return };
    let p = Paint { shader, anti_alias: false, ..Default::default() };
    if let Some(rr) = rect(r.x, r.y, r.w, r.h) {
        canvas.fill_rect(rr, &p);
    }
}

/// Bilinear sample of a premultiplied float image (pixel centres at `i + 0.5`, clamped at the edges).
fn sample(px: &[f32], w: usize, h: usize, x: f32, y: f32) -> [f32; 4] {
    let fx = (x - 0.5).clamp(0.0, (w - 1) as f32);
    let fy = (y - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let at = |x: usize, y: usize, c: usize| px[(y * w + x) * 4 + c];
    let mut out = [0.0; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let a = at(x0, y0, c) * (1.0 - tx) + at(x1, y0, c) * tx;
        let b = at(x0, y1, c) * (1.0 - tx) + at(x1, y1, c) * tx;
        *o = a * (1.0 - ty) + b * ty;
    }
    out
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The edited image: tone/detail first (the SkSL shader of colorMatrix.ts, evaluated per image pixel with its
/// sample offsets mapped from destination units), then the colour matrix (applied to unpremultiplied colour).
fn apply_edits(img: &RasterImage, cm: Option<&Cm>, tone: Option<&ImageAdjust>, sxi: f64, syi: f64) -> RasterImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let src: Vec<f32> = img.pixmap.data().iter().map(|v| f32::from(*v) / 255.0).collect();
    let mut out = src.clone();
    if let Some(a) = tone.filter(|a| crate::color_matrix::has_tone(Some(a))) {
        let hi = (a.highlights.unwrap_or(0.0) / 100.0) as f32;
        let sh = (a.shadows.unwrap_or(0.0) / 100.0) as f32;
        let sharp = ((a.sharpen.unwrap_or(0.0) / 100.0) * 0.9) as f32;
        let clar = ((a.clarity.unwrap_or(0.0) / 100.0) * 0.45) as f32;
        // `texel` = max(scale, 1) destination units = that many / scale image pixels.
        let tx = (sxi.max(1.0) / sxi) as f32;
        let ty = (syi.max(1.0) / syi) as f32;
        let blur4 = |x: f32, y: f32, dx: f32, dy: f32| -> [f32; 4] {
            let a = sample(&src, w, h, x + dx, y);
            let b = sample(&src, w, h, x - dx, y);
            let c = sample(&src, w, h, x, y + dy);
            let d = sample(&src, w, h, x, y - dy);
            [(a[0] + b[0] + c[0] + d[0]) * 0.25, (a[1] + b[1] + c[1] + d[1]) * 0.25, (a[2] + b[2] + c[2] + d[2]) * 0.25, (a[3] + b[3] + c[3] + d[3]) * 0.25]
        };
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let i = (y * w + x) * 4;
                let mut c = [src[i], src[i + 1], src[i + 2], src[i + 3]];
                if sharp != 0.0 {
                    let b = blur4(px, py, tx, ty);
                    for k in 0..3 {
                        c[k] += (c[k] - b[k]) * sharp;
                    }
                }
                if clar != 0.0 {
                    let b6 = blur4(px, py, tx * 6.0, ty * 6.0);
                    let b12 = blur4(px, py, tx * 12.0, ty * 12.0);
                    let b3 = blur4(px, py, tx * 3.0, ty * 3.0);
                    for k in 0..3 {
                        let b = (b6[k] + b12[k] + b3[k]) / 3.0;
                        c[k] += (c[k] - b) * clar;
                    }
                }
                let al = c[3].max(0.0001);
                let l = (c[0] / al) * 0.2126 + (c[1] / al) * 0.7152 + (c[2] / al) * 0.0722;
                let sw = 1.0 - smoothstep(0.0, 0.55, l);
                let hw = smoothstep(0.45, 1.0, l);
                for v in c.iter_mut().take(3) {
                    *v += al * (sh * 0.3 * sw + hi * 0.3 * hw);
                    *v = v.clamp(0.0, al);
                }
                out[i..i + 4].copy_from_slice(&c);
            }
        }
    }
    let px = crate::color_matrix::pixelate_amount(tone);
    if px > 0.0 {
        pixelate(&mut out, w, h, crate::color_matrix::pixelate_cell(px, w.max(h) as f64));
    }
    let mut bytes: Vec<u8> = out.iter().map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8).collect();
    if let Some(m) = cm {
        unpremultiply(&mut bytes);
        for p in bytes.as_chunks_mut::<4>().0 {
            let v = [f64::from(p[0]) / 255.0, f64::from(p[1]) / 255.0, f64::from(p[2]) / 255.0, f64::from(p[3]) / 255.0];
            for r in 0..4 {
                let o = m[r * 5] * v[0] + m[r * 5 + 1] * v[1] + m[r * 5 + 2] * v[2] + m[r * 5 + 3] * v[3] + m[r * 5 + 4];
                p[r] = (o.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        premultiply(&mut bytes);
    }
    let size = tiny_skia::IntSize::from_wh(w as u32, h as u32).expect("non-empty image");
    RasterImage::new(tiny_skia::Pixmap::from_vec(bytes, size).expect("valid size"))
}

/// Pixelate a premultiplied float image in place: every `cell`×`cell` block becomes its average colour.
fn pixelate(px: &mut [f32], w: usize, h: usize, cell: usize) {
    if cell < 2 || w == 0 || h == 0 {
        return;
    }
    for by in (0..h).step_by(cell) {
        for bx in (0..w).step_by(cell) {
            let (x1, y1) = ((bx + cell).min(w), (by + cell).min(h));
            let mut sum = [0.0f32; 4];
            for y in by..y1 {
                for x in bx..x1 {
                    let i = (y * w + x) * 4;
                    for k in 0..4 {
                        sum[k] += px[i + k];
                    }
                }
            }
            let n = ((x1 - bx) * (y1 - by)) as f32;
            let avg = sum.map(|v| v / n);
            for y in by..y1 {
                for x in bx..x1 {
                    let i = (y * w + x) * 4;
                    px[i..i + 4].copy_from_slice(&avg);
                }
            }
        }
    }
}

#[cfg(test)]
mod pixelate_tests {
    #[test]
    fn blocks_take_their_average() {
        // 4×1 image, cells of 2: [0,1] and [2,3] average pairwise.
        let mut px = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.2, 0.2, 0.2, 1.0, 0.4, 0.4, 0.4, 1.0];
        super::pixelate(&mut px, 4, 1, 2);
        assert_eq!(&px[0..4], &[0.5, 0.5, 0.5, 1.0]);
        assert_eq!(&px[4..8], &[0.5, 0.5, 0.5, 1.0]);
        assert!((px[8] - 0.3).abs() < 1e-6 && (px[12] - 0.3).abs() < 1e-6);
    }
}
