// SPDX-License-Identifier: Apache-2.0
//! Page pictures with the hayro rasteriser (the TS uses poppler's `pdftocairo -png -r 144`): white background,
//! `ceil(size × 144 / 72)` pixels like pdftocairo.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{self, Pixmap, RenderContext};
use hayro::{RenderCache, RenderSettings, render_into};

pub(crate) const DPI_SCALE: f64 = 144.0 / 72.0;

/// RGBA pixels of a rendered page.
pub(crate) struct Img {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// Render every page (up to `count`) of `bytes`; `None` entries for pages that could not be rendered.
pub(crate) fn render_pages(bytes: &[u8], count: usize) -> Vec<Option<Img>> {
    let Ok(pdf) = Pdf::new(bytes.to_vec()) else { return (0..count).map(|_| None).collect() };
    let cache = RenderCache::new();
    let settings = InterpreterSettings { render_annotations: false, ..InterpreterSettings::default() };
    pdf.pages()
        .iter()
        .take(count)
        .map(|page| {
            let v = super::view_box(page);
            let (mut w, mut h) = (v[2] - v[0], v[3] - v[1]);
            if matches!(page.rotation(), hayro::hayro_syntax::page::Rotation::Horizontal | hayro::hayro_syntax::page::Rotation::FlippedHorizontal) {
                std::mem::swap(&mut w, &mut h);
            }
            let pw = (w * DPI_SCALE - 1e-6).ceil().max(1.0);
            let ph = (h * DPI_SCALE - 1e-6).ceil().max(1.0);
            if pw > 16384.0 || ph > 16384.0 {
                return None;
            }
            let (pw, ph) = (pw as u16, ph as u16);
            let mut ctx = RenderContext::new(pw, ph);
            let transform = hayro::kurbo::Affine::scale(DPI_SCALE) * hayro::hayro_interpret::util::TransformExt::to_kurbo(&page.initial_transform(true));
            render_into(page, &cache, &settings, &RenderSettings::default(), &mut ctx, transform);
            ctx.flush();
            let mut pixmap = Pixmap::new(pw, ph);
            ctx.render_with(
                &mut pixmap,
                &mut vello_cpu::Resources::default(),
                vello_cpu::RasterizerSettings { target_init: vello_cpu::TargetInit::Clear(WHITE), ..Default::default() },
            );
            // Opaque background: premultiplied = straight RGBA.
            Some(Img { width: usize::from(pw), height: usize::from(ph), data: pixmap.data_as_u8_slice().to_vec() })
        })
        .collect()
}

/// 8-bit RGB PNG of a rendered page.
pub(crate) fn png(img: &Img) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width as u32, img.height as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let Ok(mut w) = enc.write_header() else { return out };
        let rgb: Vec<u8> = img.data.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        let _ = w.write_image_data(&rgb);
    }
    out
}
