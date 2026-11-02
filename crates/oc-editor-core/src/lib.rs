// SPDX-License-Identifier: Apache-2.0
//! Pinselo editor engine — the Rust port of `@oc/editor-core` (see docs/adr/0001-rust-core.md).
//!
//! The editor owns selection, tools, gestures, the clipboard model and every editing command, and writes
//! through an `oc_crdt::DocStore` (DocOps in, DocOps out). It is DOM-free and will be compiled to WASM behind a
//! thin TS facade in `apps/web`; the React shell keeps DOM events, IME, focus and ARIA.
//!
//! * **Inputs are plain data**: pointer events as scene points + [`Mods`] + [`HitTarget`], keys as
//!   `KeyboardEvent.key`/`code` strings, commands with JSON-shaped arguments. [`Editor::dispatch`] accepts every
//!   command by its TS name with JSON arguments (the facade contract; the golden replay uses it too).
//! * **Outputs are serialisable**: [`EditorState`], [`Editor::doc_json`] / [`Editor::view_json`], command results
//!   (serde), plus [`Editor::version`] / [`Editor::subscribe`] change notifications.
//! * **Sync**: in the browser the yjs `Y.Doc` stays authoritative; the editor holds a yrs mirror. Feed remote
//!   updates with [`Editor::apply_remote_update`] and forward [`Editor::drain_local_updates`] to the yjs doc.
//! * **No time, fs or threads**: the clock and the RNG are injected ([`EditorOptions`]); text measurement is the
//!   injected [`TextService`].
//!
//! # API surface (mirrors the TS `@oc/editor-core` exports used by `apps/web`, 1:1)
//!
//! Module functions/constants (TS name → Rust):
//! `pageSlots`→[`page_slots`], `screenToScene`→[`screen_to_scene`], `sceneToScreen`→[`scene_to_screen`],
//! `slotOf`, `pageAtScene`, `zoomAt`, `fitRect`, `clampZoom`, `pageGap`, `MIN_ZOOM`/`MAX_ZOOM`, `Viewport`;
//! `Handle`, `CORNERS`, `EDGES`, `resizeBox`, `scaleAbout`, `rotateAbout`, `scaleBody`, `scalePatches`,
//! `cropResize`; `designColors`, `recolorOps`, `paletteMap`, `fontSetOps`, `FontSet`; `auditDesign`,
//! `describeElement`, `minTextPx`, `A11yIssue`, `ElementKindLabel`; `templatePages`; `coverCrop`,
//! `withKeyframe`; types `Mods`, `HitTarget`, `EditorState`, `ClipboardPayload`, `FindMatch`, `CopiedStyle`,
//! `TextService` (a trait here).
//!
//! `Editor` members used by `apps/web` (all implemented; snake_case in Rust, camelCase via `dispatch`):
//! state/doc/view/version/subscribe/store (`store.ydoc` → [`Editor::store`], `store.checkpoint`,
//! `store.apply`, `store.actor`, `undo.undoStack/redoStack.length` → [`Editor::can_undo`]/[`Editor::can_redo`]),
//! textService (→ [`Editor::set_text_service`]), apply, insert, select, selectAll,
//! clearSelection, selected, selectedDeep, selectionBox, setActivePage, setHover, pageOrigin,
//! setViewport, setViewportSize, zoomTo, zoomBy, panBy, zoomToFit, zoomToElement, updateActivePageFromScroll,
//! pointerDown, pointerMove, pointerUp, cancelGesture, doubleClick, handleKey, textShortcut, typeText,
//! deleteText, moveCaret, selectedText, startTextEdit, setTextStyle, setParagraphStyle, currentTextStyle,
//! currentParagraph, editingTableText, editingTextFrame, updateSelected (closure → `update_selected_with`,
//! or per-element patches via `update_selected_map`), copy, cut, paste, duplicate, deleteSelection, align,
//! distribute, tidy, arrange, reorderTo, flip, group, ungroup, toggleLock,
//! copyStyle, cancelStyleBrush, find, replace, revealMatch, setGeometry, setGuides, startCrop, stopCrop,
//! setCropZoom, cropZoom, replaceImage, dropIntoContainer, addPage, duplicatePage, deletePage, movePage,
//! updatePage, resizeDesign, setLanguage, replaceColors, applyPalette, applyFonts, addPageNumbers,
//! bindableElements, setDataBinding, bulkCreate, undo, redo, remeasureText, setTool, setBrush,
//! cellTab, setCells, setCellAlign, setCellBorders, setColumnWidth,
//! tableStructure, tableTargetRange, mergeCellRange, unmergeCellRange, setTableStyle, previewAnimation,
//! stopPreview, currentTime, play/pause/togglePlay, seek, tick, pageStart, setPageDuration, setTiming,
//! insertVideo, splitAtPlayhead, audioClips, addAudio, updateAudio, removeAudio, splitAudio,
//! captions, setCaptions, upsertCue, setRecordKeyframes, setKeyframe, removeKeyframe,
//! moveKeyframe, clearKeyframes.
//! Also ported (not used by apps/web today): addGuide, cellAt, cellTextEl, clearCells, editShapeLabel,
//! fillFrame, gridCellAt, setGridCell, keyframeTime, nudge, pasteStyle,
//! plainText, play, scrollToPage, selectCellRange, selectionFrame, setCell, setPresenting, startCellEdit,
//! stopCellEdit, stopTextEdit, tableTargets, toPage, viewCenter, overrides, CLIPBOARD_MIME.
//! New for the WASM bridge: [`Editor::apply_remote_update`], [`Editor::drain_local_updates`],
//! [`Editor::reload`], [`Editor::flush`], [`Editor::store_apply_with`], [`Editor::rng_mut`] (the id stream for
//! host-built elements such as [`template_pages`]), [`Editor::doc_json`], [`Editor::view_json`],
//! [`Editor::dispatch`] (see [`dispatch`] for the argument conventions).
//!
//! # Parity
//!
//! tools/golden/editor.ts scripts the TS editor through deterministic sessions (tests/golden/editor/); the Rust
//! tests replay them through [`Editor::dispatch`] and compare state, document, overrides, undo availability,
//! notification count and random draws after every step. Known deviations from TS:
//!
//! * **Reactions to undo/redo are untracked.** Auto-height and connector fix-ups caused by an undo or
//!   redo are written with an origin the undo manager ignores. yjs runs them inside the undo transaction's
//!   cleanup (its `undoing` flag still set), so they become an extra redo/undo step; yrs resets that flag before
//!   observers may write, and a tracked write would clear the redo stack instead. Rust keeps exactly the undone
//!   steps on the redo stack (TS's extra step, which reverts the fix-up, does not exist).
//! * **Turning a list off writes `list: null`** where TS removes the paragraph attribute
//!   (`setParagraphStyle({ list: undefined })`): JSON DocOps cannot carry `undefined`. Readers treat both alike.
//! * **`designColors` ties** (equal counts) follow the input's element/key iteration order. TS sees JS insertion
//!   order; typed documents (serde field order) and yrs snapshots (unordered maps) do not keep it.
//! * **Schema-invalid elements** (e.g. a lock without `by`) parse as `Element::Unknown` and are skipped by hit
//!   testing and commands; TS works on such objects structurally.

pub mod a11y;
pub mod dispatch;
pub mod doc;
pub mod editor;
pub mod env;
pub mod resize;
pub mod state;
pub mod styles;
pub mod templates;
pub mod text_service;
pub mod transform;
pub mod util;
pub mod viewport;

pub use a11y::{A11yIssue, A11yIssueCode, ElementDescription, ElementKindLabel, Severity, audit_design, describe_element, min_text_px};
pub use dispatch::DispatchError;
pub use doc::DocView;
pub use editor::commands::{Arrange, FindOpts, GeometryPatch, PageNumberPosition, ResolvedImage, cover_crop};
pub use editor::text::{CaretKey, EditingTextFrame};
pub use editor::timeline::{MediaAsset, with_keyframe};
pub use editor::{CellTextFrame, Editor, EditorOptions};
pub use env::{Clock, ManualClock, Mulberry32, OsRandom, RandomSource};
pub use resize::resize_doc;
pub use state::*;
pub use styles::{ColorCount, FontSet, design_colors, font_set_ops, palette_map, recolor_ops};
pub use templates::{ImportedPage, template_pages};
pub use text_service::TextService;
pub use transform::{CORNERS, EDGES, Handle, Patches, ResizeResult, crop_resize, refit_crop, resize_box, rotate_about, scale_about, scale_body, scale_patches};
pub use viewport::{
    MAX_ZOOM, MIN_ZOOM, PageSlot, Viewport, clamp_zoom, fit_rect, page_at_scene, page_gap, page_slots, scene_to_screen, screen_to_scene,
    slot_of, zoom_at,
};
