//! The collab LISTEN connection survives a killed backend (services/collab/src/main.ts with @oc/db createListener):
//! it reconnects, LISTENs on oc_revoke / oc_doc_update again, and refuses tickets issued before the reconnect (less
//! the 30 s skew allowance), because revocations sent while it was down are lost. Skipped without OC_DATABASE_URL.
use std::time::Duration;

use oc_collab::server::{RECONNECT_SKEW_MS, Server, Settings};
use oc_db::testing::TestDb;

const DESIGN: &str = "abcdefghijklmnop";
const ROOM: &str = "design:abcdefghijklmnop";

fn ticket(settings: &Settings, sub: &str, iat: i64) -> String {
    let claims = oc_auth::TicketClaims {
        sub: sub.into(),
        name: "T".into(),
        color: "#0c7a80".into(),
        aud: oc_auth::Audience::Collab,
        design: DESIGN.into(),
        role: oc_auth::Role::Edit,
        brand_locked: None,
    };
    oc_auth::sign_ticket_at(&settings.secret, &claims, 300, iat)
}

async fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_listens_again_and_refuses_older_tickets() {
    let Some(t) = TestDb::create().await else {
        eprintln!("skipped: OC_DATABASE_URL not set");
        return;
    };
    let settings = Settings::from_config(oc_config::config());
    let server = Server::new(t.pool.clone(), settings.clone());
    let timing = oc_db::ListenerTiming { min_delay: Duration::from_millis(50), ping_every: Duration::from_millis(500), ..Default::default() };
    let listener = oc_collab::listen_notifications_with(server.clone(), timing).await.unwrap();

    let now = oc_js::now_ms();
    let old = ticket(&settings, "u-old", now - RECONNECT_SKEW_MS - 5_000);
    let recent = ticket(&settings, "u-recent", now - 1_000);
    assert!(server.authenticate(ROOM, &old).is_ok(), "accepted before any reconnect");

    let killed: Vec<bool> = sqlx::query_scalar("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = 'collab' AND datname = current_database()")
        .fetch_all(&t.pool)
        .await
        .unwrap();
    assert_eq!(killed, vec![true]);
    wait_for("the reconnect", || listener.connects() == 2).await;
    assert_eq!(server.authenticate(ROOM, &old).unwrap_err(), "revoked");
    assert!(server.authenticate(ROOM, &recent).is_ok(), "tickets within the skew allowance still pass");

    // oc_revoke reaches the new connection.
    let at = oc_js::now_ms();
    sqlx::query("SELECT pg_notify('oc_revoke', $1)").bind(serde_json::json!({ "designId": DESIGN, "userId": "u-recent", "at": at }).to_string()).execute(&t.pool).await.unwrap();
    wait_for("the revocation", || server.authenticate(ROOM, &recent).is_err()).await;
    assert!(server.authenticate(ROOM, &ticket(&settings, "u-recent", at)).is_ok(), "a ticket fetched after the revocation passes");

    listener.close().await;
    t.drop_db().await;
}
