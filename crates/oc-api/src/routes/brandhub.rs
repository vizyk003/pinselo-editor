//! Port of services/api/src/routes/brandhub.ts — Brand Hub (A8): multiple brand kits per workspace, brand
//! controls (team settings), brand-template info for the editor, "update designs from template", and the design
//! approval workflow. Every route runs `requireUser` first (the plugin's `preHandler`).
//!
//! Also exported for other modules: [`KIT_COLS`], [`default_kit`], [`save_kit`], [`accessible_kits`].

use serde_json::Map;

use crate::auth::{at_least, design_role, personal_workspace};
use crate::billing;
use crate::brandhub::{self, approval_current, approval_gate, brand_controls, control_on, governing_workspace, is_admin_role, is_brand_admin, role_in, team_entitlements, CONTROL_KEYS};
use crate::prelude::*;
use crate::routes::{brand, designs, teams};
use oc_auth::Role;

/// Routes with their own `config.rateLimit`.
pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/brand-kits", get(list_kits).post(create_kit))
        .route("/api/brand-kits/{id}", get(read_kit).patch(patch_kit).delete(delete_kit))
        .route("/api/brand-kits/{id}/default", post(default_kit_route))
        .route("/api/workspaces/{id}/brand-controls", get(get_controls).put(put_controls))
        .route("/api/designs/{id}/brand", get(design_brand))
        .route("/api/templates/{id}/republish", post(republish))
        .route("/api/templates/{id}/push", post(push))
        .route("/api/designs/{id}/approval", get(get_approval).post(request_approval).delete(cancel_approval))
        .route("/api/designs/{id}/approval/decision", post(decide))
        .route("/api/approvals", get(queue))
        .route("/api/designs/{id}/approval/gate", get(gate))
}

// ---------------------------------------------------------------- shared helpers

pub const KIT_COLS: &str = r#"k.id, k.workspace_id AS "workspaceId", k.name, k.is_default AS "isDefault", k.colors, k.palettes, k.fonts, k.logos, k.photos, k.graphics, k.voice, k.updated_at AS "updatedAt""#;

/// `defaultKit(workspaceId)`: the workspace's default kit as a row (`KIT_COLS`).
pub async fn default_kit(state: &AppState, workspace_id: uuid::Uuid) -> Result<Option<serde_json::Value>, sqlx::Error> {
    crate::pg::json_one(&state.pool, &format!("SELECT {KIT_COLS} FROM brand_kits k WHERE k.workspace_id = $1 ORDER BY k.is_default DESC, k.created_at LIMIT 1"), &[crate::pg::uuid(workspace_id)]).await
}

/// `saveKit(workspaceId, userId, kit, id?)`: insert or update; returns the kit id. `kit` is the merged JS object.
pub async fn save_kit(state: &AppState, workspace_id: uuid::Uuid, user_id: uuid::Uuid, kit: &serde_json::Value, id: Option<&str>) -> Result<String, sqlx::Error> {
    let kid = id.map_or_else(|| oc_db::uuid().to_string(), str::to_owned);
    let js = |k: &str| crate::http::jsjson::stringify(kit.get(k).unwrap_or(&serde_json::Value::Null));
    sqlx::query(
        "INSERT INTO brand_kits(id, workspace_id, name, is_default, colors, palettes, fonts, logos, photos, graphics, voice, created_by, updated_at)
     VALUES ($1::uuid,$2,$3, NOT EXISTS (SELECT 1 FROM brand_kits WHERE workspace_id = $2 AND is_default), $4::jsonb,$5::jsonb,$6::jsonb,$7::jsonb,$8::jsonb,$9::jsonb,$10,$11, now())
     ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, colors = EXCLUDED.colors, palettes = EXCLUDED.palettes, fonts = EXCLUDED.fonts, logos = EXCLUDED.logos,
       photos = EXCLUDED.photos, graphics = EXCLUDED.graphics, voice = EXCLUDED.voice, updated_at = now()",
    )
    .bind(&kid)
    .bind(workspace_id)
    .bind(kit.get("name").and_then(serde_json::Value::as_str))
    .bind(js("colors"))
    .bind(js("palettes"))
    .bind(js("fonts"))
    .bind(js("logos"))
    .bind(js("photos"))
    .bind(js("graphics"))
    .bind(kit.get("voice").and_then(serde_json::Value::as_str))
    .bind(user_id)
    .execute(&state.pool)
    .await?;
    Ok(kid)
}

/// `accessibleKits(userId)`: brand kits every workspace of the user can see.
pub async fn accessible_kits(state: &AppState, user_id: Uuid) -> Result<Vec<Value>, sqlx::Error> {
    let kits = pg::json_rows(
        &state.pool,
        &format!(
            "SELECT {KIT_COLS}, w.name AS workspace, w.personal, m.role FROM brand_kits k JOIN workspaces w ON w.id = k.workspace_id
     JOIN memberships m ON m.workspace_id = k.workspace_id AND m.user_id = $1 ORDER BY w.personal DESC, w.name, k.is_default DESC, k.created_at"
        ),
        &[pg::uuid(user_id)],
    )
    .await?;
    Ok(kits
        .into_iter()
        .map(|mut k| {
            let can = k.get("personal") == Some(&Value::Bool(true)) || k.get("role").and_then(Value::as_str) != Some("member");
            if let Some(o) = k.as_object_mut() {
                o.insert("canEdit".into(), json!(can));
            }
            k
        })
        .collect())
}

fn hex() -> z::Schema {
    z::str().regex("^#[0-9a-fA-F]{6}$", "^#[0-9a-fA-F]{6}$")
}
fn asset_id() -> z::Schema {
    z::str().regex("^[0-9A-Za-z]{16}$", "^[0-9A-Za-z]{16}$")
}
fn color() -> z::Schema {
    z::obj([("name", z::str().max(40.0)), ("hex", hex())])
}
fn font() -> z::Schema {
    z::str().max(80.0).optional()
}

/// `KitBody` (`partial`: `KitBody.partial()`, whose defaults still apply in zod 4) plus `workspaceId` when
/// `extend` (the create route).
fn kit_body(partial: bool, extend: bool) -> z::Schema {
    let name = z::str().min(1.0).max(80.0);
    let fields = [
        ("name", if partial { name.optional() } else { name }),
        ("colors", z::array(color()).max(60.0).default(json!([]))),
        ("palettes", z::array(z::obj([("name", z::str().max(60.0)), ("colors", z::array(color()).max(40.0))])).max(20.0).default(json!([]))),
        ("fonts", z::obj([("heading", font()), ("subheading", font()), ("body", font()), ("quote", font()), ("caption", font())]).default(json!({}))),
        ("logos", z::array(asset_id()).max(30.0).default(json!([]))),
        ("photos", z::array(asset_id()).max(200.0).default(json!([]))),
        ("graphics", z::array(asset_id()).max(200.0).default(json!([]))),
        ("voice", z::str().max(4000.0).default(json!(""))),
    ];
    if extend {
        let [a, b, c, d, e, f, g, h] = fields;
        z::obj([a, b, c, d, e, f, g, h, ("workspaceId", z::str().uuid().optional())])
    } else {
        z::obj(fields)
    }
}

/// `kitRole(userId, workspaceId)`: the personal owner and team owners, admins and designers edit; members view.
async fn kit_role(s: &AppState, user_id: Uuid, workspace_id: Uuid) -> Result<Option<&'static str>, sqlx::Error> {
    let r: Option<(String, bool)> = sqlx::query_as("SELECT m.role, w.personal FROM memberships m JOIN workspaces w ON w.id = m.workspace_id WHERE m.workspace_id = $1 AND m.user_id = $2")
        .bind(workspace_id)
        .bind(user_id)
        .fetch_optional(&s.pool)
        .await?;
    Ok(r.map(|(role, personal)| if personal || role != "member" { "edit" } else { "view" }))
}

async fn kit_limit(s: &AppState, user_id: Uuid, workspace_id: Uuid) -> Result<i64, ApiError> {
    let personal: Option<bool> = sqlx::query_scalar("SELECT personal FROM workspaces WHERE id = $1").bind(workspace_id).fetch_optional(&s.pool).await?;
    Ok(if personal == Some(true) { billing::entitlements_of(s, user_id).await?.0.brand_kits } else { team_entitlements(s, workspace_id).await?.brand_kits.max(1) })
}

struct OwnKit {
    workspace_id: Uuid,
    is_default: bool,
    role: Option<&'static str>,
}

async fn own_kit(s: &AppState, user_id: Uuid, id: &str) -> Result<Option<OwnKit>, sqlx::Error> {
    if id.len() != 36 || !id.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || c == b'-') {
        return Ok(None);
    }
    let k: Option<(Uuid, bool)> = sqlx::query_as("SELECT workspace_id, is_default FROM brand_kits WHERE id = $1::uuid").bind(id).fetch_optional(&s.pool).await?;
    let Some((workspace_id, is_default)) = k else { return Ok(None) };
    Ok(Some(OwnKit { workspace_id, is_default, role: kit_role(s, user_id, workspace_id).await? }))
}

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

fn forbidden() -> Reply {
    reply::json(403, json!({ "error": "forbidden" }))
}

async fn audit(s: &AppState, req: &Req, actor: Uuid, workspace: Option<Uuid>, action: &str, target: Option<String>, meta: Option<Value>) -> Result<(), sqlx::Error> {
    oc_audit::audit_at(&s.pool, oc_audit::AuditEvent { actor_id: Some(actor), workspace_id: workspace, action: action.into(), target, meta, ip: Some(req.ip.clone()) }, s.now()).await
}

fn opt_uuid(u: Option<Uuid>) -> Value {
    u.map_or(Value::Null, |u| json!(u.to_string()))
}

// ---------------------------------------------------------------- brand kits

async fn list_kits(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    type Ws = (Uuid, String, bool, String, sqlx::types::Json<Value>);
    let workspaces: Vec<Ws> = sqlx::query_as(
        "SELECT w.id, w.name, w.personal, m.role, w.brand_controls FROM memberships m JOIN workspaces w ON w.id = m.workspace_id WHERE m.user_id = $1 ORDER BY w.personal DESC, w.name",
    )
    .bind(u.id)
    .fetch_all(&s.pool)
    .await?;
    let kits = accessible_kits(&s, u.id).await?;
    let ws: Vec<Value> = workspaces
        .into_iter()
        .map(|(id, name, personal, role, controls)| {
            let can = personal || role != "member";
            json!({ "id": id.to_string(), "name": name, "personal": personal, "role": role, "canEdit": can, "controls": if personal { json!({}) } else { controls.0 } })
        })
        .collect();
    Ok(Reply::json(json!({ "kits": kits, "workspaces": ws })))
}

async fn create_kit(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = kit_body(false, true).parse(req.body())?;
    let ws = match b.get("workspaceId").and_then(Value::as_str) {
        Some(w) => Uuid::parse_str(w).map_err(ApiError::internal)?,
        None => personal_workspace(&s, u.id).await?,
    };
    if kit_role(&s, u.id, ws).await? != Some("edit") {
        return Ok(reply::json(403, json!({ "error": "forbidden", "message": "Only owners, admins and designers can create brand kits." })));
    }
    let n: i32 = sqlx::query_scalar("SELECT count(*)::int AS n FROM brand_kits WHERE workspace_id = $1").bind(ws).fetch_optional(&s.pool).await?.unwrap_or(0);
    let limit = kit_limit(&s, u.id, ws).await?;
    if i64::from(n) >= limit {
        let message = if limit == 1 { "The Free plan includes one brand kit. Upgrade to Pro or Teams for more.".to_owned() } else { format!("This plan includes up to {limit} brand kits.") };
        return Err(ApiError::Upgrade { feature: "brandKits".into(), message });
    }
    let mut kit = b.as_object().cloned().unwrap_or_default();
    kit.remove("workspaceId");
    let id = save_kit(&s, ws, u.id, &Value::Object(kit), None).await?;
    audit(&s, &req, u.id, Some(ws), "brand.kit_created", Some(id.clone()), None).await?;
    Ok(Reply::json(json!({ "id": id })))
}

async fn read_kit(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(k) = own_kit(&s, u.id, &id).await?.filter(|k| k.role.is_some()) else { return Ok(not_found()) };
    let row = pg::json_one(&s.pool, &format!("SELECT {KIT_COLS} FROM brand_kits k WHERE k.id = $1::uuid"), &[pg::text(&id)]).await?;
    let mut kit = row.and_then(|r| r.as_object().cloned()).unwrap_or_default();
    kit.insert("canEdit".into(), json!(k.role == Some("edit")));
    Ok(Reply::json(json!({ "kit": kit })))
}

async fn patch_kit(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let patch = kit_body(true, false).parse(req.body())?;
    let Some(k) = own_kit(&s, u.id, &id).await?.filter(|k| k.role.is_some()) else { return Ok(not_found()) };
    if k.role != Some("edit") {
        return Ok(forbidden());
    }
    let cur = pg::json_one(&s.pool, "SELECT name, colors, palettes, fonts, logos, photos, graphics, voice FROM brand_kits WHERE id = $1::uuid", &[pg::text(&id)]).await?;
    let mut next = cur.and_then(|c| c.as_object().cloned()).unwrap_or_default();
    // Only the fields actually sent: the schema's defaults would otherwise empty the rest of the kit.
    let sent = req.body().and_then(Value::as_object);
    if let Value::Object(p) = patch {
        for (key, v) in p {
            if sent.is_some_and(|b| b.contains_key(&key)) {
                next.insert(key, v);
            }
        }
    }
    save_kit(&s, k.workspace_id, u.id, &Value::Object(next), Some(&id)).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn default_kit_route(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(k) = own_kit(&s, u.id, &id).await?.filter(|k| k.role.is_some()) else { return Ok(not_found()) };
    if k.role != Some("edit") {
        return Ok(forbidden());
    }
    let mut t = s.pool.begin().await?;
    pg::exec(&mut *t, "UPDATE brand_kits SET is_default = false WHERE workspace_id = $1 AND is_default", &[pg::uuid(k.workspace_id)]).await?;
    pg::exec(&mut *t, "UPDATE brand_kits SET is_default = true WHERE id = $1::uuid", &[pg::text(&id)]).await?;
    t.commit().await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn delete_kit(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(k) = own_kit(&s, u.id, &id).await?.filter(|k| k.role.is_some()) else { return Ok(not_found()) };
    if k.role != Some("edit") {
        return Ok(forbidden());
    }
    let mut t = s.pool.begin().await?;
    pg::exec(&mut *t, "DELETE FROM brand_kits WHERE id = $1::uuid", &[pg::text(&id)]).await?;
    // Keep exactly one default while kits remain.
    if k.is_default {
        pg::exec(&mut *t, "UPDATE brand_kits SET is_default = true WHERE id = (SELECT id FROM brand_kits WHERE workspace_id = $1 ORDER BY created_at LIMIT 1)", &[pg::uuid(k.workspace_id)]).await?;
    }
    t.commit().await?;
    audit(&s, &req, u.id, Some(k.workspace_id), "brand.kit_deleted", Some(id.clone()), None).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- brand controls (team settings, admins)

async fn get_controls(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(role) = teams::team_role_str(&s, &u.id.to_string(), &id).await? else { return Ok(not_found()) };
    let ws = Uuid::parse_str(&id).map_err(ApiError::internal)?;
    let e = team_entitlements(&s, ws).await?;
    Ok(Reply::json(json!({
        "controls": brand_controls(&s, Some(ws)).await?,
        "canEdit": is_admin_role(Some(&role)),
        "role": role,
        "entitled": { "brandControls": e.brand_controls, "approvals": e.approvals },
    })))
}

async fn put_controls(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj(CONTROL_KEYS.map(|k| (k, z::bool().optional()))).parse(req.body())?;
    let personal: Option<bool> = sqlx::query_scalar("SELECT personal FROM workspaces WHERE id = $1::uuid").bind(&id).fetch_optional(&s.pool).await?;
    let admin = match personal {
        Some(_) => is_admin_role(teams::team_role_str(&s, &u.id.to_string(), &id).await?.as_deref()),
        None => false,
    };
    let Some(personal) = personal.filter(|_| admin) else {
        return Ok(reply::json(403, json!({ "error": "forbidden", "message": "Only team owners and admins can change brand controls." })));
    };
    if personal {
        return Ok(reply::json(400, json!({ "error": "team_only", "message": "Brand controls apply to teams." })));
    }
    let ws = Uuid::parse_str(&id).map_err(ApiError::internal)?;
    let mut next = brand_controls(&s, Some(ws)).await?;
    if let Value::Object(m) = b {
        for (k, v) in m {
            next.insert(k, v);
        }
    }
    let e = team_entitlements(&s, ws).await?;
    if ["colors", "fonts", "requireTemplate", "restrictUploads"].iter().any(|k| control_on(&next, k)) && !e.brand_controls {
        return Err(ApiError::Upgrade { feature: "brandControls".into(), message: "Brand controls are part of the Teams plan.".into() });
    }
    if control_on(&next, "requireApproval") && !e.approvals {
        return Err(ApiError::Upgrade { feature: "approvals".into(), message: "Required approvals are part of the Teams plan.".into() });
    }
    let next = Value::Object(next);
    pg::exec(&s.pool, "UPDATE workspaces SET brand_controls = $2::jsonb WHERE id = $1", &[pg::uuid(ws), pg::text(crate::http::jsjson::stringify(&next))]).await?;
    audit(&s, &req, u.id, Some(ws), "brand.controls_changed", None, Some(next.clone())).await?;
    Ok(Reply::json(json!({ "controls": next })))
}

// ---------------------------------------------------------------- per-design brand context (editor)

async fn design_brand(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    let ws = governing_workspace(&s, &id).await?;
    type Row = (Option<String>, Option<i32>, Option<bool>, Option<i32>, Option<String>);
    let d: Option<Row> = sqlx::query_as("SELECT d.template_id, d.template_version, t.brand, t.version, t.title FROM designs d LEFT JOIN templates t ON t.id = d.template_id WHERE d.id = $1")
        .bind(&id)
        .fetch_optional(&s.pool)
        .await?;
    let a = brandhub::latest_approval(&s, &id).await?;
    let role = role_in(&s, u.id, ws).await?;
    let brand_admin = is_brand_admin(&s, u.id, &id).await?;
    let controls = brand_controls(&s, ws).await?;
    let template = match d {
        Some((Some(tid), tv, brand, version, title)) => json!({ "id": tid, "title": title, "brand": brand == Some(true), "version": tv.unwrap_or(1), "latest": version.unwrap_or(1) }),
        _ => Value::Null,
    };
    let approval = match &a {
        Some(a) => json!({ "status": a.status, "current": approval_current(&s, Some(a)).await? }),
        None => Value::Null,
    };
    let published = pg::json_rows(&s.pool, "SELECT id, title, version, brand FROM templates WHERE source_design_id = $1 ORDER BY created_at", &[pg::text(&id)]).await?;
    Ok(Reply::json(json!({
        "workspaceId": opt_uuid(ws), "role": role, "brandAdmin": brand_admin, "controls": controls, "template": template, "approval": approval, "published": published,
    })))
}

// ---------------------------------------------------------------- brand templates: republish + push updates

/// Re-snapshot the template from its source design (admins/designers); bumps the version.
async fn republish(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let t: Option<(Option<Uuid>, Option<String>, bool)> = sqlx::query_as("SELECT workspace_id, source_design_id, brand FROM templates WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some((Some(ws), Some(source), brand)) = t else { return Ok(not_found()) };
    let role = role_in(&s, u.id, Some(ws)).await?;
    if role.as_deref().is_none_or(|r| r == "member") {
        return Ok(forbidden());
    }
    let Some(doc) = designs::load_doc(&s, &source).await? else { return Ok(reply::json(400, json!({ "error": "empty" }))) };
    let locks = if brand { Value::Object(brandhub::brand_locks_of(&doc)) } else { json!({}) };
    let dims = doc.pointer("/meta/dims");
    let dim = |k: &str| dims.and_then(|d| d.get(k)).and_then(Value::as_f64);
    sqlx::query("UPDATE templates SET doc = $2::jsonb, locks = $3::jsonb, width = $4, height = $5, version = version + 1, updated_at = now() WHERE id = $1")
        .bind(&id)
        .bind(crate::http::jsjson::stringify(&doc))
        .bind(crate::http::jsjson::stringify(&locks))
        .bind(dim("w"))
        .bind(dim("h"))
        .execute(&s.pool)
        .await?;
    let version: i32 = sqlx::query_scalar("SELECT version FROM templates WHERE id = $1").bind(&id).fetch_one(&s.pool).await?;
    Ok(Reply::json(json!({ "ok": true, "version": version })))
}

/// "Update designs from template" (admins): re-applies the template's brand-locked elements (geometry, content
/// and style as locked) to every design created from it that is on an older version. Unlocked elements and
/// members' own additions are left alone.
async fn push(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    type Tpl = (Option<Uuid>, sqlx::types::Json<Value>, sqlx::types::Json<Value>, i32, bool);
    let t: Option<Tpl> = sqlx::query_as("SELECT workspace_id, doc, locks, version, brand FROM templates WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some((Some(ws), tdoc, locks, version, true)) = t else { return Ok(not_found()) };
    if !is_admin_role(role_in(&s, u.id, Some(ws)).await?.as_deref()) {
        return Ok(reply::json(403, json!({ "error": "forbidden", "message": "Only team owners and admins can update designs from a template." })));
    }
    let locks = locks.0.as_object().cloned().unwrap_or_default();
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM designs WHERE template_id = $1 AND trashed_at IS NULL AND COALESCE(template_version, 1) < $2").bind(&id).bind(version).fetch_all(&s.pool).await?;
    let mut updated = 0;
    let mut failed: Vec<String> = Vec::new();
    for did in &ids {
        // Current state = snapshot + op-log tail (what the collab server holds).
        let Some(y) = brandhub::load_ydoc(&s, did).await? else { continue };
        let ops = {
            let cur = oc_crdt::from_y_value(&y);
            drop(y);
            brandhub::template_update_ops(&tdoc.0, &cur, &locks)
        };
        let ok = if ops.is_empty() { true } else { brandhub::apply_server_ops(&s, did, &ops).await?.is_ok() };
        if ok {
            updated += 1;
            pg::exec(&s.pool, "UPDATE designs SET template_version = $2 WHERE id = $1", &[pg::text(did), Arg::Int4(Some(version))]).await?;
        } else {
            failed.push(did.clone());
        }
    }
    audit(&s, &req, u.id, Some(ws), "template.pushed", Some(id.clone()), Some(json!({ "updated": updated, "failed": failed.len() }))).await?;
    Ok(Reply::json(json!({ "updated": updated, "failed": failed, "total": ids.len() })))
}

// ---------------------------------------------------------------- approvals

/// A whole `design_approvals` row (`latestApproval`).
#[derive(sqlx::FromRow)]
struct Approval {
    id: Uuid,
    requested_by: Option<Uuid>,
    reviewers: Vec<Uuid>,
    message: String,
    status: String,
    decided_by: Option<Uuid>,
    decision_note: Option<String>,
    decided_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn latest(s: &AppState, design_id: &str) -> Result<Option<(Approval, brandhub::ApprovalRow)>, ApiError> {
    let full: Option<Approval> = sqlx::query_as("SELECT * FROM design_approvals WHERE design_id = $1 AND status <> 'cancelled' ORDER BY created_at DESC LIMIT 1").bind(design_id).fetch_optional(&s.pool).await?;
    let Some(full) = full else { return Ok(None) };
    let row = brandhub::latest_approval(s, design_id).await?;
    Ok(row.map(|r| (full, r)))
}

async fn people(s: &AppState, ids: &[Uuid]) -> Result<Vec<Value>, sqlx::Error> {
    sqlx::query("SELECT id, name, email, color FROM users WHERE id = ANY($1::uuid[])").bind(ids).fetch_all(&s.pool).await?.iter().map(pg::row).collect()
}

async fn admins_of(s: &AppState, ws: Uuid) -> Result<Vec<Value>, sqlx::Error> {
    pg::json_rows(
        &s.pool,
        "SELECT u.id, u.name, u.email, u.color FROM memberships m JOIN users u ON u.id = m.user_id WHERE m.workspace_id = $1 AND m.role IN ('owner','admin') ORDER BY u.name",
        &[pg::uuid(ws)],
    )
    .await
}

async fn can_review(s: &AppState, user_id: Uuid, reviewers: &[Uuid], ws: Option<Uuid>) -> Result<bool, ApiError> {
    Ok(reviewers.contains(&user_id) || is_admin_role(role_in(s, user_id, ws).await?.as_deref()))
}

fn iso(t: Option<chrono::DateTime<chrono::Utc>>) -> Value {
    t.map_or(Value::Null, |t| json!(oc_js::iso_string(&t)))
}

async fn get_approval(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    let ws = governing_workspace(&s, &id).await?;
    let a = latest(&s, &id).await?;
    let required = control_on(&brand_controls(&s, ws).await?, "requireApproval");
    let admins = match ws {
        Some(w) => admins_of(&s, w).await?,
        None => vec![],
    };
    let Some((a, row)) = a else { return Ok(Reply::json(json!({ "approval": null, "required": required, "suggestedReviewers": admins }))) };
    let mut ids = a.reviewers.clone();
    ids.extend(a.requested_by);
    ids.extend(a.decided_by);
    let who = people(&s, &ids).await?;
    let by = |uid: Option<Uuid>| -> Value {
        let Some(uid) = uid else { return Value::Null };
        let key = uid.to_string();
        who.iter().find(|p| p.get("id").and_then(Value::as_str) == Some(key.as_str())).cloned().unwrap_or(Value::Null)
    };
    let reviewers: Vec<Value> = a.reviewers.iter().map(|r| by(Some(*r))).filter(|v| !v.is_null()).collect();
    let current = approval_current(&s, Some(&row)).await?;
    let can = can_review(&s, u.id, &a.reviewers, ws).await?;
    Ok(Reply::json(json!({
        "required": required, "suggestedReviewers": admins,
        "approval": {
            "id": a.id.to_string(), "status": a.status, "message": a.message, "createdAt": iso(Some(a.created_at)), "decidedAt": iso(a.decided_at), "note": a.decision_note,
            "requestedBy": by(a.requested_by), "decidedBy": by(a.decided_by), "reviewers": reviewers,
            "current": current, "canReview": can,
        },
    })))
}

async fn design_title(s: &AppState, id: &str) -> Result<String, sqlx::Error> {
    let t: Option<String> = sqlx::query_scalar("SELECT title FROM designs WHERE id = $1").bind(id).fetch_optional(&s.pool).await?;
    Ok(t.unwrap_or_else(|| "a design".into()))
}

async fn add_comment(s: &AppState, design_id: &str, author: Uuid, body: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO comments(id, design_id, author_id, body) VALUES ($1,$2,$3,$4)").bind(oc_db::uuid()).bind(design_id).bind(author).bind(body).execute(&s.pool).await.map(|_| ())
}

async fn request_approval(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("message", z::str().max(2000.0).default(json!(""))), ("reviewerIds", z::array(z::str().uuid()).max(20.0).optional())]).parse(Some(&req.body_or_empty()))?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::Edit) {
        return Ok(forbidden());
    }
    let message = b["message"].as_str().unwrap_or("").to_owned();
    let ws = governing_workspace(&s, &id).await?;
    let given: Vec<String> = b.get("reviewerIds").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
    let mut reviewers = if !given.is_empty() {
        given
    } else if let Some(w) = ws {
        let r: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM memberships WHERE workspace_id = $1 AND role IN ('owner','admin')").bind(w).fetch_all(&s.pool).await?;
        r.into_iter().map(|u| u.to_string()).collect()
    } else {
        vec![]
    };
    let me = u.id.to_string();
    reviewers.retain(|r| *r != me);
    // Reviewers must be able to open the design (or be admins of the governing team).
    let mut ok: Vec<Uuid> = Vec::new();
    for r in &reviewers {
        let rid = Uuid::parse_str(r).map_err(ApiError::internal)?;
        if at_least(design_role(&s, Some(rid), &id).await?, Role::View) || is_admin_role(role_in(&s, rid, ws).await?.as_deref()) {
            ok.push(rid);
        }
    }
    if ok.is_empty() {
        return Ok(reply::json(400, json!({ "error": "no_reviewers", "message": "Choose at least one reviewer who can open this design." })));
    }
    let aid = oc_db::uuid();
    let mut t = s.pool.begin().await?;
    sqlx::query("UPDATE design_approvals SET status = 'cancelled', updated_at = now() WHERE design_id = $1 AND status = 'pending'").bind(&id).execute(&mut *t).await?;
    sqlx::query("INSERT INTO design_approvals(id, design_id, workspace_id, requested_by, reviewers, message) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(aid)
        .bind(&id)
        .bind(ws)
        .bind(u.id)
        .bind(&ok)
        .bind(&message)
        .execute(&mut *t)
        .await?;
    t.commit().await?;
    // Reuse comments: the request is posted on the design so the conversation stays in one place.
    let suffix = if message.is_empty() { String::new() } else { format!(": {message}") };
    add_comment(&s, &id, u.id, &format!("Sent for approval{suffix}")).await?;
    let title = design_title(&s, &id).await?;
    let url = format!("{}/design/{id}?approval=1", s.cfg.web_origin);
    for p in people(&s, &ok).await? {
        let pid = p.get("id").and_then(Value::as_str).and_then(|x| Uuid::parse_str(x).ok()).unwrap_or_default();
        brand::notify(&s, pid, "approval", &id, u.id, &format!("{} asked you to approve “{title}”{suffix}", u.name)).await?;
        let email = p.get("email").and_then(Value::as_str).unwrap_or("");
        oc_mail::send_email(
            &s.pool,
            email,
            &oc_mail::Template::ApprovalRequest { actor: u.name.clone(), title: title.clone(), message: crate::js::take(&message, 500), url: url.clone() },
            Some(pid),
        )
        .await?;
    }
    audit(&s, &req, u.id, ws, "design.approval_requested", Some(id.clone()), Some(json!({ "reviewers": ok.len() }))).await?;
    Ok(Reply::json(json!({ "id": aid.to_string() })))
}

async fn decide(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("decision", z::enumeration(&["approve", "changes"])), ("note", z::str().max(2000.0).default(json!("")))]).parse(req.body())?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    let Some((a, _)) = latest(&s, &id).await?.filter(|(a, _)| a.status == "pending") else {
        return Ok(reply::json(409, json!({ "error": "no_request", "message": "There is no pending approval request for this design." })));
    };
    let ws = governing_workspace(&s, &id).await?;
    if !can_review(&s, u.id, &a.reviewers, ws).await? {
        return Ok(reply::json(403, json!({ "error": "forbidden", "message": "You are not a reviewer of this design." })));
    }
    let approved = b["decision"].as_str() == Some("approve");
    let note = b["note"].as_str().unwrap_or("").to_owned();
    let snap = if approved { brandhub::doc_snapshot(&s, &id).await? } else { None };
    sqlx::query(
        "UPDATE design_approvals SET status = $2, decided_by = $3, decision_note = $4, decided_at = now(), approved_snapshot = $5,
         approved_doc_at = (SELECT updated_at FROM doc_state WHERE design_id = $6), updated_at = now() WHERE id = $1",
    )
    .bind(a.id)
    .bind(if approved { "approved" } else { "changes_requested" })
    .bind(u.id)
    .bind(if note.is_empty() { None } else { Some(note.as_str()) })
    .bind(snap)
    .bind(&id)
    .execute(&s.pool)
    .await?;
    let suffix = if note.is_empty() { String::new() } else { format!(": {note}") };
    add_comment(&s, &id, u.id, &format!("{}{suffix}", if approved { "Approved" } else { "Requested changes" })).await?;
    let title = design_title(&s, &id).await?;
    if let Some(rb) = a.requested_by {
        let verb = if approved { "approved" } else { "requested changes on" };
        brand::notify(&s, rb, "approval", &id, u.id, &format!("{} {verb} “{title}”{suffix}", u.name)).await?;
        let email: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = $1").bind(rb).fetch_optional(&s.pool).await?;
        if let Some(email) = email
            && rb != u.id
        {
            oc_mail::send_email(
                &s.pool,
                &email,
                &oc_mail::Template::ApprovalDecision { actor: u.name.clone(), title, approved, note: crate::js::take(&note, 500), url: format!("{}/design/{id}", s.cfg.web_origin) },
                Some(rb),
            )
            .await?;
        }
    }
    audit(&s, &req, u.id, ws, if approved { "design.approved" } else { "design.changes_requested" }, Some(id.clone()), None).await?;
    Ok(Reply::json(json!({ "ok": true, "status": if approved { "approved" } else { "changes_requested" } })))
}

async fn cancel_approval(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some((a, _)) = latest(&s, &id).await?.filter(|(a, _)| a.status == "pending") else { return Ok(not_found()) };
    if a.requested_by != Some(u.id) && design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    sqlx::query("UPDATE design_approvals SET status = 'cancelled', updated_at = now() WHERE id = $1").bind(a.id).execute(&s.pool).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

/// Review queue: pending requests where I'm a reviewer.
async fn queue(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let rows = pg::json_rows(
        &s.pool,
        r#"SELECT a.id, a.design_id AS "designId", d.title, a.message, a.created_at AS "createdAt", u.name AS "requestedBy" FROM design_approvals a
       JOIN designs d ON d.id = a.design_id LEFT JOIN users u ON u.id = a.requested_by
       WHERE a.status = 'pending' AND $1 = ANY(a.reviewers) AND d.trashed_at IS NULL ORDER BY a.created_at DESC LIMIT 100"#,
        &[pg::uuid(u.id)],
    )
    .await?;
    Ok(Reply::json(json!({ "approvals": rows })))
}

/// Lets clients ask before offering download/share/publish.
async fn gate(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    Ok(Reply::json(match approval_gate(&s, u.id, &id).await? {
        Some(Value::Object(g)) => {
            let mut m = Map::new();
            m.insert("allowed".into(), json!(false));
            m.extend(g);
            Value::Object(m)
        }
        _ => json!({ "allowed": true }),
    }))
}
