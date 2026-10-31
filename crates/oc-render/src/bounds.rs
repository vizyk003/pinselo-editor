// SPDX-License-Identifier: Apache-2.0
//! Display-list groups and their bounds, for interactive hosts that repaint only what changed.
//!
//! A display list is a flat op list; every element resolves to one balanced run of ops at the top level (a `save`
//! or `layer` and its matching `restore`, or a single op). [`groups`] splits a list into those runs and computes a
//! conservative page-space bounding box for each: path bounds plus stroke reach, line points plus markers, image
//! and video destinations, text boxes widened by their laid-out size, overflow, glyph overhang and text effects,
//! and layer effects (drop shadow, blur) around everything drawn inside the layer.
//!
//! [`dirty_region`] compares the groups of two lists of the same page and returns the page-space area whose pixels
//! may differ (`Full` when the background, the size or the stacking order changed). [`Renderer::draw_culled`]
//! draws a list skipping groups that lie outside the canvas clip — together they let the editor redraw a drag
//! frame by painting only the union of the moved element's old and new boxes.

use std::collections::HashMap;
use std::ops::Range;

use oc_schema::elements::Effect;
use oc_schema::primitives::{Rect, StrokeAlign};
use oc_schema::text::{TextBody, TextEffect};

use crate::assets::AssetProvider;
use crate::displaylist::{DLItem, DisplayList};
use crate::path;
use crate::raster::{Canvas, Renderer, rect};

/// One element's run of ops.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// The element (`srcEl` of the first op that names one) and its occurrence among groups of that element.
    pub key: (String, u32),
    pub range: Range<usize>,
    /// Page-space bounds; `None` when nothing in the group draws.
    pub bounds: Option<Rect>,
    /// How far (page units) the group's filters (layer shadows and blurs, text effects) gather pixels from: a
    /// partial repaint must draw this much beyond the area it keeps, so blurs see their whole input.
    pub reach: f64,
    /// Hash of the group's ops (equal ops, equal hash), to tell whether cached pixels of it are still current.
    pub hash: u64,
    /// Whether the group composites with a non-normal blend mode (it must be drawn onto what is below it).
    pub blends: bool,
}

/// What changed between two display lists of a page.
#[derive(Debug, Clone, PartialEq)]
pub enum Dirty {
    /// Nothing visible changed.
    None,
    /// Only this page-space area.
    Area(Rect),
    /// The whole page.
    Full,
}

#[derive(Clone, Copy)]
struct Aabb {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Aabb {
    fn add(a: Option<Aabb>, b: Aabb) -> Aabb {
        match a {
            None => b,
            Some(a) => Aabb { x0: a.x0.min(b.x0), y0: a.y0.min(b.y0), x1: a.x1.max(b.x1), y1: a.y1.max(b.y1) },
        }
    }

    fn pad(self, p: f64) -> Aabb {
        Aabb { x0: self.x0 - p, y0: self.y0 - p, x1: self.x1 + p, y1: self.y1 + p }
    }

    fn rect(self) -> Rect {
        Rect { x: self.x0, y: self.y0, w: self.x1 - self.x0, h: self.y1 - self.y0 }
    }

    fn of(r: &Rect) -> Aabb {
        Aabb { x0: r.x, y0: r.y, x1: r.x + r.w, y1: r.y + r.h }
    }

    fn finite(&self) -> bool {
        self.x0.is_finite() && self.y0.is_finite() && self.x1.is_finite() && self.y1.is_finite()
    }
}

type M = [f64; 6];

fn mul(a: &M, b: &M) -> M {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

/// Page-space box of a local rectangle under `m`.
fn map(m: &M, r: Aabb) -> Aabb {
    let pts = [(r.x0, r.y0), (r.x1, r.y0), (r.x0, r.y1), (r.x1, r.y1)];
    let mut out: Option<Aabb> = None;
    for (x, y) in pts {
        let (px, py) = (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        out = Some(Aabb::add(out, Aabb { x0: px, y0: py, x1: px, y1: py }));
    }
    out.unwrap_or(r)
}

/// Largest axis scale of a matrix (for local distances such as blur radii).
fn scale_of(m: &M) -> f64 {
    m[0].hypot(m[1]).max(m[2].hypot(m[3]))
}

fn max_size(body: &TextBody) -> f64 {
    let mut s = body.default_run.size;
    for p in &body.paragraphs {
        for r in &p.runs {
            if let Some(z) = r.style.size {
                s = s.max(z);
            }
        }
    }
    if s.is_finite() { s.max(1.0) } else { 16.0 }
}

/// How far a text effect draws outside the glyphs, in text units.
fn effect_reach(e: Option<&TextEffect>, size: f64) -> f64 {
    match e {
        Some(TextEffect::Shadow(s)) => s.dx.abs() + s.dy.abs() + s.blur * 1.5,
        Some(TextEffect::Neon(n)) => (n.intensity / 100.0) * size * 0.15 * 3.0 * 3.0,
        Some(TextEffect::Echo(e)) => 3.0 * (e.dx.abs() + e.dy.abs()),
        Some(TextEffect::Glitch(g)) => (g.offset / 100.0).abs() * size,
        Some(TextEffect::Outline(o)) => o.width,
        Some(TextEffect::Hollow(h)) => h.width,
        Some(TextEffect::Background(b)) => b.padding.abs() * 2.0,
        None => 0.0,
    }
}

/// How far a layer's effects move content outward, in local units.
fn layer_reach(effects: &[Effect]) -> f64 {
    effects
        .iter()
        .map(|e| match e {
            Effect::DropShadow(d) => d.dx.abs() + d.dy.abs() + d.blur * 1.5,
            Effect::Blur(b) => b.radius * 1.5,
            Effect::Outline(_) => 0.0,
        })
        .fold(0.0, f64::max)
}

impl Renderer {
    /// The display list's top-level groups with page-space bounds (text is laid out to size it; layouts are
    /// cached, so drawing afterwards does not lay out again).
    pub fn groups(&mut self, dl: &DisplayList) -> Vec<Group> {
        let mut out: Vec<Group> = Vec::new();
        let mut seen: HashMap<String, u32> = HashMap::new();
        let mut i = 0;
        let n = dl.items.len();
        while i < n {
            let start = i;
            let mut depth = 0i32;
            // Matrix stack and, per open layer, (reach in page units, bounds before the layer).
            let mut ms: Vec<M> = vec![[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]];
            let mut layers: Vec<(usize, f64)> = Vec::new();
            let mut acc: Option<Aabb> = None;
            let mut layer_acc: Vec<Option<Aabb>> = Vec::new();
            let mut key: Option<String> = None;
            let mut reach = 0.0f64;
            let mut blends = false;
            loop {
                let it = &dl.items[i];
                let m = *ms.last().unwrap_or(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
                if key.is_none() {
                    key = src_el(it).map(str::to_owned);
                }
                match it {
                    DLItem::Save { matrix } => {
                        ms.push(mul(&m, matrix));
                        depth += 1;
                    }
                    DLItem::Layer { effects, blend, .. } => {
                        blends |= !matches!(blend, oc_schema::primitives::BlendMode::Normal);
                        ms.push(m);
                        let r = layer_reach(effects) * scale_of(&m);
                        reach = reach.max(r);
                        layers.push((ms.len(), r));
                        layer_acc.push(acc.take());
                        depth += 1;
                    }
                    DLItem::Mask { .. } => {
                        // Bounds of a masked group: the mask and the content together (conservative).
                        ms.push(m);
                        layers.push((ms.len(), 0.0));
                        layer_acc.push(acc.take());
                        depth += 1;
                    }
                    DLItem::Restore => {
                        if let Some(&(level, reach)) = layers.last()
                            && level == ms.len()
                        {
                            layers.pop();
                            let inner = acc.take().map(|a| a.pad(reach));
                            let before = layer_acc.pop().flatten();
                            acc = match (before, inner) {
                                (Some(b), Some(x)) => Some(Aabb::add(Some(b), x)),
                                (b, x) => b.or(x),
                            };
                        }
                        if ms.len() > 1 {
                            ms.pop();
                        }
                        depth -= 1;
                    }
                    other => {
                        if let DLItem::Text { body, effect, .. } = other {
                            reach = reach.max(effect_reach(effect.as_ref(), max_size(body)) * scale_of(&m));
                        }
                        if let Some(b) = self.item_bounds(other, &m) {
                            acc = Some(Aabb::add(acc, b));
                        }
                    }
                }
                i += 1;
                if depth <= 0 || i >= n {
                    break;
                }
            }
            // Unbalanced tail: close open layers conservatively.
            while let Some((_, reach)) = layers.pop() {
                let inner = acc.take().map(|a| a.pad(reach));
                let before = layer_acc.pop().flatten();
                acc = match (before, inner) {
                    (Some(b), Some(x)) => Some(Aabb::add(Some(b), x)),
                    (b, x) => b.or(x),
                };
            }
            let k = key.unwrap_or_else(|| format!("#{start}"));
            let nth = seen.entry(k.clone()).or_insert(0);
            let bounds = acc.filter(Aabb::finite).map(|a| a.pad(1.0).rect());
            let hash = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                format!("{:?}", &dl.items[start..i]).hash(&mut h);
                h.finish()
            };
            out.push(Group { key: (k, *nth), range: start..i, bounds, reach, hash, blends });
            *nth += 1;
        }
        out
    }

    fn item_bounds(&mut self, it: &DLItem, m: &M) -> Option<Aabb> {
        let local = match it {
            DLItem::Path { path: d, stroke, .. } => {
                let p = path::parse(d)?.to_skia()?;
                let b = p.bounds();
                let mut a = Aabb { x0: f64::from(b.left()), y0: f64::from(b.top()), x1: f64::from(b.right()), y1: f64::from(b.bottom()) };
                if let Some(s) = stroke {
                    let w = if s.stroke.align == StrokeAlign::Center { s.stroke.width } else { s.stroke.width * 2.0 };
                    // Miter joins reach up to miter-limit (4) × half the width.
                    a = a.pad(w.max(0.0) * 2.0);
                }
                a
            }
            DLItem::Line { points, stroke, .. } => {
                let mut a: Option<Aabb> = None;
                for p in points {
                    a = Some(Aabb::add(a, Aabb { x0: p[0], y0: p[1], x1: p[0], y1: p[1] }));
                }
                a?.pad(stroke.width.max(1.0) * 4.0)
            }
            DLItem::Image { dst, .. } | DLItem::Video { dst, .. } => Aabb::of(dst),
            DLItem::Qr { bbox, .. } | DLItem::Placeholder { bbox, .. } => Aabb::of(bbox),
            DLItem::Text { key, body, bbox, effect, curve, fit, .. } => {
                let size = max_size(body);
                let pad = size * 0.6 + effect_reach(effect.as_ref(), size) + 2.0;
                if curve.is_some_and(|c| c != 0.0 && !c.is_nan()) || fit.is_some() {
                    Aabb::of(bbox).pad(pad)
                } else {
                    let l = self.text.layout(key, body, bbox.w, effect.as_ref());
                    let wide = l.paras.iter().map(|p| p.longest_line.max(p.width)).fold(bbox.w, f64::max);
                    let over_x = (wide - bbox.w).max(0.0);
                    let over_y = (l.height - bbox.h).max(0.0);
                    Aabb { x0: bbox.x - over_x, y0: bbox.y - over_y, x1: bbox.x + bbox.w + over_x, y1: bbox.y + bbox.h.max(l.height) + over_y }.pad(pad)
                }
            }
            DLItem::Clip { .. } | DLItem::Save { .. } | DLItem::Restore | DLItem::Layer { .. } | DLItem::Mask { .. } | DLItem::MaskContent => return None,
        };
        Some(map(m, local))
    }

    /// Draw a display list like [`Renderer::draw`], skipping groups (from [`Renderer::groups`] of the same list)
    /// whose bounds do not meet the canvas clip.
    pub fn draw_culled(&mut self, canvas: &mut Canvas, dl: &DisplayList, groups: &[Group], assets: &dyn AssetProvider, background: bool, items: bool) {
        self.draw_groups(canvas, dl, groups, 0..groups.len(), assets, background, items);
    }

    /// [`Renderer::draw_culled`] for the groups `range` only (the background when `background`).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_groups(&mut self, canvas: &mut Canvas, dl: &DisplayList, groups: &[Group], range: std::ops::Range<usize>, assets: &dyn AssetProvider, background: bool, items: bool) {
        let depth = canvas.depth();
        if background && let Some(r) = rect(0.0, 0.0, dl.width, dl.height) {
            let page_path = tiny_skia::PathBuilder::from_rect(r);
            for bg in &dl.background {
                self.fill_path_with(canvas, &page_path, bg, assets);
            }
        }
        if items {
            let m = canvas.transform();
            let clip = canvas.clip_bounds();
            for g in &groups[range.start.min(groups.len())..range.end.min(groups.len())] {
                let Some(b) = &g.bounds else { continue };
                let d = map(&[f64::from(m.sx), f64::from(m.ky), f64::from(m.kx), f64::from(m.sy), f64::from(m.tx), f64::from(m.ty)], Aabb::of(b));
                if d.x1 < f64::from(clip.left()) || d.y1 < f64::from(clip.top()) || d.x0 > f64::from(clip.right()) || d.y0 > f64::from(clip.bottom()) {
                    continue;
                }
                for it in &dl.items[g.range.clone()] {
                    self.item(canvas, it, assets);
                }
            }
        }
        canvas.restore_to(depth);
    }
}

fn src_el(it: &DLItem) -> Option<&str> {
    match it {
        DLItem::Layer { src_el, .. }
        | DLItem::Path { src_el, .. }
        | DLItem::Line { src_el, .. }
        | DLItem::Image { src_el, .. }
        | DLItem::Text { src_el, .. }
        | DLItem::Qr { src_el, .. }
        | DLItem::Placeholder { src_el, .. }
        | DLItem::Video { src_el, .. }
        | DLItem::Mask { src_el, .. } => Some(src_el.as_str()),
        DLItem::Save { .. } | DLItem::Restore | DLItem::Clip { .. } | DLItem::MaskContent => None,
    }
}

fn union(a: Option<Aabb>, b: Option<&Rect>) -> Option<Aabb> {
    match b {
        Some(r) => Some(Aabb::add(a, Aabb::of(r))),
        None => a,
    }
}

/// The page-space area whose pixels may differ between `old` and `new` (two lists of the same page, with their
/// [`Renderer::groups`]).
pub fn dirty_region(old: &DisplayList, old_groups: &[Group], new: &DisplayList, new_groups: &[Group]) -> Dirty {
    dirty_region_with(old, old_groups, new, new_groups, |_, _| None)
}

/// [`dirty_region`] where `refine(old_group, new_group)` may narrow what a changed group dirties (`Some(None)`:
/// nothing visible changed; `Some(Some(area))`: only `area`; `None`: both groups' bounds).
fn dirty_region_with(old: &DisplayList, old_groups: &[Group], new: &DisplayList, new_groups: &[Group], mut refine: impl FnMut(&Group, &Group) -> Option<Option<Aabb>>) -> Dirty {
    if old.width != new.width || old.height != new.height || old.background != new.background {
        return Dirty::Full;
    }
    let index: HashMap<&(String, u32), usize> = old_groups.iter().enumerate().map(|(i, g)| (&g.key, i)).collect();
    let mut matched = vec![false; old_groups.len()];
    let mut last: Option<usize> = None;
    let mut acc: Option<Aabb> = None;
    for g in new_groups {
        match index.get(&g.key) {
            Some(&oi) => {
                if last.is_some_and(|l| oi < l) {
                    // Stacking order changed: what is above what differs everywhere the groups overlap.
                    return Dirty::Full;
                }
                last = Some(oi);
                matched[oi] = true;
                let o = &old_groups[oi];
                if old.items[o.range.clone()] != new.items[g.range.clone()] {
                    match refine(o, g) {
                        Some(Some(r)) => acc = Some(Aabb::add(acc, r)),
                        Some(None) => {}
                        None => acc = union(union(acc, o.bounds.as_ref()), g.bounds.as_ref()),
                    }
                }
            }
            None => acc = union(acc, g.bounds.as_ref()),
        }
    }
    for (i, o) in old_groups.iter().enumerate() {
        if !matched[i] {
            acc = union(acc, o.bounds.as_ref());
        }
    }
    match acc {
        None => Dirty::None,
        Some(a) => {
            // Clamp to the page (content outside it is clipped by the host anyway).
            let x0 = a.x0.max(0.0);
            let y0 = a.y0.max(0.0);
            let x1 = a.x1.min(new.width);
            let y1 = a.y1.min(new.height);
            if x1 <= x0 || y1 <= y0 { Dirty::None } else { Dirty::Area(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }) }
        }
    }
}

/// How a display list can be painted in three layers while some of its groups change (a drag, typing): the
/// groups `from..to` of `new` hold every change since `old`; the groups before them (and the background) and the
/// groups after them are as they were, so their pixels can be cached and the changing groups drawn between them.
/// Source-over compositing is associative, so `below + changing + above` equals the whole list drawn at once (up
/// to 8-bit rounding of the cached layer).
#[derive(Debug, Clone, PartialEq)]
pub struct Layering {
    pub from: usize,
    pub to: usize,
    /// Hash of the background and the groups before `from`.
    pub below: u64,
    /// Hash of the groups from `to` on.
    pub above: u64,
}

/// The [`Layering`] of `new` against `old` (two lists of a page), `None` when there is no change, the change is
/// not confined to a run of groups in the same stacking order, or a group above it blends non-normally (it must
/// see what is below it).
pub fn layering(old: &DisplayList, old_groups: &[Group], new: &DisplayList, new_groups: &[Group]) -> Option<Layering> {
    if old.width != new.width || old.height != new.height || old.background != new.background {
        return None;
    }
    let index: HashMap<&(String, u32), usize> = old_groups.iter().enumerate().map(|(i, g)| (&g.key, i)).collect();
    let mut changed: Option<(usize, usize)> = None;
    let mut last: Option<usize> = None;
    let push = |i: usize, c: &mut Option<(usize, usize)>| *c = Some(c.map_or((i, i + 1), |(a, b)| (a.min(i), b.max(i + 1))));
    let mut matched = 0;
    for (i, g) in new_groups.iter().enumerate() {
        match index.get(&g.key) {
            Some(&oi) => {
                if last.is_some_and(|l| oi < l) {
                    return None;
                }
                last = Some(oi);
                matched += 1;
                if old_groups[oi].hash != g.hash {
                    push(i, &mut changed);
                }
            }
            None => push(i, &mut changed),
        }
    }
    // Deleted groups: the change sits where they were (between their neighbours).
    if matched < old_groups.len() {
        let kept: std::collections::HashSet<&(String, u32)> = new_groups.iter().map(|g| &g.key).collect();
        for (oi, g) in old_groups.iter().enumerate() {
            if kept.contains(&g.key) {
                continue;
            }
            // First new group whose old index is after the deleted one.
            let at = new_groups.iter().position(|n| index.get(&n.key).is_some_and(|&o| o > oi)).unwrap_or(new_groups.len());
            changed = Some(changed.map_or((at, at), |(a, b)| (a.min(at), b.max(at))));
        }
    }
    let (from, to) = changed?;
    if new_groups[to.min(new_groups.len())..].iter().any(|g| g.blends) {
        return None;
    }
    use std::hash::{Hash, Hasher};
    let mut hb = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", new.background).hash(&mut hb);
    for g in &new_groups[..from] {
        g.hash.hash(&mut hb);
    }
    let mut ha = std::collections::hash_map::DefaultHasher::new();
    for g in &new_groups[to..] {
        g.hash.hash(&mut ha);
    }
    Some(Layering { from, to, below: hb.finish(), above: ha.finish() })
}

/// Whether groups `b` of `new` are groups `a` of `old` moved by a common translation (a drag): the same ops,
/// except that each group's outermost `save` matrix has the same linear part and a translation offset by the
/// same `(dx, dy)`. Returns that offset (page units).
pub fn translation(old: &DisplayList, old_groups: &[Group], a: std::ops::Range<usize>, new: &DisplayList, new_groups: &[Group], b: std::ops::Range<usize>) -> Option<(f64, f64)> {
    let (ga, gb) = (old_groups.get(a)?, new_groups.get(b)?);
    if ga.len() != gb.len() || ga.is_empty() {
        return None;
    }
    let mut d: Option<(f64, f64)> = None;
    for (x, y) in ga.iter().zip(gb) {
        let (ix, iy) = (&old.items[x.range.clone()], &new.items[y.range.clone()]);
        if ix.len() != iy.len() || x.key != y.key {
            return None;
        }
        match (ix.first(), iy.first()) {
            (Some(DLItem::Save { matrix: m }), Some(DLItem::Save { matrix: n })) => {
                if m[..4] != n[..4] {
                    return None;
                }
                let o = (n[4] - m[4], n[5] - m[5]);
                match d {
                    None => d = Some(o),
                    Some(p) if (p.0 - o.0).abs() < 1e-9 && (p.1 - o.1).abs() < 1e-9 => {}
                    Some(_) => return None,
                }
                if ix[1..] != iy[1..] {
                    return None;
                }
            }
            _ => {
                if ix != iy {
                    return None;
                }
                if d.is_some_and(|p| p != (0.0, 0.0)) {
                    return None;
                }
                d.get_or_insert((0.0, 0.0));
            }
        }
    }
    d
}

/// Union of the bounds of `groups[range]` (page units), `None` when nothing in them draws.
pub fn groups_bounds(groups: &[Group], range: std::ops::Range<usize>) -> Option<Rect> {
    let mut acc: Option<Aabb> = None;
    for g in groups.get(range)? {
        acc = union(acc, g.bounds.as_ref());
    }
    acc.map(Aabb::rect)
}

impl Renderer {
    /// [`dirty_region`], narrowing edited text to the lines that changed: when the only difference in a group is
    /// a text op's body (same box, effect and placement), the old and new layouts are compared line by line and only
    /// the changed lines (from the first changed glyph run on the first of them) are dirty — typing repaints a line,
    /// not the whole text box.
    pub fn dirty_region_fine(&mut self, old: &DisplayList, old_groups: &[Group], new: &DisplayList, new_groups: &[Group]) -> Dirty {
        dirty_region_with(old, old_groups, new, new_groups, |o, g| self.text_change(&old.items[o.range.clone()], &new.items[g.range.clone()]))
    }

    fn text_change(&mut self, a: &[DLItem], b: &[DLItem]) -> Option<Option<Aabb>> {
        if a.len() != b.len() {
            return None;
        }
        // Exactly one differing op, a text op whose body alone changed.
        let mut at = None;
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            if x != y {
                if at.is_some() {
                    return None;
                }
                at = Some(i);
            }
        }
        let i = at?;
        let (DLItem::Text { key: k0, body: b0, bbox: x0, effect: e0, valign: v0, curve: c0, fit: f0, .. }, DLItem::Text { key: k1, body: b1, bbox: x1, effect: e1, valign: v1, curve: c1, fit: f1, .. }) = (&a[i], &b[i]) else { return None };
        if k0 != k1 || x0 != x1 || e0 != e1 || v0 != v1 || c0 != c1 || f0 != f1 || c1.is_some() || f1.is_some() {
            return None;
        }
        // The text op's matrix: the saves enclosing it.
        let mut ms: Vec<M> = vec![[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]];
        for it in &b[..i] {
            match it {
                DLItem::Save { matrix } => {
                    let m = mul(ms.last()?, matrix);
                    ms.push(m);
                }
                DLItem::Layer { .. } => {
                    let m = *ms.last()?;
                    ms.push(m);
                }
                DLItem::Restore => {
                    ms.pop();
                }
                _ => {}
            }
        }
        let m = *ms.last()?;
        let new_l = self.text.layout(k1, b1, x1.w, e1.as_ref());
        let old_l = self.text.layout_uncached(b0, x0.w, e0.as_ref());
        // Vertical alignment moves every line when the height changes.
        if old_l.height != new_l.height && !matches!(v1, None | Some(oc_schema::elements::CellVAlign::Top)) {
            return None;
        }
        let dy = match v1 {
            Some(oc_schema::elements::CellVAlign::Middle) => (x1.h - new_l.height) / 2.0,
            Some(oc_schema::elements::CellVAlign::Bottom) => x1.h - new_l.height,
            _ => 0.0,
        };
        let size = max_size(b0).max(max_size(b1));
        let pad = size * 0.6 + effect_reach(e1.as_ref(), size) + 2.0;
        let right = new_l.paras.iter().chain(&old_l.paras).map(|p| p.x + p.longest_line.max(p.width)).fold(x1.w.max(new_l.width).max(old_l.width), f64::max);
        // Flattened lines: (paragraph, line).
        type Line<'l> = (&'l oc_text::LaidParagraph, &'l oc_text::LaidLine);
        let (lo, ln): (Vec<Line<'_>>, Vec<Line<'_>>) = (flat_lines(&old_l), flat_lines(&new_l));
        let same_run = |p: &oc_text::LaidParagraph, r: &oc_text::GlyphRun, q: &oc_text::LaidParagraph, s: &oc_text::GlyphRun| r == s && p.styles.get(r.style) == q.styles.get(s.style);
        let mut acc: Option<Aabb> = None;
        let mut box_of = |p: &oc_text::LaidParagraph, l: &oc_text::LaidLine, from_x: Option<f64>| {
            let y0 = p.y + l.top;
            let x0 = from_x.map_or((p.x + l.left).min(0.0) - pad, |x| p.x + x - pad);
            acc = Some(Aabb::add(acc, Aabb { x0, y0: y0 - pad, x1: right + pad, y1: y0 + l.height + pad }));
        };
        for k in 0..lo.len().max(ln.len()) {
            match (lo.get(k), ln.get(k)) {
                (Some(&(p, l)), Some(&(q, n))) => {
                    let same_box = p.x == q.x && p.y == q.y && l.top == n.top && l.height == n.height && l.left == n.left && p.bullet == q.bullet;
                    if same_box && l.glyph_runs.len() == n.glyph_runs.len() && l.glyph_runs.iter().zip(&n.glyph_runs).all(|(r, s)| same_run(p, r, q, s)) {
                        continue;
                    }
                    // From the first glyph that differs (glyphs before it, in identical runs, are drawn identically).
                    let fx = if same_box { first_change_x(p, l, q, n) } else { None };
                    box_of(p, l, fx);
                    box_of(q, n, fx);
                }
                (Some(&(p, l)), None) => box_of(p, l, None),
                (None, Some(&(q, n))) => box_of(q, n, None),
                (None, None) => {}
            }
        }
        let Some(r) = acc else { return Some(None) };
        let r = Aabb { x0: r.x0 + x1.x, y0: r.y0 + x1.y + dy, x1: r.x1 + x1.x, y1: r.y1 + x1.y + dy };
        Some(Some(map(&m, r)))
    }
}

/// Every line of a layout with its paragraph, in order.
fn flat_lines(l: &oc_text::TextLayout) -> Vec<(&oc_text::LaidParagraph, &oc_text::LaidLine)> {
    l.paras.iter().flat_map(|p| p.lines.iter().map(move |ln| (p, ln))).collect()
}

/// Paragraph-space x of the first glyph that differs between two lines in the same place, `None` when the change
/// starts at the line's beginning (or cannot be placed). Glyph x is paragraph-relative (alignment included).
fn first_change_x(p: &oc_text::LaidParagraph, l: &oc_text::LaidLine, q: &oc_text::LaidParagraph, n: &oc_text::LaidLine) -> Option<f64> {
    let mut last_end: Option<f64> = None;
    for (r, s) in l.glyph_runs.iter().zip(&n.glyph_runs) {
        let same_face = r.face == s.face && r.size == s.size && r.synthetic_italic == s.synthetic_italic && r.rtl == s.rtl && p.styles.get(r.style) == q.styles.get(s.style);
        if !same_face || r.rtl {
            return last_end;
        }
        for (g, h) in r.glyphs.iter().zip(&s.glyphs) {
            if g != h {
                return Some(g.x.min(h.x));
            }
            last_end = Some(g.x);
        }
        if r.glyphs.len() != s.glyphs.len() {
            let k = r.glyphs.len().min(s.glyphs.len());
            return r.glyphs.get(k).map(|g| g.x).into_iter().chain(s.glyphs.get(k).map(|g| g.x)).reduce(f64::min).or(last_end);
        }
    }
    last_end
}
