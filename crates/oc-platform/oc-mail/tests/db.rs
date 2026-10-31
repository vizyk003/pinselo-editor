//! Outbox + job protocol vs tests/golden/platform/mail_outbox.json (rows produced by TS `sendEmail`).
//! Skipped unless OC_DATABASE_URL is set; runs in a throwaway database.
use oc_db::testing::TestDb;
use oc_mail::*;
use serde_json::Value;
use uuid::Uuid;

#[tokio::test]
async fn outbox_rows_match_ts() {
    let Some(t) = TestDb::create().await else {
        eprintln!("skipped: OC_DATABASE_URL not set");
        return;
    };
    let pool = &t.pool;
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/mail_outbox.json");
    let g: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let user_id: Uuid = g["user"]["id"].as_str().unwrap().parse().unwrap();
    let email = g["user"]["email"].as_str().unwrap();
    sqlx::query("INSERT INTO users(id, email, name, color, locale) VALUES ($1,$2,'Ann','#f06',$3)")
        .bind(user_id)
        .bind(email.to_lowercase())
        .bind(g["user"]["locale"].as_str().unwrap())
        .execute(pool)
        .await
        .unwrap();
    let tpl: Template = serde_json::from_value(g["template"].clone()).unwrap();
    let id = send_email(pool, email, &tpl, None).await.unwrap().unwrap();
    let (to, subject, html, text, category, status, error): (String, String, String, String, String, String, Option<String>) =
        sqlx::query_as("SELECT to_email, subject, html, text, category, status, error FROM email_outbox WHERE id = $1").bind(id).fetch_one(pool).await.unwrap();
    let row = &g["row"];
    assert_eq!(to, row["to_email"].as_str().unwrap());
    assert_eq!(subject, row["subject"].as_str().unwrap());
    assert_eq!(html, row["html"].as_str().unwrap());
    assert_eq!(text, row["text"].as_str().unwrap());
    assert_eq!(category, row["category"].as_str().unwrap());
    assert_eq!(status, row["status"].as_str().unwrap());
    assert_eq!(error, None);
    let (queue, payload, max_attempts): (String, sqlx::types::Json<Value>, i32) =
        sqlx::query_as("SELECT queue, payload, max_attempts FROM jobs WHERE queue = 'email'").fetch_one(pool).await.unwrap();
    assert_eq!(queue, g["job"]["queue"].as_str().unwrap());
    assert_eq!(max_attempts as i64, g["job"]["max_attempts"].as_i64().unwrap());
    assert_eq!(payload.0.as_object().unwrap().keys().collect::<Vec<_>>(), vec!["id"]);
    assert_eq!(payload.0["id"].as_str().unwrap(), id.to_string());

    sqlx::query("INSERT INTO email_preferences(user_id, shares) VALUES ($1, false)").bind(user_id).execute(pool).await.unwrap();
    assert_eq!(send_email(pool, email, &tpl, Some(user_id)).await.unwrap().is_none(), g["suppressedReturnsNull"].as_bool().unwrap());
    assert_eq!(send_email(pool, email, &Template::Magic { url: "https://x".into() }, Some(user_id)).await.unwrap().is_some(), g["securityIgnoresPrefs"].as_bool().unwrap());

    // Dev capture: without OC_SMTP_URL the message stays in the outbox, marked sent.
    // SAFETY: test-only; no other thread in this test binary reads OC_SMTP_URL concurrently.
    unsafe { std::env::remove_var("OC_SMTP_URL") };
    deliver(pool, id).await.unwrap();
    let (status, has_sent_at): (String, bool) = sqlx::query_as("SELECT status, sent_at IS NOT NULL FROM email_outbox WHERE id = $1").bind(id).fetch_one(pool).await.unwrap();
    assert_eq!(status, g["devDelivery"]["status"].as_str().unwrap());
    assert_eq!(has_sent_at, g["devDelivery"]["has_sent_at"].as_bool().unwrap());
    t.drop_db().await;
}
