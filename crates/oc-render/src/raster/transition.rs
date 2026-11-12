// SPDX-License-Identifier: Apache-2.0
//! Page transitions (`packages/render-canvaskit/src/transition.ts`): draws one frame of a
//! [`TransitionFrame`](crate::motion::TransitionFrame) — each side's offset, scale, flip, rotation, clip, blur and
//! alpha — around the caller's page drawing. Shared by Present mode and video export.

use tiny_skia::{PathBuilder, Rect};

use super::canvas::{Canvas, Filter, LayerPaint};
use crate::motion::{TransitionClip, TransitionFrame, TransitionLayer};

/// Which page a transition layer shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Prev,
    Next,
}

/// Draw a transition frame on a `w × h` page. `draw_page` draws a page's content in page coordinates
/// (0..w × 0..h) onto the canvas it is given.
pub fn draw_transition(c: &mut Canvas, w: f64, h: f64, f: &TransitionFrame, mut draw_page: impl FnMut(Side, &mut Canvas)) {
    let mut layer = |side: Side, l: Option<&TransitionLayer>, c: &mut Canvas| {
        let Some(l) = l else { return };
        if l.alpha <= 0.001 {
            return;
        }
        c.save();
        c.translate((l.dx * w + w / 2.0) as f32, (l.dy * h + h / 2.0) as f32);
        if l.rot != 0.0 {
            c.rotate(l.rot as f32);
        }
        c.scale((l.scale * l.sx) as f32, l.scale as f32);
        c.translate((-w / 2.0) as f32, (-h / 2.0) as f32);
        match &l.clip {
            Some(TransitionClip::Rect { x, y, w: cw, h: ch }) => {
                if let Some(r) = Rect::from_xywh((x * w) as f32, (y * h) as f32, (cw * w) as f32, (ch * h) as f32) {
                    c.clip_rect(r);
                } else {
                    c.clip_empty();
                }
            }
            Some(TransitionClip::Circle { r }) => match PathBuilder::from_circle((w / 2.0) as f32, (h / 2.0) as f32, (r * w.hypot(h)) as f32) {
                Some(p) => c.clip_path(&p, false),
                None => c.clip_empty(),
            },
            None => {}
        }
        // Blur is a fraction of the shorter side; below a tenth of a page pixel it is not worth a layer.
        let blur = l.blur * w.min(h);
        let layered = l.alpha < 1.0 || blur > 0.1;
        if layered {
            let m = c.transform();
            let (sx, sy) = super::axis_scales(&m);
            // Clamped at the clip: a blurred page keeps its colour up to the frame's edge instead of fading out there.
            let filters = if blur > 0.1 { vec![Filter::BlurClamp { sx: blur as f32 * sx, sy: blur as f32 * sy, bounds: c.clip_bounds() }] } else { Vec::new() };
            c.save_layer(LayerPaint { opacity: l.alpha.clamp(0.0, 1.0) as f32, filters, ..Default::default() });
        }
        draw_page(side, c);
        if layered {
            c.restore();
        }
        c.restore();
    };
    if f.next_first == Some(true) {
        layer(Side::Next, Some(&f.next), c);
        layer(Side::Prev, f.prev.as_ref(), c);
    } else {
        layer(Side::Prev, f.prev.as_ref(), c);
        layer(Side::Next, Some(&f.next), c);
    }
}
