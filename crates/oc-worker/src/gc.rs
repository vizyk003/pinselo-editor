//! Object-store garbage collection (port of services/worker/src/gc.ts). Content-addressed blobs are shared
//! and never deleted inline, so a periodic mark-and-sweep reclaims the ones nothing points at any more:
//!  1. list stored objects older than the grace period (uploads write the blob before their row);
//!  2. mark every sha referenced anywhere: typed columns, plus any 64-hex string inside JSON documents and
//!     Yjs states/updates/versions (conservative: a false match only keeps a blob alive);
//!  3. delete unmarked candidates.
//!
//! One run at a time across workers (advisory lock). `OC_GC_DRY_RUN=1` only reports.

use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use oc_storage::ObjectStore;
use serde::Serialize;
use sqlx::PgPool;

/// `"ocgc"`.
pub const LOCK: i64 = 0x6f63_6763;

static HEX64: LazyLock<regex::bytes::Regex> = LazyLock::new(|| regex::bytes::Regex::new("[0-9a-f]{64}").expect("valid regex"));

/// Typed columns holding object shas.
pub const COLUMNS: [&str; 11] = [
    "SELECT sha256 AS s FROM assets",
    "SELECT proxy_sha256 AS s FROM assets WHERE proxy_sha256 IS NOT NULL",
    "SELECT poster_sha256 AS s FROM assets WHERE poster_sha256 IS NOT NULL",
    "SELECT thumb_sha256 AS s FROM designs WHERE thumb_sha256 IS NOT NULL",
    "SELECT thumbnail AS s FROM templates WHERE thumbnail IS NOT NULL",
    "SELECT sha256 AS s FROM custom_fonts",
    "SELECT sha256 AS s FROM email_images",
    "SELECT html_sha256 AS s FROM sites WHERE html_sha256 IS NOT NULL",
    "SELECT sha256 AS s FROM google_font_files",
    "SELECT file_sha AS s FROM print_orders WHERE file_sha IS NOT NULL",
    "SELECT sha AS s FROM icc_profiles",
];

/// JSON documents that can embed shas (template docs, job payloads/results such as exports and imports,
/// brand kits, scheduled-post renders, REST API JPEG exports).
pub const JSON_SOURCES: [(&str, &str); 6] =
    [("templates", "doc"), ("jobs", "payload"), ("jobs", "result"), ("brand_kits", "logos"), ("social_posts", "media"), ("api_exports", "files")];

/// Binary Yjs data: the current state, pending updates and version snapshots (`table`, `col`, `key`).
pub const BINARY_SOURCES: [(&str, &str, &str); 3] =
    [("doc_state", "state", "design_id"), ("doc_updates", "update", "design_id || ':' || seq"), ("design_versions", "state", "id::text")];

/// Every sha referenced anywhere.
pub async fn mark_referenced(pool: &PgPool) -> Result<HashSet<String>, sqlx::Error> {
    let mut live = HashSet::new();
    for sql in COLUMNS {
        let rows: Vec<Option<String>> = sqlx::query_scalar(sql).fetch_all(pool).await?;
        live.extend(rows.into_iter().flatten());
    }
    for (t, c) in JSON_SOURCES {
        let sql = format!("SELECT DISTINCT m[1] AS s FROM {t}, regexp_matches({c}::text, '([0-9a-f]{{64}})', 'g') AS m WHERE {c} IS NOT NULL");
        let rows: Vec<Option<String>> = sqlx::query_scalar(&sql).fetch_all(pool).await?;
        live.extend(rows.into_iter().flatten());
    }
    for (table, col, key) in BINARY_SOURCES {
        let sql = format!("SELECT {key} AS k, {col} AS v FROM {table} WHERE {key} > $1 ORDER BY 1 LIMIT 200");
        let mut after = String::new();
        loop {
            let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(&sql).bind(&after).fetch_all(pool).await?;
            let Some(last) = rows.last() else { break };
            after = last.0.clone();
            for (_, v) in &rows {
                // `buffer.toString('latin1')` maps bytes 1:1, so matching the bytes is the same.
                live.extend(HEX64.find_iter(v).map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned()));
            }
        }
    }
    Ok(live)
}

/// `collectGarbage` options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GcOptions {
    /// Objects younger than this are never candidates (default 48 h, `OC_GC_GRACE_HOURS`).
    pub grace_hours: f64,
    /// Only report (`OC_GC_DRY_RUN=1`).
    pub dry_run: bool,
}

impl GcOptions {
    /// Defaults from the environment: `Number(process.env.OC_GC_GRACE_HOURS ?? 48)`,
    /// `process.env.OC_GC_DRY_RUN === '1'`.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let grace_hours = match get("OC_GC_GRACE_HOURS") {
            None => 48.0,
            Some(v) => js_number(&v),
        };
        GcOptions { grace_hours, dry_run: get("OC_GC_DRY_RUN").as_deref() == Some("1") }
    }
}

/// `Number(string)`.
fn js_number(s: &str) -> f64 {
    let t = s.trim_matches(oc_js::is_js_whitespace);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ if t.bytes().any(|b| b.is_ascii_alphabetic() && b != b'e' && b != b'E') => f64::NAN,
        _ => t.parse().unwrap_or(f64::NAN),
    }
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GcReport {
    pub scanned: usize,
    pub deleted: usize,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

impl GcReport {
    fn empty(scanned: usize, skipped: Option<&str>) -> Self {
        GcReport { scanned, deleted: 0, bytes: 0, skipped: skipped.map(str::to_string) }
    }

    /// The worker's log line.
    pub fn log_line(&self) -> String {
        format!(
            "[worker] object gc: {}/{} objects, {} MB reclaimed{}",
            self.deleted,
            self.scanned,
            to_fixed_1(self.bytes as f64 / 1e6),
            self.skipped.as_ref().map(|s| format!(" ({s})")).unwrap_or_default()
        )
    }
}

/// `x.toFixed(1)` for a non-negative finite `x`: like Rust's exact formatting, except that an exact tie
/// rounds up (Rust rounds it to even).
pub fn to_fixed_1(x: f64) -> String {
    let y = x * 20.0;
    let exact = x.mul_add(20.0, -y) == 0.0;
    if exact && y.fract() == 0.0 && y % 2.0 == 1.0 {
        // x = (2k + 1) / 20: halfway between k/10 and (k + 1)/10.
        return format!("{:.1}", (y + 1.0) / 20.0);
    }
    format!("{x:.1}")
}

#[derive(Debug, thiserror::Error)]
pub enum GcError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Storage(#[from] oc_storage::StorageError),
}

/// `collectGarbage(opts)`; `now` is `Date.now()`.
pub async fn collect_garbage(pool: &PgPool, store: &ObjectStore, opts: GcOptions, now: SystemTime) -> Result<GcReport, GcError> {
    let mut c = pool.acquire().await?;
    let ok: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1) AS ok").bind(LOCK).fetch_one(&mut *c).await?;
    if !ok {
        return Ok(GcReport::empty(0, Some("another run holds the lock")));
    }
    let res = sweep(pool, store, opts, now).await;
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)").bind(LOCK).execute(&mut *c).await;
    res
}

async fn sweep(pool: &PgPool, store: &ObjectStore, opts: GcOptions, now: SystemTime) -> Result<GcReport, GcError> {
    let grace_ms = opts.grace_hours * 3_600_000.0;
    let now_ms = now.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis() as f64;
    let cutoff = now_ms - grace_ms;
    let mut candidates: Vec<(String, u64)> = Vec::new();
    let listed = store.list().await?;
    let scanned = listed.len();
    for o in listed {
        let modified = o.modified.duration_since(SystemTime::UNIX_EPOCH).unwrap_or(Duration::ZERO).as_millis() as f64;
        if modified < cutoff {
            candidates.push((o.sha, o.size));
        }
    }
    if candidates.is_empty() {
        return Ok(GcReport::empty(scanned, None));
    }
    // Mark after listing, so any reference written before now is seen.
    let live = mark_referenced(pool).await?;
    // Sanity brake: an empty mark with stored objects means a misconfigured database, not garbage.
    if live.is_empty() && scanned > 0 {
        return Ok(GcReport::empty(scanned, Some("no references found; refusing to sweep")));
    }
    let (mut deleted, mut bytes) = (0, 0);
    for (sha, size) in candidates {
        if live.contains(&sha) {
            continue;
        }
        if !opts.dry_run {
            store.delete(&sha).await?;
        }
        deleted += 1;
        bytes += size;
    }
    Ok(GcReport { scanned, deleted, bytes, skipped: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_and_log() {
        let o = GcOptions::from_env(|_| None);
        assert_eq!(o, GcOptions { grace_hours: 48.0, dry_run: false });
        let o = GcOptions::from_env(|k| Some(if k == "OC_GC_GRACE_HOURS" { " 2 ".into() } else { "1".into() }));
        assert_eq!(o, GcOptions { grace_hours: 2.0, dry_run: true });
        assert!(GcOptions::from_env(|k| (k == "OC_GC_GRACE_HOURS").then(|| "x".into())).grace_hours.is_nan());
        let r = GcReport { scanned: 10, deleted: 2, bytes: 1_250_000, skipped: None };
        assert_eq!(r.log_line(), "[worker] object gc: 2/10 objects, 1.3 MB reclaimed");
        assert_eq!(GcReport::empty(0, Some("x")).log_line(), "[worker] object gc: 0/0 objects, 0.0 MB reclaimed (x)");
        assert_eq!(to_fixed_1(0.25), "0.3");
        assert_eq!(to_fixed_1(0.35), "0.3"); // 0.35 is below the tie as a double
        assert_eq!(to_fixed_1(2.5), "2.5");
        assert_eq!(to_fixed_1(0.05), "0.1");
        assert_eq!(to_fixed_1(1.25), "1.3");
    }

    #[test]
    fn hex_runs() {
        let a = "a".repeat(64);
        let v = format!("{a}{a}zz{}", "b".repeat(100));
        let m: Vec<_> = HEX64.find_iter(v.as_bytes()).map(|m| m.as_bytes().to_vec()).collect();
        assert_eq!(m.len(), 3);
    }
}
