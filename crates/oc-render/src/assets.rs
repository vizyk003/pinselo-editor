// SPDX-License-Identifier: Apache-2.0
//! Assets: encoded bytes come in through [`AssetProvider`] (no I/O in the library, so it builds for WASM) and are
//! decoded once — PNG, JPEG (EXIF orientation applied), WebP and GIF (first frame) with the `image` crate, SVG
//! with usvg (rasterised by resvg at the size it is drawn). Recoloured SVG keys (`<id>/recolor/<pairs>`) are
//! derived from the base asset's SVG source when the provider has no bytes for the key itself.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tiny_skia::{IntSize, Pixmap};

/// Supplies the encoded bytes of an asset by key: an asset id, `<id>/poster` (a video's poster frame) or
/// `<id>/recolor/<pairs>` (a recoloured SVG, A4).
pub trait AssetProvider {
    fn asset(&self, key: &str) -> Option<Cow<'_, [u8]>>;
}

/// No assets: images draw their placeholders.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoAssets;

impl AssetProvider for NoAssets {
    fn asset(&self, _key: &str) -> Option<Cow<'_, [u8]>> {
        None
    }
}

impl AssetProvider for HashMap<String, Vec<u8>> {
    fn asset(&self, key: &str) -> Option<Cow<'_, [u8]>> {
        self.get(key).map(|v| Cow::Borrowed(v.as_slice()))
    }
}

/// An [`AssetProvider`] from a closure.
pub struct FnAssets<F>(pub F);

impl<F: Fn(&str) -> Option<Vec<u8>>> AssetProvider for FnAssets<F> {
    fn asset(&self, key: &str) -> Option<Cow<'_, [u8]>> {
        (self.0)(key).map(Cow::Owned)
    }
}

/// Supplies decoded video frames (the host decodes video). `key` identifies the playback (a grid holds one video
/// per cell). Without it, videos show their poster. Frames are shared (`Arc`): a tiled host draws the same frame
/// into many tiles.
pub trait VideoFrames {
    fn frame(&self, asset: &str, source_time_ms: f64, key: &str) -> Option<Arc<Pixmap>>;
}

/// A decoded raster with lazily built mip levels (each half the size of the previous, box-filtered).
/// A cached trilinear blend: (level, weight in 1/16) and the image.
type BlendEntry = ((u32, u32), Arc<Pixmap>);

/// A cached rasterisation of an SVG: pixel size and the raster.
type RasterEntry = ((u32, u32), Arc<RasterImage>);

pub struct RasterImage {
    pub pixmap: Arc<Pixmap>,
    mips: Mutex<Vec<Arc<Pixmap>>>,
    /// Trilinear blends of two adjacent levels by (level, weight in 1/16).
    blends: Mutex<Vec<BlendEntry>>,
}

impl std::fmt::Debug for RasterImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RasterImage").field("w", &self.pixmap.width()).field("h", &self.pixmap.height()).finish()
    }
}

impl RasterImage {
    pub fn new(pixmap: Pixmap) -> Self {
        Self::shared(Arc::new(pixmap))
    }

    pub fn shared(pixmap: Arc<Pixmap>) -> Self {
        RasterImage { pixmap, mips: Mutex::new(Vec::new()), blends: Mutex::new(Vec::new()) }
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// Mip level `n` (`0` = the image itself) and its scale relative to level 0.
    pub fn level(&self, n: u32) -> (Arc<Pixmap>, f32) {
        if n == 0 {
            return (self.pixmap.clone(), 1.0);
        }
        let mut mips = self.mips.lock().unwrap_or_else(|e| e.into_inner());
        while (mips.len() as u32) < n {
            let prev: &Pixmap = mips.last().map_or(self.pixmap.as_ref(), |p| p.as_ref());
            if prev.width() <= 1 && prev.height() <= 1 {
                break;
            }
            let next = downsample(prev);
            mips.push(Arc::new(next));
        }
        let k = (mips.len() as u32).min(n);
        if k == 0 {
            return (self.pixmap.clone(), 1.0);
        }
        let p = mips[k as usize - 1].clone();
        let s = p.width() as f32 / self.pixmap.width() as f32;
        (p, s)
    }
}

impl RasterImage {
    /// Skia's `MipmapMode::Linear` for a fractional mip `level` (`log2` of the minification): levels ⌊level⌋ and
    /// ⌊level⌋ + 1 blended by the fraction, at the resolution of the finer one. Returns the image and its scale
    /// relative to level 0.
    pub fn trilinear(&self, level: f32) -> (Arc<Pixmap>, f32) {
        if level.is_nan() || level <= 0.0 {
            return (self.pixmap.clone(), 1.0);
        }
        let lo = level.floor() as u32;
        let t = ((level - level.floor()) * 16.0).round() as u32;
        if t == 0 {
            return self.level(lo);
        }
        if t >= 16 {
            return self.level(lo + 1);
        }
        let (fine, fs) = self.level(lo);
        let (coarse, _) = self.level(lo + 1);
        if Arc::ptr_eq(&fine, &coarse) || coarse.width() == fine.width() && coarse.height() == fine.height() {
            return (fine, fs);
        }
        let mut cache = self.blends.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, p)) = cache.iter().find(|(k, _)| *k == (lo, t)) {
            return (p.clone(), fs);
        }
        let blended = Arc::new(blend_levels(&fine, &coarse, t as f32 / 16.0));
        if cache.len() >= 4 {
            cache.remove(0);
        }
        cache.push(((lo, t), blended.clone()));
        (blended, fs)
    }
}

/// `fine × (1 − t) + upsample(coarse) × t` (bilinear upsampling, premultiplied).
fn blend_levels(fine: &Pixmap, coarse: &Pixmap, t: f32) -> Pixmap {
    let (w, h) = (fine.width() as usize, fine.height() as usize);
    let (cw, ch) = (coarse.width() as usize, coarse.height() as usize);
    let (sx, sy) = (cw as f32 / w as f32, ch as f32 / h as f32);
    let src = coarse.data();
    let mut out = fine.clone();
    let d = out.data_mut();
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (ch - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy - fy.floor());
        let y1 = (y0 + 1).min(ch - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (cw - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx - fx.floor());
            let x1 = (x0 + 1).min(cw - 1);
            let o = (y * w + x) * 4;
            for c in 0..4 {
                let at = |xx: usize, yy: usize| f32::from(src[(yy * cw + xx) * 4 + c]);
                let up = (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty;
                d[o + c] = (f32::from(d[o + c]) * (1.0 - t) + up * t).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    // Premultiplied channels may not exceed alpha after rounding.
    for p in d.as_chunks_mut::<4>().0 {
        for c in 0..3 {
            p[c] = p[c].min(p[3]);
        }
    }
    out
}

/// Halve a premultiplied pixmap with a 2×2 box filter (odd edges repeat).
fn downsample(src: &Pixmap) -> Pixmap {
    let (w, h) = (src.width(), src.height());
    let (nw, nh) = (w.div_ceil(2).max(1), h.div_ceil(2).max(1));
    let mut out = Pixmap::new(nw, nh).expect("non-empty");
    let s = src.data();
    let d = out.data_mut();
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sx = (x * 2 + dx).min(w - 1);
                let sy = (y * 2 + dy).min(h - 1);
                let i = ((sy * w + sx) * 4) as usize;
                for c in 0..4 {
                    acc[c] += u32::from(s[i + c]);
                }
            }
            let o = ((y * nw + x) * 4) as usize;
            for c in 0..4 {
                d[o + c] = ((acc[c] + 2) / 4) as u8;
            }
        }
    }
    out
}

/// A decoded asset.
#[derive(Clone)]
pub enum Decoded {
    Raster(Arc<RasterImage>),
    Svg(Arc<SvgImage>),
}

/// A parsed SVG with a cache of rasterisations by pixel size.
pub struct SvgImage {
    pub tree: resvg::usvg::Tree,
    rasters: Mutex<Vec<RasterEntry>>,
}

impl SvgImage {
    /// Natural size in CSS px.
    pub fn size(&self) -> (f32, f32) {
        let s = self.tree.size();
        (s.width(), s.height())
    }

    /// The SVG rasterised to `w × h` pixels (cached; the most recent few sizes are kept).
    pub fn raster(&self, w: u32, h: u32) -> Option<Arc<RasterImage>> {
        let (w, h) = (w.clamp(1, 4096), h.clamp(1, 4096));
        let mut cache = self.rasters.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, r)) = cache.iter().find(|(k, _)| *k == (w, h)) {
            return Some(r.clone());
        }
        let mut pix = Pixmap::new(w, h)?;
        let (sw, sh) = self.size();
        let t = tiny_skia::Transform::from_scale(w as f32 / sw.max(1e-3), h as f32 / sh.max(1e-3));
        resvg::render(&self.tree, t, &mut pix.as_mut());
        let r = Arc::new(RasterImage::new(pix));
        if cache.len() >= 4 {
            cache.remove(0);
        }
        cache.push(((w, h), r.clone()));
        Some(r)
    }
}

/// Whether bytes look like SVG (optionally after a BOM, whitespace, an XML declaration or comments).
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(2048)];
    let s = String::from_utf8_lossy(head);
    let t = s.trim_start_matches('\u{feff}').trim_start();
    t.starts_with("<svg") || ((t.starts_with("<?xml") || t.starts_with("<!--") || t.starts_with("<!DOCTYPE")) && t.contains("<svg"))
}

/// usvg options for untrusted SVG: no file access for external `href`s (data URLs only), no system fonts.
fn svg_options() -> resvg::usvg::Options<'static> {
    let mut o = resvg::usvg::Options::default();
    o.image_href_resolver.resolve_string = Box::new(|_, _| None);
    o
}

pub fn decode_svg(bytes: &[u8]) -> Option<SvgImage> {
    let tree = resvg::usvg::Tree::from_data(bytes, &svg_options()).ok()?;
    Some(SvgImage { tree, rasters: Mutex::new(Vec::new()) })
}

/// Decode a raster (PNG/JPEG/WebP/GIF first frame) into a premultiplied pixmap.
pub fn decode_raster(bytes: &[u8]) -> Option<Pixmap> {
    use image::ImageDecoder;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    img.apply_orientation(orientation);
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut data = rgba.into_raw();
    premultiply(&mut data);
    Pixmap::from_vec(data, IntSize::from_wh(w, h)?)
}

/// Premultiply RGBA8 in place.
pub fn premultiply(data: &mut [u8]) {
    for p in data.as_chunks_mut::<4>().0 {
        let a = u16::from(p[3]);
        if a == 255 {
            continue;
        }
        for c in &mut p[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
}

/// Unpremultiply RGBA8 in place.
pub fn unpremultiply(data: &mut [u8]) {
    // Opaque pixels need nothing: an all-opaque buffer (a scene over an opaque background) is left as it is.
    if data.as_chunks::<4>().0.iter().all(|p| p[3] == 255) {
        return;
    }
    for p in data.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a == 255 {
            continue;
        }
        if a == 0 {
            p[..3].fill(0);
            continue;
        }
        for c in &mut p[..3] {
            *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

/// Decode any supported asset.
pub fn decode(bytes: &[u8]) -> Option<Decoded> {
    if is_svg(bytes) {
        return decode_svg(bytes).map(|s| Decoded::Svg(Arc::new(s)));
    }
    decode_raster(bytes).map(|p| Decoded::Raster(Arc::new(RasterImage::new(p))))
}

/// Decoded assets by key (`None` = known to be unavailable or undecodable).
#[derive(Default)]
pub struct ImageCache {
    entries: HashMap<String, Option<Decoded>>,
}

impl std::fmt::Debug for ImageCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageCache").field("entries", &self.entries.len()).finish()
    }
}

impl ImageCache {
    /// The decoded asset for `key`, loading it through `assets` on first use. Recolour keys fall back to recolouring
    /// the base asset's SVG source, then to the base asset itself (as the editor shows while the variant loads).
    pub fn get(&mut self, key: &str, assets: &dyn AssetProvider) -> Option<Decoded> {
        if let Some(hit) = self.entries.get(key) {
            return hit.clone();
        }
        let decoded = match assets.asset(key) {
            Some(bytes) => decode(&bytes),
            None => match oc_schema::svg::parse_recolor_key(key) {
                Some(rk) => match assets.asset(&rk.asset) {
                    Some(bytes) if is_svg(&bytes) => {
                        let src = String::from_utf8_lossy(&bytes);
                        let svg = oc_schema::svg::recolor_svg(&src, &rk.map);
                        decode_svg(svg.as_bytes()).map(|s| Decoded::Svg(Arc::new(s))).or_else(|| decode(&bytes))
                    }
                    Some(bytes) => decode(&bytes),
                    None => None,
                },
                None => None,
            },
        };
        if self.entries.len() > 512 {
            self.entries.clear();
        }
        self.entries.insert(key.to_owned(), decoded.clone());
        decoded
    }

    /// Forget a key (e.g. after its bytes changed).
    pub fn invalidate(&mut self, key: &str) {
        self.entries.remove(key);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Keep only the keys `keep` accepts.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.entries.retain(|k, _| keep(k));
    }
}
