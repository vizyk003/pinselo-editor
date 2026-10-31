//! Application state shared by every route module.
//!
//! The TS service reads `process.env`, `Date.now()` and module-level singletons (`objects()`, the pg pool)
//! directly. Here they hang off [`AppState`] so a test can build an app with its own environment, a pinned
//! clock and a throwaway database, exactly like the golden generator runs the TS app:
//!
//! * [`AppState::env`]: environment lookup (`process.env.X`). Production reads the process environment once at
//!   start-up; tests pass a map and may change it between requests ([`AppState::set_env`]).
//! * [`AppState::now_ms`]: `Date.now()` (pinned in tests).
//! * [`AppState::spawn`]: fire-and-forget work (`void promise` in TS); tests wait with [`AppState::idle`].
//! * [`AppState::local`]: module-local singletons (TS module-level `const cache = new Map()`), keyed by type,
//!   so a route module can keep state without editing this file.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use oc_config::{Config, Paths};
use oc_storage::ObjectStore;
use sqlx::PgPool;
use tokio::sync::Notify;

use crate::http::ratelimit::RateLimits;

/// `Date.now()`: milliseconds since the epoch.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(oc_js::now_ms)
}

/// Everything needed to build an [`AppState`].
pub struct AppOptions {
    /// The environment (`process.env`): `OC_*`, `NODE_ENV`, … Config is derived from it.
    pub env: HashMap<String, String>,
    pub pool: PgPool,
    /// Object store; default: local store under the configured data directory (or S3 when `OC_S3_BUCKET`).
    pub store: Option<ObjectStore>,
    pub clock: Option<Clock>,
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub cfg: Config,
    pub paths: Paths,
    pub pool: PgPool,
    pub store: ObjectStore,
    /// Outbound HTTP (render service, OIDC, PhotoDNA, …).
    pub http: reqwest::Client,
    pub limits: RateLimits,
    env: RwLock<HashMap<String, String>>,
    clock: Clock,
    bg: Background,
    locals: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub async fn new(opts: AppOptions) -> Result<AppState, oc_config::ConfigError> {
        let env = opts.env;
        let cfg = Config::from_lookup(|k| env.get(k).cloned())?;
        let paths = Paths::new(&cfg);
        let _ = paths.ensure_dirs();
        let store = match opts.store {
            Some(s) => s,
            None => match env.get("OC_S3_BUCKET") {
                Some(bucket) => ObjectStore::S3(oc_storage::S3Store::from_env(bucket.clone(), paths.data.join("s3-cache")).await),
                None => ObjectStore::Local(oc_storage::LocalStore::new(paths.objects.clone())),
            },
        };
        let http = reqwest::Client::builder().build().expect("http client");
        Ok(AppState(Arc::new(Inner {
            cfg,
            paths,
            pool: opts.pool,
            store,
            http,
            limits: crate::router::rate_limits(),
            env: RwLock::new(env),
            clock: opts.clock.unwrap_or_else(system_clock),
            bg: Background::default(),
            locals: Mutex::new(HashMap::new()),
        })))
    }

    /// `process.env[name]`.
    pub fn env(&self, name: &str) -> Option<String> {
        self.env.read().expect("env lock").get(name).cloned()
    }

    /// `process.env[name]` when set and non-empty (the TS `if (process.env.X)` truthiness test).
    pub fn env_nonempty(&self, name: &str) -> Option<String> {
        self.env(name).filter(|v| !v.is_empty())
    }

    /// Change the environment (tests: the golden scenarios toggle capabilities between requests).
    pub fn set_env(&self, name: &str, value: Option<&str>) {
        let mut env = self.env.write().expect("env lock");
        match value {
            Some(v) => env.insert(name.to_owned(), v.to_owned()),
            None => env.remove(name),
        };
    }

    /// `config.env === 'production'`.
    pub fn prod(&self) -> bool {
        self.cfg.env == "production"
    }

    /// `Date.now()`.
    pub fn now_ms(&self) -> i64 {
        (self.clock)()
    }

    pub fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_millis(self.now_ms()).unwrap_or_default()
    }

    /// Fire-and-forget (`void promise` in TS). Errors are the task's own business.
    pub fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let bg = self.0.clone();
        bg.bg.running.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move {
            fut.await;
            if bg.bg.running.fetch_sub(1, Ordering::SeqCst) == 1 {
                bg.bg.idle.notify_waiters();
            }
        });
    }

    /// Resolves when no [`spawn`](Self::spawn)ed task is running (tests; graceful shutdown).
    pub async fn idle(&self) {
        loop {
            let notified = self.bg.idle.notified();
            if self.bg.running.load(Ordering::SeqCst) == 0 {
                return;
            }
            notified.await;
        }
    }

    /// No [`spawn`](Self::spawn)ed task is running right now.
    pub fn idle_now(&self) -> bool {
        self.bg.running.load(Ordering::SeqCst) == 0
    }

    /// A module-local singleton (the TS module-level `const x = new Map()`), created on first use.
    pub fn local<T: Default + Send + Sync + 'static>(&self) -> Arc<T> {
        let mut locals = self.locals.lock().expect("locals lock");
        let v = locals.entry(TypeId::of::<T>()).or_insert_with(|| Arc::new(T::default()) as Arc<dyn Any + Send + Sync>).clone();
        v.downcast::<T>().expect("type id matches")
    }
}

#[derive(Default)]
struct Background {
    running: AtomicUsize,
    idle: Notify,
}
