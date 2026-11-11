//! JPEG exports of the REST API for tools that publish designs unattended (e.g. to Instagram): `POST
//! /v1/designs/:id/exports` with `format: "jpeg"`, `GET /v1/exports/exp_…` and the public file links. See
//! docs/rest-api.md.
//!
//! - **Version locking.** A design's content version (`designs.content_version`, +1 for every change in the op log)
//!   can be sent with the export; when the design has moved on, the export is refused with 409 `version_changed`
//!   instead of rendering. The version is read again after the last page is rendered, so a change made while
//!   rendering also fails the export (the render service reads the snapshot and the op-log tail in one statement).
//! - **Files Instagram accepts.** Baseline JPEG, flattened onto white, tagged sRGB, 320–1440 px wide; quality steps
//!   down by 5 (not below 60) until a page is under 7.5 MiB. Fonts must load: a page whose font cannot be loaded
//!   fails the export instead of rendering a fallback face.
//! - **Public links.** Each file has an HMAC-signed link valid for 24 hours that needs no sign-in (Meta's servers
//!   fetch it), served as `image/jpeg` from a path ending in `.jpg`. Revoking the token stops its links.
//! - **Waiting, idempotency, quota.** `wait: true` holds the request up to 25 s (then 202 and poll); an
//!   `Idempotency-Key` returns the same export for 24 hours; a token may start 200 exports a day.
//!
//! Exports render in the API process (one render-service call per page); one interrupted by a restart is reported as
//! failed after 10 minutes.

use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::prelude::*;
use crate::routes::v1::{ApiUser, can_see};

/// Export ids: `exp_` + 16 random bytes, base64url.
pub const ID_PREFIX: &str = "exp_";
const MAX_PAGES: usize = 10;
const DEFAULT_WIDTH: i64 = 1080;
const MIN_WIDTH: i64 = 320;
const MAX_WIDTH: i64 = 1440;
const DEFAULT_QUALITY: i64 = 90;
const MIN_QUALITY: i64 = 60;
/// Instagram takes up to 8 MB; stay under 7.5 MiB.
const MAX_BYTES: usize = 7 * 1024 * 1024 + 512 * 1024;
const WAIT: Duration = Duration::from_secs(25);
const LINK_TTL_S: i64 = 24 * 3600;
const DAILY_EXPORTS: i64 = 200;
const PAGE_TIMEOUT: Duration = Duration::from_secs(120);

/// A page of a design as the API lists it.
pub struct PageInfo {
    /// 1-based position in the design.
    pub index: usize,
    pub id: String,
    pub name: String,
    pub hidden: bool,
}

/// The design's current content (snapshot plus op-log tail): its surface, size and pages in order.
struct Snapshot {
    surface: String,
    w: f64,
    h: f64,
    pages: Vec<PageInfo>,
}

async fn snapshot(s: &AppState, id: &str) -> Result<Option<Snapshot>, ApiError> {
    let Some(ydoc) = crate::brandhub::load_ydoc(s, id).await? else { return Ok(None) };
    let doc = oc_crdt::decode_doc(&oc_crdt::encode_state(&ydoc))?;
    let pages = oc_schema::tree::sorted_pages(&doc.pages)
        .iter()
        .enumerate()
        .map(|(i, p)| PageInfo {
            index: i + 1,
            id: p.id.to_string(),
            name: p.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| format!("Page {}", i + 1)),
            hidden: p.hidden == Some(true),
        })
        .collect();
    Ok(Some(Snapshot { surface: doc.meta.surface.as_str().to_owned(), w: doc.meta.dims.w, h: doc.meta.dims.h, pages }))
}

/// The design's pages in order (`None`: no document).
pub async fn current_pages(s: &AppState, id: &str) -> Result<Option<Vec<PageInfo>>, ApiError> {
    Ok(snapshot(s, id).await?.map(|x| x.pages))
}

/// `designs.content_version`.
pub async fn content_version(s: &AppState, id: &str) -> Result<Option<i64>, ApiError> {
    Ok(sqlx::query_scalar("SELECT content_version FROM designs WHERE id = $1").bind(id).fetch_optional(&s.pool).await?)
}

/// `format` is `jpeg` (or `jpg`): this module's export, not a queued job.
pub fn is_jpeg_request(req: &Req) -> bool {
    matches!(req.body().and_then(|b| b.get("format")).and_then(Value::as_str), Some("jpeg" | "jpg"))
}

/// An error the publisher acts on: `{ error: code, message, … }` with the status of its code.
#[derive(Debug, Clone)]
struct Fail {
    code: &'static str,
    message: String,
    current_version: Option<i64>,
}

impl Fail {
    fn new(code: &'static str, message: impl Into<String>) -> Fail {
        Fail { code, message: message.into(), current_version: None }
    }

    fn status(&self) -> u16 {
        status_of(self.code)
    }

    fn json(&self) -> Value {
        let mut v = json!({ "code": self.code, "message": self.message });
        if let Some(c) = self.current_version {
            v["currentVersion"] = json!(c);
        }
        v
    }

    fn reply(&self) -> Reply {
        let mut v = json!({ "error": self.code, "message": self.message });
        if let Some(c) = self.current_version {
            v["currentVersion"] = json!(c);
        }
        reply::json(self.status(), v)
    }
}

fn status_of(code: &str) -> u16 {
    match code {
        "version_changed" => 409,
        "invalid_export" => 422,
        "unavailable" => 503,
        _ => 500,
    }
}

fn invalid(message: impl Into<String>) -> Reply {
    Fail::new("invalid_export", message).reply()
}

fn version_changed(current: i64, asked: i64, during: bool) -> Fail {
    let message = if during {
        format!("Design changed to version {current} while exporting version {asked}")
    } else {
        format!("Design is at version {current}, export asked for {asked}")
    };
    Fail { code: "version_changed", message, current_version: Some(current) }
}

/// `POST /v1/designs/:id/exports` with `format: "jpeg"` (scope and access already checked).
pub(crate) async fn start(s: &AppState, u: &ApiUser, req: &Req, id: &str) -> ApiResult {
    let b = req.body().cloned().unwrap_or(Value::Null);
    let Some(b) = b.as_object() else { return Ok(invalid("Send a JSON object.")) };
    for k in b.keys() {
        if !matches!(k.as_str(), "format" | "version" | "width" | "quality" | "pages" | "wait") {
            return Ok(invalid(format!("Unknown field “{k}”.")));
        }
    }
    let int = |k: &str| -> Result<Option<i64>, Reply> {
        match b.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_i64().map(Some).ok_or_else(|| invalid(format!("“{k}” must be a whole number."))),
        }
    };
    let (version, width, quality) = match (int("version"), int("width"), int("quality")) {
        (Ok(v), Ok(w), Ok(q)) => (v, w.unwrap_or(DEFAULT_WIDTH), q.unwrap_or(DEFAULT_QUALITY)),
        (Err(r), _, _) | (_, Err(r), _) | (_, _, Err(r)) => return Ok(r),
    };
    if !(MIN_WIDTH..=MAX_WIDTH).contains(&width) {
        return Ok(invalid(format!("“width” must be {MIN_WIDTH} to {MAX_WIDTH} px (Instagram's range), got {width}.")));
    }
    if !(MIN_QUALITY..=100).contains(&quality) {
        return Ok(invalid(format!("“quality” must be {MIN_QUALITY} to 100, got {quality}.")));
    }
    let wait = match b.get("wait") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(w)) => *w,
        Some(_) => return Ok(invalid("“wait” must be true or false.")),
    };
    let asked_pages: Option<Vec<i64>> = match b.get("pages") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => match a.iter().map(Value::as_i64).collect::<Option<Vec<i64>>>() {
            Some(p) => Some(p),
            None => return Ok(invalid("“pages” must be page numbers (1 is the first page).")),
        },
        Some(_) => return Ok(invalid("“pages” must be a list of page numbers.")),
    };
    let key = match req.header("idempotency-key").map(str::trim) {
        Some(k) if k.is_empty() || k.len() > 200 || !k.bytes().all(|c| c.is_ascii_graphic()) => {
            return Ok(invalid("The Idempotency-Key header must be 1 to 200 visible ASCII characters."));
        }
        k => k.map(str::to_owned),
    };

    // The same key within 24 hours: the export it started (no new render, same links).
    if let Some(k) = &key {
        sqlx::query("UPDATE api_exports SET idempotency_key = NULL WHERE token_id = $1 AND idempotency_key = $2 AND created_at < now() - interval '24 hours'")
            .bind(u.token_id)
            .bind(k)
            .execute(&s.pool)
            .await?;
        if let Some(r) = replay(s, u, req, id, k, wait).await? {
            return Ok(r);
        }
    }

    let Some(current) = content_version(s, id).await? else { return Ok(crate::routes::v1::not_found()) };
    if let Some(v) = version
        && v != current
    {
        return Ok(version_changed(current, v, false).reply());
    }
    let Some(snap) = snapshot(s, id).await? else { return Ok(crate::routes::v1::not_found()) };
    if matches!(snap.surface.as_str(), "doc" | "sheet") {
        return Ok(invalid(format!("JPEG export is for designs with fixed pages, not a {}.", snap.surface)));
    }
    let n = snap.pages.len();
    let pages: Vec<usize> = match &asked_pages {
        Some(p) => {
            let mut out: Vec<usize> = Vec::new();
            for &x in p {
                let Some(i) = usize::try_from(x).ok().filter(|i| (1..=n).contains(i)) else {
                    return Ok(invalid(format!("Page {x} is out of range: the design has {n} page{}.", if n == 1 { "" } else { "s" })));
                };
                if out.contains(&i) {
                    return Ok(invalid(format!("Page {i} is listed twice.")));
                }
                out.push(i);
            }
            out
        }
        None => snap.pages.iter().filter(|p| !p.hidden).map(|p| p.index).collect(),
    };
    if pages.is_empty() {
        return Ok(invalid("No pages to export."));
    }
    if pages.len() > MAX_PAGES {
        return Ok(invalid(format!("At most {MAX_PAGES} pages per export (Instagram's carousel limit); asked for {}.", pages.len())));
    }

    // 200 exports a day per token (replays do not count).
    let (count, retry): (i64, Option<f64>) = sqlx::query_as(
        "SELECT count(*), extract(epoch FROM min(created_at) + interval '24 hours' - now())::float8 FROM api_exports WHERE token_id = $1 AND created_at > now() - interval '24 hours'",
    )
    .bind(u.token_id)
    .fetch_one(&s.pool)
    .await?;
    if count >= DAILY_EXPORTS {
        let secs = retry.unwrap_or(3600.0).ceil().max(1.0) as i64;
        return Ok(reply::json(429, json!({ "error": "rate_limited", "message": format!("This token started {DAILY_EXPORTS} exports in the last 24 hours. Retry in {secs} s.") }))
            .header("retry-after", secs.to_string()));
    }

    let mut warnings: Vec<Value> = Vec::new();
    let ratio = snap.w / snap.h.max(1.0);
    if ratio < 0.8 - 1e-3 {
        warnings.push(json!({ "code": "aspect_ratio", "message": format!("The design is {}×{} px, taller than 4:5: Instagram's feed crops or refuses it.", crate::js::num(snap.w), crate::js::num(snap.h)) }));
    } else if ratio > 1.91 + 1e-3 {
        warnings.push(json!({ "code": "aspect_ratio", "message": format!("The design is {}×{} px, wider than 1.91:1: Instagram's feed crops or refuses it.", crate::js::num(snap.w), crate::js::num(snap.h)) }));
    }

    let export_id = format!("{ID_PREFIX}{}", URL_SAFE_NO_PAD.encode(crate::auth::random_bytes(16)));
    let request = json!({ "format": "jpeg", "version": version, "width": width, "quality": quality, "pages": pages, "wait": wait });
    let inserted: Option<String> = sqlx::query_scalar(
        "INSERT INTO api_exports(id, token_id, user_id, design_id, design_version, idempotency_key, request, warnings) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT (token_id, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING RETURNING id",
    )
    .bind(&export_id)
    .bind(u.token_id)
    .bind(u.id)
    .bind(id)
    .bind(current)
    .bind(&key)
    .bind(&request)
    .bind(Value::Array(warnings))
    .fetch_optional(&s.pool)
    .await?;
    if inserted.is_none() {
        // Another request with the same key won the race.
        if let Some(k) = &key
            && let Some(r) = replay(s, u, req, id, k, wait).await?
        {
            return Ok(r);
        }
        return Err(ApiError::internal("export not recorded"));
    }
    let pool = s.pool.clone();
    s.spawn(async move {
        let _ = sqlx::query("DELETE FROM api_exports WHERE created_at < now() - interval '7 days'").execute(&pool).await;
    });
    let job = Job { export_id: export_id.clone(), design_id: id.to_owned(), user_id: u.id, token_id: u.token_id, version: current, pages, width, quality };
    let st = s.clone();
    s.spawn(async move { run(st, job).await });
    respond(s, u, req, &export_id, wait, true).await
}

/// The export an idempotency key started, if any.
async fn replay(s: &AppState, u: &ApiUser, req: &Req, id: &str, key: &str, wait: bool) -> Result<Option<Reply>, ApiError> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT id, design_id FROM api_exports WHERE token_id = $1 AND idempotency_key = $2").bind(u.token_id).bind(key).fetch_optional(&s.pool).await?;
    let Some((export_id, design)) = row else { return Ok(None) };
    if design != id {
        return Ok(Some(invalid(format!("This Idempotency-Key was used for design {design} in the last 24 hours."))));
    }
    respond(s, u, req, &export_id, wait, true).await.map(Some)
}

/// `GET /v1/exports/exp_…`.
pub(crate) async fn status(s: &AppState, u: &ApiUser, req: &Req, export_id: &str) -> ApiResult {
    respond(s, u, req, export_id, false, false).await
}

/// The export as the API shows it, waiting up to 25 s for it to finish when `wait`. `post`: a failed export answers
/// with its error's status (the publisher acts on it); `GET` answers 200 with `status: "failed"`.
async fn respond(s: &AppState, u: &ApiUser, _req: &Req, export_id: &str, wait: bool, post: bool) -> ApiResult {
    let deadline = Instant::now() + if wait { WAIT } else { Duration::ZERO };
    loop {
        sqlx::query(
            "UPDATE api_exports SET status = 'failed', finished_at = now(), error = jsonb_build_object('code', 'render_failed', 'message', 'The export was interrupted. Start it again.')
             WHERE id = $1 AND status = 'rendering' AND created_at < now() - interval '10 minutes'",
        )
        .bind(export_id)
        .execute(&s.pool)
        .await?;
        let row = pg::json_one(
            &s.pool,
            "SELECT id, user_id, design_id, design_version::text AS design_version, status, files, warnings, error, created_at, expires_at,
                    extract(epoch FROM expires_at)::float8 AS exp_s
             FROM api_exports WHERE id = $1",
            &[pg::text(export_id)],
        )
        .await?;
        let Some(row) = row.filter(|r| r["user_id"].as_str() == Some(&u.id.to_string())) else { return Ok(crate::routes::v1::not_found()) };
        let design = row["design_id"].as_str().unwrap_or("").to_owned();
        if !can_see(s, u, &design).await? {
            return Ok(crate::routes::v1::not_found());
        }
        let status = row["status"].as_str().unwrap_or("").to_owned();
        if status == "rendering" && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(300)).await;
            continue;
        }
        let out = export_json(s, &row);
        return Ok(match status.as_str() {
            "rendering" => Reply::json(out).status(202),
            "failed" if post => {
                let code = row["error"]["code"].as_str().unwrap_or("render_failed");
                let mut body = out;
                body["error"] = json!(code);
                body["message"] = row["error"]["message"].clone();
                if let Some(c) = row["error"].get("currentVersion") {
                    body["currentVersion"] = c.clone();
                }
                reply::json(status_of(code), body)
            }
            _ => Reply::json(out),
        });
    }
}

fn export_json(s: &AppState, row: &Value) -> Value {
    let id = row["id"].as_str().unwrap_or("");
    let mut out = json!({
        "id": id,
        "status": row["status"],
        "designId": row["design_id"],
        "designVersion": row["design_version"].as_str().and_then(|v| v.parse::<i64>().ok()),
        "createdAt": row["created_at"],
        "statusUrl": format!("{}/v1/exports/{id}", s.cfg.web_origin),
        "warnings": if row["warnings"].is_array() { row["warnings"].clone() } else { json!([]) },
    });
    if row["status"] == "done" {
        let exp = row["exp_s"].as_f64().unwrap_or(0.0) as i64;
        let files: Vec<Value> = row["files"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|f| {
                        let page = f["page"].as_u64().unwrap_or(0);
                        json!({
                            "page": page, "url": file_url(s, id, page, exp), "width": f["width"], "height": f["height"],
                            "bytes": f["bytes"], "contentType": "image/jpeg",
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        out["files"] = Value::Array(files);
        out["expiresAt"] = row["expires_at"].clone();
    }
    if row["status"] == "failed" {
        out["error"] = row["error"].clone();
    }
    out
}

// ---------------------------------------------------------------- rendering

struct Job {
    export_id: String,
    design_id: String,
    user_id: Uuid,
    token_id: Uuid,
    version: i64,
    /// 1-based page positions, in the order asked for.
    pages: Vec<usize>,
    width: i64,
    quality: i64,
}

async fn run(s: AppState, job: Job) {
    let t0 = Instant::now();
    let r = render_all(&s, &job).await;
    let r = match r {
        // The version again: an edit while rendering could have reached some pages and not others.
        Ok(files) => match content_version(&s, &job.design_id).await {
            Ok(Some(v)) if v == job.version => Ok(files),
            Ok(Some(v)) => Err(version_changed(v, job.version, true)),
            Ok(None) => Err(Fail::new("render_failed", "The design was deleted while exporting.")),
            Err(e) => Err(Fail::new("unavailable", format!("{e:?}"))),
        },
        Err(f) => Err(f),
    };
    let (status, detail) = match &r {
        Ok(f) => ("done", format!("{} files, {} bytes", f.len(), f.iter().map(|x| x["bytes"].as_u64().unwrap_or(0)).sum::<u64>())),
        Err(f) => ("failed", f.code.to_owned()),
    };
    let res = match r {
        Ok(files) => {
            sqlx::query("UPDATE api_exports SET status = 'done', files = $2, finished_at = now(), expires_at = now() + make_interval(secs => $3) WHERE id = $1")
                .bind(&job.export_id)
                .bind(Value::Array(files))
                .bind(LINK_TTL_S as f64)
                .execute(&s.pool)
                .await
        }
        Err(f) => sqlx::query("UPDATE api_exports SET status = 'failed', error = $2, finished_at = now() WHERE id = $1").bind(&job.export_id).bind(f.json()).execute(&s.pool).await,
    };
    if let Err(e) = res {
        eprintln!("[api] export {} could not be saved: {e}", job.export_id);
    }
    // Never the token or the signed links.
    println!(
        "[api] v1 export {} token={} design={} version={} pages={} {}ms {status} ({detail})",
        job.export_id,
        job.token_id,
        job.design_id,
        job.version,
        job.pages.len(),
        t0.elapsed().as_millis()
    );
}

/// Every page: render, shrink under the size limit, tag sRGB, store. `files` rows in the order asked for.
async fn render_all(s: &AppState, job: &Job) -> Result<Vec<Value>, Fail> {
    let embeds = crate::routes::docs::embed_allow_list(s, Some(job.user_id), &job.design_id).await.map_err(|e| Fail::new("unavailable", format!("{e:?}")))?;
    let mut files = Vec::new();
    for &page in &job.pages {
        let mut quality = job.quality;
        let jpeg = loop {
            let bytes = render_page(s, &job.design_id, page, job.width, quality, &embeds).await?;
            if bytes.len() <= MAX_BYTES {
                break bytes;
            }
            if quality - 5 < MIN_QUALITY {
                return Err(Fail::new(
                    "invalid_export",
                    format!("Page {page} is {:.1} MB even at quality {quality}; Instagram takes up to 8 MB. Simplify the page or export it narrower.", bytes.len() as f64 / 1_048_576.0),
                ));
            }
            quality -= 5;
        };
        let jpeg = with_srgb_profile(&jpeg);
        let Some((w, h)) = jpeg_size(&jpeg) else { return Err(Fail::new("render_failed", format!("Page {page} did not render as a JPEG."))) };
        let sha = s.store.put(&jpeg, Some("image/jpeg")).await.map_err(|e| Fail::new("unavailable", format!("Could not store page {page}: {e}")))?;
        files.push(json!({ "page": page, "sha": sha, "width": w, "height": h, "bytes": jpeg.len(), "quality": quality }));
    }
    Ok(files)
}

/// One page (1-based) as a JPEG from the render service, fonts strictly loaded.
async fn render_page(s: &AppState, design_id: &str, page: usize, width: i64, quality: i64, embeds: &[String]) -> Result<Vec<u8>, Fail> {
    let body = json!({
        "designId": design_id, "format": "jpg", "pages": [page - 1], "width": width, "quality": quality,
        "transparent": false, "strictFonts": true, "embeds": embeds,
    });
    let res = s
        .http
        .post(format!("{}/internal/export", s.cfg.render_url))
        .header("content-type", "application/json")
        .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&s.cfg.secret))
        .body(crate::http::jsjson::stringify(&body))
        .timeout(PAGE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Fail::new("unavailable", "The render service is not reachable."))?;
    let status = res.status();
    if !status.is_success() {
        let t: Value = res.bytes().await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
        let msg = t["message"].as_str().or(t["error"].as_str()).unwrap_or("unknown error").to_owned();
        return Err(if t["error"] == "fonts_unavailable" {
            Fail::new("render_failed", format!("Page {page}: {msg}. Nothing was rendered with a fallback font."))
        } else {
            Fail::new("render_failed", format!("Page {page} failed to render: {}", crate::js::take(&msg, 300)))
        });
    }
    let ty = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
    if ty != "image/jpeg" {
        return Err(Fail::new("render_failed", format!("Page {page} came back as {ty}, not a JPEG.")));
    }
    res.bytes().await.map(|b| b.to_vec()).map_err(|e| Fail::new("unavailable", e.to_string()))
}

// ---------------------------------------------------------------- JPEG bytes

/// The JPEG with an sRGB ICC profile (APP2 `ICC_PROFILE`) after its SOI/JFIF header, so viewers and Instagram read
/// the colours as sRGB. Unchanged when it is not a JPEG or already carries a profile.
pub fn with_srgb_profile(jpeg: &[u8]) -> Vec<u8> {
    if jpeg.len() < 4 || jpeg[..2] != [0xFF, 0xD8] || segments(jpeg).any(|(m, d)| m == 0xE2 && d.starts_with(b"ICC_PROFILE\0")) {
        return jpeg.to_vec();
    }
    let mut at = 2;
    if jpeg[2] == 0xFF && jpeg[3] == 0xE0 && jpeg.len() >= 6 {
        at = (4 + usize::from(u16::from_be_bytes([jpeg[4], jpeg[5]]))).min(jpeg.len());
    }
    let icc = oc_color::registry::SRGB_ICC;
    let len = 2 + 12 + 2 + icc.len();
    let mut out = Vec::with_capacity(jpeg.len() + len + 2);
    out.extend_from_slice(&jpeg[..at]);
    out.extend_from_slice(&[0xFF, 0xE2]);
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.extend_from_slice(b"ICC_PROFILE\0");
    out.extend_from_slice(&[1, 1]);
    out.extend_from_slice(icc);
    out.extend_from_slice(&jpeg[at..]);
    out
}

/// The marker segments before the scan: `(marker, payload)`.
fn segments(jpeg: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut i = 2;
    std::iter::from_fn(move || {
        if i + 4 > jpeg.len() || jpeg[i] != 0xFF {
            return None;
        }
        let m = jpeg[i + 1];
        if m == 0xDA || m == 0xD9 {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]));
        let end = (i + 2 + len).min(jpeg.len());
        let data = &jpeg[(i + 4).min(end)..end];
        i = end;
        Some((m, data))
    })
}

/// Width and height from the frame header; `None` unless it is a baseline (SOF0) JPEG.
pub fn jpeg_size(jpeg: &[u8]) -> Option<(u32, u32)> {
    if jpeg.len() < 4 || jpeg[..2] != [0xFF, 0xD8] {
        return None;
    }
    let (_, d) = segments(jpeg).find(|(m, _)| (0xC0..=0xCF).contains(m) && !matches!(m, 0xC4 | 0xC8 | 0xCC))
        .filter(|(m, _)| *m == 0xC0)?;
    (d.len() >= 5).then(|| (u32::from(u16::from_be_bytes([d[3], d[4]])), u32::from(u16::from_be_bytes([d[1], d[2]]))))
}

// ---------------------------------------------------------------- public links

fn sign(secret: &str, export_id: &str, page: u64, exp: i64) -> String {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("hmac key");
    m.update(format!("api-export.{export_id}.{page}.{exp}").as_bytes());
    URL_SAFE_NO_PAD.encode(m.finalize().into_bytes())
}

/// `…/v1/files/exp_…/<page>.jpg?e=<expiry>&sig=<signature>`.
fn file_url(s: &AppState, export_id: &str, page: u64, exp: i64) -> String {
    format!("{}/v1/files/{export_id}/{page}.jpg?e={exp}&sig={}", s.cfg.web_origin, sign(&s.cfg.secret, export_id, page, exp))
}

/// `GET /v1/files/:export/:page.jpg`: no sign-in (Meta's servers fetch it), signed, until the export's expiry, while
/// the token that made it is live.
pub async fn public_file(State(s): State<AppState>, Path((export_id, file)): Path<(String, String)>, req: Req) -> ApiResult {
    let gone = || reply::json(404, json!({ "error": "not_found", "message": "This link is invalid or has expired. Export the design again." }));
    let Some(page) = file.strip_suffix(".jpg").and_then(|p| p.parse::<u64>().ok()) else { return Ok(gone()) };
    let (Some(e), Some(sig)) = (req.query_str("e").and_then(|e| e.parse::<i64>().ok()), req.query_str("sig")) else { return Ok(gone()) };
    if e < s.now_ms().div_euclid(1000) {
        return Ok(gone());
    }
    let want = sign(&s.cfg.secret, &export_id, page, e);
    use subtle::ConstantTimeEq as _;
    if !(want.len() == sig.len() && bool::from(want.as_bytes().ct_eq(sig.as_bytes()))) {
        return Ok(gone());
    }
    let row: Option<(Uuid, Uuid, Option<Value>)> =
        sqlx::query_as("SELECT user_id, token_id, files FROM api_exports WHERE id = $1 AND status = 'done' AND expires_at > now()").bind(&export_id).fetch_optional(&s.pool).await?;
    let Some((user, token, Some(files))) = row else { return Ok(gone()) };
    let Some(sha) = files.as_array().and_then(|a| a.iter().find(|f| f["page"].as_u64() == Some(page))).and_then(|f| f["sha"].as_str()).map(str::to_owned) else {
        return Ok(gone());
    };
    if !crate::mcp::files::grant_live(&s, &user.to_string(), &token.to_string()).await? {
        return Ok(gone());
    }
    let bytes = s.store.get(&sha).await?;
    Ok(Reply::bytes("image/jpeg", bytes)
        .header("content-disposition", format!("inline; filename=\"{export_id}-{page}.jpg\""))
        .header("cache-control", "private, max-age=3600"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SOI, JFIF APP0, SOF0 (1080×1350, 3 components), SOS, EOI.
    fn tiny_jpeg() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        v.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        v.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x05, 0x46, 0x04, 0x38, 0x03, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        v.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x12, 0x34, 0xFF, 0xD9]);
        v
    }

    #[test]
    fn reads_baseline_size() {
        assert_eq!(jpeg_size(&tiny_jpeg()), Some((1080, 1350)));
        let mut progressive = tiny_jpeg();
        let i = progressive.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        progressive[i + 1] = 0xC2;
        assert_eq!(jpeg_size(&progressive), None);
        assert_eq!(jpeg_size(b"\x89PNG"), None);
    }

    #[test]
    fn tags_srgb_after_jfif_once() {
        let j = tiny_jpeg();
        let t = with_srgb_profile(&j);
        assert_eq!(&t[..20], &j[..20]);
        assert_eq!(&t[20..22], &[0xFF, 0xE2]);
        assert_eq!(&t[24..36], b"ICC_PROFILE\0");
        assert_eq!(t.len(), j.len() + 2 + 2 + 12 + 2 + oc_color::registry::SRGB_ICC.len());
        assert_eq!(jpeg_size(&t), Some((1080, 1350)));
        assert_eq!(with_srgb_profile(&t), t);
    }

    #[test]
    fn signatures_bind_export_page_and_expiry() {
        let a = sign("k", "exp_a", 1, 100);
        assert_eq!(a, sign("k", "exp_a", 1, 100));
        assert_ne!(a, sign("k", "exp_a", 2, 100));
        assert_ne!(a, sign("k", "exp_b", 1, 100));
        assert_ne!(a, sign("k", "exp_a", 1, 101));
        assert_ne!(a, sign("other", "exp_a", 1, 100));
    }

    #[test]
    fn edit_links_resolve_to_ids() {
        use crate::routes::v1::design_id_param;
        assert_eq!(design_id_param("WgYq99H11jFflr5B"), "WgYq99H11jFflr5B");
        assert_eq!(design_id_param("https://pinselo.com/design/WgYq99H11jFflr5B?page=2"), "WgYq99H11jFflr5B");
        assert_eq!(design_id_param("https://pinselo.com/design/WgYq99H11jFflr5B/edit#x"), "WgYq99H11jFflr5B");
    }
}
