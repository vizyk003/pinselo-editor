// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use oc_schema::text::TextBody;
use oc_text::{FontManifest, FontRegistry, TextEngine, required_faces};

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

pub fn fonts_dir() -> PathBuf {
    repo_root().join("fixtures/fonts")
}

pub fn golden(name: &str) -> serde_json::Value {
    let p = repo_root().join("tests/golden/text").join(name);
    serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}

pub fn manifest() -> FontManifest {
    serde_json::from_slice(&std::fs::read(fonts_dir().join("manifest.json")).unwrap()).unwrap()
}

/// Load every face a body needs from fixtures/fonts/files (hash-checked).
pub fn ensure(engine: &mut TextEngine, body: &TextBody) {
    let reqs = required_faces(&engine.fonts, body);
    for file in engine.fonts.missing(reqs.iter()) {
        let bytes = std::fs::read(fonts_dir().join("files").join(&file)).unwrap();
        engine.fonts.insert_face(&file, bytes).unwrap();
    }
}

pub fn engine() -> TextEngine {
    let mut e = TextEngine::new(FontRegistry::new(manifest()));
    let inter = oc_text::FaceRequest::new("Inter", 400.0, false);
    for file in e.fonts.missing([&inter]) {
        let bytes = std::fs::read(fonts_dir().join("files").join(&file)).unwrap();
        e.fonts.insert_face(&file, bytes).unwrap();
    }
    e
}
