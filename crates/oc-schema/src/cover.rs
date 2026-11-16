// SPDX-License-Identifier: Apache-2.0
//! The cover of a video post: which frame of the exported MP4 stands for it on Instagram and Facebook.
//!
//! A Content Planner post keeps its choice in `options.cover`: `"last_frame"` (the default, also when the key is
//! missing), `"first_frame"`, or a time in seconds. [`cover_frame`] turns the choice into a frame of the video as the
//! MP4 export lays it out (`oc-render-svc` `video::frames`): every visible page for its length at [`VIDEO_FPS`], with
//! a page's transition drawn over its first frames.
//!
//! "The last frame" is the last frame that shows a page on its own: when the last page is no longer than its
//! transition, every one of its frames is mid-transition, and the cover is the previous page's last frame instead.
//! A time past the end of the video is clamped to that same frame.

use serde_json::Value;

use crate::design::{DesignDoc, Page};
use crate::js::round;
use crate::timeline::timeline_of;

/// Frame rate of the MP4 export when none is asked for (what publishing renders).
pub const VIDEO_FPS: f64 = 30.0;

/// `options.cover` of a video post.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cover {
    LastFrame,
    FirstFrame,
    /// A time in the video, in seconds (0 or more).
    AtSeconds(f64),
}

/// Why an `options.cover` value was refused.
pub const COVER_HINT: &str = "cover must be \"last_frame\", \"first_frame\" or a time in seconds (0 or more).";

impl Cover {
    /// Read `options.cover`. Missing or `null` is the default (`LastFrame`); a numeric string is read as seconds.
    pub fn parse(v: Option<&Value>) -> Result<Cover, &'static str> {
        let secs = |n: f64| if n.is_finite() && n >= 0.0 { Ok(Cover::AtSeconds(n)) } else { Err(COVER_HINT) };
        match v {
            None | Some(Value::Null) => Ok(Cover::LastFrame),
            Some(Value::String(s)) => match s.trim() {
                "last_frame" => Ok(Cover::LastFrame),
                "first_frame" => Ok(Cover::FirstFrame),
                t => t.parse::<f64>().map_err(|_| COVER_HINT).and_then(secs),
            },
            Some(Value::Number(n)) => n.as_f64().map_or(Err(COVER_HINT), secs),
            Some(_) => Err(COVER_HINT),
        }
    }

    /// The value as stored in `options.cover`.
    pub fn to_value(self) -> Value {
        match self {
            Cover::LastFrame => Value::from("last_frame"),
            Cover::FirstFrame => Value::from("first_frame"),
            // Whole seconds print like JS numbers (`600`, not `600.0`).
            Cover::AtSeconds(s) if s.fract() == 0.0 && s < 9_007_199_254_740_992.0 => Value::from(s as u64),
            Cover::AtSeconds(s) => serde_json::Number::from_f64(s).map_or(Value::Null, Value::Number),
        }
    }
}

/// One frame of the exported video.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverFrame<'a> {
    /// 0-based frame number in the MP4.
    pub index: u64,
    /// The frame's time in the MP4 (ms): Instagram's `thumb_offset`.
    pub video_ms: f64,
    /// The page on screen and the time inside it (ms), as the export renders it.
    pub page: &'a Page,
    pub page_ms: f64,
    /// The page being left when the frame is mid-transition (an explicit time can land there).
    pub transition_from: Option<&'a Page>,
}

/// Frames of each visible page at `fps`, as the export counts them: `max(1, round(ms / 1000 · fps))`.
fn frame_counts(doc: &DesignDoc, fps: f64) -> Vec<(&Page, u64)> {
    timeline_of(doc, false)
        .spans
        .into_iter()
        .map(|s| (s.page, round((s.duration_ms / 1000.0) * fps).max(1.0) as u64))
        .collect()
}

/// The frame `cover` picks in the design's MP4 at `fps`; `None` when no page is visible.
pub fn cover_frame(doc: &DesignDoc, cover: Cover, fps: f64) -> Option<CoverFrame<'_>> {
    let spans = frame_counts(doc, fps);
    let total: u64 = spans.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return None;
    }
    // Where frame `k` falls: (span index, frame inside the span, first frame of the span).
    let locate = |k: u64| {
        let mut start = 0;
        for (j, (_, n)) in spans.iter().enumerate() {
            if k < start + n {
                return (j, k - start, start);
            }
            start += n;
        }
        unreachable!("frame {k} is inside the video")
    };
    let page_ms = |i: u64| (i as f64 / fps) * 1000.0;
    // A page's transition plays over its first frames (never the first page's: nothing comes before it).
    let mid_transition = |j: usize, i: u64| {
        j > 0 && spans[j].0.transition.as_ref().is_some_and(|t| page_ms(i) < t.duration_ms)
    };
    let last_shown = || {
        let mut k = total - 1;
        loop {
            let (j, i, start) = locate(k);
            if !mid_transition(j, i) {
                return k;
            }
            k = start - 1;
        }
    };
    let index = match cover {
        Cover::FirstFrame => 0,
        Cover::LastFrame => last_shown(),
        Cover::AtSeconds(s) => {
            // The frame on screen at that time (a hair of slack so 0.7 s × 30 is frame 21, not 20).
            let k = (s * fps + 1e-6).floor();
            if k >= total as f64 { last_shown() } else { k as u64 }
        }
    };
    let (j, i, _) = locate(index);
    Some(CoverFrame {
        index,
        video_ms: page_ms(index),
        page: spans[j].0,
        page_ms: page_ms(i),
        transition_from: mid_transition(j, i).then(|| spans[j - 1].0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::{PageTransition, TransitionKind};
    use crate::primitives::Extra;
    use serde_json::json;

    /// Pages of the given lengths (ms), in order, ids `p1`, `p2`, …
    fn pages(lengths: &[f64]) -> DesignDoc {
        let mut d = crate::factory::empty_design(&crate::presets::preset_design_init(None, &Default::default()).meta_init());
        let first = d.pages.values().next().unwrap().clone();
        d.pages.clear();
        for (i, ms) in lengths.iter().enumerate() {
            let mut p = first.clone();
            p.id = format!("p{}", i + 1);
            p.order = format!("a{i}");
            p.duration_ms = Some(*ms);
            d.pages.insert(p.id.to_string(), p);
        }
        d
    }

    fn fade(d: &mut DesignDoc, page: &str, ms: f64) {
        d.pages.get_mut(page).unwrap().transition =
            Some(PageTransition { kind: TransitionKind::Fade, duration_ms: ms, direction: None, extra: Extra::new() });
    }

    fn at(d: &DesignDoc, c: Cover) -> (u64, String, f64) {
        let f = cover_frame(d, c, VIDEO_FPS).unwrap();
        (f.index, f.page.id.to_string(), round(f.page_ms))
    }

    #[test]
    fn the_default_is_the_last_frame() {
        assert_eq!(Cover::parse(None), Ok(Cover::LastFrame));
        assert_eq!(Cover::parse(Some(&Value::Null)), Ok(Cover::LastFrame));
        assert_eq!(Cover::parse(Some(&json!("last_frame"))), Ok(Cover::LastFrame));
        // Three 2 s pages at 30 fps: 180 frames, the last one 1967 ms into the third page.
        let d = pages(&[2000.0, 2000.0, 2000.0]);
        assert_eq!(at(&d, Cover::LastFrame), (179, "p3".into(), 1967.0));
        assert_eq!(cover_frame(&d, Cover::LastFrame, VIDEO_FPS).unwrap().video_ms, 179.0 / 30.0 * 1000.0);
    }

    #[test]
    fn the_last_frame_is_never_mid_transition() {
        // A long fade into the last page still ends before the page does: its last frame is the cover.
        let mut d = pages(&[2000.0, 2000.0]);
        fade(&mut d, "p2", 800.0);
        assert_eq!(at(&d, Cover::LastFrame), (119, "p2".into(), 1967.0));
        // The last page is all transition: the previous page's last frame stands for the video.
        let mut d = pages(&[2000.0, 500.0]);
        fade(&mut d, "p2", 1000.0);
        let f = cover_frame(&d, Cover::LastFrame, VIDEO_FPS).unwrap();
        assert_eq!((f.index, &*f.page.id, f.transition_from), (59, "p1", None));
    }

    #[test]
    fn an_explicit_time_picks_the_frame_on_screen_then() {
        assert_eq!(Cover::parse(Some(&json!(2.5))), Ok(Cover::AtSeconds(2.5)));
        assert_eq!(Cover::parse(Some(&json!(" 0.7 "))), Ok(Cover::AtSeconds(0.7)));
        let d = pages(&[2000.0, 2000.0, 2000.0]);
        assert_eq!(at(&d, Cover::AtSeconds(2.5)), (75, "p2".into(), 500.0));
        assert_eq!(at(&d, Cover::AtSeconds(0.7)).0, 21);
        assert_eq!(at(&d, Cover::AtSeconds(0.0)), at(&d, Cover::FirstFrame));
        assert_eq!(at(&d, Cover::FirstFrame), (0, "p1".into(), 0.0));
        // A time inside a transition is kept, with the page being left for the preview to blend.
        let mut d = pages(&[2000.0, 2000.0]);
        fade(&mut d, "p2", 1000.0);
        let f = cover_frame(&d, Cover::AtSeconds(2.2), VIDEO_FPS).unwrap();
        assert_eq!((f.index, &*f.page.id, f.transition_from.map(|p| &*p.id)), (66, "p2", Some("p1")));
    }

    #[test]
    fn a_time_past_the_end_is_clamped_to_the_last_frame() {
        let d = pages(&[2000.0, 2000.0, 2000.0]);
        assert_eq!(at(&d, Cover::AtSeconds(99.0)), at(&d, Cover::LastFrame));
        // Exactly the video's length is already past its last frame (5.967 s).
        assert_eq!(at(&d, Cover::AtSeconds(6.0)).0, 179);
        let mut d = pages(&[2000.0, 500.0]);
        fade(&mut d, "p2", 1000.0);
        assert_eq!(at(&d, Cover::AtSeconds(60.0)), at(&d, Cover::LastFrame));
    }

    #[test]
    fn refuses_other_values() {
        for v in [json!(-1), json!("middle"), json!(true), json!([1]), json!("NaN"), json!("inf")] {
            assert_eq!(Cover::parse(Some(&v)), Err(COVER_HINT), "{v}");
        }
        assert_eq!(Cover::AtSeconds(1.5).to_value(), json!(1.5));
        assert_eq!(Cover::AtSeconds(600.0).to_value().to_string(), "600");
        assert_eq!(Cover::LastFrame.to_value(), json!("last_frame"));
    }

    #[test]
    fn hidden_pages_are_not_in_the_video() {
        let mut d = pages(&[2000.0, 2000.0]);
        d.pages.get_mut("p2").unwrap().hidden = Some(true);
        assert_eq!(at(&d, Cover::LastFrame), (59, "p1".into(), 1967.0));
        d.pages.get_mut("p1").unwrap().hidden = Some(true);
        assert!(cover_frame(&d, Cover::LastFrame, VIDEO_FPS).is_none());
    }
}
