//! Port of services/api/src/routes/brandhub.ts — Brand Hub (A8): multiple brand kits per workspace, brand
//! controls (team settings), brand-template info for the editor. Every route runs `requireUser` first (the plugin's `preHandler`).
//!
//! Also exported for other modules: [`KIT_COLS`], [`default_kit`], [`save_kit`], [`accessible_kits`].

use crate::auth::{at_least, design_role, personal_workspace};
use crate::brandhub::{brand_controls, governing_workspace, is_admin_role, role_in, CONTROL_KEYS};
use crate::prelude::*;
use crate::routes::{designs, teams};
use oc_auth::Role;

/// Routes with their own `config.rateLimit`.
pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/brand-kits", get(list_kits).post(create_kit))
        .route("/api/brand-kits/{id}", get(read_kit).patch(patch_kit).delete(delete_kit))
        .route("/api/brand-kits/{id}/default", post(default_kit_route))
        .route("/api/brand-kits/{id}/guidelines", get(get_guidelines).put(put_guidelines))
        .route("/api/workspaces/{id}/brand-controls", get(get_controls).put(put_controls))
        .route("/api/designs/{id}/brand", get(design_brand))
        .route("/api/templates/{id}/republish", post(republish))
}

// ---------------------------------------------------------------- shared helpers

pub const KIT_COLS: &str = r#"k.id, k.workspace_id AS "workspaceId", k.name, k.is_default AS "isDefault", k.colors, k.palettes, k.fonts, k.logos, k.photos, k.graphics, k.voice, k.text_styles AS "textStyles", k.updated_at AS "updatedAt""#;

/// `defaultKit(workspaceId)`: the workspace's default kit as a row (`KIT_COLS`).
pub async fn default_kit(state: &AppState, workspace_id: uuid::Uuid) -> Result<Option<serde_json::Value>, sqlx::Error> {
    crate::pg::json_one(&state.pool, &format!("SELECT {KIT_COLS} FROM brand_kits k WHERE k.workspace_id = $1 ORDER BY k.is_default DESC, k.created_at LIMIT 1"), &[crate::pg::uuid(workspace_id)]).await
}

/// `saveKit(workspaceId, userId, kit, id?)`: insert or update; returns the kit id. `kit` is the merged JS object.
pub async fn save_kit(state: &AppState, workspace_id: uuid::Uuid, user_id: uuid::Uuid, kit: &serde_json::Value, id: Option<&str>) -> Result<String, sqlx::Error> {
    let kid = id.map_or_else(|| oc_db::uuid().to_string(), str::to_owned);
    let js = |k: &str| crate::http::jsjson::stringify(kit.get(k).unwrap_or(&serde_json::Value::Null));
    sqlx::query(
        "INSERT INTO brand_kits(id, workspace_id, name, is_default, colors, palettes, fonts, logos, photos, graphics, voice, created_by, text_styles, updated_at)
     VALUES ($1::uuid,$2,$3, NOT EXISTS (SELECT 1 FROM brand_kits WHERE workspace_id = $2 AND is_default), $4::jsonb,$5::jsonb,$6::jsonb,$7::jsonb,$8::jsonb,$9::jsonb,$10,$11, COALESCE($12::jsonb, '[]'), now())
     ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, colors = EXCLUDED.colors, palettes = EXCLUDED.palettes, fonts = EXCLUDED.fonts, logos = EXCLUDED.logos,
       photos = EXCLUDED.photos, graphics = EXCLUDED.graphics, voice = EXCLUDED.voice, text_styles = COALESCE($12::jsonb, brand_kits.text_styles), updated_at = now()",
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
    .bind(kit.get("textStyles").filter(|v| v.is_array()).map(crate::http::jsjson::stringify))
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

pub fn hex() -> z::Schema {
    z::str().regex("^#[0-9a-fA-F]{6}$", "^#[0-9a-fA-F]{6}$")
}
pub fn asset_id() -> z::Schema {
    z::str().regex("^[0-9A-Za-z]{16}$", "^[0-9A-Za-z]{16}$")
}
pub fn color() -> z::Schema {
    z::obj([("name", z::str().max(40.0)), ("hex", hex())])
}
pub fn font() -> z::Schema {
    z::str().max(80.0).optional()
}

/// Kit font roles, which text styles belong to.
pub const TEXT_ROLES: [&str; 5] = ["heading", "subheading", "body", "quote", "caption"];
/// Caps on a kit's text styles.
pub const MAX_TEXT_STYLES: usize = 30;

/// A text style: how one kind of text looks beyond its font (the font comes from `fonts[role]`). Letter spacing is in
/// thousandths of an em and line height a multiple of the size, as in the editor; `size` (px) is a reference for a
/// typical page, not a rule; `colors` are the colours the text may take (any, when empty).
pub fn text_style() -> z::Schema {
    z::obj([
        ("name", z::str().trim().min(1.0).max(40.0)),
        ("role", z::enumeration(&TEXT_ROLES)),
        ("weight", z::num().int().min(100.0).max(900.0).optional()),
        ("italic", z::bool().optional()),
        ("uppercase", z::bool().optional()),
        ("letterSpacing", z::num().min(-500.0).max(2000.0).optional()),
        ("lineHeight", z::num().min(0.5).max(4.0).optional()),
        ("size", z::num().min(4.0).max(1000.0).optional()),
        ("colors", z::array(hex()).max(12.0).optional()),
        ("note", z::str().max(300.0).optional()),
    ])
}

/// `KitBody` (`partial`: `KitBody.partial()`, whose defaults still apply in zod 4) plus `workspaceId` when
/// `extend` (the create route).
pub fn kit_body(partial: bool, extend: bool) -> z::Schema {
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
        ("textStyles", z::array(text_style()).max(MAX_TEXT_STYLES as f64).default(json!([]))),
    ];
    if extend {
        let [a, b, c, d, e, f, g, h, i] = fields;
        z::obj([a, b, c, d, e, f, g, h, i, ("workspaceId", z::str().uuid().optional())])
    } else {
        z::obj(fields)
    }
}

/// `kitRole(userId, workspaceId)`: the personal owner and team owners, admins and designers edit; members view.
pub async fn kit_role(s: &AppState, user_id: Uuid, workspace_id: Uuid) -> Result<Option<&'static str>, sqlx::Error> {
    let r: Option<(String, bool)> = sqlx::query_as("SELECT m.role, w.personal FROM memberships m JOIN workspaces w ON w.id = m.workspace_id WHERE m.workspace_id = $1 AND m.user_id = $2")
        .bind(workspace_id)
        .bind(user_id)
        .fetch_optional(&s.pool)
        .await?;
    Ok(r.map(|(role, personal)| if personal || role != "member" { "edit" } else { "view" }))
}

/// Fair-use cap on brand kits per workspace.
pub const MAX_BRAND_KITS: i32 = 1000;

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
        .map(|(id, name, personal, role, mut controls)| {
            let can = personal || role != "member";
            if let Value::Object(m) = &mut controls.0 {
                m.retain(|k, _| CONTROL_KEYS.contains(&k.as_str()));
            }
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
    if n >= MAX_BRAND_KITS {
        return Ok(reply::json(402, json!({ "error": "upgrade_required", "feature": "brandKits", "message": format!("You can have up to {MAX_BRAND_KITS} brand kits.") })));
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
    let cur = pg::json_one(&s.pool, "SELECT name, colors, palettes, fonts, logos, photos, graphics, voice, text_styles AS \"textStyles\" FROM brand_kits WHERE id = $1::uuid", &[pg::text(&id)]).await?;
    let mut next = cur.and_then(|c| c.as_object().cloned()).unwrap_or_default();
    // Only the fields actually sent: the schema's defaults would otherwise empty the rest of the kit.
    let sent = req.body().and_then(Value::as_object);
    // What changed, old and new, for the audit log: kits hold a team's brand, so edits are traced like their creation.
    let mut changed = serde_json::Map::new();
    if let Value::Object(p) = patch {
        for (key, v) in p {
            if sent.is_some_and(|b| b.contains_key(&key)) {
                let old = next.insert(key.clone(), v.clone()).unwrap_or(Value::Null);
                if old != v {
                    changed.insert(key, json!({ "from": old, "to": v }));
                }
            }
        }
    }
    save_kit(&s, k.workspace_id, u.id, &Value::Object(next), Some(&id)).await?;
    if !changed.is_empty() {
        audit(&s, &req, u.id, Some(k.workspace_id), "brand.kit_updated", Some(id.clone()), Some(json!({ "changed": changed }))).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- guidelines

/// Caps on a kit's written guidelines: enough for a full brand book, small enough to hand to an assistant in parts.
pub const MAX_GUIDELINE_SECTIONS: usize = 60;
const MAX_GUIDELINE_TITLE: f64 = 120.0;
const MAX_GUIDELINE_BODY: f64 = 60_000.0;
pub const MAX_GUIDELINES_TOTAL: usize = 300_000;

/// A kit's guideline sections, `[{ title, body }]`, as the guidelines route and the MCP brand kit tools take them.
pub fn guideline_sections() -> z::Schema {
    z::array(z::obj([("title", z::str().min(1.0).max(MAX_GUIDELINE_TITLE)), ("body", z::str().max(MAX_GUIDELINE_BODY))])).max(MAX_GUIDELINE_SECTIONS as f64)
}

fn guidelines_body() -> z::Schema {
    z::obj([("sections", guideline_sections())])
}

/// What a guidelines write changed, by outline (titles and sizes): the audit log records this, not the text.
pub struct GuidelinesWrite {
    pub before: Vec<Value>,
    pub after: Vec<Value>,
    /// Whether any text changed, not just the outline.
    pub changed: bool,
}

fn outline(sections: &[Value]) -> Vec<Value> {
    sections.iter().map(|x| json!({ "title": x["title"], "chars": x["body"].as_str().map_or(0, |b| b.chars().count()) })).collect()
}

/// Replace a kit's guidelines with already validated sections (`guideline_sections`); `Err` is the message when
/// they're over the total size cap. The caller checks the user may edit the kit.
pub async fn write_guidelines(s: &AppState, kit_id: &str, sections: &[Value]) -> Result<Result<GuidelinesWrite, String>, sqlx::Error> {
    let sections: Vec<Value> = sections.iter().map(|x| json!({ "title": x["title"].as_str().unwrap_or("").trim(), "body": x["body"].as_str().unwrap_or("") })).collect();
    let total: usize = sections.iter().map(|x| x["body"].as_str().map_or(0, |b| b.chars().count())).sum();
    if total > MAX_GUIDELINES_TOTAL {
        return Ok(Err(format!("Guidelines can hold up to {MAX_GUIDELINES_TOTAL} characters in all; these have {total}.")));
    }
    let old = kit_guidelines(s, kit_id).await?;
    pg::exec(&s.pool, "UPDATE brand_kits SET guidelines = $2::jsonb, updated_at = now() WHERE id = $1::uuid", &[pg::text(kit_id), pg::text(serde_json::to_string(&sections).unwrap_or_else(|_| "[]".into()))]).await?;
    Ok(Ok(GuidelinesWrite { before: outline(&old), after: outline(&sections), changed: old != sections }))
}

/// A kit's guideline sections, `[{ title, body }]` (empty when it has none or doesn't exist).
pub async fn kit_guidelines(s: &AppState, kit_id: &str) -> Result<Vec<Value>, sqlx::Error> {
    let g: Option<sqlx::types::Json<Value>> = sqlx::query_scalar("SELECT guidelines FROM brand_kits WHERE id = $1::uuid").bind(kit_id).fetch_optional(&s.pool).await?;
    Ok(g.and_then(|g| g.0.as_array().cloned()).unwrap_or_default())
}

/// Each kit's guideline outline, `kit id → [{ title, chars }]`, for kits that have guidelines.
pub async fn guideline_outlines(s: &AppState, kit_ids: &[String]) -> Result<std::collections::HashMap<String, Vec<Value>>, sqlx::Error> {
    let rows: Vec<(String, sqlx::types::Json<Value>)> = sqlx::query_as(
        "SELECT id::text, COALESCE(jsonb_agg(jsonb_build_object('title', g->>'title', 'chars', char_length(g->>'body')) ORDER BY n), '[]')
         FROM brand_kits, jsonb_array_elements(guidelines) WITH ORDINALITY AS e(g, n) WHERE id::text = ANY($1) GROUP BY id",
    )
    .bind(kit_ids)
    .fetch_all(&s.pool)
    .await?;
    Ok(rows.into_iter().filter_map(|(id, o)| o.0.as_array().cloned().map(|a| (id, a))).collect())
}

async fn get_guidelines(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(k) = own_kit(&s, u.id, &id).await?.filter(|k| k.role.is_some()) else { return Ok(not_found()) };
    Ok(Reply::json(json!({ "sections": kit_guidelines(&s, &id).await?, "canEdit": k.role == Some("edit") })))
}

async fn put_guidelines(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = guidelines_body().parse(req.body())?;
    let Some(k) = own_kit(&s, u.id, &id).await?.filter(|k| k.role.is_some()) else { return Ok(not_found()) };
    if k.role != Some("edit") {
        return Ok(forbidden());
    }
    let sections = b["sections"].as_array().cloned().unwrap_or_default();
    let w = match write_guidelines(&s, &id, &sections).await? {
        Ok(w) => w,
        Err(message) => return Ok(reply::json(400, json!({ "error": "too_long", "message": message }))),
    };
    // The audit log records the outline (titles and sizes), not the text: a brand book would swamp it.
    if w.changed {
        audit(&s, &req, u.id, Some(k.workspace_id), "brand.kit_guidelines_updated", Some(id.clone()), Some(json!({ "from": w.before, "to": w.after }))).await?;
    }
    Ok(Reply::json(json!({ "ok": true, "sections": w.after })))
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
    Ok(Reply::json(json!({
        "controls": brand_controls(&s, Some(ws)).await?,
        "canEdit": is_admin_role(Some(&role)),
        "role": role,
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
    let role = role_in(&s, u.id, ws).await?;
    let controls = brand_controls(&s, ws).await?;
    let template = match d {
        Some((Some(tid), tv, brand, version, title)) => json!({ "id": tid, "title": title, "brand": brand == Some(true), "version": tv.unwrap_or(1), "latest": version.unwrap_or(1) }),
        _ => Value::Null,
    };
    let published = pg::json_rows(&s.pool, "SELECT id, title, version, brand FROM templates WHERE source_design_id = $1 ORDER BY created_at", &[pg::text(&id)]).await?;
    Ok(Reply::json(json!({
        "workspaceId": opt_uuid(ws), "role": role, "controls": controls, "template": template, "published": published,
    })))
}

// ---------------------------------------------------------------- brand templates: republish

/// Re-snapshot the template from its source design (admins/designers); bumps the version.
async fn republish(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let t: Option<(Option<Uuid>, Option<String>)> = sqlx::query_as("SELECT workspace_id, source_design_id FROM templates WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some((Some(ws), Some(source))) = t else { return Ok(not_found()) };
    let role = role_in(&s, u.id, Some(ws)).await?;
    if role.as_deref().is_none_or(|r| r == "member") {
        return Ok(forbidden());
    }
    let Some(doc) = designs::load_doc(&s, &source).await? else { return Ok(reply::json(400, json!({ "error": "empty" }))) };
    let dims = doc.pointer("/meta/dims");
    let dim = |k: &str| dims.and_then(|d| d.get(k)).and_then(Value::as_f64);
    sqlx::query("UPDATE templates SET doc = $2::jsonb, width = $3, height = $4, version = version + 1, updated_at = now() WHERE id = $1")
        .bind(&id)
        .bind(crate::http::jsjson::stringify(&doc))
        .bind(dim("w"))
        .bind(dim("h"))
        .execute(&s.pool)
        .await?;
    let version: i32 = sqlx::query_scalar("SELECT version FROM templates WHERE id = $1").bind(&id).fetch_one(&s.pool).await?;
    Ok(Reply::json(json!({ "ok": true, "version": version })))
}
