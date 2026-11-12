//! Port of services/api/src/routes/social.ts — Content Planner (A16): connected social accounts (OAuth 2.0 +
//! PKCE, Bluesky app passwords), scheduled posts, the publish log, and signed public media links for networks
//! that pull files by URL. Adapters, the account store, token crypto and scheduling come from `oc-social`.
//!
//! The media links and the OAuth callback need no sign-in; every other route runs `requireUser` first (the
//! child plugin's `preHandler`).
//!
//! Test seam (not in production): with `OC_SOCIAL_FAKE_BASE` set, every `https://host/path` provider call goes to
//! `<base>/host/path` (the parity transcripts point it at their fake server).

use chrono::{DateTime, Utc};
use oc_social::publish::{PostRow, log_event, schedule_publish};
use oc_social::store::{AccountRow, SaveAccountsError, ensure_local_account, public_account, save_accounts, visible_accounts};
use oc_social::{AuthKind, AuthorizeParams, ExchangeParams, Http};
use serde_json::Map;

use crate::auth::{at_least, design_role};
use crate::prelude::*;
use oc_auth::Role;

/// Routes with their own `config.rateLimit`.
pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/social/media/{file}", get(media))
        .route("/api/social/callback/{provider}", get(callback))
        .route("/api/social/providers", get(providers))
        .route("/api/social/accounts", get(accounts))
        .route("/api/social/connect/{provider}", post(connect))
        .route(
            "/api/social/accounts/{id}",
            patch(patch_account).delete(delete_account),
        )
        .route("/api/social/posts", get(list_posts).post(create_post))
        .route(
            "/api/social/posts/{id}",
            patch(update_post).delete(delete_post),
        )
        .route("/api/social/posts/{id}/publish-now", post(publish_now))
        .route("/api/social/posts/{id}/submit", post(submit_post))
        .route("/api/social/posts/{id}/approve", post(approve_post))
        .route(
            "/api/social/posts/{id}/request-changes",
            post(request_changes),
        )
        .route("/api/social/posts/{id}/events", get(events))
}

// ---------------------------------------------------------------- helpers

/// The adapters' HTTP client and environment (`process.env` of the app).
pub fn http(s: &AppState) -> Http {
    let env = s.clone();
    let h = Http::with_env(move |k| env.env(k));
    match s.env_nonempty("OC_SOCIAL_FAKE_BASE").filter(|_| !s.prod()) {
        Some(base) => h.with_url_rewrite(move |u| match u.strip_prefix("https://") {
            Some(rest) => format!("{}/{rest}", base.trim_end_matches('/')),
            None => u.to_owned(),
        }),
        None => h,
    }
}

fn redirect_uri(s: &AppState, provider: &str) -> String {
    format!("{}/api/social/callback/{provider}", s.cfg.web_origin)
}

fn is_uuid(id: &str) -> bool {
    z::str().uuid().safe_parse(Some(&json!(id))).is_ok()
}

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

fn iso(t: Option<DateTime<Utc>>) -> Value {
    t.map_or(Value::Null, |t| json!(oc_js::iso_string(&t)))
}

/// `validTz(tz)`: an IANA zone `Intl.DateTimeFormat` accepts (case-insensitively, from the system tz database) or
/// a UTC offset; otherwise `'UTC'`.
fn valid_tz(tz: &str) -> String {
    static OFFSET: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^[+-](?:[01]\d|2[0-3])(?::?[0-5]\d)?$").expect("regex"));
    if OFFSET.is_match(tz) || tz.eq_ignore_ascii_case("utc") {
        return tz.to_owned();
    }
    const NOT_ZONES: [&str; 9] = ["zone.tab", "zone1970.tab", "iso3166.tab", "leapseconds", "tzdata.zi", "posixrules", "leap-seconds.list", "+VERSION", "SECURITY"];
    let mut dir = std::path::PathBuf::from("/usr/share/zoneinfo");
    let parts: Vec<&str> = tz.split('/').collect();
    let ok = !tz.is_empty()
        && !matches!(parts.first().map(|p| p.to_ascii_lowercase()).as_deref(), Some("posix" | "right"))
        && parts.iter().enumerate().all(|(i, part)| {
            if part.is_empty() || *part == "." || *part == ".." || NOT_ZONES.contains(part) {
                return false;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else { return false };
            let found = entries.filter_map(Result::ok).find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part));
            match found {
                Some(e) => {
                    let is_dir = e.path().is_dir();
                    dir = e.path();
                    if i + 1 == parts.len() { !is_dir } else { is_dir }
                }
                None => false,
            }
        });
    if ok { tz.to_owned() } else { "UTC".into() }
}

/// `publicPost(p)`.
pub(crate) fn public_post(p: &PostRow, design_title: Option<&str>) -> Value {
    json!({
        "id": p.id.to_string(), "designId": p.design_id, "designTitle": design_title, "pages": p.pages, "format": p.format, "caption": p.caption,
        "hashtags": p.hashtags, "accounts": p.accounts.iter().map(Uuid::to_string).collect::<Vec<_>>(), "options": p.options.0, "scheduledAt": iso(p.scheduled_at),
        "timezone": p.timezone, "status": p.status, "results": p.results.0, "error": p.error, "publishedAt": iso(p.published_at), "createdAt": iso(Some(p.created_at)),
        "updatedAt": iso(Some(p.updated_at)), "ownerId": p.owner_id.to_string(), "mediaCount": p.media.0.len(),
        "publisher": p.publisher, "channels": p.channels, "fields": p.fields.0,
        "submittedBy": p.submitted_by.map(|u| u.to_string()), "submittedAt": iso(p.submitted_at),
        "approvedBy": p.approved_by.map(|u| u.to_string()), "approvedAt": iso(p.approved_at), "approvedVersion": p.approved_version,
        "claimedAt": iso(p.claimed_at),
    })
}

pub(crate) async fn post_for(s: &AppState, user: Uuid, id: &str) -> Result<Option<PostRow>, sqlx::Error> {
    if !is_uuid(id) {
        return Ok(None);
    }
    sqlx::query_as(
        "SELECT p.* FROM social_posts p WHERE p.id = $1::uuid
       AND (p.owner_id = $2 OR p.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $2))",
    )
    .bind(id)
    .bind(user)
    .fetch_optional(&s.pool)
    .await
}

/// Every selected account must be one the user may post to (`400 unknown_account` otherwise).
async fn check_accounts(s: &AppState, user: Uuid, ids: &[String]) -> Result<Vec<AccountRow>, ApiError> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let visible = visible_accounts(&s.pool, user).await?;
    ids.iter()
        .map(|id| visible.iter().find(|a| a.id.to_string() == *id).cloned().ok_or_else(|| ApiError::status(400, "unknown_account")))
        .collect()
}

/// Server-side guard mirroring the dialog: caption lengths and media support per network.
fn problems(rows: &[AccountRow], caption: &str, hashtags: &[String], format: &str) -> Vec<String> {
    let text = oc_social::text::compose_text(caption, hashtags);
    let mut out = Vec::new();
    for r in rows {
        let Some(a) = oc_social::adapter(&r.provider) else { continue };
        let l = a.limits();
        if oc_social::text::char_count(&text, Some(a.id())) > l.max_chars {
            out.push(format!("{}: caption is over {} characters", a.label(), l.max_chars));
        }
        if format == "mp4" && !l.video {
            out.push(format!("{}: videos aren’t supported", a.label()));
        }
        if format != "mp4" && l.images == 0 {
            out.push(format!("{}: needs a video (MP4)", a.label()));
        }
    }
    out
}

/// The checks of a post about to be scheduled; `Some(reply)` refuses it.
fn schedule_checks(
    rows: &[AccountRow],
    scheduled: bool,
    text: (&str, &[String], &str),
) -> Option<Refusal> {
    if rows.is_empty() {
        return Some(Refusal(
            400,
            json!({ "error": "no_accounts", "message": "Pick at least one account." }),
        ));
    }
    if !scheduled {
        return Some(Refusal(
            400,
            json!({ "error": "no_time", "message": "Pick a date and time." }),
        ));
    }
    let p = problems(rows, text.0, text.1, text.2);
    if !p.is_empty() {
        return Some(Refusal(
            400,
            json!({ "error": "invalid_post", "message": p.join("; ") }),
        ));
    }
    None
}

fn post_body(partial: bool) -> z::Schema {
    let format = z::enumeration(&["png", "jpg", "mp4"]);
    let mut fields: Vec<(&str, z::Schema)> = vec![
        (
            "pages",
            z::array(z::num().int().min(0.0).max(999.0))
                .max(100.0)
                .default(json!([])),
        ),
        ("format", if partial { format.optional() } else { format }),
        ("caption", z::str().max(70_000.0).default(json!(""))),
        (
            "hashtags",
            z::array(z::str().max(100.0)).max(60.0).default(json!([])),
        ),
        (
            "accounts",
            z::array(z::str().uuid()).max(30.0).default(json!([])),
        ),
        ("options", z::record(z::any()).default(json!({}))),
        (
            "scheduledAt",
            z::str().datetime_offset().nullable().optional(),
        ),
        ("timezone", z::str().max(64.0).default(json!("UTC"))),
        (
            "status",
            z::enumeration(&["draft", "scheduled", "in_review"]).optional(),
        ),
        (
            "channels",
            z::array(z::enumeration(&CHANNELS)).max(2.0).optional(),
        ),
        ("fields", z::record(z::str().max(20_000.0)).optional()),
    ];
    if !partial {
        fields.insert(0, ("designId", z::str().min(1.0).max(64.0)));
        fields.push((
            "publisher",
            z::enumeration(&["pinselo", "external"]).default(json!("pinselo")),
        ));
    }
    z::obj_vec(fields)
}

/// Channels an external publisher posts to.
pub(crate) const CHANNELS: [&str; 2] = ["instagram", "facebook"];

/// Editorial fields: at most 40 keys of up to 64 characters (values were checked by the schema).
fn clean_fields(v: Option<&Value>) -> Result<Option<Map<String, Value>>, ApiError> {
    let Some(Value::Object(m)) = v else {
        return Ok(None);
    };
    if m.len() > 40 || m.keys().any(|k| k.is_empty() || k.chars().count() > 64) {
        return Err(ApiError::status(400, "invalid_fields"));
    }
    Ok(Some(
        m.iter()
            .filter(|(_, v)| v.as_str().is_some_and(|s| !s.trim().is_empty()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    ))
}

/// The checks of an external post about to be reviewed or approved; `Some(reply)` refuses it.
fn external_checks(channels: &[String], scheduled: bool) -> Option<Refusal> {
    if channels.is_empty() {
        return Some(Refusal(
            400,
            json!({ "error": "no_channels", "message": "Pick Instagram, Facebook or both." }),
        ));
    }
    if !scheduled {
        return Some(Refusal(
            400,
            json!({ "error": "no_time", "message": "Pick a date and time." }),
        ));
    }
    None
}

fn duplicate_ref() -> Refusal {
    Refusal(
        409,
        json!({ "error": "duplicate_ref", "message": "Another post in this workspace already uses this post ID." }),
    )
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505")
}

/// Approvers: in a team workspace its admins and owners; a personal post's owner.
pub(crate) async fn can_approve(s: &AppState, user: Uuid, p: &PostRow) -> Result<bool, ApiError> {
    match p.workspace_id {
        None => Ok(p.owner_id == user),
        Some(ws) => Ok(crate::routes::teams::team_role(s, user, ws)
            .await?
            .is_some_and(|r| r == "admin" || r == "owner")),
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
}

/// `hashtags.map((h) => h.replace(/^#+/, '')).filter(Boolean)`.
fn clean_tags(tags: &[String]) -> Vec<String> {
    tags.iter().map(|h| h.trim_start_matches('#').to_owned()).filter(|h| !h.is_empty()).collect()
}

fn uuids(ids: &[String]) -> Result<Vec<Uuid>, ApiError> {
    ids.iter().map(|i| Uuid::parse_str(i).map_err(ApiError::internal)).collect()
}

/// `new Date(iso).getTime()` of a zod-validated ISO datetime.
fn parse_time(s: &str) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(s).map(|t| t.with_timezone(&Utc)).map_err(ApiError::internal)
}

// ---------------------------------------------------------------- public: signed media links + OAuth callback

async fn media(State(s): State<AppState>, Path(file): Path<String>, req: Req) -> ApiResult {
    static NAME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^([0-9a-f]{64})\.(png|jpg|mp4)$").expect("regex"));
    let exp = crate::routes::sites::js_number(req.query.get("exp"));
    let sig = match req.query.get("sig") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(x)) => x.clone(),
        Some(Value::Array(a)) => a.iter().map(|x| x.as_str().unwrap_or("")).collect::<Vec<_>>().join(","),
        Some(other) => crate::http::jsjson::stringify(other),
    };
    let Some(m) = NAME.captures(&file) else { return Ok(not_found()) };
    let (sha, ext) = (m[1].to_owned(), m[2].to_owned());
    if !oc_social::crypto::verify_media_sig(&sha, exp, &sig, s.now_ms() as f64, &s.cfg.secret) {
        return Ok(not_found());
    }
    if !s.store.exists(&sha).await? {
        return Ok(not_found());
    }
    let ct = match ext.as_str() {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        _ => "video/mp4",
    };
    Ok(Reply::bytes(ct, s.store.get(&sha).await?).header("cache-control", "public, max-age=86400"))
}

fn back(s: &AppState, params: &[(&str, &str)]) -> Reply {
    let mut q = form_urlencoded::Serializer::new(String::new());
    for (k, v) in params {
        q.append_pair(k, v);
    }
    Reply::redirect(&format!("{}/settings/accounts?{}", s.cfg.web_origin, q.finish()))
}

async fn callback(State(s): State<AppState>, Path(provider): Path<String>, req: Req) -> ApiResult {
    let qs = |k: &str| req.query.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned);
    let st: Option<(Uuid, String, String)> = match req.query.get("state").filter(|v| !matches!(v, Value::Null) && v.as_str() != Some("")) {
        Some(Value::String(state)) => {
            sqlx::query_as("DELETE FROM social_oauth_states WHERE state = $1 AND created_at > now() - interval '15 minutes' RETURNING user_id, provider, code_verifier")
                .bind(state)
                .fetch_optional(&s.pool)
                .await?
        }
        _ => None,
    };
    let Some((user_id, st_provider, verifier)) = st.filter(|st| st.1 == provider) else {
        return Ok(back(&s, &[("error", "The connection request expired. Try again.")]));
    };
    let _ = st_provider;
    let error = qs("error");
    let Some(code) = qs("code").filter(|_| error.is_none()) else {
        let msg = req.query.get("error_description").and_then(Value::as_str).map(str::to_owned).or(req.query.get("error").and_then(Value::as_str).map(str::to_owned)).unwrap_or_else(|| "Access was not granted.".into());
        return Ok(back(&s, &[("error", &msg)]));
    };
    let h = http(&s);
    let p = ExchangeParams { code, redirect_uri: redirect_uri(&s, &provider), code_verifier: verifier };
    let Some(a) = oc_social::adapter(&provider) else { return Ok(back(&s, &[("error", "This network is not configured.")])) };
    let fut = a.exchange(&h, &p);
    let Some(fut) = fut.filter(|_| a.configured(&h)) else { return Ok(back(&s, &[("error", "This network is not configured.")])) };
    let accounts = match fut.await {
        Ok(list) => list,
        Err(e) => return Ok(back(&s, &[("error", &e.message)])),
    };
    match save_accounts(&s.pool, &s.cfg.secret, user_id, &provider, &accounts).await {
        Ok(_) => {}
        Err(SaveAccountsError::Refused(e)) => return Ok(back(&s, &[("error", &e.message)])),
        Err(_) => return Ok(back(&s, &[("error", "Connecting the account failed.")])),
    }
    let audit = oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: Some(user_id), action: "social.connect".into(), meta: Some(json!({ "provider": provider, "accounts": accounts.len() })), ..Default::default() },
        s.now(),
    )
    .await;
    if audit.is_err() {
        return Ok(back(&s, &[("error", "Connecting the account failed.")]));
    }
    Ok(back(&s, &[("connected", &provider), ("n", &accounts.len().to_string())]))
}

// ---------------------------------------------------------------- signed in

async fn providers(State(s): State<AppState>, req: Req) -> ApiResult {
    require_user(&s, &req).await?;
    Ok(Reply::json(json!({ "providers": oc_social::provider_list(&http(&s)) })))
}

async fn accounts(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    ensure_local_account(&s.pool, u.id).await?;
    let rows = visible_accounts(&s.pool, u.id).await?;
    Ok(Reply::json(json!({ "accounts": rows.iter().map(|r| public_account(r, Some(u.id))).collect::<Vec<_>>() })))
}

async fn connect(State(s): State<AppState>, Path(provider): Path<String>, req: Req) -> ApiResult {
    use base64::Engine as _;
    let u = require_user(&s, &req).await?;
    let Some(a) = oc_social::adapter(&provider).filter(|a| a.id() != "local") else { return Ok(reply::json(404, json!({ "error": "unknown_provider" }))) };
    let h = http(&s);
    if !a.configured(&h) {
        return Ok(reply::json(501, json!({ "error": "not_configured", "message": format!("{} isn’t configured on this server. Set {}.", a.label(), a.env().join(" and ")) })));
    }
    if a.auth() == AuthKind::AppPassword {
        let b = z::obj([("identifier", z::str().min(3.0).max(200.0)), ("password", z::str().min(4.0).max(200.0))]).parse(req.body())?;
        let (id, pw) = (crate::js::trim(b["identifier"].as_str().unwrap_or("")).to_owned(), crate::js::trim(b["password"].as_str().unwrap_or("")).to_owned());
        let failed = |m: &str| reply::json(400, json!({ "error": "connect_failed", "message": m }));
        let Some(fut) = a.connect_with_password(&h, &id, &pw) else { return Err(ApiError::internal("connectWithPassword is not a function")) };
        let list = match fut.await {
            Ok(l) => l,
            Err(e) => return Ok(failed(&e.message)),
        };
        let ids = match save_accounts(&s.pool, &s.cfg.secret, u.id, a.id(), &list).await {
            Ok(ids) => ids,
            Err(SaveAccountsError::Refused(e)) => return Ok(failed(&e.message)),
            Err(_) => return Ok(failed("Could not sign in with that handle and app password.")),
        };
        let ev = oc_audit::AuditEvent { actor_id: Some(u.id), action: "social.connect".into(), meta: Some(json!({ "provider": a.id() })), ..Default::default() };
        if oc_audit::audit_at(&s.pool, ev, s.now()).await.is_err() {
            return Ok(failed("Could not sign in with that handle and app password."));
        }
        return Ok(Reply::json(json!({ "ok": true, "ids": ids.iter().map(Uuid::to_string).collect::<Vec<_>>() })));
    }
    let b64 = |n: usize| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(auth::random_bytes(n));
    let (state, verifier) = (b64(24), b64(48));
    pg::exec(&s.pool, "INSERT INTO social_oauth_states(state, user_id, provider, code_verifier) VALUES ($1,$2,$3,$4)", &[pg::text(&state), pg::uuid(u.id), pg::text(a.id()), pg::text(&verifier)]).await?;
    let p = AuthorizeParams { state, redirect_uri: redirect_uri(&s, a.id()), code_challenge: oc_social::http::pkce_challenge(&verifier) };
    let Some(url) = a.authorize_url(&h, &p) else { return Err(ApiError::internal("authorizeUrl is not a function")) };
    Ok(Reply::json(json!({ "url": url })))
}

async fn patch_account(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("workspaceId", z::str().uuid().nullable().optional()), ("defaultBoardId", z::str().max(64.0).optional())]).parse(req.body())?;
    let row = if is_uuid(&id) { pg::one(&s.pool, "SELECT 1 FROM social_accounts WHERE id = $1::uuid AND user_id = $2", &[pg::text(&id), pg::uuid(u.id)]).await? } else { None };
    if row.is_none() {
        return Ok(not_found());
    }
    let ws = b.get("workspaceId");
    if let Some(w) = ws.and_then(Value::as_str).filter(|w| !w.is_empty()) {
        let m = pg::one(&s.pool, "SELECT 1 FROM memberships m JOIN workspaces w ON w.id = m.workspace_id WHERE m.workspace_id = $1::uuid AND m.user_id = $2 AND NOT w.personal", &[pg::text(w), pg::uuid(u.id)]).await?;
        if m.is_none() {
            return Ok(reply::json(403, json!({ "error": "forbidden" })));
        }
    }
    pg::exec(
        &s.pool,
        "UPDATE social_accounts SET workspace_id = CASE WHEN $3 THEN $2::uuid ELSE workspace_id END,
           meta = CASE WHEN $4::text IS NULL THEN meta ELSE meta || jsonb_build_object('defaultBoardId', $4::text) END, updated_at = now() WHERE id = $1::uuid",
        &[pg::text(&id), Arg::Text(ws.and_then(Value::as_str).map(str::to_owned)), Arg::Bool(Some(ws.is_some())), Arg::Text(b.get("defaultBoardId").and_then(Value::as_str).map(str::to_owned))],
    )
    .await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn delete_account(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let r: Option<String> = if is_uuid(&id) {
        sqlx::query_scalar("DELETE FROM social_accounts WHERE id = $1::uuid AND user_id = $2 AND provider <> 'local' RETURNING provider").bind(&id).bind(u.id).fetch_optional(&s.pool).await?
    } else {
        None
    };
    let Some(provider) = r else { return Ok(not_found()) };
    oc_audit::audit_at(&s.pool, oc_audit::AuditEvent { actor_id: Some(u.id), action: "social.disconnect".into(), meta: Some(json!({ "provider": provider })), ..Default::default() }, s.now()).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- posts

#[derive(sqlx::FromRow)]
struct ListedPost {
    approver_name: Option<String>,
    design_version: Option<i64>,
    workspace_name: Option<String>,
    #[sqlx(flatten)]
    post: PostRow,
    design_title: Option<String>,
}

async fn list_posts(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let q = z::obj([
        ("from", z::str().datetime_offset().optional()),
        ("to", z::str().datetime_offset().optional()),
        ("account", z::str().uuid().optional()),
        ("designId", z::str().max(64.0).optional()),
        ("status", z::str().max(20.0).optional()),
        // One brand at a time: a team's id, or `personal` for posts outside every team.
        ("workspace", z::str().max(36.0).optional()),
    ])
    .parse(Some(&req.query))?;
    let qs = |k: &str| q.get(k).and_then(Value::as_str).map(str::to_owned);
    let rows: Vec<ListedPost> = sqlx::query_as(
        "SELECT p.*, d.title AS design_title, ua.name AS approver_name, d.content_version AS design_version, w.name AS workspace_name
         FROM social_posts p JOIN designs d ON d.id = p.design_id
         LEFT JOIN users ua ON ua.id = p.approved_by
         LEFT JOIN workspaces w ON w.id = p.workspace_id
         WHERE (p.owner_id = $1 OR p.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1))
           AND ($2::timestamptz IS NULL OR p.scheduled_at IS NULL OR p.scheduled_at >= $2::timestamptz)
           AND ($3::timestamptz IS NULL OR p.scheduled_at IS NULL OR p.scheduled_at < $3::timestamptz)
           AND ($4::uuid IS NULL OR $4::uuid = ANY(p.accounts))
           AND ($5::text IS NULL OR p.design_id = $5)
           AND ($6::text IS NULL OR p.status = $6)
           AND ($7::text IS NULL OR ($7 = 'personal' AND p.workspace_id IS NULL) OR p.workspace_id::text = $7)
         ORDER BY p.scheduled_at NULLS LAST, p.created_at, p.id LIMIT 1000",
    )
    .bind(u.id)
    .bind(qs("from"))
    .bind(qs("to"))
    .bind(qs("account"))
    .bind(qs("designId"))
    .bind(qs("status"))
    .bind(qs("workspace").map(|w| w.to_lowercase()))
    .fetch_all(&s.pool)
    .await?;
    Ok(Reply::json(json!({ "posts": rows.iter().map(|r| {
        let mut v = public_post(&r.post, r.design_title.as_deref());
        v["approvedByName"] = json!(r.approver_name);
        v["designVersion"] = json!(r.design_version);
        v["workspaceId"] = json!(r.post.workspace_id.map(|w| w.to_string()));
        v["workspaceName"] = json!(r.workspace_name);
        v
    }).collect::<Vec<_>>() })))
}

async fn create_post(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    Ok(match create_post_as(&s, u.id, req.body(), None).await? {
        Ok(p) => Reply::json(json!({ "post": public_post(&p, None) })),
        Err(r) => r.reply(),
    })
}

/// Creates a post for `user` from a request body (web app and `POST /v1/planner/posts`). `only_ws` limits the design
/// to one workspace (a workspace-limited API token).
/// A refused request: HTTP status and JSON body (`{ error, message? }`).
pub(crate) struct Refusal(pub u16, pub Value);

impl Refusal {
    pub(crate) fn reply(self) -> Reply {
        reply::json(self.0, self.1)
    }
    /// The human-readable reason (AI assistants show it).
    pub(crate) fn message(&self) -> String {
        self.1
            .get("message")
            .or_else(|| self.1.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("refused")
            .to_owned()
    }
}

/// `Ok(Err(refusal))` refuses the request.
pub(crate) type PostOutcome = Result<Result<PostRow, Refusal>, ApiError>;

pub(crate) async fn create_post_as(
    s: &AppState,
    user: Uuid,
    body: Option<&Value>,
    only_ws: Option<Uuid>,
) -> PostOutcome {
    let b = post_body(false).parse(body)?;
    let bs = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let design_id = crate::routes::v1::design_id_param(&bs("designId"));
    if !at_least(design_role(s, Some(user), &design_id).await?, Role::Edit) {
        return Ok(Err(Refusal(404, json!({ "error": "not_found" }))));
    }
    let ws: Option<(Uuid, bool)> = sqlx::query_as("SELECT w.id, w.personal FROM designs d JOIN workspaces w ON w.id = d.workspace_id WHERE d.id = $1").bind(&design_id).fetch_optional(&s.pool).await?;
    if only_ws.is_some() && ws.map(|(id, _)| id) != only_ws {
        return Ok(Err(Refusal(404, json!({ "error": "not_found" }))));
    }
    let publisher = bs("publisher");
    let external = publisher == "external";
    let accounts = strings(b.get("accounts"));
    let channels = strings(b.get("channels"));
    let hashtags = strings(b.get("hashtags"));
    let fields = clean_fields(b.get("fields"))?.unwrap_or_default();
    let rows = check_accounts(s, user, &accounts).await?;
    let scheduled_at = b
        .get("scheduledAt")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let status = b.get("status").and_then(Value::as_str).map_or_else(
        || {
            if external {
                "draft".to_owned()
            } else {
                "scheduled".to_owned()
            }
        },
        str::to_owned,
    );
    if external {
        if !accounts.is_empty() {
            return Ok(Err(Refusal(
                400,
                json!({ "error": "external_accounts", "message": "Posts published by an outside tool have channels, not accounts." }),
            )));
        }
        if status == "scheduled" {
            return Ok(Err(Refusal(
                400,
                json!({ "error": "needs_approval", "message": "Submit the post for review; it is published once approved." }),
            )));
        }
        if status == "in_review"
            && let Some(r) = external_checks(&channels, scheduled_at.is_some())
        {
            return Ok(Err(r));
        }
    } else if status != "draft"
        && let Some(r) = schedule_checks(
            &rows,
            scheduled_at.is_some(),
            (&bs("caption"), &hashtags, &bs("format")),
        )
    {
        return Ok(Err(r));
    }
    let pages: Vec<i32> = b
        .get("pages")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_f64)
                .map(|f| f as i32)
                .collect()
        })
        .unwrap_or_default();
    let in_review = status == "in_review";
    let inserted = sqlx::query_as::<_, PostRow>(
        "INSERT INTO social_posts(id, owner_id, workspace_id, design_id, pages, format, caption, hashtags, accounts, options, scheduled_at, timezone, status,
                                  publisher, channels, fields, submitted_by, submitted_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::jsonb,$11::timestamptz,$12,$13,$14,$15,$16::jsonb,$17,CASE WHEN $17::uuid IS NULL THEN NULL ELSE now() END) RETURNING *",
    )
    .bind(oc_db::uuid())
    .bind(user)
    .bind(ws.and_then(|(id, personal)| (!personal).then_some(id)))
    .bind(&design_id)
    .bind(&pages)
    .bind(bs("format"))
    .bind(bs("caption"))
    .bind(clean_tags(&hashtags))
    .bind(uuids(&accounts)?)
    .bind(crate::http::jsjson::stringify(b.get("options").unwrap_or(&json!({}))))
    .bind(&scheduled_at)
    .bind(valid_tz(&bs("timezone")))
    .bind(&status)
    .bind(&publisher)
    .bind(&channels)
    .bind(crate::http::jsjson::stringify(&Value::Object(fields)))
    .bind(in_review.then_some(user))
    .fetch_one(&s.pool)
    .await;
    let post = match inserted {
        Err(e) if is_unique_violation(&e) => return Ok(Err(duplicate_ref())),
        other => other?,
    };
    let kind = match status.as_str() {
        "scheduled" => "scheduled",
        "in_review" => "submitted",
        _ => "created",
    };
    log_event(&s.pool, post.id, kind, scheduled_at.as_deref(), None, None).await?;
    if post.status == "scheduled" {
        schedule_publish(
            &s.pool,
            post.id,
            post.owner_id,
            post.scheduled_at,
            s.now_ms(),
        )
        .await?;
    }
    Ok(Ok(post))
}

fn locked() -> Reply {
    reply::json(
        409,
        json!({ "error": "locked", "message": "This post was already published." }),
    )
}

async fn update_post(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(post) = post_for(&s, u.id, &id).await? else {
        return Ok(not_found());
    };
    Ok(match update_post_as(&s, u.id, post, req.body()).await? {
        Ok(p) => Reply::json(json!({ "post": public_post(&p, None) })),
        Err(r) => r.reply(),
    })
}

/// Updates `post` for `user` (web app and `PATCH /v1/planner/posts/{id}`). Any change to an approved post sends it
/// back to review: the approval covered what was approved, not what it became.
pub(crate) async fn update_post_as(
    s: &AppState,
    user: Uuid,
    post: PostRow,
    body: Option<&Value>,
) -> PostOutcome {
    if post.status == "publishing" || post.status == "published" {
        return Ok(Err(Refusal(
            409,
            json!({ "error": "locked", "message": "This post was already published." }),
        )));
    }
    let b = post_body(true).parse(body)?;
    let bs = |k: &str| b.get(k).and_then(Value::as_str).map(str::to_owned);
    let external = post.publisher == "external";
    let pages: Vec<i32> = match b.get("pages").and_then(Value::as_array) {
        Some(a) => a
            .iter()
            .filter_map(Value::as_f64)
            .map(|f| f as i32)
            .collect(),
        None => post.pages.clone(),
    };
    let format = bs("format").unwrap_or_else(|| post.format.clone());
    let caption = bs("caption").unwrap_or_else(|| post.caption.clone());
    let hashtags = clean_tags(
        &b.get("hashtags")
            .map_or_else(|| post.hashtags.clone(), |v| strings(Some(v))),
    );
    let accounts: Vec<String> = b.get("accounts").map_or_else(
        || post.accounts.iter().map(Uuid::to_string).collect(),
        |v| strings(Some(v)),
    );
    let channels: Vec<String> = b
        .get("channels")
        .map_or_else(|| post.channels.clone(), |v| strings(Some(v)));
    let fields = clean_fields(b.get("fields"))?.unwrap_or_else(|| post.fields.0.clone());
    let options = b
        .get("options")
        .cloned()
        .unwrap_or_else(|| Value::Object(post.options.0.clone()));
    let scheduled_at: Option<DateTime<Utc>> = match b.get("scheduledAt") {
        None => post.scheduled_at,
        Some(Value::String(t)) => Some(parse_time(t)?),
        Some(_) => None,
    };
    let timezone = valid_tz(&bs("timezone").unwrap_or_else(|| post.timezone.clone()));
    let was_approved =
        post.approved_at.is_some() && matches!(post.status.as_str(), "approved" | "scheduled");
    let status = match bs("status") {
        Some(st) => st,
        None if was_approved || post.status == "approved" => "in_review".into(),
        None if post.status == "failed" => {
            if external {
                "in_review".into()
            } else {
                "scheduled".into()
            }
        }
        None => post.status.clone(),
    };
    // An approved post that is edited goes back to review instead of staying scheduled.
    let status = if status == "scheduled" && (external || was_approved) {
        "in_review".to_owned()
    } else {
        status
    };
    let rows = check_accounts(s, user, &accounts).await?;
    if external {
        if !accounts.is_empty() {
            return Ok(Err(Refusal(
                400,
                json!({ "error": "external_accounts", "message": "Posts published by an outside tool have channels, not accounts." }),
            )));
        }
        if status == "in_review"
            && let Some(r) = external_checks(&channels, scheduled_at.is_some())
        {
            return Ok(Err(r));
        }
    } else if status != "draft"
        && let Some(r) = schedule_checks(
            &rows,
            scheduled_at.is_some(),
            (&caption, &hashtags, &format),
        )
    {
        return Ok(Err(r));
    }
    // Re-render on the next run (the design may have changed); keep results already published (or of unknown
    // outcome, which an external publisher must not retry) for targets still selected.
    let targets: Vec<String> = if external {
        channels.clone()
    } else {
        accounts.clone()
    };
    let results: Map<String, Value> = post
        .results
        .0
        .iter()
        .filter(|(k, r)| {
            matches!(
                r.get("status").and_then(Value::as_str),
                Some("published" | "unknown")
            ) && targets.contains(k)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let submitted = status == "in_review" && post.status != "in_review";
    let updated = sqlx::query_as::<_, PostRow>(
        "UPDATE social_posts SET pages = $2, format = $3, caption = $4, hashtags = $5, accounts = $6, options = $7::jsonb, scheduled_at = $8, timezone = $9,
           status = $10, media = '[]', results = $11::jsonb, error = NULL, job_id = NULL, channels = $12, fields = $13::jsonb,
           submitted_by = CASE WHEN $14 THEN $15 ELSE submitted_by END, submitted_at = CASE WHEN $14 THEN now() ELSE submitted_at END,
           approved_by = NULL, approved_at = NULL, approved_version = NULL, claim_token = NULL, claimed_at = NULL, updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(post.id)
    .bind(&pages)
    .bind(&format)
    .bind(&caption)
    .bind(&hashtags)
    .bind(uuids(&accounts)?)
    .bind(crate::http::jsjson::stringify(&options))
    .bind(scheduled_at)
    .bind(&timezone)
    .bind(&status)
    .bind(crate::http::jsjson::stringify(&Value::Object(results)))
    .bind(&channels)
    .bind(crate::http::jsjson::stringify(&Value::Object(fields)))
    .bind(submitted)
    .bind(user)
    .fetch_one(&s.pool)
    .await;
    let updated = match updated {
        Err(e) if is_unique_violation(&e) => return Ok(Err(duplicate_ref())),
        other => other?,
    };
    let moved = post.scheduled_at.map(|t| t.timestamp_millis())
        != scheduled_at.map(|t| t.timestamp_millis());
    let kind = if submitted {
        "submitted"
    } else if status == "draft" || status == "in_review" || status == "changes_requested" {
        "updated"
    } else if moved || post.status != "scheduled" {
        "rescheduled"
    } else {
        "updated"
    };
    let message = scheduled_at.map(|t| oc_js::iso_string(&t));
    log_event(&s.pool, post.id, kind, message.as_deref(), None, None).await?;
    if updated.status == "scheduled" {
        schedule_publish(
            &s.pool,
            updated.id,
            updated.owner_id,
            updated.scheduled_at,
            s.now_ms(),
        )
        .await?;
    }
    Ok(Ok(updated))
}

// ---------------------------------------------------------------- review: submit, approve, request changes

async fn submit_post(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(post) = post_for(&s, u.id, &id).await? else {
        return Ok(not_found());
    };
    if !matches!(
        post.status.as_str(),
        "draft" | "changes_requested" | "failed"
    ) {
        return Ok(reply::json(
            409,
            json!({ "error": "wrong_status", "message": "Only drafts, posts with requested changes and failed posts can be submitted." }),
        ));
    }
    if post.publisher == "external" {
        if let Some(r) = external_checks(&post.channels, post.scheduled_at.is_some()) {
            return Ok(r.reply());
        }
    } else {
        let accounts: Vec<String> = post.accounts.iter().map(Uuid::to_string).collect();
        let rows = check_accounts(&s, u.id, &accounts).await?;
        if let Some(r) = schedule_checks(
            &rows,
            post.scheduled_at.is_some(),
            (&post.caption, &post.hashtags, &post.format),
        ) {
            return Ok(r.reply());
        }
    }
    let updated: PostRow = sqlx::query_as(
        "UPDATE social_posts SET status = 'in_review', submitted_by = $2, submitted_at = now(), approved_by = NULL, approved_at = NULL, approved_version = NULL,
           error = NULL, updated_at = now() WHERE id = $1 RETURNING *",
    )
    .bind(post.id)
    .bind(u.id)
    .fetch_one(&s.pool)
    .await?;
    log_event(&s.pool, post.id, "submitted", None, None, None).await?;
    Ok(Reply::json(json!({ "post": public_post(&updated, None) })))
}

async fn approve_post(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(post) = post_for(&s, u.id, &id).await? else {
        return Ok(not_found());
    };
    if !can_approve(&s, u.id, &post).await? {
        return Ok(reply::json(
            403,
            json!({ "error": "forbidden", "message": "Only workspace admins and owners can approve posts." }),
        ));
    }
    // An approved external post may be approved again: it records the design's current version (after a fix).
    if !matches!(post.status.as_str(), "in_review" | "failed") && !(post.status == "approved" && post.publisher == "external") {
        return Ok(reply::json(
            409,
            json!({ "error": "wrong_status", "message": "Only posts in review (or failed ones) can be approved." }),
        ));
    }
    let external = post.publisher == "external";
    if external {
        if let Some(r) = external_checks(&post.channels, post.scheduled_at.is_some()) {
            return Ok(r.reply());
        }
    } else {
        let accounts: Vec<String> = post.accounts.iter().map(Uuid::to_string).collect();
        let rows = check_accounts(&s, post.owner_id, &accounts).await?;
        if let Some(r) = schedule_checks(
            &rows,
            post.scheduled_at.is_some(),
            (&post.caption, &post.hashtags, &post.format),
        ) {
            return Ok(r.reply());
        }
    }
    let version = crate::routes::v1_export::content_version(&s, &post.design_id).await?;
    let next = if external { "approved" } else { "scheduled" };
    let updated: PostRow = sqlx::query_as(
        "UPDATE social_posts SET status = $2, approved_by = $3, approved_at = now(), approved_version = $4, error = NULL, claim_token = NULL, claimed_at = NULL,
           updated_at = now() WHERE id = $1 AND status = $5 RETURNING *",
    )
    .bind(post.id)
    .bind(next)
    .bind(u.id)
    .bind(version)
    .bind(&post.status)
    .fetch_optional(&s.pool)
    .await?
    .ok_or_else(|| ApiError::status(409, "changed"))?;
    log_event(
        &s.pool,
        post.id,
        "approved",
        version.map(|v| format!("Design version {v}")).as_deref(),
        None,
        None,
    )
    .await?;
    if updated.status == "scheduled" {
        schedule_publish(
            &s.pool,
            updated.id,
            updated.owner_id,
            updated.scheduled_at,
            s.now_ms(),
        )
        .await?;
    }
    Ok(Reply::json(json!({ "post": public_post(&updated, None) })))
}

async fn request_changes(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(post) = post_for(&s, u.id, &id).await? else {
        return Ok(not_found());
    };
    if !can_approve(&s, u.id, &post).await? {
        return Ok(reply::json(
            403,
            json!({ "error": "forbidden", "message": "Only workspace admins and owners can review posts." }),
        ));
    }
    // A post an outside publisher claimed over an hour ago and never finished can be pulled back too.
    let stuck = post.status == "publishing"
        && post.publisher == "external"
        && post
            .claimed_at
            .is_some_and(|t| s.now_ms() - t.timestamp_millis() > 3_600_000);
    if !stuck
        && !matches!(
            post.status.as_str(),
            "in_review" | "approved" | "scheduled" | "failed"
        )
    {
        return Ok(reply::json(
            409,
            json!({ "error": "wrong_status", "message": "This post is not waiting for review." }),
        ));
    }
    let b = z::obj([("note", z::str().max(20_000.0).optional())]).parse(req.body())?;
    let note = b
        .get("note")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    let mut fields = post.fields.0.clone();
    if let Some(n) = &note {
        fields.insert("reviewerNote".into(), json!(n));
    }
    let updated: PostRow = sqlx::query_as(
        "UPDATE social_posts SET status = 'changes_requested', fields = $2::jsonb, approved_by = NULL, approved_at = NULL, approved_version = NULL,
           claim_token = NULL, claimed_at = NULL, job_id = NULL, updated_at = now() WHERE id = $1 RETURNING *",
    )
    .bind(post.id)
    .bind(crate::http::jsjson::stringify(&Value::Object(fields)))
    .fetch_one(&s.pool)
    .await?;
    log_event(
        &s.pool,
        post.id,
        "changes_requested",
        note.as_deref(),
        None,
        None,
    )
    .await?;
    Ok(Reply::json(json!({ "post": public_post(&updated, None) })))
}

async fn publish_now(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(post) = post_for(&s, u.id, &id).await? else {
        return Ok(not_found());
    };
    if post.status == "publishing" || post.status == "published" {
        return Ok(locked());
    }
    if post.publisher == "external" {
        return Ok(reply::json(
            400,
            json!({ "error": "external_post", "message": "This post is published by an outside tool once approved." }),
        ));
    }
    if post.approved_at.is_none()
        && matches!(post.status.as_str(), "in_review" | "changes_requested")
    {
        return Ok(reply::json(
            409,
            json!({ "error": "needs_approval", "message": "Approve the post first." }),
        ));
    }
    let accounts: Vec<String> = post.accounts.iter().map(Uuid::to_string).collect();
    let rows = check_accounts(&s, u.id, &accounts).await?;
    if rows.is_empty() {
        return Ok(reply::json(
            400,
            json!({ "error": "no_accounts", "message": "Pick at least one account." }),
        ));
    }
    let p = problems(&rows, &post.caption, &post.hashtags, &post.format);
    if !p.is_empty() {
        return Ok(reply::json(
            400,
            json!({ "error": "invalid_post", "message": p.join("; ") }),
        ));
    }
    let updated: PostRow =
        sqlx::query_as("UPDATE social_posts SET status = 'scheduled', scheduled_at = now(), media = '[]', error = NULL, job_id = NULL, updated_at = now() WHERE id = $1 RETURNING *")
            .bind(post.id)
            .fetch_one(&s.pool)
            .await?;
    log_event(
        &s.pool,
        post.id,
        "scheduled",
        Some("Publish now"),
        None,
        None,
    )
    .await?;
    schedule_publish(
        &s.pool,
        updated.id,
        updated.owner_id,
        updated.scheduled_at,
        s.now_ms(),
    )
    .await?;
    Ok(Reply::json(json!({ "post": public_post(&updated, None) })))
}

async fn delete_post(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(post) = post_for(&s, u.id, &id).await? else { return Ok(not_found()) };
    if post.status == "publishing" {
        return Ok(reply::json(409, json!({ "error": "locked", "message": "This post is being published right now." })));
    }
    sqlx::query("DELETE FROM social_posts WHERE id = $1").bind(post.id).execute(&s.pool).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn events(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(post) = post_for(&s, u.id, &id).await? else { return Ok(not_found()) };
    let rows = pg::json_rows(
        &s.pool,
        r#"SELECT id, account_id AS "accountId", kind, message, created_at AS "createdAt" FROM social_post_events WHERE post_id = $1 ORDER BY id"#,
        &[pg::uuid(post.id)],
    )
    .await?;
    Ok(Reply::json(json!({ "events": rows })))
}
