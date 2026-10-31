//! Port of services/api/src/routes/print.ts — print orders API (A18): catalogue, quotes, preflight, saved shipping
//! addresses, orders with checkout through the billing provider, fulfiller webhooks (Gelato, Printful), signed
//! print-file links and the staff console. The helpers live in [`crate::print`].
//!
//! Also registers the Stripe event hook (`stripeEventHooks.push(onStripeEvent)`) and, through
//! [`crate::print::fulfil`], the `print-fulfil` worker.

use axum::extract::{FromRequest, MatchedPath, Request};
use axum::response::{IntoResponse, Response};
use oc_schema::print::{Destination, PrintError, PrintSelection, find_product, preflight_print, preflight_target_for, quote_print, validate_address};
use serde_json::Map;

use crate::auth::{at_least, design_role};
use crate::prelude::*;
use crate::print::catalog::print_catalog;
use crate::print::gelato::{parse_gelato_event, verify_gelato_webhook};
use crate::print::payment::{PayError, confirm_local_payment, payment_provider_id, refund, start_payment};
use crate::print::printful::{parse_printful_event, verify_printful_signature};
use crate::print::providers::enabled_providers;
use crate::print::states::{ORDER_STATUSES, customer_can_cancel, staff_choices};
use crate::print::store::{Order, OrderExt, Patch, StatusChange, StatusError, add_note, customer_view, get_order, get_order_by_number, order_number, set_status, verify_file_signature};
use crate::print::updates::apply_provider_update;
use crate::routes::designs::load_doc;

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    crate::billing::register_stripe_event_hook(crate::print::payment::on_stripe_event);
    Router::new()
        .route("/api/print/catalog", get(catalog))
        .route("/api/print/webhooks/gelato", post(gelato_webhook))
        .route("/api/print/webhooks/printful", post(printful_webhook))
        .route("/api/print/files/{file}", get(signed_file))
        .route("/api/print/quote", post(quote))
        .route("/api/print/preflight", post(preflight))
        .route("/api/print/addresses", get(addresses).post(address_create))
        .route("/api/print/addresses/{id}", put(address_update).delete(address_delete))
        .route("/api/print/orders", post(order_create).get(orders))
        .route("/api/print/orders/{id}", get(order))
        .route("/api/print/orders/{id}/pay", post(pay))
        .route("/api/print/orders/{id}/local-pay", post(local_pay))
        .route("/api/print/orders/{id}/cancel", post(cancel))
        .route("/api/print/orders/{id}/file", get(order_file))
        .route("/api/admin/print/orders", get(admin_orders))
        .route("/api/admin/print/orders/{id}", get(admin_order))
        .route("/api/admin/print/orders/{id}/file", get(admin_order_file))
        .route("/api/admin/print/orders/{id}/status", post(admin_status))
        .route("/api/admin/print/orders/{id}/retry", post(admin_retry))
}

fn not_found() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

fn print_error(e: PrintError) -> Reply {
    reply::json(400, json!({ "error": e.code, "message": e.message }))
}

fn status_error(e: StatusError) -> ApiResult {
    match e {
        StatusError::Transition(t) => Ok(reply::json(409, json!({ "error": "bad_transition", "message": t.to_string() }))),
        StatusError::Api(a) => Err(a),
    }
}

fn into_api(e: StatusError) -> ApiError {
    match e {
        StatusError::Api(a) => a,
        other => ApiError::internal(other.to_string()),
    }
}

fn pay_error(e: PayError) -> Result<String, ApiError> {
    match e {
        PayError::Api(a) => Err(a),
        PayError::Unavailable(m) => Err(ApiError::internal(format!("unavailable: {m}"))),
    }
}

/// The selection fields shared by quotes and orders (`zSelection`).
fn z_selection() -> [(&'static str, z::Schema); 4] {
    [
        ("productId", z::str().max(80.0)),
        ("sizeId", z::str().max(80.0)),
        ("options", z::record(z::str().max(80.0)).default(json!({}))),
        ("quantity", z::num().int().min(1.0).max(100000.0)),
    ]
}

fn selection(b: &Value) -> PrintSelection {
    PrintSelection {
        product_id: b["productId"].as_str().unwrap_or("").to_owned(),
        size_id: b["sizeId"].as_str().unwrap_or("").to_owned(),
        options: options_of(&b["options"]),
        quantity: b["quantity"].as_f64().unwrap_or(0.0),
    }
}

fn options_of<T: FromIterator<(String, String)>>(v: &Value) -> T {
    v.as_object().into_iter().flatten().map(|(k, x)| (k.clone(), x.as_str().unwrap_or("").to_owned())).collect()
}

fn str_or_none(b: &Value, k: &str) -> Option<String> {
    b.get(k).and_then(Value::as_str).map(str::to_owned)
}

async fn design_doc(s: &AppState, id: &str) -> Result<Option<oc_schema::DesignDoc>, ApiError> {
    match load_doc(s, id).await? {
        Some(v) => Ok(Some(serde_json::from_value(v)?)),
        None => Ok(None),
    }
}

async fn catalog(State(s): State<AppState>, _req: Req) -> ApiResult {
    let c = print_catalog(&s)?;
    // The catalogue without fulfiller mappings (those are operator configuration).
    let mut public = serde_json::to_value(&*c)?;
    for p in public["products"].as_array_mut().into_iter().flatten() {
        if let Some(o) = p.as_object_mut() {
            o.shift_remove("fulfilment");
        }
    }
    Ok(Reply::json(json!({ "catalog": public, "currency": c.currency, "payment": payment_provider_id(&s), "fulfilment": enabled_providers(&s) })))
}

// ---------------------------------------------------------------- fulfiller → us

/// The request with the rate limit skipped (`config: { rateLimit: false }`) and the raw JSON text (`rawBody`, set
/// by main.ts's JSON parser only).
async fn unlimited(s: &AppState, raw: Request) -> Result<(Req, Option<String>), ApiError> {
    let (mut parts, body) = raw.into_parts();
    parts.extensions.remove::<MatchedPath>();
    let bytes = axum::body::to_bytes(body, crate::http::req::BODY_LIMIT + 1).await.unwrap_or_default();
    let is_json = parts.headers.get("content-type").and_then(|v| v.to_str().ok()).and_then(crate::http::req::media_type).as_deref() == Some("application/json");
    let raw_text = is_json.then(|| String::from_utf8_lossy(&bytes).into_owned());
    let req = Req::from_request(Request::from_parts(parts, axum::body::Body::from(bytes)), s).await?;
    Ok((req, raw_text))
}

fn respond(r: ApiResult) -> Response {
    match r {
        Ok(r) => r.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn gelato_webhook(State(s): State<AppState>, raw: Request) -> Response {
    respond(gelato_hook(&s, raw).await)
}

async fn gelato_hook(s: &AppState, raw: Request) -> ApiResult {
    let (req, raw_text) = unlimited(s, raw).await?;
    if !verify_gelato_webhook(s, |name| req.header(name).map(str::to_owned)) {
        return Ok(reply::json(401, json!({ "error": "bad_signature" })));
    }
    let Some(body) = req.body().cloned().or_else(|| req.body_input()) else { return Err(ApiError::internal("TypeError: Cannot read properties of undefined (reading 'event')")) };
    let raw = raw_text.unwrap_or_else(|| crate::http::jsjson::stringify(&body));
    let result = match parse_gelato_event(&body, &raw) {
        Some(u) => apply_provider_update(s, "gelato", &u).await.map_err(into_api)?,
        None => "ignored",
    };
    Ok(Reply::json(json!({ "received": true, "result": result })))
}

async fn printful_webhook(State(s): State<AppState>, raw: Request) -> Response {
    respond(printful_hook(&s, raw).await)
}

async fn printful_hook(s: &AppState, raw: Request) -> ApiResult {
    let (req, raw_text) = unlimited(s, raw).await?;
    let raw = raw_text.unwrap_or_default();
    let secret = s.env("OC_PRINTFUL_WEBHOOK_SECRET").unwrap_or_default();
    if !verify_printful_signature(&raw, req.header("x-pf-webhook-signature").unwrap_or(""), &secret) {
        return Ok(reply::json(401, json!({ "error": "bad_signature" })));
    }
    let evt = crate::http::jsjson::parse(&raw).map_err(ApiError::internal)?;
    let result = match parse_printful_event(&evt) {
        Some(u) => apply_provider_update(s, "printful", &u).await.map_err(into_api)?,
        None => "ignored",
    };
    Ok(Reply::json(json!({ "received": true, "result": result })))
}

/// The print file as an attachment (`sendFile`).
async fn send_file(s: &AppState, o: &Order) -> ApiResult {
    let ty = o.str_of("file_type").unwrap_or("application/pdf").to_owned();
    let ext = if ty == "image/png" { "png" } else { "pdf" };
    let stream = s.store.stream(o.str_of("file_sha").unwrap_or(""), None).await?;
    Ok(Reply::bytes(&ty, axum::body::Body::from_stream(crate::routes::assets::tokio_util_stream(stream)))
        .header("content-disposition", format!("attachment; filename=\"{}.{ext}\"", o.text("number")))
        .header("cache-control", "private, max-age=300"))
}

fn has_file(o: &Order) -> bool {
    o.str_of("file_sha").is_some_and(|x| !x.is_empty())
}

/// Signed print-file link (fulfillers download it; the operator email links here).
async fn signed_file(State(s): State<AppState>, Path(file): Path<String>, req: Req) -> ApiResult {
    static EXT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\.(pdf|png)$").expect("regex"));
    let id = EXT.replace(&file, "").into_owned();
    let exp = z::js_number(req.query.get("exp"));
    let sig = match req.query.get("sig") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(x)) => x.clone(),
        Some(Value::Array(a)) => a.iter().map(|x| x.as_str().unwrap_or("").to_owned()).collect::<Vec<_>>().join(","),
        Some(other) => crate::http::jsjson::stringify(other),
    };
    if !verify_file_signature(&s, &id, exp, &sig) {
        return Ok(reply::json(403, json!({ "error": "forbidden" })));
    }
    match get_order(&s, &id).await? {
        Some(o) if has_file(&o) => send_file(&s, &o).await,
        _ => Ok(not_found()),
    }
}

// ---------------------------------------------------------------- quotes and preflight

async fn quote(State(s): State<AppState>, req: Req) -> ApiResult {
    require_user(&s, &req).await?;
    let [a, b, c, d] = z_selection();
    let body = z::obj([a, b, c, d, ("shippingMethod", z::str().max(40.0).optional()), ("country", z::str().max(4.0).optional()), ("state", z::str().max(100.0).optional())]).parse(req.body())?;
    let cat = print_catalog(&s)?;
    let dest = Destination { country: str_or_none(&body, "country"), state: str_or_none(&body, "state") };
    match quote_print(&cat, &selection(&body), &dest, body.get("shippingMethod").and_then(Value::as_str), Some(&cat.currency)) {
        Ok(q) => Ok(Reply::json(json!({ "quote": q }))),
        Err(e) => Ok(print_error(e)),
    }
}

async fn preflight(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("designId", z::str().max(64.0)), ("productId", z::str().max(80.0)), ("sizeId", z::str().max(80.0)), ("options", z::record(z::str()).default(json!({})))]).parse(req.body())?;
    let design_id = b["designId"].as_str().unwrap_or("");
    if !at_least(design_role(&s, Some(u.id), design_id).await?, oc_auth::Role::View) {
        return Ok(not_found());
    }
    let cat = print_catalog(&s)?;
    let Some(p) = find_product(&cat, b["productId"].as_str().unwrap_or("")) else { return Ok(reply::json(400, json!({ "error": "unknown_product" }))) };
    let Some(doc) = design_doc(&s, design_id).await? else { return Ok(not_found()) };
    match preflight_target_for(p, b["sizeId"].as_str().unwrap_or(""), &options_of(&b["options"])) {
        Ok(t) => Ok(Reply::json(json!({ "preflight": preflight_print(&doc, &t, None) }))),
        Err(e) => Ok(print_error(e)),
    }
}

// ---------------------------------------------------------------- saved addresses

async fn addresses(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let rows = pg::json_rows(&s.pool, "SELECT id, address, is_default FROM print_addresses WHERE user_id = $1 ORDER BY is_default DESC, updated_at DESC", &[pg::uuid(u.id)]).await?;
    Ok(Reply::json(json!({ "addresses": rows, "countries": print_catalog(&s)?.shipping.countries })))
}

fn z_address() -> z::Schema {
    z::obj([
        ("name", z::str().max(200.0)),
        ("company", z::str().max(200.0).optional()),
        ("line1", z::str().max(200.0)),
        ("line2", z::str().max(200.0).optional()),
        ("city", z::str().max(200.0)),
        ("state", z::str().max(100.0).optional()),
        ("postalCode", z::str().max(40.0)),
        ("country", z::str().max(4.0)),
        ("phone", z::str().max(40.0).optional()),
        ("email", z::str().max(200.0).optional()),
    ])
}

/// `saveAddress(userId, id, raw, makeDefault)`: `Err(fields)` for an invalid address.
async fn save_address(s: &AppState, user_id: Uuid, id: Option<&str>, raw: Option<&Value>, make_default: bool) -> Result<Result<Option<Value>, Value>, ApiError> {
    let parsed = z_address().parse(raw)?;
    let cat = print_catalog(s)?;
    let v = validate_address(parsed.as_object().unwrap_or(&Map::new()), Some(&cat.shipping.countries));
    if !v.ok {
        return Ok(Err(serde_json::to_value(&v.errors)?));
    }
    let count: i32 = sqlx::query_scalar("SELECT count(*)::int AS n FROM print_addresses WHERE user_id = $1").bind(user_id).fetch_one(&s.pool).await?;
    if id.is_none() && count >= 20 {
        return Ok(Err(json!({ "name": "too_many" })));
    }
    let def = make_default || count == 0;
    if def {
        sqlx::query("UPDATE print_addresses SET is_default = false WHERE user_id = $1").bind(user_id).execute(&s.pool).await?;
    }
    let address = serde_json::to_value(&v.address)?;
    let row = match id {
        Some(id) => {
            pg::json_one(
                &s.pool,
                "UPDATE print_addresses SET address = $3, is_default = is_default OR $4, updated_at = now() WHERE id = $1::uuid AND user_id = $2 RETURNING id, address, is_default",
                &[pg::text(id), pg::uuid(user_id), pg::Arg::Json(Some(address)), pg::Arg::Bool(Some(def))],
            )
            .await?
        }
        None => {
            pg::json_one(
                &s.pool,
                "INSERT INTO print_addresses(id, user_id, address, is_default) VALUES ($1,$2,$3,$4) RETURNING id, address, is_default",
                &[pg::uuid(oc_db::uuid()), pg::uuid(user_id), pg::Arg::Json(Some(address)), pg::Arg::Bool(Some(def))],
            )
            .await?
        }
    };
    Ok(Ok(row))
}

fn address_body(req: &Req) -> Result<Value, ApiError> {
    z::obj([("address", z::any()), ("default", z::bool().optional())]).parse(req.body())
}

fn invalid_address(fields: Value) -> Reply {
    reply::json(400, json!({ "error": "invalid_address", "fields": fields }))
}

static UUIDISH: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^[0-9a-f-]{36}$").expect("regex"));

async fn address_create(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = address_body(&req)?;
    match save_address(&s, u.id, None, b.get("address"), b.get("default").and_then(Value::as_bool) == Some(true)).await? {
        Err(fields) => Ok(invalid_address(fields)),
        Ok(row) => Ok(Reply::json(json!({ "address": row }))),
    }
}

async fn address_update(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !UUIDISH.is_match(&id) {
        return Ok(not_found());
    }
    let b = address_body(&req)?;
    match save_address(&s, u.id, Some(&id), b.get("address"), b.get("default").and_then(Value::as_bool) == Some(true)).await? {
        Err(fields) => Ok(invalid_address(fields)),
        Ok(Some(row)) => Ok(Reply::json(json!({ "address": row }))),
        Ok(None) => Ok(not_found()),
    }
}

async fn address_delete(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if UUIDISH.is_match(&id) {
        pg::exec(&s.pool, "DELETE FROM print_addresses WHERE id = $1::uuid AND user_id = $2", &[pg::text(&id), pg::uuid(u.id)]).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- orders

fn int_arg(f: f64) -> Arg {
    pg::text(oc_js::number_to_string(f))
}

async fn order_create(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let [a, b2, c, d] = z_selection();
    let b = z::obj([("designId", z::str().max(64.0)), a, b2, c, d, ("shippingMethod", z::str().max(40.0)), ("addressId", z::str().uuid()), ("acknowledgeWarnings", z::bool().default(json!(false)))])
        .parse(req.body())?;
    let design_id = b["designId"].as_str().unwrap_or("").to_owned();
    if !at_least(design_role(&s, Some(u.id), &design_id).await?, oc_auth::Role::View) {
        return Ok(not_found());
    }
    if let Some(gate) = crate::brandhub::approval_gate(&s, u.id, &design_id).await? {
        return Ok(reply::json(403, gate));
    }
    let cat = print_catalog(&s)?;
    let sel = selection(&b);
    let Some(p) = find_product(&cat, &sel.product_id) else { return Ok(reply::json(400, json!({ "error": "unknown_product", "message": "Unknown product." }))) };
    let addr: Option<sqlx::types::Json<Value>> =
        sqlx::query_scalar("SELECT address FROM print_addresses WHERE id = $1::uuid AND user_id = $2").bind(b["addressId"].as_str()).bind(u.id).fetch_optional(&s.pool).await?;
    let Some(addr) = addr else { return Ok(reply::json(400, json!({ "error": "unknown_address", "message": "Choose a shipping address." }))) };
    let v = validate_address(addr.0.as_object().unwrap_or(&Map::new()), Some(&cat.shipping.countries));
    if !v.ok {
        return Ok(reply::json(400, json!({ "error": "invalid_address", "fields": v.errors, "message": "Fix the shipping address first." })));
    }
    let Some(doc) = design_doc(&s, &design_id).await? else { return Ok(not_found()) };
    let dest = Destination { country: Some(v.address.country.clone()), state: v.address.state.clone() };
    let shipping = b["shippingMethod"].as_str().unwrap_or("");
    let q = match quote_print(&cat, &sel, &dest, Some(shipping), Some(&cat.currency)) {
        Ok(q) => q,
        Err(e) => return Ok(print_error(e)),
    };
    let pf = match preflight_target_for(p, &sel.size_id, &sel.options) {
        Ok(t) => preflight_print(&doc, &t, None),
        Err(e) => return Ok(print_error(e)),
    };
    if !pf.ok {
        return Ok(reply::json(400, json!({ "error": "preflight_failed", "message": "Fix the problems found by the print check first.", "preflight": pf })));
    }
    if pf.counts.warning > 0 && b["acknowledgeWarnings"].as_bool() != Some(true) {
        return Ok(reply::json(409, json!({ "error": "preflight_warnings", "message": "Review the print check warnings.", "preflight": pf })));
    }
    let id = oc_db::uuid();
    let mut number = order_number();
    for _ in 0..5 {
        if get_order_by_number(&s, &number).await?.is_none() {
            break;
        }
        number = order_number();
    }
    let size_label = p.sizes.iter().find(|x| x.id == sel.size_id).map_or(sel.size_id.clone(), |x| x.label.clone());
    pg::exec(
        &s.pool,
        "INSERT INTO print_orders(id, number, user_id, email, design_id, design_title, product_id, product_name, size_id, options, quantity, address,
           shipping_method, currency, subtotal_cents, shipping_cents, tax_cents, total_cents, price, preflight)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11::int,$12,$13,$14,$15::int,$16::int,$17::int,$18::int,$19,$20)",
        &[
            pg::uuid(id),
            pg::text(&number),
            pg::uuid(u.id),
            pg::text(&u.email),
            pg::text(&design_id),
            pg::text(&doc.meta.title),
            pg::text(&p.id),
            pg::text(format!("{} — {size_label}", p.name)),
            pg::text(&sel.size_id),
            pg::Arg::Json(Some(b["options"].clone())),
            int_arg(sel.quantity),
            pg::Arg::Json(Some(serde_json::to_value(&v.address)?)),
            pg::text(&q.shipping_method),
            pg::text(&q.currency),
            int_arg(q.subtotal),
            int_arg(q.shipping),
            int_arg(q.tax),
            int_arg(q.total),
            pg::Arg::Json(Some(serde_json::to_value(&q)?)),
            pg::Arg::Json(Some(serde_json::to_value(&pf)?)),
        ],
    )
    .await?;
    let id = id.to_string();
    add_note(&s, &id, "pending_payment", "Order created.", "customer", Some(u.id)).await?;
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent {
            actor_id: Some(u.id),
            action: "print.order_created".into(),
            target: Some(id.clone()),
            meta: Some(json!({ "number": number, "product": p.id, "quantity": sel.quantity, "total": q.total })),
            ip: Some(req.ip.clone()),
            ..Default::default()
        },
        s.now(),
    )
    .await?;
    let o = get_order(&s, &id).await?.ok_or_else(|| ApiError::internal("order vanished"))?;
    match start_payment(&s, &o).await {
        Ok(url) => Ok(Reply::json(json!({ "order": customer_view(&o), "url": url }))),
        Err(PayError::Unavailable(m)) => Ok(reply::json(503, json!({ "error": "payment_unavailable", "message": m, "order": customer_view(&o) }))),
        Err(e) => pay_error(e).map(|_| Reply::empty(500)),
    }
}

async fn orders(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let rows = pg::json_rows(&s.pool, "SELECT * FROM print_orders WHERE user_id = $1 ORDER BY created_at DESC LIMIT 200", &[pg::uuid(u.id)]).await?;
    let list: Vec<Value> = rows.iter().filter_map(Value::as_object).map(customer_view).collect();
    Ok(Reply::json(json!({ "orders": list })))
}

async fn own_order(s: &AppState, user: &User, id: &str) -> Result<Option<Order>, ApiError> {
    let uid = user.id.to_string();
    Ok(get_order(s, id).await?.filter(|o| o.str_of("user_id") == Some(uid.as_str())))
}

async fn order(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(o) = own_order(&s, &u, &id).await? else { return Ok(not_found()) };
    let events = pg::json_rows(&s.pool, "SELECT status, note, source, created_at FROM print_order_events WHERE order_id = $1::uuid ORDER BY id", &[pg::text(o.id())]).await?;
    Ok(Reply::json(json!({ "order": customer_view(&o), "events": events, "canCancel": customer_can_cancel(&o.status()) })))
}

async fn pay(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(o) = own_order(&s, &u, &id).await? else { return Ok(not_found()) };
    if o.status() != "pending_payment" {
        return Ok(reply::json(409, json!({ "error": "already_paid" })));
    }
    match start_payment(&s, &o).await {
        Ok(url) => Ok(Reply::json(json!({ "url": url }))),
        Err(PayError::Unavailable(m)) => Ok(reply::json(503, json!({ "error": "payment_unavailable", "message": m }))),
        Err(e) => pay_error(e).map(|_| Reply::empty(500)),
    }
}

async fn local_pay(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("session", z::str().max(200.0))]).parse(req.body())?;
    if own_order(&s, &u, &id).await?.is_none() {
        return Ok(not_found());
    }
    match confirm_local_payment(&s, &id, b["session"].as_str().unwrap_or(""), u.id).await {
        Ok(true) => Ok(Reply::json(json!({ "ok": true }))),
        Ok(false) => Ok(reply::json(400, json!({ "error": "expired", "message": "This checkout expired. Open the order and pay again." }))),
        Err(e) => status_error(e),
    }
}

async fn cancel(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let Some(o) = own_order(&s, &u, &id).await? else { return Ok(not_found()) };
    let st = o.status();
    if !customer_can_cancel(&st) {
        return Ok(reply::json(409, json!({ "error": "too_late", "message": "This order is already being printed and can’t be cancelled." })));
    }
    let note = if st == "pending_payment" { "Cancelled before payment.".to_owned() } else { refund(&s, &o).await? };
    let ch = StatusChange { source: "customer", actor_id: Some(u.id), note: Some(note), patch: Patch { refunded: Some(st != "pending_payment"), ..Default::default() }, ..Default::default() };
    if let Err(e) = set_status(&s, &o.id(), "cancelled", ch).await {
        return status_error(e);
    }
    Ok(Reply::json(json!({ "ok": true })))
}

async fn order_file(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    match own_order(&s, &u, &id).await? {
        Some(o) if has_file(&o) => send_file(&s, &o).await,
        _ => Ok(not_found()),
    }
}

// ---------------------------------------------------------------- staff console

fn with_next(o: &Order) -> Value {
    let mut m = o.clone();
    m.insert("next".into(), json!(staff_choices(&o.status())));
    Value::Object(m)
}

/// `String(v)` of a query value as a template literal shows it.
fn query_text(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Array(a) => a.iter().map(query_text).collect::<Vec<_>>().join(","),
        other => crate::http::jsjson::stringify(other),
    }
}

async fn admin_orders(State(s): State<AppState>, req: Req) -> ApiResult {
    auth::require_staff(&s, &req).await?;
    let mut wh: Vec<String> = Vec::new();
    let mut args: Vec<Arg> = Vec::new();
    if let Some(st) = req.query_str("status").filter(|st| ORDER_STATUSES.contains(st)) {
        args.push(pg::text(st));
        wh.push(format!("status = ${}", args.len()));
    }
    if let Some(q) = req.query.get("q").map(query_text).filter(|q| !q.is_empty()) {
        args.push(pg::text(format!("%{q}%")));
        wh.push(format!("(number ILIKE ${n} OR email ILIKE ${n})", n = args.len()));
    }
    let where_sql = if wh.is_empty() { String::new() } else { format!("WHERE {}", wh.join(" AND ")) };
    let rows = pg::json_rows(&s.pool, &format!("SELECT * FROM print_orders {where_sql} ORDER BY created_at DESC LIMIT 200"), &args).await?;
    let counts: Vec<(String, i32)> = sqlx::query_as("SELECT status, count(*)::int AS n FROM print_orders GROUP BY status").fetch_all(&s.pool).await?;
    let counts: Map<String, Value> = counts.into_iter().map(|(k, n)| (k, json!(n))).collect();
    let list: Vec<Value> = rows.iter().filter_map(Value::as_object).map(with_next).collect();
    Ok(Reply::json(json!({ "orders": list, "counts": counts, "fulfilment": enabled_providers(&s) })))
}

async fn admin_order(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    auth::require_staff(&s, &req).await?;
    let Some(o) = get_order(&s, &id).await? else { return Ok(not_found()) };
    let events = pg::json_rows(
        &s.pool,
        "SELECT e.status, e.note, e.source, e.created_at, u.name AS actor FROM print_order_events e LEFT JOIN users u ON u.id = e.actor_id WHERE order_id = $1::uuid ORDER BY e.id",
        &[pg::text(o.id())],
    )
    .await?;
    Ok(Reply::json(json!({ "order": with_next(&o), "events": events })))
}

async fn admin_order_file(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    auth::require_staff(&s, &req).await?;
    match get_order(&s, &id).await? {
        Some(o) if has_file(&o) => send_file(&s, &o).await,
        _ => Ok(not_found()),
    }
}

async fn admin_status(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = auth::require_staff(&s, &req).await?;
    let b = z::obj([
        ("status", z::enumeration(&ORDER_STATUSES)),
        ("note", z::str().max(2000.0).optional()),
        ("tracking", z::obj([("carrier", z::str().max(80.0).optional()), ("code", z::str().max(120.0).optional()), ("url", z::str().url().max(500.0).optional())]).optional()),
        ("refund", z::bool().optional()),
    ])
    .parse(req.body())?;
    let Some(o) = get_order(&s, &id).await? else { return Ok(not_found()) };
    let to = b["status"].as_str().unwrap_or("").to_owned();
    let refund_on = b.get("refund").and_then(Value::as_bool) == Some(true);
    let from = o.status();
    let mut note = b.get("note").and_then(Value::as_str).unwrap_or("").to_owned();
    if to == "cancelled" && refund_on && from != "pending_payment" {
        let r = refund(&s, &o).await?;
        note = if note.is_empty() { r } else { format!("{note} {r}") };
    }
    let mut patch = Patch::default();
    if let Some(t) = b.get("tracking").filter(|t| t.get("code").and_then(Value::as_str).is_some_and(|c| !c.is_empty()) || t.get("url").and_then(Value::as_str).is_some_and(|c| !c.is_empty())) {
        patch.tracking = Some(vec![t.clone()]);
    }
    if to == "cancelled" && refund_on {
        patch.refunded = Some(true);
    }
    let row = match set_status(&s, &id, &to, StatusChange { source: "staff", actor_id: Some(u.id), note: Some(note), patch, ..Default::default() }).await {
        Ok(r) => r,
        Err(e) => return status_error(e),
    };
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: Some(u.id), action: "print.status_changed".into(), target: Some(id.clone()), meta: Some(json!({ "from": from, "to": to })), ip: Some(req.ip.clone()), ..Default::default() },
        s.now(),
    )
    .await?;
    Ok(Reply::json(json!({ "order": with_next(&row) })))
}

async fn admin_retry(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = auth::require_staff(&s, &req).await?;
    let Some(o) = get_order(&s, &id).await? else { return Ok(not_found()) };
    let st = o.status();
    if st != "failed" && st != "paid" {
        return Ok(reply::json(409, json!({ "error": "bad_state", "message": "Only paid or failed orders can be sent again." })));
    }
    // Enqueue first, record the note, then run (the TS worker picks the job up after the response).
    let owner = o.str_of("user_id").and_then(|x| Uuid::parse_str(x).ok());
    oc_jobs::enqueue(
        &s.pool,
        crate::print::fulfil::QUEUE,
        &json!({ "orderId": o.id() }),
        oc_jobs::EnqueueOpts { owner_id: owner, max_attempts: Some(3), label: Some(format!("Print order {}", o.text("number"))), ..Default::default() },
    )
    .await?;
    add_note(&s, &o.id(), &st, "Fulfilment retried by staff.", "staff", Some(u.id)).await?;
    crate::print::fulfil::kick(&s);
    Ok(Reply::json(json!({ "ok": true })))
}
