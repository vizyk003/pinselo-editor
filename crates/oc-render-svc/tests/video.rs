// SPDX-License-Identifier: Apache-2.0
//! Golden parity of the video/GIF/audio exports with the TS render service (tests/golden/render-video/, written by
//! tools/golden/render-video.ts):
//!
//! - **gifenc**: palettes, indices and encoded bytes identical on fixed RGBA input;
//! - **GIF export**: byte-identical file over the synthetic renderer (same renderer calls, same progress);
//! - **MP4 plans**: with the encoder and mixer mocked (decoders run on the test media), identical ffmpeg command
//!   lines and audio filter graphs, identical renderer calls (page, time, caption time, scale, video frame bytes),
//!   identical rendered frames in the same order and page-transition frames within a tolerance (composed by
//!   oc-render here, CanvasKit there); M4A command lines identical;
//! - **real export**: the MP4/M4A written with the real painter and ffmpeg has the same stream layout and duration
//!   (ffprobe) and its decoded frames match the TS file's perceptually.
//!
//! The tests that run ffmpeg are skipped (with a note) when it is not installed.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex};
use std::task::{Context, Poll};

use base64::Engine as _;
use oc_gfonts::manifest::{FontFamily, FontManifest};
use oc_render_svc::gif::{GifEncoder, apply_palette, quantize};
use oc_render_svc::media::{Media, MediaInfo, Pipes, Proc, Spawner, SystemSpawner, audio_mix_args};
use oc_render_svc::oc_renderer::{OcRenderer, compose_frames};
use oc_render_svc::renderer::*;
use oc_render_svc::video::{VideoDeps, VideoQuality, export_audio, export_gif, export_mp4};
use oc_schema::DesignDoc;
use oc_schema::timeline::{timeline_of, video_slots};
use oc_schema::tree::{descendants, sorted_pages};
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWrite;

use common::{diff, root};

fn dir() -> PathBuf {
    root().join("tests/golden/render-video")
}

fn golden(rel: &str) -> Value {
    let p = dir().join(rel);
    let s = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("read {}: {e} (run `pnpm golden render-video`)", p.display()));
    serde_json::from_str(&s).expect("golden json")
}

fn design(name: &str) -> DesignDoc {
    static D: LazyLock<Value> = LazyLock::new(|| golden("designs.json"));
    serde_json::from_value(D[name].clone()).unwrap_or_else(|e| panic!("design {name}: {e}"))
}

fn media() -> Media {
    let m = golden("media.json");
    m.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                MediaInfo {
                    file: dir()
                        .join(v["file"].as_str().unwrap())
                        .to_string_lossy()
                        .into_owned(),
                    w: v["w"].as_u64().unwrap() as u32,
                    h: v["h"].as_u64().unwrap() as u32,
                    has_audio: v["hasAudio"].as_bool().unwrap(),
                    duration_ms: v["durationMs"].as_f64().unwrap(),
                    alpha: v["alpha"].as_bool().unwrap_or(false),
                },
            )
        })
        .collect()
}

fn b64d(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .expect("b64")
}

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// A video frame (RGBA) as 4×3 cells of mean RGB over every pixel (the generator's `videoSig`).
fn video_sig(px: &[u8], w: usize, h: usize) -> Vec<i64> {
    const GX: usize = 4;
    const GY: usize = 3;
    let mut sums = [0u64; GX * GY * 3];
    let mut counts = [0u64; GX * GY];
    for y in 0..h {
        let cy = y * GY / h;
        for x in 0..w {
            let c = cy * GX + x * GX / w;
            let o = (y * w + x) * 4;
            counts[c] += 1;
            for k in 0..3 {
                sums[c * 3 + k] += u64::from(px[o + k]);
            }
        }
    }
    sums.iter()
        .enumerate()
        .map(|(i, &s)| match counts[i / 3] {
            0 => 0,
            n => (s as f64 / n as f64).round() as i64,
        })
        .collect()
}

/// Video frames are decoded and scaled by ffmpeg, whose scaler rounds differently between versions (6.1 on CI's
/// Ubuntu, 8.x on Homebrew): their 4×3 mean RGB may differ by this much per channel (tools/golden/compare.mjs).
const VIDEO_SIG_TOL: i64 = 4;

/// The renderer calls: everything exact except the video-frame signatures (within [`VIDEO_SIG_TOL`]).
fn assert_renders(actual: &Value, expected: &Value, what: &str) {
    let strip = |v: &Value| {
        let mut v = v.clone();
        for r in v.as_array_mut().into_iter().flatten() {
            if let Some(vs) = r.get_mut("videos").and_then(Value::as_object_mut) {
                for x in vs.values_mut() {
                    if x.is_array() {
                        *x = json!("<frame>");
                    }
                }
            }
        }
        v
    };
    assert_json(&strip(actual), &strip(expected), what);
    let (a, e) = (actual.as_array().unwrap(), expected.as_array().unwrap());
    for (i, (ra, re)) in a.iter().zip(e).enumerate() {
        let Some(vs) = re["videos"].as_object() else {
            continue;
        };
        for (k, want) in vs {
            let (Some(want), Some(got)) = (want.as_array(), ra["videos"][k].as_array()) else {
                continue;
            };
            for (j, (g, w)) in got.iter().zip(want).enumerate() {
                let d = (g.as_i64().unwrap() - w.as_i64().unwrap()).abs();
                assert!(
                    d <= VIDEO_SIG_TOL,
                    "{what}[{i}].videos.{k}[{j}]: {g} vs {w} (> {VIDEO_SIG_TOL})"
                );
            }
        }
    }
}

fn has_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

fn assert_json(actual: &Value, expected: &Value, what: &str) {
    if let Some(d) = diff(actual, expected, what) {
        panic!("{d}");
    }
}

// ------------------------------------------------------------------ gifenc

#[test]
fn gifenc_matches_js() {
    let g = golden("gifenc.json");
    for c in g["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let (w, h) = (
            c["w"].as_u64().unwrap() as u32,
            c["h"].as_u64().unwrap() as u32,
        );
        let px = b64d(c["rgba"].as_str().unwrap());
        for (n, q) in c["quantize"].as_object().unwrap() {
            let pal = quantize(&px, n.parse().unwrap());
            let want: Vec<[u8; 3]> = serde_json::from_value(q["palette"].clone()).unwrap();
            assert_eq!(pal, want, "{name}: quantize(…, {n}) palette");
            assert_eq!(
                sha(&apply_palette(&px, &pal)),
                q["index"].as_str().unwrap(),
                "{name}: applyPalette with {n} colours"
            );
        }
        let pal = quantize(&px, 256);
        let p16 = quantize(&px, 16);
        let mut enc = GifEncoder::new();
        enc.write_frame(&apply_palette(&px, &pal), w, h, Some(&pal), 83.0);
        enc.write_frame(&apply_palette(&px, &p16), w, h, Some(&p16), 40.0);
        enc.finish();
        let want = b64d(c["gif"].as_str().unwrap());
        let got = enc.into_bytes();
        if got != want {
            let at = got
                .iter()
                .zip(&want)
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(want.len()));
            panic!(
                "{name}: GIF bytes differ at {at} (len {} vs {})",
                got.len(),
                want.len()
            );
        }
    }
}

// ------------------------------------------------------------------ the synthetic renderer

/// `pattern` of the generator: integer arithmetic over (page index, time, caption time).
fn pattern(w: u32, h: u32, pi: i64, time_ms: f64, cap_ms: f64) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut px = vec![0u8; w * h * 4];
    let tq = (time_ms / 40.0).floor() as i64;
    let cq = (cap_ms / 100.0).floor() as i64;
    for y in 0..h {
        let g = ((y as i64 * 2 + tq * 5) & 255) as u8;
        let row = &mut px[y * w * 4..(y + 1) * w * 4];
        for (x, p) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = x as i64;
            p[0] = ((x + pi * 64 + tq * 3) & 255) as u8;
            p[1] = g;
            p[2] = (((x >> 3) * 16 + pi * 50 + cq) & 255) as u8;
            p[3] = 255;
        }
    }
    px
}

#[derive(Default)]
struct Fake {
    log: Mutex<Vec<Value>>,
}

fn ni<'a, T: Send + 'a>() -> RenderFuture<'a, T> {
    Box::pin(async { Err(RenderError::NotImplemented("fake")) })
}

impl Renderer for Fake {
    fn engine(&self) -> &'static str {
        "fake"
    }
    fn render_rgba<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
        o: &'a RasterOptions,
    ) -> RenderFuture<'a, Rgba> {
        Box::pin(async move {
            let w = (doc.meta.dims.w * o.scale).round().max(1.0) as u32;
            let h = (doc.meta.dims.h * o.scale).round().max(1.0) as u32;
            let pi = sorted_pages(&doc.pages)
                .iter()
                .position(|p| *p.id == *page_id)
                .map_or(-1, |i| i as i64);
            let mut videos = BTreeMap::new();
            for e in descendants(&doc.elements, page_id) {
                if e.base().is_some_and(|b| b.visible == Some(false)) {
                    continue;
                }
                for v in video_slots(e) {
                    let f = o.video_frames.as_ref().and_then(|m| m.get(&v.key));
                    videos.insert(
                        v.key.clone(),
                        f.map_or(Value::Null, |f| {
                            json!(video_sig(&f.pixels, f.width as usize, f.height as usize))
                        }),
                    );
                }
            }
            self.log.lock().unwrap().push(json!({ "page": page_id, "timeMs": o.time_ms, "captionTimeMs": o.caption_time_ms, "scale": o.scale, "videos": videos }));
            Ok(Rgba {
                width: w,
                height: h,
                pixels: pattern(
                    w,
                    h,
                    pi,
                    o.time_ms.unwrap_or(0.0),
                    o.caption_time_ms.unwrap_or(0.0),
                ),
            })
        })
    }
    fn render_image<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
        _: &'a RasterOptions,
        _: ImageFormat,
    ) -> RenderFuture<'a, Vec<u8>> {
        ni()
    }
    fn compose_transition<'a>(
        &'a self,
        prev: &'a Rgba,
        next: &'a Rgba,
        t: &'a TransitionSpec,
    ) -> RenderFuture<'a, Rgba> {
        Box::pin(async move { compose_frames(prev, next, t) })
    }
    fn vector_pdf<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a [String],
        _: &'a VectorPdfOptions,
    ) -> RenderFuture<'a, Vec<u8>> {
        ni()
    }
    fn raster_pdf<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a [String],
        _: &'a RasterPdfOptions,
    ) -> RenderFuture<'a, Vec<u8>> {
        ni()
    }
    fn page_svg<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
        _: &'a SvgOptions,
    ) -> RenderFuture<'a, String> {
        ni()
    }
    fn elements_svg<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
        _: &'a std::collections::BTreeSet<String>,
        _: &'a SvgOptions,
    ) -> RenderFuture<'a, String> {
        ni()
    }
    fn text_faces<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a DesignDoc,
        _: &'a str,
    ) -> RenderFuture<'a, Vec<FaceRequest>> {
        ni()
    }
    fn pick_face<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a FaceRequest,
    ) -> RenderFuture<'a, PickedFace> {
        ni()
    }
    fn layout_text<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        _: &'a oc_schema::TextBody,
        _: f64,
    ) -> RenderFuture<'a, TextMeasure> {
        ni()
    }
    fn encode_png<'a>(&'a self, _: &'a Rgba) -> RenderFuture<'a, Vec<u8>> {
        ni()
    }
    fn transcode_png<'a>(&'a self, _: &'a [u8]) -> RenderFuture<'a, Option<DecodedImage>> {
        ni()
    }
    fn rasterize_svg<'a>(&'a self, _: &'a str, _: u32) -> RenderFuture<'a, Vec<u8>> {
        ni()
    }
    fn path_commands(&self, _: &str) -> RenderResult<Option<Vec<PathCmd>>> {
        Err(RenderError::NotImplemented("fake"))
    }
}

/// Bundled fonts only (no Google catalogue, no network, no database), the test assets (video posters: poster.png).
struct Host {
    manifest: FontManifest,
    dir: PathBuf,
}

impl Host {
    fn new() -> Host {
        Host {
            manifest: common::bundled_manifest(),
            dir: common::bundled_fonts_dir(),
        }
    }
}

impl RenderHost for Host {
    fn asset_bytes<'a>(&'a self, key: &'a str) -> RenderFuture<'a, Vec<u8>> {
        let assets = root().join("crates/oc-render/tests/assets");
        let file = if key.ends_with("/poster") {
            Some("poster.png")
        } else {
            match key.split('/').next() {
                Some("photo") => Some("photo.jpg"),
                Some("tile") => Some("tile.png"),
                _ => None,
            }
        };
        let r = file
            .map(|f| std::fs::read(assets.join(f)).unwrap())
            .ok_or_else(|| RenderError::Failed(format!("asset {key} not found")));
        Box::pin(async move { r })
    }
    fn font_file<'a>(&'a self, file: &'a str) -> RenderFuture<'a, Vec<u8>> {
        let r = common::bundled_font_file(file).map_err(RenderError::Failed);
        Box::pin(async move { r })
    }
    fn custom_families<'a>(&'a self, _: &'a [String]) -> RenderFuture<'a, Vec<FontFamily>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn manifest(&self) -> &FontManifest {
        &self.manifest
    }
    fn fonts_dir(&self) -> PathBuf {
        self.dir.clone()
    }
}

// ------------------------------------------------------------------ the recording spawner

/// The generator's frame signature: 8×6 grid of mean RGB over the pixels at (2 + 4i, 2 + 4j), FNV-1a of those.
fn signature(px: &[u8], w: usize, h: usize) -> (Vec<i64>, u32) {
    const GX: usize = 8;
    const GY: usize = 6;
    let mut sums = vec![0i64; GX * GY * 3];
    let mut counts = vec![0i64; GX * GY];
    let mut crc: u32 = 0x811c9dc5;
    let mut y = 2;
    while y < h {
        let cy = y * GY / h;
        let mut x = 2;
        while x < w {
            let cx = x * GX / w;
            let o = (y * w + x) * 4;
            let c = cy * GX + cx;
            counts[c] += 1;
            for k in 0..3 {
                sums[c * 3 + k] += i64::from(px[o + k]);
                crc = (crc ^ u32::from(px[o + k])).wrapping_mul(0x01000193);
            }
            x += 4;
        }
        y += 4;
    }
    let sig = sums
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let n = counts[i / 3];
            if n == 0 {
                0
            } else {
                oc_schema::js::round(*s as f64 / n as f64) as i64
            }
        })
        .collect();
    (sig, crc)
}

type Frames = Arc<Mutex<Vec<(Vec<i64>, u32)>>>;

/// The encoder's stdin: cuts the stream into frames and records their signatures.
struct SigWriter {
    frame: usize,
    w: usize,
    h: usize,
    buf: Vec<u8>,
    out: Frames,
}

impl AsyncWrite for SigWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let me = &mut *self;
        me.buf.extend_from_slice(data);
        while me.buf.len() >= me.frame {
            let s = signature(&me.buf[..me.frame], me.w, me.h);
            me.out.lock().unwrap().push(s);
            me.buf.drain(..me.frame);
        }
        Poll::Ready(Ok(data.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Logs every command line; decoders run for real, the encoder and the mixer are mocked (like the generator).
struct Recorder {
    argv: Mutex<Vec<Vec<String>>>,
    frames: Frames,
    real: bool,
}

impl Recorder {
    fn new(real: bool) -> Recorder {
        Recorder {
            argv: Mutex::default(),
            frames: Arc::default(),
            real,
        }
    }

    fn take(&self) -> Vec<Vec<String>> {
        std::mem::take(&mut *self.argv.lock().unwrap())
    }
}

static TMP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^.*/oc-(video|audio)-[A-Za-z0-9]{6}").unwrap());

fn norm(s: &str) -> String {
    let media = dir().join("media").to_string_lossy().into_owned();
    TMP_RE.replace(s, "<tmp>").replace(&media, "<media>")
}

impl Spawner for Recorder {
    fn spawn(&self, argv: &[String], pipes: Pipes) -> std::io::Result<Proc> {
        self.argv
            .lock()
            .unwrap()
            .push(argv.iter().map(|a| norm(a)).collect());
        if self.real || pipes == Pipes::Stdout {
            return SystemSpawner.spawn(argv, pipes);
        }
        std::fs::write(argv.last().unwrap(), "mock")?;
        let stdin = if pipes == Pipes::Stdin {
            let s = argv.iter().position(|a| a == "-s").unwrap();
            let (w, h) = argv[s + 1].split_once('x').unwrap();
            let (w, h): (usize, usize) = (w.parse().unwrap(), h.parse().unwrap());
            Some(Box::pin(SigWriter {
                frame: w * h * 4,
                w,
                h,
                buf: Vec::new(),
                out: self.frames.clone(),
            }) as Pin<Box<dyn AsyncWrite + Send>>)
        } else {
            None
        };
        Ok(Proc {
            stdin,
            stdout: None,
            done: Box::pin(async { Ok(()) }),
        })
    }
}

fn argv_json(a: &[Vec<String>]) -> Value {
    json!(a)
}

/// A temporary directory removed on drop (also when an assertion fails).
struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl std::ops::Deref for Tmp {
    type Target = PathBuf;
    fn deref(&self) -> &PathBuf {
        &self.0
    }
}

fn tmp_root(tag: &str) -> Tmp {
    let d = std::env::temp_dir().join(format!("oc-render-svc-test-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d)
}

type ProgressLog = Mutex<Vec<(f64, String)>>;

fn progress_json(p: &ProgressLog) -> Value {
    json!(
        p.lock()
            .unwrap()
            .iter()
            .map(|(k, l)| json!([k, l]))
            .collect::<Vec<_>>()
    )
}

// ------------------------------------------------------------------ GIF export

#[test]
fn gif_export_matches_ts() {
    let g = golden("gif/gif-deck.json");
    let doc = design("gif-deck");
    let fake = Fake::default();
    let host = Host::new();
    let m = media();
    let sp = Recorder::new(false);
    let tmp = tmp_root("gif");
    let log: ProgressLog = Mutex::default();
    let cb = |k: f64, l: &str| log.lock().unwrap().push((k, l.to_string()));
    let bytes = block_on(export_gif(
        VideoDeps {
            renderer: &fake,
            host: &host,
            media: &m,
            spawner: &sp,
            tmp: &tmp,
        },
        &doc,
        Some(&cb),
    ))
    .unwrap();
    assert_renders(
        &json!(*fake.log.lock().unwrap()),
        &g["renders"],
        "gif-deck.renders",
    );
    assert_json(&progress_json(&log), &g["progress"], "gif-deck.progress");
    let want = b64d(g["bytes"].as_str().unwrap());
    if bytes != want {
        let at = bytes
            .iter()
            .zip(&want)
            .position(|(a, b)| a != b)
            .unwrap_or(bytes.len().min(want.len()));
        panic!(
            "gif-deck: GIF bytes differ at {at} (len {} vs {})",
            bytes.len(),
            want.len()
        );
    }
    assert_eq!(sha(&bytes), g["sha256"].as_str().unwrap());
}

// ------------------------------------------------------------------ MP4 plans

/// Composed (page-transition) frames: the 8×6 grid of mean colours may differ by rasterisation only.
const TRANSITION_MEAN_TOL: f64 = 1.0;
const TRANSITION_MAX_TOL: i64 = 8;

/// One plan file: `exportMp4` + `exportAudio` with the encoder and mixer mocked.
fn check_plan(file: &str) {
    if !has_ffmpeg() {
        eprintln!("{file}: ffmpeg not installed, skipped (the video decoders run ffmpeg)");
        return;
    }
    let n = file;
    let g = golden(&format!("plans/{n}"));
    let name = g["name"].as_str().unwrap();
    let doc = design(name);
    let quality = VideoQuality::parse(g["quality"].as_str().unwrap());
    let fps = g["fps"].as_f64();
    let m = media();
    let host = Host::new();
    let fake = Fake::default();
    let sp = Recorder::new(false);
    let tmp = tmp_root(&n.replace('.', "-"));
    let log: ProgressLog = Mutex::default();
    let cb = |k: f64, l: &str| log.lock().unwrap().push((k, l.to_string()));
    let deps = VideoDeps {
        renderer: &fake,
        host: &host,
        media: &m,
        spawner: &sp,
        tmp: &tmp,
    };
    block_on(export_mp4(deps, &doc, quality, fps, Some(&cb)))
        .unwrap_or_else(|e| panic!("{n}: {e}"));
    let mp4 = &g["mp4"];
    assert_json(
        &argv_json(&sp.take()),
        &mp4["argv"],
        &format!("{n}.mp4.argv"),
    );
    assert_renders(
        &json!(*fake.log.lock().unwrap()),
        &mp4["renders"],
        &format!("{n}.mp4.renders"),
    );
    assert_json(
        &progress_json(&log),
        &mp4["progress"],
        &format!("{n}.mp4.progress"),
    );
    // Frames written to the encoder.
    let frames = sp.frames.lock().unwrap().clone();
    assert_eq!(
        frames.len() as u64,
        mp4["frameCount"].as_u64().unwrap(),
        "{n}: frame count"
    );
    let (mut worst, mut composed, mut i) = ((0.0f64, 0i64), 0usize, 0usize);
    for run in mp4["frames"].as_array().unwrap() {
        let want: Vec<i64> = serde_json::from_value(run["sig"].clone()).unwrap();
        let crc = run["crc"].as_u64().unwrap() as u32;
        for _ in 0..run["n"].as_u64().unwrap() {
            let (sig, c) = &frames[i];
            if run["kind"] == "r" {
                assert!(*c == crc && *sig == want, "{n}: rendered frame {i} differs");
            } else {
                let d: Vec<i64> = sig.iter().zip(&want).map(|(a, b)| (a - b).abs()).collect();
                let mean = d.iter().sum::<i64>() as f64 / d.len() as f64;
                let max = *d.iter().max().unwrap();
                assert!(
                    mean <= TRANSITION_MEAN_TOL && max <= TRANSITION_MAX_TOL,
                    "{n}: transition frame {i}: mean |Δ| {mean:.2}, max {max}\n  ours   {sig:?}\n  theirs {want:?}"
                );
                worst = (worst.0.max(mean), worst.1.max(max));
                composed += 1;
            }
            i += 1;
        }
    }
    // Audio.
    let total = timeline_of(&doc, false).total_ms;
    assert_eq!(g["totalMs"].as_f64().unwrap(), total, "{n}: totalMs");
    let mix = |k: usize| {
        audio_mix_args(&doc, &m, total, k).map_or(Value::Null, |x| json!({ "inputs": x.inputs.iter().map(|a| norm(a)).collect::<Vec<_>>(), "filter": norm_all(&x.filter) }))
    };
    assert_json(&mix(1), &g["mix1"], &format!("{n}.mix1"));
    assert_json(&mix(0), &g["mix0"], &format!("{n}.mix0"));
    let audio = block_on(export_audio(deps, &doc)).unwrap();
    assert_eq!(
        audio.is_none(),
        g["m4a"]["silent"].as_bool().unwrap(),
        "{n}: silent"
    );
    assert_json(
        &argv_json(&sp.take()),
        &g["m4a"]["argv"],
        &format!("{n}.m4a.argv"),
    );
    eprintln!(
        "{n}: {} frames, {composed} transition frames, worst mean |Δ| {:.2}, max {}",
        frames.len(),
        worst.0,
        worst.1
    );
}

macro_rules! plan_tests {
    ($($t:ident => $f:literal),+ $(,)?) => {
        $(#[test]
        fn $t() {
            check_plan($f);
        })+

        /// Every plan file has a test.
        #[test]
        fn every_plan_is_checked() {
            let mut names: Vec<String> = std::fs::read_dir(dir().join("plans")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            let mut known = vec![$($f.to_string()),+];
            known.sort();
            assert_eq!(names, known);
        }
    };
}

plan_tests! {
    plan_pitch_720p => "deck-deck-pitch-modern-720p-10fps.json",
    plan_pitch_1080p => "deck-deck-pitch-modern-1080p-10fps.json",
    plan_sales_720p => "deck-vd-43-sales-720p-10fps.json",
    plan_keynote_720p => "deck-vd-deck-keynote-720p-10fps.json",
    plan_video_reel_720p => "video-reel-720p-30fps.json",
}

fn norm_all(s: &str) -> String {
    s.replace(
        &dir().join("media").to_string_lossy().into_owned(),
        "<media>",
    )
}

// ------------------------------------------------------------------ real export

fn probe(file: &Path) -> Value {
    let o = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(file)
        .output()
        .unwrap();
    let j: Value = serde_json::from_slice(&o.stdout).unwrap();
    let num = |v: &Value| {
        v.as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(f64::NAN)
    };
    json!({
        "duration": num(&j["format"]["duration"]),
        "formats": j["format"]["format_name"],
        "streams": j["streams"].as_array().unwrap().iter().map(|s| {
            let mut o = json!({ "type": s["codec_type"], "codec": s["codec_name"], "duration": num(&s["duration"]) });
            if s["codec_type"] == "video" {
                for (k, src) in [("width", "width"), ("height", "height"), ("pixFmt", "pix_fmt"), ("rate", "r_frame_rate")] {
                    o[k] = s[src].clone();
                }
            } else {
                o["sampleRate"] = s["sample_rate"].clone();
                o["channels"] = s["channels"].clone();
            }
            o
        }).collect::<Vec<_>>(),
    })
}

/// Stream layout equal, durations within 50 ms.
fn same_probe(ours: &Value, theirs: &Value, what: &str) {
    let close = |a: &Value, b: &Value| (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() <= 0.05;
    assert!(
        close(&ours["duration"], &theirs["duration"]),
        "{what}: duration {} vs {}",
        ours["duration"],
        theirs["duration"]
    );
    assert_eq!(ours["formats"], theirs["formats"], "{what}: container");
    let (a, b) = (
        ours["streams"].as_array().unwrap(),
        theirs["streams"].as_array().unwrap(),
    );
    assert_eq!(a.len(), b.len(), "{what}: streams {ours} vs {theirs}");
    for (x, y) in a.iter().zip(b) {
        assert!(
            close(&x["duration"], &y["duration"]),
            "{what}: stream duration {x} vs {y}"
        );
        let strip = |v: &Value| {
            let mut v = v.clone();
            v.as_object_mut().unwrap().remove("duration");
            v
        };
        assert_eq!(strip(x), strip(y), "{what}: stream layout");
    }
}

fn grab(file: &Path, t: f64) -> Vec<u8> {
    let o = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &oc_schema::js::num_to_string(t),
            "-i",
        ])
        .arg(file)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=160:90",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .unwrap();
    o.stdout
}

/// Mean CIE76 ΔE between two sRGB rgb24 images.
fn mean_delta_e(a: &[u8], b: &[u8]) -> f64 {
    fn lab(p: &[u8]) -> [f64; 3] {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (lin(p[0]), lin(p[1]), lin(p[2]));
        let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
        let f = |t: f64| {
            if t > 0.008856 {
                t.cbrt()
            } else {
                7.787 * t + 16.0 / 116.0
            }
        };
        [
            116.0 * f(y) - 16.0,
            500.0 * (f(x) - f(y)),
            200.0 * (f(y) - f(z)),
        ]
    }
    let n = a.len().min(b.len()) / 3;
    let mut sum = 0.0;
    for i in 0..n {
        let (p, q) = (lab(&a[i * 3..i * 3 + 3]), lab(&b[i * 3..i * 3 + 3]));
        sum += ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
    }
    sum / n.max(1) as f64
}

/// Decoded frames of the real export: H.264 at CRF 19 over two different rasterisers and libx264 versions
/// (ffmpeg 6.1 on CI vs 8.1 on Homebrew measured a mean ΔE of 1.06; tools/golden/compare.mjs uses the same).
const REAL_FRAME_TOL: f64 = 2.0;

#[test]
fn real_export_matches_ts() {
    if !has_ffmpeg() {
        eprintln!("real_export_matches_ts: ffmpeg not installed, skipped");
        return;
    }
    let g = golden("real.json");
    let doc = design(g["name"].as_str().unwrap());
    let host = Host::new();
    let r = OcRenderer::new(&host);
    let m = media();
    let sp = Recorder::new(true);
    let tmp = tmp_root("real");
    let deps = VideoDeps {
        renderer: &r,
        host: &host,
        media: &m,
        spawner: &sp,
        tmp: &tmp,
    };
    let mp4 = block_on(export_mp4(
        deps,
        &doc,
        VideoQuality::parse(g["quality"].as_str().unwrap()),
        None,
        None,
    ))
    .unwrap();
    assert_json(&argv_json(&sp.take()), &g["argv"], "real.argv");
    let m4a = block_on(export_audio(deps, &doc)).unwrap().expect("audio");
    assert_json(&argv_json(&sp.take()), &g["m4a"]["argv"], "real.m4a.argv");
    let (fv, fa) = (tmp.join("real.mp4"), tmp.join("real.m4a"));
    std::fs::write(&fv, &mp4).unwrap();
    std::fs::write(&fa, &m4a).unwrap();
    same_probe(&probe(&fv), &g["mp4"]["probe"], "mp4");
    same_probe(&probe(&fa), &g["m4a"]["probe"], "m4a");
    let mut report = Vec::new();
    for f in g["mp4"]["frames"].as_array().unwrap() {
        let t = f["t"].as_f64().unwrap();
        let ours = grab(&fv, t);
        let theirs = b64d(f["rgb"].as_str().unwrap());
        assert_eq!(ours.len(), theirs.len(), "frame at {t}s");
        let de = mean_delta_e(&ours, &theirs);
        report.push(format!("{t}s ΔE {de:.2}"));
        assert!(
            de <= REAL_FRAME_TOL,
            "frame at {t}s: mean ΔE {de:.2} > {REAL_FRAME_TOL}"
        );
    }
    eprintln!("real export: {}", report.join(", "));
}

#[test]
fn quality_scales() {
    let doc = design("video-reel");
    assert_eq!(
        oc_render_svc::video::video_scale(&doc, VideoQuality::P720),
        1280.0 / 641.0
    );
    assert_eq!(
        oc_render_svc::video::video_scale(&doc, VideoQuality::K4),
        3840.0 / 641.0
    );
    assert_eq!(VideoQuality::parse("bogus"), VideoQuality::P1080);
}
