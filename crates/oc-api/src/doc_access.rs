//! Port of services/api/src/docAccess.ts — embedded designs in docs (DC-030) are shown only to people who can
//! view them.

use oc_auth::Role;
use uuid::Uuid;

use crate::auth;
use crate::http::error::ApiError;
use crate::state::AppState;

/// The most embeds a doc's export resolves (bounds the access checks and renders).
pub const MAX_EMBEDS: usize = 50;

fn valid_id(id: &str) -> bool {
    let n = id.encode_utf16().count();
    (1..=64).contains(&n) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `filterViewable(userId, ids, role)`: the ids the user can at least view (in order, deduplicated, at most
/// `MAX_EMBEDS`), using `designRole`.
pub async fn filter_viewable(state: &AppState, user_id: Option<Uuid>, ids: &[String]) -> Result<Vec<String>, ApiError> {
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<&String> = ids.iter().filter(|id| seen.insert(id.as_str())).take(MAX_EMBEDS).collect();
    let mut out = Vec::new();
    for id in unique {
        if !valid_id(id) {
            continue;
        }
        if auth::at_least(auth::design_role(state, user_id, id).await?, Role::View) {
            out.push(id.clone());
        }
    }
    Ok(out)
}

/// `viewableEmbeds(userId, state, role)`: designs embedded in a doc's persisted Yjs state that the user may view.
pub async fn viewable_embeds(state: &AppState, user_id: Option<Uuid>, doc_state: &[u8]) -> Result<Vec<String>, ApiError> {
    let ids = oc_crdt::doc_body::embedded_design_ids_of_state(doc_state)?;
    filter_viewable(state, user_id, &ids).await
}
