//! The Supabase lockdown (crates/oc-platform/oc-db/lockdown.sql, run by oc_db::migrate at three points) on a throwaway cluster
//! (tests/common/cluster.rs), so the cluster-wide API roles never touch the shared development server. Migrations run
//! as a NON-superuser role that owns its database, as `postgres` does on Supabase, with Supabase's schema and default
//! grants emulated. Mirrors the core of packages/db/src/lockdown.db.test.ts:
//! - a fresh database with every migration plus objects the Data API would serve ends locked down;
//! - a run whose migration fails still locks down what it committed (point 2) and what appeared outside migrations
//!   during the run (point 3), and anon can never write to schema_migrations (point 1);
//! - plain Postgres (no auth schema) is left alone.
//!
//! Skipped when no Postgres server binaries (initdb, pg_ctl) are found.

mod common;

use std::path::PathBuf;

use common::cluster::{SUPERUSER, TestCluster};
use oc_db::checks::{CheckContext, check_database_exposure, exposure_messages};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};

const MIGRATOR: &str = "oc_migrator";
const API_ROLES: [&str; 3] = ["anon", "authenticated", "service_role"];

const SUPABASE_LIKE: &str = "
  CREATE SCHEMA auth;
  GRANT USAGE ON SCHEMA public TO anon, authenticated, service_role;
  ALTER DEFAULT PRIVILEGES FOR ROLE oc_migrator IN SCHEMA public GRANT ALL ON TABLES TO anon, authenticated, service_role;
  ALTER DEFAULT PRIVILEGES FOR ROLE oc_migrator IN SCHEMA public GRANT ALL ON SEQUENCES TO anon, authenticated, service_role;
  ALTER DEFAULT PRIVILEGES FOR ROLE oc_migrator IN SCHEMA public GRANT ALL ON FUNCTIONS TO anon, authenticated, service_role;";

const SIM_OBJECTS: &str = "
  CREATE TABLE oc_sim_items (id bigserial PRIMARY KEY, secret text, label text);
  GRANT SELECT (label) ON oc_sim_items TO anon;
  CREATE TABLE oc_sim_public (id int);
  GRANT SELECT ON oc_sim_public TO PUBLIC;
  CREATE SEQUENCE oc_sim_seq;
  GRANT USAGE ON SEQUENCE oc_sim_seq TO PUBLIC;
  CREATE TABLE oc_sim_parted (id int) PARTITION BY RANGE (id);
  CREATE TABLE oc_sim_part1 PARTITION OF oc_sim_parted FOR VALUES FROM (0) TO (10);
  CREATE VIEW oc_sim_view AS SELECT id, secret FROM oc_sim_items;
  CREATE MATERIALIZED VIEW oc_sim_matview AS SELECT count(*) AS n FROM oc_sim_items;
  CREATE FUNCTION oc_sim_fn() RETURNS int LANGUAGE sql AS 'SELECT 1';
  CREATE FUNCTION oc_sim_definer() RETURNS SETOF text LANGUAGE sql SECURITY DEFINER SET search_path = public AS 'SELECT secret FROM oc_sim_items';
  CREATE PROCEDURE oc_sim_proc() LANGUAGE sql AS 'SELECT 1';";

async fn pool(url: &str) -> PgPool {
    PgPoolOptions::new().max_connections(2).connect(url).await.unwrap()
}

/// Every privilege the API roles hold on public objects (PUBLIC's included), as "kind name role" strings.
async fn api_privileges(q: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "WITH api(role) AS (SELECT unnest($1::text[]))
         SELECT 'table ' || c.oid::regclass || ' ' || api.role AS p
           FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace, api
          WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
            AND (has_table_privilege(api.role, c.oid, 'SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER')
                 OR has_any_column_privilege(api.role, c.oid, 'SELECT, INSERT, UPDATE, REFERENCES'))
         UNION ALL
         SELECT 'sequence ' || c.oid::regclass || ' ' || api.role
           FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace, api
          WHERE n.nspname = 'public' AND c.relkind = 'S' AND has_sequence_privilege(api.role, c.oid, 'USAGE, SELECT, UPDATE')
         UNION ALL
         SELECT 'function ' || p.oid::regprocedure || ' ' || api.role
           FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace, api
          WHERE n.nspname = 'public' AND has_function_privilege(api.role, p.oid, 'EXECUTE')
         ORDER BY 1",
    )
    .bind(API_ROLES.map(String::from).to_vec())
    .fetch_all(q)
    .await
    .unwrap()
}

async fn tables_without_rls(q: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT c.oid::regclass::text FROM pg_class c WHERE c.relnamespace = 'public'::regnamespace AND c.relkind IN ('r', 'p') AND NOT c.relrowsecurity ORDER BY 1")
        .fetch_all(q)
        .await
        .unwrap()
}

/// Runs `sql` as anon (SET ROLE from the superuser): "ok" or the SQLSTATE.
async fn as_anon(su: &PgPool, sql: &str) -> String {
    let mut t = su.begin().await.unwrap();
    t.execute("SET LOCAL ROLE anon").await.unwrap();
    let r = match t.execute(sql).await {
        Ok(_) => "ok".to_owned(),
        Err(e) => e.as_database_error().and_then(|d| d.code()).map(|c| c.into_owned()).unwrap_or_else(|| e.to_string()),
    };
    let _ = t.rollback().await;
    r
}

fn migrations_copy(keep: impl Fn(&str) -> bool, extra: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oc-migrations-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = oc_db::migrations_dir();
    for f in oc_db::migrate::migration_files(&src).unwrap() {
        if keep(&f) {
            std::fs::copy(src.join(&f), dir.join(&f)).unwrap();
        }
    }
    for (f, sql) in extra {
        std::fs::write(dir.join(f), sql).unwrap();
    }
    dir
}

struct Db {
    mig: PgPool,
    su: PgPool,
}

async fn create_db(c: &TestCluster, admin: &PgPool, name: &str, supabase_like: bool) -> Db {
    sqlx::raw_sql(&format!("CREATE DATABASE {name} OWNER {MIGRATOR} TEMPLATE template0")).execute(admin).await.unwrap();
    let su = pool(&c.url(name, SUPERUSER)).await;
    if supabase_like {
        sqlx::raw_sql(SUPABASE_LIKE).execute(&su).await.unwrap();
    }
    Db { mig: pool(&c.url(name, MIGRATOR)).await, su }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supabase_lockdown_at_all_three_points() {
    let Some(cluster) = TestCluster::start() else {
        eprintln!("skipped: no Postgres server binaries (initdb, pg_ctl)");
        return;
    };
    let admin = pool(&cluster.url("postgres", SUPERUSER)).await;
    sqlx::raw_sql(&format!(
        "CREATE ROLE anon NOLOGIN; CREATE ROLE authenticated NOLOGIN; CREATE ROLE service_role NOLOGIN;
         CREATE ROLE {MIGRATOR} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;"
    ))
    .execute(&admin)
    .await
    .unwrap();

    // ---- A fresh Supabase-like database: every migration plus objects the Data API would serve.
    let fresh = create_db(&cluster, &admin, "oc_fresh", true).await;
    let dir = migrations_copy(|_| true, &[("9990_sim_objects.sql", SIM_OBJECTS)]);
    let applied = oc_db::migrate(&fresh.mig, &dir).await.unwrap();
    assert!(applied.contains(&"9990_sim_objects.sql".to_owned()));
    assert_eq!(api_privileges(&fresh.su).await, Vec::<String>::new());
    assert_eq!(tables_without_rls(&fresh.su).await, Vec::<String>::new());
    assert_eq!(as_anon(&fresh.su, "SELECT label FROM oc_sim_items").await, "42501");
    assert_eq!(as_anon(&fresh.su, "SELECT * FROM oc_sim_definer()").await, "42501");
    assert_eq!(as_anon(&fresh.su, "INSERT INTO schema_migrations(name) VALUES ('x')").await, "42501");
    let r = check_database_exposure(&fresh.mig).await.unwrap();
    assert!(r.supabase && r.exposed.is_empty() && r.no_rls.is_empty() && r.rls_blocks_app.is_empty(), "{r:?}");
    assert_eq!(exposure_messages("migrate", &r, CheckContext::Migrate, true), (vec![(false, "[migrate] Supabase exposure check: OK".to_owned())], false));
    // Later objects get no default grants; a manual grant shows up in the report, and the next run removes it.
    sqlx::raw_sql("CREATE TABLE oc_sim_later (id int); GRANT SELECT ON oc_sim_later TO anon;").execute(&fresh.mig).await.unwrap();
    let r = check_database_exposure(&fresh.mig).await.unwrap();
    assert_eq!(r.exposed.iter().map(|e| format!("{} {} {}", e.kind, e.object, e.role)).collect::<Vec<_>>(), vec!["table oc_sim_later anon"]);
    assert_eq!(r.no_rls, vec!["oc_sim_later"]);
    assert!(exposure_messages("migrate", &r, CheckContext::Migrate, true).1, "exposed is fatal in production");
    assert!(oc_db::migrate(&fresh.mig, &dir).await.unwrap().is_empty());
    assert_eq!(api_privileges(&fresh.su).await, Vec::<String>::new());
    assert_eq!(tables_without_rls(&fresh.su).await, Vec::<String>::new());

    // ---- A run whose migration fails. 0001 commits a table; while the run goes on, another session creates an
    // object granted to anon (outside any migration); 0002 fails. Point 2 must have locked 0001's table down at its
    // own COMMIT, point 3 must lock the outside object down although the run failed.
    let partial = create_db(&cluster, &admin, "oc_partial", true).await;
    let dir = migrations_copy(
        |_| false,
        &[
            ("0001_ok.sql", "CREATE TABLE oc_first (id bigserial PRIMARY KEY, secret text);"),
            ("0002_bad.sql", "CREATE TABLE oc_never (id int); SELECT 1/0;"),
        ],
    );
    let between = std::sync::Mutex::new(Vec::<(Vec<String>, Vec<String>)>::new());
    let outside_url = cluster.url("oc_partial", MIGRATOR);
    let su_url = cluster.url("oc_partial", SUPERUSER);
    let res = oc_db::migrate::migrate_with(&partial.mig, &dir, |file| {
        assert_eq!(file, "0001_ok.sql");
        // Another session, between the transactions of the run.
        let (outside_url, su_url) = (outside_url.clone(), su_url.clone());
        let snap = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let su = pool(&su_url).await;
                let snap = (api_privileges(&su).await, tables_without_rls(&su).await);
                let other = pool(&outside_url).await;
                sqlx::raw_sql("CREATE TABLE oc_outside (id int); GRANT ALL ON oc_outside TO anon;").execute(&other).await.unwrap();
                other.close().await;
                su.close().await;
                snap
            })
        })
        .join()
        .unwrap();
        between.lock().unwrap().push(snap);
    })
    .await;
    match res {
        Err(oc_db::migrate::MigrateError::Apply { file, .. }) => assert_eq!(file, "0002_bad.sql"),
        other => panic!("expected 0002 to fail, got {other:?}"),
    }
    // Point 2: what 0001 committed was never reachable.
    assert_eq!(between.into_inner().unwrap(), vec![(Vec::<String>::new(), Vec::<String>::new())]);
    // Point 3: the object created during the failed run is locked down too.
    assert_eq!(api_privileges(&partial.su).await, Vec::<String>::new());
    assert_eq!(tables_without_rls(&partial.su).await, Vec::<String>::new());
    assert_eq!(as_anon(&partial.su, "SELECT * FROM oc_outside").await, "42501");
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM schema_migrations").fetch_all(&partial.su).await.unwrap();
    assert_eq!(names, vec!["0001_ok.sql"]);
    // Point 1: anon can never write to schema_migrations.
    assert_eq!(as_anon(&partial.su, "INSERT INTO schema_migrations(name) VALUES ('x')").await, "42501");

    // ---- Plain Postgres (no auth schema): a no-op, and the report says so.
    let plain = create_db(&cluster, &admin, "oc_plain", false).await;
    sqlx::raw_sql("GRANT USAGE ON SCHEMA public TO anon").execute(&plain.su).await.unwrap();
    let dir = migrations_copy(|_| false, &[("0001_t.sql", "CREATE TABLE oc_t (id int); GRANT SELECT ON oc_t TO anon;")]);
    oc_db::migrate(&plain.mig, &dir).await.unwrap();
    assert_eq!(api_privileges(&plain.su).await, vec!["table oc_t anon"]);
    let r = check_database_exposure(&plain.mig).await.unwrap();
    assert!(!r.supabase);
    assert_eq!(exposure_messages("migrate", &r, CheckContext::Migrate, true).0[0].1, "[migrate] Supabase exposure check: not a Supabase database, nothing to lock down");

    for p in [fresh.mig, fresh.su, partial.mig, partial.su, plain.mig, plain.su, admin] {
        p.close().await;
    }
}
