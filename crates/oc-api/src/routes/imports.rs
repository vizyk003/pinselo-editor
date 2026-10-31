//! Port of services/api/src/routes/imports.ts — imports (IM-050/060): upload a PDF or PowerPoint file; a background
//! job (`import` queue, drained in the API process like the TS `work('import', runImport)`) turns it into an
//! editable design with the importers of `oc-import`.
//!
//! * `POST /api/imports`
//! * `POST /api/designs/:id/doc/convert`
//! * `GET /api/imports/:job`
//!
//! The worker is started by the binary (`start_worker`, like the plugin registration in TS) and, idempotently, by
//! the first import request (so an app built without the binary's start-up, e.g. the parity harness, still
//! drains the queue).

use std::sync::{Arc, OnceLock};

use serde_json::Map;

use crate::http::multipart;
use crate::prelude::*;
use crate::routes::assets::{Ingest, IngestOpts, TempDir, ingest_file};
use crate::routes::designs::{CreateOpts, create_design_from_doc};
use crate::routes::docs::embed_allow_list;

/// Routes with their own `config.rateLimit`.
pub const LIMITS: &[RouteLimit] = &[];

const QUEUE: &str = "import";
const MAX_IMPORT: u64 = 200 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/imports", post(create))
        .route("/api/designs/{id}/doc/convert", post(convert))
        .route("/api/imports/{job}", get(status))
}

#[derive(Default)]
struct WorkerSlot(OnceLock<oc_jobs::Worker>);

/// `work<ImportJob>('import', runImport, { concurrency: 1 })`, once per app state.
pub fn start_worker(s: &AppState) {
    let slot = s.local::<WorkerSlot>();
    slot.0.get_or_init(|| {
        let s2 = s.clone();
        oc_jobs::work(
            s.pool.clone(),
            QUEUE,
            move |payload: Value, ctx: oc_jobs::JobContext| {
                let s = s2.clone();
                async move { run_import(&s, &payload, &ctx).await.map_err(oc_jobs::HandlerError::from) }
            },
            oc_jobs::WorkOpts { concurrency: 1 },
        )
    });
}

#[derive(Default)]
struct Families(OnceLock<Arc<std::collections::HashSet<String>>>);

/// Bundled and Google Fonts families: imported decks keep their fonts whenever Google has them.
async fn pinned_families(s: &AppState) -> Result<Arc<std::collections::HashSet<String>>, String> {
    let slot = s.local::<Families>();
    if let Some(f) = slot.0.get() {
        return Ok(f.clone());
    }
    let g = oc_gfonts::GoogleFonts::new(s.paths.fonts.clone(), s.pool.clone(), s.store.clone());
    let m = g.full_manifest().await.map_err(|e| e.to_string())?;
    let set: std::collections::HashSet<String> = m.families.iter().map(|f| f.family.clone()).collect();
    Ok(slot.0.get_or_init(|| Arc::new(set)).clone())
}

/// The job error text (TS: `error.message`; a database error carries Postgres' message, as with node-postgres).
fn err_text(e: ApiError) -> String {
    match e {
        ApiError::Internal(m) => m.strip_prefix("error returned from database: ").map(str::to_owned).unwrap_or(m),
        other => format!("{other:?}"),
    }
}

/// `runImport(payload, ctx)`.
async fn run_import(s: &AppState, p: &Value, ctx: &oc_jobs::JobContext) -> Result<Value, String> {
    let sha = p["sha"].as_str().unwrap_or("");
    let kind = p["kind"].as_str().unwrap_or("pptx").to_owned();
    let name = p["name"].as_str().unwrap_or("").to_owned();
    let user_id = p["userId"].as_str().and_then(|u| Uuid::parse_str(u).ok()).ok_or("invalid userId")?;
    let bytes = s.store.get(sha).await.map_err(|e| e.to_string())?;
    let fams = pinned_families(s).await?;
    let dir = TempDir::new("oc-import-").await.map_err(|e| e.to_string())?;
    ctx.progress(0.05, Some(if kind == "pdf" { "Reading PDF" } else { "Reading presentation" })).await.map_err(|e| e.to_string())?;
    static EXT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\.(pdf|pptx)$").expect("regex"));
    let title = crate::js::take(&EXT.replace(&name, ""), 120);

    // The importers are synchronous (CPU-bound); media go through the async ingest pipeline from a blocking thread.
    let handle = tokio::runtime::Handle::current();
    let (s2, ctx2, dir_path, is_pdf) = (s.clone(), ctx.clone(), dir.0.clone(), kind == "pdf");
    let result = tokio::task::spawn_blocking(move || {
        let mut assets: Vec<Value> = Vec::new();
        let mut n = 0usize;
        let mut media = |data: &[u8], mname: &str| -> Option<String> {
            n += 1;
            let f = dir_path.join(format!("m{n}"));
            std::fs::write(&f, data).ok()?;
            let r = handle.block_on(ingest_file(&s2, &f, &dir_path, IngestOpts { name: mname, owner_id: user_id, ip: None, source: None, attribution: None, skip_quota: false }));
            let Ok(Ingest::Asset(a)) = r else { return None };
            let id = a["id"].as_str().map(str::to_owned);
            assets.push(a);
            if n.is_multiple_of(3) {
                let _ = handle.block_on(ctx2.progress(0.85f64.min(0.1 + n as f64 / 100.0), Some(&format!("Imported {n} images"))));
            }
            id
        };
        let available = |f: &str| fams.contains(f);
        let opts = oc_import::ImportOptions { font_available: &available, title: Some(title) };
        let doc = if is_pdf { oc_import::import_pdf(&bytes, &opts, &mut media) } else { oc_import::import_pptx(&bytes, &opts, &mut media) };
        doc.map(|d| (d, assets))
    })
    .await
    .map_err(|e| e.to_string())?;
    let (doc, assets) = result.map_err(|e| e.to_string())?;
    let mut doc = oc_schema::js::to_value(&doc).map_err(|e| e.to_string())?;
    if let Some(m) = doc.get_mut("assets").and_then(Value::as_object_mut) {
        for a in assets {
            if let Some(id) = a["id"].as_str() {
                m.insert(id.to_owned(), a.clone());
            }
        }
    }
    ctx.progress(0.92, Some("Creating design")).await.map_err(|e| e.to_string())?;
    // node-postgres hands the folder id to Postgres as text, which rejects a malformed one.
    let folder_id = match p.get("folderId").and_then(Value::as_str) {
        Some(f) => Some(Uuid::parse_str(f).map_err(|_| format!("invalid input syntax for type uuid: \"{f}\""))?),
        None => None,
    };
    let workspace_id = match p.get("workspaceId").and_then(Value::as_str) {
        Some(w) => Some(Uuid::parse_str(w).map_err(|_| format!("invalid input syntax for type uuid: \"{w}\""))?),
        None => None,
    };
    let design_id = create_design_from_doc(s, user_id, &doc, CreateOpts { folder_id, workspace_id, ..Default::default() }).await.map_err(err_text)?;
    let pages = doc.get("pages").and_then(Value::as_object).map_or(0, Map::len);
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent {
            actor_id: Some(user_id),
            action: "design.imported".into(),
            target: Some(design_id.clone()),
            meta: Some(json!({ "kind": kind, "pages": pages })),
            ..Default::default()
        },
        s.now(),
    )
    .await
    .map_err(|e| e.to_string())?;
    drop(dir);
    Ok(json!({ "designId": design_id, "pages": pages }))
}

/// A JS value as node-postgres sends it (text; `null`/`undefined` → NULL).
fn js_arg(v: &Value) -> Arg {
    match v {
        Value::Null => Arg::Text(None),
        Value::String(t) => Arg::Text(Some(t.clone())),
        Value::Number(n) => Arg::Text(Some(crate::http::jsjson::number_text(n))),
        other => Arg::Text(Some(crate::http::jsjson::stringify(other))),
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

async fn create(State(s): State<AppState>, mut req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    start_worker(&s);
    let dir = TempDir::new("oc-imp-up-").await?;
    let src = dir.0.join("src");
    let Some(file) = multipart::save_file(&mut req, MAX_IMPORT, &src).await? else {
        return Ok(reply::json(400, json!({ "error": "no_file" })));
    };
    let buf = tokio::fs::read(&src).await?;
    if file.truncated {
        return Ok(reply::json(413, json!({ "error": "too_large", "message": "Imports are limited to 200 MB." })));
    }
    let is_pdf = buf.starts_with(b"%PDF-");
    let is_zip = buf.first() == Some(&0x50) && buf.get(1) == Some(&0x4b);
    let is_pptx = is_zip && contains(&buf, b"ppt/presentation.xml");
    if !is_pdf && !is_pptx {
        return Ok(reply::json(415, json!({ "error": "unsupported_type", "message": "Import a PDF or a PowerPoint (.pptx) file." })));
    }
    // Importing into a folder (Projects '+ Add new' ▸ Upload, g17): the folder must be one the user may file designs
    // in, and a team folder makes the imported design part of that team (as POST /api/designs does).
    let q = z::obj([("folderId", z::str().uuid().optional())]).parse(Some(&req.query))?;
    let f = match q.get("folderId").and_then(Value::as_str) {
        Some(fid) => {
            let Some(f) = crate::projects::folder_access(&s, u.id, Uuid::parse_str(fid).map_err(ApiError::internal)?).await? else {
                return Ok(reply::json(404, json!({ "error": "folder_not_found" })));
            };
            Some(f)
        }
        None => None,
    };
    if let Some(f) = &f {
        if !f.file {
            return Ok(reply::json(403, json!({ "error": "forbidden" })));
        }
        // Brand controls (A8): an import isn't from a brand template, so whoever must start team designs from one can't import.
        if !f.personal
            && (f.role.as_deref() == Some("member")
                || (!crate::brandhub::is_admin_role(f.role.as_deref()) && crate::brandhub::control_on(&crate::brandhub::brand_controls(&s, Some(f.workspace_id)).await?, "requireTemplate")))
        {
            return Ok(reply::json(403, json!({ "error": "template_required", "message": "This team’s designs start from its brand templates." })));
        }
    }
    let sha = s.store.put(&buf, Some(if is_pdf { "application/pdf" } else { "application/vnd.openxmlformats-officedocument.presentationml.presentation" })).await?;
    let folder_id = f.as_ref().map(|f| f.id.to_string());
    let workspace_id = f.as_ref().filter(|f| !f.personal).map(|f| f.workspace_id.to_string());
    let job = oc_jobs::enqueue(
        &s.pool,
        QUEUE,
        &json!({ "sha": sha, "kind": if is_pdf { "pdf" } else { "pptx" }, "name": file.filename, "userId": u.id.to_string(), "folderId": folder_id, "workspaceId": workspace_id }),
        oc_jobs::EnqueueOpts { owner_id: Some(u.id), max_attempts: Some(1), label: Some("Queued".into()), ..Default::default() },
    )
    .await?;
    Ok(Reply::json(json!({ "jobId": job.to_string() })))
}

/// Docs → editable design pages (a new design; the doc stays as it is).
async fn convert(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !auth::at_least(auth::design_role(&s, Some(u.id), &id).await?, oc_auth::Role::View) {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let embeds = embed_allow_list(&s, Some(u.id), &id).await?;
    let res = s
        .http
        .post(format!("{}/internal/doc-to-design", s.cfg.render_url))
        .header("content-type", "application/json")
        .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&s.cfg.secret))
        .body(crate::http::jsjson::stringify(&json!({ "designId": id, "embeds": embeds })))
        .send()
        .await
        .ok()
        .filter(|r| r.status().is_success());
    let Some(res) = res else { return Ok(reply::json(502, json!({ "error": "render_unavailable" }))) };
    let body: Value = crate::http::jsjson::parse(&res.text().await?).map_err(ApiError::internal)?;
    let doc = body.get("doc").cloned().unwrap_or(Value::Null);
    let generated = body.get("generated").and_then(Value::as_array).cloned().unwrap_or_default();
    // Embedded designs were rendered to pictures: they become the converting user's images.
    if !generated.is_empty() {
        let ws = auth::personal_workspace(&s, u.id).await?;
        for a in &generated {
            let sha = a["sha256"].as_str().unwrap_or("");
            let bytes = s.store.get(sha).await?.len() as i64;
            pg::exec(
                &s.pool,
                "INSERT INTO assets(id, workspace_id, owner_id, sha256, mime, kind, bytes, width, height, name, source) VALUES ($1,$2,$3,$4,'image/png','image',$5,$6::text::int,$7::text::int,$8,'upload') ON CONFLICT (id) DO NOTHING",
                &[js_arg(&a["id"]), pg::uuid(ws), pg::uuid(u.id), js_arg(&a["sha256"]), Arg::Int(Some(bytes)), js_arg(&a["w"]), js_arg(&a["h"]), js_arg(&a["name"])],
            )
            .await?;
        }
    }
    let design_id = create_design_from_doc(&s, u.id, &doc, CreateOpts::default()).await?;
    Ok(Reply::json(json!({ "id": design_id })))
}

async fn status(State(s): State<AppState>, Path(job): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    start_worker(&s);
    static JOB_ID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^[0-9a-f-]{36}$").expect("regex"));
    if !JOB_ID.is_match(&job) {
        return Ok(Reply::empty(404));
    }
    let j = match Uuid::parse_str(&job) {
        Ok(id) => oc_jobs::get_job(&s.pool, id).await?,
        Err(_) => {
            // node-postgres sends the text and Postgres rejects it (`invalid input syntax for type uuid`).
            sqlx::query("SELECT $1::uuid").bind(&job).execute(&s.pool).await?;
            None
        }
    };
    let Some(j) = j.filter(|j| j.queue == QUEUE && j.owner_id == Some(u.id)) else {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    };
    let status = match j.status {
        oc_jobs::JobStatus::Failed => "error",
        oc_jobs::JobStatus::Queued => "queued",
        oc_jobs::JobStatus::Running => "running",
        oc_jobs::JobStatus::Done => "done",
        oc_jobs::JobStatus::Cancelled => "cancelled",
    };
    // float4 → node-postgres parses the text form.
    let progress = j.progress.to_string().parse::<f64>().unwrap_or(0.0);
    let mut out = Map::new();
    out.insert("status".into(), json!(status));
    out.insert("progress".into(), crate::http::jsjson::number_value(progress).unwrap_or(json!(0)));
    out.insert("label".into(), json!(j.label));
    out.insert("error".into(), json!(j.error));
    if let Some(r) = j.result.as_ref().map(|r| &r.0).filter(|r| r.is_object()) {
        if let Some(d) = r.get("designId").filter(|v| !v.is_null()) {
            out.insert("designId".into(), d.clone());
        }
        if let Some(p) = r.get("pages").filter(|v| !v.is_null()) {
            out.insert("pages".into(), p.clone());
        }
    }
    Ok(Reply::json(Value::Object(out)))
}
