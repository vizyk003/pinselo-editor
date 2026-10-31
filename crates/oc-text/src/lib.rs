// SPDX-License-Identifier: Apache-2.0
//! Pinselo text engine — the Rust port of `@oc/text` (ADR-0001).
//!
//! CanvasKit/SkParagraph is replaced by a pure-Rust stack, so the same engine runs in the browser (WASM) and on
//! the server (native): **parley** (itemization, bidi, ICU4X line breaking, alignment) shaping with **HarfRust**,
//! **fontique** as the font collection (system fonts disabled), and **skrifa** for font parsing, metrics and
//! inspection.
//!
//! - [`FontRegistry`]: manifest-driven registry. Fonts come in as bytes (hash-checked against the manifest);
//!   filesystem loading is behind the `native` feature ([`native`]).
//! - [`fallback_families`]: the per-script Noto fallback chain.
//! - [`TextEngine`]: paragraph layout producing [`TextLayout`] (lines, glyph runs, clusters, list markers,
//!   measured box), shrink-to-fit ([`TextEngine::layout_fit`]), text on an arc ([`TextEngine::curved`]); caret
//!   and selection geometry are methods on [`TextLayout`].
//! - [`FontInspector`] / [`ParsingInspector`] / [`inspect_font`]: validation and metadata of untrusted font
//!   bytes (used by `oc-gfonts`).
//!
//! The vertical metric model reproduces SkParagraph as configured by the TS (half-leading line boxes, a strut,
//! whole-pixel line heights, floored wrap width); see [`layout`]. Glyph positions are HarfRust's and are not
//! expected to equal SkParagraph's to the sub-pixel.

mod caret;
mod case;
mod curved;
mod fallback;
mod fonts;
mod inspect;
pub mod layout;
mod manifest;
#[cfg(feature = "native")]
pub mod native;
mod utf16;
mod wrap;

pub use case::apply_case;
pub use fallback::fallback_families;
pub use fonts::{FALLBACK_FAMILY, FaceRequest, FontError, FontRegistry, LoadedFace};
pub use inspect::{FaceMetrics, FontInfo, FontInspector, InspectError, MAX_FONT_BYTES, MagicBytesInspector, ParsingInspector, inspect_font, inspect_font_index};
pub use layout::{
    ClusterBox, CurvedGlyph, CurvedLayout, CurvedRun, GlyphRun, LaidLine, LaidParagraph, MissingFace, PositionedGlyph, TextEngine, TextLayout, faces,
    paint_to_color, required_faces, scale_body,
};
pub use manifest::{FontFace, FontFamily, FontManifest};
