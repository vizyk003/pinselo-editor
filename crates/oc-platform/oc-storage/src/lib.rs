//! ObjectStore port (ADR-0014): content-addressed blobs keyed by sha256 (`a/{sha[0..2]}/{sha}`).
//! [`LocalStore`] for dev/self-host; [`S3Store`] (S3, R2, MinIO, SeaweedFS) when `OC_S3_BUCKET` is set.
//! [`ObjectStore::file`] always yields a local path (S3 objects are cached on disk) because ffmpeg/resvg
//! need files. Key layout, temp-file naming and S3 parameters match `packages/storage` exactly.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::SystemTime;

use base64::Engine as _;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt};
use tokio::sync::OnceCell;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("bad object key")]
    BadKey,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("s3: {0}")]
    S3(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    Local,
    S3,
}

impl StoreKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StoreKind::Local => "local",
            StoreKind::S3 => "s3",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub sha: String,
    pub size: u64,
    pub modified: SystemTime,
}

/// Inclusive byte range (`{start, end}` in TS; both ends inclusive like HTTP `Range`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

pub type ByteStream = Pin<Box<dyn AsyncRead + Send>>;

/// Lowercase hex sha256.
pub fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// Object key: `a/{sha[0..2]}/{sha}`.
pub fn object_key(sha: &str) -> String {
    let pre: String = sha.chars().take(2).collect();
    format!("a/{pre}/{sha}")
}

/// `/^[0-9a-f]{64}$/`.
pub fn is_valid_sha(sha: &str) -> bool {
    sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `x-amz-checksum-sha256` value for an object: base64 of the raw digest.
pub fn checksum_sha256_b64(sha: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(hex::decode(sha).unwrap_or_default())
}

fn tmp_path(f: &Path) -> PathBuf {
    let mut s = f.as_os_str().to_owned();
    s.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(s)
}

// ------------------------------------------------------------------ local

#[derive(Debug, Clone)]
pub struct LocalStore {
    root: PathBuf,
}

impl LocalStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        LocalStore { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn p(&self, sha: &str) -> Result<PathBuf> {
        if !is_valid_sha(sha) {
            return Err(StorageError::BadKey);
        }
        Ok(self.root.join(object_key(sha)))
    }

    pub async fn put(&self, bytes: &[u8]) -> Result<String> {
        let sha = sha256_hex(bytes);
        let f = self.p(&sha)?;
        if !f.exists() {
            tokio::fs::create_dir_all(f.parent().expect("key has a parent")).await?;
            let tmp = tmp_path(&f);
            tokio::fs::write(&tmp, bytes).await?;
            tokio::fs::rename(&tmp, &f).await?;
        } else {
            // Re-uploading existing bytes refreshes the age, so GC's grace period covers the new reference.
            let f2 = f.clone();
            let _ = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                let now = SystemTime::now();
                std::fs::File::options().write(true).open(&f2)?.set_times(std::fs::FileTimes::new().set_accessed(now).set_modified(now))
            })
            .await;
        }
        Ok(sha)
    }

    pub async fn get(&self, sha: &str) -> Result<Vec<u8>> {
        Ok(tokio::fs::read(self.p(sha)?).await?)
    }

    pub async fn file(&self, sha: &str) -> Result<PathBuf> {
        self.p(sha)
    }

    pub async fn size(&self, sha: &str) -> Result<u64> {
        Ok(tokio::fs::metadata(self.p(sha)?).await?.len())
    }

    pub async fn stream(&self, sha: &str, range: Option<ByteRange>) -> Result<ByteStream> {
        let mut f = tokio::fs::File::open(self.p(sha)?).await?;
        Ok(match range {
            Some(r) => {
                f.seek(std::io::SeekFrom::Start(r.start)).await?;
                Box::pin(f.take((r.end + 1).saturating_sub(r.start)))
            }
            None => Box::pin(f),
        })
    }

    pub async fn exists(&self, sha: &str) -> Result<bool> {
        Ok(self.p(sha)?.exists())
    }

    pub async fn list(&self) -> Result<Vec<StoredObject>> {
        let base = self.root.join("a");
        let mut out = Vec::new();
        if !base.exists() {
            return Ok(out);
        }
        let mut dirs = tokio::fs::read_dir(&base).await?;
        while let Some(d) = dirs.next_entry().await? {
            let Ok(mut files) = tokio::fs::read_dir(d.path()).await else { continue };
            while let Ok(Some(f)) = files.next_entry().await {
                let name = f.file_name().to_string_lossy().into_owned();
                if !is_valid_sha(&name) {
                    continue;
                }
                if let Ok(st) = tokio::fs::metadata(f.path()).await {
                    out.push(StoredObject { sha: name, size: st.len(), modified: st.modified().unwrap_or(SystemTime::UNIX_EPOCH) });
                }
            }
        }
        Ok(out)
    }

    pub async fn delete(&self, sha: &str) -> Result<()> {
        match tokio::fs::remove_file(self.p(sha)?).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

// ------------------------------------------------------------------ s3

#[derive(Debug, Clone)]
pub struct S3Store {
    s3: aws_sdk_s3::Client,
    bucket: String,
    cache_dir: PathBuf,
    /// `OC_S3_CHECKSUMS=off`: for S3-compatible stores that reject flexible checksums (Supabase Storage, plan 12 §D).
    /// Keys are content-addressed, so callers that hash on read still detect corruption.
    checksums: bool,
}

/// An endpoint with a path (Supabase's `.../storage/v1/s3`) needs a trailing slash: the SDK otherwise joins the bucket
/// straight onto the last segment (`/s3bucket/key`).
fn s3_endpoint(ep: &str) -> String {
    let has_path = ep.split_once("://").is_some_and(|(_, rest)| rest.contains('/'));
    if has_path && !ep.ends_with('/') { format!("{ep}/") } else { ep.to_owned() }
}

/// Whether `OC_S3_CHECKSUMS` leaves flexible checksums on (anything but `off`).
pub fn s3_checksums_enabled(setting: Option<&str>) -> bool {
    setting != Some("off")
}

fn s3err(e: impl std::fmt::Display) -> StorageError {
    StorageError::S3(e.to_string())
}

impl S3Store {
    /// Region from `OC_S3_REGION` (default `auto`); `OC_S3_ENDPOINT` switches on a custom endpoint with
    /// path-style addressing; `OC_S3_CHECKSUMS=off` computes and validates checksums only when an operation requires
    /// them, and sends no `x-amz-checksum-sha256` on put. Credentials come from the standard AWS provider chain (as in
    /// the TS SDK).
    pub async fn from_env(bucket: impl Into<String>, cache_dir: impl Into<PathBuf>) -> Self {
        let region = std::env::var("OC_S3_REGION").unwrap_or_else(|_| "auto".into());
        let shared = aws_config::from_env().region(aws_config::Region::new(region)).load().await;
        let mut b = aws_sdk_s3::config::Builder::from(&shared);
        if let Ok(ep) = std::env::var("OC_S3_ENDPOINT") {
            b = b.endpoint_url(s3_endpoint(&ep)).force_path_style(true);
        }
        let checksums = s3_checksums_enabled(std::env::var("OC_S3_CHECKSUMS").ok().as_deref());
        if !checksums {
            b = b
                .request_checksum_calculation(aws_sdk_s3::config::RequestChecksumCalculation::WhenRequired)
                .response_checksum_validation(aws_sdk_s3::config::ResponseChecksumValidation::WhenRequired);
        }
        Self::with_client(aws_sdk_s3::Client::from_conf(b.build()), bucket, cache_dir).with_checksums(checksums)
    }

    pub fn with_client(s3: aws_sdk_s3::Client, bucket: impl Into<String>, cache_dir: impl Into<PathBuf>) -> Self {
        S3Store { s3, bucket: bucket.into(), cache_dir: cache_dir.into(), checksums: true }
    }

    /// Send `x-amz-checksum-sha256` on put (the default) or not (`OC_S3_CHECKSUMS=off`).
    pub fn with_checksums(mut self, on: bool) -> Self {
        self.checksums = on;
        self
    }

    pub async fn put(&self, bytes: &[u8], content_type: Option<&str>) -> Result<String> {
        let sha = sha256_hex(bytes);
        if !self.exists(&sha).await? {
            self.s3
                .put_object()
                .bucket(&self.bucket)
                .key(object_key(&sha))
                .body(bytes.to_vec().into())
                .set_content_type(content_type.map(str::to_owned))
                .set_checksum_sha256(self.checksums.then(|| checksum_sha256_b64(&sha)))
                .send()
                .await
                .map_err(s3err)?;
        }
        Ok(sha)
    }

    pub async fn get(&self, sha: &str) -> Result<Vec<u8>> {
        let r = self.s3.get_object().bucket(&self.bucket).key(object_key(sha)).send().await.map_err(s3err)?;
        Ok(r.body.collect().await.map_err(s3err)?.into_bytes().to_vec())
    }

    pub async fn file(&self, sha: &str) -> Result<PathBuf> {
        let f = self.cache_dir.join(object_key(sha));
        if !f.exists() {
            tokio::fs::create_dir_all(f.parent().expect("key has a parent")).await?;
            let r = self.s3.get_object().bucket(&self.bucket).key(object_key(sha)).send().await.map_err(s3err)?;
            let tmp = tmp_path(&f);
            let mut out = tokio::fs::File::create(&tmp).await?;
            let mut body = r.body.into_async_read();
            tokio::io::copy(&mut body, &mut out).await?;
            drop(out);
            tokio::fs::rename(&tmp, &f).await?;
        }
        Ok(f)
    }

    pub async fn size(&self, sha: &str) -> Result<u64> {
        let r = self.s3.head_object().bucket(&self.bucket).key(object_key(sha)).send().await.map_err(s3err)?;
        Ok(r.content_length.unwrap_or(0).max(0) as u64)
    }

    pub async fn stream(&self, sha: &str, range: Option<ByteRange>) -> Result<ByteStream> {
        let r = self
            .s3
            .get_object()
            .bucket(&self.bucket)
            .key(object_key(sha))
            .set_range(range.map(|r| format!("bytes={}-{}", r.start, r.end)))
            .send()
            .await
            .map_err(s3err)?;
        Ok(Box::pin(r.body.into_async_read()))
    }

    pub async fn exists(&self, sha: &str) -> Result<bool> {
        Ok(self.s3.head_object().bucket(&self.bucket).key(object_key(sha)).send().await.is_ok())
    }

    pub async fn list(&self) -> Result<Vec<StoredObject>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let r = self
                .s3
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix("a/")
                .set_continuation_token(token.take())
                .send()
                .await
                .map_err(s3err)?;
            for o in r.contents() {
                let sha = o.key().and_then(|k| k.rsplit('/').next()).unwrap_or("");
                if is_valid_sha(sha) {
                    let modified = o.last_modified().and_then(|d| SystemTime::try_from(*d).ok()).unwrap_or_else(SystemTime::now);
                    out.push(StoredObject { sha: sha.to_owned(), size: o.size().unwrap_or(0).max(0) as u64, modified });
                }
            }
            token = if r.is_truncated() == Some(true) { r.next_continuation_token().map(str::to_owned) } else { None };
            if token.is_none() {
                return Ok(out);
            }
        }
    }

    pub async fn delete(&self, sha: &str) -> Result<()> {
        if !is_valid_sha(sha) {
            return Err(StorageError::BadKey);
        }
        self.s3.delete_object().bucket(&self.bucket).key(object_key(sha)).send().await.map_err(s3err)?;
        let _ = tokio::fs::remove_file(self.cache_dir.join(object_key(sha))).await;
        Ok(())
    }
}

// ------------------------------------------------------------------ dispatch

#[derive(Debug, Clone)]
pub enum ObjectStore {
    Local(LocalStore),
    S3(S3Store),
}

macro_rules! dispatch {
    ($self:ident, $s:ident => $e:expr) => {
        match $self {
            ObjectStore::Local($s) => $e,
            ObjectStore::S3($s) => $e,
        }
    };
}

impl ObjectStore {
    pub fn kind(&self) -> StoreKind {
        match self {
            ObjectStore::Local(_) => StoreKind::Local,
            ObjectStore::S3(_) => StoreKind::S3,
        }
    }
    pub async fn put(&self, bytes: &[u8], content_type: Option<&str>) -> Result<String> {
        match self {
            ObjectStore::Local(s) => s.put(bytes).await,
            ObjectStore::S3(s) => s.put(bytes, content_type).await,
        }
    }
    pub async fn get(&self, sha: &str) -> Result<Vec<u8>> {
        dispatch!(self, s => s.get(sha).await)
    }
    /// A local file with the object's bytes (cached download for S3).
    pub async fn file(&self, sha: &str) -> Result<PathBuf> {
        dispatch!(self, s => s.file(sha).await)
    }
    pub async fn size(&self, sha: &str) -> Result<u64> {
        dispatch!(self, s => s.size(sha).await)
    }
    pub async fn stream(&self, sha: &str, range: Option<ByteRange>) -> Result<ByteStream> {
        dispatch!(self, s => s.stream(sha, range).await)
    }
    pub async fn exists(&self, sha: &str) -> Result<bool> {
        dispatch!(self, s => s.exists(sha).await)
    }
    /// Every stored object (for garbage collection).
    pub async fn list(&self) -> Result<Vec<StoredObject>> {
        dispatch!(self, s => s.list().await)
    }
    pub async fn delete(&self, sha: &str) -> Result<()> {
        dispatch!(self, s => s.delete(sha).await)
    }
}

static STORE: OnceCell<ObjectStore> = OnceCell::const_new();

/// The process-wide store: S3 when `OC_S3_BUCKET` is set (cache in `<data>/s3-cache`), else local
/// (`<data>/objects`).
pub async fn objects() -> &'static ObjectStore {
    STORE
        .get_or_init(|| async {
            let paths = oc_config::paths();
            match std::env::var("OC_S3_BUCKET") {
                Ok(bucket) => ObjectStore::S3(S3Store::from_env(bucket, paths.data.join("s3-cache")).await),
                Err(_) => ObjectStore::Local(LocalStore::new(paths.objects.clone())),
            }
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_endpoint_slash() {
        assert_eq!(s3_endpoint("https://r.storage.supabase.co/storage/v1/s3"), "https://r.storage.supabase.co/storage/v1/s3/");
        assert_eq!(s3_endpoint("https://r.storage.supabase.co/storage/v1/s3/"), "https://r.storage.supabase.co/storage/v1/s3/");
        assert_eq!(s3_endpoint("http://minio:9000"), "http://minio:9000");
    }

    #[test]
    fn s3_checksums_switch() {
        assert!(s3_checksums_enabled(None));
        assert!(s3_checksums_enabled(Some("")));
        assert!(s3_checksums_enabled(Some("on")));
        assert!(!s3_checksums_enabled(Some("off")));
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oc-storage-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[tokio::test]
    async fn local_roundtrip() {
        let root = tmpdir("rt");
        let s = ObjectStore::Local(LocalStore::new(&root));
        let sha = s.put(b"hello world", None).await.unwrap();
        assert_eq!(sha, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
        assert_eq!(s.file(&sha).await.unwrap(), root.join("a/b9").join(&sha));
        assert_eq!(s.get(&sha).await.unwrap(), b"hello world");
        assert_eq!(s.size(&sha).await.unwrap(), 11);
        assert!(s.exists(&sha).await.unwrap());
        assert_eq!(s.put(b"hello world", None).await.unwrap(), sha);
        let mut buf = String::new();
        s.stream(&sha, Some(ByteRange { start: 2, end: 4 })).await.unwrap().read_to_string(&mut buf).await.unwrap();
        assert_eq!(buf, "llo");
        std::fs::write(root.join("a/b9/junk"), b"x").unwrap();
        let l = s.list().await.unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].size, 11);
        s.delete(&sha).await.unwrap();
        s.delete(&sha).await.unwrap();
        assert!(!s.exists(&sha).await.unwrap());
        assert!(matches!(s.get("../etc").await, Err(StorageError::BadKey)));
        let _ = std::fs::remove_dir_all(root);
    }
}
