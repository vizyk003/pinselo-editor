// SPDX-License-Identifier: Apache-2.0
//! Frame outlines (A13): library clip paths scale from the unit box; built-in clips use the shape generator.

use oc_schema::elements::{FrameClip, FrameEl, FrameOverlay};
use serde::Serialize;

use crate::shapes::{Params, scale_unit_path, shape_path};

/// Clip outline of a frame in its box space (0..w, 0..h), from its `clip` and optional library `clipPath`.
pub fn frame_outline(clip: &FrameClip, clip_path: Option<&str>, w: f64, h: f64) -> String {
    match clip_path {
        Some(p) if !p.is_empty() => scale_unit_path(p, w, h),
        _ => shape_path(clip.as_str(), w, h, &Params::new(), None),
    }
}

/// [`frame_outline`] of a frame element.
pub fn frame_el_outline(el: &FrameEl, w: f64, h: f64) -> String {
    frame_outline(&el.clip, el.clip_path.as_deref(), w, h)
}

/// An overlay with its path scaled to the frame box (`{ ...overlay, path }`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OverlayPath<'a> {
    #[serde(flatten)]
    pub overlay: &'a FrameOverlay,
    pub path: String,
}

/// Overlay chrome paths of a frame in its box space.
pub fn frame_overlays(overlay: Option<&[FrameOverlay]>, w: f64, h: f64) -> Vec<OverlayPath<'_>> {
    overlay.unwrap_or_default().iter().map(|o| OverlayPath { overlay: o, path: scale_unit_path(&o.d, w, h) }).collect()
}
