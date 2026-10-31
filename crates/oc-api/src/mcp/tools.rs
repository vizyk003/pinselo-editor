//! Port of services/api/src/mcp/tools.ts — the AI connector's tools. Each call runs as the token's user with the
//! token's scopes; design access uses the same roles as the editor, and edits go through the live-collaborator path
//! in [`super::live`].

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine as _;
use oc_auth::Role;
use serde_json::{Map, Value, json};

use super::edit::{EditContext, run_edits};
use super::live::{self, Agent, Presence};
use super::view::{self, design_summary, design_view};
use crate::auth::{at_least, design_role};
use crate::bearer::BearerUser;
use crate::http::error::ApiError;
use crate::pg;
use crate::search::like_escape;
use crate::state::AppState;

/// A failure the assistant is told about (`ToolError` / `UpgradeRequired`), or an unexpected one.
pub enum Fail {
    Tool(String),
    Other(String),
}

impl From<ApiError> for Fail {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::Upgrade { message, .. } => Fail::Tool(message),
            other => Fail::Other(format!("{other:?}")),
        }
    }
}
impl From<sqlx::Error> for Fail {
    fn from(e: sqlx::Error) -> Self {
        Fail::Other(e.to_string())
    }
}
impl From<oc_crdt::CrdtError> for Fail {
    fn from(e: oc_crdt::CrdtError) -> Self {
        Fail::Other(e.to_string())
    }
}

type R = Result<Value, Fail>;

fn tool_err<T>(m: impl Into<String>) -> Result<T, Fail> {
    Err(Fail::Tool(m.into()))
}

/// `JSON.stringify(v, null, 1)` with JS number formatting.
pub fn pretty(v: &Value) -> String {
    pretty_indent(v, 1)
}

/// `JSON.stringify(v, null, indent)`.
pub fn pretty_indent(v: &Value, indent: usize) -> String {
    fn go(v: &Value, depth: usize, out: &mut String, indent: usize) {
        let pad = |d: usize| " ".repeat(d * indent);
        match v {
            Value::Array(a) if !a.is_empty() => {
                out.push_str("[\n");
                for (i, x) in a.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    go(x, depth + 1, out, indent);
                    if i + 1 < a.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad(depth));
                out.push(']');
            }
            Value::Object(m) if !m.is_empty() => {
                out.push_str("{\n");
                let n = m.len();
                for (i, (k, x)) in m.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    out.push_str(&crate::http::jsjson::stringify(&Value::String(k.clone())));
                    out.push_str(": ");
                    go(x, depth + 1, out, indent);
                    if i + 1 < n {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&pad(depth));
                out.push('}');
            }
            other => out.push_str(&crate::http::jsjson::stringify(other)),
        }
    }
    let mut out = String::new();
    go(v, 0, &mut out, indent);
    out
}

/// `text(v)`: a text content result (strings as they are, values as indented JSON).
pub fn text(v: &Value) -> Value {
    let t = match v {
        Value::String(s) => s.clone(),
        other => pretty(other),
    };
    json!({ "content": [{ "type": "text", "text": t }] })
}

fn need(u: &BearerUser, scope: &str) -> Result<(), Fail> {
    if u.scopes.iter().any(|s| s == scope) {
        return Ok(());
    }
    tool_err(format!("This connection is not allowed to do that (it needs the \"{scope}\" permission). Reconnect the app and allow it, or use a token with that scope."))
}

async fn access(s: &AppState, u: &BearerUser, design_id: &str, min: Role) -> Result<Role, Fail> {
    let Some(role) = design_role(s, Some(u.id), design_id).await? else {
        return tool_err(format!("No design {design_id} that you can open. Use list_designs to find one."));
    };
    if !at_least(Some(role), min) {
        return tool_err("You can view this design but not edit it. Ask its owner for edit access, or make a copy.");
    }
    Ok(role)
}

async fn agent_for(s: &AppState, u: &BearerUser, design_id: &str) -> Result<Agent, Fail> {
    let role = access(s, u, design_id, Role::Edit).await?;
    Ok(Agent {
        user_id: u.id,
        user_name: u.name.clone(),
        name: if u.client_name.is_empty() { "AI assistant".into() } else { u.client_name.clone() },
        token_id: u.token_id,
        role,
        brand_admin: crate::brandhub::is_brand_admin(s, u.id, design_id).await?,
    })
}

fn edit_url(s: &AppState, id: &str) -> String {
    format!("{}/design/{id}", s.cfg.web_origin)
}

/// The JSON snapshot and typed document of a Y.Doc.
fn snapshot(ydoc: &yrs::Doc) -> Result<(Value, oc_schema::design::DesignDoc), Fail> {
    let json = oc_crdt::from_y_value(ydoc);
    let typed = match serde_json::from_value(json.clone()) {
        Ok(t) => t,
        Err(_) => oc_crdt::from_y(ydoc)?,
    };
    Ok((json, typed))
}

/// Import an image for an edit: an https URL, a stock result or an existing asset of the user.
async fn import_image(s: &AppState, u: &BearerUser, src: &Value, doc_assets: &Map<String, Value>) -> Result<Value, Fail> {
    let by_name;
    let src = if let Some(b) = src.get("brand") {
        let kits = crate::routes::brandhub::accessible_kits(s, u.id).await.map_err(|e| Fail::Other(e.to_string()))?;
        let id = kit_asset_by_name(s, &kits, b["kit"].as_str().unwrap_or(""), b["name"].as_str().unwrap_or("")).await?;
        by_name = json!({ "asset": id });
        &by_name
    } else {
        src
    };
    if let Some(id) = src.get("asset").and_then(Value::as_str) {
        if let Some(a) = doc_assets.get(id) {
            return Ok(a.clone());
        }
        let a = pg::json_one(
            &s.pool,
            "SELECT id, kind, sha256, mime, width, height, name FROM assets WHERE id = $1 AND trashed_at IS NULL
       AND (owner_id = $2 OR workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $2))",
            &[pg::text(id), pg::uuid(u.id)],
        )
        .await?;
        let Some(a) = a else {
            return tool_err(format!("No image asset {id}. Use list_uploads or get_brand_kit for asset ids."));
        };
        return Ok(json!({ "id": a["id"], "kind": a["kind"], "sha256": a["sha256"], "mime": a["mime"], "w": a["width"], "h": a["height"], "name": a["name"] }));
    }
    need(u, "assets:write")?;
    let run = async {
        let dir = crate::routes::assets::TempDir::new("oc-mcp-").await.map_err(|e| Fail::Other(e.to_string()))?;
        let file = dir.0.join("src");
        let (name, attribution, source);
        if let Some(st) = src.get("stock") {
            let pid = st["provider"].as_str().unwrap_or("");
            let Some(p) = crate::stock::PROVIDERS.iter().copied().find(|p| p.id() == pid && p.enabled(s)) else {
                return tool_err(format!("Unknown media provider {pid}."));
            };
            let kind = st.get("type").and_then(Value::as_str).unwrap_or("image");
            let m = p.media(s, kind, st["id"].as_str().unwrap_or("")).await.map_err(Fail::Other)?;
            crate::routes::content::download(&m.url, |h| p.host_allowed(h), &file).await.map_err(Fail::Other)?;
            attribution = Some(m.attribution);
            name = m.filename;
            source = if kind == "image" { "stock" } else { "gif" };
        } else {
            let url = src["url"].as_str().unwrap_or("");
            crate::routes::content::download(url, |_| true, &file).await.map_err(Fail::Other)?;
            let last = url::Url::parse(url).ok().and_then(|u| u.path().rsplit('/').next().map(str::to_owned)).unwrap_or_default();
            let decoded = percent_encoding::percent_decode_str(&last).decode_utf8().map(|d| d.into_owned()).unwrap_or(last.clone());
            name = crate::js::take(if decoded.is_empty() { "image" } else { &decoded }, 120);
            attribution = None;
            source = "upload";
        }
        let r = crate::routes::assets::ingest_file(
            s,
            &file,
            &dir.0,
            crate::routes::assets::IngestOpts { name: &name, owner_id: u.id, ip: None, source: Some(source), attribution, skip_quota: false },
        )
        .await
        .map_err(|e| Fail::Other(format!("{e:?}")))?;
        match r {
            crate::routes::assets::Ingest::Error { error, message, .. } => tool_err(message.unwrap_or_else(|| error.to_owned())),
            crate::routes::assets::Ingest::Asset(a) => {
                if a["kind"] != "image" && a["kind"] != "svg" {
                    return tool_err("That file is not an image.");
                }
                Ok(a)
            }
        }
    };
    match run.await {
        Ok(a) => Ok(a),
        Err(Fail::Tool(m)) => Err(Fail::Tool(m)),
        Err(Fail::Other(m)) => tool_err(format!("Couldn’t import the image: {m}")),
    }
}

/// The render service's layout checks of the touched pages, as `[{ id, problem, message }]`; `None` when it can't
/// answer in time (the edit itself has already succeeded).
async fn page_check(s: &AppState, design_id: &str, page_id: Option<&str>, touched: &[String]) -> Option<Vec<Value>> {
    lint(s, &json!({ "designId": design_id, "pageIds": page_id.into_iter().collect::<Vec<_>>(), "elementIds": touched })).await
}

/// `page_check` for whole pages (at most the first 10).
async fn pages_check(s: &AppState, design_id: &str, pages: &[String]) -> Option<Vec<Value>> {
    lint(s, &json!({ "designId": design_id, "pageIds": pages.iter().take(10).collect::<Vec<_>>() })).await
}

async fn lint(s: &AppState, body: &Value) -> Option<Vec<Value>> {
    let res = render(s, "/internal/page-lint", body, 8_000).await.ok()?;
    let v: Value = serde_json::from_str(&res.text().await.ok()?).ok()?;
    let pages = v["pages"].as_array()?;
    let many = pages.len() > 1;
    Some(
        pages
            .iter()
            .flat_map(|p| {
                let page = p["pageId"].clone();
                p["issues"].as_array().cloned().unwrap_or_default().into_iter().map(move |i| {
                    let mut o = json!({ "id": i["id"], "problem": i["kind"], "message": i["message"] });
                    if many {
                        o["page"] = page.clone();
                    }
                    o
                })
            })
            .collect(),
    )
}

async fn render(s: &AppState, path: &str, body: &Value, timeout_ms: u64) -> Result<reqwest::Response, Fail> {
    let res = s
        .http
        .post(format!("{}{path}", s.cfg.render_url))
        .header("content-type", "application/json")
        .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&s.cfg.secret))
        .body(crate::http::jsjson::stringify(body))
        .timeout(Duration::from_millis(timeout_ms))
        .send()
        .await;
    match res {
        Err(_) => tool_err("The render service is not running."),
        Ok(r) if !r.status().is_success() => {
            let t = r.text().await.unwrap_or_default();
            tool_err(format!("Rendering failed: {}", crate::js::take(&t, 300)))
        }
        Ok(r) => Ok(r),
    }
}

fn opt_str(a: &Value, k: &str) -> Option<String> {
    a.get(k).and_then(Value::as_str).map(str::to_owned)
}

fn trimmed_query(a: &Value) -> Option<String> {
    opt_str(a, "query").map(|q| crate::js::trim(&q).to_owned()).filter(|q| !q.is_empty())
}

#[derive(Default)]
struct Fonts(OnceLock<oc_gfonts::GoogleFonts>);

/// `JSON.stringify(page)` in messages (`No page ${page}.`).
fn page_label(p: &Value) -> String {
    match p {
        Value::String(s) => s.clone(),
        other => crate::http::jsjson::stringify(other),
    }
}

/// Run one tool (arguments already validated).
pub async fn call(s: &AppState, u: &BearerUser, name: &str, a: &Value) -> R {
    match name {
        "get_design_guide" => Ok(text(&json!(super::guide::guide(s.cfg.brand.name)))),
        "list_designs" => {
            need(u, "designs:read")?;
            let q = trimmed_query(a).map(|q| like_escape(&q));
            let rows = pg::json_rows(
                &s.pool,
                "SELECT d.id, d.title, d.surface, d.width, d.height, d.page_count, d.updated_at FROM designs d
       WHERE d.trashed_at IS NULL AND (d.owner_id = $1 OR d.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1) OR EXISTS (SELECT 1 FROM design_shares s WHERE s.design_id = d.id AND s.user_id = $1))
       AND ($2::text IS NULL OR d.title ILIKE '%' || $2 || '%' ESCAPE '\\') ORDER BY d.updated_at DESC LIMIT $3",
                &[pg::uuid(u.id), pg::opt_text(q), pg::Arg::Int(Some(a.get("limit").and_then(Value::as_f64).unwrap_or(20.0) as i64))],
            )
            .await?;
            Ok(text(&json!({ "designs": rows.iter().map(|r| design_summary(r, &s.cfg.web_origin)).collect::<Vec<_>>() })))
        }
        "list_size_presets" => {
            let q = opt_str(a, "query").map(|q| crate::js::trim(&q.to_lowercase()).to_owned());
            let list: Vec<Value> = oc_schema::presets::SIZE_PRESETS
                .iter()
                .filter_map(|p| serde_json::to_value(p).ok())
                .filter(|p| {
                    q.as_deref().is_none_or(|q| q.is_empty() || {
                        let hay = format!("{} {} {} {}", p["id"].as_str().unwrap_or(""), p["label"].as_str().unwrap_or(""), p["group"].as_str().unwrap_or(""), p["surface"].as_str().unwrap_or(""));
                        hay.to_lowercase().contains(q)
                    })
                })
                .map(|p| json!({ "id": p["id"], "label": p["label"], "group": p["group"], "type": p["surface"], "w": p["w"], "h": p["h"], "unit": p["unit"] }))
                .collect();
            Ok(text(&json!({ "presets": list })))
        }
        "search_templates" => {
            need(u, "designs:read")?;
            let p = crate::search::SearchParams {
                q: opt_str(a, "query").unwrap_or_default(),
                kind: "templates".into(),
                owner: "anyone".into(),
                modified: "any".into(),
                surface: None,
                limit: a.get("limit").and_then(Value::as_f64).unwrap_or(12.0) as i64,
            };
            let now = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(s.now_ms()).unwrap_or_default();
            let b = crate::search::templates_sql(u.id, &p, now);
            let rows = pg::json_rows(&s.pool, &b.sql, &b.params).await?;
            let list: Vec<Value> = rows
                .iter()
                .map(|t| {
                    let mut m = Map::new();
                    m.insert("id".into(), t["id"].clone());
                    m.insert("title".into(), t["title"].clone());
                    m.insert("category".into(), t["category"].clone());
                    m.insert("preset".into(), t["preset"].clone());
                    m.insert("size".into(), json!(format!("{}×{}", jsn(&t["width"]), jsn(&t["height"]))));
                    m.insert("tags".into(), t["tags"].clone());
                    if t["team"] == true {
                        m.insert("team".into(), json!(true));
                    }
                    if t["brand"] == true {
                        m.insert("brandTemplate".into(), json!(true));
                    }
                    m.insert("thumbnailUrl".into(), json!(format!("{}/api/templates/{}/thumbnail.png", s.cfg.web_origin, crate::js::encode_uri_component(t["id"].as_str().unwrap_or("")))));
                    Value::Object(m)
                })
                .collect();
            Ok(text(&json!({ "templates": list })))
        }
        "search_media" => {
            need(u, "designs:read")?;
            let kind = opt_str(a, "type").unwrap_or_else(|| "image".into());
            let q = crate::js::trim(a["query"].as_str().unwrap_or("")).to_owned();
            let page = a.get("page").and_then(Value::as_f64).unwrap_or(1.0) as i64;
            let providers: Vec<_> = crate::stock::PROVIDERS.iter().copied().filter(|p| p.enabled(s) && p.kinds().contains(&kind.as_str())).collect();
            let results = futures::future::join_all(providers.iter().map(|p| p.search(s, &kind, &q, page))).await;
            let items: Vec<Value> = results
                .into_iter()
                .flat_map(|r| r.ok().and_then(|v| v["items"].as_array().cloned()).unwrap_or_default())
                .take(30)
                .map(|i| {
                    let mut m = Map::new();
                    for k in ["provider", "id", "title", "w", "h", "thumb", "license"] {
                        if let Some(v) = i.get(k) {
                            m.insert(k.into(), v.clone());
                        }
                    }
                    if view::truthy(i.get("author")) {
                        m.insert("author".into(), i["author"].clone());
                    }
                    Value::Object(m)
                })
                .collect();
            Ok(text(&json!({ "results": items })))
        }
        "list_uploads" => {
            need(u, "designs:read")?;
            let q = trimmed_query(a).map(|q| like_escape(&q));
            let rows = pg::json_rows(
                &s.pool,
                "SELECT id, name, kind, width, height, created_at FROM assets WHERE owner_id = $1 AND trashed_at IS NULL AND kind IN ('image','svg') AND source <> 'emoji'
       AND ($2::text IS NULL OR name ILIKE '%' || $2 || '%' ESCAPE '\\') ORDER BY created_at DESC LIMIT $3",
                &[pg::uuid(u.id), pg::opt_text(q), pg::Arg::Int(Some(a.get("limit").and_then(Value::as_f64).unwrap_or(30.0) as i64))],
            )
            .await?;
            let list: Vec<Value> = rows.iter().map(|r| json!({ "asset": r["id"], "name": r["name"], "w": r["width"], "h": r["height"], "uploaded": r["created_at"] })).collect();
            Ok(text(&json!({ "uploads": list })))
        }
        "get_brand_kit" => {
            need(u, "designs:read")?;
            let kits = crate::routes::brandhub::accessible_kits(s, u.id).await?;
            if kits.is_empty() {
                return Ok(text(&json!("No brand kit yet. Follow the design system the user selected or named, if you can read it; otherwise pick colours and fonts that suit the request. Check list_uploads for their logo and images.")));
            }
            // File names let the assistant choose without opening every image.
            let ids: Vec<String> = kits
                .iter()
                .flat_map(|k| ["logos", "graphics", "photos"].map(|key| &k[key]))
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            let names: HashMap<String, String> = sqlx::query_as::<_, (String, Option<String>)>("SELECT id, name FROM assets WHERE id = ANY($1) AND trashed_at IS NULL")
                .bind(&ids)
                .fetch_all(&s.pool)
                .await?
                .into_iter()
                .map(|(id, name)| (id, name.unwrap_or_default()))
                .collect();
            let list: Vec<Value> = kits
                .iter()
                .map(|k| {
                    let mut m = Map::new();
                    m.insert("name".into(), k["name"].clone());
                    if !view::truthy(k.get("personal")) {
                        m.insert("team".into(), k["workspace"].clone());
                    }
                    if view::truthy(k.get("isDefault")) {
                        m.insert("default".into(), k["isDefault"].clone());
                    }
                    for key in ["colors", "palettes", "fonts"] {
                        if let Some(v) = k.get(key) {
                            m.insert(key.into(), v.clone());
                        }
                    }
                    for (key, cap) in [("logos", 30), ("graphics", 60), ("photos", 30)] {
                        let items = k[key].as_array().map(|ids| ids.iter().take(cap).map(|id| named_asset(id, &names)).collect()).unwrap_or_default();
                        m.insert(key.into(), Value::Array(items));
                    }
                    if view::truthy(k.get("voice")) {
                        m.insert("voice".into(), k["voice"].clone());
                    }
                    Value::Object(m)
                })
                .collect();
            Ok(text(&json!({ "kits": list })))
        }
        "search_fonts" => {
            let g = s.local::<Fonts>().0.get_or_init(|| oc_gfonts::GoogleFonts::new(s.paths.fonts.clone(), s.pool.clone(), s.store.clone())).clone();
            let manifest = serde_json::to_value(g.full_manifest().await.map_err(|e| Fail::Other(e.to_string()))?).map_err(|e| Fail::Other(e.to_string()))?;
            let pop: HashMap<String, f64> = g.catalog().iter().map(|f| (f.family.clone(), f.popularity.as_f64().unwrap_or(f64::NAN))).collect();
            let q = opt_str(a, "query").map(|q| crate::js::trim(&q.to_lowercase()).to_owned());
            let cat = opt_str(a, "category");
            let mut list: Vec<&Value> = manifest["families"]
                .as_array()
                .map(|f| {
                    f.iter()
                        .filter(|f| {
                            let fam = f["family"].as_str().unwrap_or("").to_lowercase();
                            q.as_deref().is_none_or(|q| q.is_empty() || fam.contains(q)) && cat.as_deref().is_none_or(|c| f["category"] == c)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let exact = |f: &Value| q.as_deref().is_some_and(|q| f["family"].as_str().unwrap_or("").to_lowercase() == q);
            let p = |f: &Value| pop.get(f["family"].as_str().unwrap_or("")).copied().unwrap_or(1e6);
            list.sort_by(|x, y| {
                let d = f64::from(u8::from(exact(y))) - f64::from(u8::from(exact(x)));
                let d = if d != 0.0 { d } else { p(x) - p(y) };
                d.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
            });
            let limit = a.get("limit").and_then(Value::as_f64).unwrap_or(30.0) as usize;
            Ok(text(&json!({ "fonts": list.iter().take(limit).map(|f| json!({ "family": f["family"], "category": f["category"] })).collect::<Vec<_>>() })))
        }
        "create_design" => {
            need(u, "designs:write")?;
            let unit = opt_str(a, "unit");
            if unit.as_deref().is_some_and(|x| x != "px") && (a.get("width").is_none() || a.get("height").is_none()) {
                return tool_err("Give both width and height with a unit.");
            }
            let mut b = Map::new();
            b.insert("title".into(), a["title"].clone());
            for (from, to) in [("preset", "preset"), ("width", "w"), ("height", "h"), ("unit", "unit"), ("template_id", "templateId")] {
                if let Some(v) = a.get(from) {
                    b.insert(to.into(), v.clone());
                }
            }
            let id = match crate::routes::designs::create_design_for_user(s, u.id, &Value::Object(b), "ai").await? {
                Ok(id) => id,
                Err((_, body)) => {
                    return tool_err(match body.get("message").and_then(Value::as_str) {
                        Some(m) => m.to_owned(),
                        None => format!("Couldn’t create the design ({}).", body["error"].as_str().unwrap_or("")),
                    });
                }
            };
            let view = match live::read_doc(s, &id, u.token_id).await? {
                Some(ydoc) => {
                    let (json, typed) = snapshot(&ydoc)?;
                    Value::Object(design_view(&json, &typed, None))
                }
                None => Value::Null,
            };
            Ok(text(&json!({ "id": id, "editUrl": edit_url(s, &id), "design": view })))
        }
        "copy_design" => {
            need(u, "designs:write")?;
            let id = a["design_id"].as_str().unwrap_or("").to_owned();
            access(s, u, &id, Role::View).await?;
            let mut b = Map::new();
            if let Some(t) = a.get("title") {
                b.insert("title".into(), t.clone());
            }
            let size = match (opt_str(a, "preset"), a.get("width").and_then(Value::as_f64), a.get("height").and_then(Value::as_f64)) {
                (Some(p), _, _) => match oc_schema::presets::find_preset(&p) {
                    Some(p) => Some((p.w, p.h, json!(p.unit))),
                    None => return tool_err(format!("Unknown size preset \"{p}\". Use list_size_presets for the ids.")),
                },
                (None, Some(w), Some(h)) => Some((w, h, a.get("unit").cloned().unwrap_or(json!("px")))),
                (None, None, None) => None,
                _ => return tool_err("Give both width and height (or a preset)."),
            };
            let resized = size.is_some();
            if let Some((w, h, unit)) = size {
                b.insert("w".into(), json!(w));
                b.insert("h".into(), json!(h));
                b.insert("unit".into(), unit);
                b.insert("scaleContent".into(), json!(a.get("scale_content").and_then(Value::as_bool).unwrap_or(false)));
            }
            let new_id = match crate::routes::designs::copy_design_for(s, u.id, &id, &b, true, &|_| {}).await? {
                Ok(id) => id,
                Err((_, body)) => return tool_err(format!("Couldn’t copy the design ({}).", body["error"].as_str().unwrap_or(""))),
            };
            let (view, pages) = match live::read_doc(s, &new_id, u.token_id).await? {
                Some(ydoc) => {
                    let (json, typed) = snapshot(&ydoc)?;
                    (Value::Object(design_view(&json, &typed, None)), super::view::sorted_page_ids(&typed))
                }
                None => (Value::Null, Vec::new()),
            };
            let mut out = json!({ "id": new_id, "editUrl": edit_url(s, &new_id), "resized": resized, "design": view });
            // A resize is where text stops fitting: check every page of the copy (at most 10).
            if resized
                && let Some(issues) = pages_check(s, &new_id, &pages).await
            {
                out["check"] = if issues.is_empty() { json!("No layout problems found on the resized pages.") } else { Value::Array(issues) };
            }
            Ok(text(&out))
        }
        "bulk_create" => {
            need(u, "designs:write")?;
            let id = a["design_id"].as_str().unwrap_or("").to_owned();
            access(s, u, &id, Role::View).await?;
            let rows: Vec<Map<String, Value>> = a["rows"].as_array().into_iter().flatten().filter_map(|r| r.as_object().cloned()).collect();
            let titles: Vec<String> = a["titles"].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(str::to_owned)).collect();
            let source_title: String = sqlx::query_scalar("SELECT title FROM designs WHERE id = $1").bind(&id).fetch_one(&s.pool).await.map_err(|e| Fail::Other(e.to_string()))?;
            let mut created = Vec::new();
            let mut matched: Map<String, Value> = Map::new();
            for (i, row) in rows.iter().enumerate() {
                let first = row.values().find_map(|v| v.as_str().filter(|t| !t.trim().is_empty())).map(|t| crate::js::take(t.trim(), 60));
                let title = titles.get(i).cloned().unwrap_or_else(|| match &first {
                    Some(v) => crate::js::take(&format!("{source_title} – {v}"), 200),
                    None => format!("{source_title} ({})", i + 1),
                });
                let counts = std::sync::Mutex::new(Map::new());
                let fill = |doc: &mut Value| *counts.lock().expect("counts") = super::edit::fill_row(doc, row);
                let b: Map<String, Value> = [("title".to_owned(), json!(title))].into_iter().collect();
                let new_id = match crate::routes::designs::copy_design_for(s, u.id, &id, &b, true, &fill).await? {
                    Ok(nid) => nid,
                    Err((_, body)) => return tool_err(format!("Couldn’t create copy {} ({}).", i + 1, body["error"].as_str().unwrap_or(""))),
                };
                let counts = counts.into_inner().expect("counts");
                for (k, n) in &counts {
                    let prev = matched.get(k).and_then(Value::as_u64).unwrap_or(0);
                    matched.insert(k.clone(), json!(prev + n.as_u64().unwrap_or(0)));
                }
                created.push(json!({ "id": new_id, "title": title, "editUrl": edit_url(s, &new_id), "filled": counts }));
            }
            let mut out = json!({ "created": created });
            let missing: Vec<&String> = matched.iter().filter(|(_, n)| n.as_u64() == Some(0)).map(|(k, _)| k).collect();
            if !missing.is_empty() {
                out["warning"] = json!(format!(
                    "Not found in the design (nothing was filled for): {}. Use the exact placeholder text, or bind elements to these columns.",
                    missing.iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(", ")
                ));
            }
            // Filled-in text is where boxes overflow: check the first page of the first three copies.
            let mut checks = Vec::new();
            for c in created.iter().take(3) {
                let cid = c["id"].as_str().unwrap_or("");
                if let Some(issues) = lint(s, &json!({ "designId": cid })).await
                    && !issues.is_empty()
                {
                    checks.push(json!({ "id": cid, "issues": issues }));
                }
            }
            if !checks.is_empty() {
                out["check"] = Value::Array(checks);
            }
            Ok(text(&out))
        }
        "get_design" => {
            need(u, "designs:read")?;
            let id = a["design_id"].as_str().unwrap_or("").to_owned();
            access(s, u, &id, Role::View).await?;
            let Some(ydoc) = live::read_doc(s, &id, u.token_id).await? else {
                return tool_err("This design has no content yet.");
            };
            let (json, typed) = snapshot(&ydoc)?;
            let page = a.get("page");
            let page_id = match page {
                Some(Value::Number(n)) => view::sorted_page_ids(&typed).get((n.as_f64().unwrap_or(0.0) as usize).wrapping_sub(1)).cloned(),
                Some(Value::String(p)) => Some(p.clone()),
                _ => None,
            };
            if let Some(p) = page
                && page_id.as_ref().is_none_or(|pid| json["pages"].get(pid).is_none())
            {
                return tool_err(format!("No page {}.", page_label(p)));
            }
            let mut out = Map::new();
            out.insert("id".into(), json!(id));
            out.insert("editUrl".into(), json!(edit_url(s, &id)));
            for (k, v) in design_view(&json, &typed, page_id.as_deref()) {
                out.insert(k, v);
            }
            if let Some(r) = super::brand::rules(s, u.id, &id).await? {
                out.insert("brand".into(), r.view());
            }
            Ok(text(&Value::Object(out)))
        }
        "edit_design" => edit_design(s, u, a).await,
        "screenshot_page" => {
            need(u, "designs:read")?;
            let id = a["design_id"].as_str().unwrap_or("").to_owned();
            access(s, u, &id, Role::View).await?;
            let page_id = match a.get("page") {
                Some(Value::String(p)) => Value::String(p.clone()),
                Some(p @ Value::Number(n)) => {
                    let found = match live::read_doc(s, &id, u.token_id).await? {
                        Some(ydoc) => {
                            let (_, typed) = snapshot(&ydoc)?;
                            view::sorted_page_ids(&typed).get((n.as_f64().unwrap_or(0.0) as usize).wrapping_sub(1)).cloned()
                        }
                        None => None,
                    };
                    match found {
                        Some(pid) => Value::String(pid),
                        None => return tool_err(format!("No page {}.", page_label(p))),
                    }
                }
                _ => Value::Null,
            };
            let res = render(s, "/internal/page-png", &json!({ "designId": id, "pageId": page_id, "w": a.get("width").cloned().unwrap_or(json!(1024)) }), 120_000).await?;
            let png = res.bytes().await.map_err(|e| Fail::Other(e.to_string()))?;
            Ok(json!({ "content": [{ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png" }] }))
        }
        "export_design" => {
            need(u, "exports")?;
            let id = a["design_id"].as_str().unwrap_or("").to_owned();
            access(s, u, &id, Role::View).await?;
            if let Some(g) = crate::brandhub::approval_gate(s, u.id, &id).await? {
                return tool_err(g["message"].as_str().unwrap_or(""));
            }
            let format = a["format"].as_str().unwrap_or("").to_owned();
            let mut body = Map::new();
            body.insert("designId".into(), json!(id));
            body.insert("format".into(), json!(format));
            if let Some(p) = a.get("pages").and_then(Value::as_array) {
                body.insert("pages".into(), Value::Array(p.iter().map(|x| super::edit::jnum(x.as_f64().unwrap_or(0.0) - 1.0)).collect()));
            }
            body.insert("scale".into(), a.get("scale").cloned().unwrap_or(json!(1)));
            body.insert("quality".into(), json!(90));
            body.insert("transparent".into(), json!(false));
            body.insert("embeds".into(), json!(crate::routes::docs::embed_allow_list(s, Some(u.id), &id).await?));
            let res = render(s, "/internal/export", &Value::Object(body), 300_000).await?;
            let ty = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_owned();
            let cd = res.headers().get("content-disposition").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
            let name = filename_of(&cd).unwrap_or_else(|| format!("design.{format}"));
            let bytes = res.bytes().await.map_err(|e| Fail::Other(e.to_string()))?;
            let sha = s.store.put(&bytes, Some(&ty)).await.map_err(|e| Fail::Other(e.to_string()))?;
            Ok(text(&json!({ "file": name, "size": bytes.len(), "downloadUrl": super::files::signed_file_url(s, &sha, &name, &ty, &super::files::FileOwner { user_id: u.id, grant_id: u.grant_id }) })))
        }
        "undo_ai_changes" => {
            need(u, "designs:write")?;
            let id = a["design_id"].as_str().unwrap_or("").to_owned();
            let agent = agent_for(s, u, &id).await?;
            let Some(cp) = live::latest_checkpoint(s, &id).await? else {
                return tool_err("There is no AI checkpoint for this design.");
            };
            let doc = oc_crdt::decode_doc_value(&cp.state)?;
            let actor = format!("ai:{}", agent.token_id);
            let now = s.now_ms() as f64;
            // No new checkpoint: undoing twice restores the same state.
            live::change(s, &id, &agent, |ydoc| oc_crdt::replace_content(ydoc, &doc, Some(oc_crdt::OcOrigin::Agent.origin()), &actor, now), |_| Presence::default(), false).await?;
            Ok(text(&json!({
                "restored": cp.label.unwrap_or_else(|| "checkpoint".into()),
                "savedAt": cp.created_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
                "editUrl": edit_url(s, &id),
            })))
        }
        _ => tool_err(format!("MCP error -32602: Tool {name} not found")),
    }
}

/// `/filename="?([^";]+)"?/` of a Content-Disposition header.
fn filename_of(cd: &str) -> Option<String> {
    let i = cd.find("filename=")?;
    let rest = cd[i + 9..].strip_prefix('"').unwrap_or(&cd[i + 9..]);
    let name: String = rest.chars().take_while(|c| *c != '"' && *c != ';').collect();
    (!name.is_empty()).then_some(name)
}

/// A number as JS prints it.
fn jsn(v: &Value) -> String {
    crate::js::num(v.as_f64().unwrap_or(f64::NAN))
}

async fn edit_design(s: &AppState, u: &BearerUser, a: &Value) -> R {
    need(u, "designs:write")?;
    let id = a["design_id"].as_str().unwrap_or("").to_owned();
    let agent = agent_for(s, u, &id).await?;
    let ops = a["operations"].as_array().cloned().unwrap_or_default();
    // The team's brand controls, as the editor applies them to its pickers: nothing is applied if an operation breaks one.
    if let Some(r) = super::brand::rules(s, u.id, &id).await?
        && let Some(m) = r.check(&ops)
    {
        return tool_err(m);
    }
    // Images first (network), so the edit itself runs in one go.
    let doc_assets = match live::read_doc(s, &id, u.token_id).await? {
        Some(ydoc) => oc_crdt::from_y_value(&ydoc)["assets"].as_object().cloned().unwrap_or_default(),
        None => Map::new(),
    };
    let mut images = HashMap::new();
    for (i, op) in ops.iter().enumerate() {
        let src = match op["op"].as_str() {
            Some("add_image") => op.get("source"),
            Some("update") => op.get("image"),
            _ => None,
        };
        if let Some(src) = src {
            match import_image(s, u, src, &doc_assets).await {
                Ok(img) => {
                    images.insert(i, img);
                }
                Err(Fail::Tool(m)) | Err(Fail::Other(m)) => return tool_err(format!("Operation {} ({}): {m}", i + 1, op["op"].as_str().unwrap_or(""))),
            }
        }
    }
    let actor = format!("ai:{}", agent.token_id);
    let now = s.now_ms() as f64;
    let brand_admin = agent.brand_admin;
    let r = live::change(
        s,
        &id,
        &agent,
        move |ydoc| {
            let ctx = EditContext { ydoc, actor, brand_admin, images, now };
            let res = run_edits(&ops, &ctx);
            let pages = {
                use yrs::{Map as _, Transact as _};
                let pages = oc_crdt::roots(ydoc).pages;
                let txn = ydoc.transact();
                pages.len(&txn)
            };
            (res, pages)
        },
        |(v, _)| Presence { selection: Some(v.touched.clone()), page_id: v.page_id.clone() },
        true,
    )
    .await?;
    let (v, pages) = r.value;
    match &v.title {
        Some(t) => pg::exec(&s.pool, "UPDATE designs SET page_count = $2 , title = $3 WHERE id = $1", &[pg::text(&id), pg::Arg::Int(Some(i64::from(pages))), pg::text(t)]).await?,
        None => pg::exec(&s.pool, "UPDATE designs SET page_count = $2 WHERE id = $1", &[pg::text(&id), pg::Arg::Int(Some(i64::from(pages)))]).await?,
    };
    let mut out = Map::new();
    out.insert("applied".into(), json!(format!("{} of {} operations", v.applied, v.total)));
    if !v.refs.is_empty() {
        out.insert("refs".into(), Value::Object(v.refs.clone()));
    }
    out.insert("created".into(), json!(v.created.iter().flatten().collect::<Vec<_>>()));
    if v.replaced > 0 {
        out.insert("replaced".into(), json!(v.replaced));
    }
    if let Some((index, op, message)) = &v.error {
        let earlier = if v.applied > 0 { " Earlier operations were applied." } else { "" };
        out.insert("error".into(), json!(format!("Operation {} ({op}) failed: {message}{earlier}", index + 1)));
    }
    if !r.acknowledged {
        out.insert("warning".into(), json!("The change was sent but not yet confirmed as saved; check again in a moment."));
    }
    // Layout checks of the pages the edit touched (the render service lays text out and looks behind it), so the
    // assistant can fix overflowing or unreadable text without a screenshot. Left out if the render service can't
    // answer; then the assistant is pointed at screenshot_page as before.
    let check = if v.applied > 0 { page_check(s, &id, v.page_id.as_deref(), &v.touched).await } else { None };
    let next = match &check {
        Some(issues) if issues.is_empty() => {
            out.insert("check".into(), json!("No layout problems found (text fits, stays on the page and is readable)."));
            "Look at it with screenshot_page if you want to judge the design itself."
        }
        Some(issues) => {
            out.insert("check".into(), Value::Array(issues.clone()));
            "Fix the problems listed in check with another edit_design, then confirm with screenshot_page."
        }
        None => "Check the result with screenshot_page.",
    };
    out.insert("next".into(), json!(next));
    let mut res = text(&Value::Object(out));
    if v.error.is_some() {
        res["isError"] = json!(v.applied == 0);
    }
    Ok(res)
}

/// Error results.
pub fn failure(name: &str, f: Fail) -> Value {
    let message = match f {
        Fail::Tool(m) => m,
        Fail::Other(e) => {
            tracing::error!("[mcp] {name}: {e}");
            "Something went wrong on the server. Try again.".into()
        }
    };
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

/// A kit asset as the assistant sees it: `{ asset, name }`.
fn named_asset(id: &Value, names: &HashMap<String, String>) -> Value {
    let name = id.as_str().and_then(|i| names.get(i)).map_or("", String::as_str);
    json!({ "asset": id, "name": name })
}

/// The kit named `kit`: an exact match (ignoring case), else the only kit whose name contains it.
fn find_kit<'a>(kits: &'a [Value], kit: &str) -> Result<&'a Value, String> {
    let want = kit.trim().to_lowercase();
    let name = |k: &Value| k["name"].as_str().unwrap_or("").to_lowercase();
    if let Some(k) = kits.iter().find(|k| name(k) == want) {
        return Ok(k);
    }
    let close: Vec<&Value> = if want.is_empty() { Vec::new() } else { kits.iter().filter(|k| name(k).contains(&want)).collect() };
    match close.as_slice() {
        [k] => Ok(k),
        _ => {
            let all: Vec<&str> = kits.iter().filter_map(|k| k["name"].as_str()).collect();
            Err(format!("No single brand kit matches \"{kit}\". Brand kits: {}", all.join(", ")))
        }
    }
}

/// `add_image { source: { brand: { kit, name } } }`: the id of the kit's logo, graphic or photo with that file name.
/// A wrong name lists the kit's files, so the assistant can correct it instead of drawing a stand-in.
async fn kit_asset_by_name(s: &AppState, kits: &[Value], kit: &str, name: &str) -> Result<String, Fail> {
    let k = find_kit(kits, kit).map_err(Fail::Tool)?;
    let ids: Vec<String> = ["logos", "graphics", "photos"]
        .iter()
        .filter_map(|key| k[*key].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let files: Vec<(String, Option<String>)> = sqlx::query_as("SELECT id, name FROM assets WHERE id = ANY($1) AND trashed_at IS NULL")
        .bind(&ids)
        .fetch_all(&s.pool)
        .await
        .map_err(|e| Fail::Other(e.to_string()))?;
    // Only the file name counts, so a design system path like assets/underlines/hullam.png matches too.
    let base = |n: &str| n.rsplit('/').next().unwrap_or(n).to_lowercase();
    let want = base(name.trim());
    if let Some((id, _)) = files.iter().find(|(_, n)| n.as_deref().map(base).as_deref() == Some(want.as_str())) {
        return Ok(id.clone());
    }
    let mut names: Vec<&str> = files.iter().filter_map(|(_, n)| n.as_deref()).collect();
    names.sort_unstable();
    Err(Fail::Tool(format!("Brand kit \"{}\" has no file named \"{name}\". Its files: {}", k["name"].as_str().unwrap_or(""), names.join(", "))))
}

#[cfg(test)]
mod kit_lookup_tests {
    use super::*;

    #[test]
    fn finds_a_kit_by_exact_or_only_partial_name() {
        let kits = vec![json!({ "name": "Amigos" }), json!({ "name": "Amigos alt green" }), json!({ "name": "Horror Weeks" })];
        assert_eq!(find_kit(&kits, "amigos").unwrap()["name"], "Amigos");
        assert_eq!(find_kit(&kits, "alt green").unwrap()["name"], "Amigos alt green");
        assert_eq!(find_kit(&kits, "HORROR").unwrap()["name"], "Horror Weeks");
    }

    #[test]
    fn an_unclear_or_unknown_kit_lists_the_kits() {
        let kits = vec![json!({ "name": "Amigos one" }), json!({ "name": "Amigos two" })];
        assert!(find_kit(&kits, "amigos").unwrap_err().contains("Amigos one, Amigos two"));
        assert!(find_kit(&kits, "peritus").is_err());
        assert!(find_kit(&kits, " ").is_err());
    }
}
