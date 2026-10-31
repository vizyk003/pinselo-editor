//! Content Planner social publishing — the Rust port of `services/api/src/social` (A16), shared by the
//! worker (`publish` job, scheduler sweep) and the API (connect flows, account store, scheduling).
//!
//! * [`adapters`]: one [`SocialAdapter`] per network (publish, token refresh, OAuth / app-password connect).
//! * [`http`]: the `call` / `poll` / form helpers over an injectable environment ([`Http`]).
//! * [`crypto`]: token encryption at rest and signed public media links.
//! * [`text`]: caption composition and per-network text rules.
//! * [`store`]: `social_accounts` persistence (encrypted tokens, refresh-on-use, visibility).
//! * [`publish`]: the `publish` job, scheduling and the sweep.
//!
//! Database rows, JSON shapes, messages and HTTP requests are identical to the TypeScript implementation.

pub mod adapters;
pub mod crypto;
pub mod http;
pub mod imagesize;
pub mod publish;
pub mod store;
pub mod text;
pub mod types;

pub use http::Http;
pub use types::*;

use adapters::{bluesky, linkedin, local, meta, pinterest, threads, tiktok, x, youtube};
use serde_json::{Map, Value};

/// Every adapter, in the TS registry order.
pub static ADAPTERS: [&'static dyn SocialAdapter; 10] = [
    &local::LOCAL,
    &meta::INSTAGRAM,
    &meta::FACEBOOK,
    &linkedin::LINKEDIN,
    &x::XCOM,
    &tiktok::TIKTOK,
    &youtube::YOUTUBE,
    &pinterest::PINTEREST,
    &threads::THREADS,
    &bluesky::BLUESKY,
];

/// `adapter(id)`.
pub fn adapter(id: &str) -> Option<&'static dyn SocialAdapter> {
    ADAPTERS.iter().copied().find(|a| a.id() == id)
}

/// `providerList()`: `[{ id, label, auth, configured, env, limits }]` for the connect UI.
pub fn provider_list(http: &Http) -> Vec<Value> {
    ADAPTERS
        .iter()
        .map(|a| {
            let mut m = Map::new();
            m.insert("id".into(), Value::from(a.id()));
            m.insert("label".into(), Value::from(a.label()));
            m.insert("auth".into(), Value::from(a.auth().as_str()));
            m.insert("configured".into(), Value::from(a.configured(http)));
            m.insert(
                "env".into(),
                Value::from(a.env().iter().map(|s| Value::from(*s)).collect::<Vec<_>>()),
            );
            m.insert("limits".into(), a.limits().to_json());
            Value::Object(m)
        })
        .collect()
}

/// The parts of `@oc/config` the planner reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocialConfig {
    /// `OC_SECRET`: token encryption, media signatures, the internal service token.
    pub secret: String,
    pub web_origin: String,
    /// Public media links use this origin when set, else `web_origin`.
    pub usercontent_origin: String,
    pub render_url: String,
    pub brand_name: String,
}

impl SocialConfig {
    pub fn from_config(c: &oc_config::Config) -> Self {
        SocialConfig {
            secret: c.secret.clone(),
            web_origin: c.web_origin.clone(),
            usercontent_origin: c.usercontent_origin.clone(),
            render_url: c.render_url.clone(),
            brand_name: c.brand.name.to_string(),
        }
    }

    /// `config.usercontentOrigin || config.webOrigin`.
    pub fn media_base(&self) -> &str {
        if self.usercontent_origin.is_empty() {
            &self.web_origin
        } else {
            &self.usercontent_origin
        }
    }
}

/// `Date.now()` (ms since the epoch); injected so tests and the library never depend on the system clock.
pub type Clock = std::sync::Arc<dyn Fn() -> i64 + Send + Sync>;

/// Everything the store and the publish job touch: the module-level `db`, `objects()`, `config`, `fetch`,
/// `process.env` and `Date.now` of the TS implementation, injected.
#[derive(Clone)]
pub struct Social {
    pub pool: sqlx::PgPool,
    pub http: Http,
    pub objects: oc_storage::ObjectStore,
    pub cfg: SocialConfig,
    pub clock: Clock,
}

impl std::fmt::Debug for Social {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Social")
            .field("http", &self.http)
            .field("objects", &self.objects)
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

impl Social {
    /// A context on the system clock.
    pub fn new(
        pool: sqlx::PgPool,
        http: Http,
        objects: oc_storage::ObjectStore,
        cfg: SocialConfig,
    ) -> Self {
        Social {
            pool,
            http,
            objects,
            cfg,
            clock: std::sync::Arc::new(oc_js::now_ms),
        }
    }

    /// Replace the clock.
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = std::sync::Arc::new(clock);
        self
    }

    /// `Date.now()`.
    pub fn now_ms(&self) -> i64 {
        (self.clock)()
    }
}

/// The message of a database error as node-postgres reports it (`error.message`), else the error text.
pub fn db_message(e: &sqlx::Error) -> String {
    match e.as_database_error() {
        Some(d) => d.message().to_string(),
        None => e.to_string(),
    }
}
