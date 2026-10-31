//! Port of services/api/src/routes/billing.ts — billing API (PL-020..026): plans, checkout, customer portal,
//! Stripe webhooks and the local stand-in provider. Paid plans are off unless `OC_BILLING=1` (no pricing, no
//! checkout); the Stripe webhook stays on either way (print orders).

use axum::extract::{FromRequest, MatchedPath, Request};
use axum::response::{IntoResponse, Response};

use crate::analytics;
use crate::auth::personal_workspace;
use crate::billing::{self, CheckoutInput, PLAN_IDS, PlanId};
use crate::prelude::*;
use crate::routes::teams::team_role_str;

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/billing/plans", get(plans))
        .route("/api/billing/stripe/webhook", post(stripe_webhook))
        .route("/api/billing", get(overview))
        .route("/api/billing/checkout", post(checkout))
        .route("/api/billing/portal", post(portal))
        .route("/api/billing/local/confirm", post(local_confirm))
        .route("/api/billing/local/cancel", post(local_cancel))
        .route("/api/billing/local/seats", post(local_seats))
}

async fn can_manage(s: &AppState, user_id: Uuid, workspace_id: &str) -> Result<bool, ApiError> {
    let r = team_role_str(s, &user_id.to_string(), workspace_id).await?;
    Ok(matches!(r.as_deref(), Some("owner" | "admin")))
}

async fn audit(s: &AppState, actor: Uuid, workspace_id: &str, action: &str, meta: Option<Value>, ip: &str) -> Result<(), ApiError> {
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: Some(actor), workspace_id: Uuid::parse_str(workspace_id).ok(), action: action.into(), meta, ip: Some(ip.into()), ..Default::default() },
        s.now(),
    )
    .await?;
    Ok(())
}

async fn plans(State(s): State<AppState>, _req: Req) -> ApiResult {
    let enabled = billing::plans_enabled(&s);
    let list: Vec<Value> = if enabled { PLAN_IDS.iter().map(|p| serde_json::to_value(billing::plan(*p))).collect::<Result<_, _>>()? } else { vec![] };
    Ok(Reply::json(json!({ "enabled": enabled, "plans": list, "provider": billing::provider_id(&s) })))
}

/// Stripe → us. Signature over the raw body; events are applied idempotently. `config: { rateLimit: false }`:
/// the route is not counted (the matched path is dropped before [`Req`] would count it).
async fn stripe_webhook(State(s): State<AppState>, raw: Request) -> Response {
    match webhook(&s, raw).await {
        Ok(r) => r.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn webhook(s: &AppState, raw: Request) -> ApiResult {
    let (mut parts, body) = raw.into_parts();
    parts.extensions.remove::<MatchedPath>();
    let bytes = axum::body::to_bytes(body, crate::http::req::BODY_LIMIT + 1).await.unwrap_or_default();
    let json_body = parts.headers.get("content-type").and_then(|v| v.to_str().ok()).and_then(crate::http::req::media_type).as_deref() == Some("application/json");
    let raw_text = json_body.then(|| String::from_utf8_lossy(&bytes).into_owned());
    let req = Req::from_request(Request::from_parts(parts, axum::body::Body::from(bytes)), s).await?;
    let raw_body = raw_text.unwrap_or_default();
    let secret = s.env_nonempty("OC_STRIPE_WEBHOOK_SECRET");
    let sig = req.header("stripe-signature").unwrap_or("");
    if !secret.is_some_and(|secret| billing::verify_stripe_signature(&raw_body, sig, &secret, s.now_ms())) {
        return Ok(reply::json(400, json!({ "error": "bad_signature" })));
    }
    let evt = crate::http::jsjson::parse(&raw_body).map_err(ApiError::internal)?;
    billing::apply_stripe_event(s, &evt).await?;
    Ok(Reply::json(json!({ "received": true })))
}

async fn overview(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let ws = match req.query.get("workspace") {
        Some(Value::String(w)) => w.clone(),
        Some(other) => crate::http::jsjson::stringify(other),
        None => personal_workspace(&s, u.id).await?.to_string(),
    };
    if team_role_str(&s, &u.id.to_string(), &ws).await?.is_none() {
        return Ok(Reply::json(json!({ "error": "not_found" })));
    }
    let wsa = || pg::text(ws.clone());
    let sub = pg::json_one(
        &s.pool,
        "SELECT plan, status, seats, interval, provider, current_period_end, cancel_at_period_end, trial_end FROM subscriptions WHERE workspace_id = $1::uuid",
        &[wsa()],
    )
    .await?;
    let invoices = pg::json_rows(
        &s.pool,
        "SELECT id, amount_cents, currency, status, url, period_start, period_end, created_at FROM invoices WHERE workspace_id = $1::uuid ORDER BY created_at DESC LIMIT 24",
        &[wsa()],
    )
    .await?;
    let used: Option<String> = sqlx::query_scalar("SELECT coalesce(sum(bytes),0)::text AS total FROM assets WHERE owner_id = $1 AND trashed_at IS NULL").bind(u.id).fetch_optional(&s.pool).await?;
    let members: Option<i32> = sqlx::query_scalar("SELECT count(*)::int AS n FROM memberships WHERE workspace_id = $1::uuid").bind(&ws).fetch_optional(&s.pool).await?;
    let manageable = pg::json_rows(
        &s.pool,
        "SELECT w.id, w.name, w.personal, coalesce(s.plan, 'free') AS plan FROM memberships m JOIN workspaces w ON w.id = m.workspace_id
         LEFT JOIN subscriptions s ON s.workspace_id = w.id WHERE m.user_id = $1 AND m.role IN ('owner','admin') ORDER BY w.personal DESC, w.name",
        &[pg::uuid(u.id)],
    )
    .await?;
    let (e, p) = billing::entitlements_of(&s, u.id).await?;
    let storage = crate::http::jsjson::number_value(used.as_deref().unwrap_or("0").parse::<f64>().unwrap_or(0.0)).unwrap_or(json!(0));
    Ok(Reply::json(json!({
        "workspaceId": ws, "subscription": sub, "invoices": invoices, "members": members.unwrap_or(1), "entitlements": billing::entitlements_json(&e, p),
        "usage": { "storageBytes": storage }, "provider": billing::provider_id(&s), "manageable": manageable,
    })))
}

async fn checkout(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !billing::plans_enabled(&s) {
        return Ok(reply::json(404, json!({ "error": "not_found", "message": "Everything is free; there is no plan to buy." })));
    }
    let b = z::obj([
        ("plan", z::enumeration(&["pro", "teams"])),
        ("interval", z::enumeration(&["month", "year"])),
        ("workspaceId", z::str().uuid().optional()),
        ("seats", z::num().int().min(1.0).max(5000.0).optional()),
    ])
    .parse(req.body())?;
    let plan = b["plan"].as_str().unwrap_or("").to_owned();
    let interval = b["interval"].as_str().unwrap_or("").to_owned();
    let personal = personal_workspace(&s, u.id).await?.to_string();
    let ws = b.get("workspaceId").and_then(Value::as_str).map_or_else(|| personal.clone(), str::to_owned);
    if !can_manage(&s, u.id, &ws).await? {
        return Ok(reply::json(403, json!({ "error": "forbidden", "message": "Only team owners and admins can change the plan." })));
    }
    if plan == "pro" && ws != personal {
        return Ok(reply::json(400, json!({ "error": "pro_is_personal", "message": "Pro is for your personal workspace. Choose Teams for a team." })));
    }
    if plan == "teams" && ws == personal {
        return Ok(reply::json(400, json!({ "error": "teams_needs_team", "message": "Create a team first, then upgrade it to Teams." })));
    }
    let members: Option<i32> = sqlx::query_scalar("SELECT count(*)::int AS n FROM memberships WHERE workspace_id = $1::uuid").bind(&ws).fetch_optional(&s.pool).await?;
    let members = i64::from(members.unwrap_or(1));
    let seats = if plan == "pro" { 1 } else { billing::plan(PlanId::Teams).min_seats.max(members).max(b.get("seats").and_then(Value::as_f64).unwrap_or(0.0) as i64) };
    if billing::provider_id(&s) == "local" && !billing::local_billing_allowed(&s) {
        return Ok(reply::json(503, json!({ "error": "billing_unavailable", "message": "Billing is not configured." })));
    }
    let input = CheckoutInput { workspace_id: ws.clone(), plan: plan.clone(), interval: interval.clone(), seats, email: u.email.clone(), user_id: u.id.to_string() };
    let url = billing::provider_checkout(&s, &input).await?;
    audit(&s, u.id, &ws, "billing.checkout_started", Some(json!({ "plan": plan, "interval": interval, "seats": seats })), &req.ip).await?;
    Ok(Reply::json(json!({ "url": url })))
}

async fn portal(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("workspaceId", z::str().uuid())]).parse(req.body())?;
    let ws = b["workspaceId"].as_str().unwrap_or("").to_owned();
    if !can_manage(&s, u.id, &ws).await? {
        return Ok(reply::json(403, json!({ "error": "forbidden" })));
    }
    Ok(Reply::json(json!({ "url": billing::provider_portal(&s, &ws).await? })))
}

// ------------------------------------------------ local provider (development / self-hosted without Stripe)

async fn local_confirm(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !billing::local_billing_allowed(&s) || !billing::plans_enabled(&s) {
        return Ok(Reply::empty(404));
    }
    let b = z::obj([("session", z::str().max(200.0))]).parse(req.body())?;
    let session = b["session"].as_str().unwrap_or("").to_owned();
    let r: Result<String, ApiError> = async {
        let ws = billing::local_confirm(&s, &session).await?;
        audit(&s, u.id, &ws, "billing.plan_changed", Some(json!({ "provider": "local" })), &req.ip).await?;
        analytics::track(&s, Some(u.id), "plan_changed", json!({ "provider": "local" })).await?;
        Ok(ws)
    }
    .await;
    match r {
        Ok(ws) => Ok(Reply::json(json!({ "workspaceId": ws }))),
        Err(_) => Ok(reply::json(400, json!({ "error": "expired", "message": "This checkout expired. Start again from Pricing." }))),
    }
}

async fn local_cancel(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !billing::local_billing_allowed(&s) {
        return Ok(Reply::empty(404));
    }
    let b = z::obj([("workspaceId", z::str().uuid()), ("resume", z::bool().optional())]).parse(req.body())?;
    let ws = b["workspaceId"].as_str().unwrap_or("").to_owned();
    let resume = b.get("resume").and_then(Value::as_bool);
    if !can_manage(&s, u.id, &ws).await? {
        return Ok(reply::json(403, json!({ "error": "forbidden" })));
    }
    billing::local_cancel(&s, &ws, resume.unwrap_or(false)).await?;
    audit(&s, u.id, &ws, if resume == Some(true) { "billing.resumed" } else { "billing.canceled" }, None, &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn local_seats(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !billing::local_billing_allowed(&s) {
        return Ok(Reply::empty(404));
    }
    let b = z::obj([("workspaceId", z::str().uuid()), ("seats", z::num().int().min(1.0).max(5000.0))]).parse(req.body())?;
    let ws = b["workspaceId"].as_str().unwrap_or("").to_owned();
    let seats = b["seats"].as_f64().unwrap_or(0.0) as i64;
    if !can_manage(&s, u.id, &ws).await? {
        return Ok(reply::json(403, json!({ "error": "forbidden" })));
    }
    let members: Option<i32> = sqlx::query_scalar("SELECT count(*)::int AS n FROM memberships WHERE workspace_id = $1::uuid").bind(&ws).fetch_optional(&s.pool).await?;
    let wsu = Uuid::parse_str(&ws).map_err(ApiError::internal)?;
    let p = billing::workspace_plan(&s, wsu).await?;
    let need = i64::from(members.unwrap_or(1)).max(billing::plan(p.plan).min_seats);
    if seats < need {
        return Ok(reply::json(400, json!({ "error": "too_few_seats", "message": format!("You need at least {need} seats.") })));
    }
    billing::local_set_seats(&s, &ws, seats).await?;
    Ok(Reply::json(json!({ "ok": true })))
}
