// SPDX-License-Identifier: Apache-2.0
//! Design + pages. docs/plan/03-document-model-and-renderer.md §2.2.
//!
//! Records keyed by id (`pages`, `elements`, `assets`, table `cells`) are `IndexMap`s: they keep the JSON
//! insertion order, which is what JS objects do for non-integer keys, so a parse → serialise round trip
//! reproduces the input order.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::elements::{AnimDirection, AnimPreset, Element};
use crate::macros::string_enum;
use crate::primitives::{AssetId, ElementId, Extra, PageId, Paint, Unit};

string_enum! {
    pub enum Surface {
        Graphic = "graphic", Presentation = "presentation", Video = "video", Doc = "doc", Sheet = "sheet",
        Whiteboard = "whiteboard", Print = "print", Website = "website", Email = "email",
    }
}

pub const CURRENT_SCHEMA: i64 = 1;
pub const LAYOUT_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dims {
    pub w: f64,
    pub h: f64,
    pub unit: Unit,
    pub dpi: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateOrigin {
    pub template_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignMeta {
    pub schema_version: i64,
    pub min_client_schema: i64,
    pub layout_version: i64,
    pub surface: Surface,
    /// Page size in document px at 96 dpi; `unit` is display-only.
    pub dims: Dims,
    pub lang: String,
    pub title: String,
    pub epoch: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_origin: Option<TemplateOrigin>,
    /// Design soundtrack (atomic list, LWW).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<Vec<AudioClip>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captions: Option<CaptionTrack>,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum GuideAxis { X = "x", Y = "y" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub axis: GuideAxis,
    pub pos: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    /// Page transitions (MV-010).
    pub enum TransitionKind {
        Fade = "fade", Dissolve = "dissolve", Slide = "slide", Push = "push", Cover = "cover", Reveal = "reveal",
        Zoom = "zoom", Wipe = "wipe", Iris = "iris", Split = "split", Flip = "flip", Blur = "blur", Spin = "spin",
        Drop = "drop",
    }
}
pub const TRANSITION_KINDS: &[&str] = &[
    "fade", "dissolve", "slide", "push", "cover", "reveal", "zoom", "wipe", "iris", "split", "flip", "blur", "spin",
    "drop",
];

fn lenient_camera<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<PageCamera>, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok()))
}

/// A page camera (videos). Tracks hold page-time keys like element keyframes: `zoom` (1 = the page as it is),
/// `x` and `y` (where it looks, px from the page centre) and `rotate` (degrees). Zoom never goes below what keeps
/// the page edges out of view.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PageCamera {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<Vec<crate::elements::Keyframe>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<Vec<crate::elements::Keyframe>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<Vec<crate::elements::Keyframe>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<Vec<crate::elements::Keyframe>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shake: Option<Vec<CameraShake>>,
    /// Top-level elements the camera leaves where they are (a logo, a caption bar).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A burst of camera shake from `start_ms` to `end_ms` (page time): up to `amount` px of travel, `hz` new
/// positions a second, `seed` picking the pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraShake {
    pub start_ms: f64,
    pub end_ms: f64,
    pub amount: f64,
    pub hz: f64,
    #[serde(default)]
    pub seed: u32,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTransition {
    pub kind: TransitionKind,
    pub duration_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<AnimDirection>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageAnim {
    pub preset: AnimPreset,
    pub duration_ms: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum MobileLayout { Auto = "auto", Desktop = "desktop" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub id: PageId,
    /// Fractional index; page order = sort by (order, id).
    pub order: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// Locked page (g96): its elements can't be selected, moved or edited until the page is unlocked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    pub background: Vec<Paint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Page length in video/GIF export and auto-advancing presentations (default 5000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    /// Custom ruler guides (ED-061), page px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guides: Option<Vec<Guide>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<PageTransition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim: Option<PageAnim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile_layout: Option<MobileLayout>,
    /// A camera over the page in videos: zoom, pan, turn and shake over page time. Read leniently: a camera that
    /// does not read leaves the page as it is.
    #[serde(default, deserialize_with = "lenient_camera", skip_serializing_if = "Option::is_none")]
    pub camera: Option<PageCamera>,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum AssetKind { Image = "image", Svg = "svg", Video = "video", Audio = "audio" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attribution {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetRef {
    pub id: AssetId,
    pub kind: AssetKind,
    pub sha256: String,
    pub mime: String,
    pub w: f64,
    pub h: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<Attribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_audio: Option<bool>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One clip on the design-level soundtrack (MV-030). Times in design time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioClip {
    pub id: String,
    pub asset: AssetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub start_ms: f64,
    pub trim_start_ms: f64,
    pub duration_ms: f64,
    pub volume: f64,
    pub fade_in_ms: f64,
    pub fade_out_ms: f64,
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub loop_: Option<bool>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionCue {
    pub id: String,
    pub start_ms: f64,
    pub end_ms: f64,
    pub text: String,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum CaptionPosition { Bottom = "bottom", Top = "top", Middle = "middle" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionStyle {
    pub family: String,
    pub size: f64,
    pub color: String,
    pub background: String,
    pub position: CaptionPosition,
    /// Colour of the word being spoken: the cue's words light up one after another across its time. Unset: the
    /// whole cue shows in `color`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionTrack {
    pub cues: Vec<CaptionCue>,
    pub style: CaptionStyle,
    pub visible: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Plain-JSON snapshot of a design (templates, fixtures, exports and the API).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignDoc {
    pub meta: DesignMeta,
    pub pages: IndexMap<PageId, Page>,
    /// Missing in the JSON = empty (the TS validator treats it the same way).
    #[serde(default)]
    pub elements: IndexMap<ElementId, Element>,
    #[serde(default)]
    pub assets: IndexMap<AssetId, AssetRef>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl DesignDoc {
    /// Parses a design from JSON (no validation; see `validate::validate_doc`).
    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        serde_json::from_str(s)
    }

    /// Serialises like `JSON.stringify` (integral numbers without `.0`).
    pub fn to_json(&self) -> serde_json::Result<String> {
        crate::js::to_string(self)
    }
}
