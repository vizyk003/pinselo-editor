//! Errors and the Fastify error handler of services/api/src/main.ts:
//!
//! ```ts
//! if (err instanceof ZodError) return reply.code(400).send({ error: 'invalid_request', issues: err.issues });
//! if (err instanceof UpgradeRequired) return reply.code(402).send({ error: 'upgrade_required', feature, message });
//! const status = err.statusCode ?? 500;
//! return reply.code(status).send({ error: status >= 500 ? 'internal' : err.message });
//! ```
//!
//! Handlers return `Result<Reply, ApiError>`; `?` on a database/storage error yields the 500 `internal` reply.
//! [`ApiError::Reply`] is an early `return reply.code(...).send(...)` from a guard (`requireUser` etc.).

use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::http::reply::{Reply, json};

pub type ApiResult = Result<Reply, ApiError>;

#[derive(Debug)]
pub enum ApiError {
    /// A finished reply (guards: 401/403/404 …).
    Reply(ReplyBox),
    /// `ZodError` → 400 `{error: 'invalid_request', issues}`.
    Invalid(Vec<serde_json::Value>),
    /// `UpgradeRequired` → 402.
    Upgrade { feature: String, message: String },
    /// An `Error` with `statusCode` (Fastify's own errors: 413, 415, 400 JSON syntax, 429 rate limit, …).
    Status { status: u16, message: String, close: bool },
    /// Anything unexpected → 500 `{error: 'internal'}` (logged).
    Internal(String),
}

/// `Reply` is not `Debug`; keep it boxed behind a tiny wrapper.
pub struct ReplyBox(pub Box<Reply>);

impl std::fmt::Debug for ReplyBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Reply({})", self.0.status)
    }
}

impl ApiError {
    pub fn reply(r: Reply) -> ApiError {
        ApiError::Reply(ReplyBox(Box::new(r)))
    }
    pub fn status(status: u16, message: impl Into<String>) -> ApiError {
        ApiError::Status { status, message: message.into(), close: false }
    }
    pub fn internal(e: impl std::fmt::Display) -> ApiError {
        ApiError::Internal(e.to_string())
    }
}

impl From<Reply> for ApiError {
    fn from(r: Reply) -> Self {
        ApiError::reply(r)
    }
}

macro_rules! internal_from {
    ($($t:ty),*) => {$(
        impl From<$t> for ApiError {
            fn from(e: $t) -> Self {
                ApiError::Internal(e.to_string())
            }
        }
    )*};
}
internal_from!(sqlx::Error, oc_storage::StorageError, std::io::Error, serde_json::Error, oc_crdt::CrdtError, reqwest::Error, oc_mail::MailError);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Reply(r) => r.0.into_response(),
            ApiError::Invalid(issues) => json(400, json!({ "error": "invalid_request", "issues": issues })).into_response(),
            ApiError::Upgrade { feature, message } => json(402, json!({ "error": "upgrade_required", "feature": feature, "message": message })).into_response(),
            ApiError::Status { status, message, close } => {
                let body = if status >= 500 { json!({ "error": "internal" }) } else { json!({ "error": message }) };
                if status >= 500 {
                    tracing::error!(status, "{message}");
                }
                let r = json(status, body);
                // Fastify closes the connection after a body it could not parse (the client may still be sending).
                if close { r.header("connection", "close").into_response() } else { r.into_response() }
            }
            ApiError::Internal(msg) => {
                tracing::error!("internal error: {msg}");
                json(500, json!({ "error": "internal" })).into_response()
            }
        }
    }
}
