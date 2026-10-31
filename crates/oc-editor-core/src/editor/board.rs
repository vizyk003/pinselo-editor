// SPDX-License-Identifier: Apache-2.0
//! Whiteboard facilitation (WB-004, WB-020..): shape labels, sticky notes, sections, mind maps; freehand draw.

use oc_crdt::rng::new_id;
use oc_crdt::DocOp;
use oc_geometry::{selection_bounds, world_bounds};
use oc_schema::color::{solid_hex};
use oc_schema::design::Surface;
use oc_schema::elements::{
    BoardMeta, DropShadowEffect, Effect, Element, ElementBase, LineAttach, LineRoute, MindMeta, SectionCollapsed, SectionMeta,
    ShapeKind, StickyAuthor, StickyMeta,
};
use oc_schema::factory::{
    DrawOpts, LineOpts, ShapeOpts, TextOpts, base_init, make_draw, make_group, make_line, make_shape, make_stroke, make_text,
};
use oc_schema::primitives::{ColorValue, Rect, Vec2, Xf};
use oc_schema::text::{RunStyleOverride, TextAlign};
use oc_schema::tree::{children_of, descendants};
use oc_schema::Extra;
use serde_json::{Map, Value};

use super::{Editor, el_id, el_xf, locked_content, locked_move, one, op_add, op_update, op_xf};
use oc_schema::tree::Elements;
use crate::state::*;
use crate::util::{jm, jv, max, min, round};
use crate::viewport::fit_rect;
use crate::whiteboard::{
    cluster_layout, is_section, is_sticky, mind_child_position, mind_layout, mind_node_of, mind_root, section_members, sections_of,
};

fn hexc(s: &str) -> ColorValue {
    oc_schema::color::hex(s).unwrap_or_else(|_| ColorValue::srgb(0.0, 0.0, 0.0, 1.0))
}

fn board_of(el: &Element) -> Option<&BoardMeta> {
    el.base().and_then(|b| b.board.as_ref())
}

impl Editor {
    /// A fresh `BaseInit` with an id drawn from the editor's random source (TS `makeX` draws `newId()` first).
    pub(crate) fn fresh_base(&mut self, xf: Xf) -> ElementBase {
        let mut b = base_init(xf);
        b.id = new_id(self.rng.as_mut());
        b
    }

    pub(crate) fn fresh_id(&mut self) -> String {
        new_id(self.rng.as_mut())
    }

    /// Shape labels (WB-004): text that lives inside a shape. The label is a centred text element grouped
    /// with the shape, so moving or resizing the shape carries it along.
    pub fn edit_shape_label(&mut self, shape_id: &str) {
        let doc = self.doc();
        let Some(shape @ Element::Shape(sh)) = doc.el(shape_id) else { return };
        if locked_content(shape) {
            return;
        }
        let parent = doc.el(&sh.base.parent_id);
        // Already labelled: edit the existing text.
        if let Some(Element::Group(pg)) = parent {
            let label = children_of(doc.els(), &pg.base.id)
                .into_iter()
                .find(|e| matches!(e, Element::Text(t) if t.base.name.as_deref() == Some("Shape label")))
                .map(el_id);
            if let Some(label) = label {
                let gid = pg.base.id.clone();
                self.state.entered_group = Some(gid.clone());
                self.select(std::slice::from_ref(&label), false);
                self.state.entered_group = Some(gid);
                self.start_text_edit(&label, None, false);
                return;
            }
        }
        let sx = sh.base.xf.clone();
        let pad = min(sx.w, sx.h) * 0.12;
        let size = max(12.0, min(48.0, round(min(sx.w, sx.h) / 6.0)));
        let base = self.fresh_base(Xf::new(sx.x + pad, sx.y + sx.h / 2.0 - size * 0.7, max(20.0, sx.w - pad * 2.0), size * 1.4, sx.rot));
        let mut text = make_text(
            base,
            TextOpts {
                text: Some(String::new()),
                style: Some(RunStyleOverride { size: Some(size), ..Default::default() }),
                align: Some(TextAlign::Center),
                body: None,
            },
        );
        text.base.name = Some("Shape label".into());
        if doc.page_of(shape_id).is_none() {
            return;
        }
        let text_id = text.base.id.clone();
        text.base.parent_id = sh.base.parent_id.clone();
        let parent_id = sh.base.parent_id.clone();
        let had_board = sh.base.board.is_some();
        self.apply(vec![op_add(jm(&Element::Text(text)), &parent_id, Some(shape_id))], false);
        self.select(&[shape_id.to_string(), text_id.clone()], false);
        self.group();
        let g = self.state.selection.first().cloned();
        if let Some(g) = &g
            && had_board
        {
            self.adopt_board_role(shape_id, g);
        }
        if let Some(g) = g {
            self.state.entered_group = Some(g.clone());
            self.select(std::slice::from_ref(&text_id), false);
            self.state.entered_group = Some(g);
            self.start_text_edit(&text_id, None, false);
        }
    }

    /// A shape that gains a label is wrapped in a group; the group takes over its whiteboard role (mind-map node,
    /// section, …): the metadata, glued connectors and mind-map children now point at the group.
    fn adopt_board_role(&mut self, from_id: &str, to_id: &str) {
        let doc = self.doc();
        let Some(from) = doc.el(from_id) else { return };
        let Some(board) = board_of(from) else { return };
        let mut ops = vec![op_update(to_id, one("board", jv(board))), op_update(from_id, one("board", Value::Null))];
        for e in doc.els().values() {
            if e.is_deleted() || e.id() == Some(from_id) {
                continue;
            }
            if let Some(b) = board_of(e)
                && b.mind.as_ref().and_then(|m| m.parent.as_ref()).and_then(|p| p.as_deref()) == Some(from_id)
            {
                let mut nb = jm(b);
                nb.insert("mind".into(), Value::Object(one("parent", Value::String(to_id.into()))));
                ops.push(op_update(&el_id(e), one("board", Value::Object(nb))));
            }
            if let Element::Line(l) = e {
                let mut patch = Map::new();
                if let Some(a) = l.start_attach.as_ref().filter(|a| a.id == from_id) {
                    patch.insert("startAttach".into(), jv(&LineAttach { id: to_id.into(), ..a.clone() }));
                }
                if let Some(a) = l.end_attach.as_ref().filter(|a| a.id == from_id) {
                    patch.insert("endAttach".into(), jv(&LineAttach { id: to_id.into(), ..a.clone() }));
                }
                if !patch.is_empty() {
                    ops.push(op_update(&l.base.id, patch));
                }
            }
        }
        self.apply(ops, true);
    }

    /// Centre of the visible canvas in page coordinates (default: the active page).
    pub fn view_center(&self, page_id: Option<&str>) -> Vec2 {
        let vp = self.state.viewport;
        let pid = page_id.map(str::to_string).unwrap_or_else(|| self.state.active_page_id.clone());
        self.to_page(&pid, [vp.x + vp.width / vp.zoom / 2.0, vp.y + vp.height / vp.zoom / 2.0])
    }

    /// Add a sticky note: a group of a coloured square and a handwriting text box, tagged with its colour and
    /// author (`board.sticky`). Placed at `at` (page coords) or the centre of the view. Returns the group id.
    /// `author`: `None` = the editor's user, `Some(None)` = anonymous.
    pub fn add_sticky(&mut self, color: &str, text: Option<&str>, at: Option<Vec2>, size: Option<f64>, author: Option<Option<User>>) -> Option<String> {
        let s = max(40.0, round(size.unwrap_or(240.0)));
        let author = match author {
            None => self.user.clone(),
            Some(a) => a,
        };
        let mut gb = self.fresh_base(Xf::new(0.0, 0.0, s, s, 0.0));
        gb.name = Some("Sticky note".into());
        gb.board = Some(BoardMeta {
            sticky: Some(StickyMeta {
                color: color.to_string(),
                author: author.map(|a| StickyAuthor { id: a.id, name: a.name, extra: Extra::new() }),
                extra: Extra::new(),
            }),
            ..Default::default()
        });
        let g = make_group(gb);
        let bb = self.fresh_base(Xf::new(0.0, 0.0, s, s, 0.0));
        let mut params = indexmap::IndexMap::new();
        params.insert("cornerRadius".to_string(), 4.0);
        let mut bg = make_shape(bb, ShapeOpts { kind: Some(ShapeKind::Rect), fill: solid_hex(color).ok(), stroke: None, params: Some(params) });
        bg.base.parent_id = g.base.id.clone();
        bg.base.effects = Some(vec![Effect::DropShadow(DropShadowEffect {
            dx: 0.0,
            dy: s * 0.03,
            blur: s * 0.08,
            color: ColorValue::srgb(0.0, 0.0, 0.0, 0.18),
            extra: Extra::new(),
        })]);
        let tb = self.fresh_base(Xf::new(s * 0.1, s * 0.1, s * 0.8, s * 0.2, 0.0));
        let mut txt = make_text(
            tb,
            TextOpts {
                text: Some(text.unwrap_or("").to_string()),
                style: Some(RunStyleOverride { family: Some("Caveat".into()), size: Some(round(s * 0.12)), ..Default::default() }),
                align: None,
                body: None,
            },
        );
        txt.base.parent_id = g.base.id.clone();
        let at = at.or_else(|| (self.doc().doc.meta.surface == Surface::Whiteboard).then(|| self.view_center(None)));
        let ids = self.insert(vec![jm(&Element::Group(g)), jm(&Element::Shape(bg)), jm(&Element::Text(txt))], at, true, false);
        ids.into_iter().next()
    }

    /// Recolour sticky notes (the selection by default). Returns how many were recoloured.
    pub fn set_sticky_color(&mut self, color: &str, ids: Option<&[String]>) -> usize {
        let ids: Vec<String> = ids.map(<[String]>::to_vec).unwrap_or_else(|| self.state.selection.clone());
        let doc = self.doc();
        let mut ops = Vec::new();
        for id in &ids {
            let Some(g) = doc.el(id) else { continue };
            if !is_sticky(Some(g)) {
                continue;
            }
            let mut b = board_of(g).cloned().unwrap_or_default();
            if let Some(st) = &mut b.sticky {
                st.color = color.to_string();
            }
            ops.push(op_update(id, one("board", jv(&b))));
            if let Some(bg) = children_of(doc.els(), id).into_iter().find(|e| matches!(e, Element::Shape(_))) {
                let fill = solid_hex(color).map(|p| jv(&vec![p])).unwrap_or(Value::Array(vec![]));
                ops.push(op_update(&el_id(bg), one("fills", fill)));
            }
        }
        let n = ops.len();
        if n > 0 {
            self.apply(ops, false);
        }
        if n > 0 { ids.len() } else { 0 }
    }

    /// Arrange the selected sticky notes into one tidy grid per colour. Returns how many moved.
    pub fn cluster_stickies(&mut self, ids: Option<&[String]>) -> usize {
        let ids: Vec<String> = ids.map(<[String]>::to_vec).unwrap_or_else(|| self.state.selection.clone());
        let doc = self.doc();
        let layout = cluster_layout(doc.els(), &ids, 24.0);
        let mut ops = Vec::new();
        for (id, (x, y)) in layout {
            let Some(el) = doc.el(&id) else { continue };
            if locked_move(el) {
                continue;
            }
            ops.push(op_xf(&id, &Xf { x, y, ..el_xf(el) }));
        }
        let n = ops.len();
        if n > 0 {
            self.apply(ops, false);
        }
        n
    }

    /// New section: a titled rect at the back that carries the elements inside it. Wraps the selection when
    /// there is one, otherwise appears in the middle of the view.
    pub fn create_section(&mut self, title: &str) -> Option<String> {
        let doc = self.doc();
        let page_id = self.state.active_page_id.clone();
        if !doc.has_page(&page_id) {
            return None;
        }
        let sel: Vec<String> =
            self.state.selection.iter().filter(|id| doc.el(id).and_then(|e| e.parent_id()) == Some(page_id.as_str())).cloned().collect();
        let b = if sel.is_empty() { None } else { selection_bounds(doc.els(), &sel) };
        let bar = 56.0;
        let rect = match b {
            Some(b) => {
                let pad = 40.0;
                Rect::new(b.x - pad, b.y - pad - bar, b.w + pad * 2.0, b.h + pad * 2.0 + bar)
            }
            None => {
                let vp = self.state.viewport;
                let [cx, cy] = self.view_center(Some(&page_id));
                let w = round(min(1200.0, (vp.width / vp.zoom) * 0.6));
                let h = round(w * 0.66);
                Rect::new(cx - w / 2.0, cy - h / 2.0, w, h)
            }
        };
        let existing = sections_of(doc.els(), &page_id);
        let order = existing
            .iter()
            .filter_map(|s| board_of(s).and_then(|b| b.section.as_ref()).map(|x| x.order))
            .fold(0.0, max)
            + 1.0;
        let base = self.fresh_base(Xf::new(rect.x, rect.y, rect.w, rect.h, 0.0));
        let mut params = indexmap::IndexMap::new();
        params.insert("cornerRadius".to_string(), 16.0);
        let mut el = make_shape(
            base,
            ShapeOpts {
                kind: Some(ShapeKind::Rect),
                fill: solid_hex("#f1f5f9").ok(),
                stroke: Some(make_stroke(Some(hexc("#cbd5e1")), Some(2.0), None)),
                params: Some(params),
            },
        );
        el.base.name = Some(title.to_string());
        el.base.board = Some(BoardMeta {
            section: Some(SectionMeta { title: title.to_string(), order, collapsed: None, extra: Extra::new() }),
            ..Default::default()
        });
        // Above any section that encloses it (nested sections stay visible), otherwise at the very back.
        let holder = children_of(doc.els(), &page_id)
            .into_iter()
            .rfind(|e| {
                let x = el_xf(e);
                is_section(Some(e)) && x.x <= rect.x && x.y <= rect.y && x.x + x.w >= rect.x + rect.w && x.y + x.h >= rect.y + rect.h
            })
            .map(el_id);
        el.base.parent_id = page_id.clone();
        let id = el.base.id.clone();
        let r = self.apply(vec![op_add(jm(&Element::Shape(el)), &page_id, Some(holder.as_deref().unwrap_or("bottom")))], false);
        if !r.ok {
            return None;
        }
        self.select(std::slice::from_ref(&id), false);
        Some(id)
    }

    pub fn rename_section(&mut self, id: &str, title: &str) {
        let doc = self.doc();
        let Some(el) = doc.el(id) else { return };
        let Some(b) = board_of(el).filter(|b| b.section.is_some()) else { return };
        let mut nb = b.clone();
        if let Some(s) = &mut nb.section {
            s.title = title.to_string();
        }
        let mut patch = one("name", Value::String(title.into()));
        patch.insert("board".into(), jv(&nb));
        self.apply(vec![op_update(id, patch)], false);
    }

    /// Collapse a section to its title bar (hiding its members) or expand it again.
    pub fn toggle_section(&mut self, id: &str) {
        let doc = self.doc();
        let Some(el) = doc.el(id) else { return };
        let Some(b) = board_of(el) else { return };
        let Some(sec) = &b.section else { return };
        let x = el_xf(el);
        let mut ops: Vec<DocOp> = Vec::new();
        if let Some(collapsed) = &sec.collapsed {
            let mut nb = b.clone();
            if let Some(s) = &mut nb.section {
                s.collapsed = None;
            }
            ops.push(op_update(id, one("board", jv(&nb))));
            ops.push(op_xf(id, &Xf { h: collapsed.h, ..x }));
            for m in &collapsed.hidden {
                if doc.live(m).is_some() {
                    ops.push(op_update(m, one("visible", Value::Null)));
                }
            }
        } else {
            let hidden: Vec<String> = section_members(doc.els(), id)
                .into_iter()
                .filter(|m| doc.el(m).is_some_and(|e| super::visible(e) && !locked_move(e)))
                .collect();
            let mut nb = b.clone();
            if let Some(s) = &mut nb.section {
                s.collapsed = Some(SectionCollapsed { h: x.h, hidden: hidden.clone(), extra: Extra::new() });
            }
            ops.push(op_update(id, one("board", jv(&nb))));
            ops.push(op_xf(id, &Xf { h: min(x.h, 64.0), ..x }));
            for m in &hidden {
                ops.push(op_update(m, one("visible", Value::Bool(false))));
            }
            self.set(|s| s.selection.retain(|x| !hidden.contains(x)));
        }
        self.apply(ops, false);
    }

    /// Move a section up (-1) or down (+1) in the navigator order.
    pub fn move_section(&mut self, id: &str, dir: i32) {
        let doc = self.doc();
        let Some(page_id) = doc.page_of(id) else { return };
        let mut list: Vec<Element> = sections_of(doc.els(), &page_id).into_iter().cloned().collect();
        let Some(i) = list.iter().position(|s| s.id() == Some(id)) else { return };
        let j = i as i64 + dir as i64;
        if j < 0 || j >= list.len() as i64 {
            return;
        }
        list.swap(i, j as usize);
        let mut ops = Vec::new();
        for (k, s) in list.iter().enumerate() {
            let Some(b) = board_of(s) else { continue };
            let Some(sec) = &b.section else { continue };
            if sec.order != (k + 1) as f64 {
                let mut nb = b.clone();
                if let Some(x) = &mut nb.section {
                    x.order = (k + 1) as f64;
                }
                ops.push(op_update(&el_id(s), one("board", jv(&nb))));
            }
        }
        if !ops.is_empty() {
            self.apply(ops, false);
        }
    }

    /// Fit a section (or any element) in the view. TS default `pad = 48`.
    pub fn zoom_to_element(&mut self, id: &str, pad: f64) {
        let doc = self.doc();
        let Some(page_id) = doc.page_of(id) else { return };
        if doc.el(id).is_none() {
            return;
        }
        let [ox, oy] = self.page_origin(&page_id);
        let b = world_bounds(doc.els(), id).unwrap_or_default();
        self.set_active_page(&page_id, false);
        let vp = fit_rect(&self.state.viewport, &Rect::new(b.x + ox, b.y + oy, b.w, b.h), pad);
        self.set_viewport(vp);
    }

    /// Mind map (WB-040): add a child (to the right, glued with a curved connector) or a sibling of the selected
    /// node. The new node copies the node's look with empty text and starts text editing. Returns its id.
    pub fn mind_map_add(&mut self, child: bool) -> Option<String> {
        let doc = self.doc();
        let from = self
            .state
            .editing_text
            .as_ref()
            .map(|e| e.id.clone())
            .or_else(|| (self.state.selection.len() == 1).then(|| self.state.selection[0].clone()))?;
        let node = mind_node_of(doc.els(), &from)?.clone();
        if locked_content(&node) || is_section(Some(&node)) || matches!(node, Element::Line(_)) || !doc.has_page(node.parent_id().unwrap_or("")) {
            return None;
        }
        let node_board = board_of(&node).cloned();
        if self.state.editing_text.is_some() && node_board.as_ref().and_then(|b| b.mind.as_ref()).is_none() {
            return None;
        }
        let node_id = el_id(&node);
        let parent_id = if child {
            Some(node_id.clone())
        } else {
            node_board.as_ref().and_then(|b| b.mind.as_ref()).and_then(|m| m.parent.clone()).flatten()
        }?;
        let parent = doc.el(&parent_id).filter(|p| !p.is_deleted())?.clone();
        let page_id = node.parent_id().unwrap_or("").to_string();
        let (elements, roots) = self.clone_subtrees(std::slice::from_ref(&node_id));
        let root_id = roots.first()?.clone();
        let (px, py) = mind_child_position(doc.els(), &parent_id, el_xf(&node).h);
        let user = self.user.clone();
        let els: Vec<Map<String, Value>> = elements
            .into_iter()
            .map(|mut e| {
                if crate::util::s(e.get("id")) == Some(root_id.as_str()) {
                    let mut board = e.get("board").and_then(Value::as_object).cloned().unwrap_or_default();
                    board.insert("mind".into(), Value::Object(one("parent", Value::String(parent_id.clone()))));
                    if let Some(st) = board.get("sticky").and_then(Value::as_object).cloned() {
                        let mut ns = one("color", st.get("color").cloned().unwrap_or(Value::Null));
                        if let Some(u) = &user {
                            ns.insert("author".into(), jv(&StickyAuthor { id: u.id.clone(), name: u.name.clone(), extra: Extra::new() }));
                        }
                        board.insert("sticky".into(), Value::Object(ns));
                    }
                    if let Some(Value::Object(xf)) = e.get_mut("xf") {
                        xf.insert("x".into(), crate::util::num(px));
                        xf.insert("y".into(), crate::util::num(py));
                    }
                    e.insert("board".into(), Value::Object(board));
                }
                if crate::util::s(e.get("type")) == Some("text")
                    && let Some(Value::Object(body)) = e.get_mut("body")
                    && let Some(Value::Array(ps)) = body.get_mut("paragraphs")
                    && let Some(Value::Object(p0)) = ps.first().cloned()
                {
                    let mut p0 = p0;
                    p0.insert("runs".into(), Value::Array(vec![]));
                    *ps = vec![Value::Object(p0)];
                }
                e
            })
            .collect();
        let mut ops: Vec<DocOp> = Vec::new();
        let parent_board = board_of(&parent).cloned();
        if child && parent_board.as_ref().and_then(|b| b.mind.as_ref()).is_none() {
            let mut nb = parent_board.unwrap_or_default();
            nb.mind = Some(MindMeta { parent: Some(None), extra: Extra::new() });
            ops.push(op_update(&parent_id, one("board", jv(&nb))));
        }
        for mut e in els {
            let is_root = crate::util::s(e.get("id")) == Some(root_id.as_str());
            if is_root {
                e.insert("parentId".into(), Value::String(page_id.clone()));
            }
            let parent = if is_root { page_id.clone() } else { crate::util::s(e.get("parentId")).unwrap_or("").to_string() };
            ops.push(op_add(e, &parent, None));
        }
        let lb = self.fresh_base(Xf::new(0.0, 0.0, 1.0, 1.0, 0.0));
        let mut line = make_line(lb, LineOpts { color: Some(hexc("#94a3b8")), width: Some(3.0), ..Default::default() });
        line.base.parent_id = page_id.clone();
        line.base.name = Some("Mind map link".into());
        line.route = Some(LineRoute::Curved);
        line.start_attach = Some(LineAttach { id: parent_id.clone(), anchor: oc_schema::elements::AnchorSide::Right, extra: Extra::new() });
        line.end_attach = Some(LineAttach { id: root_id.clone(), anchor: oc_schema::elements::AnchorSide::Left, extra: Extra::new() });
        ops.push(op_add(jm(&Element::Line(line)), &page_id, None));
        if self.state.editing_text.is_some() {
            self.set(|s| s.editing_text = None);
        }
        if !self.apply(ops, false).ok {
            return None;
        }
        self.set(|s| s.entered_group = None);
        self.select(std::slice::from_ref(&root_id), false);
        let root = self.doc().el(&root_id).cloned();
        match root {
            Some(Element::Group(_)) => {
                self.edit_group_text(&root_id);
            }
            Some(Element::Text(_)) => self.start_text_edit(&root_id, None, false),
            Some(Element::Shape(_)) => self.edit_shape_label(&root_id),
            _ => {}
        }
        // Labelling a bare shape wraps it in a group, which becomes the node.
        let d = self.doc();
        Some(mind_node_of(d.els(), &root_id).map_or(root_id.clone(), el_id))
    }

    /// Enter a group (sticky note, labelled shape) and start editing its first text element.
    pub fn edit_group_text(&mut self, group_id: &str) -> bool {
        let doc = self.doc();
        let Some(label) = descendants(doc.els(), group_id).into_iter().find(|e| matches!(e, Element::Text(_)) && !e.is_deleted()) else {
            return false;
        };
        let parent = label.parent_id().unwrap_or("").to_string();
        let lid = el_id(label);
        self.state.entered_group = Some(parent.clone());
        self.select(std::slice::from_ref(&lid), false);
        self.state.entered_group = Some(parent);
        self.start_text_edit(&lid, None, false);
        true
    }

    /// Re-layout the mind map containing the element (default: the first selected) as a balanced tree.
    /// Returns how many nodes moved.
    pub fn layout_mind_map(&mut self, id: Option<&str>) -> usize {
        let Some(id) = id.map(str::to_string).or_else(|| self.state.selection.first().cloned()) else { return 0 };
        let doc = self.doc();
        let Some(node) = mind_node_of(doc.els(), &id) else { return 0 };
        let pos = mind_layout(doc.els(), &mind_root(doc.els(), &el_id(node)), None, None);
        let mut ops = Vec::new();
        for (nid, (x, y)) in pos {
            let Some(el) = doc.el(&nid) else { continue };
            let ex = el_xf(el);
            if locked_move(el) || ((ex.x - x).abs() < 0.5 && (ex.y - y).abs() < 0.5) {
                continue;
            }
            ops.push(op_xf(&nid, &Xf { x, y, ..ex }));
        }
        let n = ops.len();
        if n > 0 {
            self.apply(ops, false);
        }
        n
    }

    // ------------------------------------------------------------------ freehand draw (PT-010)

    pub fn set_tool(&mut self, tool: Tool) {
        self.set(|s| {
            s.tool = tool;
            s.drawing = None;
            if tool != Tool::Pen {
                s.pen = None;
            }
            if tool != Tool::Select {
                s.path_edit = None;
            }
            if tool != Tool::Select {
                s.editing_text = None;
            }
            if matches!(tool, Tool::Draw | Tool::Erase | Tool::Pen) {
                s.selection = vec![];
            }
        });
    }

    /// Patch the brush (`kind`, `color`, `size`).
    pub fn set_brush(&mut self, kind: Option<oc_schema::elements::Brush>, color: Option<String>, size: Option<f64>) {
        self.set(|s| {
            if let Some(k) = kind {
                s.brush.kind = k;
            }
            if let Some(c) = color {
                s.brush.color = c;
            }
            if let Some(z) = size {
                s.brush.size = z;
            }
        });
    }

    pub(crate) fn finish_stroke(&mut self) {
        let Some(d) = self.state.drawing.clone() else { return };
        self.set(|s| s.drawing = None);
        let pts = &d.points;
        if pts.len() < 3 {
            return;
        }
        let BrushState { size, kind, color } = self.state.brush.clone();
        let pad = size
            * match kind {
                oc_schema::elements::Brush::Highlighter => 3.0,
                oc_schema::elements::Brush::Marker => 1.6,
                _ => 1.0,
            };
        let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for c in pts.chunks(3) {
            x0 = min(x0, c[0]);
            y0 = min(y0, c[1]);
            x1 = max(x1, c[0]);
            y1 = max(y1, c[1]);
        }
        x0 -= pad;
        y0 -= pad;
        x1 += pad;
        y1 += pad;
        let mut local = Vec::with_capacity(pts.len());
        for c in pts.chunks(3) {
            local.push(oc_schema::js::to_fixed_num(c[0] - x0, 2));
            local.push(oc_schema::js::to_fixed_num(c[1] - y0, 2));
            local.push(c.get(2).copied().unwrap_or(0.5));
        }
        let w = max(1.0, x1 - x0);
        let h = max(1.0, y1 - y0);
        let base = self.fresh_base(Xf::new(x0, y0, w, h, 0.0));
        let el = make_draw(base, local, DrawOpts { brush: Some(kind), color: Some(hexc(&color)), size: Some(size) });
        self.apply(vec![op_add(jm(&Element::Draw(el)), &d.page_id, None)], false);
    }

    /// End an eraser drag (g55): the trail erases what it crossed, as one undo step.
    pub(crate) fn finish_erase(&mut self) {
        let Some(d) = self.state.drawing.clone() else { return };
        self.set(|s| s.drawing = None);
        let radius = eraser_radius(self.state.brush.size);
        self.erase_strokes(&d.page_id, &d.points, radius);
    }

    /// Erase the parts of the page's freehand strokes within `radius` (page px) of `path` (page-space
    /// `[x, y, pressure]` triples, the eraser trail). A crossed stroke splits into the pieces left on either side;
    /// a stroke erased whole is deleted. Locked and hidden strokes stay. One undo step; returns the strokes changed.
    pub fn erase_strokes(&mut self, page_id: &str, path: &[f64], radius: f64) -> usize {
        let doc = self.doc();
        if doc.page_locked(page_id) || path.len() < 2 {
            return 0;
        }
        let trail: Vec<Vec2> = path.chunks(3).filter(|c| c.len() >= 2).map(|c| [c[0], c[1]]).collect();
        let mut ops: Vec<DocOp> = Vec::new();
        let mut changed = 0;
        let strokes: Vec<oc_schema::elements::DrawEl> = doc
            .els()
            .values()
            .filter_map(|e| match e {
                Element::Draw(d) if !e.is_deleted() && super::visible(e) && !locked_move(e) && !locked_content(e) => Some(d.clone()),
                _ => None,
            })
            .filter(|d| doc.page_of(&d.base.id).as_deref() == Some(page_id))
            .collect();
        for d in strokes {
            let Some(pieces) = erase_stroke(doc.els(), &d, &trail, radius) else { continue };
            changed += 1;
            let id = d.base.id.clone();
            if pieces.is_empty() {
                ops.push(DocOp::Delete { id });
                continue;
            }
            let raw = doc.raw_el(&id).cloned().unwrap_or_default();
            let parent = d.base.parent_id.clone();
            let mut prev = id.clone();
            for (i, (xf, points, base)) in pieces.into_iter().enumerate() {
                if i == 0 {
                    ops.push(op_xf(&id, &xf));
                    let mut patch = one("points", jv(&points));
                    patch.insert("base".into(), jv(&base));
                    ops.push(op_update(&id, patch));
                } else {
                    let mut el = raw.clone();
                    let nid = self.fresh_id();
                    el.insert("id".into(), Value::String(nid.clone()));
                    el.insert("xf".into(), jv(&xf));
                    el.insert("points".into(), jv(&points));
                    el.insert("base".into(), jv(&base));
                    ops.push(op_add(el, &parent, Some(&prev)));
                    prev = nid;
                }
            }
        }
        if !ops.is_empty() {
            self.apply(ops, false);
            let doc = self.doc();
            let sel: Vec<String> = self.state.selection.iter().filter(|i| doc.live(i).is_some()).cloned().collect();
            self.set(|s| s.selection = sel);
        }
        changed
    }
}

/// Eraser radius (page px) for a brush weight: the weight slider sets the eraser's width too.
pub(crate) fn eraser_radius(size: f64) -> f64 {
    max(4.0, size)
}

fn brush_factor(b: &oc_schema::elements::Brush) -> f64 {
    match b {
        oc_schema::elements::Brush::Highlighter => 3.0,
        oc_schema::elements::Brush::Marker => 1.6,
        _ => 1.0,
    }
}

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 <= 1e-12 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
}

fn trail_dist(p: Vec2, trail: &[Vec2]) -> f64 {
    match trail {
        [] => f64::INFINITY,
        [a] => ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt(),
        _ => trail.windows(2).map(|w| seg_dist(p, w[0], w[1])).fold(f64::INFINITY, f64::min),
    }
}

/// The pieces of one stroke left after erasing along `trail` (page space): each piece's xf, points and base size,
/// with the box fitted to the piece. `None` when the eraser missed the stroke; an empty list when it erased it all.
fn erase_stroke(elements: &Elements, d: &oc_schema::elements::DrawEl, trail: &[Vec2], radius: f64) -> Option<Vec<(Xf, Vec<f64>, oc_schema::elements::Size)>> {
    let x = &d.base.xf;
    let (bw, bh) = (d.base_size.w.max(1e-6), d.base_size.h.max(1e-6));
    let (sx, sy) = (x.w / bw, x.h / bh);
    let m = oc_geometry::world_matrix(elements, &d.base.id);
    // Page px per stroke unit (the element scale times any group scale).
    let unit = ((m[0] * m[3] - m[1] * m[2]).abs() * (sx * sy).abs()).sqrt().max(1e-6);
    let half = d.size * brush_factor(&d.brush) / 2.0;
    let reach = radius + half * unit;
    // Densify so a long segment crossed between two far-apart points is still cut.
    let step = max(0.5, (radius / unit) / 2.0);
    let src: Vec<[f64; 3]> = d.points.chunks(3).filter(|c| c.len() == 3).map(|c| [c[0], c[1], c[2]]).collect();
    if src.is_empty() {
        return None;
    }
    let mut dense: Vec<[f64; 3]> = vec![src[0]];
    for w in src.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let n = (len / step).ceil().clamp(1.0, 400.0) as usize;
        for k in 1..=n {
            let t = k as f64 / n as f64;
            dense.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]);
        }
    }
    let erased: Vec<bool> = dense
        .iter()
        .map(|p| trail_dist(oc_geometry::apply(&m, [p[0] * sx, p[1] * sy]), trail) <= reach)
        .collect();
    if !erased.iter().any(|e| *e) {
        return None;
    }
    let mut runs: Vec<Vec<[f64; 3]>> = Vec::new();
    let mut cur: Vec<[f64; 3]> = Vec::new();
    for (p, e) in dense.iter().zip(&erased) {
        if *e {
            if cur.len() >= 2 {
                runs.push(std::mem::take(&mut cur));
            }
            cur.clear();
        } else {
            cur.push(*p);
        }
    }
    if cur.len() >= 2 {
        runs.push(cur);
    }
    let pad = d.size * brush_factor(&d.brush);
    let (cx, cy) = (x.w / 2.0, x.h / 2.0);
    let rot = x.rot.to_radians();
    let pieces = runs
        .into_iter()
        .map(|run| {
            // Thin the densified run back to about the original spacing (keeps strokes compact).
            let mut kept: Vec<[f64; 3]> = Vec::new();
            for (i, p) in run.iter().enumerate() {
                let last = i + 1 == run.len();
                if kept.last().is_none_or(|q: &[f64; 3]| last || ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt() >= 1.5) {
                    kept.push(*p);
                }
            }
            let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
            for p in &kept {
                x0 = min(x0, p[0]);
                y0 = min(y0, p[1]);
                x1 = max(x1, p[0]);
                y1 = max(y1, p[1]);
            }
            x0 -= pad;
            y0 -= pad;
            x1 += pad;
            y1 += pad;
            let (nbw, nbh) = (max(1.0, x1 - x0), max(1.0, y1 - y0));
            let mut points = Vec::with_capacity(kept.len() * 3);
            for p in &kept {
                points.push(oc_schema::js::to_fixed_num(p[0] - x0, 2));
                points.push(oc_schema::js::to_fixed_num(p[1] - y0, 2));
                points.push(p[2]);
            }
            // The new box's centre, moved from the old one within the element's (flipped, rotated) frame.
            let (nw, nh) = (nbw * sx, nbh * sy);
            let mut dx = (x0 * sx + nw / 2.0) - cx;
            let mut dy = (y0 * sy + nh / 2.0) - cy;
            if x.flip_x == Some(true) {
                dx = -dx;
            }
            if x.flip_y == Some(true) {
                dy = -dy;
            }
            let (c, s) = (rot.cos(), rot.sin());
            let (wx, wy) = (x.x + cx + dx * c - dy * s, x.y + cy + dx * s + dy * c);
            let xf = Xf { x: wx - nw / 2.0, y: wy - nh / 2.0, w: nw, h: nh, ..x.clone() };
            (xf, points, oc_schema::elements::Size { w: nbw, h: nbh, extra: Extra::new() })
        })
        .collect();
    Some(pieces)
}
