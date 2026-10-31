use oc_db::testing::TestDb;

#[test]
fn migration_files_are_the_ts_ones() {
    let dir = oc_db::migrations_dir();
    assert!(dir.ends_with("crates/oc-platform/oc-db/migrations"));
    let files = oc_db::migrate::migration_files(&dir).unwrap();
    assert_eq!(files.first().map(String::as_str), Some("0001_init.sql"));
    assert!(files.windows(2).all(|w| w[0] < w[1]));
}

#[tokio::test]
async fn migrates_once_and_tracks_in_schema_migrations() {
    let Some(t) = TestDb::create().await else {
        eprintln!("skipped: OC_DATABASE_URL not set");
        return;
    };
    let files = oc_db::migrate::migration_files(&oc_db::migrations_dir()).unwrap();
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM schema_migrations ORDER BY name").fetch_all(&t.pool).await.unwrap();
    assert_eq!(names, files);
    // Second run is a no-op (this is what keeps a TS-migrated database from being re-migrated).
    assert!(oc_db::migrate(&t.pool, &oc_db::migrations_dir()).await.unwrap().is_empty());
    // A database where TS recorded only the first N migrations gets just the rest.
    sqlx::query("DELETE FROM schema_migrations WHERE name = $1").bind(files.last().unwrap()).execute(&t.pool).await.unwrap();
    let r = oc_db::migrate(&t.pool, &oc_db::migrations_dir()).await;
    // Re-applying the last file may legitimately fail (non-idempotent DDL) — but it must be attempted alone.
    match r {
        Ok(applied) => assert_eq!(applied, vec![files.last().unwrap().clone()]),
        Err(oc_db::migrate::MigrateError::Apply { file, .. }) => assert_eq!(&file, files.last().unwrap()),
        Err(e) => panic!("{e}"),
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM schema_migrations").fetch_one(&t.pool).await.unwrap();
    assert!(n as usize >= files.len() - 1);
    let tx_val: i32 = oc_db::tx(&t.pool, |c| Box::pin(async move { sqlx::query_scalar::<_, i32>("SELECT 41 + 1").fetch_one(&mut *c).await })).await.unwrap();
    assert_eq!(tx_val, 42);
    t.drop_db().await;
}
