// SPDX-License-Identifier: Apache-2.0
//! Core value types shared by every element. See docs/plan/03-document-model-and-renderer.md §2.1.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::macros::string_enum;

/// Unknown fields of a loose (passthrough) object, preserved verbatim in insertion order.
pub type Extra = Map<String, Value>;

/// Ids are plain strings on the wire (16-char base62 for ids this build generates).
pub type DesignId = String;
pub type PageId = String;
pub type ElementId = String;
pub type AssetId = String;

pub type Vec2 = [f64; 2];

/// Affine `[a,b,c,d,e,f]`: x' = a x + c y + e; y' = b x + d y + f.
pub type Mat2D = [f64; 6];

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const UNIT: Rect = Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }
}

/// A rect that keeps unknown fields (crops and grid cells are stored rects).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StoredRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

impl From<Rect> for StoredRect {
    fn from(r: Rect) -> Self {
        StoredRect { x: r.x, y: r.y, w: r.w, h: r.h, extra: Extra::new() }
    }
}

impl From<&StoredRect> for Rect {
    fn from(r: &StoredRect) -> Self {
        Rect { x: r.x, y: r.y, w: r.w, h: r.h }
    }
}

/// Canonical element transform. Written atomically to the CRDT (one value).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Xf {
    /// Top-left of the unrotated box, in parent (page) space, document px at 96 dpi.
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Degrees, normalized to (-180, 180]. Rotation is about the box centre.
    pub rot: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_x: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_y: Option<bool>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Xf {
    pub fn new(x: f64, y: f64, w: f64, h: f64, rot: f64) -> Self {
        Xf { x, y, w, h, rot, ..Default::default() }
    }
}

string_enum! {
    pub enum Unit { Px = "px", Mm = "mm", Cm = "cm", In = "in", Pt = "pt" }
}

/// sRGB / Display-P3 channels (0..1).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CmykColor {
    pub c: f64,
    pub m: f64,
    pub y: f64,
    pub k: f64,
    pub a: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SpotColor {
    pub swatch: String,
    pub tint: f64,
    pub a: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Authored colour. Only sRGB is produced by the MVP editor; other spaces are preserved verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "space")]
pub enum ColorValue {
    #[serde(rename = "srgb")]
    Srgb(RgbColor),
    #[serde(rename = "display-p3")]
    DisplayP3(RgbColor),
    #[serde(rename = "cmyk")]
    Cmyk(CmykColor),
    #[serde(rename = "spot")]
    Spot(SpotColor),
}

impl ColorValue {
    /// An sRGB colour (channels taken as given; see `color::rgb` for the clamping constructor).
    pub fn srgb(r: f64, g: f64, b: f64, a: f64) -> Self {
        ColorValue::Srgb(RgbColor { r, g, b, a, extra: Extra::new() })
    }
    pub fn space(&self) -> &'static str {
        match self {
            ColorValue::Srgb(_) => "srgb",
            ColorValue::DisplayP3(_) => "display-p3",
            ColorValue::Cmyk(_) => "cmyk",
            ColorValue::Spot(_) => "spot",
        }
    }
    pub fn alpha(&self) -> f64 {
        match self {
            ColorValue::Srgb(c) | ColorValue::DisplayP3(c) => c.a,
            ColorValue::Cmyk(c) => c.a,
            ColorValue::Spot(c) => c.a,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub t: f64,
    pub color: ColorValue,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolidPaint {
    pub color: ColorValue,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearPaint {
    pub stops: Vec<GradientStop>,
    pub angle: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadialPaint {
    pub stops: Vec<GradientStop>,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum ImageFit { Fill = "fill", Fit = "fit", Tile = "tile" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImagePaint {
    pub asset: AssetId,
    /// `fill` = cover (centred), `fit` = contain, `tile`.
    pub fit: ImageFit,
    /// Normalized source rect (like `ImageEl.crop`) pinning the part of the image shown in the box; wins over
    /// `fit`. A photo dropped onto a shape is stored this way so it can be repositioned inside the outline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<StoredRect>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k")]
pub enum Paint {
    #[serde(rename = "solid")]
    Solid(SolidPaint),
    #[serde(rename = "linear")]
    Linear(LinearPaint),
    #[serde(rename = "radial")]
    Radial(RadialPaint),
    #[serde(rename = "image")]
    Image(ImagePaint),
}

impl Paint {
    pub fn solid_color(color: ColorValue) -> Self {
        Paint::Solid(SolidPaint { color, extra: Extra::new() })
    }
    pub fn k(&self) -> &'static str {
        match self {
            Paint::Solid(_) => "solid",
            Paint::Linear(_) => "linear",
            Paint::Radial(_) => "radial",
            Paint::Image(_) => "image",
        }
    }
}

string_enum! {
    pub enum StrokeAlign { Center = "center", Inside = "inside", Outside = "outside" }
}
string_enum! {
    pub enum StrokeCap { Butt = "butt", Round = "round", Square = "square" }
}
string_enum! {
    pub enum StrokeJoin { Miter = "miter", Round = "round", Bevel = "bevel" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f64,
    pub align: StrokeAlign,
    pub cap: StrokeCap,
    pub join: StrokeJoin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<Vec<f64>>,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum BlendMode {
        Normal = "normal", Multiply = "multiply", Screen = "screen", Overlay = "overlay", Darken = "darken",
        Lighten = "lighten", ColorDodge = "color-dodge", ColorBurn = "color-burn", HardLight = "hard-light",
        SoftLight = "soft-light", Difference = "difference", Exclusion = "exclusion", Hue = "hue",
        Saturation = "saturation", Color = "color", Luminosity = "luminosity",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UrlLink {
    pub url: String,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageLink {
    pub page_id: PageId,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum LinkTarget {
    #[serde(rename = "url")]
    Url(UrlLink),
    #[serde(rename = "page")]
    Page(PageLink),
}

impl LinkTarget {
    pub fn url(url: impl Into<String>) -> Self {
        LinkTarget::Url(UrlLink { url: url.into(), extra: Extra::new() })
    }
}
