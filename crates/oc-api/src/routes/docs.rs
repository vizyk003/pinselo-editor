//! Port of services/api/src/routes/docs.ts — Docs (DC-030..033): embedded-design access, page pictures for
//! embeds, and Word import.
//!
//! * `GET /api/designs/:id/pages/:page` — a design page as PNG through the render service (viewers only), with
//!   the TS's 200-entry in-process cache keyed by design version.
//! * `POST /api/docs/import-docx` — Word → HTML for "Import into a new doc" (the editor turns the HTML into doc
//!   blocks). The conversion is `oc_import::docx` (a port of the mammoth conversion with the route's style map);
//!   pictures in the file become the user's uploads, external links to files are never followed.
//!
//! Also exported: [`embed_allow_list`].

use std::sync::{LazyLock, Mutex};

use regex::Regex;

use crate::http::multipart;
use crate::prelude::*;
use crate::routes::assets::{Ingest, IngestOpts, TempDir, ingest_file};

/// Routes with their own `config.rateLimit`.
pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/designs/{id}/pages/{page}", get(page_png)).route("/api/docs/import-docx", post(import_docx))
}

/// Small cache of rendered page pictures, keyed by design version.
#[derive(Default)]
struct PageCache(Mutex<(std::collections::HashMap<String, bytes::Bytes>, std::collections::VecDeque<String>)>);

/// `Map.set` + evict the oldest entries beyond 200.
fn remember(c: &PageCache, k: String, v: bytes::Bytes) {
    let mut g = c.0.lock().expect("page cache");
    let (map, order) = &mut *g;
    if map.insert(k.clone(), v).is_none() {
        order.push_back(k);
    }
    while map.len() > 200 {
        let Some(old) = order.pop_front() else { break };
        map.remove(&old);
    }
}

/// JS `Number(value)` for a query string value (`undefined` → NaN).
fn js_number(v: Option<&Value>) -> f64 {
    let Some(Value::String(s)) = v else { return f64::NAN };
    let t = crate::js::trim(s);
    if t.is_empty() {
        return 0.0;
    }
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(h, 16).map_or(f64::NAN, |v| v as f64);
    }
    match t {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ if t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')) => t.parse::<f64>().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

/// A design page as PNG (`page` = page id, or a 1-based number; `.png` optional). Viewers of the design only.
async fn page_png(State(s): State<AppState>, Path((id, page)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    static PAGE_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\w-]{1,64}$").expect("regex"));
    let page_id = page.strip_suffix(".png").unwrap_or(&page).to_owned();
    let n = js_number(req.query.get("w"));
    let n = if n.is_nan() || n == 0.0 { 800.0 } else { n };
    let w = 64f64.max(2400f64.min(crate::js::round(n)));
    if !PAGE_ID.is_match(&page_id) || !auth::at_least(auth::design_role(&s, Some(u.id), &id).await?, oc_auth::Role::View) {
        return Ok(not_found());
    }
    let d: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT updated_at FROM designs WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some(updated) = d else { return Ok(not_found()) };
    let key = format!("{id}:{page_id}:{}:{}", crate::js::num(w), updated.timestamp_millis());
    let cache = s.local::<PageCache>();
    let cc = |r: Reply| r.header("cache-control", "private, max-age=30");
    let hit = cache.0.lock().expect("page cache").0.get(&key).cloned();
    if let Some(hit) = hit {
        return Ok(cc(Reply::bytes("image/png", hit)));
    }
    let res = s
        .http
        .post(format!("{}/internal/page-png", s.cfg.render_url))
        .header("content-type", "application/json")
        .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&s.cfg.secret))
        .body(crate::http::jsjson::stringify(&json!({ "designId": id, "pageId": page_id, "w": w })))
        .send()
        .await
        .ok();
    let Some(res) = res else { return Ok(cc(reply::json(502, json!({ "error": "render_unavailable" })))) };
    if !res.status().is_success() {
        return Ok(cc(if res.status().as_u16() == 404 { not_found() } else { reply::json(502, json!({ "error": "render_unavailable" })) }));
    }
    let buf = res.bytes().await?;
    remember(&cache, key, buf.clone());
    Ok(cc(Reply::bytes("image/png", buf)))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Word → HTML for "Import into a new doc". Pictures become the user's uploads (at most 100).
async fn import_docx(State(s): State<AppState>, mut req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let up = TempDir::new("oc-docx-up-").await?;
    let src = up.0.join("src");
    let Some(file) = multipart::save_file(&mut req, 50 * 1024 * 1024, &src).await? else {
        return Ok(reply::json(400, json!({ "error": "no_file" })));
    };
    let buf = tokio::fs::read(&src).await?;
    if file.truncated {
        return Ok(reply::json(413, json!({ "error": "too_large", "message": "Word imports are limited to 50 MB." })));
    }
    if !(buf.first() == Some(&0x50) && buf.get(1) == Some(&0x4b) && contains(&buf, b"word/document.xml")) {
        return Ok(reply::json(415, json!({ "error": "unsupported_type", "message": "Import a Word (.docx) file." })));
    }
    let dir = TempDir::new("oc-docx-").await?;
    let (s2, dir_path, ip, owner) = (s.clone(), dir.0.clone(), req.ip.clone(), u.id);
    let handle = tokio::runtime::Handle::current();
    let result = tokio::task::spawn_blocking(move || {
        let mut n = 0usize;
        oc_import::docx::docx_to_html(&buf, &mut |image: &oc_import::docx::DocxImage<'_>| {
            if n >= 100 {
                return String::new();
            }
            let f = dir_path.join(format!("img-{n}"));
            n += 1;
            if std::fs::write(&f, image.bytes).is_err() {
                return String::new();
            }
            let name = format!("Imported image {n}");
            match handle.block_on(ingest_file(&s2, &f, &dir_path, IngestOpts { name: &name, owner_id: owner, ip: Some(&ip), source: None, attribution: None, skip_quota: false })) {
                Ok(Ingest::Asset(a)) => format!("/api/assets/{}", a["id"].as_str().unwrap_or("")),
                _ => String::new(),
            }
        })
    })
    .await
    .map_err(ApiError::internal)?;
    drop(dir);
    let Ok(r) = result else {
        return Ok(reply::json(422, json!({ "error": "unreadable", "message": "That Word file could not be read." })));
    };
    static DOCX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.docx$").expect("regex"));
    let title = crate::js::take(&DOCX.replace(&file.filename, ""), 200);
    let title = if title.is_empty() { "Imported doc".to_owned() } else { title };
    Ok(Reply::json(json!({ "html": r.html, "title": title, "warnings": r.messages.len() })))
}

// ---------------------------------------------------------------- shared helpers (ported)

/// `embedAllowList(userId, docId)`: designs embedded in `docId` that `userId` may view (empty for non-doc designs).
pub async fn embed_allow_list(state: &AppState, user_id: Option<uuid::Uuid>, doc_id: &str) -> Result<Vec<String>, crate::http::error::ApiError> {
    let r: Option<(Vec<u8>, String)> =
        sqlx::query_as("SELECT s.state, d.surface FROM doc_state s JOIN designs d ON d.id = s.design_id WHERE s.design_id = $1").bind(doc_id).fetch_optional(&state.pool).await?;
    match r {
        Some((st, surface)) if surface == "doc" => crate::doc_access::viewable_embeds(state, user_id, &st).await,
        _ => Ok(vec![]),
    }
}
