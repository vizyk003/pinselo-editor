//! Port of services/api/src/routes/v1.ts — the public REST API v1 (API-001..010), authenticated with personal
//! access tokens (`Authorization: Bearer oc_pat_…`) carrying scopes: designs (list, read, create, document
//! JSON), autofill (fill a template's data bindings), asynchronous exports, asset upload and webhooks; plus the
//! signed-in token and webhook management of Settings → Developers. The OpenAPI document is
//! [`crate::routes::openapi`].
//!
//! Fastify order for the token routes: the plugin's `onRequest` token check runs before the route's rate limit
//! (a rejected token is not counted and gets no rate-limit headers), the limit is keyed by the `Authorization`
//! header (autofill: by IP) with its own counter per route, and only then is the body parsed. So these handlers
//! take the raw request and build [`Req`] themselves after the limit (with the matched path removed so `Req`
//! does not count the request again).

use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::{FromRequest, MatchedPath, Request};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use oc_auth::Role;
use serde_json::Map;

use crate::auth::{at_least, design_role};
use crate::brandhub;
use crate::http::ratelimit::{self, Limiter, MINUTE};
use crate::http::req::Sink;
use crate::js;
use crate::prelude::*;
use crate::routes::assets::{Ingest, IngestOpts, TempDir, ingest_file};
use crate::routes::designs::{CreateOpts, create_design_from_doc, load_doc};

pub const SCOPES: [&str; 5] = ["designs:read", "designs:write", "assets:write", "exports", "webhooks"];

/// The token routes count with their own keyed limiters (see the module docs); only the signed-in routes and
/// `/v1/openapi.json` go through the shared ones.
pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/openapi.json", get(openapi))
        .route("/v1/me", get(me))
        .route("/v1/designs", get(list_designs).post(create_design))
        .route("/v1/designs/{id}", get(read_design))
        .route("/v1/designs/{id}/document", get(document))
        .route("/v1/designs/{id}/autofill", post(autofill))
        .route("/v1/designs/{id}/exports", post(start_export))
        .route("/v1/exports/{job}", get(export_status))
        .route("/v1/exports/{job}/file", get(export_file))
        .route("/v1/assets", post(upload_asset))
        .route("/v1/webhooks", get(list_webhooks).post(create_webhook))
        .route("/v1/webhooks/{id}", delete(delete_webhook))
        .route("/api/developer", get(developer))
        .route("/api/developer/tokens", post(create_token))
        .route("/api/developer/tokens/{id}", delete(revoke_token))
        .route("/api/developer/webhooks", post(dev_create_webhook))
        .route("/api/developer/webhooks/{id}/ping", post(ping_webhook))
        .route("/api/developer/webhooks/{id}", delete(dev_delete_webhook))
}

fn sha(s: &str) -> String {
    crate::auth::sha(s)
}

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

/// `req.apiUser`.
struct ApiUser {
    id: Uuid,
    scopes: Vec<String>,
}

/// One `LocalStore` per token route (`store.child(routeOptions)`).
#[derive(Default)]
struct V1Limits(Mutex<HashMap<&'static str, std::sync::Arc<Limiter>>>);

/// The per-route limit of a token route.
#[derive(Clone, Copy)]
enum Limit {
    /// `{ max: 120, timeWindow: '1 minute', keyGenerator: authorization ?? ip }`
    Token,
    /// `{ max: 30, timeWindow: '1 minute' }` (autofill: keyed by IP)
    Autofill,
}

/// `tokenAuth` (onRequest), the route's rate limit, then the body: the API user and the parsed request.
async fn api(s: &AppState, raw: Request, route: &'static str, limit: Limit) -> Result<(ApiUser, Req), ApiError> {
    let (mut parts, body) = raw.into_parts();
    let auth = parts.headers.get("authorization").map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    let h = auth.clone().unwrap_or_default();
    if !(h.starts_with("Bearer oc_pat_") || h.starts_with("Bearer oc_oat_")) {
        return Err(reply::json(401, json!({ "error": "unauthenticated", "message": "Send Authorization: Bearer <personal access token>." })).into());
    }
    // OAuth access tokens were granted to an AI app for the MCP endpoint only (bearer.rs: audience).
    if h.starts_with("Bearer oc_oat_") {
        return Err(reply::json(
            401,
            json!({ "error": "invalid_token", "message": "OAuth access tokens of connected AI apps only work on the MCP endpoint (/mcp). Use a personal access token." }),
        )
        .into());
    }
    let Some(t) = crate::bearer::resolve_bearer(s, Some(&h), crate::bearer::Audience::Rest).await? else {
        return Err(reply::json(401, json!({ "error": "invalid_token" })).into());
    };
    let (user_id, scopes) = (t.id, t.scopes);
    // The route's rate limit.
    let peer = parts.extensions.get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().map(|c| c.0);
    let ip = crate::http::req::client_ip(&parts.headers, peer);
    let (max, key) = match limit {
        Limit::Token => (120, auth.unwrap_or_else(|| ip.clone())),
        Limit::Autofill => (30, ratelimit::normalize_ip(&ip)),
    };
    let limiter = {
        let l = s.local::<V1Limits>();
        let mut m = l.0.lock().expect("v1 limits");
        m.entry(route).or_insert_with(|| std::sync::Arc::new(Limiter::new(max, MINUTE))).clone()
    };
    let c = limiter.incr(&key, s.now_ms());
    if let Some(sink) = parts.extensions.get::<Sink>() {
        sink.set("x-ratelimit-limit", c.max);
        sink.set("x-ratelimit-remaining", if c.exceeded { 0 } else { c.remaining });
        sink.set("x-ratelimit-reset", c.ttl_seconds);
        if c.exceeded {
            sink.set("retry-after", c.ttl_seconds);
        }
    }
    if c.exceeded {
        return Err(ApiError::status(429, format!("Rate limit exceeded, retry in {}", ratelimit::format_ms(c.ttl_seconds * 1000))));
    }
    parts.extensions.remove::<MatchedPath>();
    let req = Req::from_request(Request::from_parts(parts, body), s).await?;
    Ok((ApiUser { id: user_id, scopes }, req))
}

/// `need(req, reply, scope)`.
fn need(u: &ApiUser, scope: &str) -> Result<(), ApiError> {
    if u.scopes.iter().any(|x| x == scope) {
        return Ok(());
    }
    Err(reply::json(403, json!({ "error": "insufficient_scope", "message": format!("This token needs the {scope} scope.") })).into())
}

/// `reply.code(ing.status).send({ error: ing.error, message: ing.message })` (`message` may be undefined).
fn ingest_error(status: u16, error: &str, message: Option<String>) -> Reply {
    let mut m = Map::new();
    m.insert("error".into(), json!(error));
    if let Some(msg) = message {
        m.insert("message".into(), json!(msg));
    }
    reply::json(status, Value::Object(m))
}

fn respond(r: ApiResult) -> Response {
    match r {
        Ok(r) => r.into_response(),
        Err(e) => e.into_response(),
    }
}

const DESIGN_COLS: &str = "id, title, surface, width, height, page_count, created_at, updated_at";

/// `designOut(d)`.
fn design_out(s: &AppState, d: &Value) -> Value {
    let id = d["id"].as_str().unwrap_or("");
    json!({
        "id": id, "title": d["title"], "type": d["surface"], "width": d["width"], "height": d["height"], "pages": d["page_count"],
        "createdAt": d["created_at"], "updatedAt": d["updated_at"], "editUrl": format!("{}/design/{id}", s.cfg.web_origin),
        "thumbnailUrl": format!("{}/api/designs/{id}/thumbnail.png", s.cfg.web_origin),
    })
}

async fn openapi(State(s): State<AppState>, _req: Req) -> ApiResult {
    Ok(Reply::json(crate::routes::openapi::openapi(&s.cfg.web_origin)))
}

async fn me(State(s): State<AppState>, raw: Request) -> Response {
    respond(
        async {
            let (u, _req) = api(&s, raw, "GET /v1/me", Limit::Token).await?;
            let user = pg::json_one(&s.pool, "SELECT id, name, email FROM users WHERE id = $1", &[pg::uuid(u.id)]).await?;
            Ok(Reply::json(json!({ "user": user, "scopes": u.scopes })))
        }
        .await,
    )
}

async fn list_designs(State(s): State<AppState>, raw: Request) -> Response {
    respond(
        async {
            let (u, req) = api(&s, raw, "GET /v1/designs", Limit::Token).await?;
            need(&u, "designs:read")?;
            let q = z::obj([
                ("limit", z::coerce_num().int().min(1.0).max(100.0).default(json!(25))),
                ("before", z::str().datetime().optional()),
                ("q", z::str().max(100.0).optional()),
            ])
            .parse(Some(&req.query))?;
            let limit = q["limit"].as_f64().unwrap_or(25.0) as i64;
            let rows = pg::json_rows(
                &s.pool,
                &format!(
                    "SELECT d.id, d.title, d.surface, d.width, d.height, d.page_count, d.created_at, d.updated_at FROM designs d
         WHERE d.trashed_at IS NULL AND (d.owner_id = $1 OR {}
           OR EXISTS (SELECT 1 FROM memberships m JOIN workspaces w ON w.id = m.workspace_id WHERE m.workspace_id = d.workspace_id AND m.user_id = $1 AND NOT w.personal))
           AND ($2::timestamptz IS NULL OR d.updated_at < $2::timestamptz) AND ($3::text IS NULL OR d.title ILIKE '%' || $3 || '%')
         ORDER BY d.updated_at DESC LIMIT $4",
                    crate::share_access::shared_with_sql("d.id", "$1")
                ),
                &[pg::uuid(u.id), Arg::Text(q.get("before").and_then(Value::as_str).map(str::to_owned)), Arg::Text(q.get("q").and_then(Value::as_str).map(str::to_owned)), Arg::Int(Some(limit + 1))],
            )
            .await?;
            let items: Vec<Value> = rows.iter().take(limit as usize).map(|d| design_out(&s, d)).collect();
            let next = if rows.len() as i64 > limit { items.last().map(|i| i["updatedAt"].clone()).unwrap_or(Value::Null) } else { Value::Null };
            Ok(Reply::json(json!({ "items": items, "next": next })))
        }
        .await,
    )
}

async fn read_design(State(s): State<AppState>, Path(id): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, _req) = api(&s, raw, "GET /v1/designs/:id", Limit::Token).await?;
            need(&u, "designs:read")?;
            if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
                return Ok(not_found());
            }
            let d = pg::json_one(&s.pool, &format!("SELECT {DESIGN_COLS} FROM designs WHERE id = $1"), &[pg::text(&id)]).await?;
            Ok(match d {
                Some(d) => Reply::json(design_out(&s, &d)),
                None => not_found(),
            })
        }
        .await,
    )
}

async fn document(State(s): State<AppState>, Path(id): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, _req) = api(&s, raw, "GET /v1/designs/:id/document", Limit::Token).await?;
            need(&u, "designs:read")?;
            if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
                return Ok(not_found());
            }
            Ok(match load_doc(&s, &id).await? {
                Some(doc) => Reply::json(json!({ "document": doc })),
                None => not_found(),
            })
        }
        .await,
    )
}

/// JS truthiness of a JSON value.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn emit(s: &AppState, user: Uuid, event: oc_webhooks::WebhookEvent, data: Value) {
    let pool = s.pool.clone();
    s.spawn(async move {
        let _ = oc_webhooks::emit_event(&pool, user, event, &data).await;
    });
}

async fn create_design(State(s): State<AppState>, raw: Request) -> Response {
    respond(
        async {
            let (u, req) = api(&s, raw, "POST /v1/designs", Limit::Token).await?;
            need(&u, "designs:write")?;
            let b = z::obj([
                ("title", z::str().min(1.0).max(200.0).default(json!("Untitled design"))),
                ("preset", z::str().optional()),
                ("width", z::num().min(16.0).max(20000.0).optional()),
                ("height", z::num().min(16.0).max(20000.0).optional()),
                ("document", z::any().optional()),
            ])
            .parse(Some(&req.body_or_empty()))?;
            let title = b["title"].as_str().unwrap_or("Untitled design").to_owned();
            let mut doc = if truthy(b.get("document")) {
                b["document"].clone()
            } else {
                let p = b.get("preset").and_then(Value::as_str).and_then(oc_schema::presets::find_preset);
                let init = oc_schema::factory::MetaInit {
                    w: b.get("width").and_then(Value::as_f64).or(p.map(|p| p.w)).unwrap_or(1080.0),
                    h: b.get("height").and_then(Value::as_f64).or(p.map(|p| p.h)).unwrap_or(1080.0),
                    unit: Some(p.map_or(oc_schema::Unit::Px, |p| p.unit.clone())),
                    surface: Some(p.map_or(oc_schema::Surface::Graphic, |p| p.surface.clone())),
                    title: Some(title.clone()),
                    pages: None,
                };
                oc_schema::js::to_value(&oc_schema::factory::empty_design(&init))?
            };
            set_title(&mut doc, &title);
            let id = create_design_from_doc(&s, u.id, &doc, CreateOpts::default()).await?;
            emit(&s, u.id, oc_webhooks::WebhookEvent::DesignCreated, json!({ "designId": id, "title": title, "via": "api" }));
            let d = pg::json_one(&s.pool, &format!("SELECT {DESIGN_COLS} FROM designs WHERE id = $1"), &[pg::text(&id)]).await?.unwrap_or(Value::Null);
            Ok(Reply::json(design_out(&s, &d)).status(201))
        }
        .await,
    )
}

/// `doc = { ...doc, meta: { ...doc.meta, title } }`.
fn set_title(doc: &mut Value, title: &str) {
    let mut meta = doc.get("meta").and_then(Value::as_object).cloned().unwrap_or_default();
    meta.insert("title".into(), json!(title));
    if let Some(o) = doc.as_object_mut() {
        o.insert("meta".into(), Value::Object(meta));
    }
}

/// `coverCropFor(boxAspect, imageAspect)` of @oc/render-core.
pub(crate) fn cover_crop_for(ba: f64, ia: f64) -> Value {
    let rect = |x: f64, y: f64, w: f64, h: f64| json!({ "x": num(x), "y": num(y), "w": num(w), "h": num(h) });
    if ia == 0.0 || ia.is_nan() || ba == 0.0 || ba.is_nan() {
        return rect(0.0, 0.0, 1.0, 1.0);
    }
    if ia > ba {
        let w = ba / ia;
        return rect((1.0 - w) / 2.0, 0.0, w, 1.0);
    }
    let h = ia / ba;
    rect(0.0, (1.0 - h) / 2.0, 1.0, h)
}

fn num(f: f64) -> Value {
    crate::http::jsjson::number_value(f).unwrap_or(Value::Null)
}

/// Autofill: copy a template design and fill its data-bound elements (see Bulk create). Text values replace the
/// text (keeping its style); image values are https URLs (imported) or asset ids.
async fn autofill(State(s): State<AppState>, Path(id): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, req) = api(&s, raw, "POST /v1/designs/:id/autofill", Limit::Autofill).await?;
            need(&u, "designs:write")?;
            if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
                return Ok(not_found());
            }
            let b = z::obj([("title", z::str().max(200.0).optional()), ("data", z::record(z::str().max(10000.0)))]).parse(req.body())?;
            let data = b["data"].as_object().cloned().unwrap_or_default();
            let Some(src) = load_doc(&s, &id).await? else { return Ok(not_found()) };
            let mut doc = src.clone();
            let dir = TempDir::new("oc-autofill-").await?;
            let ids: Vec<String> = doc.get("elements").and_then(Value::as_object).map(|m| m.keys().cloned().collect()).unwrap_or_default();
            for eid in ids {
                let el = doc["elements"][&eid].clone();
                let Some(col) = el.pointer("/dataBinding/column").and_then(Value::as_str).filter(|c| !c.is_empty()) else { continue };
                let Some(value) = data.get(col).and_then(Value::as_str) else { continue };
                let ty = el.get("type").and_then(Value::as_str).unwrap_or("");
                if ty == "text" {
                    let p0 = el.pointer("/body/paragraphs/0").and_then(Value::as_object).cloned().ok_or_else(|| ApiError::internal("TypeError: paragraphs[0]"))?;
                    let mut attrs = p0.clone();
                    let runs = attrs.remove("runs");
                    let style = runs.as_ref().and_then(|r| r.get(0)).and_then(|r| r.get("style")).cloned().filter(|v| !v.is_null()).unwrap_or_else(|| json!({}));
                    let paragraphs: Vec<Value> = value
                        .split('\n')
                        .map(|l| l.strip_suffix('\r').unwrap_or(l))
                        .map(|line| {
                            let mut p = attrs.clone();
                            p.insert("runs".into(), json!([{ "text": line, "style": style }]));
                            Value::Object(p)
                        })
                        .collect();
                    let mut body = el.get("body").and_then(Value::as_object).cloned().unwrap_or_default();
                    body.insert("paragraphs".into(), Value::Array(paragraphs));
                    let mut ne = el.as_object().cloned().unwrap_or_default();
                    ne.insert("body".into(), Value::Object(body));
                    doc["elements"][&eid] = Value::Object(ne);
                } else if ty == "image" || ty == "frame" {
                    let mut asset: Option<Value> = None;
                    if value.starts_with("https://") {
                        let host = crate::routes::embeds::hostname_of(value).ok_or_else(|| ApiError::internal("Invalid URL"))?;
                        if !crate::stock::is_public_host(&host) {
                            return Ok(reply::json(400, json!({ "error": "bad_url" })));
                        }
                        let f = dir.0.join(oc_schema::ids::new_id());
                        if let Err(e) = crate::routes::content::download(value, |_| true, &f).await {
                            // A redirect or DNS record that leads off the public internet is the same refusal as the URL itself.
                            if matches!(e.as_str(), "media host not allowed" | "media host resolves to a private address" | "insecure media URL") {
                                return Ok(reply::json(400, json!({ "error": "bad_url" })));
                            }
                            return Ok(reply::json(400, json!({ "error": "fetch_failed", "message": format!("Couldn’t fetch {value}") })));
                        }
                        let name = value.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("image").to_owned();
                        let ing = ingest_file(
                            &s,
                            &f,
                            &dir.0,
                            IngestOpts { name: &name, owner_id: u.id, ip: None, source: None, attribution: Some(json!({ "provider": "url", "sourceUrl": value })), skip_quota: false },
                        )
                        .await?;
                        match ing {
                            Ingest::Asset(a) => asset = Some(a),
                            Ingest::Error { status, error, message } => return Ok(ingest_error(status, error, message)),
                        }
                    } else {
                        let a = pg::json_one(&s.pool, "SELECT id, sha256, mime, kind, width, height, name FROM assets WHERE id = $1 AND owner_id = $2", &[pg::text(value), pg::uuid(u.id)]).await.ok().flatten();
                        if let Some(a) = a {
                            asset = Some(json!({ "id": a["id"], "kind": a["kind"], "sha256": a["sha256"], "mime": a["mime"], "w": a["width"], "h": a["height"], "name": a["name"] }));
                        }
                    }
                    let Some(asset) = asset.filter(|a| matches!(a["kind"].as_str(), Some("image" | "svg"))) else {
                        return Ok(reply::json(400, json!({ "error": "bad_image", "message": format!("“{col}” must be an https image URL or one of your image asset ids.") })));
                    };
                    let aid = asset["id"].as_str().unwrap_or("").to_owned();
                    let mut assets = doc.get("assets").and_then(Value::as_object).cloned().unwrap_or_default();
                    assets.insert(aid.clone(), asset.clone());
                    doc["assets"] = Value::Object(assets);
                    let f = |v: &Value| v.as_f64().unwrap_or(f64::NAN);
                    let crop = cover_crop_for(f(&el["xf"]["w"]) / f(&el["xf"]["h"]), f(&asset["w"]) / f(&asset["h"]));
                    let mut ne = el.as_object().cloned().unwrap_or_default();
                    if ty == "image" {
                        ne.insert("asset".into(), json!(aid));
                        ne.insert("crop".into(), crop);
                    } else {
                        ne.insert("content".into(), json!({ "asset": aid, "crop": crop }));
                    }
                    doc["elements"][&eid] = Value::Object(ne);
                }
            }
            drop(dir);
            let title = b.get("title").and_then(Value::as_str).map_or_else(|| format!("{} (autofill)", src.pointer("/meta/title").and_then(Value::as_str).unwrap_or("undefined")), str::to_owned);
            set_title(&mut doc, &title);
            // Stays linked to the source's brand template (its brand locks keep applying).
            let link: Option<(Option<String>, Option<Uuid>, Option<i32>)> =
                sqlx::query_as("SELECT template_id, brand_workspace_id, template_version FROM designs WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
            let (template_id, brand_workspace_id, template_version) = link.unwrap_or((None, None, None));
            let new_id = create_design_from_doc(&s, u.id, &doc, CreateOpts { template_id, brand_workspace_id, template_version, ..Default::default() }).await?;
            emit(&s, u.id, oc_webhooks::WebhookEvent::DesignCreated, json!({ "designId": new_id, "title": title, "via": "autofill", "template": id }));
            Ok(Reply::json(json!({ "id": new_id, "editUrl": format!("{}/design/{new_id}", s.cfg.web_origin) })).status(201))
        }
        .await,
    )
}

async fn start_export(State(s): State<AppState>, Path(id): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, req) = api(&s, raw, "POST /v1/designs/:id/exports", Limit::Token).await?;
            need(&u, "exports")?;
            if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
                return Ok(not_found());
            }
            let b = z::obj([
                ("format", z::enumeration(&["pdf", "pdf-print", "pptx", "mp4", "gif", "m4a"])),
                ("quality", z::enumeration(&["720p", "1080p", "4k"]).optional()),
                ("print", crate::routes::print_profiles::print_export_options().optional()),
            ])
            .parse(req.body())?;
            if let Some(gate) = brandhub::approval_gate(&s, u.id, &id).await? {
                return Ok(reply::json(403, gate)); // brand approvals (A8)
            }
            let Some(print) = crate::routes::print_profiles::resolve_print_options(&s, u.id, b.get("print")).await? else {
                return Ok(crate::routes::print_profiles::profile_not_found());
            };
            let mut payload = Map::new();
            payload.insert("designId".into(), json!(id));
            payload.insert("userId".into(), json!(u.id.to_string()));
            payload.insert("format".into(), b["format"].clone());
            if let Some(q) = b.get("quality") {
                payload.insert("quality".into(), q.clone());
            }
            payload.extend(print);
            let job = oc_jobs::enqueue(&s.pool, "export", &Value::Object(payload), oc_jobs::EnqueueOpts { owner_id: Some(u.id), max_attempts: Some(2), ..Default::default() }).await?;
            Ok(Reply::json(json!({ "id": job.to_string(), "status": "queued", "statusUrl": format!("{}/v1/exports/{job}", s.cfg.web_origin) })).status(202))
        }
        .await,
    )
}

/// `getJob(id).catch(() => null)` as a node-postgres row.
async fn job_row(s: &AppState, id: &str) -> Option<Value> {
    pg::json_one(&s.pool, "SELECT * FROM jobs WHERE id = $1::uuid", &[pg::text(id)]).await.ok().flatten()
}

async fn export_status(State(s): State<AppState>, Path(job): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, _req) = api(&s, raw, "GET /v1/exports/:job", Limit::Token).await?;
            need(&u, "exports")?;
            let j = job_row(&s, &job).await;
            let Some(j) = j.filter(|j| j["queue"] == "export" && j["owner_id"].as_str() == Some(&u.id.to_string())) else { return Ok(not_found()) };
            let status = j["status"].as_str().unwrap_or("");
            let mut out = Map::new();
            out.insert("id".into(), j["id"].clone());
            out.insert("status".into(), json!(if status == "failed" { "error" } else { status }));
            out.insert("progress".into(), j["progress"].clone());
            if !j["error"].is_null() {
                out.insert("error".into(), j["error"].clone());
            }
            if status == "done" {
                let r = &j["result"];
                for (k, v) in [("filename", r.get("filename")), ("size", r.get("size"))] {
                    if let Some(v) = v {
                        out.insert(k.into(), v.clone());
                    }
                }
                out.insert("downloadUrl".into(), json!(format!("{}/v1/exports/{}/file", s.cfg.web_origin, j["id"].as_str().unwrap_or(""))));
            }
            Ok(Reply::json(Value::Object(out)))
        }
        .await,
    )
}

async fn export_file(State(s): State<AppState>, Path(job): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, _req) = api(&s, raw, "GET /v1/exports/:job/file", Limit::Token).await?;
            need(&u, "exports")?;
            let j = job_row(&s, &job).await;
            let ok = j.as_ref().filter(|j| j["owner_id"].as_str() == Some(&u.id.to_string()) && j["status"] == "done" && j["result"].is_object());
            let Some(j) = ok else { return Ok(reply::json(404, json!({ "error": "not_ready" }))) };
            let r = &j["result"];
            let filename = r["filename"].as_str().unwrap_or("").replace('"', "");
            let bytes = s.store.get(r["sha"].as_str().unwrap_or("")).await?;
            Ok(Reply::bytes(r["type"].as_str().unwrap_or("application/octet-stream"), bytes).header("content-disposition", format!("attachment; filename=\"{filename}\"")))
        }
        .await,
    )
}

async fn upload_asset(State(s): State<AppState>, raw: Request) -> Response {
    respond(
        async {
            let (u, mut req) = api(&s, raw, "POST /v1/assets", Limit::Token).await?;
            need(&u, "assets:write")?;
            let dir = TempDir::new("oc-v1-").await?;
            let src = dir.0.join("src");
            let Some(file) = crate::http::multipart::save_file(&mut req, s.cfg.max_media_bytes as u64, &src).await? else {
                return Ok(reply::json(400, json!({ "error": "no_file" })));
            };
            if file.truncated {
                // `file.toBuffer()` throws RequestFileTooLargeError.
                return Err(ApiError::status(413, "request file too large"));
            }
            let r = ingest_file(&s, &src, &dir.0, IngestOpts { name: &file.filename, owner_id: u.id, ip: Some(&req.ip), source: None, attribution: None, skip_quota: false }).await?;
            Ok(match r {
                Ingest::Asset(a) => Reply::json(json!({ "asset": a })).status(201),
                Ingest::Error { status, error, message } => ingest_error(status, error, message),
            })
        }
        .await,
    )
}

// ---------------------------------------------------------------- webhooks

fn webhook_body(req: &Req) -> Result<(String, Vec<String>), ApiError> {
    let b = z::obj([("url", z::str().url().max(2000.0)), ("events", z::array(z::enumeration(&oc_webhooks::WEBHOOK_EVENTS)).min(1.0))]).parse(req.body())?;
    let events = b["events"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    Ok((b["url"].as_str().unwrap_or("").to_owned(), events))
}

async fn insert_webhook(s: &AppState, user: Uuid, url: &str, events: &[String]) -> Result<(Uuid, String), ApiError> {
    let id = oc_db::uuid();
    let secret = format!("whsec_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(crate::auth::random_bytes(24)));
    sqlx::query("INSERT INTO webhooks(id, user_id, url, events, secret) VALUES ($1,$2,$3,$4,$5)").bind(id).bind(user).bind(url).bind(events).bind(&secret).execute(&s.pool).await?;
    Ok((id, secret))
}

async fn list_webhooks(State(s): State<AppState>, raw: Request) -> Response {
    respond(
        async {
            let (u, _req) = api(&s, raw, "GET /v1/webhooks", Limit::Token).await?;
            need(&u, "webhooks")?;
            let items = pg::json_rows(&s.pool, "SELECT id, url, events, active, created_at FROM webhooks WHERE user_id = $1 ORDER BY created_at", &[pg::uuid(u.id)]).await?;
            Ok(Reply::json(json!({ "items": items })))
        }
        .await,
    )
}

async fn create_webhook(State(s): State<AppState>, raw: Request) -> Response {
    respond(
        async {
            let (u, req) = api(&s, raw, "POST /v1/webhooks", Limit::Token).await?;
            need(&u, "webhooks")?;
            let (url, events) = webhook_body(&req)?;
            let (id, secret) = insert_webhook(&s, u.id, &url, &events).await?;
            Ok(Reply::json(json!({ "id": id.to_string(), "url": url, "events": events, "secret": secret })).status(201))
        }
        .await,
    )
}

async fn delete_webhook(State(s): State<AppState>, Path(id): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, _req) = api(&s, raw, "DELETE /v1/webhooks/:id", Limit::Token).await?;
            need(&u, "webhooks")?;
            pg::exec(&s.pool, "DELETE FROM webhooks WHERE id = $1::uuid AND user_id = $2", &[pg::text(&id), pg::uuid(u.id)]).await?;
            Ok(Reply::empty(204))
        }
        .await,
    )
}

// ---------------------------------------------------------------- token & webhook management (signed-in UI)

async fn developer(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let tokens = pg::json_rows(
        &s.pool,
        "SELECT id, name, prefix, scopes, last_used_at, expires_at, created_at FROM api_tokens WHERE user_id = $1 AND kind = 'pat' AND revoked_at IS NULL ORDER BY created_at DESC",
        &[pg::uuid(u.id)],
    )
    .await?;
    let webhooks = pg::json_rows(
        &s.pool,
        "SELECT w.id, w.url, w.events, w.active, w.failures, w.created_at,
           (SELECT json_agg(x ORDER BY x.created_at DESC) FROM (SELECT id, event, status, response_code, error, attempts, created_at FROM webhook_deliveries d WHERE d.webhook_id = w.id ORDER BY created_at DESC LIMIT 10) x) AS deliveries
         FROM webhooks w WHERE w.user_id = $1 ORDER BY w.created_at",
        &[pg::uuid(u.id)],
    )
    .await?;
    Ok(Reply::json(json!({ "tokens": tokens, "webhooks": webhooks, "scopes": SCOPES, "events": oc_webhooks::WEBHOOK_EVENTS })))
}

async fn create_token(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("name", z::str().min(1.0).max(80.0)), ("scopes", z::array(z::enumeration(&SCOPES)).min(1.0)), ("expiresInDays", z::num().int().min(1.0).max(366.0).optional())]).parse(req.body())?;
    let token = format!("oc_pat_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(crate::auth::random_bytes(24)));
    let id = oc_db::uuid();
    let scopes: Vec<String> = b["scopes"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    sqlx::query(
        "INSERT INTO api_tokens(id, user_id, name, token_sha256, prefix, scopes, expires_at) VALUES ($1,$2,$3,$4,$5,$6, CASE WHEN $7::int IS NULL THEN NULL ELSE now() + make_interval(days => $7::int) END)",
    )
    .bind(id)
    .bind(u.id)
    .bind(b["name"].as_str().unwrap_or(""))
    .bind(sha(&token))
    .bind(js::take(&token, 12))
    .bind(&scopes)
    .bind(b.get("expiresInDays").and_then(Value::as_f64).map(|d| d as i32))
    .execute(&s.pool)
    .await?;
    Ok(Reply::json(json!({ "id": id.to_string(), "token": token })))
}

async fn revoke_token(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    pg::exec(&s.pool, "UPDATE api_tokens SET revoked_at = now() WHERE id = $1::uuid AND user_id = $2 AND kind = 'pat'", &[pg::text(&id), pg::uuid(u.id)]).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn dev_create_webhook(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let (url, events) = webhook_body(&req)?;
    let (id, secret) = insert_webhook(&s, u.id, &url, &events).await?;
    Ok(Reply::json(json!({ "id": id.to_string(), "secret": secret })))
}

async fn ping_webhook(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let w: Option<Uuid> = sqlx::query_scalar("SELECT id FROM webhooks WHERE id = $1::uuid AND user_id = $2").bind(&id).bind(u.id).fetch_optional(&s.pool).await?;
    let Some(w) = w else { return Ok(Reply::json(json!({ "ok": false }))) };
    sqlx::query("UPDATE webhooks SET active = true, failures = 0, events = CASE WHEN 'ping' = ANY(events) THEN events ELSE events || '{ping}' END WHERE id = $1").bind(w).execute(&s.pool).await?;
    oc_webhooks::emit_event(&s.pool, u.id, oc_webhooks::WebhookEvent::Ping, &json!({ "message": "Hello from Pinselo" })).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn dev_delete_webhook(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    pg::exec(&s.pool, "DELETE FROM webhooks WHERE id = $1::uuid AND user_id = $2", &[pg::text(&id), pg::uuid(u.id)]).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crop() {
        assert_eq!(cover_crop_for(1.0, 2.0), json!({ "x": 0.25, "y": 0, "w": 0.5, "h": 1 }));
        assert_eq!(cover_crop_for(2.0, 1.0), json!({ "x": 0, "y": 0.25, "w": 1, "h": 0.5 }));
        assert_eq!(cover_crop_for(0.0, 1.0), json!({ "x": 0, "y": 0, "w": 1, "h": 1 }));
    }
}
