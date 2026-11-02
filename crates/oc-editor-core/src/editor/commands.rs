// SPDX-License-Identifier: Apache-2.0
//! Element commands: insert/delete, clipboard, z-order, align/distribute, grouping, locks, crop, copy style,
//! find & replace, exact geometry, guides, grids/frames, bulk create, pages, design styles, history.

use std::collections::HashMap;

use indexmap::IndexMap;
use oc_crdt::{DocOp, PageOp};
use oc_geometry::{
    AlignEdge, AlignItem, Axis, HitTestOpts, align_deltas, apply, apply_vector, distribute_deltas, hit_test, invert, local_matrix,
    multiply, normalize_angle, parent_inverse, selection_bounds, world_bounds, world_matrix, xf_from_matrix,
};
use oc_schema::primitives::Unit;
use oc_schema::elements::{Element, GroupEl};
use oc_schema::factory::{TextOpts, make_group, make_text, grid_cell_box};
use oc_schema::primitives::{ImageFit, Paint, Rect, StoredRect, Vec2, Xf};
use oc_schema::text::{RunStyleOverride, TextAlign, body_plain_text};
use oc_schema::tree::{children_of, descendants, sorted_pages};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{Editor, el_id, el_xf, lock_of, locked_content, locked_move, one, op_add, op_meta, op_page_update, op_update, op_xf};
use crate::state::*;
use crate::styles::{FontSet, font_set_ops, palette_map, recolor_ops};
use crate::transform::{refit_crop, scale_patches};
use crate::util::{byte_to_utf16, jm, jv, len16, max, min, num, round, s, slice16};

/// Crop rect (normalized) that makes an image of `img_aspect` cover a box of `box_aspect`.
pub fn cover_crop(box_aspect: f64, img_aspect: f64) -> Rect {
    if img_aspect > box_aspect {
        let w = box_aspect / img_aspect;
        return Rect::new((1.0 - w) / 2.0, 0.0, w, 1.0);
    }
    let h = img_aspect / box_aspect;
    Rect::new(0.0, (1.0 - h) / 2.0, 1.0, h)
}

/// Page-number placement (ED-095).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PageNumberPosition {
    #[default]
    BottomRight,
    BottomCenter,
    BottomLeft,
    TopRight,
}

/// Z-order commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arrange {
    Forward,
    Backward,
    Front,
    Back,
}

/// Exact geometry (position panel, ED-063); page-space.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct GeometryPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rot: Option<f64>,
}

/// Find options.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindOpts {
    #[serde(default)]
    pub match_case: bool,
    #[serde(default)]
    pub whole_word: bool,
    /// `replace` only: replace just this match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only: Option<FindMatch>,
}

/// An image resolved for bulk create.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedImage {
    pub id: String,
    pub w: f64,
    pub h: f64,
}

fn xf_of(m: &Map<String, Value>) -> Xf {
    m.get("xf").and_then(crate::util::parse::<Xf>).unwrap_or_default()
}

fn set_xf(m: &mut Map<String, Value>, xf: &Xf) {
    m.insert("xf".into(), jv(xf));
}

impl Editor {
    /// Page numbers (ED-095): a `{{page}}` text box at the same spot on every page (skipping pages that already
    /// have one). One undo step. Returns the number of boxes added.
    pub fn add_page_numbers(&mut self, text: Option<&str>, position: Option<PageNumberPosition>, size: Option<f64>) -> usize {
        let doc = self.doc();
        let (w, h) = (doc.doc.meta.dims.w, doc.doc.meta.dims.h);
        let size = size.unwrap_or_else(|| max(10.0, round(min(w, h) / 40.0)));
        let bw = size * 8.0;
        let m = round(min(w, h) * 0.04);
        let pos = position.unwrap_or_default();
        let x = match pos {
            PageNumberPosition::BottomLeft => m,
            PageNumberPosition::BottomCenter => (w - bw) / 2.0,
            _ => w - m - bw,
        };
        let y = if pos == PageNumberPosition::TopRight { m } else { h - m - size * 1.4 };
        let align = match pos {
            PageNumberPosition::BottomLeft => TextAlign::Start,
            PageNumberPosition::BottomCenter => TextAlign::Center,
            _ => TextAlign::End,
        };
        let mut ops = Vec::new();
        for page in sorted_pages(&doc.doc.pages) {
            let has = children_of(doc.els(), &page.id)
                .iter()
                .any(|e| matches!(e, Element::Text(t) if body_plain_text(&t.body).contains("{{page}}")));
            if has {
                continue;
            }
            let base = self.fresh_base(Xf::new(x, y, bw, size * 1.4, 0.0));
            let el = make_text(
                base,
                TextOpts {
                    text: Some(text.unwrap_or("{{page}}").to_string()),
                    style: Some(RunStyleOverride { size: Some(size), ..Default::default() }),
                    align: Some(align.clone()),
                    body: None,
                },
            );
            let mut m = jm(&Element::Text(el));
            m.insert("name".into(), Value::String("Page number".into()));
            ops.push(op_add(m, &page.id, None));
        }
        let n = ops.len();
        if n > 0 {
            self.apply(ops, false);
        }
        n
    }

    /// Insert elements (JSON) on the active page, centred (or at a page point), and select them.
    pub fn insert(&mut self, els: Vec<Map<String, Value>>, at: Option<Vec2>, select: bool, edit: bool) -> Vec<String> {
        let page_id = self.state.active_page_id.clone();
        let doc = self.doc();
        let (w, h) = (doc.doc.meta.dims.w, doc.doc.meta.dims.h);
        if els.is_empty() {
            return vec![];
        }
        let id_of = |e: &Map<String, Value>| s(e.get("id")).unwrap_or("").to_string();
        let parent_of = |e: &Map<String, Value>| s(e.get("parentId")).unwrap_or("").to_string();
        let ids: Vec<String> = els.iter().map(id_of).collect();
        let roots: Vec<&Map<String, Value>> =
            els.iter().filter(|e| !ids.contains(&parent_of(e)) || parent_of(e) == page_id).collect();
        let bx = roots.iter().map(|e| xf_of(e).x).fold(f64::INFINITY, min);
        let by = roots.iter().map(|e| xf_of(e).y).fold(f64::INFINITY, min);
        let bw = roots.iter().map(|e| xf_of(e).x + xf_of(e).w).fold(f64::NEG_INFINITY, max) - bx;
        let bh = roots.iter().map(|e| xf_of(e).y + xf_of(e).h).fold(f64::NEG_INFINITY, max) - by;
        let target = at.unwrap_or([w / 2.0, h / 2.0]);
        let dx = target[0] - (bx + bw / 2.0);
        let dy = target[1] - (by + bh / 2.0);
        let is_root = |e: &Map<String, Value>| !ids.contains(&parent_of(e));
        let ops: Vec<DocOp> = els
            .iter()
            .map(|el| {
                if is_root(el) {
                    let mut m = el.clone();
                    let x = xf_of(el);
                    set_xf(&mut m, &Xf { x: x.x + dx, y: x.y + dy, ..x });
                    op_add(m, &page_id, None)
                } else {
                    op_add(el.clone(), &parent_of(el), None)
                }
            })
            .collect();
        let r = self.apply(ops, false);
        let added: Vec<String> = els.iter().filter(|e| is_root(e)).map(id_of).collect();
        if r.ok && select {
            self.select(&added, false);
            if edit && added.len() == 1 && s(els[0].get("type")) == Some("text") {
                let id = added[0].clone();
                self.start_text_edit(&id, None, false);
            }
        }
        added
    }

    pub fn delete_selection(&mut self) {
        if self.state.editing_text.is_some() {
            return;
        }
        let doc = self.doc();
        let ids: Vec<String> = self
            .state
            .selection
            .iter()
            .filter(|id| !doc.el(id).and_then(lock_of).is_some_and(|l| l.delete == Some(true)))
            .cloned()
            .collect();
        if ids.is_empty() {
            return;
        }
        self.apply(ids.into_iter().map(|id| DocOp::Delete { id }).collect(), false);
        self.set(|s| {
            s.selection = vec![];
            s.cropping_id = None;
        });
    }

    /// Deep copy of subtrees with fresh ids (JSON); roots keep their parent.
    pub(crate) fn clone_subtrees(&mut self, ids: &[String]) -> (Vec<Map<String, Value>>, Vec<String>) {
        let doc = self.doc();
        let mut all: Vec<Map<String, Value>> = Vec::new();
        for id in ids {
            let Some(raw) = doc.raw_el(id) else { continue };
            all.push(raw.clone());
            for d in descendants(doc.els(), id) {
                if let Some(r) = doc.raw_el(d.id().unwrap_or("")) {
                    all.push(r.clone());
                }
            }
        }
        let mut remap: HashMap<String, String> = HashMap::new();
        for e in &all {
            let nid = self.fresh_id();
            remap.insert(s(e.get("id")).unwrap_or("").to_string(), nid);
        }
        let elements = all
            .into_iter()
            .map(|mut e| {
                let id = s(e.get("id")).unwrap_or("").to_string();
                let parent = s(e.get("parentId")).unwrap_or("").to_string();
                e.insert("id".into(), Value::String(remap.get(&id).cloned().unwrap_or_default()));
                e.insert("parentId".into(), Value::String(remap.get(&parent).cloned().unwrap_or(parent)));
                e
            })
            .collect();
        let roots = ids.iter().filter_map(|id| remap.get(id).cloned()).collect();
        (elements, roots)
    }

    pub fn duplicate(&mut self) {
        let sel = self.state.selection.clone();
        if sel.is_empty() {
            return;
        }
        let (elements, roots) = self.clone_subtrees(&sel);
        let ops = elements
            .into_iter()
            .map(|mut el| {
                let is_root = roots.iter().any(|r| Some(r.as_str()) == s(el.get("id")));
                let parent = s(el.get("parentId")).unwrap_or("").to_string();
                if is_root {
                    let x = xf_of(&el);
                    set_xf(&mut el, &Xf { x: x.x + 20.0, y: x.y + 20.0, ..x });
                }
                op_add(el, &parent, is_root.then_some("top"))
            })
            .collect();
        if self.apply(ops, false).ok {
            self.select(&roots, false);
        }
    }

    /// Copy the selection to the internal clipboard (and return it for the system clipboard).
    pub fn copy(&mut self) -> Option<ClipboardPayload> {
        if self.state.selection.is_empty() {
            return None;
        }
        let sel = self.state.selection.clone();
        let (elements, roots) = self.clone_subtrees(&sel);
        let mut asset_ids: Vec<String> = Vec::new();
        for e in &elements {
            match s(e.get("type")) {
                Some("image") | Some("video") => asset_ids.extend(s(e.get("asset")).map(str::to_string)),
                Some("frame") => asset_ids.extend(e.get("content").and_then(|c| s(c.get("asset"))).map(str::to_string)),
                Some("grid") => {
                    for c in e.get("cells").and_then(Value::as_array).into_iter().flatten() {
                        if let Some(a) = s(c.get("asset")).filter(|a| !a.is_empty()) {
                            asset_ids.push(a.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        let doc = self.doc();
        let assets: Map<String, Value> = doc.snap.assets.iter().filter(|(id, _)| asset_ids.contains(id)).map(|(k, v)| (k.clone(), v.clone())).collect();
        // Roots are re-expressed in page space so they paste correctly from inside groups.
        let els = elements
            .into_iter()
            .map(|mut e| {
                let eid = s(e.get("id")).unwrap_or("").to_string();
                let Some(i) = roots.iter().position(|r| *r == eid) else { return e };
                let src = &sel[i];
                let m = world_matrix(doc.els(), src);
                let x = xf_of(&e);
                let w = xf_from_matrix(&m, x.w, x.h);
                set_xf(&mut e, &Xf { x: w.x, y: w.y, rot: w.rot, ..x });
                e
            })
            .collect();
        self.clipboard = Some(ClipboardPayload { kind: CLIPBOARD_MIME.into(), elements: els, roots, assets });
        self.clipboard.clone()
    }

    pub fn cut(&mut self) -> Option<ClipboardPayload> {
        let p = self.copy();
        self.delete_selection();
        p
    }

    /// Paste a payload (`None` = the internal clipboard). Ids are refreshed every time. TS default `offset = 20`.
    pub fn paste(&mut self, payload: Option<&ClipboardPayload>, offset: f64) {
        let internal = payload.is_none();
        let Some(payload) = payload.cloned().or_else(|| self.clipboard.clone()) else { return };
        if payload.elements.is_empty() {
            return;
        }
        let mut remap: HashMap<String, String> = HashMap::new();
        for e in &payload.elements {
            let nid = self.fresh_id();
            remap.insert(s(e.get("id")).unwrap_or("").to_string(), nid);
        }
        let page_id = self.state.active_page_id.clone();
        let mut ops: Vec<DocOp> = payload.assets.values().map(|a| DocOp::Asset { asset: a.clone() }).collect();
        let mut roots = Vec::new();
        for e in &payload.elements {
            let eid = s(e.get("id")).unwrap_or("");
            let is_root = payload.roots.iter().any(|r| r == eid);
            let id = remap.get(eid).cloned().unwrap_or_default();
            if is_root {
                roots.push(id.clone());
            }
            let parent = if is_root {
                page_id.clone()
            } else {
                remap.get(s(e.get("parentId")).unwrap_or("")).cloned().unwrap_or_default()
            };
            let mut m = e.clone();
            m.insert("id".into(), Value::String(id));
            m.insert("parentId".into(), Value::String(parent.clone()));
            if is_root {
                let x = xf_of(e);
                set_xf(&mut m, &Xf { x: x.x + offset, y: x.y + offset, ..x });
            }
            ops.push(op_add(m, &parent, None));
        }
        if self.apply(ops, false).ok {
            self.select(&roots, false);
        }
        if internal {
            let mut next = payload.clone();
            for e in next.elements.iter_mut() {
                if payload.roots.iter().any(|r| Some(r.as_str()) == s(e.get("id"))) {
                    let x = xf_of(e);
                    set_xf(e, &Xf { x: x.x + offset, y: x.y + offset, ..x });
                }
            }
            self.clipboard = Some(next);
        }
    }

    /// The internal clipboard.
    pub fn clipboard(&self) -> Option<&ClipboardPayload> {
        self.clipboard.as_ref()
    }

    pub fn nudge(&mut self, dx: f64, dy: f64) {
        let doc = self.doc();
        let moving: Vec<Element> = self.selected();
        let ops: Vec<DocOp> = moving
            .iter()
            .filter(|e| !locked_move(e))
            .map(|e| {
                let [pdx, pdy] = apply_vector(&parent_inverse(doc.els(), e), [dx, dy]);
                let x = el_xf(e);
                op_xf(&el_id(e), &Xf { x: x.x + pdx, y: x.y + pdy, ..x })
            })
            .collect();
        if !ops.is_empty() {
            self.apply(ops, true);
        }
    }

    pub fn arrange(&mut self, where_: Arrange) {
        let doc = self.doc();
        let sel = self.selected();
        let ordered: Vec<&Element> =
            if matches!(where_, Arrange::Forward | Arrange::Back) { sel.iter().rev().collect() } else { sel.iter().collect() };
        let mut ops = Vec::new();
        for el in ordered {
            let sibs = children_of(doc.els(), el.parent_id().unwrap_or(""));
            let i = sibs.iter().position(|x| x.id() == el.id()).map_or(-1, |i| i as i64);
            let id = el_id(el);
            match where_ {
                Arrange::Front => ops.push(DocOp::Reorder { id, after: "top".into() }),
                Arrange::Back => ops.push(DocOp::Reorder { id, after: "bottom".into() }),
                Arrange::Forward if i < sibs.len() as i64 - 1 => ops.push(DocOp::Reorder { id, after: el_id(sibs[(i + 1) as usize]) }),
                Arrange::Backward if i > 0 => {
                    let after = if i > 1 { el_id(sibs[(i - 2) as usize]) } else { "bottom".into() };
                    ops.push(DocOp::Reorder { id, after });
                }
                _ => {}
            }
        }
        if !ops.is_empty() {
            self.apply(ops, false);
        }
    }

    /// Move an element to a z-position among its siblings (layers panel drag): after an id, `"top"` or `"bottom"`.
    pub fn reorder_to(&mut self, id: &str, after: &str) {
        self.apply(vec![DocOp::Reorder { id: id.into(), after: after.into() }], false);
    }

    pub fn align(&mut self, edge: AlignEdge) {
        let sel = self.selected();
        if sel.is_empty() {
            return;
        }
        let doc = self.doc();
        let items: Vec<AlignItem> =
            sel.iter().map(|e| AlignItem { id: el_id(e), bounds: world_bounds(doc.els(), &el_id(e)).unwrap_or_default() }).collect();
        let to = if sel.len() == 1 {
            Rect::new(0.0, 0.0, doc.doc.meta.dims.w, doc.doc.meta.dims.h)
        } else {
            let ids: Vec<String> = sel.iter().map(el_id).collect();
            selection_bounds(doc.els(), &ids).unwrap_or_default()
        };
        self.apply_deltas(align_deltas(&items, edge, &to));
    }

    pub fn distribute(&mut self, axis: Axis) {
        let doc = self.doc();
        let items: Vec<AlignItem> = self
            .selected()
            .iter()
            .map(|e| AlignItem { id: el_id(e), bounds: world_bounds(doc.els(), &el_id(e)).unwrap_or_default() })
            .collect();
        self.apply_deltas(distribute_deltas(&items, axis));
    }

    /// Tidy: arrange the selection in a grid with equal gaps.
    pub fn tidy(&mut self) {
        let doc = self.doc();
        let sel = self.selected();
        if sel.len() < 2 {
            return;
        }
        let mut items: Vec<(String, Rect)> = sel.iter().map(|e| (el_id(e), world_bounds(doc.els(), &el_id(e)).unwrap_or_default())).collect();
        items.sort_by(|a, b| crate::util::num_cmp(a.1.y, b.1.y).then_with(|| crate::util::num_cmp(a.1.x, b.1.x)));
        let cols = (items.len() as f64).sqrt().ceil() as usize;
        let cw = items.iter().map(|i| i.1.w).fold(f64::NEG_INFINITY, max);
        let ch = items.iter().map(|i| i.1.h).fold(f64::NEG_INFINITY, max);
        let gap = max(8.0, min(cw, ch) * 0.1);
        let ids: Vec<String> = sel.iter().map(el_id).collect();
        let origin = selection_bounds(doc.els(), &ids).unwrap_or_default();
        let mut deltas: IndexMap<String, Vec2> = IndexMap::new();
        for (i, (id, b)) in items.iter().enumerate() {
            let cx = origin.x + (i % cols) as f64 * (cw + gap) + (cw - b.w) / 2.0;
            let cy = origin.y + (i / cols) as f64 * (ch + gap) + (ch - b.h) / 2.0;
            deltas.insert(id.clone(), [cx - b.x, cy - b.y]);
        }
        self.apply_deltas(deltas);
    }

    fn apply_deltas(&mut self, deltas: IndexMap<String, Vec2>) {
        let doc = self.doc();
        let mut ops = Vec::new();
        for (id, [dx, dy]) in deltas {
            let Some(el) = doc.el(&id) else { continue };
            if locked_move(el) || (dx.abs() < 1e-6 && dy.abs() < 1e-6) {
                continue;
            }
            let [pdx, pdy] = apply_vector(&parent_inverse(doc.els(), el), [dx, dy]);
            let x = el_xf(el);
            ops.push(op_xf(&id, &Xf { x: x.x + pdx, y: x.y + pdy, ..x }));
        }
        if !ops.is_empty() {
            self.apply(ops, false);
        }
    }

    pub fn flip(&mut self, axis: Axis) {
        self.update_selected_with(
            &mut |e| {
                let mut x = el_xf(e);
                match axis {
                    Axis::X => x.flip_x = Some(!x.flip_x.unwrap_or(false)),
                    Axis::Y => x.flip_y = Some(!x.flip_y.unwrap_or(false)),
                }
                Some(one("xf", jv(&x)))
            },
            false,
            false,
        );
    }

    pub fn toggle_lock(&mut self) {
        let sel: Vec<Element> = self.selected();
        let any_unlocked = sel.iter().any(|e| !locked_move(e));
        if sel.is_empty() {
            return;
        }
        let lock = if any_unlocked {
            let mut m = Map::new();
            for k in ["move", "resize", "rotate", "delete"] {
                m.insert(k.into(), Value::Bool(true));
            }
            m.insert("by".into(), Value::String("user".into()));
            Value::Object(m)
        } else {
            Value::Null
        };
        self.apply(sel.iter().map(|e| op_update(&el_id(e), one("lock", lock.clone()))).collect(), false);
    }

    // ------------------------------------------------------------------ grouping

    pub fn group(&mut self) {
        self.group_with(None);
    }

    /// Group the selection (same parent, at least two). `mask` (DS-107) makes the group a masked group.
    pub(crate) fn group_with(&mut self, mask: Option<Value>) -> Option<String> {
        let doc = self.doc();
        let sel = self.selected();
        if sel.len() < 2 {
            return None;
        }
        let parent = sel[0].parent_id().unwrap_or("").to_string();
        if sel.iter().any(|e| e.parent_id() != Some(parent.as_str())) {
            return None;
        }
        let ids: Vec<String> = sel.iter().map(el_id).collect();
        let b = selection_bounds(doc.els(), &ids)?;
        // Group box is axis-aligned in the parent's space.
        let [gx, gy] = match doc.el(&parent) {
            Some(p) => apply(&invert(&world_matrix(doc.els(), p.id().unwrap_or(""))), [b.x, b.y]),
            None => [b.x, b.y],
        };
        let base = self.fresh_base(Xf::new(gx, gy, b.w, b.h, 0.0));
        let g: GroupEl = make_group(base);
        let gid = g.base.id.clone();
        let ordered: Vec<&Element> = children_of(doc.els(), &parent).into_iter().filter(|e| ids.iter().any(|i| Some(i.as_str()) == e.id())).collect();
        let top = ordered.last()?;
        let mut gm = jm(&Element::Group(g));
        if let Some(m) = mask {
            gm.insert("mask".into(), m);
        }
        let mut ops = vec![op_add(gm, &parent, Some(top.id().unwrap_or("")))];
        let mut prev = "bottom".to_string();
        for el in &ordered {
            let x = el_xf(el);
            ops.push(DocOp::Reparent {
                id: el_id(el),
                parent: gid.clone(),
                after: Some(prev.clone()),
                xf: Some(jv(&Xf { x: x.x - gx, y: x.y - gy, ..x })),
            });
            prev = el_id(el);
        }
        if self.apply(ops, false).ok {
            self.select(std::slice::from_ref(&gid), false);
            return Some(gid);
        }
        None
    }

    pub fn ungroup(&mut self) {
        let doc = self.doc();
        let mut ops = Vec::new();
        let mut released = Vec::new();
        for g in self.selected() {
            let Element::Group(grp) = &g else { continue };
            let gm = local_matrix(&grp.base.xf);
            let mut prev = grp.base.id.clone();
            for c in children_of(doc.els(), &grp.base.id) {
                let cx = el_xf(c);
                let xf = xf_from_matrix(&multiply(&gm, &local_matrix(&cx)), cx.w, cx.h);
                ops.push(DocOp::Reparent { id: el_id(c), parent: grp.base.parent_id.clone(), after: Some(prev.clone()), xf: Some(jv(&xf)) });
                prev = el_id(c);
                released.push(el_id(c));
            }
            ops.push(DocOp::Delete { id: grp.base.id.clone() });
        }
        if !ops.is_empty() && self.apply(ops, false).ok {
            self.select(&released, false);
        }
    }

    // ------------------------------------------------------------------ crop

    pub fn start_crop(&mut self, id: &str) {
        let doc = self.doc();
        if !doc.el(id).is_some_and(|el| self.media_crop(el).is_some()) {
            return;
        }
        self.store_mut().checkpoint();
        let id = id.to_string();
        self.set(|s| {
            s.cropping_id = Some(id.clone());
            s.selection = vec![id];
        });
    }

    pub fn stop_crop(&mut self) {
        self.set(|s| s.cropping_id = None);
    }

    /// Zoom the image inside its crop (crop-mode slider). 1 = the image just covers the box.
    pub fn set_crop_zoom(&mut self, id: &str, zoom: f64) {
        let doc = self.doc();
        let Some(el) = doc.el(id) else { return };
        let Some(crop) = self.media_crop(el) else { return };
        let x = el_xf(el);
        let box_aspect = x.w / x.h;
        let img_aspect = (box_aspect * crop.h) / crop.w;
        let fit = cover_crop(box_aspect, img_aspect);
        let z = max(1.0, zoom);
        let w = fit.w / z;
        let h = fit.h / z;
        let cx = crop.x + crop.w / 2.0;
        let cy = crop.y + crop.h / 2.0;
        let next = Rect::new(min(1.0 - w, max(0.0, cx - w / 2.0)), min(1.0 - h, max(0.0, cy - h / 2.0)), w, h);
        if let Some(patch) = self.crop_patch(el, &next.into()) {
            self.apply(vec![op_update(id, patch)], false);
        }
    }

    pub fn crop_zoom(&self, id: &str) -> f64 {
        let doc = self.doc();
        let Some(el) = doc.el(id) else { return 1.0 };
        let Some(crop) = self.media_crop(el) else { return 1.0 };
        let x = el_xf(el);
        let box_aspect = x.w / x.h;
        let fit = cover_crop(box_aspect, (box_aspect * crop.h) / crop.w);
        fit.w / crop.w
    }

    /// The part of the photo shown in an image, a filled frame or a shape filled with a photo (its topmost image
    /// fill). A shape's photo never stretches: its stored crop is refit to the box's aspect (oc-render
    /// `shape_photo` does the same). Without a stored crop: the cover crop of a `fill`-fit photo of known size.
    /// `None` when there is none. Port of `Editor.mediaCrop`.
    pub fn media_crop(&self, el: &Element) -> Option<StoredRect> {
        match el {
            Element::Image(i) => Some(i.crop.clone()),
            Element::Frame(f) => f.content.as_ref().map(|c| c.crop.clone()),
            Element::Shape(sh) => {
                let fill = sh.fills.iter().rev().find_map(|p| match p {
                    Paint::Image(ip) => Some(ip),
                    _ => None,
                })?;
                let doc = self.doc();
                let a = doc.doc.assets.get(&fill.asset);
                let t = |x: f64| x != 0.0 && !x.is_nan();
                let xf = &sh.base.xf;
                let sized = a.is_some_and(|a| t(a.w) && t(a.h)) && t(xf.w) && t(xf.h);
                if let Some(c) = &fill.crop
                    && [c.x, c.y, c.w, c.h].iter().all(|v| v.is_finite())
                    && c.w > 0.0
                    && c.h > 0.0
                {
                    return Some(match a {
                        Some(a) if sized => refit_crop(c, (c.w * a.w, c.h * a.h), (xf.w, xf.h)),
                        _ => c.clone(),
                    });
                }
                match a {
                    Some(a) if sized && fill.fit == ImageFit::Fill => Some(cover_crop(xf.w / xf.h, a.w / a.h).into()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Patch that sets the crop `media_crop` reads. Port of `Editor.cropPatch`.
    pub(crate) fn crop_patch(&self, el: &Element, crop: &StoredRect) -> Option<Map<String, Value>> {
        match el {
            Element::Image(_) => Some(one("crop", jv(crop))),
            Element::Frame(f) => f.content.as_ref().map(|c| {
                let mut m = jm(c);
                m.insert("crop".into(), jv(crop));
                one("content", Value::Object(m))
            }),
            Element::Shape(sh) => {
                let i = sh.fills.iter().rposition(|p| matches!(p, Paint::Image(_)))?;
                let fills: Vec<Value> = sh
                    .fills
                    .iter()
                    .enumerate()
                    .map(|(j, p)| {
                        let mut v = jv(p);
                        if j == i
                            && let Value::Object(m) = &mut v
                        {
                            m.insert("crop".into(), jv(crop));
                        }
                        v
                    })
                    .collect();
                Some(one("fills", Value::Array(fills)))
            }
            _ => None,
        }
    }

    /// Put an image into a frame (drop onto frame) or replace an image's asset, keeping the box.
    pub fn replace_image(&mut self, id: &str, asset: &str, aspect: f64) {
        let doc = self.doc();
        let Some(el) = doc.el(id) else { return };
        let x = el_xf(el);
        let crop = cover_crop(x.w / x.h, aspect);
        match el {
            Element::Image(img) => {
                let mut patch = one("asset", Value::String(asset.into()));
                patch.insert("crop".into(), jv(&crop));
                // A recolour map belongs to the old SVG's colours (A4).
                if img.svg_colors.is_some() {
                    patch.insert("svgColors".into(), Value::Null);
                }
                self.apply(vec![op_update(id, patch)], false);
            }
            Element::Frame(_) => self.fill_frame(id, asset, Some(aspect)),
            _ => {}
        }
    }

    // ------------------------------------------------------------------ copy style (ED-070)

    /// Pick up the style of the first selected element; the next click (or `paste_style`) applies it.
    pub fn copy_style(&mut self) -> Option<CopiedStyle> {
        let el = self.selected().into_iter().next()?;
        let raw = jm(&el);
        let pick = |m: &Map<String, Value>, keys: &[&str]| -> Map<String, Value> {
            keys.iter().filter_map(|k| m.get(*k).filter(|v| !v.is_null()).map(|v| (k.to_string(), v.clone()))).collect()
        };
        let mut style = CopiedStyle {
            from: el.type_name().to_string(),
            base: pick(&raw, &["opacity", "blend", "effects"]),
            text: None,
            shape: None,
            image: None,
            line: None,
        };
        match &el {
            Element::Text(t) => {
                let st = self.current_text_style().unwrap_or_else(|| jm(&t.body.default_run));
                let run = pick(&st, &["family", "weight", "italic", "size", "fill", "tracking", "underline", "strike", "caseTransform", "highlight"]);
                let paragraph = t.body.paragraphs.first().map(|p| pick(&jm(p), &["align", "lineHeight", "spaceAfter"])).unwrap_or_default();
                style.text = Some(CopiedText { run, paragraph, effect: raw.get("effect").cloned() });
            }
            Element::Shape(sh) => {
                let mut m = pick(&raw, &["fills", "strokes"]);
                if let Some(cr) = sh.params.get("cornerRadius") {
                    m.insert("cornerRadius".into(), num(*cr));
                }
                style.shape = Some(m);
            }
            Element::Image(_) => style.image = Some(pick(&raw, &["edits", "cornerRadius"])),
            Element::Frame(f) => {
                let mut m = Map::new();
                if let Some(e) = f.content.as_ref().and_then(|c| c.edits.as_ref()) {
                    m.insert("edits".into(), jv(e));
                }
                style.image = Some(m);
            }
            Element::Line(_) => style.line = Some(pick(&raw, &["stroke", "startMarker", "endMarker", "route"])),
            _ => {}
        }
        let st = style.clone();
        self.set(|s| s.style_brush = Some(st));
        Some(style)
    }

    pub fn cancel_style_brush(&mut self) {
        if self.state.style_brush.is_some() {
            self.set(|s| s.style_brush = None);
        }
    }

    /// Apply a copied style to elements (defaults: the selection and the armed style). Incompatible parts are skipped.
    pub fn paste_style(&mut self, ids: &[String], style: Option<&CopiedStyle>) {
        let Some(style) = style.cloned() else { return };
        let doc = self.doc();
        let mut ops = Vec::new();
        for id in ids {
            let Some(el) = doc.el(id) else { continue };
            if lock_of(el).is_some_and(|l| l.style == Some(true)) {
                continue;
            }
            let b = &style.base;
            let mut patch = one("opacity", b.get("opacity").cloned().unwrap_or(num(1.0)));
            patch.insert("blend".into(), b.get("blend").cloned().unwrap_or(Value::Null));
            patch.insert("effects".into(), b.get("effects").cloned().unwrap_or(Value::Null));
            match el {
                Element::Text(t) => {
                    if let Some(tx) = &style.text {
                        let len = self.text_len(t);
                        let mut marks = Map::new();
                        for k in ["family", "weight", "italic", "size", "fill", "tracking"] {
                            if let Some(v) = tx.run.get(k) {
                                marks.insert(k.into(), v.clone());
                            }
                        }
                        for k in ["underline", "strike", "caseTransform", "highlight"] {
                            marks.insert(k.into(), tx.run.get(k).cloned().unwrap_or(Value::Null));
                        }
                        ops.push(DocOp::TextFormat { id: id.clone(), marks, offset: Some(0.0), len: Some(len as f64) });
                        ops.push(DocOp::TextParagraph { id: id.clone(), attrs: tx.paragraph.clone(), offset: Some(0.0), len: Some(len as f64) });
                        patch.insert("effect".into(), tx.effect.clone().unwrap_or(Value::Null));
                    }
                }
                Element::Shape(sh) => {
                    if let Some(s) = &style.shape {
                        patch.insert("fills".into(), s.get("fills").cloned().unwrap_or(Value::Null));
                        patch.insert("strokes".into(), s.get("strokes").cloned().unwrap_or(Value::Null));
                        if let Some(cr) = s.get("cornerRadius") {
                            let mut p = jm(&sh.params);
                            p.insert("cornerRadius".into(), cr.clone());
                            patch.insert("params".into(), Value::Object(p));
                        }
                    }
                }
                Element::Image(_) | Element::Video(_) => {
                    if let Some(im) = &style.image {
                        if matches!(el, Element::Image(_)) {
                            patch.insert("edits".into(), im.get("edits").cloned().unwrap_or(Value::Null));
                        }
                        if let Some(cr) = im.get("cornerRadius") {
                            patch.insert("cornerRadius".into(), cr.clone());
                        }
                    }
                }
                Element::Frame(f) => {
                    if let (Some(im), Some(c)) = (&style.image, &f.content) {
                        let mut m = jm(c);
                        match im.get("edits") {
                            Some(e) => m.insert("edits".into(), e.clone()),
                            None => m.remove("edits"),
                        };
                        patch.insert("content".into(), Value::Object(m));
                    }
                }
                Element::Line(_) => {
                    if let Some(l) = &style.line {
                        for k in ["stroke", "startMarker", "endMarker", "route"] {
                            patch.insert(k.into(), l.get(k).cloned().unwrap_or(Value::Null));
                        }
                    }
                }
                _ => {}
            }
            ops.push(op_update(id, patch));
        }
        if !ops.is_empty() {
            self.apply(ops, false);
        }
    }

    // ------------------------------------------------------------------ find & replace (ED-072)

    /// Case-insensitive (by default) search across every text element of the design.
    pub fn find(&self, query: &str, opts: &FindOpts) -> Vec<FindMatch> {
        if query.is_empty() {
            return vec![];
        }
        let esc = regex::escape(query);
        let pat = if opts.whole_word { format!(r"(?-u:\b){esc}(?-u:\b)") } else { esc };
        let pat = if opts.match_case { pat } else { format!("(?i){pat}") };
        let Ok(re) = regex::Regex::new(&pat) else { return vec![] };
        let doc = self.doc();
        let mut out = Vec::new();
        fn walk(doc: &super::Doc, re: &regex::Regex, page: &str, parent: &str, out: &mut Vec<FindMatch>) {
            for el in children_of(doc.els(), parent) {
                if let Element::Group(g) = el {
                    walk(doc, re, page, &g.base.id, out);
                }
                let Element::Text(t) = el else { continue };
                let text = body_plain_text(&t.body);
                for m in re.find_iter(&text) {
                    let i = byte_to_utf16(&text, m.start());
                    let n = len16(m.as_str());
                    out.push(FindMatch {
                        id: t.base.id.clone(),
                        page_id: page.to_string(),
                        offset: i,
                        length: n,
                        context: slice16(&text, (i - 24).max(0), i + n + 24),
                    });
                }
            }
        }
        for page in sorted_pages(&doc.doc.pages) {
            walk(&doc, &re, &page.id, &page.id, &mut out);
        }
        out
    }

    /// Select and reveal a match (selects the text range in edit mode).
    pub fn reveal_match(&mut self, m: &FindMatch) {
        self.set_active_page(&m.page_id, true);
        self.select(std::slice::from_ref(&m.id), false);
        self.start_text_edit(&m.id, None, false);
        let id = m.id.clone();
        let (a, f) = (m.offset, m.offset + m.length);
        self.set(|s| s.editing_text = Some(TextEditState { id, anchor: a, focus: f }));
    }

    /// Replace matches (all when `opts.only` is `None`). One undo step. Returns the number replaced.
    pub fn replace(&mut self, query: &str, replacement: &str, opts: &FindOpts) -> usize {
        let all = match &opts.only {
            Some(m) => vec![m.clone()],
            None => self.find(query, opts),
        };
        if all.is_empty() {
            return 0;
        }
        let doc = self.doc();
        let mut by_el: IndexMap<String, Vec<FindMatch>> = IndexMap::new();
        for m in &all {
            by_el.entry(m.id.clone()).or_default().push(m.clone());
        }
        let mut ops = Vec::new();
        for (id, mut ms) in by_el {
            if doc.el(&id).is_some_and(locked_content) {
                continue;
            }
            // Back to front within each element so earlier offsets stay valid.
            ms.sort_by_key(|m| std::cmp::Reverse(m.offset));
            for m in ms {
                // Insert after the match (inherits the matched text's formatting), then remove the match.
                if !replacement.is_empty() {
                    ops.push(DocOp::TextInsert { id: id.clone(), offset: (m.offset + m.length) as f64, text: replacement.into(), marks: None });
                }
                ops.push(DocOp::TextDelete { id: id.clone(), offset: m.offset as f64, len: m.length as f64 });
            }
        }
        if self.state.editing_text.is_some() {
            self.set(|s| s.editing_text = None);
        }
        if self.apply(ops, false).ok { all.len() } else { 0 }
    }

    // ------------------------------------------------------------------ exact geometry (ED-063)

    /// Set page-space position / size / rotation of the single selection (or move a multi-selection).
    pub fn set_geometry(&mut self, g: GeometryPatch) {
        let sel: Vec<Element> = self.selected().into_iter().filter(|e| !locked_move(e)).collect();
        if sel.is_empty() {
            return;
        }
        let doc = self.doc();
        if sel.len() > 1 {
            let b = self.selection_box().unwrap_or_default();
            let dx = g.x.map_or(0.0, |x| x - b.x);
            let dy = g.y.map_or(0.0, |y| y - b.y);
            let ops = sel
                .iter()
                .map(|e| {
                    let [pdx, pdy] = apply_vector(&parent_inverse(doc.els(), e), [dx, dy]);
                    let x = el_xf(e);
                    op_xf(&el_id(e), &Xf { x: x.x + pdx, y: x.y + pdy, ..x })
                })
                .collect();
            self.apply(ops, true);
            return;
        }
        let el = &sel[0];
        let id = el_id(el);
        let ex = el_xf(el);
        let mut xf = Xf {
            w: max(1.0, g.w.unwrap_or(ex.w)),
            h: max(1.0, g.h.unwrap_or(ex.h)),
            rot: g.rot.map_or(ex.rot, normalize_angle),
            ..ex.clone()
        };
        // Keep the centre when size/rotation change; then position by the page-space bounding box.
        xf.x = ex.x + (ex.w - xf.w) / 2.0;
        xf.y = ex.y + (ex.h - xf.h) / 2.0;
        let mut ops = vec![op_xf(&id, &xf)];
        for (cid, patch) in scale_patches(doc.els(), el, xf.w / ex.w, xf.h / ex.h) {
            ops.push(op_update(&cid, patch));
        }
        self.apply(ops, true);
        if g.x.is_some() || g.y.is_some() {
            let doc = self.doc();
            let b = world_bounds(doc.els(), &id).unwrap_or_default();
            let dx = g.x.map_or(0.0, |x| x - b.x);
            let dy = g.y.map_or(0.0, |y| y - b.y);
            let Some(cur) = doc.el(&id) else { return };
            let [pdx, pdy] = apply_vector(&parent_inverse(doc.els(), cur), [dx, dy]);
            let cx = el_xf(cur);
            self.apply(vec![op_xf(&id, &Xf { x: cx.x + pdx, y: cx.y + pdy, ..cx })], true);
        }
    }

    // ------------------------------------------------------------------ ruler guides (ED-061)

    /// Replace a page's ruler guides (`[{ axis: 'x'|'y', pos }]`).
    pub fn set_guides(&mut self, page_id: &str, guides: &[(String, f64)]) {
        let v = if guides.is_empty() {
            Value::Null
        } else {
            Value::Array(
                guides
                    .iter()
                    .map(|(axis, pos)| {
                        let mut m = one("axis", Value::String(axis.clone()));
                        m.insert("pos".into(), num(round(pos * 10.0) / 10.0));
                        Value::Object(m)
                    })
                    .collect(),
            )
        };
        self.apply(vec![op_page_update(page_id, one("guides", v))], false);
    }

    pub fn add_guide(&mut self, page_id: &str, axis: &str, pos: f64) {
        let doc = self.doc();
        let mut gs: Vec<(String, f64)> = doc
            .doc
            .pages
            .get(page_id)
            .and_then(|p| p.guides.as_ref())
            .map(|g| g.iter().map(|g| (g.axis.as_str().to_string(), g.pos)).collect())
            .unwrap_or_default();
        gs.push((axis.to_string(), pos));
        self.set_guides(page_id, &gs);
    }

    // ------------------------------------------------------------------ photo grids (IM-030)

    /// Index of the grid cell under a page point, or -1.
    pub fn grid_cell_at(&self, id: &str, page_point: Vec2) -> i64 {
        let doc = self.doc();
        let Some(Element::Grid(g)) = doc.el(id) else { return -1 };
        let [x, y] = apply(&invert(&world_matrix(doc.els(), id)), page_point);
        g.cells
            .iter()
            .position(|c| {
                let b = grid_cell_box(g.gap, &g.base.xf, &Rect::from(&c.rect));
                x >= b.x && y >= b.y && x <= b.x + b.w && y <= b.y + b.h
            })
            .map_or(-1, |i| i as i64)
    }

    pub fn set_grid_cell(&mut self, id: &str, index: i64, asset: Option<&str>) {
        let doc = self.doc();
        let Some(Element::Grid(g)) = doc.el(id) else { return };
        if index < 0 || index as usize >= g.cells.len() {
            return;
        }
        // Video assets play in the cell (A13) with their own clip settings.
        let video = asset.and_then(|a| self.video_clip_for(a));
        let cells: Vec<Value> = g
            .cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if i as i64 != index {
                    return jv(c);
                }
                let mut m = one("rect", jv(&c.rect));
                if let Some(a) = asset {
                    m.insert("asset".into(), Value::String(a.into()));
                    if let Some(v) = &video {
                        m.insert("video".into(), Value::Object(v.clone()));
                    }
                }
                Value::Object(m)
            })
            .collect();
        self.apply(vec![op_update(id, one("cells", Value::Array(cells)))], false);
    }

    /// Default clip settings when a video asset fills a frame or grid cell; `None` for images.
    fn video_clip_for(&self, asset: &str) -> Option<Map<String, Value>> {
        let doc = self.doc();
        let a = doc.snap.assets.get(asset)?;
        if s(a.get("kind")) != Some("video") {
            return None;
        }
        let dur = crate::util::f(a.get("durationMs")).filter(|d| *d != 0.0 && !d.is_nan()).unwrap_or(5000.0);
        let animated = crate::util::truthy(a.get("animated"));
        let mut m = one("trimStartMs", num(0.0));
        m.insert("trimEndMs".into(), num(dur));
        m.insert("speed".into(), num(1.0));
        m.insert("volume".into(), num(1.0));
        m.insert("muted".into(), Value::Bool(animated));
        m.insert("loop".into(), Value::Bool(animated));
        Some(m)
    }

    /// Put an image or video into a frame, cover-cropped to the frame box.
    pub fn fill_frame(&mut self, id: &str, asset: &str, aspect: Option<f64>) {
        let doc = self.doc();
        let Some(Element::Frame(f)) = doc.el(id) else { return };
        let ar = aspect.unwrap_or_else(|| {
            let a = doc.snap.assets.get(asset);
            let w = crate::util::f(a.and_then(|a| a.get("w"))).unwrap_or(0.0);
            let h = crate::util::f(a.and_then(|a| a.get("h"))).unwrap_or(0.0);
            if w != 0.0 && h != 0.0 && !w.is_nan() && !h.is_nan() { w / h } else { 0.0 }
        });
        let crop = if ar != 0.0 && !ar.is_nan() { cover_crop(f.base.xf.w / f.base.xf.h, ar) } else { Rect::UNIT };
        let video = self.video_clip_for(asset);
        let mut content = one("asset", Value::String(asset.into()));
        content.insert("crop".into(), jv(&crop));
        if let Some(v) = video {
            content.insert("video".into(), Value::Object(v));
        }
        self.apply(vec![op_update(id, one("content", Value::Object(content)))], false);
    }

    /// Drop an asset at a page point: fills a grid cell or a frame under the point. Returns true when used.
    pub fn drop_into_container(&mut self, page_id: &str, local: Vec2, asset: &str) -> bool {
        let doc = self.doc();
        let hit = hit_test(doc.els(), page_id, local, HitTestOpts { tolerance: 0.0, enter_group: self.state.entered_group.as_deref() }).cloned();
        match &hit {
            Some(Element::Grid(g)) => {
                let i = self.grid_cell_at(&g.base.id, local);
                if i >= 0 {
                    let id = g.base.id.clone();
                    self.set_grid_cell(&id, i, Some(asset));
                    self.select(&[id], false);
                    return true;
                }
            }
            Some(Element::Frame(f)) if f.base.lock.as_ref().and_then(|l| l.content) != Some(true) => {
                let id = f.base.id.clone();
                self.fill_frame(&id, asset, None);
                self.select(&[id], false);
                return true;
            }
            _ => {}
        }
        false
    }

    // ------------------------------------------------------------------ bulk create (ED-090)

    /// Elements on a page that can take data: text, images, frames.
    pub fn bindable_elements(&self, page_id: Option<&str>) -> Vec<Element> {
        let doc = self.doc();
        let pid = page_id.map(str::to_string).unwrap_or_else(|| self.state.active_page_id.clone());
        descendants(doc.els(), &pid)
            .into_iter()
            .filter(|e| matches!(e, Element::Text(_) | Element::Image(_) | Element::Frame(_)) && !e.is_deleted())
            .cloned()
            .collect()
    }

    pub fn set_data_binding(&mut self, id: &str, column: Option<&str>) {
        let v = column.map_or(Value::Null, |c| Value::Object(one("column", Value::String(c.into()))));
        self.apply(vec![op_update(id, one("dataBinding", v))], false);
    }

    /// One new page per data row, cloned from `page_id` with bound elements filled in. Returns the new page ids.
    pub fn bulk_create(&mut self, page_id: &str, rows: &[IndexMap<String, String>], images: &HashMap<String, ResolvedImage>) -> Vec<String> {
        let doc = self.doc();
        let Some(page) = doc.snap.pages.get(page_id).cloned() else { return vec![] };
        if rows.is_empty() {
            return vec![];
        }
        let els: Vec<Element> = descendants(doc.els(), page_id).into_iter().cloned().collect();
        let mut ops = Vec::new();
        let mut created = Vec::new();
        let mut after = page_id.to_string();
        for row in rows {
            let pid = self.fresh_id();
            created.push(pid.clone());
            let mut rest = (*page).clone();
            rest.remove("order");
            rest.insert("id".into(), Value::String(pid.clone()));
            ops.push(DocOp::Page(PageOp::Add { page: Some(rest), after: Some(Some(after.clone())) }));
            after = pid.clone();
            let mut remap: HashMap<String, String> = HashMap::from([(page_id.to_string(), pid.clone())]);
            for e in &els {
                let nid = self.fresh_id();
                remap.insert(el_id(e), nid);
            }
            for e in &els {
                let mut el = doc.raw_el(e.id().unwrap_or("")).cloned().unwrap_or_else(|| jm(e));
                el.insert("id".into(), Value::String(remap.get(&el_id(e)).cloned().unwrap_or_default()));
                let parent = remap.get(e.parent_id().unwrap_or("")).cloned().unwrap_or_else(|| pid.clone());
                el.insert("parentId".into(), Value::String(parent.clone()));
                let col = e.base().and_then(|b| b.data_binding.as_ref()).map(|d| d.column.clone());
                if let Some(value) = col.and_then(|c| row.get(&c)) {
                    match e {
                        Element::Text(t) => {
                            let p0 = t.body.paragraphs.first();
                            let style = p0.and_then(|p| p.runs.first()).map_or_else(Map::new, |r| jm(&r.style));
                            let mut attrs = p0.map(jm).unwrap_or_default();
                            attrs.remove("runs");
                            let paragraphs: Vec<Value> = value
                                .split('\n')
                                .map(|line| {
                                    let line = line.strip_suffix('\r').unwrap_or(line);
                                    let mut p = attrs.clone();
                                    let mut run = one("text", Value::String(line.into()));
                                    run.insert("style".into(), Value::Object(style.clone()));
                                    p.insert("runs".into(), Value::Array(vec![Value::Object(run)]));
                                    Value::Object(p)
                                })
                                .collect();
                            let mut body = el.get("body").and_then(Value::as_object).cloned().unwrap_or_default();
                            body.insert("paragraphs".into(), Value::Array(paragraphs));
                            el.insert("body".into(), Value::Object(body));
                        }
                        Element::Image(_) | Element::Frame(_) => {
                            if let Some(a) = images.get(value) {
                                let x = el_xf(e);
                                let crop = cover_crop(x.w / x.h, a.w / a.h);
                                if matches!(e, Element::Image(_)) {
                                    el.insert("asset".into(), Value::String(a.id.clone()));
                                    el.insert("crop".into(), jv(&crop));
                                } else {
                                    let mut c = one("asset", Value::String(a.id.clone()));
                                    c.insert("crop".into(), jv(&crop));
                                    el.insert("content".into(), Value::Object(c));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                ops.push(op_add(el, &parent, None));
            }
        }
        if !self.apply(ops, false).ok {
            return vec![];
        }
        created
    }

    // ------------------------------------------------------------------ pages

    /// Add a page after `after` (`None` = at the end). Returns its id.
    pub fn add_page(&mut self, after: Option<&str>) -> Option<String> {
        let r = self.apply(vec![DocOp::Page(PageOp::Add { page: None, after: after.map(|a| Some(a.to_string())) })], false);
        let id = r.created.first().cloned()?;
        self.set_active_page(&id, false);
        self.scroll_to_page(&id);
        Some(id)
    }

    /// `addPage()` with the TS default (after the active page).
    pub fn add_page_after_active(&mut self) -> Option<String> {
        let a = self.state.active_page_id.clone();
        self.add_page(if a.is_empty() { None } else { Some(&a) })
    }

    pub fn duplicate_page(&mut self, page_id: Option<&str>) {
        let pid = page_id.map(str::to_string).unwrap_or_else(|| self.state.active_page_id.clone());
        let r = self.apply(vec![DocOp::Page(PageOp::Duplicate { page_id: pid, new_page_id: None })], false);
        if let Some(id) = r.created.first().cloned() {
            self.set_active_page(&id, false);
            self.scroll_to_page(&id);
        }
    }

    pub fn delete_page(&mut self, page_id: Option<&str>) {
        let pid = page_id.map(str::to_string).unwrap_or_else(|| self.state.active_page_id.clone());
        let doc = self.doc();
        let pages: Vec<String> = sorted_pages(&doc.doc.pages).iter().map(|p| p.id.clone()).collect();
        if pages.len() <= 1 {
            return;
        }
        let i = pages.iter().position(|p| *p == pid).map_or(-1, |i| i as i64);
        self.apply(vec![DocOp::Page(PageOp::Delete { page_id: pid })], false);
        let next = pages.get((i + 1) as usize).or_else(|| if i >= 1 { pages.get((i - 1) as usize) } else { None }).cloned();
        if let Some(n) = next {
            self.set_active_page(&n, false);
        }
    }

    pub fn move_page(&mut self, page_id: &str, to_index: i64) {
        let doc = self.doc();
        let pages: Vec<String> = sorted_pages(&doc.doc.pages).iter().filter(|p| p.id != page_id).map(|p| p.id.clone()).collect();
        let after = if to_index <= 0 { None } else { pages.get((to_index.min(pages.len() as i64) - 1) as usize).cloned() };
        self.apply(vec![DocOp::Page(PageOp::Move { page_id: page_id.into(), after })], false);
    }

    // ------------------------------------------------------------------ design styles

    /// Replace colours everywhere (hex → hex). One undo step. Returns the number of changes.
    pub fn replace_colors(&mut self, map: &IndexMap<String, String>) -> usize {
        let ops = recolor_ops(&self.doc().doc, map);
        let n = ops.len();
        if n > 0 {
            self.apply(ops, false);
        }
        n
    }

    /// Recolour the design with a palette (luminance-ranked mapping).
    pub fn apply_palette(&mut self, colors: &[String]) -> usize {
        let m = palette_map(&self.doc().doc, colors);
        self.replace_colors(&m)
    }

    /// Apply a heading/body font pair to every text.
    pub fn apply_fonts(&mut self, set: &FontSet) -> usize {
        let ops = font_set_ops(&self.doc().doc, set);
        let n = ops.len();
        if n > 0 {
            self.apply(ops, false);
        }
        n
    }

    /// Document language (BCP-47) for screen readers, hyphenation and spell-check.
    pub fn set_language(&mut self, lang: &str) {
        self.apply(vec![op_meta(one("lang", Value::String(lang.into())))], false);
    }

    /// Patch a page (JSON `Partial<Page>`; `null` deletes a key).
    pub fn update_page(&mut self, page_id: &str, patch: &Map<String, Value>) {
        self.apply(vec![op_page_update(page_id, patch.clone())], false);
    }

    /// Change the design's page size; elements keep their position unless `scale_content` (Canva's "Resize").
    pub fn resize_design(&mut self, w: f64, h: f64, unit: Option<Unit>, scale_content: bool) {
        let doc = self.doc();
        let (ow, oh) = (doc.doc.meta.dims.w, doc.doc.meta.dims.h);
        let mut dims = doc.doc.meta.dims.clone();
        dims.w = w;
        dims.h = h;
        if let Some(u) = unit {
            dims.unit = u;
        }
        let mut ops = vec![op_meta(one("dims", jv(&dims)))];
        if scale_content {
            let sc = min(w / ow, h / oh);
            let dx = (w - ow * sc) / 2.0;
            let dy = (h - oh * sc) / 2.0;
            for p in sorted_pages(&doc.doc.pages) {
                for el in children_of(doc.els(), &p.id) {
                    let x = el_xf(el);
                    ops.push(op_xf(&el_id(el), &Xf { x: x.x * sc + dx, y: x.y * sc + dy, w: x.w * sc, h: x.h * sc, ..x }));
                    for (id, patch) in scale_patches(doc.els(), el, sc, sc) {
                        ops.push(op_update(&id, patch));
                    }
                }
            }
        }
        self.apply(ops, false);
    }

    // ------------------------------------------------------------------ history

    /// Undo; the caret is clamped to the new text length (TS does this in a microtask; here synchronously).
    pub fn undo(&mut self) {
        let e = self.state.editing_text.clone();
        self.in_history = true;
        self.store_mut().undo();
        self.process_changes();
        self.in_history = false;
        self.clamp_caret(e);
    }

    pub fn redo(&mut self) {
        self.in_history = true;
        self.store_mut().redo();
        self.process_changes();
        self.in_history = false;
    }

    pub fn can_undo(&self) -> bool {
        self.store().can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.store().can_redo()
    }

    fn clamp_caret(&mut self, e: Option<TextEditState>) {
        let Some(e) = e else { return };
        if self.state.editing_text.as_ref().map(|x| &x.id) != Some(&e.id) {
            return;
        }
        let Some(t) = self.edit_target() else { return };
        let Some(ts) = &self.text_service else { return };
        let len = ts.length(&t.el);
        self.set(|s| s.editing_text = Some(TextEditState { id: e.id.clone(), anchor: e.anchor.min(len), focus: e.focus.min(len) }));
    }

    // ------------------------------------------------------------------ present mode

    pub fn set_presenting(&mut self, on: bool) {
        self.set(|s| {
            s.presenting = on;
            s.editing_text = None;
            if on {
                s.selection = vec![];
            }
        });
    }


}
