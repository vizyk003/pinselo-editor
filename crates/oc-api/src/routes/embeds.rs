//! Port of services/api/src/routes/embeds.ts — embeds (A11): validate a pasted URL against the allow-list, fetch
//! title + thumbnail via the provider's oEmbed endpoint (server-side, so the browser never talks to third
//! parties while editing), and store the thumbnail as the poster asset.

use std::time::Duration;

use crate::http::ratelimit::MINUTE;
use crate::js;
use crate::prelude::*;
use crate::routes::assets::{Ingest, IngestOpts, TempDir, ingest_file};
use crate::stock::is_public_host;

pub const LIMITS: &[RouteLimit] =
    &[RouteLimit { method: "POST", path: "/api/embeds", max: 30, window_ms: MINUTE }, RouteLimit { method: "GET", path: "/api/public/oembed", max: 120, window_ms: MINUTE }];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/embeds", post(embed))
        .route("/api/public/oembed", get(oembed))
        .route("/api/public/designs/{id}/thumbnail.png", get(public_thumbnail))
}

// ---------------------------------------------------------------- outbound oEmbed (g135, public)

struct PublicDesign {
    id: String,
    title: String,
    owner: String,
    width: f64,
    height: f64,
    thumb_sha256: Option<String>,
}

/// `openPublicDesign(key, reply)`: a design by id or custom slug when its public view link is open to anyone right
/// now (on, not expired, no password); otherwise the reply to send.
async fn open_public_design(s: &AppState, key: &str) -> Result<Result<PublicDesign, Reply>, ApiError> {
    use crate::share_links::{looks_like_design_id, normalize_slug};
    type Row = (String, String, String, f64, f64, Option<String>, bool, Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>, Option<String>);
    let by_id = looks_like_design_id(key);
    let d: Option<Row> = sqlx::query_as(&format!(
        "SELECT d.id, d.title, u.name AS owner, d.width::float8, d.height::float8, d.thumb_sha256, d.public_view, d.trashed_at, d.public_expires_at, d.public_password_hash
     FROM designs d JOIN users u ON u.id = d.owner_id WHERE {}",
        if by_id { "d.id = $1" } else { "d.public_slug = $1" }
    ))
    .bind(if by_id { key.to_owned() } else { normalize_slug(key) })
    .fetch_optional(&s.pool)
    .await?;
    let Some((id, title, owner, width, height, thumb_sha256, public_view, trashed, expires, password)) = d else { return Ok(Err(reply::json(404, json!({ "error": "not_found" })))) };
    if !public_view || trashed.is_some() || expires.is_some_and(|e| e.timestamp_millis() < s.now_ms()) {
        return Ok(Err(reply::json(404, json!({ "error": "not_found" }))));
    }
    // oEmbed 401: the resource exists but isn't public (a password-protected link can't be embedded unattended).
    if password.is_some_and(|p| !p.is_empty()) {
        return Ok(Err(reply::json(401, json!({ "error": "password_protected" }))));
    }
    Ok(Ok(PublicDesign { id, title, owner, width, height, thumb_sha256 }))
}

/// oEmbed 1.0 provider for public view links: `?url=<origin>/view/<id or slug>[&maxwidth&maxheight]`. JSON only.
async fn oembed(State(s): State<AppState>, req: Req) -> ApiResult {
    let q = z::obj([
        ("url", z::str().max(2000.0)),
        ("format", z::str().max(10.0).optional()),
        ("maxwidth", z::coerce_num().int().positive().max(10000.0).optional()),
        ("maxheight", z::coerce_num().int().positive().max(10000.0).optional()),
    ])
    .parse(Some(&req.query))?;
    let acao = |r: Reply| r.header("access-control-allow-origin", "*");
    if q.get("format").and_then(Value::as_str).is_some_and(|f| !f.is_empty() && f != "json") {
        return Ok(acao(reply::json(501, json!({ "error": "format_not_supported" }))));
    }
    let Some(key) = crate::share_links::view_key_of(q["url"].as_str().unwrap_or(""), &s.cfg.web_origin) else {
        return Ok(acao(reply::json(404, json!({ "error": "not_found" }))));
    };
    let d = match open_public_design(&s, &key).await? {
        Ok(d) => d,
        Err(r) => return Ok(acao(r)),
    };
    let mut thumb = None;
    if let Some(sha) = d.thumb_sha256.as_deref().filter(|x| !x.is_empty()) {
        let bytes = s.store.get(sha).await.unwrap_or_default();
        if let Some((w, h)) = crate::share_links::png_size(&bytes) {
            thumb = Some((format!("{}/api/public/designs/{}/thumbnail.png", s.cfg.web_origin, d.id), w, h));
        }
    }
    let body = crate::share_links::oembed_for(&crate::share_links::OembedInput {
        web_origin: &s.cfg.web_origin,
        brand: oc_config::BRAND.name,
        key: &d.id,
        title: &d.title,
        author: &d.owner,
        w: d.width,
        h: d.height,
        max_width: q.get("maxwidth").and_then(Value::as_f64),
        max_height: q.get("maxheight").and_then(Value::as_f64),
        thumb,
    });
    Ok(acao(Reply::json(body)).header("cache-control", "public, max-age=300"))
}

/// Thumbnail of a public design (oEmbed thumbnail_url, link previews); same openness rule as the embed.
async fn public_thumbnail(State(s): State<AppState>, Path(id): Path<String>, _req: Req) -> ApiResult {
    if !crate::share_links::looks_like_design_id(&id) {
        return Ok(Reply::empty(404));
    }
    let d = match open_public_design(&s, &id).await? {
        Ok(d) => d,
        Err(r) => return Ok(r),
    };
    let Some(sha) = d.thumb_sha256.filter(|x| !x.is_empty()) else { return Ok(Reply::empty(404)) };
    Ok(Reply::bytes("image/png", s.store.get(&sha).await?).header("cache-control", "public, max-age=300"))
}

/// `new URL(s).hostname` (WHATWG).
pub fn hostname_of(s: &str) -> Option<String> {
    url::Url::parse(s).ok().map(|u| u.host_str().unwrap_or("").to_owned())
}

async fn embed(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("url", z::str().max(2000.0))]).parse(req.body())?;
    let url = b["url"].as_str().unwrap_or("").to_owned();
    let Some(target) = oc_schema::embeds::parse_embed_url(&url) else {
        return Ok(reply::json(400, json!({ "error": "unsupported", "message": "Paste a YouTube, Vimeo, Spotify, SoundCloud, Google Maps, Loom, Figma or CodePen link." })));
    };
    let provider = serde_json::to_value(&target.provider)?.as_str().unwrap_or("").to_owned();
    let mut title = String::new();
    let mut thumb: Option<Value> = None;
    if let Some(oembed) = &target.oembed {
        let r = s.http.get(oembed).timeout(Duration::from_secs(8)).header("user-agent", "Pinselo oEmbed").send().await;
        if let Ok(r) = r
            && r.status().is_success()
            && let Ok(text) = r.text().await
            && let Ok(j) = crate::http::jsjson::parse(&text)
        {
            // `(j.title ?? '').slice(0, 300)` throws for a non-string title (caught: no title, no poster).
            match j.get("title") {
                None | Some(Value::Null) => {
                    thumb = j.get("thumbnail_url").cloned();
                }
                Some(Value::String(t)) => {
                    title = js::take(t, 300);
                    thumb = j.get("thumbnail_url").cloned();
                }
                Some(_) => {}
            }
        }
    }
    // YouTube's oEmbed thumbnail is low-res; the maxres/hq image is predictable.
    if provider == "youtube"
        && let Some(id) = target.src.split("embed/").nth(1).map(|r| r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect::<String>()).filter(|id| !id.is_empty())
    {
        thumb = Some(Value::String(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")));
    }
    let mut poster = Value::Null;
    let thumb_str = thumb.as_ref().map(|t| match t {
        Value::String(x) => x.clone(),
        other => crate::http::jsjson::stringify(other),
    });
    if let Some(t) = thumb_str.filter(|t| !t.is_empty() && t.starts_with("https://")) {
        // `new URL(thumb)` outside the try: an unparsable URL fails the request.
        let host = hostname_of(&t).ok_or_else(|| ApiError::internal("Invalid URL"))?;
        if is_public_host(&host) {
            let dir = TempDir::new("oc-embed-").await?;
            if let Ok(r) = s.http.get(&t).timeout(Duration::from_secs(10)).send().await
                && r.status().is_success()
                && let Ok(bytes) = r.bytes().await
            {
                let f = dir.0.join("thumb");
                if tokio::fs::write(&f, &bytes).await.is_ok() {
                    let name = format!("{} poster", if title.is_empty() { provider.as_str() } else { title.as_str() });
                    let ing = ingest_file(
                        &s,
                        &f,
                        &dir.0,
                        IngestOpts {
                            name: &name,
                            owner_id: u.id,
                            ip: Some(&req.ip),
                            source: Some("stock"),
                            attribution: Some(json!({ "provider": provider, "sourceUrl": url })),
                            skip_quota: false,
                        },
                    )
                    .await;
                    if let Ok(Ingest::Asset(a)) = ing {
                        poster = a;
                    }
                }
            }
        }
    }
    Ok(Reply::json(json!({
        "provider": provider, "src": target.src, "kind": serde_json::to_value(&target.kind)?, "aspect": target.aspect,
        "title": title, "url": url, "poster": poster,
    })))
}
