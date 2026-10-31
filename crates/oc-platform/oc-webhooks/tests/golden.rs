//! Parity with tests/golden/platform/webhooks.json and webhook_delivery.json (a real TS delivery).
use oc_webhooks::*;
use serde_json::Value;

fn golden(name: &str) -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/");
    serde_json::from_str(&std::fs::read_to_string(format!("{p}{name}")).unwrap()).unwrap()
}

#[test]
fn signatures_and_events() {
    let g = golden("webhooks.json");
    let events: Vec<String> = serde_json::from_value(g["events"].clone()).unwrap();
    assert_eq!(events, WEBHOOK_EVENTS);
    for s in g["signatures"].as_array().unwrap() {
        assert_eq!(sign_payload_at(s["secret"].as_str().unwrap(), s["body"].as_str().unwrap(), s["t"].as_i64().unwrap()), s["signature"].as_str().unwrap());
    }
}

#[test]
fn delivery_body_and_signature_match_ts_worker() {
    let g = golden("webhook_delivery.json");
    let body = body_from_jsonb_text(g["payloadText"].as_str().unwrap()).unwrap();
    assert_eq!(body, g["request"]["body"].as_str().unwrap());
    let sig = g["request"]["signature"].as_str().unwrap();
    let t: i64 = sig.strip_prefix("t=").unwrap().split(',').next().unwrap().parse().unwrap();
    assert_eq!(t, g["deliveredAt"].as_i64().unwrap() / 1000);
    assert_eq!(sign_payload_at(g["hook"]["secret"].as_str().unwrap(), &body, t), sig);
    assert_eq!(g["request"]["userAgent"], USER_AGENT);
    assert_eq!(g["request"]["contentType"], "application/json");
    assert_eq!(g["request"]["delivery"], g["deliveryId"]);
    assert_eq!(g["job"]["max_attempts"].as_i64().unwrap(), MAX_ATTEMPTS as i64);
}
