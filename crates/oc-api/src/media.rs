//! Port of services/api/src/media.ts — media ingest (MV-020/MV-030): ffprobe + ffmpeg in a separate process
//! (no in-process decoders). Every upload gets a normalized playback proxy, a poster frame and waveform peaks;
//! the perceptual hash of the upload hash gate.

use std::path::Path;
use std::process::Stdio;

use serde_json::Value;
use tokio::io::AsyncReadExt as _;

#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    /// `'video' | 'audio'`
    pub kind: &'static str,
    pub duration_ms: i64,
    pub width: i64,
    pub height: i64,
    pub has_audio: bool,
    pub rotation: f64,
}

/// `run(cmd, args, { stdout })`: stdout bytes, or the error message (`<cmd> failed (<code>): <stderr tail>`).
pub async fn run(cmd: &str, args: &[&str], capture_stdout: bool) -> Result<Vec<u8>, String> {
    let mut c = tokio::process::Command::new(cmd);
    c.args(args).stdin(Stdio::null()).stdout(if capture_stdout { Stdio::piped() } else { Stdio::null() }).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = c.spawn().map_err(|e| format!("spawn {cmd}: {e}"))?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let read_out = async {
        if let Some(s) = so.as_mut() {
            let _ = s.read_to_end(&mut out).await;
        }
    };
    let read_err = async {
        if let Some(s) = se.as_mut() {
            let _ = s.read_to_end(&mut err).await;
        }
    };
    tokio::join!(read_out, read_err);
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if status.success() {
        Ok(out)
    } else {
        let e = String::from_utf8_lossy(&err);
        let tail: String = e.chars().rev().take(500).collect::<Vec<_>>().into_iter().rev().collect();
        Err(format!("{cmd} failed ({}): {tail}", status.code().map_or("null".into(), |c| c.to_string())))
    }
}

fn num(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() { 0.0 } else { t.parse().unwrap_or(f64::NAN) }
        }
        Some(Value::Null) | None => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        _ => f64::NAN,
    }
}

/// `probe(file)`.
pub async fn probe(file: &Path) -> Option<Probe> {
    let f = file.to_string_lossy();
    let out = run("ffprobe", &["-v", "error", "-print_format", "json", "-show_streams", "-show_format", &f], true).await.ok()?;
    let json: Value = serde_json::from_slice(&out).ok()?;
    let streams = json.get("streams").and_then(Value::as_array).cloned().unwrap_or_default();
    let attached = |s: &Value| s.get("disposition").and_then(|d| d.get("attached_pic")).is_some_and(|v| num(Some(v)) != 0.0);
    let v = streams.iter().find(|s| s.get("codec_type").and_then(Value::as_str) == Some("video") && !attached(s));
    let a = streams.iter().find(|s| s.get("codec_type").and_then(Value::as_str) == Some("audio"));
    // Number(json.format?.duration ?? v?.duration ?? a?.duration ?? 0)
    let dur = json
        .get("format")
        .and_then(|f| f.get("duration"))
        .filter(|d| !d.is_null())
        .or_else(|| v.and_then(|v| v.get("duration")).filter(|d| !d.is_null()))
        .or_else(|| a.and_then(|a| a.get("duration")).filter(|d| !d.is_null()));
    let duration_ms = crate::js::round(num(dur) * 1000.0);
    if v.is_none() && a.is_none() {
        return None;
    }
    if duration_ms.is_nan() || duration_ms <= 0.0 {
        return None;
    }
    let side_rot = v
        .and_then(|v| v.get("side_data_list"))
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|d| d.get("rotation").is_some()))
        .map(|d| num(d.get("rotation")))
        .or_else(|| v.and_then(|v| v.get("tags")).and_then(|t| t.get("rotate")).map(|r| num(Some(r))))
        .unwrap_or(0.0);
    let rotated = (side_rot.abs() % 180.0) == 90.0;
    let dim = |k: &str| v.map_or(0.0, |v| num(v.get(k)));
    Some(Probe {
        kind: if v.is_some() { "video" } else { "audio" },
        duration_ms: duration_ms as i64,
        width: if v.is_some() { (if rotated { dim("height") } else { dim("width") }) as i64 } else { 0 },
        height: if v.is_some() { (if rotated { dim("width") } else { dim("height") }) as i64 } else { 0 },
        has_audio: a.is_some(),
        rotation: side_rot,
    })
}

/// `firstFrameHasAlpha(file)`.
pub async fn first_frame_has_alpha(file: &Path) -> bool {
    let f = file.to_string_lossy();
    match run("ffmpeg", &["-hide_banner", "-loglevel", "error", "-i", &f, "-frames:v", "1", "-vf", "scale=64:64:flags=neighbor", "-f", "rawvideo", "-pix_fmt", "rgba", "-"], true).await {
        Ok(px) => px.iter().skip(3).step_by(4).any(|&a| a < 250),
        Err(_) => false,
    }
}

/// `makeAlphaProxy(src, dst)`: transparent animated GIF → VP9 WebM with alpha.
pub async fn make_alpha_proxy(src: &Path, dst: &Path) -> Result<(), String> {
    let (s, d) = (src.to_string_lossy(), dst.to_string_lossy());
    run(
        "ffmpeg",
        &[
            "-hide_banner", "-loglevel", "error", "-y", "-i", &s, "-vf", "scale='min(1280,iw)':-2:flags=lanczos,format=yuva420p", "-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "0",
            "-crf", "32", "-auto-alt-ref", "0", "-an", &d,
        ],
        true,
    )
    .await
    .map(|_| ())
}

/// `makeProxy(src, dst, p)`: H.264 yuv420p ≤ 1920 on the long side, 30 fps cap, AAC 160k (or AAC M4A).
pub async fn make_proxy(src: &Path, dst: &Path, p: &Probe) -> Result<(), String> {
    let (s, d) = (src.to_string_lossy().into_owned(), dst.to_string_lossy().into_owned());
    if p.kind == "video" {
        let mut args: Vec<&str> = vec![
            "-hide_banner", "-loglevel", "error", "-y", "-i", &s, "-vf",
            "scale='if(gt(iw,ih),min(1920,iw),-2)':'if(gt(iw,ih),-2,min(1920,ih))',fps='min(30,source_fps)',format=yuv420p", "-c:v", "libx264", "-preset", "veryfast", "-crf", "21", "-g",
            "30",
        ];
        if p.has_audio {
            args.extend(["-c:a", "aac", "-b:a", "160k", "-ac", "2"]);
        } else {
            args.push("-an");
        }
        args.extend(["-movflags", "+faststart", &d]);
        run("ffmpeg", &args, false).await.map(|_| ())
    } else {
        run("ffmpeg", &["-hide_banner", "-loglevel", "error", "-y", "-i", &s, "-vn", "-c:a", "aac", "-b:a", "192k", "-ac", "2", "-movflags", "+faststart", &d], false).await.map(|_| ())
    }
}

/// `makePoster(src, dst, durationMs)`.
pub async fn make_poster(src: &Path, dst: &Path, duration_ms: i64) -> Result<(), String> {
    let at = (500f64).min(duration_ms as f64 / 2.0) / 1000.0;
    let at = crate::js::num(at);
    let (s, d) = (src.to_string_lossy(), dst.to_string_lossy());
    run("ffmpeg", &["-hide_banner", "-loglevel", "error", "-y", "-ss", &at, "-i", &s, "-frames:v", "1", "-q:v", "3", &d], false).await.map(|_| ())
}

/// `+(m / 32768).toFixed(3)` for an integer peak `m` (JS rounds a tie up).
fn peak(m: i64) -> f64 {
    let n = (m * 1000 * 2 + 32768) / (2 * 32768);
    format!("{}.{:03}", n / 1000, n % 1000).parse().unwrap_or(0.0)
}

/// `waveform(src, buckets = 600)`: peak envelope (0..1) from mono 8 kHz PCM.
pub async fn waveform(src: &Path, buckets: usize) -> Result<Vec<f64>, String> {
    let s = src.to_string_lossy();
    let pcm = run("ffmpeg", &["-hide_banner", "-loglevel", "error", "-i", &s, "-vn", "-ac", "1", "-ar", "8000", "-f", "s16le", "-"], true).await?;
    let n = pcm.len() >> 1;
    if n == 0 {
        return Ok(vec![]);
    }
    let per = (n / buckets).max(1);
    let sample = |i: usize| i64::from(i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]));
    let mut out = Vec::new();
    let mut b = 0;
    while b * per < n && out.len() < buckets {
        let mut m = 0i64;
        for i in b * per..n.min((b + 1) * per) {
            m = m.max(sample(i).abs());
        }
        out.push(peak(m));
        b += 1;
    }
    Ok(out)
}

/// `perceptualHash(file)`: 64-bit difference hash (dHash) of an image or video frame, 16 hex digits.
pub async fn perceptual_hash(file: &Path) -> Option<String> {
    let f = file.to_string_lossy();
    let px = run("ffmpeg", &["-hide_banner", "-loglevel", "error", "-i", &f, "-frames:v", "1", "-vf", "scale=9:8:flags=area,format=gray", "-f", "rawvideo", "-"], true).await.ok()?;
    if px.len() < 72 {
        return None;
    }
    let mut bits: u64 = 0;
    for y in 0..8 {
        for x in 0..8 {
            bits = (bits << 1) | u64::from(px[y * 9 + x] > px[y * 9 + x + 1]);
        }
    }
    Some(format!("{bits:016x}"))
}

/// `hamming(a, b)` of two hex hashes.
pub fn hamming(a: &str, b: &str) -> u32 {
    let p = |s: &str| u128::from_str_radix(s, 16).unwrap_or(0);
    (p(a) ^ p(b)).count_ones()
}
