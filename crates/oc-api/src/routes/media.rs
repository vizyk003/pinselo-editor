//! Port of services/api/src/routes/media.ts — video/audio services: export jobs (Postgres queue → render
//! worker). Every route runs `requireUser` first.

use oc_auth::Role;

use crate::auth::{at_least, design_role};
use crate::prelude::*;
use crate::routes::assets::tokio_util_stream;
use crate::routes::{docs, print_profiles};

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/designs/{id}/export-jobs", post(export_job))
        .route("/api/export-jobs/{job}", get(job_status))
        .route("/api/export-jobs/{job}/file", get(job_file))
}

async fn export_job(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([
        ("format", z::enumeration(&["mp4", "gif", "m4a", "pdf", "pdf-print", "pdf-flat", "pptx", "docx"])),
        ("quality", z::enumeration(&["720p", "1080p", "4k"]).optional()),
        ("print", print_profiles::print_export_options().optional()),
    ])
    .parse(req.body())?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let Some(print) = print_profiles::resolve_print_options(&s, u.id, b.get("print")).await? else { return Ok(print_profiles::profile_not_found()) };
    let active: Option<i32> =
        sqlx::query_scalar("SELECT count(*)::int AS n FROM jobs WHERE owner_id = $1 AND queue = 'export' AND status IN ('queued','running')").bind(u.id).fetch_optional(&s.pool).await?;
    if active.unwrap_or(0) >= 3 {
        return Ok(reply::json(429, json!({ "error": "too_many_exports", "message": "Wait for your other exports to finish." })));
    }
    let mut payload = serde_json::Map::new();
    payload.insert("designId".into(), json!(id));
    payload.insert("userId".into(), json!(u.id.to_string()));
    if let Value::Object(m) = &b {
        for (k, v) in m {
            payload.insert(k.clone(), v.clone());
        }
    }
    payload.extend(print);
    payload.insert("embeds".into(), json!(docs::embed_allow_list(&s, Some(u.id), &id).await?));
    let jid = oc_jobs::enqueue(&s.pool, "export", &Value::Object(payload), oc_jobs::EnqueueOpts { owner_id: Some(u.id), max_attempts: Some(2), ..Default::default() }).await?;
    Ok(Reply::json(json!({ "id": jid.to_string() })))
}

/// `ownJob(jobId, userId)`: an export job of this user (the id is passed to Postgres as text, so a malformed
/// one fails there like in TS).
async fn own_job(s: &AppState, job: &str, user: Uuid) -> Result<Option<oc_jobs::Job>, ApiError> {
    if job.len() != 36 || !job.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || c == b'-') {
        return Ok(None);
    }
    let j: Option<oc_jobs::Job> = sqlx::query_as("SELECT * FROM jobs WHERE id = $1::uuid").bind(job).fetch_optional(&s.pool).await?;
    Ok(j.filter(|j| j.queue == "export" && j.owner_id == Some(user)))
}

fn float(f: f32) -> Value {
    crate::http::jsjson::number_value(f.to_string().parse().unwrap_or(f64::NAN)).unwrap_or(Value::Null)
}

async fn job_status(State(s): State<AppState>, Path(job): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(j) = own_job(&s, &job, u.id).await? else { return Ok(reply::json(404, json!({ "error": "not_found" }))) };
    let r = j.result.as_ref().map(|r| r.0.clone());
    let status = match j.status {
        oc_jobs::JobStatus::Failed | oc_jobs::JobStatus::Cancelled => "error",
        oc_jobs::JobStatus::Queued => "queued",
        oc_jobs::JobStatus::Running => "running",
        oc_jobs::JobStatus::Done => "done",
    };
    let mut out = serde_json::Map::new();
    out.insert("id".into(), json!(j.id.to_string()));
    out.insert("status".into(), json!(status));
    out.insert("progress".into(), float(j.progress));
    out.insert("label".into(), json!(j.label.clone().unwrap_or_default()));
    if let Some(e) = &j.error {
        out.insert("error".into(), json!(e));
    }
    for k in ["filename", "size"] {
        if let Some(v) = r.as_ref().and_then(|r| r.get(k)) {
            out.insert(k.into(), v.clone());
        }
    }
    Ok(Reply::json(Value::Object(out)))
}

async fn job_file(State(s): State<AppState>, Path(job): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let j = own_job(&s, &job, u.id).await?;
    let r = j.as_ref().and_then(|j| j.result.as_ref()).map(|r| r.0.clone());
    let sha = r.as_ref().and_then(|r| r.get("sha")).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_owned);
    let (Some(j), Some(sha), Some(r)) = (j, sha, r) else { return Ok(reply::json(404, json!({ "error": "not_ready" }))) };
    if j.status != oc_jobs::JobStatus::Done {
        return Ok(reply::json(404, json!({ "error": "not_ready" })));
    }
    let text = |k: &str| match r.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(v) => crate::http::jsjson::stringify(v),
        None => "undefined".into(),
    };
    let stream = s.store.stream(&sha, None).await?;
    Ok(Reply::bytes(&text("type"), axum::body::Body::from_stream(tokio_util_stream(stream)))
        .header("content-length", text("size"))
        .header("content-disposition", format!("attachment; filename=\"{}\"", text("filename").replace('"', ""))))
}
