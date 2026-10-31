// SPDX-License-Identifier: Apache-2.0
//! Vector editing (DS-100) and masks (DS-107): the pen tool, node editing of path elements, Boolean
//! operations, converting outlines (text, SVG) into paths, and masking.
//!
//! A path element is a `custom` shape (`d` in its unit box). Node editing works on the outline in the element's
//! own pixel space; every edit is written back normalised to the outline's new tight bounds, with the box moved
//! so nothing shifts on the page (rotation and flips kept).
//!
//! Keyboard: with the pen, arrow keys move a cursor, Space places a node there, Enter finishes, C closes the
//! path, Backspace removes the last node and Escape cancels. While editing nodes, Tab picks the next node,
//! arrows nudge the selected nodes, `+` adds a node after them, S toggles smooth/corner, Delete removes them and
//! Escape or Enter stops editing.

use oc_crdt::DocOp;
use oc_geometry::{
    BoolOp, NodePart, NodeRef, PathNode, VectorPath, apply, boolean, invert, local_matrix, shape_path, world_matrix,
};
use oc_schema::elements::{Element, MaskMode};
use oc_schema::primitives::{Mat2D, Paint, Vec2, Xf};
use oc_schema::tree::children_of;
use serde_json::{Map, Value, json};

use super::{Editor, Gesture, el_id, el_xf, locked_content, one, op_add, op_update};
use crate::state::*;
use crate::util::{hypot, jv};

/// Screen pixels within which a click lands on a node, the first node (to close) or a segment.
const HIT_PX: f64 = 8.0;

fn dist(a: Vec2, b: Vec2) -> f64 {
    hypot(a[0] - b[0], a[1] - b[1])
}

/// Matrix part only (no translation).
fn linear(m: &Mat2D) -> Mat2D {
    [m[0], m[1], m[2], m[3], 0.0, 0.0]
}

/// `p` constrained to 45° steps from `from` (Shift).
fn snap45(from: Vec2, p: Vec2) -> Vec2 {
    let (dx, dy) = (p[0] - from[0], p[1] - from[1]);
    let len = hypot(dx, dy);
    if len == 0.0 {
        return p;
    }
    let a = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4;
    [from[0] + len * a.cos(), from[1] + len * a.sin()]
}

/// The outline of a shape element in its own pixel space (custom paths and every preset kind).
pub fn shape_outline(el: &Element) -> Option<VectorPath> {
    let Element::Shape(s) = el else { return None };
    let x = &s.base.xf;
    let d = shape_path(s.kind.as_str(), x.w, x.h, &s.params, s.d.as_deref());
    let p = VectorPath::parse(&d);
    (!p.is_empty()).then_some(p)
}

/// The patch that gives a shape `path` (in the pixel space of `origin`'s box) as its outline: kind `custom`, `d`
/// in the unit box of the path's bounds and the box moved so the outline stays where it is.
pub fn path_patch(origin: &Xf, path: &VectorPath) -> Option<Map<String, Value>> {
    let (b, unit) = path.to_unit(1.0)?;
    let c = apply(&local_matrix(origin), [b.x + b.w / 2.0, b.y + b.h / 2.0]);
    let xf = Xf { x: c[0] - b.w / 2.0, y: c[1] - b.h / 2.0, w: b.w, h: b.h, ..origin.clone() };
    let mut m = Map::new();
    m.insert("kind".into(), json!("custom"));
    m.insert("d".into(), json!(unit.to_d()));
    m.insert("params".into(), json!({}));
    m.insert("xf".into(), jv(&xf));
    Some(m)
}

/// A path element (custom shape JSON) for `path` given in its parent's space, upright.
fn path_element(base: Map<String, Value>, path: &VectorPath) -> Option<Map<String, Value>> {
    let (b, unit) = path.to_unit(1.0)?;
    let mut m = base;
    m.insert("type".into(), json!("shape"));
    m.insert("kind".into(), json!("custom"));
    m.insert("d".into(), json!(unit.to_d()));
    m.insert("params".into(), json!({}));
    m.insert("xf".into(), jv(&Xf::new(b.x, b.y, b.w, b.h, 0.0)));
    Some(m)
}

/// Nodes of the path being edited, in page space (for the host's overlay).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PathEditView {
    pub id: String,
    pub subpaths: Vec<oc_geometry::SubPath>,
    pub selected: Vec<NodeRef>,
}

impl Editor {
    // ------------------------------------------------------------------ pen

    fn new_pen(&self, page_id: &str) -> PenState {
        PenState { page_id: page_id.to_string(), nodes: vec![], cursor: None, dragging: false }
    }

    /// Pen click (page point): close on the first node, else add a node whose handles a drag pulls out.
    pub(crate) fn pen_down(&mut self, page_id: &str, local: Vec2, mods: Mods) {
        let zoom = self.state.viewport.zoom.max(1e-6);
        let mut pen = match self.state.pen.clone() {
            Some(p) if p.page_id == page_id => p,
            Some(_) => {
                self.pen_finish(false);
                self.new_pen(page_id)
            }
            None => self.new_pen(page_id),
        };
        if pen.nodes.len() >= 2 && dist(local, pen.nodes[0].p) * zoom <= HIT_PX {
            self.pen_commit(pen, true);
            return;
        }
        let p = match pen.nodes.last() {
            Some(last) if mods.shift => snap45(last.p, local),
            _ => local,
        };
        pen.nodes.push(PathNode::corner(p));
        pen.dragging = true;
        pen.cursor = Some(p);
        let pid = page_id.to_string();
        self.set(|s| {
            s.pen = Some(pen);
            s.selection = vec![];
            s.active_page_id = pid;
            s.gesturing = true;
        });
    }

    /// Pen pointer move: drags the new node's handles out (Alt: only the outgoing one), else moves the preview
    /// cursor. Returns false when the pen is not drawing.
    pub(crate) fn pen_move(&mut self, p: Vec2, mods: Mods) -> bool {
        let Some(mut pen) = self.state.pen.clone() else { return false };
        let local = self.to_page(&pen.page_id, p);
        let zoom = self.state.viewport.zoom.max(1e-6);
        if pen.dragging {
            let Some(n) = pen.nodes.last_mut() else { return true };
            if dist(local, n.p) * zoom < 2.0 {
                return true;
            }
            n.cout = Some(local);
            if mods.alt {
                n.smooth = false;
            } else {
                n.cin = Some([2.0 * n.p[0] - local[0], 2.0 * n.p[1] - local[1]]);
                n.smooth = true;
            }
        } else {
            pen.cursor = Some(match pen.nodes.last() {
                Some(last) if mods.shift => snap45(last.p, local),
                _ => local,
            });
        }
        self.set(|s| s.pen = Some(pen));
        true
    }

    pub(crate) fn pen_up(&mut self) -> bool {
        let Some(mut pen) = self.state.pen.clone() else { return false };
        pen.dragging = false;
        self.set(|s| {
            s.pen = Some(pen);
            s.gesturing = false;
        });
        true
    }

    /// Finish the path being drawn (`close` joins the last node to the first). Fewer than two nodes draw
    /// nothing. Returns the new element's id.
    pub fn pen_finish(&mut self, close: bool) -> Option<String> {
        let pen = self.state.pen.clone()?;
        self.pen_commit(pen, close)
    }

    /// Drop the path being drawn.
    pub fn pen_cancel(&mut self) {
        self.set(|s| {
            s.pen = None;
            s.gesturing = false;
        });
    }

    fn pen_commit(&mut self, pen: PenState, close: bool) -> Option<String> {
        self.set(|s| {
            s.pen = None;
            s.gesturing = false;
        });
        let closed = close && pen.nodes.len() >= 3;
        if pen.nodes.len() < 2 {
            return None;
        }
        let path = VectorPath { subpaths: vec![oc_geometry::SubPath { nodes: pen.nodes, closed }] };
        let base = self.fresh_base(Xf::new(0.0, 0.0, 1.0, 1.0, 0.0));
        let mut m = crate::util::jm(&base);
        // Closed paths are filled, open ones stroked.
        if closed {
            m.insert("fills".into(), json!([{ "k": "solid", "color": { "space": "srgb", "r": 0.5450980392156862, "g": 0.3607843137254902, "b": 0.9647058823529412, "a": 1 } }]));
            m.insert("strokes".into(), json!([]));
        } else {
            m.insert("fills".into(), json!([]));
            m.insert("strokes".into(), jv(&[oc_schema::factory::make_stroke(None, Some(2.0), None)]));
        }
        m.insert("name".into(), json!("Path"));
        let el = path_element(m, &path)?;
        let id = crate::util::s(el.get("id")).unwrap_or("").to_string();
        let r = self.apply(vec![op_add(el, &pen.page_id, None)], false);
        if !r.ok {
            return None;
        }
        self.set(|s| s.tool = Tool::Select);
        self.select(std::slice::from_ref(&id), false);
        Some(id)
    }

    /// Pen keyboard: arrows move the cursor, Space places a node, Enter finishes, C closes, Backspace removes the
    /// last node, Escape cancels (and leaves the pen when nothing is being drawn).
    pub(crate) fn pen_key(&mut self, k: &str, mods: Mods) -> bool {
        let page = self.state.active_page_id.clone();
        let step = if mods.shift { 10.0 } else { 1.0 };
        let arrow = match k {
            "ArrowLeft" => Some([-step, 0.0]),
            "ArrowRight" => Some([step, 0.0]),
            "ArrowUp" => Some([0.0, -step]),
            "ArrowDown" => Some([0.0, step]),
            _ => None,
        };
        if let Some([dx, dy]) = arrow {
            let mut pen = self.state.pen.clone().unwrap_or_else(|| self.new_pen(&page));
            let dims = &self.doc().doc.meta.dims;
            let start = pen.nodes.last().map(|n| n.p).unwrap_or([dims.w / 2.0, dims.h / 2.0]);
            let c = pen.cursor.unwrap_or(start);
            pen.cursor = Some([c[0] + dx, c[1] + dy]);
            self.set(|s| s.pen = Some(pen));
            return true;
        }
        match k {
            " " | "Spacebar" => {
                let mut pen = self.state.pen.clone().unwrap_or_else(|| self.new_pen(&page));
                let dims = &self.doc().doc.meta.dims;
                let c = pen.cursor.unwrap_or([dims.w / 2.0, dims.h / 2.0]);
                if pen.nodes.len() >= 2 && dist(c, pen.nodes[0].p) < 0.5 {
                    self.pen_commit(pen, true);
                    return true;
                }
                pen.nodes.push(PathNode::corner(c));
                pen.cursor = Some(c);
                self.set(|s| s.pen = Some(pen));
                true
            }
            "Enter" => {
                if self.state.pen.is_some() {
                    self.pen_finish(false);
                }
                true
            }
            "c" => {
                if self.state.pen.as_ref().is_some_and(|p| p.nodes.len() >= 3) {
                    self.pen_finish(true);
                }
                true
            }
            "Backspace" | "Delete" => {
                if let Some(mut pen) = self.state.pen.clone() {
                    pen.nodes.pop();
                    let keep = !pen.nodes.is_empty();
                    self.set(|s| s.pen = keep.then_some(pen));
                }
                true
            }
            "Escape" => {
                if self.state.pen.is_some() {
                    self.pen_cancel();
                } else {
                    self.set(|s| s.tool = Tool::Select);
                }
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------------ node editing

    /// Start editing the nodes of a shape (any kind; it becomes a custom path on the first edit).
    pub fn start_path_edit(&mut self, id: &str) -> bool {
        let doc = self.doc();
        let Some(el) = doc.live(id) else { return false };
        if !matches!(el, Element::Shape(_)) || locked_content(el) || self.is_brand_locked(Some(el)) {
            return false;
        }
        let id = id.to_string();
        self.store_mut().checkpoint();
        self.select(std::slice::from_ref(&id), false);
        self.set(|s| {
            s.path_edit = Some(PathEditState { id: id.clone(), nodes: vec![] });
            s.cropping_id = None;
        });
        true
    }

    pub fn stop_path_edit(&mut self) {
        if self.state.path_edit.is_some() {
            self.set(|s| s.path_edit = None);
        }
    }

    /// Select nodes of the path being edited.
    pub fn select_path_nodes(&mut self, nodes: Vec<NodeRef>) {
        if let Some(mut pe) = self.state.path_edit.clone() {
            pe.nodes = nodes;
            self.set(|s| s.path_edit = Some(pe));
        }
    }

    /// The edited path's outline (own pixel space, overrides applied) and world matrix.
    fn edit_geometry(&self, id: &str) -> Option<(VectorPath, Mat2D, Element)> {
        let v = self.view();
        let el = v.live(id)?.clone();
        let path = shape_outline(&el)?;
        Some((path, world_matrix(v.els(), id), el))
    }

    /// Nodes of the path being edited, page space, for the host's overlay.
    pub fn path_edit_view(&self) -> Option<PathEditView> {
        let pe = self.state.path_edit.as_ref()?;
        let (path, m, _) = self.edit_geometry(&pe.id)?;
        Some(PathEditView { id: pe.id.clone(), subpaths: path.transformed(&m).subpaths, selected: pe.nodes.clone() })
    }

    /// Write an edited outline (own pixel space of `origin`) as one undo step.
    fn commit_path(&mut self, id: &str, origin: &Element, path: &VectorPath) -> bool {
        if path.is_empty() || path.node_count() < 2 {
            self.apply(vec![DocOp::Delete { id: id.to_string() }], false);
            self.set(|s| {
                s.path_edit = None;
                s.selection = vec![];
            });
            return true;
        }
        let Some(mut patch) = path_patch(&el_xf(origin), path) else { return false };
        let xf = patch.remove("xf").unwrap_or(Value::Null);
        self.apply(vec![op_update(id, patch), DocOp::Xf { id: id.to_string(), xf }], false).ok
    }

    /// Pointer down while editing nodes. Returns false when the click leaves node editing (the host's normal
    /// selection handling takes over).
    pub(crate) fn path_down(&mut self, local: Vec2, mods: Mods, target: &HitTarget) -> bool {
        let Some(pe) = self.state.path_edit.clone() else { return false };
        let Some((path, m, el)) = self.edit_geometry(&pe.id) else {
            self.stop_path_edit();
            return false;
        };
        let inv = invert(&m);
        let p = apply(&inv, local);
        let zoom = self.state.viewport.zoom.max(1e-6);
        let (node, part) = match target {
            HitTarget::PathNode { s, i, part } => (NodeRef { s: *s, i: *i }, *part),
            HitTarget::Canvas => {
                // A click on the outline adds a node there (Alt-click on a node toggles smooth/corner).
                let Some((s, seg, t, d)) = path.nearest_segment(p) else { return false };
                if d * zoom > HIT_PX {
                    self.stop_path_edit();
                    return false;
                }
                let mut np = path.clone();
                let Some(r) = np.split_segment(s, seg, t) else { return true };
                if self.commit_path(&pe.id, &el, &np) {
                    self.select_path_nodes(vec![r]);
                }
                return true;
            }
            _ => {
                self.stop_path_edit();
                return false;
            }
        };
        if path.node(node).is_none() {
            return true;
        }
        let mut sel = pe.nodes.clone();
        if part == NodePart::Anchor {
            if mods.alt {
                let smooth = !path.node(node).is_some_and(|n| n.smooth || n.cin.is_some() || n.cout.is_some());
                let mut np = path.clone();
                np.set_smooth(node, smooth);
                self.commit_path(&pe.id, &el, &np);
                self.select_path_nodes(vec![node]);
                return true;
            }
            if mods.shift {
                if let Some(i) = sel.iter().position(|r| *r == node) {
                    sel.remove(i);
                } else {
                    sel.push(node);
                }
            } else if !sel.contains(&node) {
                sel = vec![node];
            }
            self.select_path_nodes(sel.clone());
        }
        self.gesture = Some(Gesture::PathEdit { id: pe.id.clone(), start: p, origin: Box::new(el), path, node, part, nodes: sel, moved: false });
        self.set(|s| s.gesturing = true);
        true
    }

    /// Node or handle drag (page point `local`).
    pub(crate) fn path_move(&mut self, local: Vec2, mods: Mods) {
        let Some(Gesture::PathEdit { id, start, origin, path, node, part, nodes, moved }) = self.gesture.clone() else { return };
        let m = world_matrix(self.doc().els(), &id);
        let p = apply(&invert(&m), local);
        let zoom = self.state.viewport.zoom.max(1e-6);
        if !moved && dist(p, start) * zoom < 2.0 {
            return;
        }
        self.gesture = Some(Gesture::PathEdit { id: id.clone(), start, origin: origin.clone(), path: path.clone(), node, part, nodes: nodes.clone(), moved: true });
        let mut np = path.clone();
        match part {
            NodePart::Anchor => {
                let (mut dx, mut dy) = (p[0] - start[0], p[1] - start[1]);
                if mods.shift {
                    if dx.abs() > dy.abs() { dy = 0.0 } else { dx = 0.0 }
                }
                np.move_nodes(&nodes, [dx, dy]);
            }
            NodePart::In | NodePart::Out => {
                if mods.alt
                    && let Some(n) = np.node_mut(node)
                {
                    n.smooth = false;
                }
                let anchor = np.node(node).map(|n| n.p).unwrap_or(p);
                let to = if mods.shift { snap45(anchor, p) } else { p };
                np.move_handle(node, part, to, false);
            }
        }
        if let Some(patch) = path_patch(&el_xf(&origin), &np) {
            self.set_override(&id, patch);
        }
        self.set(|s| s.gesturing = true);
    }

    pub(crate) fn path_up(&mut self) {
        let Some(Gesture::PathEdit { id, origin, moved, .. }) = self.gesture.take() else { return };
        let o = self.overrides.get(&id).cloned();
        self.clear_overrides();
        if moved && let Some(mut o) = o {
            let xf = o.remove("xf").unwrap_or(Value::Null);
            let _ = origin;
            self.apply(vec![op_update(&id, o), DocOp::Xf { id: id.clone(), xf }], false);
        }
        self.set(|s| s.gesturing = false);
    }

    /// Apply an edit to the selected nodes of the path being edited (one undo step).
    fn edit_nodes(&mut self, f: impl FnOnce(&mut VectorPath, &[NodeRef]) -> Option<Vec<NodeRef>>) -> bool {
        let Some(pe) = self.state.path_edit.clone() else { return false };
        let Some((path, _, el)) = self.edit_geometry(&pe.id) else { return false };
        let mut np = path.clone();
        let next = f(&mut np, &pe.nodes);
        if np == path {
            if let Some(n) = next {
                self.select_path_nodes(n);
            }
            return false;
        }
        self.store_mut().checkpoint();
        self.commit_path(&pe.id, &el, &np);
        if let Some(n) = next {
            self.select_path_nodes(n);
        }
        true
    }

    /// Move the selected nodes by a page-space offset.
    pub fn nudge_path_nodes(&mut self, dx: f64, dy: f64) -> bool {
        let Some(pe) = self.state.path_edit.clone() else { return false };
        let m = world_matrix(self.doc().els(), &pe.id);
        let d = apply(&linear(&invert(&m)), [dx, dy]);
        self.edit_nodes(|p, sel| {
            p.move_nodes(sel, d);
            None
        })
    }

    /// Make the selected nodes smooth or corners (`None` toggles on the first one).
    pub fn set_path_nodes_smooth(&mut self, smooth: Option<bool>) -> bool {
        self.edit_nodes(|p, sel| {
            let on = smooth.unwrap_or_else(|| !sel.first().and_then(|r| p.node(*r)).is_some_and(|n| n.smooth || n.cin.is_some() || n.cout.is_some()));
            for r in sel {
                p.set_smooth(*r, on);
            }
            None
        })
    }

    /// Delete the selected nodes (the path goes when fewer than two remain).
    pub fn delete_path_nodes(&mut self) -> bool {
        self.edit_nodes(|p, sel| {
            p.delete_nodes(sel);
            Some(vec![])
        })
    }

    /// Add a node halfway along the segment after each selected node (or after the last node).
    pub fn add_path_node(&mut self) -> bool {
        self.edit_nodes(|p, sel| {
            let targets: Vec<NodeRef> = if sel.is_empty() {
                p.subpaths.first().map(|sp| vec![NodeRef { s: 0, i: sp.nodes.len().saturating_sub(if sp.closed { 1 } else { 2 }) }]).unwrap_or_default()
            } else {
                sel.to_vec()
            };
            let mut sorted = targets;
            sorted.sort();
            let mut out = Vec::new();
            for r in sorted.iter().rev() {
                if let Some(n) = p.split_segment(r.s, r.i, 0.5) {
                    out.push(n);
                }
            }
            Some(out)
        })
    }

    /// Keys while editing nodes.
    pub(crate) fn path_key(&mut self, k: &str, mods: Mods) -> bool {
        let Some(pe) = self.state.path_edit.clone() else { return false };
        let step = if mods.shift { 10.0 } else { 1.0 };
        match k {
            "Escape" | "Enter" => {
                self.stop_path_edit();
                true
            }
            "ArrowLeft" => self.nudge_path_nodes(-step, 0.0) || true,
            "ArrowRight" => self.nudge_path_nodes(step, 0.0) || true,
            "ArrowUp" => self.nudge_path_nodes(0.0, -step) || true,
            "ArrowDown" => self.nudge_path_nodes(0.0, step) || true,
            "Backspace" | "Delete" => {
                if pe.nodes.is_empty() {
                    self.stop_path_edit();
                    return false;
                }
                self.delete_path_nodes();
                true
            }
            "Tab" => {
                let Some((path, _, _)) = self.edit_geometry(&pe.id) else { return true };
                let refs = path.refs();
                if refs.is_empty() {
                    return true;
                }
                let cur = pe.nodes.last().and_then(|r| refs.iter().position(|x| x == r));
                let n = refs.len();
                let i = match cur {
                    None => if mods.shift { n - 1 } else { 0 },
                    Some(c) => if mods.shift { (c + n - 1) % n } else { (c + 1) % n },
                };
                self.select_path_nodes(vec![refs[i]]);
                true
            }
            "s" => {
                self.set_path_nodes_smooth(None);
                true
            }
            "+" | "=" => {
                self.add_path_node();
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------------ Booleans and conversions

    /// Combine the selected shapes (same parent, at least two) into one path styled like the bottom one.
    /// Returns the new path's id (`None` when nothing is left, e.g. intersecting shapes that do not meet).
    pub fn boolean_op(&mut self, op: BoolOp) -> Option<String> {
        let doc = self.doc();
        let sel = self.selected();
        if sel.len() < 2 || sel.iter().any(|e| !matches!(e, Element::Shape(_)) || locked_content(e) || self.is_brand_locked(Some(e))) {
            return None;
        }
        let parent = sel[0].parent_id().unwrap_or("").to_string();
        if sel.iter().any(|e| e.parent_id() != Some(parent.as_str())) {
            return None;
        }
        let ids: Vec<String> = sel.iter().map(el_id).collect();
        let ordered: Vec<&Element> = children_of(doc.els(), &parent).into_iter().filter(|e| ids.iter().any(|i| Some(i.as_str()) == e.id())).collect();
        let to_parent = doc.el(&parent).map(|p| invert(&world_matrix(doc.els(), p.id().unwrap_or("")))).unwrap_or(oc_geometry::IDENTITY);
        let paths: Vec<VectorPath> = ordered
            .iter()
            .filter_map(|e| shape_outline(e).map(|p| p.transformed(&world_matrix(doc.els(), e.id().unwrap_or(""))).transformed(&to_parent)))
            .collect();
        let refs: Vec<&VectorPath> = paths.iter().collect();
        let out = boolean(&refs, op);
        if out.is_empty() {
            return None;
        }
        let bottom = ordered.first()?;
        let top = ordered.last()?.id().unwrap_or("").to_string();
        let mut base = doc.raw_el(bottom.id().unwrap_or(""))?.clone();
        let id = self.fresh_id();
        base.insert("id".into(), json!(id));
        base.remove("deleted");
        base.insert("name".into(), json!(match op {
            BoolOp::Union => "Union",
            BoolOp::Subtract => "Subtract",
            BoolOp::Intersect => "Intersect",
            BoolOp::Exclude => "Exclude",
        }));
        let el = path_element(base, &out)?;
        let mut ops = vec![op_add(el, &parent, Some(&top))];
        ops.extend(ids.iter().map(|i| DocOp::Delete { id: i.clone() }));
        if !self.apply(ops, false).ok {
            return None;
        }
        self.select(std::slice::from_ref(&id), false);
        Some(id)
    }

    /// Replace an element by a filled path (`d` in the element's own pixel space): outline text, flatten a shape.
    /// Keeps position, rotation, opacity, effects and name. Returns the new path's id.
    pub fn replace_with_path(&mut self, id: &str, d: &str, fill: Option<Paint>) -> Option<String> {
        let doc = self.doc();
        let el = doc.live(id)?.clone();
        if locked_content(&el) || self.is_brand_locked(Some(&el)) {
            return None;
        }
        let path = VectorPath::parse(d);
        let mut patch = path_patch(&el_xf(&el), &path)?;
        let raw = doc.raw_el(id)?;
        let nid = self.fresh_id();
        let mut m = Map::new();
        for k in ["parentId", "z", "opacity", "blend", "visible", "name", "role", "link", "altText", "decorative", "effects", "anim", "emphasis", "keyframes", "animOut", "motionPath", "timing"] {
            if let Some(v) = raw.get(k) {
                m.insert(k.into(), v.clone());
            }
        }
        m.insert("id".into(), json!(nid));
        m.insert("type".into(), json!("shape"));
        m.append(&mut patch);
        let fill = fill.map_or_else(|| json!({ "k": "solid", "color": { "space": "srgb", "r": 0.12, "g": 0.14, "b": 0.16, "a": 1 } }), |p| jv(&p));
        m.insert("fills".into(), json!([fill]));
        m.insert("strokes".into(), json!([]));
        let parent = el.parent_id().unwrap_or("").to_string();
        if !self.apply(vec![op_add(m, &parent, Some(id)), DocOp::Delete { id: id.to_string() }], false).ok {
            return None;
        }
        self.select(std::slice::from_ref(&nid), false);
        Some(nid)
    }

    /// Replace an element by a group of elements given in its own pixel space (SVG import as editable vectors).
    /// `els` are element JSON: roots have `parentId` `""` (they go into the new group), the rest name their parent.
    pub fn replace_with_elements(&mut self, id: &str, els: Vec<Map<String, Value>>) -> Option<String> {
        let doc = self.doc();
        let el = doc.live(id)?.clone();
        if locked_content(&el) || self.is_brand_locked(Some(&el)) || els.is_empty() {
            return None;
        }
        let parent = el.parent_id().unwrap_or("").to_string();
        let base = self.fresh_base(el_xf(&el));
        let mut g = crate::util::jm(&Element::Group(oc_schema::factory::make_group(base)));
        if let Some(name) = doc.raw_el(id).and_then(|r| r.get("name")) {
            g.insert("name".into(), name.clone());
        }
        let gid = crate::util::s(g.get("id")).unwrap_or("").to_string();
        let mut ops = vec![op_add(g, &parent, Some(id))];
        for mut e in els {
            let p = crate::util::s(e.get("parentId")).unwrap_or("").to_string();
            let p = if p.is_empty() { gid.clone() } else { p };
            e.insert("parentId".into(), json!(p));
            ops.push(op_add(e, &p, None));
        }
        ops.push(DocOp::Delete { id: id.to_string() });
        if !self.apply(ops, false).ok {
            return None;
        }
        self.select(std::slice::from_ref(&gid), false);
        Some(gid)
    }

    // ------------------------------------------------------------------ masks (DS-107)

    /// Mask the selection: the top-most selected element becomes the mask of the others (same parent, at least
    /// two). Returns the masked group's id.
    pub fn mask_selection(&mut self, mode: MaskMode) -> Option<String> {
        let doc = self.doc();
        let sel = self.selected();
        if sel.len() < 2 {
            return None;
        }
        let parent = sel[0].parent_id().unwrap_or("").to_string();
        let ids: Vec<String> = sel.iter().map(el_id).collect();
        let top = children_of(doc.els(), &parent).into_iter().rfind(|e| ids.iter().any(|i| Some(i.as_str()) == e.id()))?.id()?.to_string();
        let mask = json!({ "id": top, "mode": mode.as_str() });
        self.group_with(Some(mask))
    }

    /// The masked group the selection belongs to: a selected masked group, or the parent of a selected child.
    pub fn masked_group(&self) -> Option<String> {
        let doc = self.doc();
        let id = self.state.selection.first()?;
        let el = doc.el(id)?;
        if let Element::Group(g) = el
            && g.mask.is_some()
        {
            return Some(id.clone());
        }
        let p = el.parent_id()?;
        match doc.el(p) {
            Some(Element::Group(g)) if g.mask.is_some() => Some(p.to_string()),
            _ => None,
        }
    }

    /// Switch the mask of the selected masked group between alpha and luminance.
    pub fn set_mask_mode(&mut self, mode: MaskMode) -> bool {
        let Some(gid) = self.masked_group() else { return false };
        let Some(Element::Group(g)) = self.doc().el(&gid).cloned() else { return false };
        let Some(mut m) = g.mask.clone() else { return false };
        if m.mode == mode {
            return false;
        }
        m.mode = mode;
        self.apply(vec![op_update(&gid, one("mask", jv(&m)))], false).ok
    }

    /// Release the mask: the group's elements go back to the page (or parent) as they were.
    pub fn release_mask(&mut self) -> bool {
        let Some(gid) = self.masked_group() else { return false };
        self.select(std::slice::from_ref(&gid), false);
        self.ungroup();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_patch_keeps_the_outline_in_place() {
        // A rotated 100×50 box whose outline grows to the right: the box moves so the old corner stays put.
        let origin = Xf::new(10.0, 20.0, 100.0, 50.0, 30.0);
        let path = VectorPath::parse("M0 0 L150 0 L150 50 L0 50 Z");
        let patch = path_patch(&origin, &path).unwrap();
        let xf: Xf = serde_json::from_value(patch["xf"].clone()).unwrap();
        assert_eq!((xf.w, xf.h, xf.rot), (150.0, 50.0, 30.0));
        let before = apply(&local_matrix(&origin), [0.0, 0.0]);
        let after = apply(&local_matrix(&xf), [0.0, 0.0]);
        assert!(dist(before, after) < 1e-9, "{before:?} {after:?}");
        assert_eq!(patch["d"], json!("M0 0 L1 0 L1 1 L0 1 Z"));
    }

    #[test]
    fn snap45_constrains() {
        let p = snap45([0.0, 0.0], [10.0, 1.0]);
        assert!((p[1]).abs() < 1e-9);
        let q = snap45([0.0, 0.0], [10.0, 9.0]);
        assert!((q[0] - q[1]).abs() < 1e-9);
    }
}
