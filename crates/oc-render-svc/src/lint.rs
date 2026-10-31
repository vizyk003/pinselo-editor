//! Layout checks for AI assistants (MCP `edit_design`): what a person would notice in a screenshot, as data, so an
//! assistant can fix its own work without looking. Uses the real text layout and the rendered page, so it sees what
//! the user sees: text that doesn't fit its box or runs off the page, elements outside the page, text that is hard to
//! read on what is actually behind it (images and gradients included), text boxes on top of each other, tiny text.

use oc_schema::color::to_rgba;
use oc_schema::elements::{AutoFit, Element, TextEl, TextFrameMode};
use oc_schema::primitives::{Paint, Rect};
use oc_schema::tree::page_elements;
use oc_schema::DesignDoc;
use serde::Serialize;

use crate::geom::world_bounds;
use crate::renderer::{RasterOptions, RenderHost, Renderer, Rgba};

/// At most this many issues per page (the worst first), so a messy page doesn't flood the assistant.
pub const MAX_ISSUES: usize = 12;

/// One problem on a page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    /// `text_overflow`, `text_off_page`, `off_page`, `partly_off_page`, `low_contrast`, `text_overlap`, `tiny_text`.
    pub kind: &'static str,
    /// The element (for `text_overlap`, the upper one).
    pub id: String,
    /// What is wrong and what to do, in one sentence.
    pub message: String,
    #[serde(skip)]
    severity: u8,
}

fn issue(kind: &'static str, id: &str, severity: u8, message: String) -> Issue {
    Issue { kind, id: id.to_owned(), message, severity }
}

/// WCAG relative luminance of a display sRGB colour (0..1 channels).
fn luminance(r: f64, g: f64, b: f64) -> f64 {
    let ch = |c: f64| if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
}

/// WCAG contrast ratio of two luminances.
pub fn contrast(a: f64, b: f64) -> f64 {
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// Display sRGB + alpha, 0..1.
type Rgba4 = (f64, f64, f64, f64);

/// The text's colour (solid paints only), size and weight: the first run that sets one, else the body defaults.
fn text_style(t: &TextEl) -> (Option<Rgba4>, f64, f64) {
    let runs = t.body.paragraphs.iter().flat_map(|p| p.runs.iter()).filter(|r| !r.text.trim().is_empty());
    let mut fill = None;
    let (mut size, mut weight) = (t.body.default_run.size, t.body.default_run.weight);
    for r in runs {
        if fill.is_none() {
            fill = r.style.fill.as_ref();
        }
        size = size.max(r.style.size.unwrap_or(0.0));
        weight = weight.max(r.style.weight.unwrap_or(0.0));
    }
    let paint = fill.unwrap_or(&t.body.default_run.fill);
    let color = match paint {
        Paint::Solid(s) => {
            let c = to_rgba(&s.color);
            Some((c.r, c.g, c.b, c.a))
        }
        _ => None,
    };
    (color, size, weight)
}

fn has_text(t: &TextEl) -> bool {
    t.body.paragraphs.iter().any(|p| p.runs.iter().any(|r| !r.text.trim().is_empty()))
}

/// Luminances of up to ~400 evenly spread pixels of `r` (document px) in `img` rendered at `scale`.
fn sample(img: &Rgba, r: &Rect, scale: f64) -> Vec<f64> {
    let (w, h) = (img.width as f64, img.height as f64);
    let x0 = (r.x * scale).clamp(0.0, w - 1.0);
    let y0 = (r.y * scale).clamp(0.0, h - 1.0);
    let x1 = ((r.x + r.w) * scale).clamp(0.0, w - 1.0);
    let y1 = ((r.y + r.h) * scale).clamp(0.0, h - 1.0);
    if x1 <= x0 || y1 <= y0 {
        return Vec::new();
    }
    let n = 20.0;
    let mut out = Vec::new();
    for i in 0..20 {
        for j in 0..20 {
            let px = (x0 + (x1 - x0) * (f64::from(i) + 0.5) / n) as usize;
            let py = (y0 + (y1 - y0) * (f64::from(j) + 0.5) / n) as usize;
            let k = (py * img.width as usize + px) * 4;
            if let Some(p) = img.pixels.get(k..k + 3) {
                out.push(luminance(f64::from(p[0]) / 255.0, f64::from(p[1]) / 255.0, f64::from(p[2]) / 255.0));
            }
        }
    }
    out
}

fn overlap_area(a: &Rect, b: &Rect) -> f64 {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

fn px(v: f64) -> String {
    format!("{}", v.round())
}

/// Checks one page. `renderer`/`host` lay text out and render the page without its text (the background the text
/// is read against).
pub async fn lint_page(renderer: &dyn Renderer, host: &dyn RenderHost, doc: &DesignDoc, page_id: &str) -> Vec<Issue> {
    let (pw, ph) = (doc.meta.dims.w, doc.meta.dims.h);
    let page = Rect { x: 0.0, y: 0.0, w: pw, h: ph };
    let els: Vec<&Element> = page_elements(&doc.elements, page_id)
        .into_iter()
        .filter(|e| e.base().is_some_and(|b| b.visible != Some(false)))
        .collect();
    let mut issues = Vec::new();
    // Text boxes as laid out: (id, box, rotated).
    let mut texts: Vec<(String, Rect, &TextEl)> = Vec::new();
    for el in &els {
        let Some(base) = el.base() else { continue };
        let id = base.id.to_string();
        let mut b = world_bounds(&doc.elements, &id);
        // Text as laid out: an auto-height box is as tall as its text, whatever height is stored.
        let text = match el {
            Element::Text(t) if has_text(t) => Some(t),
            _ => None,
        };
        let rotated = base.xf.rot.abs() > 0.5;
        if let Some(t) = text.filter(|_| !rotated)
            && let Ok(m) = renderer.layout_text(host, &t.body, base.xf.w).await
        {
            let fixed = t.frame.mode == TextFrameMode::Fixed && t.auto_fit != AutoFit::Shrink;
            if fixed && m.height > base.xf.h + 1.0 {
                issues.push(issue(
                    "text_overflow",
                    &id,
                    3,
                    format!(
                        "Text needs {}px but its box is {}px tall, so it spills out; make the box taller (h), the text smaller, or shorten it.",
                        px(m.height),
                        px(base.xf.h)
                    ),
                ));
            }
            b.h = b.h.max(m.height);
        }
        if b.w <= 0.0 && b.h <= 0.0 {
            continue;
        }
        if overlap_area(&b, &page) == 0.0 {
            issues.push(issue("off_page", &id, 2, format!("{} is entirely outside the page; move it onto the page or delete it.", label(el))));
            continue;
        }
        let Some(t) = text else { continue };
        if overlap_area(&b, &page) < b.w * b.h - 1.0 {
            issues.push(issue(
                "text_off_page",
                &id,
                3,
                format!("Text runs past the page edge (box {},{} {}×{} on a {}×{} page); move it in or make it smaller.", px(b.x), px(b.y), px(b.w), px(b.h), px(pw), px(ph)),
            ));
        }
        let (_, size, _) = text_style(t);
        if size > 0.0 && size < pw.min(ph) * 0.012 {
            issues.push(issue("tiny_text", &id, 1, format!("Text at {}px is very small for a {}×{} page; use at least {}px.", px(size), px(pw), px(ph), px((pw.min(ph) * 0.015).ceil()))));
        }
        texts.push((id, b, t));
    }
    // Text on top of text.
    for i in 0..texts.len() {
        for j in (i + 1)..texts.len() {
            let (a, b) = (&texts[i], &texts[j]);
            let o = overlap_area(&a.1, &b.1);
            if o > 0.15 * (a.1.w * a.1.h).min(b.1.w * b.1.h) {
                issues.push(issue("text_overlap", &b.0, 2, format!("This text overlaps text {}; move one of them or make them smaller.", a.0)));
            }
        }
    }
    // Contrast against what is really behind the text: the page rendered without any text.
    if !texts.is_empty() {
        let scale = (512.0 / pw.max(ph)).min(1.0);
        let opts = RasterOptions { scale, skip: texts.iter().map(|t| t.0.clone()).collect(), ..RasterOptions::default() };
        if let Ok(img) = renderer.render_rgba(host, doc, page_id, &opts).await {
            for (id, r, t) in &texts {
                let (Some((cr, cg, cb, ca)), size, weight) = text_style(t) else { continue };
                let mut lum = sample(&img, r, scale);
                if lum.is_empty() {
                    continue;
                }
                lum.sort_by(f64::total_cmp);
                let alpha = ca * els.iter().find_map(|e| e.base().filter(|b| *b.id == **id).map(|b| b.opacity)).unwrap_or(1.0);
                let text_lum = |bg: f64| {
                    // Semi-transparent text is mixed with the background (approximately, in luminance).
                    alpha * luminance(cr, cg, cb) + (1.0 - alpha) * bg
                };
                // The ratio over the background, and over the worst fifth of it (busy photos, gradients).
                let ratios: Vec<f64> = {
                    let mut v: Vec<f64> = lum.iter().map(|&bg| contrast(text_lum(bg), bg)).collect();
                    v.sort_by(f64::total_cmp);
                    v
                };
                let typical = ratios[ratios.len() / 2];
                let worst = ratios[ratios.len() / 5];
                let large = size >= 24.0 || (size >= 18.66 && weight >= 700.0);
                let need = if large { 3.0 } else { 4.5 };
                if typical < need || worst < need * 0.67 {
                    let where_ = if typical < need { "its background" } else { "part of its background" };
                    issues.push(issue(
                        "low_contrast",
                        id,
                        if typical < need { 3 } else { 2 },
                        format!(
                            "Text is hard to read on {where_} (contrast {:.1}:1, needs {need}:1); change the text colour, add a solid shape behind it, or move it.",
                            if typical < need { typical } else { worst }
                        ),
                    ));
                }
            }
        }
    }
    issues.sort_by_key(|i| std::cmp::Reverse(i.severity));
    issues.truncate(MAX_ISSUES);
    issues
}

fn label(el: &Element) -> String {
    let kind = match el {
        Element::Text(_) => "Text",
        Element::Image(_) => "An image",
        Element::Shape(_) => "A shape",
        Element::Group(_) => "A group",
        _ => "An element",
    };
    match el.base().and_then(|b| b.name.as_deref()) {
        Some(n) if !n.is_empty() => format!("{kind} \"{n}\""),
        _ => kind.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_matches_wcag() {
        let white = luminance(1.0, 1.0, 1.0);
        let black = luminance(0.0, 0.0, 0.0);
        assert!((contrast(white, black) - 21.0).abs() < 1e-9);
        // #767676 on white is the classic 4.54:1.
        let g = 0x76 as f64 / 255.0;
        assert!((contrast(white, luminance(g, g, g)) - 4.54).abs() < 0.01);
    }

    #[test]
    fn overlap() {
        let a = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        assert_eq!(overlap_area(&a, &Rect { x: 5.0, y: 5.0, w: 10.0, h: 10.0 }), 25.0);
        assert_eq!(overlap_area(&a, &Rect { x: 20.0, y: 0.0, w: 5.0, h: 5.0 }), 0.0);
    }
}
