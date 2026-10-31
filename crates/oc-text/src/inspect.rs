// SPDX-License-Identifier: Apache-2.0
//! Font-file inspection: validating untrusted bytes (Google downloads, uploaded brand fonts) and reading the
//! metadata the registry needs. This is the parsing half of the `HOOK(oc-text)` seam in `oc-gfonts`.

use skrifa::raw::TableProvider;
use skrifa::raw::tables::os2::SelectionFlags;
use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider};

/// Largest font file accepted (the TS check).
pub const MAX_FONT_BYTES: usize = 40 * 1024 * 1024;

/// Validation of downloaded/uploaded font bytes. `Err(message)` rejects them.
pub trait FontInspector: Send + Sync + 'static {
    fn validate(&self, bytes: &[u8]) -> Result<(), String>;
}

/// Exactly the TS check: at most 40 MiB and a TrueType/OpenType magic number (`00010000`, `true`, `OTTO`).
#[derive(Debug, Clone, Copy, Default)]
pub struct MagicBytesInspector;

fn magic_ok(bytes: &[u8]) -> bool {
    let magic = bytes.get(..4).unwrap_or(bytes);
    [[0x00, 0x01, 0x00, 0x00], *b"true", *b"OTTO"].iter().any(|m| m.as_slice() == magic)
}

impl FontInspector for MagicBytesInspector {
    fn validate(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_FONT_BYTES || !magic_ok(bytes) { Err("not a font file".into()) } else { Ok(()) }
    }
}

/// The magic-number check plus a real parse: the table directory, `head`, `maxp` (≥ 1 glyph), `hhea`/`hmtx` and a
/// Unicode `cmap` must all be readable, so a file that passes can actually be shaped.
#[derive(Debug, Clone, Copy, Default)]
pub struct ParsingInspector;

impl FontInspector for ParsingInspector {
    fn validate(&self, bytes: &[u8]) -> Result<(), String> {
        MagicBytesInspector.validate(bytes)?;
        inspect_font(bytes).map(|_| ()).map_err(|e| format!("not a font file: {e}"))
    }
}

/// What the registry (and font uploads) need to know about one face.
#[derive(Debug, Clone, PartialEq)]
pub struct FontInfo {
    /// Typographic family name (name ID 16, else 1).
    pub family: String,
    /// Typographic subfamily (name ID 17, else 2), e.g. "Bold Italic".
    pub subfamily: String,
    /// `OS/2.usWeightClass` (400 when absent).
    pub weight: u16,
    /// Italic or oblique per `OS/2.fsSelection` / `head.macStyle`, or a non-zero `post.italicAngle`.
    pub italic: bool,
    pub glyph_count: u16,
    pub units_per_em: u16,
    /// Has `fvar` axes.
    pub variable: bool,
    pub metrics: FaceMetrics,
}

/// Vertical metrics in em units (ascent positive up, descent positive down), chosen like FreeType/Skia:
/// `OS/2` typo metrics when `USE_TYPO_METRICS` is set, else `hhea`, else typo/win when `hhea` is zero.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FaceMetrics {
    pub ascent: f64,
    pub descent: f64,
    pub leading: f64,
    pub underline_offset: f64,
    pub underline_thickness: f64,
    pub strikeout_offset: f64,
    pub strikeout_thickness: f64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum InspectError {
    #[error("unreadable font data: {0}")]
    Parse(String),
    #[error("missing or invalid `{0}` table")]
    Table(&'static str),
    #[error("font has no glyphs")]
    NoGlyphs,
    #[error("font has no Unicode cmap")]
    NoCmap,
}

/// Parse a font (first face of a collection) and read its metadata.
pub fn inspect_font(bytes: &[u8]) -> Result<FontInfo, InspectError> {
    inspect_font_index(bytes, 0)
}

pub fn inspect_font_index(bytes: &[u8], index: u32) -> Result<FontInfo, InspectError> {
    let font = FontRef::from_index(bytes, index).map_err(|e| InspectError::Parse(e.to_string()))?;
    let head = font.head().map_err(|_| InspectError::Table("head"))?;
    let maxp = font.maxp().map_err(|_| InspectError::Table("maxp"))?;
    if maxp.num_glyphs() == 0 {
        return Err(InspectError::NoGlyphs);
    }
    font.hhea().map_err(|_| InspectError::Table("hhea"))?;
    font.hmtx().map_err(|_| InspectError::Table("hmtx"))?;
    let charmap = font.charmap();
    if !charmap.has_map() {
        return Err(InspectError::NoCmap);
    }
    let os2 = font.os2().ok();
    let post = font.post().ok();
    let name = |ids: &[StringId]| {
        ids.iter()
            .find_map(|id| font.localized_strings(*id).english_or_first().map(|s| s.chars().collect::<String>()))
            .unwrap_or_default()
    };
    let family = name(&[StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]);
    let subfamily = name(&[StringId::TYPOGRAPHIC_SUBFAMILY_NAME, StringId::SUBFAMILY_NAME]);
    let weight = os2.as_ref().map(|o| o.us_weight_class()).filter(|w| *w > 0).unwrap_or(400);
    let italic = os2.as_ref().is_some_and(|o| o.fs_selection().intersects(SelectionFlags::ITALIC | SelectionFlags::OBLIQUE))
        || head.mac_style().bits() & 0x2 != 0
        || post.as_ref().is_some_and(|p| p.italic_angle().to_f64() != 0.0);
    Ok(FontInfo {
        family,
        subfamily,
        weight,
        italic,
        glyph_count: maxp.num_glyphs(),
        units_per_em: head.units_per_em(),
        variable: !font.axes().is_empty(),
        metrics: face_metrics(&font),
    })
}

pub(crate) fn face_metrics(font: &FontRef<'_>) -> FaceMetrics {
    let m = font.metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    let upem = f64::from(m.units_per_em.max(1));
    let (uo, ut) = m.underline.map(|d| (f64::from(d.offset), f64::from(d.thickness))).unwrap_or((-0.1 * upem, 0.05 * upem));
    let (so, st) = m.strikeout.map(|d| (f64::from(d.offset), f64::from(d.thickness))).unwrap_or((0.3 * upem, 0.05 * upem));
    FaceMetrics {
        ascent: f64::from(m.ascent) / upem,
        descent: -f64::from(m.descent) / upem,
        leading: f64::from(m.leading) / upem,
        underline_offset: uo / upem,
        underline_thickness: ut / upem,
        strikeout_offset: so / upem,
        strikeout_thickness: st / upem,
    }
}
