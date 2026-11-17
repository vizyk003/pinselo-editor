//! A brand kit's videos: Brand Hub keeps them (and keeps them when other fields change), the AI connector's
//! get_brand_kit lists them, update_brand_kit adds and removes only video uploads, and everyone who can see the kit
//! can place them with add_video. Throwaway database; skipped without `OC_DATABASE_URL`.

mod harness;

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use harness::{API, CLOCK, SECRET, WEB};
use http_body_util::BodyExt as _;
use oc_api::AppState;
use oc_db::testing::TestDb;
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

/// A request with a session cookie or a bearer token: `(status, JSON body, cookies set)`.
async fn send(s: &AppState, method: &str, url: &str, auth: &str, body: Option<Value>) -> (u16, Value, String) {
    let mut req = axum::http::Request::builder().method(method).uri(url).header("x-forwarded-for", "198.51.100.7").header("accept", "application/json, text/event-stream");
    req = if let Some(t) = auth.strip_prefix("Bearer ") { req.header("authorization", format!("Bearer {t}")) } else { req.header("cookie", auth) };
    let req = match body {
        Some(b) => req.header("content-type", "application/json").body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    let res = oc_api::app(s.clone()).oneshot(req).await.expect("response");
    let status = res.status().as_u16();
    let cookies = res.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()?.split(';').next().map(str::to_owned)).collect::<Vec<_>>().join("; ");
    let raw = res.into_body().collect().await.expect("body").to_bytes().to_vec();
    (status, serde_json::from_slice(&raw).unwrap_or(Value::Null), cookies)
}

async fn login(s: &AppState, email: &str) -> String {
    let (status, body, cookies) = send(s, "POST", "/api/auth/login", "", Some(json!({ "email": email, "password": "correct horse battery 42" }))).await;
    assert_eq!(status, 200, "{body:?}");
    cookies
}

/// A tools/call: `(isError, the text parsed as JSON when it is JSON, else as a string)`.
async fn tool(s: &AppState, bearer: &str, name: &str, args: Value) -> (bool, Value) {
    let (status, body, _) = send(s, "POST", "/mcp", bearer, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } }))).await;
    assert_eq!(status, 200, "{body}");
    let text = body["result"]["content"][0]["text"].as_str().unwrap_or("").to_owned();
    (body["result"]["isError"] == true, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

async fn bearer(pool: &sqlx::PgPool, user: Uuid) -> String {
    let token = format!("oc_pat_{}", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO api_tokens(id, user_id, name, token_sha256, prefix, scopes) VALUES ($1,$2,'assistant',$3,$4,$5)")
        .bind(Uuid::new_v4())
        .bind(user)
        .bind(oc_api::auth::sha(&token))
        .bind(&token[..12])
        .bind(vec!["designs:read".to_owned(), "designs:write".to_owned()])
        .execute(pool)
        .await
        .expect("token");
    format!("Bearer {token}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_kit_holds_videos() {
    let Some(db) = TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping");
        return;
    };
    let url = db.url.clone();
    let task = tokio::spawn(async move {
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(3).connect(&url).await.expect("connect test db");
        let data = std::env::temp_dir().join(format!("oc-kit-videos-test-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&data).expect("data dir");
        let env: HashMap<String, String> = [("NODE_ENV", "development"), ("OC_DATABASE_URL", url.as_str()), ("OC_DATA_DIR", data.to_str().expect("dir")), ("OC_SECRET", SECRET), ("OC_WEB_ORIGIN", WEB), ("OC_API_URL", API)]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into_iter()
            .collect();
        let clock: oc_api::state::Clock = Arc::new(|| CLOCK);
        let s = AppState::new(oc_api::AppOptions { env, pool: pool.clone(), store: None, clock: Some(clock) }).await.expect("app state");
        let pw = || oc_api::auth::NewUser { password: Some("correct horse battery 42"), verified: true, ..Default::default() };
        let ada = oc_api::auth::create_user(&s, "ada@example.test", "Ada", pw()).await.expect("ada").id;
        let bob = oc_api::auth::create_user(&s, "bob@example.test", "Bob", pw()).await.expect("bob").id;
        // Ada owns the UniHop team; Bob is a member (uses its kits, can't change them).
        let team = Uuid::new_v4();
        sqlx::query("INSERT INTO workspaces(id, name, personal) VALUES ($1, 'UniHop', false)").bind(team).execute(&pool).await.expect("ws");
        sqlx::query("INSERT INTO memberships(workspace_id, user_id, role) VALUES ($1, $2, 'owner'), ($1, $3, 'member')").bind(team).bind(ada).bind(bob).execute(&pool).await.expect("members");
        let personal: Uuid = sqlx::query_scalar("SELECT id FROM workspaces WHERE personal AND owner_id = $1").bind(ada).fetch_one(&pool).await.expect("personal");
        // Ada's uploads, in her personal space: two clips and a photo.
        for (id, mime, kind, name) in [
            ("KitVideo00000001", "video/mp4", "video", "campus-tour.mp4"),
            ("KitVideo00000002", "video/quicktime", "video", "logo-sting.mov"),
            ("KitPhoto00000001", "image/jpeg", "image", "students.jpg"),
        ] {
            sqlx::query("INSERT INTO assets(id, workspace_id, owner_id, sha256, mime, kind, bytes, width, height, name, duration_ms) VALUES ($1, $2, $3, $4, $5, $6, 1234, 1080, 1920, $7, CASE WHEN $6 = 'video' THEN 8000 END)")
                .bind(id)
                .bind(personal)
                .bind(ada)
                .bind(format!("{:064x}", id.bytes().map(usize::from).sum::<usize>()))
                .bind(mime)
                .bind(kind)
                .bind(name)
                .execute(&pool)
                .await
                .expect("asset");
        }
        let a = login(&s, "ada@example.test").await;

        // ---- Brand Hub: a team kit with a video; other edits keep it; a kit made without videos has none
        let (status, body, _) = send(&s, "POST", "/api/brand-kits", &a, Some(json!({ "name": "UniHop.eu", "workspaceId": team.to_string(), "videos": ["KitVideo00000001"] }))).await;
        assert_eq!(status, 200, "{body:?}");
        let kit = body["id"].as_str().expect("id").to_owned();
        let videos = async |id: &str| -> Value {
            let (status, body, _) = send(&s, "GET", &format!("/api/brand-kits/{id}"), &a, None).await;
            assert_eq!(status, 200, "{body:?}");
            body["kit"]["videos"].clone()
        };
        assert_eq!(videos(&kit).await, json!(["KitVideo00000001"]));
        let (status, body, _) = send(&s, "PATCH", &format!("/api/brand-kits/{kit}"), &a, Some(json!({ "voice": "Plain and warm.", "photos": ["KitPhoto00000001"] }))).await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(videos(&kit).await, json!(["KitVideo00000001"]), "a patch without videos keeps them");
        let (status, body, _) = send(&s, "PATCH", &format!("/api/brand-kits/{kit}"), &a, Some(json!({ "videos": ["KitVideo00000001", "KitVideo00000002"] }))).await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(videos(&kit).await, json!(["KitVideo00000001", "KitVideo00000002"]));
        let (status, body, _) = send(&s, "PATCH", &format!("/api/brand-kits/{kit}"), &a, Some(json!({ "videos": ["not an id"] }))).await;
        assert_eq!(status, 400, "{body:?}");
        let (_, body, _) = send(&s, "POST", "/api/brand-kits", &a, Some(json!({ "name": "Everyday" }))).await;
        assert_eq!(videos(body["id"].as_str().expect("id")).await, json!([]));

        // ---- The AI connector: get_brand_kit names the clips; update_brand_kit takes only video uploads as videos
        let ada_ai = bearer(&pool, ada).await;
        let (err, r) = tool(&s, &ada_ai, "get_brand_kit", json!({})).await;
        assert!(!err, "{r}");
        let k = r["kits"].as_array().expect("kits").iter().find(|k| k["name"] == "UniHop.eu").expect("kit").clone();
        let named: Vec<(&str, &str, &str)> = k["videos"].as_array().expect("videos").iter().map(|v| (v["asset"].as_str().unwrap_or(""), v["name"].as_str().unwrap_or(""), v["orientation"].as_str().unwrap_or(""))).collect();
        assert_eq!(named, vec![("KitVideo00000001", "campus-tour.mp4", "vertical"), ("KitVideo00000002", "logo-sting.mov", "vertical")], "{k}");
        let everyday = r["kits"].as_array().expect("kits").iter().find(|k| k["name"] == "Everyday").expect("kit").clone();
        assert!(everyday.get("videos").is_none(), "a kit without videos doesn't list any: {everyday}");
        let (err, r) = tool(&s, &ada_ai, "update_brand_kit", json!({ "brand_kit": "UniHop.eu", "add_videos": ["KitPhoto00000001"] })).await;
        assert!(err && r.as_str().unwrap_or("").contains("aren't video uploads of yours: KitPhoto00000001"), "{r}");
        let (err, r) = tool(&s, &ada_ai, "update_brand_kit", json!({ "brand_kit": "UniHop.eu", "add_photos": ["KitVideo00000002"] })).await;
        assert!(err && r.as_str().unwrap_or("").contains("aren't image uploads of yours: KitVideo00000002"), "{r}");
        let (err, r) = tool(&s, &ada_ai, "update_brand_kit", json!({ "brand_kit": "UniHop.eu", "remove_assets": ["KitVideo00000002"] })).await;
        assert!(!err, "{r}");
        assert_eq!(videos(&kit).await, json!(["KitVideo00000001"]));
        let (err, r) = tool(&s, &ada_ai, "update_brand_kit", json!({ "brand_kit": "UniHop.eu", "add_videos": ["KitVideo00000002"] })).await;
        assert!(!err, "{r}");
        assert_eq!(videos(&kit).await, json!(["KitVideo00000001", "KitVideo00000002"]));
        let (err, r) = tool(&s, &ada_ai, "create_brand_kit", json!({ "name": "Clips", "videos": ["KitVideo00000002"] })).await;
        assert!(!err, "{r}");
        assert_eq!(r["kit"]["videos"].as_array().map(Vec::len).or_else(|| r["videos"].as_array().map(Vec::len)), Some(1), "{r}");

        // ---- describe_asset: Ada describes and tags her clip; get_brand_kit shows it with the clip's shape and length,
        // and list_uploads finds it by a tag
        let (err, r) = tool(&s, &ada_ai, "describe_asset", json!({
            "asset": "KitVideo00000001", "description": "Students walking across a sunny campus, filmed from behind.",
            "tags": ["Campus", " students ", "campus", "summer"],
        })).await;
        assert!(!err, "{r}");
        assert_eq!(r["described"]["tags"], json!(["campus", "students", "summer"]), "trimmed, lower case, once each: {r}");
        let (_, r) = tool(&s, &ada_ai, "get_brand_kit", json!({})).await;
        let k = r["kits"].as_array().expect("kits").iter().find(|k| k["name"] == "UniHop.eu").expect("kit").clone();
        assert_eq!(
            k["videos"][0],
            json!({ "asset": "KitVideo00000001", "name": "campus-tour.mp4", "description": "Students walking across a sunny campus, filmed from behind.",
                    "tags": ["campus", "students", "summer"], "w": 1080, "h": 1920, "orientation": "vertical", "seconds": 8, "sound": false }),
        );
        assert!(k["photos"][0].get("orientation").is_none() && k["photos"][0].get("description").is_none(), "photos stay as they were: {k}");
        let (_, r) = tool(&s, &ada_ai, "list_uploads", json!({ "type": "video", "query": "summer" })).await;
        let found: Vec<&str> = r["uploads"].as_array().expect("uploads").iter().filter_map(|u| u["asset"].as_str()).collect();
        assert_eq!(found, vec!["KitVideo00000001"], "{r}");
        let (err, r) = tool(&s, &ada_ai, "describe_asset", json!({ "asset": "KitVideo00000001", "tags": [] })).await;
        assert!(!err && r["described"].get("tags").is_none() && r["described"]["description"].is_string(), "tags cleared, description kept: {r}");
        let (err, r) = tool(&s, &ada_ai, "describe_asset", json!({ "asset": "KitVideo00000001" })).await;
        assert!(err, "{r}");

        // ---- Bob, a member, places the kit's clip in his design, though Ada uploaded it; her other uploads stay hers
        let bob_ai = bearer(&pool, bob).await;
        let (err, r) = tool(&s, &bob_ai, "describe_asset", json!({ "asset": "KitVideo00000001", "description": "Bob's words" })).await;
        assert!(err && r.as_str().unwrap_or("").contains("can edit a brand kit"), "a member who can't edit the kit can't describe its files: {r}");
        let (status, d, _) = send(&s, "POST", "/v1/designs", &bob_ai, Some(json!({ "title": "Bob's reel", "width": 1080, "height": 1920 }))).await;
        assert_eq!(status, 201, "{d:?}");
        let design = d["id"].as_str().expect("id").to_owned();
        let place = async |asset: &str| tool(&s, &bob_ai, "edit_design", json!({ "design_id": design, "operations": [{ "op": "add_video", "source": { "asset": asset }, "x": 0, "y": 0, "w": 1080, "h": 1920 }] })).await;
        let (err, r) = place("KitVideo00000001").await;
        assert!(!err, "{r}");
        sqlx::query("UPDATE brand_kits SET videos = '[]' WHERE id = $1::uuid").bind(&kit).execute(&pool).await.expect("clear");
        let (err, r) = place("KitVideo00000002").await;
        assert!(err, "out of the kit, Ada's clip is out of Bob's reach: {r}");

        pool.close().await;
    });
    let res = task.await;
    db.drop_db().await;
    if let Err(e) = res {
        std::panic::resume_unwind(e.into_panic());
    }
}
