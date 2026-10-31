// SPDX-License-Identifier: Apache-2.0
//! Element types. docs/plan/03-document-model-and-renderer.md §2.3, §2.5. Unknown element types are preserved
//! (`Element::Unknown`).

use indexmap::IndexMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::macros::string_enum;
use crate::primitives::{
    AssetId, BlendMode, ColorValue, ElementId, Extra, LinkTarget, PageId, Paint, StoredRect, Stroke, Vec2, Xf,
};
use crate::text::{TextAlign, TextBody, TextEffect};

string_enum! {
    pub enum ElementType {
        Text = "text", Shape = "shape", Line = "line", Image = "image", Frame = "frame", Group = "group", Qr = "qr",
        Draw = "draw", Table = "table", Chart = "chart", Video = "video", Grid = "grid", Form = "form", Embed = "embed",
    }
}
pub const ELEMENT_TYPES: &[&str] =
    &["text", "shape", "line", "image", "frame", "group", "qr", "draw", "table", "chart", "video", "grid", "form", "embed"];

string_enum! {
    pub enum SemanticRole {
        Headline = "headline", Subhead = "subhead", Body = "body", Quote = "quote", Caption = "caption", Cta = "cta",
        Logo = "logo", HeroImage = "hero-image", Background = "background", Decor = "decor", Price = "price",
        Legal = "legal",
    }
}

string_enum! {
    pub enum LockBy { User = "user", Brand = "brand" }
}

/// Element lock. `by: 'user'` locks are a convenience; `by: 'brand'` locks come from a brand template (A8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockMask {
    #[serde(default, rename = "move", skip_serializing_if = "Option::is_none")]
    pub move_: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resize: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<bool>,
    pub by: LockBy,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropShadowEffect {
    pub dx: f64,
    pub dy: f64,
    pub blur: f64,
    pub color: ColorValue,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlurEffect {
    pub radius: f64,
    #[serde(flatten)]
    pub extra: Extra,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineEffect {
    pub width: f64,
    pub color: ColorValue,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k")]
pub enum Effect {
    #[serde(rename = "dropShadow")]
    DropShadow(DropShadowEffect),
    #[serde(rename = "blur")]
    Blur(BlurEffect),
    #[serde(rename = "outline")]
    Outline(OutlineEffect),
}

macro_rules! opt_f64_struct {
    ($(#[$m:meta])* $name:ident { $($field:ident : $json:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
        pub struct $name {
            $(
                #[serde(default, rename = $json, skip_serializing_if = "Option::is_none")]
                pub $field: Option<f64>,
            )+
            #[serde(flatten)]
            pub extra: Extra,
        }
    };
}

opt_f64_struct! {
    ImageAdjust {
        brightness: "brightness", contrast: "contrast", saturation: "saturation", temperature: "temperature",
        tint: "tint", blur: "blur", vignette: "vignette", highlights: "highlights", shadows: "shadows",
        whites: "whites", blacks: "blacks", sharpen: "sharpen", clarity: "clarity", fade: "fade",
        pixelate: "pixelate",
    }
}

string_enum! {
    pub enum ImageFilterPreset {
        None = "none", Mono = "mono", Sepia = "sepia", Vivid = "vivid", Fade = "fade", Cool = "cool", Warm = "warm",
        Noir = "noir", Retro = "retro", Chrome = "chrome", Dramatic = "dramatic", Vintage = "vintage",
        Summer = "summer", Dusk = "dusk", Lomo = "lomo", Crisp = "crisp", Pastel = "pastel", Matte = "matte",
        TealOrange = "teal-orange", Blush = "blush", Forest = "forest", Silver = "silver", Contrast = "contrast",
        Selenium = "selenium", Cyanotype = "cyanotype", Invert = "invert", Infrared = "infrared", Golden = "golden",
        Arctic = "arctic", Rose = "rose", Mint = "mint", Lavender = "lavender", Sunset = "sunset",
        Duo = "duo", Neon = "neon",
    }
}
pub const FILTER_PRESETS: &[&str] = &[
    "none", "vivid", "crisp", "chrome", "dramatic", "warm", "summer", "cool", "teal-orange", "dusk", "fade", "matte",
    "pastel", "blush", "forest", "vintage", "retro", "lomo", "sepia", "mono", "noir", "silver", "contrast", "selenium",
    "cyanotype", "invert", "infrared", "golden", "arctic", "rose", "mint", "lavender", "sunset", "duo", "neon",
];

/// Duotone (IM-012): luminance mapped from `shadow` to `highlight`, blended by `amount` (0..1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Duotone {
    pub shadow: ColorValue,
    pub highlight: ColorValue,
    pub amount: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageFilter {
    pub preset: ImageFilterPreset,
    pub amount: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageEditStack {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjust: Option<ImageAdjust>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<ImageFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duotone: Option<Duotone>,
    pub version: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    /// Entrance animation (MV-002).
    pub enum AnimPreset {
        Fade = "fade", Rise = "rise", Drop = "drop", Pan = "pan", Pop = "pop", Zoom = "zoom", Wipe = "wipe",
        Blur = "blur", Tumble = "tumble", Breathe = "breathe", Slide = "slide", Flip = "flip", Stomp = "stomp",
        Spin = "spin", Bounce = "bounce", Float = "float", Shrink = "shrink", Typewriter = "typewriter",
    }
}
pub const ANIM_PRESETS: &[&str] = &[
    "fade", "rise", "drop", "pan", "slide", "pop", "zoom", "shrink", "stomp", "bounce", "wipe", "blur", "flip", "spin",
    "tumble", "float", "breathe", "typewriter",
];
string_enum! {
    pub enum AnimDirection { Up = "up", Down = "down", Left = "left", Right = "right" }
}
string_enum! {
    /// Text can animate as a whole or one line (paragraph), word or letter at a time.
    pub enum AnimUnit { Element = "element", Line = "line", Word = "word", Char = "char" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementAnim {
    pub preset: AnimPreset,
    pub duration_ms: f64,
    pub delay_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<AnimDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<AnimUnit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stagger_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_click: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    /// Keyframes (MV-100): properties animated over page time, as offsets from the element's own values.
    pub enum KeyframeProp { X = "x", Y = "y", Scale = "scale", Rotate = "rotate", Opacity = "opacity", Blur = "blur" }
}
pub const KEYFRAME_PROPS: &[&str] = &["x", "y", "scale", "rotate", "opacity", "blur"];
string_enum! {
    pub enum KeyframeEase {
        Linear = "linear", Ease = "ease", EaseIn = "ease-in", EaseOut = "ease-out", Back = "back", Hold = "hold",
    }
}

/// `t` = page time (ms); `ease` shapes the segment that ENDS at this key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub t: f64,
    pub v: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ease: Option<KeyframeEase>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `Partial<Record<KeyframeProp, Keyframe[]>>`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Keyframes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<Vec<Keyframe>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<Vec<Keyframe>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Vec<Keyframe>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<Vec<Keyframe>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<Vec<Keyframe>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur: Option<Vec<Keyframe>>,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    /// Looping emphasis animation (MV-004).
    pub enum EmphasisPreset {
        Pulse = "pulse", Wiggle = "wiggle", Bounce = "bounce", Flicker = "flicker", Spin = "spin",
        Heartbeat = "heartbeat", Float = "float", Swing = "swing",
    }
}
pub const EMPHASIS_PRESETS: &[&str] = &["pulse", "wiggle", "bounce", "flicker", "spin", "heartbeat", "float", "swing"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementEmphasis {
    pub preset: EmphasisPreset,
    pub period_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementAnimOut {
    pub preset: AnimPreset,
    pub duration_ms: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum MotionEasing { Linear = "linear", Ease = "ease" }
}

/// Motion path (MV-011): a polyline of offsets (box px) relative to the element's position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionPath {
    pub points: Vec<f64>,
    pub duration_ms: f64,
    pub delay_ms: f64,
    pub easing: MotionEasing,
    #[serde(flatten)]
    pub extra: Extra,
}

/// When an element is visible on its page (page time, ms). Omitted = whole page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementTiming {
    pub start_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum OriginChannel { Editor = "editor", Template = "template", Ai = "ai", Api = "api", Import = "import" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignOrigin {
    pub channel: OriginChannel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataBinding {
    pub column: String,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tombstone {
    pub at: f64,
    pub by: String,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StickyAuthor {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StickyMeta {
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<StickyAuthor>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionCollapsed {
    pub h: f64,
    pub hidden: Vec<ElementId>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionMeta {
    pub title: String,
    pub order: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<SectionCollapsed>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `Option<Option<T>>` that distinguishes an absent field (`None`) from an explicit `null` (`Some(None)`).
fn double_option<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MindMeta {
    /// `None` = absent, `Some(None)` = explicit `null` (root).
    #[serde(default, deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]
    pub parent: Option<Option<ElementId>>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Whiteboard metadata (WB-020..): sticky notes, sections and mind-map nodes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BoardMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticky: Option<StickyMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<SectionMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mind: Option<MindMeta>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Fields shared by every element (everything but `type`, which is the `Element` variant).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementBase {
    pub id: ElementId,
    /// Flat map + parent pointer. Order within a parent = sort by (z, id).
    pub parent_id: String,
    pub z: String,
    pub xf: Xf,
    pub opacity: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<BlendMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<LockMask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<SemanticRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decorative: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<Effect>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim: Option<ElementAnim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emphasis: Option<ElementEmphasis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyframes: Option<Keyframes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim_out: Option<ElementAnimOut>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_path: Option<MotionPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<ElementTiming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<DesignOrigin>,
    /// Bulk create (ED-090): the data column that fills this element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_binding: Option<DataBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<BoardMeta>,
    /// Tombstone. Deleted elements stay in the map so undo can restore concurrent edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Tombstone>,
    /// Fields written by a newer client; preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<Map<String, Value>>,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum ShapeKind {
        Rect = "rect", Ellipse = "ellipse", Triangle = "triangle", Polygon = "polygon", Star = "star", Arrow = "arrow",
        Heart = "heart", Diamond = "diamond", Hexagon = "hexagon", Speech = "speech", Cross = "cross",
        Chevron = "chevron", Octagon = "octagon", Pentagon = "pentagon", Parallelogram = "parallelogram",
        Trapezoid = "trapezoid", RightTriangle = "right-triangle", Ring = "ring", HalfCircle = "half-circle",
        Moon = "moon", Lightning = "lightning", Cloud = "cloud", Seal = "seal", Tag = "tag", Ribbon = "ribbon",
        DoubleArrow = "double-arrow", Plus = "plus", Blob = "blob", Arch = "arch", Drop = "drop", Burst = "burst",
        Flag = "flag", Squircle = "squircle", Pill = "pill", SnipCorner = "snip-corner", SnipTwo = "snip-two",
        RoundOne = "round-one", FoldedCorner = "folded-corner", Plaque = "plaque", Frame = "frame",
        Ticket = "ticket", Shield = "shield", Cylinder = "cylinder", Cube = "cube", BlockArc = "block-arc",
        Pie = "pie", Star6 = "star-6", Star8 = "star-8", Starburst12 = "starburst-12", Sparkle = "sparkle",
        Badge = "badge", Gear = "gear", Flower = "flower", Sun = "sun", Leaf = "leaf", CrossX = "cross-x",
        Check = "check", ChevronDouble = "chevron-double", CurvedArrow = "curved-arrow",
        UTurnArrow = "u-turn-arrow", CircularArrow = "circular-arrow", Banner = "banner", Wave = "wave",
        CalloutRect = "callout-rect", CalloutRound = "callout-round", ThoughtBubble = "thought-bubble",
        Document = "document", MultiDocument = "multi-document", PredefinedProcess = "predefined-process",
        ManualInput = "manual-input", Delay = "delay", OffPage = "off-page", InternalStorage = "internal-storage",
        Display = "display",
        /// Free-form outline from `d` (imports: PowerPoint custom geometry, SVG, PDF).
        Custom = "custom",
    }
}
/// Order drives the Elements panel grid.
pub const SHAPE_KINDS: &[&str] = &[
    "rect", "ellipse", "triangle", "right-triangle", "diamond", "squircle", "pill", "half-circle", "arch", "ring",
    "block-arc", "pie", "snip-corner", "snip-two", "round-one", "plaque", "folded-corner", "frame", "ticket",
    "cylinder", "cube", "pentagon", "hexagon", "octagon", "polygon", "parallelogram", "trapezoid", "shield", "star",
    "star-6", "star-8", "starburst-12", "burst", "seal", "badge", "sparkle", "sun", "gear", "flower", "heart", "moon",
    "drop", "leaf", "cloud", "lightning", "blob", "cross", "plus", "cross-x", "check", "arrow", "double-arrow",
    "chevron", "chevron-double", "curved-arrow", "u-turn-arrow", "circular-arrow", "tag", "ribbon", "flag", "banner",
    "wave", "speech", "callout-rect", "callout-round", "thought-bubble", "document", "multi-document",
    "predefined-process", "manual-input", "delay", "off-page", "internal-storage", "display",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub kind: ShapeKind,
    /// cornerRadius, sides, points, innerRatio, …
    pub params: IndexMap<String, f64>,
    /// For kind 'custom': absolute SVG path (M L C Q Z) in a unit box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<String>,
    pub fills: Vec<Paint>,
    pub strokes: Vec<Stroke>,
}

string_enum! {
    pub enum LineMarker {
        None = "none", Arrow = "arrow", Triangle = "triangle", Circle = "circle", Square = "square", Bar = "bar",
        Stealth = "stealth", Diamond = "diamond", TriangleOutline = "triangle-outline",
        CircleOutline = "circle-outline", SquareOutline = "square-outline", DiamondOutline = "diamond-outline",
    }
}
pub const LINE_MARKERS: &[&str] = &[
    "none", "arrow", "triangle", "stealth", "triangle-outline", "circle", "circle-outline", "square", "square-outline",
    "diamond", "diamond-outline", "bar",
];
string_enum! {
    /// Where a connector end is glued (WB-010).
    pub enum AnchorSide { Top = "top", Right = "right", Bottom = "bottom", Left = "left", Center = "center" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineAttach {
    pub id: ElementId,
    pub anchor: AnchorSide,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum LineRoute { Straight = "straight", Elbow = "elbow", Curved = "curved" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineEl {
    #[serde(flatten)]
    pub base: ElementBase,
    /// Endpoints in element space (0..w, 0..h of xf).
    pub points: Vec<Vec2>,
    pub stroke: Stroke,
    pub start_marker: LineMarker,
    pub end_marker: LineMarker,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_attach: Option<LineAttach>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_attach: Option<LineAttach>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<LineRoute>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub asset: AssetId,
    /// Normalized source crop (0..1).
    pub crop: StoredRect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<ImageEditStack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    /// SVG assets only: original colour → replacement `#rrggbb[aa]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub svg_colors: Option<IndexMap<String, String>>,
}

string_enum! {
    pub enum FrameClip {
        Rect = "rect", Ellipse = "ellipse", Triangle = "triangle", Star = "star", Heart = "heart",
        Hexagon = "hexagon", Diamond = "diamond", Rounded = "rounded",
    }
}

/// Unit-box path (0..1) filled above a frame's content; holes are counter-wound (nonzero fill).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameOverlay {
    pub d: String,
    pub fill: Paint,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameContent {
    pub asset: AssetId,
    pub crop: StoredRect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<ImageEditStack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<MediaClip>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub clip: FrameClip,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<Vec<FrameOverlay>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<FrameContent>,
    pub placeholder_fill: Paint,
}

string_enum! {
    /// How a mask's pixels gate the masked content (DS-107): by coverage (alpha) or by brightness × coverage.
    pub enum MaskMode { Alpha = "alpha", Luminance = "luminance" }
}

/// A group whose child `id` masks the group's other children (DS-107). The mask element itself is not drawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupMask {
    pub id: ElementId,
    pub mode: MaskMode,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupEl {
    #[serde(flatten)]
    pub base: ElementBase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<GroupMask>,
}

string_enum! {
    pub enum TextFrameMode { AutoHeight = "autoHeight", Fixed = "fixed" }
}
string_enum! {
    pub enum AutoFit { None = "none", Shrink = "shrink" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextFrame {
    pub mode: TextFrameMode,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub body: TextBody,
    pub frame: TextFrame,
    pub auto_fit: AutoFit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<TextEffect>,
    /// Vertical anchor of the text in a fixed-height box (default top).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valign: Option<CellVAlign>,
}

string_enum! {
    pub enum QrEcc { L = "L", M = "M", Q = "Q", H = "H" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QrEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub data: String,
    pub ecc: QrEcc,
    pub fg: ColorValue,
    pub bg: ColorValue,
}

string_enum! {
    /// Freehand brush (PT-010).
    pub enum Brush { Pen = "pen", Marker = "marker", Highlighter = "highlighter" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub w: f64,
    pub h: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawEl {
    #[serde(flatten)]
    pub base: ElementBase,
    /// `[x, y, pressure]` triples in the element box at its creation size (`base`).
    pub points: Vec<f64>,
    #[serde(rename = "base")]
    pub base_size: Size,
    pub brush: Brush,
    pub color: ColorValue,
    pub size: f64,
}

/// One side of a cell border; width 0 hides it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellBorder {
    pub width: f64,
    pub color: ColorValue,
    #[serde(flatten)]
    pub extra: Extra,
}

impl CellBorder {
    pub fn new(width: f64, color: ColorValue) -> Self {
        CellBorder { width, color, extra: Extra::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CellBorders {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<CellBorder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<CellBorder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<CellBorder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub l: Option<CellBorder>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellSpan {
    pub rows: f64,
    pub cols: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

string_enum! {
    pub enum CellAlign { Start = "start", Center = "center", End = "end" }
}
string_enum! {
    pub enum CellVAlign { Top = "top", Middle = "middle", Bottom = "bottom" }
}

impl CellAlign {
    pub fn to_text_align(&self) -> TextAlign {
        TextAlign::parse(self.as_str())
    }
}

/// Table cell (ED-028, A12).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TableCell {
    /// Plain-text mirror of the cell.
    pub text: String,
    /// Rich content (A12); authoritative when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<TextBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<CellAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valign: Option<CellVAlign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub borders: Option<CellBorders>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<CellSpan>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl TableCell {
    pub fn text(text: impl Into<String>) -> Self {
        TableCell { text: text.into(), ..Default::default() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableStyle {
    pub family: String,
    pub size: f64,
    pub color: ColorValue,
    pub border_color: ColorValue,
    pub border_width: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_fill: Option<Paint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_color: Option<ColorValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe_fill: Option<Paint>,
    pub padding: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Table (ED-028). Track sizes are fractions of the box; cells are sparse, keyed "r:c".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub rows: Vec<f64>,
    pub cols: Vec<f64>,
    pub cells: IndexMap<String, TableCell>,
    pub style: TableStyle,
}

string_enum! {
    /// Static chart (PR-060).
    pub enum ChartKind {
        Bar = "bar", Column = "column", Line = "line", Area = "area", Pie = "pie", Donut = "donut",
        StackedColumn = "stacked-column", StackedBar = "stacked-bar", Combo = "combo", Scatter = "scatter",
        Bubble = "bubble", Radar = "radar", Funnel = "funnel", ProgressRing = "progress-ring",
        ProgressBar = "progress-bar", Gauge = "gauge",
    }
}
pub const CHART_KINDS: &[&str] = &[
    "column", "bar", "stacked-column", "stacked-bar", "line", "area", "combo", "pie", "donut", "scatter", "bubble",
    "radar", "funnel", "progress-ring", "progress-bar", "gauge",
];

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartFormat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thousands: Option<bool>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartSeries {
    pub name: String,
    pub values: Vec<f64>,
    pub color: ColorValue,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Linked data (A15): the chart mirrors a range of a sheet design.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartSource {
    pub design_id: String,
    pub sheet: String,
    pub range: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartStyle {
    pub family: String,
    pub text_color: ColorValue,
    pub grid_color: ColorValue,
    pub show_legend: bool,
    pub show_values: bool,
    pub show_grid: bool,
    pub font_size: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ChartFormat>,
    /// progress/gauge maximum (default 100)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// pie/donut/funnel: colour per slice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_colors: Option<Vec<ColorValue>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub kind: ChartKind,
    pub labels: Vec<String>,
    pub series: Vec<ChartSeries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ChartSource>,
    pub style: ChartStyle,
}

/// Media clip (MV-020): a window of a source file played at `speed`. Source times in ms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaClip {
    pub trim_start_ms: f64,
    pub trim_end_ms: f64,
    pub speed: f64,
    pub volume: f64,
    pub muted: bool,
    #[serde(rename = "loop")]
    pub loop_: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub asset: AssetId,
    pub clip: MediaClip,
    pub crop: StoredRect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<ImageEditStack>,
}

/// Photo grid cell (IM-030). `video` set = the cell holds a video asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridCell {
    pub rect: StoredRect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<StoredRect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<ImageEditStack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<MediaClip>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub cells: Vec<GridCell>,
    /// Spacing between cells and at the outer edge, in px.
    pub gap: f64,
    pub radius: f64,
    pub placeholder_fill: Paint,
}

string_enum! {
    /// Forms (FM-001).
    pub enum FormFieldKind {
        Text = "text", Longtext = "longtext", Email = "email", Number = "number", Date = "date", Choice = "choice",
        Checkboxes = "checkboxes",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormField {
    pub id: String,
    pub kind: FormFieldKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl FormField {
    pub fn new(id: &str, kind: FormFieldKind, label: &str, required: Option<bool>) -> Self {
        FormField {
            id: id.into(),
            kind,
            label: label.into(),
            required,
            options: None,
            placeholder: None,
            extra: Extra::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormStyle {
    pub family: String,
    pub size: f64,
    pub color: ColorValue,
    pub accent: ColorValue,
    pub field_fill: ColorValue,
    pub border_color: ColorValue,
    pub radius: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub title: String,
    pub fields: Vec<FormField>,
    pub submit_label: String,
    pub thanks: String,
    pub style: FormStyle,
}

string_enum! {
    pub enum EmbedProvider {
        Youtube = "youtube", Vimeo = "vimeo", Spotify = "spotify", Soundcloud = "soundcloud", Maps = "maps",
        Loom = "loom", Figma = "figma", Codepen = "codepen",
    }
}

/// Embedded media (A11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedEl {
    #[serde(flatten)]
    pub base: ElementBase,
    pub url: String,
    pub provider: EmbedProvider,
    pub src: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<AssetId>,
}

/// Any element. Unknown `type`s (written by newer clients) are kept verbatim in `Unknown`.
// Every known variant embeds the same `ElementBase`, so the variants are similar in size; boxing them would only
// add an indirection to every element access.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum Element {
    #[serde(rename = "shape")]
    Shape(ShapeEl),
    #[serde(rename = "line")]
    Line(LineEl),
    #[serde(rename = "image")]
    Image(ImageEl),
    #[serde(rename = "frame")]
    Frame(FrameEl),
    #[serde(rename = "group")]
    Group(GroupEl),
    #[serde(rename = "text")]
    Text(TextEl),
    #[serde(rename = "qr")]
    Qr(QrEl),
    #[serde(rename = "draw")]
    Draw(DrawEl),
    #[serde(rename = "table")]
    Table(TableEl),
    #[serde(rename = "chart")]
    Chart(ChartEl),
    #[serde(rename = "video")]
    Video(VideoEl),
    #[serde(rename = "grid")]
    Grid(GridEl),
    #[serde(rename = "form")]
    Form(FormEl),
    #[serde(rename = "embed")]
    Embed(EmbedEl),
    /// An element type this build does not know (the whole JSON object, `type` included).
    #[serde(untagged)]
    Unknown(Map<String, Value>),
}

impl<'de> Deserialize<'de> for Element {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut map = Map::<String, Value>::deserialize(d)?;
        let ty = match map.get("type") {
            Some(Value::String(s)) if ELEMENT_TYPES.contains(&s.as_str()) => s.clone(),
            _ => return Ok(Element::Unknown(map)),
        };
        map.remove("type");
        let v = Value::Object(map);
        macro_rules! go {
            ($variant:ident) => {
                serde_json::from_value(v).map(Element::$variant).map_err(D::Error::custom)
            };
        }
        match ty.as_str() {
            "shape" => go!(Shape),
            "line" => go!(Line),
            "image" => go!(Image),
            "frame" => go!(Frame),
            "group" => go!(Group),
            "text" => go!(Text),
            "qr" => go!(Qr),
            "draw" => go!(Draw),
            "table" => go!(Table),
            "chart" => go!(Chart),
            "video" => go!(Video),
            "grid" => go!(Grid),
            "form" => go!(Form),
            "embed" => go!(Embed),
            _ => unreachable!("checked against ELEMENT_TYPES"),
        }
    }
}

macro_rules! each_known {
    ($e:expr, $x:ident => $body:expr, $unknown:pat => $ub:expr) => {
        match $e {
            Element::Shape($x) => $body,
            Element::Line($x) => $body,
            Element::Image($x) => $body,
            Element::Frame($x) => $body,
            Element::Group($x) => $body,
            Element::Text($x) => $body,
            Element::Qr($x) => $body,
            Element::Draw($x) => $body,
            Element::Table($x) => $body,
            Element::Chart($x) => $body,
            Element::Video($x) => $body,
            Element::Grid($x) => $body,
            Element::Form($x) => $body,
            Element::Embed($x) => $body,
            $unknown => $ub,
        }
    };
}

impl Element {
    /// Shared fields; `None` for unknown element types.
    pub fn base(&self) -> Option<&ElementBase> {
        each_known!(self, e => Some(&e.base), Element::Unknown(_) => None)
    }

    pub fn base_mut(&mut self) -> Option<&mut ElementBase> {
        each_known!(self, e => Some(&mut e.base), Element::Unknown(_) => None)
    }

    /// The `type` string.
    pub fn type_name(&self) -> &str {
        match self {
            Element::Shape(_) => "shape",
            Element::Line(_) => "line",
            Element::Image(_) => "image",
            Element::Frame(_) => "frame",
            Element::Group(_) => "group",
            Element::Text(_) => "text",
            Element::Qr(_) => "qr",
            Element::Draw(_) => "draw",
            Element::Table(_) => "table",
            Element::Chart(_) => "chart",
            Element::Video(_) => "video",
            Element::Grid(_) => "grid",
            Element::Form(_) => "form",
            Element::Embed(_) => "embed",
            Element::Unknown(m) => m.get("type").and_then(Value::as_str).unwrap_or(""),
        }
    }

    fn unknown_str<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
        m.get(key).and_then(Value::as_str)
    }

    /// `el.id` (for unknown types: the `id` string field, if any).
    pub fn id(&self) -> Option<&str> {
        each_known!(self, e => Some(e.base.id.as_str()), Element::Unknown(m) => Self::unknown_str(m, "id"))
    }

    pub fn parent_id(&self) -> Option<&str> {
        each_known!(self, e => Some(e.base.parent_id.as_str()), Element::Unknown(m) => Self::unknown_str(m, "parentId"))
    }

    pub fn z(&self) -> Option<&str> {
        each_known!(self, e => Some(e.base.z.as_str()), Element::Unknown(m) => Self::unknown_str(m, "z"))
    }

    /// Truthiness of `el.deleted`.
    pub fn is_deleted(&self) -> bool {
        each_known!(self, e => e.base.deleted.is_some(), Element::Unknown(m) => m.get("deleted").is_some_and(crate::tree::truthy))
    }

    pub fn xf(&self) -> Option<&Xf> {
        self.base().map(|b| &b.xf)
    }
}

macro_rules! from_variant {
    ($($variant:ident($ty:ty)),+ $(,)?) => {
        $(impl From<$ty> for Element {
            fn from(e: $ty) -> Self {
                Element::$variant(e)
            }
        })+
    };
}

from_variant!(
    Shape(ShapeEl), Line(LineEl), Image(ImageEl), Frame(FrameEl), Group(GroupEl), Text(TextEl), Qr(QrEl), Draw(DrawEl),
    Table(TableEl), Chart(ChartEl), Video(VideoEl), Grid(GridEl), Form(FormEl), Embed(EmbedEl),
);

/// Fields stored at the top level of an element map (everything else is type-specific `props`).
pub const BASE_KEYS: &[&str] = &[
    "id", "type", "parentId", "z", "xf", "opacity", "blend", "visible", "lock", "name", "role", "link", "altText",
    "decorative", "effects", "anim", "emphasis", "keyframes", "animOut", "motionPath", "timing", "origin",
    "dataBinding", "deleted", "ext",
];

pub fn is_container(el: &Element) -> bool {
    matches!(el, Element::Group(_))
}

/// `PageId | ElementId` parent pointer type used by helpers.
pub type ParentId = PageId;
