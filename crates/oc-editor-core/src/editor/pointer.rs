// SPDX-License-Identifier: Apache-2.0
//! Pointer input (scene coordinates): selection, marquee, move/resize/rotate gestures, connector ends, table
//! cells, crop panning, freehand strokes, double-click.

use std::collections::HashSet;

use indexmap::IndexMap;
use oc_crdt::DocOp;
use super::text::text_anchor_dy;
use oc_geometry::{
    HitTestOpts, RulerGuide, SnapResult, apply, apply_vector, hit_test, invert, marquee_select, nearest_anchor, normalize_angle,
    parent_inverse, snap_angle, snap_move, world_bounds, world_matrix,
};
use oc_schema::design::{GuideAxis, Surface};
use oc_schema::elements::{Element, LineAttach};
use oc_schema::keyframes::{keyframe_neutral, keyframe_value};
use oc_schema::primitives::{Rect, Vec2, Xf};
use oc_schema::table::{anchor_of, resize_column};
use oc_schema::tree::{Elements, children_of, descendants};
use serde_json::{Map, Value};

use super::{Doc, Editor, Gesture, el_id, el_xf, locked_move, one, op_add, op_update, visible};
use crate::state::*;
use crate::transform::{Handle, crop_resize, resize_box, rotate_about, scale_about, scale_patches};
use crate::util::{atan2, cos, hypot, jv, merge, round, sin};
use crate::viewport::page_at_scene;
use crate::whiteboard::{is_section, with_section_members};

fn ruler_guides(doc: &Doc, page_id: &str) -> Vec<RulerGuide> {
    doc.doc
        .pages
        .get(page_id)
        .and_then(|p| p.guides.as_ref())
        .map(|gs| {
            gs.iter()
                .filter_map(|g| match g.axis {
                    GuideAxis::X => Some(RulerGuide { axis: oc_geometry::Axis::X, pos: g.pos }),
                    GuideAxis::Y => Some(RulerGuide { axis: oc_geometry::Axis::Y, pos: g.pos }),
                    GuideAxis::Other(_) => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Editor {
    /// Pointer down at a scene point. `button` 0 = primary (others are ignored).
    pub fn pointer_down(&mut self, p: Vec2, mods: Mods, target: HitTarget, button: u8) {
        if button != 0 {
            return;
        }
        let doc = self.view();
        let Some(slot) = page_at_scene(&doc.doc, p) else { return };
        let page_id = slot.page.id.clone();
        let local = self.to_page(&page_id, p);
        self.store_mut().checkpoint();
        // A locked page (g96) only becomes the active page: nothing on it is picked, drawn on or erased.
        if doc.page_locked(&page_id) {
            self.set(|s| {
                s.active_page_id = page_id.clone();
                s.selection = vec![];
                s.editing_text = None;
                s.entered_group = None;
            });
            return;
        }
        // Pen (DS-100) and node editing take the click before selection does.
        if self.state.tool == Tool::Pen {
            self.pen_down(&page_id, local, mods);
            return;
        }
        if self.state.path_edit.is_some() && self.path_down(local, mods, &target) {
            return;
        }
        if matches!(self.state.tool, Tool::Draw | Tool::Erase) {
            let pid = page_id.clone();
            self.set(|s| {
                s.drawing = Some(Drawing { page_id: pid.clone(), points: vec![local[0], local[1], 0.5] });
                s.active_page_id = pid;
                s.selection = vec![];
            });
            return;
        }
        // Table column border drag (A12).
        if let HitTarget::TableCol { id, index } = &target
            && let Some(Element::Table(t)) = doc.el(id)
            && t.base.lock.as_ref().and_then(|l| l.resize) != Some(true)
        {
            self.gesture = Some(Gesture::ColResize { id: id.clone(), page_id, index: *index });
            self.set(|s| s.gesturing = true);
            return;
        }

        // Inside a table being edited: clicks move the caret or pick cells; shift extends a cell range.
        if let Some(ec) = self.state.editing_cell.clone() {
            let slot_pos = match doc.el(&ec.id) {
                Some(Element::Table(_)) => self.table_slot_at(&ec.id, local),
                _ => None,
            };
            if let (Some(Element::Table(t)), Some(sp)) = (doc.el(&ec.id), slot_pos) {
                let a = anchor_of(t, sp.r, sp.c);
                let tid = t.base.id.clone();
                if mods.shift {
                    self.select_cell_range(&tid, oc_schema::table::CellPos { r: ec.r, c: ec.c }, a);
                    self.gesture =
                        Some(Gesture::CellSelect { id: tid, page_id, start: oc_schema::table::CellPos { r: ec.r, c: ec.c }, moved: true });
                    return;
                }
                let typing_same = self.state.editing_text.as_ref().is_some_and(|e| e.id == tid)
                    && a.r == ec.r
                    && a.c == ec.c
                    && self.state.cell_range.is_none();
                if typing_same {
                    let off = self.cell_offset_at(&tid, a.r, a.c, local);
                    let id2 = tid.clone();
                    self.set(|s| s.editing_text = Some(TextEditState { id: id2, anchor: off, focus: off }));
                } else {
                    self.start_cell_edit(&tid, a.r, a.c, Some(local), false);
                }
                self.gesture = Some(Gesture::CellSelect { id: tid, page_id, start: a, moved: false });
                return;
            }
            self.set(|s| {
                s.editing_cell = None;
                s.cell_range = None;
                if s.editing_text.as_ref().is_some_and(|e| e.id == ec.id) {
                    s.editing_text = None;
                }
            });
        }

        // Connector ends: drag to move/glue (WB-010).
        if let HitTarget::Endpoint { end } = target
            && self.state.selection.len() == 1
            && let Some(Element::Line(l)) = doc.el(&self.state.selection[0])
        {
            let m = world_matrix(doc.els(), &l.base.id);
            let pt = if end == 0 { l.points.last() } else { l.points.first() };
            let other = apply(&m, pt.copied().unwrap_or([0.0, 0.0]));
            self.gesture = Some(Gesture::Endpoint { id: l.base.id.clone(), end, page_id, other });
            self.set(|s| s.gesturing = true);
            return;
        }

        // "Copy style" armed: a click applies it to the element under the pointer.
        if self.state.style_brush.is_some() && target == HitTarget::Canvas {
            let hit = hit_test(doc.els(), &page_id, local, HitTestOpts { tolerance: 0.0, enter_group: self.state.entered_group.as_deref() })
                .map(el_id);
            if let Some(h) = hit {
                let style = self.state.style_brush.clone();
                self.paste_style(std::slice::from_ref(&h), style.as_ref());
                self.select(&[h], false);
            }
            if !mods.shift {
                self.set(|s| s.style_brush = None);
            }
            return;
        }

        if let HitTarget::Handle { handle } = target
            && !self.state.selection.is_empty()
        {
            let mut origin: IndexMap<String, Element> = IndexMap::new();
            for el in self.selected() {
                let id = el_id(&el);
                origin.insert(id.clone(), el.clone());
                for d in descendants(doc.els(), &id) {
                    origin.insert(el_id(d), d.clone());
                }
            }
            let bounds = self.selection_box().unwrap_or_default();
            let single = self.state.selection.len() == 1;
            self.gesture = Some(Gesture::Resize { handle, origin, bounds, single, page_id });
            self.set(|s| s.gesturing = true);
            return;
        }
        if target == HitTarget::Rotate && !self.state.selection.is_empty() {
            let b = self.selection_box().unwrap_or_default();
            let pivot = [b.x + b.w / 2.0, b.y + b.h / 2.0];
            let origin: IndexMap<String, Xf> = self.selected().iter().map(|e| (el_id(e), el_xf(e))).collect();
            let start_angle = atan2(local[1] - pivot[1], local[0] - pivot[0]);
            self.gesture = Some(Gesture::Rotate { origin, pivot, start_angle, page_id });
            self.set(|s| s.gesturing = true);
            return;
        }

        // Cropping mode: drag pans the image inside its frame; click outside exits.
        if let Some(cid) = self.state.cropping_id.clone() {
            if let Some(el) = doc.el(&cid) {
                let hit = hit_test(doc.els(), &page_id, local, HitTestOpts { tolerance: 0.0, enter_group: self.state.entered_group.as_deref() })
                    .map(el_id);
                if hit.as_deref() == Some(cid.as_str())
                    && let Some(crop) = self.media_crop(el)
                {
                    self.gesture = Some(Gesture::CropPan { id: cid, start: local, crop });
                    return;
                }
            }
            self.set(|s| s.cropping_id = None);
        }

        // Text editing: clicks inside the edited box move the caret.
        if let Some(editing) = self.state.editing_text.clone() {
            if let (Some(Element::Text(t)), Some(ts)) = (doc.el(&editing.id), self.text_service.as_ref()) {
                let [bx, by] = apply(&invert(&world_matrix(doc.els(), &t.base.id)), local);
                if bx >= -4.0 && by >= -4.0 && bx <= t.base.xf.w + 4.0 && by <= t.base.xf.h + 4.0 {
                    let off = ts.offset_at(t, bx, by - text_anchor_dy(ts.as_ref(), t));
                    let id = t.base.id.clone();
                    let anchor = if mods.shift { editing.anchor } else { off };
                    self.set(|s| s.editing_text = Some(TextEditState { id: id.clone(), anchor, focus: off }));
                    self.gesture = Some(Gesture::TextSelect { id });
                    return;
                }
            }
            self.set(|s| s.editing_text = None);
        }

        let hit = hit_test(doc.els(), &page_id, local, HitTestOpts { tolerance: 0.0, enter_group: self.state.entered_group.as_deref() }).cloned();
        let Some(hit) = hit.filter(|h| !locked_move(h)) else {
            if let Some(h) = hit_test(doc.els(), &page_id, local, HitTestOpts { tolerance: 0.0, enter_group: self.state.entered_group.as_deref() })
                && locked_move(h)
            {
                let id = el_id(h);
                self.select(&[id], false);
                return;
            }
            let pid = page_id.clone();
            self.set(|s| {
                s.active_page_id = pid;
                s.entered_group = None;
            });
            let additive = if mods.shift { self.state.selection.clone() } else { vec![] };
            self.gesture = Some(Gesture::Marquee { start: local, page_id, additive });
            if !mods.shift {
                self.set(|s| s.selection = vec![]);
            }
            return;
        };
        let hid = el_id(&hit);
        let mut selection = self.state.selection.clone();
        if mods.shift {
            if selection.contains(&hid) {
                selection.retain(|i| *i != hid);
            } else {
                selection.push(hid.clone());
            }
            // Shift-selection only makes sense within one page.
            selection.retain(|i| doc.page_of(i).as_deref() == Some(page_id.as_str()));
        } else if !selection.contains(&hid) {
            selection = vec![hid.clone()];
        }
        self.select(&selection, false);
        if !selection.contains(&hid) {
            return;
        }
        // Whiteboard sections carry the elements inside them.
        let moving: Vec<Element> = if doc.doc.meta.surface == Surface::Whiteboard {
            with_section_members(doc.els(), &selection).iter().filter_map(|i| doc.el(i).cloned()).collect()
        } else {
            self.selected()
        };
        let origin: IndexMap<String, Xf> = moving.iter().filter(|e| !locked_move(e)).map(|e| (el_id(e), el_xf(e))).collect();
        // Alt/Option-drag leaves the originals and drops copies (ED-012).
        let start_bounds = self.selection_box().unwrap_or_default();
        self.gesture = Some(Gesture::Move { start: local, origin, start_bounds, page_id, moved: false, duplicate: mods.alt });
    }

    /// Pointer move (scene point). `pressure` feeds freehand strokes (TS default 0.5).
    pub fn pointer_move(&mut self, p: Vec2, mods: Mods, pressure: f64) {
        if let Some(d) = self.state.drawing.clone() {
            let [x, y] = self.to_page(&d.page_id, p);
            let n = d.points.len();
            // Drop points closer than 1.5 screen px to keep strokes compact.
            if hypot(x - d.points[n - 3], y - d.points[n - 2]) * self.state.viewport.zoom < 1.5 {
                return;
            }
            self.set(|s| {
                if let Some(dr) = &mut s.drawing {
                    dr.points.extend([x, y, pressure]);
                }
            });
            return;
        }
        if self.state.tool == Tool::Pen && self.pen_move(p, mods) {
            return;
        }
        if let Some(Gesture::PathEdit { id, .. }) = &self.gesture {
            let page = self.doc().page_of(id).unwrap_or_default();
            let local = self.to_page(&page, p);
            self.path_move(local, mods);
            return;
        }
        let Some(g) = self.gesture.clone() else { return };
        let doc = self.doc();
        let page_for = match &g {
            Gesture::PathEdit { .. } => return,
            Gesture::TextSelect { id } | Gesture::CropPan { id, .. } => doc.page_of(id).unwrap_or_default(),
            Gesture::Move { page_id, .. }
            | Gesture::Endpoint { page_id, .. }
            | Gesture::Resize { page_id, .. }
            | Gesture::Rotate { page_id, .. }
            | Gesture::Marquee { page_id, .. }
            | Gesture::CellSelect { page_id, .. }
            | Gesture::ColResize { page_id, .. } => page_id.clone(),
        };
        let local = self.to_page(&page_for, p);
        let thresh = 6.0 / self.state.viewport.zoom;

        match g {
            Gesture::PathEdit { .. } => {}
            Gesture::Move { start, origin, start_bounds, page_id, moved, duplicate } => {
                let mut dx = local[0] - start[0];
                let mut dy = local[1] - start[1];
                if !moved && hypot(dx, dy) * self.state.viewport.zoom < 3.0 {
                    return;
                }
                self.gesture = Some(Gesture::Move { start, origin: origin.clone(), start_bounds, page_id: page_id.clone(), moved: true, duplicate });
                if mods.shift {
                    if dx.abs() > dy.abs() {
                        dy = 0.0;
                    } else {
                        dx = 0.0;
                    }
                }
                let moved_r = Rect { x: start_bounds.x + dx, y: start_bounds.y + dy, ..start_bounds };
                let page = Rect::new(0.0, 0.0, doc.doc.meta.dims.w, doc.doc.meta.dims.h);
                // ⌘/Ctrl while dragging turns snapping off.
                let snap = if mods.meta {
                    SnapResult::default()
                } else {
                    let ex: HashSet<String> = origin.keys().cloned().collect();
                    snap_move(&moved_r, &page, &self.top_level_siblings(&page_id, &ex), thresh, &ruler_guides(&doc, &page_id))
                };
                dx += snap.dx;
                dy += snap.dy;
                self.clear_overrides();
                for (id, xf) in &origin {
                    let Some(el) = doc.el(id) else { continue };
                    // Delta is in page space; convert to the element's parent space (groups may be rotated).
                    let [pdx, pdy] = apply_vector(&parent_inverse(doc.els(), el), [dx, dy]);
                    self.set_override(id, one("xf", jv(&Xf { x: xf.x + pdx, y: xf.y + pdy, ..xf.clone() })));
                }
                self.set(|s| {
                    s.guides = snap.guides;
                    s.spacing = snap.spacing;
                    s.gesturing = true;
                });
            }
            Gesture::Endpoint { id, end, page_id, other } => {
                let Some(Element::Line(line)) = doc.el(&id) else { return };
                // Glue to the nearest anchor of the element under the pointer (excluding the line itself).
                let mut pt = local;
                let mut target: Option<ConnectTarget> = None;
                let radius = 14.0 / self.state.viewport.zoom;
                for cand in children_of(doc.els(), &page_id).into_iter().rev() {
                    if cand.id() == Some(id.as_str()) || matches!(cand, Element::Line(_)) || !visible(cand) {
                        continue;
                    }
                    let cid = el_id(cand);
                    let b = world_bounds(doc.els(), &cid).unwrap_or_default();
                    if local[0] < b.x - radius || local[0] > b.x + b.w + radius || local[1] < b.y - radius || local[1] > b.y + b.h + radius {
                        continue;
                    }
                    if let Some(a) = nearest_anchor(doc.els(), &cid, local, crate::util::max(radius, crate::util::min(b.w, b.h) / 2.0)) {
                        pt = a.point;
                        target = Some(ConnectTarget { id: cid, anchor: a.anchor, point: a.point });
                        break;
                    }
                }
                if mods.shift && target.is_none() {
                    // Shift: constrain to 45° steps from the other end.
                    let q = std::f64::consts::PI / 4.0;
                    let ang = round(atan2(local[1] - other[1], local[0] - other[0]) / q) * q;
                    let len = hypot(local[0] - other[0], local[1] - other[1]);
                    pt = [other[0] + cos(ang) * len, other[1] + sin(ang) * len];
                }
                let (a, b) = if end == 0 { (pt, other) } else { (other, pt) };
                let patch = line_from_ends(doc.els(), &Element::Line(line.clone()), a, b);
                self.set_override(&id, patch);
                self.set(|s| {
                    s.connect_target = target;
                    s.gesturing = true;
                });
            }
            Gesture::Resize { handle, origin, bounds, single, .. } => {
                self.resize_move(handle, &origin, bounds, single, local, mods);
            }
            Gesture::Rotate { origin, pivot, start_angle, .. } => {
                let a = atan2(local[1] - pivot[1], local[0] - pivot[0]);
                let mut delta = ((a - start_angle) * 180.0) / std::f64::consts::PI;
                if origin.len() == 1 {
                    let xf0 = origin.values().next().cloned().unwrap_or_default();
                    delta = snap_angle(normalize_angle(xf0.rot + delta), mods.shift) - xf0.rot;
                } else if mods.shift {
                    delta = round(delta / 15.0) * 15.0;
                }
                self.clear_overrides();
                for (id, xf) in &origin {
                    let Some(el) = doc.el(id) else { continue };
                    let inv = parent_inverse(doc.els(), el);
                    let pv = apply(&inv, pivot);
                    self.set_override(id, one("xf", jv(&rotate_about(xf, pv, delta))));
                }
                self.emit();
            }
            Gesture::Marquee { start, page_id, additive } => {
                let r = Rect {
                    x: start[0].min(local[0]),
                    y: start[1].min(local[1]),
                    w: (local[0] - start[0]).abs(),
                    h: (local[1] - start[1]).abs(),
                };
                let [ox, oy] = self.page_origin(&page_id);
                // Sections are only marquee-selected when fully enclosed (dragging inside one selects its contents).
                let hits: Vec<String> = marquee_select(doc.els(), &page_id, &r)
                    .into_iter()
                    .filter(|e| {
                        let x = el_xf(e);
                        !is_section(Some(e)) || (x.x >= r.x && x.y >= r.y && x.x + x.w <= r.x + r.w && x.y + x.h <= r.y + r.h)
                    })
                    .map(el_id)
                    .collect();
                let mut sel: indexmap::IndexSet<String> = additive.into_iter().collect();
                sel.extend(hits);
                self.set(|s| {
                    s.marquee = Some(Rect { x: r.x + ox, y: r.y + oy, ..r });
                    s.selection = sel.into_iter().collect();
                });
            }
            Gesture::CellSelect { id, page_id, start, moved } => {
                let Some(Element::Table(t)) = doc.el(&id) else { return };
                let Some(sp) = self.table_slot_at(&id, local) else { return };
                let a = anchor_of(t, sp.r, sp.c);
                if !moved && a.r == start.r && a.c == start.c {
                    // Drag inside the cell selects text.
                    if let Some(e) = self.state.editing_text.clone()
                        && e.id == id
                    {
                        let focus = self.cell_offset_at(&id, a.r, a.c, local);
                        self.set(|s| s.editing_text = Some(TextEditState { focus, ..e }));
                    }
                    return;
                }
                self.gesture = Some(Gesture::CellSelect { id: id.clone(), page_id, start, moved: true });
                self.select_cell_range(&id, start, a);
            }
            Gesture::ColResize { id, index, .. } => {
                let Some(Element::Table(t)) = doc.el(&id) else { return };
                let [x, _] = apply(&invert(&world_matrix(doc.els(), &id)), local);
                self.set_override(&id, one("cols", jv(&resize_column(t, index, x, None))));
                self.set(|s| s.gesturing = true);
            }
            Gesture::TextSelect { id } => {
                let (Some(Element::Text(t)), Some(e), Some(ts)) = (doc.el(&id), self.state.editing_text.clone(), self.text_service.as_ref()) else {
                    return;
                };
                let [bx, by] = apply(&invert(&world_matrix(doc.els(), &id)), local);
                let focus = ts.offset_at(t, bx, by - text_anchor_dy(ts.as_ref(), t));
                self.set(|s| s.editing_text = Some(TextEditState { focus, ..e }));
            }
            Gesture::CropPan { id, start, crop } => {
                let Some(el) = doc.el(&id) else { return };
                let [dx, dy] = apply_vector(&invert(&world_matrix(doc.els(), &id)), [local[0] - start[0], local[1] - start[1]]);
                let x = el_xf(el);
                let nx = (1.0 - crop.w).min(0f64.max(crop.x - (dx / x.w) * crop.w));
                let ny = (1.0 - crop.h).min(0f64.max(crop.y - (dy / x.h) * crop.h));
                let next = oc_schema::primitives::StoredRect { x: nx, y: ny, ..crop };
                if let Some(patch) = self.crop_patch(el, &next) {
                    self.set_override(&id, patch);
                }
                self.emit();
            }
        }
    }

    fn resize_move(&mut self, handle: Handle, origin: &IndexMap<String, Element>, bounds: Rect, single: bool, local: Vec2, mods: Mods) {
        let doc = self.doc();
        self.clear_overrides();
        let odoc = origin_doc(doc.els(), origin);
        if single {
            let Some(el) = self.state.selection.first().and_then(|s| origin.get(s)).cloned() else { return };
            let id = el_id(&el);
            let inv = parent_inverse(doc.els(), &el);
            let p = apply(&inv, local);
            let is_corner = handle.is_corner();
            // A shape filled with a photo keeps its aspect on corners like an image (its photo never stretches).
            let photo_shape = matches!(el, Element::Shape(_)) && self.media_crop(&el).is_some();
            let aspect_locked = matches!(el, Element::Text(_) | Element::Image(_) | Element::Group(_) | Element::Qr(_))
                || matches!(&el, Element::Frame(f) if f.content.is_some())
                || photo_shape;
            let keep_aspect = if is_corner { aspect_locked != mods.shift } else { false };
            let xf = el_xf(&el);
            let r = resize_box(&xf, handle, p, keep_aspect, 4.0);
            match &el {
                Element::Image(img) if !is_corner => {
                    let (nxf, crop) = crop_resize(&xf, &img.crop, &r.xf, handle);
                    let mut patch = one("xf", jv(&nxf));
                    patch.insert("crop".into(), jv(&crop));
                    self.set_override(&id, patch);
                }
                Element::Text(t) if !is_corner => {
                    let h = self.text_service.as_ref().map_or(xf.h, |ts| ts.height(t, r.xf.w));
                    self.set_override(&id, one("xf", jv(&Xf { h, ..r.xf.clone() })));
                }
                _ => {
                    self.set_override(&id, one("xf", jv(&r.xf)));
                    for (cid, patch) in scale_patches(&odoc, &el, r.sx, r.sy) {
                        let prev = self.overrides.get(&cid).cloned().unwrap_or_default();
                        self.set_override(&cid, merge(&prev, &patch));
                    }
                }
            }
        } else {
            // Multi-selection: uniform scale about the opposite corner of the selection bounds.
            let b = bounds;
            let ax = if handle.has('w') {
                b.x + b.w
            } else if handle.has('e') {
                b.x
            } else {
                b.x + b.w / 2.0
            };
            let ay = if handle.has('n') {
                b.y + b.h
            } else if handle.has('s') {
                b.y
            } else {
                b.y + b.h / 2.0
            };
            let sx = if handle.has('e') || handle.has('w') { (local[0] - ax).abs() / b.w } else { f64::INFINITY };
            let sy = if handle.has('n') || handle.has('s') { (local[1] - ay).abs() / b.h } else { f64::INFINITY };
            let m = crate::util::min(sx, sy);
            let s = crate::util::max(0.02, if m == f64::INFINITY { 1.0 } else { m });
            for id in self.state.selection.clone() {
                let Some(el) = origin.get(&id) else { continue };
                let pivot = apply(&parent_inverse(doc.els(), el), [ax, ay]);
                self.set_override(&id, one("xf", jv(&scale_about(&el_xf(el), pivot, s))));
                for (cid, patch) in scale_patches(&odoc, el, s, s) {
                    let prev = self.overrides.get(&cid).cloned().unwrap_or_default();
                    self.set_override(&cid, merge(&prev, &patch));
                }
            }
        }
        self.set(|s| s.gesturing = true);
    }

    /// Abandon the gesture in progress without committing it (a second finger turned a drag into a pinch).
    pub fn cancel_gesture(&mut self) {
        if let Some(mut pen) = self.state.pen.clone().filter(|p| p.dragging) {
            pen.dragging = false;
            self.set(|s| {
                s.pen = Some(pen);
                s.gesturing = false;
            });
        }
        if self.gesture.is_none() && self.state.drawing.is_none() {
            return;
        }
        self.gesture = None;
        self.clear_overrides();
        self.set(|s| {
            s.gesturing = false;
            s.marquee = None;
            s.guides = vec![];
            s.spacing = vec![];
            s.connect_target = None;
            s.drawing = None;
        });
    }

    fn end_gesture_state(&mut self) {
        self.set(|s| {
            s.guides = vec![];
            s.spacing = vec![];
            s.marquee = None;
            s.gesturing = false;
        });
    }

    pub fn pointer_up(&mut self) {
        if matches!(self.gesture, Some(Gesture::PathEdit { .. })) {
            self.path_up();
            return;
        }
        if self.state.tool == Tool::Pen && self.state.pen.as_ref().is_some_and(|p| p.dragging) {
            self.pen_up();
            return;
        }
        if self.state.drawing.is_some() {
            if self.state.tool == Tool::Erase {
                self.finish_erase();
            } else {
                self.finish_stroke();
            }
            return;
        }
        let g = self.gesture.take();
        if let Some(Gesture::Endpoint { id, end, .. }) = &g {
            let o = self.overrides.get(id).cloned();
            self.clear_overrides();
            let attach = self.state.connect_target.as_ref().map(|t| LineAttach { id: t.id.clone(), anchor: t.anchor.clone(), extra: Default::default() });
            if let Some(o) = o
                && let Some(xf) = o.get("xf")
            {
                let key = if *end == 0 { "startAttach" } else { "endAttach" };
                let mut patch = Map::new();
                if let Some(pts) = o.get("points") {
                    patch.insert("points".into(), pts.clone());
                }
                patch.insert(key.into(), attach.map_or(Value::Null, |a| jv(&a)));
                self.apply(vec![DocOp::Xf { id: id.clone(), xf: xf.clone() }, op_update(id, patch)], false);
            }
            self.set(|s| {
                s.connect_target = None;
                s.gesturing = false;
                s.guides = vec![];
            });
            return;
        }
        if let Some(Gesture::Move { origin, moved: true, duplicate: true, .. }) = &g
            && !self.overrides.is_empty()
        {
            let src_roots: Vec<String> = origin.keys().cloned().collect();
            let (elements, roots) = self.clone_subtrees(&src_roots);
            let ops: Vec<DocOp> = elements
                .into_iter()
                .map(|mut el| {
                    let eid = crate::util::s(el.get("id")).unwrap_or("").to_string();
                    let i = roots.iter().position(|r| *r == eid);
                    if let Some(i) = i
                        && let Some(xf) = self.overrides.get(&src_roots[i]).and_then(|o| o.get("xf"))
                    {
                        el.insert("xf".into(), xf.clone());
                    }
                    let parent = crate::util::s(el.get("parentId")).unwrap_or("").to_string();
                    op_add(el, &parent, i.map(|_| "top"))
                })
                .collect();
            self.clear_overrides();
            if self.apply(ops, false).ok {
                self.select(&roots, false);
            }
            self.end_gesture_state();
            return;
        }
        let is_move = matches!(g, Some(Gesture::Move { .. }));
        let is_rotate = matches!(g, Some(Gesture::Rotate { .. }));
        if (is_move || is_rotate) && self.state.record_keyframes && self.state.playhead.is_some() && !self.overrides.is_empty() {
            // Record mode: the gesture becomes keyframes at the playhead; the element's base position stays put.
            let doc = self.doc();
            let playhead = self.state.playhead.unwrap_or(0.0);
            let mut ops = Vec::new();
            for (id, patch) in self.overrides.clone() {
                let Some(el) = doc.el(&id) else { continue };
                let Some(pxf) = patch.get("xf").and_then(crate::util::parse::<Xf>) else { continue };
                let Some(page) = doc.page_of(&id) else { continue };
                let t = 0f64.max(round(playhead - self.page_start(&page)));
                let orig = doc.raw_el(&id).and_then(|m| m.get("keyframes")).and_then(Value::as_object).cloned();
                let mut k = orig.clone().unwrap_or_default();
                let mut changed = false;
                let mut put = |k: &mut Map<String, Value>, prop: &str, delta: f64| {
                    if delta.abs() < 0.01 {
                        return;
                    }
                    let neutral = keyframe_neutral(prop).unwrap_or(0.0);
                    let keys = super::timeline::keys_of(k, prop);
                    let cur = keyframe_value(keys.as_deref(), t, neutral);
                    *k = super::timeline::with_keyframe(k, prop, t, cur + delta, None);
                    changed = true;
                };
                let ex = el_xf(el);
                if is_move {
                    put(&mut k, "x", pxf.x - ex.x);
                    put(&mut k, "y", pxf.y - ex.y);
                } else {
                    put(&mut k, "rotate", pxf.rot - ex.rot);
                }
                if changed {
                    ops.push(op_update(&id, one("keyframes", Value::Object(k))));
                }
            }
            self.clear_overrides();
            if !ops.is_empty() {
                self.apply(ops, false);
            }
            self.end_gesture_state();
            return;
        }
        let commits = matches!(g, Some(Gesture::Move { .. } | Gesture::Resize { .. } | Gesture::Rotate { .. } | Gesture::CropPan { .. } | Gesture::ColResize { .. }));
        if commits && !self.overrides.is_empty() {
            let doc = self.doc();
            let mut ops = Vec::new();
            for (id, patch) in self.overrides.clone() {
                let Some(el) = doc.el(&id) else { continue };
                let mut rest = patch.clone();
                let xf = rest.remove("xf");
                // Dragging a whole connector frees its glued ends.
                if is_move && let Element::Line(l) = el
                    && (l.start_attach.is_some() || l.end_attach.is_some())
                {
                    rest.insert("startAttach".into(), Value::Null);
                    rest.insert("endAttach".into(), Value::Null);
                }
                if !rest.is_empty() {
                    ops.push(op_update(&id, rest));
                }
                if let Some(xf) = xf {
                    ops.push(DocOp::Xf { id: id.clone(), xf });
                }
            }
            self.clear_overrides();
            if !ops.is_empty() {
                self.apply(ops, false);
            }
        }
        self.clear_overrides();
        self.end_gesture_state();
    }

    pub fn double_click(&mut self, p: Vec2) {
        // Pen: a double-click finishes an open path (its second click added a node on top of the first one).
        if self.state.tool == Tool::Pen {
            if let Some(mut pen) = self.state.pen.clone() {
                let zoom = self.state.viewport.zoom.max(1e-6);
                let n = pen.nodes.len();
                if n >= 2 && hypot(pen.nodes[n - 1].p[0] - pen.nodes[n - 2].p[0], pen.nodes[n - 1].p[1] - pen.nodes[n - 2].p[1]) * zoom < 4.0 {
                    let last = pen.nodes.pop();
                    if let (Some(l), Some(prev)) = (last, pen.nodes.last_mut())
                        && l.cout.is_some()
                    {
                        prev.cout = l.cout;
                    }
                    self.set(|s| s.pen = Some(pen));
                }
                self.pen_finish(false);
            }
            return;
        }
        let doc = self.doc();
        let Some(slot) = page_at_scene(&doc.doc, p) else { return };
        let page_id = slot.page.id.clone();
        let local = self.to_page(&page_id, p);
        let sel = self.selected();
        // Double-click on a group enters it.
        if sel.len() == 1
            && let Element::Group(g) = &sel[0]
        {
            let gid = g.base.id.clone();
            let inner = hit_test(doc.els(), &page_id, local, HitTestOpts { tolerance: 0.0, enter_group: Some(&gid) }).cloned();
            if let Some(inner) = inner
                && inner.id() != Some(gid.as_str())
            {
                self.state.entered_group = Some(gid.clone());
                self.select(&[el_id(&inner)], false);
                self.state.entered_group = Some(gid);
                self.emit();
                if matches!(inner, Element::Text(_)) {
                    self.start_text_edit(&el_id(&inner), Some(local), false);
                }
                return;
            }
        }
        let hit = hit_test(doc.els(), &page_id, local, HitTestOpts { tolerance: 0.0, enter_group: self.state.entered_group.as_deref() }).cloned();
        let Some(hit) = hit else { return };
        let hid = el_id(&hit);
        match &hit {
            Element::Text(_) => self.start_text_edit(&hid, Some(local), true),
            Element::Table(_) => {
                if let Some(c) = self.cell_at(&hid, local) {
                    self.start_cell_edit(&hid, c.r, c.c, Some(local), true);
                }
            }
            Element::Image(_) => self.start_crop(&hid),
            Element::Frame(f) if f.content.is_some() => self.start_crop(&hid),
            Element::Shape(_) if self.media_crop(&hit).is_some() => self.start_crop(&hid),
            // Paths (DS-100): double-click edits their nodes.
            Element::Shape(sh) if sh.kind == oc_schema::elements::ShapeKind::Custom => {
                self.start_path_edit(&hid);
            }
            Element::Shape(_) if !is_section(Some(&hit)) => self.edit_shape_label(&hid),
            _ => {}
        }
    }
}

/// Elements with the gesture's start state swapped in (`originDoc`).
fn origin_doc(elements: &Elements, origin: &IndexMap<String, Element>) -> Elements {
    let mut out = elements.clone();
    for (id, el) in origin {
        out.insert(id.clone(), el.clone());
    }
    out
}

/// Line patch whose box wraps two page-space ends (top-level or inside a group).
pub(crate) fn line_from_ends(elements: &Elements, el: &Element, a: Vec2, b: Vec2) -> Map<String, Value> {
    let inv = parent_inverse(elements, el);
    let pa = apply(&inv, a);
    let pb = apply(&inv, b);
    let x = pa[0].min(pb[0]);
    let y = pa[1].min(pb[1]);
    let w = crate::util::max(1.0, (pb[0] - pa[0]).abs());
    let h = crate::util::max(1.0, (pb[1] - pa[1]).abs());
    let mut m = one("xf", jv(&Xf::new(x, y, w, h, 0.0)));
    m.insert("points".into(), jv(&[[pa[0] - x, pa[1] - y], [pb[0] - x, pb[1] - y]]));
    m
}
