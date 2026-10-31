// SPDX-License-Identifier: Apache-2.0
//! Text on an arc (ED-041), `TextEngine.curved` in the TS: the body is laid out as one line (paragraphs joined
//! with spaces) and every glyph is placed on a circle so the arc's midpoint baseline sits at the origin; the
//! result is translated into a `width × height` box.

use std::f64::consts::PI;

use oc_schema::text::{Run, TextAlign, TextBody};

use crate::layout::{CurvedGlyph, CurvedLayout, CurvedRun, TextEngine, paint_to_color};

pub(crate) fn curved(engine: &mut TextEngine, body: &TextBody, curve: f64) -> Option<CurvedLayout> {
    let first = body.paragraphs.first()?;
    let mut runs: Vec<Run> = Vec::new();
    for (i, p) in body.paragraphs.iter().enumerate() {
        if i > 0 {
            runs.push(Run::plain(" "));
        }
        runs.extend(p.runs.iter().cloned());
    }
    runs.retain(|r| !r.text.is_empty());
    if runs.is_empty() {
        return None;
    }
    let mut para = first.clone();
    para.align = TextAlign::Start;
    para.list = None;
    para.runs = runs;
    let flat = TextBody { paragraphs: vec![para], default_run: body.default_run.clone(), extra: body.extra.clone() };
    let laid = engine.layout_uncached(&flat, 1e6, None);
    let p = laid.paras.first()?;
    let line = p.lines.first()?;

    struct G {
        id: u32,
        face: String,
        x: f64,
        adv: f64,
        size: f64,
        style: usize,
    }
    // Advances are `Font.getGlyphWidths` of a default (hinted, non-subpixel) SkFont: whole pixels.
    let mut glyphs: Vec<G> = Vec::new();
    for run in &line.glyph_runs {
        if run.face.is_empty() {
            return None;
        }
        for g in &run.glyphs {
            glyphs.push(G { id: g.id, face: run.face.clone(), x: g.x, adv: engine.face_advance(&run.face, g.id, run.size), size: run.size, style: run.style });
        }
    }
    if glyphs.is_empty() {
        return None;
    }
    let total = glyphs.iter().map(|g| g.x + g.adv).fold(1.0f64, f64::max);
    // CanvasKit's ShapedLine top/bottom are the runs' raw font ascent/descent around the baseline.
    let (mut asc, mut desc) = (0.0f64, 0.0f64);
    for run in &line.glyph_runs {
        if let Some(face) = engine.fonts.face_by_file(&run.face) {
            asc = asc.max(face.info.metrics.ascent * run.size);
            desc = desc.max(face.info.metrics.descent * run.size);
        }
    }
    let c = curve.clamp(-100.0, 100.0);
    let span = (c.abs() / 100.0) * PI * 2.0 * 0.95;
    let r = total / span;
    let up = c > 0.0;
    struct X {
        cos: f64,
        sin: f64,
        tx: f64,
        ty: f64,
    }
    let xforms: Vec<X> = glyphs
        .iter()
        .map(|g| {
            let theta = (g.x + g.adv / 2.0 - total / 2.0) / r;
            let rot = if up { theta } else { -theta };
            let cx = r * theta.sin();
            let cy = if up { r - r * theta.cos() } else { -(r - r * theta.cos()) };
            let (sin, cos) = rot.sin_cos();
            X { cos, sin, tx: cx - cos * (g.adv / 2.0), ty: cy - sin * (g.adv / 2.0) }
        })
        .collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (t, g) in xforms.iter().zip(&glyphs) {
        for (lx, ly) in [(0.0, -asc), (g.adv, -asc), (0.0, desc), (g.adv, desc)] {
            let px = t.tx + t.cos * lx - t.sin * ly;
            let py = t.ty + t.sin * lx + t.cos * ly;
            x0 = x0.min(px);
            y0 = y0.min(py);
            x1 = x1.max(px);
            y1 = y1.max(py);
        }
    }
    let placed: Vec<CurvedGlyph> = xforms
        .iter()
        .zip(&glyphs)
        .map(|(t, g)| CurvedGlyph { id: g.id, face: g.face.clone(), size: g.size, style: g.style, cos: t.cos, sin: t.sin, tx: t.tx - x0, ty: t.ty - y0 })
        .collect();
    let mut out_runs = Vec::new();
    let mut i = 0;
    while i < placed.len() {
        let mut j = i;
        while j < placed.len() && placed[j].style == placed[i].style && placed[j].size == placed[i].size && placed[j].face == placed[i].face {
            j += 1;
        }
        let style = placed[i].style;
        out_runs.push(CurvedRun {
            face: placed[i].face.clone(),
            size: placed[i].size,
            style,
            color: paint_to_color(p.styles.get(style).map(|s| &s.fill)),
            glyphs: i..j,
        });
        i = j;
    }
    Some(CurvedLayout { width: (x1 - x0).max(1.0), height: (y1 - y0).max(1.0), styles: p.styles.clone(), runs: out_runs, glyphs: placed })
}
