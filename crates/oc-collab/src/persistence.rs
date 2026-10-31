//! Postgres persistence (the `onLoadDocument` / `beforeHandleMessage` / `onChange` / `onStoreDocument`
//! hooks of services/collab/src/main.ts). Tables and statements are unchanged:
//! `doc_state` (compacted snapshot + `seq` watermark), `doc_updates` (op log), `designs`, `design_versions`.

use serde_json::{Map, Value};
use sqlx::PgPool;
use uuid::Uuid;
use yrs::{Doc, Map as _, Transact};

use oc_crdt::repair::{RepairReport, upgrade_y};

/// `onLoadDocument`: snapshot + op-log tail, then schema migrations and the structural repair pass (CO-004).
/// A repair is persisted through the op log like any other change. Returns the repair report when one ran.
pub async fn load(pool: &PgPool, design_id: &str, doc: &Doc) -> Result<Option<RepairReport>, String> {
    let row: Option<(Vec<u8>, i64)> = sqlx::query_as("SELECT state, seq FROM doc_state WHERE design_id = $1")
        .bind(design_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    if let Some((state, _)) = &row {
        oc_crdt::apply_update(doc, state, None).map_err(|e| e.to_string())?;
    }
    // Replay the op-log tail written after the last snapshot.
    let tail: Vec<(Vec<u8>,)> = sqlx::query_as("SELECT update FROM doc_updates WHERE design_id = $1 AND seq > $2 ORDER BY seq")
        .bind(design_id)
        .bind(row.as_ref().map_or(0, |r| r.1))
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    for (u,) in &tail {
        oc_crdt::apply_update(doc, u, None).map_err(|e| e.to_string())?;
    }
    if page_count(doc) > 0 {
        let rep = upgrade_y(doc, oc_js::now_ms() as f64).map_err(|e| e.to_string())?;
        if rep.orphans > 0 || rep.cycles > 0 || rep.transforms > 0 || rep.migrated_from.is_some() {
            println!("[collab] repaired {design_id} {}", serde_json::to_string(&rep).unwrap_or_default());
            // Persist the repair through the op log like any other change.
            sqlx::query("INSERT INTO doc_updates(design_id, update) VALUES ($1, $2)")
                .bind(design_id)
                .bind(oc_crdt::encode_state(doc))
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(Some(rep));
        }
    }
    Ok(None)
}

/// `roots(document).pages.size`.
pub fn page_count(doc: &Doc) -> u32 {
    let r = oc_crdt::roots(doc);
    let txn = doc.transact();
    r.pages.len(&txn)
}

/// The document meta (`readMeta(roots(document).meta)`).
pub fn meta(doc: &Doc) -> Map<String, Value> {
    let r = oc_crdt::roots(doc);
    let txn = doc.transact();
    oc_crdt::read_meta(&txn, &r.meta)
}

/// Append one update to the op log.
pub async fn log_update(pool: &PgPool, design_id: &str, update: &[u8], actor: Option<Uuid>) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO doc_updates(design_id, update, actor_id) VALUES ($1, $2, $3)")
        .bind(design_id)
        .bind(update)
        .bind(actor)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Logged updates the snapshot doesn't cover yet: `doc_state.seq < seq <= upto`, in order.
pub async fn unfolded(pool: &PgPool, design_id: &str, upto: i64) -> Result<Vec<Vec<u8>>, sqlx::Error> {
    sqlx::query_scalar("SELECT update FROM doc_updates WHERE design_id = $1 AND seq > COALESCE((SELECT seq FROM doc_state WHERE design_id = $1), 0) AND seq <= $2 ORDER BY seq")
        .bind(design_id)
        .bind(upto)
        .fetch_all(pool)
        .await
}

/// `SELECT max(seq) FROM doc_updates` for a design (`'0'` when the log is empty).
pub async fn max_seq(pool: &PgPool, design_id: &str) -> Result<i64, sqlx::Error> {
    let v: Option<i64> = sqlx::query_scalar("SELECT max(seq) FROM doc_updates WHERE design_id = $1").bind(design_id).fetch_one(pool).await?;
    Ok(v.unwrap_or(0))
}

/// What `onStoreDocument` writes.
pub struct StoreData {
    pub upto: i64,
    pub state: Vec<u8>,
    pub page_count: u32,
    pub meta: Map<String, Value>,
}

/// `onStoreDocument` (DB part): fold the log into the snapshot, truncate it, mirror the dims onto the
/// designs row. The version snapshot and thumbnail refresh are done by the caller.
pub async fn store(pool: &PgPool, design_id: &str, d: &StoreData) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO doc_state(design_id, state, seq, updated_at) VALUES ($1, $2, $3, now())
       ON CONFLICT (design_id) DO UPDATE SET state = EXCLUDED.state, seq = GREATEST(doc_state.seq, EXCLUDED.seq), updated_at = now()",
    )
    .bind(design_id)
    .bind(&d.state)
    .bind(d.upto)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM doc_updates WHERE design_id = $1 AND seq <= $2").bind(design_id).bind(d.upto).execute(pool).await?;
    // The title is owned by the designs row (renamed via the API); the doc meta mirrors it.
    let dims = d.meta.get("dims");
    let num = |k: &str| dims.and_then(|x| x.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
    let unit = dims.and_then(|x| x.get("unit")).and_then(Value::as_str).unwrap_or("px").to_owned();
    sqlx::query("UPDATE designs SET updated_at = now(), page_count = $2, width = $3, height = $4, unit = $5 WHERE id = $1")
        .bind(design_id)
        .bind(d.page_count as i32)
        .bind(num("w"))
        .bind(num("h"))
        .bind(unit)
        .execute(pool)
        .await?;
    Ok(())
}

/// Automatic version snapshot.
pub async fn insert_version(pool: &PgPool, design_id: &str, state: &[u8], created_by: Option<Uuid>) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO design_versions(id, design_id, state, created_by) VALUES ($1,$2,$3,$4)")
        .bind(oc_db::uuid())
        .bind(design_id)
        .bind(state)
        .bind(created_by)
        .execute(pool)
        .await
        .map(|_| ())
}
