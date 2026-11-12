//! The MCP tools for posts Pinselo publishes itself: list_social_accounts, create_planner_post with accounts
//! (drafted or submitted, never scheduled by an assistant; a Reel exports as MP4), check_publishing_limit and
//! get_post_results. Meta is the in-process fake (`OC_SOCIAL_FAKE_BASE`); skipped without `OC_DATABASE_URL`.

mod harness;

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use harness::{API, CLOCK, Fake, SECRET, WEB};
use http_body_util::BodyExt as _;
use oc_api::AppState;
use oc_db::testing::TestDb;
use oc_social::{ConnectedAccount, Tokens};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

const GRAPH: &str = "/graph.facebook.com/v27.0";
const IG: &str = "17841440769007273";
const PAGE: &str = "1438856472642415";

async fn send(s: &AppState, method: &str, url: &str, auth: &str, body: Option<Value>) -> (u16, Value) {
    let mut req = axum::http::Request::builder()
        .method(method)
        .uri(url)
        .header("x-forwarded-for", "198.51.100.9")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {auth}"));
    req = req.header("content-type", "application/json");
    let res = oc_api::app(s.clone()).oneshot(req.body(Body::from(body.map(|b| b.to_string()).unwrap_or_default())).expect("req")).await.expect("res");
    let status = res.status().as_u16();
    let raw = res.into_body().collect().await.expect("body").to_bytes().to_vec();
    (status, serde_json::from_slice(&raw).unwrap_or(Value::Null))
}

/// A tools/call: `(isError, the text parsed as JSON when it is JSON, else as a string)`.
async fn tool(s: &AppState, token: &str, name: &str, args: Value) -> (bool, Value) {
    let (status, body) = send(s, "POST", "/mcp", token, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } }))).await;
    assert_eq!(status, 200, "{body}");
    let text = body["result"]["content"][0]["text"].as_str().unwrap_or("").to_owned();
    (body["result"]["isError"] == true, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

#[tokio::test]
async fn publisher_tools() {
    let Some(db) = TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping");
        return;
    };
    let url = db.url.clone();
    let task = tokio::spawn(async move {
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(3).connect(&url).await.expect("connect test db");
        let data = std::env::temp_dir().join(format!("oc-mcp-publishing-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&data).expect("data dir");
        let fake = Fake::start().await;
        let env: HashMap<String, String> = [
            ("NODE_ENV", "development"),
            ("OC_DATABASE_URL", url.as_str()),
            ("OC_DATA_DIR", data.to_str().expect("dir")),
            ("OC_SECRET", SECRET),
            ("OC_WEB_ORIGIN", WEB),
            ("OC_API_URL", API),
            ("OC_SOCIAL_FAKE_BASE", fake.base.as_str()),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .into_iter()
        .collect();
        let clock: oc_api::state::Clock = Arc::new(|| CLOCK);
        let s = AppState::new(oc_api::AppOptions { env, pool: pool.clone(), store: None, clock: Some(clock) }).await.expect("app state");

        let ada = oc_api::auth::create_user(&s, "ada@example.test", "Ada", oc_api::auth::NewUser { verified: true, ..Default::default() }).await.expect("user").id;
        let team = Uuid::new_v4();
        sqlx::query("INSERT INTO workspaces(id, name, personal) VALUES ($1, 'Unihop', false)").bind(team).execute(&pool).await.expect("ws");
        sqlx::query("INSERT INTO memberships(workspace_id, user_id, role) VALUES ($1, $2, 'owner')").bind(team).bind(ada).execute(&pool).await.expect("member");
        let token = format!("oc_pat_{}", Uuid::new_v4().simple());
        sqlx::query("INSERT INTO api_tokens(id, user_id, name, token_sha256, prefix, scopes) VALUES ($1,$2,'assistant',$3,$4,$5)")
            .bind(Uuid::new_v4())
            .bind(ada)
            .bind(oc_api::auth::sha(&token))
            .bind(&token[..12])
            .bind(vec!["designs:read".to_owned(), "designs:write".to_owned()])
            .execute(&pool)
            .await
            .expect("token");

        // The team's Meta app and its Page + Instagram account, as Settings connects them.
        let app = oc_social::apps::save_meta_app(&pool, SECRET, team, ada, "1999329587405510", Some("0123456789abcdef0123456789abcdef"), Some("UniHop Publisher"), Some("v27.0")).await.expect("app");
        let tokens = Tokens { access_token: "PAGE-TOKEN".into(), ..Tokens::default() };
        let mut ids = Vec::new();
        for (provider, ext, handle) in [("facebook", PAGE, None), ("instagram", IG, Some("unihop.eu"))] {
            let acc = ConnectedAccount { external_id: ext.into(), display_name: "UniHop".into(), handle: handle.map(str::to_owned), avatar_url: None, tokens: tokens.clone(), meta: None };
            let id = oc_social::store::save_accounts(&pool, SECRET, ada, provider, &[acc]).await.expect("save")[0];
            oc_social::apps::link_accounts(&pool, &[id], Some(team), Some(&app), "system_user").await.expect("link");
            ids.push(id);
        }
        let (fb, ig) = (ids[0], ids[1]);

        // ---- the tools are announced
        let (_, list) = send(&s, "POST", "/mcp", &token, Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))).await;
        let names: Vec<&str> = list["result"]["tools"].as_array().expect("tools").iter().filter_map(|t| t["name"].as_str()).collect();
        for n in ["list_social_accounts", "check_publishing_limit", "get_post_results"] {
            assert!(names.contains(&n), "{n} in {names:?}");
        }

        // ---- list_social_accounts
        let (err, r) = tool(&s, &token, "list_social_accounts", json!({})).await;
        assert!(!err, "{r}");
        let accs = r["accounts"].as_array().expect("accounts");
        assert_eq!(accs.len(), 2, "{r}");
        let ig_row = accs.iter().find(|a| a["network"] == "instagram").expect("ig");
        assert_eq!(ig_row["id"], ig.to_string().as_str());
        assert_eq!(ig_row["handle"], "unihop.eu");
        assert_eq!(ig_row["team"], "Unihop");
        assert_eq!((ig_row["status"].as_str(), ig_row["team_meta_app"].as_bool(), ig_row["can_publish"].as_bool()), (Some("ok"), Some(true), Some(true)));
        assert_eq!(ig_row["connected_via"], "Meta system-user token");
        let (_, r) = tool(&s, &token, "list_social_accounts", json!({ "team": "personal" })).await;
        assert_eq!(r["accounts"], json!([]));
        let (err, r) = tool(&s, &token, "list_social_accounts", json!({ "team": "Nope" })).await;
        assert!(err && r.as_str().unwrap_or("").contains("The user's teams: Unihop"), "{r}");

        // ---- create_planner_post with accounts: Pinselo publishes; a Reel exports as MP4; never scheduled
        let (_, d) = send(&s, "POST", "/v1/designs", &token, Some(json!({ "title": "REEL · Okurken çalışmak", "width": 1080, "height": 1920 }))).await;
        let design = d["id"].as_str().expect("design").to_owned();
        let (err, r) = tool(&s, &token, "create_planner_post", json!({
            "design": design, "accounts": [ig.to_string(), fb.to_string()], "caption": "Okurken çalışabilir misin?",
            "scheduled_at": "2025-01-02T17:23:00+01:00", "timezone": "Europe/Budapest",
            "fields": { "ref": "P-009", "formatLabel": "Reel", "hungarianTranslation": "csak a lektornak" }, "submit": true,
        })).await;
        assert!(!err, "{r}");
        let post_id = r["post"]["id"].as_str().expect("post id").to_owned();
        let row: (String, String, String, Vec<Uuid>) = sqlx::query_as("SELECT publisher, format, status, accounts FROM social_posts WHERE id = $1::uuid").bind(&post_id).fetch_one(&pool).await.expect("post");
        assert_eq!((row.0.as_str(), row.1.as_str(), row.2.as_str()), ("pinselo", "mp4", "in_review"));
        assert_eq!(row.3, vec![ig, fb]);
        let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE payload->>'postId' = $1").bind(&post_id).fetch_one(&pool).await.expect("jobs");
        assert_eq!(jobs, 0, "nothing is queued to publish before a person approves");

        let (err, r) = tool(&s, &token, "create_planner_post", json!({ "design": design, "accounts": [ig.to_string()], "caption": "Taslak" })).await;
        assert!(!err, "{r}");
        assert_eq!(r["post"]["status"], "draft", "without submit it stays a draft, never scheduled");
        assert_eq!(r["post"]["format"], "jpg");
        let (err, r) = tool(&s, &token, "create_planner_post", json!({ "design": design, "accounts": [ig.to_string()], "channels": ["instagram"] })).await;
        assert!(err && r.as_str().unwrap_or("").contains("not both"), "{r}");
        // Without accounts, it is still a post for the team's outside publisher.
        let (err, r) = tool(&s, &token, "create_planner_post", json!({ "design": design, "channels": ["instagram"], "caption": "Dış" })).await;
        assert!(!err, "{r}");
        assert_eq!(r["post"]["publisher"], "external");

        // ---- check_publishing_limit, through the account's team app and token
        fake.set_default(&format!("{GRAPH}/{IG}/content_publishing_limit"), &json!({ "json": { "data": [{ "quota_usage": 3, "config": { "quota_total": 100, "quota_duration": 86400 } }] } }));
        let (err, r) = tool(&s, &token, "check_publishing_limit", json!({ "account": "@unihop.eu" })).await;
        assert!(!err, "{r}");
        assert_eq!((r["used"].as_i64(), r["total"].as_i64(), r["remaining"].as_i64()), (Some(3), Some(100), Some(97)));
        let req = fake.requests(&format!("{GRAPH}/{IG}/content_publishing_limit")).pop().expect("request");
        assert_eq!(req["headers"]["authorization"], "Bearer PAGE-TOKEN", "{req}");
        fake.set_default(&format!("{GRAPH}/{PAGE}/video_reels"), &json!({ "json": { "data": [
            { "id": "r1", "created_time": "2025-01-01T09:00:00+0000" },
            { "id": "r0", "created_time": "2024-12-30T09:00:00+0000" },
        ] } }));
        let (err, r) = tool(&s, &token, "check_publishing_limit", json!({ "account": fb.to_string() })).await;
        assert!(!err, "{r}");
        assert_eq!((r["reels_used"].as_i64(), r["reels_remaining"].as_i64()), (Some(1), Some(29)));
        let (err, r) = tool(&s, &token, "check_publishing_limit", json!({ "account": "UniHop" })).await;
        assert!(err && r.as_str().unwrap_or("").contains("matches 2 accounts"), "{r}");

        // ---- get_post_results after the worker published it
        sqlx::query(
            "UPDATE social_posts SET status = 'published', published_at = now(),
               results = jsonb_build_object($2::text, jsonb_build_object('status', 'published', 'url', 'https://www.instagram.com/reel/X/', 'at', '2025-01-02T16:24:00.000Z'),
                                            $3::text, jsonb_build_object('status', 'failed', 'error', 'Facebook could not process the reel', 'at', '2025-01-02T16:25:00.000Z'))
             WHERE id = $1::uuid",
        )
        .bind(&post_id)
        .bind(ig.to_string())
        .bind(fb.to_string())
        .execute(&pool)
        .await
        .expect("published");
        oc_social::publish::log_event(&pool, Uuid::parse_str(&post_id).expect("uuid"), "published", Some("https://www.instagram.com/reel/X/"), Some(json!({ "externalId": "IGMEDIA1" })), Some(ig)).await.expect("event");
        fake.set_default(&format!("{GRAPH}/IGMEDIA1"), &json!({ "json": { "id": "IGMEDIA1", "permalink": "https://www.instagram.com/reel/X/", "media_type": "VIDEO", "like_count": 42, "comments_count": 5 } }));
        fake.set_default(&format!("{GRAPH}/IGMEDIA1/insights"), &json!({ "json": { "data": [
            { "name": "reach", "total_value": { "value": 1200 } }, { "name": "saved", "total_value": { "value": 31 } }, { "name": "shares", "total_value": { "value": 9 } },
        ] } }));
        let (err, r) = tool(&s, &token, "get_post_results", json!({ "post": "P-009" })).await;
        assert!(!err, "{r}");
        assert_eq!(r["status"], "published");
        let t = r["targets"].as_array().expect("targets");
        assert_eq!(t[0]["network"], "instagram");
        assert_eq!(t[0]["post_id"], "IGMEDIA1");
        assert_eq!((t[0]["numbers"]["reach"].as_i64(), t[0]["numbers"]["saves"].as_i64(), t[0]["numbers"]["likes"].as_i64()), (Some(1200), Some(31), Some(42)));
        assert_eq!((t[1]["network"].as_str(), t[1]["status"].as_str()), (Some("facebook"), Some("failed")));
        assert_eq!(t[1]["error"], "Facebook could not process the reel");

        // A token without designs:read can't use them.
        let weak = format!("oc_pat_{}", Uuid::new_v4().simple());
        sqlx::query("INSERT INTO api_tokens(id, user_id, name, token_sha256, prefix, scopes) VALUES ($1,$2,'weak',$3,$4,$5)")
            .bind(Uuid::new_v4())
            .bind(ada)
            .bind(oc_api::auth::sha(&weak))
            .bind(&weak[..12])
            .bind(vec!["exports".to_owned()])
            .execute(&pool)
            .await
            .expect("weak");
        let (err, r) = tool(&s, &weak, "list_social_accounts", json!({})).await;
        assert!(err && r.as_str().unwrap_or("").contains("designs:read"), "{r}");

        pool.close().await;
        let _ = std::fs::remove_dir_all(&data);
    })
    .await;
    db.drop_db().await;
    if let Err(e) = task {
        std::panic::resume_unwind(e.into_panic());
    }
}
