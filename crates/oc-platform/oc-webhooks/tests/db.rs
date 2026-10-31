//! Fan-out + delivery against a local HTTP endpoint, compared with the TS delivery in
//! tests/golden/platform/webhook_delivery.json. Skipped unless OC_DATABASE_URL is set (throwaway database).
use oc_db::testing::TestDb;
use oc_webhooks::*;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

/// Accept one HTTP request, reply with `status`, return (headers lowercased, body).
async fn one_request(listener: tokio::net::TcpListener, status: u16) -> (Vec<(String, String)>, String) {
    let (mut s, _) = listener.accept().await.unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = s.read(&mut tmp).await.unwrap();
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..i]).into_owned();
            let headers: Vec<(String, String)> = head.lines().skip(1).filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_owned())).collect();
            let len: usize = headers.iter().find(|(k, _)| k == "content-length").map(|(_, v)| v.parse().unwrap()).unwrap_or(0);
            while buf.len() < i + 4 + len {
                let n = s.read(&mut tmp).await.unwrap();
                buf.extend_from_slice(&tmp[..n]);
            }
            let body = String::from_utf8(buf[i + 4..i + 4 + len].to_vec()).unwrap();
            s.write_all(format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").as_bytes()).await.unwrap();
            return (headers, body);
        }
    }
}

fn header<'a>(h: &'a [(String, String)], k: &str) -> &'a str {
    h.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str()).unwrap_or("")
}

#[tokio::test]
async fn delivery_matches_ts() {
    let Some(t) = TestDb::create().await else {
        eprintln!("skipped: OC_DATABASE_URL not set");
        return;
    };
    let pool = &t.pool;
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/webhook_delivery.json");
    let g: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let user: Uuid = g["user"]["id"].as_str().unwrap().parse().unwrap();
    sqlx::query("INSERT INTO users(id, email, name, color) VALUES ($1, 'ann@example.com', 'Ann', '#f06')").bind(user).execute(pool).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hook: Uuid = g["hook"]["id"].as_str().unwrap().parse().unwrap();
    let secret = g["hook"]["secret"].as_str().unwrap();
    let events: Vec<String> = serde_json::from_value(g["hook"]["events"].clone()).unwrap();
    sqlx::query("INSERT INTO webhooks(id, user_id, url, events, secret) VALUES ($1,$2,$3,$4,$5)")
        .bind(hook)
        .bind(user)
        .bind(format!("http://localhost:{port}/hook"))
        .bind(&events)
        .bind(secret)
        .execute(pool)
        .await
        .unwrap();

    // 1) Rust emit: payload shape {id, event, created, data} and the job row.
    let n = emit_event(pool, user, WebhookEvent::DesignCreated, &g["data"]).await.unwrap();
    assert_eq!(n as u64, g["emitCount"].as_u64().unwrap());
    assert_eq!(emit_event(pool, user, WebhookEvent::FormResponse, &g["data"]).await.unwrap(), 0);
    let (did, payload): (Uuid, sqlx::types::Json<Value>) = sqlx::query_as("SELECT id, payload FROM webhook_deliveries").fetch_one(pool).await.unwrap();
    let ts_payload: Value = serde_json::from_str(g["payloadText"].as_str().unwrap()).unwrap();
    let keys = |v: &Value| {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(keys(&payload.0), keys(&ts_payload));
    assert_eq!(oc_js::canonical(&payload.0["data"]), oc_js::canonical(&ts_payload["data"]));
    assert_eq!(payload.0["id"].as_str().unwrap(), did.to_string());
    assert!(chrono::DateTime::parse_from_rfc3339(payload.0["created"].as_str().unwrap()).is_ok());
    let (jp, max_attempts): (sqlx::types::Json<Value>, i32) = sqlx::query_as("SELECT payload, max_attempts FROM jobs WHERE queue = 'webhook'").fetch_one(pool).await.unwrap();
    assert_eq!(jp.0["deliveryId"].as_str().unwrap(), did.to_string());
    assert_eq!(max_attempts as i64, g["job"]["max_attempts"].as_i64().unwrap());

    // 2) Rust delivery of the exact payload the TS run stored → same body, valid signature, same headers.
    let ts_id: Uuid = g["deliveryId"].as_str().unwrap().parse().unwrap();
    sqlx::query("INSERT INTO webhook_deliveries(id, webhook_id, event, payload) VALUES ($1,$2,'design.created',$3::jsonb)")
        .bind(ts_id)
        .bind(hook)
        .bind(g["payloadText"].as_str().unwrap())
        .execute(pool)
        .await
        .unwrap();
    let server = tokio::spawn(one_request(listener, 204));
    deliver_webhook(pool, ts_id).await.unwrap();
    let (h, body) = server.await.unwrap();
    assert_eq!(body, g["request"]["body"].as_str().unwrap());
    let sig = header(&h, "oc-signature");
    let ts: i64 = sig.strip_prefix("t=").unwrap().split(',').next().unwrap().parse().unwrap();
    assert_eq!(sig, sign_payload_at(secret, &body, ts));
    assert_eq!(header(&h, "user-agent"), g["request"]["userAgent"].as_str().unwrap());
    assert_eq!(header(&h, "content-type"), g["request"]["contentType"].as_str().unwrap());
    assert_eq!(header(&h, "oc-delivery"), g["request"]["delivery"].as_str().unwrap());
    let (status, attempts, code, error, delivered): (String, i32, Option<i32>, Option<String>, bool) =
        sqlx::query_as("SELECT status, attempts, response_code, error, delivered_at IS NOT NULL FROM webhook_deliveries WHERE id = $1").bind(ts_id).fetch_one(pool).await.unwrap();
    let a = &g["after"];
    assert_eq!(status, a["status"].as_str().unwrap());
    assert_eq!(attempts as i64, a["attempts"].as_i64().unwrap());
    assert_eq!(code.map(i64::from), a["response_code"].as_i64());
    assert_eq!(error, None);
    assert_eq!(delivered, a["delivered"].as_bool().unwrap());

    // 3) Failure path: HTTP 500 → error "HTTP 500", status pending, failures incremented, Err for retry.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let server = tokio::spawn(one_request(listener, 500));
    let err = deliver_webhook(pool, did).await.unwrap_err();
    server.await.unwrap();
    assert_eq!(err.to_string(), "HTTP 500");
    let (status, code, error): (String, Option<i32>, Option<String>) =
        sqlx::query_as("SELECT status, response_code, error FROM webhook_deliveries WHERE id = $1").bind(did).fetch_one(pool).await.unwrap();
    assert_eq!((status.as_str(), code, error.as_deref()), ("pending", Some(500), Some("HTTP 500")));
    let failures: i32 = sqlx::query_scalar("SELECT failures FROM webhooks WHERE id = $1").bind(hook).fetch_one(pool).await.unwrap();
    assert_eq!(failures, 1);
    // Sixth attempt marks the delivery failed; 50 consecutive failures switch the hook off.
    sqlx::query("UPDATE webhook_deliveries SET attempts = 5 WHERE id = $1").bind(did).execute(pool).await.unwrap();
    sqlx::query("UPDATE webhooks SET failures = 49 WHERE id = $1").bind(hook).execute(pool).await.unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let server = tokio::spawn(one_request(listener, 500));
    assert!(deliver_webhook(pool, did).await.is_err());
    server.await.unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM webhook_deliveries WHERE id = $1").bind(did).fetch_one(pool).await.unwrap();
    assert_eq!(status, "failed");
    let active: bool = sqlx::query_scalar("SELECT active FROM webhooks WHERE id = $1").bind(hook).fetch_one(pool).await.unwrap();
    assert!(!active);
    // Inactive hook: delivery is a no-op.
    deliver_webhook(pool, did).await.unwrap();
    t.drop_db().await;
}

/// emit_event only fans out for active owners (port of packages/webhooks/src/webhooks.db.test.ts): a disabled (e.g.
/// tombstoned) or deleted account's webhook stays silent even while the webhook row itself is still active.
#[tokio::test]
async fn emit_event_active_owners_only() {
    let Some(t) = TestDb::create().await else { return };
    let pool = &t.pool;
    let owner = async |name: &str| -> (Uuid, Uuid) {
        let (id, hook) = (Uuid::new_v4(), Uuid::new_v4());
        sqlx::query("INSERT INTO users(id, email, name, color) VALUES ($1, $2, $3, '#0c7a80')").bind(id).bind(format!("wh-{name}@example.test")).bind(name).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO webhooks(id, user_id, url, events, secret) VALUES ($1, $2, 'https://hooks.example.test/x', $3, 'secret')")
            .bind(hook)
            .bind(id)
            .bind(vec!["comment.created".to_owned()])
            .execute(pool)
            .await
            .unwrap();
        (id, hook)
    };
    let deliveries = async |hook: Uuid| -> i64 { sqlx::query_scalar("SELECT count(*) FROM webhook_deliveries WHERE webhook_id = $1").bind(hook).fetch_one(pool).await.unwrap() };
    let data = serde_json::json!({ "designId": "d1" });

    let (active, hook) = owner("active").await;
    assert_eq!(emit_event(pool, active, WebhookEvent::CommentCreated, &data).await.unwrap(), 1);
    assert_eq!(deliveries(hook).await, 1);
    assert_eq!(emit_event(pool, active, WebhookEvent::DesignCreated, &data).await.unwrap(), 0);

    let (disabled, hook) = owner("disabled").await;
    sqlx::query("UPDATE users SET disabled_at = now(), disabled_reason = 'supabase_squat' WHERE id = $1").bind(disabled).execute(pool).await.unwrap();
    let still_active: bool = sqlx::query_scalar("SELECT active FROM webhooks WHERE id = $1").bind(hook).fetch_one(pool).await.unwrap();
    assert!(still_active);
    assert_eq!(emit_event(pool, disabled, WebhookEvent::CommentCreated, &data).await.unwrap(), 0);
    assert_eq!(deliveries(hook).await, 0);

    let (deleted, hook) = owner("deleted").await;
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1").bind(deleted).execute(pool).await.unwrap();
    assert_eq!(emit_event(pool, deleted, WebhookEvent::CommentCreated, &data).await.unwrap(), 0);
    assert_eq!(deliveries(hook).await, 0);
    t.drop_db().await;
}
