//! Port of services/api/src/billing.ts — plans and entitlements (PL-020..026).
//!
//! Ported: `PLANS`, `UpgradeRequired`, `plansEnabled`, `allFree`, `planOf`, `entitlementsOf`, `workspacePlan`,
//! `workspaceEntitlements`, `requireFeature`. Paid plans are off unless `OC_BILLING=1`: then everyone gets every
//! feature (`allFree`), storage stays the `OC_UPLOAD_QUOTA` fair-use quota.
//!
//! Also the providers (`localProvider`, `stripeProvider`, `stripe()`), `verifyStripeSignature`,
//! `applyStripeEvent` (+ `stripeEventHooks`: [`register_stripe_event_hook`]), `billingCurrency`,
//! `localBillingAllowed`.

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::http::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanId {
    Free,
    Pro,
    Teams,
}

impl PlanId {
    pub fn as_str(self) -> &'static str {
        match self {
            PlanId::Free => "free",
            PlanId::Pro => "pro",
            PlanId::Teams => "teams",
        }
    }
    pub fn parse(s: &str) -> Option<PlanId> {
        match s {
            "free" => Some(PlanId::Free),
            "pro" => Some(PlanId::Pro),
            "teams" => Some(PlanId::Teams),
            _ => None,
        }
    }
}

/// `Entitlements` (JSON key order as in the TS object literals).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlements {
    pub storage_bytes: i64,
    pub brand_kits: i64,
    #[serde(rename = "export4k")]
    pub export_4k: bool,
    pub premium_templates: bool,
    pub version_history_days: Option<i64>,
    pub team_members: Option<i64>,
    pub approvals: bool,
    pub brand_controls: bool,
    pub priority_support: bool,
    pub custom_domains: bool,
}

/// `keyof Entitlements` for `requireFeature`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    Export4k,
    PremiumTemplates,
    Approvals,
    BrandControls,
    PrioritySupport,
    CustomDomains,
}

impl Feature {
    pub fn key(self) -> &'static str {
        match self {
            Feature::Export4k => "export4k",
            Feature::PremiumTemplates => "premiumTemplates",
            Feature::Approvals => "approvals",
            Feature::BrandControls => "brandControls",
            Feature::PrioritySupport => "prioritySupport",
            Feature::CustomDomains => "customDomains",
        }
    }
}

impl Entitlements {
    pub fn has(&self, f: Feature) -> bool {
        match f {
            Feature::Export4k => self.export_4k,
            Feature::PremiumTemplates => self.premium_templates,
            Feature::Approvals => self.approvals,
            Feature::BrandControls => self.brand_controls,
            Feature::PrioritySupport => self.priority_support,
            Feature::CustomDomains => self.custom_domains,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub month: i64,
    pub year: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: PlanId,
    pub name: &'static str,
    pub tagline: &'static str,
    /// Price in cents per seat.
    pub price: Price,
    pub min_seats: i64,
    pub trial_days: i64,
    pub entitlements: Entitlements,
    pub highlights: Vec<&'static str>,
}

const GB: i64 = 1024 * 1024 * 1024;

/// `PLANS[id]`.
pub fn plan(id: PlanId) -> Plan {
    match id {
        PlanId::Free => Plan {
            id,
            name: "Free",
            tagline: "For anyone to design anything, on their own or with others.",
            price: Price { month: 0, year: 0 },
            min_seats: 1,
            trial_days: 0,
            entitlements: Entitlements {
                storage_bytes: 5 * GB,
                brand_kits: 1,
                export_4k: false,
                premium_templates: false,
                version_history_days: Some(30),
                team_members: Some(10),
                approvals: false,
                brand_controls: false,
                priority_support: false,
                custom_domains: false,
            },
            highlights: vec!["Every editor feature, video and whiteboards", "All free templates", "5 GB of storage", "Real-time collaboration, teams up to 10", "Exports up to 1080p"],
        },
        PlanId::Pro => Plan {
            id,
            name: "Pro",
            tagline: "For individuals who want premium content and more power.",
            price: Price { month: 1500, year: 12000 },
            min_seats: 1,
            trial_days: 30,
            entitlements: Entitlements {
                storage_bytes: 1024 * GB,
                brand_kits: 100,
                export_4k: true,
                premium_templates: true,
                version_history_days: None,
                team_members: Some(10),
                approvals: false,
                brand_controls: false,
                priority_support: true,
                custom_domains: true,
            },
            highlights: vec!["Everything in Free", "Premium templates", "1 TB of storage", "4K video export", "Unlimited version history", "100 brand kits"],
        },
        PlanId::Teams => Plan {
            id,
            name: "Teams",
            tagline: "For teams who create together with brand controls.",
            price: Price { month: 1000, year: 10000 },
            min_seats: 3,
            trial_days: 30,
            entitlements: Entitlements {
                storage_bytes: 1024 * GB,
                brand_kits: 1000,
                export_4k: true,
                premium_templates: true,
                version_history_days: None,
                team_members: None,
                approvals: true,
                brand_controls: true,
                priority_support: true,
                custom_domains: true,
            },
            highlights: vec![
                "Everything in Pro, for every member",
                "Unlimited members (billed per seat)",
                "Brand controls and design approvals",
                "Team-wide admin and usage reports",
            ],
        },
    }
}

pub const PLAN_IDS: [PlanId; 3] = [PlanId::Free, PlanId::Pro, PlanId::Teams];

/// `plansEnabled()`: paid plans exist only with `OC_BILLING=1`.
pub fn plans_enabled(state: &AppState) -> bool {
    state.env("OC_BILLING").as_deref() == Some("1")
}

/// `allFree()`: every feature, unlimited members; storage stays the fair-use quota.
pub fn all_free(state: &AppState) -> Entitlements {
    Entitlements { storage_bytes: state.cfg.upload_quota, team_members: None, ..plan(PlanId::Teams).entitlements }
}

const ACTIVE: &str = "s.status IN ('active','trialing','past_due')";

/// `planOf(userId)`: the best plan across a user's workspaces.
pub async fn plan_of(state: &AppState, user_id: Uuid) -> Result<PlanId, ApiError> {
    let sql = format!(
        "SELECT s.plan FROM subscriptions s JOIN memberships m ON m.workspace_id = s.workspace_id
     WHERE m.user_id = $1 AND {ACTIVE}
     ORDER BY CASE s.plan WHEN 'teams' THEN 2 WHEN 'pro' THEN 1 ELSE 0 END DESC LIMIT 1"
    );
    let r: Option<String> = sqlx::query_scalar(&sql).bind(user_id).fetch_optional(&state.pool).await?;
    Ok(r.as_deref().and_then(PlanId::parse).unwrap_or(PlanId::Free))
}

/// `entitlementsOf(userId)` (with its `plan`).
pub async fn entitlements_of(state: &AppState, user_id: Uuid) -> Result<(Entitlements, PlanId), ApiError> {
    if !plans_enabled(state) {
        return Ok((all_free(state), PlanId::Free));
    }
    let p = plan_of(state, user_id).await?;
    let mut e = plan(p).entitlements;
    // Self-hosters can lift the free storage quota.
    if p == PlanId::Free && state.env_nonempty("OC_UPLOAD_QUOTA").is_some() {
        e.storage_bytes = state.cfg.upload_quota;
    }
    Ok((e, p))
}

/// `entitlementsOf(userId)` as the JSON object (`{ ...entitlements, plan }`).
pub fn entitlements_json(e: &Entitlements, p: PlanId) -> Value {
    let mut v = serde_json::to_value(e).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        m.insert("plan".into(), Value::String(p.as_str().into()));
    }
    v
}

#[derive(Debug, Clone)]
pub struct WorkspacePlan {
    pub plan: PlanId,
    pub status: String,
    pub seats: i64,
}

/// `workspacePlan(workspaceId)`.
pub async fn workspace_plan(state: &AppState, workspace_id: Uuid) -> Result<WorkspacePlan, ApiError> {
    let sql = format!("SELECT plan, status, seats FROM subscriptions s WHERE workspace_id = $1 AND {ACTIVE}");
    let r: Option<(String, String, i32)> = sqlx::query_as(&sql).bind(workspace_id).fetch_optional(&state.pool).await?;
    Ok(match r {
        Some((plan, status, seats)) => WorkspacePlan { plan: PlanId::parse(&plan).unwrap_or(PlanId::Free), status, seats: i64::from(seats) },
        None => WorkspacePlan { plan: PlanId::Free, status: "active".into(), seats: 1 },
    })
}

/// `workspaceEntitlements(workspaceId)`: a team's own entitlements come from its plan.
pub async fn workspace_entitlements(state: &AppState, workspace_id: Uuid) -> Result<Entitlements, ApiError> {
    if !plans_enabled(state) {
        return Ok(all_free(state));
    }
    let wp = workspace_plan(state, workspace_id).await?;
    // Teams bills per seat, so its member cap is the seat count.
    Ok(if wp.plan == PlanId::Teams { Entitlements { team_members: Some(wp.seats), ..plan(PlanId::Teams).entitlements } } else { plan(wp.plan).entitlements })
}

/// `requireFeature(userId, feature, message)`: throws `UpgradeRequired` (402).
pub async fn require_feature(state: &AppState, user_id: Uuid, feature: Feature, message: &str) -> Result<(), ApiError> {
    let (e, _) = entitlements_of(state, user_id).await?;
    if !e.has(feature) {
        return Err(ApiError::Upgrade { feature: feature.key().into(), message: message.into() });
    }
    Ok(())
}

// ---------------------------------------------------------------- providers (ported with routes/billing.ts)

/// `CheckoutInput` (stored as the `billing` data of a local checkout token).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutInput {
    pub workspace_id: String,
    pub plan: String,
    pub interval: String,
    pub seats: i64,
    pub email: String,
    pub user_id: String,
}

/// `billingProvider().id`: Stripe when `OC_STRIPE_SECRET_KEY` is set, otherwise the local stand-in.
pub fn provider_id(state: &AppState) -> &'static str {
    if state.env_nonempty("OC_STRIPE_SECRET_KEY").is_some() { "stripe" } else { "local" }
}

/// `localBillingAllowed()`: never in production without `OC_LOCAL_BILLING=1`, never next to Stripe.
pub fn local_billing_allowed(state: &AppState) -> bool {
    state.env_nonempty("OC_STRIPE_SECRET_KEY").is_none() && (!state.prod() || state.env("OC_LOCAL_BILLING").as_deref() == Some("1"))
}

/// `billingCurrency()`: currency for one-time charges (print orders), ISO 4217, default usd.
pub fn billing_currency(state: &AppState) -> String {
    state.env("OC_BILLING_CURRENCY").unwrap_or_else(|| "usd".into()).to_lowercase()
}

/// Fields of `upsertSubscription(s)`.
#[derive(Debug, Clone, Default)]
pub struct SubscriptionUpsert {
    pub workspace_id: String,
    pub plan: String,
    pub status: String,
    /// The JS value (a non-integer number fails in Postgres, as in TS).
    pub seats: String,
    pub interval: String,
    pub provider: String,
    pub customer_id: Option<String>,
    pub subscription_id: Option<String>,
    pub period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub cancel_at_period_end: Option<bool>,
    pub trial_end: Option<chrono::DateTime<chrono::Utc>>,
}

/// `upsertSubscription(s)`.
pub async fn upsert_subscription(state: &AppState, s: &SubscriptionUpsert) -> Result<(), sqlx::Error> {
    use crate::pg::{self, Arg};
    pg::exec(
        &state.pool,
        "INSERT INTO subscriptions(workspace_id, plan, status, seats, interval, provider, customer_id, subscription_id, current_period_end, cancel_at_period_end, trial_end, updated_at)
     VALUES ($1::uuid,$2,$3,$4::int,$5,$6,$7,$8,$9,$10,$11, now())
     ON CONFLICT (workspace_id) DO UPDATE SET plan = $2, status = $3, seats = $4::int, interval = $5, provider = $6,
       customer_id = COALESCE($7, subscriptions.customer_id), subscription_id = COALESCE($8, subscriptions.subscription_id),
       current_period_end = $9, cancel_at_period_end = $10, trial_end = $11, updated_at = now()",
        &[
            pg::text(&s.workspace_id),
            pg::text(&s.plan),
            pg::text(&s.status),
            pg::text(&s.seats),
            pg::text(&s.interval),
            pg::text(&s.provider),
            Arg::Text(s.customer_id.clone()),
            Arg::Text(s.subscription_id.clone()),
            Arg::Time(s.period_end),
            Arg::Bool(Some(s.cancel_at_period_end.unwrap_or(false))),
            Arg::Time(s.trial_end),
        ],
    )
    .await?;
    Ok(())
}

/// `localProvider.checkout(i)`: a single-use token for the in-app confirmation page.
pub async fn local_checkout(state: &AppState, i: &CheckoutInput) -> Result<String, ApiError> {
    let user = Uuid::parse_str(&i.user_id).ok();
    let token = crate::auth::issue_token(state, "billing", user, None, Some(serde_json::json!({ "billing": i })), 30).await?;
    Ok(format!("{}/billing/checkout?session={token}", state.cfg.web_origin))
}

/// `localProvider.confirm(token)`: the workspace id, or `Err` (expired, unknown or broken data).
pub async fn local_confirm(state: &AppState, token: &str) -> Result<String, ApiError> {
    let t = crate::auth::consume_token(state, "billing", token).await?;
    let raw = t.and_then(|t| t.data).and_then(|d| d.0.get("billing").cloned()).filter(|b| !b.is_null());
    let Some(raw) = raw else { return Err(ApiError::internal("expired")) };
    let i: CheckoutInput = serde_json::from_value(raw)?;
    let now = state.now_ms();
    let plan_id = PlanId::parse(&i.plan).ok_or_else(|| ApiError::internal("unknown plan"))?;
    let p = plan(plan_id);
    let existing: Option<Option<chrono::DateTime<chrono::Utc>>> =
        sqlx::query_scalar("SELECT trial_end FROM subscriptions WHERE workspace_id = $1::uuid").bind(&i.workspace_id).fetch_optional(&state.pool).await?;
    let existing_trial = existing.flatten();
    let use_trial = p.trial_days > 0 && existing_trial.is_none();
    let days = if use_trial { p.trial_days } else if i.interval == "year" { 365 } else { 30 };
    let period_end = chrono::DateTime::from_timestamp_millis(now + days * 86_400_000).unwrap_or_default();
    upsert_subscription(
        state,
        &SubscriptionUpsert {
            workspace_id: i.workspace_id.clone(),
            plan: i.plan.clone(),
            status: if use_trial { "trialing" } else { "active" }.into(),
            seats: i.seats.to_string(),
            interval: i.interval.clone(),
            provider: "local".into(),
            period_end: Some(period_end),
            trial_end: if use_trial { Some(period_end) } else { existing_trial },
            ..Default::default()
        },
    )
    .await?;
    if !use_trial {
        let price = if i.interval == "year" { p.price.year } else { p.price.month };
        sqlx::query("INSERT INTO invoices(id, workspace_id, amount_cents, currency, status, period_start, period_end) VALUES ($1,$2::uuid,$3,$4,$5, now(), $6)")
            .bind(format!("local_{now}"))
            .bind(&i.workspace_id)
            .bind((price * i.seats) as i32)
            .bind("usd")
            .bind("paid")
            .bind(period_end)
            .execute(&state.pool)
            .await?;
    }
    Ok(i.workspace_id)
}

/// `localProvider.cancel(workspaceId, resume)`.
pub async fn local_cancel(state: &AppState, workspace_id: &str, resume: bool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE subscriptions SET cancel_at_period_end = $2, updated_at = now() WHERE workspace_id = $1::uuid").bind(workspace_id).bind(!resume).execute(&state.pool).await?;
    Ok(())
}

/// `localProvider.setSeats(workspaceId, seats)`.
pub async fn local_set_seats(state: &AppState, workspace_id: &str, seats: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE subscriptions SET seats = $2, updated_at = now() WHERE workspace_id = $1::uuid").bind(workspace_id).bind(seats as i32).execute(&state.pool).await?;
    Ok(())
}

/// `form(obj)`: Stripe's nested form encoding (`a[b][0]=…`), `null`/`undefined` skipped.
pub fn stripe_form(obj: &serde_json::Map<String, Value>, prefix: &str, out: &mut Vec<String>) {
    for (k, v) in obj {
        if v.is_null() {
            continue;
        }
        let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}[{k}]") };
        match v {
            Value::Object(m) => stripe_form(m, &key, out),
            Value::Array(a) => {
                let m: serde_json::Map<String, Value> = a.iter().enumerate().map(|(i, x)| (i.to_string(), x.clone())).collect();
                stripe_form(&m, &key, out);
            }
            Value::String(s) => out.push(format!("{}={}", oc_js::encode_uri_component(&key), oc_js::encode_uri_component(s))),
            Value::Bool(b) => out.push(format!("{}={b}", oc_js::encode_uri_component(&key))),
            Value::Number(n) => out.push(format!("{}={}", oc_js::encode_uri_component(&key), oc_js::number_to_string(n.as_f64().unwrap_or(f64::NAN)))),
            Value::Null => {}
        }
    }
}

/// `stripe(method, path, body)`: a Stripe REST call (form-encoded). Also used for one-time payments (print orders).
pub async fn stripe(state: &AppState, method: &str, path: &str, body: Option<&serde_json::Map<String, Value>>) -> Result<Value, ApiError> {
    let key = state.env("OC_STRIPE_SECRET_KEY").unwrap_or_else(|| "undefined".into());
    let m = if method == "GET" { reqwest::Method::GET } else { reqwest::Method::POST };
    let mut rb = state
        .http
        .request(m, format!("https://api.stripe.com/v1{path}"))
        .header("authorization", format!("Bearer {key}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("stripe-version", "2025-09-30.clover");
    if let Some(b) = body {
        let mut parts = Vec::new();
        stripe_form(b, "", &mut parts);
        rb = rb.body(parts.join("&"));
    }
    let res = rb.send().await?;
    let status = res.status();
    let j: Value = crate::http::jsjson::parse(&res.text().await?).map_err(ApiError::internal)?;
    if !status.is_success() {
        let msg = j.pointer("/error/message").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| status.as_u16().to_string());
        return Err(ApiError::internal(format!("stripe: {msg}")));
    }
    Ok(j)
}

fn stripe_price_id(state: &AppState, plan: &str, interval: &str) -> Result<String, ApiError> {
    let name = format!("OC_STRIPE_PRICE_{}_{}", plan.to_uppercase(), interval.to_uppercase());
    state.env_nonempty(&name).ok_or_else(|| ApiError::internal(format!("{name} is not set")))
}

/// `stripeProvider.checkout(i)`.
pub async fn stripe_checkout(state: &AppState, i: &CheckoutInput) -> Result<String, ApiError> {
    use serde_json::json;
    let customer: Option<Option<String>> = sqlx::query_scalar("SELECT customer_id FROM subscriptions WHERE workspace_id = $1::uuid").bind(&i.workspace_id).fetch_optional(&state.pool).await?;
    let customer = match customer.flatten() {
        Some(c) => c,
        None => {
            let body = json!({ "email": i.email, "metadata": { "workspace_id": i.workspace_id } });
            let c = stripe(state, "POST", "/customers", body.as_object()).await?;
            let id = c.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
            upsert_subscription(
                state,
                &SubscriptionUpsert {
                    workspace_id: i.workspace_id.clone(),
                    plan: "free".into(),
                    status: "incomplete".into(),
                    seats: "1".into(),
                    interval: i.interval.clone(),
                    provider: "stripe".into(),
                    customer_id: Some(id.clone()),
                    ..Default::default()
                },
            )
            .await?;
            id
        }
    };
    let had_trial: Option<Option<chrono::DateTime<chrono::Utc>>> =
        sqlx::query_scalar("SELECT trial_end FROM subscriptions WHERE workspace_id = $1::uuid").bind(&i.workspace_id).fetch_optional(&state.pool).await?;
    let trial = PlanId::parse(&i.plan).map_or(0, |p| plan(p).trial_days);
    let mut sub = json!({ "metadata": { "workspace_id": i.workspace_id, "plan": i.plan } });
    if had_trial.flatten().is_none() {
        sub["trial_period_days"] = json!(trial);
    }
    let tax = state.env("OC_STRIPE_TAX").as_deref() == Some("1");
    let mut body = json!({
        "mode": "subscription", "customer": customer, "client_reference_id": i.workspace_id, "allow_promotion_codes": true,
        "line_items": { "0": { "price": stripe_price_id(state, &i.plan, &i.interval)?, "quantity": i.seats } },
        "subscription_data": sub,
    });
    if tax {
        body["automatic_tax"] = json!({ "enabled": true });
        body["customer_update"] = json!({ "address": "auto" });
    }
    body["success_url"] = json!(format!("{}/settings/billing?checkout=success", state.cfg.web_origin));
    body["cancel_url"] = json!(format!("{}/pricing?checkout=cancelled", state.cfg.web_origin));
    let s = stripe(state, "POST", "/checkout/sessions", body.as_object()).await?;
    Ok(s.get("url").and_then(Value::as_str).unwrap_or("").to_owned())
}

/// `stripeProvider.portal(workspaceId)`.
pub async fn stripe_portal(state: &AppState, workspace_id: &str) -> Result<String, ApiError> {
    let c: Option<Option<String>> = sqlx::query_scalar("SELECT customer_id FROM subscriptions WHERE workspace_id = $1::uuid").bind(workspace_id).fetch_optional(&state.pool).await?;
    let Some(customer) = c.flatten() else { return Err(ApiError::internal("no customer")) };
    let body = serde_json::json!({ "customer": customer, "return_url": format!("{}/settings/billing", state.cfg.web_origin) });
    let r = stripe(state, "POST", "/billing_portal/sessions", body.as_object()).await?;
    Ok(r.get("url").and_then(Value::as_str).unwrap_or("").to_owned())
}

/// `billingProvider().checkout(i)`.
pub async fn provider_checkout(state: &AppState, i: &CheckoutInput) -> Result<String, ApiError> {
    if provider_id(state) == "stripe" { stripe_checkout(state, i).await } else { local_checkout(state, i).await }
}

/// `billingProvider().portal(workspaceId)`.
pub async fn provider_portal(state: &AppState, workspace_id: &str) -> Result<String, ApiError> {
    if provider_id(state) == "stripe" { stripe_portal(state, workspace_id).await } else { Ok(format!("{}/settings/billing", state.cfg.web_origin)) }
}

/// Node's `Buffer.from(s, 'hex')`: pairs of hex digits up to the first invalid one.
fn node_hex(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        let (Some(h), Some(l)) = ((b[i] as char).to_digit(16), (b[i + 1] as char).to_digit(16)) else { break };
        out.push((h * 16 + l) as u8);
        i += 2;
    }
    out
}

/// `verifyStripeSignature(payload, header, secret, now)`: Stripe-Signature v1 scheme, 5-minute tolerance.
pub fn verify_stripe_signature(payload: &str, header: &str, secret: &str, now_ms: i64) -> bool {
    use hmac::{Hmac, Mac};
    let mut t_raw: Option<String> = None;
    for p in header.split(',') {
        let mut it = p.split('=');
        if it.next() == Some("t") {
            t_raw = Some(it.next().map(str::to_owned).unwrap_or_else(|| "undefined".into()));
        }
    }
    let t = t_raw.as_deref().map_or(f64::NAN, crate::zod::js_string_to_number);
    if t.is_nan() || t == 0.0 || (now_ms as f64 / 1000.0 - t).abs() > 300.0 {
        return false;
    }
    let Ok(mut mac) = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()) else { return false };
    mac.update(format!("{}.{payload}", oc_js::number_to_string(t)).as_bytes());
    let expected = mac.finalize().into_bytes();
    header.split(',').filter(|p| p.starts_with("v1=")).any(|p| {
        let got = node_hex(&p[3..]);
        got.len() == expected.len() && subtle::ConstantTimeEq::ct_eq(got.as_slice(), expected.as_slice()).into()
    })
}

/// An extra Stripe event handler (`stripeEventHooks`, e.g. print orders on `checkout.session.completed`); runs
/// after the idempotency check.
pub type StripeHook = fn(AppState, Value) -> futures::future::BoxFuture<'static, Result<(), ApiError>>;

static STRIPE_EVENT_HOOKS: std::sync::Mutex<Vec<StripeHook>> = std::sync::Mutex::new(Vec::new());

/// `stripeEventHooks.push(hook)` (idempotent: a module registering from its `routes()` may be built twice).
pub fn register_stripe_event_hook(hook: StripeHook) {
    let mut v = STRIPE_EVENT_HOOKS.lock().expect("stripe hooks");
    if !v.iter().any(|h| std::ptr::fn_addr_eq(*h, hook)) {
        v.push(hook);
    }
}

/// `String(v)` of a JSON value passed to node-postgres (`None` = null/undefined).
fn pg_text(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(oc_js::number_to_string(n.as_f64().unwrap_or(f64::NAN))),
        Some(Value::Bool(b)) => Some(b.to_string()),
        Some(other) => Some(crate::http::jsjson::stringify(other)),
    }
}

fn type_error(what: &str) -> ApiError {
    ApiError::internal(format!("TypeError: Cannot read properties of undefined (reading '{what}')"))
}

/// `applyStripeEvent(evt)`: apply one webhook event (idempotent by event id).
pub async fn apply_stripe_event(state: &AppState, evt: &Value) -> Result<(), ApiError> {
    use crate::pg::{self, Arg};
    let fresh = pg::json_one(
        &state.pool,
        "INSERT INTO billing_events(id, type, payload) VALUES ($1,$2,$3::jsonb) ON CONFLICT DO NOTHING RETURNING id",
        &[Arg::Text(pg_text(evt.get("id"))), Arg::Text(pg_text(evt.get("type"))), pg::text(crate::http::jsjson::stringify(evt))],
    )
    .await?;
    if fresh.is_none() {
        return Ok(());
    }
    let hooks: Vec<StripeHook> = STRIPE_EVENT_HOOKS.lock().expect("stripe hooks").clone();
    for h in hooks {
        h(state.clone(), evt.clone()).await?;
    }
    let o = evt.get("data").ok_or_else(|| type_error("object"))?.get("object").cloned().unwrap_or(Value::Null);
    let ty = evt.get("type").and_then(Value::as_str).ok_or_else(|| type_error("startsWith"))?;
    let customer_ws = |customer: Option<String>| async move {
        let r: Option<uuid::Uuid> = sqlx::query_scalar("SELECT workspace_id FROM subscriptions WHERE customer_id = $1").bind(customer).fetch_optional(&state.pool).await?;
        Ok::<_, ApiError>(r.map(|u| u.to_string()))
    };
    if ty.starts_with("customer.subscription.") {
        let meta = o.get("metadata").filter(|m| !m.is_null()).ok_or_else(|| type_error("workspace_id"))?;
        let ws = match pg_text(meta.get("workspace_id")) {
            Some(w) => Some(w),
            None => customer_ws(pg_text(o.get("customer"))).await?,
        };
        let Some(ws) = ws else { return Ok(()) };
        let item = o.get("items").filter(|i| !i.is_null()).ok_or_else(|| type_error("data"))?.get("data").and_then(|d| d.get(0)).filter(|x| !x.is_null()).cloned();
        let status = if ty == "customer.subscription.deleted" {
            "canceled".to_owned()
        } else {
            match o.get("status").and_then(Value::as_str) {
                Some(s @ ("active" | "trialing" | "past_due" | "canceled")) => s.to_owned(),
                _ => "incomplete".to_owned(),
            }
        };
        let period_end = item.as_ref().and_then(|i| i.get("current_period_end")).filter(|v| !v.is_null()).or_else(|| o.get("current_period_end")).and_then(Value::as_f64).filter(|f| *f != 0.0);
        let to_time = |secs: f64| chrono::DateTime::from_timestamp_millis((secs * 1000.0) as i64);
        let plan = if status == "canceled" { "free".to_owned() } else { pg_text(meta.get("plan")).unwrap_or_else(|| "pro".into()) };
        let seats = item.as_ref().and_then(|i| pg_text(i.get("quantity"))).unwrap_or_else(|| "1".into());
        let interval = match &item {
            Some(i) => pg_text(i.get("price").ok_or_else(|| type_error("recurring"))?.get("recurring").ok_or_else(|| type_error("interval"))?.get("interval")).unwrap_or_else(|| "month".into()),
            None => "month".into(),
        };
        upsert_subscription(
            state,
            &SubscriptionUpsert {
                workspace_id: ws,
                plan,
                status,
                seats,
                interval,
                provider: "stripe".into(),
                customer_id: pg_text(o.get("customer")),
                subscription_id: pg_text(o.get("id")),
                period_end: period_end.and_then(to_time),
                cancel_at_period_end: o.get("cancel_at_period_end").and_then(Value::as_bool),
                trial_end: o.get("trial_end").and_then(Value::as_f64).filter(|f| *f != 0.0).and_then(to_time),
            },
        )
        .await?;
    } else if ty == "invoice.paid" || ty == "invoice.payment_failed" {
        let Some(ws) = customer_ws(pg_text(o.get("customer"))).await? else { return Ok(()) };
        let paid = ty == "invoice.paid";
        pg::exec(
            &state.pool,
            "INSERT INTO invoices(id, workspace_id, amount_cents, currency, status, url, period_start, period_end) VALUES ($1,$2::uuid,$3::int,$4,$5,$6, to_timestamp($7::float8), to_timestamp($8::float8))
       ON CONFLICT (id) DO UPDATE SET status = $5, url = $6",
            &[
                Arg::Text(pg_text(o.get("id"))),
                pg::text(ws.clone()),
                Arg::Text(pg_text(o.get(if paid { "amount_paid" } else { "amount_due" }))),
                Arg::Text(pg_text(o.get("currency"))),
                pg::text(if paid { "paid" } else { "failed" }),
                Arg::Text(pg_text(o.get("hosted_invoice_url"))),
                Arg::Text(pg_text(o.get("period_start"))),
                Arg::Text(pg_text(o.get("period_end"))),
            ],
        )
        .await?;
        if !paid {
            sqlx::query("UPDATE subscriptions SET status = 'past_due', updated_at = now() WHERE workspace_id = $1::uuid").bind(&ws).execute(&state.pool).await?;
        }
    }
    Ok(())
}
