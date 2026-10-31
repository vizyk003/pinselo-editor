//! Port of services/api/src/routes/dev.ts — development-only mailbox: read outbox mail (magic links, invites)
//! without SMTP.
//!
//! The TS plugin is only registered when `config.env !== 'production'` and `OC_SMTP_URL` is unset at boot; axum
//! routes are static, so the handler checks the same condition and answers Fastify's plain 404 otherwise.

use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};

use crate::prelude::*;

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/dev/mail", get(mail))
}

fn enabled(s: &AppState) -> bool {
    !s.prod() && s.env("OC_SMTP_URL").is_none_or(|v| v.is_empty())
}

async fn mail(State(s): State<AppState>, raw: Request) -> Response {
    if !enabled(&s) {
        return crate::http::not_found(raw).await;
    }
    match handle(&s, raw).await {
        Ok(r) => r.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn handle(s: &AppState, raw: Request) -> ApiResult {
    let req = Req::from_request(raw, s).await?;
    // `to ?? null` — a repeated parameter is an array; node-postgres sends it as an array literal.
    let to = match req.query.get("to") {
        None => None,
        Some(Value::String(t)) => Some(t.clone()),
        Some(Value::Array(a)) => Some(format!(
            "{{{}}}",
            a.iter().map(|v| format!("\"{}\"", v.as_str().unwrap_or("").replace('\\', "\\\\").replace('"', "\\\""))).collect::<Vec<_>>().join(",")
        )),
        Some(_) => None,
    };
    let messages = pg::json_rows(
        &s.pool,
        r#"SELECT id, to_email AS "to", subject, text, html, status, created_at AS "createdAt" FROM email_outbox
         WHERE ($1::text IS NULL OR to_email = lower($1)) ORDER BY created_at DESC LIMIT 50"#,
        &[Arg::Text(to)],
    )
    .await?;
    Ok(Reply::json(json!({ "messages": messages })))
}
