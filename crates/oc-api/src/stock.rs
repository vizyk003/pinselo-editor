//! Port of services/api/src/stock.ts — stock media adapters (EL-030..034): one search/get interface over
//! Openverse (no key needed, the default), Unsplash, Pexels, Pixabay and GIPHY. Imports re-fetch the item by id
//! from the provider (never a client-supplied URL) and only download from the provider's media hosts.
//!
//! Deviation: each provider's API base URL can be overridden (`OC_OPENVERSE_API_URL`, `OC_UNSPLASH_API_URL`,
//! `OC_PEXELS_API_URL`, `OC_PIXABAY_API_URL`, `OC_GIPHY_API_URL`) so the parity harness can point them at its
//! fake; the TS scenario rewrites `fetch` to the same addresses. Unset, the real hosts are used.

use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::state::AppState;

pub const KINDS: [&str; 5] = ["image", "video", "audio", "gif", "sticker"];
const PER_PAGE: i64 = 30;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    Openverse,
    Unsplash,
    Pexels,
    Pixabay,
    Giphy,
}

pub const PROVIDERS: [Provider; 5] = [Provider::Openverse, Provider::Unsplash, Provider::Pexels, Provider::Pixabay, Provider::Giphy];

static UNSPLASH_HOSTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\.)unsplash\.com$").expect("regex"));
static PEXELS_HOSTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\.)pexels\.com$").expect("regex"));
static PIXABAY_HOSTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\.)pixabay\.com$").expect("regex"));
static GIPHY_HOSTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\.)giphy\.com$").expect("regex"));

/// A stock media item to import: where to download it and the credit line.
pub struct StockMedia {
    pub url: String,
    pub filename: String,
    pub attribution: Value,
}

/// A provider error (`Error.message`).
pub type StockError = String;

impl Provider {
    pub fn id(self) -> &'static str {
        match self {
            Provider::Openverse => "openverse",
            Provider::Unsplash => "unsplash",
            Provider::Pexels => "pexels",
            Provider::Pixabay => "pixabay",
            Provider::Giphy => "giphy",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Openverse => "Openverse",
            Provider::Unsplash => "Unsplash",
            Provider::Pexels => "Pexels",
            Provider::Pixabay => "Pixabay",
            Provider::Giphy => "GIPHY",
        }
    }

    pub fn kinds(self) -> &'static [&'static str] {
        match self {
            Provider::Openverse => &["image", "audio"],
            Provider::Unsplash => &["image"],
            Provider::Pexels | Provider::Pixabay => &["image", "video"],
            Provider::Giphy => &["gif", "sticker"],
        }
    }

    /// Hosts media may be downloaded from (Openverse aggregates many hosts; downloads are size- and type-checked).
    pub fn host_allowed(self, host: &str) -> bool {
        match self {
            Provider::Openverse => true,
            Provider::Unsplash => UNSPLASH_HOSTS.is_match(host),
            Provider::Pexels => PEXELS_HOSTS.is_match(host),
            Provider::Pixabay => PIXABAY_HOSTS.is_match(host),
            Provider::Giphy => GIPHY_HOSTS.is_match(host),
        }
    }

    fn key_env(self) -> &'static str {
        match self {
            Provider::Openverse => "",
            Provider::Unsplash => "OC_UNSPLASH_KEY",
            Provider::Pexels => "OC_PEXELS_KEY",
            Provider::Pixabay => "OC_PIXABAY_KEY",
            Provider::Giphy => "OC_GIPHY_KEY",
        }
    }

    pub fn enabled(self, s: &AppState) -> bool {
        match self {
            Provider::Openverse => s.env("OC_STOCK_OPENVERSE").as_deref() != Some("0"),
            p => s.env_nonempty(p.key_env()).is_some(),
        }
    }

    fn key(self, s: &AppState) -> String {
        s.env(self.key_env()).unwrap_or_else(|| "undefined".into())
    }

    pub async fn search(self, s: &AppState, kind: &str, q: &str, page: i64) -> Result<Value, StockError> {
        let enc = crate::js::encode_uri_component;
        let or = |q: &str, d: &str| if q.is_empty() { d.to_owned() } else { q.to_owned() };
        match self {
            Provider::Openverse => {
                let path = if kind == "audio" { "audio" } else { "images" };
                let auth = openverse_auth(s).await?;
                // Anonymous requests may ask for at most 20 results per page.
                let size = if auth.is_some() { PER_PAGE } else { 20 };
                let url = format!("{}/v1/{path}/?q={}&page={page}&page_size={size}&license=cc0,pdm,by&mature=false", base(s, self), enc(&or(q, "nature")));
                let r = get_json(s, &url, auth.map(|a| ("authorization", a))).await?;
                let k = if kind == "audio" { "audio" } else { "image" };
                let items: Vec<Value> = arr(&r["results"]).iter().map(|x| ov_item(k, x)).collect();
                Ok(json!({ "items": items, "hasMore": (page as f64) < num(&r["page_count"]) }))
            }
            Provider::Unsplash => {
                let url = format!("{}/search/photos?query={}&page={page}&per_page={PER_PAGE}&content_filter=high", base(s, self), enc(&or(q, "minimal")));
                let r = get_json(s, &url, Some(("authorization", format!("Client-ID {}", self.key(s))))).await?;
                let items: Vec<Value> = arr(&r["results"]).iter().map(un_item).collect();
                Ok(json!({ "items": items, "hasMore": (page as f64) < num(&r["total_pages"]) }))
            }
            Provider::Pexels => {
                let auth = Some(("authorization", self.key(s)));
                if kind == "video" {
                    let url = format!("{}/videos/search?query={}&page={page}&per_page={PER_PAGE}", base(s, self), enc(&or(q, "nature")));
                    let r = get_json(s, &url, auth).await?;
                    let items: Vec<Value> = arr(&r["videos"]).iter().map(px_video).collect();
                    return Ok(json!({ "items": items, "hasMore": truthy(&r["next_page"]) }));
                }
                let url = format!("{}/v1/search?query={}&page={page}&per_page={PER_PAGE}", base(s, self), enc(&or(q, "minimal")));
                let r = get_json(s, &url, auth).await?;
                let items: Vec<Value> = arr(&r["photos"]).iter().map(px_photo).collect();
                Ok(json!({ "items": items, "hasMore": truthy(&r["next_page"]) }))
            }
            Provider::Pixabay => {
                let b = if kind == "video" { format!("{}/api/videos/", base(s, self)) } else { format!("{}/api/", base(s, self)) };
                let url = format!("{b}?key={}&q={}&page={page}&per_page={PER_PAGE}&safesearch=true", self.key(s), enc(q));
                let r = get_json(s, &url, None).await?;
                let items: Vec<Value> = arr(&r["hits"]).iter().map(|x| if kind == "video" { pb_video(x) } else { pb_image(x) }).collect();
                Ok(json!({ "items": items, "hasMore": ((page * PER_PAGE) as f64) < num(&r["totalHits"]) }))
            }
            Provider::Giphy => {
                let path = if kind == "sticker" { "stickers" } else { "gifs" };
                let mode = if q.is_empty() { "trending" } else { "search" };
                let url = format!("{}/v1/{path}/{mode}?api_key={}&q={}&limit={PER_PAGE}&offset={}&rating=g", base(s, self), self.key(s), enc(q), (page - 1) * PER_PAGE);
                let r = get_json(s, &url, None).await?;
                let k = if kind == "sticker" { "sticker" } else { "gif" };
                let items: Vec<Value> = arr(&r["data"]).iter().map(|g| gif_item(g, k)).collect();
                let p = &r["pagination"];
                Ok(json!({ "items": items, "hasMore": num(&p["offset"]) + num(&p["count"]) < num(&p["total_count"]) }))
            }
        }
    }

    pub async fn media(self, s: &AppState, kind: &str, id: &str) -> Result<StockMedia, StockError> {
        let enc = crate::js::encode_uri_component;
        match self {
            Provider::Openverse => {
                let path = if kind == "audio" { "audio" } else { "images" };
                let auth = openverse_auth(s).await?;
                let x = get_json(s, &format!("{}/v1/{path}/{}/", base(s, self), enc(id)), auth.map(|a| ("authorization", a))).await?;
                let item = ov_item(if kind == "audio" { "audio" } else { "image" }, &x);
                let title = js_str_or(&x["title"], id);
                Ok(StockMedia { url: js_string(&x["url"]), filename: title, attribution: attribution_of(&item) })
            }
            Provider::Unsplash => {
                let auth = format!("Client-ID {}", self.key(s));
                let p = get_json(s, &format!("{}/photos/{}", base(s, self), enc(id)), Some(("authorization", auth.clone()))).await?;
                // API guideline: register the download.
                let dl = js_string(&p["links"]["download_location"]);
                let s2 = s.clone();
                s.spawn(async move {
                    let _ = s2.http.get(dl).header("authorization", auth).send().await;
                });
                let item = un_item(&p);
                Ok(StockMedia { url: format!("{}&w=2400&fm=jpg&q=85", js_string(&p["urls"]["full"])), filename: format!("unsplash-{id}.jpg"), attribution: attribution_of(&item) })
            }
            Provider::Pexels => {
                let auth = Some(("authorization", self.key(s)));
                if kind == "video" {
                    let v = get_json(s, &format!("{}/videos/videos/{}", base(s, self), enc(id)), auth).await?;
                    // Best MP4 up to 1920 wide.
                    let files = arr(&v["video_files"]);
                    let mut mp4: Vec<&Value> = files.iter().filter(|x| x["file_type"].as_str() == Some("video/mp4") && num(&x["width"]) <= 1920.0).collect();
                    mp4.sort_by(|a, b| num(&b["width"]).partial_cmp(&num(&a["width"])).unwrap_or(std::cmp::Ordering::Equal));
                    let f = mp4.first().copied().or(files.first()).ok_or("Cannot read properties of undefined (reading 'link')")?;
                    let item = px_video(&v);
                    return Ok(StockMedia { url: js_string(&f["link"]), filename: format!("pexels-{id}.mp4"), attribution: attribution_of(&item) });
                }
                let p = get_json(s, &format!("{}/v1/photos/{}", base(s, self), enc(id)), auth).await?;
                let item = px_photo(&p);
                Ok(StockMedia { url: js_string(&p["src"]["large2x"]), filename: format!("pexels-{id}.jpg"), attribution: attribution_of(&item) })
            }
            Provider::Pixabay => {
                let b = if kind == "video" { format!("{}/api/videos/", base(s, self)) } else { format!("{}/api/", base(s, self)) };
                let r = get_json(s, &format!("{b}?key={}&id={}", self.key(s), enc(id)), None).await?;
                let x = arr(&r["hits"]).first().cloned().ok_or("not found")?;
                if kind == "video" {
                    let item = pb_video(&x);
                    let large = &x["videos"]["large"];
                    let pick = if truthy(&large["url"]) { large } else { &x["videos"]["medium"] };
                    return Ok(StockMedia { url: js_string(&pick["url"]), filename: format!("pixabay-{id}.mp4"), attribution: attribution_of(&item) });
                }
                let item = pb_image(&x);
                Ok(StockMedia { url: js_string(&x["largeImageURL"]), filename: format!("pixabay-{id}.jpg"), attribution: attribution_of(&item) })
            }
            Provider::Giphy => {
                let r = get_json(s, &format!("{}/v1/gifs/{}?api_key={}", base(s, self), enc(id), self.key(s)), None).await?;
                let item = gif_item(&r["data"], if kind == "sticker" { "sticker" } else { "gif" });
                // The GIF itself (not the MP4) so transparent stickers keep their alpha through ingest.
                Ok(StockMedia { url: js_string(&r["data"]["images"]["original"]["url"]), filename: format!("giphy-{id}.gif"), attribution: attribution_of(&item) })
            }
        }
    }
}

/// The provider's API base URL (overridable for the parity harness, see the module docs).
fn base(s: &AppState, p: Provider) -> String {
    let (env, default) = match p {
        Provider::Openverse => ("OC_OPENVERSE_API_URL", "https://api.openverse.org"),
        Provider::Unsplash => ("OC_UNSPLASH_API_URL", "https://api.unsplash.com"),
        Provider::Pexels => ("OC_PEXELS_API_URL", "https://api.pexels.com"),
        Provider::Pixabay => ("OC_PIXABAY_API_URL", "https://pixabay.com"),
        Provider::Giphy => ("OC_GIPHY_API_URL", "https://api.giphy.com"),
    };
    s.env_nonempty(env).unwrap_or_else(|| default.to_owned())
}

/// `getJson(url, headers)`.
async fn get_json(s: &AppState, url: &str, header: Option<(&str, String)>) -> Result<Value, StockError> {
    let mut rq = s.http.get(url).header("accept", "application/json").header("user-agent", "Pinselo/1.0 (+self-hosted)").timeout(Duration::from_secs(15));
    if let Some((k, v)) = header {
        rq = rq.header(k, v);
    }
    let r = rq.send().await.map_err(|_| "fetch failed".to_owned())?;
    if !r.status().is_success() {
        return Err(format!("provider responded {}", r.status().as_u16()));
    }
    let text = r.text().await.map_err(|e| e.to_string())?;
    crate::http::jsjson::parse(&text)
}

#[derive(Default)]
struct OvToken(Mutex<Option<(String, i64)>>);

/// Openverse client-credentials token (cached per process, refreshed a minute before it expires).
async fn openverse_auth(s: &AppState) -> Result<Option<String>, StockError> {
    let (Some(id), Some(secret)) = (s.env_nonempty("OC_OPENVERSE_CLIENT_ID"), s.env_nonempty("OC_OPENVERSE_CLIENT_SECRET")) else { return Ok(None) };
    let cache = s.local::<OvToken>();
    let cur = cache.0.lock().expect("ov token").clone();
    let token = match cur {
        Some((t, exp)) if exp >= s.now_ms() + 60_000 => t,
        _ => {
            let body: String = form_urlencoded::Serializer::new(String::new()).append_pair("grant_type", "client_credentials").append_pair("client_id", &id).append_pair("client_secret", &secret).finish();
            let r = s
                .http
                .post(format!("{}/v1/auth_tokens/token/", base(s, Provider::Openverse)))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .map_err(|_| "fetch failed".to_owned())?;
            let j = crate::http::jsjson::parse(&r.text().await.map_err(|e| e.to_string())?)?;
            let t = js_string(&j["access_token"]);
            *cache.0.lock().expect("ov token") = Some((t.clone(), s.now_ms() + (num(&j["expires_in"]) * 1000.0) as i64));
            t
        }
    };
    Ok(Some(format!("Bearer {token}")))
}

// ---------------------------------------------------------------- JS value helpers

fn arr(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

fn num(v: &Value) -> f64 {
    crate::zod::js_number(Some(v))
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// `String(v)` (`undefined` for a missing value).
fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "undefined".into(),
        other => crate::http::jsjson::stringify(other),
    }
}

/// `v || fallback` for a string-ish value.
fn js_str_or(v: &Value, fallback: &str) -> String {
    if truthy(v) { js_string(v) } else { fallback.to_owned() }
}

/// An object without its `undefined` (missing) members.
fn obj(kv: Vec<(&str, Option<Value>)>) -> Value {
    let mut m = Map::new();
    for (k, v) in kv {
        if let Some(v) = v {
            m.insert(k.to_owned(), v);
        }
    }
    Value::Object(m)
}

/// A member as JS reads it: `None` when undefined (missing), `null` kept.
fn m(o: &Value, k: &str) -> Option<Value> {
    o.get(k).cloned()
}

/// `v ?? …`: `None` for null or undefined.
fn nn(v: &Value) -> Option<Value> {
    (!v.is_null()).then(|| v.clone())
}

fn s(v: &str) -> Option<Value> {
    Some(json!(v))
}

// ---------------------------------------------------------------- item shapes

fn ov_license(l: &Value, v: &Value) -> String {
    let ver = if truthy(v) { format!(" {}", js_string(v)) } else { String::new() };
    format!("CC {}{ver}", js_string(l).to_uppercase()).replacen("CC CC0", "CC0", 1).replacen("CC PDM", "Public Domain", 1)
}

fn ov_item(kind: &str, x: &Value) -> Value {
    let thumb = if kind == "image" { json!(format!("https://api.openverse.org/v1/images/{}/thumb/", js_string(&x["id"]))) } else { nn(&x["thumbnail"]).unwrap_or(json!("")) };
    obj(vec![
        ("provider", s("openverse")),
        ("id", m(x, "id")),
        ("kind", s(kind)),
        ("title", Some(if truthy(&x["title"]) { x["title"].clone() } else { json!("Untitled") })),
        ("thumb", Some(thumb)),
        ("w", Some(nn(&x["width"]).unwrap_or(json!(0)))),
        ("h", Some(nn(&x["height"]).unwrap_or(json!(0)))),
        ("durationMs", m(x, "duration")),
        ("author", m(x, "creator")),
        ("authorUrl", m(x, "creator_url")),
        ("license", Some(json!(ov_license(&x["license"], &x["license_version"])))),
        ("licenseUrl", m(x, "license_url")),
        ("sourceUrl", m(x, "foreign_landing_url")),
    ])
}

fn un_item(p: &Value) -> Value {
    let title = [&p["description"], &p["alt_description"]].into_iter().find(|v| truthy(v)).cloned().unwrap_or(json!("Photo"));
    obj(vec![
        ("provider", s("unsplash")),
        ("id", m(p, "id")),
        ("kind", s("image")),
        ("title", Some(title)),
        ("thumb", m(&p["urls"], "small")),
        ("w", m(p, "width")),
        ("h", m(p, "height")),
        ("author", m(&p["user"], "name")),
        ("authorUrl", Some(json!(format!("{}?utm_source=pinselo&utm_medium=referral", js_string(&p["user"]["links"]["html"]))))),
        ("license", s("Unsplash License")),
        ("licenseUrl", s("https://unsplash.com/license")),
        ("sourceUrl", m(&p["links"], "html")),
    ])
}

fn px_photo(p: &Value) -> Value {
    obj(vec![
        ("provider", s("pexels")),
        ("id", Some(json!(js_string(&p["id"])))),
        ("kind", s("image")),
        ("title", Some(if truthy(&p["alt"]) { p["alt"].clone() } else { json!("Photo") })),
        ("thumb", m(&p["src"], "medium")),
        ("w", m(p, "width")),
        ("h", m(p, "height")),
        ("author", m(p, "photographer")),
        ("authorUrl", m(p, "photographer_url")),
        ("license", s("Pexels License")),
        ("licenseUrl", s("https://www.pexels.com/license/")),
        ("sourceUrl", m(p, "url")),
    ])
}

fn px_video(v: &Value) -> Value {
    obj(vec![
        ("provider", s("pexels")),
        ("id", Some(json!(js_string(&v["id"])))),
        ("kind", s("video")),
        ("title", s("Video")),
        ("thumb", m(v, "image")),
        ("w", m(v, "width")),
        ("h", m(v, "height")),
        ("durationMs", crate::http::jsjson::number_value(num(&v["duration"]) * 1000.0)),
        ("author", m(&v["user"], "name")),
        ("authorUrl", m(&v["user"], "url")),
        ("license", s("Pexels License")),
        ("licenseUrl", s("https://www.pexels.com/license/")),
        ("sourceUrl", m(v, "url")),
    ])
}

fn pb_author(x: &Value) -> [(&'static str, Option<Value>); 2] {
    [("author", m(x, "user")), ("authorUrl", Some(json!(format!("https://pixabay.com/users/{}-{}/", js_string(&x["user"]), js_string(&x["user_id"])))))]
}

fn pb_image(x: &Value) -> Value {
    let mut kv = vec![
        ("provider", s("pixabay")),
        ("id", Some(json!(js_string(&x["id"])))),
        ("kind", s("image")),
        ("title", m(x, "tags")),
        ("thumb", m(x, "webformatURL")),
        ("w", m(x, "imageWidth")),
        ("h", m(x, "imageHeight")),
    ];
    kv.extend(pb_author(x));
    kv.extend([("license", s("Pixabay Content License")), ("licenseUrl", s("https://pixabay.com/service/license-summary/")), ("sourceUrl", m(x, "pageURL"))]);
    obj(kv)
}

fn pb_video(x: &Value) -> Value {
    let v = &x["videos"];
    let thumb = nn(&v["tiny"]["thumbnail"]).or_else(|| nn(&v["small"]["thumbnail"])).unwrap_or(json!(""));
    let mut kv = vec![
        ("provider", s("pixabay")),
        ("id", Some(json!(js_string(&x["id"])))),
        ("kind", s("video")),
        ("title", m(x, "tags")),
        ("thumb", Some(thumb)),
        ("w", m(&v["medium"], "width")),
        ("h", m(&v["medium"], "height")),
        ("durationMs", crate::http::jsjson::number_value(num(&x["duration"]) * 1000.0)),
    ];
    kv.extend(pb_author(x));
    kv.extend([("license", s("Pixabay Content License")), ("licenseUrl", s("https://pixabay.com/service/license-summary/")), ("sourceUrl", m(x, "pageURL"))]);
    obj(kv)
}

fn gif_item(g: &Value, kind: &str) -> Value {
    let o = &g["images"]["original"];
    obj(vec![
        ("provider", s("giphy")),
        ("id", m(g, "id")),
        ("kind", s(kind)),
        ("title", Some(if truthy(&g["title"]) { g["title"].clone() } else { json!("GIF") })),
        ("thumb", m(&g["images"]["fixed_width"], "url")),
        ("w", crate::http::jsjson::number_value(num(&o["width"])).or(Some(Value::Null))),
        ("h", crate::http::jsjson::number_value(num(&o["height"])).or(Some(Value::Null))),
        ("author", nn(&g["user"]["display_name"]).or_else(|| m(g, "username"))),
        ("authorUrl", m(&g["user"], "profile_url")),
        ("license", s("GIPHY Terms")),
        ("licenseUrl", s("https://support.giphy.com/hc/en-us/articles/360020027752-GIPHY-User-Terms-of-Service")),
        ("sourceUrl", m(g, "url")),
    ])
}

/// `attributionOf(item)` (undefined members dropped, as `JSON.stringify` does).
pub fn attribution_of(i: &Value) -> Value {
    let g = |k: &str| m(i, k);
    obj(vec![
        ("provider", g("provider")),
        ("title", g("title")),
        ("author", g("author")),
        ("authorUrl", g("authorUrl")),
        ("license", g("license")),
        ("licenseUrl", g("licenseUrl")),
        ("sourceUrl", g("sourceUrl")),
    ])
}

static PRIVATE_V6: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[?(::1|fc|fd|fe80)").expect("regex"));
static IPV4: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)\.(\d+)\.(\d+)\.(\d+)$").expect("regex"));

/// `isPublicHost(hostname)`: private, loopback and link-local addresses are never fetched (SSRF guard).
pub fn is_public_host(hostname: &str) -> bool {
    let h = hostname.to_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") || h.ends_with(".internal") {
        return false;
    }
    if PRIVATE_V6.is_match(&h) {
        return false;
    }
    if let Some(c) = IPV4.captures(&h) {
        let a: f64 = c[1].parse().unwrap_or(f64::NAN);
        let b: f64 = c[2].parse().unwrap_or(f64::NAN);
        if a == 10.0 || a == 127.0 || a == 0.0 || (a == 169.0 && b == 254.0) || (a == 172.0 && (16.0..=31.0).contains(&b)) || (a == 192.0 && b == 168.0) || (a == 100.0 && (64.0..=127.0).contains(&b)) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_hosts() {
        for h in ["localhost", "a.localhost", "x.local", "db.internal", "[::1]", "fd00::1", "10.1.2.3", "127.0.0.1", "169.254.1.1", "172.16.0.1", "192.168.1.1", "100.64.0.1", "0.0.0.0"] {
            assert!(!is_public_host(h), "{h}");
        }
        for h in ["example.com", "8.8.8.8", "172.32.0.1", "100.128.0.1"] {
            assert!(is_public_host(h), "{h}");
        }
    }
}
