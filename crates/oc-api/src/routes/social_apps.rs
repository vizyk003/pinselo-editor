//! Content Planner: a team workspace's own Meta app and the accounts connected through it.
//!
//!   GET    /api/workspaces/{id}/meta-app            the app (no secret) and whether the server has one; members
//!   PUT    /api/workspaces/{id}/meta-app            set it; checked with Meta first; owners and admins
//!   DELETE /api/workspaces/{id}/meta-app            remove it; its accounts need reconnecting; owners and admins
//!   POST   /api/workspaces/{id}/meta-app/accounts   connect Pages or Instagram accounts with a pasted Meta
//!                                                   system-user token; owners and admins
//!
//! Accounts connected here are shared with the workspace and publish through its app (`oc_social::apps`).
//! The system-user token is checked and used to read the Pages' own tokens; it is not stored.

use oc_social::apps::{delete_meta_app, link_accounts, save_meta_app, verify_meta_app, workspace_meta_app};
use oc_social::store::{SaveAccountsError, public_account, save_accounts};

use super::social::http;
use super::teams::{rank, team_role};
use crate::prelude::*;

/// Routes with their own `config.rateLimit`.
pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/workspaces/{id}/meta-app", get(read).put(save).delete(remove))
        .route("/api/workspaces/{id}/meta-app/accounts", post(connect_accounts))
}

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}
fn forbidden() -> Reply {
    reply::json(403, json!({ "error": "forbidden", "message": "Only the team’s owners and admins can change its Meta app." }))
}
fn refused(code: &str, message: &str) -> Reply {
    reply::json(400, json!({ "error": code, "message": message }))
}

/// The caller's role in a team workspace (`None`: not a member, a malformed id, or a personal workspace).
async fn role(s: &AppState, user: Uuid, id: &str) -> Result<Option<String>, ApiError> {
    let Ok(ws) = Uuid::parse_str(id) else { return Ok(None) };
    let personal: Option<bool> = sqlx::query_scalar("SELECT personal FROM workspaces WHERE id = $1").bind(ws).fetch_optional(&s.pool).await?;
    if personal != Some(false) {
        return Ok(None);
    }
    Ok(team_role(s, user, ws).await?)
}

fn is_admin(r: &Option<String>) -> bool {
    r.as_deref().is_some_and(|r| rank(r) >= rank("admin"))
}

async fn read(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let r = role(&s, u.id, &id).await?;
    if r.is_none() {
        return Ok(not_found());
    }
    let ws = Uuid::parse_str(&id).map_err(|_| ApiError::internal("uuid"))?;
    let app = workspace_meta_app(&s.pool, &s.cfg.secret, ws).await.map_err(|e| ApiError::internal(&e.message))?;
    let accounts: i64 = match &app {
        Some(a) => sqlx::query_scalar("SELECT count(*) FROM social_accounts WHERE social_app_id = $1").bind(a.id).fetch_one(&s.pool).await?,
        None => 0,
    };
    let server = oc_social::adapter("facebook").is_some_and(|a| a.configured(&http(&s)));
    Ok(Reply::json(json!({
        "app": app.map(|a| a.public()),
        "accounts": accounts,
        "serverApp": server,
        "canManage": is_admin(&r),
    })))
}

const APP_ID_RE: &str = "^[0-9]{5,25}$";
const VERSION_RE: &str = "^v[0-9]{1,3}\\.[0-9]$";

async fn save(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let r = role(&s, u.id, &id).await?;
    if r.is_none() {
        return Ok(not_found());
    }
    if !is_admin(&r) {
        return Ok(forbidden());
    }
    let b = z::obj([
        ("appId", z::str().regex(APP_ID_RE, APP_ID_RE)),
        ("appSecret", z::str().min(16.0).max(128.0).optional()),
        ("graphVersion", z::str().regex(VERSION_RE, VERSION_RE).nullable().optional()),
    ])
    .parse(req.body())?;
    let ws = Uuid::parse_str(&id).map_err(|_| ApiError::internal("uuid"))?;
    let app_id = b["appId"].as_str().unwrap_or("").to_owned();
    let version = b.get("graphVersion").and_then(Value::as_str).map(str::to_owned);
    let current = workspace_meta_app(&s.pool, &s.cfg.secret, ws).await.map_err(|e| ApiError::internal(&e.message))?;
    let secret = match b.get("appSecret").and_then(Value::as_str) {
        Some(x) => crate::js::trim(x).to_owned(),
        None => match current.as_ref().filter(|c| c.app_id == app_id) {
            Some(c) => c.app_secret.clone(),
            None => return Ok(refused("secret_required", "Enter the app secret.")),
        },
    };
    // Meta must accept the ID and secret before anything is stored.
    let name = match verify_meta_app(&http(&s), &app_id, &secret, version.as_deref()).await {
        Ok(n) => n,
        Err(e) => return Ok(refused("meta_refused", &e.message)),
    };
    let app = save_meta_app(&s.pool, &s.cfg.secret, ws, u.id, &app_id, Some(&secret), Some(&name).filter(|n| !n.is_empty()).map(String::as_str), version.as_deref())
        .await
        .map_err(|e| ApiError::internal(&e.message))?;
    let ev = oc_audit::AuditEvent {
        actor_id: Some(u.id),
        workspace_id: Some(ws),
        action: "social.app.save".into(),
        meta: Some(json!({ "provider": "meta", "appId": app_id, "changedApp": current.is_some_and(|c| c.app_id != app_id) })),
        ..Default::default()
    };
    oc_audit::audit_at(&s.pool, ev, s.now()).await?;
    Ok(Reply::json(json!({ "app": app.public() })))
}

async fn remove(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let r = role(&s, u.id, &id).await?;
    if r.is_none() {
        return Ok(not_found());
    }
    if !is_admin(&r) {
        return Ok(forbidden());
    }
    let ws = Uuid::parse_str(&id).map_err(|_| ApiError::internal("uuid"))?;
    if !delete_meta_app(&s.pool, ws).await.map_err(|e| ApiError::internal(&e.message))? {
        return Ok(not_found());
    }
    let ev = oc_audit::AuditEvent { actor_id: Some(u.id), workspace_id: Some(ws), action: "social.app.delete".into(), meta: Some(json!({ "provider": "meta" })), ..Default::default() };
    oc_audit::audit_at(&s.pool, ev, s.now()).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn connect_accounts(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let r = role(&s, u.id, &id).await?;
    if r.is_none() {
        return Ok(not_found());
    }
    if !is_admin(&r) {
        return Ok(forbidden());
    }
    let b = z::obj([
        ("provider", z::enumeration(&["facebook", "instagram"])),
        ("token", z::str().min(20.0).max(1000.0)),
    ])
    .parse(req.body())?;
    let ws = Uuid::parse_str(&id).map_err(|_| ApiError::internal("uuid"))?;
    let provider = b["provider"].as_str().unwrap_or("").to_owned();
    let token = crate::js::trim(b["token"].as_str().unwrap_or("")).to_owned();
    let Some(app) = workspace_meta_app(&s.pool, &s.cfg.secret, ws).await.map_err(|e| ApiError::internal(&e.message))? else {
        return Ok(refused("no_app", "Set up the team’s Meta app first."));
    };
    let list = match oc_social::adapters::meta::connect_system_user(&app.http(&http(&s)), &provider, &token).await {
        Ok(l) => l,
        Err(e) => return Ok(refused("connect_failed", &e.message)),
    };
    let ids = match save_accounts(&s.pool, &s.cfg.secret, u.id, &provider, &list).await {
        Ok(ids) => ids,
        Err(SaveAccountsError::Refused(e)) => return Ok(refused("connect_failed", &e.message)),
        Err(SaveAccountsError::Db(e)) => return Err(e.into()),
    };
    link_accounts(&s.pool, &ids, Some(ws), Some(&app), "system_user").await.map_err(|e| ApiError::internal(&e.message))?;
    let ev = oc_audit::AuditEvent {
        actor_id: Some(u.id),
        workspace_id: Some(ws),
        action: "social.connect".into(),
        meta: Some(json!({ "provider": provider, "accounts": ids.len(), "via": "system_user" })),
        ..Default::default()
    };
    oc_audit::audit_at(&s.pool, ev, s.now()).await?;
    let rows: Vec<oc_social::store::AccountRow> = sqlx::query_as("SELECT * FROM social_accounts WHERE id = ANY($1) ORDER BY display_name").bind(&ids).fetch_all(&s.pool).await?;
    Ok(Reply::json(json!({ "accounts": rows.iter().map(|r| public_account(r, Some(u.id))).collect::<Vec<_>>() })))
}
