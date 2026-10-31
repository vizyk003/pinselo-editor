//! Port of services/api/src/print/printful.ts — the Printful adapter (API v2), enabled when `OC_PRINTFUL_API_KEY` (a
//! private token) is set; `OC_PRINTFUL_STORE_ID` picks the store for account-level tokens. `POST /v2/orders`, then
//! `POST /v2/orders/{id}/confirmation` (skipped when `OC_PRINTFUL_CONFIRM=0` → stays a draft). Webhooks v2 are
//! signed: `x-pf-webhook-signature` = hex HMAC-SHA256 of the raw body keyed with the hex-decoded
//! `OC_PRINTFUL_WEBHOOK_SECRET`.
//!
//! Deviation: `OC_PRINTFUL_API_URL` overrides the API host (the parity harness points it at its fake).

use hmac::{Hmac, Mac};
use oc_schema::print::PrintProduct;
use serde_json::{Map, Value, json};
use sha2::Sha256;

use crate::print::types::{FulfilmentOrder, ProviderUpdate, SubmitResult, tracking};
use crate::state::AppState;

pub const PRINTFUL_API: &str = "https://api.printful.com";

fn money(cents: f64) -> String {
    crate::print::store::js_to_fixed(cents / 100.0, 2)
}

/// `printfulVariant(product, sizeId, options)`.
pub fn printful_variant(p: &PrintProduct, size_id: &str, options: &[(String, String)]) -> Result<f64, String> {
    let m = p.fulfilment.as_ref().and_then(|f| f.printful.as_ref()).ok_or_else(|| format!("{} is not available from Printful", p.id))?;
    let vars = std::iter::once(("size".to_owned(), size_id.to_owned())).chain(options.iter().cloned()).collect();
    let key = oc_schema::print::fill_template(&m.variant_key, &vars, None).map_err(|e| e.message)?;
    m.variants.get(&key).copied().filter(|v| *v != 0.0).ok_or_else(|| format!("{}: no Printful variant for {key}", p.id))
}

fn put_opt(m: &mut Map<String, Value>, k: &str, v: Option<&String>) {
    if let Some(v) = v {
        m.insert(k.into(), json!(v));
    }
}

/// `printfulBody(order, product, fileUrl)`.
pub fn printful_body(o: &FulfilmentOrder, p: &PrintProduct, file_url: &str) -> Result<Value, String> {
    let m = p.fulfilment.as_ref().and_then(|f| f.printful.as_ref()).ok_or_else(|| "Cannot read properties of undefined (reading 'printful')".to_owned())?;
    let a = &o.address;
    let mut recipient = Map::new();
    recipient.insert("name".into(), json!(a.name));
    put_opt(&mut recipient, "company", a.company.as_ref());
    recipient.insert("address1".into(), json!(a.line1));
    put_opt(&mut recipient, "address2", a.line2.as_ref());
    recipient.insert("city".into(), json!(a.city));
    put_opt(&mut recipient, "state_code", a.state.as_ref());
    recipient.insert("country_code".into(), json!(a.country));
    recipient.insert("zip".into(), json!(a.postal_code));
    put_opt(&mut recipient, "phone", a.phone.as_ref());
    recipient.insert("email".into(), json!(a.email.clone().unwrap_or_else(|| o.email.clone())));
    let variant = printful_variant(p, &o.size_id, &o.options)?;
    Ok(json!({
        "external_id": o.number,
        "shipping": o.shipping_method.to_uppercase(),
        "recipient": recipient,
        "order_items": [{
            "source": "catalog", "catalog_variant_id": variant, "quantity": o.quantity, "external_id": format!("{}-1", o.number),
            "placements": [{ "placement": m.placement, "technique": m.technique, "layers": [{ "type": "file", "url": file_url }] }],
        }],
        "retail_costs": { "currency": o.currency.to_uppercase(), "shipping": money(o.shipping_cents), "tax": money(o.tax_cents) },
    }))
}

async fn call(s: &AppState, path: &str, body: Option<&Value>) -> Result<Value, String> {
    let base = s.env_nonempty("OC_PRINTFUL_API_URL").unwrap_or_else(|| PRINTFUL_API.into());
    let mut rq = s
        .http
        .post(format!("{base}{path}"))
        .header("authorization", format!("Bearer {}", s.env("OC_PRINTFUL_API_KEY").unwrap_or_default()))
        .header("content-type", "application/json");
    if let Some(store) = s.env_nonempty("OC_PRINTFUL_STORE_ID") {
        rq = rq.header("x-pf-store-id", store);
    }
    if let Some(b) = body {
        rq = rq.body(crate::http::jsjson::stringify(b));
    }
    let res = rq.send().await.map_err(|_| "fetch failed".to_owned())?;
    let status = res.status();
    let j = res.text().await.ok().and_then(|t| crate::http::jsjson::parse(&t).ok()).unwrap_or_else(|| json!({}));
    if !status.is_success() {
        let msg = j
            .pointer("/error/message")
            .filter(|v| !v.is_null())
            .or_else(|| j.get("detail").filter(|v| !v.is_null()))
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => crate::http::jsjson::stringify(other),
            })
            .unwrap_or_else(|| status.as_u16().to_string());
        return Err(format!("printful: {msg}"));
    }
    Ok(j)
}

fn data_id(j: &Value) -> Result<String, String> {
    match j.pointer("/data/id") {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Number(n)) => Ok(crate::http::jsjson::number_text(n)),
        Some(Value::Null) => Ok("null".into()),
        Some(other) => Ok(crate::http::jsjson::stringify(other)),
        None if j.get("data").is_some_and(|d| !d.is_null()) => Ok("undefined".into()),
        None => Err("Cannot read properties of undefined (reading 'id')".into()),
    }
}

/// `printful.submit(order, product, fileUrl)`.
pub async fn submit(s: &AppState, o: &FulfilmentOrder, p: &PrintProduct, file_url: &str) -> Result<SubmitResult, String> {
    let body = printful_body(o, p, file_url)?;
    let created = call(s, "/v2/orders", Some(&body)).await?;
    let id = data_id(&created)?;
    if s.env("OC_PRINTFUL_CONFIRM").as_deref() != Some("0") {
        let c = call(s, &format!("/v2/orders/{id}/confirmation"), None).await?;
        let st = c.pointer("/data/status").and_then(Value::as_str);
        if c.get("data").is_none_or(Value::is_null) {
            return Err("Cannot read properties of undefined (reading 'status')".into());
        }
        return Ok(SubmitResult { provider_order_id: Some(id), status: map_printful_status(st).unwrap_or("submitted").into(), note: None });
    }
    Ok(SubmitResult { provider_order_id: Some(id), status: "submitted".into(), note: Some("Created as a draft in Printful (OC_PRINTFUL_CONFIRM=0).".into()) })
}

pub fn map_printful_status(s: Option<&str>) -> Option<&'static str> {
    Some(match s? {
        "draft" | "pending" | "inreview" | "onhold" => "submitted",
        "inprocess" | "partial" => "in_production",
        "fulfilled" => "shipped",
        "canceled" => "cancelled",
        "failed" => "failed",
        _ => return None,
    })
}

/// `verifyPrintfulSignature(raw, header, secretHex)`.
pub fn verify_printful_signature(raw: &str, header: &str, secret_hex: &str) -> bool {
    if header.is_empty() || secret_hex.is_empty() || !secret_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    // Buffer.from(secretHex, 'hex') drops a trailing odd digit.
    let even = &secret_hex[..secret_hex.len() - secret_hex.len() % 2];
    let Ok(key) = hex::decode(even) else { return false };
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("hmac key");
    mac.update(raw.as_bytes());
    let expected = mac.finalize().into_bytes();
    let h = crate::js::trim(header);
    let mut got = Vec::new();
    let b = h.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        let (Some(x), Some(y)) = ((b[i] as char).to_digit(16), (b[i + 1] as char).to_digit(16)) else { break };
        got.push((x * 16 + y) as u8);
        i += 2;
    }
    got.len() == expected.len() && subtle::ConstantTimeEq::ct_eq(got.as_slice(), expected.as_slice()).into()
}

fn js_str(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => crate::http::jsjson::number_text(n),
        Some(other) => crate::http::jsjson::stringify(other),
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

/// `parsePrintfulEvent(event)`.
pub fn parse_printful_event(e: &Value) -> Option<ProviderUpdate> {
    let order = e.pointer("/data/order").filter(|o| truthy(Some(o)))?;
    let ty = e.get("type").filter(|t| truthy(Some(t)))?;
    let ty = js_str(Some(ty));
    let sh = e.pointer("/data/shipment").filter(|x| truthy(Some(x)));
    let tr = match sh {
        Some(sh) if truthy(sh.get("tracking_number")) || truthy(sh.get("tracking_url")) => vec![tracking(sh.get("carrier"), sh.get("tracking_number"), sh.get("tracking_url"))],
        _ => vec![],
    };
    let order_status = order.get("status").and_then(Value::as_str);
    let status = match ty.as_str() {
        "shipment_sent" => Some("shipped"),
        "shipment_delivered" => Some("delivered"),
        "shipment_returned" => Some("returned"),
        "order_failed" => Some("failed"),
        "order_canceled" => Some("cancelled"),
        "order_updated" | "order_created" => map_printful_status(order_status),
        _ => None,
    };
    let reason = e.pointer("/data/reason").filter(|r| truthy(Some(r)));
    Some(ProviderUpdate {
        event_id: format!("printful:{ty}:{}:{}:{}", js_str(order.get("id")), js_str(sh.and_then(|x| x.get("id"))), js_str(e.get("occurred_at"))),
        order_number: order.get("external_id").filter(|v| !v.is_null()).map(|v| js_str(Some(v))),
        provider_order_id: truthy(order.get("id")).then(|| js_str(order.get("id"))),
        status,
        tracking: tr,
        note: format!("Printful: {ty}{}", reason.map(|r| format!(" — {}", js_str(Some(r)))).unwrap_or_default()),
    })
}
