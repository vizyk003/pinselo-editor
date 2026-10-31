//! Durable job queue on Postgres (PL-061 Postgres fallback / ADR-0013): rows claimed with
//! `FOR UPDATE SKIP LOCKED`, woken by `LISTEN oc_jobs` / `pg_notify('oc_jobs', queue)` (one reconnecting LISTEN
//! connection per process and database, shared by every worker loop; a 5 s poll is the backstop), retried with
//! backoff (`2^attempts * 5` seconds), progress persisted. SQL statements, labels, defaults and the
//! notification channel are identical to `packages/jobs`, so TS and Rust workers/producers interoperate
//! on the same `jobs` table.

use std::future::Future;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::PgPool;
use sqlx::types::Json;
use tokio::sync::Notify;
use uuid::Uuid;

/// The NOTIFY channel.
pub const CHANNEL: &str = "oc_jobs";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

/// A `jobs` row (`SELECT *`).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    pub queue: String,
    pub payload: Json<Value>,
    pub status: JobStatus,
    pub priority: i32,
    pub run_at: DateTime<Utc>,
    pub attempts: i32,
    pub max_attempts: i32,
    pub progress: f32,
    pub label: Option<String>,
    pub result: Option<Json<Value>>,
    pub error: Option<String>,
    pub owner_id: Option<Uuid>,
    pub locked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// `enqueue` options; `Default` matches the TS defaults (no delay, 3 attempts, priority 0, label "Queued").
#[derive(Debug, Clone, Default)]
pub struct EnqueueOpts {
    pub owner_id: Option<Uuid>,
    pub delay_ms: Option<u64>,
    pub max_attempts: Option<i32>,
    pub priority: Option<i32>,
    pub label: Option<String>,
}

pub async fn enqueue<P: Serialize + ?Sized>(pool: &PgPool, queue: &str, payload: &P, opts: EnqueueOpts) -> Result<Uuid, sqlx::Error> {
    let id = oc_db::uuid();
    sqlx::query(
        "INSERT INTO jobs(id, queue, payload, owner_id, run_at, max_attempts, priority, label) VALUES ($1,$2,$3,$4, now() + $5::text::double precision * interval '1 millisecond', $6, $7, $8)",
    )
    .bind(id)
    .bind(queue)
    .bind(Json(payload))
    .bind(opts.owner_id)
    .bind(opts.delay_ms.unwrap_or(0).to_string())
    .bind(opts.max_attempts.unwrap_or(3))
    .bind(opts.priority.unwrap_or(0))
    .bind(opts.label.unwrap_or_else(|| "Queued".into()))
    .execute(pool)
    .await?;
    sqlx::query("SELECT pg_notify($1, $2)").bind(CHANNEL).bind(queue).execute(pool).await?;
    Ok(id)
}

pub async fn get_job(pool: &PgPool, id: Uuid) -> Result<Option<Job>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM jobs WHERE id = $1").bind(id).fetch_optional(pool).await
}

/// Handed to a handler alongside its payload.
#[derive(Clone)]
pub struct JobContext {
    pub job: Job,
    pool: PgPool,
}

impl JobContext {
    /// Persist progress (clamped to 0..=1) and optionally a label.
    pub async fn progress(&self, fraction: f64, label: Option<&str>) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE jobs SET progress = $2, label = COALESCE($3, label) WHERE id = $1")
            .bind(self.job.id)
            .bind(fraction.clamp(0.0, 1.0) as f32)
            .bind(label)
            .execute(&self.pool)
            .await
            .map(|_| ())
    }
}

pub type HandlerError = Box<dyn std::error::Error + Send + Sync>;
pub type HandlerResult = Result<Value, HandlerError>;

/// A job handler. The `Err` display string becomes the job's `error` (TS: `error.message`).
pub trait Handler: Send + Sync + 'static {
    fn call(&self, payload: Value, ctx: JobContext) -> BoxFuture<'static, HandlerResult>;
}

impl<F, Fut> Handler for F
where
    F: Fn(Value, JobContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult> + Send + 'static,
{
    fn call(&self, payload: Value, ctx: JobContext) -> BoxFuture<'static, HandlerResult> {
        Box::pin(self(payload, ctx))
    }
}

/// Wrap a handler that takes a typed payload; a payload that does not deserialize fails the attempt.
pub fn typed<P, F, Fut, R>(f: F) -> impl Handler
where
    P: DeserializeOwned + Send + 'static,
    R: Serialize,
    F: Fn(P, JobContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<R, HandlerError>> + Send + 'static,
{
    let f = Arc::new(f);
    move |payload: Value, ctx: JobContext| {
        let f = f.clone();
        async move {
            let p: P = serde_json::from_value(payload)?;
            Ok(serde_json::to_value(f(p, ctx).await?)?)
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WorkOpts {
    pub concurrency: usize,
}

impl Default for WorkOpts {
    fn default() -> Self {
        WorkOpts { concurrency: 1 }
    }
}

// ------------------------------------------------------------------ the shared LISTEN connection

/// Wakers by queue name for one database.
type Wakers = Arc<Mutex<HashMap<String, Vec<(u64, Arc<Notify>)>>>>;

struct Shared {
    listener: Arc<oc_db::Listener>,
    wakers: Wakers,
}

/// One LISTEN connection per process and database (plan 12 §A2), shared by every [`work`] loop: it reconnects on
/// its own and then wakes every loop, since notifications sent while it was down are lost (the 5 s poll in each loop
/// is the backstop). Closed when the last loop of that database stops. Keyed by database so tests with several
/// throwaway databases in one process stay separate.
fn shared() -> &'static Mutex<HashMap<String, Shared>> {
    static S: OnceLock<Mutex<HashMap<String, Shared>>> = OnceLock::new();
    S.get_or_init(Mutex::default)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn db_key(pool: &PgPool) -> String {
    let o = pool.connect_options();
    format!("{}:{}:{}:{}", o.get_host(), o.get_port(), o.get_database().unwrap_or(""), o.get_username())
}

fn wake_all(w: &Wakers) {
    lock(w).values().flatten().for_each(|(_, n)| n.notify_one());
}

/// A worker loop's registration with the shared listener.
struct Registration {
    key: String,
    queue: String,
    id: u64,
    done: AtomicBool,
}

fn register(pool: &PgPool, queue: &str, wake: Arc<Notify>) -> Registration {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let key = db_key(pool);
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    let mut all = lock(shared());
    // A listener whose runtime went away (a finished test) is replaced.
    if all.get(&key).is_some_and(|s| !s.listener.is_running()) {
        all.remove(&key);
    }
    let entry = all.entry(key.clone()).or_insert_with(|| {
        let wakers: Wakers = Arc::default();
        let (on_n, on_r) = (wakers.clone(), wakers.clone());
        let listener = oc_db::create_listener(
            (*pool.connect_options()).clone(),
            &[CHANNEL],
            move |n| {
                if let Some(v) = lock(&on_n).get(n.payload()) {
                    v.iter().for_each(|(_, w)| w.notify_one());
                }
            },
            oc_db::ListenerOptions::named("oc-jobs").on_reconnect(move || wake_all(&on_r)),
        );
        Shared { listener: Arc::new(listener), wakers }
    });
    lock(&entry.wakers).entry(queue.to_owned()).or_default().push((id, wake));
    Registration { key, queue: queue.to_owned(), id, done: AtomicBool::new(false) }
}

impl Registration {
    /// Idempotent, and it only touches its own registration: a second call must not remove a newer loop's entry for
    /// the same queue, or close a listener that other loops still share.
    async fn unregister(&self) {
        if self.done.swap(true, Ordering::SeqCst) {
            return;
        }
        let closing = {
            let mut all = lock(shared());
            let Some(s) = all.get(&self.key) else { return };
            let empty = {
                let mut w = lock(&s.wakers);
                if let Some(v) = w.get_mut(&self.queue) {
                    v.retain(|(id, _)| *id != self.id);
                    if v.is_empty() {
                        w.remove(&self.queue);
                    }
                }
                w.is_empty()
            };
            if empty { all.remove(&self.key).map(|s| s.listener) } else { None }
        };
        if let Some(l) = closing {
            l.close().await;
        }
    }
}

/// Whether this process holds a shared `oc_jobs` listener for `pool`'s database (tests).
pub fn shared_listener_active(pool: &PgPool) -> bool {
    lock(shared()).contains_key(&db_key(pool))
}

/// A running worker loop; [`Worker::stop`] ends it (in-flight jobs finish on their own).
pub struct Worker {
    stopped: Arc<AtomicBool>,
    wake: Arc<Notify>,
    registration: Registration,
    lp: tokio::task::JoinHandle<()>,
}

impl Worker {
    pub async fn stop(self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.wake.notify_one();
        self.registration.unregister().await;
        let _ = self.lp.await;
    }
}

async fn claim(pool: &PgPool, queue: &str) -> Result<Option<Job>, sqlx::Error> {
    let mut t = pool.begin().await?;
    let job: Option<Job> = sqlx::query_as(
        "SELECT * FROM jobs WHERE queue = $1 AND status = 'queued' AND run_at <= now()
         ORDER BY priority DESC, run_at LIMIT 1 FOR UPDATE SKIP LOCKED",
    )
    .bind(queue)
    .fetch_optional(&mut *t)
    .await?;
    let Some(mut job) = job else {
        t.commit().await?;
        return Ok(None);
    };
    sqlx::query("UPDATE jobs SET status = 'running', attempts = attempts + 1, locked_at = now(), label = 'Running' WHERE id = $1")
        .bind(job.id)
        .execute(&mut *t)
        .await?;
    t.commit().await?;
    job.attempts += 1;
    Ok(Some(job))
}

async fn run_one(pool: PgPool, job: Job, handler: Arc<dyn Handler>) {
    let id = job.id;
    let (attempts, max_attempts) = (job.attempts, job.max_attempts);
    let payload = job.payload.0.clone();
    let ctx = JobContext { job, pool: pool.clone() };
    let res = match tokio::spawn(handler.call(payload, ctx)).await {
        Ok(r) => r,
        Err(e) => Err(e.to_string().into()),
    };
    let q = match res {
        Ok(result) => sqlx::query("UPDATE jobs SET status = 'done', progress = 1, label = 'Done', result = $2, finished_at = now(), error = NULL WHERE id = $1")
            .bind(id)
            .bind(Json(result))
            .execute(&pool)
            .await,
        Err(e) => {
            let msg = e.to_string();
            if attempts < max_attempts {
                sqlx::query("UPDATE jobs SET status = 'queued', error = $2, run_at = now() + ($3 || ' seconds')::interval, label = 'Retrying' WHERE id = $1")
                    .bind(id)
                    .bind(msg)
                    .bind(retry_delay_secs(attempts).to_string())
                    .execute(&pool)
                    .await
            } else {
                sqlx::query("UPDATE jobs SET status = 'failed', error = $2, finished_at = now(), label = 'Failed' WHERE id = $1").bind(id).bind(msg).execute(&pool).await
            }
        }
    };
    if let Err(e) = q {
        tracing::error!(job = %id, "job bookkeeping failed: {e}");
    }
}

/// Backoff before the next attempt, after `attempts` attempts: `2^attempts * 5` seconds.
pub fn retry_delay_secs(attempts: i32) -> i64 {
    2_i64.saturating_pow(attempts.max(0) as u32).saturating_mul(5)
}

/// Run every due job of `queue`, one after another, in the calling task: the same claim and bookkeeping as
/// [`work`] without a loop that waits for more (a producer that drains its own queue in-process when no worker loop
/// runs, e.g. the API under the parity harness). Returns how many jobs ran.
pub async fn drain(pool: &PgPool, queue: &str, handler: impl Handler) -> usize {
    let handler: Arc<dyn Handler> = Arc::new(handler);
    let mut n = 0;
    while let Ok(Some(job)) = claim(pool, queue).await {
        run_one(pool.clone(), job, handler.clone()).await;
        n += 1;
    }
    n
}

/// Run a worker loop for one queue.
pub fn work(pool: PgPool, queue: impl Into<String>, handler: impl Handler, opts: WorkOpts) -> Worker {
    let queue: String = queue.into();
    let handler: Arc<dyn Handler> = Arc::new(handler);
    let stopped = Arc::new(AtomicBool::new(false));
    let wake = Arc::new(Notify::new());
    let running = Arc::new(AtomicUsize::new(0));

    let registration = register(&pool, &queue, wake.clone());

    let lp = {
        let (stopped, wake) = (stopped.clone(), wake.clone());
        tokio::spawn(async move {
            // Recover jobs orphaned by a crashed worker (running for > 15 min).
            let _ = sqlx::query("UPDATE jobs SET status = 'queued', label = 'Recovered' WHERE queue = $1 AND status = 'running' AND locked_at < now() - interval '15 minutes'")
                .bind(&queue)
                .execute(&pool)
                .await;
            while !stopped.load(Ordering::SeqCst) {
                if running.load(Ordering::SeqCst) < opts.concurrency.max(1)
                    && let Ok(Some(job)) = claim(&pool, &queue).await
                {
                    running.fetch_add(1, Ordering::SeqCst);
                    let (pool, handler, running, wake) = (pool.clone(), handler.clone(), running.clone(), wake.clone());
                    tokio::spawn(async move {
                        run_one(pool, job, handler).await;
                        running.fetch_sub(1, Ordering::SeqCst);
                        wake.notify_one();
                    });
                    continue;
                }
                let _ = tokio::time::timeout(Duration::from_secs(5), wake.notified()).await;
            }
        })
    };

    Worker { stopped, wake, registration, lp }
}

/// A scheduled task handle; dropping it does not cancel — call [`Cron::stop`].
pub struct Cron(tokio::task::JoinHandle<()>);

impl Cron {
    pub fn stop(&self) {
        self.0.abort();
    }
}

/// Schedule a recurring job every `period` (first run after one period, like `setInterval`); errors are logged.
pub fn every<F, Fut, E>(name: impl Into<String>, period: Duration, f: F) -> Cron
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), E>> + Send + 'static,
    E: std::fmt::Display,
{
    let name = name.into();
    Cron(tokio::spawn(async move {
        let mut iv = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        loop {
            iv.tick().await;
            let fut = f();
            let name = name.clone();
            tokio::spawn(async move {
                if let Err(e) = fut.await {
                    eprintln!("[cron {name}] {e}");
                }
            });
        }
    }))
}

/// Service liveness for the public status page: upsert now and every 30 s.
pub fn heartbeat<F>(pool: PgPool, service: impl Into<String>, info: F) -> Cron
where
    F: Fn() -> Value + Send + Sync + 'static,
{
    let service = service.into();
    Cron(tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_secs(30));
        loop {
            iv.tick().await;
            let _ = sqlx::query("INSERT INTO service_heartbeats(service, at, info) VALUES ($1, now(), $2) ON CONFLICT (service) DO UPDATE SET at = now(), info = $2")
                .bind(&service)
                .bind(Json(info()))
                .execute(&pool)
                .await;
        }
    }))
}
