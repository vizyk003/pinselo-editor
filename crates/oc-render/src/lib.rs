// SPDX-License-Identifier: Apache-2.0
//! Pinselo renderer — the Rust port of `@oc/render-core` and `@oc/render-canvaskit` (ADR-0001): one pure-Rust
//! engine for the browser (WASM) and the server (native), so exports look exactly like the editor.
//!
//! **Resolve** (pure data, matches the TS exactly — verified against goldens generated from the TS in
//! `tests/golden/render/`):
//! * [`resolve_page`]: document page → [`DisplayList`] (save/restore/layer/clip/path/line/image/text/qr/
//!   placeholder/video ops), with [`motion`] (entrance presets, emphasis loops, keyframes, exits, motion paths,
//!   click steps, page transitions), [`text_units`] (text revealed by line/word/letter), the compound
//!   [`builders`] (tables, charts, forms, freehand), [`qr`] (a port of the `qrcode` package) and
//!   [`color_matrix`] (image filter/adjustment matrices).
//!
//! **Draw** (the same geometry in every backend; paths parsed once by [`path`] with Skia's SVG path semantics):
//! * [`raster`]: [`Renderer`] draws a display list with tiny-skia into an RGBA pixmap — paints, strokes
//!   (alignment, caps, joins, dashes), layers (opacity, blend modes, drop shadow, blur), clips, images (PNG,
//!   JPEG, WebP, GIF, SVG via resvg; crops, mipmaps, colour matrices, tone/detail, blur, vignette), text through
//!   oc-text (hinted glyphs like CanvasKit's glyph cache, text effects, colour glyphs), QR codes, video frames
//!   ([`VideoFrames`]) and page transitions ([`raster::transition`]).
//! * [`svg`]: the display list as an SVG document with real text.
//! * `pdf` (feature `pdf`, default): the display lists as a vector PDF (krilla) with embedded, subsetted fonts
//!   and real text.
//!
//! Assets come in as bytes through [`AssetProvider`]; fonts through oc-text's `FontRegistry` (see
//! [`Renderer::missing_fonts`]). Nothing in the library does I/O; the `native` feature adds filesystem loading
//! (the `native` module).

pub mod assets;
pub mod bounds;
pub mod builders;
pub mod color_matrix;
pub mod displaylist;
pub mod glyphs;
pub mod jsfmt;
pub mod motion;
#[cfg(feature = "native")]
pub mod native;
pub mod path;
#[cfg(feature = "pdf")]
pub mod pdf;
#[cfg(feature = "pdf")]
pub mod pdfx_check;
pub mod qr;
pub mod raster;
pub mod outline;
pub mod resolve;
pub mod svg;
pub mod svg_import;
pub mod text_units;

pub use bounds::{Dirty, Group, Layering, dirty_region, groups_bounds, layering, translation};
pub use assets::{AssetProvider, FnAssets, NoAssets, VideoFrames};
pub use displaylist::{DLItem, DisplayList, PathStroke, ResolvedPaint, TextFit, ops_of};
pub use raster::{Canvas, RenderOptions, Renderer, encode_jpeg, encode_png};
pub use tiny_skia;
#[cfg(feature = "native")]
pub use native::DirAssets;
pub use resolve::{ResolveOptions, cover_crop_for, refit_crop_for, resolve_page, resolve_paint, shape_photo};
