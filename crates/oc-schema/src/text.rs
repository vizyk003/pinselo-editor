// SPDX-License-Identifier: Apache-2.0
//! Text model, snapshot form. In the CRDT the body lives in a Y.XmlFragment (paragraph elements holding
//! Y.XmlText with marks). docs/plan/03-document-model-and-renderer.md §2.4.

use serde::{Deserialize, Serialize};

use crate::macros::string_enum;
use crate::primitives::{ColorValue, Extra, LinkTarget, Paint};

string_enum! {
    pub enum CaseTransform { None = "none", Upper = "upper", Lower = "lower", Title = "title" }
}
string_enum! {
    /// Superscript / subscript: the run is drawn smaller, raised or lowered (`BASELINE_SCALE`, `baseline_offset`).
    pub enum BaselineShift { Super = "super", Sub = "sub" }
}

/// Size of a superscript or subscript run relative to its font size.
pub const BASELINE_SCALE: f64 = 0.62;

/// Vertical offset (px, down positive) of a shifted run of nominal font size `size`.
pub fn baseline_offset(shift: Option<&BaselineShift>, size: f64) -> f64 {
    match shift {
        Some(BaselineShift::Super) => -size * 0.36,
        Some(BaselineShift::Sub) => size * 0.16,
        _ => 0.0,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStyle {
    pub family: String,
    pub weight: f64,
    pub italic: bool,
    /// px
    pub size: f64,
    pub fill: Paint,
    /// em/1000
    pub tracking: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_transform: Option<CaseTransform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<ColorValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<BaselineShift>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `Partial<RunStyle>`: a run's overrides of the body's default run.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStyleOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_transform: Option<CaseTransform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<ColorValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<BaselineShift>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    pub style: RunStyleOverride,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkTarget>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Run {
    pub fn plain(text: impl Into<String>) -> Self {
        Run { text: text.into(), style: RunStyleOverride::default(), link: None, extra: Extra::new() }
    }
}

string_enum! {
    pub enum TextAlign { Start = "start", Center = "center", End = "end", Justify = "justify" }
}
string_enum! {
    pub enum ListKind { Bullet = "bullet", Number = "number" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListSpec {
    pub kind: ListKind,
    pub level: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paragraph {
    pub runs: Vec<Run>,
    pub align: TextAlign,
    /// multiple of font size
    pub line_height: f64,
    pub space_after: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<ListSpec>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextBody {
    pub paragraphs: Vec<Paragraph>,
    pub default_run: RunStyle,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowTextEffect {
    pub dx: f64,
    pub dy: f64,
    pub blur: f64,
    pub color: ColorValue,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HollowTextEffect {
    pub width: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineTextEffect {
    pub width: f64,
    pub color: ColorValue,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundTextEffect {
    pub color: ColorValue,
    pub padding: f64,
    pub radius: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeonTextEffect {
    pub intensity: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EchoTextEffect {
    pub dx: f64,
    pub dy: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlitchTextEffect {
    pub offset: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k")]
pub enum TextEffect {
    #[serde(rename = "shadow")]
    Shadow(ShadowTextEffect),
    #[serde(rename = "hollow")]
    Hollow(HollowTextEffect),
    #[serde(rename = "outline")]
    Outline(OutlineTextEffect),
    #[serde(rename = "background")]
    Background(BackgroundTextEffect),
    #[serde(rename = "neon")]
    Neon(NeonTextEffect),
    /// Three fading copies behind the text along (dx, dy).
    #[serde(rename = "echo")]
    Echo(EchoTextEffect),
    /// Chromatic split: cyan and magenta copies offset sideways (% of font size).
    #[serde(rename = "glitch")]
    Glitch(GlitchTextEffect),
}

/// Paragraph attributes that live on the paragraph node (not on runs): `Omit<Paragraph, 'runs'>`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphAttrs {
    pub align: TextAlign,
    pub line_height: f64,
    pub space_after: f64,
    pub list: Option<ListSpec>,
}

pub fn default_paragraph() -> ParagraphAttrs {
    ParagraphAttrs { align: TextAlign::Start, line_height: 1.3, space_after: 0.0, list: None }
}

impl ParagraphAttrs {
    /// `{ ...attrs, runs }`.
    pub fn with_runs(&self, runs: Vec<Run>) -> Paragraph {
        Paragraph {
            runs,
            align: self.align.clone(),
            line_height: self.line_height,
            space_after: self.space_after,
            list: self.list.clone(),
            extra: Extra::new(),
        }
    }
}

pub fn body_plain_text(body: &TextBody) -> String {
    body.paragraphs
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `{ ...body.defaultRun, ...run.style }`.
pub fn resolve_run_style(body: &TextBody, run: &Run) -> RunStyle {
    apply_override(&body.default_run, &run.style)
}

/// `{ ...base, ...over }` for run styles.
pub fn apply_override(base: &RunStyle, over: &RunStyleOverride) -> RunStyle {
    let mut out = base.clone();
    if let Some(v) = &over.family {
        out.family = v.clone();
    }
    if let Some(v) = over.weight {
        out.weight = v;
    }
    if let Some(v) = over.italic {
        out.italic = v;
    }
    if let Some(v) = over.size {
        out.size = v;
    }
    if let Some(v) = &over.fill {
        out.fill = v.clone();
    }
    if let Some(v) = over.tracking {
        out.tracking = v;
    }
    if over.underline.is_some() {
        out.underline = over.underline;
    }
    if over.strike.is_some() {
        out.strike = over.strike;
    }
    if over.case_transform.is_some() {
        out.case_transform = over.case_transform.clone();
    }
    if over.highlight.is_some() {
        out.highlight = over.highlight.clone();
    }
    if over.baseline.is_some() {
        out.baseline = over.baseline.clone();
    }
    for (k, v) in &over.extra {
        out.extra.insert(k.clone(), v.clone());
    }
    out
}
