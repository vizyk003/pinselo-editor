// SPDX-License-Identifier: Apache-2.0
//! Animation preview, playback & timeline (MV), soundtrack, captions and keyframes (MV-100).

use oc_crdt::{DocOp, PageOp};
use oc_schema::elements::Element;
use oc_schema::factory::{base_init, make_video};
use oc_schema::keyframes::keyframe_neutral;
use oc_schema::primitives::{Vec2, Xf};
use oc_schema::timeline::{locate_time, page_duration_ms, timeline_of, video_source_time};
use oc_schema::tree::children_of;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{Editor, el_id, one, op_add, op_meta, op_page_update, op_update};
use crate::state::*;
use crate::util::{f, jm, max, min, num, round, s};

/// `withKeyframe` over JSON keyframes: a key set at `t` (replacing one within 20 ms); a first key after 0 also
/// pins the start at neutral.
pub fn with_keyframe(k: &Map<String, Value>, prop: &str, t: f64, v: f64, ease: Option<&str>) -> Map<String, Value> {
    let mut keys: Vec<Value> = k
        .get(prop)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|x| (f(x.get("t")).unwrap_or(f64::NAN) - t).abs() > 20.0).cloned().collect())
        .unwrap_or_default();
    if keys.is_empty() && t > 0.0 {
        let mut z = one("t", num(0.0));
        z.insert("v".into(), num(keyframe_neutral(prop).unwrap_or(0.0)));
        keys.push(Value::Object(z));
    }
    let mut key = one("t", num(t));
    key.insert("v".into(), num(round(v * 1000.0) / 1000.0));
    if let Some(e) = ease {
        key.insert("ease".into(), Value::String(e.into()));
    }
    keys.push(Value::Object(key));
    keys.sort_by(|a, b| crate::util::num_cmp(f(a.get("t")).unwrap_or(f64::NAN), f(b.get("t")).unwrap_or(f64::NAN)));
    let mut out = k.clone();
    out.insert(prop.to_string(), Value::Array(keys));
    out
}

/// Typed keys of one property (for `keyframe_value`).
pub(crate) fn keys_of(k: &Map<String, Value>, prop: &str) -> Option<Vec<oc_schema::elements::Keyframe>> {
    k.get(prop).and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// A video/audio asset reference for insert commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAsset {
    pub id: String,
    #[serde(default)]
    pub w: f64,
    #[serde(default)]
    pub h: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Editor {
    // ------------------------------------------------------------------ animation preview

    pub fn preview_animation(&mut self, page_id: Option<&str>) {
        let pid = page_id.map(str::to_string).unwrap_or_else(|| self.state.active_page_id.clone());
        let now = self.clock.now();
        self.set(|s| s.preview = Some(Preview { page_id: pid, started_at: now }));
    }

    pub fn stop_preview(&mut self) {
        if self.state.preview.is_some() {
            self.set(|s| s.preview = None);
        }
    }

    // ------------------------------------------------------------------ playback

    fn total_ms(&self) -> f64 {
        timeline_of(&self.doc().doc, false).total_ms
    }

    /// Current design time, advancing with the clock while playing (`now` defaults to the clock).
    pub fn current_time(&self, now: Option<f64>) -> f64 {
        let s = &self.state;
        if s.playing
            && let Some(a) = s.play_anchor
        {
            let now = now.unwrap_or_else(|| self.clock.now());
            return min(self.total_ms(), a.time + (now - a.at));
        }
        s.playhead.unwrap_or(0.0)
    }

    pub fn play(&mut self) {
        let total = self.total_ms();
        let mut t = self.state.playhead.unwrap_or_else(|| self.page_start(&self.state.active_page_id.clone()));
        if t >= total - 50.0 {
            t = 0.0;
        }
        let now = self.clock.now();
        self.set(|s| {
            s.playing = true;
            s.playhead = Some(t);
            s.play_anchor = Some(PlayAnchor { at: now, time: t });
            s.editing_text = None;
            s.cropping_id = None;
        });
    }

    pub fn pause(&mut self) {
        let t = self.current_time(None);
        self.set(|s| {
            s.playing = false;
            s.playhead = Some(t);
            s.play_anchor = None;
        });
    }

    pub fn toggle_play(&mut self) {
        if self.state.playing { self.pause() } else { self.play() }
    }

    /// Move the playhead (design time). `None` leaves timeline mode.
    pub fn seek(&mut self, t: Option<f64>) {
        let Some(t) = t else {
            self.set(|s| {
                s.playhead = None;
                s.playing = false;
                s.play_anchor = None;
            });
            return;
        };
        let doc = self.doc();
        let total = timeline_of(&doc.doc, false).total_ms;
        let time = max(0.0, min(total, t));
        let page = locate_time(&doc.doc, time).map(|l| l.span.page.id.clone());
        let now = self.clock.now();
        self.set(|s| {
            s.playhead = Some(time);
            s.play_anchor = if s.playing { Some(PlayAnchor { at: now, time }) } else { None };
            if let Some(p) = page
                && p != s.active_page_id
            {
                s.active_page_id = p;
            }
        });
    }

    /// Called every animation frame by the host while playing: stops at the end and follows pages.
    pub fn tick(&mut self) {
        if !self.state.playing {
            return;
        }
        let t = self.current_time(None);
        let doc = self.doc();
        let total = timeline_of(&doc.doc, false).total_ms;
        if t >= total {
            self.set(|s| {
                s.playing = false;
                s.playhead = Some(total);
                s.play_anchor = None;
            });
            return;
        }
        if let Some(l) = locate_time(&doc.doc, t)
            && l.span.page.id != self.state.active_page_id
        {
            let pid = l.span.page.id.clone();
            self.state.active_page_id = pid.clone();
            self.state.selection = vec![];
            self.scroll_to_page(&pid);
        }
    }

    pub fn page_start(&self, page_id: &str) -> f64 {
        let doc = self.doc();
        timeline_of(&doc.doc, false).spans.iter().find(|s| s.page.id == page_id).map_or(0.0, |s| s.start_ms)
    }

    pub fn set_page_duration(&mut self, page_id: &str, ms: f64) {
        let d = round(max(500.0, min(10.0 * 60_000.0, ms)));
        self.apply(vec![op_page_update(page_id, one("durationMs", num(d)))], true);
    }

    /// `timing` as JSON (`{ startMs, endMs? }`) or `None` to clear.
    pub fn set_timing(&mut self, id: &str, timing: Option<Value>) {
        self.apply(vec![op_update(id, one("timing", timing.unwrap_or(Value::Null)))], true);
    }

    /// Insert a video sized to fit ~70% of the page (animated GIFs/stickers: 40%, silent, looping).
    pub fn insert_video(&mut self, asset: &MediaAsset, at: Option<Vec2>) -> Vec<String> {
        let doc = self.doc();
        let (ww, hh) = (doc.doc.meta.dims.w, doc.doc.meta.dims.h);
        let animated = asset.animated == Some(true);
        let k = if animated { 0.4 } else { 0.7 };
        let sc = min((ww * k) / asset.w, (hh * k) / asset.h);
        let mut base = base_init(Xf::new(0.0, 0.0, asset.w * sc, asset.h * sc, 0.0));
        base.id = self.fresh_id();
        let mut el = make_video(base, &asset.id, asset.duration_ms.unwrap_or(5000.0));
        if animated {
            el.clip.loop_ = true;
            el.clip.muted = true;
        }
        self.insert(vec![jm(&Element::Video(el))], at, true, false)
    }

    /// Split: the selected video at the playhead, or else the whole page into two pages.
    pub fn split_at_playhead(&mut self) {
        let t = self.state.playhead.unwrap_or_else(|| self.current_time(None));
        let doc = self.doc();
        let Some(loc) = locate_time(&doc.doc, t) else { return };
        let page_ms = page_duration_ms(doc.els(), loc.span.page);
        let page_time = loc.page_time_ms;
        let page_id = loc.span.page.id.clone();
        let sel = self.selected().into_iter().find_map(|e| match e {
            Element::Video(v) => Some(v),
            _ => None,
        });
        if let Some(sel) = sel {
            let Some(src) = video_source_time(&sel, page_time, page_ms) else { return };
            if src <= sel.clip.trim_start_ms + 50.0 || src >= sel.clip.trim_end_ms - 50.0 {
                return;
            }
            let start = sel.base.timing.as_ref().map_or(0.0, |t| t.start_ms);
            let mut second = jm(&Element::Video(sel.clone()));
            let new_id = self.fresh_id();
            second.insert("id".into(), Value::String(new_id.clone()));
            let mut clip2 = jm(&sel.clip);
            clip2.insert("trimStartMs".into(), num(src));
            second.insert("clip".into(), Value::Object(clip2));
            let mut timing2 = one("startMs", num(page_time));
            if let Some(e) = sel.base.timing.as_ref().and_then(|t| t.end_ms) {
                timing2.insert("endMs".into(), num(e));
            }
            second.insert("timing".into(), Value::Object(timing2));
            let mut clip1 = jm(&sel.clip);
            clip1.insert("trimEndMs".into(), num(src));
            clip1.insert("loop".into(), Value::Bool(false));
            let mut timing1 = one("startMs", num(start));
            timing1.insert("endMs".into(), num(page_time));
            let mut patch = one("clip", Value::Object(clip1));
            patch.insert("timing".into(), Value::Object(timing1));
            let parent = sel.base.parent_id.clone();
            self.apply(vec![op_update(&sel.base.id, patch), op_add(second, &parent, Some(&sel.base.id))], false);
            self.select(&[new_id], false);
            return;
        }
        // Split the page: same content, first part keeps [0, t), second gets [t, end) with videos advanced.
        let cut = round(page_time);
        if cut < 200.0 || cut > page_ms - 200.0 {
            return;
        }
        let r = self.apply(vec![DocOp::Page(PageOp::Duplicate { page_id: page_id.clone(), new_page_id: None })], false);
        let Some(copy) = r.created.first().cloned() else { return };
        let mut ops = vec![op_page_update(&page_id, one("durationMs", num(cut))), {
            let mut p = one("durationMs", num(page_ms - cut));
            p.insert("transition".into(), Value::Null);
            op_page_update(&copy, p)
        }];
        let doc = self.doc();
        for el in children_of(doc.els(), &copy) {
            let id = el_id(el);
            match el {
                Element::Video(v) => {
                    if let Some(src) = video_source_time(v, cut, page_ms) {
                        let mut clip = jm(&v.clip);
                        clip.insert("trimStartMs".into(), num(src));
                        let mut patch = one("clip", Value::Object(clip));
                        patch.insert("timing".into(), Value::Null);
                        ops.push(op_update(&id, patch));
                    }
                }
                _ => match el.base().and_then(|b| b.timing.as_ref()) {
                    Some(tm) => {
                        let st = max(0.0, tm.start_ms - cut);
                        let mut timing = one("startMs", num(st));
                        if let Some(e) = tm.end_ms {
                            timing.insert("endMs".into(), num(max(st, e - cut)));
                        }
                        let mut patch = one("timing", Value::Object(timing));
                        patch.insert("anim".into(), Value::Null);
                        ops.push(op_update(&id, patch));
                    }
                    None => ops.push(op_update(&id, one("anim", Value::Null))),
                },
            }
        }
        self.apply(ops, false);
    }

    // ---- soundtrack (meta.audio, atomic list)

    /// The soundtrack clips (JSON `AudioClip[]`).
    pub fn audio_clips(&self) -> Vec<Map<String, Value>> {
        self.doc()
            .snap
            .meta
            .get("audio")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|c| c.as_object().cloned()).collect())
            .unwrap_or_default()
    }

    fn set_audio(&mut self, clips: Vec<Map<String, Value>>, merge: bool) {
        let v = Value::Array(clips.into_iter().map(Value::Object).collect());
        self.apply(vec![op_meta(one("audio", v))], merge);
    }

    /// Add a soundtrack clip at `at` (default: the playhead or 0). Returns the clip id.
    pub fn add_audio(&mut self, asset: &MediaAsset, at: Option<f64>) -> String {
        let at = at.unwrap_or_else(|| self.state.playhead.unwrap_or(0.0));
        let total = max(self.total_ms(), 1000.0);
        let len = min(asset.duration_ms.unwrap_or(total), max(1000.0, total - at));
        let id = self.fresh_id();
        let mut clip = one("id", Value::String(id.clone()));
        clip.insert("asset".into(), Value::String(asset.id.clone()));
        if let Some(n) = asset.name.as_ref().filter(|n| !n.is_empty()) {
            clip.insert("name".into(), Value::String(n.clone()));
        }
        for (k, v) in [("startMs", at), ("trimStartMs", 0.0), ("durationMs", len), ("volume", 1.0), ("fadeInMs", 0.0), ("fadeOutMs", 800.0)] {
            clip.insert(k.into(), num(v));
        }
        let mut clips = self.audio_clips();
        clips.push(clip);
        self.set_audio(clips, false);
        id
    }

    /// Patch a clip (TS default `merge = true`).
    pub fn update_audio(&mut self, id: &str, patch: &Map<String, Value>, merge: bool) {
        let clips = self
            .audio_clips()
            .into_iter()
            .map(|c| if s(c.get("id")) == Some(id) { crate::util::merge(&c, patch) } else { c })
            .collect();
        self.set_audio(clips, merge);
    }

    pub fn remove_audio(&mut self, id: &str) {
        let clips = self.audio_clips().into_iter().filter(|c| s(c.get("id")) != Some(id)).collect();
        self.set_audio(clips, false);
    }

    /// Split a clip at `t` (default: the playhead or 0).
    pub fn split_audio(&mut self, id: &str, t: Option<f64>) {
        let t = t.unwrap_or_else(|| self.state.playhead.unwrap_or(0.0));
        let clips = self.audio_clips();
        let Some(c) = clips.iter().find(|x| s(x.get("id")) == Some(id)).cloned() else { return };
        let start = f(c.get("startMs")).unwrap_or(0.0);
        let dur = f(c.get("durationMs")).unwrap_or(0.0);
        if t <= start + 100.0 || t >= start + dur - 100.0 {
            return;
        }
        let first = t - start;
        let mut a = c.clone();
        a.insert("durationMs".into(), num(first));
        a.insert("fadeOutMs".into(), num(min(f(c.get("fadeOutMs")).unwrap_or(0.0), first / 2.0)));
        let mut b = c.clone();
        b.insert("id".into(), Value::String(self.fresh_id()));
        b.insert("startMs".into(), num(t));
        b.insert("trimStartMs".into(), num(f(c.get("trimStartMs")).unwrap_or(0.0) + first));
        b.insert("durationMs".into(), num(dur - first));
        b.insert("fadeInMs".into(), num(0.0));
        let next = clips.into_iter().flat_map(|x| if s(x.get("id")) == Some(id) { vec![a.clone(), b.clone()] } else { vec![x] }).collect();
        self.set_audio(next, false);
    }

    // ---- captions (meta.captions)

    /// The caption track (JSON `CaptionTrack`; a default one when the design has none).
    pub fn captions(&self) -> Map<String, Value> {
        let doc = self.doc();
        if let Some(Value::Object(c)) = doc.snap.meta.get("captions") {
            return c.clone();
        }
        let mut style = one("family", Value::String("Inter".into()));
        style.insert("size".into(), num(round(doc.doc.meta.dims.h * 0.04)));
        style.insert("color".into(), Value::String("#ffffff".into()));
        style.insert("background".into(), Value::String("#000000".into()));
        style.insert("position".into(), Value::String("bottom".into()));
        let mut t = one("cues", Value::Array(vec![]));
        t.insert("visible".into(), Value::Bool(true));
        t.insert("style".into(), Value::Object(style));
        t
    }

    pub fn set_captions(&mut self, track: &Map<String, Value>, merge: bool) {
        let mut t = track.clone();
        let mut cues: Vec<Value> = track.get("cues").and_then(Value::as_array).cloned().unwrap_or_default();
        cues.sort_by(|a, b| crate::util::num_cmp(f(a.get("startMs")).unwrap_or(f64::NAN), f(b.get("startMs")).unwrap_or(f64::NAN)));
        t.insert("cues".into(), Value::Array(cues));
        self.apply(vec![op_meta(one("captions", Value::Object(t)))], merge);
    }

    /// Insert or patch a cue (JSON `Partial<CaptionCue>`). Returns its id.
    pub fn upsert_cue(&mut self, cue: &Map<String, Value>) -> String {
        let mut track = self.captions();
        let id = match s(cue.get("id")) {
            Some(i) => i.to_string(),
            None => self.fresh_id(),
        };
        let mut cues: Vec<Value> = track.get("cues").and_then(Value::as_array).cloned().unwrap_or_default();
        let exists = cues.iter().any(|c| s(c.get("id")) == Some(id.as_str()));
        if exists {
            for c in cues.iter_mut() {
                if s(c.get("id")) == Some(id.as_str())
                    && let Value::Object(m) = c
                {
                    let mut n = crate::util::merge(m, cue);
                    n.insert("id".into(), Value::String(id.clone()));
                    *m = n;
                }
            }
        } else {
            let mut n = one("startMs", num(0.0));
            n.insert("endMs".into(), num(2000.0));
            n.insert("text".into(), Value::String(String::new()));
            let mut n = crate::util::merge(&n, cue);
            n.insert("id".into(), Value::String(id.clone()));
            cues.push(Value::Object(n));
        }
        track.insert("cues".into(), Value::Array(cues));
        self.set_captions(&track, true);
        id
    }

    // ------------------------------------------------------------------ keyframes (MV-100)

    pub fn set_record_keyframes(&mut self, on: bool) {
        self.set(|s| s.record_keyframes = on);
    }

    /// Page time of the playhead on an element's page (`None` when the timeline isn't open).
    pub fn keyframe_time(&self, id: &str) -> Option<f64> {
        let page = self.doc().page_of(id)?;
        let ph = self.state.playhead?;
        Some(max(0.0, round(ph - self.page_start(&page))))
    }

    fn keyframes_json(&self, id: &str) -> Option<Option<Map<String, Value>>> {
        let doc = self.doc();
        let raw = doc.raw_el(id)?;
        Some(raw.get("keyframes").and_then(Value::as_object).cloned())
    }

    pub fn set_keyframe(&mut self, id: &str, prop: &str, t: f64, v: f64, ease: Option<&str>) {
        let Some(k) = self.keyframes_json(id) else { return };
        let next = with_keyframe(&k.unwrap_or_default(), prop, t, v, ease);
        self.apply(vec![op_update(id, one("keyframes", Value::Object(next)))], false);
    }

    pub fn remove_keyframe(&mut self, id: &str, prop: &str, t: f64) {
        let Some(Some(k)) = self.keyframes_json(id) else { return };
        let Some(keys) = k.get(prop).and_then(Value::as_array) else { return };
        let filtered: Vec<Value> = keys.iter().filter(|x| (f(x.get("t")).unwrap_or(f64::NAN) - t).abs() > 20.0).cloned().collect();
        let mut next = k.clone();
        if filtered.is_empty() {
            next.remove(prop);
        } else {
            next.insert(prop.to_string(), Value::Array(filtered));
        }
        let v = if next.is_empty() { Value::Null } else { Value::Object(next) };
        self.apply(vec![op_update(id, one("keyframes", v))], false);
    }

    /// Move a keyframe in time (all properties keyed at `from` move together when `prop` is `None`).
    pub fn move_keyframe(&mut self, id: &str, from: f64, to: f64, prop: Option<&str>) {
        let Some(Some(k)) = self.keyframes_json(id) else { return };
        let mut next = Map::new();
        for (p, keys) in &k {
            let Some(arr) = keys.as_array() else {
                next.insert(p.clone(), keys.clone());
                continue;
            };
            let mut moved: Vec<Value> = arr
                .iter()
                .map(|x| {
                    let kt = f(x.get("t")).unwrap_or(f64::NAN);
                    if (kt - from).abs() <= 20.0 && prop.is_none_or(|pp| pp == p) {
                        let mut m = x.as_object().cloned().unwrap_or_default();
                        m.insert("t".into(), num(max(0.0, round(to))));
                        Value::Object(m)
                    } else {
                        x.clone()
                    }
                })
                .collect();
            moved.sort_by(|a, b| crate::util::num_cmp(f(a.get("t")).unwrap_or(f64::NAN), f(b.get("t")).unwrap_or(f64::NAN)));
            next.insert(p.clone(), Value::Array(moved));
        }
        self.apply(vec![op_update(id, one("keyframes", Value::Object(next)))], true);
    }

    pub fn clear_keyframes(&mut self, id: &str) {
        if matches!(self.keyframes_json(id), Some(Some(_))) {
            self.apply(vec![op_update(id, one("keyframes", Value::Null))], false);
        }
    }

}
