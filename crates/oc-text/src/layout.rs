// SPDX-License-Identifier: Apache-2.0
//! Paragraph layout (`packages/text/src/layout.ts`). One parley layout per model paragraph; shaping (HarfRust),
//! bidi, line breaking (ICU4X UAX #14) and alignment come from parley. The vertical model is SkParagraph's, as the
//! TS configured it, reproduced here so heights match the TS engine:
//!
//! - every run's line box is `lineHeight × fontSize`, split around the baseline with *half leading*:
//!   `ascent = (H + A − D) / 2`, `descent = (H − A + D) / 2` (A/D: the face's ascent/descent at that size);
//! - a *strut* (the body's default family, `max(run sizes)`, same multiplier, half leading) sets the minimum
//!   ascent/descent of every line;
//! - a line's height is `round(max descent + max ascent)` — SkParagraph rounds line heights to whole pixels —
//!   and the paragraph height is the sum of its line heights;
//! - an empty paragraph has SkParagraph's "empty metrics": the default run's face, plus the leading needed to
//!   reach `lineHeight × size` from its *rounded* intrinsic height, merged with the strut;
//! - the wrap width is floored to whole pixels (SkParagraph's `applyRoundingHack`).
//!
//! All metric arithmetic is done in `f32` in the same order as Skia so that `.5` cases round the same way.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use oc_schema::primitives::{ColorValue, Paint};
use oc_schema::text::{ListKind, RunStyle, TextAlign, TextBody, TextEffect, resolve_run_style};
use icu_segmenter::LineSegmenter;
use icu_segmenter::options::{LineBreakOptions, LineBreakStrictness};
use icu_properties::{CodePointMapData, CodePointSetData};
use icu_properties::props::{DefaultIgnorableCodePoint, Ideographic};
use icu_segmenter::GraphemeClusterSegmenter;
use parley::setting::Tag;
use parley::{BaseDirection, FontFamily, FontFamilyName, FontFeature, FontFeatures, Layout, LayoutContext, LineHeight, StyleProperty};

use crate::case::apply_case;
use crate::fallback::fallback_families;
use crate::fonts::{FALLBACK_FAMILY, FaceRequest, FontRegistry, LoadedFace};
use crate::inspect::FaceMetrics;
use crate::utf16::Utf16Map;
use crate::wrap::{WrapCluster, break_lines, justify_shifts};

/// Model run index carried through parley as the "brush".
type Brush = u32;

/// A positioned glyph in paragraph space. `y` is on the painted baseline, which SkParagraph rounds to whole pixels
/// (`top + round(ascent)`, not [`LaidLine::baseline`]). Tracking is added after every glyph, as in SkParagraph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    pub id: u32,
    pub x: f64,
    pub y: f64,
    /// Advance including tracking (justification widens the gaps between clusters, see [`ClusterBox`]).
    pub advance: f64,
}

/// Glyphs sharing one face, size and model run.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    /// Manifest file of the face (look it up with [`FontRegistry::face_by_file`]).
    pub face: String,
    pub size: f64,
    /// Index into [`LaidParagraph::styles`] (the model run).
    pub style: usize,
    /// Italic was asked for but the face is upright (renderers may skew).
    pub synthetic_italic: bool,
    pub rtl: bool,
    pub x: f64,
    pub advance: f64,
    pub glyphs: Vec<PositionedGlyph>,
}

/// One grapheme cluster, for caret and selection geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClusterBox {
    /// UTF-16 range in [`LaidParagraph::text`].
    pub start: usize,
    pub end: usize,
    pub x: f64,
    pub advance: f64,
    pub rtl: bool,
    pub whitespace: bool,
}

/// One line (SkParagraph `LineMetrics` plus glyphs). Offsets are UTF-16 into [`LaidParagraph::text`].
#[derive(Debug, Clone, PartialEq)]
pub struct LaidLine {
    pub start: usize,
    pub end: usize,
    pub end_no_ws: usize,
    pub hard_break: bool,
    /// Top of the line box (paragraph space).
    pub top: f64,
    /// Whole pixels.
    pub height: f64,
    /// `LineMetrics.baseline`: `top + height − descent`.
    pub baseline: f64,
    /// `LineMetrics.ascent/descent`: the largest run ascent and descent on the line (the strut not included; it only
    /// counts for `height` and `baseline`).
    pub ascent: f64,
    pub descent: f64,
    /// `LineMetrics.left`: the line's shift (alignment; half a letter spacing for unshifted tracked lines).
    pub left: f64,
    /// `LineMetrics.width` (to 0.01 px): the visible advance (trailing whitespace excluded; the full width for
    /// justified lines).
    pub width: f64,
    /// Glyphs to paint, in visual order (trailing whitespace is not painted, as in SkParagraph).
    pub glyph_runs: Vec<GlyphRun>,
    /// Clusters in visual order, trailing whitespace included.
    pub clusters: Vec<ClusterBox>,
}

/// A laid-out model paragraph (or a list marker).
#[derive(Debug, Clone, PartialEq)]
pub struct LaidParagraph {
    /// Box-space position.
    pub x: f64,
    pub y: f64,
    /// UTF-16 length of the model text (the paragraph separator is not included).
    pub len: usize,
    /// Flat UTF-16 offset of the paragraph's first character.
    pub start: usize,
    pub height: f64,
    pub align: TextAlign,
    /// Layout width (before flooring).
    pub width: f64,
    /// The laid-out text (case transforms applied).
    pub text: String,
    /// Resolved style of each model run.
    pub styles: Vec<RunStyle>,
    pub lines: Vec<LaidLine>,
    /// `getLongestLine`: the widest line before justification; `f32::MIN` for an empty paragraph, as SkParagraph.
    pub longest_line: f64,
    /// `getMaxIntrinsicWidth`: the unbroken width (to 0.01 px).
    pub max_intrinsic_width: f64,
    /// List marker, positioned in box space (`x = para.x − marker width − 6`).
    pub bullet: Option<Box<LaidParagraph>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    pub width: f64,
    pub height: f64,
    pub paras: Vec<LaidParagraph>,
    /// Total flat UTF-16 length (paragraph separators count as one).
    pub length: usize,
    pub effect: Option<TextEffect>,
}

/// Text on an arc (ED-041): glyph placements in a box of `width × height`.
#[derive(Debug, Clone, PartialEq)]
pub struct CurvedGlyph {
    pub id: u32,
    pub face: String,
    pub size: f64,
    /// Index into [`CurvedLayout::styles`].
    pub style: usize,
    pub cos: f64,
    pub sin: f64,
    pub tx: f64,
    pub ty: f64,
}

/// A contiguous same-face, same-size, same-style glyph range (`TextBlob` in the TS).
#[derive(Debug, Clone, PartialEq)]
pub struct CurvedRun {
    pub face: String,
    pub size: f64,
    pub style: usize,
    pub color: ColorValue,
    /// Range in [`CurvedLayout::glyphs`].
    pub glyphs: std::ops::Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CurvedLayout {
    pub width: f64,
    pub height: f64,
    pub styles: Vec<RunStyle>,
    pub runs: Vec<CurvedRun>,
    pub glyphs: Vec<CurvedGlyph>,
}

/// Scale every font size in a body (shrink-to-fit), `scaleBody`.
pub fn scale_body(body: &TextBody, s: f64) -> TextBody {
    let mut out = body.clone();
    out.default_run.size = body.default_run.size * s;
    for p in &mut out.paragraphs {
        for r in &mut p.runs {
            if let Some(size) = r.style.size.filter(|v| *v != 0.0) {
                r.style.size = Some(size * s);
            }
        }
    }
    out
}

/// First colour of a paint (`paintToColor`).
pub fn paint_to_color(p: Option<&Paint>) -> ColorValue {
    let black = || ColorValue::srgb(0.0, 0.0, 0.0, 1.0);
    match p {
        Some(Paint::Solid(s)) => s.color.clone(),
        Some(Paint::Linear(l)) => l.stops.first().map(|s| s.color.clone()).unwrap_or_else(black),
        Some(Paint::Radial(r)) => r.stops.first().map(|s| s.color.clone()).unwrap_or_else(black),
        _ => black(),
    }
}

fn paragraph_model_text(p: &oc_schema::text::Paragraph) -> String {
    p.runs.iter().map(|r| r.text.as_str()).collect()
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Every face a body references (`TextEngine.faces`, the export path's preload list).
pub fn faces(body: &TextBody) -> Vec<FaceRequest> {
    let d = &body.default_run;
    let mut out = vec![FaceRequest::new(d.family.clone(), d.weight, d.italic)];
    for p in &body.paragraphs {
        let fb = fallback_families(&paragraph_model_text(p));
        for r in &p.runs {
            let s = resolve_run_style(body, r);
            out.push(FaceRequest::new(s.family.clone(), s.weight, s.italic));
            for f in &fb {
                out.push(FaceRequest::new(*f, s.weight, false));
            }
        }
    }
    out
}

/// Every face the Rust engine may use for a body: [`faces`] plus each run's Inter (second family of every stack)
/// and fallback faces at the run's own slant, and the strut face (default family at 400 upright).
pub fn required_faces(fonts: &FontRegistry, body: &TextBody) -> Vec<FaceRequest> {
    let mut out = faces(body);
    let d = &body.default_run;
    out.push(FaceRequest::new(fonts.resolve_family(&d.family), 400.0, false));
    for p in &body.paragraphs {
        let fb: Vec<&str> = fallback_families(&paragraph_model_text(p)).into_iter().filter(|f| fonts.has(f)).collect();
        let styles: Vec<RunStyle> = if p.runs.is_empty() { vec![d.clone()] } else { p.runs.iter().map(|r| resolve_run_style(body, r)).collect() };
        for s in &styles {
            out.push(FaceRequest::new(FALLBACK_FAMILY, s.weight, s.italic));
            for f in &fb {
                out.push(FaceRequest::new(*f, s.weight, s.italic));
            }
        }
    }
    out
}

// ------------------------------------------------------------------ metrics (Skia model, f32)

/// Font metrics at a size, as SkFontMetrics (ascent negative).
#[derive(Clone, Copy)]
struct SkMetrics {
    ascent: f32,
    descent: f32,
    leading: f32,
}

fn sk_metrics(m: &FaceMetrics, size: f32) -> SkMetrics {
    SkMetrics { ascent: -(m.ascent as f32) * size, descent: (m.descent as f32) * size, leading: (m.leading as f32) * size }
}

/// `Run::calculateMetrics` with half leading: (correct ascent, correct descent).
fn run_line_metrics(m: &FaceMetrics, size: f32, height_multiplier: f32) -> (f32, f32) {
    let fm = sk_metrics(m, size);
    let mut asc = fm.ascent - fm.leading * 0.5;
    let mut desc = fm.descent + fm.leading * 0.5;
    if height_multiplier.abs() <= f32::EPSILON * 16.0 {
        return (asc, desc);
    }
    let run_height = height_multiplier * size;
    let intrinsic = desc - asc;
    let extra = (run_height - intrinsic) / 2.0;
    asc -= extra;
    desc += extra;
    (asc, desc)
}

/// `ParagraphImpl::resolveStrut` with half leading.
fn strut_metrics(m: &FaceMetrics, size: f32, height_multiplier: f32) -> (f32, f32) {
    let fm = sk_metrics(m, size);
    let occupied = fm.descent - fm.ascent;
    let flexible = height_multiplier * size - occupied;
    (fm.ascent - flexible / 2.0, fm.descent + flexible / 2.0)
}

fn sk_round(v: f64) -> f64 {
    v.round()
}

/// SkParagraph's `littleRound` (to 0.01 px), applied to reported widths "to match Flutter tests":
/// `SkScalarRoundToScalar(a * 100.0) / 100.0` with `a` an `f32`, the product in `f64`, the rounding in `f32`.
fn little_round(v: f64) -> f64 {
    let x = (f64::from(v as f32) * 100.0) as f32;
    f64::from(((f64::from((x + 0.5).floor())) / 100.0) as f32)
}

// ------------------------------------------------------------------ engine

struct CacheEntry {
    body: TextBody,
    width: f64,
    generation: u64,
    effect: Option<TextEffect>,
    layout: Arc<TextLayout>,
}

struct FitEntry {
    body: TextBody,
    width: f64,
    height: f64,
    generation: u64,
    scale: f64,
}

struct CurvedEntry {
    body: TextBody,
    curve: f64,
    generation: u64,
    layout: Option<Arc<CurvedLayout>>,
}

/// One styled span of a paragraph handed to the shaper.
struct Span {
    range: std::ops::Range<usize>,
    style: RunStyle,
    brush: Brush,
    /// Baseline shift (px, down positive) of a superscript/subscript run; its `style.size` is already reduced.
    dy: f64,
}

/// What a paragraph needs besides its spans.
struct ParaSpec<'a> {
    text: &'a str,
    spans: Vec<Span>,
    align: TextAlign,
    line_height: f64,
    width: f64,
    /// Strut: (face metrics, size). `None` disables it (list markers).
    strut: Option<(FaceMetrics, f64)>,
    /// Paragraph default style (empty-paragraph metrics).
    base: &'a RunStyle,
    fallback: &'a [&'static str],
}

/// A sub-range of a span bound to exactly one face (private fontique family name and its attributes).
struct Piece {
    range: std::ops::Range<usize>,
    key: String,
    weight: parley::FontWeight,
    style: parley::FontStyle,
    width: parley::FontWidth,
}

/// Skia sets the FreeType character size in 26.6 fixed point (`SkScalarToFDot6`, truncating), so glyph advances
/// scale with the size truncated to 1/64 px; metrics use the exact size.
fn advance_size(size: f64) -> f32 {
    ((size as f32 * 64.0) as i32) as f32 / 64.0
}

fn tracking_px(s: &RunStyle) -> f32 {
    (s.tracking / 1000.0 * s.size) as f32
}

/// SkParagraph disables standard ligatures for positive letter spacing (and only then, and only `liga`).
const LIGA_OFF: &[FontFeature] = &[FontFeature::new(Tag::new(b"liga"), 0)];

fn is_nbsp(c: char) -> bool {
    matches!(c, '\u{00A0}' | '\u{2007}' | '\u{202F}' | '\u{2060}' | '\u{FEFF}')
}

/// Line break opportunities (byte offsets, sorted) as ICU gives them to SkParagraph. Two differences between
/// ICU4X and ICU4C are corrected:
///
/// - ICU4X applies the untailored LB25 (`PO × OP`, `PR × OP`, `PO × HY`, `PR × HY` always); ICU's rules only keep
///   those pairs together when a number follows (`(PR | PO) CM* (OP | HY) CM* NU`), so `100%(note` breaks after `%`.
/// - ICU4X always breaks at the end of a run of complex-context (SA: Thai, Lao, Khmer, Myanmar) characters, where
///   its dictionary segmentation stops. ICU4C's rules see SA as AL (or CM for Mn/Mc, UAX #14 LB1) around the
///   dictionary range, so `ไทย.`, `ไทย2026`, `ไทยabc` or `ไทย)` have no break there and `ไทย ` breaks only after
///   the space. Those positions are decided by the rules on a copy of the text with SA replaced by AL/CM.
fn break_opportunities(text: &str) -> Vec<usize> {
    use icu_properties::props::LineBreak as Lb;
    let mut out: Vec<usize> = line_segmenter().segment_str(text).collect();
    let lb = CodePointMapData::<Lb>::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut changed = false;
    for k in 1..chars.len() {
        let prev = lb.get(chars[k - 1].1);
        let cur = lb.get(chars[k].1);
        if !matches!(prev, Lb::PostfixNumeric | Lb::PrefixNumeric) || !matches!(cur, Lb::OpenPunctuation | Lb::Hyphen) {
            continue;
        }
        let next = chars[k + 1..].iter().map(|(_, c)| lb.get(*c)).find(|p| *p != Lb::CombiningMark);
        if next != Some(Lb::Numeric) && out.binary_search(&chars[k].0).is_err() {
            out.push(chars[k].0);
            changed = true;
        }
    }
    if changed {
        out.sort_unstable();
    }
    if chars.iter().any(|(_, c)| lb.get(*c) == Lb::ComplexContext) {
        sa_run_ends(text, &chars, &mut out);
    }
    out
}

/// AL and CM stand-ins with the same UTF-8 length as the SA character they replace (index: length − 1).
const SA_AS_AL: [char; 4] = ['a', '\u{00DF}', '\u{0800}', '\u{10000}'];
const SA_AS_CM: [char; 4] = ['\u{0001}', '\u{0300}', '\u{20D0}', '\u{101FD}'];

/// Re-decide the break at the end of every SA run by the plain UAX #14 rules (SA resolved per LB1).
fn sa_run_ends(text: &str, chars: &[(usize, char)], out: &mut Vec<usize>) {
    use icu_properties::props::{GeneralCategory, LineBreak as Lb};
    let lb = CodePointMapData::<Lb>::new();
    let gc = CodePointMapData::<GeneralCategory>::new();
    let resolved: String = chars
        .iter()
        .map(|&(_, c)| {
            if lb.get(c) != Lb::ComplexContext {
                return c;
            }
            let n = c.len_utf8() - 1;
            if matches!(gc.get(c), GeneralCategory::NonspacingMark | GeneralCategory::SpacingMark) { SA_AS_CM[n] } else { SA_AS_AL[n] }
        })
        .collect();
    debug_assert_eq!(resolved.len(), text.len());
    let mut opt = LineBreakOptions::default();
    opt.strictness = Some(LineBreakStrictness::Normal);
    let rules: Vec<usize> = LineSegmenter::new_for_non_complex_scripts(opt).segment_str(&resolved).collect();
    for k in 1..chars.len() {
        if lb.get(chars[k - 1].1) != Lb::ComplexContext || lb.get(chars[k].1) == Lb::ComplexContext {
            continue;
        }
        let at = chars[k].0;
        let want = rules.binary_search(&at).is_ok();
        match out.binary_search(&at) {
            Ok(i) if !want => {
                out.remove(i);
            }
            Err(i) if want => out.insert(i, at),
            _ => {}
        }
    }
}

/// UAX #14 break opportunities (`line-break: normal`; dictionaries for SE-Asian scripts with `complex-scripts`).
fn line_segmenter() -> icu_segmenter::LineSegmenterBorrowed<'static> {
    let mut opt = LineBreakOptions::default();
    opt.strictness = Some(LineBreakStrictness::Normal);
    #[cfg(feature = "complex-scripts")]
    {
        LineSegmenter::new_dictionary(opt)
    }
    #[cfg(not(feature = "complex-scripts"))]
    {
        LineSegmenter::new_for_non_complex_scripts(opt)
    }
}

/// A face some layout needed but did not have (see [`TextEngine::report_missing_faces`]): the request as the text
/// asked for it, the family it resolved to and the manifest file that would satisfy it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MissingFace {
    pub family: String,
    pub weight: f64,
    pub italic: bool,
    pub file: String,
}

/// Missing-face log: every file is reported once (until the host forgets it, e.g. after a failed load).
#[derive(Debug, Default)]
struct MissingLog {
    reported: std::collections::HashSet<String>,
    queue: Vec<MissingFace>,
}

/// The text engine: a font registry plus parley's scratch space and per-key caches (`TextEngine`).
pub struct TextEngine {
    pub fonts: FontRegistry,
    lcx: LayoutContext<Brush>,
    cache: HashMap<String, CacheEntry>,
    fit_cache: HashMap<String, FitEntry>,
    curved_cache: HashMap<String, CurvedEntry>,
    /// `Some` when the host wants to hear about missing faces (the TS `requestFonts` side effect of layout).
    missing: Option<MissingLog>,
}

impl std::fmt::Debug for TextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextEngine").field("fonts", &self.fonts).field("cached", &self.cache.len()).finish_non_exhaustive()
    }
}

impl TextEngine {
    pub fn new(fonts: FontRegistry) -> Self {
        TextEngine { fonts, lcx: LayoutContext::new(), cache: HashMap::new(), fit_cache: HashMap::new(), curved_cache: HashMap::new(), missing: None }
    }

    /// Start (or stop) recording the faces layouts need but the registry has not loaded. The TS engine requested
    /// them from inside `layout` (`requestFonts`); this engine never does I/O, so a host that loads fonts lazily
    /// turns this on, drains [`Self::take_missing_faces`] after laying out or drawing, registers the bytes and
    /// re-measures (layout caches key on [`FontRegistry::generation`]). Each file is reported once.
    pub fn report_missing_faces(&mut self, on: bool) {
        match (on, self.missing.is_some()) {
            (true, false) => self.missing = Some(MissingLog::default()),
            (false, true) => self.missing = None,
            _ => {}
        }
    }

    /// The missing faces recorded since the last call (each file once, in request order).
    pub fn take_missing_faces(&mut self) -> Vec<MissingFace> {
        self.missing.as_mut().map(|m| std::mem::take(&mut m.queue)).unwrap_or_default()
    }

    /// Number of recorded missing faces not taken yet.
    pub fn pending_missing_faces(&self) -> usize {
        self.missing.as_ref().map_or(0, |m| m.queue.len())
    }

    /// Allow `file` to be reported again (its load failed and may be retried).
    pub fn forget_missing_face(&mut self, file: &str) {
        if let Some(m) = self.missing.as_mut() {
            m.reported.remove(file);
        }
    }

    /// Record the faces `body` needs that are not loaded (when reporting is on).
    fn note_missing(&mut self, body: &TextBody) {
        let Some(log) = self.missing.as_mut() else { return };
        for r in required_faces(&self.fonts, body) {
            let Some((fam, face)) = self.fonts.pick_face(&r.family, r.weight, r.italic) else { continue };
            if self.fonts.face_by_file(&face.file).is_some() || log.reported.contains(&face.file) {
                continue;
            }
            log.reported.insert(face.file.clone());
            log.queue.push(MissingFace { family: fam.family.clone(), weight: r.weight, italic: r.italic, file: face.file.clone() });
        }
    }

    /// Files still needed to lay `body` out exactly (`requestFonts` without the I/O). Empty when ready.
    pub fn missing_fonts(&self, body: &TextBody) -> Vec<String> {
        let reqs = required_faces(&self.fonts, body);
        self.fonts.missing(reqs.iter())
    }

    /// `requestFonts`: true when every face the body needs is loaded.
    pub fn fonts_ready(&self, body: &TextBody) -> bool {
        self.missing_fonts(body).is_empty()
    }

    /// Nearest *loaded* face of a family by the `pickFace` rule (exact pick when loaded).
    fn loaded_face(&self, family: &str, weight: f64, italic: bool) -> Option<&LoadedFace> {
        if let Some(f) = self.fonts.loaded_face(family, weight, italic) {
            return Some(f);
        }
        let (fam, _) = self.fonts.pick_face(family, weight, italic)?;
        let loaded: Vec<&crate::manifest::FontFace> = fam.faces.iter().filter(|f| self.fonts.face_by_file(&f.file).is_some()).collect();
        let has_slant = loaded.iter().any(|f| f.italic == italic);
        let mut best: Option<&crate::manifest::FontFace> = None;
        for f in loaded.into_iter().filter(|f| !has_slant || f.italic == italic) {
            match best {
                Some(b) if (f64::from(f.weight) - weight).abs() >= (f64::from(b.weight) - weight).abs() => {}
                _ => best = Some(f),
            }
        }
        best.and_then(|f| self.fonts.face_by_file(&f.file))
    }

    /// The faces of a run's font stack: `[family, Inter, ...fallback]`, loaded ones only, deduplicated.
    fn stack(&self, s: &RunStyle, fallback: &[&str]) -> Vec<&LoadedFace> {
        let mut out: Vec<&LoadedFace> = Vec::new();
        let primary = self.fonts.resolve_family(&s.family).to_owned();
        for fam in std::iter::once(primary.as_str()).chain(std::iter::once(FALLBACK_FAMILY)).chain(fallback.iter().copied()) {
            if !self.fonts.has(fam) {
                continue;
            }
            if let Some(f) = self.loaded_face(fam, s.weight, s.italic)
                && !out.iter().any(|o| o.file == f.file)
            {
                out.push(f);
            }
        }
        out
    }

    fn face_is_italic(&self, file: &str) -> bool {
        self.fonts.manifest().families.iter().flat_map(|f| &f.faces).find(|f| f.file == file).is_some_and(|f| f.italic)
    }

    /// Font fallback as SkParagraph does it: each grapheme takes the first face of its run's stack that has
    /// glyphs for all of its (non-default-ignorable) characters; a grapheme no face covers stays with the first
    /// face. (parley would pick the face with the best *partial* coverage, e.g. one that only has the ZWJ of an
    /// emoji sequence.)
    fn pieces(&self, spec: &ParaSpec<'_>) -> Vec<Piece> {
        let ignorable = CodePointSetData::new::<DefaultIgnorableCodePoint>();
        let graphemes = GraphemeClusterSegmenter::new();
        let mut out: Vec<Piece> = Vec::new();
        for sp in &spec.spans {
            if sp.range.is_empty() {
                continue;
            }
            let stack = self.stack(&sp.style, spec.fallback);
            let Some(first) = stack.first() else { continue };
            let text = &spec.text[sp.range.clone()];
            let covered = |f: &LoadedFace, g: &str| g.chars().filter(|c| !ignorable.contains(*c)).all(|c| f.covers(c));
            let push = |out: &mut Vec<Piece>, f: &LoadedFace, r: std::ops::Range<usize>| match out.last_mut() {
                Some(last) if last.key == f.key && last.range.end == r.start => last.range.end = r.end,
                _ => out.push(Piece { range: r, key: f.key.clone(), weight: f.weight, style: f.style, width: f.width }),
            };
            if stack.len() == 1 || covered(first, text) {
                push(&mut out, first, sp.range.clone());
                continue;
            }
            let bounds: Vec<usize> = graphemes.segment_str(text).collect();
            let mut prev: &LoadedFace = first;
            for w in bounds.windows(2) {
                let g = &text[w[0]..w[1]];
                let face = if g.chars().all(|c| ignorable.contains(c)) {
                    prev
                } else {
                    stack.iter().copied().find(|f| covered(f, g)).unwrap_or(first)
                };
                push(&mut out, face, sp.range.start + w[0]..sp.range.start + w[1]);
                prev = face;
            }
        }
        out
    }

    /// Shape one paragraph with parley (no line breaking yet).
    fn shape(&mut self, spec: &ParaSpec<'_>, pieces: &[Piece]) -> Layout<Brush> {
        let lh = spec.line_height as f32;
        let mut builder = self.lcx.ranged_builder(&mut self.fonts.fcx, spec.text, 1.0, false);
        builder.set_base_direction(BaseDirection::Ltr);
        builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(lh)));
        for sp in &spec.spans {
            if sp.range.is_empty() {
                continue;
            }
            let r = sp.range.clone();
            builder.push(StyleProperty::Brush(sp.brush), r.clone());
            builder.push(StyleProperty::FontSize(advance_size(sp.style.size)), r.clone());
            // Letter spacing is applied here, per glyph (see `build`), not by parley (per atom).
            if tracking_px(&sp.style) > 0.0 {
                builder.push(StyleProperty::FontFeatures(FontFeatures::List(Cow::Borrowed(LIGA_OFF))), r.clone());
            }
            builder.push(StyleProperty::Underline(sp.style.underline.unwrap_or(false)), r.clone());
            builder.push(StyleProperty::Strikethrough(sp.style.strike.unwrap_or(false)), r);
        }
        for p in pieces {
            let r = p.range.clone();
            builder.push(StyleProperty::FontFamily(FontFamily::Single(FontFamilyName::Named(Cow::Owned(p.key.clone())))), r.clone());
            builder.push(StyleProperty::FontWeight(p.weight), r.clone());
            builder.push(StyleProperty::FontStyle(p.style), r.clone());
            builder.push(StyleProperty::FontWidth(p.width), r);
        }
        builder.build(spec.text)
    }

    /// Lay out one paragraph at the origin.
    fn build(&mut self, spec: ParaSpec<'_>) -> (Vec<LaidLine>, f64, f64, f64) {
        let lh = spec.line_height as f32;
        let strut = spec.strut.map(|(m, size)| strut_metrics(&m, size as f32, lh));
        if spec.text.is_empty() {
            // ParagraphImpl::computeEmptyMetrics.
            let base_face = self.loaded_face(self.fonts.resolve_family(&spec.base.family), spec.base.weight, spec.base.italic);
            let m = base_face.map(|f| f.info.metrics).unwrap_or_default();
            let size = spec.base.size as f32;
            let fm = sk_metrics(&m, size);
            let intrinsic = sk_round(f64::from(fm.descent) - f64::from(fm.ascent) + f64::from(fm.leading)) as f32;
            let strut_height = lh * size;
            let leading = fm.leading + strut_height - intrinsic;
            let (mut asc, mut desc) = (fm.ascent, fm.descent);
            if let Some((sa, sd)) = strut {
                asc = asc.min(sa);
                desc = desc.max(sd);
            }
            let h = sk_round(f64::from(desc) - f64::from(asc) + f64::from(leading));
            return (Vec::new(), h, f64::from(f32::MIN), 0.0);
        }

        // Bind every grapheme to one face first (immutable borrow of the registry).
        let pieces = self.pieces(&spec);

        let max_advance = spec.width.max(1.0).floor() as f32;
        let mut layout = self.shape(&spec, &pieces);
        let text = spec.text;

        // Letter spacing as SkParagraph applies it: after every glyph (`Run::addSpacesEvenly`), so a cluster
        // of n glyphs (a Thai consonant with its marks, an unresolved emoji sequence) gets n spacings.
        let spacing: Vec<f32> = spec.spans.iter().map(|sp| tracking_px(&sp.style)).collect();
        let spacing_of = |brush: Brush| spacing.get(brush as usize).copied().unwrap_or(0.0);

        // 1. Measure: one unbroken line, clusters in logical order.
        layout.break_all_lines(None);
        let soft = break_opportunities(text);
        // SkParagraph shapes consecutive style blocks with the same font attributes as one block
        // (`matchOneAttribute(kFont)`): spans get the index of the first span of their font group.
        let font_group: Vec<usize> = (0..spec.spans.len())
            .map(|i| {
                let a = &spec.spans[i].style;
                let same = |b: &RunStyle| b.family == a.family && b.weight == a.weight && b.italic == a.italic && b.size == a.size;
                let mut j = i;
                while j > 0 && same(&spec.spans[j - 1].style) {
                    j -= 1;
                }
                j
            })
            .collect();
        let mut logical: Vec<(std::ops::Range<usize>, WrapCluster)> = Vec::new();
        for line in layout.lines() {
            for run in line.runs() {
                let first_new = logical.len();
                // Per new cluster: (font group, unresolved).
                let mut keys: Vec<(usize, bool)> = Vec::new();
                for c in run.clusters() {
                    let tr = c.text_range();
                    let s = &text[tr.clone()];
                    let ws = !s.is_empty() && s.chars().all(|ch| ch.is_whitespace() && !is_nbsp(ch));
                    let glyphs = c.glyphs().count();
                    let wc = WrapCluster {
                        width: c.advance() + glyphs as f32 * spacing_of(c.style().brush),
                        ws_break: ws && !s.contains('\n'),
                        intra_ws: !s.is_empty() && s.chars().all(is_nbsp),
                        soft_break: soft.binary_search(&tr.end).is_ok(),
                        hard_break: s.contains('\n') || s.contains('\u{2028}'),
                        glyphs,
                        start_pos: 0,
                        end_pos: 0,
                    };
                    // A ligature is one cluster for the wrapper (as for HarfBuzz/SkParagraph); parley splits it into
                    // graphemes and composes lines atom by atom.
                    if c.is_ligature_continuation()
                        && let Some((prev_r, prev)) = logical.last_mut()
                        && prev_r.end == tr.start
                    {
                        prev_r.end = tr.end;
                        prev.width += wc.width;
                        prev.soft_break = wc.soft_break;
                        prev.hard_break |= wc.hard_break;
                        prev.glyphs += wc.glyphs;
                        continue;
                    }
                    // A grapheme with a `.notdef` glyph is unresolved: no face of the stack covers it.
                    let unresolved = c.glyphs().any(|g| g.id == 0) && !s.chars().all(char::is_control);
                    keys.push((font_group.get(c.style().brush as usize).copied().unwrap_or(0), unresolved));
                    logical.push((tr, wc));
                }
                // Glyph positions inside SkParagraph's runs, in visual order. SkParagraph's runs are its shaping
                // runs split at font-group changes, and at every change between resolved and unresolved graphemes
                // (`OneLineShaper::finish` copies each resolved/unresolved block into a run of its own).
                let rtl = run.is_rtl();
                let mut k = first_new;
                while k < logical.len() {
                    let key = keys[k - first_new];
                    let mut e = k;
                    while e < logical.len() && keys[e - first_new] == key {
                        e += 1;
                    }
                    let total: usize = logical[k..e].iter().map(|(_, c)| c.glyphs).sum();
                    let mut cum = 0usize;
                    for (_, c) in &mut logical[k..e] {
                        let (a, b) = if rtl { (total - cum - c.glyphs, total - cum) } else { (cum, cum + c.glyphs) };
                        c.start_pos = a;
                        c.end_pos = b;
                        cum += c.glyphs;
                    }
                    k = e;
                }
            }
        }
        logical.sort_by_key(|(r, _)| r.start);
        if let Some((_, last)) = logical.last_mut() {
            last.soft_break = true;
        }

        // 2. Break like SkParagraph, then let parley compose (bidi-reorder) exactly those lines.
        let wrap: Vec<WrapCluster> = logical.iter().map(|(_, c)| c.clone()).collect();
        let ranges = break_lines(&wrap, max_advance);
        let counts: Vec<u32> = ranges
            .iter()
            .map(|r| {
                let (a, b) = (logical[r.start].0.start, logical[r.end - 1].0.end);
                text[a..b].chars().count() as u32
            })
            .collect();
        let mut breaker = layout.break_lines();
        for n in &counts {
            breaker.break_next_with_length(*n);
        }
        breaker.finish();

        // 3. Lines: vertical metrics, alignment/justification, clusters and glyphs.
        let ideo = CodePointSetData::new::<Ideographic>();
        let u16map = Utf16Map::new(text);
        let mut lines = Vec::new();
        let mut top = 0.0f64;
        let mut longest = f64::from(f32::MIN);
        let line_count = layout.len();
        for (li, line) in layout.lines().enumerate() {
            let range = line.text_range();
            let trimmed_end = range.start + text[range.clone()].trim_end().len();
            // Vertical metrics: the runs of the visible text (all of it on a whitespace-only line), then the strut
            // (`fSizes`). `LineMetrics.ascent/descent` are the maxima over every run on the line, without the strut
            // (`fMaxRunMetrics`).
            let mut asc = 0.0f32;
            let mut desc = 0.0f32;
            let mut any = false;
            let (mut run_asc, mut run_desc) = (0.0f32, 0.0f32);
            for (ri, run) in line.runs().enumerate() {
                let tr = run.text_range();
                let m = self.fonts.face_by_blob(run.font().data.id()).map(|f| f.info.metrics).unwrap_or_default();
                let size = run.clusters().next().map(|c| c.style().brush).and_then(|b| spec.spans.get(b as usize)).map(|sp| sp.style.size as f32).unwrap_or(run.font_size());
                let (a, d) = run_line_metrics(&m, size, lh);
                (run_asc, run_desc) = if ri == 0 { (a, d) } else { (run_asc.min(a), run_desc.max(d)) };
                if !(tr.start < trimmed_end || trimmed_end == range.start) {
                    continue;
                }
                if any {
                    asc = asc.min(a);
                    desc = desc.max(d);
                } else {
                    (asc, desc) = (a, d);
                    any = true;
                }
            }
            if !any {
                (run_asc, run_desc) = strut.unwrap_or((0.0, 0.0));
            }
            if let Some((sa, sd)) = strut {
                if any {
                    asc = asc.min(sa);
                    desc = desc.max(sd);
                } else {
                    (asc, desc) = (sa, sd);
                }
            }
            let height = sk_round(f64::from(desc) - f64::from(asc));
            // `LineMetrics.baseline` is Flutter's (`top + height − descent`, so the rounding of the line height goes
            // above the baseline); glyphs are painted on `top + round(ascent)` (`TextLine::buildTextBlob`).
            let baseline = top + height - f64::from(desc);
            let paint_baseline = top + (f64::from(-asc) + 0.5).floor();

            // Visual clusters with their natural advances.
            struct VC<'a> {
                c: parley::Cluster<'a, Brush>,
                adv: f32,
                ghost: bool,
                ws: bool,
                ideo: bool,
            }
            let mut vcs: Vec<VC<'_>> = Vec::new();
            for run in line.runs() {
                for c in run.visual_clusters() {
                    let tr = c.text_range();
                    let s = &text[tr.clone()];
                    vcs.push(VC {
                        adv: c.advance() + c.glyphs().count() as f32 * spacing_of(c.style().brush),
                        ghost: tr.start >= trimmed_end,
                        ws: !s.is_empty() && s.chars().all(|ch| ch.is_whitespace() && !is_nbsp(ch)),
                        ideo: s.chars().next().is_some_and(|ch| ideo.contains(ch)),
                        c,
                    });
                }
            }
            // Ghost (trailing) whitespace takes no room in the line, wherever bidi puts it: its advance only counts
            // when it hangs off the visual end of the line (caret positions after the last character).
            let first_visible = vcs.iter().position(|v| !v.ghost);
            let last_visible = vcs.iter().rposition(|v| !v.ghost);
            let collapsed = |k: usize| vcs[k].ghost && first_visible.is_some_and(|f| f < k) && last_visible.is_some_and(|l| k < l);
            let visible_w: f32 = vcs.iter().filter(|v| !v.ghost).map(|v| v.adv).sum();
            let delta = max_advance - visible_w;
            let last_line = li + 1 == line_count;
            let mut shifts = vec![0.0f32; vcs.len()];
            // SkParagraph starts a line half a letter spacing in (the shift of its first cluster); centring and end
            // alignment replace that shift, they do not add to it (`TextLine::format`).
            let first = vcs.iter().filter(|v| !v.ghost).min_by_key(|v| v.c.text_range().start);
            let mut offset = first.map(|v| spacing_of(v.c.style().brush) / 2.0).unwrap_or(0.0);
            let mut justified = false;
            if delta > 0.0 {
                match spec.align {
                    TextAlign::Center => offset = delta / 2.0,
                    TextAlign::End => offset = delta,
                    TextAlign::Justify if !last_line => {
                        let widths: Vec<f32> = vcs.iter().map(|v| v.adv).collect();
                        let ws: Vec<bool> = vcs.iter().map(|v| v.ws).collect();
                        let id: Vec<bool> = vcs.iter().map(|v| v.ideo).collect();
                        let gh: Vec<bool> = vcs.iter().map(|v| v.ghost).collect();
                        if let Some(s) = justify_shifts(&widths, &ws, &id, &gh, max_advance) {
                            shifts = s;
                            justified = true;
                        }
                    }
                    _ => {}
                }
            }
            // `LineMetrics.left` is the line's shift.
            let left = f64::from(offset);

            // Trailing whitespace that bidi puts on the visual left must not push the visible text right.
            let lead_ghost: f32 = vcs.iter().take_while(|v| v.ghost).map(|v| v.adv).sum();
            if first_visible.is_some() {
                offset -= lead_ghost;
            }
            let mut clusters = Vec::with_capacity(vcs.len());
            let mut glyph_runs: Vec<GlyphRun> = Vec::new();
            let mut run_key: Option<(u64, u32, bool)> = None;
            let mut pen = 0.0f32;
            for (k, v) in vcs.iter().enumerate() {
                let x = pen + offset + shifts[k];
                let tr = v.c.text_range();
                // Justification widens the gap before the next cluster: the shift difference is part of this one.
                let next_shift = shifts.get(k + 1).copied().unwrap_or(shifts[k]);
                let adv = if collapsed(k) {
                    0.0
                } else {
                    v.adv + if v.ghost { 0.0 } else { next_shift - shifts[k] }
                };
                clusters.push(ClusterBox {
                    start: u16map.at(tr.start),
                    end: u16map.at(tr.end),
                    x: f64::from(x),
                    advance: f64::from(adv),
                    rtl: v.c.is_rtl(),
                    whitespace: v.ws,
                });
                // Trailing whitespace is not painted (SkParagraph paints lines without their ghost spaces).
                if v.ghost {
                    if !collapsed(k) {
                        pen += v.adv;
                    }
                    continue;
                }
                let run = v.c.run();
                let brush = v.c.style().brush;
                let key = (run.font().data.id(), brush, run.is_rtl());
                if run_key != Some(key) {
                    run_key = Some(key);
                    let face = self.fonts.face_by_blob(key.0).map(|f| f.file.clone()).unwrap_or_default();
                    let span = spec.spans.get(brush as usize);
                    glyph_runs.push(GlyphRun {
                        synthetic_italic: span.is_some_and(|s| s.style.italic) && !self.face_is_italic(&face),
                        face,
                        size: span.map(|s| s.style.size).unwrap_or(f64::from(run.font_size())),
                        style: brush as usize,
                        rtl: run.is_rtl(),
                        x: f64::from(x),
                        advance: 0.0,
                        glyphs: Vec::new(),
                    });
                }
                let gr = glyph_runs.last_mut().expect("glyph run");
                // SkParagraph spaces every glyph: the spacing follows the glyph (`Run::addSpacesEvenly`).
                let ls = spacing_of(brush);
                // Justification (`TextLine::justify`) moves a gap's whitespace glyphs with the word after the gap,
                // while its cluster box keeps its place and widens.
                let mut gx = if v.ws { pen + offset + next_shift } else { x };
                let shift = spec.spans.get(brush as usize).map_or(0.0, |s| s.dy);
                for g in v.c.glyphs() {
                    gr.glyphs.push(PositionedGlyph { id: g.id, x: f64::from(gx + g.x), y: paint_baseline + shift + f64::from(g.y), advance: f64::from(g.advance + ls) });
                    gx += g.advance + ls;
                }
                gr.advance += f64::from(adv);
                pen += v.adv;
            }
            // `LineMetrics.width` (`fAdvance.fX`): the visible advance, the full width once justified.
            let width = little_round(if justified { f64::from(max_advance) } else { f64::from(visible_w) });
            // `getLongestLine`: the widest line before justification (a whitespace-only line counts its spaces).
            let natural = if visible_w.abs() < 1e-4 { vcs.iter().map(|v| v.adv).sum::<f32>() } else { visible_w };
            longest = longest.max(f64::from(natural));
            lines.push(LaidLine {
                start: u16map.at(range.start),
                end: u16map.at(range.end),
                end_no_ws: u16map.at(trimmed_end),
                hard_break: last_line,
                top,
                height,
                baseline,
                ascent: f64::from(-run_asc),
                descent: f64::from(run_desc),
                left,
                width,
                glyph_runs,
                clusters,
            });
            top += height;
        }
        let intrinsic = little_round(f64::from(wrap.iter().map(|c| c.width).sum::<f32>()));
        (lines, top, longest, intrinsic)
    }

    fn paragraph(&mut self, body: &TextBody, idx: usize, width: f64) -> LaidParagraph {
        let p = &body.paragraphs[idx];
        let model = paragraph_model_text(p);
        let fallback = fallback_families(&model);
        let base = &body.default_run;
        let styles: Vec<RunStyle> = p.runs.iter().map(|r| resolve_run_style(body, r)).collect();
        let mut text = String::new();
        let mut spans = Vec::new();
        for (i, (r, s)) in p.runs.iter().zip(&styles).enumerate() {
            let start = text.len();
            text.push_str(&apply_case(&r.text, s.case_transform.as_ref()));
            let mut style = s.clone();
            let dy = oc_schema::text::baseline_offset(s.baseline.as_ref(), s.size);
            if s.baseline.is_some() && dy != 0.0 {
                style.size = s.size * oc_schema::text::BASELINE_SCALE;
            }
            spans.push(Span { range: start..text.len(), style, brush: i as Brush, dy });
        }
        let strut_size = if styles.is_empty() { base.size } else { styles.iter().map(|s| s.size).fold(f64::NEG_INFINITY, f64::max) };
        let strut_face = self.loaded_face(self.fonts.resolve_family(&base.family), 400.0, false).map(|f| f.info.metrics);
        let (lines, height, longest, intrinsic) = self.build(ParaSpec {
            text: &text,
            spans,
            align: p.align.clone(),
            line_height: p.line_height,
            width,
            strut: strut_face.map(|m| (m, strut_size)),
            base,
            fallback: &fallback,
        });
        LaidParagraph {
            x: 0.0,
            y: 0.0,
            len: utf16_len(&model),
            start: 0,
            height,
            align: p.align.clone(),
            width,
            text,
            styles,
            lines,
            longest_line: longest,
            max_intrinsic_width: intrinsic,
            bullet: None,
        }
    }

    fn bullet(&mut self, body: &TextBody, idx: usize, n: usize) -> LaidParagraph {
        let p = &body.paragraphs[idx];
        let s = p.runs.first().map(|r| resolve_run_style(body, r)).unwrap_or_else(|| body.default_run.clone());
        let text = if p.list.as_ref().is_some_and(|l| l.kind == ListKind::Number) { format!("{n}.") } else { "•".to_owned() };
        let width = (s.size * 1.1).floor();
        let (lines, height, longest, intrinsic) = self.build(ParaSpec {
            text: &text,
            spans: vec![Span { range: 0..text.len(), style: s.clone(), brush: 0, dy: 0.0 }],
            align: TextAlign::End,
            line_height: p.line_height,
            width,
            strut: None,
            base: &s,
            fallback: &[],
        });
        LaidParagraph {
            x: 0.0,
            y: 0.0,
            len: utf16_len(&text),
            start: 0,
            height,
            align: TextAlign::End,
            width,
            text,
            styles: vec![s],
            lines,
            longest_line: longest,
            max_intrinsic_width: intrinsic,
            bullet: None,
        }
    }

    /// Lay out a body at a box width (uncached).
    pub fn layout_uncached(&mut self, body: &TextBody, width: f64, effect: Option<&TextEffect>) -> TextLayout {
        let mut paras = Vec::with_capacity(body.paragraphs.len());
        let mut y = 0.0;
        let mut start = 0usize;
        let mut list_n = 0usize;
        for (i, p) in body.paragraphs.iter().enumerate() {
            let indent = p.list.as_ref().map(|l| body.default_run.size * 1.4 * (l.level + 1.0)).unwrap_or(0.0);
            list_n = if p.list.as_ref().is_some_and(|l| l.kind == ListKind::Number) { list_n + 1 } else { 0 };
            let w = (width - indent).max(1.0);
            let mut para = self.paragraph(body, i, w);
            para.x = indent;
            para.y = y;
            para.start = start;
            if p.list.is_some() {
                let mut b = self.bullet(body, i, list_n);
                b.x = indent - b.width - 6.0;
                b.y = y;
                para.bullet = Some(Box::new(b));
            }
            y += para.height + p.space_after;
            start += para.len + 1;
            paras.push(para);
        }
        TextLayout { width, height: y.max(1.0), paras, length: start.saturating_sub(1), effect: effect.cloned() }
    }

    /// Lay out a body at a given box width. Cached per key (`layout`).
    pub fn layout(&mut self, key: &str, body: &TextBody, width: f64, effect: Option<&TextEffect>) -> Arc<TextLayout> {
        let generation = self.fonts.generation();
        if let Some(hit) = self.cache.get(key)
            && hit.width == width
            && hit.generation == generation
            && hit.effect.as_ref() == effect
            && hit.body == *body
        {
            return hit.layout.clone();
        }
        self.note_missing(body);
        let layout = Arc::new(self.layout_uncached(body, width, effect));
        self.cache.insert(key.to_owned(), CacheEntry { body: body.clone(), width, generation, effect: effect.cloned(), layout: layout.clone() });
        layout
    }

    pub fn dispose(&mut self, key: &str) {
        self.cache.remove(key);
        self.fit_cache.remove(key);
        self.curved_cache.remove(key);
    }

    pub fn dispose_all(&mut self) {
        self.cache.clear();
        self.fit_cache.clear();
        self.curved_cache.clear();
    }

    /// Shrink-to-fit (ED-044): the largest scale ≤ 1 at which the body fits `height` (9-step bisection from
    /// 0.05, like the TS), laid out. Returns the plain layout and scale 1 when it already fits.
    pub fn layout_fit(&mut self, key: &str, body: &TextBody, width: f64, height: f64, effect: Option<&TextEffect>) -> (Arc<TextLayout>, f64) {
        let full = self.layout(key, body, width, effect);
        if full.height <= height + 0.5 {
            return (full, 1.0);
        }
        let generation = self.fonts.generation();
        let cached = self.fit_cache.get(key).filter(|h| h.width == width && h.height == height && h.generation == generation && h.body == *body).map(|h| h.scale);
        let scale = match cached {
            Some(s) => s,
            None => {
                let (mut lo, mut hi) = (0.05f64, 1.0f64);
                for _ in 0..9 {
                    let mid = (lo + hi) / 2.0;
                    let l = self.layout_uncached(&scale_body(body, mid), width, effect);
                    if l.height <= height {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                self.fit_cache.insert(key.to_owned(), FitEntry { body: body.clone(), width, height, generation, scale: lo });
                lo
            }
        };
        (self.layout(&format!("{key}:fit"), &scale_body(body, scale), width, effect), scale)
    }

    /// Lay out a body as one line on an arc (`curved`). `curve` in −100..100 (positive arches upward).
    /// `None` when there is nothing to draw or no face is loaded.
    pub fn curved(&mut self, key: &str, body: &TextBody, curve: f64) -> Option<Arc<CurvedLayout>> {
        let generation = self.fonts.generation();
        if let Some(hit) = self.curved_cache.get(key)
            && hit.curve == curve
            && hit.generation == generation
            && hit.body == *body
        {
            return hit.layout.clone();
        }
        self.note_missing(body);
        let layout = self.curved_uncached(body, curve).map(Arc::new);
        self.curved_cache.insert(key.to_owned(), CurvedEntry { body: body.clone(), curve, generation, layout: layout.clone() });
        layout
    }

    fn curved_uncached(&mut self, body: &TextBody, curve: f64) -> Option<CurvedLayout> {
        crate::curved::curved(self, body, curve)
    }

    /// Hinted (whole-pixel) advance of a glyph, like `Font.getGlyphWidths` on a default SkFont.
    pub(crate) fn face_advance(&self, face: &str, glyph: u32, size: f64) -> f64 {
        use skrifa::MetadataProvider;
        let Some(f) = self.fonts.face_by_file(face) else { return 0.0 };
        let Ok(font) = skrifa::FontRef::new(f.data()) else { return 0.0 };
        let gm = font.glyph_metrics(skrifa::instance::Size::new(size as f32), skrifa::instance::LocationRef::default());
        f64::from(gm.advance_width(skrifa::GlyphId::new(glyph)).unwrap_or(0.0)).round()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use icu_properties::props::LineBreak as Lb;

    fn pieces(t: &str) -> Vec<&str> {
        let b = break_opportunities(t);
        let mut out = Vec::new();
        let mut prev = 0;
        for &x in b.iter().filter(|&&x| x > 0) {
            out.push(&t[prev..x]);
            prev = x;
        }
        out
    }

    #[test]
    fn sa_stand_ins_resolve_like_lb1() {
        let lb = CodePointMapData::<Lb>::new();
        for (i, c) in SA_AS_AL.iter().enumerate() {
            assert_eq!((c.len_utf8(), lb.get(*c)), (i + 1, Lb::Alphabetic), "{c:?}");
        }
        for (i, c) in SA_AS_CM.iter().enumerate() {
            assert_eq!((c.len_utf8(), lb.get(*c)), (i + 1, Lb::CombiningMark), "{c:?}");
        }
    }

    #[test]
    fn complex_run_ends_follow_the_rules() {
        // Dictionary breaks inside the run are kept.
        assert_eq!(pieces("ภาษาไทย"), ["ภาษา", "ไทย"]);
        // No break before trailing punctuation, digits, letters or closing brackets (SA acts as AL).
        assert_eq!(pieces("ภาษาไทย."), ["ภาษา", "ไทย."]);
        assert_eq!(pieces("ภาษา2026"), ["ภาษา2026"]);
        assert_eq!(pieces("ภาษาabc"), ["ภาษาabc"]);
        assert_eq!(pieces("(ภาษา)"), ["(ภาษา)"]);
        assert_eq!(pieces("ภาษา\""), ["ภาษา\""]);
        // A space breaks after itself only.
        assert_eq!(pieces("ภาษา ไทย"), ["ภาษา ", "ไทย"]);
        // Ordinary rules elsewhere.
        assert_eq!(pieces("ภาษา-abc"), ["ภาษา-", "abc"]);
        assert_eq!(pieces("hello world"), ["hello ", "world"]);
        assert_eq!(pieces("100%(note"), ["100%", "(note"]);
    }
}
