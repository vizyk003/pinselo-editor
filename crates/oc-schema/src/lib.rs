// SPDX-License-Identifier: Apache-2.0
//! Pinselo document model — the Rust port of `@oc/schema` (see docs/adr/0001-rust-core.md).
//!
//! * Types serialise to the same JSON as the TS shapes (`serde`, camelCase, optional fields omitted, unknown
//!   fields of loose objects preserved in `extra`). Use [`js::to_string`] / [`js::to_value`] (or
//!   [`DesignDoc::to_json`]) to get `JSON.stringify`-identical number formatting (`1080`, not `1080.0`).
//! * Records keyed by id are `IndexMap`s (JSON insertion order, like JS objects).
//! * Pure helpers reproduce the TS results exactly (verified against goldens generated from the TS package in
//!   `tests/golden/schema/`).

mod macros;

pub mod color;
pub mod design;
pub mod doc_blocks;
pub mod elements;
pub mod embeds;
pub mod factory;
pub mod frame_glyphs;
pub mod fractional;
pub mod frames;
pub mod ids;
pub mod js;
pub mod keyframes;
pub mod links;
pub mod migrate;
pub mod presets;
pub mod primitives;
pub mod print;
pub mod svg;
pub mod table;
pub mod text;
pub mod timeline;
pub mod tree;
pub mod validate;

pub use design::{AssetRef, DesignDoc, DesignMeta, Page, Surface, CURRENT_SCHEMA, LAYOUT_VERSION};
pub use elements::{Element, ElementBase, ElementType};
pub use primitives::{ColorValue, Extra, Paint, Rect, Stroke, Unit, Xf};
pub use text::{Paragraph, Run, RunStyle, TextBody};
pub use validate::{ValidationIssue, parse_doc, validate_doc};
