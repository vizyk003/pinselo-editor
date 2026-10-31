//! Port of services/api/src/mcp/index.ts — the AI connector: an MCP server at `/mcp` that lets AI assistants such
//! as Claude, ChatGPT and Codex create and edit the user's designs. Requests carry a bearer token: an OAuth access
//! token from connecting the app (see [`crate::routes::oauth`]), or a personal access token. Without one the
//! response points the client at the OAuth metadata (RFC 9728), which starts the sign-in.
//!
//! The TS endpoint runs `@modelcontextprotocol/sdk` 1.32's `McpServer` over `StreamableHTTPServerTransport` in
//! stateless JSON-response mode (a fresh server per request, `reply.hijack()`). This module implements the same wire
//! protocol directly — the transport's request checks (Accept, Content-Type, JSON-RPC shapes, batches,
//! notifications, the protocol-version header) and the server's `initialize`, `ping`, `tools/list` and
//! `tools/call` — so responses match the SDK's byte for byte (key order included; `tools.json` is the SDK's
//! `tools/list` result, generated from the TS tool definitions).

mod brand;
mod edit;
mod files;
mod guide;
mod layout;
mod live;
mod schema;
mod tools;
mod view;

use std::sync::LazyLock;

use axum::body::Body;
use axum::extract::{FromRequest, MatchedPath, Request};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::auth::{at_least, design_role};
use crate::bearer::{Audience, bearer_shaped, resolve_bearer, sha256};
use crate::http::Hijacked;
use crate::http::ratelimit::{self, Limiter, MINUTE};
use crate::http::req::Sink;
use crate::prelude::*;
use serde_json::Map;

pub const MCP_PATH: &str = "/mcp";

/// `/mcp` counts with its own limiter keyed by the bearer token (see [`post_mcp`]).
pub const LIMITS: &[RouteLimit] = &[];

const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 5] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05", "2024-10-07"];
const MAX_BATCH_SIZE: usize = 100;

static TOOLS: LazyLock<Value> = LazyLock::new(|| serde_json::from_str(include_str!("tools.json")).expect("tools.json"));

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(MCP_PATH, post(post_mcp).get(not_allowed).delete(not_allowed))
        .route("/api/mcp/files/{sha}", get(files::download))
        .route("/api/designs/{id}/ai-checkpoint", get(ai_checkpoint))
}

/// No server-initiated streams or sessions in stateless mode.
async fn not_allowed(req: Req) -> ApiResult {
    let _ = req;
    Ok(reply::json(405, json!({ "jsonrpc": "2.0", "error": { "code": -32000, "message": "Method not allowed." }, "id": null })).header("allow", "POST"))
}

/// Editor: the latest checkpoint an assistant saved before changing this design ("Undo AI changes").
async fn ai_checkpoint(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, oc_auth::Role::View) {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let cp = live::latest_checkpoint(&s, &id).await?;
    Ok(Reply::json(json!({ "checkpoint": cp.map(|c| json!({ "id": c.id.to_string(), "agent": c.agent, "createdAt": c.created_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string() })) })))
}

#[derive(Default)]
struct McpLimiter(std::sync::OnceLock<std::sync::Arc<Limiter>>);

/// `mcpRateKey(ip, authorization)`: the client IP plus the token's hash. Hosted assistants call from shared
/// addresses, so the token separates connections and the IP separates the callers of one token.
pub fn mcp_rate_key(ip: &str, authorization: Option<&str>) -> String {
    match authorization.filter(|a| !a.is_empty()) {
        Some(a) => format!("{ip} {}", sha256(a)),
        None => ip.to_owned(),
    }
}

const GATE_CAP: usize = 10_000;

/// Failures per IP (`(count, window start)`) and recently valid token hashes (`valid until`).
type GateState = (std::collections::HashMap<String, (u32, i64)>, std::collections::HashMap<String, i64>);

/// `FailureGate`: failed token lookups per client IP. After `max` in a window, unknown tokens from that address are
/// refused with 429 before touching the database; tokens that resolved recently skip the gate.
pub struct FailureGate {
    max: u32,
    window_ms: i64,
    valid_ms: i64,
    state: std::sync::Mutex<GateState>,
}

impl Default for FailureGate {
    fn default() -> Self {
        FailureGate::new(20, MINUTE, 10 * MINUTE)
    }
}

impl FailureGate {
    pub fn new(max: u32, window_ms: i64, valid_ms: i64) -> FailureGate {
        FailureGate { max, window_ms, valid_ms, state: std::sync::Mutex::new(Default::default()) }
    }

    /// Seconds until `ip` may present an unknown token again (0: it may now).
    pub fn retry_after(&self, ip: &str, token_hash: &str, now: i64) -> i64 {
        let g = self.state.lock().expect("gate lock");
        if g.1.get(token_hash).is_some_and(|&until| until > now) {
            return 0;
        }
        match g.0.get(ip) {
            Some(&(n, start)) if start + self.window_ms > now && n >= self.max => (((start + self.window_ms - now) as f64) / 1000.0).ceil() as i64,
            _ => 0,
        }
    }

    pub fn fail(&self, ip: &str, now: i64) {
        let mut g = self.state.lock().expect("gate lock");
        let window = self.window_ms;
        let e = g.0.entry(ip.to_owned()).or_insert((0, now));
        if e.1 + window > now && e.0 > 0 {
            e.0 += 1;
        } else {
            *e = (1, now);
        }
        if g.0.len() > GATE_CAP {
            let oldest = g.0.iter().min_by_key(|(_, v)| v.1).map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                g.0.remove(&k);
            }
        }
    }

    pub fn succeed(&self, token_hash: &str, now: i64) {
        let mut g = self.state.lock().expect("gate lock");
        g.1.insert(token_hash.to_owned(), now + self.valid_ms);
        if g.1.len() > GATE_CAP {
            let oldest = g.1.iter().min_by_key(|(_, v)| **v).map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                g.1.remove(&k);
            }
        }
    }
}

fn unauthorized(s: &AppState, error: Option<&str>) -> Reply {
    let metadata = format!("{}/.well-known/oauth-protected-resource{MCP_PATH}", s.cfg.web_origin);
    let challenge = match error {
        Some(e) => format!("Bearer resource_metadata=\"{metadata}\", error=\"{e}\""),
        None => format!("Bearer resource_metadata=\"{metadata}\""),
    };
    reply::json(
        401,
        json!({ "error": error.unwrap_or("unauthenticated"), "message": format!("Connect {} in your AI app (it opens a sign-in page), or send Authorization: Bearer <token>.", s.cfg.brand.name) }),
    )
    .header("www-authenticate", challenge)
}

async fn post_mcp(State(s): State<AppState>, raw: Request) -> Response {
    match post_inner(&s, raw).await {
        Ok(r) => r,
        Err(e) => e.into_response(),
    }
}

async fn post_inner(s: &AppState, raw: Request) -> Result<Response, ApiError> {
    let (mut parts, body) = raw.into_parts();
    let auth = parts.headers.get("authorization").map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    let peer = parts.extensions.get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().map(|c| c.0);
    let ip = crate::http::req::client_ip(&parts.headers, peer);
    let key = mcp_rate_key(&ip, auth.as_deref());
    let limiter = s.local::<McpLimiter>().0.get_or_init(|| std::sync::Arc::new(Limiter::new(240, MINUTE))).clone();
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
    if auth.as_deref().is_none_or(str::is_empty) {
        return Ok(unauthorized(s, None).into_response());
    }
    let token_hash = sha256(auth.as_deref().unwrap_or(""));
    let gate = s.local::<FailureGate>();
    let wait = gate.retry_after(&ip, &token_hash, s.now_ms());
    if wait > 0 {
        return Ok(reply::json(429, json!({ "error": "rate_limited", "message": format!("Too many invalid tokens from this address. Try again in {wait} seconds.") }))
            .header("retry-after", wait.to_string())
            .into_response());
    }
    let Some(user) = resolve_bearer(s, auth.as_deref(), Audience::Mcp).await? else {
        // Only lookups count: a malformed header never reaches the database.
        if bearer_shaped(auth.as_deref()) {
            gate.fail(&ip, s.now_ms());
        }
        return Ok(unauthorized(s, Some("invalid_token")).into_response());
    };
    gate.succeed(&token_hash, s.now_ms());
    Ok(transport(s, &user, &req).await)
}

// ---------------------------------------------------------------- transport (stateless, JSON responses)

fn raw_json(status: u16, body: Option<&Value>) -> Response {
    let mut r = match body {
        Some(v) => {
            let mut r = Response::new(Body::from(crate::http::jsjson::stringify(v)));
            r.headers_mut().insert("content-type", HeaderValue::from_static("application/json"));
            r
        }
        None => Response::new(Body::empty()),
    };
    *r.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    r.extensions_mut().insert(Hijacked);
    r
}

/// `createJsonErrorResponse(status, code, message)`.
fn json_error(status: u16, code: i64, message: &str) -> Response {
    raw_json(status, Some(&json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message }, "id": null })))
}

fn is_int(v: &Value) -> bool {
    v.as_f64().is_some_and(|f| f.fract() == 0.0 && f.is_finite())
}
fn is_id(v: &Value) -> bool {
    v.is_string() || is_int(v)
}
fn keys_in(m: &Map<String, Value>, allowed: &[&str]) -> bool {
    m.keys().all(|k| allowed.contains(&k.as_str()))
}
fn opt_object(m: &Map<String, Value>, k: &str) -> bool {
    match m.get(k) {
        None => true,
        Some(Value::Object(o)) => o.get("_meta").is_none_or(Value::is_object),
        Some(_) => false,
    }
}

fn is_request(v: &Value) -> bool {
    let Some(m) = v.as_object() else { return false };
    keys_in(m, &["jsonrpc", "id", "method", "params"]) && m.get("jsonrpc") == Some(&json!("2.0")) && m.get("id").is_some_and(is_id) && m.get("method").is_some_and(Value::is_string) && opt_object(m, "params")
}
fn is_notification(v: &Value) -> bool {
    let Some(m) = v.as_object() else { return false };
    keys_in(m, &["jsonrpc", "method", "params"]) && m.get("jsonrpc") == Some(&json!("2.0")) && m.get("method").is_some_and(Value::is_string) && opt_object(m, "params")
}
fn is_result(v: &Value) -> bool {
    let Some(m) = v.as_object() else { return false };
    keys_in(m, &["jsonrpc", "id", "result"]) && m.get("jsonrpc") == Some(&json!("2.0")) && m.get("id").is_some_and(is_id) && m.get("result").is_some_and(Value::is_object)
}
fn is_error(v: &Value) -> bool {
    let Some(m) = v.as_object() else { return false };
    let err_ok = m.get("error").and_then(Value::as_object).is_some_and(|e| e.get("code").is_some_and(is_int) && e.get("message").is_some_and(Value::is_string));
    keys_in(m, &["jsonrpc", "id", "error"]) && m.get("jsonrpc") == Some(&json!("2.0")) && m.get("id").is_none_or(is_id) && err_ok
}
fn is_message(v: &Value) -> bool {
    is_request(v) || is_notification(v) || is_result(v) || is_error(v)
}

/// `isInitializeRequest(m)`.
fn is_initialize(v: &Value) -> bool {
    v.get("method") == Some(&json!("initialize")) && init_params().safe_parse(v.get("params")).is_ok()
}

fn init_params() -> z::Schema {
    z::loose_obj([
        ("protocolVersion", z::str()),
        ("capabilities", z::loose_obj([])),
        ("clientInfo", z::loose_obj([("name", z::str()), ("version", z::str())])),
    ])
}

/// `handleRequest` for a POST: the transport's checks, then every request handled in order.
async fn transport(s: &AppState, user: &crate::bearer::BearerUser, req: &Req) -> Response {
    let accept = req.header("accept").unwrap_or("");
    if !accept.contains("application/json") || !accept.contains("text/event-stream") {
        return json_error(406, -32000, "Not Acceptable: Client must accept both application/json and text/event-stream");
    }
    let ct = req.header("content-type").unwrap_or("");
    if ct != "application/json" && crate::http::req::media_type(ct).as_deref() != Some("application/json") {
        return json_error(415, -32000, "Unsupported Media Type: Content-Type must be application/json");
    }
    let Some(raw) = req.body() else {
        return json_error(400, -32700, "Parse error: Invalid JSON");
    };
    if raw.as_array().is_some_and(|a| a.len() > MAX_BATCH_SIZE) {
        return json_error(400, -32600, &format!("Invalid Request: Batch must not exceed {MAX_BATCH_SIZE} messages"));
    }
    let messages: Vec<Value> = match raw {
        Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    if !messages.iter().all(is_message) {
        return json_error(400, -32700, "Parse error: Invalid JSON-RPC message");
    }
    let init = messages.iter().any(is_initialize);
    if init && messages.len() > 1 {
        return json_error(400, -32600, "Invalid Request: Only one initialization request is allowed");
    }
    if !init
        && let Some(v) = req.header("mcp-protocol-version")
        && !SUPPORTED_PROTOCOL_VERSIONS.contains(&v)
    {
        return json_error(400, -32000, &format!("Bad Request: Unsupported protocol version: {v} (supported versions: {})", SUPPORTED_PROTOCOL_VERSIONS.join(", ")));
    }
    if !messages.iter().any(is_request) {
        return raw_json(202, None);
    }
    // Responses go out in the order of their request ids (a repeated id answers once, at its first position).
    let mut order: Vec<Value> = Vec::new();
    let mut answers: Vec<(Value, Value)> = Vec::new();
    for m in messages.iter().filter(|m| is_request(m)) {
        let id = m["id"].clone();
        if !order.contains(&id) {
            order.push(id.clone());
        }
        let r = handle(s, user, m).await;
        answers.retain(|(i, _)| *i != id);
        answers.push((id, r));
    }
    let responses: Vec<Value> = order.iter().filter_map(|id| answers.iter().find(|(i, _)| i == id).map(|(_, r)| r.clone())).collect();
    if responses.len() == 1 { raw_json(200, Some(&responses[0])) } else { raw_json(200, Some(&Value::Array(responses))) }
}

fn result(id: &Value, r: Value) -> Value {
    json!({ "result": r, "jsonrpc": "2.0", "id": id })
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A request whose params fail the method's schema: the SDK throws the ZodError, answered as an internal error
/// whose message is zod 4's `JSON.stringify(issues, null, 2)`.
fn params_error(id: &Value, issues: Vec<Value>) -> Value {
    let issues: Vec<Value> = issues
        .into_iter()
        .map(|mut i| {
            let mut p = vec![json!("params")];
            p.extend(i["path"].as_array().cloned().unwrap_or_default());
            i["path"] = Value::Array(p);
            i
        })
        .collect();
    error(id, -32603, &tools::pretty_indent(&Value::Array(issues), 2))
}

async fn handle(s: &AppState, user: &crate::bearer::BearerUser, m: &Value) -> Value {
    let id = &m["id"];
    match m["method"].as_str().unwrap_or("") {
        "initialize" => {
            if let Err(issues) = init_params().safe_parse(m.get("params")) {
                return params_error(id, issues);
            }
            let asked = m["params"]["protocolVersion"].as_str().unwrap_or("");
            let version = if SUPPORTED_PROTOCOL_VERSIONS.contains(&asked) { asked } else { LATEST_PROTOCOL_VERSION };
            result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": true } },
                    "serverInfo": { "name": "pinselo", "title": s.cfg.brand.name, "version": "1.0.0" },
                    "instructions": guide::instructions(s.cfg.brand.name),
                }),
            )
        }
        "ping" => result(id, json!({})),
        "tools/list" => result(id, TOOLS.clone()),
        "tools/call" => {
            let schema = z::loose_obj([("name", z::str()), ("arguments", z::record(z::any()).optional())]);
            let params = match schema.safe_parse(m.get("params")) {
                Ok(p) => p,
                Err(issues) => return params_error(id, issues),
            };
            let name = params["name"].as_str().unwrap_or("").to_owned();
            if !schema::is_tool(&name) {
                return result(id, tools::failure(&name, tools::Fail::Tool(format!("MCP error -32602: Tool {name} not found"))));
            }
            let args = match schema::validate(&name, params.get("arguments")) {
                Ok(a) => a,
                Err(msg) => return result(id, tools::failure(&name, tools::Fail::Tool(msg))),
            };
            match tools::call(s, user, &name, &args).await {
                Ok(r) => result(id, r),
                Err(f) => result(id, tools::failure(&name, f)),
            }
        }
        _ => error(id, -32601, "Method not found"),
    }
}

#[cfg(test)]
mod rate_tests {
    use super::*;

    #[test]
    fn rate_key_is_ip_plus_token_hash() {
        assert_eq!(mcp_rate_key("203.0.113.7", Some("Bearer oc_oat_x")), format!("203.0.113.7 {}", sha256("Bearer oc_oat_x")));
        assert_ne!(mcp_rate_key("203.0.113.8", Some("Bearer oc_oat_x")), mcp_rate_key("203.0.113.7", Some("Bearer oc_oat_x")));
        assert_eq!(mcp_rate_key("203.0.113.7", None), "203.0.113.7");
    }

    #[test]
    fn failure_gate_blocks_spraying_but_not_known_tokens() {
        let g = FailureGate::new(3, 60_000, 600_000);
        let now = 1_000_000;
        g.succeed("good", now);
        for i in 0..3 {
            assert_eq!(g.retry_after("ip1", &format!("bogus{i}"), now), 0);
            g.fail("ip1", now);
        }
        assert_eq!(g.retry_after("ip1", "bogus-new", now + 1000), 59);
        assert_eq!(g.retry_after("ip1", "good", now + 1000), 0);
        assert_eq!(g.retry_after("ip2", "bogus-new", now), 0);
        assert_eq!(g.retry_after("ip1", "bogus-new", now + 60_000), 0);
    }
}
