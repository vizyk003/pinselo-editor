// SPDX-License-Identifier: Apache-2.0
//! Line-end markers as SVG path data in marker space: the line end sits at (0, 0) and the line arrives from −x.
//! Shared by the canvas painter, SVG and PDF exporters so every surface draws identical ends.

use oc_schema::elements::LineMarker;
use oc_schema::primitives::Vec2;
use serde::{Deserialize, Serialize};

use crate::jsmath::{f3, hypot, max, min};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkerMode {
    /// Paints the area.
    Fill,
    /// Strokes the outline with the line's width (round joins).
    Stroke,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkerShape {
    pub d: String,
    pub mode: MarkerMode,
    /// How far to pull the line back from its end so it doesn't show through hollow markers.
    pub inset: f64,
}

fn circle(r: f64) -> String {
    format!("M{r} 0 A{r} {r} 0 1 1 {n} 0 A{r} {r} 0 1 1 {r} 0 Z", r = f3(r), n = f3(-r))
}

fn square(k: f64) -> String {
    format!("M{n} {n} L{p} {n} L{p} {p} L{n} {p} Z", n = f3(-k), p = f3(k))
}

fn diamond(k: f64) -> String {
    format!("M{p} 0 L0 {p} L{n} 0 L0 {n} Z", n = f3(-k), p = f3(k))
}

/// Marker outline for a line of the given stroke width; `None` for `none` (and unknown markers).
pub fn line_marker_shape(kind: &LineMarker, width: f64) -> Option<MarkerShape> {
    use MarkerMode::{Fill, Stroke};
    let s = max(8.0, width * 3.0);
    let o = s * 1.15; // hollow markers are a touch larger so the hole stays readable
    let shape = |d: String, mode: MarkerMode, inset: f64| Some(MarkerShape { d, mode, inset });
    match kind {
        LineMarker::Arrow => shape(format!("M{} {} L0 0 L{} {}", f3(-s), f3(-s * 0.6), f3(-s), f3(s * 0.6)), Stroke, 0.0),
        LineMarker::Triangle => shape(format!("M{} {} L0 0 L{} {} Z", f3(-s), f3(-s * 0.6), f3(-s), f3(s * 0.6)), Fill, s * 0.5),
        LineMarker::Stealth => shape(
            format!("M{} {} L0 0 L{} {} L{} 0 Z", f3(-s * 1.1), f3(-s * 0.65), f3(-s * 1.1), f3(s * 0.65), f3(-s * 0.75)),
            Fill,
            s * 0.6,
        ),
        LineMarker::Circle => shape(circle(s * 0.45), Fill, 0.0),
        LineMarker::Square => shape(square(s * 0.45), Fill, 0.0),
        LineMarker::Diamond => shape(diamond(s * 0.6), Fill, 0.0),
        LineMarker::Bar => shape(format!("M0 {} L0 {}", f3(-s * 0.6), f3(s * 0.6)), Stroke, 0.0),
        LineMarker::TriangleOutline => {
            // Inset by half the stroke so the outline's tip lands on the line end.
            let t = width / 2.0;
            shape(
                format!("M{} {} L{} 0 L{} {} Z", f3(-o - t), f3(-o * 0.6), f3(-t), f3(-o - t), f3(o * 0.6)),
                Stroke,
                o + t,
            )
        }
        LineMarker::CircleOutline => shape(circle(o * 0.42), Stroke, o * 0.42),
        LineMarker::SquareOutline => shape(square(o * 0.42), Stroke, o * 0.42),
        LineMarker::DiamondOutline => shape(diamond(o * 0.58), Stroke, o * 0.58),
        LineMarker::None | LineMarker::Other(_) => None,
    }
}

/// The polyline with each end pulled back by its marker's inset (never past the neighbouring point).
pub fn trim_line_for_markers(points: &[Vec2], start: &LineMarker, end: &LineMarker, width: f64) -> Vec<Vec2> {
    let mut pts = points.to_vec();
    if pts.len() < 2 {
        return pts;
    }
    let last = pts.len() - 1;
    let mut pull = |i: usize, j: usize, by: f64| {
        if by <= 0.0 {
            return;
        }
        let [ax, ay] = pts[i];
        let [bx, by_] = pts[j];
        let len = hypot(bx - ax, by_ - ay);
        if len < 1e-6 {
            return;
        }
        let k = min(by, len * 0.9) / len;
        pts[i] = [ax + (bx - ax) * k, ay + (by_ - ay) * k];
    };
    let inset = |m: &LineMarker| line_marker_shape(m, width).map_or(0.0, |s| s.inset);
    pull(0, 1, inset(start));
    pull(last, last - 1, inset(end));
    pts
}
