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
use crate::brandhub::approval_gate;
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
        .route("/api/social/accounts/{id}", patch(patch_account).delete(delete_account))
        .route("/api/social/posts", get(list_posts).post(create_post))
        .route("/api/social/posts/{id}", patch(update_post).delete(delete_post))
        .route("/api/social/posts/{id}/publish-now", post(publish_now))
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
fn public_post(p: &PostRow, design_title: Option<&str>) -> Value {
    json!({
        "id": p.id.to_string(), "designId": p.design_id, "designTitle": design_title, "pages": p.pages, "format": p.format, "caption": p.caption,
        "hashtags": p.hashtags, "accounts": p.accounts.iter().map(Uuid::to_string).collect::<Vec<_>>(), "options": p.options.0, "scheduledAt": iso(p.scheduled_at),
        "timezone": p.timezone, "status": p.status, "results": p.results.0, "error": p.error, "publishedAt": iso(p.published_at), "createdAt": iso(Some(p.created_at)),
        "updatedAt": iso(Some(p.updated_at)), "ownerId": p.owner_id.to_string(), "mediaCount": p.media.0.len(),
    })
}

async fn post_for(s: &AppState, user: Uuid, id: &str) -> Result<Option<PostRow>, sqlx::Error> {
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
async fn schedule_checks(s: &AppState, user: Uuid, design_id: &str, rows: &[AccountRow], scheduled: bool, text: (&str, &[String], &str)) -> Result<Option<Reply>, ApiError> {
    if let Some(gate) = approval_gate(s, user, design_id).await? {
        return Ok(Some(reply::json(403, gate))); // brand approvals (A8)
    }
    if rows.is_empty() {
        return Ok(Some(reply::json(400, json!({ "error": "no_accounts", "message": "Pick at least one account." }))));
    }
    if !scheduled {
        return Ok(Some(reply::json(400, json!({ "error": "no_time", "message": "Pick a date and time." }))));
    }
    let p = problems(rows, text.0, text.1, text.2);
    if !p.is_empty() {
        return Ok(Some(reply::json(400, json!({ "error": "invalid_post", "message": p.join("; ") }))));
    }
    Ok(None)
}

fn post_body(partial: bool) -> z::Schema {
    let format = z::enumeration(&["png", "jpg", "mp4"]);
    let fields = [
        ("pages", z::array(z::num().int().min(0.0).max(999.0)).max(100.0).default(json!([]))),
        ("format", if partial { format.optional() } else { format }),
        ("caption", z::str().max(70_000.0).default(json!(""))),
        ("hashtags", z::array(z::str().max(100.0)).max(60.0).default(json!([]))),
        ("accounts", z::array(z::str().uuid()).max(30.0).default(json!([]))),
        ("options", z::record(z::any()).default(json!({}))),
        ("scheduledAt", z::str().datetime_offset().nullable().optional()),
        ("timezone", z::str().max(64.0).default(json!("UTC"))),
        ("status", z::enumeration(&["draft", "scheduled"]).default(json!("scheduled"))),
    ];
    if partial {
        z::obj(fields)
    } else {
        let [a, b, c, d, e, f, g, h, i] = fields;
        z::obj([("designId", z::str().min(1.0).max(64.0)), a, b, c, d, e, f, g, h, i])
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
    ])
    .parse(Some(&req.query))?;
    let qs = |k: &str| q.get(k).and_then(Value::as_str).map(str::to_owned);
    let rows: Vec<ListedPost> = sqlx::query_as(
        "SELECT p.*, d.title AS design_title FROM social_posts p JOIN designs d ON d.id = p.design_id
         WHERE (p.owner_id = $1 OR p.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1))
           AND ($2::timestamptz IS NULL OR p.scheduled_at IS NULL OR p.scheduled_at >= $2::timestamptz)
           AND ($3::timestamptz IS NULL OR p.scheduled_at IS NULL OR p.scheduled_at < $3::timestamptz)
           AND ($4::uuid IS NULL OR $4::uuid = ANY(p.accounts))
           AND ($5::text IS NULL OR p.design_id = $5)
           AND ($6::text IS NULL OR p.status = $6)
         ORDER BY p.scheduled_at NULLS LAST, p.created_at, p.id LIMIT 1000",
    )
    .bind(u.id)
    .bind(qs("from"))
    .bind(qs("to"))
    .bind(qs("account"))
    .bind(qs("designId"))
    .bind(qs("status"))
    .fetch_all(&s.pool)
    .await?;
    Ok(Reply::json(json!({ "posts": rows.iter().map(|r| public_post(&r.post, r.design_title.as_deref())).collect::<Vec<_>>() })))
}

async fn create_post(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = post_body(false).parse(req.body())?;
    let bs = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let design_id = bs("designId");
    if !at_least(design_role(&s, Some(u.id), &design_id).await?, Role::Edit) {
        return Ok(not_found());
    }
    let accounts = strings(b.get("accounts"));
    let hashtags = strings(b.get("hashtags"));
    let rows = check_accounts(&s, u.id, &accounts).await?;
    let scheduled_at = b.get("scheduledAt").and_then(Value::as_str).map(str::to_owned);
    let status = bs("status");
    if status == "scheduled"
        && let Some(r) = schedule_checks(&s, u.id, &design_id, &rows, scheduled_at.is_some(), (&bs("caption"), &hashtags, &bs("format"))).await?
    {
        return Ok(r);
    }
    let ws: Option<(Uuid, bool)> = sqlx::query_as("SELECT w.id, w.personal FROM designs d JOIN workspaces w ON w.id = d.workspace_id WHERE d.id = $1").bind(&design_id).fetch_optional(&s.pool).await?;
    let pages: Vec<i32> = b.get("pages").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).map(|f| f as i32).collect()).unwrap_or_default();
    let post: PostRow = sqlx::query_as(
        "INSERT INTO social_posts(id, owner_id, workspace_id, design_id, pages, format, caption, hashtags, accounts, options, scheduled_at, timezone, status)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::jsonb,$11::timestamptz,$12,$13) RETURNING *",
    )
    .bind(oc_db::uuid())
    .bind(u.id)
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
    .fetch_one(&s.pool)
    .await?;
    log_event(&s.pool, post.id, if status == "scheduled" { "scheduled" } else { "created" }, scheduled_at.as_deref(), None, None).await?;
    if post.status == "scheduled" {
        schedule_publish(&s.pool, post.id, post.owner_id, post.scheduled_at, s.now_ms()).await?;
    }
    Ok(Reply::json(json!({ "post": public_post(&post, None) })))
}

fn locked() -> Reply {
    reply::json(409, json!({ "error": "locked", "message": "This post was already published." }))
}

async fn update_post(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(post) = post_for(&s, u.id, &id).await? else { return Ok(not_found()) };
    if post.status == "publishing" || post.status == "published" {
        return Ok(locked());
    }
    let b = post_body(true).parse(req.body())?;
    let bs = |k: &str| b.get(k).and_then(Value::as_str).map(str::to_owned);
    let pages: Vec<i32> = match b.get("pages").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_f64).map(|f| f as i32).collect(),
        None => post.pages.clone(),
    };
    let format = bs("format").unwrap_or_else(|| post.format.clone());
    let caption = bs("caption").unwrap_or_else(|| post.caption.clone());
    let hashtags = clean_tags(&b.get("hashtags").map_or_else(|| post.hashtags.clone(), |v| strings(Some(v))));
    let accounts: Vec<String> = b.get("accounts").map_or_else(|| post.accounts.iter().map(Uuid::to_string).collect(), |v| strings(Some(v)));
    let options = b.get("options").cloned().unwrap_or_else(|| Value::Object(post.options.0.clone()));
    let scheduled_at: Option<DateTime<Utc>> = match b.get("scheduledAt") {
        None => post.scheduled_at,
        Some(Value::String(t)) => Some(parse_time(t)?),
        Some(_) => None,
    };
    let timezone = valid_tz(&bs("timezone").unwrap_or_else(|| post.timezone.clone()));
    let status = bs("status").unwrap_or_else(|| if post.status == "failed" { "scheduled".into() } else { post.status.clone() });
    let rows = check_accounts(&s, u.id, &accounts).await?;
    if status == "scheduled"
        && let Some(r) = schedule_checks(&s, u.id, &post.design_id, &rows, scheduled_at.is_some(), (&caption, &hashtags, &format)).await?
    {
        return Ok(r);
    }
    // Re-render on the next run (the design may have changed); keep results of accounts already published.
    let results: Map<String, Value> =
        post.results.0.iter().filter(|(aid, r)| r.get("status").and_then(Value::as_str) == Some("published") && accounts.contains(aid)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let updated: PostRow = sqlx::query_as(
        "UPDATE social_posts SET pages = $2, format = $3, caption = $4, hashtags = $5, accounts = $6, options = $7::jsonb, scheduled_at = $8, timezone = $9,
           status = $10, media = '[]', results = $11::jsonb, error = NULL, job_id = NULL, updated_at = now() WHERE id = $1 RETURNING *",
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
    .fetch_one(&s.pool)
    .await?;
    let moved = post.scheduled_at.map(|t| t.timestamp_millis()) != scheduled_at.map(|t| t.timestamp_millis());
    let kind = if status == "draft" {
        "updated"
    } else if moved || post.status != "scheduled" {
        "rescheduled"
    } else {
        "updated"
    };
    let message = scheduled_at.map(|t| oc_js::iso_string(&t));
    log_event(&s.pool, post.id, kind, message.as_deref(), None, None).await?;
    if updated.status == "scheduled" {
        schedule_publish(&s.pool, updated.id, updated.owner_id, updated.scheduled_at, s.now_ms()).await?;
    }
    Ok(Reply::json(json!({ "post": public_post(&updated, None) })))
}

async fn publish_now(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(post) = post_for(&s, u.id, &id).await? else { return Ok(not_found()) };
    if post.status == "publishing" || post.status == "published" {
        return Ok(locked());
    }
    if let Some(gate) = approval_gate(&s, u.id, &post.design_id).await? {
        return Ok(reply::json(403, gate));
    }
    let accounts: Vec<String> = post.accounts.iter().map(Uuid::to_string).collect();
    let rows = check_accounts(&s, u.id, &accounts).await?;
    if rows.is_empty() {
        return Ok(reply::json(400, json!({ "error": "no_accounts", "message": "Pick at least one account." })));
    }
    let p = problems(&rows, &post.caption, &post.hashtags, &post.format);
    if !p.is_empty() {
        return Ok(reply::json(400, json!({ "error": "invalid_post", "message": p.join("; ") })));
    }
    let updated: PostRow =
        sqlx::query_as("UPDATE social_posts SET status = 'scheduled', scheduled_at = now(), media = '[]', error = NULL, job_id = NULL, updated_at = now() WHERE id = $1 RETURNING *")
            .bind(post.id)
            .fetch_one(&s.pool)
            .await?;
    log_event(&s.pool, post.id, "scheduled", Some("Publish now"), None, None).await?;
    schedule_publish(&s.pool, updated.id, updated.owner_id, updated.scheduled_at, s.now_ms()).await?;
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
