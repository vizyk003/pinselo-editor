//! Port of services/api/src/routes/content.ts — the content library (batch 4): emoji stickers, stock media
//! search/import, uploaded brand fonts and the template studio (publish a design as a public or team template).
//!
//! Also exported for other modules: [`families_for`] (misc's font manifest).
//!
//! Engine differences (see ROUTES.md): emoji are rasterised with Rust resvg (same size, different PNG bytes);
//! uploaded fonts are read with a small sfnt reader instead of opentype.js (`name` and `OS/2` tables, the same
//! "English" name records opentype.js picks; a file without TrueType or CFF outlines is corrupt in both).

use std::path::Path as FsPath;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use regex::Regex;
use serde_json::Map;
use tokio::io::AsyncWriteExt as _;

use crate::auth::{at_least, design_role, personal_workspace};
use crate::http::ratelimit::MINUTE;
use crate::http::req::Body;
use crate::prelude::*;
use crate::routes::assets::{FitTo, Ingest, IngestOpts, TempDir, ingest_file, rasterize_svg};
use crate::routes::teams::team_role_str;
use crate::stock::{self, PROVIDERS, Provider};

pub const LIMITS: &[RouteLimit] = &[
    RouteLimit { method: "POST", path: "/api/stock/import", max: 60, window_ms: MINUTE },
    RouteLimit { method: "POST", path: "/api/assets/import-url", max: 120, window_ms: MINUTE },
];

const MAX_STOCK_BYTES: u64 = 300 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/emoji/index.json", get(emoji_index))
        .route("/api/emoji/{hex}", get(emoji_svg))
        .route("/api/stock/providers", get(stock_providers))
        .route("/api/emoji/{hex}/asset", post(emoji_asset))
        .route("/api/stock/search", get(stock_search))
        .route("/api/stock/import", post(stock_import))
        .route("/api/assets/import-url", post(import_url))
        .route("/api/designs/{id}/credits", get(credits))
        .route("/api/fonts/custom", get(fonts_list).post(font_upload))
        .route("/api/fonts/custom/{id}", delete(font_delete))
        .route("/api/designs/{id}/fonts", get(design_fonts))
        .route("/api/templates", post(template_publish))
        .route("/api/templates/{id}", patch(template_update).delete(template_delete))
        .route("/api/admin/templates", get(admin_templates))
        .route("/api/email-images/{file}", get(email_image))
        .route("/api/fonts/files/custom/{file}", get(custom_font_file))
}

static EMOJI_HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9A-F-]{2,60}$").expect("regex"));
static SHA_HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{64}$").expect("regex"));
static FONT_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z0-9_-]+)\.(ttf|otf)$").expect("regex"));

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

fn err_body(error: &str, message: String) -> Value {
    json!({ "error": error, "message": message })
}

// ---------------------------------------------------------------- downloads (stock imports, image URLs)

/// `download(url, hosts, dest)`: public https hosts only, size-capped, redirects re-checked, each connection pinned to
/// the addresses that passed the check (DNS rebinding). Errors are the `Error.message` the TS throws.
pub(crate) async fn download(url: &str, host_ok: impl Fn(&str) -> bool, dest: &FsPath) -> Result<(), String> {
    let mut target = url::Url::parse(url).map_err(|_| "Invalid URL".to_owned())?;
    for _ in 0..4 {
        if target.scheme() != "https" {
            return Err("insecure media URL".into());
        }
        let host = target.host_str().unwrap_or("").to_owned();
        if !host_ok(&host) || !stock::is_public_host(&host) {
            return Err("media host not allowed".into());
        }
        let bare = host.trim_start_matches('[').trim_end_matches(']').to_owned();
        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((bare.as_str(), 443)).await.map_err(|e| format!("getaddrinfo {e} {host}"))?.collect();
        if addrs.is_empty() || addrs.iter().any(|a| !oc_netguard::is_public_ip(a.ip())) {
            return Err("media host resolves to a private address".into());
        }
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).resolve_to_addrs(&bare, &addrs).build().map_err(|e| e.to_string())?;
        let r = client.get(target.clone()).header("user-agent", "Pinselo/1.0").timeout(Duration::from_secs(120)).send().await.map_err(|_| "fetch failed".to_owned())?;
        let status = r.status().as_u16();
        if (300..400).contains(&status)
            && let Some(loc) = r.headers().get("location").and_then(|v| v.to_str().ok())
        {
            target = target.join(loc).map_err(|_| "Invalid URL".to_owned())?;
            continue;
        }
        if !r.status().is_success() {
            return Err(format!("download failed ({status})"));
        }
        let len: u64 = r.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
        if len > MAX_STOCK_BYTES {
            return Err("file too large".into());
        }
        let mut out = tokio::fs::File::create(dest).await.map_err(|e| e.to_string())?;
        let mut n: u64 = 0;
        let mut r = r;
        while let Some(chunk) = r.chunk().await.map_err(|_| "terminated".to_owned())? {
            n += chunk.len() as u64;
            if n > MAX_STOCK_BYTES {
                return Err("file too large".into());
            }
            out.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        out.flush().await.map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err("too many redirects".into())
}

/// `renderTemplateThumb(templateId)`: the render service draws the template's thumbnail; failures are ignored.
async fn render_template_thumb(s: &AppState, template_id: &str) {
    let _ = s
        .http
        .post(format!("{}/internal/template-thumbnail", s.cfg.render_url))
        .header("content-type", "application/json")
        .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&s.cfg.secret))
        .body(crate::http::jsjson::stringify(&json!({ "templateId": template_id })))
        .send()
        .await;
}

// ---------------------------------------------------------------- emoji (public, immutable)

async fn emoji_index(State(s): State<AppState>, _req: Req) -> ApiResult {
    let f = s.paths.emoji.join("index.json");
    if !f.exists() {
        return Ok(Reply::json(json!([])));
    }
    Ok(Reply::bytes("application/json", tokio::fs::read(&f).await?).header("cache-control", "public, max-age=86400"))
}

async fn emoji_svg(State(s): State<AppState>, Path(hex): Path<String>, _req: Req) -> ApiResult {
    let hex = hex.strip_suffix(".svg").unwrap_or(&hex);
    if !EMOJI_HEX.is_match(hex) {
        return Ok(Reply::empty(404));
    }
    let f = s.paths.emoji.join("svg").join(format!("{hex}.svg"));
    if !f.exists() {
        return Ok(Reply::empty(404));
    }
    Ok(Reply::bytes("image/svg+xml", tokio::fs::read(&f).await?).header("cache-control", IMMUTABLE).header("content-security-policy", "default-src 'none'; style-src 'unsafe-inline'"))
}

// ---------------------------------------------------------------- stock catalogue

async fn stock_providers(State(s): State<AppState>, _req: Req) -> ApiResult {
    let providers: Vec<Value> = PROVIDERS.iter().filter(|p| p.enabled(&s)).map(|p| json!({ "id": p.id(), "label": p.label(), "kinds": p.kinds() })).collect();
    Ok(Reply::json(json!({ "providers": providers })))
}

fn find_provider(s: &AppState, id: &str) -> Option<Provider> {
    PROVIDERS.into_iter().find(|p| p.id() == id && p.enabled(s))
}

/// Emoji as a design asset: rasterised once (512 px, transparent), shared by content hash.
async fn emoji_asset(State(s): State<AppState>, Path(hex): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let f = s.paths.emoji.join("svg").join(format!("{hex}.svg"));
    if !EMOJI_HEX.is_match(&hex) || !f.exists() {
        return Ok(not_found());
    }
    let svg = tokio::fs::read_to_string(&f).await?;
    let png = rasterize_svg(&svg, FitTo::Width(512)).map_err(ApiError::internal)?;
    let sha = oc_storage::sha256_hex(&png);
    let name = format!("emoji {hex}");
    let existing = pg::json_one(
        &s.pool,
        "SELECT id, width, height FROM assets WHERE owner_id = $1 AND sha256 = $2 AND source = 'emoji' AND trashed_at IS NULL LIMIT 1",
        &[pg::uuid(u.id), pg::text(&sha)],
    )
    .await?;
    if let Some(e) = existing {
        return Ok(Reply::json(json!({ "asset": { "id": e["id"], "kind": "image", "sha256": sha, "mime": "image/png", "w": e["width"], "h": e["height"], "name": name } })));
    }
    s.store.put(&png, Some("image/png")).await?;
    let id = oc_schema::ids::new_id();
    let ws = personal_workspace(&s, u.id).await?;
    sqlx::query(
        "INSERT INTO assets(id, workspace_id, owner_id, sha256, mime, kind, bytes, width, height, name, source, attribution) VALUES ($1,$2,$3,$4,'image/png','image',$5,512,512,$6,'emoji',$7)",
    )
    .bind(&id)
    .bind(ws)
    .bind(u.id)
    .bind(&sha)
    .bind(png.len() as i64)
    .bind(&name)
    .bind(sqlx::types::Json(json!({ "provider": "noto-emoji", "license": "Apache-2.0", "licenseUrl": "https://github.com/googlefonts/noto-emoji/blob/main/LICENSE" })))
    .execute(&s.pool)
    .await?;
    Ok(Reply::json(json!({ "asset": { "id": id, "kind": "image", "sha256": sha, "mime": "image/png", "w": 512, "h": 512, "name": name } })))
}

async fn stock_search(State(s): State<AppState>, req: Req) -> ApiResult {
    require_user(&s, &req).await?;
    let q = z::obj([
        ("provider", z::str()),
        ("kind", z::enumeration(&stock::KINDS)),
        ("q", z::str().max(100.0).default(json!(""))),
        ("page", z::coerce_num().int().min(1.0).max(50.0).default(json!(1))),
    ])
    .parse(Some(&req.query))?;
    let kind = q["kind"].as_str().unwrap_or("");
    let Some(p) = find_provider(&s, q["provider"].as_str().unwrap_or("")).filter(|p| p.kinds().contains(&kind)) else {
        return Ok(reply::json(404, json!({ "error": "unknown_provider" })));
    };
    let text = crate::js::trim(q["q"].as_str().unwrap_or(""));
    match p.search(&s, kind, text, q["page"].as_f64().unwrap_or(1.0) as i64).await {
        Ok(r) => Ok(Reply::json(r)),
        Err(e) => Ok(reply::json(502, err_body("provider_error", format!("{} is not responding right now. ({e})", p.label())))),
    }
}

async fn stock_import(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("provider", z::str()), ("kind", z::enumeration(&stock::KINDS)), ("id", z::str().max(200.0))]).parse(req.body())?;
    let Some(p) = find_provider(&s, b["provider"].as_str().unwrap_or("")) else {
        return Ok(reply::json(404, json!({ "error": "unknown_provider" })));
    };
    let kind = b["kind"].as_str().unwrap_or("");
    let dir = TempDir::new("oc-stock-").await?;
    let r: Result<Result<Reply, String>, ApiError> = async {
        let m = match p.media(&s, kind, b["id"].as_str().unwrap_or("")).await {
            Ok(m) => m,
            Err(e) => return Ok(Err(e)),
        };
        let src = dir.0.join("src");
        if let Err(e) = download(&m.url, |h| p.host_allowed(h), &src).await {
            return Ok(Err(e));
        }
        let source = if kind == "gif" || kind == "sticker" { "gif" } else { "stock" };
        let r = ingest_file(&s, &src, &dir.0, IngestOpts { name: &m.filename, owner_id: u.id, ip: Some(&req.ip), source: Some(source), attribution: Some(m.attribution.clone()), skip_quota: false }).await?;
        Ok(Ok(match r {
            Ingest::Error { status, error, message } => reply::json(status, ingest_error(error, message)),
            Ingest::Asset(mut a) => {
                if let Some(o) = a.as_object_mut() {
                    o.insert("attribution".into(), m.attribution);
                }
                Reply::json(json!({ "asset": a }))
            }
        }))
    }
    .await;
    match r {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(msg)) => Ok(reply::json(502, err_body("import_failed", format!("Couldn’t import from {}: {msg}", p.label())))),
        // Anything else thrown in the try block (database, storage) is caught by the same handler.
        Err(e) => Ok(reply::json(502, err_body("import_failed", format!("Couldn’t import from {}: {}", p.label(), error_message(&e))))),
    }
}

/// JS truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn ingest_error(error: &str, message: Option<String>) -> Value {
    let mut m = Map::new();
    m.insert("error".into(), json!(error));
    if let Some(msg) = message {
        m.insert("message".into(), json!(msg));
    }
    Value::Object(m)
}

fn error_message(e: &ApiError) -> String {
    match e {
        ApiError::Internal(m) => m.clone(),
        ApiError::Status { message, .. } => message.clone(),
        _ => "internal".into(),
    }
}

/// Import an image from a public https URL (bulk create image columns). Private networks are refused.
async fn import_url(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("url", z::str().url().max(2000.0))]).parse(req.body())?;
    let url = b["url"].as_str().unwrap_or("").to_owned();
    let dir = TempDir::new("oc-url-").await?;
    let r: Result<Result<Reply, String>, ApiError> = async {
        let src = dir.0.join("src");
        if let Err(e) = download(&url, |_| true, &src).await {
            return Ok(Err(e));
        }
        let last = url::Url::parse(&url).ok().and_then(|x| x.path().rsplit('/').next().map(str::to_owned)).unwrap_or_default();
        let last = if last.is_empty() { "image".to_owned() } else { last };
        let Some(name) = crate::http::cookie::decode_uri_component(&last) else { return Ok(Err("URI malformed".into())) };
        let name = crate::js::take(&name, 120);
        let r = ingest_file(&s, &src, &dir.0, IngestOpts { name: &name, owner_id: u.id, ip: Some(&req.ip), source: Some("upload"), attribution: Some(json!({ "provider": "url", "sourceUrl": url })), skip_quota: false }).await?;
        Ok(Ok(match r {
            Ingest::Error { status, error, message } => reply::json(status, ingest_error(error, message)),
            Ingest::Asset(a) => Reply::json(json!({ "asset": a })),
        }))
    }
    .await;
    match r {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(msg)) => Ok(reply::json(400, err_body("fetch_failed", format!("Couldn’t fetch that image: {msg}")))),
        Err(e) => Ok(reply::json(400, err_body("fetch_failed", format!("Couldn’t fetch that image: {}", error_message(&e))))),
    }
}

/// Credits for every licensed asset a design uses (shown in Share/Download, included in exports' notes).
async fn credits(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, oc_auth::Role::View) {
        return Ok(not_found());
    }
    let st: Option<Vec<u8>> = sqlx::query_scalar("SELECT state FROM doc_state WHERE design_id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some(st) = st else { return Ok(Reply::json(json!({ "credits": [] }))) };
    let doc = oc_crdt::decode_doc_value(&st)?;
    let mut ids: Vec<Option<String>> = Vec::new();
    let mut add = |v: &Value| {
        let id = v.as_str().map(str::to_owned);
        if !ids.contains(&id) {
            ids.push(id);
        }
    };
    for e in doc["elements"].as_object().into_iter().flat_map(Map::values) {
        if e.get("deleted").is_some_and(|d| !matches!(d, Value::Null | Value::Bool(false))) {
            continue;
        }
        match e["type"].as_str() {
            Some("image" | "video") => add(&e["asset"]),
            Some("frame") if e.get("content").is_some_and(|c| !c.is_null()) => add(&e["content"]["asset"]),
            Some("grid") => {
                for c in e["cells"].as_array().into_iter().flatten() {
                    if truthy(&c["asset"]) {
                        add(&c["asset"]);
                    }
                }
            }
            _ => {}
        }
    }
    for a in doc["meta"]["audio"].as_array().into_iter().flatten() {
        add(&a["asset"]);
    }
    let rows = if ids.is_empty() {
        vec![]
    } else {
        let list: Vec<Option<String>> = ids;
        sqlx::query_as::<_, (String, String, Option<sqlx::types::Json<Value>>)>("SELECT id, name, attribution FROM assets WHERE id = ANY($1) AND attribution IS NOT NULL").bind(&list).fetch_all(&s.pool).await?
    };
    let credits: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, attribution)| {
            let mut m = Map::new();
            m.insert("asset".into(), json!(id));
            m.insert("name".into(), json!(name));
            if let Some(Value::Object(a)) = attribution.map(|a| a.0) {
                for (k, v) in a {
                    m.insert(k, v);
                }
            }
            Value::Object(m)
        })
        .collect();
    Ok(Reply::json(json!({ "credits": credits })))
}

// ---------------------------------------------------------------- uploaded fonts (brand kit, FT-020)

async fn fonts_list(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let fonts = pg::json_rows(
        &s.pool,
        "SELECT f.id, f.family, f.weight, f.italic, f.workspace_id AS \"workspaceId\", w.name AS workspace, f.created_at FROM custom_fonts f
         JOIN workspaces w ON w.id = f.workspace_id JOIN memberships m ON m.workspace_id = f.workspace_id AND m.user_id = $1 ORDER BY f.family, f.weight",
        &[pg::uuid(u.id)],
    )
    .await?;
    Ok(Reply::json(json!({ "fonts": fonts })))
}

/// Every part of a multipart body (`for await (const part of req.parts())`): the last file's name and bytes and the
/// fields. A file over `fileSize` (Fastify's `bodyLimit`, 10 MiB) is `RequestFileTooLargeError` (413).
async fn read_parts(req: &mut Req) -> Result<(Option<(String, Vec<u8>)>, Map<String, Value>), ApiError> {
    let Body::Multipart { content_type, body } = std::mem::replace(&mut req.body, Body::None) else {
        return Err(ApiError::status(406, "the request is not multipart"));
    };
    let Some(body) = body.into_inner().ok().flatten() else {
        return Err(ApiError::status(406, "the request is not multipart"));
    };
    let boundary = multer::parse_boundary(&content_type).map_err(|_| ApiError::status(406, "the request is not multipart"))?;
    let mut mp = multer::Multipart::new(body.into_data_stream(), boundary);
    let mut file = None;
    let mut fields = Map::new();
    let limit = crate::http::req::BODY_LIMIT;
    loop {
        let mut field = match mp.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return Err(ApiError::status(400, e.to_string())),
        };
        let name = field.name().unwrap_or("").to_owned();
        match field.file_name().map(str::to_owned) {
            Some(filename) => {
                let mut bytes = Vec::new();
                let mut over = false;
                while let Some(chunk) = field.chunk().await.map_err(|e| ApiError::status(400, e.to_string()))? {
                    if bytes.len() + chunk.len() > limit {
                        over = true;
                    } else {
                        bytes.extend_from_slice(&chunk);
                    }
                }
                if over {
                    return Err(ApiError::status(413, "request file too large"));
                }
                file = Some((filename, bytes));
            }
            None => {
                let v = field.text().await.map_err(|e| ApiError::status(400, e.to_string()))?;
                fields.insert(name, json!(v));
            }
        }
    }
    Ok((file, fields))
}

#[derive(Default)]
struct BuiltinFamilies(Mutex<Option<Arc<Vec<String>>>>);

/// The lower-cased names of every built-in family (`fullManifest(paths.fonts)`: pinned + Google catalogue).
fn builtin_families(s: &AppState) -> Result<Arc<Vec<String>>, String> {
    let cache = s.local::<BuiltinFamilies>();
    let mut cur = cache.0.lock().expect("families");
    // fullManifest re-reads manifest.json every time; the catalogue is loaded once.
    let pinned: Value = serde_json::from_slice(&std::fs::read(s.paths.fonts.join("manifest.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut names: Vec<String> = pinned["families"].as_array().into_iter().flatten().filter_map(|f| f["family"].as_str()).map(str::to_lowercase).collect();
    let catalog = match cur.as_ref() {
        Some(c) => c.clone(),
        None => {
            let f = s.paths.fonts.join("google-catalog.json");
            let list: Vec<String> = if f.exists() {
                let v: Value = serde_json::from_slice(&std::fs::read(&f).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                v.as_array().into_iter().flatten().filter_map(|f| f["family"].as_str()).map(str::to_lowercase).collect()
            } else {
                vec![]
            };
            let c = Arc::new(list);
            *cur = Some(c.clone());
            c
        }
    };
    names.extend(catalog.iter().cloned());
    Ok(Arc::new(names))
}

async fn font_upload(State(s): State<AppState>, mut req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let (file, fields) = read_parts(&mut req).await?;
    let Some((filename, bytes)) = file else { return Ok(reply::json(400, json!({ "error": "no_file" }))) };
    let field = |k: &str| fields.get(k).and_then(Value::as_str).map(str::to_owned);
    if field("licenseAck").as_deref() != Some("true") {
        return Ok(reply::json(400, err_body("license", "Confirm you have the right to use this font.".into())));
    }
    let ws = match field("workspaceId").filter(|w| !w.is_empty()) {
        Some(w) => w,
        None => personal_workspace(&s, u.id).await?.to_string(),
    };
    let role = team_role_str(&s, &u.id.to_string(), &ws).await?;
    let Some(role) = role.filter(|r| r != "member") else { return Ok(reply::json(403, json!({ "error": "forbidden" }))) };
    // Brand controls (A8): "restrict uploads" leaves brand fonts to owners and admins.
    let ws_uuid = Uuid::parse_str(&ws).ok();
    if role == "designer" && crate::brandhub::control_on(&crate::brandhub::brand_controls(&s, ws_uuid).await?, "restrictUploads") {
        return Ok(reply::json(403, err_body("uploads_restricted", "Your team only lets owners and admins upload brand fonts.".into())));
    }
    let magic = bytes.get(..4).unwrap_or(&bytes[..]);
    let format = match magic {
        b"OTTO" => "otf",
        [0, 1, 0, 0] | b"true" => "ttf",
        _ => return Ok(reply::json(415, err_body("unsupported_font", "Upload a TTF or OTF font file.".into()))),
    };
    if bytes.len() > 15 * 1024 * 1024 {
        return Ok(reply::json(413, json!({ "error": "too_large" })));
    }
    let corrupt = || Ok(reply::json(415, err_body("corrupt_font", "That font file could not be read.".into())));
    let Some(info) = sfnt::read(&bytes) else { return corrupt() };
    let base = Regex::new(r"\.\w+$").expect("regex").replace(&filename, "").into_owned();
    let mut family = crate::js::take(crate::js::trim(&info.preferred_family.clone().or(info.family.clone()).unwrap_or(base)), 70);
    // Never shadow a built-in family: an uploaded "Lobster" becomes "Lobster (uploaded)".
    let Ok(pinned) = builtin_families(&s) else { return corrupt() };
    if pinned.contains(&family.to_lowercase()) {
        family = format!("{family} (uploaded)");
    }
    let weight = ((f64::from(info.weight.unwrap_or(400)) / 100.0 + 0.5).floor() * 100.0) as i32;
    static ITALIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)italic|oblique").expect("regex"));
    let sub = info.preferred_subfamily.clone().or(info.subfamily.clone()).unwrap_or_default();
    let italic = info.fs_selection.unwrap_or(0) & 1 != 0 || ITALIC.is_match(&sub);
    let sha = s.store.put(&bytes, Some(if format == "otf" { "font/otf" } else { "font/ttf" })).await?;
    let id = oc_schema::ids::new_id();
    let row: String = sqlx::query_scalar(
        "INSERT INTO custom_fonts(id, workspace_id, uploaded_by, family, weight, italic, sha256, bytes, format, license_ack) VALUES ($1,$2::uuid,$3,$4,$5,$6,$7,$8,$9,true)
         ON CONFLICT (workspace_id, sha256) DO UPDATE SET family = EXCLUDED.family RETURNING id",
    )
    .bind(&id)
    .bind(&ws)
    .bind(u.id)
    .bind(&family)
    .bind(weight)
    .bind(italic)
    .bind(&sha)
    .bind(bytes.len() as i32)
    .bind(format)
    .fetch_one(&s.pool)
    .await?;
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent {
            actor_id: Some(u.id),
            workspace_id: ws_uuid,
            action: "brand.font_uploaded".into(),
            meta: Some(json!({ "family": family, "weight": weight, "italic": italic })),
            ip: Some(req.ip.clone()),
            ..Default::default()
        },
        s.now(),
    )
    .await?;
    Ok(Reply::json(json!({ "font": { "id": row, "family": family, "weight": weight, "italic": italic, "file": format!("custom/{row}.{format}"), "sha256": sha, "bytes": bytes.len() } })))
}

async fn font_delete(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let ws: Option<Uuid> = sqlx::query_scalar("SELECT workspace_id FROM custom_fonts WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let role = match ws {
        Some(w) => crate::routes::teams::team_role(&s, u.id, w).await?,
        None => None,
    };
    if ws.is_none() || role.is_none_or(|r| r == "member") {
        return Ok(Reply::empty(404));
    }
    sqlx::query("DELETE FROM custom_fonts WHERE id = $1").bind(&id).execute(&s.pool).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

/// Uploaded families a design can use: those of its workspace (collaborators from elsewhere see them too).
async fn design_fonts(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, oc_auth::Role::View) {
        return Ok(not_found());
    }
    let families = families_for(&s, "SELECT f.* FROM custom_fonts f JOIN designs d ON d.workspace_id = f.workspace_id WHERE d.id = $1", &[pg::text(&id)]).await?;
    Ok(Reply::json(json!({ "families": families })))
}

/// `familiesFor(sql, params)`: custom font rows grouped into manifest families (first face per weight/style wins).
pub async fn families_for(s: &AppState, sql: &str, args: &[Arg]) -> Result<Vec<Value>, ApiError> {
    let rows = pg::json_rows(&s.pool, sql, args).await?;
    let mut order: Vec<String> = Vec::new();
    let mut by: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for r in rows {
        let family = r["family"].as_str().unwrap_or("").to_owned();
        let f = by.entry(family.clone()).or_insert_with(|| {
            order.push(family.clone());
            json!({ "family": family, "slug": format!("custom-{}", r["id"].as_str().unwrap_or("")), "category": "uploaded", "license": "Uploaded", "faces": [] })
        });
        let faces = f["faces"].as_array_mut().expect("faces");
        if !faces.iter().any(|x| x["weight"] == r["weight"] && x["italic"] == r["italic"]) {
            faces.push(json!({
                "weight": r["weight"], "italic": r["italic"],
                "file": format!("custom/{}.{}", r["id"].as_str().unwrap_or(""), r["format"].as_str().unwrap_or("")),
                "sha256": r["sha256"], "bytes": r["bytes"],
            }));
        }
    }
    Ok(order.into_iter().filter_map(|k| by.remove(&k)).collect())
}

// ---------------------------------------------------------------- template studio (TP-040)

async fn template_publish(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([
        ("designId", z::str()),
        ("title", z::str().min(1.0).max(120.0)),
        ("category", z::str().min(1.0).max(60.0)),
        ("tags", z::array(z::str().max(30.0)).max(20.0).default(json!([]))),
        ("description", z::str().max(500.0).optional()),
        ("scope", z::enumeration(&["public", "team"])),
        ("premium", z::bool().optional()),
        // Brand template (A8): elements locked `by: 'brand'` stay locked for members who start from it.
        ("brand", z::bool().optional()),
    ])
    .parse(req.body())?;
    let design_id = b["designId"].as_str().unwrap_or("").to_owned();
    let d: Option<(Uuid, bool)> =
        sqlx::query_as("SELECT d.workspace_id, w.personal FROM designs d JOIN workspaces w ON w.id = d.workspace_id WHERE d.id = $1").bind(&design_id).fetch_optional(&s.pool).await?;
    let Some((dws, personal)) = d else { return Ok(not_found()) };
    if !at_least(design_role(&s, Some(u.id), &design_id).await?, oc_auth::Role::Edit) {
        return Ok(not_found());
    }
    let scope = b["scope"].as_str().unwrap_or("");
    if scope == "public" && !u.is_staff {
        return Ok(reply::json(403, err_body("forbidden", "Only staff can publish to the public catalogue.".into())));
    }
    if scope == "team" {
        if personal {
            return Ok(reply::json(400, err_body("team_only", "Move the design into a team to publish it as a team template.".into())));
        }
        let role = crate::routes::teams::team_role(&s, u.id, dws).await?;
        if role.is_none_or(|r| r == "member") {
            return Ok(reply::json(403, err_body("forbidden", "Designers, admins and owners can publish team templates.".into())));
        }
    }
    let st: Option<Vec<u8>> = sqlx::query_scalar("SELECT state FROM doc_state WHERE design_id = $1").bind(&design_id).fetch_optional(&s.pool).await?;
    let Some(st) = st else { return Ok(reply::json(400, err_body("empty", "Open the design once before publishing it.".into()))) };
    let doc = oc_crdt::decode_doc_value(&st)?;
    if doc["pages"].as_object().is_none_or(Map::is_empty) {
        return Ok(reply::json(400, json!({ "error": "empty" })));
    }
    let id = format!("t_{}", oc_schema::ids::new_id());
    let brand = scope == "team" && b.get("brand").and_then(Value::as_bool) == Some(true);
    let title = b["title"].as_str().unwrap_or("");
    let category = b["category"].as_str().unwrap_or("");
    let tags: Vec<String> = b["tags"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    let description = b.get("description").and_then(Value::as_str);
    let mut search = vec![title.to_owned(), category.to_owned()];
    search.extend(tags.iter().cloned());
    search.push(description.unwrap_or("").to_owned());
    let locks = if brand { Value::Object(crate::brandhub::brand_locks_of(&doc)) } else { json!({}) };
    sqlx::query(
        "INSERT INTO templates(id, title, category, preset, tags, search_text, width, height, doc, published, premium, workspace_id, created_by, description, brand, locks, source_design_id)
         VALUES ($1,$2,$3,'custom',$4,$5,$6,$7,$8,true,$9,$10,$11,$12,$13,$14,$15)",
    )
    .bind(&id)
    .bind(title)
    .bind(category)
    .bind(&tags)
    .bind(search.join(" "))
    .bind(doc["meta"]["dims"]["w"].as_f64())
    .bind(doc["meta"]["dims"]["h"].as_f64())
    .bind(sqlx::types::Json(&doc))
    .bind(scope == "public" && b.get("premium").and_then(Value::as_bool) == Some(true))
    .bind(if scope == "team" { Some(dws) } else { None })
    .bind(u.id)
    .bind(description)
    .bind(brand)
    .bind(sqlx::types::Json(&locks))
    .bind(if scope == "team" { Some(design_id.as_str()) } else { None })
    .execute(&s.pool)
    .await?;
    render_template_thumb(&s, &id).await;
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent {
            actor_id: Some(u.id),
            workspace_id: if scope == "team" { Some(dws) } else { None },
            action: "template.published".into(),
            target: Some(id.clone()),
            meta: Some(json!({ "scope": scope })),
            ip: Some(req.ip.clone()),
        },
        s.now(),
    )
    .await?;
    Ok(Reply::json(json!({ "id": id })))
}

/// The template's workspace (`None` = the public catalogue), or `Err(404)` when there is no such template; then the
/// edit rule: team templates need a designer or above, public ones staff.
async fn template_access(s: &AppState, u: &User, id: &str) -> Result<Result<(), Reply>, ApiError> {
    let t: Option<(Option<Uuid>,)> = sqlx::query_as("SELECT workspace_id FROM templates WHERE id = $1").bind(id).fetch_optional(&s.pool).await?;
    let Some((ws,)) = t else { return Ok(Err(Reply::empty(404))) };
    let allowed = match ws {
        Some(w) => crate::routes::teams::team_role(s, u.id, w).await?.is_some_and(|r| r != "member"),
        None => u.is_staff,
    };
    Ok(if allowed { Ok(()) } else { Err(reply::json(403, json!({ "error": "forbidden" }))) })
}

async fn template_update(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([
        ("title", z::str().min(1.0).max(120.0).optional()),
        ("category", z::str().max(60.0).optional()),
        ("tags", z::array(z::str().max(30.0)).max(20.0).optional()),
        ("published", z::bool().optional()),
        ("premium", z::bool().optional()),
        ("description", z::str().max(500.0).optional()),
    ])
    .parse(req.body())?;
    type Row = (Option<Uuid>, String, String, Vec<String>, Option<String>);
    let t: Option<Row> = sqlx::query_as("SELECT workspace_id, title, category, tags, description FROM templates WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some((_, ttitle, tcategory, ttags, tdescription)) = t else { return Ok(Reply::empty(404)) };
    if let Err(r) = template_access(&s, &u, &id).await? {
        return Ok(r);
    }
    let bs = |k: &str| b.get(k).and_then(Value::as_str).map(str::to_owned);
    let title = bs("title").unwrap_or(ttitle);
    let category = bs("category").unwrap_or(tcategory);
    let tags: Vec<String> = match b.get("tags").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        None => ttags,
    };
    let mut search = vec![title.clone(), category.clone()];
    search.extend(tags.iter().cloned());
    search.push(bs("description").or(tdescription).unwrap_or_default());
    sqlx::query(
        "UPDATE templates SET title = $2, category = $3, tags = $4, search_text = $5, published = COALESCE($6, published),
           premium = CASE WHEN workspace_id IS NULL THEN COALESCE($7, premium) ELSE false END, description = COALESCE($8, description), updated_at = now() WHERE id = $1",
    )
    .bind(&id)
    .bind(&title)
    .bind(&category)
    .bind(&tags)
    .bind(search.join(" "))
    .bind(b.get("published").and_then(Value::as_bool))
    .bind(b.get("premium").and_then(Value::as_bool))
    .bind(bs("description"))
    .execute(&s.pool)
    .await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn template_delete(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if let Err(r) = template_access(&s, &u, &id).await? {
        return Ok(r);
    }
    sqlx::query("DELETE FROM templates WHERE id = $1").bind(&id).execute(&s.pool).await?;
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: Some(u.id), action: "template.deleted".into(), target: Some(id.clone()), ip: Some(req.ip.clone()), ..Default::default() },
        s.now(),
    )
    .await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn admin_templates(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !u.is_staff {
        return Ok(Reply::empty(403));
    }
    let templates = pg::json_rows(
        &s.pool,
        "SELECT id, title, category, tags, published, premium, uses, workspace_id, created_at, updated_at FROM templates WHERE workspace_id IS NULL ORDER BY updated_at DESC LIMIT 500",
        &[],
    )
    .await?;
    Ok(Reply::json(json!({ "templates": templates })))
}

// ---------------------------------------------------------------- public files

/// Email export slices: content-addressed, public and immutable (email clients fetch them anonymously).
async fn email_image(State(s): State<AppState>, Path(file): Path<String>, _req: Req) -> ApiResult {
    let sha = file.strip_suffix(".png").unwrap_or(&file);
    if !SHA_HEX.is_match(sha) || pg::one(&s.pool, "SELECT 1 FROM email_images WHERE sha256 = $1", &[pg::text(sha)]).await?.is_none() {
        return Ok(Reply::empty(404));
    }
    Ok(Reply::bytes("image/png", s.store.get(sha).await?).header("cache-control", IMMUTABLE).header("access-control-allow-origin", "*"))
}

/// Uploaded font files: unguessable ids, public like asset bytes (viewers of shared designs need them).
async fn custom_font_file(State(s): State<AppState>, Path(file): Path<String>, _req: Req) -> ApiResult {
    let f: Option<(String, String)> = match FONT_FILE.captures(&file) {
        Some(c) => sqlx::query_as("SELECT sha256, format FROM custom_fonts WHERE id = $1").bind(&c[1]).fetch_optional(&s.pool).await?,
        None => None,
    };
    let Some((sha, format)) = f else { return Ok(Reply::empty(404)) };
    Ok(Reply::bytes(if format == "otf" { "font/otf" } else { "font/ttf" }, s.store.get(&sha).await?)
        .header("cache-control", IMMUTABLE)
        // Any origin, no credentials: sandboxed published sites (opaque origin, SEC-02) load these with CORS too.
        .header("access-control-allow-origin", "*"))
}

/// What the font upload reads from an sfnt file (opentype.js `font.names.*.en` and `tables.os2`).
mod sfnt {
    pub struct Info {
        pub family: Option<String>,
        pub subfamily: Option<String>,
        pub preferred_family: Option<String>,
        pub preferred_subfamily: Option<String>,
        pub weight: Option<u16>,
        pub fs_selection: Option<u16>,
    }

    fn u16_at(b: &[u8], i: usize) -> Option<u16> {
        Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]))
    }

    fn u32_at(b: &[u8], i: usize) -> Option<u32> {
        Some(u32::from_be_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
    }

    /// Mac OS Roman, 0x80..=0xFF.
    const MAC_ROMAN: &str = "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»…\u{a0}ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{f8ff}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";

    fn mac_roman(b: &[u8]) -> String {
        let high: Vec<char> = MAC_ROMAN.chars().collect();
        b.iter().map(|&c| if c < 0x80 { c as char } else { high.get(usize::from(c - 0x80)).copied().unwrap_or('\u{fffd}') }).collect()
    }

    /// `opentype.parse(bytes)`: `None` where opentype.js throws (truncated directory, no TrueType/CFF outlines).
    pub fn read(b: &[u8]) -> Option<Info> {
        let n = usize::from(u16_at(b, 4)?);
        let mut tables = std::collections::HashMap::new();
        for i in 0..n {
            let e = 12 + 16 * i;
            let tag = b.get(e..e + 4)?;
            let off = u32_at(b, e + 8)? as usize;
            let len = u32_at(b, e + 12)? as usize;
            if off.checked_add(len)? > b.len() {
                return None;
            }
            tables.insert(tag.to_vec(), &b[off..off + len]);
        }
        let has = |t: &[u8]| tables.contains_key(t);
        let outlines = (has(b"glyf") && has(b"loca") && has(b"head") && has(b"maxp")) || has(b"CFF ");
        if !outlines {
            return None;
        }
        let mut info = Info { family: None, subfamily: None, preferred_family: None, preferred_subfamily: None, weight: None, fs_selection: None };
        if let Some(name) = tables.get(b"name".as_slice()) {
            let count = usize::from(u16_at(name, 2)?);
            let strings = usize::from(u16_at(name, 4)?);
            for i in 0..count {
                let r = 6 + 12 * i;
                let (platform, encoding, language, id) = (u16_at(name, r)?, u16_at(name, r + 2)?, u16_at(name, r + 4)?, u16_at(name, r + 6)?);
                let (len, off) = (usize::from(u16_at(name, r + 8)?), usize::from(u16_at(name, r + 10)?));
                let Some(raw) = name.get(strings + off..strings + off + len) else { continue };
                // opentype.js keys a record by language; only Mac English (0) and Windows en-US (0x409) are `en`.
                let text = match (platform, encoding, language) {
                    (3, 1 | 10, 0x409) => {
                        let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                        String::from_utf16_lossy(&units)
                    }
                    (1, 0, 0) => mac_roman(raw),
                    _ => continue,
                };
                if text.is_empty() {
                    continue;
                }
                match id {
                    1 => info.family = Some(text),
                    2 => info.subfamily = Some(text),
                    16 => info.preferred_family = Some(text),
                    17 => info.preferred_subfamily = Some(text),
                    _ => {}
                }
            }
        }
        if let Some(os2) = tables.get(b"OS/2".as_slice()) {
            info.weight = u16_at(os2, 4);
            info.fs_selection = u16_at(os2, 62);
        }
        Some(info)
    }
}
