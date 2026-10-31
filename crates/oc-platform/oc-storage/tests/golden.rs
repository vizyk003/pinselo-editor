//! Parity with tests/golden/platform/storage.json: digests, key layout, on-disk paths and S3 checksums.
use oc_storage::*;
use serde_json::Value;

fn golden(name: &str) -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/");
    serde_json::from_str(&std::fs::read_to_string(format!("{p}{name}")).unwrap()).unwrap()
}

#[tokio::test]
async fn layout_matches_ts_local_store() {
    let g = golden("storage.json");
    assert_eq!(g["kind"], StoreKind::Local.as_str());
    let root = std::env::temp_dir().join(format!("oc-storage-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = ObjectStore::Local(LocalStore::new(&root));
    for o in g["objects"].as_array().unwrap() {
        let bytes = match o["bytesHex"].as_str() {
            Some(h) => hex_decode(h),
            None => vec![o["fill"]["byte"].as_u64().unwrap() as u8; o["fill"]["len"].as_u64().unwrap() as usize],
        };
        let sha = o["sha256"].as_str().unwrap();
        assert_eq!(sha256_hex(&bytes), sha);
        assert_eq!(object_key(sha), o["file"].as_str().unwrap());
        assert_eq!(object_key(sha), o["objectPath"].as_str().unwrap());
        assert_eq!(checksum_sha256_b64(sha), o["checksumSha256"].as_str().unwrap());
        assert_eq!(store.put(&bytes, None).await.unwrap(), sha);
        let f = store.file(sha).await.unwrap();
        assert_eq!(f.strip_prefix(&root).unwrap().to_str().unwrap(), o["file"].as_str().unwrap());
        assert_eq!(store.size(sha).await.unwrap(), o["size"].as_u64().unwrap());
    }
    let mut listed: Vec<(String, u64)> = store.list().await.unwrap().into_iter().map(|o| (o.sha, o.size)).collect();
    listed.sort();
    let want: Vec<(String, u64)> = g["listed"].as_array().unwrap().iter().map(|o| (o["sha"].as_str().unwrap().to_owned(), o["size"].as_u64().unwrap())).collect();
    assert_eq!(listed, want);
    assert_eq!(store.get("../../etc/passwd").await.unwrap_err().to_string(), g["badKeyError"].as_str().unwrap());
    let _ = std::fs::remove_dir_all(root);
}

fn hex_decode(h: &str) -> Vec<u8> {
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect()
}
