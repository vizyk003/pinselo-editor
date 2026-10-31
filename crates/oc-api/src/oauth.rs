//! Port of services/api/src/oauth.ts — the OAuth 2.1 authorization server of the AI connector (MCP). An assistant
//! registers itself (dynamic client registration, RFC 7591, or a client ID metadata document: an https URL as
//! `client_id`), sends the user to the consent page, and trades the PKCE-protected code for a 1-hour access token
//! (`oc_oat_…`) and a 60-day refresh token (`oc_ort_…`). Both live on one `api_tokens` row (kind `oauth`);
//! refreshing revokes the row and issues the next one in the same family, and replaying a rotated refresh token (or
//! a used code) revokes the whole family. Only SHA-256 hashes of codes, tokens and client secrets are stored.
//!
//! Test seam (not in production): with `OC_OAUTH_FAKE_BASE` set, a client metadata document `https://host/path` is
//! fetched from `<base>/host/path` (services/api's oauth.ts has the same seam).

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::bearer::{new_token, sha256};
use crate::http::error::ApiError;
use crate::js;
use crate::pg::{self, Arg};
use crate::state::AppState;

/// What a connected app can be granted: every API scope except managing webhooks.
pub const OAUTH_SCOPES: [&str; 4] = ["designs:read", "designs:write", "assets:write", "exports"];
pub const DEFAULT_SCOPES: [&str; 4] = ["designs:read", "designs:write", "assets:write", "exports"];
/// Asked for by generic clients; refresh tokens are always issued and there is no OpenID Connect layer.
const IGNORED_SCOPES: [&str; 4] = ["openid", "profile", "email", "offline_access"];
pub const ACCESS_TTL: i64 = 3600;
const REFRESH_DAYS: i32 = 60;
const CIMD_MAX_AGE_MS: i64 = 24 * 3600 * 1000;
const CIMD_MAX_BYTES: usize = 64 * 1024;
const CIMD_TIMEOUT: Duration = Duration::from_secs(5);
const LOOPBACK: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

pub fn mcp_resource(s: &AppState) -> String {
    format!("{}/mcp", s.cfg.web_origin)
}

/// An RFC 6749 error: `error` is the protocol code, the message is safe to show.
#[derive(Debug)]
pub struct OAuthError {
    pub error: &'static str,
    pub message: String,
    pub status: u16,
}

fn oe(error: &'static str, message: impl Into<String>) -> OAuthError {
    OAuthError { error, message: message.into(), status: 400 }
}

/// `OAuthError` or an unexpected failure (database …).
#[derive(Debug)]
pub enum Fail {
    OAuth(OAuthError),
    Api(ApiError),
}

impl From<OAuthError> for Fail {
    fn from(e: OAuthError) -> Self {
        Fail::OAuth(e)
    }
}
impl From<ApiError> for Fail {
    fn from(e: ApiError) -> Self {
        Fail::Api(e)
    }
}
impl From<sqlx::Error> for Fail {
    fn from(e: sqlx::Error) -> Self {
        Fail::Api(e.into())
    }
}

pub type OResult<T> = Result<T, Fail>;

// ---------------------------------------------------------------- discovery

/// RFC 9728: where the MCP endpoint's tokens come from.
pub fn protected_resource_metadata(s: &AppState) -> Value {
    json!({
        "resource": mcp_resource(s),
        "authorization_servers": [s.cfg.web_origin],
        "scopes_supported": OAUTH_SCOPES,
        "bearer_methods_supported": ["header"],
        "resource_name": s.cfg.brand.name,
    })
}

/// RFC 8414 authorization server metadata.
pub fn authorization_server_metadata(s: &AppState) -> Value {
    let o = &s.cfg.web_origin;
    json!({
        "issuer": o,
        "authorization_endpoint": format!("{o}/oauth/authorize"),
        "token_endpoint": format!("{o}/api/oauth/token"),
        "registration_endpoint": format!("{o}/api/oauth/register"),
        "revocation_endpoint": format!("{o}/api/oauth/revoke"),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "revocation_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "scopes_supported": OAUTH_SCOPES,
        "authorization_response_iss_parameter_supported": true,
        "client_id_metadata_document_supported": true,
    })
}

// ---------------------------------------------------------------- request checks

/// `new URL(raw)` (WHATWG; the `url` crate).
fn parse_url(raw: &str) -> Option<url::Url> {
    url::Url::parse(raw).ok()
}

/// `u.hostname` (IPv6 in brackets).
fn hostname(u: &url::Url) -> String {
    match u.host() {
        Some(url::Host::Ipv6(a)) => format!("[{a}]"),
        Some(h) => h.to_string(),
        None => String::new(),
    }
}

/// `new URL(raw).host`: the host with a non-default port.
pub fn url_host(raw: &str) -> Option<String> {
    let u = parse_url(raw)?;
    let h = hostname(&u);
    Some(match u.port() {
        Some(p) => format!("{h}:{p}"),
        None => h,
    })
}

/// `clientIdentity(client)`: a metadata-document client's domain is verified (it was fetched from that https URL);
/// a dynamically registered client named itself.
pub fn client_identity(c: &OAuthClient) -> (bool, Option<String>) {
    if c.kind == "cimd" { (true, url_host(&c.id)) } else { (false, None) }
}

/// Why a redirect URI can't be registered, or `None`: https, or http on a loopback host, never a fragment.
pub fn redirect_uri_problem(raw: &str) -> Option<&'static str> {
    if js::len(raw) > 2000 {
        return Some("is too long");
    }
    let Some(u) = parse_url(raw) else { return Some("is not a URL") };
    if raw.contains('#') {
        return Some("must not contain a fragment");
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Some("must not contain credentials");
    }
    if u.scheme() == "https" {
        return None;
    }
    if u.scheme() == "http" && LOOPBACK.contains(&hostname(&u).as_str()) {
        return None;
    }
    Some("must use https (http only on localhost, 127.0.0.1 or [::1])")
}

/// Exact match — except that a loopback redirect may use any port (RFC 8252 §7.3).
pub fn redirect_matches(registered: &[String], uri: &str) -> bool {
    if registered.iter().any(|r| r == uri) {
        return true;
    }
    let Some(u) = parse_url(uri) else { return false };
    if u.scheme() != "http" || !LOOPBACK.contains(&hostname(&u).as_str()) || uri.contains('#') {
        return false;
    }
    registered.iter().any(|r| {
        let Some(mut x) = parse_url(r) else { return false };
        if x.scheme() != "http" || hostname(&x) != hostname(&u) {
            return false;
        }
        if x.set_port(u.port()).is_err() {
            return false;
        }
        x.as_str() == u.as_str()
    })
}

/// `String(v ?? '')` for request values.
fn js_str(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => js::num(n.as_f64().unwrap_or(f64::NAN)),
        Some(Value::Array(a)) => a.iter().map(|x| if x.is_null() { String::new() } else { js_str(Some(x)) }).collect::<Vec<_>>().join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// The requested API scopes (generic OIDC ones dropped), or `None` when one can't be granted.
fn asked_scopes(raw: Option<&Value>) -> Option<Vec<String>> {
    let s = js_str(raw);
    let asked: Vec<String> = s.split(js::is_ws).filter(|x| !x.is_empty() && !IGNORED_SCOPES.contains(x)).map(str::to_owned).collect();
    if asked.iter().any(|x| !OAUTH_SCOPES.contains(&x.as_str())) { None } else { Some(asked) }
}

/// Scopes to grant, in canonical order; the default set when none is asked for.
pub fn parse_scopes(raw: Option<&Value>) -> Option<Vec<String>> {
    let asked = asked_scopes(raw)?;
    Some(if asked.is_empty() {
        DEFAULT_SCOPES.iter().map(|s| (*s).to_owned()).collect()
    } else {
        OAUTH_SCOPES.iter().filter(|s| asked.iter().any(|a| a == *s)).map(|s| (*s).to_owned()).collect()
    })
}

/// RFC 8707 resource indicator: `Ok(None)` when absent, the MCP endpoint when it names this server, `Err` otherwise.
/// A resource indicator that names another server.
#[derive(Debug)]
pub struct OtherResource;

pub fn parse_resource(s: &AppState, raw: Option<&Value>) -> Result<Option<String>, OtherResource> {
    match raw {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(x)) if x.is_empty() => return Ok(None),
        _ => {}
    }
    let r = js_str(raw);
    let r = r.trim_end_matches('/');
    if r == mcp_resource(s) || r == s.cfg.web_origin { Ok(Some(mcp_resource(s))) } else { Err(OtherResource) }
}

fn invalid_target(s: &AppState) -> OAuthError {
    oe("invalid_target", format!("Tokens are only issued for {}.", mcp_resource(s)))
}

/// `pkceChallenge(verifier)`: base64url(sha256(verifier)).
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn safe_equal(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq as _;
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

fn clean_name(s: &str) -> String {
    let no_ctl: String = s.chars().filter(|c| !matches!(*c as u32, 0..=0x1f | 0x7f)).collect();
    js::take(js::trim(&no_ctl), 80)
}

fn url_or_null(raw: Option<&Value>, https_only: bool) -> Option<String> {
    let Some(Value::String(r)) = raw else { return None };
    if js::len(r) > 2000 {
        return None;
    }
    let u = parse_url(r)?;
    (u.scheme() == "https" || (!https_only && u.scheme() == "http")).then(|| u.as_str().to_owned())
}

fn is_challenge(s: &str) -> bool {
    s.len() == 43 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

fn is_verifier(s: &str) -> bool {
    (43..=128).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"._~-".contains(&c))
}

// ---------------------------------------------------------------- clients

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OAuthClient {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub client_uri: Option<String>,
    pub logo_uri: Option<String>,
    pub client_secret_sha256: Option<String>,
    pub fetched_at: Option<chrono::DateTime<chrono::Utc>>,
}

const CLIENT_COLS: &str = "id, kind, name, redirect_uris, client_uri, logo_uri, client_secret_sha256, fetched_at";

struct Metadata {
    name: String,
    redirect_uris: Vec<String>,
    client_uri: Option<String>,
    logo_uri: Option<String>,
}

/// Display metadata shared by registration and metadata documents.
fn client_metadata(m: &Map<String, Value>, default_name: &str) -> Result<Metadata, OAuthError> {
    let uris = match m.get("redirect_uris") {
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= 20 && a.iter().all(Value::is_string) => {
            a.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>()
        }
        _ => return Err(oe("invalid_redirect_uri", "redirect_uris must be a list of 1 to 20 URLs.")),
    };
    for u in &uris {
        if let Some(p) = redirect_uri_problem(u) {
            return Err(oe("invalid_redirect_uri", format!("Redirect URI {u} {p}.")));
        }
    }
    let name = match m.get("client_name") {
        None => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(oe("invalid_client_metadata", "client_name must be a string.")),
    };
    let mut seen = Vec::new();
    for u in uris {
        if !seen.contains(&u) {
            seen.push(u);
        }
    }
    let n = clean_name(&name);
    Ok(Metadata {
        name: if n.is_empty() { default_name.to_owned() } else { n },
        redirect_uris: seen,
        client_uri: url_or_null(m.get("client_uri"), false),
        logo_uri: url_or_null(m.get("logo_uri"), true),
    })
}

/// RFC 7591 dynamic client registration.
pub async fn register_client(s: &AppState, body: &Map<String, Value>) -> OResult<Value> {
    let method = match body.get("token_endpoint_auth_method") {
        None | Some(Value::Null) => "none".to_owned(),
        Some(Value::String(m)) if ["none", "client_secret_post", "client_secret_basic"].contains(&m.as_str()) => m.clone(),
        Some(_) => return Err(oe("invalid_client_metadata", "token_endpoint_auth_method must be none, client_secret_post or client_secret_basic.").into()),
    };
    if let Some(g) = body.get("grant_types") {
        let ok = match g {
            Value::Array(a) => {
                a.iter().any(|x| x == "authorization_code") && a.iter().all(|x| x == "authorization_code" || x == "refresh_token")
            }
            _ => false,
        };
        if !ok {
            return Err(oe("invalid_client_metadata", "Only the authorization_code and refresh_token grants are supported.").into());
        }
    }
    if let Some(r) = body.get("response_types") {
        let ok = matches!(r, Value::Array(a) if a.iter().all(|x| x == "code"));
        if !ok {
            return Err(oe("invalid_client_metadata", "Only response_type code is supported.").into());
        }
    }
    let m = client_metadata(body, "AI assistant")?;
    let id = Uuid::new_v4().to_string();
    let secret = (method != "none").then(|| format!("oc_cs_{}", URL_SAFE_NO_PAD.encode(crate::auth::random_bytes(32))));
    let created: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "INSERT INTO oauth_clients(id, kind, name, redirect_uris, client_uri, logo_uri, client_secret_sha256) VALUES ($1,'dcr',$2,$3,$4,$5,$6) RETURNING created_at",
    )
    .bind(&id)
    .bind(&m.name)
    .bind(&m.redirect_uris)
    .bind(&m.client_uri)
    .bind(&m.logo_uri)
    .bind(secret.as_deref().map(sha256))
    .fetch_one(&s.pool)
    .await?;
    let mut out = Map::new();
    out.insert("client_id".into(), json!(id));
    out.insert("client_id_issued_at".into(), json!(created.timestamp_millis().div_euclid(1000)));
    out.insert("client_name".into(), json!(m.name));
    out.insert("redirect_uris".into(), json!(m.redirect_uris));
    out.insert("grant_types".into(), json!(["authorization_code", "refresh_token"]));
    out.insert("response_types".into(), json!(["code"]));
    out.insert("token_endpoint_auth_method".into(), json!(method));
    out.insert("scope".into(), json!(OAUTH_SCOPES.join(" ")));
    if let Some(u) = m.client_uri {
        out.insert("client_uri".into(), json!(u));
    }
    if let Some(u) = m.logo_uri {
        out.insert("logo_uri".into(), json!(u));
    }
    if let Some(sec) = secret {
        out.insert("client_secret".into(), json!(sec));
        out.insert("client_secret_expires_at".into(), json!(0));
    }
    Ok(Value::Object(out))
}

/// A client ID metadata document URL: https with a path, without fragment, credentials or dot segments.
pub fn is_metadata_url(id: &str) -> bool {
    static DOTS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"/\.\.?(/|\?|$)").expect("regex"));
    if !id.starts_with("https://") || id.contains('#') || DOTS.is_match(id) {
        return false;
    }
    parse_url(id).is_some_and(|u| u.path() != "/" && u.username().is_empty() && u.password().is_none())
}

/// `isPublicAddress(ip)` (services/api/src/netguard.ts): refuses loopback, private, link-local (cloud metadata),
/// CGNAT, documentation, benchmarking, multicast and reserved ranges, and IPv6 forms that can wrap an IPv4 address
/// (mapped, compatible, NAT64, 6to4, Teredo).
pub fn is_public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    let v4_blocked: [(u32, u32); 15] = [
        (0x0000_0000, 8), (0x0a00_0000, 8), (0x6440_0000, 10), (0x7f00_0000, 8), (0xa9fe_0000, 16), (0xac10_0000, 12), (0xc000_0000, 24), (0xc000_0200, 24),
        (0xc058_6300, 24), (0xc0a8_0000, 16), (0xc612_0000, 15), (0xc633_6400, 24), (0xcb00_7100, 24), (0xe000_0000, 4), (0xf000_0000, 4),
    ];
    let v6_blocked: [(u128, u32); 12] = [
        (0, 96), (0xffff_0000_0000, 96), (0x0064_ff9b_0000_0000_0000_0000_0000_0000, 96), (0x0064_ff9b_0001_0000_0000_0000_0000_0000, 48),
        (0x0100_0000_0000_0000_0000_0000_0000_0000, 64), (0x2001_0000_0000_0000_0000_0000_0000_0000, 32), (0x2001_0db8_0000_0000_0000_0000_0000_0000, 32),
        (0x2002_0000_0000_0000_0000_0000_0000_0000, 16), (0xfc00_0000_0000_0000_0000_0000_0000_0000, 7), (0xfe80_0000_0000_0000_0000_0000_0000_0000, 10),
        (0xfec0_0000_0000_0000_0000_0000_0000_0000, 10), (0xff00_0000_0000_0000_0000_0000_0000_0000, 8),
    ];
    match ip {
        IpAddr::V4(a) => {
            let x = u32::from(a);
            !v4_blocked.iter().any(|&(net, bits)| x >> (32 - bits) == net >> (32 - bits))
        }
        IpAddr::V6(a) => {
            let x = u128::from(a);
            !v6_blocked.iter().any(|&(net, bits)| x >> (128 - bits) == net >> (128 - bits))
        }
    }
}

/// `publicLookup`: every address `host` resolves to, refused if ANY is not public. The client then connects to
/// exactly these addresses (`resolve_to_addrs`), so DNS rebinding can't slip in between the check and the connect.
pub async fn resolve_public(host: &str, port: u16) -> Result<Vec<std::net::SocketAddr>, String> {
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port)).await.map_err(|e| e.to_string())?.collect();
    if addrs.is_empty() || addrs.iter().any(|a| !is_public_ip(a.ip())) {
        return Err(format!("{host} resolves to a non-public address"));
    }
    Ok(addrs)
}

/// GET a metadata document: public hosts only (the name, an IP literal, and every address it resolves to, pinned for
/// the connection), no redirects, 5 s in all, at most 64 KB of JSON.
async fn fetch_metadata(s: &AppState, url: &str) -> Result<Map<String, Value>, String> {
    let u = parse_url(url).ok_or("bad url")?;
    if !crate::stock::is_public_host(&hostname(&u)) {
        return Err("not a public host".into());
    }
    match u.host() {
        Some(url::Host::Ipv4(a)) if !is_public_ip(a.into()) => return Err("not a public address".into()),
        Some(url::Host::Ipv6(a)) if !is_public_ip(a.into()) => return Err("not a public address".into()),
        _ => {}
    }
    let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(CIMD_TIMEOUT);
    let target = match s.env_nonempty("OC_OAUTH_FAKE_BASE").filter(|_| !s.prod()) {
        Some(base) => format!("{base}/{}", &url["https://".len()..]),
        None => {
            if let Some(url::Host::Domain(d)) = u.host() {
                let addrs = tokio::time::timeout(CIMD_TIMEOUT, resolve_public(d, u.port_or_known_default().unwrap_or(443))).await.map_err(|_| "dns timeout".to_owned())??;
                builder = builder.resolve_to_addrs(d, &addrs);
            }
            url.to_owned()
        }
    };
    let client = builder.build().map_err(|e| e.to_string())?;
    let mut r = client
        .get(&target)
        .header("accept", "application/json")
        .header("user-agent", format!("{}-oauth/1.0", s.cfg.brand.name))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if r.status().is_redirection() {
        return Err("redirects are not followed".into());
    }
    if !r.status().is_success() {
        return Err(format!("HTTP {}", r.status().as_u16()));
    }
    if r.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) > CIMD_MAX_BYTES as f64 {
        return Err("too large".into());
    }
    let mut buf = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(|e| e.to_string())? {
        buf.extend_from_slice(&chunk);
        if buf.len() > CIMD_MAX_BYTES {
            return Err("too large".into());
        }
    }
    match serde_json::from_slice::<Value>(&buf).map_err(|e| e.to_string())? {
        Value::Object(m) => Ok(m),
        _ => Err("not a JSON object".into()),
    }
}

async fn client_row(s: &AppState, id: &str) -> Result<Option<OAuthClient>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {CLIENT_COLS} FROM oauth_clients WHERE id = $1")).bind(id).fetch_optional(&s.pool).await
}

/// The registered client, fetching (and caching for a day) a metadata document for URL client IDs.
pub async fn load_client(s: &AppState, id: &str) -> OResult<OAuthClient> {
    if id.is_empty() || js::len(id) > 2000 {
        return Err(oe("invalid_client", "Missing or unknown client_id.").into());
    }
    let row = client_row(s, id).await?;
    if !is_metadata_url(id) {
        return row.ok_or_else(|| oe("invalid_client", "This app isn’t registered (unknown client_id).").into());
    }
    if let Some(r) = &row
        && let Some(f) = r.fetched_at
        && s.now_ms() - f.timestamp_millis() < CIMD_MAX_AGE_MS
    {
        return Ok(r.clone());
    }
    let doc = match fetch_metadata(s, id).await {
        Ok(d) => d,
        Err(_) => {
            // A cached copy keeps working while the document's host is briefly unreachable.
            if let Some(r) = row {
                return Ok(r);
            }
            return Err(oe("invalid_client", format!("Couldn’t load the app’s client metadata document from {id}.")).into());
        }
    };
    if doc.get("client_id") != Some(&Value::String(id.to_owned())) {
        return Err(oe("invalid_client", "The client metadata document’s client_id doesn’t match its URL.").into());
    }
    if !matches!(doc.get("client_name"), Some(Value::String(n)) if !clean_name(n).is_empty()) {
        return Err(oe("invalid_client", "The client metadata document has no client_name.").into());
    }
    if doc.get("token_endpoint_auth_method").is_some_and(|m| m != "none") {
        return Err(oe("invalid_client", "Client metadata documents must use token_endpoint_auth_method \"none\".").into());
    }
    let m = client_metadata(&doc, "").map_err(|e| oe("invalid_client", format!("The client metadata document is invalid: {}", e.message)))?;
    let c: OAuthClient = sqlx::query_as(&format!(
        "INSERT INTO oauth_clients(id, kind, name, redirect_uris, client_uri, logo_uri, fetched_at) VALUES ($1,'cimd',$2,$3,$4,$5, now())
     ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, redirect_uris = EXCLUDED.redirect_uris, client_uri = EXCLUDED.client_uri,
       logo_uri = EXCLUDED.logo_uri, fetched_at = now()
     RETURNING {CLIENT_COLS}"
    ))
    .bind(id)
    .bind(&m.name)
    .bind(&m.redirect_uris)
    .bind(&m.client_uri)
    .bind(&m.logo_uri)
    .fetch_one(&s.pool)
    .await?;
    Ok(c)
}

// ---------------------------------------------------------------- authorization (consent page)

pub struct AuthorizeRequest {
    pub client: OAuthClient,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub resource: Option<String>,
    pub code_challenge: String,
    pub state: Option<String>,
}

/// Validates an authorization request (zod-parsed params); the client and its redirect URI are checked first.
pub async fn check_authorization(s: &AppState, p: &Value) -> OResult<AuthorizeRequest> {
    let g = |k: &str| p.get(k).and_then(Value::as_str);
    let client = load_client(s, g("client_id").unwrap_or("")).await?;
    let redirect_uri = match g("redirect_uri").filter(|r| !r.is_empty()) {
        Some(r) => r.to_owned(),
        None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
        None => String::new(),
    };
    if redirect_uri.is_empty() {
        return Err(oe("invalid_redirect_uri", "The app didn’t say where to return to (redirect_uri).").into());
    }
    if !redirect_matches(&client.redirect_uris, &redirect_uri) {
        return Err(oe("invalid_redirect_uri", "The redirect_uri isn’t registered for this app.").into());
    }
    if g("response_type") != Some("code") {
        return Err(oe("invalid_request", "Only response_type=code is supported.").into());
    }
    if g("code_challenge_method") != Some("S256") || !is_challenge(g("code_challenge").unwrap_or("")) {
        return Err(oe("invalid_request", "A PKCE code_challenge with method S256 is required.").into());
    }
    let Some(scopes) = parse_scopes(p.get("scope")) else {
        return Err(oe("invalid_scope", format!("Unsupported scope. Available: {}.", OAUTH_SCOPES.join(", "))).into());
    };
    let resource = parse_resource(s, p.get("resource")).map_err(|_| invalid_target(s))?;
    if g("state").map_or(0, js::len) > 2000 {
        return Err(oe("invalid_request", "state is too long.").into());
    }
    Ok(AuthorizeRequest { client, redirect_uri, scopes, resource, code_challenge: g("code_challenge").unwrap_or("").to_owned(), state: g("state").map(str::to_owned) })
}

/// `redirectWith(uri, params)`: `uri` with query parameters set (`URLSearchParams.set`, re-serialized).
pub fn redirect_with(uri: &str, params: &[(&str, Option<&str>)]) -> String {
    let Some(mut u) = parse_url(uri) else { return uri.to_owned() };
    let mut pairs: Vec<(String, String)> = u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    for (k, v) in params {
        let Some(v) = v else { continue };
        if let Some(i) = pairs.iter().position(|(pk, _)| pk == k) {
            pairs[i].1 = (*v).to_owned();
            let mut j = i + 1;
            while j < pairs.len() {
                if pairs[j].0 == *k { pairs.remove(j); } else { j += 1; }
            }
        } else {
            pairs.push(((*k).to_owned(), (*v).to_owned()));
        }
    }
    let q = form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs.iter()).finish();
    u.set_query(if q.is_empty() { None } else { Some(&q) });
    u.as_str().to_owned()
}

/// A single-use authorization code, valid for 5 minutes.
pub async fn issue_code(s: &AppState, user_id: Uuid, a: &AuthorizeRequest) -> OResult<String> {
    let code = URL_SAFE_NO_PAD.encode(crate::auth::random_bytes(32));
    sqlx::query(
        "INSERT INTO oauth_codes(code_sha256, client_id, user_id, redirect_uri, code_challenge, scopes, resource, expires_at)
     VALUES ($1,$2,$3,$4,$5,$6,$7, now() + interval '5 minutes')",
    )
    .bind(sha256(&code))
    .bind(&a.client.id)
    .bind(user_id)
    .bind(&a.redirect_uri)
    .bind(&a.code_challenge)
    .bind(&a.scopes)
    .bind(&a.resource)
    .execute(&s.pool)
    .await?;
    Ok(code)
}

// ---------------------------------------------------------------- token endpoint

pub struct Credentials {
    pub client_id: String,
    pub secret: Option<String>,
}

/// JS `decodeURIComponent(s.replace(/\+/g, ' '))`, or `s` when it throws.
fn dec(s: &str) -> String {
    let t = s.replace('+', " ");
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() || !b[i + 1].is_ascii_hexdigit() || !b[i + 2].is_ascii_hexdigit() {
                return s.to_owned();
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    match percent_encoding::percent_decode_str(&t).decode_utf8() {
        Ok(d) => d.into_owned(),
        Err(_) => s.to_owned(),
    }
}

/// Client credentials from `Authorization: Basic` (client_secret_basic) or the body.
pub fn client_credentials(authorization: Option<&str>, body: &Map<String, Value>) -> Result<Credentials, OAuthError> {
    let body_id = body.get("client_id").and_then(Value::as_str).unwrap_or("").to_owned();
    let body_secret = body.get("client_secret").and_then(Value::as_str).map(str::to_owned);
    let h = authorization.unwrap_or("");
    let basic = h.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("basic")) && {
        let rest = &h[5..];
        let b64 = rest.trim_start_matches(|c: char| c.is_whitespace());
        b64.len() < rest.len() && !b64.is_empty() && b64.bytes().all(|c| c.is_ascii_alphanumeric() || b"+/=".contains(&c))
    };
    if !basic {
        return Ok(Credentials { client_id: body_id, secret: body_secret });
    }
    if body_secret.as_deref().is_some_and(|x| !x.is_empty()) {
        return Err(oe("invalid_request", "Use one client authentication method."));
    }
    let b64 = h[5..].trim_start_matches(|c: char| c.is_whitespace());
    let raw = String::from_utf8_lossy(&node_base64(b64)).into_owned();
    let (id, secret) = match raw.find(':') {
        None => (dec(&raw), None),
        Some(i) => (dec(&raw[..i]), Some(dec(&raw[i + 1..]))),
    };
    if !body_id.is_empty() && body_id != id {
        return Err(OAuthError { error: "invalid_client", message: "client_id doesn’t match the Authorization header.".into(), status: 401 });
    }
    Ok(Credentials { client_id: id, secret })
}

/// `Buffer.from(s, 'base64')`: lenient (stops at the first `=`, ignores the rest).
fn node_base64(s: &str) -> Vec<u8> {
    let core: String = s.split('=').next().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/').collect();
    let mut padded = core.clone();
    match core.len() % 4 {
        1 => padded.truncate(core.len() - 1),
        2 => padded.push_str("=="),
        3 => padded.push('='),
        _ => {}
    }
    STANDARD.decode(padded.as_bytes()).unwrap_or_default()
}

/// The client making a token or revocation request; confidential clients must present their secret.
pub async fn authenticate_client(s: &AppState, cred: &Credentials) -> OResult<OAuthClient> {
    let c = if cred.client_id.is_empty() { None } else { client_row(s, &cred.client_id).await? };
    let Some(c) = c else {
        return Err(OAuthError { error: "invalid_client", message: "Unknown client_id.".into(), status: 401 }.into());
    };
    if let Some(h) = &c.client_secret_sha256
        && !cred.secret.as_deref().is_some_and(|sec| !sec.is_empty() && safe_equal(&sha256(sec), h))
    {
        return Err(OAuthError { error: "invalid_client", message: "Client authentication failed.".into(), status: 401 }.into());
    }
    Ok(c)
}

async fn revoke_family(s: &AppState, family: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_tokens SET revoked_at = now() WHERE family = $1 AND revoked_at IS NULL").bind(family).execute(&s.pool).await.map(|_| ())
}

async fn user_active(s: &AppState, user_id: Uuid) -> Result<bool, sqlx::Error> {
    Ok(pg::one(&s.pool, "SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL AND disabled_at IS NULL", &[pg::uuid(user_id)]).await?.is_some())
}

/// Access + refresh token on one row. `resource` is the token's audience: always the MCP endpoint (bearer.rs).
async fn issue_tokens(s: &AppState, user_id: Uuid, client: &OAuthClient, scopes: &[String], family: Uuid, resource: &str) -> OResult<Value> {
    let access = new_token("oat");
    let refresh = new_token("ort");
    sqlx::query(
        "INSERT INTO api_tokens(id, user_id, name, token_sha256, prefix, scopes, expires_at, kind, client_id, refresh_sha256, refresh_expires_at, family, resource)
     VALUES ($1,$2,$3,$4,$5,$6, now() + make_interval(secs => $7), 'oauth', $8, $9, now() + make_interval(days => $10), $11, $12)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(&client.name)
    .bind(sha256(&access))
    .bind(&access[..12])
    .bind(scopes)
    .bind(ACCESS_TTL as f64)
    .bind(&client.id)
    .bind(sha256(&refresh))
    .bind(REFRESH_DAYS)
    .bind(family)
    .bind(resource)
    .execute(&s.pool)
    .await?;
    pg::exec(&s.pool, "UPDATE oauth_clients SET last_used_at = now() WHERE id = $1", &[pg::text(&client.id)]).await?;
    Ok(json!({ "access_token": access, "token_type": "Bearer", "expires_in": ACCESS_TTL, "refresh_token": refresh, "scope": scopes.join(" ") }))
}

/// `str(v)`: a non-empty string, else `None`.
fn str_of<'a>(b: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    b.get(k).and_then(Value::as_str).filter(|v| !v.is_empty())
}

/// grant_type=authorization_code. Every check (expiry, client, redirect URI, PKCE verifier, resource, account) runs
/// BEFORE the code is spent, so someone who only stole the code can't burn it; a code is spent only by a successful
/// exchange, atomically, and a replay of a spent code revokes what it issued.
pub async fn exchange_code(s: &AppState, client: &OAuthClient, body: &Map<String, Value>) -> OResult<Value> {
    let (Some(code), Some(verifier)) = (str_of(body, "code"), str_of(body, "code_verifier")) else {
        return Err(oe("invalid_request", "code and code_verifier are required.").into());
    };
    let hash = sha256(code);
    type CodeRow = (String, Uuid, String, String, Vec<String>, Option<String>, bool, bool, Option<Uuid>);
    let row: Option<CodeRow> = sqlx::query_as(
        "SELECT client_id, user_id, redirect_uri, code_challenge, scopes, resource, expires_at > now() AS live, used_at IS NOT NULL AS used, family
     FROM oauth_codes WHERE code_sha256 = $1",
    )
    .bind(&hash)
    .fetch_optional(&s.pool)
    .await?;
    let replayed = async |family: Option<Uuid>| -> OResult<Value> {
        if let Some(f) = family {
            revoke_family(s, f).await?;
        }
        Err(oe("invalid_grant", "The authorization code is invalid or was already used.").into())
    };
    let Some((client_id, user_id, redirect, challenge, scopes, resource, live, used, family)) = row else { return replayed(None).await };
    if used {
        return replayed(family).await;
    }
    if !live {
        return Err(oe("invalid_grant", "The authorization code has expired.").into());
    }
    if client_id != client.id {
        return Err(oe("invalid_grant", "The authorization code was issued to another client.").into());
    }
    if let Some(r) = str_of(body, "redirect_uri")
        && r != redirect
    {
        return Err(oe("invalid_grant", "redirect_uri doesn’t match the authorization request.").into());
    }
    if !is_verifier(verifier) || !safe_equal(&pkce_challenge(verifier), &challenge) {
        return Err(oe("invalid_grant", "PKCE verification failed.").into());
    }
    let asked = parse_resource(s, body.get("resource")).map_err(|_| invalid_target(s))?;
    if let (Some(a), Some(r)) = (&asked, &resource)
        && a != r
    {
        return Err(invalid_target(s).into());
    }
    if !user_active(s, user_id).await? {
        return Err(oe("invalid_grant", "The account is no longer active.").into());
    }
    // Spend it: of two concurrent exchanges, the loser sees a spent code (a replay).
    let family = Uuid::new_v4();
    let spent: Option<Option<Uuid>> =
        sqlx::query_scalar("UPDATE oauth_codes SET used_at = now(), family = $2 WHERE code_sha256 = $1 AND used_at IS NULL RETURNING family").bind(&hash).bind(family).fetch_optional(&s.pool).await?;
    if spent.is_none() {
        let winner: Option<Option<Uuid>> = sqlx::query_scalar("SELECT family FROM oauth_codes WHERE code_sha256 = $1").bind(&hash).fetch_optional(&s.pool).await?;
        return replayed(winner.flatten()).await;
    }
    let res = resource.or(asked).unwrap_or_else(|| mcp_resource(s));
    issue_tokens(s, user_id, client, &scopes, family, &res).await
}

/// grant_type=refresh_token with rotation; a rotated or revoked refresh token signs the whole family out.
pub async fn refresh_tokens(s: &AppState, client: &OAuthClient, body: &Map<String, Value>) -> OResult<Value> {
    let Some(token) = str_of(body, "refresh_token") else { return Err(oe("invalid_request", "refresh_token is required.").into()) };
    type Row = (Uuid, Uuid, Option<String>, Vec<String>, Option<Uuid>, Option<String>, bool, Option<bool>);
    let t: Option<Row> = sqlx::query_as(
        "SELECT id, user_id, client_id, scopes, family, resource, revoked_at IS NOT NULL AS revoked, refresh_expires_at > now() AS live
     FROM api_tokens WHERE refresh_sha256 = $1 AND kind = 'oauth'",
    )
    .bind(sha256(token))
    .fetch_optional(&s.pool)
    .await?;
    let Some((id, user_id, client_id, granted, family, resource, revoked, live)) = t.filter(|t| t.2.as_deref() == Some(client.id.as_str())) else {
        return Err(oe("invalid_grant", "Unknown refresh token.").into());
    };
    let reused = || oe("invalid_grant", "This refresh token was already used; the connection was signed out for safety.");
    let _ = client_id;
    if revoked {
        if let Some(f) = family {
            revoke_family(s, f).await?;
        }
        return Err(reused().into());
    }
    if live != Some(true) {
        return Err(oe("invalid_grant", "The refresh token has expired.").into());
    }
    let mut scopes = granted.clone();
    if body.contains_key("scope") {
        let asked = asked_scopes(body.get("scope"));
        let Some(asked) = asked.filter(|a| a.iter().all(|x| granted.contains(x))) else {
            return Err(oe("invalid_scope", "A refresh can only keep or narrow the granted scopes.").into());
        };
        if !asked.is_empty() {
            scopes = OAUTH_SCOPES.iter().filter(|s| asked.iter().any(|a| a == *s)).map(|s| (*s).to_owned()).collect();
        }
    }
    let asked = parse_resource(s, body.get("resource")).map_err(|_| invalid_target(s))?;
    if let (Some(a), Some(r)) = (&asked, &resource)
        && a != r
    {
        return Err(invalid_target(s).into());
    }
    if !user_active(s, user_id).await? {
        return Err(oe("invalid_grant", "The account is no longer active.").into());
    }
    // Rotate atomically: of two concurrent refreshes with the same token, the loser is treated as a replay.
    let rotated = pg::one(&s.pool, "UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL RETURNING id", &[pg::uuid(id)]).await?;
    let family = family.unwrap_or_else(Uuid::new_v4);
    if rotated.is_none() {
        revoke_family(s, family).await?;
        return Err(reused().into());
    }
    let res = resource.or(asked).unwrap_or_else(|| mcp_resource(s));
    issue_tokens(s, user_id, client, &scopes, family, &res).await
}

/// RFC 7009: revoke an access or refresh token (both share a row). Unknown tokens are ignored.
pub async fn revoke_token(s: &AppState, token: &str, client: Option<&OAuthClient>) -> OResult<()> {
    let h = sha256(token);
    let t: Option<(Uuid, Option<Uuid>, Option<String>)> =
        sqlx::query_as("SELECT id, family, client_id FROM api_tokens WHERE kind = 'oauth' AND (token_sha256 = $1 OR refresh_sha256 = $1)").bind(&h).fetch_optional(&s.pool).await?;
    let Some((id, family, client_id)) = t else { return Ok(()) };
    if let Some(c) = client
        && client_id.as_deref() != Some(c.id.as_str())
    {
        return Ok(());
    }
    match family {
        Some(f) => revoke_family(s, f).await?,
        None => {
            pg::exec(&s.pool, "UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL", &[pg::uuid(id)]).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- connected apps (Settings → Developers)

/// The user's apps with a live grant, one entry per app.
pub async fn list_connections(s: &AppState, user_id: Uuid) -> Result<Vec<Value>, ApiError> {
    let rows = pg::json_rows(
        &s.pool,
        "WITH live AS (
       SELECT client_id, family, scopes FROM api_tokens
       WHERE user_id = $1 AND kind = 'oauth' AND revoked_at IS NULL AND refresh_expires_at > now())
     SELECT c.id, c.kind, c.name, c.client_uri, c.logo_uri,
       (SELECT array_agg(DISTINCT s) FROM live l, unnest(l.scopes) s WHERE l.client_id = c.id) AS scopes,
       (SELECT min(t.created_at) FROM api_tokens t WHERE t.family IN (SELECT family FROM live l WHERE l.client_id = c.id)) AS connected_at,
       (SELECT max(t.last_used_at) FROM api_tokens t WHERE t.user_id = $1 AND t.client_id = c.id) AS last_used_at
     FROM oauth_clients c WHERE c.id IN (SELECT client_id FROM live)
     ORDER BY connected_at DESC",
        &[pg::uuid(user_id)],
    )
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let granted: Vec<&str> = r["scopes"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let scopes: Vec<&str> = OAUTH_SCOPES.iter().copied().filter(|x| granted.contains(x)).collect();
            // A metadata-document client is identified by its URL's domain; a dynamically registered one only by its name.
            let domain = (r["kind"] == "cimd").then(|| r["id"].as_str().and_then(url_host)).flatten();
            json!({
                "clientId": r["id"], "name": r["name"], "uri": r["client_uri"], "logo": r["logo_uri"],
                "scopes": scopes, "connectedAt": r["connected_at"], "lastUsedAt": r["last_used_at"],
                "domain": domain,
            })
        })
        .collect())
}

/// Disconnect an app: revoke every token (and pending code) the user granted it.
pub async fn disconnect(s: &AppState, user_id: Uuid, client_id: &str) -> Result<(), ApiError> {
    let args: [Arg; 2] = [pg::uuid(user_id), pg::text(client_id)];
    pg::exec(&s.pool, "UPDATE api_tokens SET revoked_at = now() WHERE user_id = $1 AND client_id = $2 AND kind = 'oauth' AND revoked_at IS NULL", &args).await?;
    pg::exec(&s.pool, "UPDATE oauth_codes SET used_at = now() WHERE user_id = $1 AND client_id = $2 AND used_at IS NULL", &args).await?;
    Ok(())
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn only_public_addresses_are_public() {
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700::1111", "2a00:1450:4001:80b::200e"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1", "127.1.2.3", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "0.0.0.0", "0.1.2.3", "100.64.0.1", "169.254.169.254", "192.0.0.170",
            "198.18.0.1", "224.0.0.1", "255.255.255.255", "::", "::1", "::ffff:127.0.0.1", "::ffff:8.8.8.8", "::127.0.0.1", "fd00:ec2::254", "fc00::1", "fe80::1",
            "ff02::1", "64:ff9b::7f00:1", "2002:7f00:1::", "2001:0:4136:e378::1", "2001:db8::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn names_resolving_to_private_addresses_are_refused() {
        assert!(resolve_public("localhost", 443).await.unwrap_err().contains("non-public"));
    }

    #[test]
    fn client_identity_is_verified_only_for_metadata_documents() {
        let c = |id: &str, kind: &str| OAuthClient {
            id: id.into(), kind: kind.into(), name: "Claude".into(), redirect_uris: vec![], client_uri: None, logo_uri: None, client_secret_sha256: None, fetched_at: None,
        };
        assert_eq!(client_identity(&c("https://claude.ai/oauth/claude-code-client-metadata", "cimd")), (true, Some("claude.ai".into())));
        assert_eq!(client_identity(&c("https://x.example:8443/c.json", "cimd")), (true, Some("x.example:8443".into())));
        assert_eq!(client_identity(&c("6f1c…", "dcr")), (false, None));
    }
}
