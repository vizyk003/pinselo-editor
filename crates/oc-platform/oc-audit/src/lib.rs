//! Tamper-evident audit log (TS-022, chain v2 — see migration `0007_audit_chain_v2`): each event stores
//! `hash = sha256_hex(prev_hash ‖ canonical(event))` where `prev_hash` is the previous chained row's hash
//! (`''` for the first) and `canonical` is sorted-key JSON of
//! `{a: action, t: target, u: actor_id, w: workspace_id, m: meta, ip, at: created_at.toISOString()}`.
//! The event's own columns are hashed, so the verifier detects edited rows, deleted rows and inserts that
//! bypassed this module. Rows written here verify with the TS verifier and vice versa.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::types::Json;
use uuid::Uuid;

/// Advisory lock key that serializes chain writers (shared with TS).
pub const CHAIN_LOCK: i64 = 4242;

/// The hashed columns of an `audit_events` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HashedFields {
    pub action: String,
    pub target: Option<String>,
    pub actor_id: Option<Uuid>,
    pub workspace_id: Option<Uuid>,
    pub meta: Option<Value>,
    pub ip: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A stored row as read by the verifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditRow {
    pub id: i64,
    pub prev_hash: Option<String>,
    pub hash: Option<String>,
    #[serde(flatten)]
    pub fields: HashedFields,
}

/// The canonical body that is hashed after `prev`.
pub fn canonical_body(r: &HashedFields) -> String {
    let s = |v: &Option<String>| v.as_ref().map_or(Value::Null, |x| Value::String(x.clone()));
    let u = |v: &Option<Uuid>| v.map_or(Value::Null, |x| Value::String(x.to_string()));
    let mut m = serde_json::Map::new();
    m.insert("a".into(), Value::String(r.action.clone()));
    m.insert("t".into(), s(&r.target));
    m.insert("u".into(), u(&r.actor_id));
    m.insert("w".into(), u(&r.workspace_id));
    m.insert("m".into(), r.meta.clone().unwrap_or(Value::Null));
    m.insert("ip".into(), s(&r.ip));
    m.insert("at".into(), Value::String(oc_js::iso_string(&r.created_at)));
    oc_js::canonical(&Value::Object(m))
}

/// `digest(prev, row)`.
pub fn digest(prev: &str, r: &HashedFields) -> String {
    let mut h = Sha256::new();
    h.update(prev.as_bytes());
    h.update(canonical_body(r).as_bytes());
    hex::encode(h.finalize())
}

/// Verify rows (ordered by id, starting at the first chained row). Returns the id of the first altered,
/// removed-after or unchained event, or `None`.
pub fn verify_rows<'a>(rows: impl IntoIterator<Item = &'a AuditRow>) -> Option<i64> {
    let mut prev = String::new();
    for r in rows {
        let Some(hash) = r.hash.as_deref().filter(|h| !h.is_empty()) else { return Some(r.id) };
        if r.prev_hash.as_deref().unwrap_or("") != prev || digest(&prev, &r.fields) != hash {
            return Some(r.id);
        }
        prev = hash.to_owned();
    }
    None
}

/// An event to record.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuditEvent {
    pub actor_id: Option<Uuid>,
    pub workspace_id: Option<Uuid>,
    pub action: String,
    pub target: Option<String>,
    pub meta: Option<Value>,
    pub ip: Option<String>,
}

/// Truncate to millisecond precision so the stored timestamp round-trips exactly.
pub fn ms_precision(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis()).single().unwrap_or(t)
}

/// Append an event to the chain (serialized with `pg_advisory_xact_lock(4242)`).
pub async fn audit(pool: &PgPool, e: AuditEvent) -> Result<(), sqlx::Error> {
    audit_at(pool, e, Utc::now()).await
}

/// [`audit`] with an explicit timestamp (truncated to ms).
pub async fn audit_at(pool: &PgPool, e: AuditEvent, now: DateTime<Utc>) -> Result<(), sqlx::Error> {
    let mut t = pool.begin().await?;
    // Serialize writers so the chain is linear.
    sqlx::query("SELECT pg_advisory_xact_lock(4242)").execute(&mut *t).await?;
    let prev: String = sqlx::query_scalar::<_, String>("SELECT hash FROM audit_events WHERE hash IS NOT NULL ORDER BY id DESC LIMIT 1").fetch_optional(&mut *t).await?.unwrap_or_default();
    let row = HashedFields {
        action: e.action,
        target: e.target,
        actor_id: e.actor_id,
        workspace_id: e.workspace_id,
        meta: e.meta.filter(|m| !m.is_null()),
        ip: e.ip,
        created_at: ms_precision(now),
    };
    let hash = digest(&prev, &row);
    sqlx::query("INSERT INTO audit_events(workspace_id, actor_id, action, target, meta, ip, created_at, prev_hash, hash) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(row.workspace_id)
        .bind(row.actor_id)
        .bind(&row.action)
        .bind(&row.target)
        .bind(row.meta.as_ref().map(Json))
        .bind(&row.ip)
        .bind(row.created_at)
        .bind(if prev.is_empty() { None } else { Some(prev.as_str()) })
        .bind(&hash)
        .execute(&mut *t)
        .await?;
    t.commit().await
}

type DbRow = (i64, Option<String>, Option<String>, String, Option<String>, Option<Uuid>, Option<Uuid>, Option<Json<Value>>, Option<String>, DateTime<Utc>);

/// Load the chained rows exactly like the TS verifier.
pub async fn load_chain(pool: &PgPool) -> Result<Vec<AuditRow>, sqlx::Error> {
    let rows: Vec<DbRow> = sqlx::query_as(
        "SELECT id, prev_hash, hash, action, target, actor_id, workspace_id, meta, ip, created_at FROM audit_events
     WHERE id >= COALESCE((SELECT min(id) FROM audit_events WHERE hash IS NOT NULL), 0) ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, prev_hash, hash, action, target, actor_id, workspace_id, meta, ip, created_at)| AuditRow {
            id,
            prev_hash,
            hash,
            fields: HashedFields { action, target, actor_id, workspace_id, meta: meta.map(|j| j.0), ip, created_at },
        })
        .collect())
}

/// Verify the chain; returns the id of the first altered, removed-after or unchained event, or `None`.
pub async fn verify_audit_chain(pool: &PgPool) -> Result<Option<i64>, sqlx::Error> {
    Ok(verify_rows(&load_chain(pool).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_shape() {
        let f = HashedFields {
            action: "x".into(),
            target: None,
            actor_id: None,
            workspace_id: None,
            meta: Some(serde_json::json!({"b": 1.0, "a": [0.5]})),
            ip: Some("1.2.3.4".into()),
            created_at: Utc.timestamp_millis_opt(1_700_000_000_123).unwrap(),
        };
        assert_eq!(canonical_body(&f), r#"{"a":"x","at":"2023-11-14T22:13:20.123Z","ip":"1.2.3.4","m":{"a":[0.5],"b":1},"t":null,"u":null,"w":null}"#);
    }
}
