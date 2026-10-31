// SPDX-License-Identifier: Apache-2.0
//! Export watermark (plan 11 g64): a text mark added on top of every page of the design before it is drawn, so
//! images, PDFs, SVGs and ZIPs all carry it through the normal renderer. The design itself is not changed.

use oc_schema::color::hex;
use oc_schema::design::DesignDoc;
use oc_schema::elements::{Element, TextEl};
use oc_schema::factory::{TextOpts, base_init, make_text, xf};
use oc_schema::primitives::{Extra, Paint, SolidPaint};
use oc_schema::text::{RunStyleOverride, TextAlign};
use oc_schema::tree::sorted_pages;
use serde::Deserialize;

/// Where the mark goes: one of the corners, the centre, or repeated across the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WatermarkPosition {
    #[default]
    Center,
    Tiled,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The export request's `watermark` field.
#[derive(Debug, Clone, Deserialize)]
pub struct Watermark {
    pub text: String,
    #[serde(default)]
    pub position: WatermarkPosition,
    /// 0..1 (default 0.3).
    #[serde(default)]
    pub opacity: Option<f64>,
    /// Degrees (default −30 in the centre and tiled, 0 in corners).
    #[serde(default)]
    pub rotation: Option<f64>,
    /// Text height as a fraction of the page's shorter side (default 0.08).
    #[serde(default)]
    pub size: Option<f64>,
    /// `#rrggbb` (default white with a dark outline-free fill: `#ffffff`).
    #[serde(default)]
    pub color: Option<String>,
}

const MAX_TEXT: usize = 120;

/// Add the watermark to every page of `doc` (above all content). Returns how many marks were added.
pub fn apply(doc: &mut DesignDoc, w: &Watermark) -> usize {
    let text: String = w.text.trim().chars().take(MAX_TEXT).collect();
    if text.is_empty() {
        return 0;
    }
    let (pw, ph) = (doc.meta.dims.w, doc.meta.dims.h);
    if !(pw > 0.0 && ph > 0.0) {
        return 0;
    }
    let short = pw.min(ph);
    let size = (short * w.size.unwrap_or(0.08).clamp(0.02, 0.4)).max(4.0);
    let opacity = w.opacity.unwrap_or(0.3).clamp(0.02, 1.0);
    let corner = !matches!(w.position, WatermarkPosition::Center | WatermarkPosition::Tiled);
    let rot = w.rotation.unwrap_or(if corner { 0.0 } else { -30.0 }).clamp(-180.0, 180.0);
    let color = w.color.as_deref().and_then(|c| hex(c).ok()).unwrap_or_else(|| hex("#ffffff").expect("valid colour"));
    let style = RunStyleOverride {
        family: Some("Inter".into()),
        weight: Some(700.0),
        size: Some(size),
        fill: Some(Paint::Solid(SolidPaint { color, extra: Extra::new() })),
        ..Default::default()
    };
    // A line of text is about 1.3 font sizes tall; its width is estimated (0.62 em per character) for placement.
    let line_h = size * 1.3;
    let est_w = (text.chars().count() as f64 * size * 0.62 + size).min(pw * 0.96);
    let margin = short * 0.04;
    let mark = |page: &str, i: usize, x: f64, y: f64, w: f64, align: TextAlign| -> Element {
        let mut base = base_init(xf(x, y, w, line_h, rot));
        base.id = format!("watermark-{page}-{i}");
        base.parent_id = page.to_string();
        // Above every fractional index the editor writes ('a0'…'Zz…' sort before 'zz').
        base.z = format!("zz{i:05}");
        base.opacity = opacity;
        base.decorative = Some(true);
        let el: TextEl = make_text(base, TextOpts { text: Some(text.clone()), style: Some(style.clone()), align: Some(align), body: None });
        Element::Text(el)
    };
    let pages: Vec<String> = sorted_pages(&doc.pages).iter().map(|p| p.id.to_string()).collect();
    let mut added = 0;
    for page in &pages {
        let mut els = Vec::new();
        match w.position {
            WatermarkPosition::Center => els.push(mark(page, 0, (pw - est_w) / 2.0, (ph - line_h) / 2.0, est_w, TextAlign::Center)),
            WatermarkPosition::Tiled => {
                let (sx, sy) = (est_w + size * 2.0, line_h * 3.5);
                let (cols, rows) = (((pw / sx).ceil() as usize + 2).min(40), ((ph / sy).ceil() as usize + 2).min(60));
                for r in 0..rows {
                    for c in 0..cols {
                        // Alternate rows shift by half a cell (brick pattern).
                        let x = c as f64 * sx - sx + if r % 2 == 1 { sx / 2.0 } else { 0.0 };
                        let y = r as f64 * sy - sy;
                        els.push(mark(page, els.len(), x, y, est_w, TextAlign::Center));
                    }
                }
            }
            WatermarkPosition::TopLeft => els.push(mark(page, 0, margin, margin, est_w, TextAlign::Start)),
            WatermarkPosition::TopRight => els.push(mark(page, 0, pw - margin - est_w, margin, est_w, TextAlign::End)),
            WatermarkPosition::BottomLeft => els.push(mark(page, 0, margin, ph - margin - line_h, est_w, TextAlign::Start)),
            WatermarkPosition::BottomRight => els.push(mark(page, 0, pw - margin - est_w, ph - margin - line_h, est_w, TextAlign::End)),
        }
        for el in els {
            if let Element::Text(t) = &el {
                doc.elements.insert(t.base.id.clone(), el.clone());
                added += 1;
            }
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use oc_schema::factory::{MetaInit, empty_design};

    fn design() -> DesignDoc {
        empty_design(&MetaInit { w: 1080.0, h: 1080.0, pages: Some(2), ..Default::default() })
    }

    fn wm(position: WatermarkPosition) -> Watermark {
        Watermark { text: "DRAFT".into(), position, opacity: None, rotation: None, size: None, color: None }
    }

    #[test]
    fn one_mark_per_page_above_the_content() {
        let mut doc = design();
        let n = apply(&mut doc, &wm(WatermarkPosition::BottomRight));
        assert_eq!(n, doc.pages.len());
        let Some(Element::Text(t)) = doc.elements.values().last() else { panic!("text mark") };
        assert!(t.base.z.as_str() > "a0");
        assert_eq!(t.base.opacity, 0.3);
        assert_eq!(t.base.xf.rot, 0.0);
        assert!(t.base.xf.x + t.base.xf.w <= doc.meta.dims.w);
    }

    #[test]
    fn tiled_marks_cover_the_page_and_empty_text_adds_nothing() {
        let mut doc = design();
        assert!(apply(&mut doc, &wm(WatermarkPosition::Tiled)) > 4);
        let mut doc = design();
        assert_eq!(apply(&mut doc, &Watermark { text: "  ".into(), ..wm(WatermarkPosition::Center) }), 0);
    }
}
