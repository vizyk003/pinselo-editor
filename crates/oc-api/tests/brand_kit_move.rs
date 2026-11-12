//! Moving a brand kit to another workspace (`PATCH /api/brand-kits/{id}` with `workspaceId`): it keeps its id, each
//! workspace keeps exactly one default, and only people who may edit kits on both sides can do it. Throwaway
//! database; skipped without `OC_DATABASE_URL`.

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

async fn send(s: &AppState, method: &str, url: &str, cookie: &str, body: Option<Value>) -> (u16, Value, String) {
    let req = axum::http::Request::builder().method(method).uri(url).header("x-forwarded-for", "198.51.100.7").header("cookie", cookie);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_kit_moves_between_workspaces() {
    let Some(db) = TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping");
        return;
    };
    let url = db.url.clone();
    let task = tokio::spawn(async move {
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(3).connect(&url).await.expect("connect test db");
        let data = std::env::temp_dir().join(format!("oc-kit-move-test-{}", Uuid::new_v4().simple()));
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

        // Ada owns the UniHop team; Bob is a member there (views kits, cannot add them).
        let team = Uuid::new_v4();
        sqlx::query("INSERT INTO workspaces(id, name, personal) VALUES ($1, 'UniHop', false)").bind(team).execute(&pool).await.expect("ws");
        sqlx::query("INSERT INTO memberships(workspace_id, user_id, role) VALUES ($1, $2, 'owner'), ($1, $3, 'member')").bind(team).bind(ada).bind(bob).execute(&pool).await.expect("members");
        let (a, b) = (login(&s, "ada@example.test").await, login(&s, "bob@example.test").await);

        // Ada has two personal kits: the first is her default.
        let mut ids = Vec::new();
        for name in ["Everyday", "UniHop.eu"] {
            let (status, body, _) = send(&s, "POST", "/api/brand-kits", &a, Some(json!({ "name": name, "voice": "Plain and honest." }))).await;
            assert_eq!(status, 200, "{body:?}");
            ids.push(body["id"].as_str().expect("id").to_owned());
        }
        let kit = async |id: &str| -> (Uuid, bool, String) { sqlx::query_as("SELECT workspace_id, is_default, voice FROM brand_kits WHERE id = $1::uuid").bind(id).fetch_one(&pool).await.expect("kit") };
        let personal = kit(&ids[0]).await.0;
        assert_eq!((kit(&ids[0]).await.1, kit(&ids[1]).await.1), (true, false));

        // Bob can't move his own kit into a team where he is only a member, nor touch Ada's kit.
        let (status, body, _) = send(&s, "POST", "/api/brand-kits", &b, Some(json!({ "name": "Bob's" }))).await;
        assert_eq!(status, 200, "{body:?}");
        let bobs = body["id"].as_str().expect("id").to_owned();
        assert_eq!(send(&s, "PATCH", &format!("/api/brand-kits/{bobs}"), &b, Some(json!({ "workspaceId": team }))).await.0, 403);
        assert_ne!(kit(&bobs).await.0, team);
        assert_eq!(send(&s, "PATCH", &format!("/api/brand-kits/{}", ids[1]), &b, Some(json!({ "workspaceId": team }))).await.0, 404);

        // Ada moves the UniHop kit into the team: same id, same content, and it is the team's default (its first kit).
        let (status, body, _) = send(&s, "PATCH", &format!("/api/brand-kits/{}", ids[1]), &a, Some(json!({ "workspaceId": team }))).await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(kit(&ids[1]).await, (team, true, "Plain and honest.".to_owned()));
        assert!(kit(&ids[0]).await.1);
        // Bob now sees it through the team, read-only.
        let (_, body, _) = send(&s, "GET", &format!("/api/brand-kits/{}", ids[1]), &b, None).await;
        assert_eq!((body["kit"]["name"].as_str(), body["kit"]["canEdit"].as_bool()), (Some("UniHop.eu"), Some(false)));

        // Moving her default kit too: it is not the default where a default exists, and her personal space has none left.
        let (status, body, _) = send(&s, "PATCH", &format!("/api/brand-kits/{}", ids[0]), &a, Some(json!({ "workspaceId": team, "name": "Everyday brand" }))).await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!((kit(&ids[0]).await.0, kit(&ids[0]).await.1), (team, false));
        let defaults: i64 = sqlx::query_scalar("SELECT count(*) FROM brand_kits WHERE workspace_id = $1 AND is_default").bind(team).fetch_one(&pool).await.expect("count");
        assert_eq!(defaults, 1);
        // And back: it is her personal default again, and the rename made in the same request stuck.
        let (status, body, _) = send(&s, "PATCH", &format!("/api/brand-kits/{}", ids[0]), &a, Some(json!({ "workspaceId": personal }))).await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!((kit(&ids[0]).await.0, kit(&ids[0]).await.1), (personal, true));
        let name: String = sqlx::query_scalar("SELECT name FROM brand_kits WHERE id = $1::uuid").bind(&ids[0]).fetch_one(&pool).await.expect("name");
        assert_eq!(name, "Everyday brand");

        pool.close().await;
        let _ = std::fs::remove_dir_all(&data);
    })
    .await;
    db.drop_db().await;
    if let Err(e) = task {
        std::panic::resume_unwind(e.into_panic());
    }
}
