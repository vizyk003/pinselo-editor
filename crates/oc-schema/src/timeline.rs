// SPDX-License-Identifier: Apache-2.0
//! Design timeline (MV-001): pages play back-to-back; each element has an optional visibility window.

use serde::Serialize;
use serde_json::Value;

use crate::design::{DesignDoc, Page};
use crate::elements::{Element, MediaClip, VideoEl};
use crate::factory::media_clip_length_ms;
use crate::tree::{Elements, children_of, sorted_pages};

pub const DEFAULT_PAGE_MS: f64 = 5000.0;

/// Page length: explicit, else long enough for its videos, else 5 s.
pub fn page_duration_ms(elements: &Elements, page: &Page) -> f64 {
    if let Some(d) = page.duration_ms
        && d != 0.0
        && !d.is_nan()
    {
        return d;
    }
    let mut end: f64 = 0.0;
    for el in children_of(elements, &page.id) {
        for v in video_slots(el) {
            if !v.clip.loop_ {
                end = end.max(timing_of(el).0.unwrap_or(0.0) + media_clip_length_ms(v.clip));
            }
        }
    }
    DEFAULT_PAGE_MS.max((end / 100.0).ceil() * 100.0)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSpan<'a> {
    pub page: &'a Page,
    pub index: usize,
    pub start_ms: f64,
    pub duration_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline<'a> {
    pub spans: Vec<PageSpan<'a>>,
    pub total_ms: f64,
}

pub fn timeline_of(doc: &DesignDoc, include_hidden: bool) -> Timeline<'_> {
    let mut t = 0.0;
    let mut spans = Vec::new();
    for (index, page) in sorted_pages(&doc.pages).into_iter().enumerate() {
        if page.hidden == Some(true) && !include_hidden {
            continue;
        }
        let d = page_duration_ms(&doc.elements, page);
        spans.push(PageSpan { page, index, start_ms: t, duration_ms: d });
        t += d;
    }
    Timeline { spans, total_ms: t }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocatedTime<'a> {
    pub span: PageSpan<'a>,
    pub page_time_ms: f64,
}

/// Which page plays at design time `t`, and the time inside it.
pub fn locate_time(doc: &DesignDoc, t: f64) -> Option<LocatedTime<'_>> {
    let spans = timeline_of(doc, false).spans;
    for s in &spans {
        if t < s.start_ms + s.duration_ms {
            return Some(LocatedTime { span: s.clone(), page_time_ms: 0f64.max(t - s.start_ms) });
        }
    }
    spans.last().map(|last| LocatedTime { span: last.clone(), page_time_ms: last.duration_ms })
}

/// `(timing.startMs, timing.endMs)` of any element (unknown types read the raw JSON).
fn timing_of(el: &Element) -> (Option<f64>, Option<f64>) {
    match el {
        Element::Unknown(m) => {
            let t = m.get("timing").and_then(Value::as_object);
            let num = |k: &str| t.and_then(|t| t.get(k)).and_then(Value::as_f64);
            (num("startMs"), num("endMs"))
        }
        _ => {
            let t = el.base().and_then(|b| b.timing.as_ref());
            (t.map(|t| t.start_ms), t.and_then(|t| t.end_ms))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Window {
    pub start: f64,
    pub end: f64,
}

fn window(timing: (Option<f64>, Option<f64>), video: Option<&MediaClip>, page_ms: f64) -> Window {
    let start = 0f64.max(timing.0.unwrap_or(0.0));
    let mut end = timing.1.unwrap_or(page_ms);
    if let Some(clip) = video
        && !clip.loop_
        && timing.1.is_none()
    {
        end = page_ms.min(start + media_clip_length_ms(clip));
    }
    Window { start, end: start.max(end) }
}

/// Visibility window of an element in page time.
pub fn element_window(el: &Element, page_ms: f64) -> Window {
    let video = match el {
        Element::Video(v) => Some(&v.clip),
        _ => None,
    };
    window(timing_of(el), video, page_ms)
}

fn source_time(w: Window, clip: &MediaClip, page_time_ms: f64) -> Option<f64> {
    if page_time_ms < w.start || page_time_ms >= w.end {
        return None;
    }
    let len = clip.trim_end_ms - clip.trim_start_ms;
    let mut local = (page_time_ms - w.start) * clip.speed;
    if clip.loop_ && len > 0.0 {
        local %= len;
    }
    Some(clip.trim_start_ms.max((clip.trim_end_ms - 1.0).min(clip.trim_start_ms + local)))
}

/// Source time (ms) of a video at page time t, or `None` when it isn't playing.
pub fn video_source_time(el: &VideoEl, page_time_ms: f64, page_ms: f64) -> Option<f64> {
    let t = el.base.timing.as_ref();
    let w = window((t.map(|t| t.start_ms), t.and_then(|t| t.end_ms)), Some(&el.clip), page_ms);
    source_time(w, &el.clip, page_time_ms)
}

/// Source time of a clip played by element `el` (a video, or a frame / grid cell holding a video) at page time
/// t, or `None` outside the element's window.
pub fn media_source_time(el: &Element, clip: &MediaClip, page_time_ms: f64, page_ms: f64) -> Option<f64> {
    source_time(element_window(el, page_ms), clip, page_time_ms)
}

/// One video played by an element: `key` is unique per playback (grid cells get `<id>#<index>`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VideoSlot<'a> {
    pub key: String,
    pub asset: &'a str,
    pub clip: &'a MediaClip,
}

/// Every video an element plays (A13): video elements, frames holding a video and video grid cells.
pub fn video_slots(el: &Element) -> Vec<VideoSlot<'_>> {
    match el {
        Element::Video(v) => vec![VideoSlot { key: v.base.id.clone(), asset: &v.asset, clip: &v.clip }],
        Element::Frame(f) => match &f.content {
            Some(c) => match &c.video {
                Some(clip) => vec![VideoSlot { key: f.base.id.clone(), asset: &c.asset, clip }],
                None => vec![],
            },
            None => vec![],
        },
        Element::Grid(g) => g
            .cells
            .iter()
            .enumerate()
            .filter_map(|(i, c)| match (&c.asset, &c.video) {
                (Some(a), Some(clip)) if !a.is_empty() => {
                    Some(VideoSlot { key: format!("{}#{i}", g.base.id), asset: a, clip })
                }
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}
