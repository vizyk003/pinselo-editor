// SPDX-License-Identifier: Apache-2.0
//! Print sizes for PDF Print export and print preflight.

use std::collections::HashSet;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::color::to_rgba;
use crate::design::DesignDoc;
use crate::elements::{Element, TextEl};
use crate::factory::grid_cell_box;
use crate::js;
use crate::presets::{IN, MM, inch, mm};
use crate::primitives::{ColorValue, Paint, Rect, Unit, Xf};
use crate::tree::{page_elements, sorted_pages};

// ---------------------------------------------------------------- catalogue

/// A standard trim size (or, for apparel/mugs/bags, the print area). px @ 96 dpi like design dims.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrintSize {
    pub id: String,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintProduct {
    pub id: String,
    pub sizes: Vec<PrintSize>,
    /// Bleed added around the trim (0 for print-area products such as t-shirts).
    pub bleed_mm: f64,
    pub rotatable: bool,
}

/// The sizes the editor recognises as print products, for default CMYK and bleed in PDF Print export.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrintCatalog {
    pub products: Vec<PrintProduct>,
}

fn size(id: &str, w: f64, h: f64) -> PrintSize {
    PrintSize { id: id.into(), w, h }
}

fn product(id: &str, bleed_mm: f64, rotatable: bool, sizes: Vec<PrintSize>) -> PrintProduct {
    PrintProduct { id: id.into(), sizes, bleed_mm, rotatable }
}

pub static DEFAULT_PRINT_CATALOG: LazyLock<PrintCatalog> = LazyLock::new(|| PrintCatalog {
    products: vec![
        product("business-cards", 3.0, true, vec![size("us", inch(3.5), inch(2.0)), size("eu", mm(85.0), mm(55.0))]),
        product("flyers", 3.0, true, vec![size("a5", mm(148.0), mm(210.0)), size("a4", mm(210.0), mm(297.0)), size("letter", inch(8.5), inch(11.0))]),
        product("posters", 3.0, true, vec![size("18x24", inch(18.0), inch(24.0)), size("a2", mm(420.0), mm(594.0)), size("a3", mm(297.0), mm(420.0))]),
        product("invitations", 3.0, true, vec![size("5x7", inch(5.0), inch(7.0))]),
        product("postcards", 3.0, true, vec![size("6x4", inch(6.0), inch(4.0)), size("a6", mm(148.0), mm(105.0))]),
        product("stickers", 2.0, false, vec![size("2in", inch(2.0), inch(2.0)), size("3in", inch(3.0), inch(3.0)), size("4in", inch(4.0), inch(4.0))]),
        product("t-shirts", 0.0, false, vec![size("front", inch(12.0), inch(16.0))]),
        product("mugs", 0.0, false, vec![size("11oz", inch(9.0), inch(3.5))]),
        product("tote-bags", 0.0, false, vec![size("front", inch(14.0), inch(14.0))]),
    ],
});

// ---------------------------------------------------------------- preflight

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PreflightFix {
    #[serde(rename = "resize-design")]
    ResizeDesign { w: f64, h: f64, unit: Unit },
    #[serde(rename = "extend-to-bleed", rename_all = "camelCase")]
    ExtendToBleed { element_id: String, xf: Xf },
    #[serde(rename = "thicken-line", rename_all = "camelCase")]
    ThickenLine { element_id: String, width: f64 },
    #[serde(rename = "add-page")]
    AddPage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightIssue {
    /// 'size_mismatch' | 'size_scaled' | 'page_count_over' | 'page_count_under' | 'low_dpi' | 'dpi_below_recommended'
    /// | 'text_outside_safe' | 'bleed_missing' | 'out_of_gamut' | 'thin_line' | 'font_missing' | 'not_printable'
    pub code: String,
    /// 'error' | 'warning' | 'info'
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    /// Values for the message (numbers are already rounded).
    pub vars: Map<String, Value>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<PreflightFix>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightTarget {
    pub w: f64,
    pub h: f64,
    pub unit: Unit,
    pub bleed_mm: f64,
    pub safe_mm: f64,
    pub rotatable: bool,
    pub min_dpi: f64,
    pub good_dpi: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Trim {
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SeverityCounts {
    pub error: usize,
    pub warning: usize,
    pub info: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreflightResult {
    pub ok: bool,
    pub issues: Vec<PreflightIssue>,
    /// Print size ÷ design size (1 when the design already has the product size).
    pub scale: f64,
    pub trim: Trim,
    pub counts: SeverityCounts,
}

/// Generic print target for a design on its own: its own size, 3 mm bleed, 5 mm safe zone.
pub fn standalone_print_target(doc: &DesignDoc) -> PreflightTarget {
    PreflightTarget {
        w: doc.meta.dims.w,
        h: doc.meta.dims.h,
        unit: doc.meta.dims.unit.clone(),
        bleed_mm: 3.0,
        safe_mm: 5.0,
        rotatable: false,
        min_dpi: 150.0,
        good_dpi: 300.0,
        pages: None,
    }
}

/// Rough check: does this sRGB colour fall outside a typical coated CMYK gamut?
pub fn cmyk_gamut_risk(c: &ColorValue) -> bool {
    if matches!(c, ColorValue::Cmyk(_) | ColorValue::Spot(_)) {
        return false;
    }
    let x = to_rgba(c);
    let (r, g, b, a) = (x.r, x.g, x.b, x.a);
    if a < 0.05 {
        return false;
    }
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let v = max;
    let sat = if max == 0.0 { 0.0 } else { (max - min) / max };
    if sat < 0.82 || v < 0.72 {
        return false;
    }
    let d = max - min;
    let mut hue = 0.0;
    if d > 0.0 {
        hue = if max == r {
            60.0 * (((g - b) / d) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
    }
    if hue < 0.0 {
        hue += 360.0;
    }
    let safe = hue < 12.0 || (45.0..=70.0).contains(&hue) || (175.0..=200.0).contains(&hue) || (300.0..=345.0).contains(&hue) || hue > 350.0;
    if safe {
        return false;
    }
    if (12.0..45.0).contains(&hue) {
        return sat > 0.92 && v > 0.95;
    }
    true
}

fn paint_colors(p: Option<&Paint>) -> Vec<&ColorValue> {
    match p {
        Some(Paint::Solid(s)) => vec![&s.color],
        Some(Paint::Linear(l)) => l.stops.iter().map(|s| &s.color).collect(),
        Some(Paint::Radial(r)) => r.stops.iter().map(|s| &s.color).collect(),
        _ => vec![],
    }
}

fn element_colors(el: &Element) -> Vec<&ColorValue> {
    match el {
        Element::Shape(e) => e
            .fills
            .iter()
            .flat_map(|f| paint_colors(Some(f)))
            .chain(e.strokes.iter().flat_map(|s| paint_colors(Some(&s.paint))))
            .collect(),
        Element::Line(e) => paint_colors(Some(&e.stroke.paint)),
        Element::Text(e) => std::iter::once(Some(&e.body.default_run.fill))
            .chain(e.body.paragraphs.iter().flat_map(|p| p.runs.iter().map(|r| r.style.fill.as_ref())))
            .flat_map(paint_colors)
            .collect(),
        Element::Qr(e) => vec![&e.fg, &e.bg],
        Element::Draw(e) => vec![&e.color],
        Element::Frame(e) => {
            if e.content.is_some() {
                vec![]
            } else {
                paint_colors(Some(&e.placeholder_fill))
            }
        }
        _ => vec![],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// Axis-aligned bounds of a (possibly rotated) element box, page px.
pub fn element_bounds(xf: &Xf) -> Bounds {
    let cx = xf.x + xf.w / 2.0;
    let cy = xf.y + xf.h / 2.0;
    let r = (xf.rot * std::f64::consts::PI) / 180.0;
    let hw = (r.cos().abs() * xf.w + r.sin().abs() * xf.h) / 2.0;
    let hh = (r.sin().abs() * xf.w + r.cos().abs() * xf.h) / 2.0;
    Bounds { x0: cx - hw, y0: cy - hh, x1: cx + hw, y1: cy + hh }
}

struct ImageSource {
    src_w: f64,
    src_h: f64,
    box_w: f64,
    box_h: f64,
}

fn image_sources(doc: &DesignDoc, el: &Element) -> Vec<ImageSource> {
    let mut out = Vec::new();
    let mut add = |id: &str, crop: Option<(f64, f64)>, box_w: f64, box_h: f64| {
        let Some(a) = doc.assets.get(id) else { return };
        if a.kind.as_str() == "svg" || !(a.w > 0.0 && a.h > 0.0) {
            return;
        }
        let (cw, ch) = crop.unwrap_or((1.0, 1.0));
        out.push(ImageSource { src_w: a.w * cw, src_h: a.h * ch, box_w, box_h });
    };
    match el {
        Element::Image(e) => add(&e.asset, Some((e.crop.w, e.crop.h)), e.base.xf.w, e.base.xf.h),
        Element::Frame(e) => {
            if let Some(c) = &e.content
                && c.video.is_none()
            {
                add(&c.asset, Some((c.crop.w, c.crop.h)), e.base.xf.w, e.base.xf.h);
            }
        }
        Element::Grid(e) => {
            for c in &e.cells {
                let Some(asset) = c.asset.as_deref().filter(|a| !a.is_empty()) else { continue };
                if c.video.is_some() {
                    continue;
                }
                let bx = grid_cell_box(e.gap, &e.base.xf, &Rect::from(&c.rect));
                add(asset, c.crop.as_ref().map(|r| (r.w, r.h)), bx.w, bx.h);
            }
        }
        Element::Shape(e) => {
            for f in &e.fills {
                if let Paint::Image(i) = f {
                    add(&i.asset, None, e.base.xf.w, e.base.xf.h);
                }
            }
        }
        _ => {}
    }
    out
}

fn fonts_of(el: &TextEl) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |f: &str| {
        if !out.iter().any(|x| x == f) {
            out.push(f.to_string());
        }
    };
    push(&el.body.default_run.family);
    for p in &el.body.paragraphs {
        for r in &p.runs {
            if let Some(f) = r.style.family.as_deref().filter(|f| !f.is_empty()) {
                push(f);
            }
        }
    }
    out
}

/// `px` in a display unit, rounded (`pxToDisplay`).
fn px_to_display(px: f64, unit: &Unit) -> String {
    let per = match unit {
        Unit::Px => 1.0,
        Unit::Mm => MM,
        Unit::Cm => MM * 10.0,
        Unit::In => IN,
        Unit::Pt => 96.0 / 72.0,
        Unit::Other(_) => f64::NAN,
    };
    let v = px / per;
    if *unit == Unit::Px { js::num_to_string(js::round(v)) } else { js::num_to_string(js::round(v * 10.0) / 10.0) }
}

/// JS `n.toString(16)` for an integer-valued number.
fn int_hex(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    let i = n as i64;
    if i < 0 { format!("-{:x}", -i) } else { format!("{i:x}") }
}

/// Print preflight (size, page count, image resolution, safe zone, bleed, gamut, hairlines, fonts, interactivity).
/// `available_fonts` = `opts.availableFonts.has`. Elements of unknown types are skipped (TS would throw on them).
pub fn preflight_print(doc: &DesignDoc, target: &PreflightTarget, available_fonts: Option<&dyn Fn(&str) -> bool>) -> PreflightResult {
    preflight_print_with(doc, target, available_fonts, None)
}

/// `opts.outOfGamut`: `Some(verdict)` overrides the heuristic for a colour, `None` keeps it.
pub type OutOfGamut<'a> = &'a dyn Fn(&ColorValue) -> Option<bool>;

/// [`preflight_print`] with `opts.outOfGamut`: an out-of-gamut test that replaces the `cmyk_gamut_risk` heuristic
/// (e.g. a colour-managed check against the chosen output profile); `None` from it falls back to the heuristic.
pub fn preflight_print_with(
    doc: &DesignDoc,
    target: &PreflightTarget,
    available_fonts: Option<&dyn Fn(&str) -> bool>,
    out_of_gamut: Option<OutOfGamut<'_>>,
) -> PreflightResult {
    let mut issues: Vec<PreflightIssue> = Vec::new();
    let (dw, dh) = (doc.meta.dims.w, doc.meta.dims.h);
    let (mut tw, mut th) = (target.w, target.h);
    if target.rotatable && (dw > dh) != (tw > th) && (dw - dh).abs() > 1.0 {
        std::mem::swap(&mut tw, &mut th);
    }
    let tol = MM;
    let exact = (dw - tw).abs() <= tol && (dh - th).abs() <= tol;
    let same_aspect = (dw / dh - tw / th).abs() / (tw / th) < 0.01;
    let scale = if exact { 1.0 } else if same_aspect { tw / dw } else { (tw / dw).min(th / dh) };
    let size = |w: f64, h: f64| format!("{} × {} {}", px_to_display(w, &target.unit), px_to_display(h, &target.unit), target.unit);
    let issue = |code: &str, severity: &str, vars: Value, message: String| PreflightIssue {
        code: code.into(),
        severity: severity.into(),
        page_id: None,
        page_index: None,
        element_id: None,
        vars: match vars {
            Value::Object(m) => m,
            _ => Map::new(),
        },
        message,
        fix: None,
    };
    if !exact {
        let fix = PreflightFix::ResizeDesign { w: tw, h: th, unit: target.unit.clone() };
        let vars = json!({ "design": size(dw, dh), "product": size(tw, th) });
        let mut i = if same_aspect {
            issue("size_scaled", "warning", vars, format!("The design ({}) will be scaled to the product size ({}).", size(dw, dh), size(tw, th)))
        } else {
            issue(
                "size_mismatch",
                "error",
                vars,
                format!("The design is {} but the product is {}. Resize the design to the product.", size(dw, dh), size(tw, th)),
            )
        };
        i.fix = Some(fix);
        issues.push(i);
    }

    let pages: Vec<_> = sorted_pages(&doc.pages).into_iter().filter(|p| p.hidden != Some(true)).collect();
    if let Some(expected) = target.pages {
        let n = pages.len() as f64;
        let e = js::num_to_string(expected);
        if n > expected {
            issues.push(issue(
                "page_count_over",
                "error",
                json!({ "pages": pages.len(), "expected": expected }),
                format!("The design has {} pages but this product prints {e}.", pages.len()),
            ));
        } else if n < expected {
            let mut i = issue(
                "page_count_under",
                "warning",
                json!({ "pages": pages.len(), "expected": expected }),
                format!("This product prints {e} sides; the design has {}, so the back will be blank.", pages.len()),
            );
            i.fix = Some(PreflightFix::AddPage);
            issues.push(i);
        }
    }

    let bleed = (target.bleed_mm * MM) / scale;
    let safe = (target.safe_mm * MM) / scale;
    let hairline = (0.25 * (96.0 / 72.0)) / scale;
    let (w_, h_) = (dw, dh);
    let mut gamut_seen: HashSet<String> = HashSet::new();
    let mut font_seen: HashSet<String> = HashSet::new();
    let bleed_s = js::num_to_string(target.bleed_mm);
    let safe_s = js::num_to_string(target.safe_mm);

    let mut gamut = |issues: &mut Vec<PreflightIssue>, c: &ColorValue, page_id: &str, page_index: usize, element_id: Option<&str>| {
        if !out_of_gamut.and_then(|f| f(c)).unwrap_or_else(|| cmyk_gamut_risk(c)) {
            return;
        }
        let x = to_rgba(c);
        let hex = format!(
            "#{}",
            [x.r, x.g, x.b].iter().map(|v| format!("{:0>2}", int_hex(js::round(v * 255.0)))).collect::<String>()
        );
        if !gamut_seen.insert(hex.clone()) {
            return;
        }
        let mut i = issue("out_of_gamut", "info", json!({ "color": hex }), format!("{hex} is brighter than print inks can reproduce and will print duller."));
        i.page_id = Some(page_id.into());
        i.page_index = Some(page_index);
        i.element_id = element_id.map(String::from);
        issues.push(i);
    };

    for (page_index, page) in pages.iter().enumerate() {
        for bg in &page.background {
            for c in paint_colors(Some(bg)) {
                gamut(&mut issues, c, &page.id, page_index, None);
            }
        }
        for el in page_elements(&doc.elements, &page.id) {
            let Some(base) = el.base() else { continue };
            if base.visible == Some(false) || base.deleted.is_some() {
                continue;
            }
            let at = |mut i: PreflightIssue| {
                i.page_id = Some(page.id.clone());
                i.page_index = Some(page_index);
                i.element_id = Some(base.id.clone());
                i
            };
            let name = base.name.clone().unwrap_or_else(|| el.type_name().to_string());
            let frame_video = matches!(el, Element::Frame(f) if f.content.as_ref().is_some_and(|c| c.video.is_some()));
            if matches!(el, Element::Video(_) | Element::Embed(_) | Element::Form(_)) || frame_video {
                issues.push(at(issue(
                    "not_printable",
                    "warning",
                    json!({ "name": name }),
                    format!("“{name}” is interactive or moving and prints as a still image."),
                )));
            }
            for src in image_sources(doc, el) {
                let dpi = (src.src_w / ((src.box_w * scale) / 96.0)).min(src.src_h / ((src.box_h * scale) / 96.0));
                let dpi = {
                    // Math.min propagates NaN; f64::min does not.
                    let a = src.src_w / ((src.box_w * scale) / 96.0);
                    let b = src.src_h / ((src.box_h * scale) / 96.0);
                    if a.is_nan() || b.is_nan() { f64::NAN } else { dpi }
                };
                if !dpi.is_finite() {
                    continue;
                }
                let rd = js::round(dpi);
                if dpi < target.min_dpi {
                    issues.push(at(issue(
                        "low_dpi",
                        "warning",
                        json!({ "dpi": rd, "min": target.min_dpi, "name": name }),
                        format!(
                            "“{name}” prints at {} dpi (below {}); it will look blurry. Use a larger image or make it smaller.",
                            js::num_to_string(rd),
                            js::num_to_string(target.min_dpi)
                        ),
                    )));
                } else if dpi < target.good_dpi {
                    issues.push(at(issue(
                        "dpi_below_recommended",
                        "info",
                        json!({ "dpi": rd, "good": target.good_dpi, "name": name }),
                        format!(
                            "“{name}” prints at {} dpi; {} dpi is recommended for sharp detail.",
                            js::num_to_string(rd),
                            js::num_to_string(target.good_dpi)
                        ),
                    )));
                }
            }
            let b = element_bounds(&base.xf);
            if let Element::Text(t) = el {
                let has_text = t.body.paragraphs.iter().any(|p| p.runs.iter().any(|r| !js::trim(&r.text).is_empty()));
                if target.safe_mm > 0.0
                    && has_text
                    && (b.x0 < safe - 0.5 || b.y0 < safe - 0.5 || b.x1 > w_ - safe + 0.5 || b.y1 > h_ - safe + 0.5)
                {
                    issues.push(at(issue(
                        "text_outside_safe",
                        "warning",
                        json!({ "name": name, "safe": target.safe_mm }),
                        format!("Text “{name}” is within {safe_s} mm of the edge and may be trimmed."),
                    )));
                }
                if let Some(has) = available_fonts {
                    for f in fonts_of(t) {
                        if !has(&f) && !font_seen.contains(&f) {
                            font_seen.insert(f.clone());
                            issues.push(at(issue(
                                "font_missing",
                                "warning",
                                json!({ "font": f }),
                                format!("The font “{f}” isn’t available to the print renderer and will be replaced."),
                            )));
                        }
                    }
                }
            }
            if target.bleed_mm > 0.0 && base.parent_id == page.id && !matches!(el, Element::Text(_) | Element::Line(_) | Element::Group(_)) {
                let (tl, tt) = (b.x0 <= 0.5 && b.x0 > -bleed + 0.5, b.y0 <= 0.5 && b.y0 > -bleed + 0.5);
                let (tr, tb) = (b.x1 >= w_ - 0.5 && b.x1 < w_ + bleed - 0.5, b.y1 >= h_ - 0.5 && b.y1 < h_ + bleed - 0.5);
                let span_x = b.x1.min(w_) - b.x0.max(0.0);
                let span_y = b.y1.min(h_) - b.y0.max(0.0);
                let mut sides: Vec<char> = Vec::new();
                for (k, t) in [('l', tl), ('r', tr)] {
                    if t && span_y > h_ * 0.2 {
                        sides.push(k);
                    }
                }
                for (k, t) in [('t', tt), ('b', tb)] {
                    if t && span_x > w_ * 0.2 {
                        sides.push(k);
                    }
                }
                if !sides.is_empty() {
                    let mut xf = base.xf.clone();
                    if base.xf.rot == 0.0 {
                        if sides.contains(&'l') {
                            xf.w += xf.x + bleed;
                            xf.x = -bleed;
                        }
                        if sides.contains(&'t') {
                            xf.h += xf.y + bleed;
                            xf.y = -bleed;
                        }
                        if sides.contains(&'r') {
                            xf.w = w_ + bleed - xf.x;
                        }
                        if sides.contains(&'b') {
                            xf.h = h_ + bleed - xf.y;
                        }
                    }
                    let mut i = at(issue(
                        "bleed_missing",
                        "warning",
                        json!({ "name": name, "bleed": target.bleed_mm }),
                        format!("“{name}” stops at the trim edge; extend it {bleed_s} mm past the edge so no white line shows after cutting."),
                    ));
                    if base.xf.rot == 0.0 {
                        i.fix = Some(PreflightFix::ExtendToBleed { element_id: base.id.clone(), xf });
                    }
                    issues.push(i);
                }
            }
            let stroke_w = match el {
                Element::Line(l) => Some(l.stroke.width),
                Element::Shape(s) if !s.strokes.is_empty() => Some(s.strokes.iter().map(|x| x.width).fold(f64::INFINITY, f64::min)),
                _ => None,
            };
            if let Some(sw) = stroke_w
                && sw > 0.0
                && sw < hairline
            {
                let width = js::round(sw * scale * 0.75 * 100.0) / 100.0;
                let mut i = at(issue(
                    "thin_line",
                    "warning",
                    json!({ "name": name, "width": width }),
                    format!("“{name}” has a {} pt line; lines under 0.25 pt may not print.", js::num_to_string(width)),
                ));
                i.fix = Some(PreflightFix::ThickenLine { element_id: base.id.clone(), width: (hairline * 100.0).ceil() / 100.0 });
                issues.push(i);
            }
            for c in element_colors(el) {
                gamut(&mut issues, c, &page.id, page_index, Some(&base.id));
            }
        }
    }

    let mut counts = SeverityCounts::default();
    for i in &issues {
        match i.severity.as_str() {
            "error" => counts.error += 1,
            "warning" => counts.warning += 1,
            _ => counts.info += 1,
        }
    }
    PreflightResult { ok: counts.error == 0, issues, scale, trim: Trim { w: tw, h: th }, counts }
}

/// Bleed in design px for a product (for the export job).
pub fn bleed_px_for(bleed_mm: f64) -> f64 {
    bleed_mm * MM
}

/// Unique colours that can fall outside a CMYK gamut (sRGB / Display-P3, not transparent), in the order preflight
/// meets them: page backgrounds, then visible elements. Capped at `max` (for a colour-managed gamut check).
/// TS `printColors(doc, max = 512)`.
pub fn print_colors(doc: &DesignDoc, max: usize) -> Vec<ColorValue> {
    let mut out: Vec<ColorValue> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut add = |c: &ColorValue| {
        if out.len() >= max || matches!(c, ColorValue::Cmyk(_) | ColorValue::Spot(_)) || to_rgba(c).a < 0.05 {
            return;
        }
        if seen.insert(print_color_key(c)) {
            out.push(c.clone());
        }
    };
    for page in sorted_pages(&doc.pages).into_iter().filter(|p| p.hidden != Some(true)) {
        for bg in &page.background {
            paint_colors(Some(bg)).into_iter().for_each(&mut add);
        }
        for el in page_elements(&doc.elements, &page.id) {
            let Some(base) = el.base() else { continue };
            if base.visible == Some(false) || el.is_deleted() {
                continue;
            }
            element_colors(el).into_iter().for_each(&mut add);
        }
    }
    out
}

/// Identity of a colour for gamut lookups (space + rounded components, alpha ignored). TS `printColorKey`.
pub fn print_color_key(c: &ColorValue) -> String {
    let x = to_rgba(c);
    let parts: Vec<String> = [x.r, x.g, x.b].iter().map(|v| js::num_to_string(js::round(v * 1000.0))).collect();
    format!("{}:{}", c.space(), parts.join(","))
}
