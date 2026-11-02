//! Port of services/api/src/routes/sharing.ts — sharing (plan 11, batch 2): "Add people" search over teammates,
//! groups and past collaborators (g131), group shares (g149), the team access level and custom public link slug
//! (g131), "Remove everyone's access" (g132) and template links that hand out copies (g136). Per-person shares
//! stay in routes/designs.rs; team groups themselves live in routes/teams.rs.

use oc_auth::Role;

use crate::analytics;
use crate::auth::{at_least, design_role, random_token};
use crate::http::ratelimit::MINUTE;
use crate::prelude::*;
use crate::routes::{brand, designs, teams};
use crate::share_links::{contains_pattern, normalize_slug, slug_message, slug_problem};

pub const LIMITS: &[RouteLimit] = &[RouteLimit { method: "POST", path: "/api/template-links/{token}/use", max: 20, window_ms: MINUTE }];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/public/template-links/{token}", get(template_landing))
        .route("/api/public/template-links/{token}/thumbnail.png", get(template_thumbnail))
        .route("/api/designs/{id}/sharing", get(sharing))
        .route("/api/sharing/people", get(people))
        .route("/api/designs/{id}/group-shares", post(group_share))
        .route("/api/designs/{id}/group-shares/{group_id}", delete(group_unshare))
        .route("/api/designs/{id}/team-access", put(team_access))
        .route("/api/designs/{id}/slug", put(slug))
        .route("/api/designs/{id}/make-private", post(make_private))
        .route("/api/designs/{id}/template-link", post(template_link).delete(template_link_off))
        .route("/api/template-links/{token}/use", post(template_use))
}

/// Group shares notify at most this many members.
const NOTIFY_CAP: i64 = 200;

fn forbidden() -> Reply {
    reply::json(403, json!({ "error": "forbidden" }))
}

async fn audit(s: &AppState, actor: Uuid, workspace: Option<Uuid>, action: &str, target: &str, meta: Value, ip: &str) -> Result<(), ApiError> {
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: Some(actor), workspace_id: workspace, action: action.into(), target: Some(target.to_owned()), meta: Some(meta), ip: Some(ip.to_owned()) },
        s.now(),
    )
    .await?;
    Ok(())
}

fn token_ok(t: &str) -> bool {
    (16..=64).contains(&t.len()) && t.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// `templateSource(token)`: the design behind a live template link (not trashed).
async fn template_source(s: &AppState, token: &str) -> Result<Option<Value>, ApiError> {
    if !token_ok(token) {
        return Ok(None);
    }
    Ok(pg::json_one(
        &s.pool,
        "SELECT d.id, d.title, u.name AS owner, d.width, d.height, d.unit, d.surface, d.page_count, d.thumb_sha256
     FROM designs d JOIN users u ON u.id = d.owner_id WHERE d.template_token = $1 AND d.trashed_at IS NULL",
        &[pg::text(token)],
    )
    .await?)
}

// ---------------------------------------------------------------- public (no sign-in)

async fn template_landing(State(s): State<AppState>, Path(token): Path<String>, _req: Req) -> ApiResult {
    let Some(d) = template_source(&s, &token).await? else { return Ok(reply::json(404, json!({ "error": "not_found" }))) };
    Ok(Reply::json(json!({
        "title": d["title"], "owner": d["owner"], "width": d["width"], "height": d["height"], "unit": d["unit"], "surface": d["surface"],
        "pages": d["page_count"], "thumb": d["thumb_sha256"].is_string(),
    }))
    .header("cache-control", "private, no-store"))
}

async fn template_thumbnail(State(s): State<AppState>, Path(token): Path<String>, _req: Req) -> ApiResult {
    let d = template_source(&s, &token).await?;
    let Some(sha) = d.as_ref().and_then(|d| d["thumb_sha256"].as_str()).filter(|x| !x.is_empty()) else { return Ok(Reply::empty(404)) };
    // Private: a disabled link must stop showing the preview, so no shared caches.
    Ok(Reply::bytes("image/png", s.store.get(sha).await?).header("cache-control", "private, max-age=60"))
}

// ---------------------------------------------------------------- share panel state

async fn sharing(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let role = design_role(&s, Some(u.id), &id).await?;
    if !at_least(role, Role::View) {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let owner = role == Some(Role::Owner);
    let d = pg::json_one(
        &s.pool,
        "SELECT d.workspace_id, w.personal, w.name AS team_name, d.team_access, d.public_view, d.public_slug, d.template_token, d.template_uses,
                EXISTS (SELECT 1 FROM memberships m WHERE m.workspace_id = d.workspace_id AND m.user_id = $2) AS member
         FROM designs d JOIN workspaces w ON w.id = d.workspace_id WHERE d.id = $1",
        &[pg::text(&id), pg::uuid(u.id)],
    )
    .await?
    .ok_or_else(|| ApiError::internal("Cannot read properties of null"))?;
    // Group names are team data: shown to members of the group's team, and to whoever manages the design.
    let groups = pg::json_rows(
        &s.pool,
        "SELECT g.id, g.name, w.name AS team, gs.role, (SELECT count(*)::int FROM team_group_members m WHERE m.group_id = g.id) AS members
         FROM design_group_shares gs JOIN team_groups g ON g.id = gs.group_id JOIN workspaces w ON w.id = g.workspace_id
         WHERE gs.design_id = $1 AND ($3::boolean OR EXISTS (SELECT 1 FROM memberships m WHERE m.workspace_id = g.workspace_id AND m.user_id = $2))
         ORDER BY gs.created_at",
        &[pg::text(&id), pg::uuid(u.id), Arg::Bool(Some(owner))],
    )
    .await?;
    let personal = d["personal"].as_bool().unwrap_or(false);
    let member = d["member"].as_bool().unwrap_or(false);
    let team = if !personal && (owner || member) { json!({ "id": d["workspace_id"], "name": d["team_name"], "access": d["team_access"] }) } else { Value::Null };
    let public_view = d["public_view"].as_bool().unwrap_or(false);
    Ok(Reply::json(json!({
        "role": role.map(|r| r.as_str()),
        "team": team,
        "groups": groups,
        "slug": if public_view || owner { d["public_slug"].clone() } else { Value::Null },
        "templateLink": if owner { json!({ "token": d["template_token"], "uses": d["template_uses"] }) } else { Value::Null },
    })))
}

// ---------------------------------------------------------------- "Add people" search (g131)

async fn people(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let q = z::obj([("q", z::str().max(100.0).optional()), ("design", z::str().max(64.0).optional()), ("limit", z::coerce_num().int().min(1.0).max(20.0).default(json!(8)))]).parse(Some(&req.query))?;
    let mut design_id: Option<String> = None;
    if let Some(d) = q.get("design").and_then(Value::as_str).filter(|d| !d.is_empty()) {
        if !at_least(design_role(&s, Some(u.id), d).await?, Role::View) {
            return Ok(reply::json(404, json!({ "error": "not_found" })));
        }
        design_id = Some(d.to_owned());
    }
    let term = crate::js::trim(q.get("q").and_then(Value::as_str).unwrap_or("")).to_owned();
    let pattern = (!term.is_empty()).then(|| contains_pattern(&term));
    let limit = q["limit"].as_f64().unwrap_or(8.0) as i64;
    let people = pg::json_rows(
        &s.pool,
        "WITH known AS (
           SELECT m2.user_id AS id, true AS teammate, NULL::timestamptz AS shared_at
             FROM memberships m1 JOIN workspaces w ON w.id = m1.workspace_id AND NOT w.personal
             JOIN memberships m2 ON m2.workspace_id = m1.workspace_id
            WHERE m1.user_id = $1
           UNION ALL
           SELECT s.user_id, false, s.created_at FROM design_shares s JOIN designs d ON d.id = s.design_id WHERE d.owner_id = $1
           UNION ALL
           SELECT d.owner_id, false, s.created_at FROM design_shares s JOIN designs d ON d.id = s.design_id WHERE s.user_id = $1
         )
         SELECT u.id, u.name, u.email, u.color, bool_or(k.teammate) AS teammate, max(k.shared_at) AS shared_at
         FROM known k JOIN users u ON u.id = k.id
         WHERE u.id <> $1 AND u.deleted_at IS NULL AND u.disabled_at IS NULL
           AND ($2::text IS NULL OR u.name ILIKE $2 ESCAPE '\\' OR u.email ILIKE $2 ESCAPE '\\')
           AND ($3::text IS NULL OR (u.id <> (SELECT owner_id FROM designs WHERE id = $3)
                AND NOT EXISTS (SELECT 1 FROM design_shares x WHERE x.design_id = $3 AND x.user_id = u.id)))
         GROUP BY u.id
         ORDER BY max(k.shared_at) DESC NULLS LAST, u.name
         LIMIT $4",
        &[pg::uuid(u.id), Arg::Text(pattern.clone()), Arg::Text(design_id.clone()), Arg::Int(Some(limit))],
    )
    .await?;
    let groups = pg::json_rows(
        &s.pool,
        "SELECT g.id, g.name, w.name AS team, (SELECT count(*)::int FROM team_group_members gm WHERE gm.group_id = g.id) AS members
         FROM team_groups g JOIN workspaces w ON w.id = g.workspace_id
         JOIN memberships m ON m.workspace_id = g.workspace_id AND m.user_id = $1
         WHERE ($2::text IS NULL OR g.name ILIKE $2 ESCAPE '\\' OR w.name ILIKE $2 ESCAPE '\\')
           AND ($3::text IS NULL OR NOT EXISTS (SELECT 1 FROM design_group_shares x WHERE x.design_id = $3 AND x.group_id = g.id))
         ORDER BY g.name LIMIT 5",
        &[pg::uuid(u.id), Arg::Text(pattern), Arg::Text(design_id)],
    )
    .await?;
    Ok(Reply::json(json!({ "people": people, "groups": groups, "suggested": term.is_empty() })))
}

// ---------------------------------------------------------------- group shares (g149)

async fn group_share(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("groupId", z::str().uuid()), ("role", z::enumeration(&["view", "comment", "edit"]))]).parse(req.body())?;
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    let (group_id, role) = (b["groupId"].as_str().unwrap_or(""), b["role"].as_str().unwrap_or(""));
    // Only groups of the sharer's own teams.
    let g: Option<(String, Uuid)> = sqlx::query_as("SELECT g.name, g.workspace_id FROM team_groups g JOIN memberships m ON m.workspace_id = g.workspace_id AND m.user_id = $2 WHERE g.id = $1::uuid")
        .bind(group_id)
        .bind(u.id)
        .fetch_optional(&s.pool)
        .await?;
    let Some((gname, gws)) = g else { return Ok(reply::json(404, json!({ "error": "group_not_found" }))) };
    // xmax = 0 only for a freshly inserted row.
    let inserted: Option<bool> = sqlx::query_scalar(
        "INSERT INTO design_group_shares(design_id, group_id, role, created_by) VALUES ($1,$2::uuid,$3,$4)
         ON CONFLICT (design_id, group_id) DO UPDATE SET role = EXCLUDED.role
         RETURNING (xmax = 0) AS inserted",
    )
    .bind(&id)
    .bind(group_id)
    .bind(role)
    .bind(u.id)
    .fetch_optional(&s.pool)
    .await?;
    audit(&s, u.id, Some(gws), "design.share_group", &id, json!({ "group": group_id, "role": role }), &req.ip).await?;
    if inserted != Some(true) {
        // Role change: reconnect live sessions with the new role.
        teams::revoke_live(&s, Some(&id), None, None).await?;
    } else {
        let title: Option<String> = sqlx::query_scalar("SELECT title FROM designs WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
        let title = title.unwrap_or_else(|| "a design".into());
        let members: Vec<Uuid> = sqlx::query_scalar("SELECT gm.user_id FROM team_group_members gm JOIN memberships m ON m.workspace_id = $2 AND m.user_id = gm.user_id WHERE gm.group_id = $1::uuid LIMIT $3")
            .bind(group_id)
            .bind(gws)
            .bind(NOTIFY_CAP)
            .fetch_all(&s.pool)
            .await?;
        let how = match role {
            "edit" => "can edit",
            "comment" => "can comment",
            _ => "can view",
        };
        for m in members {
            brand::notify(&s, m, "share", &id, u.id, &format!("{} shared “{title}” with {gname} ({how})", u.name)).await?;
        }
    }
    Ok(Reply::json(json!({ "ok": true })))
}

async fn group_unshare(State(s): State<AppState>, Path((id, group_id)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    if z::str().uuid().safe_parse(Some(&json!(group_id))).is_err() {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let gid = Uuid::parse_str(&group_id).map_err(ApiError::internal)?;
    pg::exec(&s.pool, "DELETE FROM design_group_shares WHERE design_id = $1 AND group_id = $2", &[pg::text(&id), pg::uuid(gid)]).await?;
    teams::revoke_live(&s, Some(&id), None, None).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- team access level (g131)

async fn team_access(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("access", z::enumeration(&["view", "comment", "edit"]).nullable())]).parse(req.body())?;
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    let d: Option<(bool, Uuid)> = sqlx::query_as("SELECT w.personal, d.workspace_id FROM designs d JOIN workspaces w ON w.id = d.workspace_id WHERE d.id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some((false, ws)) = d else { return Ok(reply::json(400, json!({ "error": "not_team", "message": "Only team designs have a team access level." }))) };
    let access = b.get("access").and_then(Value::as_str).map(str::to_owned);
    pg::exec(&s.pool, "UPDATE designs SET team_access = $2 WHERE id = $1", &[pg::text(&id), Arg::Text(access.clone())]).await?;
    audit(&s, u.id, Some(ws), "design.team_access", &id, json!({ "access": access }), &req.ip).await?;
    teams::revoke_live(&s, Some(&id), None, None).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- custom public link (g131)

fn slug_taken() -> Reply {
    reply::json(409, json!({ "error": "slug_taken", "message": "That link is already taken." }))
}

async fn slug(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("slug", z::str().max(200.0).nullable())]).parse(req.body())?;
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    let slug = b.get("slug").and_then(Value::as_str).map(normalize_slug);
    if let Some(slug) = &slug {
        if let Some(problem) = slug_problem(slug) {
            return Ok(reply::json(400, json!({ "error": format!("slug_{problem}"), "message": slug_message(problem) })));
        }
        if pg::one(&s.pool, "SELECT 1 FROM designs WHERE public_slug = $1 AND id <> $2", &[pg::text(slug), pg::text(&id)]).await?.is_some() {
            return Ok(slug_taken());
        }
        // Slugs are never recycled: the slug stays with the account (or design) that first claimed it.
        let owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM designs WHERE id = $1").bind(&id).fetch_one(&s.pool).await?;
        pg::exec(&s.pool, "INSERT INTO public_slug_claims(slug, owner_id, design_id) VALUES ($1,$2,$3) ON CONFLICT (slug) DO NOTHING", &[pg::text(slug), pg::uuid(owner), pg::text(&id)]).await?;
        let claim: Option<(Option<Uuid>, Option<String>)> = sqlx::query_as("SELECT owner_id, design_id FROM public_slug_claims WHERE slug = $1").bind(slug).fetch_optional(&s.pool).await?;
        match claim {
            Some((o, d)) if o == Some(owner) || d.as_deref() == Some(id.as_str()) => {}
            _ => return Ok(slug_taken()),
        }
    }
    if let Err(e) = pg::exec(&s.pool, "UPDATE designs SET public_slug = $2 WHERE id = $1", &[pg::text(&id), Arg::Text(slug.clone())]).await {
        // Two owners racing for the same slug: the unique index decides.
        if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") {
            return Ok(slug_taken());
        }
        return Err(e.into());
    }
    audit(&s, u.id, None, "design.public_slug", &id, json!({ "slug": slug }), &req.ip).await?;
    Ok(Reply::json(json!({ "slug": slug })))
}

// ---------------------------------------------------------------- remove everyone's access (g132)

async fn make_private(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    let mut t = s.pool.begin().await?;
    let people = sqlx::query("DELETE FROM design_shares WHERE design_id = $1").bind(&id).execute(&mut *t).await?.rows_affected();
    let groups = sqlx::query("DELETE FROM design_group_shares WHERE design_id = $1").bind(&id).execute(&mut *t).await?.rows_affected();
    // Restricted collaboration link, public view link off, template link off.
    sqlx::query("UPDATE designs SET link_access = 'restricted', public_view = false, template_token = NULL WHERE id = $1").bind(&id).execute(&mut *t).await?;
    t.commit().await?;
    let removed = json!({ "people": people, "groups": groups });
    audit(&s, u.id, None, "design.make_private", &id, removed.clone(), &req.ip).await?;
    teams::revoke_live(&s, Some(&id), None, None).await?;
    Ok(Reply::json(json!({ "ok": true, "removed": removed })))
}

// ---------------------------------------------------------------- template link (g136)

async fn template_link(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("regenerate", z::bool().default(json!(false)))]).parse(Some(&req.body_or_empty()))?;
    let regenerate = b["regenerate"].as_bool().unwrap_or(false);
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    let cur: Option<(Option<String>, i32)> = sqlx::query_as("SELECT template_token, template_uses FROM designs WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    if let Some((Some(tok), uses)) = &cur
        && !tok.is_empty()
        && !regenerate
    {
        return Ok(Reply::json(json!({ "token": tok, "uses": uses })));
    }
    let token = random_token(18);
    pg::exec(&s.pool, "UPDATE designs SET template_token = $2 WHERE id = $1", &[pg::text(&id), pg::text(&token)]).await?;
    audit(&s, u.id, None, "design.template_link", &id, json!({ "enabled": true, "regenerated": regenerate }), &req.ip).await?;
    Ok(Reply::json(json!({ "token": token, "uses": cur.map_or(0, |c| c.1) })))
}

async fn template_link_off(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    pg::exec(&s.pool, "UPDATE designs SET template_token = NULL WHERE id = $1", &[pg::text(&id)]).await?;
    audit(&s, u.id, None, "design.template_link", &id, json!({ "enabled": false }), &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

/// Use a template link: the signed-in person gets their own copy; the original stays out of reach.
async fn template_use(State(s): State<AppState>, Path(token): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(src) = template_source(&s, &token).await? else {
        return Ok(reply::json(404, json!({ "error": "not_found", "message": "This template link was turned off or doesn’t exist." })));
    };
    let src_id = src["id"].as_str().unwrap_or("").to_owned();
    let Some(mut doc) = designs::load_doc(&s, &src_id).await? else { return Ok(reply::json(404, json!({ "error": "not_found" }))) };
    // A fresh design in the user's own workspace: no brand-template link, no folder, no shares.
    if let Some(meta) = doc.get_mut("meta").and_then(Value::as_object_mut) {
        meta.remove("templateOrigin");
    }
    let new_id = designs::create_design_from_doc(&s, u.id, &doc, designs::CreateOpts::default()).await?;
    pg::exec(&s.pool, "UPDATE designs SET template_uses = template_uses + 1 WHERE id = $1", &[pg::text(&src_id)]).await?;
    let (s2, uid, surface) = (s.clone(), u.id, src["surface"].clone());
    s.spawn(async move {
        let _ = analytics::track(&s2, Some(uid), "template_link_used", json!({ "surface": surface })).await;
    });
    Ok(Reply::json(json!({ "id": new_id })))
}
