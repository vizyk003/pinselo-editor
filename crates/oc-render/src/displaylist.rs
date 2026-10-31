// SPDX-License-Identifier: Apache-2.0
//! Backend-neutral display list (`packages/render-core/src/displaylist.ts`, docs/plan/03 §5.3). The resolver
//! turns a page of the document into a flat list of drawing ops; the raster painter, the SVG writer and the PDF
//! writer all consume it.
//!
//! The types serialise to the same JSON as the TS objects (`op`/`kind` tags, camelCase fields, optional fields
//! omitted, `sourceTimeMs: null` kept), which is what the golden tests compare.

use indexmap::IndexSet;
use oc_schema::elements::{CellVAlign, Effect, ImageEditStack, LineMarker, MaskMode};
use oc_schema::primitives::BlendMode;
use oc_schema::primitives::{AssetId, ColorValue, ElementId, GradientStop, ImageFit, Mat2D, Rect, StoredRect, Stroke, Vec2};
use oc_schema::text::{TextBody, TextEffect};
use serde::{Serialize, Serializer};

/// A paint resolved against the box it fills (gradients and image fills need the box).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum ResolvedPaint {
    #[serde(rename = "solid")]
    Solid { color: ColorValue },
    #[serde(rename = "linear")]
    Linear {
        stops: Vec<GradientStop>,
        angle: f64,
        #[serde(rename = "box")]
        bbox: Rect,
    },
    #[serde(rename = "radial")]
    Radial {
        stops: Vec<GradientStop>,
        #[serde(rename = "box")]
        bbox: Rect,
    },
    #[serde(rename = "image")]
    Image {
        asset: AssetId,
        fit: ImageFit,
        #[serde(rename = "box")]
        bbox: Rect,
    },
}

/// A path's stroke: the authored stroke plus its resolved paint.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PathStroke {
    pub stroke: Stroke,
    pub paint: ResolvedPaint,
}

/// Shrink-to-fit marker of a text op (`fit: 'shrink'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TextFit {
    Shrink,
}

fn bits<S: Serializer>(m: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.collect_seq(m.iter())
}

/// One drawing op. `save`/`layer` push state that the matching `restore` pops; `clip` narrows the current state.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "op")]
pub enum DLItem {
    #[serde(rename = "save")]
    Save { matrix: Mat2D },
    #[serde(rename = "restore")]
    Restore,
    /// Opacity / blend / effects group (an offscreen layer).
    #[serde(rename = "layer", rename_all = "camelCase")]
    Layer { opacity: f64, blend: BlendMode, effects: Vec<Effect>, src_el: ElementId },
    #[serde(rename = "clip")]
    Clip { path: String },
    /// A masked group (DS-107): the items up to the matching `maskContent` draw the mask (never shown); the items
    /// after it, up to the `restore` that closes this op, draw the content seen through it.
    #[serde(rename = "mask", rename_all = "camelCase")]
    Mask { mode: MaskMode, src_el: ElementId },
    #[serde(rename = "maskContent")]
    MaskContent,
    #[serde(rename = "path", rename_all = "camelCase")]
    Path {
        path: String,
        fills: Vec<ResolvedPaint>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stroke: Option<PathStroke>,
        src_el: ElementId,
    },
    #[serde(rename = "line", rename_all = "camelCase")]
    Line { points: Vec<Vec2>, stroke: Stroke, paint: ResolvedPaint, start_marker: LineMarker, end_marker: LineMarker, src_el: ElementId },
    #[serde(rename = "image", rename_all = "camelCase")]
    Image {
        asset: AssetId,
        dst: Rect,
        crop: StoredRect,
        #[serde(skip_serializing_if = "Option::is_none")]
        edits: Option<ImageEditStack>,
        #[serde(skip_serializing_if = "Option::is_none")]
        clip: Option<String>,
        src_el: ElementId,
    },
    /// `curve` (-100..100) sets text on an arc; `fit: 'shrink'` scales type down until it fits the box height.
    #[serde(rename = "text", rename_all = "camelCase")]
    Text {
        key: String,
        body: TextBody,
        #[serde(rename = "box")]
        bbox: Rect,
        #[serde(skip_serializing_if = "Option::is_none")]
        effect: Option<TextEffect>,
        #[serde(skip_serializing_if = "Option::is_none")]
        valign: Option<CellVAlign>,
        #[serde(skip_serializing_if = "Option::is_none")]
        curve: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fit: Option<TextFit>,
        src_el: ElementId,
    },
    #[serde(rename = "qr", rename_all = "camelCase")]
    Qr {
        #[serde(serialize_with = "bits")]
        modules: Vec<u8>,
        size: usize,
        #[serde(rename = "box")]
        bbox: Rect,
        fg: ColorValue,
        bg: ColorValue,
        src_el: ElementId,
    },
    #[serde(rename = "placeholder", rename_all = "camelCase")]
    Placeholder {
        #[serde(rename = "box")]
        bbox: Rect,
        label: String,
        src_el: ElementId,
    },
    /// Video frame at `source_time_ms` (`None` = poster frame, used for static renders and thumbnails). `key`
    /// identifies the playback (frame source) when it differs from `src_el` — a grid holds one video per cell.
    #[serde(rename = "video", rename_all = "camelCase")]
    Video {
        asset: AssetId,
        dst: Rect,
        crop: StoredRect,
        source_time_ms: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        clip: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        src_el: ElementId,
        #[serde(skip_serializing_if = "Option::is_none")]
        edits: Option<ImageEditStack>,
    },
}

impl DLItem {
    /// The `op` tag.
    pub fn op(&self) -> &'static str {
        match self {
            DLItem::Save { .. } => "save",
            DLItem::Restore => "restore",
            DLItem::Layer { .. } => "layer",
            DLItem::Clip { .. } => "clip",
            DLItem::Mask { .. } => "mask",
            DLItem::MaskContent => "maskContent",
            DLItem::Path { .. } => "path",
            DLItem::Line { .. } => "line",
            DLItem::Image { .. } => "image",
            DLItem::Text { .. } => "text",
            DLItem::Qr { .. } => "qr",
            DLItem::Placeholder { .. } => "placeholder",
            DLItem::Video { .. } => "video",
        }
    }
}

/// A resolved page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DisplayList {
    pub width: f64,
    pub height: f64,
    pub background: Vec<ResolvedPaint>,
    pub items: Vec<DLItem>,
    /// Assets referenced, in first-use order (the export path preloads these).
    pub assets: IndexSet<AssetId>,
}

impl DisplayList {
    pub fn new(width: f64, height: f64) -> Self {
        DisplayList { width, height, background: Vec::new(), items: Vec::new(), assets: IndexSet::new() }
    }
}

/// Every DL item's op in order (`opsOf`), for tests and analysis.
pub fn ops_of(dl: &DisplayList) -> Vec<&'static str> {
    dl.items.iter().map(DLItem::op).collect()
}
