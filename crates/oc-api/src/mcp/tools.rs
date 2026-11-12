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

/// A failure the assistant is told about (`ToolError`), or an unexpected one.
pub enum Fail {
    Tool(String),
    Other(String),
}

impl From<ApiError> for Fail {
    fn from(e: ApiError) -> Self {
        Fail::Other(format!("{e:?}"))
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

pub(super) type R = Result<Value, Fail>;

pub(super) fn tool_err<T>(m: impl Into<String>) -> Result<T, Fail> {
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

pub(super) fn need(u: &BearerUser, scope: &str) -> Result<(), Fail> {
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
/// What a file kind is called in errors.
fn kind_word(want: &str) -> &'static str {
    match want {
        "video" => "video",
        "audio" => "sound",
        _ => "image",
    }
}

/// AssetRef JSON of an asset row or ingest result: media keep their length and sound; sounds have no size.
fn media_ref(mut a: Value, want: &str) -> Value {
    if want == "image" {
        return a;
    }
    for k in ["w", "h"] {
        if !a[k].is_number() {
            a[k] = json!(0);
        }
    }
    if let Some(m) = a.as_object_mut() {
        m.retain(|_, v| !v.is_null());
    }
    a
}

/// An image, video or sound for an operation (`want`: "image", "video" or "audio"), from the user's uploads, a brand
/// kit, stock, or a URL, as the design's AssetRef JSON.
async fn import_media(s: &AppState, u: &BearerUser, src: &Value, doc_assets: &Map<String, Value>, want: &str) -> Result<Value, Fail> {
    let fits = |kind: &Value| match want {
        "image" => kind == "image" || kind == "svg",
        k => kind == k,
    };
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
        if let Some(a) = doc_assets.get(id).filter(|a| fits(&a["kind"])) {
            return Ok(a.clone());
        }
        let a = pg::json_one(
            &s.pool,
            "SELECT id, kind, sha256, mime, width, height, name, duration_ms, has_audio FROM assets WHERE id = $1 AND trashed_at IS NULL
       AND (owner_id = $2 OR workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $2)
            -- A brand kit's logos, photos and graphics are for everyone who can see the kit, wherever they were uploaded.
            OR EXISTS (SELECT 1 FROM brand_kits k JOIN memberships m ON m.workspace_id = k.workspace_id AND m.user_id = $2
                       WHERE k.logos ? assets.id OR k.photos ? assets.id OR k.graphics ? assets.id))",
            &[pg::text(id), pg::uuid(u.id)],
        )
        .await?;
        let Some(a) = a else {
            return tool_err(format!("No {} asset {id}. Use list_uploads or get_brand_kit for asset ids.", kind_word(want)));
        };
        if !fits(&a["kind"]) {
            return tool_err(format!("{id} is a {} file, not a {}.", a["kind"].as_str().unwrap_or("other"), kind_word(want)));
        }
        let mut r = json!({ "id": a["id"], "kind": a["kind"], "sha256": a["sha256"], "mime": a["mime"], "w": a["width"], "h": a["height"], "name": a["name"] });
        if want != "image" {
            r["durationMs"] = a["duration_ms"].clone();
            r["hasAudio"] = a["has_audio"].clone();
        }
        return Ok(media_ref(r, want));
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
            let kind = st.get("type").and_then(Value::as_str).unwrap_or(want);
            let m = p.media(s, kind, st["id"].as_str().unwrap_or("")).await.map_err(Fail::Other)?;
            crate::routes::content::download(&m.url, |h| p.host_allowed(h), &file).await.map_err(Fail::Other)?;
            attribution = Some(m.attribution);
            name = m.filename;
            source = if kind == "gif" || kind == "sticker" { "gif" } else { "stock" };
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
                if !fits(&a["kind"]) {
                    return tool_err(format!("That file is not {}.", if want == "image" { "an image" } else if want == "video" { "a video" } else { "a sound" }));
                }
                Ok(media_ref(a, want))
            }
        }
    };
    match run.await {
        Ok(a) => Ok(a),
        Err(Fail::Tool(m)) => Err(Fail::Tool(m)),
        Err(Fail::Other(m)) => tool_err(format!("Couldn’t import the {}: {m}", kind_word(want))),
    }
}

/// Base64 passes through the model's context: about 85,000 characters here, so only tiny files.
const MAX_INLINE_IMAGE_BYTES: usize = 64 * 1024;
const USE_LINK: &str = "Base64 has to pass through your context. Use create_upload_link.";

/// An image sent through the tool call itself: the user's attachment as the chat app passes it (ChatGPT fills `file`
/// in with a download link for a tool that declares it under `_meta["openai/fileParams"]`), or a small file as
/// base64. Stored by the normal ingest (sniffing, hash gate, quota).
async fn upload_image(s: &AppState, u: &BearerUser, a: &Value) -> R {
    use crate::routes::assets::{Ingest, IngestOpts, TempDir, ingest_file, sniff};
    need(u, "assets:write")?;
    let dir = TempDir::new("oc-mcp-").await.map_err(|e| Fail::Other(e.to_string()))?;
    let file = dir.0.join("src");
    let mut name = opt_str(a, "filename");
    if let Some(f) = a.get("file").filter(|f| !f.is_null()) {
        let url = f.get("download_url").and_then(Value::as_str).or_else(|| f.as_str()).unwrap_or("").trim().to_owned();
        if !url.starts_with("https://") {
            return tool_err("file must be the attachment as the chat app passes it to tools (with a download_url). If this app doesn't pass attachments, use create_upload_link.");
        }
        if name.is_none() {
            name = f.get("file_name").and_then(Value::as_str).map(str::to_owned);
        }
        crate::routes::content::download(&url, |_| true, &file).await.map_err(|m| Fail::Tool(format!("Couldn’t fetch the file: {m}")))?;
    } else {
        let data = a["data_base64"].as_str().unwrap_or("");
        if data.is_empty() {
            return tool_err("Pass file (the attachment, where the chat app passes attachments to tools) or data_base64 (a file up to 64 KB). For anything larger, use create_upload_link.");
        }
        // Decoded size, from the encoded length, before decoding anything too large.
        if data.len() / 4 * 3 > MAX_INLINE_IMAGE_BYTES + 3 {
            return tool_err(format!("That file is over the 64 KB limit of upload_image. {USE_LINK}"));
        }
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data.trim()) else {
            return tool_err("data_base64 isn't valid base64.");
        };
        if bytes.len() > MAX_INLINE_IMAGE_BYTES {
            return tool_err(format!("That file is over the 64 KB limit of upload_image. {USE_LINK}"));
        }
        tokio::fs::write(&file, &bytes).await.map_err(|e| Fail::Other(e.to_string()))?;
    }
    // Only images here, checked before anything is stored.
    let head = {
        use tokio::io::AsyncReadExt as _;
        let mut buf = vec![0u8; 4096];
        let mut f = tokio::fs::File::open(&file).await.map_err(|e| Fail::Other(e.to_string()))?;
        let mut off = 0;
        while off < buf.len() {
            let n = f.read(&mut buf[off..]).await.map_err(|e| Fail::Other(e.to_string()))?;
            if n == 0 {
                break;
            }
            off += n;
        }
        buf.truncate(off);
        buf
    };
    if !matches!(sniff(&head), Some((_, "image" | "svg"))) {
        return tool_err("That file is not an image (SVG, PNG, JPEG, WebP or GIF).");
    }
    let r = ingest_file(s, &file, &dir.0, IngestOpts { name: name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or("image"), owner_id: u.id, ip: None, source: None, attribution: None, skip_quota: false }).await?;
    match r {
        Ingest::Error { error, message, .. } => tool_err(message.unwrap_or_else(|| error.to_owned())),
        Ingest::Asset(asset) => Ok(text(&json!({ "asset": asset["id"], "name": asset["name"], "w": asset["w"], "h": asset["h"], "next": "Place it with add_image { asset }." }))),
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

pub(super) fn opt_str(a: &Value, k: &str) -> Option<String> {
    a.get(k).and_then(Value::as_str).map(str::to_owned)
}

fn trimmed_query(a: &Value) -> Option<String> {
    opt_str(a, "query").map(|q| crate::js::trim(&q).to_owned()).filter(|q| !q.is_empty())
}

#[derive(Default)]
struct Fonts(OnceLock<oc_gfonts::GoogleFonts>);

/// What to do with a design's brand kit (create_design and get_design).
const KIT_NEXT: &str = "If the kit lists guidelines, read them first with get_brand_guidelines and follow them throughout. Design with this kit: its fonts for each role, its textStyles for every text (weight, italic, capitals, letterSpacing, lineHeight, colours; edit_design's check flags text that strays), only its colours, its logo, and its graphics and photos via add_image { asset } (or { brand: { kit, name } }). Follow its voice for every word. Never draw a stand-in for a brand element the kit has. If missingFonts is set, tell the user which font files to upload.";

fn gfonts(s: &AppState) -> oc_gfonts::GoogleFonts {
    s.local::<Fonts>().0.get_or_init(|| oc_gfonts::GoogleFonts::new(s.paths.fonts.clone(), s.pool.clone(), s.store.clone())).clone()
}

/// Lower-case names of every family the editor can draw for this user: bundled, Google and the fonts uploaded to
/// their workspaces (what `/api/fonts/manifest.json` gives the editor).
async fn usable_families(s: &AppState, user_id: uuid::Uuid) -> Result<std::collections::HashSet<String>, Fail> {
    let manifest = gfonts(s).full_manifest().await.map_err(|e| Fail::Other(e.to_string()))?;
    let mut set: std::collections::HashSet<String> = manifest.families.iter().map(|f| f.family.to_lowercase()).collect();
    let uploaded: Vec<String> = sqlx::query_scalar("SELECT DISTINCT f.family FROM custom_fonts f JOIN memberships m ON m.workspace_id = f.workspace_id WHERE m.user_id = $1")
        .bind(user_id)
        .fetch_all(&s.pool)
        .await?;
    set.extend(uploaded.into_iter().map(|f| f.to_lowercase()));
    Ok(set)
}

/// Kit fonts (by role) that are neither a Google or bundled font nor uploaded where this user can use them. The
/// editor would draw a fallback for them, so the assistant must say so instead of using them silently.
fn missing_fonts(kit: &Value, usable: &std::collections::HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for family in kit["fonts"].as_object().into_iter().flat_map(|f| f.values()).filter_map(Value::as_str) {
        let f = family.trim();
        if !f.is_empty() && !usable.contains(&f.to_lowercase()) && !out.iter().any(|o| o.eq_ignore_ascii_case(f)) {
            out.push(f.to_owned());
        }
    }
    out
}

/// What the assistant sees of brand kits (`get_brand_kit`, and the chosen kit in `create_design` and `get_design`):
/// colours, fonts by role, logos, graphics and photos with their file names, voice, and fonts that won't render.
pub(super) async fn kit_views(s: &AppState, user_id: uuid::Uuid, kits: &[&Value]) -> Result<Vec<Value>, Fail> {
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
    let usable = usable_families(s, user_id).await?;
    let kit_ids: Vec<String> = kits.iter().filter_map(|k| k["id"].as_str().map(str::to_owned)).collect();
    let outlines = crate::routes::brandhub::guideline_outlines(s, &kit_ids).await?;
    Ok(kits
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
            // How each kind of text looks beyond its font: edit_design's check holds text to these.
            let styles = super::brand::styles_view(k);
            if !styles.is_empty() {
                m.insert("textStyles".into(), Value::Array(styles));
            }
            let missing = missing_fonts(k, &usable);
            if !missing.is_empty() {
                m.insert(
                    "missingFonts".into(),
                    json!({
                        "families": missing,
                        "note": "These kit fonts aren't uploaded, so the editor shows a fallback font in their place. Still use them for their roles (the design switches to them once they're uploaded), and tell the user plainly that the font files must be uploaded in Brand Hub (the kit's Fonts) before the design looks right.",
                    }),
                );
            }
            for (key, cap) in [("logos", 30), ("graphics", 60), ("photos", 30)] {
                let items = k[key].as_array().map(|ids| ids.iter().take(cap).map(|id| named_asset(id, &names)).collect()).unwrap_or_default();
                m.insert(key.into(), Value::Array(items));
            }
            if view::truthy(k.get("voice")) {
                m.insert("voice".into(), k["voice"].clone());
            }
            // How the brand moves (ms, px): edit_design's animation operations use these as their defaults.
            if k["motion"].as_object().is_some_and(|m| !m.is_empty()) {
                m.insert("motion".into(), k["motion"].clone());
            }
            // The kit's brand book: only its outline here; get_brand_guidelines returns the text.
            if let Some(o) = k["id"].as_str().and_then(|id| outlines.get(id)).filter(|o| !o.is_empty()) {
                m.insert("guidelines".into(), json!({ "sections": o, "read": "Call get_brand_guidelines { brand_kit } and follow them before designing." }));
            }
            Value::Object(m)
        })
        .collect())
}

/// Every live element on the shown pages (one page, or all), in page and stacking order, as (id, raw element).
fn text_elements<'a>(doc: &'a Value, typed: &oc_schema::design::DesignDoc, page_id: Option<&str>) -> Vec<(String, &'a Value)> {
    let mut out = Vec::new();
    for pid in view::sorted_page_ids(typed).into_iter().filter(|p| page_id.is_none_or(|q| q == p)) {
        let mut stack = view::children(typed, &pid);
        stack.reverse();
        while let Some(id) = stack.pop() {
            let mut kids = view::children(typed, &id);
            kids.reverse();
            stack.extend(kids);
            if let Some(el) = doc["elements"].get(&id).filter(|e| e["type"] == "text") {
                out.push((id, el));
            }
        }
    }
    out
}

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
        "list_planner_posts" => {
            need(u, "designs:read")?;
            // One brand at a time: a team by name or id, or "personal" for posts outside every team.
            let want = opt_str(a, "team").map(|t| crate::js::trim(&t).to_lowercase()).filter(|t| !t.is_empty());
            let personal = want.as_deref() == Some("personal");
            let team: Option<uuid::Uuid> = match want.as_deref().filter(|_| !personal) {
                Some(t) => {
                    let teams: Vec<(uuid::Uuid, String)> = sqlx::query_as(
                        "SELECT w.id, w.name FROM workspaces w JOIN memberships m ON m.workspace_id = w.id
                         WHERE m.user_id = $1 AND NOT w.personal ORDER BY w.name",
                    )
                    .bind(u.id)
                    .fetch_all(&s.pool)
                    .await
                    .map_err(ApiError::from)?;
                    match teams.iter().find(|(id, name)| id.to_string() == t || name.to_lowercase() == t) {
                        Some((id, _)) => Some(*id),
                        None => {
                            let names: Vec<&str> = teams.iter().map(|(_, n)| n.as_str()).collect();
                            return tool_err(if names.is_empty() {
                                "The user has no teams. Leave team out, or pass \"personal\".".to_owned()
                            } else {
                                format!("No team {t}. The user's teams: {}. Pass one of them, or \"personal\".", names.join(", "))
                            });
                        }
                    }
                }
                None => None,
            };
            let rows: Vec<oc_social::publish::PostRow> = sqlx::query_as(
                "SELECT p.* FROM social_posts p
                 WHERE (p.owner_id = $1 OR p.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1))
                   AND ($2::text IS NULL OR p.status = $2)
                   AND ($3::timestamptz IS NULL OR p.scheduled_at >= $3::timestamptz)
                   AND ($4::timestamptz IS NULL OR p.scheduled_at < $4::timestamptz)
                   AND ($5::uuid IS NULL OR p.workspace_id = $5)
                   AND (NOT $6 OR p.workspace_id IS NULL)
                 ORDER BY p.scheduled_at NULLS LAST, p.created_at LIMIT 200",
            )
            .bind(u.id)
            .bind(opt_str(a, "status"))
            .bind(opt_str(a, "from"))
            .bind(opt_str(a, "to"))
            .bind(team)
            .bind(personal)
            .fetch_all(&s.pool)
            .await
            .map_err(ApiError::from)?;
            let mut posts = Vec::with_capacity(rows.len());
            for p in &rows {
                posts.push(crate::routes::v1_planner::out(s, p).await?);
            }
            Ok(text(&json!({ "posts": posts })))
        }
        "get_planner_post" => {
            need(u, "designs:read")?;
            let key = opt_str(a, "post").unwrap_or_default();
            match crate::routes::v1_planner::find_for(s, u.id, None, &key).await? {
                Ok(Some(p)) => Ok(text(
                    &json!({ "post": crate::routes::v1_planner::out(s, &p).await? }),
                )),
                Ok(None) => tool_err(format!("No planner post {key}.")),
                Err(()) => tool_err(format!(
                    "Several posts use the post ID {key}; use the post's id."
                )),
            }
        }
        "create_planner_post" | "update_planner_post" => {
            need(u, "designs:write")?;
            // The planner's web rules apply as they are: an assistant drafts and submits, a person approves.
            let mut body = Map::new();
            for (from, to) in [
                ("channels", "channels"),
                ("scheduled_at", "scheduledAt"),
                ("timezone", "timezone"),
                ("caption", "caption"),
                ("fields", "fields"),
            ] {
                if let Some(v) = a.get(from) {
                    body.insert(to.into(), v.clone());
                }
            }
            if a.get("submit").and_then(Value::as_bool) == Some(true) {
                body.insert("status".into(), json!("in_review"));
            }
            let outcome = if name == "create_planner_post" {
                body.insert("designId".into(), a["design"].clone());
                body.insert("publisher".into(), json!("external"));
                body.insert("format".into(), json!("jpg"));
                body.entry("status").or_insert(json!("draft"));
                crate::routes::social::create_post_as(s, u.id, Some(&Value::Object(body)), None)
                    .await?
            } else {
                let key = opt_str(a, "post").unwrap_or_default();
                let post = match crate::routes::v1_planner::find_for(s, u.id, None, &key).await? {
                    Ok(Some(p)) => p,
                    Ok(None) => return tool_err(format!("No planner post {key}.")),
                    Err(()) => {
                        return tool_err(format!(
                            "Several posts use the post ID {key}; use the post's id."
                        ));
                    }
                };
                crate::routes::social::update_post_as(s, u.id, post, Some(&Value::Object(body)))
                    .await?
            };
            match outcome {
                Ok(p) => Ok(text(
                    &json!({ "post": crate::routes::v1_planner::out(s, &p).await? }),
                )),
                Err(refusal) => tool_err(refusal.message()),
            }
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
                    for k in ["provider", "id", "title", "w", "h", "thumb", "license", "duration"] {
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
            // Images by default; videos and sounds for add_video and add_audio.
            let kinds = match a.get("type").and_then(Value::as_str) {
                Some("video") => "kind = 'video'",
                Some("audio") => "kind = 'audio'",
                Some("all") => "kind IN ('image','svg','video','audio')",
                _ => "kind IN ('image','svg')",
            };
            let rows = pg::json_rows(
                &s.pool,
                &format!("SELECT id, name, kind, width, height, duration_ms, created_at FROM assets WHERE owner_id = $1 AND trashed_at IS NULL AND {kinds} AND source <> 'emoji'
       AND ($2::text IS NULL OR name ILIKE '%' || $2 || '%' ESCAPE '\\') ORDER BY created_at DESC LIMIT $3"),
                &[pg::uuid(u.id), pg::opt_text(q), pg::Arg::Int(Some(a.get("limit").and_then(Value::as_f64).unwrap_or(30.0) as i64))],
            )
            .await?;
            let list: Vec<Value> = rows
                .iter()
                .map(|r| {
                    let mut o = json!({ "asset": r["id"], "name": r["name"], "w": r["width"], "h": r["height"], "uploaded": r["created_at"] });
                    if r["kind"] == "video" || r["kind"] == "audio" {
                        o["kind"] = r["kind"].clone();
                        o["duration"] = super::motion::secs(view::num(&r["duration_ms"]));
                        if r["kind"] == "audio"
                            && let Some(m) = o.as_object_mut()
                        {
                            m.remove("w");
                            m.remove("h");
                        }
                    }
                    o
                })
                .collect();
            Ok(text(&json!({ "uploads": list })))
        }
        "upload_image" => upload_image(s, u, a).await,
        "create_upload_link" => {
            need(u, "assets:write")?;
            let Some(link) = crate::routes::upload_links::create(s, u.id, opt_str(a, "filename").as_deref(), opt_str(a, "purpose").as_deref()).await? else {
                return tool_err("Too many upload links in the last hour. Try again later.");
            };
            Ok(text(&json!({
                "id": link.id.to_string(),
                "url": link.url,
                "expiresAt": oc_js::iso_string(&link.expires_at),
                "singleUse": true,
                "send": format!("curl -sS -T <file> \"{}\"", link.url),
                "next": "Send the file yourself if you can run commands or make HTTP requests (a file attached to the chat is usually at /mnt/user-data/uploads/<name>): the reply names the asset, which you place with add_image { asset }. If you can't, give the user the url: the page takes the file, and get_upload_link with id then tells you the asset.",
            })))
        }
        "get_upload_link" => {
            need(u, "assets:write")?;
            let id = a["id"].as_str().and_then(|i| uuid::Uuid::parse_str(i).ok()).unwrap_or_default();
            match crate::routes::upload_links::status(s, u.id, id).await? {
                None => tool_err("No upload link with that id."),
                Some((status, assets)) => {
                    let mut out = json!({ "status": status });
                    if let Some(a) = assets.first() {
                        out["asset"] = json!(a);
                        out["assets"] = json!(assets);
                        out["next"] = json!("Place it with add_image { asset }.");
                    }
                    Ok(text(&out))
                }
            }
        }
        "get_brand_kit" => {
            need(u, "designs:read")?;
            let kits = crate::routes::brandhub::accessible_kits(s, u.id).await?;
            if kits.is_empty() {
                return Ok(text(&json!("No brand kit yet. Follow the design system the user selected or named, if you can read it; otherwise pick colours and fonts that suit the request. Check list_uploads for their logo and images.")));
            }
            let list = kit_views(s, u.id, &kits.iter().collect::<Vec<_>>()).await?;
            Ok(text(&json!({
                "kits": list,
                "next": "Pass the kit's name as brand_kit to create_design, so the design is made where the kit and its fonts live. A kit that lists guidelines has a written brand book: read it with get_brand_guidelines before designing.",
            })))
        }
        "create_brand_kit" => super::brand_write::create(s, u, a).await,
        "update_brand_kit" => super::brand_write::update(s, u, a).await,
        "get_brand_guidelines" => {
            need(u, "designs:read")?;
            let kits = crate::routes::brandhub::accessible_kits(s, u.id).await?;
            let k = match find_kit(&kits, a["brand_kit"].as_str().unwrap_or("")) {
                Ok(k) => k,
                Err(e) => return tool_err(e),
            };
            let all = crate::routes::brandhub::kit_guidelines(s, k["id"].as_str().unwrap_or("")).await?;
            Ok(text(&guidelines_view(k["name"].clone(), &all, a.get("sections").and_then(Value::as_array))))
        }
        "search_fonts" => {
            let g = gfonts(s);
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
            // A team's kit puts the design in that team, where the kit's fonts, templates and brand rules apply.
            let want = opt_str(a, "brand_kit").filter(|k| !k.trim().is_empty());
            let kits = if want.is_some() { crate::routes::brandhub::accessible_kits(s, u.id).await? } else { Vec::new() };
            let kit = match &want {
                Some(w) => Some(find_kit(&kits, w).map_err(Fail::Tool)?),
                None => None,
            };
            if let Some(k) = kit {
                if !view::truthy(k.get("personal"))
                    && let Some(ws) = k["workspaceId"].as_str()
                {
                    b.insert("workspaceId".into(), json!(ws));
                }
                b.insert("brandKit".into(), json!({ "id": k["id"], "name": k["name"] }));
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
            let mut out = json!({ "id": id, "editUrl": edit_url(s, &id), "design": view });
            match kit {
                Some(k) => {
                    out["brandKit"] = kit_views(s, u.id, &[k]).await?.pop().unwrap_or(Value::Null);
                    out["next"] = json!(KIT_NEXT);
                }
                None => {
                    // Assistants that skipped get_brand_kit learn here that the user has kits to design with.
                    let names: Vec<Value> = crate::routes::brandhub::accessible_kits(s, u.id).await?.iter().map(|k| k["name"].clone()).collect();
                    if !names.is_empty() {
                        out["brandKits"] = json!(names);
                        out["next"] = json!("No brand kit was named. If this design is for one of these brands, or for a design system the user selected, call get_brand_kit and use that kit's fonts, colours, logo and graphics (create_design with brand_kit links the design to it).");
                    }
                }
            }
            Ok(text(&out))
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
            // Timing problems of the pages shown (and of the whole video).
            let shown: Option<Vec<String>> = page_id.clone().map(|p| vec![p]);
            let video = super::motion::video_checks(&json, &typed, shown.as_deref());
            if !video.is_empty() {
                out.insert("videoCheck".into(), Value::Array(video));
            }
            let motion = super::brand::motion_kit(s, u.id, &id, &json).await?;
            if motion.m.as_object().is_some_and(|m| !m.is_empty()) {
                out.insert("motion".into(), json!({ "kit": motion.name, "rules": motion.m, "note": "Animations and page transitions in edit_design take these as their defaults (times in ms, travel in px)." }));
            }
            // Text that strays from the kit's fonts and text styles, on the pages shown.
            if let Some(k) = super::brand::style_kit(s, u.id, &id, &json).await? {
                let issues = k.check(text_elements(&json, &typed, page_id.as_deref()).iter().map(|(i, e)| (i.as_str(), *e)));
                if !issues.is_empty() {
                    out.insert("styleCheck".into(), json!({ "kit": k.name, "issues": issues, "fix": "Set these with edit_design (update: weight, italic, case, letterSpacing, lineHeight, color) so the text follows the kit's textStyles." }));
                }
            }
            // The kit the design was made with (create_design { brand_kit }), while the user can still use it.
            if let Some(kit_id) = json.pointer("/meta/brandKit/id").and_then(Value::as_str) {
                let kits = crate::routes::brandhub::accessible_kits(s, u.id).await?;
                if let Some(k) = kits.iter().find(|k| k["id"].as_str() == Some(kit_id)) {
                    out.insert("brandKit".into(), kit_views(s, u.id, &[k]).await?.pop().unwrap_or(Value::Null));
                    out.insert("next".into(), json!(KIT_NEXT));
                }
            }
            Ok(text(&Value::Object(out)))
        }
        "edit_design" => edit_design(s, u, a).await,
        "screenshot_page" => {
            need(u, "designs:read")?;
            let id = a["design_id"].as_str().unwrap_or("").to_owned();
            access(s, u, &id, Role::View).await?;
            let timed = a.get("time").is_some() || a.get("frames").is_some() || a.get("times").is_some();
            // The design is read when a page number must be resolved or the page's length is needed.
            let typed = if matches!(a.get("page"), Some(Value::Number(_))) || timed {
                match live::read_doc(s, &id, u.token_id).await? {
                    Some(ydoc) => Some(snapshot(&ydoc)?.1),
                    None => None,
                }
            } else {
                None
            };
            let page_id = match a.get("page") {
                Some(Value::String(p)) => Value::String(p.clone()),
                Some(p @ Value::Number(n)) => match typed.as_ref().and_then(|t| view::sorted_page_ids(t).get((n.as_f64().unwrap_or(0.0) as usize).wrapping_sub(1)).cloned()) {
                    Some(pid) => Value::String(pid),
                    None => return tool_err(format!("No page {}.", page_label(p))),
                },
                _ => Value::Null,
            };
            if !timed {
                let res = render(s, "/internal/page-png", &json!({ "designId": id, "pageId": page_id, "w": a.get("width").cloned().unwrap_or(json!(1024)) }), 120_000).await?;
                let png = res.bytes().await.map_err(|e| Fail::Other(e.to_string()))?;
                return Ok(json!({ "content": [{ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png" }] }));
            }
            // Moments of the page's playback: one (`time`), evenly spread frames ending on its last moment (`frames`), or
            // the given ones (`times`), in seconds of page time.
            let len = typed.as_ref().map_or(oc_schema::timeline::DEFAULT_PAGE_MS, |t| {
                let tl = oc_schema::timeline::timeline_of(t, true);
                let pid = page_id.as_str().map(str::to_owned).or_else(|| view::sorted_page_ids(t).first().cloned());
                tl.spans.iter().find(|sp| Some(sp.page.id.to_string()) == pid).map_or(oc_schema::timeline::DEFAULT_PAGE_MS, |sp| sp.duration_ms)
            });
            let times: Vec<f64> = if let Some(t) = a.get("time").and_then(Value::as_f64) {
                vec![t * 1000.0]
            } else if let Some(ts) = a.get("times").and_then(Value::as_array) {
                ts.iter().filter_map(Value::as_f64).map(|t| t * 1000.0).collect()
            } else {
                let n = a.get("frames").and_then(Value::as_f64).unwrap_or(6.0).clamp(2.0, 12.0) as usize;
                (1..=n).map(|i| (len * i as f64 / n as f64).round()).collect()
            };
            let one = times.len() == 1;
            let w = a.get("width").and_then(Value::as_f64).unwrap_or(if one { 1024.0 } else { (2400.0 / times.len().min(4) as f64).min(360.0).floor() });
            let res = render(s, "/internal/page-png", &json!({ "designId": id, "pageId": page_id, "w": w, "timesMs": times }), 180_000).await?;
            let png = res.bytes().await.map_err(|e| Fail::Other(e.to_string()))?;
            let at: Vec<String> = times.iter().map(|t| format!("{} s", crate::js::num((t.clamp(0.0, len) / 10.0).round() / 100.0))).collect();
            let note = if one {
                format!("The page at {} of its {} s.", at[0], crate::js::num((len / 10.0).round() / 100.0))
            } else {
                format!("Frames at {} (left to right, then the next row) of the page's {} s. Videos show their poster frame.", at.join(", "), crate::js::num((len / 10.0).round() / 100.0))
            };
            Ok(json!({ "content": [{ "type": "text", "text": note }, { "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png" }] }))
        }
        "export_design" => {
            need(u, "exports")?;
            let id = a["design_id"].as_str().unwrap_or("").to_owned();
            access(s, u, &id, Role::View).await?;
            let format = a["format"].as_str().unwrap_or("").to_owned();
            let mut body = Map::new();
            body.insert("designId".into(), json!(id));
            body.insert("format".into(), json!(format));
            if let Some(p) = a.get("pages").and_then(Value::as_array) {
                body.insert("pages".into(), Value::Array(p.iter().map(|x| super::edit::jnum(x.as_f64().unwrap_or(0.0) - 1.0)).collect()));
            }
            body.insert("scale".into(), a.get("scale").cloned().unwrap_or(json!(1)));
            if let Some(w) = a.get("width") {
                body.insert("width".into(), w.clone());
            }
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
    let ops_in = ops.clone();
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
        let (src, want) = match op["op"].as_str() {
            Some("add_image") => (op.get("source"), "image"),
            Some("update") => (op.get("image"), "image"),
            Some("add_video") => (op.get("source"), "video"),
            Some("add_audio") => (op.get("source"), "audio"),
            _ => (None, "image"),
        };
        if let Some(src) = src {
            match import_media(s, u, src, &doc_assets, want).await {
                Ok(img) => {
                    images.insert(i, img);
                }
                Err(Fail::Tool(m)) | Err(Fail::Other(m)) => return tool_err(format!("Operation {} ({}): {m}", i + 1, op["op"].as_str().unwrap_or(""))),
            }
        }
    }
    let kit = match live::read_doc(s, &id, u.token_id).await? {
        Some(ydoc) => super::brand::motion_kit(s, u.id, &id, &oc_crdt::from_y_value(&ydoc)).await?,
        None => super::motion::Kit::default(),
    };
    let actor = format!("ai:{}", agent.token_id);
    let now = s.now_ms() as f64;
    let r = live::change(
        s,
        &id,
        &agent,
        move |ydoc| {
            let ctx = EditContext { ydoc, actor, images, now, kit };
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
    let mut check = if v.applied > 0 { page_check(s, &id, v.page_id.as_deref(), &v.touched).await } else { None };
    // The kit's fonts and text styles, for the text this edit touched.
    if v.applied > 0
        && let Some(ydoc) = live::read_doc(s, &id, u.token_id).await?
    {
        let (json, typed) = snapshot(&ydoc)?;
        if let Some(k) = super::brand::style_kit(s, u.id, &id, &json).await? {
            let issues = k.check(v.touched.iter().filter_map(|t| json["elements"].get(t).map(|e| (t.as_str(), e))));
            if !issues.is_empty() {
                check.get_or_insert_with(Vec::new).extend(issues);
            }
        }
        // Timing: text on screen too briefly to read, the parts of a Reels or Stories video the app covers, length.
        let pages: Vec<String> = v.touched.iter().filter_map(|t| oc_schema::tree::page_of(&typed.pages, &typed.elements, t).map(str::to_owned)).chain(v.page_id.clone()).collect();
        let issues = super::motion::video_checks(&json, &typed, if timing_only(&ops_in) { None } else { Some(&pages) });
        if !issues.is_empty() {
            check.get_or_insert_with(Vec::new).extend(issues);
        }
    }
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

/// Whether a batch changes only design-wide timing (page lengths, transitions, sound, captions), so the video checks
/// look at every page.
fn timing_only(ops: &[Value]) -> bool {
    ops.iter().all(|o| matches!(o["op"].as_str(), Some("update_page" | "add_audio" | "update_audio" | "remove_audio" | "set_captions" | "move_page" | "delete_page")))
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
pub(super) fn find_kit<'a>(kits: &'a [Value], kit: &str) -> Result<&'a Value, String> {
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

/// How much guideline text one `get_brand_guidelines` reply carries before the rest is left for a follow-up call.
const GUIDELINES_BUDGET: usize = 40_000;

/// `get_brand_guidelines`: the asked-for sections (titles matched loosely), or every section that fits the budget,
/// in order; sections left out are listed by title so the assistant can ask for them next.
fn guidelines_view(kit: Value, all: &[Value], wanted: Option<&Vec<Value>>) -> Value {
    if all.is_empty() {
        return json!({ "kit": kit, "sections": [], "note": "This kit has no written guidelines. Follow its voice, colours and fonts, and any design system the user gave you." });
    }
    let title = |x: &Value| x["title"].as_str().unwrap_or("").to_owned();
    let size = |x: &Value| x["body"].as_str().map_or(0, |b| b.chars().count());
    let wanted: Vec<String> = wanted.into_iter().flatten().filter_map(Value::as_str).map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
    let picked: Vec<&Value> = if wanted.is_empty() {
        all.iter().collect()
    } else {
        all.iter().filter(|x| wanted.iter().any(|w| title(x).to_lowercase().contains(w))).collect()
    };
    let (mut used, mut sections, mut more) = (0usize, Vec::new(), Vec::new());
    for x in picked {
        let n = size(x);
        // The first section always goes out whole, however long, so a single big brand book still reads.
        if sections.is_empty() || used + n <= GUIDELINES_BUDGET {
            used += n;
            sections.push(json!({ "title": x["title"], "body": x["body"] }));
        } else {
            more.push(json!({ "title": x["title"], "chars": n }));
        }
    }
    let mut out = json!({ "kit": kit, "sections": sections });
    if sections_empty(&out) {
        out["note"] = json!(format!("No section matches. Sections: {}", all.iter().map(title).collect::<Vec<_>>().join(", ")));
    } else if !more.is_empty() {
        out["more"] = json!(more);
        out["next"] = json!("Some sections didn't fit in this reply. Ask for them with get_brand_guidelines { brand_kit, sections: [titles] } before designing what they cover.");
    } else {
        out["next"] = json!("Follow these guidelines for every choice: copy, colours, type, layout, logo use and imagery. Where they conflict with a general habit, the guidelines win.");
    }
    out
}

fn sections_empty(v: &Value) -> bool {
    v["sections"].as_array().is_none_or(Vec::is_empty)
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
    fn kit_fonts_the_editor_cannot_draw_are_missing() {
        let usable: std::collections::HashSet<String> = ["hanken grotesk", "inter"].map(str::to_owned).into();
        let kit = json!({ "fonts": { "heading": "Canter", "subheading": "canter", "body": "Hanken Grotesk", "quote": "", "caption": "INTER" } });
        assert_eq!(missing_fonts(&kit, &usable), vec!["Canter".to_owned()]);
        assert!(missing_fonts(&json!({ "fonts": {} }), &usable).is_empty());
    }

    #[test]
    fn an_unclear_or_unknown_kit_lists_the_kits() {
        let kits = vec![json!({ "name": "Amigos one" }), json!({ "name": "Amigos two" })];
        assert!(find_kit(&kits, "amigos").unwrap_err().contains("Amigos one, Amigos two"));
        assert!(find_kit(&kits, "peritus").is_err());
        assert!(find_kit(&kits, " ").is_err());
    }
}

#[cfg(test)]
mod guidelines_tests {
    use super::*;

    fn sec(title: &str, chars: usize) -> Value {
        json!({ "title": title, "body": "x".repeat(chars) })
    }

    #[test]
    fn all_sections_when_they_fit() {
        let v = guidelines_view(json!("PF"), &[sec("Voice", 10), sec("Colour", 20)], None);
        assert_eq!(v["sections"].as_array().unwrap().len(), 2);
        assert!(v.get("more").is_none());
        assert!(v["next"].as_str().unwrap().contains("guidelines win"));
    }

    #[test]
    fn long_guidelines_come_in_parts() {
        let all = [sec("Intro", 30_000), sec("Colour", 15_000), sec("Type", 5_000)];
        let v = guidelines_view(json!("PF"), &all, None);
        let titles: Vec<&str> = v["sections"].as_array().unwrap().iter().map(|s| s["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Intro", "Type"]);
        assert_eq!(v["more"][0]["title"], "Colour");
        assert_eq!(v["more"][0]["chars"], 15_000);
        // A first section over the budget still goes out whole.
        let v = guidelines_view(json!("PF"), &[sec("Book", 90_000), sec("Logo", 10)], None);
        assert_eq!(v["sections"][0]["title"], "Book");
        assert_eq!(v["more"][0]["title"], "Logo");
    }

    #[test]
    fn named_sections_match_loosely() {
        let all = [sec("Colour", 5), sec("Typography", 5), sec("Logo use", 5)];
        let want = vec![json!("logo"), json!(" TYPO ")];
        let v = guidelines_view(json!("PF"), &all, Some(&want));
        let titles: Vec<&str> = v["sections"].as_array().unwrap().iter().map(|s| s["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Typography", "Logo use"]);
        let v = guidelines_view(json!("PF"), &all, Some(&vec![json!("motion")]));
        assert!(v["note"].as_str().unwrap().contains("Colour, Typography, Logo use"));
    }

    #[test]
    fn no_guidelines() {
        let v = guidelines_view(json!("PF"), &[], None);
        assert!(v["note"].as_str().unwrap().contains("no written guidelines"));
    }
}
