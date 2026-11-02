//! Port of services/api/src/routes/misc.ts — fonts (pinned OFL files, Google Fonts on demand, brand fonts),
//! templates (catalogue, preview, document, thumbnail), folders, comments (tasks, reactions, mentions), ownership
//! transfer and access requests.

use std::sync::OnceLock;

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use oc_auth::Role;

use crate::auth::{at_least, current_user, design_role, parse_role, personal_workspace};
use crate::js;
use crate::prelude::*;
use crate::routes::brand::notify;
use crate::routes::content::families_for;
use crate::routes::teams::{revoke_live, team_role_str};

pub const LIMITS: &[RouteLimit] = &[RouteLimit { method: "POST", path: "/api/designs/{id}/request-access", max: 5, window_ms: 3_600_000 }];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/fonts/manifest.json", get(fonts_manifest))
        .route("/api/fonts/preview/{file}", get(font_preview))
        .route("/api/fonts/files/{*rel}", get(font_file))
        .route("/api/templates", get(templates))
        .route("/api/templates/{id}/preview", get(template_preview))
        .route("/api/templates/{id}", get(template))
        .route("/api/templates/{id}/thumbnail.png", get(template_thumbnail))
        .route("/api/folders", get(folders).post(create_folder))
        .route("/api/folders/{id}", patch(update_folder).delete(delete_folder))
        .route("/api/designs/{id}/comments", get(comments).post(create_comment))
        .route("/api/comments/{id}", patch(update_comment).delete(delete_comment))
        .route("/api/comments/{id}/reactions", post(react))
        .route("/api/designs/{id}/collaborators", get(collaborators))
        .route("/api/designs/{id}/transfer", post(transfer))
        .route("/api/designs/{id}/request-access", post(request_access))
        .route("/api/designs/{id}/access-requests", get(access_requests))
        .route("/api/access-requests/{id}", post(decide_access))
        .route("/api/tasks", get(tasks))
}

/// JS truthiness of a JSON value (`undefined` = `None`).
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

fn forbidden() -> Reply {
    reply::json(403, json!({ "error": "forbidden" }))
}

// ---------------------------------------------------------------- fonts (pinned OFL files, ADR-0043)

/// The process-wide Google Fonts service (`@oc/gfonts` keeps its catalogue, in-flight fetches and previews in
/// module state).
#[derive(Default)]
struct Fonts(OnceLock<oc_gfonts::GoogleFonts>);

fn gfonts(s: &AppState) -> oc_gfonts::GoogleFonts {
    s.local::<Fonts>().0.get_or_init(|| oc_gfonts::GoogleFonts::new(s.paths.fonts.clone(), s.pool.clone(), s.store.clone())).clone()
}

async fn fonts_manifest(State(s): State<AppState>, req: Req) -> ApiResult {
    // Bundled families plus the whole Google Fonts catalogue (faces load on demand).
    let pinned = gfonts(&s).full_manifest().await.map_err(ApiError::internal)?;
    let mut v = serde_json::to_value(&pinned)?;
    let Some(u) = current_user(&s, &req).await? else {
        return Ok(Reply::json(v).header("cache-control", "public, max-age=300"));
    };
    // Signed in: add the brand fonts uploaded to the user's workspaces.
    let custom = families_for(&s, "SELECT f.* FROM custom_fonts f JOIN memberships m ON m.workspace_id = f.workspace_id WHERE m.user_id = $1 ORDER BY f.created_at", &[pg::uuid(u.id)]).await?;
    if let Some(Value::Array(f)) = v.get_mut("families") {
        f.extend(custom);
    }
    Ok(Reply::json(v).header("cache-control", "private, max-age=60"))
}

/// `GET /api/fonts/preview/:slug.ttf` (Fastify's `:slug.ttf`; any other last segment is not a route).
async fn font_preview(State(s): State<AppState>, Path(file): Path<String>, raw: Request) -> Response {
    let Some(slug) = file.strip_suffix(".ttf").map(str::to_owned) else {
        return crate::http::not_found(raw).await;
    };
    // The request still goes through `Req` (rate limit), like every Fastify route.
    if let Err(e) = <Req as axum::extract::FromRequest<AppState>>::from_request(raw, &s).await {
        return e.into_response();
    }
    match gfonts(&s).font_preview(&slug).await {
        Ok(bytes) => Reply::bytes("font/ttf", bytes.as_ref().clone()).header("cache-control", "public, max-age=2592000").into_response(),
        Err(_) => Reply::empty(404).into_response(),
    }
}

/// Node's `path.normalize` for a relative POSIX path.
fn normalize_rel(p: &str) -> String {
    let trailing = p.ends_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            s => out.push(s),
        }
    }
    let mut r = out.join("/");
    if r.is_empty() {
        r = ".".into();
    }
    if trailing && r != "." {
        r.push('/');
    }
    r
}

async fn font_file(State(s): State<AppState>, Path(rel): Path<String>, _req: Req) -> ApiResult {
    // `normalize(rel).replace(/^(\.\.(\/|\\|$))+/, '')`
    let mut rel = normalize_rel(&rel);
    loop {
        if let Some(r) = rel.strip_prefix("../").or_else(|| rel.strip_prefix("..\\")) {
            rel = r.to_owned();
        } else if rel == ".." {
            rel = String::new();
        } else {
            break;
        }
    }
    if rel.starts_with("gf/") {
        return Ok(match gfonts(&s).font_file(&rel).await {
            // Public font files, any origin without credentials: sandboxed published sites (opaque origin, SEC-02).
            Ok(bytes) => Reply::bytes("font/ttf", bytes.as_ref().clone()).header("cache-control", "public, max-age=31536000, immutable").header("access-control-allow-origin", "*"),
            Err(e) => {
                tracing::warn!(err = %e, rel, "google font");
                Reply::empty(404)
            }
        });
    }
    let base = s.paths.fonts.join("files");
    let file = base.join(&rel);
    if !file.starts_with(&base) || !file.exists() {
        return Ok(Reply::empty(404));
    }
    let bytes = tokio::fs::read(&file).await?;
    Ok(Reply::bytes("font/ttf", bytes).header("cache-control", "public, max-age=31536000, immutable").header("access-control-allow-origin", "*"))
}

// ---------------------------------------------------------------- templates

/// Public catalogue plus team (brand) templates of the viewer's workspaces (`$1` = viewer id or null).
const VISIBLE: &str = "(workspace_id IS NULL OR workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1))";
const SUMMARY_COLS: &str = r#"id, title, category, preset, tags, width, height, workspace_id IS NOT NULL AS team, brand, workspace_id AS "workspaceId", version"#;

async fn templates(State(s): State<AppState>, req: Req) -> ApiResult {
    let q = z::obj([
        ("q", z::str().optional()),
        ("category", z::str().optional()),
        ("preset", z::str().optional()),
        // Tag facet (g24): one tag, or several comma-separated (a template must carry all of them).
        ("tag", z::str().max(400.0).optional()),
        ("scope", z::enumeration(&["all", "team"]).default(json!("all"))),
    ])
    .parse(Some(&req.query))?;
    let viewer = current_user(&s, &req).await?.map(|u| u.id);
    let mut params: Vec<Arg> = vec![Arg::Uuid(viewer)];
    let team_only = "workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1)";
    let mut wh: Vec<String> = vec!["published".into(), if q["scope"] == "team" { team_only.into() } else { VISIBLE.into() }];
    let qs = |k: &str| q.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned);
    if let Some(t) = qs("q") {
        params.push(pg::text(t));
        let n = params.len();
        wh.push(format!("(to_tsvector('simple', search_text) @@ plainto_tsquery('simple', ${n}) OR search_text ILIKE '%' || ${n} || '%')"));
    }
    if let Some(c) = qs("category") {
        params.push(pg::text(c));
        wh.push(format!("category = ${}", params.len()));
    }
    if let Some(p) = qs("preset") {
        params.push(pg::text(p));
        wh.push(format!("preset = ${}", params.len()));
    }
    let tags: Vec<String> = q.get("tag").and_then(Value::as_str).unwrap_or("").split(',').map(|t| js::trim(t).to_owned()).filter(|t| !t.is_empty()).take(10).collect();
    if !tags.is_empty() {
        params.push(Arg::TextArray(Some(tags)));
        wh.push(format!("tags @> ${}::text[]", params.len()));
    }
    let wh = wh.join(" AND ");
    let templates = pg::json_rows(&s.pool, &format!("SELECT {SUMMARY_COLS} FROM templates WHERE {wh}\n       ORDER BY (workspace_id IS NOT NULL) DESC, category, title LIMIT 300"), &params).await?;
    let categories = pg::json_rows(
        &s.pool,
        &format!("SELECT category, count(*)::int AS n FROM templates WHERE published AND {VISIBLE} GROUP BY category ORDER BY category"),
        &[Arg::Uuid(viewer)],
    )
    .await?;
    // Tag facet over the current result set (search and category applied), most common first.
    let tag_facet = pg::json_rows(&s.pool, &format!("SELECT tag, count(*)::int AS n FROM templates, unnest(tags) AS tag WHERE {wh} GROUP BY tag ORDER BY n DESC, tag LIMIT 30"), &params).await?;
    Ok(Reply::json(json!({ "templates": templates, "categories": categories, "tags": tag_facet })))
}

/// Template details for the preview dialog (g25): metadata, page count, similar templates and, when usable,
/// the document.
async fn template_preview(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let viewer = current_user(&s, &req).await?.map(|u| u.id);
    let t = pg::json_one(
        &s.pool,
        r#"SELECT t.id, t.title, t.category, t.preset, t.tags, t.width, t.height, t.workspace_id IS NOT NULL AS team, t.brand, t.workspace_id AS "workspaceId",
              t.description, t.uses, u.name AS creator, CASE WHEN w.personal THEN NULL ELSE w.name END AS "teamName", t.doc,
              (t.workspace_id IS NULL OR t.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $2)) AS visible
       FROM templates t LEFT JOIN users u ON u.id = t.created_by LEFT JOIN workspaces w ON w.id = t.workspace_id
       WHERE t.id = $1 AND t.published"#,
        &[pg::text(&id), Arg::Uuid(viewer)],
    )
    .await?;
    let Some(Value::Object(mut t)) = t.filter(|t| t["visible"] == json!(true)) else { return Ok(not_found()) };
    let doc = t.remove("doc").unwrap_or(Value::Null);
    t.remove("visible");
    let pages_obj = doc.get("pages").and_then(Value::as_object).ok_or_else(|| ApiError::internal("TypeError: Cannot convert undefined or null to object"))?;
    let visible_pages = pages_obj.values().filter(|p| !truthy(p.get("hidden"))).count();
    let pages = if visible_pages == 0 { 1 } else { visible_pages };
    let tags: Vec<String> = t.get("tags").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    let similar = pg::json_rows(
        &s.pool,
        &format!(
            "SELECT {SUMMARY_COLS} FROM templates
       WHERE published AND {VISIBLE} AND id <> $2 AND (category = $3 OR tags && $4::text[])
       ORDER BY (SELECT count(*) FROM unnest(tags) x WHERE x = ANY($4::text[])) DESC, (category = $3) DESC, (preset = $5) DESC, title
       LIMIT 12"
        ),
        &[Arg::Uuid(viewer), pg::text(t["id"].as_str().unwrap_or("")), pg::text(t["category"].as_str().unwrap_or("")), Arg::TextArray(Some(tags)), pg::text(t["preset"].as_str().unwrap_or(""))],
    )
    .await?;
    t.insert("pages".into(), json!(pages));
    t.insert("surface".into(), doc.pointer("/meta/surface").cloned().unwrap_or(Value::Null));
    t.insert("unit".into(), doc.pointer("/meta/dims/unit").cloned().unwrap_or(Value::Null));
    Ok(Reply::json(json!({ "template": Value::Object(t), "doc": doc, "similar": similar })))
}

async fn template(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let t = pg::json_one(&s.pool, "SELECT id, title, category, preset, doc, workspace_id FROM templates WHERE id = $1 AND published", &[pg::text(&id)]).await?;
    let Some(t) = t else { return Ok(not_found()) };
    if let Some(ws) = t["workspace_id"].as_str() {
        let u = current_user(&s, &req).await?;
        let member = match &u {
            Some(u) => pg::json_one(&s.pool, "SELECT 1 FROM memberships WHERE workspace_id = $1::uuid AND user_id = $2", &[pg::text(ws), pg::uuid(u.id)]).await?.is_some(),
            None => false,
        };
        if !member {
            return Ok(not_found());
        }
    }
    Ok(Reply::json(json!({ "template": t })))
}

async fn template_thumbnail(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let viewer = current_user(&s, &req).await?;
    // Same rule as the preview: a published catalogue template, or a team template the viewer's team owns
    // (members also see their team's drafts); staff see every draft.
    let t: Option<(Option<String>, bool, Option<bool>)> = sqlx::query_as(
        "SELECT thumbnail, workspace_id IS NOT NULL AS team,
              CASE WHEN workspace_id IS NULL THEN published OR $3
                   ELSE $3 OR workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $2) END AS visible
       FROM templates WHERE id = $1",
    )
    .bind(&id)
    .bind(viewer.as_ref().map(|u| u.id))
    .bind(viewer.as_ref().is_some_and(|u| u.is_staff))
    .fetch_optional(&s.pool)
    .await?;
    if let Some((_, _, visible)) = &t
        && *visible != Some(true)
    {
        return Ok(Reply::empty(404));
    }
    // Team thumbnails must not land in shared caches.
    let cache = if t.as_ref().is_some_and(|t| t.1) { "private, max-age=3600" } else { "public, max-age=3600" };
    if let Some(th) = t.as_ref().and_then(|t| t.0.as_deref()).filter(|th| th.len() == 64 && th.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))) {
        let bytes = s.store.get(th).await?;
        return Ok(Reply::bytes("image/png", bytes).header("cache-control", cache));
    }
    let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    let file = s.paths.thumbs.join("templates").join(format!("{safe}.png"));
    if !file.exists() {
        return Ok(Reply::empty(404));
    }
    Ok(Reply::bytes("image/png", tokio::fs::read(&file).await?).header("cache-control", cache))
}

// ---------------------------------------------------------------- folders (projects)

/// My folders plus shared team folders (CO-042): a team folder is visible to every member. Folders nest through
/// parent_id (g17); `can_manage` / `can_add` / `can_file` tell the Projects page which actions to offer.
async fn folders(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let folders = pg::json_rows(
        &s.pool,
        "SELECT f.id, f.name, f.parent_id, f.created_at, f.workspace_id, f.owner_id, u.name AS owner_name, CASE WHEN w.personal THEN NULL ELSE w.name END AS team,
                (SELECT count(*)::int FROM designs d WHERE d.folder_id = f.id AND d.trashed_at IS NULL) AS designs,
                (SELECT count(*)::int FROM folders c WHERE c.parent_id = f.id) AS subfolders,
                (fs.user_id IS NOT NULL) AS starred,
                (f.owner_id = $1 OR (NOT w.personal AND m.role IN ('owner','admin'))) AS can_manage,
                (CASE WHEN w.personal THEN f.owner_id = $1 ELSE m.role IS NOT NULL AND m.role <> 'member' END) AS can_add,
                (CASE WHEN w.personal THEN f.owner_id = $1 ELSE m.role IS NOT NULL END) AS can_file
         FROM folders f JOIN workspaces w ON w.id = f.workspace_id JOIN users u ON u.id = f.owner_id
         LEFT JOIN memberships m ON m.workspace_id = f.workspace_id AND m.user_id = $1
         LEFT JOIN folder_stars fs ON fs.folder_id = f.id AND fs.user_id = $1
         WHERE (w.personal AND f.owner_id = $1) OR (NOT w.personal AND m.user_id IS NOT NULL)
         ORDER BY (NOT w.personal), w.name, f.name",
        &[pg::uuid(u.id)],
    )
    .await?;
    Ok(Reply::json(json!({ "folders": folders })))
}

async fn create_folder(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("name", z::str().trim().min(1.0).max(100.0)), ("parentId", z::str().uuid().nullable().optional()), ("workspaceId", z::str().uuid().optional())]).parse(req.body())?;
    let mut ws = b.get("workspaceId").and_then(Value::as_str).map(str::to_owned);
    let parent = b.get("parentId").and_then(Value::as_str).map(str::to_owned);
    if let Some(pid) = parent.as_deref() {
        // A subfolder lives in its parent's workspace, so it's visible to exactly the people who see the parent.
        let pid = Uuid::parse_str(pid).map_err(ApiError::internal)?;
        let Some(p) = crate::projects::folder_access(&s, u.id, pid).await? else { return Ok(reply::json(404, json!({ "error": "parent_not_found" }))) };
        if !p.add {
            return Ok(reply::json(403, json!({ "error": "forbidden", "message": "You can’t add folders to that folder." })));
        }
        if ws.as_deref().is_some_and(|w| w != p.workspace_id.to_string()) {
            return Ok(reply::json(400, json!({ "error": "cross_workspace" })));
        }
        ws = Some(p.workspace_id.to_string());
    } else if let Some(w) = ws.as_deref() {
        let r = team_role_str(&s, &u.id.to_string(), w).await?;
        if r.as_deref().is_none_or(|r| r == "member") {
            return Ok(reply::json(403, json!({ "error": "forbidden", "message": "Only designers and admins can add team folders." })));
        }
    }
    let ws = match ws {
        Some(w) => w,
        None => personal_workspace(&s, u.id).await?.to_string(),
    };
    let id = oc_db::uuid();
    pg::exec(
        &s.pool,
        "INSERT INTO folders(id, workspace_id, owner_id, name, parent_id) VALUES ($1,$2::uuid,$3,$4,$5::uuid)",
        &[pg::uuid(id), pg::text(ws), pg::uuid(u.id), pg::text(b["name"].as_str().unwrap_or("")), Arg::Text(parent)],
    )
    .await?;
    Ok(Reply::json(json!({ "id": id.to_string() })))
}

/// `z.object({ id: z.string().uuid() }).parse(req.params)`.
fn folder_id_param(id: &str) -> Result<Uuid, ApiError> {
    let v = z::obj([("id", z::str().uuid())]).parse(Some(&json!({ "id": id })))?;
    Uuid::parse_str(v["id"].as_str().unwrap_or("")).map_err(ApiError::internal)
}

/// Rename (owner or team owner/admin), move under another folder (`parentId`, null = top level) and star for me.
async fn update_folder(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let id = folder_id_param(&id)?;
    let b = z::obj([("name", z::str().trim().min(1.0).max(100.0).optional()), ("parentId", z::str().uuid().nullable().optional()), ("starred", z::bool().optional())]).parse(req.body())?;
    let Some(f) = crate::projects::folder_access(&s, u.id, id).await? else { return Ok(not_found()) };
    if (b.get("name").is_some() || b.get("parentId").is_some()) && !f.manage {
        return Ok(forbidden());
    }
    if let Some(p) = b.get("parentId") {
        let pid = p.as_str().and_then(|x| Uuid::parse_str(x).ok());
        if let Some(fail) = crate::projects::move_folder(&s, u.id, id, pid).await? {
            return Ok(fail.reply_with_message());
        }
    }
    if let Some(name) = b.get("name").and_then(Value::as_str) {
        pg::exec(&s.pool, "UPDATE folders SET name = $2 WHERE id = $1", &[pg::uuid(id), pg::text(name)]).await?;
    }
    match b.get("starred").and_then(Value::as_bool) {
        Some(true) => {
            pg::exec(&s.pool, "INSERT INTO folder_stars(user_id, folder_id) VALUES ($1,$2) ON CONFLICT DO NOTHING", &[pg::uuid(u.id), pg::uuid(id)]).await?;
        }
        Some(false) => {
            pg::exec(&s.pool, "DELETE FROM folder_stars WHERE user_id = $1 AND folder_id = $2", &[pg::uuid(u.id), pg::uuid(id)]).await?;
        }
        None => {}
    }
    Ok(Reply::json(json!({ "ok": true })))
}

/// Deletes the folder and its subfolders; the designs inside are kept (they move out of the folder).
async fn delete_folder(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let id = folder_id_param(&id)?;
    let Some(f) = crate::projects::folder_access(&s, u.id, id).await? else { return Ok(not_found()) };
    if !f.manage {
        return Ok(forbidden());
    }
    pg::exec(&s.pool, "DELETE FROM folders WHERE id = $1", &[pg::uuid(id)]).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- comments (Postgres, not the CRDT)

/// `notifyAssignee(designId, commentId, assigneeId, actor, body, dueOn)`.
async fn notify_assignee(s: &AppState, design_id: &str, comment_id: &str, assignee: Uuid, actor: &User, body: &str, due_on: Option<&str>) -> Result<(), ApiError> {
    if assignee == actor.id {
        return Ok(());
    }
    notify(s, assignee, "assign", design_id, actor.id, &format!("{}: {body}", actor.name)).await?;
    let a: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = $1").bind(assignee).fetch_optional(&s.pool).await?;
    let d: Option<String> = sqlx::query_scalar("SELECT title FROM designs WHERE id = $1").bind(design_id).fetch_optional(&s.pool).await?;
    if let Some(email) = a {
        let t = oc_mail::Template::Assign {
            actor: actor.name.clone(),
            title: d.unwrap_or_else(|| "a design".into()),
            body: js::take(body, 280),
            due: due_on.map(str::to_owned),
            url: format!("{}/design/{design_id}?comments=1&c={comment_id}", s.cfg.web_origin),
        };
        oc_mail::send_email(&s.pool, &email, &t, Some(assignee)).await?;
    }
    Ok(())
}

async fn comments(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    let comments = pg::json_rows(
        &s.pool,
        "SELECT c.id, c.parent_id, c.page_id, c.element_id, c.body, c.resolved, c.created_at, c.edited_at, c.due_on::text AS due_on,
                u.id AS author_id, u.name AS author, u.color AS author_color,
                a.id AS assignee_id, a.name AS assignee, a.color AS assignee_color,
                COALESCE((SELECT json_agg(json_build_object('emoji', r.emoji, 'count', r.n, 'mine', r.mine)) FROM (
                  SELECT emoji, count(*)::int AS n, bool_or(user_id = $2) AS mine FROM comment_reactions WHERE comment_id = c.id GROUP BY emoji ORDER BY min(created_at)) r), '[]') AS reactions
         FROM comments c JOIN users u ON u.id = c.author_id LEFT JOIN users a ON a.id = c.assignee_id
         WHERE c.design_id = $1 ORDER BY c.created_at",
        &[pg::text(&id), pg::uuid(u.id)],
    )
    .await?;
    Ok(Reply::json(json!({ "comments": comments })))
}

const DATE_RE: (&str, &str) = (r"^\d{4}-\d{2}-\d{2}$", r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$");

async fn create_comment(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([
        ("body", z::str().min(1.0).max(5000.0)),
        ("parentId", z::str().uuid().optional()),
        ("pageId", z::str().optional()),
        ("elementId", z::str().optional()),
        ("assigneeId", z::str().uuid().optional()),
        ("dueOn", z::str().regex(DATE_RE.0, DATE_RE.1).optional()),
    ])
    .parse(req.body())?;
    let opt = |k: &str| b.get(k).and_then(Value::as_str).map(str::to_owned);
    let body = b["body"].as_str().unwrap_or("").to_owned();
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::Comment) {
        return Ok(forbidden());
    }
    // Tasks can only be assigned to people who can open the design.
    let assignee = opt("assigneeId").and_then(|a| Uuid::parse_str(&a).ok());
    if let Some(a) = assignee
        && !at_least(design_role(&s, Some(a), &id).await?, Role::View)
    {
        return Ok(reply::json(400, json!({ "error": "bad_assignee", "message": "That person doesn’t have access to this design." })));
    }
    let cid = oc_db::uuid().to_string();
    pg::exec(
        &s.pool,
        "INSERT INTO comments(id, design_id, parent_id, page_id, element_id, author_id, body, assignee_id, due_on) VALUES ($1::uuid,$2,$3::uuid,$4,$5,$6,$7,$8::uuid,$9::date)",
        &[pg::text(&cid), pg::text(&id), Arg::Text(opt("parentId")), Arg::Text(opt("pageId")), Arg::Text(opt("elementId")), pg::uuid(u.id), pg::text(&body), Arg::Text(opt("assigneeId")), Arg::Text(opt("dueOn"))],
    )
    .await?;
    if let Some(a) = assignee {
        notify_assignee(&s, &id, &cid, a, &u, &body, opt("dueOn").as_deref()).await?;
    }
    {
        let (pool, id, cid, parent, body) = (s.pool.clone(), id.clone(), cid.clone(), opt("parentId"), body.clone());
        s.spawn(async move {
            let owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_id FROM designs WHERE id = $1").bind(&id).fetch_optional(&pool).await.ok().flatten();
            if let Some(o) = owner {
                let _ = oc_webhooks::emit_event(&pool, o, oc_webhooks::WebhookEvent::CommentCreated, &json!({ "designId": id, "commentId": cid, "parentId": parent, "body": body })).await;
            }
        });
    }
    // @mentions notify the named collaborators; otherwise the owner (and the parent's author) hear about it. The
    // owner comes first (a bare UNION has no order: which row came first depended on the Postgres plan).
    let people: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT p.id, p.name, p.email FROM (SELECT u.id, u.name, u.email FROM users u WHERE u.id = (SELECT owner_id FROM designs WHERE id = $1)
         UNION SELECT u.id, u.name, u.email FROM design_shares s JOIN users u ON u.id = s.user_id WHERE s.design_id = $1
         UNION SELECT u.id, u.name, u.email FROM users u WHERE u.id IN (SELECT gm.user_id FROM design_group_shares gs JOIN team_group_members gm ON gm.group_id = gs.group_id JOIN team_groups tg ON tg.id = gs.group_id JOIN memberships tm ON tm.workspace_id = tg.workspace_id AND tm.user_id = gm.user_id WHERE gs.design_id = $1)) p
         ORDER BY p.id = (SELECT owner_id FROM designs WHERE id = $1) DESC, p.id",
    )
    .bind(&id)
    .fetch_all(&s.pool)
    .await?;
    let lower = body.to_lowercase();
    let mentioned: Vec<&(Uuid, String, String)> = people
        .iter()
        .filter(|(_, name, email)| {
            let first = name.split(' ').next().unwrap_or("").to_lowercase();
            lower.contains(&format!("@{}", name.to_lowercase())) || lower.contains(&format!("@{}", email.to_lowercase())) || lower.contains(&format!("@{first}"))
        })
        .collect();
    let snippet = format!("{}: {body}", u.name);
    let d_title: Option<String> = sqlx::query_scalar("SELECT title FROM designs WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let d_title = d_title.unwrap_or_else(|| "a design".into());
    let link = format!("{}/design/{id}?comments=1", s.cfg.web_origin);
    let short = js::take(&body, 280);
    for (pid, _, email) in &mentioned {
        notify(&s, *pid, "mention", &id, u.id, &snippet).await?;
        if *pid != u.id {
            oc_mail::send_email(&s.pool, email, &oc_mail::Template::Mention { actor: u.name.clone(), title: d_title.clone(), body: short.clone(), url: link.clone() }, Some(*pid)).await?;
        }
    }
    if let Some(parent_id) = opt("parentId") {
        let parent: Option<Uuid> = sqlx::query_scalar("SELECT author_id FROM comments WHERE id = $1::uuid").bind(&parent_id).fetch_optional(&s.pool).await?;
        if let Some(pa) = parent
            && !mentioned.iter().any(|m| m.0 == pa)
        {
            notify(&s, pa, "reply", &id, u.id, &snippet).await?;
            let email: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = $1").bind(pa).fetch_optional(&s.pool).await?;
            if let Some(email) = email
                && pa != u.id
            {
                oc_mail::send_email(&s.pool, &email, &oc_mail::Template::Reply { actor: u.name.clone(), title: d_title.clone(), body: short.clone(), url: link.clone() }, Some(pa)).await?;
            }
        }
    }
    if let Some((oid, _, oemail)) = people.first()
        && !mentioned.iter().any(|m| m.0 == *oid)
    {
        notify(&s, *oid, "comment", &id, u.id, &snippet).await?;
        if *oid != u.id {
            oc_mail::send_email(&s.pool, oemail, &oc_mail::Template::Comment { actor: u.name.clone(), title: d_title.clone(), body: short.clone(), url: link.clone() }, Some(*oid)).await?;
        }
    }
    Ok(Reply::json(json!({ "id": cid })))
}

async fn update_comment(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([
        ("resolved", z::bool().optional()),
        ("body", z::str().min(1.0).max(5000.0).optional()),
        ("assigneeId", z::str().uuid().nullable().optional()),
        ("dueOn", z::str().regex(DATE_RE.0, DATE_RE.1).nullable().optional()),
    ])
    .parse(req.body())?;
    let c: Option<(String, Uuid, String, Option<Uuid>)> =
        sqlx::query_as("SELECT design_id, author_id, body, assignee_id FROM comments WHERE id = $1::uuid").bind(&id).fetch_optional(&s.pool).await?;
    let Some((design_id, author_id, old_body, old_assignee)) = c else { return Ok(not_found()) };
    if !at_least(design_role(&s, Some(u.id), &design_id).await?, Role::Comment) {
        return Ok(not_found());
    }
    let body = b.get("body").and_then(Value::as_str).map(str::to_owned);
    if body.is_some() && author_id != u.id {
        return Ok(reply::json(403, json!({ "error": "forbidden", "message": "Only the author can edit a comment." })));
    }
    let assignee_given = b.get("assigneeId").is_some();
    let assignee = b.get("assigneeId").and_then(Value::as_str).map(str::to_owned);
    let assignee_uuid = assignee.as_deref().and_then(|a| Uuid::parse_str(a).ok());
    if let Some(a) = assignee_uuid
        && !at_least(design_role(&s, Some(a), &design_id).await?, Role::View)
    {
        return Ok(reply::json(400, json!({ "error": "bad_assignee", "message": "That person doesn’t have access to this design." })));
    }
    let due_given = b.get("dueOn").is_some();
    let due = b.get("dueOn").and_then(Value::as_str).map(str::to_owned);
    pg::exec(
        &s.pool,
        "UPDATE comments SET
           resolved = COALESCE($2, resolved), resolved_at = CASE WHEN $2::boolean IS TRUE THEN now() WHEN $2::boolean IS FALSE THEN NULL ELSE resolved_at END,
           body = COALESCE($3, body), edited_at = CASE WHEN $3::text IS NOT NULL THEN now() ELSE edited_at END,
           assignee_id = CASE WHEN $4::boolean THEN $5::uuid ELSE assignee_id END,
           due_on = CASE WHEN $6::boolean THEN $7::date ELSE due_on END
         WHERE id = $1::uuid",
        &[pg::text(&id), Arg::Bool(b.get("resolved").and_then(Value::as_bool)), Arg::Text(body.clone()), Arg::Bool(Some(assignee_given)), Arg::Text(assignee), Arg::Bool(Some(due_given)), Arg::Text(due.clone())],
    )
    .await?;
    if let Some(a) = assignee_uuid
        && Some(a) != old_assignee
    {
        notify_assignee(&s, &design_id, &id, a, &u, body.as_deref().unwrap_or(&old_body), due.as_deref()).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}

async fn react(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("emoji", z::str().min(1.0).max(16.0))]).parse(req.body())?;
    let emoji = b["emoji"].as_str().unwrap_or("").to_owned();
    let c: Option<String> = sqlx::query_scalar("SELECT design_id FROM comments WHERE id = $1::uuid").bind(&id).fetch_optional(&s.pool).await?;
    let Some(design_id) = c else { return Ok(not_found()) };
    if !at_least(design_role(&s, Some(u.id), &design_id).await?, Role::Comment) {
        return Ok(not_found());
    }
    // Toggle.
    let del = pg::exec(&s.pool, "DELETE FROM comment_reactions WHERE comment_id = $1::uuid AND user_id = $2 AND emoji = $3", &[pg::text(&id), pg::uuid(u.id), pg::text(&emoji)]).await?;
    if del == 0 {
        pg::exec(&s.pool, "INSERT INTO comment_reactions(comment_id, user_id, emoji) VALUES ($1::uuid,$2,$3) ON CONFLICT DO NOTHING", &[pg::text(&id), pg::uuid(u.id), pg::text(&emoji)]).await?;
    }
    Ok(Reply::json(json!({ "ok": true, "on": del == 0 })))
}

/// People a task can be assigned to: owner, people shared with, and team members.
async fn collaborators(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(not_found());
    }
    let people = pg::json_rows(
        &s.pool,
        "SELECT DISTINCT u.id, u.name, u.email, u.color FROM users u WHERE u.deleted_at IS NULL AND (
             u.id = (SELECT owner_id FROM designs WHERE id = $1)
             OR u.id IN (SELECT user_id FROM design_shares WHERE design_id = $1)
             OR u.id IN (SELECT gm.user_id FROM design_group_shares gs JOIN team_group_members gm ON gm.group_id = gs.group_id JOIN team_groups tg ON tg.id = gs.group_id JOIN memberships tm ON tm.workspace_id = tg.workspace_id AND tm.user_id = gm.user_id WHERE gs.design_id = $1)
             OR u.id IN (SELECT m.user_id FROM memberships m JOIN designs d ON d.workspace_id = m.workspace_id JOIN workspaces w ON w.id = m.workspace_id WHERE d.id = $1 AND NOT w.personal))
           ORDER BY u.name",
        &[pg::text(&id)],
    )
    .await?;
    Ok(Reply::json(json!({ "people": people })))
}

// ---------------------------------------------------------------- ownership & access requests (CO-043)

async fn transfer(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("userId", z::str().uuid())]).parse(req.body())?;
    let user_id = b["userId"].as_str().unwrap_or("").to_owned();
    if design_role(&s, Some(u.id), &id).await? != Some(Role::Owner) {
        return Ok(forbidden());
    }
    let target = Uuid::parse_str(&user_id).map_err(ApiError::internal)?;
    if !at_least(design_role(&s, Some(target), &id).await?, Role::View) {
        return Ok(reply::json(400, json!({ "error": "bad_user", "message": "Share the design with them first." })));
    }
    let mut t = s.pool.begin().await?;
    sqlx::query("UPDATE designs SET owner_id = $2::uuid, folder_id = NULL WHERE id = $1").bind(&id).bind(&user_id).execute(&mut *t).await?;
    sqlx::query("DELETE FROM design_shares WHERE design_id = $1 AND user_id = $2::uuid").bind(&id).bind(&user_id).execute(&mut *t).await?;
    // The previous owner keeps edit access.
    sqlx::query("INSERT INTO design_shares(design_id, user_id, role) VALUES ($1,$2,'edit') ON CONFLICT (design_id, user_id) DO UPDATE SET role = 'edit'").bind(&id).bind(u.id).execute(&mut *t).await?;
    t.commit().await?;
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: Some(u.id), action: "design.transfer".into(), target: Some(id.clone()), meta: Some(json!({ "to": user_id })), ip: Some(req.ip.clone()), ..Default::default() },
        s.now(),
    )
    .await?;
    revoke_live(&s, Some(&id), None, None).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn request_access(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("role", z::enumeration(&["view", "comment", "edit"]).default(json!("edit"))), ("message", z::str().max(500.0).optional())]).parse(Some(&req.body_or_empty()))?;
    let role = b["role"].as_str().unwrap_or("edit").to_owned();
    let message = b.get("message").and_then(Value::as_str).map(str::to_owned);
    let d: Option<(Uuid, String)> = sqlx::query_as("SELECT owner_id, title FROM designs WHERE id = $1 AND trashed_at IS NULL").bind(&id).fetch_optional(&s.pool).await?;
    let Some((owner_id, title)) = d else { return Ok(not_found()) };
    if at_least(design_role(&s, Some(u.id), &id).await?, parse_role(&role).unwrap_or(Role::Edit)) {
        return Ok(Reply::json(json!({ "ok": true, "already": true })));
    }
    pg::exec(
        &s.pool,
        "INSERT INTO access_requests(id, design_id, user_id, role, message) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (design_id, user_id) WHERE status = 'pending' DO UPDATE SET role = EXCLUDED.role, message = EXCLUDED.message",
        &[pg::uuid(oc_db::uuid()), pg::text(&id), pg::uuid(u.id), pg::text(&role), Arg::Text(message.clone())],
    )
    .await?;
    let msg = message.as_deref().filter(|m| !m.is_empty());
    notify(&s, owner_id, "share", &id, u.id, &format!("{} asked for {role} access{}", u.name, msg.map(|m| format!(": {m}")).unwrap_or_default())).await?;
    let owner: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = $1").bind(owner_id).fetch_optional(&s.pool).await?;
    if let Some(email) = owner {
        let t = oc_mail::Template::Mention {
            actor: u.name.clone(),
            title,
            body: format!("{} ({}) asked for {role} access.{}", u.name, u.email, msg.map(|m| format!(" “{m}”")).unwrap_or_default()),
            url: format!("{}/design/{id}?share=1", s.cfg.web_origin),
        };
        oc_mail::send_email(&s.pool, &email, &t, Some(owner_id)).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}

async fn access_requests(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::Edit) {
        return Ok(forbidden());
    }
    let requests = pg::json_rows(
        &s.pool,
        "SELECT r.id, r.role, r.message, r.created_at, u.id AS user_id, u.name, u.email, u.color FROM access_requests r JOIN users u ON u.id = r.user_id WHERE r.design_id = $1 AND r.status = 'pending' ORDER BY r.created_at",
        &[pg::text(&id)],
    )
    .await?;
    Ok(Reply::json(json!({ "requests": requests })))
}

async fn decide_access(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("approve", z::bool()), ("role", z::enumeration(&["view", "comment", "edit"]).optional())]).parse(req.body())?;
    let r: Option<(String, Uuid, String)> = sqlx::query_as("SELECT design_id, user_id, role FROM access_requests WHERE id = $1::uuid AND status = 'pending'").bind(&id).fetch_optional(&s.pool).await?;
    let Some((design_id, user_id, asked)) = r else { return Ok(not_found()) };
    if !at_least(design_role(&s, Some(u.id), &design_id).await?, Role::Edit) {
        return Ok(not_found());
    }
    let approve = b["approve"].as_bool().unwrap_or(false);
    pg::exec(&s.pool, "UPDATE access_requests SET status = $2, decided_at = now() WHERE id = $1::uuid", &[pg::text(&id), pg::text(if approve { "approved" } else { "declined" })]).await?;
    if approve {
        let grant = b.get("role").and_then(Value::as_str).map_or(asked, str::to_owned);
        pg::exec(
            &s.pool,
            "INSERT INTO design_shares(design_id, user_id, role) VALUES ($1,$2,$3) ON CONFLICT (design_id, user_id) DO UPDATE SET role = EXCLUDED.role",
            &[pg::text(&design_id), pg::uuid(user_id), pg::text(&grant)],
        )
        .await?;
        notify(&s, user_id, "share", &design_id, u.id, &format!("{} gave you {grant} access", u.name)).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}

/// My open tasks across designs.
async fn tasks(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let tasks = pg::json_rows(
        &s.pool,
        "SELECT c.id, c.body, c.due_on::text AS due_on, c.created_at, c.design_id, d.title AS design_title, u.name AS author, u.color AS author_color
         FROM comments c JOIN designs d ON d.id = c.design_id AND d.trashed_at IS NULL JOIN users u ON u.id = c.author_id
         WHERE c.assignee_id = $1 AND NOT c.resolved ORDER BY c.due_on NULLS LAST, c.created_at",
        &[pg::uuid(u.id)],
    )
    .await?;
    Ok(Reply::json(json!({ "tasks": tasks })))
}

async fn delete_comment(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let n = pg::exec(&s.pool, "DELETE FROM comments WHERE id = $1::uuid AND author_id = $2", &[pg::text(&id), pg::uuid(u.id)]).await?;
    Ok(if n > 0 { Reply::json(json!({ "ok": true })) } else { not_found() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_like_node() {
        assert_eq!(normalize_rel("a/./b//c"), "a/b/c");
        assert_eq!(normalize_rel("a/../../x"), "../x");
        assert_eq!(normalize_rel("../../etc/passwd"), "../../etc/passwd");
        assert_eq!(normalize_rel(""), ".");
        assert_eq!(normalize_rel("a/"), "a/");
    }
}
