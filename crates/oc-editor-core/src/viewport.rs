// SPDX-License-Identifier: Apache-2.0
//! Scene layout (pages stacked vertically) and the scene ⇄ screen mapping. Port of `viewport.ts`.

use oc_schema::primitives::{Rect, Vec2};
use oc_schema::tree::sorted_pages;
use oc_schema::{DesignDoc, Page};
use serde::{Deserialize, Serialize};

use crate::util::{max, min};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    /// Scene coordinate shown at the top-left of the canvas.
    pub x: f64,
    pub y: f64,
    /// Screen px per scene px.
    pub zoom: f64,
    /// Canvas size in CSS px.
    pub width: f64,
    pub height: f64,
}

impl Default for Viewport {
    /// The TS constructor default `{ x: 0, y: 0, zoom: 1, width: 800, height: 600 }`.
    fn default() -> Self {
        Viewport { x: 0.0, y: 0.0, zoom: 1.0, width: 800.0, height: 600.0 }
    }
}

pub const MIN_ZOOM: f64 = 0.1;
pub const MAX_ZOOM: f64 = 8.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageSlot {
    pub page: Page,
    pub index: usize,
    pub rect: Rect,
}

pub fn page_gap(doc: &DesignDoc) -> f64 {
    max(24.0, doc.meta.dims.h * 0.06)
}

/// Pages laid out top to bottom in scene space.
pub fn page_slots(doc: &DesignDoc) -> Vec<PageSlot> {
    let (w, h) = (doc.meta.dims.w, doc.meta.dims.h);
    let gap = page_gap(doc);
    sorted_pages(&doc.pages)
        .into_iter()
        .enumerate()
        .map(|(index, page)| PageSlot { page: page.clone(), index, rect: Rect::new(0.0, index as f64 * (h + gap), w, h) })
        .collect()
}

pub fn slot_of(doc: &DesignDoc, page_id: &str) -> Option<PageSlot> {
    page_slots(doc).into_iter().find(|s| s.page.id == page_id)
}

/// The page under a scene point (or the nearest one vertically).
pub fn page_at_scene(doc: &DesignDoc, p: Vec2) -> Option<PageSlot> {
    let mut best: Option<PageSlot> = None;
    let mut best_d = f64::INFINITY;
    for s in page_slots(doc) {
        let d = if p[1] < s.rect.y {
            s.rect.y - p[1]
        } else if p[1] > s.rect.y + s.rect.h {
            p[1] - (s.rect.y + s.rect.h)
        } else {
            0.0
        };
        if d < best_d {
            best_d = d;
            best = Some(s);
        }
    }
    best
}

pub fn scene_to_screen(vp: &Viewport, p: Vec2) -> Vec2 {
    [(p[0] - vp.x) * vp.zoom, (p[1] - vp.y) * vp.zoom]
}

pub fn screen_to_scene(vp: &Viewport, p: Vec2) -> Vec2 {
    [p[0] / vp.zoom + vp.x, p[1] / vp.zoom + vp.y]
}

pub fn clamp_zoom(z: f64) -> f64 {
    min(MAX_ZOOM, max(MIN_ZOOM, z))
}

/// Zoom keeping the scene point under `screen_pt` fixed.
pub fn zoom_at(vp: &Viewport, zoom: f64, screen_pt: Vec2) -> Viewport {
    let z = clamp_zoom(zoom);
    let [sx, sy] = screen_to_scene(vp, screen_pt);
    Viewport { zoom: z, x: sx - screen_pt[0] / z, y: sy - screen_pt[1] / z, ..*vp }
}

/// Fit a scene rect into the viewport with padding (screen px; TS default 48).
pub fn fit_rect(vp: &Viewport, r: &Rect, pad: f64) -> Viewport {
    let z = clamp_zoom(min((vp.width - pad * 2.0) / r.w, (vp.height - pad * 2.0) / r.h));
    Viewport { zoom: z, x: r.x + r.w / 2.0 - vp.width / 2.0 / z, y: r.y + r.h / 2.0 - vp.height / 2.0 / z, ..*vp }
}
