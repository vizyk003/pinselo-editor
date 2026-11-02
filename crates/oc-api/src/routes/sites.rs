//! Port of services/api/src/routes/sites.ts — websites (WS-001..006): publish a design as a site at
//! `/site/{slug}` or on a custom domain, with SEO fields, optional password, privacy-friendly insights and
//! DNS-verified domains.
//!
//! The public routes (serving, beacons, the password form, the TLS domain check, OG images) need no sign-in; the
//! editor routes run `requireUser` first (the child plugin's `preHandler`). The plugin also registers an
//! `application/x-www-form-urlencoded` body parser for its own routes (the password form posts one): see
//! [`form_body`]. [`custom_domain_hook`] is the app-wide `onRequest` hook (wired into the global middleware).

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING};
use axum::http::{HeaderMap, HeaderValue, Method, Uri};
use axum::middleware::{Next, from_fn};
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, Mac};
use serde_json::Map;
use sha2::Sha256;

use crate::auth::{at_least, design_role, hash_password, verify_password};
use crate::billing;
use crate::brandhub::approval_gate;
use crate::http::ratelimit::MINUTE;
use crate::http::{cookie, jsjson};
use crate::insights;
use crate::prelude::*;
use crate::site_seo::{robots_txt, sitemap_xml};
use oc_auth::Role;

/// Routes with their own `config.rateLimit`.
pub const LIMITS: &[RouteLimit] = &[
    RouteLimit { method: "POST", path: "/api/sites/{id}/e", max: 60, window_ms: MINUTE },
    RouteLimit { method: "POST", path: "/api/public/designs/{id}/view", max: 30, window_ms: MINUTE },
    RouteLimit { method: "POST", path: "/site/{slug}", max: 10, window_ms: MINUTE },
];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/site/{slug}", get(public_site).post(unlock))
        .route("/site/{slug}/{file}", get(public_seo))
        .route("/api/sites/{id}/e", post(beacon))
        .route("/api/public/designs/{id}/view", post(view_beacon))
        .route("/api/sites/domain-check", get(domain_check))
        .route("/api/sites/og/{file}", get(og_image))
        .route("/api/designs/{id}/site", get(get_site).put(put_site))
        .route("/api/designs/{id}/site/publish", post(publish))
        .route("/api/designs/{id}/site/insights", get(site_insights))
        .route("/api/designs/{id}/view-insights", get(view_insights))
        .route("/api/designs/{id}/site/preview", post(preview))
        .route("/api/designs/{id}/site/unpublish", post(unpublish))
        .route("/api/designs/{id}/site/domains", post(add_domain))
        .route("/api/designs/{id}/site/domains/{domain}/verify", post(verify_domain))
        .route("/api/designs/{id}/site/domains/{domain}", delete(remove_domain))
        .route_layer(from_fn(form_body))
}

/// `app.addContentTypeParser('application/x-www-form-urlencoded', …)` inside this plugin: the body becomes
/// `Object.fromEntries(new URLSearchParams(body))` (handed to [`Req`] as the equivalent JSON object).
pub async fn form_body(req: Request, next: Next) -> Response {
    let form = req.method() != Method::GET
        && req.method() != Method::HEAD
        && req.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).and_then(crate::http::req::media_type).as_deref() == Some("application/x-www-form-urlencoded");
    if !form {
        return next.run(req).await;
    }
    let (mut parts, body) = req.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, crate::http::req::BODY_LIMIT).await else {
        return ApiError::Status { status: 413, message: "Request body is too large".into(), close: true }.into_response();
    };
    let mut m = Map::new();
    for (k, v) in form_urlencoded::parse(&bytes) {
        m.insert(k.into_owned(), Value::String(v.into_owned()));
    }
    let json = jsjson::stringify(&Value::Object(m));
    parts.headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    parts.headers.remove(TRANSFER_ENCODING);
    parts.headers.insert(CONTENT_LENGTH, HeaderValue::from(json.len()));
    next.run(Request::from_parts(parts, Body::from(json))).await
}

// ---------------------------------------------------------------- helpers

static SLUG: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^[a-z0-9](?:[a-z0-9-]{1,46}[a-z0-9])$").expect("regex"));
const RESERVED: [&str; 10] = ["api", "admin", "www", "app", "help", "status", "login", "site", "static", "assets"];
static DOMAIN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^([a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$").expect("regex"));
const DOMAIN_JS: &str = r"^(?=.{4,253}$)([a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$";

/// `/^(?=.{4,253}$)([a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/`.
fn domain_ok(s: &str) -> bool {
    (4..=253).contains(&crate::js::len(s)) && DOMAIN.is_match(s)
}

#[derive(sqlx::FromRow, Clone)]
pub struct SiteRow {
    design_id: String,
    slug: String,
    title: String,
    description: Option<String>,
    published: bool,
    password_hash: Option<String>,
    noindex: bool,
    html_sha256: Option<String>,
    version: i32,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
    analytics: bool,
}

async fn site_by_slug(s: &AppState, slug: &str) -> Result<Option<SiteRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM sites WHERE slug = $1").bind(slug).fetch_optional(&s.pool).await
}

async fn site_by_design(s: &AppState, id: &str) -> Result<Option<SiteRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM sites WHERE design_id = $1").bind(id).fetch_optional(&s.pool).await
}

fn cookie_name(slug: &str) -> String {
    format!("oc_site_{}", slug.replace('-', "_"))
}

fn unlock_token(s: &AppState, site: &SiteRow) -> String {
    use base64::Engine as _;
    let mut mac = Hmac::<Sha256>::new_from_slice(s.cfg.secret.as_bytes()).expect("hmac key");
    mac.update(format!("site:{}:{}", site.design_id, site.password_hash.as_deref().unwrap_or("null")).as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

fn web_host(s: &AppState) -> String {
    url::Url::parse(&s.cfg.web_origin).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default()
}

/// `siteCsp` (SEC-02): nonce-only scripts; on the app origin a sandbox (opaque origin: no cookies, no same-origin
/// reads of /api) unless the page has embeds or posting forms. See services/api/src/routes/sites.ts.
pub fn site_csp(s: &AppState, nonce: &str, sandbox: bool) -> String {
    let assets = if s.cfg.usercontent_origin.is_empty() { "'self'".to_owned() } else { s.cfg.usercontent_origin.clone() };
    format!(
        "default-src 'none'; img-src 'self' data: {assets}; media-src 'self' {assets}; font-src 'self'; style-src 'unsafe-inline'; script-src 'nonce-{nonce}'; connect-src 'self'; frame-src {}; form-action 'self'; frame-ancestors *; base-uri 'none'; object-src 'none'{}",
        oc_schema::embeds::EMBED_FRAME_HOSTS.join(" "),
        if sandbox { "; sandbox allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox allow-top-navigation-by-user-activation" } else { "" }
    )
}

/// `sandboxable`: no embeds (nested iframes inherit the sandbox) and no fetch-posting forms.
pub fn sandboxable(page: &str) -> bool {
    !page.to_ascii_lowercase().contains("<iframe") && !page.contains("<form class=\"fm\"") && !page.contains("<form class=\"mf\"")
}

/// `nonceScripts`: the builder's script (the last `<script>`) gets the nonce; nothing else does.
pub fn nonce_scripts(page: &str, nonce: &str) -> String {
    match page.rfind("<script>") {
        Some(i) => format!("{}<script nonce=\"{nonce}\">{}", &page[..i], &page[i + 8..]),
        None => page.to_owned(),
    }
}

/// 32 random bytes, base64url, behind a `+` (valid in a CSP nonce; keeps the token a separate word for the parity
/// transcripts' normalization). `serveSite`.
fn site_nonce() -> String {
    format!("+{}", crate::auth::random_token(32))
}

fn site_headers(s: &AppState, r: Reply, nonce: &str, sandbox: bool) -> Reply {
    r.header("content-security-policy", site_csp(s, nonce, sandbox))
        .header("cross-origin-resource-policy", "same-origin")
        .header("x-content-type-options", "nosniff")
        .header("referrer-policy", "strict-origin-when-cross-origin")
        .header("cache-control", "public, max-age=60")
}

fn html(status: u16, body: String) -> Reply {
    Reply::bytes("text/html", body).status(status)
}

fn site_not_found() -> Reply {
    html(404, "<!doctype html><h1>Site not found</h1>".into())
}

fn password_page(site: &SiteRow, error: bool) -> String {
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{}</title>
<style>body{{font-family:system-ui,sans-serif;display:grid;place-items:center;min-height:100vh;margin:0;background:#f4f4f7}}form{{background:#fff;padding:28px;border-radius:14px;box-shadow:0 8px 30px rgba(0,0,0,.08);display:grid;gap:12px;width:min(340px,90vw)}}input,button{{font:inherit;padding:10px 12px;border-radius:8px;border:1px solid #ccc}}button{{background:#7c3aed;color:#fff;border:0;font-weight:600}}p{{margin:0;color:#b91c1c}}</style></head>
<body><form method="post"><h1 style="margin:0;font-size:20px">This site is password protected</h1>{}<input type="password" name="password" placeholder="Password" autofocus required aria-label="Password"><button>Open</button></form></body></html>"#,
        site.title.replace('<', "&lt;"),
        if error { "<p>Wrong password.</p>" } else { "" }
    )
}

/// Public URL of a site: its verified custom domain, else `/site/{slug}`.
async fn site_url(s: &AppState, site: &SiteRow) -> Result<String, sqlx::Error> {
    let d: Option<String> = sqlx::query_scalar("SELECT domain FROM site_domains WHERE design_id = $1 AND verified_at IS NOT NULL ORDER BY created_at LIMIT 1").bind(&site.design_id).fetch_optional(&s.pool).await?;
    Ok(match d {
        Some(d) => format!("https://{d}/"),
        None => format!("{}/site/{}", s.cfg.web_origin, site.slug),
    })
}

/// `ownerActive(designId)`: a disabled or deleted owner's sites are offline, whatever their published flag says (D1).
async fn owner_active(s: &AppState, design_id: &str) -> Result<bool, sqlx::Error> {
    Ok(pg::one(&s.pool, "SELECT 1 FROM designs d JOIN users u ON u.id = d.owner_id WHERE d.id = $1 AND u.disabled_at IS NULL AND u.deleted_at IS NULL", &[pg::text(design_id)]).await?.is_some())
}

async fn serve_seo(s: &AppState, site: &SiteRow, file: &str, base: &str) -> Result<Reply, ApiError> {
    if !site.published || site.html_sha256.as_deref().is_none_or(str::is_empty) || !owner_active(s, &site.design_id).await? {
        return Ok(Reply::empty(404));
    }
    let hidden = site.noindex || site.password_hash.as_deref().is_some_and(|p| !p.is_empty());
    if file == "robots.txt" {
        return Ok(Reply::bytes("text/plain; charset=utf-8", robots_txt(hidden, Some(&format!("{base}sitemap.xml")))).header("cache-control", "public, max-age=300"));
    }
    let entries = if hidden { vec![] } else { vec![(site_url(s, site).await?, site.published_at.map(|t| oc_js::iso_string(&t)))] };
    Ok(Reply::bytes("application/xml; charset=utf-8", sitemap_xml(&entries)).header("cache-control", "public, max-age=300"))
}

async fn serve_site(s: &AppState, site: &SiteRow, headers: &HeaderMap, cookies: &std::collections::HashMap<String, String>, ip: &str, app_origin: bool) -> Result<Reply, ApiError> {
    let Some(sha) = site.html_sha256.clone().filter(|h| site.published && !h.is_empty()) else { return Ok(site_not_found()) };
    if !owner_active(s, &site.design_id).await? {
        return Ok(site_not_found());
    }
    if site.password_hash.as_deref().is_some_and(|p| !p.is_empty()) && cookies.get(&cookie_name(&site.slug)) != Some(&unlock_token(s, site)) {
        let nonce = site_nonce();
        return Ok(site_headers(s, html(200, password_page(site, false)), &nonce, app_origin).header("cache-control", "no-store"));
    }
    // Privacy-friendly insights (no cookies, no IPs stored): skipped for DNT/GPC, bots, or when the owner turned them off.
    let track = site.analytics && !insights::do_not_track(headers) && !insights::is_bot(insights::header(headers, "user-agent"));
    if track {
        let (pool, id) = (s.pool.clone(), site.design_id.clone());
        s.spawn(async move {
            let _ = sqlx::query("INSERT INTO site_views(design_id, day, views) VALUES ($1, current_date, 1) ON CONFLICT (design_id, day) DO UPDATE SET views = site_views.views + 1").bind(id).execute(&pool).await;
        });
        let host = insights::header(headers, "host").split(':').next().unwrap_or("").to_owned();
        let (s2, id, h, ip, own) = (s.clone(), site.design_id.clone(), headers.clone(), ip.to_owned(), vec![host, web_host(s)]);
        s.spawn(async move {
            let _ = insights::record_visit(&s2, "site", &id, &h, &ip, &own).await;
        });
    }
    let nonce = site_nonce();
    let page = nonce_scripts(&String::from_utf8_lossy(&s.store.get(&sha).await?), &nonce);
    let sandbox = app_origin && sandboxable(&page);
    let page = if track { inject_beacon(&page, &site.design_id, &nonce) } else { page };
    Ok(site_headers(s, Reply::bytes("text/html; charset=utf-8", page), &nonce, sandbox))
}

/// `html.replace(/<\/body>\s*<\/html>\s*$/, beacon)`.
fn inject_beacon(page: &str, design_id: &str, nonce: &str) -> String {
    static END: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"</body>\s*</html>\s*\z").expect("regex"));
    match END.find(page) {
        Some(m) => format!("{}{}\n</body>\n</html>", &page[..m.start()], insights::beacon_script(design_id).replacen("<script>", &format!("<script nonce=\"{nonce}\">"), 1)),
        None => page.to_owned(),
    }
}

/// `build(designId, site)`: the site HTML from the render service.
async fn build(s: &AppState, design_id: &str, title: &str, description: Option<&str>, noindex: bool) -> Result<String, ApiError> {
    let mut body = Map::new();
    body.insert("designId".into(), json!(design_id));
    body.insert("title".into(), json!(title));
    if let Some(d) = description {
        body.insert("description".into(), json!(d));
    }
    body.insert("noindex".into(), json!(noindex));
    body.insert("assetOrigin".into(), json!(if s.cfg.usercontent_origin.is_empty() { &s.cfg.web_origin } else { &s.cfg.usercontent_origin }));
    body.insert("badge".into(), json!({ "name": s.cfg.brand.name, "url": s.cfg.web_origin }));
    let res = s
        .http
        .post(format!("{}/internal/site-build", s.cfg.render_url))
        .header("content-type", "application/json")
        .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&s.cfg.secret))
        .body(jsjson::stringify(&Value::Object(body)))
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(ApiError::internal(format!("site build failed ({})", res.status().as_u16())));
    }
    let j = jsjson::parse(&res.text().await?).map_err(ApiError::internal)?;
    match j.get("html") {
        Some(Value::String(h)) => Ok(h.clone()),
        _ => Err(ApiError::internal("site build returned no html")),
    }
}

/// Custom domains (`customDomainHook`, `onRequest`): any request whose Host is a verified domain gets that site
/// at `/` (plus its robots.txt and sitemap.xml).
pub async fn custom_domain_hook(state: &AppState, method: Method, uri: Uri, headers: HeaderMap) -> Option<Response> {
    let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("").split(':').next().unwrap_or("").to_lowercase();
    let url = uri.path_and_query().map_or_else(|| uri.path().to_owned(), |p| p.as_str().to_owned());
    if host.is_empty() || url.starts_with("/api/") || url.starts_with("/site/") || host == web_host(state) || host == "localhost" || host == "127.0.0.1" {
        return None;
    }
    let r = async {
        let d: Option<(SiteRow, Option<chrono::DateTime<chrono::Utc>>)> = {
            let row = sqlx::query("SELECT s.*, d.verified_at FROM site_domains d JOIN sites s ON s.design_id = d.design_id WHERE d.domain = $1").bind(&host).fetch_optional(&state.pool).await?;
            match row {
                Some(r) => {
                    use sqlx::{FromRow, Row};
                    Some((SiteRow::from_row(&r)?, r.try_get("verified_at")?))
                }
                None => None,
            }
        };
        let Some((site, Some(_))) = d else { return Ok::<Option<Reply>, ApiError>(None) };
        if method != Method::GET {
            return Ok(None);
        }
        if url == "/" || url.starts_with("/?") {
            let cookies = headers.get("cookie").and_then(|v| v.to_str().ok()).map(cookie::parse).unwrap_or_default();
            let ip = crate::http::req::client_ip(&headers, None);
            return Ok(Some(serve_site(state, &site, &headers, &cookies, &ip, false).await?));
        }
        if url == "/robots.txt" || url == "/sitemap.xml" {
            return Ok(Some(serve_seo(state, &site, &url[1..], &format!("https://{host}/")).await?));
        }
        Ok(None)
    }
    .await;
    match r {
        Ok(Some(reply)) => Some(reply.into_response()),
        Ok(None) => None,
        Err(e) => Some(e.into_response()),
    }
}

/// `rangeOf(q, now)`: a validated date range from `?from=&to=` (or `?days=`), clamped to a year.
pub fn range_of(q: &Value, now_ms: i64) -> (String, String) {
    let day = |k: &str| q.get(k).and_then(Value::as_str).filter(|v| insights::parse_day(v).is_some()).map(str::to_owned);
    let today = insights::utc_day(now_ms);
    let mut to = day("to").unwrap_or_else(|| today.clone());
    if to > today {
        to = today;
    }
    let n = js_number(q.get("days"));
    let n = if n == 0.0 || n.is_nan() { 30.0 } else { n };
    let days = n.clamp(1.0, 366.0);
    let to_ms = insights::parse_day(&to).unwrap_or(0) as f64;
    let mut from = day("from").unwrap_or_else(|| insights::utc_day((to_ms - (days - 1.0) * 86_400_000.0) as i64));
    let min = insights::utc_day((to_ms - 365.0 * 86_400_000.0) as i64);
    if from < min {
        from = min;
    }
    if from > to {
        from.clone_from(&to);
    }
    (from, to)
}

/// `Number(v)` of a query value (`undefined` → NaN, a repeated key → an array).
pub(crate) fn js_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::String(s)) => str_number(s),
        Some(Value::Array(a)) if a.len() == 1 => js_number(a.first()),
        Some(Value::Array(a)) if a.is_empty() => 0.0,
        Some(_) => f64::NAN,
    }
}

fn str_number(s: &str) -> f64 {
    let t = crate::js::trim(s);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    for (p, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(rest) = t.strip_prefix(p) {
            return if !rest.is_empty() && rest.chars().all(|c| c.is_digit(radix)) { u64::from_str_radix(rest, radix).map_or(f64::NAN, |n| n as f64) } else { f64::NAN };
        }
    }
    static NUM: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$").expect("regex"));
    if NUM.is_match(t) { t.parse().unwrap_or(f64::NAN) } else { f64::NAN }
}

async fn can_edit(s: &AppState, user_id: Uuid, id: &str) -> Result<bool, ApiError> {
    Ok(at_least(design_role(s, Some(user_id), id).await?, Role::Edit))
}

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

// ---------------------------------------------------------------- public serving

async fn public_site(State(s): State<AppState>, Path(slug): Path<String>, req: Req) -> ApiResult {
    let Some(site) = site_by_slug(&s, &slug).await? else { return Ok(site_not_found()) };
    serve_site(&s, &site, &req.headers, &req.cookies, &req.ip, true).await
}

async fn public_seo(State(s): State<AppState>, Path((slug, file)): Path<(String, String)>, req: Req) -> ApiResult {
    let _ = &req;
    if file != "robots.txt" && file != "sitemap.xml" {
        return Ok(Reply::empty(404));
    }
    let Some(site) = site_by_slug(&s, &slug).await? else { return Ok(Reply::empty(404)) };
    serve_seo(&s, &site, &file, &format!("{}/site/{slug}/", s.cfg.web_origin)).await
}

// ---------------------------------------------------------------- insights beacons (public)

/// Site beacon: sections seen, outbound clicks, form submissions. Sent with sendBeacon (text/plain JSON).
async fn beacon(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    if insights::do_not_track(&req.headers) || insights::is_bot(insights::header(&req.headers, "user-agent")) {
        return Ok(Reply::empty(204));
    }
    let site: Option<(bool, bool)> = sqlx::query_as("SELECT published, analytics FROM sites WHERE design_id = $1").bind(&id).fetch_optional(&s.pool).await?;
    if !matches!(site, Some((true, true))) {
        return Ok(Reply::empty(204));
    }
    let body = match &req.body {
        crate::http::req::Body::Text(t) => jsjson::parse(t).ok(),
        crate::http::req::Body::Json(v) => Some(v.clone()),
        _ => None,
    };
    if let Some(Value::Array(events)) = body.as_ref().and_then(|b| b.get("e")) {
        insights::record_events(&s, &id, events).await?;
    }
    Ok(Reply::empty(204))
}

/// Public view links (/view/:id): one beacon per open.
async fn view_beacon(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let d: Option<(bool, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as("SELECT public_view, trashed_at FROM designs WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    if !matches!(d, Some((true, None))) {
        return Ok(Reply::empty(404));
    }
    let _ = insights::record_visit(&s, "view", &id, &req.headers, &req.ip, &[web_host(&s)]).await;
    Ok(Reply::empty(204))
}

async fn unlock(State(s): State<AppState>, Path(slug): Path<String>, req: Req) -> ApiResult {
    let site = site_by_slug(&s, &slug).await?;
    let Some(site) = site.filter(|x| x.password_hash.as_deref().is_some_and(|p| !p.is_empty())) else { return Ok(Reply::redirect(&format!("/site/{slug}"))) };
    let pw = match req.body() {
        Some(Value::Object(m)) => match m.get("password") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(p)) => p.clone(),
            Some(_) => return Err(ApiError::internal("password must be a string")),
        },
        _ => String::new(),
    };
    if !verify_password(&pw, site.password_hash.as_deref()).await {
        return Ok(html(401, password_page(&site, true)));
    }
    let c = cookie::serialize(
        &cookie_name(&site.slug),
        &unlock_token(&s, &site),
        &cookie::CookieOpts { http_only: true, same_site: Some("lax"), secure: s.prod(), path: Some("/".into()), max_age: Some(7 * 86400), ..Default::default() },
    );
    Ok(Reply::redirect(&format!("/site/{}", site.slug)).cookie(c))
}

/// Caddy on-demand TLS asks before issuing a certificate for a host.
async fn domain_check(State(s): State<AppState>, req: Req) -> ApiResult {
    let found = match req.query.get("domain") {
        None => false,
        Some(Value::String(d)) if d.is_empty() => false,
        Some(Value::String(d)) => pg::one(&s.pool, "SELECT 1 FROM site_domains WHERE domain = $1 AND verified_at IS NOT NULL", &[pg::text(d.to_lowercase())]).await?.is_some(),
        // `domain.toLowerCase` of an array is not a function.
        Some(_) => return Err(ApiError::internal("domain.toLowerCase is not a function")),
    };
    Ok(if found { Reply::json(json!({ "ok": true })) } else { reply::json(404, json!({ "ok": false })) })
}

async fn og_image(State(s): State<AppState>, Path(file): Path<String>, req: Req) -> ApiResult {
    let _ = &req;
    let id = file.strip_suffix(".png").unwrap_or(&file).to_owned();
    let t: Option<Option<String>> = sqlx::query_scalar("SELECT d.thumb_sha256 FROM designs d JOIN sites s ON s.design_id = d.id WHERE d.id = $1 AND s.published").bind(&id).fetch_optional(&s.pool).await?;
    let Some(Some(sha)) = t.filter(|t| t.as_deref().is_some_and(|x| !x.is_empty())) else { return Ok(Reply::empty(404)) };
    Ok(Reply::bytes("image/png", s.store.get(&sha).await?).header("cache-control", "public, max-age=3600"))
}

// ---------------------------------------------------------------- editor

async fn get_site(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !can_edit(&s, u.id, &id).await? {
        return Ok(not_found());
    }
    let site = site_by_design(&s, &id).await?;
    let domains = pg::json_rows(&s.pool, "SELECT domain, verified_at, token FROM site_domains WHERE design_id = $1 ORDER BY created_at", &[pg::text(&id)]).await?;
    let views = pg::json_rows(&s.pool, "SELECT to_char(day, 'YYYY-MM-DD') AS day, views FROM site_views WHERE design_id = $1 AND day > current_date - 30 ORDER BY day", &[pg::text(&id)]).await?;
    let (site_json, url) = match &site {
        Some(x) => (
            json!({
                "slug": x.slug, "title": x.title, "description": x.description, "published": x.published, "hasPassword": x.password_hash.as_deref().is_some_and(|p| !p.is_empty()),
                "noindex": x.noindex, "analytics": x.analytics, "version": x.version, "publishedAt": x.published_at.map(|t| oc_js::iso_string(&t)),
            }),
            json!(format!("{}/site/{}", s.cfg.web_origin, x.slug)),
        ),
        None => (Value::Null, Value::Null),
    };
    Ok(Reply::json(json!({ "site": site_json, "url": url, "domains": domains, "views": views })))
}

async fn put_site(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !can_edit(&s, u.id, &id).await? {
        return Ok(not_found());
    }
    let b = z::obj([
        ("slug", z::str().to_lower_case()),
        ("title", z::str().min(1.0).max(120.0)),
        ("description", z::str().max(300.0).optional()),
        ("noindex", z::bool().optional()),
        ("analytics", z::bool().optional()),
        ("password", z::str().max(200.0).nullable().optional()),
    ])
    .parse(req.body())?;
    let slug = b["slug"].as_str().unwrap_or("").to_owned();
    if !SLUG.is_match(&slug) || RESERVED.contains(&slug.as_str()) {
        return Ok(reply::json(400, json!({ "error": "bad_slug", "message": "Use 3–48 lowercase letters, numbers and dashes." })));
    }
    let taken: Option<String> = sqlx::query_scalar("SELECT design_id FROM sites WHERE slug = $1").bind(&slug).fetch_optional(&s.pool).await?;
    if taken.is_some_and(|t| t != id) {
        return Ok(reply::json(409, json!({ "error": "taken", "message": "That address is taken." })));
    }
    // undefined: keep; '' or null: remove; otherwise the new password.
    let pw: Option<Option<String>> = match b.get("password") {
        None => None,
        Some(Value::String(p)) if !p.is_empty() => Some(Some(hash_password(p).await)),
        Some(_) => Some(None),
    };
    sqlx::query(
        "INSERT INTO sites(design_id, slug, title, description, noindex, password_hash, analytics) VALUES ($1,$2,$3,$4,$5,$6,COALESCE($8, true))
         ON CONFLICT (design_id) DO UPDATE SET slug = $2, title = $3, description = $4, noindex = $5,
           password_hash = CASE WHEN $7 THEN $6 ELSE sites.password_hash END, analytics = COALESCE($8, sites.analytics), updated_at = now()",
    )
    .bind(&id)
    .bind(&slug)
    .bind(b["title"].as_str())
    .bind(b.get("description").and_then(Value::as_str))
    .bind(b.get("noindex").and_then(Value::as_bool).unwrap_or(false))
    .bind(pw.clone().flatten())
    .bind(pw.is_some())
    .bind(b.get("analytics").and_then(Value::as_bool))
    .execute(&s.pool)
    .await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn publish(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !can_edit(&s, u.id, &id).await? {
        return Ok(not_found());
    }
    if let Some(gate) = approval_gate(&s, u.id, &id).await? {
        return Ok(reply::json(403, gate)); // brand approvals (A8)
    }
    let Some(site) = site_by_design(&s, &id).await? else { return Ok(reply::json(400, json!({ "error": "no_site", "message": "Choose an address first." }))) };
    let page = build(&s, &id, &site.title, site.description.as_deref(), site.noindex).await?;
    let sha = s.store.put(page.as_bytes(), Some("text/html")).await?;
    // The build takes a while: the publisher and the owner must still be active when it lands, locked so a
    // disable or squat tombstone (which takes the site offline) can't commit in between (D1).
    let mut tx = s.pool.begin().await?;
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_id FROM designs WHERE id = $1").bind(&id).fetch_optional(&mut *tx).await?;
    let live = match owner {
        Some(o) => auth::lock_active_user(&mut tx, u.id).await?.is_some() && auth::lock_active_user(&mut tx, o).await?.is_some(),
        None => false,
    };
    if live {
        sqlx::query("UPDATE sites SET html_sha256 = $2, published = true, version = version + 1, published_at = now(), updated_at = now() WHERE design_id = $1").bind(&id).bind(&sha).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if !live {
        return Ok(reply::json(403, json!({ "error": "disabled", "message": crate::login::DISABLED_MESSAGE })));
    }
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: Some(u.id), action: "site.published".into(), target: Some(id.clone()), meta: Some(json!({ "slug": site.slug })), ip: Some(req.ip.clone()), ..Default::default() },
        s.now(),
    )
    .await?;
    let url = format!("{}/site/{}", s.cfg.web_origin, site.slug);
    let (s2, uid, data) = (s.clone(), u.id, json!({ "designId": id, "url": url }));
    s.spawn(async move {
        let _ = oc_webhooks::emit_event(&s2.pool, uid, oc_webhooks::WebhookEvent::SitePublished, &data).await;
    });
    Ok(Reply::json(json!({ "ok": true, "url": url })))
}

/// Insights for a date range (default: the last 30 days, at most a year).
async fn site_insights(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !can_edit(&s, u.id, &id).await? {
        return Ok(not_found());
    }
    let (from, to) = range_of(&req.query, s.now_ms());
    Ok(Reply::json(insights::load_summary(&s, &id, "site", &from, &to).await?))
}

/// Views of the public view link (Share dialog).
async fn view_insights(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !can_edit(&s, u.id, &id).await? {
        return Ok(not_found());
    }
    let (from, to) = range_of(&req.query, s.now_ms());
    let sum = insights::load_summary(&s, &id, "view", &from, &to).await?;
    Ok(Reply::json(json!({ "from": from, "to": to, "views": sum["totals"]["views"], "visitors": sum["totals"]["visitors"], "series": sum["series"] })))
}

/// Build the site HTML without publishing (the editor's mobile/desktop preview).
async fn preview(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !can_edit(&s, u.id, &id).await? {
        return Ok(not_found());
    }
    let site = site_by_design(&s, &id).await?;
    let title: Option<String> = sqlx::query_scalar("SELECT title FROM designs WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let t = site.as_ref().map(|x| x.title.clone()).or(title).unwrap_or_else(|| "Preview".into());
    let page = build(&s, &id, &t, site.as_ref().and_then(|x| x.description.as_deref()), true).await?;
    Ok(Reply::json(json!({ "html": page })))
}

async fn unpublish(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !can_edit(&s, u.id, &id).await? {
        return Ok(not_found());
    }
    pg::exec(&s.pool, "UPDATE sites SET published = false, updated_at = now() WHERE design_id = $1", &[pg::text(&id)]).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn add_domain(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !can_edit(&s, u.id, &id).await? {
        return Ok(not_found());
    }
    billing::require_feature(&s, u.id, billing::Feature::CustomDomains, "Custom domains are part of Pro.").await?;
    let b = z::obj([("domain", z::str().to_lower_case().regex_fn(DOMAIN_JS, domain_ok))]).parse(req.body())?;
    let domain = b["domain"].as_str().unwrap_or("").to_owned();
    let host = web_host(&s);
    if domain.ends_with(&host) {
        return Ok(reply::json(400, json!({ "error": "bad_domain" })));
    }
    if pg::one(&s.pool, "SELECT 1 FROM sites WHERE design_id = $1", &[pg::text(&id)]).await?.is_none() {
        return Ok(reply::json(400, json!({ "error": "no_site", "message": "Set up the site first." })));
    }
    let token = hex::encode(auth::random_bytes(12));
    pg::exec(&s.pool, "INSERT INTO site_domains(domain, design_id, token) VALUES ($1,$2,$3) ON CONFLICT (domain) DO NOTHING", &[pg::text(&domain), pg::text(&id), pg::text(&token)]).await?;
    let row: Option<(String, String)> = sqlx::query_as("SELECT design_id, token FROM site_domains WHERE domain = $1").bind(&domain).fetch_optional(&s.pool).await?;
    let Some((_, token)) = row.filter(|(d, _)| *d == id) else {
        return Ok(reply::json(409, json!({ "error": "taken", "message": "That domain is connected to another site." })));
    };
    Ok(Reply::json(json!({
        "domain": domain, "txt": { "name": format!("_pinselo.{domain}"), "value": format!("oc-verify={token}") }, "cname": { "name": domain, "value": host },
    })))
}

async fn verify_domain(State(s): State<AppState>, Path((id, domain)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !can_edit(&s, u.id, &id).await? {
        return Ok(Reply::empty(404));
    }
    let token: Option<String> = sqlx::query_scalar("SELECT token FROM site_domains WHERE domain = $1 AND design_id = $2").bind(&domain).bind(&id).fetch_optional(&s.pool).await?;
    let Some(token) = token else { return Ok(Reply::empty(404)) };
    let want = format!("oc-verify={token}");
    let records = dns::resolve_txt(&format!("_pinselo.{domain}")).await;
    if !records.contains(&want) {
        return Ok(reply::json(
            400,
            json!({ "error": "not_verified", "message": format!("Add a TXT record at _pinselo.{domain} with the value oc-verify={token}. DNS changes can take a while.") }),
        ));
    }
    pg::exec(&s.pool, "UPDATE site_domains SET verified_at = now() WHERE domain = $1", &[pg::text(&domain)]).await?;
    oc_audit::audit_at(&s.pool, oc_audit::AuditEvent { actor_id: Some(u.id), action: "site.domain_verified".into(), target: Some(domain.clone()), ip: Some(req.ip.clone()), ..Default::default() }, s.now()).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn remove_domain(State(s): State<AppState>, Path((id, domain)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !can_edit(&s, u.id, &id).await? {
        return Ok(Reply::empty(404));
    }
    pg::exec(&s.pool, "DELETE FROM site_domains WHERE domain = $1 AND design_id = $2", &[pg::text(&domain), pg::text(&id)]).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

/// `dns.promises.resolveTxt`: every record is its strings joined (`r.join('')`); any failure is no records (the
/// route's `.catch(() => [])`). A resolver per lookup, so a retry after adding the record never hits a cached miss.
mod dns {
    use hickory_resolver::Resolver;
    use hickory_resolver::proto::rr::{RData, RecordType};

    pub async fn resolve_txt(name: &str) -> Vec<String> {
        let Ok(resolver) = Resolver::builder_tokio().and_then(|b| b.build()) else { return vec![] };
        // The trailing dot keeps the host's search domains out of it.
        let Ok(lookup) = resolver.lookup(format!("{}.", name.trim_end_matches('.')), RecordType::TXT).await else { return vec![] };
        lookup
            .answers()
            .iter()
            .filter_map(|r| match &r.data {
                RData::TXT(txt) => Some(String::from_utf8_lossy(&txt.txt_data.concat()).into_owned()),
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        // Needs the network; run by hand with `--ignored`.
        #[tokio::test]
        #[ignore]
        async fn resolves_large_txt_sets() {
            let records = super::resolve_txt("google.com").await;
            assert!(records.iter().any(|r| r.starts_with("v=spf1")), "{records:?}");
            assert!(super::resolve_txt("no-such-name.invalid").await.is_empty());
        }
    }
}
