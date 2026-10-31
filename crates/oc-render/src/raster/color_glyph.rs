// SPDX-License-Identifier: Apache-2.0
//! Colour glyphs (emoji fonts): COLRv0/v1 painted through skrifa's `ColorPainter` onto the canvas, and embedded
//! bitmaps (CBDT/sbix PNG or BGRA strikes) scaled to the font size.

use skrifa::color::{Brush, ColorPainter, ColorPalettes, ColorStop, CompositeMode, Extend, Transform as ColrTransform};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::types::BoundingBox;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use tiny_skia::{BlendMode, Color, FilterQuality, GradientStop, LinearGradient, Paint, Pattern, Point, RadialGradient, Rect, SpreadMode, SweepGradient, Transform};

use super::canvas::{Canvas, LayerPaint};
use crate::glyphs::GlyphCache;

fn composite(m: CompositeMode) -> BlendMode {
    match m {
        CompositeMode::Clear => BlendMode::Clear,
        CompositeMode::Src => BlendMode::Source,
        CompositeMode::Dest => BlendMode::Destination,
        CompositeMode::DestOver => BlendMode::DestinationOver,
        CompositeMode::SrcIn => BlendMode::SourceIn,
        CompositeMode::DestIn => BlendMode::DestinationIn,
        CompositeMode::SrcOut => BlendMode::SourceOut,
        CompositeMode::DestOut => BlendMode::DestinationOut,
        CompositeMode::SrcAtop => BlendMode::SourceAtop,
        CompositeMode::DestAtop => BlendMode::DestinationAtop,
        CompositeMode::Xor => BlendMode::Xor,
        CompositeMode::Plus => BlendMode::Plus,
        CompositeMode::Screen => BlendMode::Screen,
        CompositeMode::Overlay => BlendMode::Overlay,
        CompositeMode::Darken => BlendMode::Darken,
        CompositeMode::Lighten => BlendMode::Lighten,
        CompositeMode::ColorDodge => BlendMode::ColorDodge,
        CompositeMode::ColorBurn => BlendMode::ColorBurn,
        CompositeMode::HardLight => BlendMode::HardLight,
        CompositeMode::SoftLight => BlendMode::SoftLight,
        CompositeMode::Difference => BlendMode::Difference,
        CompositeMode::Exclusion => BlendMode::Exclusion,
        CompositeMode::Multiply => BlendMode::Multiply,
        CompositeMode::HslHue => BlendMode::Hue,
        CompositeMode::HslSaturation => BlendMode::Saturation,
        CompositeMode::HslColor => BlendMode::Color,
        CompositeMode::HslLuminosity => BlendMode::Luminosity,
        _ => BlendMode::SourceOver,
    }
}

fn spread(e: Extend) -> SpreadMode {
    match e {
        Extend::Repeat => SpreadMode::Repeat,
        Extend::Reflect => SpreadMode::Reflect,
        _ => SpreadMode::Pad,
    }
}

struct Painter<'a> {
    canvas: &'a mut Canvas,
    file: &'a str,
    data: &'a [u8],
    glyphs: &'a mut GlyphCache,
    palette: Vec<Color>,
    foreground: Color,
}

impl Painter<'_> {
    fn color(&self, index: u16, alpha: f32) -> Color {
        let mut c = if index == 0xFFFF { self.foreground } else { self.palette.get(usize::from(index)).copied().unwrap_or(Color::BLACK) };
        c.set_alpha((c.alpha() * alpha).clamp(0.0, 1.0));
        c
    }

    fn stops(&self, s: &[ColorStop]) -> Vec<GradientStop> {
        s.iter().map(|c| GradientStop::new(c.offset, self.color(c.palette_index, c.alpha))).collect()
    }
}

impl ColorPainter for Painter<'_> {
    fn push_transform(&mut self, t: ColrTransform) {
        self.canvas.save();
        self.canvas.concat(Transform::from_row(t.xx, t.yx, t.xy, t.yy, t.dx, t.dy));
    }

    fn pop_transform(&mut self) {
        self.canvas.restore();
    }

    fn push_clip_glyph(&mut self, gid: GlyphId) {
        self.canvas.save();
        match self.glyphs.outline(self.file, self.data, gid.to_u32()).and_then(|o| o.skia) {
            // Outlines are cached y-down; COLR paints in font units, y up.
            Some(p) => {
                let m = self.canvas.transform().pre_scale(1.0, -1.0);
                self.canvas.with_transform(m, |c| c.clip_path(&p, false));
            }
            None => self.canvas.clip_empty(),
        }
    }

    fn push_clip_box(&mut self, b: BoundingBox<f32>) {
        self.canvas.save();
        if let Some(r) = Rect::from_ltrb(b.x_min, b.y_min, b.x_max, b.y_max) {
            self.canvas.clip_rect(r);
        }
    }

    fn pop_clip(&mut self) {
        self.canvas.restore();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        let m = self.canvas.transform();
        let shader = match brush {
            Brush::Solid { palette_index, alpha } => Some(tiny_skia::Shader::SolidColor(self.color(palette_index, alpha))),
            Brush::LinearGradient { p0, p1, color_stops, extend } => {
                LinearGradient::new(Point::from_xy(p0.x, p0.y), Point::from_xy(p1.x, p1.y), self.stops(color_stops), spread(extend), m)
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend } => {
                RadialGradient::new(Point::from_xy(c0.x, c0.y), r0.max(0.0), Point::from_xy(c1.x, c1.y), r1.max(0.0), self.stops(color_stops), spread(extend), m)
            }
            Brush::SweepGradient { c0, start_angle, end_angle, color_stops, extend } => {
                SweepGradient::new(Point::from_xy(c0.x, c0.y), start_angle, end_angle, self.stops(color_stops), spread(extend), m)
            }
        };
        if let Some(shader) = shader {
            let paint = Paint { shader, anti_alias: true, ..Default::default() };
            self.canvas.fill_all(&paint);
        }
    }

    fn push_layer(&mut self, mode: CompositeMode) {
        self.canvas.save_layer(LayerPaint { blend: composite(mode), ..Default::default() });
    }

    fn pop_layer(&mut self) {
        self.canvas.restore();
    }
}

/// Draw glyph `gid` as a colour glyph at `at` (local glyph origin on the baseline) with `k` = size / upem.
/// Returns false when the face has no colour data for it (the caller draws the outline).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_color_glyph(canvas: &mut Canvas, font: &FontRef<'_>, file: &str, data: &[u8], glyphs: &mut GlyphCache, gid: u32, at: Transform, k: f32, size: f32, fg: Color) -> bool {
    let id = GlyphId::new(gid);
    if let Some(cg) = font.color_glyphs().get(id) {
        let palette: Vec<Color> = ColorPalettes::new(font)
            .get(0)
            .map(|p| p.colors().iter().map(|c| Color::from_rgba8(c.red, c.green, c.blue, c.alpha)).collect())
            .unwrap_or_default();
        canvas.save();
        canvas.concat(at);
        canvas.scale(k, -k);
        let mut painter = Painter { canvas, file, data, glyphs, palette, foreground: fg };
        let ok = cg.paint(LocationRef::default(), &mut painter).is_ok();
        painter.canvas.restore();
        return ok;
    }
    let strikes = skrifa::bitmap::BitmapStrikes::new(font);
    let Some(bg) = strikes.glyph_for_size(Size::new(size), id) else { return false };
    let pix = match bg.data {
        skrifa::bitmap::BitmapData::Png(bytes) => crate::assets::decode_raster(bytes),
        skrifa::bitmap::BitmapData::Bgra(bytes) => {
            let mut rgba: Vec<u8> = bytes.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
            rgba.truncate((bg.width * bg.height * 4) as usize);
            tiny_skia::IntSize::from_wh(bg.width, bg.height).and_then(|s| tiny_skia::Pixmap::from_vec(rgba, s))
        }
        skrifa::bitmap::BitmapData::Mask(_) => None,
    };
    let Some(pix) = pix else { return false };
    let s = size / bg.ppem_x.max(1.0);
    let upem_k = k;
    let x0 = bg.bearing_x * upem_k + bg.inner_bearing_x * s;
    let (w, h) = (pix.width() as f32 * s, pix.height() as f32 * s);
    let top = match bg.placement_origin {
        skrifa::bitmap::Origin::TopLeft => -(bg.bearing_y * upem_k + bg.inner_bearing_y * s),
        skrifa::bitmap::Origin::BottomLeft => -(bg.bearing_y * upem_k + bg.inner_bearing_y * s) - h,
    };
    let Some(r) = Rect::from_xywh(x0, top, w, h) else { return false };
    let t = Transform::from_row(s, 0.0, 0.0, s, x0, top);
    let paint = Paint { shader: Pattern::new(pix.as_ref(), SpreadMode::Pad, FilterQuality::Bilinear, 1.0, t), anti_alias: true, ..Default::default() };
    canvas.save();
    canvas.concat(at);
    canvas.fill_rect(r, &paint);
    canvas.restore();
    true
}
