//! Port of services/api/src/routes/assets.ts — the upload pipeline (E10): sniff the type from magic bytes
//! (never trust the client), enforce size + quota, content-address by sha256, rasterise SVG with resvg (no
//! network, no scripts; ADR-0034), media proxies via ffmpeg (crate::media), and asset serving.
//!
//! Also exported for other modules: [`sniff`], [`gif_info`], [`ingest_file`], [`image_size`], [`rasterize_svg`].
//!
//! Known engine difference: the PNG that SVG uploads and recolours are rasterised to comes from Rust resvg
//! 0.48 here and from `@resvg/resvg-js` 2.6.2 in TS; dimensions agree, encoded bytes (hence sha256/size) may
//! not. The parity transcripts mark those fields volatile.

use std::path::{Path as FsPath, PathBuf};
use std::sync::LazyLock;

use axum::extract::Request;
use axum::middleware::{Next, from_fn};
use axum::response::Response;
use regex::Regex;
use serde_json::Map;

use crate::billing;
use crate::hash_match::{self, HashVerdict};
use crate::http::multipart;
use crate::js;
use crate::media;
use crate::prelude::*;
use crate::uploads;
use crate::routes::safety;

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/assets/{id}/poster", get(poster))
        .route("/api/assets/{id}/svg", get(svg_source))
        .route("/api/assets/{id}/recolor/{pairs}", get(recolor))
        .route("/api/assets/{id}/waveform", get(waveform))
        .route("/api/assets/{id}", get(serve).delete(trash).patch(rename))
        .route("/api/assets/{id}/download", get(download))
        .route("/api/assets/{id}/restore", post(restore))
        .route("/api/assets/trash", get(trash_list))
        .route("/api/assets/batch", post(batch))
        .route("/api/assets", get(list).post(upload))
        .route_layer(from_fn(cors))
}

static CORS_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/api/assets/[^/]+(/poster|/waveform|/svg|/recolor/[^/]+)?$").expect("regex"));

/// The plugin's `onSend` hook: asset bytes may be served from the separate user-content origin; the app origin
/// reads them with CORS.
async fn cors(req: Request, next: Next) -> Response {
    let hit = req.method() == axum::http::Method::GET && CORS_PATH.is_match(req.uri().path()) && req.uri().path() != "/api/assets/trash";
    let origin = req.extensions().get::<AppState>().map(|s| s.cfg.web_origin.clone()).unwrap_or_default();
    let mut res = next.run(req).await;
    if hit {
        let h = res.headers_mut();
        if let Ok(v) = axum::http::HeaderValue::from_str(&origin) {
            h.insert("access-control-allow-origin", v);
        }
        h.insert("cross-origin-resource-policy", axum::http::HeaderValue::from_static("cross-origin"));
        h.insert("vary", axum::http::HeaderValue::from_static("origin"));
    }
    res
}

// ---------------------------------------------------------------- sniffing

/// `sniff(buf)`: `(mime, kind)` from magic bytes.
pub fn sniff(buf: &[u8]) -> Option<(&'static str, &'static str)> {
    if buf.len() < 12 {
        return None;
    }
    let ascii = |a: usize, b: usize| buf.get(a..b.min(buf.len())).map(|s| s.iter().map(|&c| if c < 0x80 { c as char } else { '\u{fffd}' }).collect::<String>()).unwrap_or_default();
    if buf[0] == 0x89 && ascii(1, 4) == "PNG" {
        return Some(("image/png", "image"));
    }
    if buf[0] == 0xff && buf[1] == 0xd8 && buf[2] == 0xff {
        return Some(("image/jpeg", "image"));
    }
    if ascii(0, 4) == "RIFF" && ascii(8, 12) == "WEBP" {
        return Some(("image/webp", "image"));
    }
    if ascii(0, 3) == "GIF" {
        return Some(("image/gif", "image"));
    }
    // Media containers (the real check is ffprobe; this only routes the upload).
    if ascii(4, 8) == "ftyp" {
        let brand = ascii(8, 12);
        static BRANDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(M4A |M4B |mp42?|isom|iso\d|avc1|qt  |M4V |3gp|dash|mmp4)").expect("regex"));
        if BRANDS.is_match(&brand) {
            let audio = brand.starts_with("M4A");
            return Some(if audio { ("audio/mp4", "audio") } else { ("video/mp4", "video") });
        }
    }
    if buf[0] == 0x1a && buf[1] == 0x45 && buf[2] == 0xdf && buf[3] == 0xa3 {
        return Some(("video/webm", "video"));
    }
    if ascii(0, 3) == "ID3" || (buf[0] == 0xff && (buf[1] & 0xe0) == 0xe0) {
        return Some(("audio/mpeg", "audio"));
    }
    if ascii(0, 4) == "RIFF" && ascii(8, 12) == "WAVE" {
        return Some(("audio/wav", "audio"));
    }
    if ascii(0, 4) == "OggS" {
        return Some(("audio/ogg", "audio"));
    }
    if ascii(0, 4) == "fLaC" {
        return Some(("audio/flac", "audio"));
    }
    if ascii(4, 12) == "ftypqt  " || ascii(4, 8) == "moov" || ascii(4, 8) == "wide" || ascii(4, 8) == "mdat" {
        return Some(("video/quicktime", "video"));
    }
    let head = String::from_utf8_lossy(&buf[..buf.len().min(1024)]);
    let head = head.trim_start_matches(js::is_ws).to_lowercase();
    if head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg")) {
        return Some(("image/svg+xml", "svg"));
    }
    None
}

/// `gifInfo(buf)`: frame count and transparency (Graphic Control Extensions).
pub fn gif_info(buf: &[u8]) -> (usize, bool) {
    let mut frames = 0;
    let mut transparent = false;
    let mut i = 13;
    while i + 4 < buf.len() {
        // 0x21 0xF9 0x04 <packed> — Graphic Control Extension; bit 0 of <packed> = transparent colour.
        if buf[i] == 0x21 && buf[i + 1] == 0xf9 && buf[i + 2] == 0x04 {
            frames += 1;
            if buf[i + 3] & 1 != 0 {
                transparent = true;
            }
            i += 6;
        }
        i += 1;
    }
    (frames, transparent)
}

/// `imageSize(buf)` of image-size 2.0.4 for the formats an upload can be (PNG, JPEG, GIF, WebP); `None` where it
/// throws.
pub fn image_size(buf: &[u8]) -> Option<(u32, u32)> {
    let u16le = |i: usize| buf.get(i..i + 2).map(|s| u32::from(u16::from_le_bytes([s[0], s[1]])));
    let ascii = |a: usize, b: usize| buf.get(a..b).map(|s| String::from_utf8_lossy(s).into_owned()).unwrap_or_default();
    match buf.first() {
        Some(0x89) | Some(0xff) => oc_social::imagesize::image_size(buf),
        Some(0x47) if ascii(0, 6) == "GIF87a" || ascii(0, 6) == "GIF89a" => Some((u16le(6)?, u16le(8)?)),
        Some(0x52) if ascii(0, 4) == "RIFF" && ascii(8, 12) == "WEBP" && ascii(12, 15) == "VP8" => {
            let chunk = ascii(12, 16);
            let input = buf.get(20..30)?;
            let u24 = |i: usize| u32::from(input[i]) | (u32::from(input[i + 1]) << 8) | (u32::from(input[i + 2]) << 16);
            if chunk == "VP8X" {
                let ext = input[0];
                if ext & 0xc0 == 0 && ext & 0x01 == 0 {
                    return Some((1 + u24(4), 1 + u24(7)));
                }
                return None;
            }
            if chunk == "VP8 " && input[0] != 0x2f {
                let i16le = |i: usize| i16::from_le_bytes([input[i], input[i + 1]]) as i32;
                return Some(((i16le(6) & 0x3fff) as u32, (i16le(8) & 0x3fff) as u32));
            }
            let sig = hex::encode(&input[3..6]);
            if chunk == "VP8L" && sig != "9d012a" {
                let w = 1 + (((u32::from(input[2]) & 0x3f) << 8) | u32::from(input[1]));
                let h = 1 + (((u32::from(input[4]) & 0xf) << 10) | (u32::from(input[3]) << 2) | ((u32::from(input[2]) & 0xc0) >> 6));
                return Some((w, h));
            }
            None
        }
        _ => None,
    }
}

/// How `@resvg/resvg-js` sizes its output (`fitTo`).
pub enum FitTo {
    Original,
    Zoom(f32),
    Width(u32),
}

/// `new Resvg(svg, { fitTo, font: { loadSystemFonts: false } })`: the intrinsic size (`.width`, `.height`).
pub fn svg_size(svg: &str) -> Result<(f32, f32), String> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
    Ok((tree.size().width(), tree.size().height()))
}

/// `new Resvg(svg, opts).render().asPng()`.
pub fn rasterize_svg(svg: &str, fit: FitTo) -> Result<Vec<u8>, String> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
    let base = tree.size().to_int_size();
    let size = match fit {
        FitTo::Original => Some(base),
        FitTo::Zoom(z) => base.scale_by(z),
        FitTo::Width(w) => base.scale_to_width(w),
    }
    .ok_or("bad size")?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("bad size")?;
    let ts = resvg::tiny_skia::Transform::from_scale(size.width() as f32 / tree.size().width(), size.height() as f32 / tree.size().height());
    resvg::render(&tree, ts, &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- ingest

pub struct IngestOpts<'a> {
    pub name: &'a str,
    pub owner_id: Uuid,
    pub ip: Option<&'a str>,
    /// `'upload' | 'emoji' | 'stock' | 'gif'`
    pub source: Option<&'a str>,
    pub attribution: Option<Value>,
    pub skip_quota: bool,
}

/// `IngestResult`: the asset, or `{ error, status, message? }`.
pub enum Ingest {
    Asset(Value),
    Error { status: u16, error: &'static str, message: Option<String> },
}

fn err(status: u16, error: &'static str, message: Option<String>) -> Result<Ingest, ApiError> {
    Ok(Ingest::Error { status, error, message })
}

async fn store_file(s: &AppState, file: &FsPath, content_type: Option<&str>) -> Result<String, ApiError> {
    Ok(s.store.put(&tokio::fs::read(file).await?, content_type).await?)
}

/// `ingestFile(src, dir, opts)`: the single ingest path for files (uploads, stock imports, emoji).
pub async fn ingest_file(s: &AppState, src: &FsPath, dir: &FsPath, o: IngestOpts<'_>) -> Result<Ingest, ApiError> {
    let mut head = vec![0u8; 4096];
    {
        use tokio::io::AsyncReadExt as _;
        let mut f = tokio::fs::File::open(src).await?;
        let mut off = 0;
        while off < head.len() {
            let n = f.read(&mut head[off..]).await?;
            if n == 0 {
                break;
            }
            off += n;
        }
    }
    let Some((mime, kind)) = sniff(&head) else {
        return err(415, "unsupported_type", Some("Upload images (PNG, JPEG, WebP, GIF, SVG), video (MP4, MOV, WebM) or audio (MP3, M4A, WAV, OGG).".into()));
    };
    let size = tokio::fs::metadata(src).await?.len() as i64;
    let gif = if mime == "image/gif" { Some(gif_info(&tokio::fs::read(src).await?)) } else { None };
    let animated = gif.is_some_and(|(frames, _)| frames > 1);
    if (kind == "image" || kind == "svg") && !animated && size > s.cfg.max_upload_bytes {
        return err(413, "too_large", Some(format!("Images are limited to {} MB", js::num(s.cfg.max_upload_bytes as f64 / 1024f64.powi(2)))));
    }
    if !o.skip_quota {
        let used: String = sqlx::query_scalar("SELECT coalesce(sum(bytes),0)::text AS total FROM assets WHERE owner_id = $1 AND trashed_at IS NULL").bind(o.owner_id).fetch_one(&s.pool).await?;
        let quota = billing::entitlements_of(s, o.owner_id).await?.0.storage_bytes;
        if used.parse::<f64>().unwrap_or(0.0) + size as f64 > quota as f64 {
            let tail = if billing::plans_enabled(s) { " or upgrade for more" } else { " to make room" };
            return err(402, "upgrade_required", Some(format!("You’ve used all {} GB of your storage. Delete uploads{tail}.", js::num(js::round(quota as f64 / 1024f64.powi(3))))));
        }
    }
    let id = oc_schema::ids::new_id();
    let ws = auth::personal_workspace(s, o.owner_id).await?;
    let name = js::take(o.name, 200);
    let source = o.source.unwrap_or(if animated { "gif" } else { "upload" }).to_owned();
    let attribution = o.attribution.clone();

    // Hash gate (TS-001): known-abuse perceptual hashes are refused and escalated, never stored.
    let phash = if kind == "svg" { None } else { media::perceptual_hash(src).await };
    let verdict = hash_match::match_upload(s, src, dir, phash.as_deref(), o.owner_id, o.ip, kind != "svg" && kind != "audio").await?;
    match verdict {
        HashVerdict::Unavailable => return err(503, "safety_check_unavailable", Some("Uploads are briefly unavailable. Try again in a minute.".into())),
        HashVerdict::Hit(hit) => {
            safety::file_report(
                s,
                safety::NewReport {
                    target_type: "user",
                    target_id: &o.owner_id.to_string(),
                    reason: &hit.category,
                    details: Some(&format!("Upload matched {} hash {} (file \"{name}\")", hit.source, hit.hash)),
                    reporter_id: None,
                    reporter_email: None,
                    legal: Some(json!({ "phash": phash })),
                },
            )
            .await?;
            oc_audit::audit_at(
                &s.pool,
                oc_audit::AuditEvent {
                    actor_id: Some(o.owner_id),
                    action: "ts.upload_blocked".into(),
                    target: Some(hit.hash.clone()),
                    meta: Some(json!({ "source": hit.source, "category": hit.category })),
                    ip: o.ip.map(str::to_owned),
                    ..Default::default()
                },
                s.now(),
            )
            .await?;
            return err(422, "blocked", Some("This file can’t be uploaded because it violates our content policy.".into()));
        }
        HashVerdict::Ok => {}
    }

    if kind == "video" || kind == "audio" || animated {
        let Some(p) = media::probe(src).await else {
            return err(415, "corrupt_media", Some("That file could not be read as video or audio.".into()));
        };
        if p.duration_ms > 60 * 60 * 1000 {
            return err(413, "too_long", Some("Media is limited to 60 minutes.".into()));
        }
        // Animated GIFs become looping video; transparent ones keep alpha in a VP9 WebM.
        let alpha = animated && gif.is_some_and(|(_, t)| t) && media::first_frame_has_alpha(src).await;
        let proxy_mime = if alpha { "video/webm" } else if p.kind == "video" { "video/mp4" } else { "audio/mp4" };
        let proxy = dir.join(if alpha { "proxy.webm" } else if p.kind == "video" { "proxy.mp4" } else { "proxy.m4a" });
        if alpha {
            media::make_alpha_proxy(src, &proxy).await.map_err(ApiError::internal)?;
        } else {
            media::make_proxy(src, &proxy, &p).await.map_err(ApiError::internal)?;
        }
        let mut poster_sha: Option<String> = None;
        if p.kind == "video" {
            let poster = dir.join(if alpha { "poster.png" } else { "poster.jpg" });
            media::make_poster(if alpha { src } else { &proxy }, &poster, if animated { 0 } else { p.duration_ms }).await.map_err(ApiError::internal)?;
            poster_sha = Some(store_file(s, &poster, Some(if alpha { "image/png" } else { "image/jpeg" })).await?);
        }
        let peaks = if p.has_audio { media::waveform(&proxy, 600).await.map_err(ApiError::internal)? } else { vec![] };
        let sha = store_file(s, src, None).await?;
        let proxy_sha = store_file(s, &proxy, Some(proxy_mime)).await?;
        // Rotation is baked into the proxy; report display dimensions from it.
        let dims = if p.kind == "video" { media::probe(&proxy).await } else { None };
        let w = dims.as_ref().map(|d| d.width).filter(|w| *w != 0).unwrap_or(p.width);
        let h = dims.as_ref().map(|d| d.height).filter(|h| *h != 0).unwrap_or(p.height);
        let peaks_json: Value = Value::Array(peaks.iter().map(|f| crate::http::jsjson::number_value(*f).unwrap_or(Value::Null)).collect());
        sqlx::query(
            "INSERT INTO assets(id, workspace_id, owner_id, sha256, mime, kind, bytes, width, height, name, duration_ms, has_audio, proxy_sha256, poster_sha256, waveform, phash, source, attribution, animated, proxy_mime)
       VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)",
        )
        .bind(&id)
        .bind(ws)
        .bind(o.owner_id)
        .bind(&sha)
        .bind(mime)
        .bind(p.kind)
        .bind(size)
        .bind(w as i32)
        .bind(h as i32)
        .bind(&name)
        .bind(p.duration_ms as i32)
        .bind(p.has_audio)
        .bind(&proxy_sha)
        .bind(&poster_sha)
        .bind(sqlx::types::Json(&peaks_json))
        .bind(&phash)
        .bind(&source)
        .bind(attribution.as_ref().map(sqlx::types::Json))
        .bind(animated)
        .bind(proxy_mime)
        .execute(&s.pool)
        .await?;
        let mut asset = Map::new();
        asset.insert("id".into(), json!(id));
        asset.insert("kind".into(), json!(p.kind));
        asset.insert("sha256".into(), json!(sha));
        asset.insert("mime".into(), json!(mime));
        asset.insert("w".into(), json!(w));
        asset.insert("h".into(), json!(h));
        asset.insert("name".into(), json!(name));
        asset.insert("durationMs".into(), json!(p.duration_ms));
        asset.insert("hasAudio".into(), json!(p.has_audio));
        if animated {
            asset.insert("animated".into(), json!(true));
        }
        return Ok(Ingest::Asset(Value::Object(asset)));
    }

    let mut buf = tokio::fs::read(src).await?;
    let mut mime = mime;
    let mut svg_sha: Option<String> = None;
    if kind == "svg" {
        // Keep the sanitised source, but the renderable asset is a PNG rasterised at up to 2048px.
        let svg = oc_schema::svg::sanitize_svg(&String::from_utf8_lossy(&buf));
        svg_sha = Some(s.store.put(svg.as_bytes(), Some("image/svg+xml")).await?);
        let (pw, ph) = svg_size(&svg).map_err(ApiError::internal)?;
        let longest = f64::from(pw.max(ph));
        let longest = if longest == 0.0 || longest.is_nan() { 512.0 } else { longest };
        buf = rasterize_svg(&svg, FitTo::Zoom((8f64).min(2048.0 / longest) as f32)).map_err(ApiError::internal)?;
        mime = "image/svg+xml";
    }
    let Some((w, h)) = image_size(&buf) else { return err(415, "corrupt_image", None) };
    if w == 0 || h == 0 || f64::from(w) * f64::from(h) > 250e6 {
        return err(415, "bad_dimensions", None);
    }
    let sha = s.store.put(&buf, None).await?;
    sqlx::query("INSERT INTO assets(id, workspace_id, owner_id, sha256, mime, kind, bytes, width, height, name, phash, source, attribution, svg_sha256) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)")
        .bind(&id)
        .bind(ws)
        .bind(o.owner_id)
        .bind(&sha)
        .bind(mime)
        .bind(kind)
        .bind(buf.len() as i64)
        .bind(w as i32)
        .bind(h as i32)
        .bind(&name)
        .bind(&phash)
        .bind(&source)
        .bind(attribution.as_ref().map(sqlx::types::Json))
        .bind(&svg_sha)
        .execute(&s.pool)
        .await?;
    Ok(Ingest::Asset(json!({ "id": id, "kind": kind, "sha256": sha, "mime": mime, "w": w, "h": h, "name": name })))
}

// ---------------------------------------------------------------- serving

const SERVABLE: &str = "moderation IN ('ok','pending')";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

async fn poster(State(s): State<AppState>, Path(id): Path<String>, _req: Req) -> ApiResult {
    let a: Option<Option<String>> = sqlx::query_scalar(&format!("SELECT poster_sha256 FROM assets WHERE id = $1 AND {SERVABLE}")).bind(&id).fetch_optional(&s.pool).await?;
    let Some(Some(sha)) = a else { return Ok(Reply::empty(404)) };
    let bytes = s.store.get(&sha).await?;
    let ty = if bytes.first() == Some(&0x89) { "image/png" } else { "image/jpeg" };
    Ok(Reply::bytes(ty, bytes).header("cache-control", IMMUTABLE))
}

/// Sanitised SVG source of an `svg` asset (the editor reads its colours for recolouring, A4).
async fn svg_source(State(s): State<AppState>, Path(id): Path<String>, _req: Req) -> ApiResult {
    let a: Option<Option<String>> = sqlx::query_scalar(&format!("SELECT svg_sha256 FROM assets WHERE id = $1 AND {SERVABLE}")).bind(&id).fetch_optional(&s.pool).await?;
    let Some(Some(sha)) = a else { return Ok(Reply::empty(404)) };
    Ok(Reply::bytes("image/svg+xml", s.store.get(&sha).await?).header("cache-control", IMMUTABLE).header("x-content-type-options", "nosniff").header("content-security-policy", CSP))
}

/// Recoloured raster of an `svg` asset: `<id>/recolor/<from-to.from-to>`. Same size as the original raster.
async fn recolor(State(s): State<AppState>, Path((id, pairs)): Path<(String, String)>, _req: Req) -> ApiResult {
    let Some(key) = oc_schema::svg::parse_recolor_key(&format!("{id}/recolor/{pairs}")) else { return Ok(Reply::empty(404)) };
    let a: Option<(Option<String>, String, i32)> =
        sqlx::query_as(&format!("SELECT svg_sha256, sha256, width FROM assets WHERE id = $1 AND {SERVABLE}")).bind(&id).fetch_optional(&s.pool).await?;
    let Some((svg_sha, sha, width)) = a else { return Ok(Reply::empty(404)) };
    // Assets without a stored source (uploaded before A4) keep their original colours.
    let Some(svg_sha) = svg_sha else {
        return Ok(Reply::bytes("image/png", s.store.get(&sha).await?).header("cache-control", IMMUTABLE).header("x-content-type-options", "nosniff"));
    };
    let svg = oc_schema::svg::recolor_svg(&String::from_utf8_lossy(&s.store.get(&svg_sha).await?), &key.map);
    let w = if width == 0 { 1024 } else { width };
    let png = rasterize_svg(&svg, FitTo::Width(w.clamp(16, 4096) as u32)).map_err(ApiError::internal)?;
    Ok(Reply::bytes("image/png", png).header("cache-control", IMMUTABLE).header("x-content-type-options", "nosniff"))
}

async fn waveform(State(s): State<AppState>, Path(id): Path<String>, _req: Req) -> ApiResult {
    let a: Option<(Option<sqlx::types::Json<Value>>, Option<i32>)> = sqlx::query_as("SELECT waveform, duration_ms FROM assets WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some((wave, dur)) = a else { return Ok(Reply::empty(404)) };
    let peaks = wave.map(|w| w.0).filter(|w| !w.is_null()).unwrap_or(json!([]));
    Ok(Reply::json(json!({ "peaks": peaks, "durationMs": dur })).header("cache-control", IMMUTABLE))
}

static RANGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"bytes=(\d*)-(\d*)").expect("regex"));

/// Asset bytes. Ids are unguessable; served with a restrictive CSP and no-sniff.
async fn serve(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    type Served = (String, String, String, Option<String>, Option<String>);
    let a: Option<Served> =
        sqlx::query_as(&format!("SELECT sha256, mime, kind, proxy_sha256, proxy_mime FROM assets WHERE id = $1 AND {SERVABLE}")).bind(&id).fetch_optional(&s.pool).await?;
    let Some((sha, mime, kind, proxy_sha, proxy_mime)) = a else { return Ok(Reply::empty(404)) };
    if (kind == "video" || kind == "audio")
        && let Some(proxy) = proxy_sha
    {
        // Media streams with HTTP Range support so the editor can seek.
        let size = s.store.size(&proxy).await?;
        let ty = proxy_mime.unwrap_or_else(|| if kind == "video" { "video/mp4".into() } else { "audio/mp4".into() });
        let base = |r: Reply| r.header("accept-ranges", "bytes").header("cache-control", IMMUTABLE).header("x-content-type-options", "nosniff");
        if let Some(m) = RANGE.captures(req.header("range").unwrap_or("")) {
            let (a, b) = (m.get(1).map_or("", |x| x.as_str()), m.get(2).map_or("", |x| x.as_str()));
            let num = |t: &str| if t.is_empty() { 0f64 } else { t.parse::<f64>().unwrap_or(f64::NAN) };
            let sz = size as f64;
            let start = if !a.is_empty() { num(a) } else { (sz - num(b)).max(0.0) };
            let end = if !a.is_empty() && !b.is_empty() { (sz - 1.0).min(num(b)) } else { sz - 1.0 };
            if start >= sz || start > end {
                return Ok(base(Reply::empty(416)).header("content-range", format!("bytes */{size}")));
            }
            let (st, en) = (start as u64, end as u64);
            let stream = s.store.stream(&proxy, Some(oc_storage::ByteRange { start: st, end: en })).await?;
            let body = axum::body::Body::from_stream(tokio_util_stream(stream));
            return Ok(base(Reply::bytes(&ty, body).status(206))
                .header("content-range", format!("bytes {st}-{en}/{size}"))
                .header("content-length", (en - st + 1).to_string()));
        }
        let stream = s.store.stream(&proxy, None).await?;
        return Ok(base(Reply::bytes(&ty, axum::body::Body::from_stream(tokio_util_stream(stream)))).header("content-length", size.to_string()));
    }
    let ty = if mime == "image/svg+xml" { "image/png".to_owned() } else { mime };
    Ok(Reply::bytes(&ty, s.store.get(&sha).await?).header("cache-control", IMMUTABLE).header("x-content-type-options", "nosniff").header("content-security-policy", CSP))
}

/// An `AsyncRead` as a byte stream for a response body (`reply.send(stream)`).
pub fn tokio_util_stream(r: oc_storage::ByteStream) -> impl futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send {
    futures::stream::unfold(Some(r), |r| async move {
        use tokio::io::AsyncReadExt as _;
        let mut r = r?;
        let mut buf = vec![0u8; 64 * 1024];
        match r.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(bytes::Bytes::from(buf)), Some(r)))
            }
            Err(e) => Some((Err(e), None)),
        }
    })
}

// ---------------------------------------------------------------- signed-in routes

/// `{ all, images, videos, audio, files }` of a counts query (zeros when there is no row).
async fn upload_counts(s: &AppState, b: &crate::search::Built) -> Result<Value, ApiError> {
    let c = pg::json_one(&s.pool, &b.sql, &b.params).await?;
    Ok(c.unwrap_or_else(|| json!({ "all": 0, "images": 0, "videos": 0, "audio": 0, "files": 0 })))
}

async fn used_bytes(s: &AppState, uid: Uuid) -> Result<f64, ApiError> {
    let used: String = sqlx::query_scalar("SELECT coalesce(sum(bytes),0)::text AS total FROM assets WHERE owner_id = $1 AND trashed_at IS NULL").bind(uid).fetch_one(&s.pool).await?;
    Ok(used.parse::<f64>().unwrap_or(0.0))
}

/// The owner's uploads (g19): optional name search, type filter, sort and paging. Without parameters, the newest 500.
async fn list(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let p = uploads::uploads_query().parse(Some(&req.query))?;
    let l = uploads::uploads_list_sql(u.id, &p, false);
    let rows = pg::json_rows(&s.pool, &l.sql, &l.params).await?;
    let c = upload_counts(&s, &uploads::upload_counts_sql(u.id, &p, false)).await?;
    let used = used_bytes(&s, u.id).await?;
    let quota = billing::entitlements_of(&s, u.id).await?.0.storage_bytes;
    let used = crate::http::jsjson::number_value(used).unwrap_or(json!(0));
    let total = c.get(p["type"].as_str().unwrap_or("all")).cloned().unwrap_or(Value::Null);
    Ok(Reply::json(json!({ "assets": rows, "usedBytes": used, "quotaBytes": quota, "total": total, "counts": c })))
}

/// Trashed uploads (g22) with the date the worker purges them; restorable until then.
async fn trash_list(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let p = uploads::uploads_query().parse(Some(&req.query))?;
    let l = uploads::uploads_list_sql(u.id, &p, true);
    let rows = pg::json_rows(&s.pool, &l.sql, &l.params).await?;
    let c = upload_counts(&s, &uploads::upload_counts_sql(u.id, &p, true)).await?;
    let total = c.get(p["type"].as_str().unwrap_or("all")).cloned().unwrap_or(Value::Null);
    Ok(Reply::json(json!({ "assets": rows, "total": total, "counts": c })))
}

/// Rename an upload (owner; not while it is in the trash).
async fn rename(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = uploads::rename_body().parse(req.body())?;
    let name = b["name"].as_str().unwrap_or("").to_owned();
    let r = pg::rows(&s.pool, "UPDATE assets SET name = $3 WHERE id = $1 AND owner_id = $2 AND trashed_at IS NULL RETURNING id", &[pg::text(&id), pg::uuid(u.id), pg::text(&name)]).await?;
    if r.is_empty() {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    Ok(Reply::json(json!({ "ok": true, "name": name })))
}

/// The original file as an attachment (owner only): source bytes, not the playback proxy; an SVG's sanitised source.
async fn download(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let a: Option<(String, String, Option<String>, Option<String>)> =
        sqlx::query_as(&format!("SELECT sha256, mime, name, svg_sha256 FROM assets WHERE id = $1 AND owner_id = $2 AND {SERVABLE}")).bind(&id).bind(u.id).fetch_optional(&s.pool).await?;
    let Some((sha, mime, name, svg_sha)) = a else { return Ok(reply::json(404, json!({ "error": "not_found" }))) };
    // SVG uploads keep a rasterised PNG as their main object; uploads from before A4 have nothing else.
    let svg = mime == "image/svg+xml";
    let obj = if svg { svg_sha.clone().unwrap_or(sha) } else { sha };
    let ty = if svg && svg_sha.is_none() { "image/png".to_owned() } else { mime };
    let size = s.store.size(&obj).await?;
    let stream = s.store.stream(&obj, None).await?;
    Ok(Reply::bytes(&ty, axum::body::Body::from_stream(tokio_util_stream(stream)))
        .header("content-disposition", uploads::content_disposition(&uploads::download_name(name.as_deref(), &ty)))
        .header("content-length", size.to_string())
        .header("cache-control", "private, no-store")
        .header("content-security-policy", "default-src 'none'; sandbox"))
}

/// `runBatch(uid, action, ids)`: restoring checks the storage quota first, like a new upload.
async fn run_batch(s: &AppState, uid: Uuid, action: &str, ids: &[String]) -> Result<Result<Vec<String>, Reply>, ApiError> {
    if action == "restore" {
        let add = uploads::restore_bytes_sql(uid, ids);
        let adding: Option<String> = pg::one(&s.pool, &format!("SELECT total::text FROM ({}) t", add.sql), &add.params).await?.map(|r| sqlx::Row::get(&r, 0));
        let extra = adding.and_then(|a| a.parse::<f64>().ok()).unwrap_or(0.0);
        let used = used_bytes(s, uid).await?;
        let quota = billing::entitlements_of(s, uid).await?.0.storage_bytes;
        if extra > 0.0 && used + extra > quota as f64 {
            let tail = if billing::plans_enabled(s) { " or upgrade for more" } else { " to make room" };
            let message = format!("Restoring would go over your {} GB of storage. Delete uploads{tail}.", js::num(js::round(quota as f64 / 1024f64.powi(3))));
            return Ok(Err(reply::json(402, json!({ "error": "upgrade_required", "message": message }))));
        }
    }
    let b = uploads::batch_sql(uid, action, ids);
    let rows: Vec<String> = pg::rows(&s.pool, &b.sql, &b.params).await?.iter().map(|r| sqlx::Row::get::<String, _>(r, 0)).collect();
    Ok(Ok(rows))
}

/// Take an upload out of the trash (owner). Refused when it no longer fits in the storage quota.
async fn restore(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    match run_batch(&s, u.id, "restore", std::slice::from_ref(&id)).await? {
        Err(r) => Ok(r),
        Ok(ids) if ids.is_empty() => Ok(reply::json(404, json!({ "error": "not_found" }))),
        Ok(_) => Ok(Reply::json(json!({ "ok": true }))),
    }
}

/// Bulk trash / restore / delete forever. Ids that aren't the caller's (or aren't in the right state) are skipped.
async fn batch(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = uploads::batch_body().parse(req.body())?;
    let mut ids: Vec<String> = Vec::new();
    for v in b["ids"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if !ids.iter().any(|x| x == v) {
            ids.push(v.to_owned());
        }
    }
    match run_batch(&s, u.id, b["action"].as_str().unwrap_or(""), &ids).await? {
        Err(r) => Ok(r),
        Ok(done) => Ok(Reply::json(json!({ "ok": true, "ids": done }))),
    }
}

/// A temporary directory removed on drop (`mkdtemp` + `finally rm`).
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub async fn new(prefix: &str) -> std::io::Result<TempDir> {
        let p = std::env::temp_dir().join(format!("{prefix}{}", oc_db::uuid().simple()));
        tokio::fs::create_dir_all(&p).await?;
        Ok(TempDir(p))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn upload(State(s): State<AppState>, mut req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let dir = TempDir::new("oc-up-").await?;
    let src = dir.0.join("src");
    let Some(file) = multipart::save_file(&mut req, s.cfg.max_media_bytes as u64, &src).await? else {
        return Ok(reply::json(400, json!({ "error": "no_file" })));
    };
    if file.truncated {
        return Ok(reply::json(413, json!({ "error": "too_large", "message": format!("Max {} MB", js::num(s.cfg.max_media_bytes as f64 / 1024f64.powi(2))) })));
    }
    let r = ingest_file(&s, &src, &dir.0, IngestOpts { name: &file.filename, owner_id: u.id, ip: Some(&req.ip), source: None, attribution: None, skip_quota: false }).await?;
    match r {
        Ingest::Asset(a) => Ok(Reply::json(json!({ "asset": a }))),
        Ingest::Error { status, error, message } => {
            let mut body = Map::new();
            body.insert("error".into(), json!(error));
            if let Some(m) = message {
                body.insert("message".into(), json!(m));
            }
            Ok(reply::json(status, Value::Object(body)))
        }
    }
}

/// Move to trash (owner) — restorable for 30 days. `?permanent=1` deletes an upload that is already in the trash.
async fn trash(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let permanent = req.query_str("permanent") == Some("1");
    let a: Option<(Option<chrono::DateTime<chrono::Utc>>, String)> =
        sqlx::query_as("SELECT trashed_at, moderation FROM assets WHERE id = $1 AND owner_id = $2").bind(&id).bind(u.id).fetch_optional(&s.pool).await?;
    // Moderator removals are on legal hold: not the owner's to trash or delete.
    let Some((trashed, moderation)) = a.filter(|(_, m)| m != "removed") else { return Ok(reply::json(404, json!({ "error": "not_found" }))) };
    if trashed.is_some() && !permanent {
        return Ok(Reply::json(json!({ "ok": true })));
    }
    // Reported and waiting for review: it can go to the trash, but stays there as evidence until a moderator decides.
    if trashed.is_some() && moderation != "ok" {
        return Ok(reply::json(409, json!({ "error": "on_hold", "message": "This upload is being reviewed and can’t be deleted yet." })));
    }
    let b = uploads::batch_sql(u.id, if trashed.is_some() { "delete" } else { "trash" }, std::slice::from_ref(&id));
    pg::rows(&s.pool, &b.sql, &b.params).await?;
    Ok(Reply::json(json!({ "ok": true })))
}
