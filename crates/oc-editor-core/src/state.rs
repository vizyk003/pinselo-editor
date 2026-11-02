// SPDX-License-Identifier: Apache-2.0
//! Editor state and the plain-data input/output types (all serde: they cross the WASM boundary as JSON).

use oc_geometry::{Guide, SpacingMark};
use oc_schema::elements::{AnchorSide, Brush};
use oc_schema::primitives::{Rect, Vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::transform::Handle;
use crate::viewport::Viewport;

/// Keyboard modifiers of a pointer or key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Mods {
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub meta: bool,
    /// Ctrl (key shortcuts treat it like meta).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ctrl: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { shift: false, alt: false, meta: false, ctrl: false };
    pub fn shift() -> Self {
        Mods { shift: true, ..Mods::NONE }
    }
    pub fn alt() -> Self {
        Mods { alt: true, ..Mods::NONE }
    }
    pub fn meta() -> Self {
        Mods { meta: true, ..Mods::NONE }
    }
}

/// What the pointer went down on (the UI hit-tests its own chrome; the canvas is the default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind")]
pub enum HitTarget {
    #[serde(rename = "handle")]
    Handle { handle: Handle },
    #[serde(rename = "rotate")]
    Rotate,
    #[serde(rename = "endpoint")]
    Endpoint { end: u8 },
    #[default]
    #[serde(rename = "canvas")]
    Canvas,
    /// Drag the border after column `index` of a table.
    #[serde(rename = "tableCol")]
    TableCol { id: String, index: i64 },
    /// A node of the path being edited (DS-100): its anchor or one of its handles.
    #[serde(rename = "pathNode")]
    PathNode { s: usize, i: usize, part: oc_geometry::NodePart },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    #[default]
    Select,
    Draw,
    Hand,
    /// Eraser for freehand strokes (g55): drags erase the parts of draw strokes they cross.
    Erase,
    /// Pen (DS-100): click for corner nodes, drag for smooth ones, click the first node to close.
    Pen,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrushState {
    pub kind: Brush,
    pub color: String,
    pub size: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEditState {
    pub id: String,
    pub anchor: i64,
    pub focus: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Drawing {
    pub page_id: String,
    pub points: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellRef {
    pub id: String,
    pub r: i64,
    pub c: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellRangeRef {
    pub id: String,
    pub r0: i64,
    pub c0: i64,
    pub r1: i64,
    pub c1: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub page_id: String,
    pub started_at: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayAnchor {
    pub at: f64,
    pub time: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectTarget {
    pub id: String,
    pub anchor: AnchorSide,
    pub point: Vec2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopiedText {
    pub run: Map<String, Value>,
    pub paragraph: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<Value>,
}

/// Style picked up by "Copy style" (ED-070), applied to compatible elements. Parts are plain JSON objects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopiedStyle {
    pub from: String,
    /// `{ opacity, blend, effects }` (absent keys = undefined).
    pub base: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<CopiedText>,
    /// `{ fills, strokes, cornerRadius? }`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Map<String, Value>>,
    /// `{ edits?, cornerRadius? }`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Map<String, Value>>,
    /// `{ stroke, startMarker, endMarker, route? }`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Map<String, Value>>,
}

pub const CLIPBOARD_MIME: &str = "application/x-opencanvas+json";

/// The clipboard model (also what goes on the system clipboard under [`CLIPBOARD_MIME`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipboardPayload {
    pub kind: String,
    pub elements: Vec<Map<String, Value>>,
    pub roots: Vec<String>,
    #[serde(default)]
    pub assets: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindMatch {
    pub id: String,
    pub page_id: String,
    pub offset: i64,
    pub length: i64,
    pub context: String,
}

/// Everything the UI renders besides the document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorState {
    pub active_page_id: String,
    pub selection: Vec<String>,
    pub hover: Option<String>,
    pub entered_group: Option<String>,
    pub editing_text: Option<TextEditState>,
    pub cropping_id: Option<String>,
    pub viewport: Viewport,
    pub guides: Vec<Guide>,
    /// Scene-space marquee while dragging on empty canvas.
    pub marquee: Option<Rect>,
    /// True while a gesture is in progress (UI hides floating toolbars).
    pub gesturing: bool,
    pub presenting: bool,
    pub tool: Tool,
    pub brush: BrushState,
    /// In-progress freehand stroke in page space (overlay preview).
    pub drawing: Option<Drawing>,
    /// Table cell being edited (the caret cell; also the anchor of a cell range).
    pub editing_cell: Option<CellRef>,
    /// Cells selected inside a table.
    pub cell_range: Option<CellRangeRef>,
    /// Animation preview: wall-clock start (ms) and the page being previewed.
    pub preview: Option<Preview>,
    /// Timeline playhead in design time; `None` = static editing view.
    pub playhead: Option<f64>,
    pub playing: bool,
    /// Wall-clock anchor while playing: playhead = anchor.time + (now - anchor.at).
    pub play_anchor: Option<PlayAnchor>,
    /// Equal-spacing measurements shown while dragging.
    pub spacing: Vec<SpacingMark>,
    /// Connector being dragged: the anchor it will glue to on release.
    pub connect_target: Option<ConnectTarget>,
    /// "Copy style" is armed: the next click applies it.
    pub style_brush: Option<CopiedStyle>,
    /// Keyframe recording (MV-100).
    pub record_keyframes: bool,
    /// Path being drawn with the pen (DS-100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pen: Option<PenState>,
    /// Path whose nodes are being edited (DS-100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_edit: Option<PathEditState>,
}

/// A path in progress with the pen: nodes in page space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PenState {
    pub page_id: String,
    pub nodes: Vec<oc_geometry::PathNode>,
    /// Where the next node would go (pointer hover or the keyboard cursor), page space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Vec2>,
    /// The last node's handles are being dragged out.
    #[serde(default)]
    pub dragging: bool,
}

/// Node editing of a path element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathEditState {
    pub id: String,
    /// Selected nodes.
    pub nodes: Vec<oc_geometry::NodeRef>,
}
