//! work(): one shared LISTEN connection per process and database (port of packages/jobs/src/jobs.db.test.ts).
//! Loops on different queues share it, a NOTIFY wakes only its queue's loop, a reconnect after pg_terminate_backend
//! wakes every loop, stopping one loop leaves the others alone, and the connection closes with the last loop.
//! Skipped unless OC_DATABASE_URL is set; runs in a throwaway database.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oc_db::testing::TestDb;
use oc_jobs::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn until(what: &str, ms: u64, mut cond: impl AsyncFnMut() -> bool) {
    let end = Instant::now() + Duration::from_millis(ms);
    while !cond().await {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// This database's job listeners (the shared connection is named oc-jobs), as their last query.
async fn listeners(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT query FROM pg_stat_activity WHERE application_name = 'oc-jobs' AND datname = current_database()").fetch_all(pool).await.unwrap()
}

type Handled = Arc<Mutex<HashMap<String, Instant>>>;

fn start(pool: &PgPool, queue: &str, handled: &Handled) -> Worker {
    let (q, h) = (queue.to_owned(), handled.clone());
    work(
        pool.clone(),
        queue,
        move |p: Value, _ctx: JobContext| {
            let (q, h) = (q.clone(), h.clone());
            async move {
                h.lock().unwrap().insert(format!("{q}:{}", p["id"].as_str().unwrap()), Instant::now());
                Ok(json!(null))
            }
        },
        WorkOpts::default(),
    )
}

/// Enqueues one job and returns how long it took to reach the handler (the poll alone would take up to 5 s).
async fn latency(pool: &PgPool, queue: &str, handled: &Handled) -> Duration {
    let id = uuid::Uuid::new_v4().to_string();
    let t0 = Instant::now();
    enqueue(pool, queue, &json!({ "id": id }), EnqueueOpts::default()).await.unwrap();
    let key = format!("{queue}:{id}");
    until(&format!("a job on {queue}"), 4000, async || handled.lock().unwrap().contains_key(&key)).await;
    handled.lock().unwrap()[&key] - t0
}

async fn idle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shares_one_connection_and_wakes_the_right_loop() {
    let Some(t) = TestDb::create().await else {
        eprintln!("skipped: OC_DATABASE_URL not set");
        return;
    };
    let pool = &t.pool;
    let handled: Handled = Arc::default();
    let listening = async || {
        until("the shared listener", 5000, async || listeners(pool).await == vec![r#"LISTEN "oc_jobs""#.to_owned()]).await;
    };
    let fast = Duration::from_millis(1500);

    let a = start(pool, "shared-a", &handled);
    let b = start(pool, "shared-b", &handled);
    listening().await;
    idle().await;
    assert!(latency(pool, "shared-a", &handled).await < fast);
    assert!(latency(pool, "shared-b", &handled).await < fast);
    assert_eq!(listeners(pool).await.len(), 1);

    // A job enqueued while the connection is down loses its NOTIFY; the reconnect (after ~1 s) wakes every loop, well
    // before the 5 s poll that started when B's last job finished.
    let killed: Vec<bool> = sqlx::query_scalar("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = 'oc-jobs' AND datname = current_database()")
        .fetch_all(pool)
        .await
        .unwrap();
    assert_eq!(killed, vec![true]);
    assert!(latency(pool, "shared-b", &handled).await < Duration::from_secs(3));
    listening().await;

    // Stopping A leaves B and the shared connection alone.
    a.stop().await;
    assert!(latency(pool, "shared-b", &handled).await < fast);
    assert_eq!(listeners(pool).await.len(), 1);

    // A newer loop on A's queue gets its own registration.
    let a2 = start(pool, "shared-a", &handled);
    idle().await;
    assert!(latency(pool, "shared-a", &handled).await < fast);

    b.stop().await;
    assert_eq!(listeners(pool).await.len(), 1);
    assert!(shared_listener_active(pool));
    a2.stop().await;
    assert!(!shared_listener_active(pool));
    until("the listener to close with the last loop", 3000, async || listeners(pool).await.is_empty()).await;

    // The next loop opens it again.
    let c = start(pool, "shared-c", &handled);
    listening().await;
    idle().await;
    assert!(latency(pool, "shared-c", &handled).await < fast);
    c.stop().await;
    until("the listener to close again", 3000, async || listeners(pool).await.is_empty()).await;
    t.drop_db().await;
}
