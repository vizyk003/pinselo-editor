// SPDX-License-Identifier: Apache-2.0
//! Vector PDF (EX-010/011, the drawing layer of `services/render/src/pdfVector.ts`) from the same display lists the
//! raster painter draws, written with **krilla**:
//!
//! - shapes, lines, tables, charts and QR codes as vector paths; gradients as PDF shadings (per-stop opacity
//!   through soft masks); strokes with the painter's alignment (inside/outside = clipped double width), caps,
//!   joins and dashes;
//! - text as real text: the faces oc-text laid out with, embedded and subsetted by krilla, every glyph placed
//!   where the layout put it, with a ToUnicode/ActualText mapping per cluster (selectable and searchable); colour
//!   glyphs (COLR/CBDT/sbix) are handled by krilla;
//! - images embedded at full resolution (PNG/JPEG/GIF/WebP natively; SVG and edited images through the raster
//!   painter at their native size), cropped by clipping;
//! - opacity and blend modes as transparency groups; effects without a PDF equivalent (drop shadows, blur, soft
//!   text effects) rasterised by the painter for just that element, with the vector text kept on top (or
//!   invisible underneath, for blurred layers) so it stays selectable;
//! - print production: bleed, TrimBox/BleedBox, crop and registration marks, colour bars;
//! - print colour (with an output profile, [`PrintSetup::output`]): **PDF/X-4** (ICC-tagged sRGB objects, authored
//!   CMYK as DeviceCMYK, spot colours as `/Separation` with a CMYK alternate, Display-P3 converted through the
//!   profile, live transparency blended in CMYK, a `/GTS_PDFX` output intent) and **PDF/X-1a:2003** (every colour
//!   and image converted to DeviceCMYK through the profile — relative colorimetric + BPC or perceptual, by
//!   `oc-color` — and transparency flattened: each element that needs transparency is replaced by a raster of
//!   the page region it covers, composited with everything underneath, at 300 dpi); in both, pure-black text
//!   under 12 pt and hairlines up to 2 pt print K-only with overprint (the rich-black rule). "Flatten" without
//!   CMYK flattens the same way in RGB.
//!
//! PDF/X document structures (output intent, `GTS_PDFXVersion`, `Trapped`, XMP) and overprint come from the
//! vendored krilla patch (`crates/vendor/krilla/PATCHES.md`); [`crate::pdfx_check`] validates the result.
//!
//! Why krilla rather than pdf-writer or printpdf: krilla sits on pdf-writer (same object model) but adds what this
//! needs and is costly to get right by hand — font subsetting (TrueType/CFF/variable, via `subsetter`) with
//! correct CID mapping and ToUnicode/ActualText, colour fonts, transparency groups and soft masks for gradient
//! stop opacity, image embedding with alpha — and it is pure Rust, so it builds for WASM like the rest of the
//! crate. printpdf is higher level but has no shaped-glyph API with cluster mapping; pdf-writer alone would mean
//! writing the font subsetting and group plumbing ourselves.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use krilla::Document;
use krilla::color::{cmyk, rgb, separation};
use krilla::geom::{Path as KPath, PathBuilder as KPathBuilder, Point as KPoint, Rect as KRect, Size as KSize, Transform as KTransform};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill as KFill, FillRule, LineCap, LineJoin, LinearGradient, RadialGradient, SpreadMethod, Stop, Stroke as KStroke, StrokeDash};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use oc_geometry::{MarkerMode, line_marker_shape, trim_line_for_markers};
use oc_schema::color::to_rgba;
use oc_schema::elements::{CellVAlign, Effect, ImageEditStack, LineMarker};
use oc_schema::primitives::{BlendMode as DocBlend, ColorValue, GradientStop, ImageFit, Rect, Stroke as DocStroke, StrokeAlign, StrokeCap, StrokeJoin};
use oc_schema::text::{TextBody, TextEffect};
use oc_text::{GlyphRun, LaidLine, LaidParagraph, paint_to_color};

use oc_color::{Converter, Ink, OutputProfile};

use crate::assets::{AssetProvider, Decoded};
use crate::color_matrix::{edit_matrix, needs_tone_shader};
use crate::displaylist::{DLItem, DisplayList, ResolvedPaint};
use crate::path::{Cmd, PathData};
use crate::raster::{Renderer, linear_points};

const PT: f64 = 0.75;
const MM: f64 = 96.0 / 25.4;

/// Which PDF/X standard to declare (and to produce).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PdfStandard {
    /// A plain PDF (with print boxes and marks when set).
    #[default]
    None,
    /// PDF/X-4: ICC-based colour, live transparency, output intent. PDF 1.6.
    PdfX4,
    /// PDF/X-1a:2003: CMYK and spot only, no transparency, output intent. PDF 1.4.
    PdfX1a,
}

impl PdfStandard {
    pub fn parse(s: &str) -> PdfStandard {
        match s {
            "pdfx4" => PdfStandard::PdfX4,
            "pdfx1a" => PdfStandard::PdfX1a,
            _ => PdfStandard::None,
        }
    }
}

/// The characterised printing condition (`/OutputIntents`): the output profile plus its registry entry.
#[derive(Debug, Clone)]
pub struct OutputIntent {
    pub profile: OutputProfile,
    /// `/OutputConditionIdentifier`, e.g. `FOGRA51` (for a custom profile: its description).
    pub condition_id: String,
    pub condition: Option<String>,
    /// `http://www.color.org` for registered conditions.
    pub registry: Option<String>,
    /// `/Info`, e.g. the profile name.
    pub info: Option<String>,
}

/// Print production: bleed, marks, colour and PDF/X.
#[derive(Debug, Clone)]
pub struct PrintSetup {
    /// Bleed beyond the trim, in mm (content extends into it; TrimBox/BleedBox are set).
    pub bleed_mm: f64,
    /// Crop marks in the slug around the bleed.
    pub crop_marks: bool,
    /// Registration targets centred on each side, in the slug.
    pub registration_marks: bool,
    /// A CMYK colour bar (solids, tints, overprints, grey balance) in the top slug.
    pub color_bars: bool,
    /// Slug width beyond the bleed for the marks, mm (default 10 mm when any mark is on, else 0).
    pub slug_mm: Option<f64>,
    pub standard: PdfStandard,
    /// The output intent. Required for PDF/X; with a CMYK profile, colours are prepared for it even without X.
    pub output: Option<OutputIntent>,
    /// RGB → CMYK rendering intent (relative colorimetric with BPC by default).
    pub intent: oc_color::Intent,
    /// Flatten transparency (always on for PDF/X-1a).
    pub flatten: bool,
    /// K-only, overprinting black for small text and hairlines (CMYK output only).
    pub black_overprint: bool,
    /// CMYK alternates for spot colours by swatch name.
    pub spots: oc_color::SpotAlternates,
}

impl Default for PrintSetup {
    fn default() -> Self {
        PrintSetup {
            bleed_mm: 3.0,
            crop_marks: true,
            registration_marks: false,
            color_bars: false,
            slug_mm: None,
            standard: PdfStandard::None,
            output: None,
            intent: oc_color::Intent::Relative,
            flatten: false,
            black_overprint: true,
            spots: oc_color::SpotAlternates::new(),
        }
    }
}

impl PrintSetup {
    /// Bleed and crop marks only (an RGB print PDF).
    pub fn marks(bleed_mm: f64, crop_marks: bool) -> PrintSetup {
        PrintSetup { bleed_mm, crop_marks, ..PrintSetup::default() }
    }

    fn any_marks(&self) -> bool {
        self.crop_marks || self.registration_marks || self.color_bars
    }

    /// Slug beyond the bleed, mm.
    pub fn slug(&self) -> f64 {
        match self.slug_mm {
            Some(s) => s.clamp(0.0, 30.0),
            None if self.any_marks() => 10.0,
            None => 0.0,
        }
    }

    /// Whether the output profile is CMYK (colours get prepared for it).
    pub fn cmyk_output(&self) -> bool {
        self.output.as_ref().is_some_and(|o| o.profile.is_cmyk())
    }
}

#[derive(Debug, Clone)]
pub struct PdfOptions {
    pub title: Option<String>,
    pub producer: Option<String>,
    pub lang: Option<String>,
    pub print: Option<PrintSetup>,
    /// Pixels per document px for rasterised effects.
    pub raster_scale: f64,
    /// Draw every text op invisibly (selectable text over a page picture: the flattened PDF's text layer).
    pub invisible_text: bool,
    /// Creation date (seconds since the Unix epoch, UTC) for the document info and XMP (PDF/X needs one).
    pub created_unix: Option<i64>,
}

impl Default for PdfOptions {
    fn default() -> Self {
        PdfOptions { title: None, producer: None, lang: None, print: None, raster_scale: 2.5, invisible_text: false, created_unix: None }
    }
}

/// Print resolution for flattened regions (300 dpi in document px).
const PRINT_SCALE: f64 = 300.0 / 96.0;
/// Largest flattened region, in pixels (the resolution drops below 300 dpi beyond it).
const MAX_REGION_PX: f64 = 80.0e6;
/// More transparent elements than this on a page are flattened as one region.
const MAX_REGIONS: usize = 32;
/// Text under this size (pt) and pure black prints K-only with overprint.
const BLACK_TEXT_PT: f64 = 12.0;
/// Strokes up to this width (pt) and pure black print K-only with overprint.
const BLACK_STROKE_PT: f64 = 2.0;

/// How colours are written.
#[derive(Clone)]
struct Inks {
    /// The output profile is CMYK: authored CMYK passes through as DeviceCMYK, spot as Separation.
    cmyk_out: bool,
    /// Convert every colour and image to CMYK (PDF/X-1a, or CMYK + flatten).
    all_cmyk: bool,
    /// Flatten transparency.
    flatten: bool,
    /// The rich-black rule.
    overprint_black: bool,
    /// Registration colour for marks: Separation /All (PDF/X) or 400 % CMYK.
    all_sep: bool,
    conv: Option<Arc<Converter>>,
    spots: oc_color::SpotAlternates,
    /// PDF/X: no /Interpolate on images, no link annotations.
    strict: bool,
}

impl Inks {
    fn rgb() -> Inks {
        Inks { cmyk_out: false, all_cmyk: false, flatten: false, overprint_black: false, all_sep: false, conv: None, spots: Default::default(), strict: false }
    }

    fn of(print: Option<&PrintSetup>) -> Result<Inks, PdfError> {
        let Some(p) = print else { return Ok(Inks::rgb()) };
        let cmyk_out = p.cmyk_output();
        if p.standard == PdfStandard::PdfX1a && !cmyk_out {
            return Err(PdfError::Write("PDF/X-1a needs a CMYK output profile".into()));
        }
        if p.standard != PdfStandard::None && p.output.is_none() {
            return Err(PdfError::Write("PDF/X needs an output profile".into()));
        }
        let conv = match &p.output {
            Some(o) if cmyk_out => Some(o.profile.converter(p.intent).map_err(|e| PdfError::Write(e.to_string()))?),
            _ => None,
        };
        let x1a = p.standard == PdfStandard::PdfX1a;
        Ok(Inks {
            cmyk_out,
            all_cmyk: x1a || (cmyk_out && p.flatten),
            flatten: x1a || p.flatten,
            overprint_black: cmyk_out && p.black_overprint,
            all_sep: p.standard != PdfStandard::None,
            conv,
            spots: p.spots.clone(),
            strict: p.standard != PdfStandard::None,
        })
    }

    fn cmyk(c: [f64; 4]) -> krilla::color::Color {
        let ch = |v: f64| (if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) } * 255.0).round() as u8;
        cmyk::Color::new(ch(c[0]), ch(c[1]), ch(c[2]), ch(c[3])).into()
    }

    fn ink_color(&self, ink: Ink) -> krilla::color::Color {
        match ink {
            Ink::Cmyk(c) => Inks::cmyk(c),
            Ink::Spot { name, tint, alternate } => {
                let ch = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let fallback = cmyk::Color::new(ch(alternate[0]), ch(alternate[1]), ch(alternate[2]), ch(alternate[3]));
                let space = separation::SeparationSpace::new(separation::SeparationColorant::Custom(name), fallback.into());
                separation::Color::new(ch(tint), space).into()
            }
        }
    }

    /// A colour for drawing (and its alpha).
    fn color(&self, c: &ColorValue) -> (krilla::color::Color, f64) {
        if !self.cmyk_out {
            let (k, a) = rgb_of(c);
            return (k.into(), a);
        }
        let a = c.alpha();
        match c {
            // PDF/X-4: sRGB stays ICC-tagged sRGB; the RIP converts it with the output intent.
            ColorValue::Srgb(_) if !self.all_cmyk => {
                let (k, a) = rgb_of(c);
                (k.into(), a)
            }
            _ => (self.ink_color(oc_color::ink(c, self.conv.as_deref(), &self.spots)), a),
        }
    }

    /// CMYK (0..1) of any colour, spot colours as their tinted alternate (gradient stops must share a space).
    fn process(&self, c: &ColorValue) -> [f64; 4] {
        match oc_color::ink(c, self.conv.as_deref(), &self.spots) {
            Ink::Cmyk(k) => k,
            Ink::Spot { tint, alternate, .. } => alternate.map(|v| v * tint),
        }
    }

    /// The rich-black rule: pure black small text or a hairline → K-only with overprint.
    fn knockout_black(&self, c: &ColorValue, small: bool) -> bool {
        self.overprint_black && small && oc_color::is_pure_black(c)
    }

    /// Registration colour (crop and registration marks).
    fn registration(&self) -> krilla::color::Color {
        if self.all_sep {
            let space = separation::SeparationSpace::new(separation::SeparationColorant::AllColorants, cmyk::Color::new(255, 255, 255, 255).into());
            separation::Color::new(255, space).into()
        } else {
            cmyk::Color::new(255, 255, 255, 255).into()
        }
    }
}

/// One page: its display list and link areas (page space px → URL).
#[derive(Debug, Clone)]
pub struct PdfPage<'a> {
    pub dl: &'a DisplayList,
    pub links: Vec<(Rect, String)>,
}

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("pdf: {0}")]
    Write(String),
}

fn nf(v: f64) -> NormalizedF32 {
    NormalizedF32::new(if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) as f32 }).unwrap_or(NormalizedF32::ONE)
}

fn rgb_of(c: &ColorValue) -> (rgb::Color, f64) {
    let x = to_rgba(c);
    let ch = |v: f64| (if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) } * 255.0).round() as u8;
    (rgb::Color::new(ch(x.r), ch(x.g), ch(x.b)), x.a)
}

fn kt(m: &[f64; 6]) -> KTransform {
    KTransform::from_row(m[0] as f32, m[1] as f32, m[2] as f32, m[3] as f32, m[4] as f32, m[5] as f32)
}

fn mul(m: &[f64; 6], n: &[f64; 6]) -> [f64; 6] {
    oc_geometry::multiply(m, n)
}

fn kpath(p: &PathData) -> Option<KPath> {
    let mut b = KPathBuilder::new();
    for c in &p.cmds {
        match *c {
            Cmd::Move(x, y) => b.move_to(x as f32, y as f32),
            Cmd::Line(x, y) => b.line_to(x as f32, y as f32),
            Cmd::Quad(x1, y1, x, y) => b.quad_to(x1 as f32, y1 as f32, x as f32, y as f32),
            Cmd::Cubic(x1, y1, x2, y2, x, y) => b.cubic_to(x1 as f32, y1 as f32, x2 as f32, y2 as f32, x as f32, y as f32),
            Cmd::Close => b.close(),
        }
    }
    b.finish()
}

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> Option<KPath> {
    kpath(&PathData::rect(x, y, w, h))
}

/// Gradient stops with Skia's normalisation (positions clamped and made monotonic, ends padded). All stops share
/// one colour space: sRGB, or (CMYK output with any non-sRGB stop, or everything-CMYK) process CMYK.
fn stops(s: &[GradientStop], inks: &Inks) -> Vec<Stop> {
    let cmyk = inks.cmyk_out && (inks.all_cmyk || s.iter().any(|g| !matches!(g.color, ColorValue::Srgb(_))));
    let mut prev = 0.0f64;
    let mut out: Vec<Stop> = Vec::new();
    for (i, g) in s.iter().enumerate() {
        let t = if g.t.is_nan() { 0.0 } else { g.t.clamp(prev, 1.0) };
        let t = if i + 1 == s.len() && s.len() > 1 { t.max(prev) } else { t };
        prev = t;
        let (c, a): (krilla::color::Color, f64) = if cmyk {
            (Inks::cmyk(inks.process(&g.color)), g.color.alpha())
        } else {
            let (c, a) = rgb_of(&g.color);
            (c.into(), a)
        };
        out.push(Stop { offset: nf(t), color: c, opacity: nf(a) });
    }
    out
}

/// How many surface pushes a display-list frame made (popped on its restore).
struct Frame {
    pushes: usize,
    ctm: [f64; 6],
}

struct Writer<'a, 'b> {
    r: &'a mut Renderer,
    assets: &'a dyn AssetProvider,
    fonts: &'b mut HashMap<String, Option<Font>>,
    opts: &'a PdfOptions,
    page_w: f64,
    page_h: f64,
    bleed: f64,
    inks: &'a Inks,
}

/// Draws onto a surface while tracking pushes.
struct Ctx<'s, 'd> {
    s: &'s mut Surface<'d>,
    ctm: [f64; 6],
    stack: Vec<Frame>,
    /// Pushes made outside any frame (clips at the top level).
    root: usize,
}

impl Ctx<'_, '_> {
    fn bump(&mut self) {
        match self.stack.last_mut() {
            Some(f) => f.pushes += 1,
            None => self.root += 1,
        }
    }
}

/// Write display lists as a vector PDF.
pub fn write_pdf(r: &mut Renderer, pages: &[PdfPage<'_>], assets: &dyn AssetProvider, opts: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    let print = opts.print.as_ref();
    let inks = Inks::of(print)?;
    let standard = print.map_or(PdfStandard::None, |p| p.standard);
    let mut settings = krilla::SerializeSettings::default();
    if let (Some(p), true) = (print, standard != PdfStandard::None) {
        let oi = p.output.as_ref().ok_or_else(|| PdfError::Write("PDF/X needs an output profile".into()))?;
        let x4 = standard == PdfStandard::PdfX4;
        // X-4: RGB objects ICC-tagged (sRGB); X-1a: device colour only (all CMYK by now).
        settings.no_device_cs = x4;
        let version = if x4 { krilla::configure::PdfVersion::Pdf16 } else { krilla::configure::PdfVersion::Pdf14 };
        settings.configuration = krilla::configure::ConfigurationBuilder::new().with_version(version).finish().map_err(|e| PdfError::Write(format!("{e:?}")))?;
        settings.enable_tagging = false;
        settings.pdfx = Some(krilla::configure::PdfX {
            version: if x4 { krilla::configure::PdfXVersion::X4 } else { krilla::configure::PdfXVersion::X1a2003 },
            output_intent: krilla::configure::PdfXOutputIntent {
                profile: oi.profile.bytes().clone(),
                components: if oi.profile.is_cmyk() { 4 } else { 3 },
                condition_identifier: oi.condition_id.clone(),
                condition: oi.condition.clone(),
                registry_name: oi.registry.clone(),
                info: oi.info.clone(),
            },
            trapped: false,
        });
    }
    let mut doc = Document::new_with(settings);
    let mut meta = krilla::metadata::Metadata::new();
    match &opts.title {
        Some(t) => meta = meta.title(t.clone()),
        // PDF/X wants a title.
        None if standard != PdfStandard::None => meta = meta.title("Untitled".to_string()),
        None => {}
    }
    if let Some(p) = &opts.producer {
        meta = meta.producer(p.clone()).creator(p.clone());
    }
    if let Some(l) = &opts.lang {
        meta = meta.language(l.clone());
    }
    match opts.created_unix {
        Some(t) => meta = meta.creation_date(pdf_date(t)),
        None if standard != PdfStandard::None => meta = meta.creation_date(pdf_date(0)),
        None => {}
    }
    doc.set_metadata(meta);
    let mut fonts: HashMap<String, Option<Font>> = HashMap::new();
    for pg in pages {
        let dl = pg.dl;
        let bleed = print.map_or(0.0, |p| p.bleed_mm.clamp(0.0, 25.0) * MM);
        let slug = print.map_or(0.0, |p| p.slug() * MM);
        let margin = bleed + slug;
        let (pw, ph) = ((dl.width + margin * 2.0) * PT, (dl.height + margin * 2.0) * PT);
        let mut settings = PageSettings::from_wh(pw as f32, ph as f32).ok_or_else(|| PdfError::Write("invalid page size".into()))?;
        if print.is_some() {
            let inset = |d: f64| KRect::from_ltrb((d * PT) as f32, (d * PT) as f32, (pw - d * PT) as f32, (ph - d * PT) as f32);
            settings = settings.with_trim_box(inset(margin)).with_bleed_box(inset(slug));
        }
        let plan = if inks.flatten { flatten_plan(r, dl, assets, bleed, &inks) } else { Vec::new() };
        let mut page = doc.start_page_with(settings);
        {
            let mut surface = page.surface();
            let mut w = Writer { r, assets, fonts: &mut fonts, opts, page_w: dl.width, page_h: dl.height, bleed, inks: &inks };
            // px (top-left, y down) → pt, offset by the bleed/slug margin.
            surface.push_transform(&KTransform::from_row(PT as f32, 0.0, 0.0, PT as f32, (margin * PT) as f32, (margin * PT) as f32));
            if let Some(clip) = rect_path(-bleed, -bleed, dl.width + bleed * 2.0, dl.height + bleed * 2.0) {
                surface.push_clip_path(&clip, &FillRule::NonZero);
            }
            let mut ctx = Ctx { s: &mut surface, ctm: oc_geometry::IDENTITY, stack: Vec::new(), root: 0 };
            let bg = PathData::rect(-bleed, -bleed, dl.width + bleed * 2.0, dl.height + bleed * 2.0);
            for p in &dl.background {
                w.fill_with(&mut ctx, &bg, p);
            }
            if inks.flatten {
                w.flattened(&mut ctx, dl, &plan);
            } else {
                w.items(&mut ctx, &dl.items);
            }
            while let Some(f) = ctx.stack.pop() {
                for _ in 0..f.pushes {
                    ctx.s.pop();
                }
            }
            for _ in 0..ctx.root {
                ctx.s.pop();
            }
            surface.pop();
            surface.pop();
            if let Some(p) = print {
                marks(&mut surface, p, &inks, dl.width, dl.height, bleed, margin);
            }
            surface.finish();
        }
        // PDF/X: no annotations inside the bleed box (links are screen-only anyway).
        if !inks.strict {
            for (rect, url) in &pg.links {
                let x0 = ((rect.x + margin) * PT) as f32;
                let y0 = ((rect.y + margin) * PT) as f32;
                if let Some(kr) = KRect::from_xywh(x0, y0, (rect.w * PT) as f32, (rect.h * PT) as f32) {
                    let action = krilla::action::LinkAction::new(url.clone());
                    let link = krilla::annotation::LinkAnnotation::new(kr, krilla::annotation::Target::Action(action.into()));
                    page.add_annotation(krilla::annotation::Annotation::new_link(link, None));
                }
            }
        }
        page.finish();
    }
    doc.finish().map_err(|e| PdfError::Write(format!("{e:?}")))
}

/// Unix seconds → a PDF date (UTC).
fn pdf_date(t: i64) -> krilla::metadata::DateTime {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    krilla::metadata::DateTime::new(y.clamp(0, 9999) as u16)
        .month(m as u8)
        .day(d as u8)
        .hour((secs / 3600) as u8)
        .minute(((secs / 60) % 60) as u8)
        .second((secs % 60) as u8)
        .utc_offset_hour(0)
        .utc_offset_minute(0)
}

/// Crop marks, registration targets and colour bars in the slug (page space in pt, y down).
fn marks(surface: &mut Surface<'_>, p: &PrintSetup, inks: &Inks, w: f64, h: f64, bleed: f64, margin: f64) {
    let t = margin;
    let slug = margin - bleed;
    if slug <= 0.0 {
        return;
    }
    let reg = inks.registration();
    let hair = |surface: &mut Surface<'_>, b: KPathBuilder| {
        if let Some(path) = b.finish() {
            surface.set_fill(None);
            surface.set_stroke(Some(KStroke { paint: reg.clone().into(), width: 0.25, ..Default::default() }));
            surface.draw_path(&path);
            surface.set_stroke(None);
        }
    };
    // Crop marks: registration colour, 0.25 pt, 2 mm off the bleed.
    if p.crop_marks {
        let off = bleed + 2.0 * MM;
        let len = (5.0 * MM).min(slug - 2.0 * MM).max(1.0 * MM);
        let mut b = KPathBuilder::new();
        for (cx, cy) in [(t, t), (t + w, t), (t, t + h), (t + w, t + h)] {
            let sx = if cx == t { -1.0 } else { 1.0 };
            let sy = if cy == t { -1.0 } else { 1.0 };
            b.move_to(((cx + sx * off) * PT) as f32, (cy * PT) as f32);
            b.line_to(((cx + sx * (off + len)) * PT) as f32, (cy * PT) as f32);
            b.move_to((cx * PT) as f32, ((cy + sy * off) * PT) as f32);
            b.line_to((cx * PT) as f32, ((cy + sy * (off + len)) * PT) as f32);
        }
        hair(surface, b);
    }
    // Registration targets: a circle with a cross, centred on each side of the slug.
    let rr = (2.5 * MM).min(slug / 2.0 - 0.5 * MM);
    if p.registration_marks && rr > 0.5 * MM {
        let d = bleed + slug / 2.0;
        let mut b = KPathBuilder::new();
        for (cx, cy) in [(t + w / 2.0, t - d), (t + w / 2.0, t + h + d), (t - d, t + h / 2.0), (t + w + d, t + h / 2.0)] {
            let (cx, cy, r) = (cx * PT, cy * PT, rr * PT);
            circle(&mut b, cx, cy, r);
            circle(&mut b, cx, cy, r * 0.5);
            b.move_to((cx - r * 1.5) as f32, cy as f32);
            b.line_to((cx + r * 1.5) as f32, cy as f32);
            b.move_to(cx as f32, (cy - r * 1.5) as f32);
            b.line_to(cx as f32, (cy + r * 1.5) as f32);
        }
        hair(surface, b);
    }
    // Colour bar: solids, 50 % tints, two-colour overprints and a grey balance patch, in the top slug from the
    // left trim to just before the top registration target.
    if p.color_bars {
        let s = (5.0 * MM).min(slug - 2.0 * MM);
        if s > 1.0 * MM {
            let y = t - bleed - slug / 2.0 - s / 2.0;
            let patches: [[f64; 4]; 13] = [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
                [1.0, 1.0, 0.0, 0.0],
                [0.0, 1.0, 1.0, 0.0],
                [1.0, 0.0, 1.0, 0.0],
                [0.5, 0.0, 0.0, 0.0],
                [0.0, 0.5, 0.0, 0.0],
                [0.0, 0.0, 0.5, 0.0],
                [0.0, 0.0, 0.0, 0.5],
                [0.5, 0.4, 0.4, 0.0],
                [0.0, 0.0, 0.0, 0.25],
            ];
            let stop = t + w / 2.0 - (if p.registration_marks { rr * 2.0 + 2.0 * MM } else { 0.0 });
            let mut x = t;
            surface.set_stroke(None);
            for c in patches {
                if x + s > stop {
                    break;
                }
                if let Some(path) = rect_path(x * PT, y * PT, s * PT, s * PT) {
                    surface.set_fill(Some(KFill { paint: Inks::cmyk(c).into(), opacity: NormalizedF32::ONE, rule: FillRule::NonZero }));
                    surface.draw_path(&path);
                }
                x += s;
            }
            surface.set_fill(None);
        }
    }
}

/// A circle as four cubics.
fn circle(b: &mut KPathBuilder, cx: f64, cy: f64, r: f64) {
    const K: f64 = 0.552_284_75;
    let f = |v: f64| v as f32;
    b.move_to(f(cx + r), f(cy));
    b.cubic_to(f(cx + r), f(cy + r * K), f(cx + r * K), f(cy + r), f(cx), f(cy + r));
    b.cubic_to(f(cx - r * K), f(cy + r), f(cx - r), f(cy + r * K), f(cx - r), f(cy));
    b.cubic_to(f(cx - r), f(cy - r * K), f(cx - r * K), f(cy - r), f(cx), f(cy - r));
    b.cubic_to(f(cx + r * K), f(cy - r), f(cx + r), f(cy - r * K), f(cx + r), f(cy));
    b.close();
}

// ---------------------------------------------------------------- transparency flattening

/// A top-level element of a page: `items[start..end]` with balanced save/restore.
#[derive(Debug, Clone, Copy)]
struct Unit {
    start: usize,
    end: usize,
}

fn units(items: &[DLItem]) -> Vec<Unit> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, it) in items.iter().enumerate() {
        match it {
            DLItem::Save { .. } | DLItem::Layer { .. } | DLItem::Mask { .. } => depth += 1,
            DLItem::Restore => depth -= 1,
            _ => {}
        }
        if depth <= 0 {
            out.push(Unit { start, end: i + 1 });
            start = i + 1;
            depth = 0;
        }
    }
    if start < items.len() {
        out.push(Unit { start, end: items.len() });
    }
    out
}

/// A flattened region: drawn (opaque, at print resolution) after `after` units, replacing the transparent units
/// that are in `hidden`.
#[derive(Debug, Clone)]
struct Region {
    /// Index of the unit after which the region raster is placed.
    after: usize,
    /// Page-space rectangle (px), snapped to the raster grid.
    rect: (f64, f64, f64, f64),
}

/// Which units need transparency, and the regions that replace them.
fn flatten_plan(r: &mut Renderer, dl: &DisplayList, assets: &dyn AssetProvider, bleed: f64, inks: &Inks) -> Vec<(usize, bool, Option<Region>)> {
    let us = units(&dl.items);
    let mut probe = Probe { r, assets, alpha: HashMap::new() };
    let mut plan: Vec<(usize, bool, Option<Region>)> = Vec::with_capacity(us.len());
    let (bx0, by0, bx1, by1) = (-bleed, -bleed, dl.width + bleed, dl.height + bleed);
    let page_bg_transparent = dl.background.iter().any(|p| probe.paint_transparent(p));
    for (ui, u) in us.iter().enumerate() {
        let items = &dl.items[u.start..u.end];
        let transparent = items.iter().any(|it| probe.item_transparent(it, inks));
        let region = if transparent {
            local_bounds(items).map(|(x0, y0, x1, y1)| {
                let reach = items.iter().map(effect_reach).fold(0.0, f64::max) + 2.0;
                let k = PRINT_SCALE;
                let snap = |v: f64, up: bool| if up { (v * k).ceil() / k } else { (v * k).floor() / k };
                (snap((x0 - reach).max(bx0), false), snap((y0 - reach).max(by0), false), snap((x1 + reach).min(bx1), true), snap((y1 + reach).min(by1), true))
            })
        } else {
            None
        };
        let region = region.filter(|(x0, y0, x1, y1)| x1 > x0 && y1 > y0).map(|rect| Region { after: ui, rect });
        plan.push((ui, transparent, region));
    }
    // A transparent page background (no paper colour) is fine: regions composite over white paper.
    let _ = page_bg_transparent;
    let n = plan.iter().filter(|p| p.2.is_some()).count();
    if n > MAX_REGIONS {
        // Too many islands: one region over their union, placed after the last transparent unit.
        let mut u: Option<(f64, f64, f64, f64)> = None;
        let mut last = 0;
        for p in plan.iter_mut() {
            if let Some(rg) = p.2.take() {
                let (a, b, c, d) = rg.rect;
                u = Some(match u {
                    None => (a, b, c, d),
                    Some((e, f, g, h)) => (e.min(a), f.min(b), g.max(c), h.max(d)),
                });
                last = rg.after;
            }
        }
        if let Some(rect) = u {
            plan[last].2 = Some(Region { after: last, rect });
        }
    }
    plan
}

/// How far drawing effects (shadows, blur, outlines) reach beyond an item's bounds.
fn effect_reach(it: &DLItem) -> f64 {
    match it {
        DLItem::Layer { effects, .. } => effects
            .iter()
            .map(|e| match e {
                Effect::DropShadow(d) => d.dx.abs() + d.dy.abs() + d.blur * 1.5,
                Effect::Blur(b) => b.radius * 1.5,
                Effect::Outline(o) => o.width * 2.0,
            })
            .sum(),
        _ => 0.0,
    }
}

/// Transparency detection for flattening (images decoded to look at their alpha).
struct Probe<'a> {
    r: &'a mut Renderer,
    assets: &'a dyn AssetProvider,
    alpha: HashMap<String, bool>,
}

fn translucent(c: &ColorValue) -> bool {
    let a = c.alpha();
    a > 0.001 && a < 0.999
}

impl Probe<'_> {
    fn image_alpha(&mut self, asset: &str, crop: Option<&Rect>, edits: Option<&ImageEditStack>) -> bool {
        let key = format!("{asset}|{}", edits.map_or(String::new(), |e| format!("{e:?}")));
        if let Some(a) = self.alpha.get(&key) {
            return *a;
        }
        let a = match rgba_pixels(self.r, self.assets, asset, crop, edits) {
            Some((px, _, _, _, _)) => px.as_chunks::<4>().0.iter().any(|p| p[3] < 255),
            None => false,
        };
        self.alpha.insert(key, a);
        a
    }

    fn paint_transparent(&mut self, p: &ResolvedPaint) -> bool {
        match p {
            ResolvedPaint::Solid { color } => translucent(color),
            ResolvedPaint::Linear { stops, .. } | ResolvedPaint::Radial { stops, .. } => stops.iter().any(|s| s.color.alpha() < 0.999),
            ResolvedPaint::Image { asset, .. } => self.image_alpha(asset, None, None),
        }
    }

    fn item_transparent(&mut self, it: &DLItem, inks: &Inks) -> bool {
        match it {
            DLItem::Save { .. } | DLItem::Restore | DLItem::Clip { .. } | DLItem::MaskContent => false,
            // Soft masks are transparency (PDF/X-1a flattens them).
            DLItem::Mask { .. } => true,
            DLItem::Layer { opacity, blend, effects, .. } => *opacity < 0.999 || pdf_blend(blend).is_some() || effects.iter().any(|e| !matches!(e, Effect::Outline(_))),
            DLItem::Path { fills, stroke, .. } => fills.iter().any(|f| self.paint_transparent(f)) || stroke.as_ref().is_some_and(|s| self.paint_transparent(&s.paint)),
            DLItem::Line { paint, .. } => self.paint_transparent(paint),
            DLItem::Image { asset, crop, edits, .. } => {
                let crop = Rect::from(crop);
                self.image_alpha(asset, Some(&crop), edits.as_ref())
            }
            DLItem::Video { asset, crop, edits, .. } => {
                let crop = Rect::from(crop);
                self.image_alpha(&format!("{asset}/poster"), Some(&crop), edits.as_ref())
            }
            DLItem::Text { body, effect, .. } => {
                if matches!(effect, Some(TextEffect::Shadow(_) | TextEffect::Neon(_) | TextEffect::Echo(_) | TextEffect::Glitch(_))) {
                    return true;
                }
                match effect {
                    Some(TextEffect::Background(bg)) if translucent(&bg.color) => return true,
                    Some(TextEffect::Outline(o)) if translucent(&o.color) => return true,
                    _ => {}
                }
                body.paragraphs.iter().flat_map(|p| p.runs.iter()).any(|run| {
                    let st = oc_schema::text::resolve_run_style(body, run);
                    let fill = paint_to_color(Some(&st.fill));
                    // Colour glyphs (emoji) are drawn in RGB with their own transparency.
                    let emoji = inks.all_cmyk && run.text.chars().any(|c| matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0xFE0F));
                    translucent(&fill) || st.highlight.as_ref().is_some_and(translucent) || emoji
                })
            }
            DLItem::Qr { fg, bg, .. } => translucent(fg) || translucent(bg),
            DLItem::Placeholder { .. } => true,
        }
    }
}

/// Decoded RGBA pixels of an image as the PDF embeds it (edits baked), plus its natural size.
fn rgba_pixels(r: &mut Renderer, assets: &dyn AssetProvider, asset: &str, crop: Option<&Rect>, edits: Option<&ImageEditStack>) -> Option<(Vec<u8>, u32, u32, f64, f64)> {
    let adjust = edits.and_then(|e| e.adjust.as_ref());
    let edited = edit_matrix(edits).is_some() || needs_tone_shader(adjust) || adjust.is_some_and(|a| a.blur.is_some_and(|b| b > 0.0) || a.vignette.is_some_and(|v| v > 0.0));
    if edited {
        let unit = Rect::UNIT;
        let p = r.bake_image(asset, crop.unwrap_or(&unit), edits, assets)?;
        let (w, h) = (p.width(), p.height());
        return Some((crate::raster::to_rgba8(&p), w, h, f64::from(w), f64::from(h)));
    }
    match r.images_get(asset, assets)? {
        Decoded::Raster(rimg) => {
            let p = &rimg.pixmap;
            Some((crate::raster::to_rgba8(p), p.width(), p.height(), f64::from(p.width()), f64::from(p.height())))
        }
        Decoded::Svg(s) => {
            let (w, h) = s.size();
            let ras = s.raster((w * 2.0).ceil() as u32, (h * 2.0).ceil() as u32)?;
            let p = &ras.pixmap;
            Some((crate::raster::to_rgba8(p), p.width(), p.height(), f64::from(w), f64::from(h)))
        }
    }
}

/// An 8-bit DeviceCMYK image (no alpha) for krilla.
#[derive(Clone)]
struct CmykImage {
    data: Arc<Vec<u8>>,
    w: u32,
    h: u32,
    key: u64,
}

impl CmykImage {
    fn new(data: Vec<u8>, w: u32, h: u32) -> CmykImage {
        use std::hash::{Hash, Hasher};
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        data.hash(&mut hs);
        CmykImage { data: Arc::new(data), w, h, key: hs.finish() }
    }
}

impl std::hash::Hash for CmykImage {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        self.w.hash(state);
        self.h.hash(state);
    }
}

impl krilla::image::CustomImage for CmykImage {
    fn color_channel(&self) -> &[u8] {
        &self.data
    }
    fn alpha_channel(&self) -> Option<&[u8]> {
        None
    }
    fn bits_per_component(&self) -> krilla::image::BitsPerComponent {
        krilla::image::BitsPerComponent::Eight
    }
    fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }
    fn icc_profile(&self) -> Option<&[u8]> {
        None
    }
    fn color_space(&self) -> krilla::image::ImageColorspace {
        krilla::image::ImageColorspace::Cmyk
    }
}

/// RGBA → an opaque image over white paper: DeviceCMYK through the profile, or RGB.
fn opaque_image(rgba: &[u8], w: u32, h: u32, conv: Option<&Converter>) -> Option<krilla::image::Image> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for p in rgba.as_chunks::<4>().0 {
        let a = u32::from(p[3]);
        for c in &p[..3] {
            // Unpremultiplied over white.
            rgb.push(((u32::from(*c) * a + 255 * (255 - a) + 127) / 255) as u8);
        }
    }
    match conv {
        Some(cv) => krilla::image::Image::from_custom(CmykImage::new(cv.srgb8_to_cmyk8(&rgb), w, h), false).ok(),
        None => {
            let mut out = Vec::with_capacity(rgba.len());
            for c in rgb.as_chunks::<3>().0 {
                out.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
            Some(krilla::image::Image::from_rgba8(out, w, h))
        }
    }
}

impl Writer<'_, '_> {
    fn font(&mut self, file: &str) -> Option<Font> {
        if let Some(f) = self.fonts.get(file) {
            return f.clone();
        }
        let f = self.r.text.fonts.face_by_file(file).and_then(|face| Font::new(Arc::new(face.data().to_vec()).into(), 0));
        self.fonts.insert(file.to_owned(), f.clone());
        f
    }

    fn items(&mut self, c: &mut Ctx<'_, '_>, items: &[DLItem]) {
        let mut i = 0;
        while i < items.len() {
            let it = &items[i];
            match it {
                DLItem::Save { matrix } => {
                    c.s.push_transform(&kt(matrix));
                    c.stack.push(Frame { pushes: 1, ctm: c.ctm });
                    c.ctm = mul(&c.ctm, matrix);
                }
                DLItem::Restore => {
                    if let Some(f) = c.stack.pop() {
                        for _ in 0..f.pushes {
                            c.s.pop();
                        }
                        c.ctm = f.ctm;
                    }
                }
                DLItem::Layer { opacity, blend, effects, .. } => {
                    if effects.iter().any(|e| !matches!(e, Effect::Outline(_))) {
                        // Shadows/blur: rasterise this element (until the layer's restore), placed in page space.
                        let mut depth = 1;
                        let mut j = i + 1;
                        while j < items.len() && depth > 0 {
                            match &items[j] {
                                DLItem::Save { .. } | DLItem::Layer { .. } | DLItem::Mask { .. } => depth += 1,
                                DLItem::Restore => depth -= 1,
                                _ => {}
                            }
                            j += 1;
                        }
                        self.raster_group(c, &items[i..j], effects);
                        // Keep text selectable underneath the raster.
                        for x in &items[i + 1..j] {
                            if let DLItem::Text { key, body, bbox, valign, curve, fit, .. } = x {
                                self.text(c, key, body, bbox, None, valign.as_ref(), *curve, fit.is_some(), Some(0.0));
                            }
                        }
                        // The layer's own restore was consumed with the group; the element's outer restore follows.
                        i = j;
                        continue;
                    }
                    let mut f = Frame { pushes: 0, ctm: c.ctm };
                    let bm = pdf_blend(blend);
                    if let Some(bm) = bm {
                        c.s.push_blend_mode(bm);
                        f.pushes += 1;
                    }
                    if *opacity < 1.0 {
                        c.s.push_opacity(nf(*opacity));
                        f.pushes += 1;
                    } else if bm.is_some() {
                        c.s.push_isolated();
                        f.pushes += 1;
                    }
                    c.stack.push(f);
                }
                DLItem::Mask { mode, .. } => {
                    // The mask's items (up to the matching maskContent) become a soft-mask stream; the content that
                    // follows is drawn through it until this op's restore (DS-107).
                    let mut depth = 0i32;
                    let mut j = i + 1;
                    while j < items.len() {
                        match &items[j] {
                            DLItem::Save { .. } | DLItem::Layer { .. } | DLItem::Mask { .. } => depth += 1,
                            DLItem::Restore => depth -= 1,
                            DLItem::MaskContent if depth == 0 => break,
                            _ => {}
                        }
                        j += 1;
                    }
                    let stream = {
                        let mut sb = c.s.stream_builder();
                        let mut s2 = sb.surface();
                        {
                            let mut c2 = Ctx { s: &mut s2, ctm: c.ctm, stack: Vec::new(), root: 0 };
                            self.items(&mut c2, &items[i + 1..j.min(items.len())]);
                            while let Some(f) = c2.stack.pop() {
                                for _ in 0..f.pushes {
                                    c2.s.pop();
                                }
                            }
                            for _ in 0..c2.root {
                                c2.s.pop();
                            }
                        }
                        s2.finish();
                        sb.finish()
                    };
                    let kind = if *mode == oc_schema::elements::MaskMode::Luminance { krilla::mask::MaskType::Luminosity } else { krilla::mask::MaskType::Alpha };
                    c.s.push_mask(krilla::mask::Mask::new(stream, kind));
                    c.stack.push(Frame { pushes: 1, ctm: c.ctm });
                    i = j + 1;
                    continue;
                }
                DLItem::MaskContent => {}
                DLItem::Clip { path } => {
                    if let Some(pd) = crate::path::parse(path) {
                        let p = kpath(&pd).or_else(|| rect_path(-3.0e4, -3.0e4, 1.0, 1.0));
                        if let Some(p) = p {
                            c.s.push_clip_path(&p, &FillRule::NonZero);
                            c.bump();
                        }
                    }
                }
                DLItem::Path { path, fills, stroke, .. } => {
                    if let Some(pd) = crate::path::parse(path) {
                        for f in fills {
                            self.fill_with(c, &pd, f);
                        }
                        if let Some(s) = stroke {
                            self.stroke_with(c, &pd, &s.stroke, &s.paint);
                        }
                    }
                }
                DLItem::Line { points, stroke, paint, start_marker, end_marker, .. } => self.line(c, points, stroke, paint, start_marker, end_marker),
                DLItem::Image { asset, dst, crop, edits, clip, .. } => {
                    let crop = Rect::from(crop);
                    self.image(c, asset, dst, &crop, edits.as_ref(), clip.as_deref(), false);
                }
                DLItem::Video { asset, dst, crop, clip, edits, .. } => {
                    let crop = Rect::from(crop);
                    self.image(c, &format!("{asset}/poster"), dst, &crop, edits.as_ref(), clip.as_deref(), true);
                }
                DLItem::Text { key, body, bbox, valign, curve, fit, .. } if self.opts.invisible_text => {
                    self.text(c, key, body, bbox, None, valign.as_ref(), *curve, fit.is_some(), Some(0.0));
                }
                DLItem::Text { key, body, bbox, effect, valign, curve, fit, .. } => {
                    let soft = matches!(effect, Some(TextEffect::Shadow(_) | TextEffect::Neon(_) | TextEffect::Echo(_) | TextEffect::Glitch(_)));
                    if soft {
                        // Soft effects: the painter's raster for the glow/shadow, vector glyphs on top for crisp,
                        // selectable text.
                        self.raster_group(c, std::slice::from_ref(it), &[]);
                        self.text(c, key, body, bbox, None, valign.as_ref(), *curve, fit.is_some(), None);
                    } else {
                        self.text(c, key, body, bbox, effect.as_ref(), valign.as_ref(), *curve, fit.is_some(), None);
                    }
                }
                DLItem::Qr { modules, size, bbox, fg, bg, .. } => {
                    self.solid_rect(c, bbox.x, bbox.y, bbox.w, bbox.h, bg);
                    let quiet = 2.0;
                    let nn = *size as f64;
                    let cell = bbox.w.min(bbox.h) / (nn + quiet * 2.0);
                    let ox = bbox.x + (bbox.w - cell * nn) / 2.0;
                    let oy = bbox.y + (bbox.h - cell * nn) / 2.0;
                    let mut pd = PathData::new();
                    for rr in 0..*size {
                        for cc in 0..*size {
                            if modules.get(rr * size + cc).copied().unwrap_or(0) != 0 {
                                pd.extend(&PathData::rect(ox + cc as f64 * cell, oy + rr as f64 * cell, cell + 0.02, cell + 0.02));
                            }
                        }
                    }
                    self.fill_with(c, &pd, &ResolvedPaint::Solid { color: fg.clone() });
                }
                DLItem::Placeholder { bbox, .. } => {
                    self.solid_rect(c, bbox.x, bbox.y, bbox.w, bbox.h, &ColorValue::srgb(0.6, 0.6, 0.65, 0.25));
                }
            }
            i += 1;
        }
    }

    fn solid_rect(&mut self, c: &mut Ctx<'_, '_>, x: f64, y: f64, w: f64, h: f64, color: &ColorValue) {
        self.fill_with(c, &PathData::rect(x, y, w, h), &ResolvedPaint::Solid { color: color.clone() });
    }

    fn fill_with(&mut self, c: &mut Ctx<'_, '_>, pd: &PathData, p: &ResolvedPaint) {
        let Some(path) = kpath(pd) else { return };
        let paint: krilla::paint::Paint = match p {
            ResolvedPaint::Solid { color } => {
                let (col, a) = self.inks.color(color);
                if a <= 0.001 {
                    return;
                }
                c.s.set_stroke(None);
                c.s.set_fill(Some(KFill { paint: col.into(), opacity: nf(a), rule: FillRule::NonZero }));
                c.s.draw_path(&path);
                return;
            }
            ResolvedPaint::Linear { stops: s, angle, bbox } => match s.len() {
                0 => return,
                1 => {
                    let (col, a) = self.inks.color(&s[0].color);
                    c.s.set_fill(Some(KFill { paint: col.into(), opacity: nf(a), rule: FillRule::NonZero }));
                    c.s.set_stroke(None);
                    c.s.draw_path(&path);
                    return;
                }
                _ => {
                    let ((x1, y1), (x2, y2)) = linear_points(*angle, bbox);
                    LinearGradient { x1: x1 as f32, y1: y1 as f32, x2: x2 as f32, y2: y2 as f32, transform: KTransform::identity(), spread_method: SpreadMethod::Pad, stops: stops(s, self.inks), anti_alias: true }.into()
                }
            },
            ResolvedPaint::Radial { stops: s, bbox } => {
                if s.is_empty() {
                    return;
                }
                let (cx, cy) = ((bbox.x + bbox.w / 2.0) as f32, (bbox.y + bbox.h / 2.0) as f32);
                let r = (bbox.w.max(bbox.h) / 2.0) as f32;
                RadialGradient { fx: cx, fy: cy, fr: 0.0, cx, cy, cr: r.max(1e-3), transform: KTransform::identity(), spread_method: SpreadMethod::Pad, stops: stops(s, self.inks), anti_alias: true }.into()
            }
            ResolvedPaint::Image { asset, fit, bbox } => {
                self.image_fill(c, &path, pd, asset, fit, bbox);
                return;
            }
        };
        c.s.set_stroke(None);
        c.s.set_fill(Some(KFill { paint, opacity: NormalizedF32::ONE, rule: FillRule::NonZero }));
        c.s.draw_path(&path);
    }

    /// An image fill: the image placed by its fit mode (tiled for `tile`), clipped to the path.
    fn image_fill(&mut self, c: &mut Ctx<'_, '_>, path: &KPath, pd: &PathData, asset: &str, fit: &ImageFit, b: &Rect) {
        let Some((img, iw, ih)) = self.embed(asset, None, None) else {
            self.fill_with(c, pd, &ResolvedPaint::Solid { color: ColorValue::srgb(0.85, 0.87, 0.9, 1.0) });
            return;
        };
        let natural = self.r.image_size(asset, self.assets).unwrap_or((iw, ih));
        let s = crate::raster::image_fit_scale(fit, b, natural.0, natural.1);
        let (dw, dh) = (natural.0 * s, natural.1 * s);
        c.s.push_clip_path(path, &FillRule::NonZero);
        let size = KSize::from_wh(dw as f32, dh as f32);
        if let Some(size) = size {
            if *fit == ImageFit::Tile {
                if let Some((x0, y0, x1, y1)) = pd.bounds() {
                    let tx0 = b.x + (b.w - dw) / 2.0;
                    let ty0 = b.y + (b.h - dh) / 2.0;
                    let i0 = ((x0 - tx0) / dw).floor() as i64;
                    let j0 = ((y0 - ty0) / dh).floor() as i64;
                    let i1 = ((x1 - tx0) / dw).ceil() as i64;
                    let j1 = ((y1 - ty0) / dh).ceil() as i64;
                    if (i1 - i0) * (j1 - j0) <= 10_000 {
                        for j in j0..j1 {
                            for i in i0..i1 {
                                c.s.push_transform(&KTransform::from_translate((tx0 + i as f64 * dw) as f32, (ty0 + j as f64 * dh) as f32));
                                c.s.draw_image(img.clone(), size);
                                c.s.pop();
                            }
                        }
                    }
                }
            } else {
                c.s.push_transform(&KTransform::from_translate((b.x + (b.w - dw) / 2.0) as f32, (b.y + (b.h - dh) / 2.0) as f32));
                c.s.draw_image(img, size);
                c.s.pop();
            }
        }
        c.s.pop();
    }

    /// Whether a stroke is a pure-black hairline (K-only with overprint), given the current transform.
    fn black_hairline(&self, c: &Ctx<'_, '_>, p: &ResolvedPaint, width: f64) -> bool {
        let pt = width * ctm_scale(&c.ctm) * PT;
        matches!(p, ResolvedPaint::Solid { color } if self.inks.knockout_black(color, pt <= BLACK_STROKE_PT))
    }

    /// Stroke a path, K-only with overprint for black hairlines.
    fn draw_stroke(&mut self, c: &mut Ctx<'_, '_>, path: &KPath, st: KStroke, overprint: bool) {
        c.s.set_fill(None);
        if overprint {
            c.s.push_overprint();
            c.s.set_stroke(Some(KStroke { paint: Inks::cmyk([0.0, 0.0, 0.0, 1.0]).into(), ..st }));
            c.s.draw_path(path);
            c.s.pop();
        } else {
            c.s.set_stroke(Some(st));
            c.s.draw_path(path);
        }
        c.s.set_stroke(None);
    }

    fn kstroke(&mut self, s: &DocStroke, p: &ResolvedPaint, width: f64) -> Option<KStroke> {
        let (paint, opacity): (krilla::paint::Paint, f64) = match p {
            ResolvedPaint::Solid { color } => {
                let (c, a) = self.inks.color(color);
                (c.into(), a)
            }
            ResolvedPaint::Linear { stops: st, angle, bbox } if st.len() > 1 => {
                let ((x1, y1), (x2, y2)) = linear_points(*angle, bbox);
                (LinearGradient { x1: x1 as f32, y1: y1 as f32, x2: x2 as f32, y2: y2 as f32, transform: KTransform::identity(), spread_method: SpreadMethod::Pad, stops: stops(st, self.inks), anti_alias: true }.into(), 1.0)
            }
            ResolvedPaint::Radial { stops: st, bbox } if st.len() > 1 => {
                let (cx, cy) = ((bbox.x + bbox.w / 2.0) as f32, (bbox.y + bbox.h / 2.0) as f32);
                let r = (bbox.w.max(bbox.h) / 2.0) as f32;
                (RadialGradient { fx: cx, fy: cy, fr: 0.0, cx, cy, cr: r.max(1e-3), transform: KTransform::identity(), spread_method: SpreadMethod::Pad, stops: stops(st, self.inks), anti_alias: true }.into(), 1.0)
            }
            ResolvedPaint::Linear { stops: st, .. } | ResolvedPaint::Radial { stops: st, .. } => {
                let (c, a) = st.first().map_or((rgb::Color::new(0, 0, 0).into(), 1.0), |g| self.inks.color(&g.color));
                (c.into(), a)
            }
            ResolvedPaint::Image { .. } => {
                let (c, _) = self.inks.color(&ColorValue::srgb(217.0 / 255.0, 221.0 / 255.0, 227.0 / 255.0, 1.0));
                (c.into(), 1.0)
            }
        };
        let dash = s.dash.as_ref().filter(|d| d.len() >= 2 && d.len() % 2 == 0 && d.iter().all(|v| *v >= 0.0) && d.iter().sum::<f64>() > 0.0);
        Some(KStroke {
            paint,
            width: width.max(0.0) as f32,
            miter_limit: 4.0,
            line_cap: match s.cap {
                StrokeCap::Round => LineCap::Round,
                StrokeCap::Square => LineCap::Square,
                _ => LineCap::Butt,
            },
            line_join: match s.join {
                StrokeJoin::Round => LineJoin::Round,
                StrokeJoin::Bevel => LineJoin::Bevel,
                _ => LineJoin::Miter,
            },
            opacity: nf(opacity),
            dash: dash.map(|d| StrokeDash { array: d.iter().map(|v| (v * s.width.max(1.0)) as f32).collect(), offset: 0.0 }),
        })
    }

    fn stroke_with(&mut self, c: &mut Ctx<'_, '_>, pd: &PathData, s: &DocStroke, p: &ResolvedPaint) {
        let Some(path) = kpath(pd) else { return };
        let op = self.black_hairline(c, p, s.width);
        match s.align {
            StrokeAlign::Inside | StrokeAlign::Outside => {
                let Some(st) = self.kstroke(s, p, s.width * 2.0) else { return };
                if s.align == StrokeAlign::Inside {
                    c.s.push_clip_path(&path, &FillRule::NonZero);
                } else {
                    let mut out = PathData::rect(-3.0e4, -3.0e4, 6.0e4, 6.0e4);
                    out.extend(pd);
                    let Some(outside) = kpath(&out) else { return };
                    c.s.push_clip_path(&outside, &FillRule::EvenOdd);
                }
                self.draw_stroke(c, &path, st, op);
                c.s.pop();
            }
            _ => {
                let Some(st) = self.kstroke(s, p, s.width) else { return };
                self.draw_stroke(c, &path, st, op);
            }
        }
    }

    fn line(&mut self, c: &mut Ctx<'_, '_>, pts: &[[f64; 2]], s: &DocStroke, p: &ResolvedPaint, start: &LineMarker, end: &LineMarker) {
        if pts.len() < 2 {
            return;
        }
        let body = trim_line_for_markers(pts, start, end, s.width);
        let width = if s.align == StrokeAlign::Center || !s.align.is_known() { s.width } else { s.width * 2.0 };
        let op = self.black_hairline(c, p, s.width);
        if let Some(path) = kpath(&PathData::polyline(&body))
            && let Some(st) = self.kstroke(s, p, width)
        {
            self.draw_stroke(c, &path, st, op);
        }
        let n = pts.len();
        for (kind, from, at) in [(start, pts[1], pts[0]), (end, pts[n - 2], pts[n - 1])] {
            let Some(m) = line_marker_shape(kind, s.width) else { continue };
            let Some(pd) = crate::path::parse(&m.d) else { continue };
            let ang = (at[1] - from[1]).atan2(at[0] - from[0]);
            c.s.push_transform(&KTransform::from_row(ang.cos() as f32, ang.sin() as f32, -ang.sin() as f32, ang.cos() as f32, at[0] as f32, at[1] as f32));
            match m.mode {
                MarkerMode::Fill => self.fill_with(c, &pd, p),
                MarkerMode::Stroke => {
                    let undashed = DocStroke { dash: None, ..s.clone() };
                    if let (Some(path), Some(st)) = (kpath(&pd), self.kstroke(&undashed, p, width)) {
                        self.draw_stroke(c, &path, st, op);
                    }
                }
            }
            c.s.pop();
        }
    }

    /// A krilla image for an asset (edited images baked first), with its pixel size.
    fn embed(&mut self, asset: &str, crop: Option<&Rect>, edits: Option<&ImageEditStack>) -> Option<(krilla::image::Image, f64, f64)> {
        if self.inks.all_cmyk {
            // PDF/X-1a: every image to DeviceCMYK through the profile (transparent ones were flattened).
            let (px, w, h, nw, nh) = rgba_pixels(self.r, self.assets, asset, crop, edits)?;
            return opaque_image(&px, w, h, self.inks.conv.as_deref()).map(|i| (i, nw, nh));
        }
        let interpolate = !self.inks.strict;
        let adjust = edits.and_then(|e| e.adjust.as_ref());
        let edited = edit_matrix(edits).is_some() || needs_tone_shader(adjust) || adjust.is_some_and(|a| a.blur.is_some_and(|b| b > 0.0) || a.vignette.is_some_and(|v| v > 0.0));
        if edited {
            let unit = Rect::UNIT;
            let p = self.r.bake_image(asset, crop.unwrap_or(&unit), edits, self.assets)?;
            let (w, h) = (p.width(), p.height());
            return Some((krilla::image::Image::from_rgba8(crate::raster::to_rgba8(&p), w, h), f64::from(w), f64::from(h)));
        }
        let bytes = self.assets.asset(asset).map(|b| b.into_owned());
        let decoded = match &bytes {
            Some(b) if !crate::assets::is_svg(b) => {
                let data: krilla::Data = Arc::new(b.clone()).into();
                let img = match crate::svg::sniff_mime(b) {
                    "image/png" => krilla::image::Image::from_png(data, interpolate).ok(),
                    "image/jpeg" => krilla::image::Image::from_jpeg(data, interpolate).ok(),
                    "image/gif" => krilla::image::Image::from_gif(data, interpolate).ok(),
                    "image/webp" => krilla::image::Image::from_webp(data, interpolate).ok(),
                    _ => None,
                };
                img.map(|i| {
                    let (w, h) = i.size();
                    (i, f64::from(w), f64::from(h))
                })
            }
            _ => None,
        };
        if decoded.is_some() {
            return decoded;
        }
        // SVG sources, recoloured variants, other formats: the painter's decode at native size (SVG at 2×).
        match self.r.images_get(asset, self.assets)? {
            Decoded::Raster(rimg) => {
                let p = &rimg.pixmap;
                Some((krilla::image::Image::from_rgba8(crate::raster::to_rgba8(p), p.width(), p.height()), f64::from(p.width()), f64::from(p.height())))
            }
            Decoded::Svg(s) => {
                let (w, h) = s.size();
                let ras = s.raster((w * 2.0).ceil() as u32, (h * 2.0).ceil() as u32)?;
                let p = &ras.pixmap;
                Some((krilla::image::Image::from_rgba8(crate::raster::to_rgba8(p), p.width(), p.height()), f64::from(w), f64::from(h)))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn image(&mut self, c: &mut Ctx<'_, '_>, asset: &str, dst: &Rect, crop: &Rect, edits: Option<&ImageEditStack>, clip: Option<&str>, video: bool) {
        let baked = edits.is_some_and(|e| edit_matrix(Some(e)).is_some() || needs_tone_shader(e.adjust.as_ref()) || e.adjust.as_ref().is_some_and(|a| a.blur.is_some_and(|b| b > 0.0) || a.vignette.is_some_and(|v| v > 0.0)));
        let mut pushes = 0;
        if let Some(cp) = clip.and_then(crate::path::parse).and_then(|p| kpath(&p)) {
            c.s.push_clip_path(&cp, &FillRule::NonZero);
            pushes += 1;
        }
        match self.embed(asset, Some(crop), edits) {
            None => {
                let col = if video { ColorValue::srgb(0.1, 0.1, 0.12, 1.0) } else { ColorValue::srgb(0.88, 0.9, 0.93, 1.0) };
                self.solid_rect(c, dst.x, dst.y, dst.w, dst.h, &col);
            }
            Some((img, _, _)) => {
                let cr = if baked { Rect::UNIT } else { *crop };
                if cr.w > 0.0 && cr.h > 0.0 {
                    let iw = dst.w / cr.w;
                    let ih = dst.h / cr.h;
                    if let Some(p) = rect_path(dst.x, dst.y, dst.w, dst.h) {
                        c.s.push_clip_path(&p, &FillRule::NonZero);
                        pushes += 1;
                    }
                    if let Some(size) = KSize::from_wh(iw as f32, ih as f32) {
                        c.s.push_transform(&KTransform::from_translate((dst.x - cr.x * iw) as f32, (dst.y - cr.y * ih) as f32));
                        c.s.draw_image(img, size);
                        c.s.pop();
                    }
                }
            }
        }
        for _ in 0..pushes {
            c.s.pop();
        }
    }

    /// Draw a page with transparency flattened: opaque units as vectors, transparent ones replaced by an opaque
    /// raster of their region (everything underneath composited in), placed where the unit was in the stacking
    /// order so later elements stay vector on top.
    fn flattened(&mut self, c: &mut Ctx<'_, '_>, dl: &DisplayList, plan: &[(usize, bool, Option<Region>)]) {
        let us = units(&dl.items);
        for (ui, u) in us.iter().enumerate() {
            let (_, transparent, region) = &plan[ui];
            if !transparent {
                self.items(c, &dl.items[u.start..u.end]);
            }
            if let Some(rg) = region {
                self.flat_region(c, dl, u.end, rg);
            }
        }
    }

    /// Rasterise the page (background + `items[..upto]`) inside `rg` over white paper, and draw it opaque.
    fn flat_region(&mut self, c: &mut Ctx<'_, '_>, dl: &DisplayList, upto: usize, rg: &Region) {
        let (x0, y0, x1, y1) = rg.rect;
        let (w, h) = (x1 - x0, y1 - y0);
        let mut k = PRINT_SCALE.max(self.opts.raster_scale);
        if w * h * k * k > MAX_REGION_PX {
            k = (MAX_REGION_PX / (w * h)).sqrt();
        }
        let b = self.bleed;
        let mut items: Vec<DLItem> = Vec::with_capacity(upto + 1);
        if !dl.background.is_empty() {
            let (bx0, by0, bx1, by1) = (-b, -b, dl.width + b, dl.height + b);
            items.push(DLItem::Path { path: format!("M{bx0} {by0}L{bx1} {by0}L{bx1} {by1}L{bx0} {by1}Z"), fills: dl.background.clone(), stroke: None, src_el: String::new() });
        }
        items.extend_from_slice(&dl.items[..upto]);
        let Some(pix) = self.r.rasterize_items(&items, &oc_geometry::IDENTITY, x0, y0, w, h, k, self.assets) else { return };
        let (pw, ph) = (pix.width(), pix.height());
        let Some(img) = opaque_image(&crate::raster::to_rgba8(&pix), pw, ph, if self.inks.all_cmyk { self.inks.conv.as_deref() } else { None }) else { return };
        let Some(size) = KSize::from_wh((f64::from(pw) / k) as f32, (f64::from(ph) / k) as f32) else { return };
        c.s.push_transform(&KTransform::from_translate(x0 as f32, y0 as f32));
        c.s.draw_image(img, size);
        c.s.pop();
    }

    /// Rasterise items (a layer group or a text item) in page space and place the image.
    fn raster_group(&mut self, c: &mut Ctx<'_, '_>, items: &[DLItem], effects: &[Effect]) {
        let Some((x0, y0, x1, y1)) = local_bounds(items) else { return };
        let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| oc_geometry::apply(&c.ctm, [x, y]));
        let scale = (c.ctm[0].hypot(c.ctm[1])).max(c.ctm[2].hypot(c.ctm[3]));
        let reach: f64 = effects
            .iter()
            .map(|e| match e {
                Effect::DropShadow(d) => d.dx.abs() + d.dy.abs() + d.blur * 1.5,
                Effect::Blur(b) => b.radius * 1.5,
                Effect::Outline(o) => o.width * 2.0,
            })
            .sum::<f64>()
            * scale
            + 4.0;
        let b = self.bleed;
        let px0 = (corners.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min) - reach).max(-b);
        let py0 = (corners.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min) - reach).max(-b);
        let px1 = (corners.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max) + reach).min(self.page_w + b);
        let py1 = (corners.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max) + reach).min(self.page_h + b);
        if px1 <= px0 || py1 <= py0 {
            return;
        }
        let k = self.opts.raster_scale;
        let Some(pix) = self.r.rasterize_items(items, &c.ctm, px0, py0, px1 - px0, py1 - py0, k, self.assets) else { return };
        // Crop to what was drawn.
        let (w, h) = (pix.width() as usize, pix.height() as usize);
        let (mut l, mut t, mut rr, mut bb) = (w, h, 0usize, 0usize);
        for (i, p) in pix.pixels().iter().enumerate() {
            if p.alpha() > 0 {
                let (x, y) = (i % w, i / w);
                l = l.min(x);
                t = t.min(y);
                rr = rr.max(x + 1);
                bb = bb.max(y + 1);
            }
        }
        if rr <= l || bb <= t {
            return;
        }
        let Some(sub) = tiny_skia::IntRect::from_ltrb(l as i32, t as i32, rr as i32, bb as i32).and_then(|r| pix.clone_rect(r)) else { return };
        let img = krilla::image::Image::from_rgba8(crate::raster::to_rgba8(&sub), sub.width(), sub.height());
        let Some(inv) = invert(&c.ctm) else { return };
        let Some(size) = KSize::from_wh((sub.width() as f64 / k) as f32, (sub.height() as f64 / k) as f32) else { return };
        c.s.push_transform(&kt(&inv));
        c.s.push_transform(&KTransform::from_translate((px0 + l as f64 / k) as f32, (py0 + t as f64 / k) as f32));
        c.s.draw_image(img, size);
        c.s.pop();
        c.s.pop();
    }

    /// Vector text. `alpha`: `Some(0.0)` draws it invisible (selectable text under a raster).
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, c: &mut Ctx<'_, '_>, key: &str, body: &TextBody, b: &Rect, effect: Option<&TextEffect>, valign: Option<&CellVAlign>, curve: Option<f64>, fit: bool, alpha: Option<f64>) {
        if let Some(cv) = curve
            && let Some(cl) = self.r.text.curved(key, body, cv)
        {
            let s = (b.w / cl.width).min(b.h / cl.height);
            let ox = b.x + (b.w - cl.width * s) / 2.0;
            let oy = b.y + (b.h - cl.height * s) / 2.0;
            let chars: Vec<char> = body.paragraphs.iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join(" ").chars().collect();
            for (gi, g) in cl.glyphs.iter().enumerate() {
                let Some(font) = self.font(&g.face) else { continue };
                let st = &cl.styles[g.style];
                let fill = paint_to_color(Some(&st.fill));
                let op = alpha.is_none() && self.inks.knockout_black(&fill, g.size * s * ctm_scale(&c.ctm) * PT < BLACK_TEXT_PT);
                let (col, a) = if op { (Inks::cmyk([0.0, 0.0, 0.0, 1.0]), 1.0) } else { self.inks.color(&fill) };
                let text: String = if chars.len() == cl.glyphs.len() { chars[gi].to_string() } else { String::new() };
                c.s.push_transform(&KTransform::from_row(
                    (g.cos * s) as f32,
                    (g.sin * s) as f32,
                    (-g.sin * s) as f32,
                    (g.cos * s) as f32,
                    (ox + g.tx * s) as f32,
                    (oy + g.ty * s) as f32,
                ));
                c.s.set_stroke(None);
                c.s.set_fill(Some(KFill { paint: col.into(), opacity: nf(alpha.unwrap_or(a)), rule: FillRule::NonZero }));
                let glyph = KrillaGlyph { glyph_id: GlyphId::new(g.id), text_range: 0..text.len(), x_advance: 0.0, x_offset: 0.0, y_offset: 0.0, y_advance: 0.0, location: None };
                if op {
                    c.s.push_overprint();
                }
                c.s.draw_glyphs(KPoint::from_xy(0.0, 0.0), &[glyph], font, &text, g.size as f32, false);
                if op {
                    c.s.pop();
                }
                c.s.pop();
            }
            return;
        }
        let layout = if fit { self.r.text.layout_fit(key, body, b.w, b.h, effect).0 } else { self.r.text.layout(key, body, b.w, effect) };
        let dy = match valign {
            Some(CellVAlign::Middle) => (b.h - layout.height) / 2.0,
            Some(CellVAlign::Bottom) => b.h - layout.height,
            _ => 0.0,
        };
        let (ox, oy) = (b.x, b.y + dy);
        if alpha.is_none()
            && let Some(TextEffect::Background(bg)) = effect
        {
            let pad = bg.padding;
            for p in &layout.paras {
                for m in &p.lines {
                    let pd = PathData::rrect(ox + p.x + m.left - pad, oy + p.y + m.baseline - m.ascent - pad / 2.0, m.width + pad * 2.0, m.ascent + m.descent + pad, bg.radius, bg.radius);
                    self.fill_with(c, &pd, &ResolvedPaint::Solid { color: bg.color.clone() });
                }
            }
        }
        for (pi, p) in layout.paras.iter().enumerate() {
            let lh = body.paragraphs.get(pi).map_or(1.3, |p| p.line_height);
            if alpha.is_none()
                && let Some(TextEffect::Outline(o)) = effect
            {
                self.paragraph(c, p, ox, oy, lh, Look::Stroke(o.width, Some(o.color.clone())), None);
            }
            let look = match effect {
                Some(TextEffect::Hollow(h)) if alpha.is_none() => Look::Stroke(h.width, None),
                _ => Look::Fill,
            };
            self.paragraph(c, p, ox, oy, lh, look, alpha);
            if let Some(bullet) = &p.bullet {
                self.paragraph(c, bullet, ox, oy, lh, Look::Fill, alpha);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paragraph(&mut self, c: &mut Ctx<'_, '_>, p: &LaidParagraph, ox: f64, oy: f64, line_height: f64, look: Look, alpha: Option<f64>) {
        let (px, py) = (ox + p.x, oy + p.y);
        // UTF-16 offset → byte offset in the laid-out text.
        let mut byte_of: Vec<usize> = Vec::with_capacity(p.text.len() + 1);
        for (bi, ch) in p.text.char_indices() {
            for _ in 0..ch.len_utf16() {
                byte_of.push(bi);
            }
        }
        byte_of.push(p.text.len());
        let b16 = |u: usize| byte_of.get(u).copied().unwrap_or(p.text.len());
        for line in &p.lines {
            for run in &line.glyph_runs {
                let Some(st) = p.styles.get(run.style).cloned() else { continue };
                let col = paint_to_color(Some(&st.fill));
                let draw_extras = alpha.is_none() && matches!(look, Look::Fill | Look::Stroke(_, None));
                if alpha.is_none()
                    && let Some(h) = &st.highlight
                    && let Some(face) = self.r.text.fonts.face_by_file(&run.face)
                {
                    let fm = face.info.metrics;
                    let hh = line_height * run.size;
                    let asc = (hh + fm.ascent * run.size - fm.descent * run.size) / 2.0;
                    let baseline = run.glyphs.first().map_or(line.top + line.baseline, |g| g.y);
                    self.fill_with(c, &PathData::rect(px + run.x, py + baseline - asc, run.advance, hh), &ResolvedPaint::Solid { color: h.clone() });
                }
                self.glyph_run(c, run, line, px, py, &p.text, &b16, &look, &col, &st, alpha);
                if draw_extras && (st.underline == Some(true) || st.strike == Some(true)) {
                    let Some(face) = self.r.text.fonts.face_by_file(&run.face) else { continue };
                    let fm = face.info.metrics;
                    let baseline = py + run.glyphs.first().map_or(line.top + line.baseline, |g| g.y);
                    let mut bars = Vec::new();
                    if st.underline == Some(true) {
                        bars.push((fm.underline_offset, fm.underline_thickness));
                    }
                    if st.strike == Some(true) {
                        bars.push((fm.strikeout_offset, fm.strikeout_thickness));
                    }
                    for (off, th) in bars {
                        let th = if th > 0.0 { th * run.size } else { run.size / 14.0 };
                        let center = baseline - off * run.size + th / 2.0;
                        self.fill_with(c, &PathData::rect(px + run.x, center - th / 2.0, run.advance, th), &ResolvedPaint::Solid { color: col.clone() });
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn glyph_run(&mut self, c: &mut Ctx<'_, '_>, run: &GlyphRun, line: &LaidLine, px: f64, py: f64, text: &str, b16: &dyn Fn(usize) -> usize, look: &Look, col: &ColorValue, st: &oc_schema::text::RunStyle, alpha: Option<f64>) {
        if run.glyphs.is_empty() {
            return;
        }
        let Some(font) = self.font(&run.face) else { return };
        let size = run.size as f32;
        // Each glyph's text: the cluster it starts in, extended over clusters no glyph starts in (ligatures).
        let cluster_of = |x: f64| line.clusters.iter().position(|cl| x >= cl.x - 0.01 && x < cl.x + cl.advance.max(0.02) - 0.01);
        let ranges: Vec<Range<usize>> = run
            .glyphs
            .iter()
            .map(|g| match cluster_of(g.x) {
                Some(ci) => {
                    let cl = &line.clusters[ci];
                    b16(cl.start)..b16(cl.end)
                }
                None => 0..0,
            })
            .collect();
        let n = run.glyphs.len();
        let glyphs: Vec<KrillaGlyph> = run
            .glyphs
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let adv = if i + 1 < n { run.glyphs[i + 1].x - g.x } else { g.advance };
                KrillaGlyph { glyph_id: GlyphId::new(g.id), text_range: ranges[i].clone(), x_advance: (adv / run.size) as f32, x_offset: 0.0, y_offset: 0.0, y_advance: 0.0, location: None }
            })
            .collect();
        let start = KPoint::from_xy((px + run.glyphs[0].x) as f32, (py + run.glyphs[0].y) as f32);
        let skew = run.synthetic_italic;
        if skew {
            c.s.push_transform(&KTransform::from_row(1.0, 0.0, -0.25, 1.0, (0.25 * (py + run.glyphs[0].y)) as f32, 0.0));
        }
        match look {
            Look::Fill => {
                let op = alpha.is_none() && self.inks.knockout_black(col, run.size * ctm_scale(&c.ctm) * PT < BLACK_TEXT_PT);
                let (k, a) = if op { (Inks::cmyk([0.0, 0.0, 0.0, 1.0]), 1.0) } else { self.inks.color(col) };
                c.s.set_stroke(None);
                c.s.set_fill(Some(KFill { paint: k.into(), opacity: nf(alpha.unwrap_or(a)), rule: FillRule::NonZero }));
                if op {
                    c.s.push_overprint();
                }
                c.s.draw_glyphs(start, &glyphs, font, text, size, false);
                if op {
                    c.s.pop();
                }
            }
            Look::Stroke(wd, color) => {
                let (k, a) = self.inks.color(color.as_ref().unwrap_or(col));
                let width = (wd * st.size / 100.0).max(0.5);
                c.s.set_fill(None);
                c.s.set_stroke(Some(KStroke { paint: k.into(), width: width as f32, line_join: LineJoin::Round, opacity: nf(a), ..Default::default() }));
                c.s.draw_glyphs(start, &glyphs, font, text, size, false);
                c.s.set_stroke(None);
            }
        }
        if skew {
            c.s.pop();
        }
    }
}

/// How glyphs are painted: filled, or stroked (`Some(colour)`: the outline effect's under-layer).
enum Look {
    Fill,
    Stroke(f64, Option<ColorValue>),
}

/// The larger axis scale of a transform.
fn ctm_scale(m: &[f64; 6]) -> f64 {
    m[0].hypot(m[1]).max(m[2].hypot(m[3]))
}

fn invert(m: &[f64; 6]) -> Option<[f64; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    Some(oc_geometry::invert(m))
}

fn pdf_blend(b: &DocBlend) -> Option<krilla::blend::BlendMode> {
    use krilla::blend::BlendMode as K;
    Some(match b {
        DocBlend::Multiply => K::Multiply,
        DocBlend::Screen => K::Screen,
        DocBlend::Overlay => K::Overlay,
        DocBlend::Darken => K::Darken,
        DocBlend::Lighten => K::Lighten,
        DocBlend::ColorDodge => K::ColorDodge,
        DocBlend::ColorBurn => K::ColorBurn,
        DocBlend::HardLight => K::HardLight,
        DocBlend::SoftLight => K::SoftLight,
        DocBlend::Difference => K::Difference,
        DocBlend::Exclusion => K::Exclusion,
        DocBlend::Hue => K::Hue,
        DocBlend::Saturation => K::Saturation,
        DocBlend::Color => K::Color,
        DocBlend::Luminosity => K::Luminosity,
        DocBlend::Normal | DocBlend::Other(_) => return None,
    })
}

/// Bounds of what a group of items draws, in the coordinate space of the group's first item (nested saves
/// applied); text boxes are padded for text that overflows its box.
fn local_bounds(items: &[DLItem]) -> Option<(f64, f64, f64, f64)> {
    let mut stack: Vec<[f64; 6]> = vec![oc_geometry::IDENTITY];
    let mut b: Option<(f64, f64, f64, f64)> = None;
    let mut add = |m: &[f64; 6], x0: f64, y0: f64, x1: f64, y1: f64| {
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            let p = oc_geometry::apply(m, [x, y]);
            b = Some(match b {
                None => (p[0], p[1], p[0], p[1]),
                Some((a, c, d, e)) => (a.min(p[0]), c.min(p[1]), d.max(p[0]), e.max(p[1])),
            });
        }
    };
    for it in items {
        let m = *stack.last().expect("stack");
        match it {
            DLItem::Save { matrix } => stack.push(mul(&m, matrix)),
            DLItem::Layer { .. } => stack.push(m),
            DLItem::Restore => {
                if stack.len() > 1 {
                    stack.pop();
                }
            }
            DLItem::Mask { .. } => stack.push(m),
            DLItem::Clip { .. } | DLItem::MaskContent => {}
            DLItem::Path { path, stroke, .. } => {
                if let Some((x0, y0, x1, y1)) = crate::path::parse(path).and_then(|p| p.bounds()) {
                    let w = stroke.as_ref().map_or(0.0, |s| s.stroke.width * 2.0);
                    add(&m, x0 - w, y0 - w, x1 + w, y1 + w);
                }
            }
            DLItem::Line { points, stroke, .. } => {
                let w = stroke.width.max(8.0) * 3.0;
                for p in points {
                    add(&m, p[0] - w, p[1] - w, p[0] + w, p[1] + w);
                }
            }
            DLItem::Image { dst, .. } | DLItem::Video { dst, .. } => add(&m, dst.x, dst.y, dst.x + dst.w, dst.y + dst.h),
            DLItem::Text { bbox, body, .. } => {
                let size = body.paragraphs.iter().flat_map(|p| p.runs.iter()).map(|r| oc_schema::text::resolve_run_style(body, r).size).fold(body.default_run.size, f64::max);
                let pad = size * 2.0;
                add(&m, bbox.x - pad, bbox.y - pad, bbox.x + bbox.w + pad, bbox.y + bbox.h.max(size) * 3.0 + pad);
            }
            DLItem::Qr { bbox, .. } | DLItem::Placeholder { bbox, .. } => add(&m, bbox.x, bbox.y, bbox.x + bbox.w, bbox.y + bbox.h),
        }
    }
    b
}
