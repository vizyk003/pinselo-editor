//! The Fastify behaviour services/api relies on, rebuilt on axum:
//!
//! | Fastify (main.ts)                                   | here                                   |
//! |-----------------------------------------------------|----------------------------------------|
//! | `Fastify({ trustProxy: true, bodyLimit: 10 MiB })`  | [`req::Req`] (`request.ip`, body limit) |
//! | `addContentTypeParser('application/json', …)`       | [`req::Req`] + [`jsjson`] (V8 messages) |
//! | `@fastify/cookie` (signed with `OC_SECRET`)         | [`cookie`]                              |
//! | `@fastify/multipart` (`limits: { files: 1 }`)       | [`multipart`]                           |
//! | `@fastify/rate-limit` (600/min + per-route)         | [`ratelimit`]                           |
//! | `setErrorHandler` (zod 400, `{error}`)         | [`error::ApiError`], [`crate::zod`]     |
//! | `onSend` security headers                           | [`global`]                              |
//! | default 404 handler (also for a wrong method)       | [`not_found`]                           |

pub mod cookie;
pub mod error;
pub mod jsjson;
pub mod multipart;
pub mod ratelimit;
pub mod reply;
pub mod req;

use axum::extract::{Request, State};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::state::AppState;

/// Response extension of a hijacked reply (`reply.hijack()`): sent as is, without the `onSend` headers.
#[derive(Clone, Copy, Debug)]
pub struct Hijacked;

/// Outermost middleware: request-scoped header sink, the custom-domain hook, and the `onSend` hook of
/// main.ts (baseline security headers on every response, `cache-control: no-store` for `/api/`).
pub async fn global(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    let url = req.uri().path_and_query().map_or_else(|| req.uri().path().to_owned(), |p| p.as_str().to_owned());
    let sink = req::Sink::default();
    req.extensions_mut().insert(sink.clone());
    // Route-level layers (a plugin's own hooks) read the state from here.
    req.extensions_mut().insert(state.clone());
    let hook = crate::routes::sites::custom_domain_hook(&state, req.method().clone(), req.uri().clone(), req.headers().clone()).await;
    let mut res = match hook {
        Some(r) => r,
        None => next.run(req).await,
    };
    // `reply.hijack()` (the MCP endpoint writes the raw response): Fastify's hooks do not run.
    if res.extensions().get::<Hijacked>().is_some() {
        return res;
    }
    let h = res.headers_mut();
    sink.apply(h);
    // axum's method-not-allowed fallback adds `allow`; Fastify answers that case with its plain 404.
    if res.status() == axum::http::StatusCode::NOT_FOUND {
        res.headers_mut().remove("allow");
    }
    let h = res.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy", HeaderValue::from_static("strict-origin-when-cross-origin"));
    h.insert("cross-origin-opener-policy", HeaderValue::from_static("same-origin"));
    h.insert("permissions-policy", HeaderValue::from_static("camera=(self), microphone=(self), geolocation=(), payment=()"));
    if !url.starts_with("/api/public/") && !url.starts_with("/site/") && !h.contains_key("x-frame-options") {
        h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    }
    if state.prod() {
        h.insert("strict-transport-security", HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"));
    }
    if !h.contains_key("cache-control") && url.starts_with("/api/") {
        h.insert("cache-control", HeaderValue::from_static("no-store"));
    }
    res
}

/// Fastify's default 404 (Fastify answers 404, not 405, for a known path with another method).
pub async fn not_found(req: Request) -> Response {
    let url = req.uri().path_and_query().map_or_else(|| req.uri().path().to_owned(), |p| p.as_str().to_owned());
    reply::json(404, json!({ "message": format!("Route {}:{} not found", req.method(), url), "error": "Not Found", "statusCode": 404 })).into_response()
}
