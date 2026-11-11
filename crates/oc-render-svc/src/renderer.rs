// SPDX-License-Identifier: Apache-2.0
//! The drawing seam: everything the render service needs from a painter, as one trait.
//!
//! The TS service calls CanvasKit through `@oc/render-canvaskit/node` (`NodeRenderer`: `render`, `renderRgba`,
//! `painter.text` layout and font picking, `displayListToSvg`, `ck.Path`, image codecs) and pdf-lib/resvg. Every
//! one of those uses is a method here, so the exporters in this crate are plain code over [`Renderer`] and the
//! painter can be swapped without touching them:
//!
//! | method | TS call it replaces | used by |
//! |---|---|---|
//! | [`Renderer::render_rgba`] | `renderer.renderRgba` | PowerPoint element pictures, video frames |
//! | [`Renderer::render_image`] | `renderer.render` (PNG/JPEG) | thumbnails, page-png, png/jpg/zip exports, doc embeds, doc charts |
//! | [`Renderer::compose_transition`] | `drawTransition` on two frames | mp4/gif page transitions |
//! | [`Renderer::vector_pdf`] | `exportVectorPdf` | pdf, pdf-print |
//! | [`Renderer::raster_pdf`] | `exportRasterPdf` | pdf-flat |
//! | [`Renderer::page_svg`] | `displayListToSvg(resolvePage(...))` | svg export, websites |
//! | [`Renderer::elements_svg`] | `displayListToSvg` of a display-list subset | website phone layouts (`svgOf`) |
//! | [`Renderer::text_faces`] | `TextEngine.faces` over a page's text items | website `@font-face` |
//! | [`Renderer::pick_face`] | `painter.text.fonts.pickFace` | website font names and files |
//! | [`Renderer::layout_text`] | `painter.text.layout` (+ `getLineMetrics`) | Docs pagination (docLayout) |
//! | [`Renderer::encode_png`] | `ck.MakeImage(...).encodeToBytes(PNG)` | PowerPoint pictures |
//! | [`Renderer::transcode_png`] | `ck.MakeImageFromEncoded(...).encodeToBytes(PNG)` | WebP/other images in .docx/.pptx |
//! | [`Renderer::rasterize_svg`] | `new Resvg(svg).render().asPng()` | recoloured SVG assets |
//! | [`Renderer::path_commands`] | `ck.Path.MakeFromSVGString(d).toCmds()` | PowerPoint custom geometry |
//!
//! Implementations fetch asset bytes and font files through the [`RenderHost`] passed to each call (the service
//! implements it over Postgres, the object store and Google Fonts). The service runs on
//! [`crate::oc_renderer::OcRenderer`] (crates/oc-render). [`StubRenderer`], whose every method returns
//! [`RenderError::NotImplemented`] (answered with `501 Not Implemented`), remains for builds and tests without a
//! painter.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use oc_gfonts::manifest::{FontFamily, FontManifest};
use oc_schema::{DesignDoc, TextBody};
use serde::{Deserialize, Serialize};

/// Why a renderer call failed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RenderError {
    /// The capability needs the painter, which this build does not have (HTTP 501).
    #[error("{0} needs the renderer (crates/oc-render), which this build does not include yet")]
    NotImplemented(&'static str),
    /// The renderer ran and failed (HTTP 500).
    #[error("{0}")]
    Failed(String),
    /// A strict render ([`RasterOptions::strict_fonts`]) could not load these font families (HTTP 422).
    #[error("these fonts could not be loaded: {}", .0.join(", "))]
    MissingFonts(Vec<String>),
}

pub type RenderResult<T> = Result<T, RenderError>;
pub type RenderFuture<'a, T> = BoxFuture<'a, RenderResult<T>>;

/// A raster in RGBA8, unpremultiplied, row-major (CanvasKit `RGBA_8888` / `Unpremul`).
#[derive(Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for Rgba {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rgba({}×{})", self.width, self.height)
    }
}

impl Rgba {
    /// A transparent raster.
    pub fn blank(width: u32, height: u32) -> Rgba {
        Rgba {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
        }
    }

    /// The `w × h` region at `(x, y)` (clamped to the raster, at least 1 × 1), as pptx.ts cut slices.
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Rgba {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        let cw = w.min(self.width - x).max(1);
        let ch = h.min(self.height - y).max(1);
        let mut out = vec![0u8; cw as usize * ch as usize * 4];
        for row in 0..ch as usize {
            let sy = y as usize + row;
            if sy >= self.height as usize {
                break;
            }
            let src = (sy * self.width as usize + x as usize) * 4;
            let n = (cw as usize * 4).min(self.pixels.len().saturating_sub(src));
            out[row * cw as usize * 4..row * cw as usize * 4 + n]
                .copy_from_slice(&self.pixels[src..src + n]);
        }
        Rgba {
            width: cw,
            height: ch,
            pixels: out,
        }
    }
}

/// Options of a page render (`RenderOptions` + `ResolveOptions` of render-canvaskit/render-core).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RasterOptions {
    /// Output pixels per document px.
    pub scale: f64,
    /// Skip the page background (transparent PNG / element pictures).
    pub transparent: bool,
    /// Elements not drawn (with their children).
    pub skip: Vec<String>,
    /// Page time for animated output (video frames); `None` = settled state.
    pub time_ms: Option<f64>,
    /// Design time of the caption overlay; `None` = no captions.
    pub caption_time_ms: Option<f64>,
    /// Decoded video frames by playback key (`VideoSlot::key`) — the TS `painter.videoFrames` hook during video
    /// export. Videos without a frame show their poster.
    #[serde(skip)]
    pub video_frames: Option<VideoFrameSet>,
    /// Fail with [`RenderError::MissingFonts`] instead of drawing a fallback face when a font of the page cannot be
    /// loaded (exports that are published as they are, e.g. the REST API's JPEG export).
    #[serde(default)]
    pub strict_fonts: bool,
}

/// Current video frames by playback key (unpremultiplied RGBA at the video's size).
pub type VideoFrameSet = Arc<std::collections::HashMap<String, Arc<Rgba>>>;

impl Default for RasterOptions {
    fn default() -> Self {
        RasterOptions {
            scale: 1.0,
            transparent: false,
            skip: Vec::new(),
            time_ms: None,
            caption_time_ms: None,
            video_frames: None,
            strict_fonts: false,
        }
    }
}

impl RasterOptions {
    pub fn at_scale(scale: f64) -> RasterOptions {
        RasterOptions {
            scale,
            ..RasterOptions::default()
        }
    }
}

/// Encoded raster formats (`format` of `renderer.render`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    /// JPEG with a quality of 0..=100 (drawn over white).
    Jpeg {
        quality: u8,
    },
}

/// Print production (`pdf-print`): bleed, marks, PDF/X standard and colour profile (see [`crate::print`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PrintSetup {
    pub bleed_mm: f64,
    pub crop_marks: bool,
    #[serde(default)]
    pub registration_marks: bool,
    #[serde(default)]
    pub color_bars: bool,
    #[serde(default)]
    pub slug_mm: Option<f64>,
    /// `none`, `pdfx4` or `pdfx1a`.
    #[serde(default)]
    pub standard: String,
    /// Flatten transparency.
    #[serde(default)]
    pub flatten: bool,
    /// `relative` (colorimetric + BPC) or `perceptual`.
    #[serde(default)]
    pub intent: String,
    /// The output profile, loaded.
    #[serde(skip)]
    pub profile: Option<ResolvedProfile>,
}

/// An output profile with its output-intent metadata.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResolvedProfile {
    /// Cache key (`bundled:<id>` or `asset:<sha256>`).
    pub key: String,
    pub bytes: Arc<Vec<u8>>,
    pub condition_id: String,
    pub condition: Option<String>,
    pub registry: Option<String>,
    pub info: Option<String>,
}

/// `VectorPdfOptions` (+ the link areas the TS computes from the document, see [`crate::oc_renderer::link_areas`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VectorPdfOptions {
    pub title: Option<String>,
    pub producer: Option<String>,
    pub print: Option<PrintSetup>,
}

/// `exportRasterPdf(r, doc, pages, print)`: page rasters (150 dpi PNG, or 300 dpi JPEG when `print`) under an
/// invisible Helvetica text layer and link annotations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RasterPdfOptions {
    pub print: bool,
    pub producer: Option<String>,
}

/// URL of an asset key for SVG that references images instead of inlining them (websites).
pub type AssetHref = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// `displayListToSvg` options.
#[derive(Clone, Default)]
pub struct SvgOptions {
    /// Reference images by URL (`assetHref`) instead of inlining them as data URIs.
    pub asset_href: Option<AssetHref>,
    /// Leave the `@font-face` block out (the host page declares the fonts); otherwise the faces a page uses are
    /// embedded.
    pub external_fonts: bool,
}

impl std::fmt::Debug for SvgOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgOptions")
            .field("asset_href", &self.asset_href.is_some())
            .field("external_fonts", &self.external_fonts)
            .finish()
    }
}

/// A requested face (`TextEngine.faces`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceRequest {
    pub family: String,
    pub weight: f64,
    pub italic: bool,
}

/// The face a request resolves to (`fonts.pickFace`: family with fallback, nearest weight/style, file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PickedFace {
    pub family: String,
    pub weight: f64,
    pub italic: bool,
    /// Manifest path (`inter/Inter_400Regular.ttf`, `gf/<slug>/<w>.ttf`, `custom/<id>.<ext>`).
    pub file: String,
}

/// One line of a laid-out paragraph (SkParagraph `LineMetrics` fields docLayout reads).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineMetrics {
    /// UTF-16 offset of the line's first character in the paragraph.
    pub start: usize,
    pub baseline: f64,
    pub descent: f64,
}

/// A laid-out text body: its height and the lines of its first paragraph (docLayout splits single-paragraph
/// bodies at line boundaries).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextMeasure {
    pub height: f64,
    pub lines: Vec<LineMetrics>,
}

/// A decoded image re-encoded as PNG, with its pixel size.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// SVG path data parsed by the painter (`SkPath::toCmds` verbs, arcs already converted).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PathCmd {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    /// Control point, end point, weight.
    Conic(f64, f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

impl PathCmd {
    /// From CanvasKit's flat `toCmds()` array (verbs 0 move, 1 line, 2 quad, 3 conic, 4 cubic, 5 close).
    pub fn from_flat(cmds: &[f64]) -> Vec<PathCmd> {
        let mut out = Vec::new();
        let mut i = 0;
        let at = |k: usize| cmds.get(k).copied().unwrap_or(f64::NAN);
        while i < cmds.len() {
            let v = cmds[i] as i64;
            i += 1;
            let c = match v {
                0 => {
                    i += 2;
                    PathCmd::Move(at(i - 2), at(i - 1))
                }
                1 => {
                    i += 2;
                    PathCmd::Line(at(i - 2), at(i - 1))
                }
                2 => {
                    i += 4;
                    PathCmd::Quad(at(i - 4), at(i - 3), at(i - 2), at(i - 1))
                }
                3 => {
                    i += 5;
                    PathCmd::Conic(at(i - 5), at(i - 4), at(i - 3), at(i - 2), at(i - 1))
                }
                4 => {
                    i += 6;
                    PathCmd::Cubic(
                        at(i - 6),
                        at(i - 5),
                        at(i - 4),
                        at(i - 3),
                        at(i - 2),
                        at(i - 1),
                    )
                }
                _ => PathCmd::Close,
            };
            out.push(c);
        }
        out
    }
}

/// A page transition being drawn (`transitionFrame(kind, progress, direction)` inputs: the page's transition
/// kind and direction, and `transitionProgress(page, t)`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionSpec {
    pub kind: String,
    pub progress: f64,
    pub direction: Option<String>,
}

/// What a renderer reads through the service: asset bytes and font files.
pub trait RenderHost: Send + Sync {
    /// Encoded bytes of an asset key: an asset id, `<id>/poster` (a video's poster frame), `<id>/recolor/<pairs>`
    /// (a recoloured SVG, rasterised) or a picture generated during the export (embedded doc pages).
    fn asset_bytes<'a>(&'a self, key: &'a str) -> RenderFuture<'a, Vec<u8>>;
    /// A font file by manifest path (bundled, pinned Google font or uploaded `custom/<id>.<ext>`).
    fn font_file<'a>(&'a self, file: &'a str) -> RenderFuture<'a, Vec<u8>>;
    /// Uploaded brand fonts for families the manifest does not know (`resolveFamilies`).
    fn custom_families<'a>(&'a self, families: &'a [String]) -> RenderFuture<'a, Vec<FontFamily>>;
    /// The font manifest (bundled fonts + the Google catalogue).
    fn manifest(&self) -> &FontManifest;
    /// Where the bundled font files live.
    fn fonts_dir(&self) -> PathBuf;
}

/// The painter, as the render service uses it. See the module docs for which TS call each method replaces.
pub trait Renderer: Send + Sync + 'static {
    /// A short name for logs (`"stub"`, `"oc-render"`).
    fn engine(&self) -> &'static str;

    /// Rasterise a page (`renderRgba`): `round(dims × scale)` pixels.
    fn render_rgba<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
        opts: &'a RasterOptions,
    ) -> RenderFuture<'a, Rgba>;

    /// Rasterise and encode a page (`render`).
    fn render_image<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
        opts: &'a RasterOptions,
        format: ImageFormat,
    ) -> RenderFuture<'a, Vec<u8>>;

    /// One page-transition frame from the outgoing and incoming frames, drawn like Present mode.
    fn compose_transition<'a>(
        &'a self,
        prev: &'a Rgba,
        next: &'a Rgba,
        transition: &'a TransitionSpec,
    ) -> RenderFuture<'a, Rgba>;

    /// Vector PDF of the pages (`exportVectorPdf`): text as embedded fonts, link annotations, print boxes.
    fn vector_pdf<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_ids: &'a [String],
        opts: &'a VectorPdfOptions,
    ) -> RenderFuture<'a, Vec<u8>>;

    /// Flattened PDF (`exportRasterPdf`).
    fn raster_pdf<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_ids: &'a [String],
        opts: &'a RasterPdfOptions,
    ) -> RenderFuture<'a, Vec<u8>>;

    /// A page as one SVG document (`displayListToSvg(resolvePage(doc, page))`).
    fn page_svg<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
        opts: &'a SvgOptions,
    ) -> RenderFuture<'a, String>;

    /// The display-list items of the given elements only (and their descendants, plus layer ops), without the
    /// page background: website phone layouts cut visuals out of a page this way.
    fn elements_svg<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
        ids: &'a BTreeSet<String>,
        opts: &'a SvgOptions,
    ) -> RenderFuture<'a, String>;

    /// The faces the text items of a page's display list request, in display-list order (duplicates kept).
    fn text_faces<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
    ) -> RenderFuture<'a, Vec<FaceRequest>>;

    /// Resolve a face request against the loaded fonts (`fonts.pickFace`).
    fn pick_face<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        request: &'a FaceRequest,
    ) -> RenderFuture<'a, PickedFace>;

    /// Lay a text body out at a width (`painter.text.layout`).
    fn layout_text<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        body: &'a TextBody,
        width: f64,
    ) -> RenderFuture<'a, TextMeasure>;

    /// PNG of a raster.
    fn encode_png<'a>(&'a self, rgba: &'a Rgba) -> RenderFuture<'a, Vec<u8>>;

    /// Decode any image the painter can read and re-encode it as PNG; `None` when it cannot be decoded.
    fn transcode_png<'a>(&'a self, bytes: &'a [u8]) -> RenderFuture<'a, Option<DecodedImage>>;

    /// Rasterise SVG source at a pixel width (height from its aspect) as PNG (resvg, no system fonts).
    fn rasterize_svg<'a>(&'a self, svg: &'a str, width: u32) -> RenderFuture<'a, Vec<u8>>;

    /// Parse SVG path data the way the painter does; `Ok(None)` for unparsable data.
    fn path_commands(&self, d: &str) -> RenderResult<Option<Vec<PathCmd>>>;
}

/// The renderer used until `crates/oc-render` is wired in: every method is [`RenderError::NotImplemented`].
#[derive(Debug, Clone, Copy, Default)]
pub struct StubRenderer;

fn ni<'a, T: Send + 'a>(what: &'static str) -> RenderFuture<'a, T> {
    Box::pin(async move { Err(RenderError::NotImplemented(what)) })
}

impl Renderer for StubRenderer {
    fn engine(&self) -> &'static str {
        "stub"
    }
    fn render_rgba<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
        _: &'a RasterOptions,
    ) -> RenderFuture<'a, Rgba> {
        ni("page rasterisation")
    }
    fn render_image<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
        _: &'a RasterOptions,
        _: ImageFormat,
    ) -> RenderFuture<'a, Vec<u8>> {
        ni("PNG/JPEG rendering")
    }
    fn compose_transition<'a>(
        &'a self,
        _: &'a Rgba,
        _: &'a Rgba,
        _: &'a TransitionSpec,
    ) -> RenderFuture<'a, Rgba> {
        ni("page transitions")
    }
    fn vector_pdf<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a [String],
        _: &'a VectorPdfOptions,
    ) -> RenderFuture<'a, Vec<u8>> {
        ni("vector PDF")
    }
    fn raster_pdf<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a [String],
        _: &'a RasterPdfOptions,
    ) -> RenderFuture<'a, Vec<u8>> {
        ni("flattened PDF")
    }
    fn page_svg<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
        _: &'a SvgOptions,
    ) -> RenderFuture<'a, String> {
        ni("SVG")
    }
    fn elements_svg<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
        _: &'a BTreeSet<String>,
        _: &'a SvgOptions,
    ) -> RenderFuture<'a, String> {
        ni("SVG")
    }
    fn text_faces<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
    ) -> RenderFuture<'a, Vec<FaceRequest>> {
        ni("text layout")
    }
    fn pick_face<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a FaceRequest,
    ) -> RenderFuture<'a, PickedFace> {
        ni("font selection")
    }
    fn layout_text<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a TextBody,
        _: f64,
    ) -> RenderFuture<'a, TextMeasure> {
        ni("text layout")
    }
    fn encode_png<'a>(&'a self, _: &'a Rgba) -> RenderFuture<'a, Vec<u8>> {
        ni("PNG encoding")
    }
    fn transcode_png<'a>(&'a self, _: &'a [u8]) -> RenderFuture<'a, Option<DecodedImage>> {
        ni("image decoding")
    }
    fn rasterize_svg<'a>(&'a self, _: &'a str, _: u32) -> RenderFuture<'a, Vec<u8>> {
        ni("SVG rasterisation")
    }
    fn path_commands(&self, _: &str) -> RenderResult<Option<Vec<PathCmd>>> {
        Err(RenderError::NotImplemented("SVG path parsing"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_commands() {
        let c = PathCmd::from_flat(&[0.0, 1.0, 2.0, 1.0, 3.0, 4.0, 5.0]);
        assert_eq!(
            c,
            vec![
                PathCmd::Move(1.0, 2.0),
                PathCmd::Line(3.0, 4.0),
                PathCmd::Close
            ]
        );
    }

    #[test]
    fn crop_clamps() {
        let mut r = Rgba::blank(4, 2);
        r.pixels[4 * 4 + 4..4 * 4 + 8].copy_from_slice(&[1, 2, 3, 4]);
        let c = r.crop(1, 1, 10, 10);
        assert_eq!((c.width, c.height), (3, 1));
        assert_eq!(&c.pixels[0..4], &[1, 2, 3, 4]);
    }
}
