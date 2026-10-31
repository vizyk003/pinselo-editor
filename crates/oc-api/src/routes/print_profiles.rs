//! Port of services/api/src/routes/printProfiles.ts — print colour profiles: the bundled ICC profiles
//! (crates/oc-color/icc-profiles.json, read through `oc_color::registry`) plus CMYK output profiles uploaded
//! per user or team, the gamut check, and resolving `print.profile` of export requests. ICC parsing and colour
//! conversion live in the render service (`/internal/icc/inspect`, `/internal/print/gamut`); this module stores
//! profiles and checks access. Every route runs `requireUser` first.
//!
//! Also the zod schemas of packages/schema/src/iccProfiles.ts (`printExportOptions`, `printGamutRequest`) and
//! `zColor` of validate.ts, which the export routes of designs, media and v1 share.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

use serde_json::Map;

use crate::brandhub::is_admin_role;
use crate::http::jsjson;
use crate::http::req::Body;
use crate::prelude::*;
use crate::zod::Schema;

pub const LIMITS: &[RouteLimit] = &[];

/// `MAX_ICC_BYTES`.
pub const MAX_ICC_BYTES: usize = 10 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/print/profiles", get(list).post(upload))
        .route("/api/print/profiles/{id}", delete(remove))
        .route("/api/print/gamut", post(gamut))
}

// ---------------------------------------------------------------- schemas (packages/schema/src/iccProfiles.ts)

/// `PRINT_STANDARDS`.
pub const PRINT_STANDARDS: [&str; 3] = ["none", "pdfx4", "pdfx1a"];
/// `RENDERING_INTENTS`.
pub const RENDERING_INTENTS: [&str; 2] = ["relative", "perceptual"];

/// `zIccProfileRef`: a bundled id, or `{ id }` of an uploaded profile.
pub fn z_icc_profile_ref() -> Schema {
    let ids: Vec<&str> = oc_color::registry::bundled().iter().map(|p| p.id.as_str()).collect();
    z::union(vec![z::enumeration(&ids), z::obj([("id", z::str().uuid())])])
}

/// `printExportOptions`: the `print` field of export requests.
pub fn print_export_options() -> Schema {
    z::obj([
        ("bleedMm", z::num().min(0.0).max(10.0)),
        ("cropMarks", z::bool()),
        ("registrationMarks", z::bool().optional()),
        ("colorBars", z::bool().optional()),
        ("slugMm", z::num().min(0.0).max(20.0).optional()),
        ("standard", z::enumeration(&PRINT_STANDARDS).optional()),
        ("flatten", z::bool().optional()),
        ("profile", z_icc_profile_ref().optional()),
        ("intent", z::enumeration(&RENDERING_INTENTS).optional()),
    ])
}

/// `zColor` (validate.ts; its `num` refines `Number.isFinite`, which every JSON number is).
pub fn z_color() -> Schema {
    z::union(vec![
        z::loose_obj([("space", z::enumeration(&["srgb", "display-p3"])), ("r", z::num()), ("g", z::num()), ("b", z::num()), ("a", z::num())]),
        z::loose_obj([("space", z::literal(json!("cmyk"))), ("c", z::num()), ("m", z::num()), ("y", z::num()), ("k", z::num()), ("a", z::num())]),
        z::loose_obj([("space", z::literal(json!("spot"))), ("swatch", z::str()), ("tint", z::num()), ("a", z::num())]),
    ])
}

/// `printGamutRequest`.
pub fn print_gamut_request() -> Schema {
    z::obj([("profile", z_icc_profile_ref()), ("intent", z::enumeration(&RENDERING_INTENTS).optional()), ("colors", z::array(z_color()).min(1.0).max(512.0))])
}

// ---------------------------------------------------------------- bundled profiles

/// `iccDir()`: where `pnpm icc:sync` puts the bundled CMYK profiles (`OC_ICC_DIR` overrides).
pub fn icc_dir(s: &AppState) -> PathBuf {
    s.env_nonempty("OC_ICC_DIR").map_or_else(|| s.paths.root.join("fixtures/icc"), PathBuf::from)
}

/// `bundledProfiles()`.
pub fn bundled_profiles(s: &AppState) -> Vec<Value> {
    let dir = icc_dir(s);
    let default = oc_color::registry::default_id();
    oc_color::registry::bundled()
        .iter()
        .map(|p| {
            json!({
                "id": p.id, "name": p.name, "description": p.description, "kind": "bundled", "colorSpace": p.color_space, "conditionId": p.condition_id, "paper": p.paper,
                "default": p.id == default, "available": p.builtin || dir.join(&p.file).exists(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------- uploaded profiles

const COLS: &str = "p.id, p.user_id, p.team_id, p.name, p.description, p.color_space, p.condition_id, p.sha, p.size, p.created_at";

static UUID_RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").expect("uuid regex"));

fn custom_view(r: &Value, can_delete: bool) -> Value {
    json!({
        "id": r["id"], "name": r["name"], "description": r["description"], "kind": "custom", "colorSpace": r["color_space"], "conditionId": r["condition_id"],
        "teamId": r["team_id"], "size": r["size"], "createdAt": r["created_at"], "canDelete": can_delete,
    })
}

/// `usableProfile(userId, id)`: an uploaded profile the user may use (their own, or one shared with a team they
/// belong to).
pub async fn usable_profile(s: &AppState, user: Uuid, id: &str) -> Result<Option<Value>, ApiError> {
    if !UUID_RE.is_match(id) {
        return Ok(None);
    }
    Ok(pg::json_one(
        &s.pool,
        &format!("SELECT {COLS} FROM icc_profiles p WHERE p.id = $1::uuid AND (p.user_id = $2 OR p.team_id IN (SELECT workspace_id FROM memberships WHERE user_id = $2))"),
        &[pg::text(id), pg::uuid(user)],
    )
    .await?)
}

/// `resolveProfileRef(userId, ref)`: `profile` as the render service takes it; `None` = unknown or not
/// accessible (→ 404).
pub async fn resolve_profile_ref(s: &AppState, user: Uuid, r: &Value) -> Result<Option<Value>, ApiError> {
    if r.is_string() {
        return Ok(Some(r.clone()));
    }
    let Some(id) = r.get("id").and_then(Value::as_str) else { return Ok(None) };
    Ok(usable_profile(s, user, id).await?.map(|p| json!({ "asset": p["sha"], "name": p["name"], "conditionId": p["condition_id"] })))
}

/// `resolvePrintOptions(userId, print)`: the `{ print? }` to spread into an export payload, with an uploaded
/// profile resolved to its stored object; `None` = profile not found (→ [`profile_not_found`]).
pub async fn resolve_print_options(s: &AppState, user: Uuid, print: Option<&Value>) -> Result<Option<Map<String, Value>>, ApiError> {
    let mut out = Map::new();
    let Some(print) = print else { return Ok(Some(out)) };
    let mut p = print.clone();
    if let Some(r) = print.get("profile") {
        let Some(profile) = resolve_profile_ref(s, user, r).await? else { return Ok(None) };
        p["profile"] = profile;
    }
    out.insert("print".into(), p);
    Ok(Some(out))
}

/// `reply.code(404).send(PROFILE_NOT_FOUND)`.
pub fn profile_not_found() -> Reply {
    reply::json(404, json!({ "error": "profile_not_found", "message": "That colour profile does not exist or is not shared with you." }))
}

// ---------------------------------------------------------------- render service

/// `InspectResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum Inspect {
    Ok { color_space: String, description: String, version: String },
    Err { status: u16, error: &'static str, message: String },
}

/// JS truthiness of a parsed JSON body (`!body`).
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// JS `String(v)` for the primitive values a render service answer can carry.
fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => oc_js::number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        Value::Array(a) => a.iter().map(|x| if x.is_null() { String::new() } else { js_string(x) }).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

fn render_unavailable(status: Option<u16>) -> String {
    status.map_or_else(|| "render service is not running".into(), |st| format!("render service answered {st}"))
}

/// The mapping of `inspectIcc`: `answer` is the render service's (status, `res.json()` or `None`), `None` when
/// the request failed. Only CMYK output-class (`prtr`, 4 channels) profiles are accepted.
pub fn inspect_result(answer: Option<(u16, Option<Value>)>) -> Inspect {
    let Some((status, body)) = answer else { return Inspect::Err { status: 502, error: "render_unavailable", message: render_unavailable(None) } };
    if status == 422 {
        let message = body.as_ref().and_then(|b| b.get("message")).and_then(Value::as_str).unwrap_or("That is not a valid ICC profile.").to_owned();
        return Inspect::Err { status: 422, error: "invalid_icc", message };
    }
    let ok = (200..300).contains(&status);
    let Some(body) = body.filter(|b| ok && truthy(b) && b.get("ok") == Some(&Value::Bool(true))) else {
        return Inspect::Err { status: 502, error: "render_unavailable", message: render_unavailable(Some(status)) };
    };
    if body.get("channels").and_then(Value::as_f64) != Some(4.0) || body.get("class").and_then(Value::as_str) != Some("prtr") {
        return Inspect::Err {
            status: 422,
            error: "unsupported_profile",
            message: "Upload a CMYK output (printer) profile. RGB, display and device-link profiles cannot be used for print export.".into(),
        };
    }
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    Inspect::Ok {
        color_space: body.get("colorSpace").filter(|v| !v.is_null()).map_or_else(|| "CMYK".into(), js_string),
        description: text("description"),
        version: text("version"),
    }
}

/// The mapping of `proxyGamut`: (status, body) to send back.
pub fn gamut_result(answer: Option<(u16, Option<Value>)>) -> (u16, Value) {
    let Some((status, body)) = answer else { return (502, json!({ "error": "render_unavailable", "message": render_unavailable(None) })) };
    match body.filter(truthy) {
        Some(b) if (200..300).contains(&status) => (200, b),
        // The render service's own client errors (e.g. a bundled profile that is not synced) are passed through.
        Some(b) if (400..500).contains(&status) => (status, b),
        _ => (502, json!({ "error": "render_unavailable", "message": render_unavailable(Some(status)) })),
    }
}

/// POST to the render service: (status, `res.json()` or `None`), `None` when it cannot be reached.
async fn render_post(s: &AppState, path: &str, content_type: &str, body: Vec<u8>) -> Option<(u16, Option<Value>)> {
    let res = s
        .http
        .post(format!("{}{path}", s.cfg.render_url))
        .header("content-type", content_type)
        .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&s.cfg.secret))
        .body(body)
        .send()
        .await
        .ok()?;
    let status = res.status().as_u16();
    let text = res.text().await.ok();
    Some((status, text.and_then(|t| jsjson::parse(&t).ok())))
}

/// `inspectIcc(bytes)`.
pub async fn inspect_icc(s: &AppState, bytes: &[u8]) -> Inspect {
    inspect_result(render_post(s, "/internal/icc/inspect", "application/vnd.iccprofile", bytes.to_vec()).await)
}

/// `proxyGamut(payload)`.
pub async fn proxy_gamut(s: &AppState, payload: &Value) -> (u16, Value) {
    gamut_result(render_post(s, "/internal/print/gamut", "application/json", jsjson::stringify(payload).into_bytes()).await)
}

// ---------------------------------------------------------------- access

/// `teamRoleOf(userId, teamId)`: the role in a team (not a personal workspace).
async fn team_role_of(s: &AppState, user: Uuid, team: &str) -> Result<Option<String>, ApiError> {
    Ok(sqlx::query_scalar("SELECT m.role FROM memberships m JOIN workspaces w ON w.id = m.workspace_id WHERE m.workspace_id = $1::uuid AND m.user_id = $2 AND NOT w.personal")
        .bind(team)
        .bind(user)
        .fetch_optional(&s.pool)
        .await?)
}

/// `canDelete(userId, p, roles?)`: owner of a personal profile; for a team profile its uploader or a team
/// owner/admin.
async fn can_delete(s: &AppState, user: Uuid, p: &Value, roles: Option<&HashMap<String, String>>) -> Result<bool, ApiError> {
    if p["user_id"].as_str() == Some(user.to_string().as_str()) {
        return Ok(true);
    }
    let Some(team) = p["team_id"].as_str() else { return Ok(false) };
    let role = match roles {
        Some(r) => r.get(team).cloned(),
        None => team_role_of(s, user, team).await?,
    };
    Ok(is_admin_role(role.as_deref()))
}

async fn audit(s: &AppState, actor: Uuid, workspace: Option<&str>, action: &str, target: &str, meta: Option<Value>, ip: &str) -> Result<(), ApiError> {
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent {
            actor_id: Some(actor),
            workspace_id: workspace.and_then(|w| Uuid::parse_str(w).ok()),
            action: action.into(),
            target: Some(target.to_owned()),
            meta,
            ip: Some(ip.to_owned()),
        },
        s.now(),
    )
    .await?;
    Ok(())
}

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

// ---------------------------------------------------------------- multipart

/// What `for await (const part of req.parts({ limits: { fileSize } }))` collects: the (last) file and the fields.
struct Parts {
    file: Option<(String, Vec<u8>)>,
    fields: HashMap<String, String>,
}

/// `req.parts(...)`; a file over `limit` is `Err(413 too_large)` (`toBuffer()` throws `FST_REQ_FILE_TOO_LARGE`).
async fn read_parts(req: &mut Req, limit: usize) -> Result<Result<Parts, Reply>, ApiError> {
    let Body::Multipart { content_type, body } = std::mem::replace(&mut req.body, Body::None) else {
        return Err(ApiError::status(406, "the request is not multipart"));
    };
    let Some(body) = body.into_inner().ok().flatten() else {
        return Err(ApiError::status(406, "the request is not multipart"));
    };
    let boundary = multer::parse_boundary(&content_type).map_err(|_| ApiError::status(406, "the request is not multipart"))?;
    let mut mp = multer::Multipart::new(body.into_data_stream(), boundary);
    let mut parts = Parts { file: None, fields: HashMap::new() };
    loop {
        let mut field = match mp.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => return Ok(Ok(parts)),
            Err(e) => return Err(ApiError::status(400, e.to_string())),
        };
        let name = field.name().unwrap_or("").to_owned();
        if let Some(filename) = field.file_name().map(str::to_owned) {
            let mut bytes = Vec::new();
            while let Some(chunk) = field.chunk().await.map_err(|e| ApiError::status(400, e.to_string()))? {
                if bytes.len() + chunk.len() > limit {
                    return Ok(Err(reply::json(413, json!({ "error": "too_large", "message": "Colour profiles can be up to 10 MB." }))));
                }
                bytes.extend_from_slice(&chunk);
            }
            parts.file = Some((filename, bytes));
        } else {
            let value = field.text().await.map_err(|e| ApiError::status(400, e.to_string()))?;
            parts.fields.insert(name, value);
        }
    }
}

// ---------------------------------------------------------------- routes

async fn list(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let q = z::obj([("teamId", z::str().uuid().optional())]).parse(Some(&req.query))?;
    let team = q.get("teamId").and_then(Value::as_str);
    if let Some(t) = team
        && team_role_of(&s, u.id, t).await?.is_none()
    {
        return Ok(not_found());
    }
    let rows = pg::json_rows(
        &s.pool,
        &format!(
            "SELECT {COLS} FROM icc_profiles p
       WHERE ((p.team_id IS NULL AND p.user_id = $1) OR p.team_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1))
         AND ($2::uuid IS NULL OR p.team_id IS NULL OR p.team_id = $2::uuid)
       ORDER BY p.team_id IS NOT NULL, p.created_at, p.id"
        ),
        &[pg::uuid(u.id), pg::opt_text(team)],
    )
    .await?;
    let roles: HashMap<String, String> = sqlx::query_as::<_, (Uuid, String)>("SELECT workspace_id, role FROM memberships WHERE user_id = $1")
        .bind(u.id)
        .fetch_all(&s.pool)
        .await?
        .into_iter()
        .map(|(w, r)| (w.to_string(), r))
        .collect();
    let mut profiles = bundled_profiles(&s);
    for r in &rows {
        profiles.push(custom_view(r, can_delete(&s, u.id, r, Some(&roles)).await?));
    }
    Ok(Reply::json(json!({ "profiles": profiles })))
}

async fn upload(State(s): State<AppState>, mut req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let parts = match read_parts(&mut req, MAX_ICC_BYTES).await? {
        Ok(p) => p,
        Err(too_large) => return Ok(too_large),
    };
    let Some((filename, bytes)) = parts.file.filter(|(_, b)| !b.is_empty()) else { return Ok(reply::json(400, json!({ "error": "no_file" }))) };
    let mut input = Map::new();
    for k in ["name", "teamId", "conditionId"] {
        if let Some(v) = parts.fields.get(k).filter(|v| !v.is_empty()) {
            input.insert(k.into(), json!(v));
        }
    }
    let meta = z::obj([("name", z::str().trim().max(120.0).optional()), ("teamId", z::str().uuid().optional()), ("conditionId", z::str().trim().max(80.0).optional())])
        .parse(Some(&Value::Object(input)))?;
    let team = meta.get("teamId").and_then(Value::as_str);
    if let Some(t) = team {
        match team_role_of(&s, u.id, t).await?.as_deref() {
            None => return Ok(not_found()),
            Some("member") => return Ok(reply::json(403, json!({ "error": "forbidden", "message": "Only team owners, admins and designers can add colour profiles." }))),
            Some(_) => {}
        }
    }
    let description = match inspect_icc(&s, &bytes).await {
        Inspect::Err { status, error, message } => return Ok(reply::json(status, json!({ "error": error, "message": message }))),
        Inspect::Ok { description, .. } => description,
    };
    let sha = s.store.put(&bytes, Some("application/vnd.iccprofile")).await?;
    let id = oc_db::uuid().to_string();
    let base = crate::js::take(&ICC_EXT.replace(&filename, ""), 120);
    let name = [meta.get("name").and_then(Value::as_str).unwrap_or("").to_owned(), crate::js::take(crate::js::trim(&description), 120), base]
        .into_iter()
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| "Custom profile".into());
    let row = pg::json_one(
        &s.pool,
        "INSERT INTO icc_profiles(id, user_id, team_id, name, description, color_space, condition_id, sha, size) VALUES ($1::uuid,$2,$3::uuid,$4,$5,'CMYK',$6,$7,$8)
       RETURNING id, user_id, team_id, name, description, color_space, condition_id, sha, size, created_at",
        &[
            pg::text(&id),
            pg::uuid(u.id),
            pg::opt_text(team),
            pg::text(&name),
            pg::text(crate::js::take(&description, 500)),
            pg::opt_text(meta.get("conditionId").and_then(Value::as_str)),
            pg::text(&sha),
            Arg::Int4(Some(bytes.len() as i32)),
        ],
    )
    .await?
    .ok_or_else(|| ApiError::internal("insert returned no row"))?;
    audit(&s, u.id, team, "print.profile_uploaded", &id, Some(json!({ "name": name, "size": bytes.len() })), &req.ip).await?;
    Ok(Reply::json(json!({ "profile": custom_view(&row, true) })).status(201))
}

static ICC_EXT: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)\.(icc|icm)$").expect("ext regex"));

async fn remove(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(p) = usable_profile(&s, u.id, &id).await? else { return Ok(not_found()) };
    if !can_delete(&s, u.id, &p, None).await? {
        return Ok(not_found());
    }
    pg::exec(&s.pool, "DELETE FROM icc_profiles WHERE id = $1::uuid", &[pg::text(&id)]).await?;
    audit(&s, u.id, p["team_id"].as_str(), "print.profile_deleted", &id, None, &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn gamut(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = print_gamut_request().parse(req.body())?;
    let Some(profile) = resolve_profile_ref(&s, u.id, &b["profile"]).await? else { return Ok(profile_not_found()) };
    let intent = b.get("intent").cloned().unwrap_or_else(|| json!("relative"));
    let (status, body) = proxy_gamut(&s, &json!({ "profile": profile, "intent": intent, "colors": b["colors"] })).await;
    Ok(Reply::json(body).status(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issues(s: &Schema, v: Value) -> String {
        jsjson::stringify(&Value::Array(s.safe_parse(Some(&v)).unwrap_err()))
    }

    #[test]
    fn inspect_mapping() {
        let cmyk = json!({ "ok": true, "colorSpace": "CMYK", "channels": 4, "class": "prtr", "description": "Coated FOGRA39", "version": "2.1.0" });
        assert_eq!(inspect_result(Some((200, Some(cmyk.clone())))), Inspect::Ok { color_space: "CMYK".into(), description: "Coated FOGRA39".into(), version: "2.1.0".into() });
        assert_eq!(
            inspect_result(Some((200, Some(json!({ "ok": true, "channels": 4, "class": "prtr", "description": 7 }))))),
            Inspect::Ok { color_space: "CMYK".into(), description: String::new(), version: String::new() }
        );
        let unsupported = |b: Value| matches!(inspect_result(Some((200, Some(b)))), Inspect::Err { status: 422, error: "unsupported_profile", .. });
        assert!(unsupported(json!({ "ok": true, "channels": 3, "class": "mntr" })));
        assert!(unsupported(json!({ "ok": true, "channels": 4, "class": "link" })));
        assert!(unsupported(json!({ "ok": true, "channels": "4", "class": "prtr" })));
        assert_eq!(
            inspect_result(Some((422, Some(json!({ "ok": false, "message": "bad header" }))))),
            Inspect::Err { status: 422, error: "invalid_icc", message: "bad header".into() }
        );
        assert_eq!(inspect_result(Some((422, None))), Inspect::Err { status: 422, error: "invalid_icc", message: "That is not a valid ICC profile.".into() });
        assert_eq!(inspect_result(None), Inspect::Err { status: 502, error: "render_unavailable", message: "render service is not running".into() });
        assert_eq!(inspect_result(Some((404, None))), Inspect::Err { status: 502, error: "render_unavailable", message: "render service answered 404".into() });
        assert_eq!(inspect_result(Some((500, Some(cmyk)))), Inspect::Err { status: 502, error: "render_unavailable", message: "render service answered 500".into() });
        assert_eq!(inspect_result(Some((200, Some(json!({ "ok": "yes" }))))), Inspect::Err { status: 502, error: "render_unavailable", message: "render service answered 200".into() });
    }

    #[test]
    fn gamut_mapping() {
        let r = json!({ "results": [{ "inGamut": true, "deltaE": 0.42, "cmyk": [0, 100, 100, 0] }] });
        assert_eq!(gamut_result(Some((200, Some(r.clone())))), (200, r));
        let e = json!({ "error": "profile_unavailable", "message": "run pnpm icc:sync" });
        assert_eq!(gamut_result(Some((422, Some(e.clone())))), (422, e));
        assert_eq!(gamut_result(Some((400, None))), (502, json!({ "error": "render_unavailable", "message": "render service answered 400" })));
        assert_eq!(gamut_result(Some((200, Some(json!(0))))), (502, json!({ "error": "render_unavailable", "message": "render service answered 200" })));
        assert_eq!(gamut_result(Some((503, Some(json!({ "error": "x" }))))), (502, json!({ "error": "render_unavailable", "message": "render service answered 503" })));
        assert_eq!(gamut_result(None), (502, json!({ "error": "render_unavailable", "message": "render service is not running" })));
    }

    /// Expected strings printed by zod 4.6.5 (`printGamutRequest` / `printExportOptions` of @oc/schema).
    #[test]
    fn schemas_match_zod() {
        let ok = print_gamut_request().safe_parse(Some(&json!({ "profile": { "id": "00000000-0000-4000-8000-000000000001", "x": 1 }, "colors": [{ "space": "cmyk", "c": 0, "m": 1, "y": 0, "k": 0, "a": 1, "name": "keep" }] }))).unwrap();
        assert_eq!(jsjson::stringify(&ok), r#"{"profile":{"id":"00000000-0000-4000-8000-000000000001"},"colors":[{"space":"cmyk","c":0,"m":1,"y":0,"k":0,"a":1,"name":"keep"}]}"#);
        assert_eq!(
            issues(&print_gamut_request(), json!({ "profile": { "id": "nope" }, "colors": [{ "space": "srgb", "r": 1, "g": 0, "b": 0, "a": 1 }] })),
            r#"[{"origin":"string","code":"invalid_format","format":"uuid","pattern":"/^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$/","path":["profile","id"],"message":"Invalid UUID"}]"#
        );
        assert_eq!(
            issues(&z::obj([("print", print_export_options().optional())]), json!({ "print": { "bleedMm": 12, "cropMarks": "yes", "profile": 3 } })),
            r#"[{"origin":"number","code":"too_big","maximum":10,"inclusive":true,"path":["print","bleedMm"],"message":"Too big: expected number to be <=10"},{"expected":"boolean","code":"invalid_type","path":["print","cropMarks"],"message":"Invalid input: expected boolean, received string"},{"code":"invalid_union","errors":[[{"code":"invalid_value","values":["pso-coated-v3","pso-uncoated-v3","iso-coated-v2","srgb"],"path":[],"message":"Invalid option: expected one of \"pso-coated-v3\"|\"pso-uncoated-v3\"|\"iso-coated-v2\"|\"srgb\""}],[{"expected":"object","code":"invalid_type","path":[],"message":"Invalid input: expected object, received number"}]],"path":["print","profile"],"message":"Invalid input"}]"#
        );
    }
}
