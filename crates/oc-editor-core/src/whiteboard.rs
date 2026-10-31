// SPDX-License-Identifier: Apache-2.0
//! Whiteboard facilitation (WB-020..): pure helpers for sticky notes, sections, mind maps, the shared timer
//! and dot voting. Port of `whiteboard.ts`. Clock values are passed in (`now`).

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use indexmap::{IndexMap, IndexSet};
use oc_crdt::board::{TimerState, VoteEntry};
use oc_geometry::{selection_bounds, world_bounds};
use oc_schema::elements::Element;
use oc_schema::primitives::Rect;
use oc_schema::tree::{Elements, children_of};
use oc_schema::Extra;
use serde::{Deserialize, Serialize};

use crate::util::max;

/// The eight sticky-note colours (Canva-style pastel set).
pub const STICKY_COLORS: [&str; 8] = ["#fef08a", "#fed7aa", "#fecaca", "#fbcfe8", "#ddd6fe", "#bfdbfe", "#a5f3fc", "#bbf7d0"];

fn live(el: Option<&Element>) -> Option<&Element> {
    el.filter(|e| !e.is_deleted())
}

fn board(el: &Element) -> Option<&oc_schema::elements::BoardMeta> {
    el.base().and_then(|b| b.board.as_ref())
}

fn xf(el: &Element) -> oc_schema::Xf {
    el.xf().cloned().unwrap_or_default()
}

fn id(el: &Element) -> &str {
    el.id().unwrap_or("")
}

/// `String.prototype.localeCompare` for ids and colour strings (ICU root collation on ASCII: digits before
/// letters, letters case-insensitive first, then lowercase before uppercase).
pub fn locale_cmp(a: &str, b: &str) -> Ordering {
    fn primary(c: char) -> (u8, char) {
        if c.is_ascii_digit() {
            (1, c)
        } else if c.is_ascii_alphabetic() {
            (2, c.to_ascii_lowercase())
        } else {
            (0, c)
        }
    }
    let pa: Vec<(u8, char)> = a.chars().map(primary).collect();
    let pb: Vec<(u8, char)> = b.chars().map(primary).collect();
    pa.cmp(&pb).then_with(|| {
        // Tertiary: lowercase sorts before uppercase.
        let ta: Vec<bool> = a.chars().map(|c| c.is_ascii_uppercase()).collect();
        let tb: Vec<bool> = b.chars().map(|c| c.is_ascii_uppercase()).collect();
        ta.cmp(&tb)
    })
}

fn cmp_f(a: f64, b: f64) -> Ordering {
    crate::util::num_cmp(a, b)
}

// ------------------------------------------------------------------ stickies

pub fn is_sticky(el: Option<&Element>) -> bool {
    el.and_then(board).is_some_and(|b| b.sticky.is_some())
}

/// Group selected stickies by colour into tidy grids, placed left-to-right from the selection's top-left.
/// TS default `gap = 24`.
pub fn cluster_layout(elements: &Elements, ids: &[String], gap: f64) -> IndexMap<String, (f64, f64)> {
    let stickies: Vec<&Element> =
        ids.iter().filter_map(|i| live(elements.get(i))).filter(|e| is_sticky(Some(e))).collect();
    let mut out = IndexMap::new();
    if stickies.is_empty() {
        return out;
    }
    let sids: Vec<&str> = stickies.iter().map(|s| id(s)).collect();
    let Some(b) = selection_bounds(elements, &sids) else { return out };
    let order = |c: &str| STICKY_COLORS.iter().position(|x| *x == c.to_lowercase()).unwrap_or(STICKY_COLORS.len());
    let mut by_color: IndexMap<String, Vec<&Element>> = IndexMap::new();
    for s in &stickies {
        let c = board(s).and_then(|b| b.sticky.as_ref()).map(|st| st.color.to_lowercase()).unwrap_or_default();
        by_color.entry(c).or_default().push(s);
    }
    let mut groups: Vec<(String, Vec<&Element>)> = by_color.into_iter().collect();
    groups.sort_by(|a, b2| order(&a.0).cmp(&order(&b2.0)).then_with(|| locale_cmp(&a.0, &b2.0)));
    let mut cx = b.x;
    for (_, mut els) in groups {
        // Keep reading order (top-to-bottom, left-to-right) inside each colour.
        els.sort_by(|p, q| cmp_f(xf(p).y - xf(q).y, 0.0).then_with(|| cmp_f(xf(p).x - xf(q).x, 0.0)));
        let cw = els.iter().map(|e| xf(e).w).fold(f64::NEG_INFINITY, max);
        let ch = els.iter().map(|e| xf(e).h).fold(f64::NEG_INFINITY, max);
        let cols = (els.len() as f64).sqrt().ceil() as usize;
        for (i, e) in els.iter().enumerate() {
            out.insert(id(e).to_string(), (cx + (i % cols) as f64 * (cw + gap), b.y + (i / cols) as f64 * (ch + gap)));
        }
        cx += cols as f64 * (cw + gap) + gap * 2.0;
    }
    out
}

// ------------------------------------------------------------------ sections

pub fn is_section(el: Option<&Element>) -> bool {
    el.and_then(board).is_some_and(|b| b.section.is_some())
}

fn inside(r: &Rect, outer: &Rect) -> bool {
    r.x >= outer.x - 0.5 && r.y >= outer.y - 0.5 && r.x + r.w <= outer.x + outer.w + 0.5 && r.y + r.h <= outer.y + outer.h + 0.5
}

/// Elements that belong to a section: top-level siblings whose bounds lie fully inside it (sections nested
/// inside count too, with their members). A collapsed section's members are the ones it hid.
pub fn section_members(elements: &Elements, section_id: &str) -> Vec<String> {
    let Some(s) = live(elements.get(section_id)) else { return vec![] };
    let Some(sec) = board(s).and_then(|b| b.section.as_ref()) else { return vec![] };
    if let Some(c) = &sec.collapsed {
        return c.hidden.iter().filter(|i| live(elements.get(*i)).is_some()).cloned().collect();
    }
    let bx = world_bounds(elements, id(s)).unwrap_or_default();
    let sxf = xf(s);
    children_of(elements, s.parent_id().unwrap_or(""))
        .into_iter()
        .filter(|e| {
            id(e) != id(s)
                && e.base().is_none_or(|b| b.visible != Some(false))
                && inside(&world_bounds(elements, id(e)).unwrap_or_default(), &bx)
        })
        // A larger section is never a member of a smaller one.
        .filter(|e| !(is_section(Some(e)) && xf(e).w * xf(e).h >= sxf.w * sxf.h))
        .map(|e| id(e).to_string())
        .collect()
}

fn section_order(e: &Element) -> f64 {
    board(e).and_then(|b| b.section.as_ref()).map_or(0.0, |s| s.order)
}

/// Sections of a page in navigator order.
pub fn sections_of<'a>(elements: &'a Elements, page_id: &str) -> Vec<&'a Element> {
    let mut v: Vec<&Element> = children_of(elements, page_id).into_iter().filter(|e| is_section(Some(e))).collect();
    v.sort_by(|a, b| cmp_f(section_order(a) - section_order(b), 0.0).then_with(|| locale_cmp(id(a), id(b))));
    v
}

/// Expand a set of moving ids with the members of any section among them (transitively).
pub fn with_section_members(elements: &Elements, ids: &[String]) -> Vec<String> {
    let mut out: IndexSet<String> = ids.iter().cloned().collect();
    let mut queue: Vec<String> = ids.to_vec();
    while let Some(i) = queue.pop() {
        if !is_section(elements.get(&i)) {
            continue;
        }
        for m in section_members(elements, &i) {
            if !out.contains(&m) {
                out.insert(m.clone());
                queue.push(m);
            }
        }
    }
    out.into_iter().collect()
}

// ------------------------------------------------------------------ mind maps

pub fn is_mind_node(el: Option<&Element>) -> bool {
    el.and_then(board).is_some_and(|b| b.mind.is_some())
}

/// The element a mind-map key acts on: the nearest top-level ancestor (stickies and labelled shapes are groups).
pub fn mind_node_of<'a>(elements: &'a Elements, el_id: &str) -> Option<&'a Element> {
    let mut cur = elements.get(el_id);
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(c) = live(cur) {
        if !seen.insert(id(c).to_string()) {
            break;
        }
        if board(c).is_some_and(|b| b.mind.is_some() || b.sticky.is_some()) {
            return Some(c);
        }
        match c.parent_id().and_then(|p| elements.get(p)) {
            None => return if matches!(c, Element::Line(_)) || is_section(Some(c)) { None } else { Some(c) },
            Some(p) => cur = Some(p),
        }
    }
    None
}

fn mind_parent(e: &Element) -> Option<&str> {
    board(e).and_then(|b| b.mind.as_ref()).and_then(|m| m.parent.as_ref()).and_then(|p| p.as_deref())
}

pub fn mind_children<'a>(elements: &'a Elements, parent: &str) -> Vec<&'a Element> {
    let mut v: Vec<&Element> = elements.values().filter(|e| !e.is_deleted() && mind_parent(e) == Some(parent)).collect();
    v.sort_by(|a, b| cmp_f(xf(a).y - xf(b).y, 0.0).then_with(|| locale_cmp(id(a), id(b))));
    v
}

pub fn mind_root(elements: &Elements, start: &str) -> String {
    let mut cur = start.to_string();
    let mut seen: HashSet<String> = HashSet::from([cur.clone()]);
    loop {
        let p = elements.get(&cur).and_then(mind_parent);
        match p {
            Some(p) if live(elements.get(p)).is_some() && !seen.contains(p) => {
                seen.insert(p.to_string());
                cur = p.to_string();
            }
            _ => return cur,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MindSpacing {
    pub gap_x: f64,
    pub gap_y: f64,
}

fn spacing_for(el: &Element) -> MindSpacing {
    let x = xf(el);
    MindSpacing { gap_x: max(60.0, x.w * 0.5), gap_y: max(20.0, x.h * 0.25) }
}

/// Where a new child of `parent_id` goes: to the right, centred on the parent or below the last child.
pub fn mind_child_position(elements: &Elements, parent_id: &str, size_h: f64) -> (f64, f64) {
    let Some(p) = elements.get(parent_id) else { return (0.0, 0.0) };
    let px = xf(p);
    let MindSpacing { gap_x, gap_y } = spacing_for(p);
    let kids = mind_children(elements, parent_id);
    let x = px.x + px.w + gap_x;
    let Some(last) = kids.last() else { return (x, px.y + px.h / 2.0 - size_h / 2.0) };
    let bottom = kids.iter().map(|k| xf(k).y + xf(k).h).fold(f64::NEG_INFINITY, max);
    (xf(last).x, bottom + gap_y)
}

/// Balanced tree layout to the right of the root: every subtree gets a band as tall as its contents and the
/// parent is centred on its children's band. The root stays where it is. Returns new top-left positions.
pub fn mind_layout(elements: &Elements, root_id: &str, gap_x: Option<f64>, gap_y: Option<f64>) -> IndexMap<String, (f64, f64)> {
    let mut out = IndexMap::new();
    let Some(root) = live(elements.get(root_id)) else { return out };
    let sp = spacing_for(root);
    let gap_x = gap_x.unwrap_or(sp.gap_x);
    let gap_y = gap_y.unwrap_or(sp.gap_y);
    let mut height: HashMap<String, f64> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    fn measure(elements: &Elements, id_: &str, gap_y: f64, seen: &mut HashSet<String>, height: &mut HashMap<String, f64>) -> f64 {
        seen.insert(id_.to_string());
        let h0 = elements.get(id_).map_or(0.0, |e| xf(e).h);
        let kids: Vec<String> =
            mind_children(elements, id_).into_iter().filter(|k| !seen.contains(id(k))).map(|k| id(k).to_string()).collect();
        let mut band = 0.0;
        for k in &kids {
            band += measure(elements, k, gap_y, seen, height);
        }
        band += gap_y * (kids.len().saturating_sub(1)) as f64;
        let h = max(h0, band);
        height.insert(id_.to_string(), h);
        h
    }
    measure(elements, root_id, gap_y, &mut seen, &mut height);
    struct Ctx<'a> {
        elements: &'a Elements,
        root_id: &'a str,
        root_x: f64,
        gap_x: f64,
        gap_y: f64,
        height: HashMap<String, f64>,
        placed: HashSet<String>,
        out: IndexMap<String, (f64, f64)>,
    }
    fn place(c: &mut Ctx<'_>, id_: &str, x: f64, center_y: f64) {
        c.placed.insert(id_.to_string());
        let ex = c.elements.get(id_).map(xf).unwrap_or_default();
        if id_ != c.root_id {
            c.out.insert(id_.to_string(), (x, center_y - ex.h / 2.0));
        }
        let kids: Vec<String> = mind_children(c.elements, id_)
            .into_iter()
            .filter(|k| c.height.contains_key(id(k)) && !c.placed.contains(id(k)))
            .map(|k| id(k).to_string())
            .collect();
        if kids.is_empty() {
            return;
        }
        let band: f64 = kids.iter().map(|k| c.height[k]).sum::<f64>() + c.gap_y * (kids.len() - 1) as f64;
        let left = (if id_ == c.root_id { c.root_x } else { x }) + ex.w + c.gap_x;
        let mut y = center_y - band / 2.0;
        for k in kids {
            let h = c.height[&k];
            place(c, &k, left, y + h / 2.0);
            y += h + c.gap_y;
        }
    }
    let rx = xf(root);
    let mut ctx = Ctx { elements, root_id, root_x: rx.x, gap_x, gap_y, height, placed: HashSet::new(), out: IndexMap::new() };
    place(&mut ctx, root_id, rx.x, rx.y + rx.h / 2.0);
    out = ctx.out;
    out
}

// ------------------------------------------------------------------ timer

/// TS default `sound = true`.
pub fn make_timer(duration_ms: f64, sound: bool) -> TimerState {
    TimerState { duration_ms, ends_at: None, remaining_ms: duration_ms, sound, by: None, extra: Extra::new() }
}

pub fn timer_remaining(t: &TimerState, now: f64) -> f64 {
    match t.ends_at {
        None => max(0.0, t.remaining_ms),
        Some(e) => max(0.0, e - now),
    }
}

pub fn timer_running(t: &TimerState, now: f64) -> bool {
    t.ends_at.is_some_and(|e| e > now)
}

pub fn timer_start(t: &TimerState, now: f64) -> TimerState {
    let r = timer_remaining(t, now);
    let left = if r != 0.0 { r } else { t.duration_ms };
    TimerState { ends_at: Some(now + left), remaining_ms: left, ..t.clone() }
}

pub fn timer_pause(t: &TimerState, now: f64) -> TimerState {
    TimerState { ends_at: None, remaining_ms: timer_remaining(t, now), ..t.clone() }
}

pub fn timer_reset(t: &TimerState) -> TimerState {
    TimerState { ends_at: None, remaining_ms: t.duration_ms, ..t.clone() }
}

pub fn timer_add(t: &TimerState, ms: f64, now: f64) -> TimerState {
    if let Some(e) = t.ends_at
        && e > now
    {
        return TimerState { ends_at: Some(e + ms), ..t.clone() };
    }
    let r = timer_remaining(t, now) + ms;
    TimerState { remaining_ms: r, ends_at: None, duration_ms: max(t.duration_ms, r), ..t.clone() }
}

pub fn format_clock(ms: f64) -> String {
    let s = (ms / 1000.0).ceil();
    let m = (s / 60.0).floor();
    // JS number → string (`-0` prints as "0"), then `padStart(2, '0')`.
    let sec = oc_schema::js::num_to_string(s % 60.0);
    let pad = if sec.encode_utf16().count() < 2 { format!("0{sec}") } else { sec };
    format!("{}:{pad}", oc_schema::js::num_to_string(m))
}

// ------------------------------------------------------------------ voting

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Voter {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub n: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoteTally {
    pub element: String,
    pub count: f64,
    pub voters: Vec<Voter>,
}

/// Dots per element for one session (only elements in scope), most votes first; ties keep scope order.
pub fn tally_votes(session_id: &str, scope: &[String], entries: &[VoteEntry]) -> Vec<VoteTally> {
    let mut pos: HashMap<&str, usize> = HashMap::new();
    for (i, s) in scope.iter().enumerate() {
        pos.entry(s.as_str()).or_insert(i);
    }
    let mut by: IndexMap<String, VoteTally> = IndexMap::new();
    for s in scope {
        by.insert(s.clone(), VoteTally { element: s.clone(), count: 0.0, voters: vec![] });
    }
    for e in entries {
        if e.session != session_id || !pos.contains_key(e.element.as_str()) || e.n <= 0.0 {
            continue;
        }
        if let Some(t) = by.get_mut(&e.element) {
            t.count += e.n;
            t.voters.push(Voter { id: e.voter.clone(), name: e.name.clone().filter(|n| !n.is_empty()), n: e.n });
        }
    }
    let mut v: Vec<VoteTally> = by.into_values().collect();
    v.sort_by(|a, b| cmp_f(b.count - a.count, 0.0).then_with(|| pos[a.element.as_str()].cmp(&pos[b.element.as_str()])));
    v
}

/// Elements with the top `n` vote counts (ties at the cut-off are included; zero-vote elements never are).
/// TS default `n = 3`.
pub fn top_voted(tally: &[VoteTally], n: usize) -> Vec<String> {
    let ranked: Vec<&VoteTally> = tally.iter().filter(|t| t.count > 0.0).collect();
    if ranked.len() <= n {
        return ranked.iter().map(|t| t.element.clone()).collect();
    }
    let Some(cut) = n.checked_sub(1).map(|i| ranked[i].count) else { return vec![] };
    ranked.iter().filter(|t| t.count >= cut).map(|t| t.element.clone()).collect()
}

pub fn votes_used(session_id: &str, entries: &[VoteEntry], voter: &str) -> f64 {
    entries.iter().filter(|e| e.session == session_id && e.voter == voter).map(|e| e.n).sum()
}

/// Dots the voter may still place.
pub fn vote_allowance(session_id: &str, per_person: f64, status: &str, entries: &[VoteEntry], voter: &str) -> f64 {
    if status != "open" {
        return 0.0;
    }
    max(0.0, per_person - votes_used(session_id, entries, voter))
}

/// Default vote scope: the selection if any, else every sticky and shape (top-level, visible) on the page.
pub fn vote_scope(elements: &Elements, page_id: &str, selection: &[String]) -> Vec<String> {
    if !selection.is_empty() {
        return selection
            .iter()
            .filter(|i| live(elements.get(*i)).is_some() && !is_section(elements.get(*i)))
            .cloned()
            .collect();
    }
    children_of(elements, page_id)
        .into_iter()
        .filter(|e| {
            e.base().is_none_or(|b| b.visible != Some(false))
                && !is_section(Some(e))
                && (is_sticky(Some(e))
                    || matches!(e, Element::Shape(_))
                    || (matches!(e, Element::Group(_)) && children_of(elements, id(e)).iter().any(|c| matches!(c, Element::Shape(_)))))
        })
        .map(|e| id(e).to_string())
        .collect()
}
