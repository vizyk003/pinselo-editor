//! Sheets, whiteboards and email designs: stored ones stay readable, nothing new starts on those surfaces, and a
//! sheet can't be copied or exported. Skipped without `OC_DATABASE_URL`.

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

async fn send(s: &AppState, url: &str, cookie: Option<&str>, body: Value) -> (u16, String, Value) {
    let mut req = axum::http::Request::builder().method("POST").uri(url).header("x-forwarded-for", "198.51.100.7").header("content-type", "application/json");
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    let res = oc_api::app(s.clone()).oneshot(req.body(Body::from(body.to_string())).expect("request")).await.expect("response");
    let status = res.status().as_u16();
    let cookie = res.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()?.split(';').next().map(str::to_owned)).collect::<Vec<_>>().join("; ");
    let raw = res.into_body().collect().await.expect("body").to_bytes();
    (status, cookie, serde_json::from_slice(&raw).unwrap_or(Value::Null))
}

#[tokio::test]
async fn nothing_new_starts_on_a_retired_surface() {
    let Some(db) = TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping");
        return;
    };
    let url = db.url.clone();
    let task = tokio::spawn(async move {
        let pool = sqlx::postgres::PgPoolOptions::new().max_connections(3).connect(&url).await.expect("connect test db");
        let data = std::env::temp_dir().join(format!("oc-retired-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&data).expect("data dir");
        let fake = Fake::start().await;
        let env = HashMap::from(
            [
                ("NODE_ENV", "development"),
                ("OC_DATABASE_URL", url.as_str()),
                ("OC_DATA_DIR", data.to_str().expect("data dir")),
                ("OC_SECRET", SECRET),
                ("OC_WEB_ORIGIN", WEB),
                ("OC_API_URL", API),
                ("OC_RENDER_URL", fake.base.as_str()),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
        );
        let clock: oc_api::state::Clock = Arc::new(|| CLOCK);
        let s = AppState::new(oc_api::AppOptions { env, pool: pool.clone(), store: None, clock: Some(clock) }).await.expect("app state");

        let signup = json!({ "email": "ada@example.test", "password": "correct horse battery 42", "name": "Ada", "birthYear": 1990, "acceptTerms": true });
        let (status, cookie, body) = send(&s, "/api/auth/signup", None, signup).await;
        assert!(status < 300, "signup: {status} {body}");
        let cookie = Some(cookie.as_str());

        // The removed presets no longer pick a surface: an old client gets an ordinary graphic.
        for preset in ["sheet", "whiteboard", "email"] {
            let (status, _, body) = send(&s, "/api/designs", cookie, json!({ "preset": preset })).await;
            assert!(status < 300, "{preset}: {status} {body}");
            let surface: String = sqlx::query_scalar("SELECT surface FROM designs WHERE id = $1").bind(body["id"].as_str().unwrap()).fetch_one(&pool).await.unwrap();
            assert_eq!(surface, "graphic", "{preset}");
        }

        // A template left in the catalogue on a retired surface can't start a design.
        for (id, surface) in [("old-sheet", "sheet"), ("old-board", "whiteboard"), ("old-email", "email")] {
            sqlx::query("INSERT INTO templates(id, title, category, preset, width, height, doc, published) VALUES ($1, 'T', 'c', 'p', 1, 1, $2, true)")
                .bind(id)
                .bind(sqlx::types::Json(json!({ "meta": { "surface": surface } })))
                .execute(&pool)
                .await
                .unwrap();
            let (status, _, body) = send(&s, "/api/designs", cookie, json!({ "templateId": id })).await;
            assert_eq!((status, body["error"].as_str()), (422, Some("unsupported_surface")), "{id}");
        }

        // A stored sheet stays where it is, but can't be copied or exported.
        let (_, _, body) = send(&s, "/api/designs", cookie, json!({ "preset": "doc" })).await;
        let id = body["id"].as_str().unwrap().to_owned();
        let doc = oc_schema::factory::empty_design(&oc_schema::factory::MetaInit { w: 100.0, h: 100.0, surface: Some(oc_schema::Surface::Sheet), ..Default::default() });
        let state = oc_crdt::encode_doc_value(&oc_schema::js::to_value(&doc).unwrap());
        sqlx::query("UPDATE designs SET surface = 'sheet' WHERE id = $1").bind(&id).execute(&pool).await.unwrap();
        sqlx::query("UPDATE doc_state SET state = $2 WHERE design_id = $1").bind(&id).bind(&state).execute(&pool).await.unwrap();
        let (status, _, body) = send(&s, &format!("/api/designs/{id}/export"), cookie, json!({ "format": "png" })).await;
        assert_eq!((status, body["error"].as_str()), (422, Some("sheet_unsupported")), "{body}");
        let (status, _, body) = send(&s, &format!("/api/designs/{id}/copy"), cookie, json!({})).await;
        assert_eq!((status, body["error"].as_str()), (422, Some("unsupported_surface")), "{body}");
        let kept: Vec<u8> = sqlx::query_scalar("SELECT state FROM doc_state WHERE design_id = $1").bind(&id).fetch_one(&pool).await.unwrap();
        assert_eq!(kept, state);

        // The export format is gone.
        let (status, _, _) = send(&s, &format!("/api/designs/{id}/export"), cookie, json!({ "format": "html-email" })).await;
        assert_eq!(status, 400);

        pool.close().await;
        let _ = std::fs::remove_dir_all(&data);
    })
    .await;
    db.drop_db().await;
    if let Err(e) = task {
        std::panic::resume_unwind(e.into_panic());
    }
}
