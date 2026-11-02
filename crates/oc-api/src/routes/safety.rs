//! Port of services/api/src/routes/safety.ts — trust & safety (TS-001/003/005/006/022/056): report intake (DSA
//! notice fields, DMCA), the staff review console with actions, statements of reasons to affected users, user
//! enforcement, the hash blocklist and the audit log. `POST /api/reports` needs no sign-in; every `/api/admin/*`
//! route runs `requireStaff` first (the child plugin's `preHandler`).
//!
//! Also exported for other modules: [`blocklist_match`], [`file_report`], [`REASONS`]; [`bootstrap_staff`] is the
//! plugin's start-up step (`OC_STAFF_EMAILS`).

use crate::auth::require_staff;
use crate::http::ratelimit::MINUTE;
use crate::prelude::*;
use crate::routes::teams;

/// Routes with their own `config.rateLimit`.
pub const LIMITS: &[RouteLimit] = &[RouteLimit { method: "POST", path: "/api/reports", max: 20, window_ms: 10 * MINUTE }];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/reports", post(report))
        .route("/api/admin/overview", get(overview))
        .route("/api/admin/reports", get(reports))
        .route("/api/admin/reports/{id}/decide", post(decide))
        .route("/api/admin/users", get(users))
        .route("/api/admin/users/{id}/{action}", post(user_action))
        .route("/api/admin/hashes", post(add_hash))
        .route("/api/admin/audit", get(audit_log))
        .route("/api/admin/jobs", get(jobs))
}

// ---------------------------------------------------------------- shared helpers

pub const REASONS: [&str; 13] = ["csam", "ncii", "violence", "hate", "harassment", "self_harm", "spam", "phishing", "copyright", "trademark", "impersonation", "privacy", "other"];

fn priority(reason: &str) -> i32 {
    match reason {
        "csam" => 100,
        "ncii" => 90,
        "self_harm" => 80,
        "violence" | "phishing" => 60,
        "hate" | "harassment" => 50,
        "privacy" => 40,
        "copyright" | "trademark" | "impersonation" => 30,
        "spam" => 10,
        _ => 0,
    }
}

/// `blocklistMatch(phash)`: the blocklist entry within Hamming distance 6.
pub async fn blocklist_match(state: &AppState, phash: Option<&str>) -> Result<Option<crate::hash_match::HashHit>, sqlx::Error> {
    let Some(phash) = phash else { return Ok(None) };
    let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT hash, category, source FROM hash_blocklist").fetch_all(&state.pool).await?;
    Ok(rows.into_iter().find(|(h, _, _)| crate::media::hamming(h, phash) <= 6).map(|(hash, category, source)| crate::hash_match::HashHit { hash, category, source }))
}

/// `fileReport(r)`: returns the report id.
pub struct NewReport<'a> {
    pub target_type: &'a str,
    pub target_id: &'a str,
    pub reason: &'a str,
    pub details: Option<&'a str>,
    pub reporter_id: Option<uuid::Uuid>,
    pub reporter_email: Option<&'a str>,
    pub legal: Option<serde_json::Value>,
}

pub async fn file_report(state: &AppState, r: NewReport<'_>) -> Result<uuid::Uuid, sqlx::Error> {
    let id = oc_db::uuid();
    sqlx::query("INSERT INTO reports(id, target_type, target_id, reason, details, reporter_id, reporter_email, legal, priority) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(id)
        .bind(r.target_type)
        .bind(r.target_id)
        .bind(r.reason)
        .bind(r.details)
        .bind(r.reporter_id)
        .bind(r.reporter_email)
        .bind(r.legal.map(sqlx::types::Json))
        .bind(priority(r.reason))
        .execute(&state.pool)
        .await?;
    Ok(id)
}

/// The plugin's start-up step: staff accounts from `OC_STAFF_EMAILS` (comma-separated). Only once the address is
/// verified: sign-up doesn't require it, so anyone could register a listed address first.
pub async fn bootstrap_staff(state: &AppState) -> Result<(), sqlx::Error> {
    let emails: Vec<String> = state.env("OC_STAFF_EMAILS").unwrap_or_default().split(',').map(|s| crate::js::trim(s).to_lowercase()).filter(|s| !s.is_empty()).collect();
    if !emails.is_empty() {
        sqlx::query("UPDATE users SET is_staff = true WHERE email = ANY($1) AND email_verified_at IS NOT NULL").bind(&emails).execute(&state.pool).await?;
    }
    Ok(())
}

async fn audit(s: &AppState, req: &Req, actor: Option<Uuid>, action: String, target: Option<String>, meta: Option<Value>) -> Result<(), sqlx::Error> {
    oc_audit::audit_at(&s.pool, oc_audit::AuditEvent { actor_id: actor, action, target, meta, ip: Some(req.ip.clone()), ..Default::default() }, s.now()).await
}

/// The affected user of a report's target.
async fn owner_of(s: &AppState, target_type: &str, target_id: &str) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    let sql = match target_type {
        "design" | "public_link" => "SELECT u.id, u.email FROM designs d JOIN users u ON u.id = d.owner_id WHERE d.id = $1",
        "asset" => "SELECT u.id, u.email FROM assets a JOIN users u ON u.id = a.owner_id WHERE a.id = $1",
        "comment" => "SELECT u.id, u.email FROM comments c JOIN users u ON u.id = c.author_id WHERE c.id = $1::uuid",
        "user" => "SELECT id, email FROM users WHERE id = $1::uuid",
        _ => return Ok(None),
    };
    sqlx::query_as(sql).bind(target_id).fetch_optional(&s.pool).await
}

/// `String.prototype.replace('_', ' ')`: the first occurrence only.
fn first_underscore(s: &str) -> String {
    s.replacen('_', " ", 1)
}

// ---------------------------------------------------------------- public intake

async fn report(State(s): State<AppState>, req: Req) -> ApiResult {
    let b = z::obj([
        ("targetType", z::enumeration(&["design", "asset", "comment", "user", "public_link"])),
        ("targetId", z::str().min(1.0).max(100.0)),
        ("reason", z::enumeration(&REASONS)),
        ("details", z::str().max(5000.0).optional()),
        ("email", z::str().email().optional()),
        (
            "legal",
            z::obj([
                ("name", z::str().max(200.0).optional()),
                ("address", z::str().max(500.0).optional()),
                ("work", z::str().max(2000.0).optional()),
                ("goodFaith", z::bool().optional()),
                ("accurate", z::bool().optional()),
                ("signature", z::str().max(200.0).optional()),
                ("location", z::str().max(500.0).optional()),
            ])
            .optional(),
        ),
    ])
    .parse(req.body())?;
    let u = auth::current_user(&s, &req).await?;
    let bs = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or("");
    let (target_type, target_id, reason) = (bs("targetType"), bs("targetId"), bs("reason"));
    let legal = b.get("legal");
    let truthy_str = |k: &str| legal.and_then(|l| l.get(k)).and_then(Value::as_str).is_some_and(|v| !v.is_empty());
    let true_flag = |k: &str| legal.and_then(|l| l.get(k)) == Some(&Value::Bool(true));
    if reason == "copyright" && !(true_flag("goodFaith") && true_flag("accurate") && truthy_str("signature") && truthy_str("name")) {
        return Ok(Reply::json(json!({
            "ok": false, "error": "incomplete_notice",
            "message": "A copyright notice needs your name, the work, a good-faith statement, an accuracy statement and your signature.",
        })));
    }
    let reporter_email = u.as_ref().map(|u| u.email.clone()).or_else(|| b.get("email").and_then(Value::as_str).map(str::to_owned));
    let id = file_report(
        &s,
        NewReport {
            target_type,
            target_id,
            reason,
            details: b.get("details").and_then(Value::as_str),
            reporter_id: u.as_ref().map(|u| u.id),
            reporter_email: reporter_email.as_deref(),
            legal: legal.cloned(),
        },
    )
    .await?;
    // NCII / CSAM: hide public exposure immediately pending review (48 h removal duty, TAKE IT DOWN Act).
    if (reason == "ncii" || reason == "csam") && (target_type == "design" || target_type == "public_link") {
        pg::exec(&s.pool, "UPDATE designs SET public_view = false WHERE id = $1", &[pg::text(target_id)]).await?;
    }
    if (reason == "ncii" || reason == "csam") && target_type == "asset" {
        pg::exec(&s.pool, "UPDATE assets SET moderation = 'pending' WHERE id = $1", &[pg::text(target_id)]).await?;
    }
    audit(&s, &req, u.as_ref().map(|u| u.id), "ts.report_filed".into(), Some(format!("{target_type}:{target_id}")), Some(json!({ "reason": reason, "report": id.to_string() }))).await?;
    Ok(Reply::json(json!({ "ok": true, "id": id.to_string() })))
}

// ---------------------------------------------------------------- staff console

async fn count(s: &AppState, sql: &str) -> Result<Value, sqlx::Error> {
    Ok(pg::json_one(&s.pool, sql, &[]).await?.and_then(|r| r.get("n").cloned()).unwrap_or(Value::Null))
}

async fn overview(State(s): State<AppState>, req: Req) -> ApiResult {
    require_staff(&s, &req).await?;
    let (users, designs, reports, assets, jobs) = futures::try_join!(
        count(&s, "SELECT count(*)::int AS n FROM users WHERE deleted_at IS NULL"),
        count(&s, "SELECT count(*)::int AS n FROM designs WHERE trashed_at IS NULL"),
        count(&s, "SELECT count(*)::int AS n FROM reports WHERE status = 'open'"),
        count(&s, "SELECT count(*)::int AS n FROM assets WHERE moderation <> 'ok'"),
        count(&s, "SELECT count(*)::int AS n FROM jobs WHERE status = 'failed' AND created_at > now() - interval '1 day'"),
    )?;
    Ok(Reply::json(json!({ "users": users, "designs": designs, "openReports": reports, "flaggedAssets": assets, "failedJobs24h": jobs })))
}

/// A query-string value as node-postgres would send it (a repeated key is an array → a Postgres array literal).
fn query_param(v: Option<&Value>, default: &str) -> Arg {
    match v {
        None => pg::text(default),
        Some(Value::String(s)) => pg::text(s.clone()),
        Some(Value::Array(a)) => {
            let items: Vec<String> = a.iter().map(|x| format!("\"{}\"", x.as_str().unwrap_or("").replace('\\', "\\\\").replace('"', "\\\""))).collect();
            pg::text(format!("{{{}}}", items.join(",")))
        }
        Some(other) => pg::text(crate::http::jsjson::stringify(other)),
    }
}

async fn reports(State(s): State<AppState>, req: Req) -> ApiResult {
    require_staff(&s, &req).await?;
    let rows = pg::json_rows(
        &s.pool,
        "SELECT r.*, u.name AS reporter_name FROM reports r LEFT JOIN users u ON u.id = r.reporter_id
           WHERE r.status = $1 ORDER BY r.priority DESC, r.created_at LIMIT 200",
        &[query_param(req.query.get("status"), "open")],
    )
    .await?;
    Ok(Reply::json(json!({ "reports": rows })))
}

async fn decide(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let me = require_staff(&s, &req).await?;
    let b = z::obj([
        ("action", z::enumeration(&["dismiss", "takedown", "remove_asset", "remove_comment", "disable_user", "restore"])),
        ("note", z::str().min(3.0).max(2000.0)),
        ("blockHash", z::bool().optional()),
    ])
    .parse(req.body())?;
    let action = b["action"].as_str().unwrap_or("");
    let note = b["note"].as_str().unwrap_or("");
    let r: Option<(String, String, String, Option<String>)> =
        sqlx::query_as("SELECT target_type, target_id, reason, reporter_email FROM reports WHERE id = $1::uuid").bind(&id).fetch_optional(&s.pool).await?;
    let Some((target_type, target_id, reason, reporter_email)) = r else {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    };
    let owner = owner_of(&s, &target_type, &target_id).await?;
    match action {
        "takedown" => {
            // On hold in the owner's trash: they can't restore it or delete it forever (see routes/designs.rs).
            pg::exec(&s.pool, "UPDATE designs SET public_view = false, trashed_at = now(), taken_down_at = now() WHERE id = $1", &[pg::text(&target_id)]).await?;
            teams::revoke_live(&s, Some(&target_id), None, None).await?;
        }
        "remove_asset" => {
            pg::exec(&s.pool, "UPDATE assets SET moderation = 'removed', trashed_at = now() WHERE id = $1", &[pg::text(&target_id)]).await?;
            if b.get("blockHash") == Some(&Value::Bool(true)) {
                let phash: Option<Option<String>> = sqlx::query_scalar("SELECT phash FROM assets WHERE id = $1").bind(&target_id).fetch_optional(&s.pool).await?;
                if let Some(Some(h)) = phash.filter(|p| p.as_deref().is_some_and(|h| !h.is_empty())) {
                    pg::exec(&s.pool, "INSERT INTO hash_blocklist(hash, source, category) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING", &[pg::text(h), pg::text("internal"), pg::text(&reason)]).await?;
                }
            }
        }
        "remove_comment" => {
            pg::exec(&s.pool, "DELETE FROM comments WHERE id = $1::uuid", &[pg::text(&target_id)]).await?;
        }
        "disable_user" => {
            if let Some((oid, _)) = &owner {
                pg::exec(&s.pool, "UPDATE users SET disabled_at = now(), disabled_reason = $2 WHERE id = $1", &[pg::uuid(*oid), pg::text(note)]).await?;
                pg::exec(&s.pool, "DELETE FROM sessions WHERE user_id = $1", &[pg::uuid(*oid)]).await?;
                teams::revoke_live(&s, None, None, Some(&oid.to_string())).await?;
            }
        }
        "restore" => {
            if target_type == "asset" {
                pg::exec(&s.pool, "UPDATE assets SET moderation = 'ok', trashed_at = NULL WHERE id = $1", &[pg::text(&target_id)]).await?;
            }
            if target_type == "design" || target_type == "public_link" {
                pg::exec(&s.pool, "UPDATE designs SET trashed_at = NULL, taken_down_at = NULL WHERE id = $1", &[pg::text(&target_id)]).await?;
            }
        }
        _ => {}
    }
    pg::exec(
        &s.pool,
        "UPDATE reports SET status = $2, decision = $3, decided_by = $4, decided_at = now() WHERE id = $1::uuid",
        &[pg::text(&id), pg::text(if action == "dismiss" { "dismissed" } else { "actioned" }), pg::text(format!("{action}: {note}")), pg::uuid(me.id)],
    )
    .await?;
    // Statement of reasons (DSA Art. 17) to the affected user; outcome to the reporter.
    if let Some((oid, email)) = &owner
        && action != "dismiss"
    {
        let body = format!(
            "We took action on your content ({target_type}) under our {} policy.\nWhat happened: {}.\nWhy: {note}\nYou can appeal by replying to this email within 6 months.",
            first_underscore(&reason),
            first_underscore(action)
        );
        if *oid != me.id {
            // `notify(...).catch(() => {})`
            let _ = sqlx::query("INSERT INTO notifications(id, user_id, kind, design_id, actor_id, body) VALUES ($1,$2,$3,$4,$5,$6)")
                .bind(oc_db::uuid())
                .bind(oid)
                .bind("comment")
                .bind(if target_type == "design" { Some(target_id.as_str()) } else { None })
                .bind(me.id)
                .bind(crate::js::take(&format!("Content moderation: {} — {note}", first_underscore(action)), 300))
                .execute(&s.pool)
                .await;
        }
        oc_mail::send_email(&s.pool, email, &oc_mail::Template::Decision { subject: "A decision about your content".into(), body }, None).await?;
    }
    if let Some(e) = reporter_email.as_deref().filter(|e| !e.is_empty()) {
        let outcome = if action == "dismiss" { "no violation found" } else { "we took action" };
        oc_mail::send_email(&s.pool, e, &oc_mail::Template::Decision { subject: "Update on your report".into(), body: format!("Thanks for your report. Outcome: {outcome}.") }, None).await?;
    }
    audit(&s, &req, Some(me.id), format!("ts.decision.{action}"), Some(format!("{target_type}:{target_id}")), Some(json!({ "report": id, "note": note }))).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn users(State(s): State<AppState>, req: Req) -> ApiResult {
    require_staff(&s, &req).await?;
    let rows = pg::json_rows(
        &s.pool,
        "SELECT id, email, name, is_staff, disabled_at, disabled_reason, email_verified_at, created_at,
                  (SELECT count(*)::int FROM designs d WHERE d.owner_id = users.id) AS designs
           FROM users WHERE deleted_at IS NULL AND (email ILIKE '%' || $1 || '%' OR name ILIKE '%' || $1 || '%') ORDER BY created_at DESC LIMIT 100",
        &[query_param(req.query.get("q"), "")],
    )
    .await?;
    Ok(Reply::json(json!({ "users": rows })))
}

async fn user_action(State(s): State<AppState>, Path((id, action)): Path<(String, String)>, req: Req) -> ApiResult {
    let me = require_staff(&s, &req).await?;
    // `const { reason = '' } = (req.body ?? {})`: a string body has no `reason`.
    let reason = match req.body() {
        Some(Value::Object(m)) => m.get("reason").cloned().unwrap_or_else(|| json!("")),
        _ => json!(""),
    };
    let reason_arg = match &reason {
        Value::Null => Arg::Text(None),
        Value::String(r) => pg::text(r.clone()),
        Value::Bool(_) | Value::Number(_) => pg::text(crate::http::jsjson::stringify(&reason)),
        other => pg::text(crate::http::jsjson::stringify(other)),
    };
    match action.as_str() {
        "disable" => {
            // A squat tombstone keeps its reason, so it can't be re-disabled into something enable revives.
            pg::exec(&s.pool, "UPDATE users SET disabled_at = now(), disabled_reason = $2 WHERE id = $1::uuid AND disabled_reason IS DISTINCT FROM 'supabase_squat'", &[pg::text(&id), reason_arg]).await?;
            pg::exec(&s.pool, "DELETE FROM sessions WHERE user_id = $1::uuid", &[pg::text(&id)]).await?;
            teams::revoke_live(&s, None, None, Some(&id)).await?;
        }
        "enable" => {
            // A squat tombstone (plan 12 §B4, D4) gave its address to the inbox's owner; it stays disabled for good.
            let r = pg::rows(&s.pool, "UPDATE users SET disabled_at = NULL, disabled_reason = NULL WHERE id = $1::uuid AND disabled_reason IS DISTINCT FROM 'supabase_squat' RETURNING id", &[pg::text(&id)]).await?;
            if r.is_empty() && pg::one(&s.pool, "SELECT 1 FROM users WHERE id = $1::uuid AND disabled_reason = 'supabase_squat'", &[pg::text(&id)]).await?.is_some() {
                return Ok(reply::json(409, json!({ "error": "tombstoned", "message": "This account was set aside because someone else proved they own its email address. It can’t be enabled again." })));
            }
        }
        _ => return Ok(reply::json(400, json!({ "error": "bad_action" }))),
    }
    audit(&s, &req, Some(me.id), format!("admin.user_{action}"), Some(id.clone()), Some(json!({ "reason": reason }))).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn add_hash(State(s): State<AppState>, req: Req) -> ApiResult {
    let me = require_staff(&s, &req).await?;
    let b = z::obj([("hash", z::str().regex("^[0-9a-f]{16}$", "^[0-9a-f]{16}$")), ("source", z::str().max(40.0)), ("category", z::str().max(40.0))]).parse(req.body())?;
    let bs = |k: &str| b[k].as_str().unwrap_or("").to_owned();
    pg::exec(&s.pool, "INSERT INTO hash_blocklist(hash, source, category) VALUES ($1,$2,$3) ON CONFLICT (hash) DO NOTHING", &[pg::text(bs("hash")), pg::text(bs("source")), pg::text(bs("category"))]).await?;
    audit(&s, &req, Some(me.id), "ts.hash_added".into(), Some(bs("hash")), Some(json!({ "source": bs("source") }))).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn audit_log(State(s): State<AppState>, req: Req) -> ApiResult {
    require_staff(&s, &req).await?;
    let broken = oc_audit::verify_audit_chain(&s.pool).await?;
    let events = pg::json_rows(
        &s.pool,
        "SELECT a.id, a.action, a.target, a.meta, a.ip, a.created_at, u.email AS actor FROM audit_events a LEFT JOIN users u ON u.id = a.actor_id ORDER BY a.id DESC LIMIT 200",
        &[],
    )
    .await?;
    Ok(Reply::json(json!({ "brokenAt": broken, "events": events })))
}

async fn jobs(State(s): State<AppState>, req: Req) -> ApiResult {
    require_staff(&s, &req).await?;
    let rows = pg::json_rows(&s.pool, "SELECT id, queue, status, attempts, label, error, created_at, finished_at FROM jobs ORDER BY created_at DESC LIMIT 100", &[]).await?;
    Ok(Reply::json(json!({ "jobs": rows })))
}
