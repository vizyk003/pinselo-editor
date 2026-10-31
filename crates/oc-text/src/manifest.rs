// SPDX-License-Identifier: Apache-2.0
//! Font manifest types (`packages/text/src/fonts.ts`: `FontFace`, `FontFamily`, `FontManifest`). The JSON shape
//! is `fixtures/fonts/manifest.json` and the merged Google catalogue (`oc-gfonts`), unchanged.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontFace {
    pub weight: u16,
    pub italic: bool,
    /// Manifest-relative path (`inter/Inter_400Regular.ttf`, `gf/<slug>/<w>[i].ttf`, `custom/<id>.ttf`).
    pub file: String,
    /// Hex SHA-256 of the file (empty for Google faces that are not pinned yet).
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontFamily {
    pub family: String,
    pub slug: String,
    pub category: String,
    pub license: String,
    /// `"google"`: fetched on demand from the Google Fonts catalogue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub faces: Vec<FontFace>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontManifest {
    pub version: serde_json::Number,
    pub families: Vec<FontFamily>,
}
