//! The Content Planner's review workflow and the REST API an outside publisher uses (docs/api.md, "Planner"):
//! submit → approve (admins only) → claim → per-channel results; Pinselo's own worker never touches external posts;
//! edits and design changes send approved posts back to review. Throwaway databases; skipped without
//! `OC_DATABASE_URL`.

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

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Res {
    status: u16,
    headers: axum::http::HeaderMap,
    body: Value,
}

async fn send(
    s: &AppState,
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Res {
    let mut req = axum::http::Request::builder()
        .method(method)
        .uri(url)
        .header("x-forwarded-for", "198.51.100.7");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = match body {
        Some(b) => req
            .header("content-type", "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    let res = oc_api::app(s.clone()).oneshot(req).await.expect("response");
    let status = res.status().as_u16();
    let headers = res.headers().clone();
    let raw = res
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    Res {
        status,
        headers,
        body: serde_json::from_slice(&raw).unwrap_or(Value::Null),
    }
}

async fn token(
    pool: &sqlx::PgPool,
    user: Uuid,
    scopes: &[&str],
    workspace: Option<Uuid>,
) -> String {
    let t = format!("oc_pat_{}", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO api_tokens(id, user_id, name, token_sha256, prefix, scopes, workspace_id) VALUES ($1,$2,'test',$3,$4,$5,$6)")
        .bind(Uuid::new_v4())
        .bind(user)
        .bind(oc_api::auth::sha(&t))
        .bind(&t[..12])
        .bind(scopes.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
        .bind(workspace)
        .execute(pool)
        .await
        .expect("token");
    t
}

async fn edit(pool: &sqlx::PgPool, design: &str) {
    sqlx::query("INSERT INTO doc_updates(design_id, update) VALUES ($1, $2)")
        .bind(design)
        .bind(oc_crdt::encode_state(&oc_crdt::new_doc()))
        .execute(pool)
        .await
        .expect("edit");
}

async fn login(s: &AppState, email: &str) -> String {
    let r = send(
        s,
        "POST",
        "/api/auth/login",
        &[],
        Some(json!({ "email": email, "password": "correct horse battery 42" })),
    )
    .await;
    assert_eq!(r.status, 200, "{:?}", r.body);
    r.headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok()?.split(';').next().map(str::to_owned))
        .collect::<Vec<_>>()
        .join("; ")
}

async fn status_of(pool: &sqlx::PgPool, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM social_posts WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("status")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_and_external_publishing() {
    let _serial = SERIAL.lock().await;
    let Some(db) = TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping");
        return;
    };
    let url = db.url.clone();
    let task = tokio::spawn(async move {
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(3).connect(&url).await.expect("connect test db");
        let data = std::env::temp_dir().join(format!("oc-planner-test-{}", Uuid::new_v4().simple()));
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

        // The UniHop team: Ada owns it, Bob designs in it.
        let team = Uuid::new_v4();
        sqlx::query("INSERT INTO workspaces(id, name, personal) VALUES ($1, 'UniHop', false)").bind(team).execute(&pool).await.expect("ws");
        sqlx::query("INSERT INTO memberships(workspace_id, user_id, role) VALUES ($1, $2, 'owner'), ($1, $3, 'designer')").bind(team).bind(ada).bind(bob).execute(&pool).await.expect("members");
        let writer = token(&pool, ada, &["designs:write"], None).await;
        let mut designs = Vec::new();
        for title in ["P-001 deadline", "P-002 fees"] {
            let r = send(&s, "POST", "/v1/designs", &[("authorization", &format!("Bearer {writer}"))], Some(json!({ "title": title, "width": 1080, "height": 1350 }))).await;
            assert_eq!(r.status, 201, "{:?}", r.body);
            let id = r.body["id"].as_str().expect("id").to_owned();
            sqlx::query("UPDATE designs SET workspace_id = $2 WHERE id = $1").bind(&id).bind(team).execute(&pool).await.expect("move");
            designs.push(id);
        }

        let (ca, cb) = (login(&s, "ada@example.test").await, login(&s, "bob@example.test").await);
        let (a, b) = ([("cookie", ca.as_str())], [("cookie", cb.as_str())]);
        let post_body = |design: &str, r: &str| {
            json!({ "designId": design, "publisher": "external", "channels": ["instagram", "facebook"], "format": "jpg",
                    "caption": "Merhaba #UniHop", "scheduledAt": "2020-11-16T17:00:00.000Z", "timezone": "Europe/Budapest", "status": "in_review",
                    "fields": { "ref": r, "pillar": "Deadlines and steps", "factIds": "F-013; F-031", "topic": "Şubat 2027 dönemi", "empty": " " } })
        };

        // Bob drafts and submits; external posts can't be scheduled without review, nor carry accounts.
        let r = send(&s, "POST", "/api/social/posts", &b, Some(json!({ "designId": designs[0], "publisher": "external", "channels": ["instagram"], "format": "jpg", "scheduledAt": "2020-11-16T17:00:00.000Z", "status": "scheduled" }))).await;
        assert_eq!((r.status, r.body["error"].as_str()), (400, Some("needs_approval")), "{:?}", r.body);
        let r = send(&s, "POST", "/api/social/posts", &b, Some(json!({ "designId": designs[0], "publisher": "external", "format": "jpg", "status": "in_review", "scheduledAt": "2020-11-16T17:00:00.000Z" }))).await;
        assert_eq!(r.body["error"], "no_channels");
        let r = send(&s, "POST", "/api/social/posts", &b, Some(post_body(&designs[0], "P-001"))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        let p1 = r.body["post"]["id"].as_str().expect("id").to_owned();
        assert_eq!(r.body["post"]["status"], "in_review");
        assert_eq!(r.body["post"]["fields"], json!({ "ref": "P-001", "pillar": "Deadlines and steps", "factIds": "F-013; F-031", "topic": "Şubat 2027 dönemi" }));
        assert_eq!(r.body["post"]["submittedBy"], bob.to_string().as_str());
        let r = send(&s, "POST", "/api/social/posts", &b, Some(post_body(&designs[1], "P-001"))).await;
        assert_eq!((r.status, r.body["error"].as_str()), (409, Some("duplicate_ref")), "{:?}", r.body);
        let r = send(&s, "POST", "/api/social/posts", &b, Some(post_body(&designs[1], "P-002"))).await;
        let p2 = r.body["post"]["id"].as_str().expect("id").to_owned();

        // Only admins and owners approve; the approval records the design version.
        let r = send(&s, "POST", &format!("/api/social/posts/{p1}/approve"), &b, None).await;
        assert_eq!(r.status, 403, "{:?}", r.body);
        let r = send(&s, "POST", &format!("/api/social/posts/{p1}/approve"), &a, None).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(r.body["post"]["status"], "approved");
        assert_eq!(r.body["post"]["approvedBy"], ada.to_string().as_str());
        assert_eq!(r.body["post"]["approvedVersion"], json!(1));
        let r = send(&s, "POST", &format!("/api/social/posts/{p1}/publish-now"), &a, None).await;
        assert_eq!(r.body["error"], "external_post");
        let r = send(&s, "GET", "/api/social/posts", &b, None).await;
        let listed = r.body["posts"].as_array().expect("posts").iter().find(|p| p["id"] == p1.as_str()).expect("listed").clone();
        assert_eq!(listed["approvedByName"], "Ada");

        // Pinselo's own worker never picks up external posts, even approved and overdue ones.
        assert_eq!(oc_social::publish::sweep(&pool, CLOCK).await.expect("sweep"), 0);
        assert_eq!(status_of(&pool, &p1).await, "approved");

        // The publisher: a token limited to the UniHop workspace.
        let tk = token(&pool, ada, &["planner:publish"], Some(team)).await;
        let h = [("authorization", format!("Bearer {tk}"))];
        let h = [(h[0].0, h[0].1.as_str())];
        let r = send(&s, "GET", "/v1/planner/posts?due=1", &h, None).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        let due: Vec<&str> = r.body["posts"].as_array().expect("posts").iter().filter_map(|p| p["ref"].as_str()).collect();
        assert_eq!(due, ["P-001"]);
        assert_eq!(r.body["posts"][0]["designVersion"], json!(1));
        let r = send(&s, "GET", "/v1/planner/posts/P-001", &h, None).await;
        assert_eq!(r.body["post"]["id"], p1.as_str());
        let personal: Uuid = sqlx::query_scalar("SELECT w.id FROM workspaces w JOIN memberships m ON m.workspace_id = w.id WHERE m.user_id = $1 AND w.personal").bind(ada).fetch_one(&pool).await.expect("personal ws");
        let elsewhere = token(&pool, ada, &["planner:publish"], Some(personal)).await;
        assert_eq!(send(&s, "GET", "/v1/planner/posts/P-001", &[("authorization", &format!("Bearer {elsewhere}"))], None).await.status, 404);
        let reader = token(&pool, ada, &["designs:read"], Some(team)).await;
        assert_eq!(send(&s, "GET", "/v1/planner/posts", &[("authorization", &format!("Bearer {reader}"))], None).await.status, 403);
        let r = send(&s, "POST", &format!("/v1/planner/posts/{p2}/claim"), &h, None).await;
        assert_eq!((r.status, r.body["error"].as_str()), (409, Some("not_claimable")), "P-002 is only in review: {:?}", r.body);

        // Claim once; a second claim loses.
        let r = send(&s, "POST", "/v1/planner/posts/P-001/claim", &h, None).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        let claim = r.body["claimToken"].as_str().expect("claim").to_owned();
        assert_eq!(r.body["post"]["status"], "publishing");
        assert_eq!(send(&s, "POST", "/v1/planner/posts/P-001/claim", &h, None).await.status, 409);
        let r = send(&s, "POST", "/v1/planner/posts/P-001/results", &h, Some(json!({ "claimToken": "clm_wrong", "results": {} }))).await;
        assert_eq!(r.body["error"], "claim_lost");
        let r = send(&s, "PATCH", &format!("/api/social/posts/{p1}"), &a, Some(json!({ "caption": "x" }))).await;
        assert_eq!(r.status, 409, "a post being published is locked");

        // Instagram goes out, Facebook fails: the post fails, Instagram's result stays.
        let r = send(&s, "POST", "/v1/planner/posts/P-001/results", &h, Some(json!({ "claimToken": claim, "results": { "instagram": { "status": "published", "id": "999", "url": "https://instagram.com/p/x" } }, "images": ["https://pinselo.test/1.jpg"] }))).await;
        assert_eq!(r.body["post"]["status"], "publishing", "{:?}", r.body);
        let r = send(&s, "POST", "/v1/planner/posts/P-001/results", &h, Some(json!({ "claimToken": claim, "results": { "facebook": { "status": "failed", "error": "(#200) pages_manage_posts" } }, "done": true }))).await;
        assert_eq!(r.body["post"]["status"], "failed", "{:?}", r.body);
        assert!(r.body["post"]["error"].as_str().expect("error").contains("facebook: (#200)"));
        assert_eq!(r.body["post"]["results"]["instagram"]["id"], "999");
        assert_eq!(r.body["post"]["fields"]["imageUrls"], "https://pinselo.test/1.jpg");

        // Fixing it goes through review again; Instagram's published result is kept, so only Facebook is retried.
        let r = send(&s, "PATCH", &format!("/api/social/posts/{p1}"), &b, Some(json!({ "caption": "Merhaba again" }))).await;
        assert_eq!(r.body["post"]["status"], "in_review", "{:?}", r.body);
        assert_eq!(r.body["post"]["results"]["instagram"]["status"], "published");
        assert_eq!(send(&s, "POST", &format!("/api/social/posts/{p1}/approve"), &a, None).await.status, 200);
        let claim = send(&s, "POST", "/v1/planner/posts/P-001/claim", &h, None).await.body["claimToken"].as_str().expect("claim").to_owned();
        let r = send(&s, "POST", "/v1/planner/posts/P-001/results", &h, Some(json!({ "claimToken": claim, "results": { "facebook": { "status": "published", "id": "1_2" } }, "done": true }))).await;
        assert_eq!(r.body["post"]["status"], "published", "{:?}", r.body);
        assert!(r.body["post"]["publishedAt"].is_string());
        let r = send(&s, "PATCH", "/v1/planner/posts/P-001", &[("authorization", &format!("Bearer {}", token(&pool, ada, &["planner:write"], Some(team)).await))], Some(json!({ "caption": "y" }))).await;
        assert_eq!(r.status, 409, "published posts are locked");

        // A design edited after approval: claiming sends the post back to review instead of posting it.
        assert_eq!(send(&s, "POST", &format!("/api/social/posts/{p2}/approve"), &a, None).await.status, 200);
        edit(&pool, &designs[1]).await;
        let r = send(&s, "POST", "/v1/planner/posts/P-002/claim", &h, None).await;
        assert_eq!((r.status, r.body["error"].as_str()), (409, Some("version_changed")), "{:?}", r.body);
        assert_eq!(r.body["currentVersion"], json!(2));
        assert_eq!(status_of(&pool, &p2).await, "in_review");

        // Request changes keeps the note in the editorial fields.
        let r = send(&s, "POST", &format!("/api/social/posts/{p2}/request-changes"), &a, Some(json!({ "note": "Mention the 20 Nov deadline." }))).await;
        assert_eq!(r.body["post"]["status"], "changes_requested", "{:?}", r.body);
        assert_eq!(r.body["post"]["fields"]["reviewerNote"], "Mention the 20 Nov deadline.");
        assert_eq!(send(&s, "POST", &format!("/api/social/posts/{p2}/submit"), &b, None).await.body["post"]["status"], "in_review");

        // Assistants draft through the API but cannot approve; refs work as ids.
        let wr = token(&pool, bob, &["planner:write"], Some(team)).await;
        let w = [("authorization", format!("Bearer {wr}"))];
        let w = [(w[0].0, w[0].1.as_str())];
        let mut p3 = post_body(&designs[0], "P-003");
        p3["status"] = json!("draft");
        let r = send(&s, "POST", "/v1/planner/posts", &w, Some(p3)).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!((r.body["post"]["ref"].as_str(), r.body["post"]["status"].as_str()), (Some("P-003"), Some("draft")));
        let r = send(&s, "PATCH", "/v1/planner/posts/P-003", &w, Some(json!({ "status": "in_review", "fields": { "ref": "P-003", "reviewerNote": "ready" } }))).await;
        assert_eq!(r.body["post"]["status"], "in_review", "{:?}", r.body);
        assert_eq!(send(&s, "POST", "/v1/planner/posts/P-003/approve", &w, None).await.status, 404, "the API has no approve route");

        // The AI connector: drafts and submits through the same rules; it cannot approve or touch published posts.
        let mcp = token(&pool, bob, &["designs:read", "designs:write"], None).await;
        let auth = format!("Bearer {mcp}");
        let call = |name: &str, args: Value| json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
        let mh = [("authorization", auth.as_str()), ("accept", "application/json, text/event-stream")];
        let r = send(&s, "POST", "/mcp", &mh, Some(call("create_planner_post", json!({
            "design": format!("{WEB}/design/{}", designs[1]), "channels": ["instagram"], "scheduled_at": "2030-01-05T17:00:00.000Z",
            "timezone": "Europe/Budapest", "caption": "Taslak", "fields": { "ref": "P-004", "pillar": "Costs" }, "submit": true,
        })))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(r.body["result"]["isError"], Value::Null, "{:?}", r.body);
        let made: Value = serde_json::from_str(r.body["result"]["content"][0]["text"].as_str().expect("text")).expect("json");
        assert_eq!((made["post"]["ref"].as_str(), made["post"]["status"].as_str(), made["post"]["publisher"].as_str()), (Some("P-004"), Some("in_review"), Some("external")));
        let r = send(&s, "POST", "/mcp", &mh, Some(call("update_planner_post", json!({ "post": "P-001", "caption": "z" })))).await;
        assert_eq!(r.body["result"]["isError"], true, "published posts are locked: {:?}", r.body);
        assert!(r.body["result"]["content"][0]["text"].as_str().expect("text").contains("already published"));
        let r = send(&s, "POST", "/mcp", &mh, Some(call("list_planner_posts", json!({ "status": "in_review" })))).await;
        let listed: Value = serde_json::from_str(r.body["result"]["content"][0]["text"].as_str().expect("text")).expect("json");
        let mut refs: Vec<&str> = listed["posts"].as_array().expect("posts").iter().filter_map(|p| p["ref"].as_str()).collect();
        refs.sort_unstable();
        assert_eq!(refs, ["P-002", "P-003", "P-004"]);

        pool.close().await;
        let _ = std::fs::remove_dir_all(&data);
    })
    .await;
    db.drop_db().await;
    if let Err(e) = task {
        std::panic::resume_unwind(e.into_panic());
    }
}
