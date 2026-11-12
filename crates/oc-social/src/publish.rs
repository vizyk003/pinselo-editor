//! The `publish` job (port of services/api/src/social/publish.ts, run by the worker): render the export
//! through the render service, upload it with each account's adapter, record per-account results and
//! events, retry transient failures with backoff, and notify the owner (in-app + email). Scheduling = a
//! delayed job at `scheduled_at` plus a sweep for safety.

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde_json::{Map, Value, json};
use sqlx::types::Json;
use uuid::Uuid;

use crate::crypto::{MEDIA_TTL_SEC, internal_token, signed_media_url_at};
use crate::http::{fetch_error_message, slice_utf16};
use crate::imagesize::image_size;
use crate::store::{AccountRow, tokens_for};
use crate::text::{char_count, compose_text, media_mime};
use crate::types::{AccountInfo, MediaFile, PostKind, PublishContext, PublishError, SocialAdapter};
use crate::{Social, SocialConfig, adapter, db_message};

/// Attempts of a `publish` job.
pub const MAX_ATTEMPTS: i32 = 5;

/// A `social_posts` row (`RETURNING *`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PostRow {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub design_id: String,
    /// 0-based page indexes; empty = all pages.
    pub pages: Vec<i32>,
    /// `'png' | 'jpg' | 'mp4'`.
    pub format: String,
    pub caption: String,
    pub hashtags: Vec<String>,
    pub accounts: Vec<Uuid>,
    pub options: Json<Map<String, Value>>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub timezone: String,
    pub status: String,
    /// Rendered files (`StoredMedia[]`), kept as stored.
    pub media: Json<Vec<Value>>,
    /// Per account id: `{ status, url?, error?, note?, at }`.
    pub results: Json<Map<String, Value>>,
    pub error: Option<String>,
    pub job_id: Option<Uuid>,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// `'pinselo'` (this worker publishes it) or `'external'` (an outside tool claims it through /v1/planner).
    pub publisher: String,
    /// External posts: `'instagram' | 'facebook'`.
    pub channels: Vec<String>,
    /// Editorial fields (`ref`, `pillar`, `source`, `topic`, `factIds`, `reviewerNote`, `formatLabel`, …).
    pub fields: Json<Map<String, Value>>,
    pub submitted_by: Option<Uuid>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<DateTime<Utc>>,
    /// The design's `content_version` when the post was approved.
    pub approved_version: Option<i64>,
    pub claim_token: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
}

/// One rendered file (`StoredMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredMedia {
    pub sha: String,
    pub mime: String,
    pub filename: String,
    pub width: f64,
    pub height: f64,
    pub bytes: usize,
    pub page: Option<i64>,
}

impl StoredMedia {
    pub fn to_json(&self) -> Value {
        json!({
            "sha": self.sha, "mime": self.mime, "filename": self.filename, "width": num(self.width),
            "height": num(self.height), "bytes": self.bytes, "page": self.page,
        })
    }

    fn from_json(v: &Value) -> StoredMedia {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
        StoredMedia {
            sha: s("sha"),
            mime: s("mime"),
            filename: s("filename"),
            width: f("width"),
            height: f("height"),
            bytes: v.get("bytes").and_then(Value::as_u64).unwrap_or(0) as usize,
            page: v.get("page").and_then(Value::as_i64),
        }
    }
}

/// A JSON number that prints like JS (integral doubles without a fraction).
fn num(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

/// The `job` of the job context (`ctx.job.id / attempts / max_attempts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobInfo {
    pub id: Uuid,
    /// This attempt's number (1-based).
    pub attempts: i32,
    pub max_attempts: i32,
}

fn tx(e: sqlx::Error) -> PublishError {
    PublishError::transient(db_message(&e))
}

/// `logEvent(postId, kind, message, data, accountId)`. `data`: `None` = undefined (SQL NULL),
/// `Some(Value::Null)` = JSON `null`.
pub async fn log_event(
    pool: &sqlx::PgPool,
    post_id: Uuid,
    kind: &str,
    message: Option<&str>,
    data: Option<Value>,
    account_id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO social_post_events(post_id, account_id, kind, message, data) VALUES ($1,$2,$3,$4,$5)")
        .bind(post_id)
        .bind(account_id)
        .bind(kind)
        .bind(message.map(|m| slice_utf16(m, 1000)))
        .bind(data.map(Json))
        .execute(pool)
        .await
        .map(|_| ())
}

// ---------------------------------------------------------------- scheduling

/// `schedulePublish(post)`: queue the publish job for the post's scheduled time (earlier jobs for the same
/// post become no-ops). `scheduled_at: None` = now.
pub async fn schedule_publish(
    pool: &sqlx::PgPool,
    post_id: Uuid,
    owner_id: Uuid,
    scheduled_at: Option<DateTime<Utc>>,
    now_ms: i64,
) -> Result<Uuid, sqlx::Error> {
    let at = scheduled_at.map_or(now_ms, |t| t.timestamp_millis());
    oc_jobs::enqueue(
        pool,
        "publish",
        &json!({ "postId": post_id.to_string() }),
        oc_jobs::EnqueueOpts {
            owner_id: Some(owner_id),
            delay_ms: Some((at - now_ms).max(0) as u64),
            max_attempts: Some(MAX_ATTEMPTS),
            priority: None,
            label: Some("Scheduled".into()),
        },
    )
    .await
}

/// A post that is due (`duePosts`).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct DuePost {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub scheduled_at: DateTime<Utc>,
}

/// Scheduled posts whose time has come and that have no publish job queued or running.
pub async fn due_posts(pool: &sqlx::PgPool, limit: i64) -> Result<Vec<DuePost>, sqlx::Error> {
    sqlx::query_as(
        "SELECT p.id, p.owner_id, p.scheduled_at FROM social_posts p
     WHERE p.status = 'scheduled' AND p.publisher = 'pinselo' AND p.scheduled_at <= now()
       AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.queue = 'publish' AND j.status IN ('queued','running') AND j.payload->>'postId' = p.id::text)
     ORDER BY p.scheduled_at LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Safety sweep (every minute): enqueue missed posts; fail posts stuck in `publishing` without a live job;
/// drop OAuth states older than an hour. Returns the number of posts queued.
pub async fn sweep(pool: &sqlx::PgPool, now_ms: i64) -> Result<usize, sqlx::Error> {
    let due = due_posts(pool, 100).await?;
    for p in &due {
        schedule_publish(pool, p.id, p.owner_id, None, now_ms).await?;
        log_event(pool, p.id, "swept", Some("Queued by the scheduler sweep"), None, None).await?;
    }
    let stuck: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE social_posts p SET status = 'failed', error = 'Publishing was interrupted. Try again.', updated_at = now()
     WHERE p.status = 'publishing' AND p.publisher = 'pinselo' AND p.updated_at < now() - interval '30 minutes'
       AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.queue = 'publish' AND j.status IN ('queued','running') AND j.payload->>'postId' = p.id::text)
     RETURNING p.id",
    )
    .fetch_all(pool)
    .await?;
    for id in stuck {
        log_event(pool, id, "failed", Some("Publishing was interrupted"), None, None).await?;
    }
    sqlx::query("DELETE FROM social_oauth_states WHERE created_at < now() - interval '1 hour'")
        .execute(pool)
        .await?;
    Ok(due.len())
}

// ---------------------------------------------------------------- rendering

/// `RenderFn`: render one page (`page: Some(i)`) or the whole video (`None`) as `'png' | 'jpg' | 'mp4'`.
pub trait Render: Send + Sync {
    fn render<'a>(
        &'a self,
        design_id: &'a str,
        format: &'a str,
        page: Option<i64>,
    ) -> BoxFuture<'a, Result<Vec<u8>, PublishError>>;
}

impl<F> Render for F
where
    F: Fn(&str, &str, Option<i64>) -> BoxFuture<'static, Result<Vec<u8>, PublishError>> + Send + Sync,
{
    fn render<'a>(
        &'a self,
        design_id: &'a str,
        format: &'a str,
        page: Option<i64>,
    ) -> BoxFuture<'a, Result<Vec<u8>, PublishError>> {
        self(design_id, format, page)
    }
}

/// `renderViaService`: the render service's internal export endpoint. JPEG falls back to PNG + ffmpeg when
/// the renderer cannot encode JPEG (`native` feature).
#[derive(Debug, Clone)]
pub struct RenderService {
    pub client: reqwest::Client,
    pub render_url: String,
    pub secret: String,
    /// `OC_FFMPEG ?? 'ffmpeg'`.
    pub ffmpeg: String,
}

impl RenderService {
    pub fn new(cfg: &SocialConfig, ffmpeg: Option<String>) -> Self {
        RenderService {
            client: reqwest::Client::new(),
            render_url: cfg.render_url.clone(),
            secret: cfg.secret.clone(),
            ffmpeg: ffmpeg.unwrap_or_else(|| "ffmpeg".into()),
        }
    }

    async fn render_once(&self, design_id: &str, format: &str, page: Option<i64>) -> Result<Vec<u8>, PublishError> {
        let mut body = Map::new();
        body.insert("designId".into(), json!(design_id));
        body.insert("format".into(), json!(format));
        if let Some(p) = page {
            body.insert("pages".into(), json!([p]));
        }
        body.insert("scale".into(), json!(1));
        body.insert("quality".into(), json!(92));
        body.insert("transparent".into(), json!(false));
        let timeout = Duration::from_secs(if format == "mp4" { 30 * 60 } else { 5 * 60 });
        let res = self
            .client
            .post(format!("{}/internal/export", self.render_url))
            .header("content-type", "application/json")
            .header("x-oc-internal", internal_token(&self.secret))
            .body(Value::Object(body).to_string())
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| PublishError::transient(format!("Render service unavailable: {}", fetch_error_message(&e))))?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            let text = res.text().await.unwrap_or_default();
            return Err(PublishError {
                message: format!("Render failed: {}", slice_utf16(&text, 200)),
                transient: status >= 500,
                status: None,
                reauth: false,
            });
        }
        res.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| PublishError::transient(format!("Render service unavailable: {}", fetch_error_message(&e))))
    }
}

impl Render for RenderService {
    fn render<'a>(
        &'a self,
        design_id: &'a str,
        format: &'a str,
        page: Option<i64>,
    ) -> BoxFuture<'a, Result<Vec<u8>, PublishError>> {
        Box::pin(async move {
            if format != "jpg" {
                return self.render_once(design_id, format, page).await;
            }
            // Some renderer builds can't encode JPEG ("encode failed"): fall back to PNG + ffmpeg.
            match self.render_once(design_id, "jpg", page).await {
                Ok(b) => Ok(b),
                Err(e) if e.transient && !e.message.contains("encode failed") => Err(e),
                Err(_) => {
                    let png = self.render_once(design_id, "png", page).await?;
                    png_to_jpeg(&self.ffmpeg, &png).await
                }
            }
        })
    }
}

/// PNG → JPEG (quality ≈ 92) through ffmpeg, which the render stack already requires.
#[cfg(feature = "native")]
pub async fn png_to_jpeg(ffmpeg: &str, png: &[u8]) -> Result<Vec<u8>, PublishError> {
    use tokio::io::AsyncWriteExt as _;
    let mut child = tokio::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-f", "png_pipe", "-i", "pipe:0", "-q:v", "2", "-f", "mjpeg", "pipe:1"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            // Node: "spawn ffmpeg ENOENT".
            let msg = if e.kind() == std::io::ErrorKind::NotFound { format!("spawn {ffmpeg} ENOENT") } else { e.to_string() };
            PublishError::new(format!("JPEG conversion failed: {msg}"))
        })?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let input = png.to_vec();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&input).await;
        let _ = stdin.shutdown().await;
    });
    let out = child.wait_with_output().await.map_err(|e| PublishError::new(format!("JPEG conversion failed: {e}")))?;
    let _ = writer.await;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(PublishError::new(format!("JPEG conversion failed: {}", slice_utf16(&err, 200))))
    }
}

/// Without the `native` feature there is no ffmpeg: the conversion fails like a missing binary.
#[cfg(not(feature = "native"))]
pub async fn png_to_jpeg(ffmpeg: &str, _png: &[u8]) -> Result<Vec<u8>, PublishError> {
    Err(PublishError::new(format!("JPEG conversion failed: spawn {ffmpeg} ENOENT")))
}

fn ext_of(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "video/mp4" => Some("mp4"),
        _ => None,
    }
}

/// What the publish job knows about the design.
#[derive(Debug, Clone, PartialEq)]
pub struct DesignInfo {
    pub title: String,
    pub page_count: usize,
    pub w: f64,
    pub h: f64,
    pub duration_ms: f64,
}

/// `SELECT title, width, height, trashed_at, page_count FROM designs`.
type DesignRow = (String, f64, f64, Option<DateTime<Utc>>, i32);

/// `designInfo(designId)`: `None` when the design is gone or trashed.
pub async fn design_info(pool: &sqlx::PgPool, design_id: &str) -> Result<Option<DesignInfo>, PublishError> {
    let d: Option<DesignRow> =
        sqlx::query_as("SELECT title, width, height, trashed_at, page_count FROM designs WHERE id = $1")
            .bind(design_id)
            .fetch_optional(pool)
            .await
            .map_err(tx)?;
    let Some((title, width, height, trashed, page_count)) = d else { return Ok(None) };
    if trashed.is_some() {
        return Ok(None);
    }
    let s: Option<Vec<u8>> = sqlx::query_scalar("SELECT state FROM doc_state WHERE design_id = $1")
        .bind(design_id)
        .fetch_optional(pool)
        .await
        .map_err(tx)?;
    let Some(state) = s else {
        return Ok(Some(DesignInfo {
            title,
            page_count: if page_count == 0 { 1 } else { page_count.max(0) as usize },
            w: width,
            h: height,
            duration_ms: 0.0,
        }));
    };
    let doc = oc_crdt::decode_doc_value(&state).map_err(|e| PublishError::transient(e.to_string()))?;
    let meta = doc.get("meta");
    let dims = meta.and_then(|m| m.get("dims")).filter(|d| d.is_object()).ok_or_else(|| {
        PublishError::transient("Cannot read properties of undefined (reading 'w')")
    })?;
    let pages: Vec<&Value> = match doc.get("pages") {
        Some(Value::Object(p)) => p.values().collect(),
        _ => Vec::new(),
    };
    let doc_title = meta.and_then(|m| m.get("title")).and_then(Value::as_str).filter(|t| !t.is_empty());
    let duration_ms = pages
        .iter()
        .filter(|p| !crate::http::truthy(p.get("hidden")))
        .map(|p| crate::http::non_null(p.get("durationMs")).and_then(Value::as_f64).unwrap_or(5000.0))
        .sum();
    Ok(Some(DesignInfo {
        title: doc_title.map_or(title, str::to_string),
        page_count: pages.len().max(1),
        w: dims.get("w").and_then(Value::as_f64).unwrap_or(f64::NAN),
        h: dims.get("h").and_then(Value::as_f64).unwrap_or(f64::NAN),
        duration_ms,
    }))
}

/// `Math.round`.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// The file name stem: `title.replace(/[^\w\- ]+/g, '').trim().slice(0, 60) || 'design'`.
pub fn safe_name(title: &str) -> String {
    let kept: String = title.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ' ')).collect();
    let t: String = kept.trim_matches(' ').chars().take(60).collect();
    if t.is_empty() { "design".into() } else { t }
}

async fn render_media(
    s: &Social,
    post: &mut PostRow,
    info: &DesignInfo,
    mime: &str,
    render: &dyn Render,
) -> Result<Vec<StoredMedia>, PublishError> {
    let have: Vec<StoredMedia> = post
        .media
        .0
        .iter()
        .filter(|m| m.get("mime").and_then(Value::as_str) == Some(mime))
        .map(StoredMedia::from_json)
        .collect();
    if !have.is_empty() {
        return Ok(have);
    }
    let fmt = ext_of(mime).unwrap_or("png");
    let safe = safe_name(&info.title);
    let mut out = Vec::new();
    if fmt == "mp4" {
        let bytes = render.render(&post.design_id, "mp4", None).await?;
        let sha = s.objects.put(&bytes, Some(mime)).await.map_err(|e| PublishError::transient(e.to_string()))?;
        out.push(StoredMedia {
            sha,
            mime: mime.into(),
            filename: format!("{safe}.mp4"),
            width: js_round(info.w),
            height: js_round(info.h),
            bytes: bytes.len(),
            page: None,
        });
    } else {
        let pages: Vec<i64> = if post.pages.is_empty() {
            (0..info.page_count as i64).collect()
        } else {
            post.pages.iter().map(|p| i64::from(*p)).filter(|p| *p < info.page_count as i64).collect()
        };
        for p in pages {
            let bytes = render.render(&post.design_id, fmt, Some(p)).await?;
            let (w, h) = match image_size(&bytes) {
                Some((w, h)) => (f64::from(w), f64::from(h)),
                None => (js_round(info.w), js_round(info.h)),
            };
            let sha = s.objects.put(&bytes, Some(mime)).await.map_err(|e| PublishError::transient(e.to_string()))?;
            out.push(StoredMedia {
                sha,
                mime: mime.into(),
                filename: format!("{safe}-{:02}.{fmt}", p + 1),
                width: w,
                height: h,
                bytes: bytes.len(),
                page: Some(p),
            });
        }
    }
    if out.is_empty() {
        return Err(PublishError::new("No pages to publish"));
    }
    post.media.0.extend(out.iter().map(StoredMedia::to_json));
    sqlx::query("UPDATE social_posts SET media = $2, updated_at = now() WHERE id = $1")
        .bind(post.id)
        .bind(Json(&post.media.0))
        .execute(&s.pool)
        .await
        .map_err(tx)?;
    log_event(&s.pool, post.id, "rendered", Some(&format!("{} × {}", out.len(), fmt.to_uppercase())), None, None)
        .await
        .map_err(tx)?;
    Ok(out)
}

async fn load_files(s: &Social, list: &[StoredMedia]) -> Result<Vec<MediaFile>, PublishError> {
    let mut out = Vec::with_capacity(list.len());
    for m in list {
        let bytes = s.objects.get(&m.sha).await.map_err(|e| PublishError::transient(e.to_string()))?;
        out.push(MediaFile {
            bytes,
            mime: m.mime.clone(),
            filename: m.filename.clone(),
            url: signed_media_url_at(&m.sha, ext_of(&m.mime).unwrap_or("bin"), MEDIA_TTL_SEC, s.cfg.media_base(), &s.cfg.secret, s.now_ms()),
            width: m.width,
            height: m.height,
        });
    }
    Ok(out)
}

/// `mediaMime(format, adapter)`: the media variant an adapter accepts (Instagram/TikTok want JPEG).
pub fn post_media_mime(format: &str, a: &dyn SocialAdapter) -> String {
    media_mime(format, a).map(str::to_string).unwrap_or_default()
}

// ---------------------------------------------------------------- the job

/// `{ status, url?, error?, note? }` of one target before the timestamp is added.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    Failed(String),
    Retrying(String),
    Published { url: Option<String>, note: Option<String> },
}

async fn set_result(s: &Social, post: &mut PostRow, account: Uuid, r: Target) -> Result<(), PublishError> {
    let mut v = Map::new();
    match r {
        Target::Failed(e) => {
            v.insert("status".into(), json!("failed"));
            v.insert("error".into(), json!(e));
        }
        Target::Retrying(e) => {
            v.insert("status".into(), json!("retrying"));
            v.insert("error".into(), json!(e));
        }
        Target::Published { url, note } => {
            v.insert("status".into(), json!("published"));
            v.insert("url".into(), url.map_or(Value::Null, Value::from));
            if let Some(n) = note {
                v.insert("note".into(), json!(n));
            }
        }
    }
    let at = DateTime::from_timestamp_millis(s.now_ms()).unwrap_or_default();
    v.insert("at".into(), json!(oc_js::iso_string(&at)));
    post.results.0.insert(account.to_string(), Value::Object(v));
    sqlx::query("UPDATE social_posts SET results = $2, updated_at = now() WHERE id = $1")
        .bind(post.id)
        .bind(Json(&post.results.0))
        .execute(&s.pool)
        .await
        .map_err(tx)?;
    Ok(())
}

/// `Math.round(maxImageBytes / 1e5) / 10` as printed in the "limits images to … MB" message.
fn megabytes(b: usize) -> String {
    oc_js::number_to_string(js_round(b as f64 / 1e5) / 10.0)
}

/// Publish one target; errors are reported per account by the caller.
#[allow(clippy::too_many_arguments)]
async fn publish_target(
    s: &Social,
    post: &mut PostRow,
    info: &DesignInfo,
    row: &AccountRow,
    a: &'static dyn SocialAdapter,
    text: &str,
    render: &dyn Render,
) -> Result<(), PublishError> {
    let account_id = row.id;
    // The workspace's own Meta app when the account was connected through one, else the server-wide app.
    let http = crate::apps::http_for_account(&s.pool, &s.http, &s.cfg.secret, row).await?;
    if !a.configured(&http) {
        return Err(PublishError::new(format!("{} isn’t configured on this server.", a.label())));
    }
    let video = post.format == "mp4";
    let limits = a.limits();
    if video && !limits.video {
        return Err(PublishError::new(format!("{} doesn’t accept videos from {}.", a.label(), s.cfg.brand_name)));
    }
    if !video && limits.images == 0 {
        return Err(PublishError::new(format!("{} only accepts videos — schedule this design as MP4.", a.label())));
    }
    if char_count(text, Some(a.id())) > limits.max_chars {
        return Err(PublishError::new(format!(
            "The caption is longer than {} allows ({} characters).",
            a.label(),
            limits.max_chars
        )));
    }
    let mime = post_media_mime(&post.format, a);
    log_event(&s.pool, post.id, "rendering", None, Some(json!({ "mime": mime })), Some(account_id)).await.map_err(tx)?;
    let stored = render_media(s, post, info, &mime, render).await?;
    let mut files = load_files(s, &stored).await?;
    files.truncate(if video { 1 } else { limits.images });
    if let Some(max) = limits.max_image_bytes.filter(|m| *m > 0)
        && files.iter().any(|f| f.mime.starts_with("image/") && f.bytes.len() > max)
    {
        return Err(PublishError::new(format!("{} limits images to {} MB — try JPG.", a.label(), megabytes(max))));
    }
    let kind = if video {
        PostKind::Video
    } else if files.len() > 1 {
        PostKind::Carousel
    } else {
        PostKind::Image
    };
    let mut options = post.options.0.clone();
    options.insert("durationMs".into(), num(info.duration_ms));
    let mut ctx = PublishContext {
        account: AccountInfo {
            external_id: row.external_id.clone(),
            display_name: row.display_name.clone(),
            handle: row.handle.clone(),
            meta: row.meta.0.clone(),
        },
        tokens: tokens_for(&s.pool, &http, &s.cfg.secret, row, false, s.now_ms() as f64).await?,
        media: files,
        kind,
        text: text.to_string(),
        hashtags: post.hashtags.clone(),
        options,
        title: info.title.clone(),
    };
    let res = match a.publish(&http, &ctx).await {
        Ok(r) => r,
        // A 401 can mean the token was revoked early: refresh once and try again.
        Err(e) if e.status == Some(401) && a.has_refresh() => {
            ctx.tokens = tokens_for(&s.pool, &http, &s.cfg.secret, row, true, s.now_ms() as f64).await?;
            a.publish(&http, &ctx).await?
        }
        Err(e) => return Err(e),
    };
    if a.id() == "local" {
        send_reminder(s, post, info, &ctx.media, text).await?;
    }
    set_result(s, post, account_id, Target::Published { url: res.url.clone(), note: res.note.clone() }).await?;
    let mut data = Map::new();
    if let Some(x) = &res.external_id {
        data.insert("externalId".into(), json!(x));
    }
    let message = res.url.as_deref().or(res.note.as_deref());
    log_event(&s.pool, post.id, "published", message, Some(Value::Object(data)), Some(account_id)).await.map_err(tx)?;
    Ok(())
}

/// `runPublish(payload, ctx, deps)`. `Err` fails the attempt (the queue retries with backoff); its message
/// becomes the job's error.
pub async fn run_publish(s: &Social, payload: &Value, job: JobInfo, render: &dyn Render) -> Result<Value, PublishError> {
    // `payload.postId` goes to Postgres as text (a malformed id fails the attempt like in TS).
    let post_id: Option<String> = crate::http::non_null(payload.get("postId")).map(|v| crate::http::js_string(Some(v)));
    let post: Option<PostRow> = sqlx::query_as(
        "UPDATE social_posts SET status = 'publishing', job_id = $2, updated_at = now()
     WHERE id = $1::text::uuid AND publisher = 'pinselo' AND ((status = 'scheduled' AND scheduled_at <= now() + interval '30 seconds') OR (status = 'publishing' AND job_id = $2))
     RETURNING *",
    )
    .bind(post_id)
    .bind(job.id)
    .fetch_optional(&s.pool)
    .await
    .map_err(tx)?;
    let Some(mut post) = post else { return Ok(json!({ "skipped": true })) };
    if job.attempts <= 1 {
        log_event(&s.pool, post.id, "publishing", None, None, None).await.map_err(tx)?;
    }
    let last_attempt = job.attempts >= job.max_attempts;

    let Some(info) = design_info(&s.pool, &post.design_id).await? else {
        return finish(s, &mut post, Some("The design was deleted or moved to the trash.")).await;
    };

    let rows: Vec<AccountRow> = sqlx::query_as(
        "SELECT a.* FROM social_accounts a WHERE a.id = ANY($1::uuid[])
       AND (a.user_id = $2 OR a.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $2))",
    )
    .bind(&post.accounts)
    .bind(post.owner_id)
    .fetch_all(&s.pool)
    .await
    .map_err(tx)?;
    let text = compose_text(&post.caption, &post.hashtags);
    let mut pending_transient: Option<String> = None;

    for account_id in post.accounts.clone() {
        let key = account_id.to_string();
        if post.results.0.get(&key).and_then(|r| r.get("status")).and_then(Value::as_str) == Some("published") {
            continue;
        }
        let row = rows.iter().find(|r| r.id == account_id);
        let a = row.and_then(|r| adapter(&r.provider));
        let (Some(row), Some(a)) = (row, a) else {
            set_result(s, &mut post, account_id, Target::Failed("The account was disconnected.".into())).await?;
            log_event(&s.pool, post.id, "failed", Some("Account disconnected"), Some(Value::Null), Some(account_id))
                .await
                .map_err(tx)?;
            continue;
        };
        if let Err(err) = publish_target(s, &mut post, &info, row, a, &text, render).await {
            if err.transient && !last_attempt {
                pending_transient = Some(err.message.clone());
                set_result(s, &mut post, account_id, Target::Retrying(err.message.clone())).await?;
                log_event(&s.pool, post.id, "retry", Some(&err.message), Some(json!({ "attempt": job.attempts })), Some(account_id))
                    .await
                    .map_err(tx)?;
            } else {
                set_result(s, &mut post, account_id, Target::Failed(err.message.clone())).await?;
                log_event(&s.pool, post.id, "failed", Some(&err.message), Some(Value::Null), Some(account_id))
                    .await
                    .map_err(tx)?;
            }
        }
    }
    if let Some(m) = pending_transient {
        // The queue retries with exponential backoff.
        return Err(PublishError::transient(m));
    }
    finish(s, &mut post, None).await
}

async fn finish(s: &Social, post: &mut PostRow, fatal: Option<&str>) -> Result<Value, PublishError> {
    let results = &post.results.0;
    let status_of = |r: &Value| r.get("status").and_then(Value::as_str).map(str::to_string);
    let failed: usize = if fatal.is_some() {
        post.accounts.len()
    } else {
        results.values().filter(|r| status_of(r).as_deref() != Some("published")).count()
    };
    let missing = post.accounts.iter().filter(|id| !crate::http::truthy(results.get(&id.to_string()))).count();
    let ok = fatal.is_none() && failed == 0 && missing == 0;
    let error: Option<String> = match fatal {
        Some(f) => Some(f.to_string()),
        None if ok => None,
        None => {
            let joined = results
                .values()
                .filter(|r| crate::http::truthy(r.get("error")))
                .map(|r| crate::http::js_string(r.get("error")))
                .collect::<Vec<_>>()
                .join(" · ");
            Some(if joined.is_empty() { "Publishing failed".into() } else { joined })
        }
    };
    let status = if ok { "published" } else { "failed" };
    sqlx::query(
        "UPDATE social_posts SET status = $2, error = $3, published_at = CASE WHEN $2 = 'published' THEN now() ELSE published_at END, updated_at = now() WHERE id = $1",
    )
    .bind(post.id)
    .bind(status)
    .bind(&error)
    .execute(&s.pool)
    .await
    .map_err(tx)?;
    if let Some(f) = fatal {
        log_event(&s.pool, post.id, "failed", Some(f), None, None).await.map_err(tx)?;
    }
    notify_owner(s, post, ok, error.as_deref()).await?;
    Ok(json!({ "status": status, "results": Value::Object(post.results.0.clone()) }))
}

// ---------------------------------------------------------------- notifications

async fn owner(s: &Social, post: &PostRow) -> Result<Option<(String, String)>, PublishError> {
    sqlx::query_as("SELECT email, name FROM users WHERE id = $1 AND deleted_at IS NULL")
        .bind(post.owner_id)
        .fetch_optional(&s.pool)
        .await
        .map_err(tx)
}

fn planner_url(s: &Social) -> String {
    format!("{}/planner", s.cfg.web_origin)
}

async fn in_app(s: &Social, post: &PostRow, body: &str) -> Result<(), PublishError> {
    sqlx::query("INSERT INTO notifications(id, user_id, kind, design_id, actor_id, body) VALUES ($1,$2,'publish',$3,NULL,$4)")
        .bind(oc_db::uuid())
        .bind(post.owner_id)
        .bind(&post.design_id)
        .bind(slice_utf16(body, 300))
        .execute(&s.pool)
        .await
        .map(|_| ())
        .map_err(tx)
}

async fn mail(s: &Social, to: &str, t: &oc_mail::Template) -> Result<(), PublishError> {
    oc_mail::send_email(&s.pool, to, t, None).await.map(|_| ()).map_err(|e| match e {
        oc_mail::MailError::Db(e) => tx(e),
        e => PublishError::transient(e.to_string()),
    })
}

async fn send_reminder(s: &Social, post: &PostRow, info: &DesignInfo, files: &[MediaFile], text: &str) -> Result<(), PublishError> {
    in_app(s, post, &format!("Time to post “{}” — your files and caption are ready.", info.title)).await?;
    if let Some((email, _)) = owner(s, post).await? {
        let t = oc_mail::Template::PostReminder {
            title: info.title.clone(),
            caption: text.to_string(),
            url: planner_url(s),
            files: files.iter().map(|f| format!("{}: {}", f.filename, f.url)).collect::<Vec<_>>().join("\n"),
        };
        mail(s, &email, &t).await?;
    }
    log_event(&s.pool, post.id, "reminder", Some(&format!("Reminder sent with {} file(s)", files.len())), None, None)
        .await
        .map_err(tx)
}

async fn notify_owner(s: &Social, post: &PostRow, ok: bool, error: Option<&str>) -> Result<(), PublishError> {
    // "Download & remind"-only posts already notified with the reminder itself.
    let providers: Vec<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, provider, display_name FROM social_accounts WHERE id = ANY($1::uuid[])")
            .bind(&post.accounts)
            .fetch_all(&s.pool)
            .await
            .map_err(tx)?;
    let external: Vec<&(Uuid, String, String)> = providers.iter().filter(|p| p.1 != "local").collect();
    if ok && external.is_empty() {
        return Ok(());
    }
    let title: String = sqlx::query_scalar::<_, String>("SELECT title FROM designs WHERE id = $1")
        .bind(&post.design_id)
        .fetch_optional(&s.pool)
        .await
        .map_err(tx)?
        .unwrap_or_else(|| "Untitled design".into());
    let provider_label = |p: &str| adapter(p).map_or_else(|| p.to_string(), |a| a.label().to_string());
    let label = |id: &str| match providers.iter().find(|p| p.0.to_string() == id) {
        Some(p) => format!("{} ({})", provider_label(&p.1), p.2),
        None => "account".into(),
    };
    if ok {
        let names = external.iter().map(|p| provider_label(&p.1)).collect::<Vec<_>>().join(", ");
        in_app(s, post, &format!("Published “{title}” to {names}.")).await?;
        let links = post
            .results
            .0
            .iter()
            .map(|(id, r)| {
                let target = crate::http::non_null(r.get("url"))
                    .or(crate::http::non_null(r.get("note")))
                    .map_or_else(|| "done".to_string(), |v| crate::http::js_string(Some(v)));
                format!("{}: {target}", label(id))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if let Some((email, _)) = owner(s, post).await? {
            mail(s, &email, &oc_mail::Template::PostPublished { title, links, url: planner_url(s) }).await?;
        }
    } else {
        in_app(s, post, &format!("Couldn’t publish “{title}”: {}", error.unwrap_or("unknown error"))).await?;
        if let Some((email, _)) = owner(s, post).await? {
            let t = oc_mail::Template::PostFailed {
                title,
                error: error.unwrap_or("Unknown error").to_string(),
                url: planner_url(s),
            };
            mail(s, &email, &t).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_and_rounding() {
        assert_eq!(safe_name("Spring sale!"), "Spring sale");
        assert_eq!(safe_name("  Été — 2026  "), "t  2026");
        assert_eq!(safe_name("***"), "design");
        assert_eq!(safe_name(&"x".repeat(80)).len(), 60);
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(megabytes(8 * 1024 * 1024), "8.4");
        assert_eq!(megabytes(1_000_000), "1");
        assert_eq!(megabytes(20 * 1024 * 1024), "21");
    }
}
