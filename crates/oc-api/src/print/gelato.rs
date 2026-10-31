//! Port of services/api/src/print/gelato.ts — the Gelato adapter (Order API v4), enabled when `OC_GELATO_API_KEY`
//! is set. `POST https://order.gelatoapis.com/v4/orders` with `X-API-KEY`. Webhooks (`order_status_updated`) are not
//! signed by Gelato: an HTTP-header authorization is configured in the Gelato dashboard (header
//! `OC_GELATO_WEBHOOK_HEADER`, default `x-oc-webhook-secret`, value `OC_GELATO_WEBHOOK_SECRET`).
//!
//! Deviation: `OC_GELATO_ORDERS_URL` overrides the endpoint (the parity harness points it at its fake).

use oc_schema::print::PrintProduct;
use serde_json::{Value, json};
use sha2::Digest as _;

use crate::print::types::{FulfilmentOrder, ProviderUpdate, SubmitResult, split_name, tracking};
use crate::state::AppState;

pub const GELATO_ORDERS_URL: &str = "https://order.gelatoapis.com/v4/orders";

/// `gelatoBody(order, product, fileUrl)`.
pub fn gelato_body(s: &AppState, o: &FulfilmentOrder, p: &PrintProduct, file_url: &str) -> Result<Value, String> {
    let g = p.fulfilment.as_ref().and_then(|f| f.gelato.as_ref()).ok_or_else(|| format!("{} is not available from Gelato", p.id))?;
    let vars = [("size".to_owned(), o.size_id.clone()), ("orientation".to_owned(), o.orientation.to_owned())].into_iter().chain(o.options.iter().cloned()).collect();
    let product_uid = oc_schema::print::fill_template(&g.product_uid, &vars, Some(&g.tokens)).map_err(|e| e.message)?;
    let (first, last) = split_name(&o.address.name);
    let a = &o.address;
    let shipment = s
        .env(&format!("OC_GELATO_SHIPMENT_{}", o.shipping_method.to_uppercase()))
        .unwrap_or_else(|| match o.shipping_method.as_str() {
            "standard" => "normal".into(),
            "express" => "express".into(),
            _ => "normal".into(),
        });
    Ok(json!({
        "orderType": if s.env("OC_GELATO_ORDER_TYPE").as_deref() == Some("draft") { "draft" } else { "order" },
        "orderReferenceId": o.number,
        "customerReferenceId": o.user_id.clone().unwrap_or_else(|| o.email.clone()),
        "currency": o.currency.to_uppercase(),
        "items": [{ "itemReferenceId": format!("{}-1", o.number), "productUid": product_uid, "files": [{ "type": "default", "url": file_url }], "quantity": o.quantity }],
        "shipmentMethodUid": shipment,
        "shippingAddress": {
            "firstName": first, "lastName": last, "companyName": a.company.clone().unwrap_or_default(), "addressLine1": a.line1, "addressLine2": a.line2.clone().unwrap_or_default(),
            "city": a.city, "postCode": a.postal_code, "state": a.state.clone().unwrap_or_default(), "country": a.country, "email": a.email.clone().unwrap_or_else(|| o.email.clone()),
            "phone": a.phone.clone().unwrap_or_default(),
        },
        "metadata": [{ "key": "orderId", "value": o.id }],
    }))
}

/// `gelato.submit(order, product, fileUrl)`.
pub async fn submit(s: &AppState, o: &FulfilmentOrder, p: &PrintProduct, file_url: &str) -> Result<SubmitResult, String> {
    let body = gelato_body(s, o, p, file_url)?;
    let url = s.env_nonempty("OC_GELATO_ORDERS_URL").unwrap_or_else(|| GELATO_ORDERS_URL.into());
    let res = s
        .http
        .post(url)
        .header("content-type", "application/json")
        .header("x-api-key", s.env("OC_GELATO_API_KEY").unwrap_or_default())
        .body(crate::http::jsjson::stringify(&body))
        .send()
        .await
        .map_err(|_| "fetch failed".to_owned())?;
    let status = res.status();
    let j = res.text().await.ok().and_then(|t| crate::http::jsjson::parse(&t).ok()).unwrap_or_else(|| json!({}));
    let id = j.get("id").filter(|v| matches!(v, Value::String(x) if !x.is_empty()) || v.is_number());
    let Some(id) = id.filter(|_| status.is_success()) else {
        let msg = match j.get("message") {
            Some(Value::String(m)) => m.clone(),
            Some(v) if !v.is_null() => crate::http::jsjson::stringify(v),
            _ => status.as_u16().to_string(),
        };
        return Err(format!("gelato: {msg}"));
    };
    let id = match id {
        Value::String(x) => x.clone(),
        other => crate::http::jsjson::stringify(other),
    };
    Ok(SubmitResult { provider_order_id: Some(id), status: map_gelato_status(j.get("fulfillmentStatus").and_then(Value::as_str)).unwrap_or("submitted").into(), note: None })
}

pub fn map_gelato_status(s: Option<&str>) -> Option<&'static str> {
    Some(match s? {
        "created" | "passed" | "pending_approval" | "on_hold" | "draft" | "uploading" | "not_connected" => "submitted",
        "printed" | "in_production" => "in_production",
        "shipped" | "in_transit" => "shipped",
        "delivered" => "delivered",
        "returned" => "returned",
        "canceled" | "cancelled" => "cancelled",
        "failed" => "failed",
        _ => return None,
    })
}

/// `verifyGelatoWebhook(headers)`.
pub fn verify_gelato_webhook(s: &AppState, header: impl Fn(&str) -> Option<String>) -> bool {
    let Some(secret) = s.env_nonempty("OC_GELATO_WEBHOOK_SECRET") else { return false };
    let name = s.env("OC_GELATO_WEBHOOK_HEADER").unwrap_or_else(|| "x-oc-webhook-secret".into()).to_lowercase();
    let got = header(&name).unwrap_or_default();
    got.len() == secret.len() && subtle::ConstantTimeEq::ct_eq(got.as_bytes(), secret.as_bytes()).into()
}

fn str_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(crate::http::jsjson::stringify(other)),
    }
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        _ => true,
    }
}

/// `parseGelatoEvent(body, raw)`.
pub fn parse_gelato_event(body: &Value, raw: &str) -> Option<ProviderUpdate> {
    let event = body.get("event").and_then(Value::as_str);
    if event != Some("order_status_updated") && event != Some("order_item_tracking_code_updated") {
        return None;
    }
    let mut tr = Vec::new();
    for i in body.get("items").and_then(Value::as_array).into_iter().flatten() {
        for f in i.get("fulfillments").and_then(Value::as_array).into_iter().flatten() {
            if truthy(f.get("trackingCode")) || truthy(f.get("trackingUrl")) {
                tr.push(tracking(f.get("shipmentMethodName"), f.get("trackingCode"), f.get("trackingUrl")));
            }
        }
    }
    let id = match body.get("id") {
        None | Some(Value::Null) => hex::encode(sha2::Sha256::digest(raw.as_bytes()))[..32].to_owned(),
        Some(v) => str_of(v).unwrap_or_default(),
    };
    let fs = body.get("fulfillmentStatus").and_then(Value::as_str);
    Some(ProviderUpdate {
        event_id: format!("gelato:{id}"),
        order_number: body.get("orderReferenceId").and_then(str_of),
        provider_order_id: body.get("orderId").and_then(str_of),
        status: if event == Some("order_status_updated") { map_gelato_status(fs) } else { None },
        tracking: tr,
        note: format!("Gelato: {}", fs.or(event).unwrap_or("")),
    })
}
