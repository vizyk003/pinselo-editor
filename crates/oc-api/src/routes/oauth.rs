//! Port of services/api/src/routes/oauth.ts — OAuth 2.1 endpoints for the AI connector (logic in
//! [`crate::oauth`]): discovery documents, dynamic client registration, the token and revocation endpoints (called by
//! the apps themselves: form-encoded or JSON, CORS-open, rate-limited), the consent page's API and the user's
//! connected apps (signed in, JSON only).

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::{Next, from_fn};
use axum::response::Response;
use serde_json::Map;

use crate::http::ratelimit::MINUTE;
use crate::oauth::{self as o, Fail};
use crate::prelude::*;

pub const LIMITS: &[RouteLimit] = &[
    RouteLimit { method: "POST", path: "/api/oauth/register", max: 20, window_ms: MINUTE },
    RouteLimit { method: "POST", path: "/api/oauth/token", max: 60, window_ms: MINUTE },
    RouteLimit { method: "POST", path: "/api/oauth/revoke", max: 60, window_ms: MINUTE },
];

const CORS: [(&str, &str); 4] = [
    ("access-control-allow-origin", "*"),
    ("access-control-allow-methods", "GET, POST, OPTIONS"),
    ("access-control-allow-headers", "authorization, content-type, mcp-protocol-version"),
    ("access-control-max-age", "86400"),
];

pub fn routes() -> Router<AppState> {
    let public = Router::new()
        .route("/api/oauth/register", post(register).options(preflight))
        .route("/api/oauth/token", post(token).options(preflight))
        .route("/api/oauth/revoke", post(revoke).options(preflight))
        .route_layer(from_fn(allow_origin))
        .route_layer(from_fn(crate::routes::sites::form_body));
    Router::new()
        .route("/.well-known/oauth-protected-resource", get(resource_metadata).options(preflight))
        .route("/.well-known/oauth-protected-resource/mcp", get(resource_metadata).options(preflight))
        .route("/.well-known/oauth-authorization-server", get(server_metadata).options(preflight))
        .merge(public)
        .route("/api/oauth/authorize/info", get(authorize_info))
        .route("/api/oauth/authorize", post(authorize))
        .route("/api/oauth/connections", get(connections))
        .route("/api/oauth/connections/{client_id}", delete(disconnect))
}

/// The public plugin's `onSend` hook: every reply (errors included) is readable cross-origin.
async fn allow_origin(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    res.headers_mut().insert("access-control-allow-origin", HeaderValue::from_static("*"));
    res
}

fn cors(mut r: Reply) -> Reply {
    for (k, v) in CORS {
        r = r.header(k, v);
    }
    r
}

async fn preflight(req: Req) -> ApiResult {
    let _ = req;
    Ok(cors(Reply::empty(204)))
}

async fn resource_metadata(State(s): State<AppState>, req: Req) -> ApiResult {
    let _ = req;
    Ok(cors(Reply::json(o::protected_resource_metadata(&s))).header("cache-control", "public, max-age=300"))
}

async fn server_metadata(State(s): State<AppState>, req: Req) -> ApiResult {
    let _ = req;
    Ok(cors(Reply::json(o::authorization_server_metadata(&s))).header("cache-control", "public, max-age=300"))
}

/// `body(req.body)`: the body when it is a plain object, else `{}`.
fn body_of(req: &Req) -> Map<String, Value> {
    match req.body() {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    }
}

/// RFC 6749 §5.2 error body (401 for client authentication failures).
fn token_error(s: &AppState, e: Fail, basic: bool) -> ApiResult {
    match e {
        Fail::Api(e) => Err(e),
        Fail::OAuth(e) => {
            let mut r = reply::json(e.status, json!({ "error": e.error, "error_description": e.message }));
            if e.status == 401 && basic {
                r = r.header("www-authenticate", format!("Basic realm=\"{}\"", s.cfg.brand.name));
            }
            Ok(r.header("cache-control", "no-store").header("pragma", "no-cache"))
        }
    }
}

fn is_basic(req: &Req) -> bool {
    req.header("authorization").and_then(|h| h.get(..6)).is_some_and(|p| p[..5].eq_ignore_ascii_case("basic") && p[5..].chars().all(char::is_whitespace))
}

async fn register(State(s): State<AppState>, req: Req) -> ApiResult {
    match o::register_client(&s, &body_of(&req)).await {
        Ok(v) => Ok(Reply::json(v).status(201).header("cache-control", "no-store")),
        Err(Fail::OAuth(e)) => Ok(reply::json(400, json!({ "error": e.error, "error_description": e.message }))),
        Err(Fail::Api(e)) => Err(e),
    }
}

async fn token(State(s): State<AppState>, req: Req) -> ApiResult {
    let b = body_of(&req);
    let basic = is_basic(&req);
    let run = async {
        let cred = o::client_credentials(req.header("authorization"), &b)?;
        let client = o::authenticate_client(&s, &cred).await?;
        match b.get("grant_type").and_then(Value::as_str) {
            Some("authorization_code") => o::exchange_code(&s, &client, &b).await,
            Some("refresh_token") => o::refresh_tokens(&s, &client, &b).await,
            _ => Err(o::OAuthError { error: "unsupported_grant_type", message: "grant_type must be authorization_code or refresh_token.".into(), status: 400 }.into()),
        }
    };
    match run.await {
        Ok(v) => Ok(Reply::json(v).header("cache-control", "no-store").header("pragma", "no-cache")),
        Err(e) => token_error(&s, e, basic),
    }
}

async fn revoke(State(s): State<AppState>, req: Req) -> ApiResult {
    let b = body_of(&req);
    let basic = is_basic(&req);
    let run = async {
        let cred = o::client_credentials(req.header("authorization"), &b)?;
        // Public clients may identify themselves or not; a confidential client has to authenticate.
        let client = if cred.client_id.is_empty() { None } else { Some(o::authenticate_client(&s, &cred).await?) };
        if let Some(t) = b.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            o::revoke_token(&s, t, client.as_ref()).await?;
        }
        Ok::<(), Fail>(())
    };
    match run.await {
        Ok(()) => Ok(Reply::json(json!({})).header("cache-control", "no-store")),
        Err(e) => token_error(&s, e, basic),
    }
}

fn authorize_params(approve: bool) -> z::Schema {
    let f = || {
        [
            ("response_type", z::str().max(40.0).optional()),
            ("client_id", z::str().max(2000.0).optional()),
            ("redirect_uri", z::str().max(2000.0).optional()),
            ("scope", z::str().max(1000.0).optional()),
            ("state", z::str().max(2000.0).optional()),
            ("code_challenge", z::str().max(200.0).optional()),
            ("code_challenge_method", z::str().max(20.0).optional()),
            ("resource", z::str().max(2000.0).optional()),
        ]
    };
    if !approve {
        return z::obj(f());
    }
    let [a, b, c, d, e, g, h, i] = f();
    z::obj([a, b, c, d, e, g, h, i, ("approve", z::bool())])
}

fn consent_error(e: Fail) -> ApiResult {
    match e {
        Fail::OAuth(e) => Ok(reply::json(400, json!({ "error": e.error, "message": e.message }))),
        Fail::Api(e) => Err(e),
    }
}

async fn authorize_info(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let p = authorize_params(false).parse(Some(&req.query))?;
    match o::check_authorization(&s, &p).await {
        Ok(a) => {
            let host = url::Url::parse(&a.redirect_uri).ok().map(|x| match x.port() {
                Some(port) => format!("{}:{port}", x.host_str().unwrap_or("")),
                None => x.host_str().unwrap_or("").to_owned(),
            });
            let (verified, domain) = o::client_identity(&a.client);
            Ok(Reply::json(json!({
                "client": { "id": a.client.id, "name": a.client.name, "uri": a.client.client_uri, "logo": a.client.logo_uri, "redirectHost": host, "verified": verified, "domain": domain },
                "scopes": a.scopes,
                "user": { "email": u.email, "name": u.name },
            })))
        }
        Err(e) => consent_error(e),
    }
}

/// `sameOrigin(origin, fetchSite)`: an Origin header must be exactly the web origin (no normalization); without
/// one, a browser's `Sec-Fetch-Site: cross-site` still refuses.
pub fn same_origin(web_origin: &str, origin: Option<&str>, fetch_site: Option<&str>) -> bool {
    match origin {
        Some(o) => o == web_origin,
        None => fetch_site != Some("cross-site"),
    }
}

/// Session-cookie POST: JSON only (like every other signed-in route), and never from another origin.
async fn authorize(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !same_origin(&s.cfg.web_origin, req.header("origin"), req.header("sec-fetch-site")) {
        return Ok(reply::json(403, json!({ "error": "forbidden", "message": "Cross-origin request refused." })));
    }
    let b = authorize_params(true).parse(req.body())?;
    let run = async {
        let a = o::check_authorization(&s, &b).await?;
        let code = if b["approve"] == true { Some(o::issue_code(&s, u.id, &a).await?) } else { None };
        let iss = s.cfg.web_origin.as_str();
        let redirect = match &code {
            Some(c) => o::redirect_with(&a.redirect_uri, &[("code", Some(c)), ("state", a.state.as_deref()), ("iss", Some(iss))]),
            None => o::redirect_with(&a.redirect_uri, &[("error", Some("access_denied")), ("state", a.state.as_deref()), ("iss", Some(iss))]),
        };
        Ok::<Value, Fail>(json!({ "redirect": redirect }))
    };
    match run.await {
        Ok(v) => Ok(Reply::json(v)),
        Err(e) => consent_error(e),
    }
}

async fn connections(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    Ok(Reply::json(json!({ "connections": o::list_connections(&s, u.id).await? })))
}

async fn disconnect(State(s): State<AppState>, Path(client_id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    o::disconnect(&s, u.id, &client_id).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::same_origin;

    #[test]
    fn consent_post_needs_the_exact_origin() {
        let web = "https://app.example";
        assert!(same_origin(web, Some(web), Some("same-origin")));
        assert!(same_origin(web, None, None));
        assert!(same_origin(web, None, Some("same-origin")));
        for o in ["https://app.example/", "null", "https://app.example:1", "http://app.example", "https://evil.example"] {
            assert!(!same_origin(web, Some(o), None), "{o}");
        }
        assert!(!same_origin(web, None, Some("cross-site")));
    }
}
