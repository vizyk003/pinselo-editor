//! Forward-only SQL migrations (expand/contract, no down-migrations; ADR-0012).
//!
//! The protocol of the former `packages/db/src/migrations.ts`: the files in `crates/oc-platform/oc-db/migrations` (read in place) are
//! applied **by file name**: names already in `schema_migrations` are skipped and the rest run in sorted order, each
//! in its own transaction together with an `INSERT INTO schema_migrations(name)`. A database migrated by the TS tool
//! is therefore never re-migrated, and vice versa.
//!
//! The Supabase lockdown ([`LOCKDOWN_SQL`], `crates/oc-platform/oc-db/lockdown.sql` verbatim) runs at the same three points as in
//! the TS: in the transaction that creates `schema_migrations`, inside every migration's transaction before COMMIT,
//! and once more when the run ends, also after a failure. So nothing a migration creates is ever committed reachable
//! through Supabase's Data API, and anon can never write to `schema_migrations`. A no-op on plain Postgres.

use std::path::{Path, PathBuf};

use sqlx::PgPool;

/// The tracking table shared with the TS migrator.
pub const MIGRATIONS_TABLE: &str = "schema_migrations";

/// The lockdown DO block (`LOCKDOWN_HOOK` in the TS), compiled in from `crates/oc-platform/oc-db/lockdown.sql`.
pub const LOCKDOWN_SQL: &str = include_str!("../lockdown.sql");

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("reading migrations: {0}")]
    Io(#[from] std::io::Error),
    #[error("migration {file}: {source}")]
    Apply { file: String, source: sqlx::Error },
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// `crates/oc-platform/oc-db/migrations` in this repository (override with `OC_MIGRATIONS_DIR`).
pub fn migrations_dir() -> PathBuf {
    std::env::var_os("OC_MIGRATIONS_DIR").map(PathBuf::from).unwrap_or_else(|| oc_config::repo_root().join("crates/oc-platform/oc-db/migrations"))
}

/// `*.sql` file names in apply order (JS default sort; names are ASCII so byte order is identical).
pub fn migration_files(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut v: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".sql"))
        .collect();
    v.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    Ok(v)
}

/// Run the lockdown alone in its own transaction.
pub async fn run_lockdown(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut t = pool.begin().await?;
    sqlx::raw_sql(LOCKDOWN_SQL).execute(&mut *t).await?;
    t.commit().await
}

/// Apply pending migrations; returns the names applied (in order).
pub async fn migrate(pool: &PgPool, dir: &Path) -> Result<Vec<String>, MigrateError> {
    migrate_with(pool, dir, |_| {}).await
}

/// [`migrate`], calling `on_applied` after each committed migration (`pnpm db:migrate` prints `applied <file>`).
pub async fn migrate_with(pool: &PgPool, dir: &Path, mut on_applied: impl FnMut(&str)) -> Result<Vec<String>, MigrateError> {
    // (1) One transaction: on Supabase a new table is granted to the API roles until the lockdown runs.
    {
        let mut t = pool.begin().await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS schema_migrations (name text PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())").execute(&mut *t).await?;
        sqlx::raw_sql(LOCKDOWN_SQL).execute(&mut *t).await?;
        t.commit().await?;
    }
    let mut applied = Vec::new();
    let run = async {
        let done: std::collections::HashSet<String> = sqlx::query_scalar("SELECT name FROM schema_migrations").fetch_all(pool).await?.into_iter().collect();
        for file in migration_files(dir)? {
            if done.contains(&file) {
                continue;
            }
            let sql = std::fs::read_to_string(dir.join(&file))?;
            // (2) The lockdown inside the migration's own transaction, before COMMIT.
            let one = async {
                let mut t = pool.begin().await?;
                sqlx::raw_sql(&sql).execute(&mut *t).await?;
                sqlx::query("INSERT INTO schema_migrations(name) VALUES ($1)").bind(&file).execute(&mut *t).await?;
                sqlx::raw_sql(LOCKDOWN_SQL).execute(&mut *t).await?;
                t.commit().await
            };
            one.await.map_err(|source| MigrateError::Apply { file: file.clone(), source })?;
            on_applied(&file);
            applied.push(file);
        }
        Ok::<(), MigrateError>(())
    };
    let failure = run.await.err();
    // (3) On every run, also one that failed or applied nothing: covers objects created outside migrations.
    if let Err(e) = run_lockdown(pool).await {
        match &failure {
            None => return Err(e.into()),
            Some(_) => eprintln!("[db] the Supabase lockdown could not run after the failed migration: {e}"),
        }
    }
    match failure {
        Some(f) => Err(f),
        None => Ok(applied),
    }
}

/// Print each distinct `WARNING` the server sends (the lockdown names objects it could not act for) once, as
/// `[db] <message>` on stderr, like the TS migrator's notice handler. sqlx reports server notices only as tracing
/// events (target `sqlx::postgres::notice`, without the HINT), so this installs a minimal global subscriber that
/// prints those and nothing else. Call it once at the start of a binary; it does nothing when a subscriber is set.
pub fn print_server_warnings() {
    let _ = tracing::subscriber::set_global_default(NoticePrinter::default());
}

#[derive(Default)]
struct NoticePrinter {
    seen: std::sync::Mutex<std::collections::HashSet<String>>,
}

const NOTICE_TARGET: &str = "sqlx::postgres::notice";

impl tracing::Subscriber for NoticePrinter {
    fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
        m.target() == NOTICE_TARGET && *m.level() <= tracing::Level::WARN
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, e: &tracing::Event<'_>) {
        if e.metadata().target() != NOTICE_TARGET || *e.metadata().level() != tracing::Level::WARN {
            return;
        }
        struct Msg(String);
        impl tracing::field::Visit for Msg {
            fn record_str(&mut self, f: &tracing::field::Field, v: &str) {
                if f.name() == "message" {
                    self.0 = v.to_owned();
                }
            }
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                if f.name() == "message" && self.0.is_empty() {
                    self.0 = format!("{v:?}").trim_matches('"').to_owned();
                }
            }
        }
        let mut m = Msg(String::new());
        e.record(&mut m);
        if !m.0.is_empty() && self.seen.lock().unwrap_or_else(|p| p.into_inner()).insert(m.0.clone()) {
            eprintln!("[db] {}", m.0);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}
