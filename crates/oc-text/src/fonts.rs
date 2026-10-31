// SPDX-License-Identifier: Apache-2.0
//! Font registry (`packages/text/src/fonts.ts`). Only pinned font files are used — no system fonts — and the
//! browser (WASM) and the server (native) load the same bytes, so layout is identical everywhere (ADR-0043).
//!
//! The registry never does I/O itself: callers hand it bytes ([`FontRegistry::insert_face`]). Which bytes are
//! still needed comes from [`FontRegistry::missing`]. Filesystem loading is in [`crate::native`] behind the
//! `native` feature.
//!
//! Every loaded face is registered in the parley/fontique collection under a private, unique family name, and
//! layout asks for exactly the face [`FontRegistry::pick_face`] chose. Face matching is therefore the TS
//! `pickFace` rule everywhere (fontique's own CSS matching never runs) and does not depend on which other faces
//! happen to be loaded.

use std::collections::HashMap;
use std::sync::Arc;

use parley::FontContext;
use parley::fontique::{Blob, Collection, CollectionOptions, FontInfoOverride, FontStyle, FontWeight, FontWidth, SourceCache};
use sha2::{Digest, Sha256};

use crate::inspect::{FontInfo, MAX_FONT_BYTES, inspect_font};
use crate::manifest::{FontFace, FontFamily, FontManifest};

/// Family used when a requested family is unknown, and the second family of every font stack.
pub const FALLBACK_FAMILY: &str = "Inter";

/// One `(family, weight, italic)` request, as in `ensure(family, weight, italic)`.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceRequest {
    pub family: String,
    pub weight: f64,
    pub italic: bool,
}

impl FaceRequest {
    pub fn new(family: impl Into<String>, weight: f64, italic: bool) -> Self {
        FaceRequest { family: family.into(), weight, italic }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("unknown font file {0}")]
    UnknownFile(String),
    #[error("font file {file} does not match its manifest hash")]
    HashMismatch { file: String },
    #[error("font file {file} is too large")]
    TooLarge { file: String },
    #[error("font file {file}: {source}")]
    Invalid { file: String, source: crate::inspect::InspectError },
    #[cfg(feature = "native")]
    #[error("reading font file {file}: {source}")]
    Io { file: String, source: std::io::Error },
    #[cfg(feature = "native")]
    #[error("reading manifest: {0}")]
    Manifest(String),
}

/// A face whose bytes are loaded.
#[derive(Clone)]
pub struct LoadedFace {
    pub file: String,
    pub info: FontInfo,
    /// Private fontique family name the face is registered under.
    pub(crate) key: String,
    pub(crate) blob: Blob<u8>,
    pub(crate) weight: FontWeight,
    pub(crate) style: FontStyle,
    pub(crate) width: FontWidth,
    /// Sorted code points the `cmap` maps to a real glyph.
    coverage: Arc<Vec<u32>>,
}

impl std::fmt::Debug for LoadedFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedFace").field("file", &self.file).field("info", &self.info).finish_non_exhaustive()
    }
}

impl LoadedFace {
    /// The raw font file bytes (for renderers that rasterise glyphs).
    pub fn data(&self) -> &[u8] {
        self.blob.data()
    }

    /// Whether the face has a glyph for `c`.
    pub fn covers(&self, c: char) -> bool {
        self.coverage.binary_search(&(c as u32)).is_ok()
    }
}

pub struct FontRegistry {
    manifest: FontManifest,
    by_family: HashMap<String, usize>,
    loaded: HashMap<String, LoadedFace>,
    by_blob: HashMap<u64, String>,
    generation: u64,
    pub(crate) fcx: FontContext,
}

impl std::fmt::Debug for FontRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontRegistry")
            .field("families", &self.manifest.families.len())
            .field("loaded", &self.loaded.len())
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl FontRegistry {
    pub fn new(manifest: FontManifest) -> Self {
        let by_family = manifest.families.iter().enumerate().map(|(i, f)| (f.family.clone(), i)).collect();
        let collection = Collection::new(CollectionOptions { shared: false, system_fonts: false });
        FontRegistry {
            manifest,
            by_family,
            loaded: HashMap::new(),
            by_blob: HashMap::new(),
            generation: 0,
            fcx: FontContext { collection, source_cache: SourceCache::default() },
        }
    }

    pub fn manifest(&self) -> &FontManifest {
        &self.manifest
    }

    pub fn families(&self) -> &[FontFamily] {
        &self.manifest.families
    }

    pub fn has(&self, family: &str) -> bool {
        self.by_family.contains_key(family)
    }

    /// Bumped whenever a face finishes loading; layout caches key on it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn family_or_fallback(&self, family: &str) -> &FontFamily {
        let i = self.by_family.get(family).or_else(|| self.by_family.get(FALLBACK_FAMILY)).copied().unwrap_or(0);
        &self.manifest.families[i]
    }

    /// Closest available face: exact italic match preferred, then nearest weight (first wins on ties), like
    /// `pickFace`. Unknown families resolve to Inter (or the first family). `None` only for an empty manifest.
    pub fn pick_face(&self, family: &str, weight: f64, italic: bool) -> Option<(&FontFamily, &FontFace)> {
        if self.manifest.families.is_empty() {
            return None;
        }
        let fam = self.family_or_fallback(family);
        let has_slant = fam.faces.iter().any(|f| f.italic == italic);
        let mut best: Option<&FontFace> = None;
        for f in fam.faces.iter().filter(|f| !has_slant || f.italic == italic) {
            match best {
                Some(b) if (f64::from(f.weight) - weight).abs() >= (f64::from(b.weight) - weight).abs() => {}
                _ => best = Some(f),
            }
        }
        best.map(|face| (fam, face))
    }

    /// Family name actually used for layout (falls back when the family is unknown).
    pub fn resolve_family<'a>(&self, family: &'a str) -> &'a str {
        if self.has(family) { family } else { FALLBACK_FAMILY }
    }

    pub fn is_loaded(&self, family: &str, weight: f64, italic: bool) -> bool {
        self.pick_face(family, weight, italic).is_some_and(|(_, f)| self.loaded.contains_key(&f.file))
    }

    /// The loaded face used for `(family, weight, italic)`, if its bytes are in.
    pub fn loaded_face(&self, family: &str, weight: f64, italic: bool) -> Option<&LoadedFace> {
        self.pick_face(family, weight, italic).and_then(|(_, f)| self.loaded.get(&f.file))
    }

    pub fn face_by_file(&self, file: &str) -> Option<&LoadedFace> {
        self.loaded.get(file)
    }

    pub(crate) fn face_by_blob(&self, blob_id: u64) -> Option<&LoadedFace> {
        self.by_blob.get(&blob_id).and_then(|f| self.loaded.get(f))
    }

    /// Manifest face whose pinned hash is `sha256` (content-addressed lookup).
    pub fn face_by_sha256(&self, sha256: &str) -> Option<&FontFace> {
        self.manifest.families.iter().flat_map(|f| &f.faces).find(|f| !f.sha256.is_empty() && f.sha256.eq_ignore_ascii_case(sha256))
    }

    fn manifest_face(&self, file: &str) -> Option<&FontFace> {
        self.manifest.families.iter().flat_map(|f| &f.faces).find(|f| f.file == file)
    }

    /// Files (deduplicated, in request order) that must be loaded before `requests` can be laid out.
    /// This is `ensure`/`ensureAll` without the I/O: fetch each file and pass it to [`Self::insert_face`].
    pub fn missing<'a>(&self, requests: impl IntoIterator<Item = &'a FaceRequest>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in requests {
            if let Some((_, f)) = self.pick_face(&r.family, r.weight, r.italic)
                && !self.loaded.contains_key(&f.file)
                && !out.contains(&f.file)
            {
                out.push(f.file.clone());
            }
        }
        out
    }

    /// Register the bytes of a manifest face. When the manifest pins a SHA-256 for the file, the bytes must
    /// match it. Loading an already-loaded file is a no-op.
    pub fn insert_face(&mut self, file: &str, bytes: Vec<u8>) -> Result<(), FontError> {
        if self.loaded.contains_key(file) {
            return Ok(());
        }
        let face = self.manifest_face(file).ok_or_else(|| FontError::UnknownFile(file.to_owned()))?;
        if !face.sha256.is_empty() {
            let digest = Sha256::digest(&bytes);
            if !hex::encode(digest).eq_ignore_ascii_case(&face.sha256) {
                return Err(FontError::HashMismatch { file: file.to_owned() });
            }
        }
        self.insert_unchecked(file, bytes)
    }

    /// Register bytes without the hash check (custom uploads whose hash is the storage key, tests).
    pub fn insert_unchecked(&mut self, file: &str, bytes: Vec<u8>) -> Result<(), FontError> {
        if self.loaded.contains_key(file) {
            return Ok(());
        }
        if bytes.len() > MAX_FONT_BYTES {
            return Err(FontError::TooLarge { file: file.to_owned() });
        }
        let info = inspect_font(&bytes).map_err(|source| FontError::Invalid { file: file.to_owned(), source })?;
        let coverage = {
            use skrifa::MetadataProvider;
            let font = skrifa::FontRef::from_index(&bytes, 0).map_err(|e| FontError::Invalid { file: file.to_owned(), source: crate::inspect::InspectError::Parse(e.to_string()) })?;
            let mut cps: Vec<u32> = font.charmap().mappings().filter(|(_, g)| g.to_u32() != 0).map(|(c, _)| c).collect();
            cps.sort_unstable();
            cps.dedup();
            Arc::new(cps)
        };
        let key = format!("oc-face:{file}");
        let blob = Blob::new(Arc::new(bytes));
        let over = FontInfoOverride { family_name: Some(&key), ..Default::default() };
        let registered = self.fcx.collection.register_fonts(blob.clone(), Some(over));
        let (weight, style, width) = registered
            .first()
            .and_then(|(_, fonts)| fonts.first())
            .map(|f| (f.weight(), f.style(), f.width()))
            .ok_or_else(|| FontError::Invalid { file: file.to_owned(), source: crate::inspect::InspectError::Parse("not registered".into()) })?;
        self.by_blob.insert(blob.id(), file.to_owned());
        self.loaded.insert(file.to_owned(), LoadedFace { file: file.to_owned(), info, key, blob, weight, style, width, coverage });
        self.generation += 1;
        Ok(())
    }

    /// Add a family at runtime (uploaded brand fonts, FT-020); an existing family with the same name is replaced.
    pub fn add_family(&mut self, f: FontFamily) {
        if let Some(&i) = self.by_family.get(&f.family) {
            self.manifest.families[i] = f;
        } else {
            self.by_family.insert(f.family.clone(), self.manifest.families.len());
            self.manifest.families.push(f);
        }
    }

    /// Number of loaded faces.
    pub fn loaded_count(&self) -> usize {
        self.loaded.len()
    }
}
