//! The reconnecting LISTEN connection (oc_db::listener) against a throwaway database: notifications arrive, a
//! backend killed with pg_terminate_backend is replaced (re-LISTEN, on_reconnect), close() is idempotent, and a
//! listener that never connects fails `ready()` on close. Skipped without OC_DATABASE_URL.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oc_db::testing::TestDb;
use oc_db::{ListenerOptions, ListenerTiming, create_listener};

fn fast() -> ListenerTiming {
    ListenerTiming {
        min_delay: Duration::from_millis(50),
        max_delay: Duration::from_millis(400),
        connect: Duration::from_secs(3),
        query: Duration::from_secs(3),
        close: Duration::from_secs(2),
        ping_every: Duration::from_millis(300),
    }
}

async fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnects_and_listens_again_after_the_backend_is_killed() {
    let Some(t) = TestDb::create().await else {
        eprintln!("skipped: OC_DATABASE_URL not set");
        return;
    };
    let got = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let reconnects = Arc::new(AtomicUsize::new(0));
    let (g, r) = (got.clone(), reconnects.clone());
    let l = create_listener(
        (*t.pool.connect_options()).clone(),
        &["oc_test_a", "oc_test_b"],
        move |n| g.lock().unwrap().push((n.channel().to_owned(), n.payload().to_owned())),
        ListenerOptions::named("oc-test-listener")
            .on_reconnect(move || {
                r.fetch_add(1, Ordering::SeqCst);
            })
            .timing(fast()),
    );
    l.ready().await.unwrap();
    assert_eq!(l.connects(), 1);
    sqlx::query("SELECT pg_notify('oc_test_a', 'one'), pg_notify('oc_test_b', 'two')").execute(&t.pool).await.unwrap();
    wait_for("first notifications", || got.lock().unwrap().len() == 2).await;
    assert_eq!(*got.lock().unwrap(), vec![("oc_test_a".to_owned(), "one".to_owned()), ("oc_test_b".to_owned(), "two".to_owned())]);

    // Kill the LISTEN backend; the listener must notice, reconnect and LISTEN on both channels again.
    let killed: Vec<bool> = sqlx::query_scalar("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = 'oc-test-listener' AND datname = current_database()")
        .fetch_all(&t.pool)
        .await
        .unwrap();
    assert_eq!(killed, vec![true]);
    wait_for("reconnect", || reconnects.load(Ordering::SeqCst) == 1).await;
    assert_eq!(l.connects(), 2);
    let listening: Vec<String> = sqlx::query_scalar("SELECT application_name FROM pg_stat_activity WHERE application_name = 'oc-test-listener' AND datname = current_database()")
        .fetch_all(&t.pool)
        .await
        .unwrap();
    assert_eq!(listening.len(), 1, "exactly one LISTEN connection after the reconnect");
    sqlx::query("SELECT pg_notify('oc_test_b', 'after')").execute(&t.pool).await.unwrap();
    wait_for("notification after reconnect", || got.lock().unwrap().iter().any(|(c, p)| c == "oc_test_b" && p == "after")).await;

    // The ping keeps a healthy connection (several ping periods pass without a reconnect).
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(l.connects(), 2);

    l.close().await;
    l.close().await;
    assert!(!l.is_running());
    l.ready().await.unwrap();
    t.drop_db().await;
}

#[tokio::test]
async fn close_before_the_first_listen_fails_ready() {
    // Nothing listens on port 1: every attempt fails and the listener keeps retrying until closed.
    let opts: sqlx::postgres::PgConnectOptions = "postgres://nobody@127.0.0.1:1/none".parse().unwrap();
    let l = create_listener(opts, &["oc_test"], |_| {}, ListenerOptions::named("oc-test-unreachable").timing(fast()));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(l.connects(), 0);
    l.close().await;
    assert!(l.ready().await.is_err());
}
