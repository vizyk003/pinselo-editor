//! Webhooks (API-008): events fan out to subscribed endpoints as signed POSTs, delivered by the worker
//! through the job queue with exponential retries (6 attempts; endpoints are switched off after 50
//! consecutive failures). Signature: `OC-Signature: t=<unix>,v1=<hex HMAC-SHA256(secret, "t.body")>`.
//!
//! The delivered body is `JSON.stringify` of the stored jsonb payload — i.e. Postgres' jsonb key order,
//! compact, numbers printed as JS would — so a Rust worker sends byte-identical bodies to a TS worker.

use std::time::Duration;

use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use sqlx::PgPool;
use uuid::Uuid;

pub const WEBHOOK_EVENTS: [&str; 7] = ["design.created", "design.deleted", "export.completed", "form.response", "comment.created", "site.published", "ping"];

/// Attempts per delivery (job `max_attempts` and the `failed` threshold).
pub const MAX_ATTEMPTS: i32 = 6;
/// Consecutive failures after which a webhook is deactivated.
pub const DISABLE_AFTER_FAILURES: i32 = 50;
pub const USER_AGENT: &str = "Pinselo-Webhooks/1";
pub const SIGNATURE_HEADER: &str = "oc-signature";
pub const DELIVERY_HEADER: &str = "oc-delivery";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WebhookEvent {
    #[serde(rename = "design.created")]
    DesignCreated,
    #[serde(rename = "design.deleted")]
    DesignDeleted,
    #[serde(rename = "export.completed")]
    ExportCompleted,
    #[serde(rename = "form.response")]
    FormResponse,
    #[serde(rename = "comment.created")]
    CommentCreated,
    #[serde(rename = "site.published")]
    SitePublished,
    #[serde(rename = "ping")]
    Ping,
}

impl WebhookEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            WebhookEvent::DesignCreated => "design.created",
            WebhookEvent::DesignDeleted => "design.deleted",
            WebhookEvent::ExportCompleted => "export.completed",
            WebhookEvent::FormResponse => "form.response",
            WebhookEvent::CommentCreated => "comment.created",
            WebhookEvent::SitePublished => "site.published",
            WebhookEvent::Ping => "ping",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        use WebhookEvent::*;
        [DesignCreated, DesignDeleted, ExportCompleted, FormResponse, CommentCreated, SitePublished, Ping].into_iter().find(|e| e.as_str() == s)
    }
}

/// `signPayload(secret, body, t)`.
pub fn sign_payload_at(secret: &str, body: &str, t: i64) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    m.update(format!("{t}.{body}").as_bytes());
    format!("t={t},v1={}", hex::encode(m.finalize().into_bytes()))
}

/// `signPayload(secret, body)` at the current time.
pub fn sign_payload(secret: &str, body: &str) -> String {
    sign_payload_at(secret, body, oc_js::now_ms().div_euclid(1000))
}

/// Payload stored for each delivery: `{ id, event, created, data }`.
#[derive(Debug, Clone, Serialize)]
pub struct EventPayload<'a> {
    pub id: Uuid,
    pub event: &'a str,
    pub created: String,
    pub data: &'a serde_json::Value,
}

/// Queue an event for every active webhook of the user that subscribes to it. Returns the number of hooks. Disabled
/// or deleted owners get nothing, so a webhook that slipped past a revocation sweep (e.g. a tombstoned account) stays
/// silent.
pub async fn emit_event(pool: &PgPool, user_id: Uuid, event: WebhookEvent, data: &serde_json::Value) -> Result<usize, sqlx::Error> {
    let hooks: Vec<Uuid> = sqlx::query_scalar(
        "SELECT w.id FROM webhooks w JOIN users u ON u.id = w.user_id
          WHERE w.user_id = $1 AND w.active AND $2 = ANY(w.events) AND u.disabled_at IS NULL AND u.deleted_at IS NULL",
    )
    .bind(user_id)
    .bind(event.as_str())
    .fetch_all(pool)
    .await?;
    for h in &hooks {
        let id = oc_db::uuid();
        let payload = EventPayload { id, event: event.as_str(), created: oc_js::iso_string(&chrono::Utc::now()), data };
        sqlx::query("INSERT INTO webhook_deliveries(id, webhook_id, event, payload) VALUES ($1,$2,$3,$4)")
            .bind(id)
            .bind(h)
            .bind(event.as_str())
            .bind(sqlx::types::Json(&payload))
            .execute(pool)
            .await?;
        oc_jobs::enqueue(pool, "webhook", &serde_json::json!({ "deliveryId": id }), oc_jobs::EnqueueOpts { max_attempts: Some(MAX_ATTEMPTS), ..Default::default() }).await?;
    }
    Ok(hooks.len())
}

/// `JSON.stringify(row.payload)` given the jsonb's text output (`payload::text`).
pub fn body_from_jsonb_text(jsonb_text: &str) -> Result<String, serde_json::Error> {
    Ok(oc_js::JsValue::parse(jsonb_text)?.stringify())
}

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    /// A failed attempt (the message is what is stored in `webhook_deliveries.error`); retried by the queue.
    #[error("{0}")]
    Attempt(String),
}

fn is_production() -> bool {
    std::env::var("NODE_ENV").as_deref() == Ok("production")
}

fn client() -> reqwest::ClientBuilder {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(10))
}

async fn attempt(url: &str, delivery_id: Uuid, secret: &str, body: &str) -> Result<u16, (Option<u16>, String)> {
    let u = url::Url::parse(url).map_err(|_| (None, "Invalid URL".to_owned()))?;
    let host = u.host_str().unwrap_or("").to_owned();
    if u.scheme() != "https" && !(!is_production() && host == "localhost") {
        return Err((None, "webhook URLs must use https".into()));
    }
    let mut client = client();
    if is_production() {
        // Blind SSRF: the status code alone would map internal ports, so the connection stays on the checked addresses.
        let private = || (None, "webhook host resolves to a private address".to_owned());
        if !oc_netguard::is_public_host(&host) {
            return Err(private());
        }
        let lookup_host = host.trim_start_matches('[').trim_end_matches(']').to_owned();
        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((lookup_host.as_str(), 0)).await.map_err(|e| (None, e.to_string()))?.collect();
        if addrs.is_empty() || addrs.iter().any(|a| !oc_netguard::is_public_ip(a.ip())) {
            return Err(private());
        }
        client = client.resolve_to_addrs(&lookup_host, &addrs);
    }
    let res = client
        .build()
        .map_err(|e| (None, e.to_string()))?
        .post(u)
        .header("content-type", "application/json")
        .header("user-agent", USER_AGENT)
        .header(SIGNATURE_HEADER, sign_payload(secret, body))
        .header(DELIVERY_HEADER, delivery_id.to_string())
        .body(body.to_owned())
        .send()
        .await
        .map_err(|e| (None, e.to_string()))?;
    let code = res.status().as_u16();
    if !res.status().is_success() {
        return Err((Some(code), format!("HTTP {code}")));
    }
    Ok(code)
}

/// Worker handler: one delivery attempt. Returns `Err(Attempt)` to let the queue retry with backoff.
pub async fn deliver_webhook(pool: &PgPool, delivery_id: Uuid) -> Result<(), WebhookError> {
    let d: Option<(Uuid, String, i32, String, String, Uuid, bool)> = sqlx::query_as(
        "SELECT d.id, d.payload::text, d.attempts, w.url, w.secret, w.id AS webhook_id, w.active FROM webhook_deliveries d JOIN webhooks w ON w.id = d.webhook_id WHERE d.id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(pool)
    .await?;
    let Some((id, payload, attempts, url, secret, webhook_id, active)) = d else { return Ok(()) };
    if !active {
        return Ok(());
    }
    let body = body_from_jsonb_text(&payload).map_err(|e| WebhookError::Attempt(e.to_string()))?;
    let (code, error) = match attempt(&url, id, &secret, &body).await {
        Ok(c) => (Some(c as i32), None),
        Err((c, e)) => (c.map(i32::from), Some(e)),
    };
    let status = match &error {
        Some(_) if attempts + 1 >= MAX_ATTEMPTS => "failed",
        Some(_) => "pending",
        None => "delivered",
    };
    sqlx::query(
        "UPDATE webhook_deliveries SET attempts = attempts + 1, response_code = $2, error = $3, status = $4, delivered_at = CASE WHEN $3::text IS NULL THEN now() ELSE delivered_at END WHERE id = $1",
    )
    .bind(id)
    .bind(code)
    .bind(&error)
    .bind(status)
    .execute(pool)
    .await?;
    if let Some(error) = error {
        // Endpoints that keep failing are switched off (the owner sees it in Settings → Developers).
        let failures: Option<i32> = sqlx::query_scalar("UPDATE webhooks SET failures = failures + 1 WHERE id = $1 RETURNING failures").bind(webhook_id).fetch_optional(pool).await?;
        if failures.unwrap_or(0) >= DISABLE_AFTER_FAILURES {
            sqlx::query("UPDATE webhooks SET active = false WHERE id = $1").bind(webhook_id).execute(pool).await?;
        }
        return Err(WebhookError::Attempt(error));
    }
    sqlx::query("UPDATE webhooks SET failures = 0 WHERE id = $1").bind(webhook_id).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events() {
        for e in WEBHOOK_EVENTS {
            assert_eq!(WebhookEvent::parse(e).unwrap().as_str(), e);
        }
    }
}
