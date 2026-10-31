// SPDX-License-Identifier: Apache-2.0
//! Text (`TextEngine.draw` / `drawCurved` of @oc/text as CanvasKit/SkParagraph painted it): line by line, the
//! highlight backgrounds, the effect shadows (shadow, neon, echo, glitch), the glyphs (filled, or stroked for
//! hollow/outline) and the underline/strike decorations; the `background` effect's rounded rects first and the
//! outline effect's stroked copy under each paragraph.

use std::sync::Arc;

use oc_schema::elements::CellVAlign;
use oc_schema::primitives::{ColorValue, Rect as SRect};
use oc_schema::text::{RunStyle, TextBody, TextEffect};
use oc_text::{CurvedLayout, GlyphRun, LaidParagraph, TextLayout, paint_to_color};
use skrifa::FontRef;
use tiny_skia::{BlendMode, Color, FillRule, IntRect, LineJoin, Mask, Pixmap, Stroke, Transform};

use super::canvas::{Canvas, device_bounds};
use super::{Renderer, blur, color, rect, solid};
use crate::path::PathData;

/// How a paragraph's glyphs are painted.
#[derive(Clone, Copy)]
enum Mode {
    /// Fill with the run colour, with the effect's shadows.
    Fill,
    /// Stroke with the run colour (hollow): width in % of the font size.
    Hollow(f64),
    /// The outline effect's under-layer: stroked with `color`, no shadows, no decorations.
    Outline(f64, Color),
    /// List markers: plain fill.
    Plain,
}

/// One text shadow (`TextShadow`): colour, offset in text space, blur sigma in text space.
struct Shadow {
    color: Color,
    dx: f64,
    dy: f64,
    sigma: f64,
}

/// SkParagraph's `TextShadow::hasShadow`.
fn has_shadow(s: &Shadow) -> bool {
    s.dx != 0.0 || s.dy != 0.0 || s.sigma != 0.0
}

/// Quantise like CanvasKit does when it turns a `Color4f` into an `SkColor`.
fn sk(c: Color) -> Color {
    let q = |v: f32| (v * 255.0).round() / 255.0;
    Color::from_rgba(q(c.red()), q(c.green()), q(c.blue()), q(c.alpha())).unwrap_or(c)
}

/// The run's text colour (`paintToColor(style.fill)`: gradients use their first stop).
fn run_color(s: &RunStyle) -> ColorValue {
    paint_to_color(Some(&s.fill))
}

/// The shadows SkParagraph draws for a run under an effect (`textStyle` in packages/text/src/layout.ts).
fn shadows(effect: Option<&TextEffect>, s: &RunStyle) -> Vec<Shadow> {
    let fill = color(&run_color(s));
    match effect {
        Some(TextEffect::Shadow(e)) => vec![Shadow { color: color(&e.color), dx: e.dx, dy: e.dy, sigma: e.blur }],
        Some(TextEffect::Neon(e)) => (1..=3).map(|i| Shadow { color: fill, dx: 0.0, dy: 0.0, sigma: (e.intensity / 100.0) * s.size * 0.15 * f64::from(i) }).collect(),
        Some(TextEffect::Echo(e)) => [3.0, 2.0, 1.0]
            .into_iter()
            .map(|i: f64| {
                let mut c = fill;
                c.set_alpha((f64::from(fill.alpha()) * (0.15 + 0.2 * (3.0 - i))).clamp(0.0, 1.0) as f32);
                Shadow { color: c, dx: e.dx * i, dy: e.dy * i, sigma: 0.0 }
            })
            .collect(),
        Some(TextEffect::Glitch(e)) => {
            let o = (e.offset / 100.0) * s.size;
            vec![
                Shadow { color: Color::from_rgba(0.0, 220.0 / 255.0, 1.0, 0.85).expect("colour"), dx: -o, dy: 0.0, sigma: 0.0 },
                Shadow { color: Color::from_rgba(1.0, 0.0, 170.0 / 255.0, 0.85).expect("colour"), dx: o, dy: 0.0, sigma: 0.0 },
            ]
        }
        _ => Vec::new(),
    }
}

/// A glyph outline with its absolute (device) transform.
struct Placed {
    path: Arc<tiny_skia::Path>,
    t: Transform,
}

impl Renderer {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_text(&mut self, canvas: &mut Canvas, key: &str, body: &TextBody, b: &SRect, effect: Option<&TextEffect>, valign: Option<&CellVAlign>, curve: Option<f64>, fit: bool) {
        if let Some(cv) = curve
            && let Some(cl) = self.text.curved(key, body, cv)
        {
            self.draw_curved(canvas, &cl, b);
            return;
        }
        let layout = if fit { self.text.layout_fit(key, body, b.w, b.h, effect).0 } else { self.text.layout(key, body, b.w, effect) };
        let dy = match valign {
            Some(CellVAlign::Middle) => (b.h - layout.height) / 2.0,
            Some(CellVAlign::Bottom) => b.h - layout.height,
            _ => 0.0,
        };
        canvas.save();
        canvas.translate(b.x as f32, (b.y + dy) as f32);
        self.draw_layout(canvas, &layout, body);
        canvas.restore();
    }

    /// `TextEngine.draw` for a laid-out body at the canvas origin (`body`: the body it was laid out from, for the
    /// paragraphs' line heights).
    pub fn draw_layout(&mut self, canvas: &mut Canvas, l: &TextLayout, body: &TextBody) {
        let effect = l.effect.as_ref();
        if let Some(TextEffect::Background(bg)) = effect {
            let paint = solid(sk(color(&bg.color)));
            let pad = bg.padding;
            for p in &l.paras {
                for m in &p.lines {
                    let left = p.x + m.left - pad;
                    let top = p.y + m.baseline - m.ascent - pad / 2.0;
                    if let Some(path) = PathData::rrect(left, top, m.width + pad * 2.0, m.ascent + m.descent + pad, bg.radius, bg.radius).to_skia() {
                        canvas.fill_path(&path, &paint);
                    }
                }
            }
        }
        for (i, p) in l.paras.iter().enumerate() {
            let lh = body.paragraphs.get(i).map_or(1.3, |p| p.line_height);
            if let Some(TextEffect::Outline(o)) = effect {
                self.draw_paragraph(canvas, p, lh, Mode::Outline(o.width, sk(color(&o.color))), effect);
            }
            let mode = match effect {
                Some(TextEffect::Hollow(h)) => Mode::Hollow(h.width),
                _ => Mode::Fill,
            };
            self.draw_paragraph(canvas, p, lh, mode, effect);
            if let Some(bullet) = &p.bullet {
                self.draw_paragraph(canvas, bullet, lh, Mode::Plain, None);
            }
        }
    }

    /// Glyph outlines of a run with their absolute (device) transforms (`dx`, `dy`: an extra offset in text space,
    /// for shadows). With `hint`, under an axis-aligned uniform scale and up to Skia's glyph-cache size limit,
    /// glyphs are hinted at their pixel size and their origins quantised like Skia's glyph masks (x to ¼ px,
    /// y to whole pixels); otherwise they are exact unhinted paths (Skia's path fallback).
    #[allow(clippy::too_many_arguments)]
    fn place_run(&mut self, base: Transform, run: &GlyphRun, ox: f64, oy: f64, dx: f64, dy: f64, hint: bool) -> Vec<Placed> {
        let Some(face) = self.text.fonts.face_by_file(&run.face) else { return Vec::new() };
        let data = face.data();
        let Some(caps) = self.glyphs.caps(&run.face, data) else { return Vec::new() };
        let aligned = base.kx.abs() < 1e-6 && base.ky.abs() < 1e-6 && base.sx > 0.0 && (base.sx - base.sy).abs() <= 1e-6 * base.sx;
        let ppem = run.size as f32 * base.sx;
        let mut out = Vec::with_capacity(run.glyphs.len());
        if hint && aligned && ppem > 0.0 && ppem <= crate::glyphs::MAX_HINTED_PPEM {
            let skew = if run.synthetic_italic { -0.25 } else { 0.0 };
            for g in &run.glyphs {
                let Some(o) = self.glyphs.hinted_outline(&run.face, data, g.id, ppem) else { continue };
                let Some(path) = o.skia else { continue };
                let x = base.sx * (ox + g.x + dx) as f32 + base.tx;
                let y = base.sy * (oy + g.y + dy) as f32 + base.ty;
                let (qx, qy) = ((x * 4.0 + 0.5).floor() / 4.0, (y + 0.5).floor());
                out.push(Placed { path, t: Transform::from_row(1.0, 0.0, skew, 1.0, qx, qy) });
            }
            return out;
        }
        let k = (run.size / f64::from(caps.upem)) as f32;
        let skew = if run.synthetic_italic { -0.25 * k } else { 0.0 };
        for g in &run.glyphs {
            let Some(o) = self.glyphs.outline(&run.face, data, g.id) else { continue };
            let Some(path) = o.skia else { continue };
            let t = Transform::from_row(k, 0.0, skew, k, (ox + g.x + dx) as f32, (oy + g.y + dy) as f32);
            out.push(Placed { path, t: base.pre_concat(t) });
        }
        out
    }

    fn draw_paragraph(&mut self, canvas: &mut Canvas, p: &LaidParagraph, line_height: f64, mode: Mode, effect: Option<&TextEffect>) {
        let (ox, oy) = (p.x, p.y);
        for line in &p.lines {
            // Backgrounds (highlight): the run's own half-leading line box (lineHeight × size around its baseline).
            for run in &line.glyph_runs {
                let Some(st) = p.styles.get(run.style) else { continue };
                let Some(h) = &st.highlight else { continue };
                let Some(face) = self.text.fonts.face_by_file(&run.face) else { continue };
                let fm = face.info.metrics;
                let hh = line_height * run.size;
                let asc = (hh + fm.ascent * run.size - fm.descent * run.size) / 2.0;
                let baseline = run.glyphs.first().map_or(line.top + line.baseline, |g| g.y);
                if let Some(r) = rect(ox + run.x, oy + baseline - asc, run.advance, hh) {
                    let mut paint = solid(sk(color(h)));
                    paint.anti_alias = false;
                    canvas.fill_rect(r, &paint);
                }
            }
            // Shadows.
            if matches!(mode, Mode::Fill | Mode::Hollow(_)) {
                for run in &line.glyph_runs {
                    let Some(st) = p.styles.get(run.style).cloned() else { continue };
                    for sh in shadows(effect, &st).iter().filter(|s| has_shadow(s)) {
                        let placed = self.place_run(canvas.transform(), run, ox, oy, sh.dx, sh.dy, true);
                        let masks = is_mask_path(&canvas.transform(), run.size);
                        draw_shadow(canvas, &placed, sk(sh.color), sh.sigma, masks);
                    }
                }
            }
            // Glyphs.
            for run in &line.glyph_runs {
                let Some(st) = p.styles.get(run.style).cloned() else { continue };
                let fill = sk(color(&run_color(&st)));
                match mode {
                    Mode::Fill | Mode::Plain => self.fill_run(canvas, run, ox, oy, fill),
                    Mode::Hollow(w) => self.stroke_run(canvas, run, ox, oy, fill, (w * st.size / 100.0).max(0.5)),
                    Mode::Outline(w, c) => self.stroke_run(canvas, run, ox, oy, c, (w * st.size / 100.0).max(0.5)),
                }
            }
            // Decorations (in the text colour; the outline under-layer's text colour is transparent).
            if matches!(mode, Mode::Outline(..)) {
                continue;
            }
            for run in &line.glyph_runs {
                let Some(st) = p.styles.get(run.style).cloned() else { continue };
                if st.underline != Some(true) && st.strike != Some(true) {
                    continue;
                }
                let Some(face) = self.text.fonts.face_by_file(&run.face) else { continue };
                let fm = face.info.metrics;
                let baseline = oy + run.glyphs.first().map_or(line.top + line.baseline, |g| g.y);
                let paint = solid(sk(color(&run_color(&st))));
                let mut bar = |offset: f64, thickness: f64| {
                    let th = if thickness > 0.0 { thickness * run.size } else { run.size / 14.0 };
                    let center = baseline - offset * run.size + th / 2.0;
                    if let Some(r) = rect(ox + run.x, center - th / 2.0, run.advance, th) {
                        canvas.fill_rect(r, &paint);
                    }
                };
                if st.underline == Some(true) {
                    bar(fm.underline_offset, fm.underline_thickness);
                }
                if st.strike == Some(true) {
                    bar(fm.strikeout_offset, fm.strikeout_thickness);
                }
            }
        }
    }

    fn fill_run(&mut self, canvas: &mut Canvas, run: &GlyphRun, ox: f64, oy: f64, c: Color) {
        let Some(face) = self.text.fonts.face_by_file(&run.face) else { return };
        let data = face.data();
        let Some(caps) = self.glyphs.caps(&run.face, data) else { return };
        if caps.colr || caps.bitmaps {
            let font = FontRef::from_index(data, 0).ok();
            let k = (run.size / f64::from(caps.upem)) as f32;
            for g in &run.glyphs {
                let at = Transform::from_translate((ox + g.x) as f32, (oy + g.y) as f32);
                if let Some(font) = &font
                    && super::color_glyph::draw_color_glyph(canvas, font, &run.face, data, &mut self.glyphs, g.id, at, k, run.size as f32, c)
                {
                    continue;
                }
                if let Some(path) = self.glyphs.outline(&run.face, data, g.id).and_then(|o| o.skia) {
                    let t = canvas.transform().pre_concat(at).pre_scale(k, k);
                    canvas.with_transform(t, |cv| cv.fill_path(&path, &solid(c)));
                }
            }
            return;
        }
        let base = canvas.transform();
        let placed = self.place_run(base, run, ox, oy, 0.0, 0.0, true);
        if is_mask_path(&base, run.size) {
            draw_glyph_masks(canvas, &placed, c);
        } else {
            let paint = solid(c);
            for pl in placed {
                canvas.with_transform(pl.t, |cv| cv.fill_path(&pl.path, &paint));
            }
        }
    }

    fn stroke_run(&mut self, canvas: &mut Canvas, run: &GlyphRun, ox: f64, oy: f64, c: Color, width: f64) {
        let paint = solid(c);
        let base = canvas.transform();
        let s = base.sx.hypot(base.ky).max(1e-6);
        for pl in self.place_run(base, run, ox, oy, 0.0, 0.0, false) {
            // The stroke width is in text px; glyph paths are in font units scaled (to the device) by `t`.
            let k = pl.t.sx.hypot(pl.t.ky).max(1e-6);
            let st = Stroke { width: (width as f32) * s / k, line_join: LineJoin::Round, ..Default::default() };
            canvas.with_transform(pl.t, |cv| cv.stroke_path(&pl.path, &paint, &st));
        }
    }

    /// `drawCurved`: the arc layout scaled uniformly into the box (centred), each run in its colour.
    fn draw_curved(&mut self, canvas: &mut Canvas, l: &CurvedLayout, b: &SRect) {
        let s = (b.w / l.width).min(b.h / l.height);
        canvas.save();
        canvas.translate((b.x + (b.w - l.width * s) / 2.0) as f32, (b.y + (b.h - l.height * s) / 2.0) as f32);
        canvas.scale(s as f32, s as f32);
        let base = canvas.transform();
        for r in &l.runs {
            let paint = solid(sk(color(&r.color)));
            let Some(face) = self.text.fonts.face_by_file(&r.face) else { continue };
            let data = face.data();
            let Some(caps) = self.glyphs.caps(&r.face, data) else { continue };
            for g in &l.glyphs[r.glyphs.clone()] {
                let Some(path) = self.glyphs.outline(&r.face, data, g.id).and_then(|o| o.skia) else { continue };
                let k = (g.size / f64::from(caps.upem)) as f32;
                let rsx = Transform::from_row(g.cos as f32, g.sin as f32, -g.sin as f32, g.cos as f32, g.tx as f32, g.ty as f32);
                canvas.with_transform(base.pre_concat(rsx).pre_scale(k, k), |cv| cv.fill_path(&path, &paint));
            }
        }
        canvas.restore();
    }
}

/// Whether Skia draws a run through its glyph cache (axis-aligned, uniformly scaled, at most 256 px): hinted
/// A8 masks with its gamma/contrast correction, instead of plain paths.
fn is_mask_path(base: &Transform, size: f64) -> bool {
    let aligned = base.kx.abs() < 1e-6 && base.ky.abs() < 1e-6 && base.sx > 0.0 && (base.sx - base.sy).abs() <= 1e-6 * base.sx;
    let ppem = size as f32 * base.sx;
    aligned && ppem > 0.0 && ppem <= crate::glyphs::MAX_HINTED_PPEM
}

fn srgb_to_lin(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn lin_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Skia's A8 glyph mask correction (`SkTMaskGamma_build_correcting_lut` with the default contrast 0.2 and sRGB
/// paint/device gamma) for a text colour: coverage → corrected coverage. Light text on dark is drawn heavier,
/// dark text on light a touch lighter, as CanvasKit draws them.
fn mask_gamma_lut(c: Color) -> [u8; 256] {
    // Luminance of the colour (`SkColorSpaceLuminance::computeLuminance`), quantised to 3 bits like the
    // scaler context's luminance colour, expanded back to 8 bits (`sk_t_scale255<3>`).
    let q = |v: f32| (v * 255.0).round() / 255.0;
    let luma = srgb_to_lin(q(c.red())) * 0.2126 + srgb_to_lin(q(c.green())) * 0.7152 + srgb_to_lin(q(c.blue())) * 0.0722;
    let lum = (lin_to_srgb(luma) * 255.0).round().clamp(0.0, 255.0) as u32;
    let i = lum >> 5;
    let src_i = (i << 5) | (i << 2) | (i >> 1);
    let src = src_i as f32 / 255.0;
    let lin_src = srgb_to_lin(src);
    let dst = 1.0 - src;
    let lin_dst = srgb_to_lin(dst);
    let contrast = 0.2 * lin_dst;
    let apply = |a: f32| a + (1.0 - a) * contrast * a;
    let mut table = [0u8; 256];
    for (k, t) in table.iter_mut().enumerate() {
        let raw = k as f32 / 255.0;
        let srca = apply(raw);
        let v = if (src - dst).abs() < 1.0 / 256.0 {
            srca
        } else {
            let lin_out = lin_src * srca + (1.0 - srca) * lin_dst;
            (lin_to_srgb(lin_out) - dst) / (src - dst)
        };
        *t = (v * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    table
}

/// Glyphs drawn like Skia's glyph-cache path: their coverage rasterised into an A8 mask, corrected with the
/// gamma/contrast table for the colour, then composited in the colour.
fn draw_glyph_masks(canvas: &mut Canvas, placed: &[Placed], c: Color) {
    let mut bounds: Option<IntRect> = None;
    for pl in placed {
        if let Some(b) = device_bounds(pl.path.bounds(), &pl.t, 1.0) {
            bounds = Some(match bounds {
                None => b,
                Some(a) => IntRect::from_ltrb(a.left().min(b.left()), a.top().min(b.top()), a.right().max(b.right()), a.bottom().max(b.bottom())).unwrap_or(a),
            });
        }
    }
    let Some(b) = bounds else { return };
    let dev = canvas.device_rect();
    let (l, t, r, bo) = (b.left().max(dev.left()), b.top().max(dev.top()), b.right().min(dev.right()), b.bottom().min(dev.bottom()));
    if r <= l || bo <= t {
        return;
    }
    let (w, h) = ((r - l) as u32, (bo - t) as u32);
    let Some(mut mask) = Mask::new(w, h) else { return };
    let shift = Transform::from_translate(-l as f32, -t as f32);
    for pl in placed {
        mask.fill_path(&pl.path, FillRule::Winding, true, shift.pre_concat(pl.t));
    }
    let lut = mask_gamma_lut(c);
    let Some(mut pix) = Pixmap::new(w, h) else { return };
    let pc = c.premultiply();
    for (o, v) in pix.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(mask.data().iter()) {
        if *v == 0 {
            continue;
        }
        let a = f32::from(lut[usize::from(*v)]) / 255.0;
        o[0] = (pc.red() * a * 255.0).round() as u8;
        o[1] = (pc.green() * a * 255.0).round() as u8;
        o[2] = (pc.blue() * a * 255.0).round() as u8;
        o[3] = (pc.alpha() * a * 255.0).round() as u8;
    }
    canvas.draw_device_pixmap(l, t, &pix, 1.0, BlendMode::SourceOver);
}

/// A shadow of glyphs: filled in `color`, blurred with `sigma` (text-space units, scaled to the device); unblurred
/// shadows go through the glyph-mask path (`masks`) like the text itself.
fn draw_shadow(canvas: &mut Canvas, placed: &[Placed], c: Color, sigma: f64, masks: bool) {
    let base = canvas.transform();
    if sigma <= 0.0 && masks {
        draw_glyph_masks(canvas, placed, c);
        return;
    }
    if sigma <= 0.0 {
        let paint = solid(c);
        for pl in placed {
            canvas.with_transform(pl.t, |cv| cv.fill_path(&pl.path, &paint));
        }
        return;
    }
    let scale = (base.sx * base.sy - base.kx * base.ky).abs().sqrt();
    let sd = (sigma as f32) * scale;
    let pad = (blur::extent(sd) + 2) as f32;
    let mut bounds: Option<IntRect> = None;
    for pl in placed {
        if let Some(b) = device_bounds(pl.path.bounds(), &pl.t, pad) {
            bounds = Some(match bounds {
                None => b,
                Some(a) => IntRect::from_ltrb(a.left().min(b.left()), a.top().min(b.top()), a.right().max(b.right()), a.bottom().max(b.bottom())).unwrap_or(a),
            });
        }
    }
    let Some(b) = bounds else { return };
    let dev = canvas.device_rect();
    let (l, t, r, bo) = (b.left().max(dev.left()), b.top().max(dev.top()), b.right().min(dev.right()), b.bottom().min(dev.bottom()));
    if r <= l || bo <= t {
        return;
    }
    let (w, h) = ((r - l) as u32, (bo - t) as u32);
    let Some(mut mask) = Mask::new(w, h) else { return };
    let shift = Transform::from_translate(-l as f32, -t as f32);
    for pl in placed {
        mask.fill_path(&pl.path, FillRule::Winding, true, shift.pre_concat(pl.t));
    }
    let mut a: Vec<f32> = mask.data().iter().map(|v| f32::from(*v) / 255.0).collect();
    blur::blur_alpha(&mut a, w as usize, h as usize, sd, sd, blur::Edge::Transparent);
    let Some(mut pix) = Pixmap::new(w, h) else { return };
    let pc = c.premultiply();
    for (o, v) in pix.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(a.iter()) {
        let v = v.clamp(0.0, 1.0);
        o[0] = (pc.red() * v * 255.0).round() as u8;
        o[1] = (pc.green() * v * 255.0).round() as u8;
        o[2] = (pc.blue() * v * 255.0).round() as u8;
        o[3] = (pc.alpha() * v * 255.0).round() as u8;
    }
    canvas.draw_device_pixmap(l, t, &pix, 1.0, BlendMode::SourceOver);
}
