// SPDX-License-Identifier: Apache-2.0
//! JSON command dispatcher: every `Editor` member `apps/web` uses, called by its TS name with the TS call's
//! arguments as a JSON array (`editor.dispatch("pointerDown", &json!([[500, 450], { "shift": false }]))`).
//! This is the WASM facade contract (the TS facade forwards `(name, [...arguments])`) and what the golden
//! replay drives.
//!
//! Argument conventions (JSON has no `undefined`):
//! * A missing trailing argument is `undefined` (the TS default applies). The facade trims trailing
//!   `undefined`s before serialising.
//! * `null` is `null`. Where TS treats `null` and `undefined` differently, so does the dispatcher:
//!   `addPage(null)` adds at the end while `addPage()` adds after the active page; `pasteStyle(ids, null)`
//!   applies nothing while `pasteStyle(ids)` applies the armed style brush.
//! * Callback and `Map` arguments are plain data: `updateSelected(patch, deep?, merge?)` takes one patch for
//!   every element, `updateSelectedMap(patches, deep?, merge?)` (Rust addition) takes `{ [id]: patch | null }`;
//!   `bulkCreate`'s image map is an object `{ value: { id, w, h } }` or a list of `[value, { id, w, h }]` entries.
//!
//! The result is the TS return value as JSON (`null` for `undefined`). Elements are returned as the stored
//! JSON (exactly what TS returns), not re-serialised typed values.
//!
//! Properties: `state`, `doc`, `view`, `version`, `overrides`, `clipboard`, `editingTableText`,
//! `canUndo`, `canRedo` (`store.undo.undoStack/redoStack.length > 0`); assignment `user=` (TS field
//! writes, no notification). Store members: `store.checkpoint` (alias `checkpoint`), `store.apply(ops, origin?)` with
//! an origin name (`local`, `agent`, `template`, `migration`, `repair`, `restore`, `remote`, …), `store.actor` /
//! `store.actor=`, and `flush` (react to host writes made directly on the yrs document). Not JSON-callable, so
//! exposed as plain methods instead: `subscribe`, the text service, `apply_remote_update`, `drain_local_updates`,
//! `reload`, `store().ydoc()`.

use std::collections::HashMap;

use indexmap::IndexMap;
use oc_crdt::{DocOp, OcOrigin};
use oc_geometry::{AlignEdge, Axis, world_matrix};
use oc_schema::elements::{Brush, Element, TextEl};
use oc_schema::primitives::{ColorValue, Unit, Vec2};
use oc_schema::table::{BorderPreset, CellPos, StructureAction, TableStylePreset};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::editor::commands::{Arrange, FindOpts, GeometryPatch, PageNumberPosition, ResolvedImage};
use crate::editor::text::CaretKey;
use crate::editor::timeline::MediaAsset;
use crate::editor::{Editor, el_id};
use crate::state::*;
use crate::styles::FontSet;
use crate::util::{jv, num};
use crate::viewport::Viewport;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DispatchError {
    #[error("unknown method {0}")]
    Unknown(String),
    #[error("bad arguments for {0}: {1}")]
    Args(String, String),
}

type R = Result<Value, DispatchError>;

/// Positional arguments of one call.
struct Args<'a> {
    method: &'a str,
    list: &'a [Value],
}

impl<'a> Args<'a> {
    fn err(&self, msg: impl Into<String>) -> DispatchError {
        DispatchError::Args(self.method.to_string(), msg.into())
    }

    /// Present and not `null` (TS: neither `undefined` nor `null`).
    fn get(&self, i: usize) -> Option<&'a Value> {
        self.list.get(i).filter(|v| !v.is_null())
    }

    /// Present, possibly `null` (TS: not `undefined`).
    fn given(&self, i: usize) -> bool {
        i < self.list.len()
    }

    fn req(&self, i: usize) -> Result<&'a Value, DispatchError> {
        self.get(i).ok_or_else(|| self.err(format!("argument {i} is required")))
    }

    fn str(&self, i: usize) -> Result<&'a str, DispatchError> {
        self.req(i)?.as_str().ok_or_else(|| self.err(format!("argument {i} must be a string")))
    }

    fn opt_str(&self, i: usize) -> Result<Option<&'a str>, DispatchError> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => v.as_str().map(Some).ok_or_else(|| self.err(format!("argument {i} must be a string"))),
        }
    }

    fn f64(&self, i: usize) -> Result<f64, DispatchError> {
        self.req(i)?.as_f64().ok_or_else(|| self.err(format!("argument {i} must be a number")))
    }

    fn opt_f64(&self, i: usize) -> Result<Option<f64>, DispatchError> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => v.as_f64().map(Some).ok_or_else(|| self.err(format!("argument {i} must be a number"))),
        }
    }

    fn i64(&self, i: usize) -> Result<i64, DispatchError> {
        let v = self.f64(i)?;
        Ok(v as i64)
    }

    /// JS truthiness of an optional flag (`undefined` → `default`).
    fn flag(&self, i: usize, default: bool) -> bool {
        match self.list.get(i) {
            None => default,
            Some(v) => crate::util::truthy(Some(v)),
        }
    }

    fn parse<T: DeserializeOwned>(&self, i: usize) -> Result<T, DispatchError> {
        serde_json::from_value(self.req(i)?.clone()).map_err(|e| self.err(format!("argument {i}: {e}")))
    }

    fn opt_parse<T: DeserializeOwned>(&self, i: usize) -> Result<Option<T>, DispatchError> {
        match self.get(i) {
            None => Ok(None),
            Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| self.err(format!("argument {i}: {e}"))),
        }
    }

    fn obj(&self, i: usize) -> Result<&'a Map<String, Value>, DispatchError> {
        self.req(i)?.as_object().ok_or_else(|| self.err(format!("argument {i} must be an object")))
    }

    /// An options object (`undefined` → `{}`).
    fn opts(&self, i: usize) -> Result<Map<String, Value>, DispatchError> {
        match self.get(i) {
            None => Ok(Map::new()),
            Some(Value::Object(m)) => Ok(m.clone()),
            Some(_) => Err(self.err(format!("argument {i} must be an object"))),
        }
    }

    fn vec2(&self, i: usize) -> Result<Vec2, DispatchError> {
        self.parse(i)
    }

    fn opt_vec2(&self, i: usize) -> Result<Option<Vec2>, DispatchError> {
        self.opt_parse(i)
    }

    fn strings(&self, i: usize) -> Result<Vec<String>, DispatchError> {
        self.parse(i)
    }

    fn opt_strings(&self, i: usize) -> Result<Option<Vec<String>>, DispatchError> {
        self.opt_parse(i)
    }

    fn mods(&self, i: usize) -> Result<Mods, DispatchError> {
        Ok(self.opt_parse(i)?.unwrap_or_default())
    }
}

fn opt<T: serde::Serialize>(v: Option<T>) -> Value {
    v.map_or(Value::Null, |x| jv(&x))
}

fn unit() -> R {
    Ok(Value::Null)
}

fn field<T: DeserializeOwned>(a: &Args, m: &Map<String, Value>, k: &str) -> Result<Option<T>, DispatchError> {
    match m.get(k).filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| a.err(format!("{k}: {e}"))),
    }
}

impl Editor {
    /// The stored JSON of elements (from the view when `view`, else the document), in order, skipping missing.
    fn raw_elements<'e>(&self, view: bool, els: impl IntoIterator<Item = &'e Element>) -> Value {
        let d = if view { self.view() } else { self.doc() };
        Value::Array(
            els.into_iter()
                .filter_map(|e| d.raw_el(e.id().unwrap_or("")).map(|m| Value::Object(m.clone())))
                .collect(),
        )
    }

    /// A text element as TS holds it: the stored JSON for real elements, the virtual element for cells.
    fn text_el_json(&self, el: &TextEl) -> Value {
        match self.view().raw_el(&el.base.id) {
            Some(m) => Value::Object(m.clone()),
            None => jv(&Element::Text(el.clone())),
        }
    }

    /// Call an editor member by its TS name with JSON arguments (see the module docs).
    pub fn dispatch(&mut self, method: &str, args: &Value) -> Result<Value, DispatchError> {
        let empty: Vec<Value> = Vec::new();
        let list = match args {
            Value::Array(a) => a.as_slice(),
            Value::Null => empty.as_slice(),
            _ => return Err(DispatchError::Args(method.to_string(), "arguments must be an array".into())),
        };
        let a = Args { method, list };
        match method {
            // ---------------------------------------------------------------- properties
            "state" => Ok(jv(&self.state)),
            "doc" => Ok(self.doc_json()),
            "view" => Ok(self.view_json()),
            "version" => Ok(num(self.version() as f64)),
            "overrides" => Ok(Value::Object(self.overrides().iter().map(|(k, v)| (k.clone(), Value::Object(v.clone()))).collect())),
            "clipboard" => Ok(opt(self.clipboard().cloned())),
            "editingTableText" => Ok(Value::Bool(self.editing_table_text())),
            "canUndo" => Ok(Value::Bool(self.can_undo())),
            "canRedo" => Ok(Value::Bool(self.can_redo())),
            "checkpoint" | "store.checkpoint" => {
                self.store_mut().checkpoint();
                unit()
            }
            "store.apply" => {
                let ops: Vec<DocOp> = a.parse(0)?;
                let origin = match a.opt_str(1)? {
                    None | Some("local") => OcOrigin::Local,
                    Some(name) => OcOrigin::ALL
                        .into_iter()
                        .find(|o| o.tag().strip_prefix("oc:") == Some(name))
                        .ok_or_else(|| a.err(format!("unknown origin {name}")))?,
                };
                Ok(jv(&self.store_apply_with(&ops, origin)))
            }
            "store.actor" => Ok(Value::String(self.store().actor.clone())),
            "store.actor=" => {
                self.store_mut().actor = a.str(0)?.to_string();
                unit()
            }
            "flush" => {
                self.flush();
                unit()
            }

            // ---------------------------------------------------------------- plumbing
            "apply" => {
                let ops: Vec<DocOp> = a.parse(0)?;
                Ok(jv(&self.apply(ops, a.flag(1, false))))
            }
            "editShapeLabel" => {
                self.edit_shape_label(a.str(0)?);
                unit()
            }
            "remeasureText" => {
                self.remeasure_text();
                unit()
            }
            "pageOrigin" => Ok(jv(&self.page_origin(a.str(0)?))),
            "toPage" => Ok(jv(&self.to_page(a.str(0)?, a.vec2(1)?))),
            "selected" => {
                let sel = self.selected();
                Ok(self.raw_elements(true, &sel))
            }
            "selectedDeep" => {
                // The selected elements come from the view, their groups' descendants from the document
                // (TS: `descendants(this.doc, …)`).
                let (v, d) = (self.view(), self.doc());
                let mut out = Vec::new();
                for el in self.selected() {
                    let id = el_id(&el);
                    out.extend(v.raw_el(&id).map(|m| Value::Object(m.clone())));
                    if matches!(el, Element::Group(_)) {
                        for c in oc_schema::tree::descendants(d.els(), &id) {
                            out.extend(d.raw_el(c.id().unwrap_or("")).map(|m| Value::Object(m.clone())));
                        }
                    }
                }
                Ok(Value::Array(out))
            }
            "selectionBox" => Ok(opt(self.selection_box())),
            "selectionFrame" => {
                let sel = self.selected();
                if sel.len() != 1 {
                    return Ok(Value::Null);
                }
                let v = self.view();
                let id = el_id(&sel[0]);
                let xf = v.raw_el(&id).and_then(|m| m.get("xf")).cloned().unwrap_or(Value::Null);
                let mut out = Map::new();
                out.insert("xf".into(), xf);
                out.insert("matrix".into(), jv(&world_matrix(v.els(), &id)));
                Ok(Value::Object(out))
            }

            // ---------------------------------------------------------------- selection
            "select" => {
                let ids = a.strings(0)?;
                let o = a.opts(1)?;
                self.select(&ids, crate::util::truthy(o.get("keepText")));
                unit()
            }
            "clearSelection" => {
                self.clear_selection();
                unit()
            }
            "selectAll" => {
                self.select_all();
                unit()
            }
            "setActivePage" => {
                self.set_active_page(a.str(0)?, a.flag(1, false));
                unit()
            }
            "setHover" => {
                self.set_hover(a.opt_str(0)?.map(str::to_string));
                unit()
            }

            // ---------------------------------------------------------------- viewport
            "setViewportSize" => {
                self.set_viewport_size(a.f64(0)?, a.f64(1)?);
                unit()
            }
            "setViewport" => {
                let vp: Viewport = a.parse(0)?;
                self.set_viewport(vp);
                unit()
            }
            "zoomTo" => {
                self.zoom_to(a.f64(0)?, a.opt_vec2(1)?);
                unit()
            }
            "zoomBy" => {
                self.zoom_by(a.f64(0)?, a.opt_vec2(1)?);
                unit()
            }
            "panBy" => {
                self.pan_by(a.f64(0)?, a.f64(1)?);
                unit()
            }
            "zoomToFit" => {
                self.zoom_to_fit();
                unit()
            }
            "scrollToPage" => {
                self.scroll_to_page(a.str(0)?);
                unit()
            }
            "updateActivePageFromScroll" => {
                self.update_active_page_from_scroll();
                unit()
            }

            // ---------------------------------------------------------------- pointer
            "pointerDown" => {
                let target: HitTarget = a.opt_parse(2)?.unwrap_or_default();
                let button = a.opt_f64(3)?.unwrap_or(0.0);
                if button != 0.0 {
                    return unit();
                }
                self.pointer_down(a.vec2(0)?, a.mods(1)?, target, 0);
                unit()
            }
            "pointerMove" => {
                self.pointer_move(a.vec2(0)?, a.mods(1)?, a.opt_f64(2)?.unwrap_or(0.5));
                unit()
            }
            "pointerUp" => {
                self.pointer_up();
                unit()
            }
            "cancelGesture" => {
                self.cancel_gesture();
                unit()
            }
            "doubleClick" => {
                self.double_click(a.vec2(0)?);
                unit()
            }

            "viewCenter" => Ok(jv(&self.view_center(a.opt_str(0)?))),
            "zoomToElement" => {
                self.zoom_to_element(a.str(0)?, a.opt_f64(1)?.unwrap_or(48.0));
                unit()
            }

            // ---------------------------------------------------------------- freehand draw
            "setTool" => {
                let tool: Tool = a.parse(0)?;
                self.set_tool(tool);
                unit()
            }
            "setBrush" => {
                let o = a.opts(0)?;
                let kind: Option<Brush> = field(&a, &o, "kind")?;
                let color: Option<String> = field(&a, &o, "color")?;
                let size: Option<f64> = field(&a, &o, "size")?;
                self.set_brush(kind, color, size);
                unit()
            }

            // ---------------------------------------------------------------- tables
            "startCellEdit" => {
                self.start_cell_edit(a.str(0)?, a.i64(1)?, a.i64(2)?, a.opt_vec2(3)?, a.flag(4, false));
                unit()
            }
            "stopCellEdit" => {
                self.stop_cell_edit();
                unit()
            }
            "selectCellRange" => {
                let from: CellPos = a.parse(1)?;
                let to: CellPos = a.parse(2)?;
                self.select_cell_range(a.str(0)?, from, to);
                unit()
            }
            "cellTab" => {
                self.cell_tab(if a.f64(0)? < 0.0 { -1 } else { 1 });
                unit()
            }
            "cellTextEl" => {
                let v = self.cell_text_el(a.str(0)?, a.i64(1)?, a.i64(2)?, None);
                Ok(match v {
                    None => Value::Null,
                    Some(v) => {
                        let mut m = Map::new();
                        m.insert("el".into(), jv(&Element::Text(v.el)));
                        m.insert("box".into(), jv(&v.box_));
                        m.insert("dy".into(), num(v.dy));
                        Value::Object(m)
                    }
                })
            }
            "editingTextFrame" => Ok(match self.editing_text_frame() {
                None => Value::Null,
                Some(f) => {
                    let mut m = Map::new();
                    let el = if f.host_id == f.el.base.id { self.text_el_json(&f.el) } else { jv(&Element::Text(f.el.clone())) };
                    m.insert("el".into(), el);
                    m.insert("hostId".into(), Value::String(f.host_id));
                    m.insert("ox".into(), num(f.ox));
                    m.insert("oy".into(), num(f.oy));
                    Value::Object(m)
                }
            }),
            "cellAt" => Ok(opt(self.cell_at(a.str(0)?, a.vec2(1)?))),
            "setCell" => {
                self.set_cell(a.str(0)?, a.i64(1)?, a.i64(2)?, a.obj(3)?);
                unit()
            }
            "tableTargetRange" => Ok(opt(self.table_target_range(a.str(0)?))),
            "tableTargets" => Ok(jv(&self.table_targets(a.str(0)?))),
            "setCells" => {
                self.set_cells(a.str(0)?, a.obj(1)?);
                unit()
            }
            "setCellAlign" => {
                self.set_cell_align(a.str(0)?, a.str(1)?);
                unit()
            }
            "setCellBorders" => {
                let preset: BorderPreset = a.parse(1)?;
                let b = a.opts(2)?;
                let width: Option<f64> = field(&a, &b, "width")?;
                let color: Option<ColorValue> = field(&a, &b, "color")?;
                self.set_cell_borders(a.str(0)?, preset, width, color);
                unit()
            }
            "mergeCellRange" => {
                self.merge_cell_range(a.str(0)?);
                unit()
            }
            "unmergeCellRange" => {
                self.unmerge_cell_range(a.str(0)?);
                unit()
            }
            "clearCells" => {
                self.clear_cells(a.str(0)?);
                unit()
            }
            "setTableStyle" => {
                let p: TableStylePreset = a.parse(1)?;
                self.set_table_style(a.str(0)?, p);
                unit()
            }
            "setColumnWidth" => {
                let index = match a.req(1)? {
                    Value::String(s) if s == "even" => None,
                    v => Some(v.as_f64().ok_or_else(|| a.err("index must be a number or 'even'"))? as i64),
                };
                self.set_column_width(a.str(0)?, index, a.opt_f64(2)?);
                unit()
            }
            "tableStructure" => {
                let action: StructureAction = a.parse(1)?;
                self.table_structure(a.str(0)?, action, a.i64(2)?);
                unit()
            }

            // ---------------------------------------------------------------- animation & timeline
            "previewAnimation" => {
                let p = a.opt_str(0)?.map(str::to_string);
                self.preview_animation(p.as_deref());
                unit()
            }
            "stopPreview" => {
                self.stop_preview();
                unit()
            }
            "currentTime" => Ok(num(self.current_time(a.opt_f64(0)?))),
            "play" => {
                self.play();
                unit()
            }
            "pause" => {
                self.pause();
                unit()
            }
            "togglePlay" => {
                self.toggle_play();
                unit()
            }
            "seek" => {
                self.seek(a.opt_f64(0)?);
                unit()
            }
            "tick" => {
                self.tick();
                unit()
            }
            "pageStart" => Ok(num(self.page_start(a.str(0)?))),
            "setPageDuration" => {
                self.set_page_duration(a.str(0)?, a.f64(1)?);
                unit()
            }
            "setTiming" => {
                self.set_timing(a.str(0)?, a.get(1).cloned());
                unit()
            }
            "insertVideo" => {
                let asset: MediaAsset = a.parse(0)?;
                Ok(jv(&self.insert_video(&asset, a.opt_vec2(1)?)))
            }
            "splitAtPlayhead" => {
                self.split_at_playhead();
                unit()
            }
            "audioClips" => Ok(jv(&self.audio_clips())),
            "addAudio" => {
                let asset: MediaAsset = a.parse(0)?;
                Ok(Value::String(self.add_audio(&asset, a.opt_f64(1)?)))
            }
            "updateAudio" => {
                self.update_audio(a.str(0)?, a.obj(1)?, a.flag(2, true));
                unit()
            }
            "removeAudio" => {
                self.remove_audio(a.str(0)?);
                unit()
            }
            "splitAudio" => {
                self.split_audio(a.str(0)?, a.opt_f64(1)?);
                unit()
            }
            "captions" => Ok(Value::Object(self.captions())),
            "setCaptions" => {
                self.set_captions(a.obj(0)?, a.flag(1, false));
                unit()
            }
            "upsertCue" => Ok(Value::String(self.upsert_cue(a.obj(0)?))),

            // ---------------------------------------------------------------- keyframes
            "setRecordKeyframes" => {
                self.set_record_keyframes(a.flag(0, false));
                unit()
            }
            "keyframeTime" => Ok(opt(self.keyframe_time(a.str(0)?))),
            "setKeyframe" => {
                let ease = a.opt_str(4)?.map(str::to_string);
                self.set_keyframe(a.str(0)?, a.str(1)?, a.f64(2)?, a.f64(3)?, ease.as_deref());
                unit()
            }
            "removeKeyframe" => {
                self.remove_keyframe(a.str(0)?, a.str(1)?, a.f64(2)?);
                unit()
            }
            "moveKeyframe" => {
                let prop = a.opt_str(3)?.map(str::to_string);
                self.move_keyframe(a.str(0)?, a.f64(1)?, a.f64(2)?, prop.as_deref());
                unit()
            }
            "clearKeyframes" => {
                self.clear_keyframes(a.str(0)?);
                unit()
            }

            // ---------------------------------------------------------------- text editing
            "startTextEdit" => {
                self.start_text_edit(a.str(0)?, a.opt_vec2(1)?, a.flag(2, false));
                unit()
            }
            "stopTextEdit" => {
                self.stop_text_edit();
                unit()
            }
            "typeText" => {
                self.type_text(a.str(0)?);
                unit()
            }
            "deleteText" => {
                self.delete_text(if a.f64(0)? < 0.0 { -1 } else { 1 }, a.flag(1, false));
                unit()
            }
            "moveCaret" => {
                let key: CaretKey = a.parse(0)?;
                self.move_caret(key, a.flag(1, false), a.flag(2, false));
                unit()
            }
            "plainText" => {
                let el: Element = a.parse(0)?;
                match el {
                    Element::Text(t) => Ok(Value::String(self.plain_text(&t))),
                    _ => Err(a.err("argument 0 must be a text element")),
                }
            }
            "selectedText" => Ok(Value::String(self.selected_text())),

            // ---------------------------------------------------------------- styling
            "setTextStyle" => {
                self.set_text_style(a.obj(0)?);
                unit()
            }
            "setParagraphStyle" => {
                self.set_paragraph_style(a.obj(0)?);
                unit()
            }
            "currentTextStyle" => Ok(opt(self.current_text_style())),
            "currentParagraph" => Ok(opt(self.current_paragraph())),
            "updateSelected" => {
                self.update_selected(a.obj(0)?, a.flag(1, false), a.flag(2, true));
                unit()
            }
            "updateSelectedMap" => {
                self.update_selected_map(a.obj(0)?, a.flag(1, false), a.flag(2, true));
                unit()
            }

            // ---------------------------------------------------------------- element commands
            "addPageNumbers" => {
                let o = a.opts(0)?;
                let text: Option<String> = field(&a, &o, "text")?;
                let position: Option<PageNumberPosition> = field(&a, &o, "position")?;
                let size: Option<f64> = field(&a, &o, "size")?;
                Ok(num(self.add_page_numbers(text.as_deref(), position, size) as f64))
            }
            "insert" => {
                let els: Vec<Map<String, Value>> = a.parse(0)?;
                let o = a.opts(2)?;
                let select = o.get("select") != Some(&Value::Bool(false));
                let edit = crate::util::truthy(o.get("edit"));
                Ok(jv(&self.insert(els, a.opt_vec2(1)?, select, edit)))
            }
            "deleteSelection" => {
                self.delete_selection();
                unit()
            }
            "duplicate" => {
                self.duplicate();
                unit()
            }
            "copy" => Ok(opt(self.copy())),
            "cut" => Ok(opt(self.cut())),
            "paste" => {
                let payload: Option<ClipboardPayload> = a.opt_parse(0)?;
                let offset = a.opt_f64(1)?.unwrap_or(20.0);
                if a.given(0) && payload.is_none() {
                    // `paste(null)`: nothing to paste.
                    return unit();
                }
                self.paste(payload.as_ref(), offset);
                unit()
            }
            "nudge" => {
                self.nudge(a.f64(0)?, a.f64(1)?);
                unit()
            }
            "arrange" => {
                let w: Arrange = a.parse(0)?;
                self.arrange(w);
                unit()
            }
            "reorderTo" => {
                self.reorder_to(a.str(0)?, a.str(1)?);
                unit()
            }
            "align" => {
                let e: AlignEdge = a.parse(0)?;
                self.align(e);
                unit()
            }
            "distribute" => {
                let x: Axis = a.parse(0)?;
                self.distribute(x);
                unit()
            }
            "tidy" => {
                self.tidy();
                unit()
            }
            "flip" => {
                let x: Axis = a.parse(0)?;
                self.flip(x);
                unit()
            }
            "toggleLock" => {
                self.toggle_lock();
                unit()
            }
            "group" => {
                self.group();
                unit()
            }
            // Vector editing (DS-100) and masks (DS-107).
            "penFinish" => Ok(opt(self.pen_finish(a.flag(0, false)))),
            "penCancel" => {
                self.pen_cancel();
                unit()
            }
            "startPathEdit" => Ok(Value::Bool(self.start_path_edit(a.str(0)?))),
            "stopPathEdit" => {
                self.stop_path_edit();
                unit()
            }
            "selectPathNodes" => {
                self.select_path_nodes(a.parse(0)?);
                unit()
            }
            "pathEditView" => Ok(opt(self.path_edit_view())),
            "nudgePathNodes" => Ok(Value::Bool(self.nudge_path_nodes(a.f64(0)?, a.f64(1)?))),
            "setPathNodesSmooth" => Ok(Value::Bool(self.set_path_nodes_smooth(a.opt_parse(0)?))),
            "deletePathNodes" => Ok(Value::Bool(self.delete_path_nodes())),
            "addPathNode" => Ok(Value::Bool(self.add_path_node())),
            "booleanOp" => Ok(opt(self.boolean_op(a.parse(0)?))),
            "replaceWithPath" => Ok(opt(self.replace_with_path(a.str(0)?, a.str(1)?, a.opt_parse(2)?))),
            "replaceWithElements" => Ok(opt(self.replace_with_elements(a.str(0)?, a.parse(1)?))),
            "maskSelection" => Ok(opt(self.mask_selection(a.parse(0)?))),
            "maskedGroup" => Ok(opt(self.masked_group())),
            "setMaskMode" => Ok(Value::Bool(self.set_mask_mode(a.parse(0)?))),
            "releaseMask" => Ok(Value::Bool(self.release_mask())),
            "ungroup" => {
                self.ungroup();
                unit()
            }
            "startCrop" => {
                self.start_crop(a.str(0)?);
                unit()
            }
            "stopCrop" => {
                self.stop_crop();
                unit()
            }
            "setCropZoom" => {
                self.set_crop_zoom(a.str(0)?, a.f64(1)?);
                unit()
            }
            "cropZoom" => Ok(num(self.crop_zoom(a.str(0)?))),
            "mediaCrop" => {
                // The TS takes the element; an id resolves to the element in the current view.
                let el: Option<Element> = match a.get(0) {
                    None | Some(Value::Null) => None,
                    Some(Value::String(id)) => self.view().el(id).cloned(),
                    Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| a.err(e.to_string()))?),
                };
                Ok(el.and_then(|el| self.media_crop(&el)).map_or(Value::Null, |c| jv(&c)))
            }
            "replaceImage" => {
                self.replace_image(a.str(0)?, a.str(1)?, a.f64(2)?);
                unit()
            }

            // ---------------------------------------------------------------- copy style
            "copyStyle" => Ok(opt(self.copy_style())),
            "cancelStyleBrush" => {
                self.cancel_style_brush();
                unit()
            }
            "pasteStyle" => {
                let ids = a.opt_strings(0)?.unwrap_or_else(|| self.state.selection.clone());
                let style: Option<CopiedStyle> = if a.given(1) { a.opt_parse(1)? } else { self.state.style_brush.clone() };
                self.paste_style(&ids, style.as_ref());
                unit()
            }

            // ---------------------------------------------------------------- find & replace
            "find" => {
                let o: FindOpts = a.opt_parse(1)?.unwrap_or_default();
                Ok(jv(&self.find(a.str(0)?, &o)))
            }
            "revealMatch" => {
                let m: FindMatch = a.parse(0)?;
                self.reveal_match(&m);
                unit()
            }
            "replace" => {
                let o: FindOpts = a.opt_parse(2)?.unwrap_or_default();
                Ok(num(self.replace(a.str(0)?, a.str(1)?, &o) as f64))
            }

            // ---------------------------------------------------------------- geometry & guides
            "setGeometry" => {
                let g: GeometryPatch = a.opt_parse(0)?.unwrap_or_default();
                self.set_geometry(g);
                unit()
            }
            "setGuides" => {
                let list: Vec<Map<String, Value>> = a.parse(1)?;
                let guides: Vec<(String, f64)> = list
                    .iter()
                    .map(|g| {
                        (crate::util::s(g.get("axis")).unwrap_or("").to_string(), crate::util::f(g.get("pos")).unwrap_or(f64::NAN))
                    })
                    .collect();
                self.set_guides(a.str(0)?, &guides);
                unit()
            }
            "addGuide" => {
                self.add_guide(a.str(0)?, a.str(1)?, a.f64(2)?);
                unit()
            }

            // ---------------------------------------------------------------- grids & frames
            "gridCellAt" => Ok(num(self.grid_cell_at(a.str(0)?, a.vec2(1)?) as f64)),
            "setGridCell" => {
                let asset = a.opt_str(2)?.map(str::to_string);
                self.set_grid_cell(a.str(0)?, a.i64(1)?, asset.as_deref());
                unit()
            }
            "fillFrame" => {
                self.fill_frame(a.str(0)?, a.str(1)?, a.opt_f64(2)?);
                unit()
            }
            "dropIntoContainer" => Ok(Value::Bool(self.drop_into_container(a.str(0)?, a.vec2(1)?, a.str(2)?))),

            // ---------------------------------------------------------------- bulk create
            "bindableElements" => {
                let p = a.opt_str(0)?.map(str::to_string);
                let els = self.bindable_elements(p.as_deref());
                Ok(self.raw_elements(false, &els))
            }
            "setDataBinding" => {
                let c = a.opt_str(1)?.map(str::to_string);
                self.set_data_binding(a.str(0)?, c.as_deref());
                unit()
            }
            "bulkCreate" => {
                let rows: Vec<IndexMap<String, String>> = a.parse(1)?;
                let images: HashMap<String, ResolvedImage> = match a.get(2) {
                    None => HashMap::new(),
                    Some(Value::Array(entries)) => {
                        let mut m = HashMap::new();
                        for e in entries {
                            let (k, v): (String, ResolvedImage) = serde_json::from_value(e.clone()).map_err(|e| a.err(e.to_string()))?;
                            m.insert(k, v);
                        }
                        m
                    }
                    Some(v) => serde_json::from_value(v.clone()).map_err(|e| a.err(e.to_string()))?,
                };
                Ok(jv(&self.bulk_create(a.str(0)?, &rows, &images)))
            }

            // ---------------------------------------------------------------- pages
            "addPage" => {
                let id = if a.given(0) {
                    let after = a.opt_str(0)?.map(str::to_string);
                    self.add_page(after.as_deref())
                } else {
                    self.add_page_after_active()
                };
                Ok(opt(id))
            }
            "duplicatePage" => {
                let p = a.opt_str(0)?.map(str::to_string);
                self.duplicate_page(p.as_deref());
                unit()
            }
            "deletePage" => {
                let p = a.opt_str(0)?.map(str::to_string);
                self.delete_page(p.as_deref());
                unit()
            }
            "movePage" => {
                self.move_page(a.str(0)?, a.i64(1)?);
                unit()
            }
            "updatePage" => {
                self.update_page(a.str(0)?, a.obj(1)?);
                unit()
            }

            // ---------------------------------------------------------------- design styles
            "replaceColors" => {
                let m: IndexMap<String, String> = a.parse(0)?;
                Ok(num(self.replace_colors(&m) as f64))
            }
            "applyPalette" => {
                let c: Vec<String> = a.parse(0)?;
                Ok(num(self.apply_palette(&c) as f64))
            }
            "applyFonts" => {
                let s: FontSet = a.parse(0)?;
                Ok(num(self.apply_fonts(&s) as f64))
            }
            "setLanguage" => {
                self.set_language(a.str(0)?);
                unit()
            }
            "resizeDesign" => {
                let u: Option<Unit> = a.opt_parse(2)?;
                self.resize_design(a.f64(0)?, a.f64(1)?, u, a.flag(3, false));
                unit()
            }

            // ---------------------------------------------------------------- history, present, keys
            "undo" => {
                self.undo();
                unit()
            }
            "redo" => {
                self.redo();
                unit()
            }
            "setPresenting" => {
                self.set_presenting(a.flag(0, false));
                unit()
            }
            "textShortcut" => {
                let code = a.opt_str(2)?.map(str::to_string);
                Ok(Value::Bool(self.text_shortcut(a.str(0)?, a.mods(1)?, code.as_deref())))
            }
            "handleKey" => {
                let code = a.opt_str(2)?.map(str::to_string);
                Ok(Value::Bool(self.handle_key(a.str(0)?, a.mods(1)?, code.as_deref())))
            }
            _ => Err(DispatchError::Unknown(method.to_string())),
        }
    }
}
