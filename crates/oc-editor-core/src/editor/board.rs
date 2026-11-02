// SPDX-License-Identifier: Apache-2.0
//! Shape labels (WB-004) and freehand draw.

use oc_crdt::rng::new_id;
use oc_crdt::DocOp;
use oc_geometry::world_bounds;
use oc_schema::elements::{BoardMeta, Element, ElementBase, LineAttach};
use oc_schema::factory::{DrawOpts, TextOpts, base_init, make_draw, make_text};
use oc_schema::primitives::{ColorValue, Rect, Vec2, Xf};
use oc_schema::text::{RunStyleOverride, TextAlign};
use oc_schema::tree::children_of;
use oc_schema::Extra;
use serde_json::{Map, Value};

use super::{Editor, el_id, locked_content, locked_move, one, op_add, op_update, op_xf};
use oc_schema::tree::Elements;
use crate::state::*;
use crate::util::{jm, jv, max, min, round};
use crate::viewport::fit_rect;

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

    /// A shape that gains a label is wrapped in a group; the group takes over the `board` metadata older documents
    /// carry (mind-map node, section, …): the metadata, glued connectors and mind-map children now point at the group.
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

    /// Fit an element in the view. TS default `pad = 48`.
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
