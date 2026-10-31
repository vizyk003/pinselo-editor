// SPDX-License-Identifier: Apache-2.0
//! Video, GIF and audio export — port of the video half of `services/render/src/exporters.ts`.
//!
//! [`frames`] renders the whole design timeline (`timelineOf`): every page for its duration at `fps`, with
//! entrance/exit animations, motion paths, keyframes, element timing and captions evaluated by the resolver from
//! (page time, design time), video elements fed decoded frames ([`VideoDecoder`] over ffmpeg →
//! [`RasterOptions::video_frames`] → oc-render's `VideoFrames`), static pages reusing their settled frame, and
//! page transitions composed from the previous page's last frame and the new frame
//! ([`Renderer::compose_transition`]).
//!
//! [`export_mp4`] pipes the frames into ffmpeg (libx264) and mixes the audio track ([`audio_mix_args`]);
//! [`export_gif`] quantises every frame with the gifenc port; [`export_audio`] is the audio-only mix (M4A).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;
use indexmap::IndexMap;
use oc_render::jsfmt::max;
use oc_render::motion::{page_settle_ms, transition_progress};
use oc_schema::DesignDoc;
use oc_schema::elements::Element;
use oc_schema::js::{round, to_fixed};
use oc_schema::timeline::{media_source_time, timeline_of, video_slots};
use oc_schema::tree::descendants;
use tokio::io::AsyncWriteExt;

use crate::gif::{GifEncoder, apply_palette, frame_key, quantize};
use crate::media::{
    Media, Pipes, Proc, Spawner, VideoDecoder, audio_mix_args, ffmpeg_argv, ffmpeg_failed,
    run_ffmpeg,
};
use crate::renderer::{
    RasterOptions, RenderError, RenderHost, RenderResult, Renderer, Rgba, TransitionSpec,
};

/// Progress callback: fraction (0..1) and a label.
pub type Progress<'a> = &'a (dyn Fn(f64, &str) + Send + Sync);

/// What the exporters need besides the document.
#[derive(Clone, Copy)]
pub struct VideoDeps<'a> {
    pub renderer: &'a dyn Renderer,
    pub host: &'a dyn RenderHost,
    /// `MediaLookup`: playback proxies by asset id.
    pub media: &'a Media,
    pub spawner: &'a dyn Spawner,
    /// Where temporary files go (`os.tmpdir()`).
    pub tmp: &'a Path,
}

/// Receives the frames of [`frames`] in order.
pub trait FrameSink: Send {
    fn frame<'a>(&'a mut self, f: &'a Rgba) -> BoxFuture<'a, RenderResult<()>>;
}

fn failed(e: impl std::fmt::Display) -> RenderError {
    RenderError::Failed(e.to_string())
}

/// JSON truthiness of an element field (`e.timing || e.animOut || …` on an unknown element type).
fn raw_truthy(m: &serde_json::Map<String, serde_json::Value>, k: &str) -> bool {
    use serde_json::Value;
    match m.get(k) {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn visible(el: &Element) -> bool {
    match el {
        Element::Unknown(m) => m.get("visible") != Some(&serde_json::Value::Bool(false)),
        _ => el.base().is_none_or(|b| b.visible != Some(false)),
    }
}

/// `e.timing || e.animOut || e.motionPath || e.emphasis || e.keyframes`.
fn has_motion(el: &Element) -> bool {
    match el {
        Element::Unknown(m) => ["timing", "animOut", "motionPath", "emphasis", "keyframes"]
            .iter()
            .any(|k| raw_truthy(m, k)),
        _ => el.base().is_some_and(|b| {
            b.timing.is_some()
                || b.anim_out.is_some()
                || b.motion_path.is_some()
                || b.emphasis.is_some()
                || b.keyframes.is_some()
        }),
    }
}

/// `frames(r, doc, fps, scale, media, progress)`: the design timeline as frames, handed to `sink` in order.
pub async fn frames(
    deps: VideoDeps<'_>,
    doc: &DesignDoc,
    fps: f64,
    scale: f64,
    sink: &mut dyn FrameSink,
    progress: Option<Progress<'_>>,
) -> RenderResult<()> {
    let tl = timeline_of(doc, false);
    let total_frames = max(1.0, round((tl.total_ms / 1000.0) * fps));
    let mut produced = 0u64;
    let mut prev_last: Option<Arc<Rgba>> = None;
    // Decoders by playback key (`Map`); closed (dropped) at the end of every page.
    let mut decoders: IndexMap<String, VideoDecoder> = IndexMap::new();
    let captions = doc
        .meta
        .captions
        .as_ref()
        .is_some_and(|c| c.visible && !c.cues.is_empty());
    for span in &tl.spans {
        let p = span.page;
        let page_ms = span.duration_ms;
        let n = max(1.0, round((page_ms / 1000.0) * fps)) as u64;
        let trans = p.transition.as_ref().filter(|_| prev_last.is_some());
        let els = descendants(&doc.elements, &p.id);
        let videos: Vec<(&Element, oc_schema::timeline::VideoSlot<'_>)> = els
            .iter()
            .filter(|e| visible(e))
            .flat_map(|e| video_slots(e).into_iter().map(move |v| (*e, v)))
            .collect();
        let dynamic = !videos.is_empty() || captions || els.iter().any(|e| has_motion(e));
        let settle_at = max(
            trans.map_or(0.0, |t| t.duration_ms),
            page_settle_ms(p, &els),
        );
        let mut settled: Option<Arc<Rgba>> = None;
        let mut last: Option<Arc<Rgba>> = None;
        for i in 0..n {
            let t = (i as f64 / fps) * 1000.0;
            let reuse = if !dynamic && t > settle_at + 50.0 {
                settled.clone()
            } else {
                None
            };
            let mut f = match reuse {
                Some(s) => s,
                None => {
                    for (el, v) in &videos {
                        let Some(src) = media_source_time(el, v.clip, t, page_ms) else {
                            continue;
                        };
                        if decoders
                            .get(&v.key)
                            .is_some_and(|d| src < d.position() - 1.0)
                        {
                            decoders.shift_remove(&v.key);
                        }
                        if !decoders.contains_key(&v.key) {
                            let Some(m) = deps.media.get(v.asset) else {
                                continue;
                            };
                            decoders.insert(
                                v.key.clone(),
                                VideoDecoder::new(&m.file, m.w, m.h, src, fps, m.alpha),
                            );
                        }
                        if let Some(d) = decoders.get_mut(&v.key) {
                            d.seek_forward(deps.spawner, src).await;
                        }
                    }
                    let current: HashMap<String, Arc<Rgba>> = decoders
                        .iter()
                        .filter_map(|(k, d)| d.image().map(|img| (k.clone(), img.clone())))
                        .collect();
                    let opts = RasterOptions {
                        scale,
                        time_ms: Some(t),
                        caption_time_ms: Some(span.start_ms + t),
                        video_frames: (!current.is_empty()).then(|| Arc::new(current)),
                        ..RasterOptions::default()
                    };
                    let f = Arc::new(
                        deps.renderer
                            .render_rgba(deps.host, doc, &p.id, &opts)
                            .await?,
                    );
                    if !dynamic && t > settle_at + 50.0 {
                        settled = Some(f.clone());
                    }
                    f
                }
            };
            if let (Some(tr), Some(prev)) = (trans, &prev_last)
                && t < tr.duration_ms
            {
                let spec = TransitionSpec {
                    kind: tr.kind.as_str().to_string(),
                    progress: transition_progress(p, t),
                    direction: tr.direction.as_ref().map(|d| d.as_str().to_string()),
                };
                f = Arc::new(deps.renderer.compose_transition(prev, &f, &spec).await?);
            }
            last = Some(f.clone());
            produced += 1;
            if produced.is_multiple_of(15)
                && let Some(pr) = progress
            {
                pr(
                    produced as f64 / total_frames,
                    &format!(
                        "Rendering frame {produced} of {}",
                        oc_schema::js::num_to_string(total_frames)
                    ),
                );
            }
            sink.frame(&f).await?;
        }
        prev_last = last;
        decoders.clear();
    }
    Ok(())
}

/// `VideoQuality`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoQuality {
    P720,
    P1080,
    K4,
}

impl VideoQuality {
    /// `'720p' | '1080p' | '4k'` (anything else: 1080p).
    pub fn parse(s: &str) -> VideoQuality {
        match s {
            "720p" => VideoQuality::P720,
            "4k" => VideoQuality::K4,
            _ => VideoQuality::P1080,
        }
    }

    fn long_side(self) -> f64 {
        match self {
            VideoQuality::P720 => 1280.0,
            VideoQuality::P1080 => 1920.0,
            VideoQuality::K4 => 3840.0,
        }
    }
}

/// `videoScale(doc, quality)`.
pub fn video_scale(doc: &DesignDoc, quality: VideoQuality) -> f64 {
    quality.long_side() / doc.meta.dims.w.max(doc.meta.dims.h)
}

/// A temporary directory (`mkdtemp(join(tmpdir(), prefix))`), removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(root: &Path, prefix: &str) -> RenderResult<TempDir> {
        for _ in 0..8 {
            let id: String = uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .chars()
                .take(6)
                .collect();
            let p = root.join(format!("{prefix}{id}"));
            match std::fs::create_dir(&p) {
                Ok(()) => return Ok(TempDir(p)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(failed(e)),
            }
        }
        Err(failed("could not create a temporary directory"))
    }

    fn file(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn s(v: &str) -> String {
    v.to_string()
}

/// The MP4 encoder: ffmpeg started on the first frame, frames written to its stdin.
struct Mp4Sink<'a> {
    spawner: &'a dyn Spawner,
    fps: f64,
    out: String,
    enc: Option<Proc>,
}

impl Mp4Sink<'_> {
    /// The encoder's arguments for the first frame's size.
    fn args(&self, w: u32, h: u32) -> Vec<String> {
        let (ew, eh) = (w - w % 2, h - h % 2);
        vec![
            s("-f"),
            s("rawvideo"),
            s("-pix_fmt"),
            s("rgba"),
            s("-s"),
            format!("{w}x{h}"),
            s("-r"),
            oc_schema::js::num_to_string(self.fps),
            s("-i"),
            s("-"),
            s("-vf"),
            format!("crop={ew}:{eh}:0:0"),
            s("-c:v"),
            s("libx264"),
            s("-pix_fmt"),
            s("yuv420p"),
            s("-preset"),
            s("veryfast"),
            s("-crf"),
            s("19"),
            s("-movflags"),
            s("+faststart"),
            self.out.clone(),
        ]
    }

    async fn finish(&mut self) -> RenderResult<()> {
        if let Some(mut p) = self.enc.take() {
            if let Some(mut stdin) = p.stdin.take() {
                let _ = stdin.shutdown().await;
            }
            p.done.await.map_err(|e| failed(ffmpeg_failed(&e)))?;
        }
        Ok(())
    }
}

impl FrameSink for Mp4Sink<'_> {
    fn frame<'a>(&'a mut self, f: &'a Rgba) -> BoxFuture<'a, RenderResult<()>> {
        Box::pin(async move {
            if self.enc.is_none() {
                let argv = ffmpeg_argv(&self.args(f.width, f.height));
                self.enc = Some(
                    self.spawner
                        .spawn(&argv, Pipes::Stdin)
                        .map_err(|e| failed(ffmpeg_failed(&e.to_string())))?,
                );
            }
            let p = self.enc.as_mut().expect("encoder");
            let ok = match p.stdin.as_mut() {
                Some(stdin) => stdin.write_all(&f.pixels).await.is_ok(),
                None => false,
            };
            if !ok {
                // The encoder went away: report its error.
                let p = self.enc.take().expect("encoder");
                drop(p.stdin);
                return Err(failed(ffmpeg_failed(
                    &p.done.await.err().unwrap_or_default(),
                )));
            }
            Ok(())
        })
    }
}

/// `exportMp4(r, doc, media, { quality, fps }, progress)`.
pub async fn export_mp4(
    deps: VideoDeps<'_>,
    doc: &DesignDoc,
    quality: VideoQuality,
    fps: Option<f64>,
    progress: Option<Progress<'_>>,
) -> RenderResult<Vec<u8>> {
    let fps = fps.unwrap_or(30.0);
    let scale = video_scale(doc, quality);
    let dir = TempDir::new(deps.tmp, "oc-video-")?;
    let silent = dir.file("video.mp4");
    let out = dir.file("out.mp4");
    let mut sink = Mp4Sink {
        spawner: deps.spawner,
        fps,
        out: silent.clone(),
        enc: None,
    };
    let scaled = |k: f64, l: &str| {
        if let Some(p) = progress {
            p(k * 0.9, l);
        }
    };
    // On error the encoder is killed with the sink.
    frames(deps, doc, fps, scale, &mut sink, Some(&scaled)).await?;
    sink.finish().await?;
    let total_ms = timeline_of(doc, false).total_ms;
    let Some(mix) = audio_mix_args(doc, deps.media, total_ms, 1) else {
        return std::fs::read(&silent).map_err(failed);
    };
    if let Some(p) = progress {
        p(0.93, "Mixing audio");
    }
    let mut args = vec![s("-i"), silent];
    args.extend(mix.inputs);
    args.extend([
        s("-filter_complex"),
        mix.filter,
        s("-map"),
        s("0:v"),
        s("-map"),
        s("[aout]"),
        s("-c:v"),
        s("copy"),
        s("-c:a"),
        s("aac"),
        s("-b:a"),
        s("192k"),
        s("-t"),
        to_fixed(total_ms / 1000.0, 3),
        s("-movflags"),
        s("+faststart"),
        out.clone(),
    ]);
    run_ffmpeg(deps.spawner, &args).await.map_err(failed)?;
    std::fs::read(&out).map_err(failed)
}

/// The GIF encoder sink (12 fps, a palette per frame unless the frame looks unchanged).
struct GifSink {
    gif: GifEncoder,
    last_palette: Option<Vec<crate::gif::Color>>,
    last_key: Option<crate::gif::FrameKey>,
    delay: f64,
}

impl FrameSink for GifSink {
    fn frame<'a>(&'a mut self, f: &'a Rgba) -> BoxFuture<'a, RenderResult<()>> {
        Box::pin(async move {
            let key = frame_key(&f.pixels);
            let palette = match &self.last_palette {
                Some(p) if self.last_key == Some(key) => p.clone(),
                _ => quantize(&f.pixels, 256),
            };
            let index = apply_palette(&f.pixels, &palette);
            self.gif
                .write_frame(&index, f.width, f.height, Some(&palette), self.delay);
            self.last_palette = Some(palette);
            self.last_key = Some(key);
            Ok(())
        })
    }
}

/// `exportGif(r, doc, media, progress)`.
pub async fn export_gif(
    deps: VideoDeps<'_>,
    doc: &DesignDoc,
    progress: Option<Progress<'_>>,
) -> RenderResult<Vec<u8>> {
    let fps = 12.0;
    let scale = 1f64.min(640.0 / doc.meta.dims.w.max(doc.meta.dims.h));
    let mut sink = GifSink {
        gif: GifEncoder::new(),
        last_palette: None,
        last_key: None,
        delay: round(1000.0 / fps),
    };
    frames(deps, doc, fps, scale, &mut sink, progress).await?;
    sink.gif.finish();
    Ok(sink.gif.into_bytes())
}

/// `exportAudio(doc, media)`: the soundtrack and video audio as M4A; `None` when the design is silent.
pub async fn export_audio(deps: VideoDeps<'_>, doc: &DesignDoc) -> RenderResult<Option<Vec<u8>>> {
    let total_ms = timeline_of(doc, false).total_ms;
    let Some(mix) = audio_mix_args(doc, deps.media, total_ms, 0) else {
        return Ok(None);
    };
    let dir = TempDir::new(deps.tmp, "oc-audio-")?;
    let out = dir.file("out.m4a");
    let mut args = mix.inputs;
    args.extend([
        s("-filter_complex"),
        mix.filter,
        s("-map"),
        s("[aout]"),
        s("-c:a"),
        s("aac"),
        s("-b:a"),
        s("192k"),
        out.clone(),
    ]);
    run_ffmpeg(deps.spawner, &args).await.map_err(failed)?;
    Ok(Some(std::fs::read(&out).map_err(failed)?))
}
