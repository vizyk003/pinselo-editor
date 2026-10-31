//! Throwaway databases for integration tests.
//!
//! [`TestDb::create`] returns `None` (test should skip) unless `OC_DATABASE_URL` is set. It connects to
//! the server's `postgres` maintenance database, creates a fresh `oc_rust_test_<uuid>` database, applies
//! the shared migrations and hands back a pool. The configured database itself is never touched.
//! Call [`TestDb::drop_db`] at the end; only `oc_rust_test_*` databases are ever dropped.

use sqlx::{Connection, PgConnection, PgPool};

pub struct TestDb {
    pub pool: PgPool,
    pub url: String,
    pub name: String,
    admin_url: String,
}

impl TestDb {
    pub async fn create() -> Option<TestDb> {
        let base = std::env::var("OC_DATABASE_URL").ok().filter(|s| !s.is_empty())?;
        let mut admin = url::Url::parse(&base).expect("OC_DATABASE_URL is a URL");
        admin.set_path("/postgres");
        let name = format!("oc_rust_test_{}", uuid::Uuid::new_v4().simple());
        let mut c = PgConnection::connect(admin.as_str()).await.expect("connect to postgres maintenance db");
        // C collation, like the golden generators' databases: text ORDER BY must not depend on the server's
        // default locale (en_US.utf8 in CI's postgres image, C on Homebrew).
        sqlx::raw_sql(&format!("CREATE DATABASE \"{name}\" TEMPLATE template0 ENCODING 'UTF8' LC_COLLATE 'C' LC_CTYPE 'C'"))
            .execute(&mut c)
            .await
            .expect("create throwaway db");
        c.close().await.ok();
        let mut url = admin.clone();
        url.set_path(&format!("/{name}"));
        let pool = crate::connect(url.as_str()).await.expect("connect throwaway db");
        crate::migrate(&pool, &crate::migrations_dir()).await.expect("migrations apply");
        Some(TestDb { pool, url: url.to_string(), name, admin_url: admin.to_string() })
    }

    pub async fn drop_db(self) {
        assert!(self.name.starts_with("oc_rust_test_"));
        self.pool.close().await;
        if let Ok(mut c) = PgConnection::connect(&self.admin_url).await {
            let _ = sqlx::raw_sql(&format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.name)).execute(&mut c).await;
        }
    }
}
