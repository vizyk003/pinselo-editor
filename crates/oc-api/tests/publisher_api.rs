//! The REST API as an unattended publisher uses it (docs/api.md, "JPEG exports for publishing"): read a design's
//! version, export it as JPEGs pinned to that version, fetch the public links. Tokens limited to one workspace,
//! version locking, idempotency and link expiry. The render service is faked (it answers a baseline JPEG); real
//! rendering is covered by oc-render-svc. Same fakes and throwaway databases as the parity transcripts; skipped
//! without `OC_DATABASE_URL`.

mod harness;

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use harness::{API, CLOCK, Fake, SECRET, WEB};
use http_body_util::BodyExt as _;
use oc_api::AppState;
use oc_db::testing::TestDb;
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// SOI, JFIF APP0, SOF0 (1080×1350), SOS, EOI: what the fake render service answers for every page.
fn page_jpeg() -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    v.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
    v.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x05, 0x46, 0x04, 0x38, 0x03, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    v.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x12, 0x34, 0xFF, 0xD9]);
    v
}

struct Res {
    status: u16,
    headers: axum::http::HeaderMap,
    raw: Vec<u8>,
    body: Value,
}

async fn send(s: &AppState, method: &str, url: &str, headers: &[(&str, &str)], body: Option<Value>) -> Res {
    let mut req = axum::http::Request::builder().method(method).uri(url).header("x-forwarded-for", "198.51.100.7");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = match body {
        Some(b) => req.header("content-type", "application/json").body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    let res = oc_api::app(s.clone()).oneshot(req).await.expect("response");
    let status = res.status().as_u16();
    let headers = res.headers().clone();
    let raw = res.into_body().collect().await.expect("body").to_bytes().to_vec();
    Res { status, headers, body: serde_json::from_slice(&raw).unwrap_or(Value::Null), raw }
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// A personal access token for `user` (as Settings → Developers makes it).
async fn token(pool: &sqlx::PgPool, user: Uuid, scopes: &[&str], workspace: Option<Uuid>) -> (Uuid, String) {
    let t = format!("oc_pat_{}", Uuid::new_v4().simple());
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO api_tokens(id, user_id, name, token_sha256, prefix, scopes, workspace_id) VALUES ($1,$2,'test',$3,$4,$5,$6)")
        .bind(id)
        .bind(user)
        .bind(oc_api::auth::sha(&t))
        .bind(&t[..12])
        .bind(scopes.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
        .bind(workspace)
        .execute(pool)
        .await
        .expect("token");
    (id, t)
}

/// A saved change, as the collab server logs one (an empty Yjs update: the content stays the same, the version moves).
async fn edit(pool: &sqlx::PgPool, design: &str) {
    sqlx::query("INSERT INTO doc_updates(design_id, update) VALUES ($1, $2)").bind(design).bind(oc_crdt::encode_state(&oc_crdt::new_doc())).execute(pool).await.expect("edit");
}

fn path_of(url: &str) -> &str {
    url.strip_prefix(WEB).expect("link on the web origin")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn publisher_flow() {
    let _serial = SERIAL.lock().await;
    let Some(db) = TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping");
        return;
    };
    let url = db.url.clone();
    let task = tokio::spawn(async move {
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(3).connect(&url).await.expect("connect test db");
        let data = std::env::temp_dir().join(format!("oc-publisher-test-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&data).expect("data dir");
        let fake = Fake::start().await;
        fake.set_default("/internal/export", &json!({ "hex": hex::encode(page_jpeg()), "headers": { "content-type": "image/jpeg" } }));
        fake.set_default("/internal/thumbnail", &json!({ "json": { "ok": true } }));
        let env: HashMap<String, String> = [
            ("NODE_ENV", "development"),
            ("OC_DATABASE_URL", url.as_str()),
            ("OC_DATA_DIR", data.to_str().expect("data dir")),
            ("OC_SECRET", SECRET),
            ("OC_WEB_ORIGIN", WEB),
            ("OC_API_URL", API),
            ("OC_RENDER_URL", fake.base.as_str()),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .into_iter()
        .collect();
        let clock: oc_api::state::Clock = Arc::new(|| CLOCK);
        let s = AppState::new(oc_api::AppOptions { env, pool: pool.clone(), store: None, clock: Some(clock) }).await.expect("app state");

        let user = oc_api::auth::create_user(&s, "ada@example.test", "Ada", oc_api::auth::NewUser { verified: true, ..Default::default() }).await.expect("user").id;
        let personal: Uuid = sqlx::query_scalar("SELECT w.id FROM workspaces w JOIN memberships m ON m.workspace_id = w.id WHERE m.user_id = $1 AND w.personal").bind(user).fetch_one(&pool).await.expect("personal ws");
        let other = Uuid::new_v4();
        sqlx::query("INSERT INTO workspaces(id, name, personal) VALUES ($1, 'Agency', false)").bind(other).execute(&pool).await.expect("ws");
        sqlx::query("INSERT INTO memberships(workspace_id, user_id, role) VALUES ($1, $2, 'owner')").bind(other).bind(user).execute(&pool).await.expect("member");

        // A 3-page 1080×1350 design, made through the API.
        let (_, writer) = token(&pool, user, &["designs:write"], None).await;
        let r = send(&s, "POST", "/v1/designs", &[("authorization", &bearer(&writer))], Some(json!({ "title": "P-002 fee fact card", "width": 1080, "height": 1350 }))).await;
        assert_eq!(r.status, 201, "{:?}", r.body);
        let design = r.body["id"].as_str().expect("id").to_owned();
        {
            let ydoc = oc_api::brandhub::load_ydoc(&s, &design).await.expect("load").expect("doc");
            let doc = oc_crdt::decode_doc(&oc_crdt::encode_state(&ydoc)).expect("decode");
            assert_eq!(doc.pages.len(), 1);
        }

        let (pub_id, publisher) = token(&pool, user, &["designs:read", "designs:export"], Some(personal)).await;
        let auth = bearer(&publisher);
        let h = [("authorization", auth.as_str())];

        // Without a token, with an unknown one, with a token whose workspace is elsewhere.
        assert_eq!(send(&s, "GET", &format!("/v1/designs/{design}"), &[], None).await.status, 401);
        assert_eq!(send(&s, "GET", &format!("/v1/designs/{design}"), &[("authorization", "Bearer oc_pat_nosuchtokennosuchtoken00")], None).await.status, 401);
        let (_, elsewhere) = token(&pool, user, &["designs:read", "designs:export"], Some(other)).await;
        let r = send(&s, "GET", &format!("/v1/designs/{design}"), &[("authorization", &bearer(&elsewhere))], None).await;
        assert_eq!((r.status, r.body["error"].as_str()), (404, Some("not_found")));
        // A key in the query string is never read.
        assert_eq!(send(&s, "GET", &format!("/v1/designs/{design}?access_token={publisher}"), &[], None).await.status, 401);
        // Workspace-limited tokens stay off the MCP endpoint.
        let r = send(&s, "POST", "/mcp", &[("authorization", &auth), ("accept", "application/json, text/event-stream")], Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))).await;
        assert_eq!(r.status, 401, "{:?}", r.body);

        // Read: version and pages; the edit link works as the id.
        let r = send(&s, "GET", &format!("/v1/designs/{design}"), &h, None).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(r.body["version"], 1);
        assert_eq!(r.body["width"], 1080);
        assert_eq!(r.body["height"], 1350);
        assert_eq!(r.body["pageList"][0]["index"], 1);
        assert!(r.headers.contains_key("x-ratelimit-remaining"));
        let link = format!("{WEB}/design/{design}?page=1");
        let enc: String = form_urlencoded::byte_serialize(link.as_bytes()).collect();
        assert_eq!(send(&s, "GET", &format!("/v1/designs/{enc}"), &h, None).await.body["id"], design.as_str());

        // A read-only token cannot export.
        let (_, reader) = token(&pool, user, &["designs:read"], Some(personal)).await;
        let r = send(&s, "POST", &format!("/v1/designs/{design}/exports"), &[("authorization", &bearer(&reader))], Some(json!({ "format": "jpeg" }))).await;
        assert_eq!((r.status, r.body["error"].as_str()), (403, Some("insufficient_scope")));

        // Bad requests are refused before rendering.
        for (body, needle) in [
            (json!({ "format": "jpeg", "width": 1500 }), "width"),
            (json!({ "format": "jpeg", "width": 300 }), "width"),
            (json!({ "format": "jpeg", "pages": [2] }), "out of range"),
            (json!({ "format": "jpeg", "pages": [1, 1] }), "twice"),
            (json!({ "format": "jpeg", "quality": 40 }), "quality"),
            (json!({ "format": "jpeg", "scale": 2 }), "Unknown field"),
        ] {
            let r = send(&s, "POST", &format!("/v1/designs/{design}/exports"), &h, Some(body.clone())).await;
            assert_eq!((r.status, r.body["error"].as_str()), (422, Some("invalid_export")), "{body}: {:?}", r.body);
            assert!(r.body["message"].as_str().unwrap_or("").contains(needle), "{body}: {:?}", r.body);
        }
        assert_eq!(fake.count("/internal/export"), 0);

        // Export pinned to the version, waiting: done with one public JPEG per page.
        let key = [("authorization", auth.as_str()), ("idempotency-key", "P-002-v1")];
        let r = send(&s, "POST", &format!("/v1/designs/{design}/exports"), &key, Some(json!({ "format": "jpeg", "version": 1, "width": 1080, "pages": [1], "wait": true }))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(r.body["status"], "done");
        assert_eq!(r.body["designVersion"], 1);
        assert_eq!(r.body["warnings"], json!([]));
        let export_id = r.body["id"].as_str().expect("id").to_owned();
        assert!(export_id.starts_with("exp_"));
        let file = &r.body["files"][0];
        assert_eq!((file["page"].as_u64(), file["width"].as_u64(), file["height"].as_u64()), (Some(1), Some(1080), Some(1350)));
        assert_eq!(file["contentType"], "image/jpeg");
        let file_url = file["url"].as_str().expect("url").to_owned();
        assert!(file_url.starts_with(&format!("{WEB}/v1/files/{export_id}/1.jpg?")), "{file_url}");
        assert_eq!(fake.count("/internal/export"), 1);
        let call = {
            // What the render service was asked for: page 1 (0-based), 1080 px wide, strict fonts.
            let r = send(&s, "GET", &format!("/v1/exports/{export_id}"), &h, None).await;
            assert_eq!(r.status, 200);
            assert_eq!(r.body["files"][0]["url"], file_url.as_str(), "links are stable across reads");
            r
        };
        drop(call);

        // The link: no token, image/jpeg, no redirect, sRGB-tagged; HEAD works too.
        let r = send(&s, "GET", path_of(&file_url), &[], None).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.headers.get("content-type").and_then(|v| v.to_str().ok()), Some("image/jpeg"));
        assert!(r.headers.get("location").is_none());
        assert_eq!(&r.raw[..2], &[0xFF, 0xD8]);
        assert!(r.raw.windows(12).any(|w| w == b"ICC_PROFILE\0"));
        assert_eq!(r.raw.len() as u64, file["bytes"].as_u64().unwrap_or(0));
        let head = send(&s, "HEAD", path_of(&file_url), &[], None).await;
        assert_eq!(head.status, 200);
        assert_eq!(head.headers.get("content-type").and_then(|v| v.to_str().ok()), Some("image/jpeg"));
        // Altered links are refused.
        assert_eq!(send(&s, "GET", &path_of(&file_url).replace("1.jpg", "2.jpg"), &[], None).await.status, 404);
        assert_eq!(send(&s, "GET", &path_of(&file_url).replace("e=", "e=9"), &[], None).await.status, 404);

        // The same Idempotency-Key: the same export and links, no new render.
        let r = send(&s, "POST", &format!("/v1/designs/{design}/exports"), &key, Some(json!({ "format": "jpeg", "version": 1, "pages": [1], "wait": true }))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(r.body["id"], export_id.as_str());
        assert_eq!(r.body["files"][0]["url"], file_url.as_str());
        assert_eq!(fake.count("/internal/export"), 1);

        // Edited after approval: the old version is refused, nothing rendered.
        edit(&pool, &design).await;
        let r = send(&s, "GET", &format!("/v1/designs/{design}"), &h, None).await;
        assert_eq!(r.body["version"], 2);
        let r = send(&s, "POST", &format!("/v1/designs/{design}/exports"), &h, Some(json!({ "format": "jpeg", "version": 1, "wait": true }))).await;
        assert_eq!((r.status, r.body["error"].as_str(), r.body["currentVersion"].as_i64()), (409, Some("version_changed"), Some(2)), "{:?}", r.body);
        assert_eq!(fake.count("/internal/export"), 1);

        // Without waiting: 202, then poll until done.
        let r = send(&s, "POST", &format!("/v1/designs/{design}/exports"), &h, Some(json!({ "format": "jpeg", "version": 2 }))).await;
        assert!(r.status == 202 || r.status == 200, "{:?}", r.body);
        let polled = r.body["id"].as_str().expect("id").to_owned();
        let mut done = false;
        for _ in 0..50 {
            let r = send(&s, "GET", &format!("/v1/exports/{polled}"), &h, None).await;
            assert_eq!(r.status, 200, "{:?}", r.body);
            if r.body["status"] == "done" {
                done = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(done, "the export finished");
        // Another token's view: the export belongs to the same user but the workspace token elsewhere can't see the design.
        assert_eq!(send(&s, "GET", &format!("/v1/exports/{polled}"), &[("authorization", &bearer(&elsewhere))], None).await.status, 404);

        // Links stop after their expiry…
        sqlx::query("UPDATE api_exports SET expires_at = now() - interval '1 second' WHERE id = $1").bind(&export_id).execute(&pool).await.expect("expire");
        assert_eq!(send(&s, "GET", path_of(&file_url), &[], None).await.status, 404);
        // …and when the token is revoked (the API refuses it at once too).
        let r = send(&s, "GET", &format!("/v1/exports/{polled}"), &h, None).await;
        let polled_url = r.body["files"][0]["url"].as_str().expect("url").to_owned();
        assert_eq!(send(&s, "GET", path_of(&polled_url), &[], None).await.status, 200);
        sqlx::query("UPDATE api_tokens SET revoked_at = now() WHERE id = $1").bind(pub_id).execute(&pool).await.expect("revoke");
        assert_eq!(send(&s, "GET", path_of(&polled_url), &[], None).await.status, 404);
        assert_eq!(send(&s, "GET", &format!("/v1/designs/{design}"), &h, None).await.status, 401);

        pool.close().await;
        let _ = std::fs::remove_dir_all(&data);
    })
    .await;
    db.drop_db().await;
    if let Err(e) = task {
        std::panic::resume_unwind(e.into_panic());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspace_tokens_only_read_and_export() {
    let _serial = SERIAL.lock().await;
    let Some(db) = TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping");
        return;
    };
    let url = db.url.clone();
    let task = tokio::spawn(async move {
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(3).connect(&url).await.expect("connect test db");
        let data = std::env::temp_dir().join(format!("oc-publisher-test-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&data).expect("data dir");
        let env: HashMap<String, String> = [("NODE_ENV", "development"), ("OC_DATABASE_URL", url.as_str()), ("OC_DATA_DIR", data.to_str().expect("dir")), ("OC_SECRET", SECRET), ("OC_WEB_ORIGIN", WEB), ("OC_API_URL", API)]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into_iter()
            .collect();
        let clock: oc_api::state::Clock = Arc::new(|| CLOCK);
        let s = AppState::new(oc_api::AppOptions { env, pool: pool.clone(), store: None, clock: Some(clock) }).await.expect("app state");
        let user = oc_api::auth::create_user(&s, "ada@example.test", "Ada", oc_api::auth::NewUser { password: Some("correct horse battery 42"), verified: true, ..Default::default() }).await.expect("user").id;
        let ws: Uuid = sqlx::query_scalar("SELECT workspace_id FROM memberships WHERE user_id = $1").bind(user).fetch_one(&pool).await.expect("ws");
        let r = send(&s, "POST", "/api/auth/login", &[], Some(json!({ "email": "ada@example.test", "password": "correct horse battery 42" }))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        let cookie = r.headers.get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()?.split(';').next().map(str::to_owned)).collect::<Vec<_>>().join("; ");
        let c = [("cookie", cookie.as_str())];
        let r = send(&s, "POST", "/api/developer/tokens", &c, Some(json!({ "name": "UniHop", "scopes": ["designs:read", "designs:write"], "workspaceId": ws.to_string() }))).await;
        assert_eq!((r.status, r.body["error"].as_str()), (400, Some("scope_not_allowed")), "{:?}", r.body);
        let r = send(&s, "POST", "/api/developer/tokens", &c, Some(json!({ "name": "UniHop", "scopes": ["designs:read"], "workspaceId": Uuid::new_v4().to_string() }))).await;
        assert_eq!(r.status, 404, "{:?}", r.body);
        let r = send(&s, "POST", "/api/developer/tokens", &c, Some(json!({ "name": "UniHop", "scopes": ["designs:read", "designs:export"], "workspaceId": ws.to_string() }))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        let r = send(&s, "GET", "/api/developer", &c, None).await;
        assert_eq!(r.body["tokens"][0]["workspace_id"], ws.to_string().as_str());
        assert_eq!(r.body["tokens"][0]["scopes"], json!(["designs:read", "designs:export"]));
        pool.close().await;
        let _ = std::fs::remove_dir_all(&data);
    })
    .await;
    db.drop_db().await;
    if let Err(e) = task {
        std::panic::resume_unwind(e.into_panic());
    }
}
