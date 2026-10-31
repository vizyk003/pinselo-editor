//! Port of services/api/src/print/payment.ts — one-time payment for print orders through the billing provider
//! abstraction: Stripe Checkout (mode=payment) when `OC_STRIPE_SECRET_KEY` is set, otherwise the local provider's
//! confirmation page (development / self-hosted without payments — the order is marked paid without a charge).
//!
//! Deviation: `OC_STRIPE_API_URL` overrides `https://api.stripe.com/v1` (the parity harness points it at its fake).

use futures::future::BoxFuture;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::http::error::ApiError;
use crate::pg;
use crate::print::fulfil;
use crate::print::store::{Order, OrderExt, Patch, StatusChange, StatusError, format_money, get_order, set_status};
use crate::state::AppState;

/// `PaymentUnavailable` (503 in the routes) or anything else.
#[derive(Debug)]
pub enum PayError {
    Unavailable(String),
    Api(ApiError),
}

impl From<ApiError> for PayError {
    fn from(e: ApiError) -> Self {
        PayError::Api(e)
    }
}

impl From<sqlx::Error> for PayError {
    fn from(e: sqlx::Error) -> Self {
        PayError::Api(e.into())
    }
}

pub fn payment_provider_id(s: &AppState) -> &'static str {
    if s.env_nonempty("OC_STRIPE_SECRET_KEY").is_some() { "stripe" } else { "local" }
}

/// `stripe(method, path, body)` of billing.ts (POST), with the base URL override.
async fn stripe(s: &AppState, path: &str, body: &Map<String, Value>) -> Result<Value, ApiError> {
    let base = s.env_nonempty("OC_STRIPE_API_URL").unwrap_or_else(|| "https://api.stripe.com/v1".into());
    let mut parts = Vec::new();
    crate::billing::stripe_form(body, "", &mut parts);
    let res = s
        .http
        .post(format!("{base}{path}"))
        .header("authorization", format!("Bearer {}", s.env("OC_STRIPE_SECRET_KEY").unwrap_or_else(|| "undefined".into())))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("stripe-version", "2025-09-30.clover")
        .body(parts.join("&"))
        .send()
        .await?;
    let status = res.status();
    let j = crate::http::jsjson::parse(&res.text().await?).map_err(ApiError::internal)?;
    if !status.is_success() {
        let msg = j.pointer("/error/message").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| status.as_u16().to_string());
        return Err(ApiError::internal(format!("stripe: {msg}")));
    }
    Ok(j)
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// `stripeCheckoutBody(o)`.
pub fn stripe_checkout_body(s: &AppState, o: &Order) -> Map<String, Value> {
    let cur = o.text("currency").to_lowercase();
    let auto_tax = s.env("OC_STRIPE_TAX").as_deref() == Some("1");
    let line = |name: String, amount: f64| json!({ "price_data": { "currency": cur, "unit_amount": amount, "product_data": { "name": name } }, "quantity": 1 });
    let mut items = vec![
        line(format!("{} × {} ({})", o.text("product_name"), oc_js::number_to_string(o.num_of("quantity")), o.text("number")), o.num_of("subtotal_cents")),
        line("Shipping".into(), o.num_of("shipping_cents")),
    ];
    if !auto_tax && o.num_of("tax_cents") > 0.0 {
        items.push(line("Estimated tax".into(), o.num_of("tax_cents")));
    }
    let id = o.id();
    let web = &s.cfg.web_origin;
    let mut m = obj(json!({
        "mode": "payment", "customer_email": o.text("email"), "client_reference_id": id,
        "line_items": Value::Object(items.into_iter().enumerate().map(|(i, x)| (i.to_string(), x)).collect()),
        "metadata": { "print_order_id": id, "print_order_number": o.text("number") },
        "payment_intent_data": { "metadata": { "print_order_id": id } },
    }));
    if auto_tax {
        m.insert("automatic_tax".into(), json!({ "enabled": true }));
    }
    m.insert("success_url".into(), json!(format!("{web}/orders/{id}?checkout=success")));
    m.insert("cancel_url".into(), json!(format!("{web}/orders/{id}?checkout=cancelled")));
    m
}

/// Start (or restart) payment for a pending order; returns the URL to send the customer to.
pub async fn start_payment(s: &AppState, o: &Order) -> Result<String, PayError> {
    if payment_provider_id(s) == "stripe" {
        let r = stripe(s, "/checkout/sessions", &stripe_checkout_body(s, o)).await?;
        let sid = match r.get("id") {
            Some(Value::String(x)) => Some(x.clone()),
            Some(Value::Null) | None => None,
            Some(other) => Some(crate::http::jsjson::stringify(other)),
        };
        pg::exec(&s.pool, "UPDATE print_orders SET payment_provider = 'stripe', payment_ref = $2, updated_at = now() WHERE id = $1::uuid", &[pg::text(o.id()), pg::opt_text(sid)]).await?;
        return Ok(match r.get("url") {
            Some(Value::String(u)) => u.clone(),
            Some(Value::Null) | None => "undefined".into(),
            Some(other) => crate::http::jsjson::stringify(other),
        });
    }
    if !crate::billing::local_billing_allowed(s) {
        return Err(PayError::Unavailable("Payments are not configured on this server.".into()));
    }
    let uid = o.str_of("user_id").and_then(|u| Uuid::parse_str(u).ok());
    let token = crate::auth::issue_token(s, "billing", uid, None, Some(json!({ "printOrderId": o.id() })), 60).await?;
    Ok(format!("{}/print/checkout?order={}&session={token}", s.cfg.web_origin, o.id()))
}

/// Payment confirmed: mark paid, email the customer, queue fulfilment. Idempotent.
pub async fn mark_paid(s: &AppState, order_id: &str, provider: &str, reference: Option<String>) -> Result<(), StatusError> {
    let Some(o) = get_order(s, order_id).await? else { return Ok(()) };
    if o.status() != "pending_payment" {
        return Ok(());
    }
    let note = if provider == "local" { "Marked paid (local billing, no charge)." } else { "Paid with Stripe." };
    let paid = set_status(
        s,
        order_id,
        "paid",
        StatusChange { source: "payment", note: Some(note.into()), patch: Patch { payment_provider: Some(provider.into()), payment_ref: reference, ..Default::default() }, ..Default::default() },
    )
    .await?;
    let tpl = oc_mail::Template::PrintConfirmed {
        number: paid.text("number"),
        product: format!("{} × {}", paid.text("product_name"), oc_js::number_to_string(paid.num_of("quantity"))),
        total: format_money(paid.num_of("total_cents"), &paid.text("currency")),
        url: format!("{}/orders/{}", s.cfg.web_origin, paid.id()),
    };
    let uid = paid.str_of("user_id").and_then(|u| Uuid::parse_str(u).ok());
    let _ = oc_mail::send_email(&s.pool, &paid.text("email"), &tpl, uid).await;
    fulfil::enqueue_fulfil(s, &paid).await?;
    Ok(())
}

/// `confirmLocalPayment(orderId, session, userId)`.
pub async fn confirm_local_payment(s: &AppState, order_id: &str, session: &str, user_id: Uuid) -> Result<bool, StatusError> {
    if payment_provider_id(s) != "local" || !crate::billing::local_billing_allowed(s) {
        return Ok(false);
    }
    let Some(t) = crate::auth::consume_token(s, "billing", session).await? else { return Ok(false) };
    let wanted = t.data.as_ref().and_then(|d| d.0.get("printOrderId")).and_then(Value::as_str);
    if wanted != Some(order_id) || t.user_id != Some(user_id) {
        return Ok(false);
    }
    mark_paid(s, order_id, "local", None).await?;
    Ok(true)
}

/// Stripe webhook hook: `checkout.session.completed` / `async_payment_succeeded` for print orders.
pub fn on_stripe_event(s: AppState, evt: Value) -> BoxFuture<'static, Result<(), ApiError>> {
    Box::pin(async move {
        let ty = evt.get("type").and_then(Value::as_str).unwrap_or("");
        if ty != "checkout.session.completed" && ty != "checkout.session.async_payment_succeeded" {
            return Ok(());
        }
        let o = evt.pointer("/data/object").cloned().unwrap_or(Value::Null);
        let Some(id) = o.pointer("/metadata/print_order_id").and_then(Value::as_str).filter(|i| !i.is_empty()) else { return Ok(()) };
        if o.get("payment_status").and_then(Value::as_str) != Some("paid") {
            return Ok(());
        }
        let reference = o.get("payment_intent").filter(|v| !v.is_null()).or_else(|| o.get("id")).and_then(Value::as_str).map(str::to_owned);
        mark_paid(&s, id, "stripe", reference).await.map_err(|e| match e {
            StatusError::Api(a) => a,
            other => ApiError::internal(other.to_string()),
        })
    })
}

/// Refund a paid order (Stripe refunds the payment intent; local billing has nothing to refund).
pub async fn refund(s: &AppState, o: &Order) -> Result<String, ApiError> {
    let provider = o.str_of("payment_provider");
    if provider == Some("stripe") && o.str_of("payment_ref").is_some_and(|r| r.starts_with("pi_")) {
        let body = obj(json!({ "payment_intent": o.text("payment_ref"), "metadata": { "print_order_id": o.id() } }));
        stripe(s, "/refunds", &body).await?;
        return Ok("Refunded through Stripe.".into());
    }
    Ok(if provider == Some("local") { "No charge was taken (local billing).".into() } else { "Refund manually in the payment provider.".into() })
}
