//! A team's own Meta app (docs/configuration.md, "Social publishing"): owners and admins set it (checked with Meta
//! first, secret sealed), connect Pages and Instagram accounts with a pasted system-user token (checked with
//! `debug_token` against that app), and Facebook login runs through it; its accounts publish through it, and
//! removing it sends them back to reconnect. Meta is the in-process fake (`OC_SOCIAL_FAKE_BASE`); skipped
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

const APP: &str = "1999329587405510";
const APP_SECRET: &str = "0123456789abcdef0123456789abcdef";
const SU_TOKEN: &str = "EAAcSystemUserTokenForUniHop0000000000000000";
const GRAPH: &str = "/graph.facebook.com/v27.0";

struct Res {
    status: u16,
    body: Value,
}

async fn send(s: &AppState, method: &str, url: &str, cookie: &str, body: Option<Value>) -> Res {
    let mut req = axum::http::Request::builder().method(method).uri(url).header("x-forwarded-for", "198.51.100.7");
    if !cookie.is_empty() {
        req = req.header("cookie", cookie);
    }
    let req = match body {
        Some(b) => req.header("content-type", "application/json").body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    let res = oc_api::app(s.clone()).oneshot(req).await.expect("response");
    let status = res.status().as_u16();
    let raw = res.into_body().collect().await.expect("body").to_bytes().to_vec();
    Res { status, body: serde_json::from_slice(&raw).unwrap_or(Value::Null) }
}

async fn login(s: &AppState, email: &str, name: &str) -> (Uuid, String) {
    let pw = "correct horse battery 42";
    let id = oc_api::auth::create_user(s, email, name, oc_api::auth::NewUser { password: Some(pw), verified: true, ..Default::default() }).await.expect("user").id;
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("x-forwarded-for", "198.51.100.7")
        .header("content-type", "application/json")
        .body(Body::from(json!({ "email": email, "password": pw }).to_string()))
        .expect("login");
    let res = oc_api::app(s.clone()).oneshot(req).await.expect("login");
    assert_eq!(res.status().as_u16(), 200);
    let cookie = res.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()?.split(';').next().map(str::to_owned)).collect::<Vec<_>>().join("; ");
    (id, cookie)
}

fn debug_token(app_id: &str, scopes: &[&str]) -> Value {
    json!({ "json": { "data": { "app_id": app_id, "type": "SYSTEM_USER", "is_valid": true, "expires_at": 0, "scopes": scopes } } })
}

const FB_SCOPES: &[&str] = &["pages_show_list", "pages_read_engagement", "pages_manage_posts", "business_management"];
const ALL_SCOPES: &[&str] = &["pages_show_list", "pages_read_engagement", "pages_manage_posts", "instagram_basic", "instagram_content_publish"];

#[tokio::test]
async fn team_meta_app() {
    let Some(db) = TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping");
        return;
    };
    let url = db.url.clone();
    let task = tokio::spawn(async move {
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(3).connect(&url).await.expect("connect test db");
        let data = std::env::temp_dir().join(format!("oc-meta-apps-test-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&data).expect("data dir");
        let fake = Fake::start().await;
        // No server-wide Meta app: everything below runs through the team's own.
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

        let (ada, ada_c) = login(&s, "ada@example.test", "Ada").await;
        let (bob, bob_c) = login(&s, "bob@example.test", "Bob").await;
        let team = Uuid::new_v4();
        sqlx::query("INSERT INTO workspaces(id, name, personal) VALUES ($1, 'Unihop', false)").bind(team).execute(&pool).await.expect("ws");
        sqlx::query("INSERT INTO memberships(workspace_id, user_id, role) VALUES ($1, $2, 'owner'), ($1, $3, 'member')").bind(team).bind(ada).bind(bob).execute(&pool).await.expect("members");
        let personal: Uuid = sqlx::query_scalar("SELECT w.id FROM workspaces w JOIN memberships m ON m.workspace_id = w.id WHERE m.user_id = $1 AND w.personal").bind(ada).fetch_one(&pool).await.expect("personal");
        let app_url = format!("/api/workspaces/{team}/meta-app");

        // ---- reading and permissions
        let r = send(&s, "GET", &app_url, &bob_c, None).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(r.body["app"], Value::Null);
        assert_eq!(r.body["serverApp"], false);
        assert_eq!(r.body["canManage"], false);
        let body = json!({ "appId": APP, "appSecret": APP_SECRET, "graphVersion": "v27.0" });
        assert_eq!(send(&s, "PUT", &app_url, &bob_c, Some(body.clone())).await.status, 403, "a member can't set it");
        assert_eq!(send(&s, "GET", &format!("/api/workspaces/{personal}/meta-app"), &ada_c, None).await.status, 404, "teams only");
        assert_eq!(send(&s, "GET", &format!("/api/workspaces/{}/meta-app", Uuid::new_v4()), &ada_c, None).await.status, 404);
        assert_eq!(send(&s, "GET", &app_url, "", None).await.status, 401);

        // ---- Meta must accept the ID and secret before anything is stored
        fake.set_default(&format!("{GRAPH}/{APP}"), &json!({ "status": 400, "json": { "error": { "message": "Error validating client secret.", "type": "OAuthException", "code": 1 } } }));
        let r = send(&s, "PUT", &app_url, &ada_c, Some(body.clone())).await;
        assert_eq!((r.status, r.body["error"].as_str()), (400, Some("meta_refused")), "{:?}", r.body);
        assert!(r.body["message"].as_str().unwrap_or("").contains("Error validating client secret"), "{:?}", r.body);
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM workspace_social_apps").fetch_one(&pool).await.expect("count");
        assert_eq!(n, 0);
        let bad = send(&s, "PUT", &app_url, &ada_c, Some(json!({ "appId": "not-a-number", "appSecret": APP_SECRET }))).await;
        assert_eq!(bad.status, 400);

        fake.set_default(&format!("{GRAPH}/{APP}"), &json!({ "json": { "id": APP, "name": "UniHop Publisher" } }));
        let r = send(&s, "PUT", &app_url, &ada_c, Some(body.clone())).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(r.body["app"]["appId"], APP);
        assert_eq!(r.body["app"]["appName"], "UniHop Publisher");
        assert_eq!(r.body["app"]["graphVersion"], "v27.0");
        assert!(!r.body.to_string().contains(APP_SECRET), "the secret never leaves the server");
        let check = &fake.requests(&format!("{GRAPH}/{APP}"))[1];
        assert!(check["path"].as_str().unwrap_or("").contains(&format!("access_token={APP}%7C{APP_SECRET}")), "checked with the app token: {check}");
        let sealed: String = sqlx::query_scalar("SELECT app_secret FROM workspace_social_apps WHERE workspace_id = $1").bind(team).fetch_one(&pool).await.expect("sealed");
        assert_ne!(sealed, APP_SECRET);
        assert_eq!(oc_social::crypto::decrypt(&sealed, SECRET).expect("opens"), APP_SECRET);
        // Changing only the version keeps the stored secret.
        let r = send(&s, "PUT", &app_url, &ada_c, Some(json!({ "appId": APP, "graphVersion": "v27.0" }))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        let r = send(&s, "GET", &app_url, &bob_c, None).await;
        assert_eq!(r.body["app"]["appId"], APP);
        assert_eq!(r.body["canManage"], false);

        // ---- connecting with a pasted system-user token
        let accounts_url = format!("{app_url}/accounts");
        let connect = |provider: &str| json!({ "provider": provider, "token": SU_TOKEN });
        assert_eq!(send(&s, "POST", &accounts_url, &bob_c, Some(connect("facebook"))).await.status, 403);

        fake.set_default(&format!("{GRAPH}/debug_token"), &json!({ "json": { "data": { "is_valid": false, "error": { "message": "Session has expired" } } } }));
        let r = send(&s, "POST", &accounts_url, &ada_c, Some(connect("facebook"))).await;
        assert_eq!((r.status, r.body["error"].as_str()), (400, Some("connect_failed")));
        assert!(r.body["message"].as_str().unwrap_or("").contains("Session has expired"), "{:?}", r.body);

        fake.set_default(&format!("{GRAPH}/debug_token"), &debug_token("424242424242", FB_SCOPES));
        let r = send(&s, "POST", &accounts_url, &ada_c, Some(connect("facebook"))).await;
        assert!(r.body["message"].as_str().unwrap_or("").contains("belongs to Meta app 424242424242"), "{:?}", r.body);

        fake.set_default(&format!("{GRAPH}/debug_token"), &debug_token(APP, &["pages_show_list"]));
        let r = send(&s, "POST", &accounts_url, &ada_c, Some(connect("facebook"))).await;
        assert!(r.body["message"].as_str().unwrap_or("").contains("lacks pages_manage_posts, pages_read_engagement"), "{:?}", r.body);

        fake.set_default(&format!("{GRAPH}/debug_token"), &debug_token(APP, FB_SCOPES));
        fake.set_default(&format!("{GRAPH}/me/accounts"), &json!({ "json": { "data": [] } }));
        let r = send(&s, "POST", &accounts_url, &ada_c, Some(connect("facebook"))).await;
        assert!(r.body["message"].as_str().unwrap_or("").contains("assign the Page to the system user"), "{:?}", r.body);
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM social_accounts WHERE provider <> 'local'").fetch_one(&pool).await.expect("count");
        assert_eq!(n, 0, "nothing saved by a refused token");

        fake.set_default(&format!("{GRAPH}/me/accounts"), &json!({ "json": { "data": [{
            "id": "1438856472642415", "name": "UniHop", "access_token": "PAGE-TOKEN-UNIHOP",
            "instagram_business_account": { "id": "17841440769007273", "username": "unihop.eu", "name": "UniHop" },
        }] } }));
        let r = send(&s, "POST", &accounts_url, &ada_c, Some(connect("facebook"))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(r.body["accounts"][0]["displayName"], "UniHop");
        assert_eq!(r.body["accounts"][0]["connectedVia"], "system_user");
        assert_eq!(r.body["accounts"][0]["workspaceApp"], true);
        assert_eq!(r.body["accounts"][0]["workspaceId"], team.to_string().as_str());
        let dbg = fake.requests(&format!("{GRAPH}/debug_token")).pop().expect("debug_token");
        let path = dbg["path"].as_str().unwrap_or("");
        assert!(path.contains(&format!("access_token={APP}%7C{APP_SECRET}")) && path.contains(&format!("input_token={SU_TOKEN}")), "{path}");

        // Instagram needs its own permissions.
        let r = send(&s, "POST", &accounts_url, &ada_c, Some(connect("instagram"))).await;
        assert!(r.body["message"].as_str().unwrap_or("").contains("lacks instagram_basic, instagram_content_publish"), "{:?}", r.body);
        fake.set_default(&format!("{GRAPH}/debug_token"), &debug_token(APP, ALL_SCOPES));
        let r = send(&s, "POST", &accounts_url, &ada_c, Some(connect("instagram"))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(r.body["accounts"][0]["handle"], "unihop.eu");

        // Stored: the Page's own token (sealed), shared with the team, linked to its app; the system-user token is not kept.
        let rows: Vec<oc_social::store::AccountRow> = sqlx::query_as("SELECT * FROM social_accounts WHERE provider <> 'local' ORDER BY provider").fetch_all(&pool).await.expect("rows");
        assert_eq!(rows.len(), 2);
        let app_row: Uuid = sqlx::query_scalar("SELECT id FROM workspace_social_apps WHERE workspace_id = $1").bind(team).fetch_one(&pool).await.expect("app");
        for row in &rows {
            assert_eq!((row.workspace_id, row.social_app_id, row.connected_via.as_str()), (Some(team), Some(app_row), "system_user"));
            let t = oc_social::crypto::open_tokens(row.tokens.as_deref().expect("tokens"), SECRET).expect("open");
            assert_eq!(t.access_token, "PAGE-TOKEN-UNIHOP");
            assert!(!row.tokens.as_deref().unwrap_or("").contains(SU_TOKEN));
        }
        assert_eq!(rows[1].external_id, "17841440769007273");
        assert_eq!(rows[1].meta.0["pageId"], "1438856472642415");
        // Bob (a member) sees the team's accounts and can schedule to them.
        let r = send(&s, "GET", "/api/social/accounts", &bob_c, None).await;
        assert_eq!(r.body["accounts"].as_array().map(|a| a.iter().filter(|x| x["provider"] != "local").count()), Some(2), "{:?}", r.body);

        // ---- publishing uses the account's app (its ID, secret and Graph version), others the server's
        let base = oc_api::routes::social::http(&s);
        let h = oc_social::apps::http_for_account(&pool, &base, SECRET, &rows[0]).await.expect("http");
        assert_eq!(h.env("OC_FACEBOOK_CLIENT_ID").as_deref(), Some(APP));
        assert_eq!(h.env("OC_FACEBOOK_GRAPH_VERSION").as_deref(), Some("v27.0"));
        assert!(oc_social::adapter("facebook").expect("fb").configured(&h));
        let mut plain = rows[0].clone();
        plain.social_app_id = None;
        let h = oc_social::apps::http_for_account(&pool, &base, SECRET, &plain).await.expect("http");
        assert_eq!(h.env("OC_FACEBOOK_CLIENT_ID"), None);

        // ---- Facebook login for the team runs through its app
        let r = send(&s, "POST", "/api/social/connect/facebook", &ada_c, Some(json!({ "workspaceId": team.to_string() }))).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        let login_url = r.body["url"].as_str().unwrap_or("");
        assert!(login_url.starts_with("https://www.facebook.com/v27.0/dialog/oauth") && login_url.contains(&format!("client_id={APP}")), "{login_url}");
        let st: Option<Uuid> = sqlx::query_scalar("SELECT workspace_id FROM social_oauth_states ORDER BY created_at DESC LIMIT 1").fetch_one(&pool).await.expect("state");
        assert_eq!(st, Some(team));
        assert_eq!(send(&s, "POST", "/api/social/connect/facebook", &bob_c, Some(json!({ "workspaceId": team.to_string() }))).await.status, 403);
        // Without a team (and no server-wide app) Facebook is not configured, as before.
        assert_eq!(send(&s, "POST", "/api/social/connect/facebook", &ada_c, Some(json!({}))).await.status, 501);

        // ---- removing the app: its accounts need reconnecting
        assert_eq!(send(&s, "DELETE", &app_url, &bob_c, None).await.status, 403);
        let r = send(&s, "DELETE", &app_url, &ada_c, None).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        let after: Vec<(String, Option<Uuid>)> = sqlx::query_as("SELECT status, social_app_id FROM social_accounts WHERE provider <> 'local'").fetch_all(&pool).await.expect("after");
        assert_eq!(after, vec![("reauth".to_owned(), None), ("reauth".to_owned(), None)]);
        assert_eq!(send(&s, "GET", &app_url, &ada_c, None).await.body["app"], Value::Null);
        assert_eq!(send(&s, "DELETE", &app_url, &ada_c, None).await.status, 404);
        let r = send(&s, "POST", &accounts_url, &ada_c, Some(connect("facebook"))).await;
        assert_eq!(r.body["error"], "no_app");
        let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_events WHERE action LIKE 'social.%' ORDER BY id").fetch_all(&pool).await.expect("audit");
        assert_eq!(actions, ["social.app.save", "social.app.save", "social.connect", "social.connect", "social.app.delete"]);

        pool.close().await;
        let _ = std::fs::remove_dir_all(&data);
    })
    .await;
    db.drop_db().await;
    if let Err(e) = task {
        std::panic::resume_unwind(e.into_panic());
    }
}
