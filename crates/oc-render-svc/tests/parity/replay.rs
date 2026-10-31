// SPDX-License-Identifier: Apache-2.0
//! A [`Renderer`] that answers with what the TS renderer answered in the golden run (SVG heads, font picks, path
//! commands, decode outcomes) and blank rasters, so the writers' own output can be compared exactly.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use oc_gfonts::manifest::{FontFamily, FontManifest};
use oc_render_svc::renderer::*;
use oc_schema::{DesignDoc, TextBody};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::common::asset;

/// What the replay's `encode_png` / `transcode_png` return (pictures the painter would make).
pub const ENCODED: &[u8] = b"\x89PNG replay: encoded raster";
pub const TRANSCODED: &[u8] = b"\x89PNG replay: transcoded image";

pub struct TestHost {
    manifest: FontManifest,
}

impl TestHost {
    pub fn new() -> TestHost {
        TestHost {
            manifest: FontManifest {
                version: 1.into(),
                families: vec![],
            },
        }
    }
}

impl RenderHost for TestHost {
    fn asset_bytes<'a>(&'a self, key: &'a str) -> RenderFuture<'a, Vec<u8>> {
        Box::pin(async move { Err(RenderError::Failed(format!("no asset {key} in tests"))) })
    }
    fn font_file<'a>(&'a self, file: &'a str) -> RenderFuture<'a, Vec<u8>> {
        Box::pin(async move { Err(RenderError::Failed(format!("no font {file} in tests"))) })
    }
    fn custom_families<'a>(&'a self, _: &'a [String]) -> RenderFuture<'a, Vec<FontFamily>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn manifest(&self) -> &FontManifest {
        &self.manifest
    }
    fn fonts_dir(&self) -> PathBuf {
        PathBuf::from("fixtures/fonts")
    }
}

#[derive(Default)]
pub struct Replay {
    /// Page SVG heads by page id.
    pub heads: HashMap<String, String>,
    /// Text faces by page id.
    pub faces: HashMap<String, Vec<FaceRequest>>,
    /// `family|weight|italic` → picked face.
    pub picks: HashMap<String, PickedFace>,
    /// SVG path data → CanvasKit `toCmds()` (or `None` for unparsable data).
    pub paths: HashMap<String, Option<Vec<f64>>>,
    /// sha256 of bytes → whether CanvasKit could decode them.
    pub decoded: HashMap<String, bool>,
    /// `renderRgba` calls made.
    pub raster_calls: Mutex<Vec<Value>>,
}

fn ok<'a, T: Send + 'a>(v: T) -> RenderFuture<'a, T> {
    Box::pin(async move { Ok(v) })
}

impl Renderer for Replay {
    fn engine(&self) -> &'static str {
        "replay"
    }
    fn render_rgba<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
        opts: &'a RasterOptions,
    ) -> RenderFuture<'a, Rgba> {
        let mut skip = opts.skip.clone();
        skip.sort();
        self.raster_calls.lock().unwrap().push(json!({ "page": page_id, "scale": opts.scale, "transparent": opts.transparent, "skip": skip }));
        let w = (oc_schema::js::round(doc.meta.dims.w * opts.scale)).max(1.0) as u32;
        let h = (oc_schema::js::round(doc.meta.dims.h * opts.scale)).max(1.0) as u32;
        ok(Rgba::blank(w, h))
    }
    fn render_image<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
        _: &'a RasterOptions,
        _: ImageFormat,
    ) -> RenderFuture<'a, Vec<u8>> {
        ok(asset("tile.png"))
    }
    fn compose_transition<'a>(
        &'a self,
        _: &'a Rgba,
        next: &'a Rgba,
        _: &'a TransitionSpec,
    ) -> RenderFuture<'a, Rgba> {
        ok(next.clone())
    }
    fn vector_pdf<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a [String],
        _: &'a VectorPdfOptions,
    ) -> RenderFuture<'a, Vec<u8>> {
        ok(b"%PDF-replay".to_vec())
    }
    fn raster_pdf<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a [String],
        _: &'a RasterPdfOptions,
    ) -> RenderFuture<'a, Vec<u8>> {
        ok(b"%PDF-replay".to_vec())
    }
    fn page_svg<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        page_id: &'a str,
        _: &'a SvgOptions,
    ) -> RenderFuture<'a, String> {
        let head = self
            .heads
            .get(page_id)
            .unwrap_or_else(|| panic!("no recorded svg head for page {page_id}"));
        ok(format!("{head}<g id=\"x\"></g></svg>"))
    }
    fn elements_svg<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
        _: &'a BTreeSet<String>,
        _: &'a SvgOptions,
    ) -> RenderFuture<'a, String> {
        ok(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"><g id=\"y\"></g></svg>"
                .to_string(),
        )
    }
    fn text_faces<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        page_id: &'a str,
    ) -> RenderFuture<'a, Vec<FaceRequest>> {
        ok(self.faces.get(page_id).cloned().unwrap_or_default())
    }
    fn pick_face<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        r: &'a FaceRequest,
    ) -> RenderFuture<'a, PickedFace> {
        let key = format!(
            "{}|{}|{}",
            r.family,
            oc_schema::js::num_to_string(r.weight),
            r.italic
        );
        // Faces the TS never asked for are never printed either.
        ok(self.picks.get(&key).cloned().unwrap_or(PickedFace {
            family: r.family.clone(),
            weight: r.weight,
            italic: r.italic,
            file: String::new(),
        }))
    }
    fn layout_text<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a TextBody,
        _: f64,
    ) -> RenderFuture<'a, TextMeasure> {
        Box::pin(async { Err(RenderError::NotImplemented("text layout")) })
    }
    fn encode_png<'a>(&'a self, _: &'a Rgba) -> RenderFuture<'a, Vec<u8>> {
        ok(ENCODED.to_vec())
    }
    fn transcode_png<'a>(&'a self, bytes: &'a [u8]) -> RenderFuture<'a, Option<DecodedImage>> {
        let sha = hex::encode(Sha256::digest(bytes));
        let ok_ = *self
            .decoded
            .get(&sha)
            .unwrap_or_else(|| panic!("TS never decoded {sha}"));
        ok(ok_.then(|| DecodedImage {
            png: TRANSCODED.to_vec(),
            width: 1,
            height: 1,
        }))
    }
    fn rasterize_svg<'a>(&'a self, _: &'a str, _: u32) -> RenderFuture<'a, Vec<u8>> {
        ok(asset("tile.png"))
    }
    fn path_commands(&self, d: &str) -> RenderResult<Option<Vec<PathCmd>>> {
        let c = self
            .paths
            .get(d)
            .unwrap_or_else(|| panic!("TS never parsed path {d:?}"));
        Ok(c.as_ref().map(|c| PathCmd::from_flat(c)))
    }
}
