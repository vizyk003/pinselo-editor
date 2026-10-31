// SPDX-License-Identifier: Apache-2.0
//! Server-side media for exports — port of `services/render/src/media.ts` and of the ffmpeg runner in
//! `exporters.ts`: sequential video frame decoding (ffmpeg → raw RGBA) and the audio mix (soundtrack clips + video
//! audio, with trims, speed, volume, fades and offsets) as ffmpeg arguments.
//!
//! Processes are started through a [`Spawner`], so the exporters can be driven without ffmpeg (the golden tests
//! record the command lines the way the TS generator records `spawn`); [`SystemSpawner`] runs the real binary.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use futures::future::BoxFuture;
use oc_render::jsfmt::{max, min};
use oc_schema::DesignDoc;
use oc_schema::elements::Element;
use oc_schema::factory::media_clip_length_ms;
use oc_schema::js::{num_to_string, round, to_fixed};
use oc_schema::timeline::{element_window, page_duration_ms, timeline_of, video_slots};
use oc_schema::tree::descendants;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

use crate::renderer::Rgba;

/// A media asset's playback proxy (H.264/AAC, or VP9 WebM with alpha), as `mediaFor` reads it from `assets`.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    /// Local file of the proxy.
    pub file: String,
    pub w: u32,
    pub h: u32,
    pub has_audio: bool,
    pub duration_ms: f64,
    /// VP9 WebM with alpha (transparent GIF stickers): libvpx is needed to decode the alpha plane.
    pub alpha: bool,
}

/// `MediaLookup`: media info by asset id.
pub type Media = HashMap<String, MediaInfo>;

/// Which pipes a process gets (the rest is ignored; stderr is always captured for the error message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pipes {
    /// `stdio: ['ignore', 'ignore', 'pipe']` — `ffmpeg(args)`.
    None,
    /// `stdio: ['pipe', 'ignore', 'pipe']` — `ffmpeg(args, true)` (the encoder).
    Stdin,
    /// `stdio: ['ignore', 'pipe', 'ignore']` — the video decoder.
    Stdout,
}

/// A started process. Dropping it kills the process (the decoder's `close()`).
pub struct Proc {
    pub stdin: Option<Pin<Box<dyn AsyncWrite + Send>>>,
    pub stdout: Option<Pin<Box<dyn AsyncRead + Send>>>,
    /// Resolves when the process exits: `Ok` for status 0, else the tail of its stderr.
    pub done: BoxFuture<'static, Result<(), String>>,
}

/// Starts processes. `argv[0]` is the program (`ffmpeg`).
pub trait Spawner: Send + Sync {
    fn spawn(&self, argv: &[String], pipes: Pipes) -> std::io::Result<Proc>;
}

/// Runs the real programs (`tokio::process`, killed on drop).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemSpawner;

impl Spawner for SystemSpawner {
    fn spawn(&self, argv: &[String], pipes: Pipes) -> std::io::Result<Proc> {
        use std::process::Stdio;
        let mut cmd = tokio::process::Command::new(&argv[0]);
        cmd.args(&argv[1..]).kill_on_drop(true);
        cmd.stdin(if pipes == Pipes::Stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(if pipes == Pipes::Stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stderr(if pipes == Pipes::Stdout {
            Stdio::null()
        } else {
            Stdio::piped()
        });
        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .map(|s| Box::pin(s) as Pin<Box<dyn AsyncWrite + Send>>);
        let stdout = child
            .stdout
            .take()
            .map(|s| Box::pin(s) as Pin<Box<dyn AsyncRead + Send>>);
        // stderr is drained while the process runs (a full pipe would stall it), like the TS `on('data')`.
        let stderr = child.stderr.take().map(|mut e| {
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let _ = e.read_to_end(&mut buf).await;
                String::from_utf8_lossy(&buf).into_owned()
            })
        });
        let done = Box::pin(async move {
            let status = child.wait().await;
            let err = match stderr {
                Some(t) => t.await.unwrap_or_default(),
                None => String::new(),
            };
            match status {
                Ok(s) if s.success() => Ok(()),
                Ok(_) => Err(err),
                Err(e) => Err(e.to_string()),
            }
        });
        Ok(Proc {
            stdin,
            stdout,
            done,
        })
    }
}

/// `err.slice(-800)` (UTF-16 code units, as JS slices).
fn tail_800(s: &str) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    String::from_utf16_lossy(&units[units.len().saturating_sub(800)..])
}

/// The error `ffmpeg(...)` rejects with.
pub fn ffmpeg_failed(stderr: &str) -> String {
    format!("ffmpeg failed: {}", tail_800(stderr))
}

/// `ffmpeg(args, stdin)`: `ffmpeg -hide_banner -loglevel error -y …args`.
pub fn ffmpeg_argv(args: &[String]) -> Vec<String> {
    ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y"]
        .iter()
        .map(|s| s.to_string())
        .chain(args.iter().cloned())
        .collect()
}

/// Run `ffmpeg …args` to completion (no stdin).
pub async fn run_ffmpeg(sp: &dyn Spawner, args: &[String]) -> Result<(), String> {
    let p = sp
        .spawn(&ffmpeg_argv(args), Pipes::None)
        .map_err(|e| ffmpeg_failed(&e.to_string()))?;
    p.done.await.map_err(|e| ffmpeg_failed(&e))
}

/// `VideoDecoder`: decodes one clip window sequentially; frames are requested with non-decreasing source times.
pub struct VideoDecoder {
    proc: Option<Proc>,
    ended: bool,
    index: i64,
    frame: Option<Arc<Rgba>>,
    file: String,
    pub w: u32,
    pub h: u32,
    start_ms: f64,
    fps: f64,
    alpha: bool,
    frame_bytes: usize,
    step_ms: f64,
}

impl VideoDecoder {
    pub fn new(file: &str, w: u32, h: u32, start_ms: f64, fps: f64, alpha: bool) -> VideoDecoder {
        VideoDecoder {
            proc: None,
            ended: false,
            index: -1,
            frame: None,
            file: file.to_string(),
            w,
            h,
            start_ms,
            fps,
            alpha,
            frame_bytes: w as usize * h as usize * 4,
            step_ms: 1000.0 / fps,
        }
    }

    /// The decoder's ffmpeg command line.
    pub fn argv(&self) -> Vec<String> {
        let mut a: Vec<String> = ["ffmpeg", "-hide_banner", "-loglevel", "error"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        if self.alpha {
            a.extend(["-c:v".to_string(), "libvpx-vp9".to_string()]);
        }
        a.extend([
            "-ss".to_string(),
            num_to_string(self.start_ms / 1000.0),
            "-i".to_string(),
            self.file.clone(),
            "-vf".to_string(),
            format!(
                "fps={},scale={}:{}",
                num_to_string(self.fps),
                self.w,
                self.h
            ),
            "-f".to_string(),
            "rawvideo".to_string(),
            "-pix_fmt".to_string(),
            "rgba".to_string(),
            "-".to_string(),
        ]);
        a
    }

    async fn next(&mut self, sp: &dyn Spawner) -> Option<Vec<u8>> {
        if self.proc.is_none() && !self.ended {
            match sp.spawn(&self.argv(), Pipes::Stdout) {
                Ok(p) => self.proc = Some(p),
                // A process that cannot start ends at once (the TS sees `close` with no data).
                Err(_) => self.ended = true,
            }
        }
        if self.ended || self.frame_bytes == 0 {
            return None;
        }
        let out = self.proc.as_mut()?.stdout.as_mut()?;
        let mut buf = vec![0u8; self.frame_bytes];
        match out.read_exact(&mut buf).await {
            Ok(_) => Some(buf),
            Err(_) => {
                // EOF (a partial last frame is dropped, as the TS does).
                self.ended = true;
                None
            }
        }
    }

    /// Advance to the frame covering `source_time_ms` (`seekForward`).
    pub async fn seek_forward(&mut self, sp: &dyn Spawner, source_time_ms: f64) {
        let want = 0f64.max(((source_time_ms - self.start_ms) / self.step_ms + 1e-6).floor());
        while (self.index as f64) < want {
            let Some(f) = self.next(sp).await else { break };
            self.frame = Some(Arc::new(Rgba {
                width: self.w,
                height: self.h,
                pixels: f,
            }));
            self.index += 1;
        }
    }

    /// The current frame (unpremultiplied RGBA, `w × h`), if any was decoded (`image`).
    pub fn image(&self) -> Option<&Arc<Rgba>> {
        self.frame.as_ref()
    }

    /// `position`: source time of the current frame.
    pub fn position(&self) -> f64 {
        self.start_ms + self.index as f64 * self.step_ms
    }

    /// `close()`: kill the process.
    pub fn close(&mut self) {
        self.proc = None;
        self.frame = None;
    }
}

/// The ffmpeg audio mix: inputs to add and the `-filter_complex` graph ending in `[aout]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMix {
    pub inputs: Vec<String>,
    pub filter: String,
}

fn visible(el: &Element) -> bool {
    match el {
        Element::Unknown(m) => m.get("visible") != Some(&serde_json::Value::Bool(false)),
        _ => el.base().is_none_or(|b| b.visible != Some(false)),
    }
}

/// `audioMixArgs(doc, media, totalMs, firstInputIndex)`: `None` when the design is silent.
pub fn audio_mix_args(
    doc: &DesignDoc,
    media: &Media,
    total_ms: f64,
    first_input_index: usize,
) -> Option<AudioMix> {
    let mut inputs: Vec<String> = Vec::new();
    let mut chains: Vec<String> = Vec::new();
    #[allow(clippy::too_many_arguments)]
    let mut add = |file: &str,
                   start_design_ms: f64,
                   trim_start_ms: f64,
                   source_len_ms: f64,
                   speed: f64,
                   volume: f64,
                   fade_in: f64,
                   fade_out: f64| {
        if source_len_ms <= 30.0 || start_design_ms >= total_ms {
            return;
        }
        let idx = first_input_index + inputs.len() / 6;
        inputs.extend([
            "-ss".to_string(),
            to_fixed(trim_start_ms / 1000.0, 3),
            "-t".to_string(),
            to_fixed(source_len_ms / 1000.0, 3),
            "-i".to_string(),
            file.to_string(),
        ]);
        let play_len = source_len_ms / speed;
        let mut tempo: Vec<String> = Vec::new();
        let mut sp = speed;
        while sp > 2.0 {
            tempo.push("atempo=2".into());
            sp /= 2.0;
        }
        while sp < 0.5 {
            tempo.push("atempo=0.5".into());
            sp /= 0.5;
        }
        if (sp - 1.0).abs() > 1e-3 {
            tempo.push(format!("atempo={}", to_fixed(sp, 4)));
        }
        let mut f: Vec<String> = vec![
            "aresample=48000".into(),
            "aformat=channel_layouts=stereo".into(),
        ];
        f.extend(tempo);
        f.push(format!("volume={}", to_fixed(volume, 3)));
        if fade_in > 0.0 {
            f.push(format!(
                "afade=t=in:st=0:d={}",
                to_fixed(fade_in / 1000.0, 3)
            ));
        }
        if fade_out > 0.0 {
            f.push(format!(
                "afade=t=out:st={}:d={}",
                to_fixed(max(0.0, (play_len - fade_out) / 1000.0), 3),
                to_fixed(fade_out / 1000.0, 3)
            ));
        }
        let d = num_to_string(round(start_design_ms));
        f.push(format!("adelay={d}|{d}"));
        chains.push(format!("[{idx}:a]{}[a{}]", f.join(","), chains.len()));
    };
    for c in doc.meta.audio.iter().flatten() {
        let Some(m) = media.get(c.asset.as_str()) else {
            continue;
        };
        let len = min(c.duration_ms, m.duration_ms - c.trim_start_ms);
        add(
            &m.file,
            c.start_ms,
            c.trim_start_ms,
            len,
            1.0,
            c.volume,
            c.fade_in_ms,
            c.fade_out_ms,
        );
    }
    for span in timeline_of(doc, false).spans {
        let page_ms = page_duration_ms(&doc.elements, span.page);
        for el in descendants(&doc.elements, &span.page.id) {
            if !visible(el) {
                continue;
            }
            for v in video_slots(el) {
                if v.clip.muted {
                    continue;
                }
                let Some(m) = media.get(v.asset) else {
                    continue;
                };
                if !m.has_audio {
                    continue;
                }
                let w = element_window(el, page_ms);
                let one = media_clip_length_ms(v.clip);
                let vis = min(
                    w.end - w.start,
                    if v.clip.loop_ { f64::INFINITY } else { one },
                );
                // Loops are expanded into repeated inputs.
                let mut off = 0.0;
                while off < vis - 30.0 {
                    let play_ms = min(one, vis - off);
                    add(
                        &m.file,
                        span.start_ms + w.start + off,
                        v.clip.trim_start_ms,
                        play_ms * v.clip.speed,
                        v.clip.speed,
                        v.clip.volume,
                        0.0,
                        min(60.0, play_ms / 4.0),
                    );
                    // A zero-length loop would never advance (the TS spins forever).
                    if !v.clip.loop_ || one <= 0.0 {
                        break;
                    }
                    off += one;
                }
            }
        }
    }
    if chains.is_empty() {
        return None;
    }
    let labels: String = (0..chains.len()).map(|i| format!("[a{i}]")).collect();
    let filter = format!(
        "{};{labels}amix=inputs={}:normalize=0:dropout_transition=0,apad,atrim=0:{},alimiter=limit=0.95[aout]",
        chains.join(";"),
        chains.len(),
        to_fixed(total_ms / 1000.0, 3)
    );
    Some(AudioMix { inputs, filter })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_prefix() {
        assert_eq!(
            ffmpeg_argv(&["-i".into(), "x".into()]).join(" "),
            "ffmpeg -hide_banner -loglevel error -y -i x"
        );
        assert_eq!(
            ffmpeg_failed(&"é".repeat(900)).chars().count(),
            "ffmpeg failed: ".len() + 800
        );
    }

    #[tokio::test]
    async fn ffmpeg_errors_carry_stderr() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            return;
        }
        let args: Vec<String> = ["-i", "/nonexistent/oc-missing.mp4", "-f", "null", "-"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let e = run_ffmpeg(&SystemSpawner, &args).await.unwrap_err();
        assert!(
            e.starts_with("ffmpeg failed: ") && e.contains("oc-missing.mp4"),
            "{e}"
        );
    }

    #[test]
    fn decoder_args() {
        let d = VideoDecoder::new("/m/clip.mp4", 64, 36, 1500.0, 30.0, true);
        assert_eq!(
            d.argv().join(" "),
            "ffmpeg -hide_banner -loglevel error -c:v libvpx-vp9 -ss 1.5 -i /m/clip.mp4 -vf fps=30,scale=64:36 -f rawvideo -pix_fmt rgba -"
        );
        assert_eq!(d.position(), 1500.0 - 1000.0 / 30.0);
    }
}
