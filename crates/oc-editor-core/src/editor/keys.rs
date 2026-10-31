// SPDX-License-Identifier: Apache-2.0
//! Keyboard commands (docs/plan/04 §3 shortcut map). Keys are named like `KeyboardEvent.key` (`"ArrowLeft"`,
//! `"z"`, `"Enter"`) and `code` like `KeyboardEvent.code` (`"Digit7"`); the host does the DOM event plumbing.

use oc_geometry::Axis;
use oc_schema::design::Surface;
use oc_schema::elements::{Element, ShapeKind};
use oc_schema::factory::{LineOpts, ShapeOpts, TextOpts, make_line, make_shape, make_text};
use oc_schema::primitives::Xf;
use oc_schema::text::RunStyleOverride;
use oc_schema::tree::{children_of, sorted_pages};
use serde_json::Value;

use super::commands::Arrange;
use super::{Editor, el_id, one};
use crate::state::*;
use crate::util::{f, jm, max, num, round, s};

fn lower_key(key: &str) -> String {
    if crate::util::len16(key) == 1 { key.to_lowercase() } else { key.to_string() }
}

/// The character a physical key produces without modifiers on a US layout (`KeyboardEvent.code` → key).
fn code_key(code: &str) -> Option<String> {
    if let Some(c) = code.strip_prefix("Key").filter(|c| c.len() == 1) {
        return Some(c.to_ascii_lowercase());
    }
    if let Some(d) = code.strip_prefix("Digit").filter(|d| d.len() == 1) {
        return Some(d.to_string());
    }
    let k = match code {
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Minus" => "-",
        "Equal" => "=",
        "Comma" => ",",
        "Period" => ".",
        _ => return None,
    };
    Some(k.to_string())
}

/// The key a shortcut matches on. Option on a Mac (and many non-US layouts) changes `key` (⌥⌘[ is `"“"`,
/// ⌥⌘L is `"¬"`), so when `key` isn't a plain ASCII character we fall back to the physical key in `code`.
fn shortcut_key(key: &str, code: Option<&str>) -> String {
    let k = lower_key(key);
    let odd = k == "Dead" || k == "Unidentified" || (crate::util::len16(&k) == 1 && !k.is_ascii());
    if odd && let Some(c) = code.and_then(code_key) {
        return c;
    }
    k
}

fn bool_val(v: Option<&Value>) -> bool {
    crate::util::truthy(v)
}

impl Editor {
    /// Text shortcuts (Canva set): ⌘B/I/U, ⇧⌘> / ⇧⌘< size, ⇧⌘L/C/R/J alignment, ⇧⌘K uppercase, ⇧⌘7 / ⇧⌘8
    /// numbered / bulleted list. Work while editing text and on selected text boxes.
    pub fn text_shortcut(&mut self, key: &str, mods: Mods, code: Option<&str>) -> bool {
        let cmd = mods.meta || mods.ctrl;
        if !cmd || mods.alt {
            return false;
        }
        let Some(st) = self.current_text_style() else { return false };
        let k = shortcut_key(key, code);
        let k = k.as_str();
        let size = f(st.get("size")).unwrap_or(0.0);
        if !mods.shift {
            let marks = match k {
                "b" => one("weight", num(if f(st.get("weight")).unwrap_or(0.0) >= 600.0 { 400.0 } else { 700.0 })),
                "i" => one("italic", Value::Bool(!bool_val(st.get("italic")))),
                "u" => one("underline", Value::Bool(!bool_val(st.get("underline")))),
                _ => return false,
            };
            self.set_text_style(&marks);
            return true;
        }
        if k == ">" || k == "." || code == Some("Period") {
            self.set_text_style(&one("size", num(round(if size < 20.0 { size + 1.0 } else { size * 1.1 }))));
            return true;
        }
        if k == "<" || k == "," || code == Some("Comma") {
            self.set_text_style(&one("size", num(max(6.0, round(if size < 20.0 { size - 1.0 } else { size / 1.1 })))));
            return true;
        }
        let align = match k {
            "l" => Some("start"),
            "c" => Some("center"),
            "r" => Some("end"),
            "j" => Some("justify"),
            _ => None,
        };
        if let Some(a) = align {
            self.set_paragraph_style(&one("align", Value::String(a.into())));
            return true;
        }
        if k == "k" {
            let next = if s(st.get("caseTransform")) == Some("upper") { "none" } else { "upper" };
            self.set_text_style(&one("caseTransform", Value::String(next.into())));
            return true;
        }
        if code == Some("Digit7") || code == Some("Digit8") || matches!(k, "7" | "8" | "&" | "*") {
            let kind = if code == Some("Digit7") || k == "7" || k == "&" { "number" } else { "bullet" };
            let cur = self.current_paragraph().and_then(|p| p.get("list").and_then(|l| s(l.get("kind")).map(str::to_string)));
            let list = if cur.as_deref() == Some(kind) {
                Value::Null
            } else {
                let mut m = one("kind", Value::String(kind.into()));
                m.insert("level".into(), num(0.0));
                Value::Object(m)
            };
            self.set_paragraph_style(&one("list", list));
            return true;
        }
        false
    }

    /// Returns true when the key was handled. Text-editing keys are handled by the host's IME proxy.
    pub fn handle_key(&mut self, key: &str, mods: Mods, code: Option<&str>) -> bool {
        let cmd = mods.meta || mods.ctrl;
        let k = shortcut_key(key, code);
        let k = k.as_str();
        if self.text_shortcut(key, mods, code) {
            return true;
        }
        if cmd && (k == "Backspace" || k == "Delete") && self.state.selection.is_empty() && self.state.editing_text.is_none() {
            // ⌘⌫ with nothing selected deletes the current page (Canva), keeping at least one.
            if sorted_pages(&self.doc().doc.pages).len() > 1 {
                let p = self.state.active_page_id.clone();
                self.delete_page(Some(&p));
                return true;
            }
            return false;
        }
        if let Some(e) = self.state.editing_text.clone() {
            if k == "Escape" {
                self.set(|s| {
                    s.editing_text = None;
                    s.selection = vec![e.id.clone()];
                });
                return true;
            }
            return false;
        }
        if !cmd {
            if self.state.tool == Tool::Pen && self.pen_key(k, mods) {
                return true;
            }
            if self.state.path_edit.is_some() && self.path_key(k, mods) {
                return true;
            }
            if k == "p" && !mods.alt && !mods.shift && !self.state.active_page_id.is_empty() {
                self.set_tool(Tool::Pen);
                return true;
            }
        }
        if cmd && k == "z" {
            if mods.shift { self.redo() } else { self.undo() }
            return true;
        }
        if cmd && k == "y" {
            self.redo();
            return true;
        }
        if cmd && k == "a" {
            self.select_all();
            return true;
        }
        if cmd && k == "d" {
            self.duplicate();
            return true;
        }
        if cmd && mods.alt && k == "c" {
            self.copy_style();
            return true;
        }
        if cmd && mods.alt && k == "v" {
            if self.state.style_brush.is_some() {
                let sel = self.state.selection.clone();
                let st = self.state.style_brush.clone();
                self.paste_style(&sel, st.as_ref());
            }
            return true;
        }
        if cmd && k == "g" {
            if mods.shift { self.ungroup() } else { self.group() }
            return true;
        }
        if cmd && mods.alt && k == "l" {
            self.toggle_lock();
            return true;
        }
        if cmd && (k == "]" || k == "[") {
            let a = if k == "]" {
                if mods.alt { Arrange::Front } else { Arrange::Forward }
            } else if mods.alt {
                Arrange::Back
            } else {
                Arrange::Backward
            };
            self.arrange(a);
            return true;
        }
        if cmd && (k == "=" || k == "+") {
            self.zoom_by(1.25, None);
            return true;
        }
        if cmd && k == "-" {
            self.zoom_by(0.8, None);
            return true;
        }
        if cmd && k == "0" {
            self.zoom_to_fit();
            return true;
        }
        if cmd && k == "Enter" {
            self.add_page_after_active();
            return true;
        }
        if !cmd && mods.alt && k == "h" && mods.shift {
            self.flip(Axis::X);
            return true;
        }
        if !cmd && mods.alt && k == "v" && mods.shift {
            self.flip(Axis::Y);
            return true;
        }
        // Quick-add (Canva-style): T text, R rectangle, C circle, L line — inserted at the page centre.
        if !cmd && !mods.alt && !mods.shift && matches!(k, "t" | "r" | "c" | "l") && !self.state.active_page_id.is_empty() {
            let doc = self.doc();
            let (w, h) = (doc.doc.meta.dims.w, doc.doc.meta.dims.h);
            let sz = round(w.min(h) * 0.25);
            let xf = |bw: f64, bh: f64| Xf::new(0.0, 0.0, bw, bh, 0.0);
            match k {
                "t" => {
                    let base = self.fresh_base(xf(round(w * 0.5), round(sz * 0.35)));
                    let el = make_text(
                        base,
                        TextOpts {
                            text: Some("Add a heading".into()),
                            style: Some(RunStyleOverride { size: Some(max(12.0, round(sz * 0.2))), ..Default::default() }),
                            ..Default::default()
                        },
                    );
                    self.insert(vec![jm(&Element::Text(el))], None, true, true);
                }
                "r" => {
                    let base = self.fresh_base(xf(sz, sz));
                    let el = make_shape(base, ShapeOpts { kind: Some(ShapeKind::Rect), ..Default::default() });
                    self.insert(vec![jm(&Element::Shape(el))], None, true, false);
                }
                "c" => {
                    let base = self.fresh_base(xf(sz, sz));
                    let el = make_shape(base, ShapeOpts { kind: Some(ShapeKind::Ellipse), ..Default::default() });
                    self.insert(vec![jm(&Element::Shape(el))], None, true, false);
                }
                _ => {
                    let base = self.fresh_base(xf(round(sz * 1.6), 24.0));
                    let el = make_line(
                        base,
                        LineOpts { color: oc_schema::color::hex("#1f2328").ok(), width: Some(max(3.0, round(sz / 60.0))), ..Default::default() },
                    );
                    self.insert(vec![jm(&Element::Line(el))], None, true, false);
                }
            }
            return true;
        }
        match k {
            "Delete" | "Backspace" => {
                if let Some(cr) = self.state.cell_range.clone() {
                    self.clear_cells(&cr.id);
                    return true;
                }
                if !self.state.selection.is_empty() {
                    self.delete_selection();
                    return true;
                }
                false
            }
            "Escape" => {
                if self.state.style_brush.is_some() {
                    self.set(|s| s.style_brush = None);
                } else if self.state.tool != Tool::Select {
                    self.set_tool(Tool::Select);
                } else if self.state.editing_cell.is_some() || self.state.cell_range.is_some() {
                    self.stop_cell_edit();
                } else if self.state.cropping_id.is_some() {
                    self.stop_crop();
                } else if let Some(g) = self.state.entered_group.clone() {
                    self.select(&[g], false);
                } else {
                    self.clear_selection();
                }
                true
            }
            "Enter" => {
                let sel = self.selected();
                let doc = self.doc();
                if doc.doc.meta.surface == Surface::Whiteboard
                    && sel.len() == 1
                    && !matches!(sel[0], Element::Text(_))
                    && sel[0].base().and_then(|b| b.board.as_ref()).and_then(|b| b.mind.as_ref()).and_then(|m| m.parent.as_ref()).and_then(|p| p.as_ref()).is_some()
                    && !mods.shift
                    && self.mind_map_add(false).is_some()
                {
                    return true;
                }
                if sel.len() == 1 && matches!(sel[0], Element::Text(_)) {
                    self.start_text_edit(&el_id(&sel[0]), None, false);
                    return true;
                }
                if sel.len() == 1 && matches!(sel[0], Element::Table(_)) {
                    self.start_cell_edit(&el_id(&sel[0]), 0, 0, None, false);
                    return true;
                }
                if self.state.cropping_id.is_some() {
                    self.stop_crop();
                    return true;
                }
                false
            }
            "Tab" => {
                // Whiteboards: Tab on a shape or sticky adds a mind-map child (Shift+Tab keeps cycling the selection).
                if self.doc().doc.meta.surface == Surface::Whiteboard
                    && !mods.shift
                    && self.state.selection.len() == 1
                    && self.mind_map_add(true).is_some()
                {
                    return true;
                }
                let parent = self.state.entered_group.clone().unwrap_or_else(|| self.state.active_page_id.clone());
                let doc = self.doc();
                let kids = children_of(doc.els(), &parent);
                if kids.is_empty() {
                    return false;
                }
                let cur = kids.iter().position(|e| self.state.selection.first().map(String::as_str) == e.id()).map_or(-1, |i| i as i64);
                let n = kids.len() as i64;
                let next = ((cur + if mods.shift { -1 } else { 1 } + n) % n) as usize;
                let id = el_id(kids[next]);
                self.select(&[id], false);
                true
            }
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
                if self.state.selection.is_empty() {
                    return false;
                }
                let step = if mods.shift { 10.0 } else { 1.0 };
                let (dx, dy) = match k {
                    "ArrowLeft" => (-step, 0.0),
                    "ArrowRight" => (step, 0.0),
                    "ArrowUp" => (0.0, -step),
                    _ => (0.0, step),
                };
                self.nudge(dx, dy);
                true
            }
            _ => false,
        }
    }

}
