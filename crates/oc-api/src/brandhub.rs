//! Port of services/api/src/brandhub.ts — Brand Hub (A8) helpers shared by routes: brand controls, the
//! workspace that governs a design, brand-admin checks, document snapshots for approvals and the approval gate
//! on download/share/publish.
//!
//! Ported: `CONTROL_KEYS`, `isAdminRole`, `brandControls`, `teamEntitlements`, `governingWorkspace`, `roleIn`,
//! `isBrandAdmin`, `loadYDoc`, `docSnapshot`, `sameSnapshot`, `latestApproval` (the columns the gate reads),
//! `approvalCurrent`, `APPROVAL_REQUIRED_MESSAGE`, `approvalGate`, `brandLocksOf`, `applyBrandLocks`,
//! `templateUpdateOps`, `applyServerOps`.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use uuid::Uuid;
use yrs::updates::decoder::Decode as _;
use yrs::updates::encoder::Encode as _;
use yrs::{ReadTxn as _, Snapshot, Transact as _};

use crate::billing::{self, Entitlements};
use crate::http::error::ApiError;
use crate::state::AppState;

pub const CONTROL_KEYS: [&str; 5] = ["colors", "fonts", "requireTemplate", "restrictUploads", "requireApproval"];

/// `isAdminRole(r)`.
pub fn is_admin_role(r: Option<&str>) -> bool {
    matches!(r, Some("owner" | "admin"))
}

/// `brandControls(workspaceId)`: the team's controls (`{}` for none / a personal workspace).
pub async fn brand_controls(state: &AppState, workspace_id: Option<Uuid>) -> Result<Map<String, Value>, ApiError> {
    let Some(ws) = workspace_id else { return Ok(Map::new()) };
    let r: Option<(Option<sqlx::types::Json<Value>>, bool)> = sqlx::query_as("SELECT brand_controls, personal FROM workspaces WHERE id = $1").bind(ws).fetch_optional(&state.pool).await?;
    Ok(match r {
        Some((c, false)) => match c.map(|j| j.0) {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        },
        _ => Map::new(),
    })
}

/// JS truthiness of a control (`Boolean(controls.requireTemplate)`).
pub fn control_on(c: &Map<String, Value>, key: &str) -> bool {
    match c.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `teamEntitlements(workspaceId)`.
pub async fn team_entitlements(state: &AppState, workspace_id: Uuid) -> Result<Entitlements, ApiError> {
    billing::workspace_entitlements(state, workspace_id).await
}

/// `governingWorkspace(designId)`: its own workspace when that is a team, otherwise the team of the brand
/// template it was started from.
pub async fn governing_workspace(state: &AppState, design_id: &str) -> Result<Option<Uuid>, ApiError> {
    let d: Option<(Uuid, bool, Option<Uuid>)> = sqlx::query_as(
        "SELECT d.workspace_id, w.personal, d.brand_workspace_id FROM designs d JOIN workspaces w ON w.id = d.workspace_id WHERE d.id = $1",
    )
    .bind(design_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(d.and_then(|(ws, personal, brand)| if personal { brand } else { Some(ws) }))
}

/// `roleIn(userId, workspaceId)`.
pub async fn role_in(state: &AppState, user_id: Uuid, workspace_id: Option<Uuid>) -> Result<Option<String>, ApiError> {
    let Some(ws) = workspace_id else { return Ok(None) };
    Ok(sqlx::query_scalar("SELECT role FROM memberships WHERE workspace_id = $1 AND user_id = $2").bind(ws).bind(user_id).fetch_optional(&state.pool).await?)
}

/// `isBrandAdmin(userId, designId)`: may change or ignore brand-template locks.
pub async fn is_brand_admin(state: &AppState, user_id: Uuid, design_id: &str) -> Result<bool, ApiError> {
    let d: Option<(Option<Uuid>, Option<bool>)> =
        sqlx::query_as("SELECT d.brand_workspace_id, t.brand FROM designs d LEFT JOIN templates t ON t.id = d.template_id WHERE d.id = $1").bind(design_id).fetch_optional(&state.pool).await?;
    let Some((Some(ws), Some(true))) = d else { return Ok(true) };
    Ok(is_admin_role(role_in(state, user_id, Some(ws)).await?.as_deref()))
}

// ---------------------------------------------------------------- document state

/// `loadYDoc(designId)`: compacted snapshot plus the op-log tail (what collab would load).
pub async fn load_ydoc(state: &AppState, design_id: &str) -> Result<Option<yrs::Doc>, ApiError> {
    let st: Option<(Vec<u8>, Option<i64>)> = sqlx::query_as("SELECT state, seq FROM doc_state WHERE design_id = $1").bind(design_id).fetch_optional(&state.pool).await?;
    let Some((bytes, seq)) = st else { return Ok(None) };
    let ydoc = oc_crdt::new_doc();
    oc_crdt::apply_update(&ydoc, &bytes, None)?;
    let tail: Vec<Vec<u8>> = sqlx::query_scalar("SELECT update FROM doc_updates WHERE design_id = $1 AND seq > $2 ORDER BY seq").bind(design_id).bind(seq.unwrap_or(0)).fetch_all(&state.pool).await?;
    for u in tail {
        oc_crdt::apply_update(&ydoc, &u, None)?;
    }
    Ok(Some(ydoc))
}

/// `docSnapshot(designId)`: `Y.encodeSnapshot(Y.snapshot(ydoc))`.
pub async fn doc_snapshot(state: &AppState, design_id: &str) -> Result<Option<Vec<u8>>, ApiError> {
    let Some(ydoc) = load_ydoc(state, design_id).await? else { return Ok(None) };
    let snap = ydoc.transact().snapshot();
    Ok(Some(snap.encode_v1()))
}

/// A snapshot in a canonical form: state vector entries and merged delete ranges per client.
type Canonical = (BTreeMap<yrs::block::ClientID, u32>, BTreeMap<yrs::block::ClientID, Vec<(u32, u32)>>);

fn canonical(s: &Snapshot) -> Canonical {
    let sv = s.state_map.iter().map(|(c, k)| (*c, *k)).collect();
    let mut ds: BTreeMap<yrs::block::ClientID, Vec<(u32, u32)>> = BTreeMap::new();
    for (client, ranges) in s.delete_set.iter() {
        let mut v: Vec<(u32, u32)> = ranges.iter().map(|r| (r.start, r.end)).filter(|(a, b)| b > a).collect();
        v.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::new();
        for (a, b) in v {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        if !merged.is_empty() {
            ds.insert(*client, merged);
        }
    }
    (sv, ds)
}

/// `sameSnapshot(a, b)`: `Y.equalSnapshots` of two encoded snapshots (false when either is missing/corrupt).
pub fn same_snapshot(a: Option<&[u8]>, b: Option<&[u8]>) -> bool {
    let (Some(a), Some(b)) = (a, b) else { return false };
    match (Snapshot::decode_v1(a), Snapshot::decode_v1(b)) {
        (Ok(a), Ok(b)) => canonical(&a) == canonical(&b),
        _ => false,
    }
}

// ---------------------------------------------------------------- approvals

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ApprovalRow {
    pub id: Uuid,
    pub design_id: String,
    pub status: String,
    pub approved_snapshot: Option<Vec<u8>>,
}

/// `latestApproval(designId)`.
pub async fn latest_approval(state: &AppState, design_id: &str) -> Result<Option<ApprovalRow>, ApiError> {
    Ok(sqlx::query_as("SELECT * FROM design_approvals WHERE design_id = $1 AND status <> 'cancelled' ORDER BY created_at DESC LIMIT 1").bind(design_id).fetch_optional(&state.pool).await?)
}

/// `approvalCurrent(a)`: approved and unchanged since.
pub async fn approval_current(state: &AppState, a: Option<&ApprovalRow>) -> Result<bool, ApiError> {
    let Some(a) = a.filter(|a| a.status == "approved") else { return Ok(false) };
    let now = doc_snapshot(state, &a.design_id).await?;
    Ok(same_snapshot(a.approved_snapshot.as_deref(), now.as_deref()))
}

pub const APPROVAL_REQUIRED_MESSAGE: &str = "This design needs to be approved before it can be downloaded, shared or published. Send it for approval from the Share menu.";

/// `approvalGate(userId, designId)`: the 403 body, or `None` when the action may proceed.
pub async fn approval_gate(state: &AppState, user_id: Uuid, design_id: &str) -> Result<Option<Value>, ApiError> {
    let ws = governing_workspace(state, design_id).await?;
    let Some(ws) = ws else { return Ok(None) };
    if !control_on(&brand_controls(state, Some(ws)).await?, "requireApproval") {
        return Ok(None);
    }
    if is_admin_role(role_in(state, user_id, Some(ws)).await?.as_deref()) {
        return Ok(None);
    }
    let a = latest_approval(state, design_id).await?;
    if approval_current(state, a.as_ref()).await? {
        return Ok(None);
    }
    let status = match &a {
        None => "none".to_owned(),
        Some(a) if a.status == "approved" => "stale".to_owned(),
        Some(a) => a.status.clone(),
    };
    let message = if status == "stale" {
        "This design changed after it was approved. Send it for approval again before downloading, sharing or publishing."
    } else {
        APPROVAL_REQUIRED_MESSAGE
    };
    Ok(Some(json!({ "error": "approval_required", "status": status, "message": message })))
}

// ---------------------------------------------------------------- brand-template helpers

/// `brandLocksOf(doc)`: element locks set `by: 'brand'`, keyed by element id.
pub fn brand_locks_of(doc: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(els) = doc.get("elements").and_then(Value::as_object) {
        for e in els.values() {
            let deleted = e.get("deleted").is_some_and(|d| !d.is_null() && d != &Value::Bool(false));
            if !deleted
                && e.get("lock").and_then(|l| l.get("by")).and_then(Value::as_str) == Some("brand")
                && let Some(id) = e.get("id").and_then(Value::as_str)
            {
                out.insert(id.to_owned(), e["lock"].clone());
            }
        }
    }
    out
}

/// `applyBrandLocks(doc, locks)`: the template's lock map is authoritative.
pub fn apply_brand_locks(doc: &mut Value, locks: &Map<String, Value>) {
    let Some(els) = doc.get_mut("elements").and_then(Value::as_object_mut) else { return };
    for e in els.values_mut() {
        let id = e.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
        let Some(obj) = e.as_object_mut() else { continue };
        if let Some(want) = locks.get(&id).filter(|w| !w.is_null()) {
            let mut l = want.as_object().cloned().unwrap_or_default();
            l.insert("by".into(), Value::String("brand".into()));
            obj.insert("lock".into(), Value::Object(l));
        } else if obj.get("lock").and_then(|l| l.get("by")).and_then(Value::as_str) == Some("brand") {
            obj.remove("lock");
        }
    }
}

// ---------------------------------------------------------------- template updates (routes/brandhub.ts push)

/// Element keys `templateUpdateOps` never copies as style.
const STYLE_SKIP: [&str; 9] = ["id", "type", "parentId", "z", "xf", "lock", "deleted", "name", "altText"];

fn style_skip(k: &str) -> bool {
    STYLE_SKIP.contains(&k) || k == "decorative"
}

fn deleted(e: &Value) -> bool {
    oc_crdt::value::truthy_opt(e.get("deleted"))
}

/// `{ ...lock, by: 'brand' }`.
fn brand_lock(lock: &Value) -> Value {
    let mut l = lock.as_object().cloned().unwrap_or_default();
    l.insert("by".into(), Value::String("brand".into()));
    Value::Object(l)
}

/// `templateUpdateOps(tpl, cur, locks)`: ops that bring a derived design's brand-locked elements in line with the
/// template (as far as they're locked). Values are compared by content (the TS code compares `JSON.stringify`
/// output, which also depends on key order; see ROUTES.md).
pub fn template_update_ops(tpl: &Value, cur: &Value, locks: &Map<String, Value>) -> Vec<oc_crdt::DocOp> {
    let mut ops = Vec::new();
    let empty = Map::new();
    let tpl_els = tpl.get("elements").and_then(Value::as_object).unwrap_or(&empty);
    let cur_els = cur.get("elements").and_then(Value::as_object).unwrap_or(&empty);
    let cur_pages = cur.get("pages").and_then(Value::as_object).unwrap_or(&empty);
    for (id, lock) in locks {
        let Some(src) = tpl_els.get(id).filter(|s| !deleted(s)) else { continue };
        let parent = src.get("parentId").and_then(Value::as_str).unwrap_or("").to_owned();
        let Some(el) = cur_els.get(id).filter(|e| !deleted(e)) else {
            // Re-add a locked element the design lost (only when its parent still exists).
            if cur_pages.contains_key(&parent) || cur_els.contains_key(&parent) {
                let mut add = src.as_object().cloned().unwrap_or_default();
                add.insert("lock".into(), brand_lock(lock));
                ops.push(oc_crdt::DocOp::Add { el: Value::Object(add), parent, after: None });
            }
            continue;
        };
        let mut patch = Map::new();
        let flag = |k: &str| oc_crdt::value::truthy_opt(lock.get(k));
        if (flag("move") || flag("resize") || flag("rotate")) && !oc_crdt::value::same(src.get("xf"), el.get("xf")) {
            patch.insert("xf".into(), src.get("xf").cloned().unwrap_or(Value::Null));
        }
        let (s, c) = (src.as_object().unwrap_or(&empty), el.as_object().unwrap_or(&empty));
        let mut keys: Vec<&String> = s.keys().collect();
        for k in c.keys() {
            if !s.contains_key(k) {
                keys.push(k);
            }
        }
        for k in keys {
            if style_skip(k) {
                continue;
            }
            let locked = if oc_crdt::brandlock::CONTENT_KEYS.contains(&k.as_str()) { flag("content") } else { flag("style") };
            if locked && !oc_crdt::value::same(s.get(k), c.get(k)) {
                patch.insert(k.clone(), s.get(k).cloned().unwrap_or(Value::Null));
            }
        }
        let want = brand_lock(lock);
        if !oc_crdt::value::same(el.get("lock"), Some(&want)) {
            patch.insert("lock".into(), want);
        }
        if !patch.is_empty() {
            ops.push(oc_crdt::DocOp::Update { id: id.clone(), patch });
        }
    }
    ops
}

/// `applyServerOps(designId, ops)`: apply DocOps to a stored design from the server (template updates). The
/// change is appended to the op log and announced to the collab server, which merges it into the live document
/// if the design is open. `Err(message)` when the ops could not be applied.
pub async fn apply_server_ops(state: &AppState, design_id: &str, ops: &[oc_crdt::DocOp]) -> Result<Result<(), String>, ApiError> {
    let Some(ydoc) = load_ydoc(state, design_id).await? else { return Ok(Err("no document".into())) };
    let merged: Vec<Vec<u8>> = {
        let captured: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>> = std::sync::Arc::default();
        let c = captured.clone();
        ydoc.observe_update_v1("oc-api-server-ops", move |_: &yrs::TransactionMut, e: &yrs::UpdateEvent| c.lock().expect("updates").push(e.update.clone()))
            .map_err(ApiError::internal)?;
        let r = oc_crdt::apply_ops(
            &ydoc,
            ops,
            Some(yrs::Origin::from("server")),
            oc_crdt::ApplyOptions { actor: Some("template".into()), now: Some(state.now_ms() as f64), ..Default::default() },
        );
        let _ = ydoc.unobserve_update_v1("oc-api-server-ops");
        drop(ydoc);
        if !r.ok {
            return Ok(Err(r.errors.first().map(|e| e.message.clone()).unwrap_or_default()));
        }
        std::mem::take(&mut *captured.lock().expect("updates"))
    };
    persist_server_updates(state, design_id, &merged, None).await?;
    Ok(Ok(()))
}

/// `persistServerUpdates(designId, updates, actorId)`: append server-made Yjs updates to the op log (merged into one
/// row) and announce them to the collab server (see [`apply_server_ops`]).
pub async fn persist_server_updates(state: &AppState, design_id: &str, updates: &[Vec<u8>], actor_id: Option<uuid::Uuid>) -> Result<(), ApiError> {
    if updates.is_empty() {
        return Ok(());
    }
    let merged = yrs::merge_updates_v1(updates).map_err(ApiError::internal)?;
    let seq: Option<String> =
        sqlx::query_scalar("INSERT INTO doc_updates(design_id, update, actor_id) VALUES ($1, $2, $3) RETURNING seq::text").bind(design_id).bind(&merged).bind(actor_id).fetch_optional(&state.pool).await?;
    let mut note = Map::new();
    note.insert("designId".into(), json!(design_id));
    note.insert("seq".into(), seq.map_or(Value::Null, Value::String));
    if merged.len() < 5000 {
        use base64::Engine as _;
        note.insert("update".into(), json!(base64::engine::general_purpose::STANDARD.encode(&merged)));
    }
    sqlx::query("SELECT pg_notify($1, $2)").bind("oc_doc_update").bind(crate::http::jsjson::stringify(&Value::Object(note))).execute(&state.pool).await?;
    sqlx::query("UPDATE designs SET updated_at = now() WHERE id = $1").bind(design_id).execute(&state.pool).await?;
    Ok(())
}
