// SPDX-License-Identifier: Apache-2.0
//! Filesystem loading for servers (feature `native`): fonts from a pinned fonts directory
//! (`<dir>/manifest.json` + `<dir>/files/…`, as oc-text's `native` loader) and assets from a directory of files
//! named by asset key.

use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

use oc_text::{FontError, FontRegistry, TextEngine};

use crate::assets::AssetProvider;
use crate::displaylist::DisplayList;
use crate::raster::Renderer;

impl Renderer {
    /// A renderer over `<fonts_dir>/manifest.json` with Inter 400 (the fallback face) loaded.
    pub fn from_fonts_dir(fonts_dir: &Path) -> Result<Renderer, FontError> {
        let mut fonts = FontRegistry::from_dir(fonts_dir)?;
        let inter = oc_text::FaceRequest::new(oc_text::FALLBACK_FAMILY, 400.0, false);
        if let Some(e) = fonts.ensure_from_dir(fonts_dir, [&inter]).into_iter().next() {
            return Err(e);
        }
        Ok(Renderer::new(TextEngine::new(fonts)))
    }

    /// Load every font file a display list needs from `<fonts_dir>/files`. Files that are not on disk are
    /// reported, not fatal (text then falls back like the editor's does while fonts load).
    pub fn load_fonts_from_dir(&mut self, fonts_dir: &Path, dl: &DisplayList) -> Vec<FontError> {
        let mut errors = Vec::new();
        for file in self.missing_fonts(dl) {
            if let Err(e) = self.text.fonts.load_file(fonts_dir, &file) {
                errors.push(e);
            }
        }
        errors
    }
}

/// Assets read from `<dir>/<key>` (keys are relative paths such as `abc123` or `abc123/poster`; keys that would
/// leave the directory are refused).
#[derive(Debug, Clone)]
pub struct DirAssets {
    pub dir: PathBuf,
}

impl DirAssets {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        DirAssets { dir: dir.into() }
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        let rel = Path::new(key);
        if key.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return None;
        }
        Some(self.dir.join(rel))
    }
}

impl AssetProvider for DirAssets {
    fn asset(&self, key: &str) -> Option<Cow<'_, [u8]>> {
        std::fs::read(self.path(key)?).ok().map(Cow::Owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_escaping_keys() {
        let a = DirAssets::new("/tmp/x");
        assert!(a.path("../etc/passwd").is_none());
        assert!(a.path("/etc/passwd").is_none());
        assert!(a.path("").is_none());
        assert_eq!(a.path("abc/poster"), Some(PathBuf::from("/tmp/x/abc/poster")));
    }
}
