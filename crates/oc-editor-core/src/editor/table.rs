// SPDX-License-Identifier: Apache-2.0
//! Tables (ED-028, A12): cell editing with the shared text engine, cell ranges, cell props, borders, styles,
//! column widths and structure edits.

use oc_crdt::body::{BodyOp, edit_body};
use oc_crdt::{DocOp, DocOpResult};
use oc_geometry::{apply, invert, world_matrix};
use oc_schema::elements::{CellBorder, Element, TableEl, TextEl};
use oc_schema::primitives::{ColorValue, Rect, Vec2, Xf};
use oc_schema::table::{
    BorderPreset, CellPos, CellRange, StructureAction, TableStylePreset, anchor_of, apply_border_preset, apply_table_style, cell_body,
    cell_content_box, cell_key, cell_text_offset, distribute_columns, merge_cells, normalize_range, range_anchors, resize_column,
    slot_at, table_anchors, table_structure_edit, unmerge_cells, with_body,
};
use oc_schema::text::body_plain_text;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{Doc, Editor, one, op_update, virtual_text};
use crate::state::*;
use crate::util::{jm, jv, len16, max};

/// A cell as a text element in the cell's content box (`box` and `dy` in table-local px).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellTextFrame {
    pub el: TextEl,
    #[serde(rename = "box")]
    pub box_: Rect,
    pub dy: f64,
}

pub(crate) struct EditTarget {
    pub el: TextEl,
    pub cell: Option<CellRef>,
}

fn table<'a>(doc: &'a Doc, id: &str) -> Option<&'a TableEl> {
    match doc.el(id) {
        Some(Element::Table(t)) => Some(t),
        _ => None,
    }
}

fn not_ok() -> DocOpResult {
    DocOpResult { ok: false, applied: 0, created: vec![], errors: vec![] }
}

impl Editor {
    /// Edit a cell with the same text engine as text boxes: the caret lives in `editingText` (id = the table) and
    /// `editingCell` says which cell. Without a point the cell's text is selected (spreadsheet-style Tab entry).
    pub fn start_cell_edit(&mut self, id: &str, r: i64, c: i64, page_pt: Option<Vec2>, select_word: bool) {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return };
        if super::locked_content(&Element::Table(t.clone())) {
            return;
        }
        let a = anchor_of(t, 0.max((t.rows.len() as i64 - 1).min(r)), 0.max((t.cols.len() as i64 - 1).min(c)));
        let len = len16(&body_plain_text(&cell_body(t, a.r, a.c)));
        let mut anchor = 0;
        let mut focus = len;
        if let Some(pt) = page_pt
            && self.text_service.is_some()
        {
            focus = self.cell_offset_at(id, a.r, a.c, pt);
            anchor = focus;
            if select_word
                && let Some(v) = self.cell_text_el(id, a.r, a.c, None)
                && let Some(ts) = &self.text_service
            {
                (anchor, focus) = ts.word_at(&v.el, focus);
            }
        }
        let page = doc.page_of(id);
        self.state.selection = vec![id.to_string()];
        if let Some(p) = page {
            self.state.active_page_id = p;
        }
        self.store_mut().checkpoint();
        self.pending_marks = None;
        let id = id.to_string();
        self.set(|s| {
            s.editing_cell = Some(CellRef { id: id.clone(), r: a.r, c: a.c });
            s.cell_range = None;
            s.editing_text = Some(TextEditState { id, anchor, focus });
        });
    }

    pub fn stop_cell_edit(&mut self) {
        let id = self.state.editing_cell.as_ref().map(|c| c.id.clone()).or_else(|| self.state.cell_range.as_ref().map(|c| c.id.clone()));
        if let Some(id) = id {
            self.set(|s| {
                s.editing_cell = None;
                s.cell_range = None;
                if s.editing_text.as_ref().is_some_and(|e| e.id == id) {
                    s.editing_text = None;
                }
            });
        }
    }

    /// Select the cells between two cells (grown to whole merged cells). A single cell goes back to typing.
    pub fn select_cell_range(&mut self, id: &str, from: CellPos, to: CellPos) {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return };
        let rg = normalize_range(t, from, to);
        let anchors = range_anchors(t, &rg);
        if anchors.len() <= 1 {
            let a = anchors.first().copied().unwrap_or(from);
            if self.state.editing_text.as_ref().is_some_and(|e| e.id == id)
                && self.state.editing_cell.as_ref().is_some_and(|c| c.r == a.r && c.c == a.c)
                && self.state.cell_range.is_none()
            {
                return;
            }
            self.start_cell_edit(id, a.r, a.c, None, false);
            return;
        }
        if let Some(cr) = &self.state.cell_range
            && cr.id == id
            && cr.r0 == rg.r0
            && cr.c0 == rg.c0
            && cr.r1 == rg.r1
            && cr.c1 == rg.c1
        {
            return;
        }
        let anchor = anchor_of(t, from.r, from.c);
        let id = id.to_string();
        self.set(|s| {
            s.selection = vec![id.clone()];
            s.editing_cell = Some(CellRef { id: id.clone(), r: anchor.r, c: anchor.c });
            s.cell_range = Some(CellRangeRef { id, r0: rg.r0, c0: rg.c0, r1: rg.r1, c1: rg.c1 });
            s.editing_text = None;
        });
    }

    /// Move the caret cell by Tab (`dir = 1`) / Shift+Tab (`-1`), wrapping across rows, skipping merged cells.
    pub fn cell_tab(&mut self, dir: i32) {
        let Some(ec) = self.state.editing_cell.clone() else { return };
        let doc = self.doc();
        let Some(t) = table(&doc, &ec.id) else { return };
        let list = table_anchors(t);
        let i = list.iter().position(|a| a.r == ec.r && a.c == ec.c).map_or(-1, |i| i as i64);
        let n = i + dir as i64;
        if n < 0 || n >= list.len() as i64 {
            if dir > 0 && !super::locked_content(&Element::Table(t.clone())) {
                // Tab from the last cell adds a row (like Canva and most table editors).
                let id = t.base.id.clone();
                self.table_structure(&id, StructureAction::AddRow, t.rows.len() as i64 - 1);
                let d2 = self.doc();
                if let Some(t2) = table(&d2, &id) {
                    let last = t2.rows.len() as i64 - 1;
                    self.start_cell_edit(&id, last, 0, None, false);
                }
            }
            return;
        }
        let a = list[n as usize];
        let id = t.base.id.clone();
        self.start_cell_edit(&id, a.r, a.c, None, false);
    }

    /// A cell as a text element in the cell's content box (default doc: the view). The id is the layout key the
    /// renderer uses (`"<table>:<r>:<c>"`), so measurement and drawing agree.
    pub fn cell_text_el(&self, id: &str, r: i64, c: i64, doc: Option<&Doc>) -> Option<CellTextFrame> {
        let view;
        let doc = match doc {
            Some(d) => d,
            None => {
                view = self.view();
                &view
            }
        };
        let t = table(doc, id)?;
        let a = anchor_of(t, r, c);
        let bx = cell_content_box(t, a.r, a.c, None);
        let el = virtual_text(&format!("{id}:{}:{}", a.r, a.c), &cell_body(t, a.r, a.c), bx.w, bx.h);
        let h = self.text_service.as_ref().map_or(bx.h, |ts| ts.height(&el, bx.w));
        let dy = cell_text_offset(t, a.r, a.c, bx.h, h);
        Some(CellTextFrame { el, box_: bx, dy })
    }

    /// The text element (real, or a cell's virtual one) the caret is in.
    pub(crate) fn edit_target(&self) -> Option<EditTarget> {
        let e = self.state.editing_text.as_ref()?;
        let doc = self.doc();
        if let Some(ec) = &self.state.editing_cell
            && ec.id == e.id
        {
            let v = self.cell_text_el(&ec.id, ec.r, ec.c, Some(&doc))?;
            return Some(EditTarget { el: v.el, cell: Some(ec.clone()) });
        }
        match doc.el(&e.id) {
            Some(Element::Text(t)) => Some(EditTarget { el: t.clone(), cell: None }),
            _ => None,
        }
    }

    pub(crate) fn cell_offset_at(&self, id: &str, r: i64, c: i64, page_pt: Vec2) -> i64 {
        let doc = self.doc();
        let Some(v) = self.cell_text_el(id, r, c, Some(&doc)) else { return 0 };
        let Some(ts) = &self.text_service else { return 0 };
        let [x, y] = apply(&invert(&world_matrix(doc.els(), id)), page_pt);
        ts.offset_at(&v.el, x - v.box_.x, y - v.box_.y - v.dy)
    }

    pub(crate) fn table_slot_at(&self, id: &str, page_pt: Vec2) -> Option<CellPos> {
        let doc = self.doc();
        let t = table(&doc, id)?;
        let [x, y] = apply(&invert(&world_matrix(doc.els(), id)), page_pt);
        slot_at(t, x, y, None)
    }

    /// Text ops on the edited text: written to the text element, or folded into the edited cell's body.
    pub(crate) fn apply_text_ops(&mut self, ops: Vec<BodyOp>, merge: bool) -> DocOpResult {
        let Some(t) = self.edit_target() else { return not_ok() };
        match t.cell {
            None => {
                let id = t.el.base.id.clone();
                let ops = ops.into_iter().map(|o| body_to_doc_op(o, &id)).collect();
                self.apply(ops, merge)
            }
            Some(cell) => self.edit_cell_bodies(&cell.id, &[CellPos { r: cell.r, c: cell.c }], &mut |_, _| ops.clone(), merge),
        }
    }

    /// Rewrite cell bodies (rich text) in one update; legacy cells are migrated as they are touched.
    pub(crate) fn edit_cell_bodies(
        &mut self,
        id: &str,
        cells: &[CellPos],
        ops: &mut dyn FnMut(&oc_schema::TextBody, CellPos) -> Vec<BodyOp>,
        merge: bool,
    ) -> DocOpResult {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return not_ok() };
        let mut next = t.cells.clone();
        for p in cells {
            let body = cell_body(t, p.r, p.c);
            let list = ops(&body, *p);
            if list.is_empty() {
                continue;
            }
            let Ok(edited) = edit_body(&body, &list) else { continue };
            let k = cell_key(p.r, p.c);
            next.insert(k.clone(), with_body(t.cells.get(&k), edited));
        }
        self.apply(vec![op_update(id, one("cells", jv(&next)))], merge)
    }

    /// Cell under a page point for a table element (the merged cell's anchor).
    pub fn cell_at(&self, id: &str, page_pt: Vec2) -> Option<CellPos> {
        let doc = self.doc();
        let t = table(&doc, id)?;
        let s = self.table_slot_at(id, page_pt)?;
        Some(anchor_of(t, s.r, s.c))
    }

    fn cells_json(t: &TableEl) -> Map<String, Value> {
        jm(&t.cells)
    }

    /// Patch one cell (JSON `Partial<TableCell>`). Setting plain `text` replaces any rich body.
    pub fn set_cell(&mut self, id: &str, r: i64, c: i64, patch: &Map<String, Value>) {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return };
        let key = format!("{r}:{c}");
        let mut cells = Self::cells_json(t);
        let mut next = one("text", Value::String(String::new()));
        if let Some(Value::Object(cur)) = cells.get(&key) {
            for (k, v) in cur {
                next.insert(k.clone(), v.clone());
            }
        }
        for (k, v) in patch {
            next.insert(k.clone(), v.clone());
        }
        if patch.contains_key("text") && !patch.contains_key("body") {
            next.remove("body");
        }
        cells.insert(key, Value::Object(next));
        self.apply(vec![op_update(id, one("cells", Value::Object(cells)))], true);
    }

    /// The range table commands act on: the selected cells, the caret cell, or (neither) the whole table.
    pub fn table_target_range(&self, id: &str) -> Option<CellRange> {
        let doc = self.doc();
        let t = table(&doc, id)?;
        if let Some(cr) = &self.state.cell_range
            && cr.id == id
        {
            return Some(CellRange { r0: cr.r0, c0: cr.c0, r1: cr.r1, c1: cr.c1 });
        }
        if let Some(ec) = &self.state.editing_cell
            && ec.id == id
        {
            let p = CellPos { r: ec.r, c: ec.c };
            return Some(normalize_range(t, p, p));
        }
        Some(CellRange { r0: 0, c0: 0, r1: t.rows.len() as i64 - 1, c1: t.cols.len() as i64 - 1 })
    }

    /// Anchor cells table commands act on (see [`Editor::table_target_range`]).
    pub fn table_targets(&self, id: &str) -> Vec<CellPos> {
        let doc = self.doc();
        match (table(&doc, id), self.table_target_range(id)) {
            (Some(t), Some(rg)) => range_anchors(t, &rg),
            _ => vec![],
        }
    }

    /// Patch every targeted cell (fill, valign, padding, bold…); `null` clears a field.
    pub fn set_cells(&mut self, id: &str, patch: &Map<String, Value>) {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return };
        let mut cells = Self::cells_json(t);
        for p in self.table_targets(id) {
            let k = cell_key(p.r, p.c);
            let mut next = one("text", Value::String(String::new()));
            if let Some(Value::Object(cur)) = cells.get(&k) {
                for (kk, v) in cur {
                    next.insert(kk.clone(), v.clone());
                }
            }
            for (f, v) in patch {
                if v.is_null() {
                    next.remove(f);
                } else {
                    next.insert(f.clone(), v.clone());
                }
            }
            cells.insert(k, Value::Object(next));
        }
        self.apply(vec![op_update(id, one("cells", Value::Object(cells)))], true);
    }

    /// Horizontal alignment of the targeted cells (written to their paragraphs too, so rich bodies follow).
    pub fn set_cell_align(&mut self, id: &str, align: &str) {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return };
        let targets = self.table_targets(id);
        let mut cells = Self::cells_json(t);
        for p in targets {
            let k = cell_key(p.r, p.c);
            let cell = t.cells.get(&k);
            let next = match cell {
                Some(c) if c.body.is_some() => {
                    let attrs = one("align", Value::String(align.into()));
                    let body = edit_body(&cell_body(t, p.r, p.c), &[BodyOp::TextParagraph { attrs, offset: None, len: None }]);
                    let mut m = match body {
                        Ok(b) => jm(&with_body(Some(c), b)),
                        Err(_) => jm(c),
                    };
                    m.insert("align".into(), Value::String(align.into()));
                    m
                }
                _ => {
                    let mut m = one("text", Value::String(String::new()));
                    if let Some(c) = cell {
                        for (kk, v) in jm(c) {
                            m.insert(kk, v);
                        }
                    }
                    m.insert("align".into(), Value::String(align.into()));
                    m
                }
            };
            cells.insert(k, Value::Object(next));
        }
        self.apply(vec![op_update(id, one("cells", Value::Object(cells)))], false);
    }

    /// Border preset over the targeted range (whole table when nothing is picked).
    pub fn set_cell_borders(&mut self, id: &str, preset: BorderPreset, width: Option<f64>, color: Option<ColorValue>) {
        let doc = self.doc();
        let (Some(t), Some(rg)) = (table(&doc, id), self.table_target_range(id)) else { return };
        let bw = width.unwrap_or(if t.style.border_width != 0.0 { t.style.border_width } else { 1.5 });
        let b = CellBorder::new(bw, color.unwrap_or_else(|| t.style.border_color.clone()));
        let cells = apply_border_preset(t, &rg, &preset, &b);
        self.apply(vec![op_update(id, one("cells", jv(&cells)))], false);
    }

    /// Merge the selected cell range into one cell.
    pub fn merge_cell_range(&mut self, id: &str) {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return };
        let Some(cr) = self.state.cell_range.clone().filter(|c| c.id == id) else { return };
        let rg = CellRange { r0: cr.r0, c0: cr.c0, r1: cr.r1, c1: cr.c1 };
        if self.apply(vec![op_update(id, one("cells", jv(&merge_cells(t, &rg))))], false).ok {
            self.start_cell_edit(id, cr.r0, cr.c0, None, false);
        }
    }

    /// Split merged cells in the targeted range back into single cells.
    pub fn unmerge_cell_range(&mut self, id: &str) {
        let doc = self.doc();
        let (Some(t), Some(rg)) = (table(&doc, id), self.table_target_range(id)) else { return };
        self.apply(vec![op_update(id, one("cells", jv(&unmerge_cells(t, &rg))))], false);
    }

    /// Clear the text of the targeted cells (Delete with a cell range selected).
    pub fn clear_cells(&mut self, id: &str) {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return };
        let mut cells = Self::cells_json(t);
        for p in self.table_targets(id) {
            let k = cell_key(p.r, p.c);
            let Some(Value::Object(cur)) = cells.get(&k).cloned() else { continue };
            let mut m = cur;
            m.remove("body");
            m.insert("text".into(), Value::String(String::new()));
            cells.insert(k, Value::Object(m));
        }
        self.apply(vec![op_update(id, one("cells", Value::Object(cells)))], false);
    }

    /// One of the table style presets.
    pub fn set_table_style(&mut self, id: &str, preset: TableStylePreset) {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return };
        let Some(st) = apply_table_style(t, &preset) else { return };
        self.apply(vec![op_update(id, jm(&st))], false);
    }

    /// Column widths: even distribution (`index = None`), or one column set to `px` (the neighbour to the right
    /// absorbs the change; the last column grows the table).
    pub fn set_column_width(&mut self, id: &str, index: Option<i64>, px: Option<f64>) {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return };
        let Some(index) = index else {
            self.apply(vec![op_update(id, one("cols", jv(&distribute_columns(&t.cols))))], false);
            return;
        };
        let Some(px) = px else { return };
        if index < 0 || index >= t.cols.len() as i64 {
            return;
        }
        let total: f64 = t.cols.iter().sum();
        let sum = if total != 0.0 && !total.is_nan() { total } else { 1.0 };
        let x0 = t.cols[..index as usize].iter().sum::<f64>() / sum * t.base.xf.w;
        if index < t.cols.len() as i64 - 1 {
            self.apply(vec![op_update(id, one("cols", jv(&resize_column(t, index, x0 + px, None))))], false);
            return;
        }
        // Last column: grow/shrink the table.
        let mut widths: Vec<f64> = t.cols.iter().map(|c| (c / sum) * t.base.xf.w).collect();
        widths[index as usize] = max(16.0, px);
        let w: f64 = widths.iter().sum();
        let cols: Vec<f64> = widths.iter().map(|x| oc_schema::js::to_fixed_num(x / w, 5)).collect();
        let mut patch = one("cols", jv(&cols));
        patch.insert("xf".into(), jv(&Xf { w, ..t.base.xf.clone() }));
        self.apply(vec![op_update(id, patch)], false);
    }

    /// Insert/remove rows or columns, keeping cell contents (and merged cells) aligned.
    pub fn table_structure(&mut self, id: &str, action: StructureAction, at: i64) {
        let doc = self.doc();
        let Some(t) = table(&doc, id) else { return };
        let Some(next) = table_structure_edit(t, &action, at) else { return };
        self.apply(vec![op_update(id, jm(&next))], false);
        self.stop_cell_edit();
    }
}

pub(crate) fn body_to_doc_op(o: BodyOp, id: &str) -> DocOp {
    let id = id.to_string();
    match o {
        BodyOp::TextInsert { offset, text, marks } => DocOp::TextInsert { id, offset, text, marks },
        BodyOp::TextDelete { offset, len } => DocOp::TextDelete { id, offset, len },
        BodyOp::TextFormat { marks, offset, len } => DocOp::TextFormat { id, marks, offset, len },
        BodyOp::TextParagraph { attrs, offset, len } => DocOp::TextParagraph { id, attrs, offset, len },
    }
}
