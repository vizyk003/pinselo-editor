// SPDX-License-Identifier: Apache-2.0
//! PDF import (IM-050) — port of services/api/src/importers/pdf.ts: every page becomes a design page with
//! editable text boxes over a picture of the page's graphics.
//!
//! * **Parser: hayro-syntax** (objects, xref incl. repair, filters, content operators) with hayro-cmap for
//!   ToUnicode/CID maps. It is already in the workspace (oc-render's tests rasterise PDFs with hayro), it is pure
//!   Rust, and the same crate family supplies the rasteriser, so one parse serves text extraction, the
//!   text-stripped copy and the page pictures. lopdf would add a second object model plus a separate renderer;
//!   pdf-rs has no maintained rasteriser.
//! * **Text** = pdf.js `getTextContent` (the TS reads text through pdf.js), ported in [`text`]: same chunking
//!   into items, the same space insertion, item widths and transforms, the same font names. Lines and blocks
//!   are then grouped exactly like the TS.
//! * **Graphics picture**: a copy of the PDF with its `BT … ET` text objects removed ([`strip`], the TS
//!   `stripTextObjects` byte for byte, written as an incremental update), rendered by hayro at 144 dpi where the
//!   TS runs poppler's `pdftocairo`. Images, vector paths and shadings end up in that picture, as in the TS.
//! * **Text colour**: measured like the TS by comparing renders with and without the text.
//!
//! Engine differences (see crates/oc-api/ROUTES.md): the page PNG bytes differ (hayro vs cairo), and so do
//! sampled text colours by anti-aliasing noise (a few 1/255 per channel); bidi reordering of right-to-left text
//! is not applied; vertical (`-V`) CID fonts are read as horizontal.

mod font;
pub mod render;
pub mod strip;
mod tables;
mod text;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::page::{Page as PdfPage, Rotation};
use oc_schema::color::rgb;
use oc_schema::elements::{Element, LockBy, LockMask};
use oc_schema::factory::{MetaInit, TextOpts, base_init, empty_design, make_image, make_text, place_all_elements};
use oc_schema::primitives::{Extra, Paint, Xf};
use oc_schema::text::{Paragraph, Run, RunStyle, RunStyleOverride, TextAlign, TextBody};
use oc_schema::{ColorValue, DesignDoc};

use crate::common::{ImportError, ImportOptions, MediaFn, map_font};
use render::Img;
use text::{FontCache, M, mul};

const PX: f64 = 96.0 / 72.0;
const MAX_PAGES: usize = 60;

/// An inheritable page box as f64 (hayro's `Rect` reads f32), normalised like pdf.js.
fn page_box(page: &PdfPage<'_>, key: &[u8]) -> Option<[f64; 4]> {
    use hayro::hayro_syntax::object::{Array, Dict};
    let mut d: Dict<'_> = page.raw().clone();
    for _ in 0..32 {
        if let Some(a) = d.get::<Array<'_>>(key) {
            let v: Vec<f64> = a.iter::<f64>().collect();
            if v.len() != 4 {
                return None;
            }
            return Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])]);
        }
        d = d.get::<Dict<'_>>(b"Parent")?;
    }
    None
}

/// pdf.js `page.view`: the crop box clipped to the media box (the media box when they do not overlap).
pub(crate) fn view_box(page: &PdfPage<'_>) -> [f64; 4] {
    let mb = page_box(page, b"MediaBox").filter(|b| b[2] > b[0] && b[3] > b[1]).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let Some(cb) = page_box(page, b"CropBox") else { return mb };
    if cb == mb {
        return mb;
    }
    let b = [cb[0].max(mb[0]), cb[1].max(mb[1]), cb[2].min(mb[2]), cb[3].min(mb[3])];
    if b[2] - b[0] > 0.0 && b[3] - b[1] > 0.0 { b } else { mb }
}

/// pdf.js `PageViewport` (no offsets, no flip).
struct Viewport {
    transform: M,
    width: f64,
    height: f64,
}

fn viewport(view: [f64; 4], rotation: i32, scale: f64) -> Viewport {
    let cx = (view[2] + view[0]) / 2.0;
    let cy = (view[3] + view[1]) / 2.0;
    let (a, b, c, d) = match rotation {
        180 => (-1.0, 0.0, 0.0, 1.0),
        90 => (0.0, 1.0, 1.0, 0.0),
        270 => (0.0, -1.0, -1.0, 0.0),
        _ => (1.0, 0.0, 0.0, -1.0),
    };
    let (ox, oy, width, height) = if a == 0.0 {
        ((cy - view[1]).abs() * scale, (cx - view[0]).abs() * scale, (view[3] - view[1]) * scale, (view[2] - view[0]) * scale)
    } else {
        ((cx - view[0]).abs() * scale, (cy - view[1]).abs() * scale, (view[2] - view[0]) * scale, (view[3] - view[1]) * scale)
    };
    Viewport {
        transform: [a * scale, b * scale, c * scale, d * scale, ox - a * scale * cx - c * scale * cy, oy - b * scale * cx - d * scale * cy],
        width,
        height,
    }
}

fn rotation_deg(p: &PdfPage<'_>) -> i32 {
    match p.rotation() {
        Rotation::None => 0,
        Rotation::Horizontal => 90,
        Rotation::Flipped => 180,
        Rotation::FlippedHorizontal => 270,
    }
}

// ---------------------------------------------------------------- font names

/// `fontFromPdfName`: family (mapped to what we ship), weight and italic from a PostScript font name.
pub fn font_from_pdf_name(raw: &str, available: &dyn Fn(&str) -> bool) -> (String, f64, bool) {
    // `^[A-Z]{6}\+`
    let b = raw.as_bytes();
    let name = if b.len() >= 7 && b[6] == b'+' && b[..6].iter().all(u8::is_ascii_uppercase) { &raw[7..] } else { raw };
    let mut parts = name.split(['-', ',']);
    let fam_part = parts.next().unwrap_or("");
    let style_part = parts.next().unwrap_or("");
    const STYLE_WORDS: &[(&[&str], f64)] = &[
        (&["black", "heavy"], 900.0),
        (&["extrabold", "ultrabold"], 800.0),
        (&["semibold", "demibold"], 600.0),
        (&["bold"], 700.0),
        (&["medium"], 500.0),
        (&["extralight", "ultralight"], 200.0),
        (&["light"], 300.0),
        (&["thin", "hairline"], 100.0),
    ];
    let (sl, fl) = (style_part.to_lowercase(), fam_part.to_lowercase());
    let weight = STYLE_WORDS.iter().find(|(ws, _)| ws.iter().any(|w| sl.contains(w) || fl.contains(w))).map_or(400.0, |(_, w)| *w);
    let nl = name.to_lowercase();
    let italic = nl.contains("italic") || nl.contains("oblique");
    // `.replace(/(MT|PS|PSMT|Std|Pro)$/g, '')`
    let mut fam = fam_part.to_owned();
    if let Some(i) = (0..fam.len()).find(|&i| fam.is_char_boundary(i) && matches!(&fam[i..], "MT" | "PS" | "PSMT" | "Std" | "Pro")) {
        fam.truncate(i);
    }
    // `.replace(/([a-z])([A-Z])/g, '$1 $2')`
    let chars: Vec<char> = fam.chars().collect();
    let mut spaced = String::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i].is_ascii_lowercase() && chars[i + 1].is_ascii_uppercase() {
            spaced.push(chars[i]);
            spaced.push(' ');
            spaced.push(chars[i + 1]);
            i += 2;
        } else {
            spaced.push(chars[i]);
            i += 1;
        }
    }
    // `.replace(/(Bold|Italic|Light|Regular|Medium|Semi ?bold)$/i, '')`
    let lower = spaced.to_lowercase();
    if let Some(i) = (0..lower.len()).find(|&i| lower.is_char_boundary(i) && matches!(&lower[i..], "bold" | "italic" | "light" | "regular" | "medium" | "semibold" | "semi bold")) {
        spaced.truncate(i);
    }
    let clean = oc_schema::js::trim(&spaced);
    let family = map_font(Some(if clean.is_empty() { "Inter" } else { clean }), available);
    (family, weight, italic)
}

// ---------------------------------------------------------------- layout

#[derive(Debug, Clone)]
struct Span {
    text: String,
    x: f64,
    y: f64,
    size: f64,
    width: f64,
    font: String,
    rot: f64,
}

struct Line {
    spans: Vec<Span>,
    x: f64,
    y: f64,
    end: f64,
    size: f64,
    font: String,
    rot: f64,
}

fn js_trim_end(s: &str) -> &str {
    s.trim_end_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

fn line_text(l: &Line) -> String {
    let mut acc = String::new();
    for (i, s) in l.spans.iter().enumerate() {
        if i > 0 {
            let prev = &l.spans[i - 1];
            let gap = s.x - (prev.x + prev.width);
            if gap > s.size * 0.15 && !acc.ends_with(' ') && !s.text.starts_with(' ') {
                acc.push(' ');
            }
        }
        acc.push_str(&s.text);
    }
    acc
}

fn cmp_f(a: f64, b: f64) -> std::cmp::Ordering {
    // JS comparator `a - b` (NaN counts as equal).
    let d = a - b;
    if d < 0.0 {
        std::cmp::Ordering::Less
    } else if d > 0.0 {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Equal
    }
}

/// `sampleTextColor`: the colour of the pixels the text adds on top of the graphics.
fn sample_text_color(a: &Img, b: &Img, x: f64, y: f64, w: f64, h: f64) -> ColorValue {
    let x0 = x.floor().max(0.0) as usize;
    let y0 = y.floor().max(0.0) as usize;
    let x1 = ((x + w).ceil().max(0.0) as usize).min(a.width);
    let y1 = ((y + h).ceil().max(0.0) as usize).min(a.height);
    let mut hits: Vec<(usize, i32)> = Vec::new();
    let mut max = 0;
    for yy in y0..y1 {
        for xx in x0..x1 {
            let i = (yy * a.width + xx) * 4;
            let (Some(pa), Some(pb)) = (a.data.get(i..i + 3), b.data.get(i..i + 3)) else { continue };
            let d = (i32::from(pa[0]) - i32::from(pb[0])).abs() + (i32::from(pa[1]) - i32::from(pb[1])).abs() + (i32::from(pa[2]) - i32::from(pb[2])).abs();
            if d > 60 {
                hits.push((i, d));
                if d > max {
                    max = d;
                }
            }
        }
    }
    let (mut sr, mut sg, mut sb, mut n) = (0.0, 0.0, 0.0, 0.0);
    for (i, d) in hits {
        if f64::from(d) < f64::from(max) * 0.85 {
            continue;
        }
        sr += f64::from(a.data[i]);
        sg += f64::from(a.data[i + 1]);
        sb += f64::from(a.data[i + 2]);
        n += 1.0;
    }
    if n > 0.0 { rgb(sr / n / 255.0, sg / n / 255.0, sb / n / 255.0, 1.0) } else { rgb(0.12, 0.13, 0.16, 1.0) }
}

fn spread(a: &[f64]) -> f64 {
    a.iter().copied().fold(f64::NEG_INFINITY, f64::max) - a.iter().copied().fold(f64::INFINITY, f64::min)
}

#[allow(clippy::too_many_arguments)]
fn page_text_elements(spans: Vec<Span>, fit: f64, ox: f64, oy: f64, scale_img: f64, with_text: &Img, without: &Img, available: &dyn Fn(&str) -> bool) -> Vec<Element> {
    let mut spans = spans;
    spans.sort_by(|a, b| cmp_f(a.y, b.y).then_with(|| cmp_f(a.x, b.x)));
    let mut lines: Vec<Line> = Vec::new();
    for s in spans {
        let found = lines.iter_mut().find(|x| {
            (x.rot - s.rot).abs() < 1.0
                && (x.y - s.y).abs() < s.size * 0.3
                && x.font == s.font
                && (x.size - s.size).abs() < s.size * 0.06
                && s.x >= x.end - s.size * 0.5
                && s.x - x.end < s.size * 1.2
        });
        match found {
            Some(l) => {
                l.end = l.end.max(s.x + s.width);
                l.spans.push(s);
            }
            None => lines.push(Line { x: s.x, y: s.y, end: s.x + s.width, size: s.size, font: s.font.clone(), rot: s.rot, spans: vec![s] }),
        }
    }
    lines.sort_by(|a, b| cmp_f(a.y, b.y).then_with(|| cmp_f(a.x, b.x)));
    let mut used = vec![false; lines.len()];
    let mut els = Vec::new();
    for li in 0..lines.len() {
        if used[li] {
            continue;
        }
        used[li] = true;
        let l = &lines[li];
        let mut block = vec![li];
        let mut last = li;
        for ci in 0..lines.len() {
            let cand = &lines[ci];
            if used[ci] || cand.font != l.font || (cand.size - l.size).abs() > l.size * 0.06 || (cand.rot - l.rot).abs() > 1.0 {
                continue;
            }
            let dy = cand.y - lines[last].y;
            if dy <= 0.0 || dy > l.size * 1.9 {
                continue;
            }
            let same_left = (cand.x - l.x).abs() < l.size * 0.6;
            let same_center = ((cand.x + cand.end) / 2.0 - (l.x + l.end) / 2.0).abs() < l.size * 0.6;
            if !same_left && !same_center {
                continue;
            }
            block.push(ci);
            used[ci] = true;
            last = ci;
        }
        let (family, weight, italic) = font_from_pdf_name(&l.font, available);
        let first = &lines[block[0]];
        let lastl = &lines[*block.last().expect("block")];
        let spacing = if block.len() > 1 { (lastl.y - first.y) / (block.len() - 1) as f64 } else { l.size * 1.2 };
        let line_height = 0.8f64.max(2.5f64.min(spacing / l.size));
        let lefts: Vec<f64> = block.iter().map(|&b| lines[b].x).collect();
        let centers: Vec<f64> = block.iter().map(|&b| (lines[b].x + lines[b].end) / 2.0).collect();
        let align = if block.len() > 1 && spread(&centers) + 0.5 < spread(&lefts) { TextAlign::Center } else { TextAlign::Start };
        let x0 = lefts.iter().copied().fold(f64::INFINITY, f64::min);
        let x1 = block.iter().map(|&b| lines[b].end).fold(f64::NEG_INFINITY, f64::max);
        let color = sample_text_color(
            with_text,
            without,
            (x0 - 1.0) * scale_img,
            (l.y - l.size) * scale_img,
            (x1 - x0 + 2.0) * scale_img,
            (lastl.y - l.y + l.size * 1.25) * scale_img,
        );
        let style = RunStyle {
            family,
            weight,
            italic,
            size: l.size * fit,
            fill: Paint::solid_color(color),
            tracking: 0.0,
            underline: None,
            strike: None,
            case_transform: None,
            highlight: None,
            baseline: None,
            extra: Extra::new(),
        };
        let body = TextBody {
            default_run: style,
            paragraphs: block
                .iter()
                .map(|&b| Paragraph {
                    runs: vec![Run { text: js_trim_end(&line_text(&lines[b])).to_owned(), style: RunStyleOverride::default(), link: None, extra: Extra::new() }],
                    align: align.clone(),
                    line_height,
                    space_after: 0.0,
                    list: None,
                    extra: Extra::new(),
                })
                .collect(),
            extra: Extra::new(),
        };
        let top = l.y - l.size * (0.82 + (line_height - 1.15) / 2.0);
        let xf = Xf::new(ox + x0 * fit, oy + top * fit, 8f64.max((x1 - x0) * fit * 1.06 + 4.0), 8f64.max(spacing * block.len() as f64 * fit), l.rot);
        els.push(Element::Text(make_text(base_init(xf), TextOpts { body: Some(body), ..Default::default() })));
    }
    els
}

/// `importPdf`.
pub fn import_pdf(bytes: &[u8], opts: &ImportOptions<'_>, media: &mut MediaFn<'_>) -> Result<DesignDoc, ImportError> {
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| ImportError::invalid(format!("{e:?}")))?;
    let pages = pdf.pages();
    if pages.is_empty() {
        return Err(ImportError::invalid("no pages"));
    }
    let count = pages.len().min(MAX_PAGES);
    let vps: Vec<Viewport> = pages.iter().take(count).map(|p| viewport(view_box(p), rotation_deg(p), PX)).collect();
    let w = js_round(vps[0].width);
    let h = js_round(vps[0].height);
    let mut doc = empty_design(&MetaInit {
        w,
        h,
        pages: Some(count),
        title: Some(opts.title.clone().unwrap_or_else(|| "Imported PDF".into())),
        ..Default::default()
    });
    let mut page_ids: Vec<(String, String)> = doc.pages.values().map(|p| (p.order.clone(), p.id.clone())).collect();
    page_ids.sort_by(|a, b| oc_schema::js::str_cmp(&a.0, &b.0));

    let stripped = strip::strip_pdf_text(bytes).unwrap_or_else(|| bytes.to_vec());
    let bg_imgs = render::render_pages(&stripped, count);
    let full_imgs = render::render_pages(bytes, count);
    let mut fonts = FontCache::default();

    for (pi, page) in pages.iter().take(count).enumerate() {
        let pn = pi + 1;
        let vp = &vps[pi];
        let fit = (w / vp.width).min(h / vp.height);
        let ox = (w - vp.width * fit) / 2.0;
        let oy = (h - vp.height * fit) / 2.0;
        let (Some(without), Some(with_text)) = (&bg_imgs[pi], &full_imgs[pi]) else {
            return Err(ImportError::invalid(format!("page {pn} could not be rendered")));
        };
        let scale_img = with_text.width as f64 / vp.width;

        let mut els: Vec<Element> = Vec::new();
        if let Some(asset) = media(&render::png(without), &format!("page-{pn}.png")) {
            let mut bg = make_image(base_init(Xf::new(ox, oy, vp.width * fit, vp.height * fit, 0.0)), &asset);
            bg.base.name = Some(format!("Page {pn} graphics"));
            bg.base.lock = Some(LockMask { move_: Some(true), resize: None, rotate: None, content: None, style: None, delete: None, by: LockBy::User, extra: Extra::new() });
            els.push(Element::Image(bg));
        }

        let view = view_box(page);
        let items = page.page_stream().map(|c| text::text_content(c, page.resources(), view, &mut fonts)).unwrap_or_default();
        let mut spans = Vec::new();
        for it in items {
            if it.str.is_empty() {
                continue;
            }
            let m = mul(&vp.transform, &it.transform);
            let size = m[2].hypot(m[3]);
            if size < 1.0 {
                continue;
            }
            spans.push(Span { text: it.str, x: m[4], y: m[5], size, width: it.width * PX, font: it.font_name, rot: m[1].atan2(m[0]) * 180.0 / std::f64::consts::PI });
        }
        els.extend(page_text_elements(spans, fit, ox, oy, scale_img, with_text, without, opts.font_available));

        let pid = page_ids[pi].1.clone();
        if let Some(p) = doc.pages.get_mut(&pid) {
            p.background = vec![Paint::solid_color(rgb(1.0, 1.0, 1.0, 1.0))];
        }
        place_all_elements(&pid, &mut els, None);
        for e in els {
            if let Some(id) = e.id().map(str::to_owned) {
                doc.elements.insert(id, e);
            }
        }
    }
    Ok(doc)
}

/// `Math.round`.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}
