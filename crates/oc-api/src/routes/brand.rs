//! Port of services/api/src/routes/brand.ts — the legacy single brand kit (CA-060..062), notifications (CO-070)
//! and public view links (CO-034). `GET /api/public/designs/:id` needs no sign-in; every other route runs
//! `requireUser` first.
//!
//! Also exported for other modules: [`notify`].

use oc_auth::Role;
use serde_json::Map;

use crate::auth::{design_role, hash_password, personal_workspace, verify_password};
use crate::prelude::*;
use crate::routes::{brandhub as brandhub_routes, designs};

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/public/designs/{id}", get(public_design))
        .route("/api/brand", get(get_kit).put(put_kit))
        .route("/api/notifications", get(notifications))
        .route("/api/notifications/read", post(mark_read))
        .route("/api/designs/{id}/public", post(set_public).get(get_public))
}

// ---------------------------------------------------------------- shared helpers

/// `notify(userId, n)`: an in-app notification (never to the actor themself).
pub async fn notify(state: &AppState, user_id: Uuid, kind: &str, design_id: &str, actor_id: Uuid, body: &str) -> Result<(), sqlx::Error> {
    if user_id == actor_id {
        return Ok(());
    }
    sqlx::query("INSERT INTO notifications(id, user_id, kind, design_id, actor_id, body) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(oc_db::uuid())
        .bind(user_id)
        .bind(kind)
        .bind(design_id)
        .bind(actor_id)
        .bind(crate::js::take(body, 300))
        .execute(&state.pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------- public view (no sign-in)

/// `:id` is a design id or a custom link slug (g131); the id only comes back once the link checks below pass.
async fn public_design(State(s): State<AppState>, Path(key): Path<String>, req: Req) -> ApiResult {
    type Row = (String, String, bool, Option<chrono::DateTime<chrono::Utc>>, String, Option<chrono::DateTime<chrono::Utc>>, Option<String>);
    let by_id = crate::share_links::looks_like_design_id(&key);
    let d: Option<Row> = sqlx::query_as(&format!(
        "SELECT d.id, d.title, d.public_view, d.trashed_at, u.name AS owner, d.public_expires_at, d.public_password_hash FROM designs d JOIN users u ON u.id = d.owner_id
       WHERE {}",
        if by_id { "d.id = $1" } else { "d.public_slug = $1" }
    ))
    .bind(if by_id { key.clone() } else { crate::share_links::normalize_slug(&key) })
    .fetch_optional(&s.pool)
    .await?;
    let Some((id, title, public_view, trashed, owner, expires, password_hash)) = d.filter(|d| d.2 && d.3.is_none()) else {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    };
    let _ = (public_view, trashed);
    // Link controls (CO-041): expiry and an optional password (sent as X-View-Password).
    if expires.is_some_and(|e| e.timestamp_millis() < s.now_ms()) {
        return Ok(reply::json(410, json!({ "error": "expired", "message": "This link has expired." })));
    }
    let mut cache = "public, max-age=30";
    if let Some(h) = password_hash.as_deref().filter(|h| !h.is_empty()) {
        let pw = req.header("x-view-password").unwrap_or("");
        if pw.is_empty() || !verify_password(pw, Some(h)).await {
            return Ok(reply::json(401, json!({ "error": "password_required", "title": title })));
        }
        cache = "private, no-store";
    }
    let doc = designs::load_doc(&s, &id).await?;
    Ok(Reply::json(json!({ "id": id, "title": title, "owner": owner, "doc": doc })).header("cache-control", cache))
}

// ---------------------------------------------------------------- brand kit (legacy single-kit API)

fn brand_body() -> z::Schema {
    let hex = || z::str().regex("^#[0-9a-fA-F]{6}$", "^#[0-9a-fA-F]{6}$");
    z::obj([
        ("name", z::str().min(1.0).max(80.0)),
        ("colors", z::array(z::obj([("name", z::str().max(40.0)), ("hex", hex())])).max(40.0)),
        ("fonts", z::obj([("heading", z::str().max(60.0).optional()), ("subheading", z::str().max(60.0).optional()), ("body", z::str().max(60.0).optional())])),
        ("logos", z::array(z::str().regex("^[0-9A-Za-z]{16}$", "^[0-9A-Za-z]{16}$")).max(20.0)),
    ])
}

async fn get_kit(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let kit = brandhub_routes::default_kit(&s, personal_workspace(&s, u.id).await?).await?;
    Ok(Reply::json(json!({ "kit": kit.unwrap_or_else(|| json!({ "name": "Brand kit", "colors": [], "fonts": {}, "logos": [] })) })))
}

async fn put_kit(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = brand_body().parse(req.body())?;
    let ws = personal_workspace(&s, u.id).await?;
    let cur = brandhub_routes::default_kit(&s, ws).await?;
    // { palettes: [], photos: [], graphics: [], voice: '', ...cur, ...b, fonts: { ...cur?.fonts, ...b.fonts } }
    let mut kit = Map::new();
    kit.insert("palettes".into(), json!([]));
    kit.insert("photos".into(), json!([]));
    kit.insert("graphics".into(), json!([]));
    kit.insert("voice".into(), json!(""));
    if let Some(Value::Object(c)) = &cur {
        for (k, v) in c {
            kit.insert(k.clone(), v.clone());
        }
    }
    if let Value::Object(bm) = &b {
        for (k, v) in bm {
            kit.insert(k.clone(), v.clone());
        }
    }
    let mut fonts = cur.as_ref().and_then(|c| c.get("fonts")).and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(bf) = b.get("fonts").and_then(Value::as_object) {
        for (k, v) in bf {
            fonts.insert(k.clone(), v.clone());
        }
    }
    kit.insert("fonts".into(), Value::Object(fonts));
    let id = cur.as_ref().and_then(|c| c.get("id")).and_then(Value::as_str).map(str::to_owned);
    brandhub_routes::save_kit(&s, ws, u.id, &Value::Object(kit), id.as_deref()).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- notifications

async fn notifications(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let items = pg::json_rows(
        &s.pool,
        "SELECT n.id, n.kind, n.design_id, n.body, n.read_at, n.created_at, u.name AS actor, u.color AS actor_color, d.title AS design_title
         FROM notifications n LEFT JOIN users u ON u.id = n.actor_id LEFT JOIN designs d ON d.id = n.design_id
         WHERE n.user_id = $1 ORDER BY n.created_at DESC LIMIT 50",
        &[pg::uuid(u.id)],
    )
    .await?;
    let unread: Option<i32> = sqlx::query_scalar("SELECT count(*)::int AS n FROM notifications WHERE user_id = $1 AND read_at IS NULL").bind(u.id).fetch_optional(&s.pool).await?;
    Ok(Reply::json(json!({ "items": items, "unread": unread.unwrap_or(0) })))
}

async fn mark_read(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("ids", z::array(z::str().uuid()).optional())]).parse(Some(&req.body_or_empty()))?;
    let ids: Vec<String> = b.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
    if ids.is_empty() {
        sqlx::query("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND read_at IS NULL").bind(u.id).execute(&s.pool).await?;
    } else {
        sqlx::query("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND id = ANY($2::uuid[])").bind(u.id).bind(&ids).execute(&s.pool).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- public link toggle (owner)

async fn set_public(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([
        ("enabled", z::bool()),
        ("expiresAt", z::str().datetime().nullable().optional()),
        ("password", z::str().min(4.0).max(200.0).nullable().optional()),
    ])
    .parse(req.body())?;
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(reply::json(403, json!({ "error": "forbidden" })));
    }
    let enabled = b["enabled"].as_bool().unwrap_or(false);
    sqlx::query("UPDATE designs SET public_view = $2 WHERE id = $1").bind(&id).bind(enabled).execute(&s.pool).await?;
    if let Some(e) = b.get("expiresAt") {
        sqlx::query("UPDATE designs SET public_expires_at = $2::timestamptz WHERE id = $1").bind(&id).bind(e.as_str()).execute(&s.pool).await?;
    }
    if let Some(p) = b.get("password") {
        let h = match p.as_str().filter(|p| !p.is_empty()) {
            Some(p) => Some(hash_password(p).await),
            None => None,
        };
        sqlx::query("UPDATE designs SET public_password_hash = $2 WHERE id = $1").bind(&id).bind(h).execute(&s.pool).await?;
    }
    let mut meta = Map::new();
    meta.insert("enabled".into(), json!(enabled));
    if let Some(e) = b.get("expiresAt").filter(|e| !e.is_null()) {
        meta.insert("expires".into(), e.clone());
    }
    if let Some(p) = b.get("password") {
        meta.insert("password".into(), json!(p.as_str().is_some_and(|p| !p.is_empty())));
    }
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: Some(u.id), action: "design.public".into(), target: Some(id.clone()), meta: Some(Value::Object(meta)), ip: Some(req.ip.clone()), ..Default::default() },
        s.now(),
    )
    .await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn get_public(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(reply::json(403, json!({ "error": "forbidden" })));
    }
    let d = pg::json_one(&s.pool, "SELECT public_view, public_expires_at, public_password_hash FROM designs WHERE id = $1", &[pg::text(&id)]).await?.unwrap_or(Value::Null);
    let has_pw = d.get("public_password_hash").and_then(Value::as_str).is_some_and(|h| !h.is_empty());
    Ok(Reply::json(json!({
        "enabled": d.get("public_view").and_then(Value::as_bool).unwrap_or(false),
        "expiresAt": d.get("public_expires_at").cloned().unwrap_or(Value::Null),
        "hasPassword": has_pw,
    })))
}
