// SPDX-License-Identifier: Apache-2.0
//! Glyph outlines (skrifa, unhinted, font units with y down) cached per face and glyph, shared by the raster
//! painter, the SVG writer and the PDF writer; colour glyph detection (COLR, CBDT/sbix bitmaps).

use std::collections::HashMap;
use std::sync::Arc;

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, SmoothMode, Target};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::path::PathData;

/// What a face can draw a glyph with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceCaps {
    pub upem: f32,
    pub colr: bool,
    pub bitmaps: bool,
}

struct Pen(PathData);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(f64::from(x), -f64::from(y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(f64::from(x), -f64::from(y));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(f64::from(cx0), -f64::from(cy0), f64::from(x), -f64::from(y));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(f64::from(cx0), -f64::from(cy0), f64::from(cx1), -f64::from(cy1), f64::from(x), -f64::from(y));
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// A glyph outline in font units, y down (`None` when the glyph has no outline).
#[derive(Clone)]
pub struct Outline {
    pub data: Arc<PathData>,
    pub skia: Option<Arc<tiny_skia::Path>>,
}

/// Largest pixel size Skia rasterises glyphs through its (hinted) glyph cache; larger glyphs are drawn as plain
/// paths.
pub const MAX_HINTED_PPEM: f32 = 256.0;

/// Outline cache keyed by face file and glyph id (and, for hinted outlines, the pixel size in 1/64 px).
#[derive(Default)]
pub struct GlyphCache {
    outlines: HashMap<(Arc<str>, u32), Option<Outline>>,
    hinted: HashMap<(Arc<str>, u32, u32), Option<Outline>>,
    hinters: HashMap<(Arc<str>, u32), Option<Arc<HintingInstance>>>,
    caps: HashMap<Arc<str>, Option<FaceCaps>>,
    files: HashMap<String, Arc<str>>,
}

impl std::fmt::Debug for GlyphCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphCache").field("outlines", &self.outlines.len()).finish()
    }
}

impl GlyphCache {
    fn key(&mut self, file: &str) -> Arc<str> {
        if let Some(k) = self.files.get(file) {
            return k.clone();
        }
        let k: Arc<str> = Arc::from(file);
        self.files.insert(file.to_owned(), k.clone());
        k
    }

    /// Units per em and colour capabilities of a face.
    pub fn caps(&mut self, file: &str, data: &[u8]) -> Option<FaceCaps> {
        let k = self.key(file);
        if let Some(c) = self.caps.get(&k) {
            return *c;
        }
        let c = FontRef::from_index(data, 0).ok().map(|f| {
            let upem = f.head().map(|h| f32::from(h.units_per_em())).unwrap_or(1000.0).max(1.0);
            let colr = f.colr().is_ok();
            let bitmaps = !skrifa::bitmap::BitmapStrikes::new(&f).is_empty();
            FaceCaps { upem, colr, bitmaps }
        });
        self.caps.insert(k, c);
        c
    }

    /// The outline of a glyph hinted at `ppem` pixels per em the way CanvasKit's FreeType scaler did with
    /// SkParagraph's `kSlight` hinting (FreeType's light autohinter: vertical-only snapping), in pixels, y down.
    pub fn hinted_outline(&mut self, file: &str, data: &[u8], gid: u32, ppem: f32) -> Option<Outline> {
        let k = self.key(file);
        let q = (ppem * 64.0).round().max(1.0) as u32;
        if let Some(o) = self.hinted.get(&(k.clone(), gid, q)) {
            return o.clone();
        }
        let font = FontRef::from_index(data, 0).ok();
        let hinter = match self.hinters.get(&(k.clone(), q)) {
            Some(h) => h.clone(),
            None => {
                let h = font.as_ref().and_then(|f| {
                    let opts = HintingOptions {
                        engine: Engine::Auto(None),
                        target: Target::Smooth { mode: SmoothMode::Light, symmetric_rendering: true, preserve_linear_metrics: true },
                    };
                    HintingInstance::new(&f.outline_glyphs(), Size::new(q as f32 / 64.0), LocationRef::default(), opts).ok().map(Arc::new)
                });
                if self.hinters.len() > 256 {
                    self.hinters.clear();
                }
                self.hinters.insert((k.clone(), q), h.clone());
                h
            }
        };
        let o = match (font, hinter) {
            (Some(f), Some(h)) => f.outline_glyphs().get(GlyphId::new(gid)).and_then(|glyph| {
                let mut pen = Pen(PathData::new());
                glyph.draw(DrawSettings::hinted(&h, false), &mut pen).ok()?;
                let skia = pen.0.to_skia().map(Arc::new);
                Some(Outline { data: Arc::new(pen.0), skia })
            }),
            _ => None,
        };
        if self.hinted.len() > 50_000 {
            self.hinted.clear();
        }
        self.hinted.insert((k, gid, q), o.clone());
        o
    }

    /// The outline of a glyph in font units (y down).
    pub fn outline(&mut self, file: &str, data: &[u8], gid: u32) -> Option<Outline> {
        let k = self.key(file);
        if let Some(o) = self.outlines.get(&(k.clone(), gid)) {
            return o.clone();
        }
        let o = FontRef::from_index(data, 0).ok().and_then(|f| {
            let glyph = f.outline_glyphs().get(GlyphId::new(gid))?;
            let mut pen = Pen(PathData::new());
            glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
            let skia = pen.0.to_skia().map(Arc::new);
            Some(Outline { data: Arc::new(pen.0), skia })
        });
        if self.outlines.len() > 50_000 {
            self.outlines.clear();
        }
        self.outlines.insert((k, gid), o.clone());
        o
    }
}
