//! Postgres access (port of `packages/db`): a lazily created process-wide pool (`OC_DATABASE_POOL_MAX` connections,
//! `OC_DATABASE_URL` through [`connection::connect_options`]: TLS, pooler and Supabase guards), a transaction helper,
//! app-generated UUID ids, the forward-only migrator with the Supabase lockdown, the exposure report and a
//! reconnecting LISTEN connection.

use std::sync::OnceLock;

use futures::future::BoxFuture;
pub use sqlx;
use sqlx::postgres::PgPoolOptions;
pub use sqlx::{PgConnection, PgPool, Postgres, Transaction};

pub mod checks;
pub mod connection;
pub mod listener;
pub mod migrate;
pub mod testing;

pub use checks::{ExposureReport, check_database_exposure, report_exposure, startup_checks};
pub use connection::{PgConfigError, connect_options, is_supabase_host};
pub use listener::{Listener, ListenerOptions, ListenerTiming, create_listener};
pub use migrate::{LOCKDOWN_SQL, MIGRATIONS_TABLE, migrate, migrations_dir};

static POOL: OnceLock<PgPool> = OnceLock::new();

/// Pool options identical to the TS `new pg.Pool({ ...pgClientConfig(), max: config.databasePoolMax })`.
pub fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new().max_connections(oc_config::config().database_pool_max.clamp(1, u32::MAX as i64) as u32)
}

/// A pool for `url` that connects on first use, after the [`connect_options`] guards.
pub fn lazy_pool(url: &str) -> Result<PgPool, PgConfigError> {
    Ok(pool_options().connect_lazy_with(connect_options(url)?))
}

/// The process-wide pool for `OC_DATABASE_URL` (created on first use; connections open lazily). A refused database
/// setup ([`PgConfigError`]) is printed and exits 1, as the TS services stop on it.
pub fn db() -> &'static PgPool {
    POOL.get_or_init(|| {
        lazy_pool(&oc_config::config().database_url).unwrap_or_else(|e| {
            eprintln!("[db] {e}");
            std::process::exit(1)
        })
    })
}

/// Connect a new pool to an explicit URL (with the [`connect_options`] guards).
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    let opts = connect_options(url).map_err(|e| sqlx::Error::Configuration(Box::new(e)))?;
    pool_options().connect_with(opts).await
}

/// Run `f` inside `BEGIN … COMMIT`, rolling back if it returns an error.
pub async fn tx<T, E, F>(pool: &PgPool, f: F) -> Result<T, E>
where
    E: From<sqlx::Error>,
    F: for<'c> FnOnce(&'c mut PgConnection) -> BoxFuture<'c, Result<T, E>>,
{
    let mut t = pool.begin().await?;
    match f(&mut t).await {
        Ok(v) => {
            t.commit().await?;
            Ok(v)
        }
        Err(e) => {
            let _ = t.rollback().await;
            Err(e)
        }
    }
}

/// Database row ids are UUIDs generated in the app (random v4, like `crypto.randomUUID()`).
pub fn uuid() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

/// Close the process-wide pool.
pub async fn close_db() {
    if let Some(p) = POOL.get() {
        p.close().await;
    }
}
