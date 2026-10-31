//! Job row shapes and worker semantics vs tests/golden/platform/jobs.json (rows produced by the TS queue).
//! Skipped unless OC_DATABASE_URL is set; runs in a throwaway database.
use std::time::Duration;

use oc_db::testing::TestDb;
use oc_jobs::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

fn golden() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/jobs.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// The same redacted shape the golden script records. `run_delay_ms` is relative to the enqueue time, or, for a
/// job rescheduled after a failed attempt, to the start of that attempt (`locked_at`), so the retry backoff is
/// checked without the worker's pick-up latency (machine-load dependent).
async fn shape(pool: &PgPool, id: Uuid, redact: &[String]) -> Value {
    let job = get_job(pool, id).await.unwrap().unwrap();
    let base = match job.status {
        JobStatus::Queued if job.attempts > 0 => job.locked_at.expect("a retried job has locked_at"),
        _ => job.created_at,
    };
    let delay_ms = (job.run_at - base).num_milliseconds() as f64;
    let mut v = serde_json::to_value(&job).unwrap();
    let o = v.as_object_mut().unwrap();
    for k in redact {
        if !o[k].is_null() {
            o.insert(k.clone(), json!("<redacted>"));
        }
    }
    o.insert("run_delay_ms".into(), json!(((delay_ms / 1000.0).round() * 1000.0) as i64));
    v
}

/// Compare with JS number semantics (0 == 0.0) via canonical JSON.
fn same(a: &Value, b: &Value) -> bool {
    oc_js::canonical(a) == oc_js::canonical(b)
}

#[tokio::test]
async fn rows_and_worker_match_ts() {
    let Some(t) = TestDb::create().await else {
        eprintln!("skipped: OC_DATABASE_URL not set");
        return;
    };
    let pool = &t.pool;
    let g = golden();
    let redact: Vec<String> = serde_json::from_value(g["redacted"].clone()).unwrap();
    sqlx::query("INSERT INTO users(id, email, name, color) VALUES ('33333333-3333-4333-8333-333333333333', 'ann@example.com', 'Ann', '#f06')").execute(pool).await.unwrap();

    for e in g["enqueued"].as_array().unwrap() {
        let o = &e["opts"];
        let opts = EnqueueOpts {
            owner_id: o["ownerId"].as_str().map(|s| s.parse().unwrap()),
            delay_ms: o["delayMs"].as_u64(),
            max_attempts: o["maxAttempts"].as_i64().map(|x| x as i32),
            priority: o["priority"].as_i64().map(|x| x as i32),
            label: o["label"].as_str().map(str::to_owned),
        };
        let id = enqueue(pool, e["queue"].as_str().unwrap(), &e["payload"], opts).await.unwrap();
        let got = shape(pool, id, &redact).await;
        assert!(same(&got, &e["row"]), "enqueue row\n got {got}\nwant {}", e["row"]);
    }

    // NOTIFY reaches a listener with the queue name as payload.
    let mut l = sqlx::postgres::PgListener::connect_with(pool).await.unwrap();
    l.listen(CHANNEL).await.unwrap();
    let ok = enqueue(pool, "golden-w", &json!({"do": "ok"}), EnqueueOpts::default()).await.unwrap();
    let n = tokio::time::timeout(Duration::from_secs(5), l.recv()).await.unwrap().unwrap();
    assert_eq!(n.payload(), "golden-w");
    let retry = enqueue(pool, "golden-w", &json!({"do": "fail"}), EnqueueOpts::default()).await.unwrap();
    let fin = enqueue(pool, "golden-w", &json!({"do": "fail"}), EnqueueOpts { max_attempts: Some(1), ..Default::default() }).await.unwrap();

    let worker = work(
        pool.clone(),
        "golden-w",
        |p: Value, ctx: JobContext| async move {
            if p["do"] == "ok" {
                ctx.progress(1.7, Some("Halfway")).await?;
                Ok(json!({"done": true, "n": 3}))
            } else {
                Err("boom: it failed".into())
            }
        },
        WorkOpts { concurrency: 2 },
    );
    let t0 = std::time::Instant::now();
    loop {
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE queue = 'golden-w' AND (status = 'running' OR (status = 'queued' AND attempts = 0))").fetch_one(pool).await.unwrap();
        if pending == 0 || t0.elapsed() > Duration::from_secs(20) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    worker.stop().await;
    for (name, id) in [("ok", ok), ("retry", retry), ("final", fin)] {
        let got = shape(pool, id, &redact).await;
        assert!(same(&got, &g["worked"][name]), "{name}\n got {got}\nwant {}", g["worked"][name]);
    }
    for (i, d) in g["retryDelaySecondsAfterAttempt"].as_array().unwrap().iter().enumerate() {
        assert_eq!(retry_delay_secs(i as i32 + 1), d.as_i64().unwrap());
    }
    t.drop_db().await;
}

#[tokio::test]
async fn heartbeat_upserts() {
    let Some(t) = TestDb::create().await else { return };
    let hb = heartbeat(t.pool.clone(), "rust-test", || json!({"v": 1}));
    // The first upsert happens at once (well before the 30 s period); wait for it rather than for a fixed time, which
    // failed on a heavily loaded machine.
    let t0 = std::time::Instant::now();
    let info: Option<Value> = loop {
        let row = sqlx::query_scalar("SELECT info FROM service_heartbeats WHERE service = 'rust-test'").fetch_optional(&t.pool).await.unwrap();
        if row.is_some() || t0.elapsed() > Duration::from_secs(20) {
            break row;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    hb.stop();
    assert_eq!(info, Some(json!({"v": 1})));
    t.drop_db().await;
}
