//! Port of services/api/src/routes/teams.ts — teams (CO-060/061, VE-001): team workspaces, email invites,
//! roles, membership management, member groups (g149). Every route runs `requireUser` first.
//!
//! Also exported for other modules: [`revoke_live`], [`team_role`] / [`team_role_str`], [`rank`].

use crate::prelude::*;

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/workspaces", get(list).post(create))
        .route("/api/workspaces/{id}", patch(rename))
        .route("/api/workspaces/{id}/members", get(members))
        .route("/api/workspaces/{id}/invites", post(invite))
        .route("/api/workspaces/{id}/invites/{inv}", delete(cancel_invite))
        .route("/api/invites/accept", post(accept))
        .route("/api/workspaces/{id}/members/{user_id}", patch(change_role).delete(remove_member))
        .route("/api/workspaces/{id}/groups", get(groups).post(create_group))
        .route("/api/workspaces/{id}/groups/{gid}", patch(rename_group).delete(delete_group))
        .route("/api/workspaces/{id}/groups/{gid}/members", post(add_group_members))
        .route("/api/workspaces/{id}/groups/{gid}/members/{user_id}", delete(remove_group_member))
}

/// `RANK[role]` (`member` 1 … `owner` 4); unknown roles rank 0.
pub fn rank(role: &str) -> u8 {
    match role {
        "member" => 1,
        "designer" => 2,
        "admin" => 3,
        "owner" => 4,
        _ => 0,
    }
}

// ---------------------------------------------------------------- shared helpers

/// `revokeLive(scope)`: tell collab to drop live connections whose access changed (CO-006).
pub async fn revoke_live(state: &AppState, design_id: Option<&str>, workspace_id: Option<Uuid>, user_id: Option<&str>) -> Result<(), sqlx::Error> {
    revoke_live_str(state, design_id, workspace_id.map(|w| w.to_string()).as_deref(), user_id).await
}

/// [`revoke_live`] with the ids as the strings the TS passed through.
pub async fn revoke_live_str(state: &AppState, design_id: Option<&str>, workspace_id: Option<&str>, user_id: Option<&str>) -> Result<(), sqlx::Error> {
    let mut scope = serde_json::Map::new();
    if let Some(d) = design_id {
        scope.insert("designId".into(), json!(d));
    }
    if let Some(w) = workspace_id {
        scope.insert("workspaceId".into(), json!(w));
    }
    if let Some(u) = user_id {
        scope.insert("userId".into(), json!(u));
    }
    scope.insert("at".into(), json!(state.now_ms()));
    sqlx::query("SELECT pg_notify($1, $2)").bind("oc_revoke").bind(crate::http::jsjson::stringify(&Value::Object(scope))).execute(&state.pool).await?;
    Ok(())
}

/// `teamRole(userId, workspaceId)`.
pub async fn team_role(state: &AppState, user_id: Uuid, workspace_id: Uuid) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT role FROM memberships WHERE workspace_id = $1 AND user_id = $2").bind(workspace_id).bind(user_id).fetch_optional(&state.pool).await
}

/// `teamRole(userId, workspaceId)` with ids from a URL (a malformed uuid fails in Postgres, as in TS).
pub async fn team_role_str(state: &AppState, user_id: &str, workspace_id: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT role FROM memberships WHERE workspace_id = $1::uuid AND user_id = $2::uuid").bind(workspace_id).bind(user_id).fetch_optional(&state.pool).await
}

fn forbidden() -> Reply {
    reply::json(403, json!({ "error": "forbidden" }))
}

async fn audit(s: &AppState, actor: Uuid, workspace: Option<Uuid>, action: &str, target: Option<&str>, meta: Option<Value>, ip: &str) -> Result<(), ApiError> {
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: Some(actor), workspace_id: workspace, action: action.into(), target: target.map(str::to_owned), meta, ip: Some(ip.to_owned()) },
        s.now(),
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------- routes

async fn list(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let rows = pg::json_rows(
        &s.pool,
        "SELECT w.id, w.name, w.personal, m.role, (SELECT count(*)::int FROM memberships x WHERE x.workspace_id = w.id) AS members
       FROM memberships m JOIN workspaces w ON w.id = m.workspace_id WHERE m.user_id = $1 ORDER BY w.personal DESC, w.name",
        &[pg::uuid(u.id)],
    )
    .await?;
    Ok(Reply::json(json!({ "workspaces": rows })))
}

async fn create(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("name", z::str().min(1.0).max(80.0))]).parse(req.body())?;
    let id = oc_db::uuid();
    let mut t = s.pool.begin().await?;
    sqlx::query("INSERT INTO workspaces(id, name, personal, owner_id) VALUES ($1,$2,false,$3)").bind(id).bind(crate::js::trim(b["name"].as_str().unwrap_or(""))).bind(u.id).execute(&mut *t).await?;
    sqlx::query("INSERT INTO memberships(workspace_id, user_id, role) VALUES ($1,$2,'owner')").bind(id).bind(u.id).execute(&mut *t).await?;
    t.commit().await?;
    audit(&s, u.id, Some(id), "team.created", None, None, &req.ip).await?;
    Ok(Reply::json(json!({ "id": id.to_string() })))
}

async fn rename(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("name", z::str().min(1.0).max(80.0))]).parse(req.body())?;
    let r = team_role_str(&s, &u.id.to_string(), &id).await?;
    if r.as_deref().is_none_or(|r| rank(r) < rank("admin")) {
        return Ok(forbidden());
    }
    sqlx::query("UPDATE workspaces SET name = $2 WHERE id = $1::uuid AND NOT personal").bind(&id).bind(crate::js::trim(b["name"].as_str().unwrap_or(""))).execute(&s.pool).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn members(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if team_role_str(&s, &u.id.to_string(), &id).await?.is_none() {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let members = pg::json_rows(
        &s.pool,
        "SELECT u.id, u.name, u.email, u.color, m.role, m.created_at,
              COALESCE((SELECT json_agg(json_build_object('id', g.id, 'name', g.name) ORDER BY g.name) FROM team_group_members gm
                        JOIN team_groups g ON g.id = gm.group_id WHERE gm.user_id = u.id AND g.workspace_id = m.workspace_id), '[]') AS groups
       FROM memberships m JOIN users u ON u.id = m.user_id
       WHERE m.workspace_id = $1::uuid ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 WHEN 'designer' THEN 2 ELSE 3 END, u.name",
        &[pg::text(&id)],
    )
    .await?;
    let invites = pg::json_rows(
        &s.pool,
        "SELECT left(id, 12) AS id, email, data->>'role' AS role, expires_at FROM auth_tokens
       WHERE kind = 'invite' AND used_at IS NULL AND expires_at > now() AND data->>'workspaceId' = $1 ORDER BY created_at DESC",
        &[pg::text(&id)],
    )
    .await?;
    Ok(Reply::json(json!({ "members": members, "invites": invites })))
}

async fn invite(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("emails", z::array(z::str().email()).min(1.0).max(50.0)), ("role", z::enumeration(&["admin", "designer", "member"]))]).parse(req.body())?;
    let r = team_role_str(&s, &u.id.to_string(), &id).await?;
    if r.as_deref().is_none_or(|r| rank(r) < rank("admin")) {
        return Ok(reply::json(403, json!({ "error": "forbidden", "message": "Only owners and admins can invite." })));
    }
    let ws_id = Uuid::parse_str(&id).map_err(ApiError::internal)?;
    let (ws_name, personal): (String, bool) = sqlx::query_as("SELECT name, personal FROM workspaces WHERE id = $1").bind(ws_id).fetch_one(&s.pool).await?;
    if personal {
        return Ok(reply::json(400, json!({ "error": "personal", "message": "Create a team to invite people." })));
    }
    let emails: Vec<String> = b["emails"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
    let role = b["role"].as_str().unwrap_or("member");
    for email in &emails {
        let token = auth::issue_token(&s, "invite", None, Some(email), Some(json!({ "workspaceId": id, "role": role, "by": u.id.to_string() })), 60 * 24 * 7).await?;
        oc_mail::send_email(
            &s.pool,
            email,
            &oc_mail::Template::Invite { actor: u.name.clone(), team: ws_name.clone(), url: format!("{}/invite?token={token}", s.cfg.web_origin) },
            None,
        )
        .await?;
    }
    audit(&s, u.id, Some(ws_id), "team.invited", None, Some(json!({ "count": emails.len(), "role": role })), &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true, "sent": emails.len() })))
}

async fn cancel_invite(State(s): State<AppState>, Path((id, inv)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let r = team_role_str(&s, &u.id.to_string(), &id).await?;
    if r.as_deref().is_none_or(|r| rank(r) < rank("admin")) {
        return Ok(forbidden());
    }
    sqlx::query("UPDATE auth_tokens SET used_at = now() WHERE kind = 'invite' AND left(id, 12) = $1 AND data->>'workspaceId' = $2").bind(&inv).bind(&id).execute(&s.pool).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn accept(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("token", z::str().max(200.0))]).parse(req.body())?;
    let t = auth::consume_token(&s, "invite", b["token"].as_str().unwrap_or("")).await?;
    let data = t.as_ref().and_then(|t| t.data.as_ref()).map(|d| d.0.clone()).unwrap_or(Value::Null);
    let ws = match data.get("workspaceId") {
        Some(Value::String(w)) if !w.is_empty() => w.clone(),
        _ => return Ok(reply::json(400, json!({ "error": "expired", "message": "This invite has expired or was already used." }))),
    };
    let Some(t) = t else { return Ok(reply::json(400, json!({ "error": "expired", "message": "This invite has expired or was already used." }))) };
    if let Some(email) = t.email.as_deref().filter(|e| !e.is_empty())
        && email != u.email.to_lowercase()
    {
        return Ok(reply::json(403, json!({ "error": "wrong_account", "message": format!("This invite was sent to {email}. Sign in with that email to accept it.") })));
    }
    let role = match data.get("role") {
        Some(Value::String(r)) => r.clone(),
        _ => "member".into(),
    };
    // Only an active account that still holds the invited address joins: a squat tombstone (which renames the
    // address and drops memberships) or a disable may have committed since requireUser (D2).
    let mut tx = s.pool.begin().await?;
    let joined = match auth::lock_active_user(&mut tx, u.id).await? {
        Some((_, email)) if t.email.as_deref().filter(|e| !e.is_empty()).is_none_or(|e| e == email.to_lowercase()) => {
            sqlx::query("INSERT INTO memberships(workspace_id, user_id, role) VALUES ($1::uuid,$2,$3) ON CONFLICT (workspace_id, user_id) DO NOTHING").bind(&ws).bind(u.id).bind(&role).execute(&mut *tx).await?;
            true
        }
        _ => false,
    };
    tx.commit().await?;
    if !joined {
        return Ok(reply::json(403, json!({ "error": "disabled", "message": "This account can no longer accept invites." })));
    }
    audit(&s, u.id, Uuid::parse_str(&ws).ok(), "team.joined", None, None, &req.ip).await?;
    Ok(Reply::json(json!({ "workspaceId": ws })))
}

async fn owner_count(s: &AppState, ws: &str) -> Result<i32, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, i32>("SELECT count(*)::int AS n FROM memberships WHERE workspace_id = $1::uuid AND role = 'owner'").bind(ws).fetch_optional(&s.pool).await?.unwrap_or(0))
}

async fn change_role(State(s): State<AppState>, Path((id, user_id)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("role", z::enumeration(&["owner", "admin", "designer", "member"]))]).parse(req.body())?;
    let role = b["role"].as_str().unwrap_or("");
    let mine = team_role_str(&s, &u.id.to_string(), &id).await?;
    let theirs = team_role_str(&s, &user_id, &id).await?;
    let (Some(mine), Some(theirs)) = (mine, theirs) else { return Ok(forbidden()) };
    if rank(&mine) < rank("admin") || rank(&theirs) > rank(&mine) || (role == "owner" && mine != "owner") {
        return Ok(forbidden());
    }
    if theirs == "owner" && role != "owner" && owner_count(&s, &id).await? <= 1 {
        return Ok(reply::json(400, json!({ "error": "last_owner", "message": "A team needs at least one owner." })));
    }
    sqlx::query("UPDATE memberships SET role = $3 WHERE workspace_id = $1::uuid AND user_id = $2::uuid").bind(&id).bind(&user_id).bind(role).execute(&s.pool).await?;
    revoke_live_str(&s, None, Some(&id), Some(&user_id)).await?;
    audit(&s, u.id, Uuid::parse_str(&id).ok(), "team.role_changed", Some(&user_id), Some(json!({ "role": role })), &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn remove_member(State(s): State<AppState>, Path((id, user_id)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let mine = team_role_str(&s, &u.id.to_string(), &id).await?;
    let theirs = team_role_str(&s, &user_id, &id).await?;
    let me = user_id == u.id.to_string();
    let Some(theirs) = theirs else { return Ok(forbidden()) };
    if !me && mine.as_deref().is_none_or(|m| rank(m) < rank("admin") || rank(&theirs) > rank(m)) {
        return Ok(forbidden());
    }
    if theirs == "owner" && owner_count(&s, &id).await? <= 1 {
        return Ok(reply::json(400, json!({ "error": "last_owner", "message": "Transfer ownership before leaving." })));
    }
    let mut tx = s.pool.begin().await?;
    sqlx::query("DELETE FROM memberships WHERE workspace_id = $1::uuid AND user_id = $2::uuid").bind(&id).bind(&user_id).execute(&mut *tx).await?;
    // Leaving a team also leaves its groups (group shares stop applying either way: they need membership).
    sqlx::query("DELETE FROM team_group_members gm USING team_groups g WHERE g.id = gm.group_id AND g.workspace_id = $1::uuid AND gm.user_id = $2::uuid")
        .bind(&id)
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Every live session of theirs reconnects with a fresh ticket.
    revoke_live_str(&s, None, None, Some(&user_id)).await?;
    audit(&s, u.id, Uuid::parse_str(&id).ok(), if me { "team.left" } else { "team.member_removed" }, Some(&user_id), None, &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- member groups (g149)
// Every member sees the team's groups (they're share targets); owners and admins manage them.

fn group_name() -> z::Schema {
    z::str().trim().min(1.0).max(60.0)
}

fn is_uuid(v: &str) -> bool {
    z::str().uuid().safe_parse(Some(&json!(v))).is_ok()
}

fn group_exists() -> Reply {
    reply::json(409, json!({ "error": "group_exists", "message": "A group with that name already exists." }))
}

fn unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505")
}

async fn is_admin(s: &AppState, user: Uuid, ws: &str) -> Result<bool, sqlx::Error> {
    Ok(team_role_str(s, &user.to_string(), ws).await?.is_some_and(|r| rank(&r) >= rank("admin")))
}

async fn groups(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if team_role_str(&s, &u.id.to_string(), &id).await?.is_none() {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let groups = pg::json_rows(
        &s.pool,
        r#"SELECT g.id, g.name, g.created_at,
              COALESCE((SELECT json_agg(gm.user_id ORDER BY u.name) FROM team_group_members gm JOIN users u ON u.id = gm.user_id
                        JOIN memberships m ON m.workspace_id = g.workspace_id AND m.user_id = gm.user_id WHERE gm.group_id = g.id), '[]') AS "memberIds"
       FROM team_groups g WHERE g.workspace_id = $1::uuid ORDER BY lower(g.name)"#,
        &[pg::text(&id)],
    )
    .await?;
    Ok(Reply::json(json!({ "groups": groups })))
}

async fn create_group(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("name", group_name()), ("memberIds", z::array(z::str().uuid()).max(500.0).default(json!([])))]).parse(req.body())?;
    if !is_admin(&s, u.id, &id).await? {
        return Ok(reply::json(403, json!({ "error": "forbidden", "message": "Only owners and admins can manage groups." })));
    }
    let personal: Option<bool> = sqlx::query_scalar("SELECT personal FROM workspaces WHERE id = $1::uuid").bind(&id).fetch_optional(&s.pool).await?;
    if personal != Some(false) {
        return Ok(reply::json(400, json!({ "error": "personal", "message": "Create a team to use groups." })));
    }
    let name = b["name"].as_str().unwrap_or("").to_owned();
    let member_ids: Vec<String> = b["memberIds"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
    let gid = oc_db::uuid();
    let r: Result<(), sqlx::Error> = async {
        let mut tx = s.pool.begin().await?;
        sqlx::query("INSERT INTO team_groups(id, workspace_id, name, created_by) VALUES ($1,$2::uuid,$3,$4)").bind(gid).bind(&id).bind(&name).bind(u.id).execute(&mut *tx).await?;
        // Only current members of this team can join its groups.
        if !member_ids.is_empty() {
            sqlx::query("INSERT INTO team_group_members(group_id, user_id) SELECT $1, m.user_id FROM memberships m WHERE m.workspace_id = $2::uuid AND m.user_id = ANY($3::uuid[]) ON CONFLICT DO NOTHING")
                .bind(gid)
                .bind(&id)
                .bind(&member_ids)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }
    .await;
    match r {
        Ok(()) => {}
        Err(e) if unique_violation(&e) => return Ok(group_exists()),
        Err(e) => return Err(e.into()),
    }
    audit(&s, u.id, Uuid::parse_str(&id).ok(), "team.group_created", Some(&gid.to_string()), Some(json!({ "name": name })), &req.ip).await?;
    Ok(Reply::json(json!({ "id": gid.to_string() })))
}

async fn rename_group(State(s): State<AppState>, Path((id, gid)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("name", group_name())]).parse(req.body())?;
    if !is_admin(&s, u.id, &id).await? {
        return Ok(forbidden());
    }
    if !is_uuid(&gid) {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    match pg::rows(&s.pool, "UPDATE team_groups SET name = $3 WHERE id = $1::uuid AND workspace_id = $2::uuid RETURNING id", &[pg::text(&gid), pg::text(&id), pg::text(b["name"].as_str().unwrap_or(""))]).await {
        Ok(r) if r.is_empty() => Ok(reply::json(404, json!({ "error": "not_found" }))),
        Ok(_) => Ok(Reply::json(json!({ "ok": true }))),
        Err(e) if unique_violation(&e) => Ok(group_exists()),
        Err(e) => Err(e.into()),
    }
}

async fn delete_group(State(s): State<AppState>, Path((id, gid)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !is_admin(&s, u.id, &id).await? {
        return Ok(forbidden());
    }
    if !is_uuid(&gid) {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    // Collect the members first (deleting the group cascades them away).
    let members: Vec<Uuid> = sqlx::query_scalar("SELECT gm.user_id FROM team_group_members gm JOIN team_groups g ON g.id = gm.group_id WHERE gm.group_id = $1::uuid AND g.workspace_id = $2::uuid")
        .bind(&gid)
        .bind(&id)
        .fetch_all(&s.pool)
        .await?;
    let d = pg::rows(&s.pool, "DELETE FROM team_groups WHERE id = $1::uuid AND workspace_id = $2::uuid RETURNING id", &[pg::text(&gid), pg::text(&id)]).await?;
    if d.is_empty() {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    for m in members {
        revoke_live_str(&s, None, None, Some(&m.to_string())).await?;
    }
    audit(&s, u.id, Uuid::parse_str(&id).ok(), "team.group_deleted", Some(&gid), None, &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn add_group_members(State(s): State<AppState>, Path((id, gid)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("userIds", z::array(z::str().uuid()).min(1.0).max(500.0))]).parse(req.body())?;
    if !is_admin(&s, u.id, &id).await? {
        return Ok(forbidden());
    }
    if !is_uuid(&gid) || pg::one(&s.pool, "SELECT 1 FROM team_groups WHERE id = $1::uuid AND workspace_id = $2::uuid", &[pg::text(&gid), pg::text(&id)]).await?.is_none() {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let mut ids: Vec<String> = Vec::new();
    for v in b["userIds"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if !ids.iter().any(|x| x == v) {
            ids.push(v.to_owned());
        }
    }
    let n: Option<i32> = sqlx::query_scalar("SELECT count(*)::int AS n FROM memberships WHERE workspace_id = $1::uuid AND user_id = ANY($2::uuid[])").bind(&id).bind(&ids).fetch_optional(&s.pool).await?;
    if (n.unwrap_or(0) as usize) < ids.len() {
        return Ok(reply::json(400, json!({ "error": "not_member", "message": "Only members of this team can join its groups." })));
    }
    let added = pg::rows(&s.pool, "INSERT INTO team_group_members(group_id, user_id) SELECT $1::uuid, unnest($2::uuid[]) ON CONFLICT DO NOTHING RETURNING user_id", &[pg::text(&gid), Arg::TextArray(Some(ids))]).await?;
    Ok(Reply::json(json!({ "ok": true, "added": added.len() })))
}

async fn remove_group_member(State(s): State<AppState>, Path((id, gid, user_id)): Path<(String, String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !is_admin(&s, u.id, &id).await? {
        return Ok(forbidden());
    }
    if !is_uuid(&gid) || !is_uuid(&user_id) {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let gone = pg::rows(
        &s.pool,
        "DELETE FROM team_group_members gm USING team_groups g WHERE g.id = gm.group_id AND gm.group_id = $1::uuid AND g.workspace_id = $2::uuid AND gm.user_id = $3::uuid RETURNING gm.user_id",
        &[pg::text(&gid), pg::text(&id), pg::text(&user_id)],
    )
    .await?;
    if !gone.is_empty() {
        revoke_live_str(&s, None, None, Some(&user_id)).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}
