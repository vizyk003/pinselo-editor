//! A dedicated LISTEN connection that survives dropped sockets, server restarts and pooler failovers (port of
//! `packages/db/src/listener.ts`, plan 12 §A2).
//!
//! It reconnects with a doubling backoff (1 s → 30 s, reset after a successful connect) and LISTENs on every channel
//! again. Notifications sent while it is reconnecting are lost, so callers keep a backstop: the job queue polls, and
//! collab catches up in `on_reconnect`. Every step has a deadline (connect, each LISTEN and ping, close), so a server
//! that accepts the TCP connection and then hangs cannot block [`Listener::ready`] or [`Listener::close`]. A
//! `SELECT 1` ping every 60 s detects half-open sockets that keepalive alone would notice only after minutes.
//!
//! sqlx's `PgListener` reconnects inside `recv()` without backoff or deadlines, so this uses `try_recv()` (a lost
//! connection is `Ok(None)` or an error) and replaces the listener itself.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgListener, PgNotification, PgPoolOptions};
use tokio::sync::watch;
use tokio::time::{Instant, timeout};

/// Deadlines and backoff (the TS `TIMING`).
#[derive(Debug, Clone, Copy)]
pub struct ListenerTiming {
    pub min_delay: Duration,
    pub max_delay: Duration,
    /// TCP, TLS and startup together.
    pub connect: Duration,
    /// Each LISTEN and each ping.
    pub query: Duration,
    /// A graceful end, after which the connection is dropped.
    pub close: Duration,
    pub ping_every: Duration,
}

impl Default for ListenerTiming {
    fn default() -> Self {
        ListenerTiming {
            min_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            connect: Duration::from_secs(10),
            query: Duration::from_secs(10),
            close: Duration::from_secs(5),
            ping_every: Duration::from_secs(60),
        }
    }
}

pub type Callback = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct ListenerOptions {
    /// Log prefix and `application_name` (default `oc-listener`).
    pub name: String,
    /// Called each time a connection other than the first is listening again. Notifications sent while it was down
    /// are lost, so this is where a caller catches up.
    pub on_reconnect: Option<Callback>,
    /// Tests only: shorter deadlines.
    pub timing: ListenerTiming,
}

impl Default for ListenerOptions {
    fn default() -> Self {
        ListenerOptions { name: "oc-listener".into(), on_reconnect: None, timing: ListenerTiming::default() }
    }
}

impl ListenerOptions {
    pub fn named(name: impl Into<String>) -> Self {
        ListenerOptions { name: name.into(), ..Default::default() }
    }
    pub fn on_reconnect(mut self, f: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_reconnect = Some(Arc::new(f));
        self
    }
    pub fn timing(mut self, t: ListenerTiming) -> Self {
        self.timing = t;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ready {
    Pending,
    Listening,
    Closed,
}

/// [`Listener::ready`] failed: the listener was closed before it was listening.
#[derive(Debug, Clone, thiserror::Error)]
#[error("[{0}] closed before it was listening")]
pub struct ListenerClosed(pub String);

/// A running listener. Dropping it does not stop it; call [`Listener::close`].
pub struct Listener {
    name: String,
    ready: watch::Receiver<Ready>,
    close_tx: watch::Sender<bool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    connects: Arc<AtomicU64>,
    timing: ListenerTiming,
}

impl Listener {
    /// Resolves once the first connection is listening on every channel; fails when [`close`](Self::close) comes
    /// first.
    pub async fn ready(&self) -> Result<(), ListenerClosed> {
        let mut rx = self.ready.clone();
        match rx.wait_for(|r| *r != Ready::Pending).await.map(|r| *r) {
            Ok(Ready::Listening) => Ok(()),
            _ => Err(ListenerClosed(self.name.clone())),
        }
    }

    /// How many times a connection finished its LISTENs (1 after startup, +1 per reconnect).
    pub fn connects(&self) -> u64 {
        self.connects.load(Ordering::SeqCst)
    }

    /// Whether the background task still runs (it stops only on close, or with its runtime).
    pub fn is_running(&self) -> bool {
        self.task.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Stop listening (idempotent). A connected listener UNLISTENs within the close deadline; one still connecting
    /// is dropped at once.
    pub async fn close(&self) {
        self.close_tx.send_replace(true);
        let task = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(t) = task {
            let deadline = self.timing.close + Duration::from_secs(1);
            if timeout(deadline, t).await.is_err() {
                eprintln!("[{}] listener did not stop within {deadline:?}", self.name);
            }
        }
    }
}

/// A dedicated LISTEN connection for `options` (application name = `opts.name`) that delivers every notification
/// on `channels` to `on_notification` (a panicking handler is logged, the listener keeps going). Needs a tokio
/// runtime.
pub fn create_listener(
    options: PgConnectOptions,
    channels: &[&str],
    on_notification: impl Fn(PgNotification) + Send + Sync + 'static,
    opts: ListenerOptions,
) -> Listener {
    let (ready_tx, ready) = watch::channel(Ready::Pending);
    let (close_tx, close_rx) = watch::channel(false);
    let connects = Arc::new(AtomicU64::new(0));
    let task = tokio::spawn(run(
        options.application_name(&opts.name),
        channels.iter().map(|c| c.to_string()).collect(),
        Arc::new(on_notification),
        opts.clone(),
        ready_tx,
        close_rx,
        connects.clone(),
    ));
    Listener { name: opts.name, ready, close_tx, task: Mutex::new(Some(task)), connects, timing: opts.timing }
}

fn fmt_delay(d: Duration) -> String {
    if d < Duration::from_secs(1) { format!("{} ms", d.as_millis()) } else { format!("{} s", (d.as_millis() as f64 / 1000.0).round()) }
}

/// Connect a fresh single-connection pool and LISTEN on every channel, within the deadlines.
async fn connect(options: &PgConnectOptions, channels: &[String], t: &ListenerTiming) -> Result<(sqlx::PgPool, PgListener), String> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(t.connect)
        .test_before_acquire(false)
        .connect_lazy_with(options.clone());
    let mut l = match timeout(t.connect, PgListener::connect_with(&pool)).await {
        Ok(Ok(l)) => l,
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => return Err(format!("connect timed out after {}", fmt_delay(t.connect))),
    };
    l.ignore_pool_close_event(true);
    l.eager_reconnect(false);
    for ch in channels {
        match timeout(t.query, l.listen(ch)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => return Err("LISTEN timed out".into()),
        }
    }
    Ok((pool, l))
}

/// Drop a listener and its pool in the background (a dead socket must not block the caller).
fn discard(pool: sqlx::PgPool, l: PgListener) {
    drop(l);
    tokio::spawn(async move { pool.close().await });
}

async fn run(
    options: PgConnectOptions,
    channels: Vec<String>,
    deliver: Arc<dyn Fn(PgNotification) + Send + Sync>,
    opts: ListenerOptions,
    ready: watch::Sender<Ready>,
    mut close_rx: watch::Receiver<bool>,
    connects: Arc<AtomicU64>,
) {
    let (name, t) = (opts.name.as_str(), opts.timing);
    let mut delay = t.min_delay;
    'outer: loop {
        if *close_rx.borrow() {
            break;
        }
        let attempt = tokio::select! {
            r = connect(&options, &channels, &t) => r,
            _ = close_rx.wait_for(|c| *c) => break 'outer,
        };
        let why = match attempt {
            Err(why) => why,
            Ok((pool, mut l)) => {
                delay = t.min_delay;
                let n = connects.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 1 {
                    ready.send_replace(Ready::Listening);
                } else if let Some(f) = &opts.on_reconnect
                    && std::panic::catch_unwind(AssertUnwindSafe(|| f())).is_err()
                {
                    eprintln!("[{name}] on_reconnect failed");
                }
                let mut ping = tokio::time::interval_at(Instant::now() + t.ping_every, t.ping_every);
                enum Ev {
                    Close,
                    Ping,
                    Recv(Result<Option<PgNotification>, sqlx::Error>),
                }
                let why = loop {
                    let ev = tokio::select! {
                        biased;
                        _ = close_rx.wait_for(|c| *c) => Ev::Close,
                        _ = ping.tick() => Ev::Ping,
                        r = l.try_recv() => Ev::Recv(r),
                    };
                    match ev {
                        Ev::Close => {
                            // Graceful: UNLISTEN and return the connection, or give up after the close deadline.
                            let _ = timeout(t.close, async {
                                let _ = l.unlisten_all().await;
                                drop(l);
                                pool.close().await;
                            })
                            .await;
                            break 'outer;
                        }
                        Ev::Ping => match timeout(t.query, sqlx::query("SELECT 1").execute(&mut l)).await {
                            Ok(Ok(_)) => {}
                            Ok(Err(e)) => break format!("ping failed: {e}"),
                            Err(_) => break "ping failed: timed out".to_owned(),
                        },
                        Ev::Recv(Ok(Some(n))) => {
                            if std::panic::catch_unwind(AssertUnwindSafe(|| deliver(n))).is_err() {
                                eprintln!("[{name}] notification handler failed");
                            }
                        }
                        Ev::Recv(Ok(None)) => break "connection ended".to_owned(),
                        Ev::Recv(Err(e)) => break e.to_string(),
                    }
                };
                discard(pool, l);
                why
            }
        };
        eprintln!("[{name}] LISTEN connection lost ({why}); reconnecting in {}", fmt_delay(delay));
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = close_rx.wait_for(|c| *c) => break,
        }
        delay = (delay * 2).min(t.max_delay);
    }
    ready.send_if_modified(|r| {
        let pending = *r == Ready::Pending;
        if pending {
            *r = Ready::Closed;
        }
        pending
    });
}
