// SPDX-License-Identifier: Apache-2.0
//! Accessibility (A11Y-010..014): a design checker and plain-language element descriptions for the
//! screen-reader mirror of the canvas. Pure functions over the document; the UI maps issue codes to
//! translated messages. Port of `a11y.ts`.

use oc_geometry::world_bounds;
use oc_schema::color::contrast_ratio;
use oc_schema::elements::Element;
use oc_schema::js::{is_js_space, trim};
use oc_schema::primitives::{ColorValue, Paint, Rect};
use oc_schema::text::{body_plain_text, resolve_run_style};
use oc_schema::tree::{descendants, sorted_pages};
use oc_schema::DesignDoc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::util::{max, min, num, round, slice16};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum A11yIssueCode {
    AltMissing,
    LowContrast,
    SmallText,
    LinkName,
    NoLanguage,
    EmptyText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A11yIssue {
    pub code: A11yIssueCode,
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Code-specific data: contrast ratio, font size, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Map<String, Value>>,
}

fn solid(fills: &[Paint]) -> Option<&ColorValue> {
    match fills.last() {
        Some(Paint::Solid(s)) => Some(&s.color),
        _ => None,
    }
}

fn contains(r: &Rect, x: f64, y: f64) -> bool {
    x >= r.x && y >= r.y && x <= r.x + r.w && y <= r.y + r.h
}

const MEDIA: &[&str] = &["image", "video", "grid", "frame", "qr", "chart"];

fn visible(el: &Element) -> bool {
    el.base().is_none_or(|b| b.visible != Some(false))
}

/// The visible solid colour behind a point: the topmost solid-filled shape below `under` in paint order, else the page.
fn backdrop(doc: &DesignDoc, page_id: &str, order: &[&Element], under: usize, x: f64, y: f64) -> Option<ColorValue> {
    for i in (0..under).rev() {
        let el = order[i];
        let b = world_bounds(&doc.elements, el.id().unwrap_or("")).unwrap_or_default();
        if !visible(el) || el.is_deleted() || !contains(&b, x, y) {
            continue;
        }
        if let Element::Shape(s) = el {
            let c = solid(&s.fills);
            return match c {
                Some(c) if s.base.opacity > 0.9 => Some(c.clone()),
                _ => None,
            };
        }
        if MEDIA.contains(&el.type_name()) {
            return None; // a photo: contrast can't be judged from the document alone
        }
    }
    doc.pages.get(page_id).and_then(|p| solid(&p.background)).cloned()
}

/// Smallest legible size, in design px, for the design's dimensions (≈ 12 px on a 1080-px social post, 18 px on a 1920 slide).
pub fn min_text_px(doc: &DesignDoc) -> f64 {
    let (w, h) = (doc.meta.dims.w, doc.meta.dims.h);
    max(9.0, round(min(w, h) / 90.0))
}

fn blank(s: Option<&String>) -> bool {
    s.is_none_or(|s| trim(s).is_empty())
}

pub fn audit_design(doc: &DesignDoc) -> Vec<A11yIssue> {
    let mut issues = Vec::new();
    let issue = |code, severity, page: Option<&str>, id: Option<&str>, data: Option<Map<String, Value>>| A11yIssue {
        code,
        severity,
        page_id: page.map(str::to_string),
        id: id.map(str::to_string),
        data,
    };
    if doc.meta.lang.is_empty() {
        issues.push(issue(A11yIssueCode::NoLanguage, Severity::Warning, None, None, None));
    }
    let min_px = min_text_px(doc);
    for page in sorted_pages(&doc.pages) {
        if page.hidden == Some(true) {
            continue;
        }
        let order: Vec<&Element> = descendants(&doc.elements, &page.id).into_iter().filter(|e| !e.is_deleted()).collect();
        for (i, el) in order.iter().enumerate() {
            let Some(b) = el.base() else { continue };
            if b.visible == Some(false) {
                continue;
            }
            let pid = Some(page.id.as_str());
            let id = Some(b.id.as_str());
            let needs_alt = matches!(el, Element::Image(_) | Element::Video(_) | Element::Embed(_) | Element::Grid(_) | Element::Chart(_) | Element::Qr(_))
                || matches!(el, Element::Frame(f) if f.content.is_some());
            if needs_alt && blank(b.alt_text.as_ref()) && b.decorative != Some(true) {
                issues.push(issue(A11yIssueCode::AltMissing, Severity::Error, pid, id, None));
            }
            if b.link.is_some() && !matches!(el, Element::Text(_)) && blank(b.alt_text.as_ref()) && blank(b.name.as_ref()) {
                issues.push(issue(A11yIssueCode::LinkName, Severity::Error, pid, id, None));
            }
            let Element::Text(t) = el else { continue };
            if trim(&body_plain_text(&t.body)).is_empty() {
                issues.push(issue(A11yIssueCode::EmptyText, Severity::Warning, pid, id, None));
                continue;
            }
            let wb = world_bounds(&doc.elements, &b.id).unwrap_or_default();
            // Judge by the smallest run that has visible characters.
            let mut small = f64::INFINITY;
            let mut worst: Option<(f64, bool)> = None;
            for p in &t.body.paragraphs {
                for r in &p.runs {
                    if trim(&r.text).is_empty() {
                        continue;
                    }
                    let st = resolve_run_style(&t.body, r);
                    let size = st.size;
                    small = min(small, size);
                    let fg = solid(std::slice::from_ref(&st.fill)).cloned();
                    let bg = backdrop(doc, &page.id, &order, i, wb.x + wb.w / 2.0, wb.y + wb.h / 2.0);
                    let (Some(fg), Some(bg)) = (fg, bg) else { continue };
                    let ratio = contrast_ratio(&fg, &bg);
                    let large = size >= min_px * 2.0 || (st.weight >= 700.0 && size >= min_px * 1.55);
                    if worst.is_none_or(|(w, _)| ratio < w) {
                        worst = Some((ratio, large));
                    }
                }
            }
            if let Some((ratio, large)) = worst {
                let needed = if large { 3.0 } else { 4.5 };
                if ratio < needed {
                    let mut data = Map::new();
                    data.insert("ratio".into(), num(round(ratio * 100.0) / 100.0));
                    data.insert("needed".into(), num(needed));
                    let sev = if ratio < 2.5 { Severity::Error } else { Severity::Warning };
                    issues.push(issue(A11yIssueCode::LowContrast, sev, pid, id, Some(data)));
                }
            }
            if small < min_px {
                let mut data = Map::new();
                data.insert("size".into(), num(round(small)));
                data.insert("min".into(), num(min_px));
                issues.push(issue(A11yIssueCode::SmallText, Severity::Warning, pid, id, Some(data)));
            }
        }
    }
    issues
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ElementKindLabel {
    Text,
    Image,
    Video,
    Shape,
    Line,
    Group,
    Frame,
    Table,
    Chart,
    #[serde(rename = "Photo grid")]
    PhotoGrid,
    Form,
    #[serde(rename = "QR code")]
    QrCode,
    Drawing,
    Embed,
    Element,
}

fn kind_of(el: &Element) -> ElementKindLabel {
    match el {
        Element::Text(_) => ElementKindLabel::Text,
        Element::Image(_) => ElementKindLabel::Image,
        Element::Video(_) => ElementKindLabel::Video,
        Element::Shape(_) => ElementKindLabel::Shape,
        Element::Line(_) => ElementKindLabel::Line,
        Element::Group(_) => ElementKindLabel::Group,
        Element::Frame(_) => ElementKindLabel::Frame,
        Element::Table(_) => ElementKindLabel::Table,
        Element::Chart(_) => ElementKindLabel::Chart,
        Element::Grid(_) => ElementKindLabel::PhotoGrid,
        Element::Form(_) => ElementKindLabel::Form,
        Element::Qr(_) => ElementKindLabel::QrCode,
        Element::Draw(_) => ElementKindLabel::Drawing,
        Element::Embed(_) => ElementKindLabel::Embed,
        Element::Unknown(_) => ElementKindLabel::Element,
    }
}

/// Plain-language description parts (the UI translates `kind`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementDescription {
    pub kind: ElementKindLabel,
    pub label: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rot: f64,
    pub locked: bool,
}

/// `s.replace(/\s+/g, ' ')`.
fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    let mut in_ws = false;
    for c in s.chars() {
        if is_js_space(c) {
            if !in_ws {
                out.push(' ');
            }
            in_ws = true;
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

pub fn describe_element(doc: &DesignDoc, el: &Element) -> ElementDescription {
    let id = el.id().unwrap_or("");
    let b = world_bounds(&doc.elements, id).unwrap_or_default();
    let base = el.base();
    let mut label = base
        .and_then(|b| b.alt_text.as_deref().map(trim).filter(|s| !s.is_empty()))
        .or_else(|| base.and_then(|b| b.name.as_deref().map(trim)))
        .unwrap_or("")
        .to_string();
    if let Element::Text(t) = el {
        label = slice16(trim(&collapse_ws(&body_plain_text(&t.body))), 0, 120);
    }
    let xf = el.xf().cloned().unwrap_or_default();
    ElementDescription {
        kind: kind_of(el),
        label,
        x: round(b.x),
        y: round(b.y),
        w: round(xf.w),
        h: round(xf.h),
        rot: round(xf.rot),
        locked: base.and_then(|b| b.lock.as_ref()).is_some_and(|l| l.move_ == Some(true)),
    }
}
