//! Several API replicas on one queue (plan 13 stage 3.4, gap B5): worker loops on separate pools, plus an
//! in-process `drain` racing them, run every job exactly once (`FOR UPDATE SKIP LOCKED`), with one attempt each.
//! Skipped unless OC_DATABASE_URL is set; runs in a throwaway database.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oc_db::testing::TestDb;
use oc_jobs::*;
use serde_json::{Value, json};

type Seen = Arc<Mutex<HashMap<String, usize>>>;

fn handler(seen: &Seen) -> impl Handler + Clone {
    let seen = seen.clone();
    move |p: Value, _ctx: JobContext| {
        let seen = seen.clone();
        async move {
            *seen.lock().unwrap().entry(p["n"].to_string()).or_default() += 1;
            tokio::time::sleep(Duration::from_millis(15)).await;
            Ok(json!(null))
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replicas_run_each_job_once() {
    let Some(t) = TestDb::create().await else {
        eprintln!("skipped: OC_DATABASE_URL not set");
        return;
    };
    const JOBS: usize = 60;
    for n in 0..JOBS {
        enqueue(&t.pool, "replicas", &json!({ "n": n }), EnqueueOpts::default()).await.unwrap();
    }
    let seen: Seen = Arc::default();
    // Three "processes": two worker loops (concurrency 2) and a kick-style drain.
    let a = oc_db::connect(&t.url).await.unwrap();
    let b = oc_db::connect(&t.url).await.unwrap();
    let wa = work(a.clone(), "replicas", handler(&seen), WorkOpts { concurrency: 2 });
    let wb = work(b.clone(), "replicas", handler(&seen), WorkOpts { concurrency: 2 });
    let drained = drain(&t.pool, "replicas", handler(&seen)).await;

    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let done: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE queue = 'replicas' AND status = 'done'").fetch_one(&t.pool).await.unwrap();
        if done as usize == JOBS {
            break;
        }
        assert!(Instant::now() < end, "only {done} of {JOBS} jobs done");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    wa.stop().await;
    wb.stop().await;

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), JOBS);
    assert!(seen.values().all(|&c| c == 1), "a job ran twice: {seen:?}");
    assert!(drained < JOBS, "the worker loops took part");
    let attempts: Vec<i32> = sqlx::query_scalar("SELECT DISTINCT attempts FROM jobs WHERE queue = 'replicas'").fetch_all(&t.pool).await.unwrap();
    assert_eq!(attempts, vec![1]);
    a.close().await;
    b.close().await;
    t.drop_db().await;
}
