// SPDX-License-Identifier: Apache-2.0
//! Filesystem font loading (feature `native`): the server's equivalent of the TS `fontFileLoader` for pinned
//! files under `<fonts_dir>/files/`.

use std::path::Path;

use crate::fonts::{FaceRequest, FontError, FontRegistry};
use crate::manifest::FontManifest;

/// Read `<fonts_dir>/manifest.json`.
pub fn read_manifest(fonts_dir: &Path) -> Result<FontManifest, FontError> {
    let raw = std::fs::read(fonts_dir.join("manifest.json")).map_err(|e| FontError::Manifest(e.to_string()))?;
    serde_json::from_slice(&raw).map_err(|e| FontError::Manifest(e.to_string()))
}

impl FontRegistry {
    /// A registry over `<fonts_dir>/manifest.json` (no faces loaded yet).
    pub fn from_dir(fonts_dir: &Path) -> Result<Self, FontError> {
        Ok(FontRegistry::new(read_manifest(fonts_dir)?))
    }

    /// Load one manifest face from `<fonts_dir>/files/<file>` (hash-checked).
    pub fn load_file(&mut self, fonts_dir: &Path, file: &str) -> Result<(), FontError> {
        if self.face_by_file(file).is_some() {
            return Ok(());
        }
        let bytes = std::fs::read(fonts_dir.join("files").join(file)).map_err(|source| FontError::Io { file: file.to_owned(), source })?;
        self.insert_face(file, bytes)
    }

    /// `ensureAll` from disk: load every face the requests resolve to. Files that are not on disk (Google faces
    /// that live in the object store) are reported, not fatal.
    pub fn ensure_from_dir<'a>(&mut self, fonts_dir: &Path, requests: impl IntoIterator<Item = &'a FaceRequest>) -> Vec<FontError> {
        let files = self.missing(requests);
        files.into_iter().filter_map(|f| self.load_file(fonts_dir, &f).err()).collect()
    }
}
