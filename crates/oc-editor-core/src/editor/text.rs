// SPDX-License-Identifier: Apache-2.0
//! Text editing (caret model over UTF-16 offsets) and styling commands.

use oc_crdt::body::BodyOp;
use oc_crdt::DocOp;
use oc_geometry::{apply, invert, world_matrix};
use oc_schema::elements::{Element, TextEl};
use oc_schema::tree::descendants;
use oc_schema::primitives::Vec2;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{Editor, el_id, locked_content, op_update};
use crate::state::*;
use crate::util::{has_js_space, index_of16, jm, last_index_of16, len16, merge, next_grapheme16, prev_grapheme16, slice16};

/// Text being edited and where its layout origin sits inside the host element (for caret/selection chrome).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditingTextFrame {
    pub el: TextEl,
    pub host_id: String,
    pub ox: f64,
    pub oy: f64,
}

/// Caret movement keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaretKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

pub(crate) fn plain_text(el: &TextEl) -> String {
    el.body.paragraphs.iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

impl Editor {
    pub fn start_text_edit(&mut self, id: &str, page_pt: Option<Vec2>, select_word: bool) {
        let doc = self.doc();
        let Some(Element::Text(el)) = doc.el(id) else { return };
        if locked_content(&Element::Text(el.clone())) {
            return;
        }
        let mut offset = self.text_service.as_ref().map_or(0, |ts| ts.length(el));
        let mut anchor;
        match (page_pt, self.text_service.as_ref()) {
            (Some(pt), Some(ts)) => {
                let [bx, by] = apply(&invert(&world_matrix(doc.els(), id)), pt);
                offset = ts.offset_at(el, bx, by - text_anchor_dy(ts.as_ref(), el));
                anchor = offset;
                if select_word {
                    (anchor, offset) = ts.word_at(el, offset);
                }
            }
            _ => anchor = 0,
        }
        self.state.selection = vec![id.to_string()];
        if let Some(p) = doc.page_of(id) {
            self.state.active_page_id = p;
        }
        self.store_mut().checkpoint();
        let id = id.to_string();
        self.set(|s| s.editing_text = Some(TextEditState { id, anchor, focus: offset }));
    }

    pub fn stop_text_edit(&mut self) {
        if self.state.editing_text.is_some() {
            self.set(|s| s.editing_text = None);
        }
    }

    fn text_range(&self) -> Option<(String, i64, i64)> {
        let e = self.state.editing_text.as_ref()?;
        Some((e.id.clone(), e.anchor.min(e.focus), e.anchor.max(e.focus)))
    }

    pub fn type_text(&mut self, s: &str) {
        // Typing over a cell range starts editing its first cell.
        if let Some(cr) = self.state.cell_range.clone()
            && self.state.editing_text.is_none()
        {
            self.start_cell_edit(&cr.id, cr.r0, cr.c0, None, false);
        }
        let Some((id, from, to)) = self.text_range() else { return };
        let mut ops = Vec::new();
        if to > from {
            ops.push(BodyOp::TextDelete { offset: from as f64, len: (to - from) as f64 });
        }
        let marks = self.pending_marks.is_some().then(|| self.merged_marks(from));
        ops.push(BodyOp::TextInsert { offset: from as f64, text: s.to_string(), marks });
        if self.apply_text_ops(ops, !has_js_space(s)).ok {
            let at = from + len16(s);
            self.set(|st| st.editing_text = Some(TextEditState { id, anchor: at, focus: at }));
        }
    }

    fn merged_marks(&self, offset: i64) -> Map<String, Value> {
        let pending = self.pending_marks.clone().unwrap_or_default();
        let Some(t) = self.edit_target() else { return pending };
        let mut pos = 0;
        let mut base = Map::new();
        'outer: for p in &t.el.body.paragraphs {
            for run in &p.runs {
                let n = len16(&run.text);
                if offset <= pos + n {
                    base = jm(&run.style);
                    break 'outer;
                }
                pos += n;
            }
            pos += 1;
        }
        merge(&base, &pending)
    }

    /// Delete backwards (`dir = -1`) or forwards (`1`), by character or word.
    pub fn delete_text(&mut self, dir: i32, word: bool) {
        if let Some(cr) = self.state.cell_range.clone()
            && self.state.editing_text.is_none()
        {
            self.clear_cells(&cr.id);
            return;
        }
        let (Some((id, mut from, mut to)), Some(t)) = (self.text_range(), self.edit_target()) else { return };
        let Some(ts) = &self.text_service else { return };
        // Steps are whole grapheme clusters: a lone surrogate would make yrs panic, and half a combining
        // sequence or ZWJ emoji is never what the user meant.
        let text = plain_text(&t.el);
        if from == to {
            if word {
                let (a, b) = ts.word_at(&t.el, if dir < 0 { 0.max(from - 1) } else { from });
                if dir < 0 {
                    from = prev_grapheme16(&text, from).min(a);
                } else {
                    to = next_grapheme16(&text, to).max(b);
                }
            } else if dir < 0 {
                from = prev_grapheme16(&text, from);
            } else {
                to = next_grapheme16(&text, to);
            }
        } else {
            // A selection that ends inside a cluster (e.g. from an older client) deletes the whole cluster.
            from = from.min(prev_grapheme16(&text, from + 1));
            to = to.max(next_grapheme16(&text, to - 1));
        }
        from = from.max(0);
        to = to.min(ts.length(&t.el));
        if to <= from {
            return;
        }
        if self.apply_text_ops(vec![BodyOp::TextDelete { offset: from as f64, len: (to - from) as f64 }], true).ok {
            self.set(|s| s.editing_text = Some(TextEditState { id, anchor: from, focus: from }));
        }
    }

    pub fn move_caret(&mut self, key: CaretKey, extend: bool, word: bool) {
        let (Some(e), Some(t)) = (self.state.editing_text.clone(), self.edit_target()) else { return };
        let Some(ts) = &self.text_service else { return };
        let el = &t.el;
        let len = ts.length(el);
        let (from, to) = (e.anchor.min(e.focus), e.anchor.max(e.focus));
        let mut f = e.focus;
        if !extend && from != to && matches!(key, CaretKey::Left | CaretKey::Right) {
            f = if key == CaretKey::Left { from } else { to };
        } else {
            match key {
                CaretKey::Left => f = if word { ts.word_at(el, 0.max(f - 1)).0 } else { prev_grapheme16(&plain_text(el), f) },
                CaretKey::Right => f = if word { ts.word_at(el, f).1 } else { next_grapheme16(&plain_text(el), f) },
                CaretKey::Up => f = ts.vertical_move(el, f, -1),
                CaretKey::Down => f = ts.vertical_move(el, f, 1),
                CaretKey::Home => f = last_index_of16(&plain_text(el), '\n' as u16, f - 1) + 1,
                CaretKey::End => {
                    let i = index_of16(&plain_text(el), '\n' as u16, f);
                    f = if i < 0 { len } else { i };
                }
            }
        }
        f = f.clamp(0, len.max(0));
        let anchor = if extend { e.anchor } else { f };
        self.set(|s| s.editing_text = Some(TextEditState { id: e.id, anchor, focus: f }));
    }

    pub fn plain_text(&self, el: &TextEl) -> String {
        plain_text(el)
    }

    pub fn selected_text(&self) -> String {
        let (Some((_, from, to)), Some(t)) = (self.text_range(), self.edit_target()) else { return String::new() };
        slice16(&plain_text(&t.el), from, to)
    }

    // ------------------------------------------------------------------ styling

    /// Apply run marks (JSON `Partial<RunStyle>`): to the text selection while editing, else to whole text
    /// elements in the selection.
    pub fn set_text_style(&mut self, marks: &Map<String, Value>) {
        if let Some((_, from, to)) = self.text_range() {
            if from == to {
                let cur = self.pending_marks.clone().unwrap_or_default();
                self.pending_marks = Some(merge(&cur, marks));
                self.emit();
                return;
            }
            self.apply_text_ops(
                vec![BodyOp::TextFormat { marks: marks.clone(), offset: Some(from as f64), len: Some((to - from) as f64) }],
                false,
            );
            return;
        }
        if let Some(cr) = self.state.cell_range.clone() {
            let targets = self.table_targets(&cr.id);
            let m = marks.clone();
            self.edit_cell_bodies(&cr.id, &targets, &mut |_, _| vec![BodyOp::TextFormat { marks: m.clone(), offset: None, len: None }], false);
            return;
        }
        let ops: Vec<DocOp> = self
            .selected_deep()
            .iter()
            .filter(|e| matches!(e, Element::Text(_)))
            .map(|e| DocOp::TextFormat { id: el_id(e), marks: marks.clone(), offset: None, len: None })
            .collect();
        if !ops.is_empty() {
            self.apply(ops, false);
        }
    }

    /// Paragraph attributes (JSON `Partial<ParagraphAttrs>`).
    pub fn set_paragraph_style(&mut self, attrs: &Map<String, Value>) {
        if let Some((_, from, to)) = self.text_range() {
            self.apply_text_ops(
                vec![BodyOp::TextParagraph { attrs: attrs.clone(), offset: Some(from as f64), len: Some((to - from) as f64) }],
                false,
            );
            return;
        }
        if let Some(cr) = self.state.cell_range.clone() {
            let targets = self.table_targets(&cr.id);
            let a = attrs.clone();
            self.edit_cell_bodies(&cr.id, &targets, &mut |_, _| vec![BodyOp::TextParagraph { attrs: a.clone(), offset: None, len: None }], false);
            return;
        }
        let ops: Vec<DocOp> = self
            .selected_deep()
            .iter()
            .filter(|e| matches!(e, Element::Text(_)))
            .map(|e| DocOp::TextParagraph { id: el_id(e), attrs: attrs.clone(), offset: None, len: None })
            .collect();
        if !ops.is_empty() {
            self.apply(ops, false);
        }
    }

    fn first_selected_text(&self) -> Option<TextEl> {
        self.selected_deep().into_iter().find_map(|e| match e {
            Element::Text(t) => Some(t),
            _ => None,
        })
    }

    /// Effective run style (JSON `RunStyle`) at the caret / for the first selected text element (toolbar state).
    pub fn current_text_style(&self) -> Option<Map<String, Value>> {
        let e = self.state.editing_text.clone();
        let el = match &e {
            Some(_) => self.edit_target().map(|t| t.el),
            None => self.range_text_el().or_else(|| self.first_selected_text()),
        }?;
        let mut style = Map::new();
        let target = e.as_ref().map_or(0, |e| 0.max(e.anchor.min(e.focus)));
        let mut pos = 0;
        'outer: for p in &el.body.paragraphs {
            for run in &p.runs {
                let n = len16(&run.text);
                if target < pos + n || (target == pos + n && target > 0) {
                    style = jm(&run.style);
                    break 'outer;
                }
                pos += n;
            }
            pos += 1;
        }
        let base = merge(&jm(&el.body.default_run), &style);
        Some(merge(&base, &self.pending_marks.clone().unwrap_or_default()))
    }

    /// Attributes of the first paragraph (JSON `ParagraphAttrs`).
    pub fn current_paragraph(&self) -> Option<Map<String, Value>> {
        let el = self.edit_target().map(|t| t.el).or_else(|| self.range_text_el()).or_else(|| self.first_selected_text())?;
        let mut p = jm(el.body.paragraphs.first()?);
        p.remove("runs");
        Some(p)
    }

    /// First cell of a selected cell range as text (toolbar state while cells are selected).
    fn range_text_el(&self) -> Option<TextEl> {
        let cr = self.state.cell_range.as_ref()?;
        let doc = self.doc();
        self.cell_text_el(&cr.id, cr.r0, cr.c0, Some(&doc)).map(|v| v.el)
    }

    /// True while the text toolbar applies to table cells (caret in a cell or a cell range).
    pub fn editing_table_text(&self) -> bool {
        self.state.cell_range.is_some()
            || self.state.editing_cell.as_ref().is_some_and(|c| self.state.editing_text.as_ref().is_some_and(|e| e.id == c.id))
    }

    /// Text being edited and where its layout origin sits inside the host element.
    pub fn editing_text_frame(&self) -> Option<EditingTextFrame> {
        let e = self.state.editing_text.as_ref()?;
        if let Some(ec) = &self.state.editing_cell
            && ec.id == e.id
        {
            let v = self.cell_text_el(&ec.id, ec.r, ec.c, None)?;
            return Some(EditingTextFrame { el: v.el, host_id: ec.id.clone(), ox: v.box_.x, oy: v.box_.y + v.dy });
        }
        match self.view().el(&e.id) {
            Some(Element::Text(t)) => {
                let oy = self.text_service.as_ref().map_or(0.0, |ts| text_anchor_dy(ts.as_ref(), t));
                Some(EditingTextFrame { el: t.clone(), host_id: t.base.id.clone(), ox: 0.0, oy })
            }
            _ => None,
        }
    }

    /// Selection plus descendants of selected groups (for styling a group's contents).
    pub fn selected_deep(&self) -> Vec<Element> {
        let doc = self.doc();
        let mut out = Vec::new();
        for el in self.selected() {
            let is_group = matches!(el, Element::Group(_));
            let id = el_id(&el);
            out.push(el);
            if is_group {
                out.extend(descendants(doc.els(), &id).into_iter().cloned());
            }
        }
        out
    }

    /// Patch every selected element with the same JSON patch.
    pub fn update_selected(&mut self, patch: &Map<String, Value>, deep: bool, merge: bool) {
        self.update_selected_with(&mut |_| Some(patch.clone()), deep, merge);
    }

    /// Patch selected elements with per-element patches (`id → patch`; missing or `null` = skip). This is the
    /// WASM-friendly form of `updateSelected(fn)`: the host computes patches from the selected elements.
    pub fn update_selected_map(&mut self, patches: &Map<String, Value>, deep: bool, merge: bool) {
        self.update_selected_with(&mut |el| patches.get(el.id().unwrap_or("")).and_then(Value::as_object).cloned(), deep, merge);
    }

    /// Patch every selected element with `f(el)` (`None` = skip).
    pub fn update_selected_with(&mut self, f: &mut dyn FnMut(&Element) -> Option<Map<String, Value>>, deep: bool, merge: bool) {
        let els = if deep { self.selected_deep() } else { self.selected() };
        let mut ops = Vec::new();
        for el in &els {
            let Some(p) = f(el) else { continue };
            ops.push(op_update(&el_id(el), p));
        }
        if !ops.is_empty() {
            self.apply(ops, merge);
        }
    }
}

/// Where a vertically anchored text (middle/bottom of a fixed box) starts below the box top, as the renderer draws it.
pub(crate) fn text_anchor_dy(ts: &dyn crate::text_service::TextService, t: &oc_schema::elements::TextEl) -> f64 {
    use oc_schema::elements::{CellVAlign, TextFrameMode};
    let anchored = matches!(t.valign, Some(CellVAlign::Middle | CellVAlign::Bottom));
    if !anchored || t.frame.mode != TextFrameMode::Fixed || t.curve.is_some_and(|c| c != 0.0) {
        return 0.0;
    }
    let free = t.base.xf.h - ts.height(t, t.base.xf.w);
    if t.valign == Some(CellVAlign::Middle) { free / 2.0 } else { free }
}
