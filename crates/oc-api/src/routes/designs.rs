//! Port of services/api/src/routes/designs.ts — designs: list/create/read/update/trash, copy, collab tickets,
//! snapshots, thumbnails, sharing, versions, export (via the render service) and page-count sync.
//! Every route runs `requireUser` first (the plugin's `preHandler`).
//!
//! Also exported for other modules: [`create_design_from_doc`], [`load_doc`], [`request_thumbnail`].

use oc_auth::{Audience, Role, TicketClaims};
use serde_json::Map;

use crate::analytics;
use crate::auth::{at_least, design_role, personal_workspace};
use crate::brandhub;
use crate::prelude::*;
use crate::projects::{self, Failure, folder_access};
use crate::routes::{brand, docs, print_profiles, teams};
use crate::share_access::shared_with_sql;

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/designs", get(list).post(create))
        .route("/api/designs/bulk", post(bulk))
        .route("/api/designs/{id}", get(read).patch(update).delete(remove))
        .route("/api/designs/{id}/restore", post(restore))
        .route("/api/designs/{id}/copy", post(copy))
        .route("/api/designs/{id}/ticket", get(ticket))
        .route("/api/designs/{id}/doc", get(doc))
        .route("/api/designs/{id}/thumbnail.png", get(thumbnail))
        .route("/api/designs/{id}/shares", get(shares).post(share))
        .route("/api/designs/{id}/shares/{user_id}", delete(unshare))
        .route("/api/designs/{id}/versions", get(versions).post(save_version))
        .route("/api/designs/{id}/versions/{vid}", get(version))
        .route("/api/designs/{id}/export", post(export))
        .route("/api/designs/{id}/touch", post(touch))
}

const LIST_SQL: &str = "
  SELECT d.id, d.title, d.surface, d.width, d.height, d.unit, d.page_count, d.link_access, d.owner_id, u.name AS owner_name,
         d.folder_id, d.workspace_id, d.thumbnail, d.trashed_at, d.created_at, d.updated_at, d.public_view, (s.user_id IS NOT NULL) AS starred,
         (d.taken_down_at IS NOT NULL) AS held
  FROM designs d
  JOIN users u ON u.id = d.owner_id
  LEFT JOIN stars s ON s.design_id = d.id AND s.user_id = $1";

#[derive(Default)]
pub struct CreateOpts {
    pub template_id: Option<String>,
    pub folder_id: Option<Uuid>,
    pub workspace_id: Option<Uuid>,
    pub brand_workspace_id: Option<Uuid>,
    pub template_version: Option<i32>,
}

/// The persisted Yjs state of a new design. Sheets are no longer supported, so nothing can start or copy one.
fn initial_state(doc: &Value) -> Result<Vec<u8>, ApiError> {
    if doc.pointer("/meta/surface").and_then(Value::as_str) == Some("sheet") {
        return Err(reply::json(422, unsupported_surface()).into());
    }
    Ok(oc_crdt::encode_doc_value(doc))
}

/// Surfaces stored designs may still carry, but that no new design starts on.
pub fn is_retired_surface(doc: &Value) -> bool {
    matches!(doc.pointer("/meta/surface").and_then(Value::as_str), Some("sheet" | "whiteboard" | "email"))
}

/// The refusal of a create on a retired surface.
pub fn unsupported_surface() -> Value {
    json!({ "error": "unsupported_surface", "message": "Sheets, whiteboards and email designs can no longer be created." })
}

/// `createDesignFromDoc(ownerId, doc, opts)`.
pub async fn create_design_from_doc(s: &AppState, owner_id: Uuid, doc: &Value, opts: CreateOpts) -> Result<String, ApiError> {
    // SEC-01: stored designs never carry javascript:/data: links (imports, copies, templates, API and MCP creates).
    let safe = oc_schema::links::with_safe_links_value(doc);
    let doc: &Value = &safe;
    let id = oc_schema::ids::new_id();
    let ws = match opts.workspace_id {
        Some(w) => w,
        None => personal_workspace(s, owner_id).await?,
    };
    let state = initial_state(doc)?;
    let meta = &doc["meta"];
    let num = |v: &Value| v.as_f64();
    let page_count = doc.get("pages").and_then(Value::as_object).map_or(0, Map::len) as i32;
    let mut t = s.pool.begin().await?;
    sqlx::query(
        "INSERT INTO designs(id, workspace_id, owner_id, title, surface, width, height, unit, page_count, template_id, folder_id, brand_workspace_id, template_version)
       VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
    )
    .bind(&id)
    .bind(ws)
    .bind(owner_id)
    .bind(meta.get("title").and_then(Value::as_str))
    .bind(meta.get("surface").and_then(Value::as_str))
    .bind(num(&meta["dims"]["w"]))
    .bind(num(&meta["dims"]["h"]))
    .bind(meta["dims"].get("unit").and_then(Value::as_str))
    .bind(page_count)
    .bind(opts.template_id.as_deref())
    .bind(opts.folder_id)
    .bind(opts.brand_workspace_id)
    .bind(opts.template_version)
    .execute(&mut *t)
    .await?;
    sqlx::query("INSERT INTO doc_state(design_id, state) VALUES ($1,$2)").bind(&id).bind(&state).execute(&mut *t).await?;
    t.commit().await?;
    let (s2, id2) = (s.clone(), id.clone());
    s.spawn(async move { request_thumbnail(&s2, &id2).await });
    Ok(id)
}

/// `loadDoc(designId)`: the persisted snapshot as JSON.
pub async fn load_doc(s: &AppState, design_id: &str) -> Result<Option<Value>, ApiError> {
    let r: Option<Vec<u8>> = sqlx::query_scalar("SELECT state FROM doc_state WHERE design_id = $1").bind(design_id).fetch_optional(&s.pool).await?;
    Ok(match r {
        Some(st) => Some(oc_crdt::decode_doc_value(&st)?),
        None => None,
    })
}

/// `requestThumbnail(designId)`: ask the render service; a render service that is down is ignored.
pub async fn request_thumbnail(s: &AppState, design_id: &str) {
    let _ = s
        .http
        .post(format!("{}/internal/thumbnail", s.cfg.render_url))
        .header("content-type", "application/json")
        .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&s.cfg.secret))
        .body(crate::http::jsjson::stringify(&json!({ "designId": design_id })))
        .send()
        .await;
}

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

fn forbidden() -> Reply {
    reply::json(403, json!({ "error": "forbidden" }))
}

fn uid_str(u: &User) -> String {
    u.id.to_string()
}

async fn list(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let q = z::obj([
        ("filter", z::enumeration(&["recent", "starred", "shared", "trash", "folder", "all", "team"]).default(json!("recent"))),
        ("workspace", z::str().uuid().optional()),
        ("folder", z::str().uuid().optional()),
        ("q", z::str().optional()),
        ("surface", z::str().optional()),
        // Projects hub (g17/g20): sort order, whose designs, and (`deep`) a folder's whole subtree for search within Projects.
        ("sort", z::enumeration(projects::DESIGN_SORTS).default(json!("edited"))),
        ("owner", z::enumeration(&["anyone", "me", "shared"]).default(json!("anyone"))),
        ("deep", z::enumeration(&["0", "1"]).optional()),
    ])
    .parse(Some(&req.query))?;
    let qs = |k: &str| q.get(k).and_then(Value::as_str).map(str::to_owned);
    let owner = qs("owner").unwrap_or_default();
    let deep = qs("deep").as_deref() == Some("1");
    let shared = shared_with_sql("d.id", "$1");
    let mut params: Vec<Arg> = vec![pg::uuid(u.id)];
    let mut wh: Vec<String> = Vec::new();
    match q["filter"].as_str().unwrap_or("recent") {
        "trash" => wh.push("d.owner_id = $1 AND d.trashed_at IS NOT NULL".into()),
        "shared" => wh.push(format!("d.trashed_at IS NULL AND d.owner_id <> $1 AND {shared}")),
        "starred" => wh.push("d.trashed_at IS NULL AND s.user_id IS NOT NULL".into()),
        "folder" => match qs("folder").filter(|f| !f.is_empty()) {
            Some(f) => {
                // A folder lists everything in it the user can open: a team folder holds the team's designs (CO-042).
                let fid = Uuid::parse_str(&f).map_err(ApiError::internal)?;
                if folder_access(&s, u.id, fid).await?.is_none() {
                    return Ok(not_found());
                }
                params.push(pg::text(f));
                let n = params.len();
                let inside = if deep { format!("IN {}", projects::subtree_sql(&format!("${n}"))) } else { format!("= ${n}::uuid") };
                wh.push(format!("d.trashed_at IS NULL AND d.folder_id {inside} AND (d.owner_id = $1 OR {})", projects::IN_MY_TEAM));
            }
            None if deep => {
                // All of Projects: my designs wherever they are, plus the designs filed in my teams' folders. The
                // "Shared with me" owner filter also brings in designs shared with me directly or through a group.
                let extra = if owner == "shared" { format!(" OR {shared}") } else { String::new() };
                wh.push(format!("d.trashed_at IS NULL AND (d.owner_id = $1 OR (d.folder_id IS NOT NULL AND {}){extra})", projects::IN_MY_TEAM));
            }
            None => wh.push("d.owner_id = $1 AND d.trashed_at IS NULL AND d.folder_id IS NULL".into()),
        },
        "team" => {
            params.push(Arg::Text(qs("workspace")));
            wh.push(format!(
                "d.trashed_at IS NULL AND d.workspace_id = ${}::uuid AND EXISTS (SELECT 1 FROM memberships mm WHERE mm.workspace_id = d.workspace_id AND mm.user_id = $1)",
                params.len()
            ));
        }
        _ => wh.push(format!(
            "d.trashed_at IS NULL AND (d.owner_id = $1 OR {shared}
          OR EXISTS (SELECT 1 FROM memberships mm JOIN workspaces ww ON ww.id = mm.workspace_id WHERE mm.workspace_id = d.workspace_id AND mm.user_id = $1 AND NOT ww.personal))"
        )),
    }
    if let Some(text) = qs("q").filter(|t| !t.is_empty()) {
        params.push(pg::text(text));
        let n = params.len();
        wh.push(format!("(to_tsvector('simple', d.title) @@ plainto_tsquery('simple', ${n}) OR d.title ILIKE '%' || ${n} || '%')"));
    }
    if let Some(surface) = qs("surface").filter(|t| !t.is_empty()) {
        params.push(pg::text(surface));
        wh.push(format!("d.surface = ${}", params.len()));
    }
    if owner == "me" {
        wh.push("d.owner_id = $1".into());
    }
    if owner == "shared" {
        wh.push("d.owner_id <> $1".into());
    }
    let sql = format!("{LIST_SQL} WHERE {} ORDER BY {} LIMIT 200", wh.join(" AND "), projects::design_order(q["sort"].as_str().unwrap_or("edited")));
    let rows = pg::json_rows(&s.pool, &sql, &params).await?;
    Ok(Reply::json(json!({ "designs": rows })))
}

// ---------------------------------------------------------------- single-design rules shared with the bulk route

/// `setStar(userId, id, on, role)`: anyone who can open the design.
async fn set_star(s: &AppState, user_id: Uuid, id: &str, on: bool, role: Option<Role>) -> Result<Option<Failure>, ApiError> {
    let role = match role {
        Some(r) => Some(r),
        None => design_role(s, Some(user_id), id).await?,
    };
    if role.is_none() {
        return Ok(Some(Failure::new(404, "not_found")));
    }
    if on {
        pg::exec(&s.pool, "INSERT INTO stars(user_id, design_id) VALUES ($1,$2) ON CONFLICT DO NOTHING", &[pg::uuid(user_id), pg::text(id)]).await?;
    } else {
        pg::exec(&s.pool, "DELETE FROM stars WHERE user_id = $1 AND design_id = $2", &[pg::uuid(user_id), pg::text(id)]).await?;
    }
    Ok(None)
}

/// `moveDesign(userId, id, folderId, role)`: files a design in a folder, or takes it out of any folder (owner only).
async fn move_design(s: &AppState, user_id: Uuid, id: &str, folder_id: Option<Uuid>, role: Option<Role>) -> Result<Option<Failure>, ApiError> {
    let r = match role {
        Some(r) => Some(r),
        None => design_role(s, Some(user_id), id).await?,
    };
    let Some(r) = r else { return Ok(Some(Failure::new(404, "not_found"))) };
    let Some(fid) = folder_id else {
        if r != Role::Owner {
            return Ok(Some(Failure::new(403, "forbidden")));
        }
        pg::exec(&s.pool, "UPDATE designs SET folder_id = NULL WHERE id = $1", &[pg::text(id)]).await?;
        return Ok(None);
    };
    let Some(f) = folder_access(s, user_id, fid).await? else { return Ok(Some(Failure::new(404, "folder_not_found"))) };
    if !f.file || !at_least(Some(r), Role::Edit) {
        return Ok(Some(Failure::new(403, "forbidden")));
    }
    // Only the design's owner files it in a personal folder or hands it to another team.
    let d: Option<(Uuid, Uuid)> = sqlx::query_as("SELECT owner_id, workspace_id FROM designs WHERE id = $1").bind(id).fetch_optional(&s.pool).await?;
    match d {
        Some((owner, ws)) if owner == user_id || (!f.personal && f.workspace_id == ws) => {}
        _ => return Ok(Some(Failure::new(403, "forbidden"))),
    }
    // Moving into a team folder makes the design part of that team (CO-042).
    let mut t = s.pool.begin().await?;
    sqlx::query("UPDATE designs SET folder_id = $2, workspace_id = CASE WHEN $3::boolean THEN workspace_id ELSE $4::uuid END WHERE id = $1")
        .bind(id)
        .bind(fid)
        .bind(f.personal)
        .bind(f.workspace_id)
        .execute(&mut *t)
        .await?;
    if !f.personal {
        // Its Content Planner posts go with it, so the team sees them. An approval only carries over when the
        // person moving the design could have given it in that team; otherwise the post waits for review there.
        let approver = matches!(f.role.as_deref(), Some("owner" | "admin"));
        let moved = sqlx::query(
            "UPDATE social_posts SET workspace_id = $2, updated_at = now(),
               status = CASE WHEN NOT $3 AND approved_at IS NOT NULL AND status IN ('approved', 'scheduled') THEN 'in_review' ELSE status END,
               approved_by = CASE WHEN $3 THEN approved_by END, approved_at = CASE WHEN $3 THEN approved_at END,
               approved_version = CASE WHEN $3 THEN approved_version END,
               claim_token = CASE WHEN $3 THEN claim_token END, claimed_at = CASE WHEN $3 THEN claimed_at END
             WHERE design_id = $1 AND workspace_id IS DISTINCT FROM $2",
        )
        .bind(id)
        .bind(f.workspace_id)
        .bind(approver)
        .execute(&mut *t)
        .await;
        match moved {
            Ok(_) => {}
            // Post IDs are unique per team (social_posts_ref): the design stays where it was.
            Err(e) if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") => {
                return Ok(Some(Failure::msg(409, "duplicate_ref", "A post in that team already uses the same post ID as one of this design's posts.")));
            }
            Err(e) => return Err(e.into()),
        }
    }
    t.commit().await?;
    Ok(None)
}

/// `trashDesign(userId, id, permanent)`: a design taken down by moderation stays on hold in the trash.
async fn trash_design(s: &AppState, user_id: Uuid, id: &str, permanent: bool) -> Result<Option<Failure>, ApiError> {
    type Row = (Uuid, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>);
    let d: Option<Row> = sqlx::query_as("SELECT owner_id, trashed_at, taken_down_at FROM designs WHERE id = $1").bind(id).fetch_optional(&s.pool).await?;
    let Some((_, trashed, taken_down)) = d.filter(|(o, ..)| *o == user_id) else { return Ok(Some(Failure::new(404, "not_found"))) };
    if taken_down.is_some() && trashed.is_some() {
        return Ok(Some(Failure::new(409, "on_hold")));
    }
    if permanent && trashed.is_some() {
        pg::exec(&s.pool, "DELETE FROM designs WHERE id = $1", &[pg::text(id)]).await?;
    } else {
        pg::exec(&s.pool, "UPDATE designs SET trashed_at = now() WHERE id = $1", &[pg::text(id)]).await?;
    }
    teams::revoke_live(s, Some(id), None, None).await?;
    let (s2, data) = (s.clone(), json!({ "designId": id, "permanent": permanent && trashed.is_some() }));
    s.spawn(async move {
        let _ = oc_webhooks::emit_event(&s2.pool, user_id, oc_webhooks::WebhookEvent::DesignDeleted, &data).await;
    });
    Ok(None)
}

/// `restoreDesign(userId, id)`.
async fn restore_design(s: &AppState, user_id: Uuid, id: &str) -> Result<Option<Failure>, ApiError> {
    let r = pg::rows(&s.pool, "UPDATE designs SET trashed_at = NULL WHERE id = $1 AND owner_id = $2 AND taken_down_at IS NULL RETURNING id", &[pg::text(id), pg::uuid(user_id)]).await?;
    if !r.is_empty() {
        return Ok(None);
    }
    let held = pg::one(&s.pool, "SELECT 1 FROM designs WHERE id = $1 AND owner_id = $2 AND taken_down_at IS NOT NULL", &[pg::text(id), pg::uuid(user_id)]).await?;
    Ok(Some(if held.is_some() { Failure::new(409, "on_hold") } else { Failure::new(404, "not_found") }))
}

/// Several designs at once (Projects bulk select, g17): each design follows the rules of its single-design route;
/// the ones the user may not change come back in `failed`.
async fn bulk(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([
        ("ids", z::array(z::str().min(1.0).max(64.0)).min(1.0).max(200.0)),
        ("action", z::enumeration(&["star", "unstar", "move", "trash", "restore", "delete"])),
        ("folderId", z::str().uuid().nullable().optional()),
    ])
    .parse(req.body())?;
    let action = b["action"].as_str().unwrap_or("");
    if action == "move" && b.get("folderId").is_none() {
        return Ok(reply::json(400, json!({ "error": "folder_required" })));
    }
    let folder = b.get("folderId").and_then(Value::as_str).and_then(|f| Uuid::parse_str(f).ok());
    let mut seen: Vec<String> = Vec::new();
    for id in b["ids"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if !seen.iter().any(|x| x == id) {
            seen.push(id.to_owned());
        }
    }
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    for id in seen {
        let fail = match action {
            "star" | "unstar" => set_star(&s, u.id, &id, action == "star", None).await?,
            "move" => move_design(&s, u.id, &id, folder, None).await?,
            "restore" => restore_design(&s, u.id, &id).await?,
            _ => trash_design(&s, u.id, &id, action == "delete").await?,
        };
        match fail {
            Some(f) => failed.push(json!({ "id": id, "error": f.error })),
            None => done.push(json!(id)),
        }
    }
    Ok(Reply::json(json!({ "done": done, "failed": failed })))
}

async fn create(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = create_design_body().parse(req.body())?;
    Ok(match create_design_for_user(&s, u.id, &b, "editor").await? {
        Ok(id) => Reply::json(json!({ "id": id })),
        Err((status, body)) => reply::json(status, body),
    })
}

/// Body of `POST /api/designs/:id/copy`: all optional; `w` and `h` together resize the copy (g101).
pub fn copy_design_body() -> z::Schema {
    z::obj([
        ("title", z::str().min(1.0).max(200.0).optional()),
        ("w", z::num().positive().max(20000.0).optional()),
        ("h", z::num().positive().max(20000.0).optional()),
        ("unit", z::enumeration(&["px", "mm", "cm", "in", "pt"]).optional()),
        ("scaleContent", z::bool().optional()),
    ])
}

/// `CreateDesignBody`.
pub fn create_design_body() -> z::Schema {
    z::obj([
        ("preset", z::str().optional()),
        ("w", z::num().positive().max(20000.0).optional()),
        ("h", z::num().positive().max(20000.0).optional()),
        ("unit", z::enumeration(&["px", "mm", "cm", "in", "pt"]).optional()),
        ("title", z::str().max(200.0).optional()),
        ("templateId", z::str().optional()),
        ("folderId", z::str().uuid().nullable().optional()),
        ("workspaceId", z::str().uuid().nullable().optional()),
    ])
}

/// `createDesignForUser(userId, body, via)`: create a design (blank from a preset/size, or from a template), applying
/// team and brand-template rules. `Ok(Err((status, body)))` is a refusal (403/404) as the route answers it.
pub async fn create_design_for_user(s: &AppState, user_id: Uuid, b: &Value, via: &str) -> Result<Result<String, (u16, Value)>, ApiError> {
    let s = s.clone();
    let u = User { id: user_id, email: String::new(), name: String::new(), color: String::new(), is_staff: false };
    let bs = |k: &str| b.get(k).and_then(Value::as_str).map(str::to_owned);
    let template_id = bs("templateId");
    let mut workspace_id = bs("workspaceId").filter(|w| !w.is_empty()).and_then(|w| Uuid::parse_str(&w).ok());
    // Creating inside a folder (Projects '+ Add new', g17): the folder must be one the user may file designs in, and a
    // team folder makes the design part of that team, under the team's brand rules below.
    if let Some(fid) = bs("folderId").filter(|f| !f.is_empty()).and_then(|f| Uuid::parse_str(&f).ok()) {
        let Some(f) = folder_access(&s, u.id, fid).await? else { return Ok(Err((404, json!({ "error": "folder_not_found" })))) };
        if !f.file {
            return Ok(Err((403, json!({ "error": "forbidden" }))));
        }
        if !f.personal {
            if workspace_id.is_some_and(|w| w != f.workspace_id) {
                return Ok(Err((400, json!({ "error": "folder_workspace" }))));
            }
            workspace_id = Some(f.workspace_id);
        }
    }
    if let Some(ws) = workspace_id {
        let r = teams::team_role(&s, u.id, ws).await?;
        let Some(r) = r else {
            return Ok(Err((403, json!({ "error": "forbidden", "message": "You can’t create designs in this team." }))));
        };
        // Brand controls (A8): members (always) and designers (when required) start team designs from the team's templates.
        let must_use_template =
            r == "member" || (!brandhub::is_admin_role(Some(&r)) && brandhub::control_on(&brandhub::brand_controls(&s, Some(ws)).await?, "requireTemplate"));
        if must_use_template {
            let own = match template_id.as_deref().filter(|t| !t.is_empty()) {
                Some(t) => pg::one(&s.pool, "SELECT 1 FROM templates WHERE id = $1 AND workspace_id = $2 AND published", &[pg::text(t), pg::uuid(ws)]).await?.is_some(),
                None => false,
            };
            if !own {
                let message = if r == "member" { "Members create team designs from the team’s brand templates." } else { "This team requires designs to start from one of its brand templates." };
                return Ok(Err((403, json!({ "error": "template_required", "message": message }))));
            }
        }
    }
    let mut brand_workspace_id: Option<Uuid> = None;
    let mut template_version: Option<i32> = None;
    let doc = match template_id.as_deref().filter(|t| !t.is_empty()) {
        Some(tid) => {
            type Template = (sqlx::types::Json<Value>, String, Option<Uuid>, bool, i32);
            let t: Option<Template> =
                sqlx::query_as("SELECT doc, title, workspace_id, brand, version FROM templates WHERE id = $1 AND published").bind(tid).fetch_optional(&s.pool).await?;
            if let Some((_, _, Some(tws), _, _)) = &t
                && pg::one(&s.pool, "SELECT 1 FROM memberships WHERE workspace_id = $1 AND user_id = $2", &[pg::uuid(*tws), pg::uuid(u.id)]).await?.is_none()
            {
                return Ok(Err((404, json!({ "error": "not_found" }))));
            }
            if t.is_some() {
                pg::exec(&s.pool, "UPDATE templates SET uses = uses + 1 WHERE id = $1", &[pg::text(tid)]).await?;
            }
            let Some((tdoc, ttitle, tws, tbrand, version)) = t else {
                return Ok(Err((404, json!({ "error": "template_not_found" }))));
            };
            let mut doc = tdoc.0;
            if is_retired_surface(&doc) {
                return Ok(Err((422, unsupported_surface())));
            }
            if let Some(w) = tws {
                brand_workspace_id = Some(w);
                template_version = Some(version);
            }
            let mut origin = Map::new();
            origin.insert("templateId".into(), json!(tid));
            if tbrand {
                origin.insert("brand".into(), json!(true));
            }
            if let Some(w) = tws {
                origin.insert("workspaceId".into(), json!(w.to_string()));
                origin.insert("version".into(), json!(version));
            }
            let mut meta = doc.get("meta").and_then(Value::as_object).cloned().unwrap_or_default();
            meta.insert("title".into(), json!(bs("title").unwrap_or(ttitle)));
            meta.insert("templateOrigin".into(), Value::Object(origin));
            if let Some(o) = doc.as_object_mut() {
                o.insert("meta".into(), Value::Object(meta));
            }
            doc
        }
        None => {
            // The preset picks the surface; an explicit size overrides the preset's. Unknown ids fall back to a square graphic.
            let preset = bs("preset").and_then(|p| oc_schema::presets::find_preset(&p));
            let o = oc_schema::presets::PresetDesignOverrides {
                w: b.get("w").and_then(Value::as_f64),
                h: b.get("h").and_then(Value::as_f64),
                unit: b.get("unit").and_then(|u| serde_json::from_value(u.clone()).ok()),
                title: bs("title"),
            };
            let init = oc_schema::presets::preset_design_init(preset, &o);
            oc_schema::js::to_value(&oc_schema::factory::empty_design(&init.meta_init()))?
        }
    };
    // The brand kit an assistant made the design with (create_design { brand_kit }), so get_design can show it later.
    let mut doc = doc;
    if let Some(kit) = b.get("brandKit").filter(|k| k.is_object())
        && let Some(meta) = doc.get_mut("meta").and_then(Value::as_object_mut)
    {
        meta.insert("brandKit".into(), kit.clone());
    }
    let folder_id = bs("folderId").and_then(|f| Uuid::parse_str(&f).ok());
    let id = create_design_from_doc(&s, u.id, &doc, CreateOpts { template_id: template_id.clone(), folder_id, workspace_id, brand_workspace_id, template_version }).await?;
    let title = doc.pointer("/meta/title").cloned().unwrap_or(Value::Null);
    let (s2, uid, id2, via) = (s.clone(), u.id, id.clone(), via.to_owned());
    s.spawn(async move {
        let _ = oc_webhooks::emit_event(&s2.pool, uid, oc_webhooks::WebhookEvent::DesignCreated, &json!({ "designId": id2, "title": title, "via": via })).await;
    });
    let (s3, props) = (s.clone(), json!({ "template": template_id, "preset": bs("preset") }));
    s.spawn(async move {
        let _ = analytics::track(&s3, Some(uid), "design_created", props).await;
    });
    Ok(Ok(id))
}

async fn read(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(role) = design_role(&s, Some(u.id), &id).await? else { return Ok(not_found()) };
    let d = pg::json_one(&s.pool, &format!("{LIST_SQL} WHERE d.id = $2"), &[pg::uuid(u.id), pg::text(&id)]).await?;
    let mut design = d.and_then(|d| d.as_object().cloned()).unwrap_or_default();
    design.insert("role".into(), json!(role.as_str()));
    Ok(Reply::json(json!({ "design": design })))
}

async fn update(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([
        ("title", z::str().min(1.0).max(200.0).optional()),
        ("folderId", z::str().uuid().nullable().optional()),
        ("linkAccess", z::enumeration(&["restricted", "view", "comment", "edit"]).optional()),
        ("starred", z::bool().optional()),
    ])
    .parse(req.body())?;
    let Some(role) = design_role(&s, Some(u.id), &id).await? else { return Ok(not_found()) };
    if let Some(starred) = b.get("starred").and_then(Value::as_bool) {
        set_star(&s, u.id, &id, starred, Some(role)).await?;
    }
    if let Some(title) = b.get("title").and_then(Value::as_str) {
        if !at_least(Some(role), Role::Edit) {
            return Ok(forbidden());
        }
        pg::exec(&s.pool, "UPDATE designs SET title = $2, updated_at = now() WHERE id = $1", &[pg::text(&id), pg::text(title)]).await?;
    }
    if let Some(folder) = b.get("folderId") {
        let fid = folder.as_str().and_then(|f| Uuid::parse_str(f).ok());
        if let Some(fail) = move_design(&s, u.id, &id, fid, Some(role)).await? {
            return Ok(fail.reply());
        }
    }
    if let Some(l) = b.get("linkAccess").and_then(Value::as_str) {
        if role != Role::Owner {
            return Ok(forbidden());
        }
        pg::exec(&s.pool, "UPDATE designs SET link_access = $2 WHERE id = $1", &[pg::text(&id), pg::text(l)]).await?;
        teams::revoke_live(&s, Some(&id), None, None).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}

/// Move to trash (owner) — restorable for 30 days. `?permanent=1` deletes from the trash.
async fn remove(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let permanent = req.query_str("permanent") == Some("1");
    Ok(match trash_design(&s, u.id, &id, permanent).await? {
        Some(f) => f.reply(),
        None => Reply::json(json!({ "ok": true })),
    })
}

async fn restore(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    Ok(match restore_design(&s, u.id, &id).await? {
        Some(f) => f.reply(),
        None => Reply::json(json!({ "ok": true })),
    })
}

async fn copy(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    // "Copy and resize" (g101): an optional new size (and title) for the copy; no body = a plain copy.
    let raw = match req.body() {
        Some(v @ Value::Object(_)) => v.clone(),
        _ => json!({}),
    };
    let b = copy_design_body().parse(Some(&raw))?;
    let b = b.as_object().cloned().unwrap_or_default();
    let resize = b.get("w").and_then(Value::as_f64).zip(b.get("h").and_then(Value::as_f64));
    let new_id = match copy_design_for(&s, u.id, &id, &b, false, &|_| {}).await? {
        Ok(id) => id,
        Err((status, body)) => return Ok(reply::json(status, body)),
    };
    // `resized` tells the client the size was applied (a plain copy answers without it).
    Ok(Reply::json(if resize.is_some() { json!({ "id": new_id, "resized": true }) } else { json!({ "id": new_id }) }))
}

/// `POST /api/designs/:id/copy` after the access check: a copy of the design (resized when `w` and `h` are given,
/// g101) next to the original when the user may add designs there. `b` is the parsed `copy_design_body`. The MCP
/// `copy_design` tool shares it; `edit` changes the copy's document before it is saved (MCP `bulk_create`). `live`
/// copies the live document even without a resize (assistants copy right after their own edits).
pub async fn copy_design_for(
    s: &AppState,
    uid: Uuid,
    id: &str,
    b: &Map<String, Value>,
    live: bool,
    edit: &(dyn Fn(&mut Value) + Sync),
) -> Result<Result<String, (u16, Value)>, ApiError> {
    let resize = b.get("w").and_then(Value::as_f64).zip(b.get("h").and_then(Value::as_f64));
    // A resized copy is made right after editing: it includes the acknowledged edits not yet folded into the snapshot.
    let loaded = if live || resize.is_some() { brandhub::load_ydoc(s, id).await?.map(|y| oc_crdt::from_y_value(&y)) } else { load_doc(s, id).await? };
    let Some(mut doc) = loaded else { return Ok(Err((404, json!({ "error": "not_found" })))) };
    let title = doc.pointer("/meta/title").map(|t| match t {
        Value::String(s) => s.clone(),
        other => crate::http::jsjson::stringify(other),
    });
    let new_title = b.get("title").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| format!("Copy of {}", title.unwrap_or_else(|| "undefined".into())));
    if let Some(meta) = doc.get_mut("meta").and_then(Value::as_object_mut) {
        meta.insert("title".into(), json!(new_title));
    }
    if let Some((w, h)) = resize {
        let unit = b.get("unit").and_then(|u| serde_json::from_value::<oc_schema::Unit>(u.clone()).ok());
        let scale = b.get("scaleContent").and_then(Value::as_bool).unwrap_or(false);
        if oc_editor_core::resize_doc(&mut doc, w, h, unit, scale).is_err() {
            return Ok(Err((422, json!({ "error": "invalid_document" }))));
        }
    }
    edit(&mut doc);
    // A copy stays linked to its brand template, so the team's brand controls keep applying.
    type Src = (Option<String>, Option<Uuid>, Option<i32>, Option<Uuid>);
    let src: Option<Src> =
        sqlx::query_as("SELECT template_id, brand_workspace_id, template_version, folder_id FROM designs WHERE id = $1").bind(id).fetch_optional(&s.pool).await?;
    let (template_id, brand_workspace_id, template_version, src_folder) = src.unwrap_or((None, None, None, None));
    // The copy lands next to the original when the user may add designs there: their own folder, or a team folder
    // they design in (then the copy belongs to that team). Otherwise it goes to their own Projects.
    let f = match src_folder {
        Some(fid) => folder_access(s, uid, fid).await?,
        None => None,
    };
    let (folder_id, workspace_id) = match f {
        Some(f) if (if f.personal { f.file } else { f.add }) => (Some(f.id), if f.personal { None } else { Some(f.workspace_id) }),
        _ => (None, None),
    };
    let new_id = create_design_from_doc(s, uid, &doc, CreateOpts { template_id, brand_workspace_id, template_version, folder_id, workspace_id }).await?;
    Ok(Ok(new_id))
}

/// Short-lived ticket for the collab websocket (role decides read-only vs read-write).
async fn ticket(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(role) = design_role(&s, Some(u.id), &id).await? else { return Ok(not_found()) };
    let claims = TicketClaims {
        sub: uid_str(&u),
        name: u.name.clone(),
        color: u.color.clone(),
        aud: Audience::Collab,
        design: id.clone(),
        role,
    };
    let token = oc_auth::sign_ticket_at(&s.cfg.secret, &claims, 300, s.now_ms());
    Ok(Reply::json(json!({ "token": token, "role": role.as_str() })))
}

/// Latest persisted snapshot as JSON (read-only views, previews, tests).
async fn doc(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    match load_doc(&s, &id).await? {
        Some(doc) => Ok(Reply::json(json!({ "doc": doc }))),
        None => Ok(not_found()),
    }
}

async fn thumbnail(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(Reply::empty(404));
    }
    let t: Option<Option<String>> = sqlx::query_scalar("SELECT thumb_sha256 FROM designs WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    if let Some(Some(sha)) = t {
        return Ok(Reply::bytes("image/png", s.store.get(&sha).await?).header("cache-control", "private, max-age=5"));
    }
    let file = s.paths.thumbs.join(format!("{id}.png")); // legacy on-disk thumbnails
    match tokio::fs::read(&file).await {
        Ok(b) => Ok(Reply::bytes("image/png", b).header("cache-control", "private, max-age=5")),
        Err(_) => Ok(Reply::empty(404).header("cache-control", "private, max-age=5")),
    }
}

// ---------------------------------------------------------------- sharing

async fn shares(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    let (link_access, owner_id): (String, Uuid) = sqlx::query_as("SELECT link_access, owner_id FROM designs WHERE id = $1").bind(&id).fetch_one(&s.pool).await?;
    let owner = pg::json_one(&s.pool, "SELECT id, name, email, color FROM users WHERE id = $1", &[pg::uuid(owner_id)]).await?;
    let people = pg::json_rows(
        &s.pool,
        "SELECT u.id, u.name, u.email, u.color, s.role FROM design_shares s JOIN users u ON u.id = s.user_id WHERE s.design_id = $1 ORDER BY s.created_at",
        &[pg::text(&id)],
    )
    .await?;
    Ok(Reply::json(json!({ "linkAccess": link_access, "owner": owner, "people": people })))
}

async fn share(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("email", z::str().email()), ("role", z::enumeration(&["view", "comment", "edit"]))]).parse(req.body())?;
    let (email, role) = (b["email"].as_str().unwrap_or(""), b["role"].as_str().unwrap_or(""));
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    let target: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE email = $1").bind(email.to_lowercase()).fetch_optional(&s.pool).await?;
    let Some(target) = target else {
        return Ok(reply::json(404, json!({ "error": "user_not_found", "message": "That person needs an account first." })));
    };
    if target == u.id {
        return Ok(reply::json(400, json!({ "error": "self" })));
    }
    pg::exec(
        &s.pool,
        "INSERT INTO design_shares(design_id, user_id, role) VALUES ($1,$2,$3) ON CONFLICT (design_id, user_id) DO UPDATE SET role = EXCLUDED.role",
        &[pg::text(&id), pg::uuid(target), pg::text(role)],
    )
    .await?;
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent {
            actor_id: Some(u.id),
            action: "design.share".into(),
            target: Some(id.clone()),
            meta: Some(json!({ "user": target.to_string(), "role": role })),
            ip: Some(req.ip.clone()),
            ..Default::default()
        },
        s.now(),
    )
    .await?;
    let title: Option<String> = sqlx::query_scalar("SELECT title FROM designs WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let title = title.unwrap_or_else(|| "a design".into());
    let can = match role {
        "edit" => "can edit",
        "comment" => "can comment",
        _ => "can view",
    };
    brand::notify(&s, target, "share", &id, u.id, &format!("{} shared “{title}” with you ({can})", u.name)).await?;
    oc_mail::send_email(
        &s.pool,
        email,
        &oc_mail::Template::Share { actor: u.name.clone(), title, url: format!("{}/design/{id}", s.cfg.web_origin), role: role.to_owned() },
        Some(target),
    )
    .await?;
    teams::revoke_live(&s, Some(&id), None, Some(&target.to_string())).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn unshare(State(s): State<AppState>, Path((id, user_id)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if z::str().uuid().safe_parse(Some(&json!(user_id))).is_err() {
        return Ok(reply::json(400, json!({ "error": "invalid_request" })));
    }
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    pg::exec(&s.pool, "DELETE FROM design_shares WHERE design_id = $1 AND user_id = $2::uuid", &[pg::text(&id), pg::text(&user_id)]).await?;
    teams::revoke_live(&s, Some(&id), None, Some(&user_id)).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- versions

async fn versions(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    let versions = pg::json_rows(
        &s.pool,
        "SELECT v.id, v.label, v.created_at, u.name AS author FROM design_versions v LEFT JOIN users u ON u.id = v.created_by
       WHERE v.design_id = $1 ORDER BY v.created_at DESC LIMIT 100",
        &[pg::text(&id)],
    )
    .await?;
    Ok(Reply::json(json!({ "versions": versions })))
}

async fn version(State(s): State<AppState>, Path((id, vid)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    let v: Option<Vec<u8>> = sqlx::query_scalar("SELECT state FROM design_versions WHERE id = $1::uuid AND design_id = $2").bind(&vid).bind(&id).fetch_optional(&s.pool).await?;
    let Some(state) = v else { return Ok(not_found()) };
    Ok(Reply::json(json!({ "doc": oc_crdt::decode_doc_value(&state)? })))
}

async fn save_version(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("label", z::str().max(100.0).optional())]).parse(Some(&req.body_or_empty()))?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::Edit) {
        return Ok(forbidden());
    }
    let st: Option<Vec<u8>> = sqlx::query_scalar("SELECT state FROM doc_state WHERE design_id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some(state) = st else { return Ok(not_found()) };
    sqlx::query("INSERT INTO design_versions(id, design_id, state, label, created_by) VALUES ($1,$2,$3,$4,$5)")
        .bind(oc_db::uuid())
        .bind(&id)
        .bind(state)
        .bind(b.get("label").and_then(Value::as_str))
        .bind(u.id)
        .execute(&s.pool)
        .await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- export

async fn export(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([
        ("format", z::enumeration(&["png", "jpg", "pdf", "pdf-print", "pdf-flat", "pptx", "zip", "svg", "mp4", "gif", "docx", "md"])),
        ("pages", z::array(z::num().int().nonnegative()).optional()),
        ("scale", z::num().min(0.1).max(4.0).default(json!(1))),
        ("quality", z::num().min(10.0).max(100.0).default(json!(90))),
        ("transparent", z::bool().default(json!(false))),
        ("print", print_profiles::print_export_options().optional()),
        (
            "watermark",
            z::obj([
                ("text", z::str().max(120.0)),
                ("position", z::enumeration(&["center", "tiled", "top-left", "top-right", "bottom-left", "bottom-right"]).optional()),
                ("opacity", z::num().min(0.0).max(1.0).optional()),
                ("rotation", z::num().min(-180.0).max(180.0).optional()),
                ("size", z::num().min(0.02).max(0.4).optional()),
                ("color", z::str().max(9.0).optional()),
            ])
            .optional(),
        ),
    ])
    .parse(req.body())?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    let surface: Option<String> = sqlx::query_scalar("SELECT surface FROM designs WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    if surface.as_deref() == Some("sheet") {
        return Ok(reply::json(422, json!({ "error": "sheet_unsupported", "message": "Sheets are no longer supported, so this design can't be exported." })));
    }
    // Print options: an uploaded colour profile ({ id }) becomes its stored object for the render service.
    let Some(print) = print_profiles::resolve_print_options(&s, u.id, b.get("print")).await? else { return Ok(print_profiles::profile_not_found()) };
    // Docs: embedded designs render only when this user can view them.
    let mut body = Map::new();
    body.insert("designId".into(), json!(id));
    if let Value::Object(m) = &b {
        for (k, v) in m {
            body.insert(k.clone(), v.clone());
        }
    }
    body.extend(print);
    body.insert("embeds".into(), json!(docs::embed_allow_list(&s, Some(u.id), &id).await?));
    let res = s
        .http
        .post(format!("{}/internal/export", s.cfg.render_url))
        .header("content-type", "application/json")
        .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&s.cfg.secret))
        .body(crate::http::jsjson::stringify(&Value::Object(body)))
        .send()
        .await
        .ok();
    let Some(res) = res else {
        return Ok(reply::json(502, json!({ "error": "render_unavailable", "message": "render service is not running" })));
    };
    if !res.status().is_success() {
        let text = res.text().await.unwrap_or_default();
        return Ok(reply::json(502, json!({ "error": "render_unavailable", "message": text })));
    }
    let ct = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_owned();
    let cd = res.headers().get("content-disposition").and_then(|v| v.to_str().ok()).unwrap_or("attachment").to_owned();
    let bytes = res.bytes().await?;
    Ok(Reply::bytes(&ct, bytes).header("content-disposition", cd))
}

/// Keep `page_count` in sync for the home grid.
async fn touch(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::Edit) {
        return Ok(forbidden());
    }
    if let Some(doc) = load_doc(&s, &id).await? {
        let n = doc.get("pages").and_then(Value::as_object).map_or(0, Map::len) as i32;
        pg::exec(&s.pool, "UPDATE designs SET page_count = $2 WHERE id = $1", &[pg::text(&id), Arg::Int4(Some(n))]).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}
