//! Port of services/api/src/brandhub.ts — Brand Hub (A8) helpers shared by routes: brand controls, the
//! workspace that governs a design, and server-side writes to stored documents.
//!
//! Ported: `CONTROL_KEYS`, `isAdminRole`, `brandControls`, `governingWorkspace`, `roleIn`, `loadYDoc`.

use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::http::error::ApiError;
use crate::state::AppState;

pub const CONTROL_KEYS: [&str; 4] = ["colors", "fonts", "requireTemplate", "restrictUploads"];

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
            Some(Value::Object(mut m)) => {
                m.retain(|k, _| CONTROL_KEYS.contains(&k.as_str()));
                m
            }
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

/// `persistServerUpdates(designId, updates, actorId)`: append server-made Yjs updates to the op log (merged into one
/// row) and announce them to the collab server, which merges them into the live document if the design is open.
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
