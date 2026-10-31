//! Pinned-face path (no network): a `google_font_files` row like the one TS wrote
//! (tests/golden/platform/gfonts_pinned.json) is served from the object store.
use oc_db::testing::TestDb;
use oc_gfonts::*;
use oc_storage::{LocalStore, ObjectStore};
use serde_json::Value;

#[tokio::test]
async fn serves_pinned_bytes() {
    let Some(t) = TestDb::create().await else {
        eprintln!("skipped: OC_DATABASE_URL not set");
        return;
    };
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/gfonts_pinned.json");
    let g: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let hex = g["bytesHex"].as_str().unwrap();
    let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    let root = std::env::temp_dir().join(format!("oc-gfonts-db-{}", std::process::id()));
    let store = ObjectStore::Local(LocalStore::new(&root));
    let sha = store.put(&bytes, Some("font/ttf")).await.unwrap();
    let row = &g["row"];
    assert_eq!(sha, row["sha256"].as_str().unwrap());
    assert_eq!(bytes.len() as u64, row["bytes"].as_u64().unwrap());
    sqlx::query("INSERT INTO google_font_files(file, sha256, bytes, source_url) VALUES ($1,$2,$3,$4)")
        .bind(row["file"].as_str().unwrap())
        .bind(&sha)
        .bind(bytes.len() as i32)
        .bind(row["source_url"].as_str().unwrap())
        .execute(&t.pool)
        .await
        .unwrap();
    let gf = GoogleFonts::new(oc_config::repo_root().join("fixtures/fonts"), t.pool.clone(), store);
    let (a, b) = tokio::join!(gf.font_file(row["file"].as_str().unwrap()), gf.load(row["file"].as_str().unwrap(), None));
    assert_eq!(a.unwrap().as_slice(), bytes.as_slice());
    assert_eq!(b.unwrap(), bytes);
    // Bundled files come from <fontsDir>/files.
    let m = gf.full_manifest().await.unwrap();
    let bundled = &m.families[0].faces[0];
    if gf.fonts_dir().join("files").join(&bundled.file).exists() {
        assert_eq!(gf.load(&bundled.file, None).await.unwrap().len() as u64, bundled.bytes);
    }
    let _ = std::fs::remove_dir_all(root);
    t.drop_db().await;
}
