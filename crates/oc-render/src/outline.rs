// SPDX-License-Identifier: Apache-2.0
//! Outline text (DS-100): a text element's glyphs, list markers and underline/strike bars as one path in the
//! element's own pixel space, laid out exactly as the painter draws it (flat: curved text is outlined straight).

use oc_schema::elements::{AutoFit, CellVAlign, TextEl, TextFrameMode};
use oc_schema::primitives::Paint;
use oc_text::LaidParagraph;

use crate::Renderer;
use crate::path::PathData;

impl Renderer {
    /// The outline of a text element (`d` in its box, nonzero fill) and the paint of its first run. `None` when
    /// it draws no glyph outlines (empty, or fonts not loaded).
    pub fn text_outline(&mut self, t: &TextEl) -> Option<(String, Paint)> {
        let key = t.base.id.as_str();
        let (w, h) = (t.base.xf.w, t.base.xf.h);
        let fit = t.auto_fit == AutoFit::Shrink && t.frame.mode == TextFrameMode::Fixed;
        let effect = t.effect.as_ref();
        let layout = if fit { self.text.layout_fit(key, &t.body, w, h, effect).0 } else { self.text.layout(key, &t.body, w, effect) };
        let dy = match (&t.valign, t.frame.mode == TextFrameMode::Fixed) {
            (Some(CellVAlign::Middle), true) => (h - layout.height) / 2.0,
            (Some(CellVAlign::Bottom), true) => h - layout.height,
            _ => 0.0,
        };
        let mut out = PathData::new();
        let mut fill: Option<Paint> = None;
        for p in &layout.paras {
            self.outline_paragraph(p, dy, &mut out, &mut fill);
            if let Some(b) = &p.bullet {
                self.outline_paragraph(b, dy, &mut out, &mut fill);
            }
        }
        if out.is_empty() {
            return None;
        }
        let d = out.to_svg(|v| {
            let r = (v * 1000.0).round() / 1000.0;
            oc_schema::js::num_to_string(if r == 0.0 { 0.0 } else { r })
        });
        Some((d, fill.unwrap_or_else(|| t.body.default_run.fill.clone())))
    }

    fn outline_paragraph(&mut self, p: &LaidParagraph, dy: f64, out: &mut PathData, fill: &mut Option<Paint>) {
        let (ox, oy) = (p.x, p.y + dy);
        for line in &p.lines {
            for run in &line.glyph_runs {
                let Some(face) = self.text.fonts.face_by_file(&run.face) else { continue };
                let data = face.data().to_vec();
                let metrics = face.info.metrics;
                let Some(caps) = self.glyphs.caps(&run.face, &data) else { continue };
                let st = p.styles.get(run.style);
                if fill.is_none() {
                    *fill = st.map(|s| s.fill.clone());
                }
                let k = run.size / f64::from(caps.upem);
                let skew = if run.synthetic_italic { -0.25 * k } else { 0.0 };
                for g in &run.glyphs {
                    let Some(o) = self.glyphs.outline(&run.face, &data, g.id) else { continue };
                    out.extend(&o.data.transformed(&[k, 0.0, skew, k, ox + g.x, oy + g.y]));
                }
                // Underline and strike-through bars, as the painter places them.
                let Some(st) = st else { continue };
                let baseline = oy + run.glyphs.first().map_or(line.top + line.baseline, |g| g.y);
                let mut bar = |offset: f64, thickness: f64| {
                    let th = if thickness > 0.0 { thickness * run.size } else { run.size / 14.0 };
                    let top = baseline - offset * run.size;
                    out.extend(&PathData::rect(ox + run.x, top, run.advance, th));
                };
                if st.underline == Some(true) {
                    bar(metrics.underline_offset, metrics.underline_thickness);
                }
                if st.strike == Some(true) {
                    bar(metrics.strikeout_offset, metrics.strikeout_thickness);
                }
            }
        }
    }
}
