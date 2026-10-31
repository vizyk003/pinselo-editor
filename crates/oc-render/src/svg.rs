// SPDX-License-Identifier: Apache-2.0
//! SVG export (PL-036, `packages/render-canvaskit/src/svg.ts`) from the same display list the raster painter draws.
//! Text is emitted as real `<text>`/`<tspan>` lines using the layout's line breaks, so it stays selectable and
//! searchable.
//!
//! Beyond the TS writer (which ignored them), the geometry follows the painter: inside/outside stroke alignment
//! (clipped double-width strokes), the text effects (background boxes, hollow/outline strokes, shadow/neon/
//! echo/glitch as SVG filters) and image edits (baked into the embedded raster).

use std::fmt::Write as _;

use base64::Engine as _;
use oc_geometry::jsmath::f3;
use oc_geometry::{MarkerMode, line_marker_shape, trim_line_for_markers};
use oc_schema::color::to_rgba;
use oc_schema::elements::{CellVAlign, Effect, LineMarker};
use oc_schema::primitives::{ColorValue, Paint, Rect, Stroke, StrokeAlign};
use oc_schema::text::{CaseTransform, Run, TextBody, TextEffect, resolve_run_style};

use crate::assets::{AssetProvider, is_svg};
use crate::color_matrix::{edit_matrix, needs_tone_shader};
use crate::displaylist::{DLItem, DisplayList, ResolvedPaint};
use crate::raster::Renderer;

/// A font file embedded as `@font-face`.
#[derive(Debug, Clone)]
pub struct SvgFont {
    pub family: String,
    pub weight: f64,
    pub italic: bool,
    pub bytes: Vec<u8>,
}

/// Maps an asset key to a URL to reference instead of inlining the bytes.
pub type AssetHref = dyn Fn(&str) -> Option<String>;

/// Options of [`display_list_to_svg`].
#[derive(Default)]
pub struct SvgOptions<'a> {
    /// Font files to embed as `@font-face`.
    pub fonts: Vec<SvgFont>,
    /// Reference images by URL instead of inlining them (websites).
    pub asset_href: Option<&'a AssetHref>,
    /// Leave out the `<style>` font block (the host page declares the fonts once).
    pub external_fonts: bool,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A family name as a quoted CSS string (SVG-01, `cssFamily`): unquoted, a name with a token that starts with a
/// digit ("Source Serif 4", "M PLUS 1p") makes the whole font-family declaration invalid.
pub fn css_family(f: &str) -> String {
    format!("'{}'", f.replace('\\', "\\\\").replace('\'', "\\'").replace(['\n', '\r', '\u{c}'], " "))
}

fn n(v: f64) -> String {
    f3(v)
}

fn color(c: &ColorValue) -> (String, f64) {
    let x = to_rgba(c);
    let ch = |v: f64| oc_schema::js::num_to_string(oc_schema::js::round(v * 255.0));
    (format!("rgb({},{},{})", ch(x.r), ch(x.g), ch(x.b)), x.a)
}

/// Mime type of encoded image bytes.
pub fn sniff_mime(b: &[u8]) -> &'static str {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if b.starts_with(&[0xff, 0xd8]) {
        "image/jpeg"
    } else if b.starts_with(b"GIF8") {
        "image/gif"
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else if is_svg(b) {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

fn data_uri(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// UTF-16 slice of a string (`String#slice` on code units).
fn slice16(s: &str, a: usize, b: usize) -> String {
    let u: Vec<u16> = s.encode_utf16().collect();
    let (a, b) = (a.min(u.len()), b.min(u.len()));
    if b <= a {
        return String::new();
    }
    String::from_utf16_lossy(&u[a..b])
}

struct Writer<'a> {
    defs: Vec<String>,
    out: Vec<String>,
    uid: usize,
    opts: &'a SvgOptions<'a>,
}

impl Writer<'_> {
    fn id(&mut self, p: &str) -> String {
        self.uid += 1;
        format!("{p}{}", self.uid)
    }

    fn paint(&mut self, p: &ResolvedPaint) -> (String, f64) {
        match p {
            ResolvedPaint::Solid { color: c } => color(c),
            ResolvedPaint::Linear { stops, angle, bbox } => {
                let gid = self.id("g");
                let s = Self::stops(stops);
                let ((x1, y1), (x2, y2)) = crate::raster::linear_points(*angle, bbox);
                self.defs.push(format!(r#"<linearGradient id="{gid}" gradientUnits="userSpaceOnUse" x1="{}" y1="{}" x2="{}" y2="{}">{s}</linearGradient>"#, n(x1), n(y1), n(x2), n(y2)));
                (format!("url(#{gid})"), 1.0)
            }
            ResolvedPaint::Radial { stops, bbox } => {
                let gid = self.id("g");
                let s = Self::stops(stops);
                self.defs.push(format!(
                    r#"<radialGradient id="{gid}" gradientUnits="userSpaceOnUse" cx="{}" cy="{}" r="{}">{s}</radialGradient>"#,
                    n(bbox.x + bbox.w / 2.0),
                    n(bbox.y + bbox.h / 2.0),
                    n(bbox.w.max(bbox.h) / 2.0)
                ));
                (format!("url(#{gid})"), 1.0)
            }
            ResolvedPaint::Image { .. } => ("#d9dde3".into(), 1.0),
        }
    }

    fn stops(stops: &[oc_schema::primitives::GradientStop]) -> String {
        stops
            .iter()
            .map(|s| {
                let (c, o) = color(&s.color);
                format!(r#"<stop offset="{}" stop-color="{c}" stop-opacity="{}"/>"#, n(s.t), n(o))
            })
            .collect()
    }

    fn stroke_attrs(&mut self, s: &Stroke, p: &ResolvedPaint, width: f64) -> String {
        let (attr, op) = self.paint(p);
        let dash = match &s.dash {
            Some(d) if !d.is_empty() => format!(r#" stroke-dasharray="{}""#, d.iter().map(|v| n(v * s.width.max(1.0))).collect::<Vec<_>>().join(" ")),
            _ => String::new(),
        };
        format!(r#"stroke="{attr}" stroke-opacity="{}" stroke-width="{}" stroke-linecap="{}" stroke-linejoin="{}"{dash}"#, n(op), n(width), s.cap.as_str(), s.join.as_str())
    }

    fn clip_def(&mut self, d: &str, evenodd_outside: bool) -> String {
        let cid = self.id("c");
        if evenodd_outside {
            self.defs.push(format!(r#"<clipPath id="{cid}"><path d="M-100000 -100000 L200000 -100000 L200000 200000 L-100000 200000 Z {d}" clip-rule="evenodd"/></clipPath>"#));
        } else {
            self.defs.push(format!(r#"<clipPath id="{cid}"><path d="{d}"/></clipPath>"#));
        }
        cid
    }
}

/// Write a display list as an SVG document.
pub fn display_list_to_svg(r: &mut Renderer, dl: &DisplayList, assets: &dyn AssetProvider, opts: &SvgOptions<'_>) -> String {
    let mut w = Writer { defs: Vec::new(), out: Vec::new(), uid: 0, opts };
    for bg in &dl.background {
        let (a, o) = w.paint(bg);
        w.out.push(format!(r#"<rect width="{}" height="{}" fill="{a}" fill-opacity="{}"/>"#, n(dl.width), n(dl.height), n(o)));
    }
    // Each 'save'/'layer' opens a <g> closed by its 'restore'; a 'clip' adds a nested <g> to the current level.
    let mut stack: Vec<usize> = Vec::new();
    // Open masks: where their mask content starts in `out`, and the mask type.
    let mut masks: Vec<(usize, &'static str)> = Vec::new();
    for it in &dl.items {
        match it {
            DLItem::Mask { mode, .. } => {
                masks.push((w.out.len(), if *mode == oc_schema::elements::MaskMode::Luminance { "luminance" } else { "alpha" }));
                stack.push(0);
            }
            DLItem::MaskContent => {
                // The mask's items move into a <mask> definition; the content opens a group seen through it.
                if let Some((start, kind)) = masks.pop() {
                    let inner: Vec<String> = w.out.drain(start.min(w.out.len())..).collect();
                    let mid = w.id("m");
                    w.defs.push(format!(
                        r#"<mask id="{mid}" maskUnits="userSpaceOnUse" x="-100000" y="-100000" width="200000" height="200000" mask-type="{kind}" style="mask-type:{kind}">{}</mask>"#,
                        inner.concat()
                    ));
                    w.out.push(format!(r#"<g mask="url(#{mid})">"#));
                    match stack.last_mut() {
                        Some(top) => *top += 1,
                        None => stack.push(1),
                    }
                }
            }
            DLItem::Save { matrix } => {
                w.out.push(format!(r#"<g transform="matrix({})">"#, matrix.iter().map(|v| n(*v)).collect::<Vec<_>>().join(" ")));
                stack.push(1);
            }
            DLItem::Layer { opacity, blend, effects, .. } => {
                let blurs: Vec<String> = effects.iter().filter_map(|e| if let Effect::Blur(b) = e { Some(format!(r#"<feGaussianBlur stdDeviation="{}"/>"#, n(b.radius / 2.0))) } else { None }).collect();
                let shadows: Vec<String> = effects
                    .iter()
                    .filter_map(|e| {
                        if let Effect::DropShadow(s) = e {
                            let (c, o) = color(&s.color);
                            Some(format!(r#"<feDropShadow dx="{}" dy="{}" stdDeviation="{}" flood-color="{c}" flood-opacity="{}"/>"#, n(s.dx), n(s.dy), n(s.blur / 2.0), n(o)))
                        } else {
                            None
                        }
                    })
                    .collect();
                let mut filter = String::new();
                if !blurs.is_empty() || !shadows.is_empty() {
                    let fid = w.id("f");
                    w.defs.push(format!(r#"<filter id="{fid}" x="-50%" y="-50%" width="200%" height="200%">{}{}</filter>"#, blurs.concat(), shadows.concat()));
                    filter = format!(r#" filter="url(#{fid})""#);
                }
                let mode = if blend.as_str() != "normal" { format!(r#" style="mix-blend-mode:{}""#, blend.as_str()) } else { String::new() };
                w.out.push(format!(r#"<g opacity="{}"{mode}{filter}>"#, n(*opacity)));
                stack.push(1);
            }
            DLItem::Restore => {
                for _ in 0..stack.pop().unwrap_or(0) {
                    w.out.push("</g>".into());
                }
            }
            DLItem::Clip { path } => {
                let cid = w.clip_def(path, false);
                w.out.push(format!(r#"<g clip-path="url(#{cid})">"#));
                if let Some(top) = stack.last_mut() {
                    *top += 1;
                } else {
                    stack.push(1);
                }
            }
            DLItem::Path { path, fills, stroke, .. } => {
                for f in fills {
                    if let ResolvedPaint::Image { asset, fit, bbox } = f {
                        image_fill(&mut w, r, path, asset, fit, bbox, assets);
                        continue;
                    }
                    let (a, o) = w.paint(f);
                    w.out.push(format!(r#"<path d="{path}" fill="{a}" fill-opacity="{}"/>"#, n(o)));
                }
                if let Some(s) = stroke {
                    match s.stroke.align {
                        StrokeAlign::Inside | StrokeAlign::Outside => {
                            let cid = w.clip_def(path, s.stroke.align == StrokeAlign::Outside);
                            let attrs = w.stroke_attrs(&s.stroke, &s.paint, s.stroke.width * 2.0);
                            w.out.push(format!(r#"<g clip-path="url(#{cid})"><path d="{path}" fill="none" {attrs}/></g>"#));
                        }
                        _ => {
                            let attrs = w.stroke_attrs(&s.stroke, &s.paint, s.stroke.width);
                            w.out.push(format!(r#"<path d="{path}" fill="none" {attrs}/>"#));
                        }
                    }
                }
            }
            DLItem::Line { points, stroke, paint, start_marker, end_marker, .. } => {
                let d = trim_line_for_markers(points, start_marker, end_marker, stroke.width)
                    .iter()
                    .enumerate()
                    .map(|(i, [x, y])| format!("{}{} {}", if i > 0 { "L" } else { "M" }, n(*x), n(*y)))
                    .collect::<Vec<_>>()
                    .join(" ");
                let width = if stroke.align == StrokeAlign::Center || !stroke.align.is_known() { stroke.width } else { stroke.width * 2.0 };
                let attrs = w.stroke_attrs(stroke, paint, width);
                w.out.push(format!(r#"<path d="{d}" fill="none" {attrs}/>"#));
                if points.len() >= 2 {
                    let fill = w.paint(paint).0;
                    let k = points.len();
                    w.out.push(marker(start_marker, points[1], points[0], stroke.width, &fill));
                    w.out.push(marker(end_marker, points[k - 2], points[k - 1], stroke.width, &fill));
                }
            }
            DLItem::Image { asset, dst, crop, edits, clip, .. } => {
                let crop = Rect::from(crop);
                image(&mut w, r, asset, dst, &crop, edits.as_ref(), clip.as_deref(), assets);
            }
            DLItem::Video { asset, dst, crop, clip, edits, .. } => {
                // Videos export their poster frame (static output).
                let crop = Rect::from(crop);
                image(&mut w, r, &format!("{asset}/poster"), dst, &crop, edits.as_ref(), clip.as_deref(), assets);
            }
            DLItem::Text { key, body, bbox, effect, valign, curve, fit, .. } => {
                let s = text(&mut w, r, key, body, bbox, effect.as_ref(), valign.as_ref(), *curve, fit.is_some());
                w.out.push(s);
            }
            DLItem::Qr { modules, size, bbox, fg, bg, .. } => {
                let quiet = 2.0;
                let cell = bbox.w.min(bbox.h) / (*size as f64 + quiet * 2.0);
                let ox = bbox.x + (bbox.w - cell * *size as f64) / 2.0;
                let oy = bbox.y + (bbox.h - cell * *size as f64) / 2.0;
                let mut d = String::new();
                for rr in 0..*size {
                    for c in 0..*size {
                        if modules.get(rr * size + c).copied().unwrap_or(0) != 0 {
                            let _ = write!(d, "M{} {}h{}v{}h{}Z", n(ox + c as f64 * cell), n(oy + rr as f64 * cell), n(cell), n(cell), n(-cell));
                        }
                    }
                }
                w.out.push(format!(r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/><path d="{d}" fill="{}"/>"#, n(bbox.x), n(bbox.y), n(bbox.w), n(bbox.h), color(bg).0, color(fg).0));
            }
            DLItem::Placeholder { bbox, .. } => {
                w.out.push(format!(r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#999" fill-opacity="0.25"/>"##, n(bbox.x), n(bbox.y), n(bbox.w), n(bbox.h)));
            }
        }
    }
    while let Some(k) = stack.pop() {
        for _ in 0..k {
            w.out.push("</g>".into());
        }
    }
    let mut font_css = String::new();
    if !opts.external_fonts {
        for f in &opts.fonts {
            let _ = write!(
                font_css,
                r#"@font-face{{font-family:"{}";font-weight:{};font-style:{};src:url(data:font/ttf;base64,{}) format("truetype")}}"#,
                f.family,
                oc_schema::js::num_to_string(f.weight),
                if f.italic { "italic" } else { "normal" },
                base64::engine::general_purpose::STANDARD.encode(&f.bytes)
            );
        }
    }
    let style = if font_css.is_empty() { String::new() } else { format!("<style>{font_css}</style>") };
    let mut doc = vec![
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" width="{}" height="{}">"#, n(dl.width), n(dl.height), n(dl.width), n(dl.height)),
        format!("<defs>{style}{}</defs>", w.defs.concat()),
    ];
    doc.extend(w.out);
    doc.push("</svg>".into());
    doc.join("\n")
}

fn marker(kind: &LineMarker, from: [f64; 2], at: [f64; 2], width: f64, fill: &str) -> String {
    let Some(m) = line_marker_shape(kind, width) else { return String::new() };
    let ang = (at[1] - from[1]).atan2(at[0] - from[0]).to_degrees();
    let look = match m.mode {
        MarkerMode::Fill => format!(r#"fill="{fill}""#),
        MarkerMode::Stroke => format!(r#"fill="none" stroke="{fill}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round""#, n(width)),
    };
    format!(r#"<g transform="translate({} {}) rotate({})"><path d="{}" {look}/></g>"#, n(at[0]), n(at[1]), n(ang), m.d)
}

#[allow(clippy::too_many_arguments)]
fn image(w: &mut Writer<'_>, r: &mut Renderer, asset: &str, dst: &Rect, crop: &Rect, edits: Option<&oc_schema::elements::ImageEditStack>, clip: Option<&str>, assets: &dyn AssetProvider) {
    let clip_attr = clip.map(|c| format!(r#" clip-path="url(#{})""#, w.clip_def(c, false))).unwrap_or_default();
    let href = w.opts.asset_href.and_then(|f| f(asset));
    let adjust = edits.and_then(|e| e.adjust.as_ref());
    let edited = edit_matrix(edits).is_some() || needs_tone_shader(adjust) || adjust.is_some_and(|a| a.blur.is_some_and(|b| b > 0.0) || a.vignette.is_some_and(|v| v > 0.0));
    let (href, crop) = if edited {
        // Edits are baked into an embedded raster of the crop (the painter's own pipeline).
        match r.bake_image(asset, crop, edits, assets) {
            Some(p) => (data_uri("image/png", &crate::raster::encode_png(&p)), Rect::UNIT),
            None => (String::new(), *crop),
        }
    } else if let Some(h) = href {
        (esc(&h), *crop)
    } else {
        match assets.asset(asset) {
            Some(b) => (data_uri(sniff_mime(&b), &b), *crop),
            // A recoloured SVG variant the host has no bytes for: recolour the base asset's SVG source.
            None => match oc_schema::svg::parse_recolor_key(asset).and_then(|k| assets.asset(&k.asset).map(|b| (k, b))) {
                Some((k, b)) if is_svg(&b) => {
                    let svg = oc_schema::svg::recolor_svg(&String::from_utf8_lossy(&b), &k.map);
                    (data_uri("image/svg+xml", svg.as_bytes()), *crop)
                }
                Some((_, b)) => (data_uri(sniff_mime(&b), &b), *crop),
                None => (String::new(), *crop),
            },
        }
    };
    let Rect { x, y, w: dw, h: dh } = *dst;
    if href.is_empty() {
        w.out.push(format!(r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#e2e5ea"{clip_attr}/>"##, n(x), n(y), n(dw), n(dh)));
        return;
    }
    // Crop: scale the full image so the crop rect maps onto dst, clipped to dst.
    let fw = dw / crop.w;
    let fh = dh / crop.h;
    let cid = w.id("c");
    w.defs.push(format!(r#"<clipPath id="{cid}"><rect x="{}" y="{}" width="{}" height="{}"/></clipPath>"#, n(x), n(y), n(dw), n(dh)));
    w.out.push(format!(
        r#"<g{clip_attr}><g clip-path="url(#{cid})"><image href="{href}" x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none"/></g></g>"#,
        n(x - crop.x * fw),
        n(y - crop.y * fh),
        n(fw),
        n(fh)
    ));
}

/// An image fill (the painter's image shader): placed by its fit mode, tiled for `tile`, clipped to the path.
fn image_fill(w: &mut Writer<'_>, r: &mut Renderer, path: &str, asset: &str, fit: &oc_schema::primitives::ImageFit, b: &Rect, assets: &dyn AssetProvider) {
    let size = r.image_size(asset, assets);
    let href = match w.opts.asset_href.and_then(|f| f(asset)) {
        Some(h) => Some(esc(&h)),
        None => assets.asset(asset).map(|bytes| data_uri(sniff_mime(&bytes), &bytes)),
    };
    let (Some((iw, ih)), Some(href)) = (size, href) else {
        w.out.push(format!(r##"<path d="{path}" fill="#d9dde3"/>"##));
        return;
    };
    let s = crate::raster::image_fit_scale(fit, b, iw, ih);
    let (dw, dh) = (iw * s, ih * s);
    let (x0, y0) = (b.x + (b.w - dw) / 2.0, b.y + (b.h - dh) / 2.0);
    let cid = w.clip_def(path, false);
    let mut g = format!(r#"<g clip-path="url(#{cid})">"#);
    let one = |x: f64, y: f64| format!(r#"<image href="{href}" x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none"/>"#, n(x), n(y), n(dw), n(dh));
    if *fit == oc_schema::primitives::ImageFit::Tile && dw > 0.0 && dh > 0.0 {
        let bounds = crate::path::parse(path).and_then(|p| p.bounds());
        if let Some((bx0, by0, bx1, by1)) = bounds {
            let (i0, j0) = (((bx0 - x0) / dw).floor() as i64, ((by0 - y0) / dh).floor() as i64);
            let (i1, j1) = (((bx1 - x0) / dw).ceil() as i64, ((by1 - y0) / dh).ceil() as i64);
            if (i1 - i0) * (j1 - j0) <= 10_000 {
                for j in j0..j1 {
                    for i in i0..i1 {
                        g.push_str(&one(x0 + i as f64 * dw, y0 + j as f64 * dh));
                    }
                }
            }
        }
    } else {
        g.push_str(&one(x0, y0));
    }
    g.push_str("</g>");
    w.out.push(g);
}

/// A filter drawing the text's shadows (effect shadow, neon, echo, glitch) under it; `None` without one.
fn shadow_filter(w: &mut Writer<'_>, effect: Option<&TextEffect>, body: &TextBody) -> Option<String> {
    let first = body.paragraphs.iter().flat_map(|p| p.runs.first()).next().map(|r| resolve_run_style(body, r)).unwrap_or_else(|| body.default_run.clone());
    let fill = oc_text::paint_to_color(Some(&first.fill));
    let (fc, fo) = color(&fill);
    let shadows: Vec<(String, f64, f64, f64, f64)> = match effect? {
        TextEffect::Shadow(e) => {
            let (c, o) = color(&e.color);
            vec![(c, o, e.dx, e.dy, e.blur)]
        }
        TextEffect::Neon(e) => (1..=3).map(|i| (fc.clone(), fo, 0.0, 0.0, (e.intensity / 100.0) * first.size * 0.15 * f64::from(i))).collect(),
        TextEffect::Echo(e) => [3.0, 2.0, 1.0].into_iter().map(|i: f64| (fc.clone(), fo * (0.15 + 0.2 * (3.0 - i)), e.dx * i, e.dy * i, 0.0)).collect(),
        TextEffect::Glitch(e) => {
            let o = (e.offset / 100.0) * first.size;
            vec![("rgb(0,220,255)".into(), 0.85, -o, 0.0, 0.0), ("rgb(255,0,170)".into(), 0.85, o, 0.0, 0.0)]
        }
        _ => return None,
    };
    let fid = w.id("t");
    let mut prims = String::new();
    let mut merge = String::new();
    for (i, (c, o, dx, dy, blur)) in shadows.iter().enumerate() {
        let _ = write!(
            prims,
            r#"<feOffset in="SourceAlpha" dx="{}" dy="{}" result="o{i}"/><feGaussianBlur in="o{i}" stdDeviation="{}" result="b{i}"/><feFlood flood-color="{c}" flood-opacity="{}"/><feComposite in2="b{i}" operator="in" result="s{i}"/>"#,
            n(*dx),
            n(*dy),
            n(*blur),
            n(*o)
        );
        let _ = write!(merge, r#"<feMergeNode in="s{i}"/>"#);
    }
    w.defs.push(format!(r#"<filter id="{fid}" x="-50%" y="-50%" width="200%" height="200%">{prims}<feMerge>{merge}<feMergeNode in="SourceGraphic"/></feMerge></filter>"#));
    Some(fid)
}

/// Lines + runs from the layout: each line becomes `<text>`, each style run a `<tspan>`.
#[allow(clippy::too_many_arguments)]
fn text(w: &mut Writer<'_>, r: &mut Renderer, key: &str, body: &TextBody, b: &Rect, effect: Option<&TextEffect>, valign: Option<&CellVAlign>, curve: Option<f64>, fit: bool) -> String {
    if let Some(cv) = curve {
        return curved_text(r, key, body, b, cv);
    }
    let (layout, scale) = if fit { r.text.layout_fit(key, body, b.w, b.h, effect) } else { (r.text.layout(key, body, b.w, effect), 1.0) };
    let dy = match valign {
        Some(CellVAlign::Middle) => (b.h - layout.height) / 2.0,
        Some(CellVAlign::Bottom) => b.h - layout.height,
        _ => 0.0,
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(TextEffect::Background(bg)) = effect {
        let (c, o) = color(&bg.color);
        let pad = bg.padding;
        for p in &layout.paras {
            for m in &p.lines {
                let d = crate::path::PathData::rrect(b.x + p.x + m.left - pad, b.y + dy + p.y + m.baseline - m.ascent - pad / 2.0, m.width + pad * 2.0, m.ascent + m.descent + pad, bg.radius, bg.radius);
                parts.push(format!(r#"<path d="{}" fill="{c}" fill-opacity="{}"/>"#, d.to_svg(n), n(o)));
            }
        }
    }
    let look = match effect {
        Some(TextEffect::Hollow(h)) => Some((None, h.width)),
        Some(TextEffect::Outline(o)) => Some((Some(color(&o.color)), o.width)),
        _ => None,
    };
    let mut lines: Vec<String> = Vec::new();
    for (pi, p) in body.paragraphs.iter().enumerate() {
        let Some(lp) = layout.paras.get(pi) else { continue };
        let full: String = p.runs.iter().map(|r| r.text.as_str()).collect();
        for m in &lp.lines {
            let seg = slice16(&full, m.start, m.end_no_ws);
            if oc_schema::js::trim(&seg).is_empty() {
                continue;
            }
            let mut spans = String::new();
            let mut pos = 0usize;
            for run in &p.runs {
                let len = oc_schema::js::utf16_len(&run.text);
                let a = pos.max(m.start);
                let bb = (pos + len).min(m.end_no_ws);
                if bb > a {
                    spans.push_str(&tspan(body, run, &slice16(&full, a, bb), scale, look.as_ref()));
                }
                pos += len;
            }
            lines.push(format!(r#"<text x="{}" y="{}" xml:space="preserve">{spans}</text>"#, n(b.x + lp.x + m.left), n(b.y + dy + lp.y + m.baseline)));
        }
    }
    match shadow_filter(w, effect, body) {
        Some(fid) => parts.push(format!(r#"<g filter="url(#{fid})">{}</g>"#, lines.concat())),
        None => parts.extend(lines),
    }
    parts.concat()
}

/// Curved text as glyph-positioned characters (one `<text>` per character, rotated onto the arc).
fn curved_text(r: &mut Renderer, key: &str, body: &TextBody, b: &Rect, curve: f64) -> String {
    let Some(cl) = r.text.curved(key, body, curve) else { return String::new() };
    let chars: Vec<char> = body.paragraphs.iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join(" ").chars().collect();
    let s = (b.w / cl.width).min(b.h / cl.height);
    let ox = b.x + (b.w - cl.width * s) / 2.0;
    let oy = b.y + (b.h - cl.height * s) / 2.0;
    // Glyphs map 1:1 to characters for simple scripts; otherwise fall back to nothing rather than garble.
    if chars.len() != cl.glyphs.len() {
        return String::new();
    }
    cl.glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let st = &cl.styles[g.style];
            let fill = match &st.fill {
                Paint::Solid(p) => color(&p.color).0,
                _ => "#000".into(),
            };
            let a = g.sin.atan2(g.cos).to_degrees();
            format!(
                r#"<text transform="translate({} {}) rotate({}) scale({})" font-family="{}, sans-serif" font-size="{}" font-weight="{}"{} fill="{fill}">{}</text>"#,
                n(ox + g.tx * s),
                n(oy + g.ty * s),
                n(a),
                n(s),
                esc(&css_family(&st.family)),
                n(g.size),
                oc_schema::js::num_to_string(st.weight),
                if st.italic { r#" font-style="italic""# } else { "" },
                esc(&chars[i].to_string())
            )
        })
        .collect()
}

/// `look`: `(None, width)` = hollow (stroke in the text colour), `(Some(colour), width)` = outline under the fill.
fn tspan(body: &TextBody, run: &Run, text: &str, scale: f64, look: Option<&(Option<(String, f64)>, f64)>) -> String {
    let mut s = resolve_run_style(body, run);
    if scale != 1.0 {
        s.size *= scale;
    }
    // Superscript / subscript: smaller, and raised or lowered (as the layout does).
    let shift = oc_schema::text::baseline_offset(s.baseline.as_ref(), s.size);
    if shift != 0.0 {
        s.size *= oc_schema::text::BASELINE_SCALE;
    }
    let (fill, opacity) = match &s.fill {
        Paint::Solid(p) => color(&p.color),
        Paint::Linear(l) if !l.stops.is_empty() => color(&l.stops[0].color),
        Paint::Radial(r) if !r.stops.is_empty() => color(&r.stops[0].color),
        _ => ("#000".into(), 1.0),
    };
    let deco = [if s.underline == Some(true) { "underline" } else { "" }, if s.strike == Some(true) { "line-through" } else { "" }].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" ");
    let t = match s.case_transform {
        Some(CaseTransform::Upper) => text.to_uppercase(),
        Some(CaseTransform::Lower) => text.to_lowercase(),
        _ => text.to_owned(),
    };
    let paint = match look {
        Some((None, wd)) => format!(r#" fill="none" stroke="{fill}" stroke-opacity="{}" stroke-width="{}" stroke-linejoin="round""#, n(opacity), n((wd * s.size / 100.0).max(0.5))),
        Some((Some((c, o)), wd)) => format!(
            r#" fill="{fill}"{} stroke="{c}" stroke-opacity="{}" stroke-width="{}" stroke-linejoin="round" paint-order="stroke""#,
            if opacity < 1.0 { format!(r#" fill-opacity="{}""#, n(opacity)) } else { String::new() },
            n(*o),
            n((wd * s.size / 100.0).max(0.5))
        ),
        None => format!(r#" fill="{fill}"{}"#, if opacity < 1.0 { format!(r#" fill-opacity="{}""#, n(opacity)) } else { String::new() }),
    };
    format!(
        r#"<tspan font-family="{}, sans-serif" font-size="{}" font-weight="{}"{}{}{paint}{}{}>{}</tspan>"#,
        esc(&css_family(&s.family)),
        n(s.size),
        oc_schema::js::num_to_string(s.weight),
        if shift != 0.0 { format!(r#" baseline-shift="{}""#, n(-shift)) } else { String::new() },
        if s.italic { r#" font-style="italic""# } else { "" },
        if s.tracking != 0.0 { format!(r#" letter-spacing="{}""#, n((s.tracking / 1000.0) * s.size)) } else { String::new() },
        if deco.is_empty() { String::new() } else { format!(r#" text-decoration="{deco}""#) },
        esc(&t)
    )
}

#[cfg(test)]
mod family_tests {
    #[test]
    fn families_are_quoted_css_strings() {
        assert_eq!(super::css_family("Source Serif 4"), "'Source Serif 4'");
        assert_eq!(super::css_family("It's \\x"), "'It\\'s \\\\x'");
    }
}
