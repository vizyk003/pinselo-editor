//! Port of services/api/src/analytics.ts — first-party product analytics (PL-040). Events carry a user id only
//! when that user granted analytics consent; otherwise they are stored anonymously.

use serde_json::Value;
use uuid::Uuid;

use crate::state::AppState;

pub const CONSENT_VERSION: &str = "2026-10-01";

/// Client events we accept; anything else is dropped.
pub const CLIENT_EVENTS: [&str; 16] = [
    "page_view", "editor_open", "element_add", "template_use", "export_start", "share_open", "present_start", "upgrade_view", "checkout_start", "search", "help_view", "upload", "comment_add",
    "ai_use", "onboarding_step", "import_done",
];

/// `analyticsConsent(userId, anonId)`.
pub async fn analytics_consent(state: &AppState, user_id: Option<Uuid>, anon_id: Option<&str>) -> Result<bool, sqlx::Error> {
    if user_id.is_none() && anon_id.is_none() {
        return Ok(false);
    }
    let r: Option<bool> = sqlx::query_scalar(
        "SELECT granted FROM consents WHERE kind = 'analytics' AND (($1::uuid IS NOT NULL AND user_id = $1) OR ($2::text IS NOT NULL AND anon_id = $2))
     ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(anon_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(r.unwrap_or(false))
}

/// `track(userId, name, props)`: server-side event. Insert errors are swallowed like the TS `.catch(() => {})`;
/// a failing consent lookup propagates (as in TS).
pub async fn track(state: &AppState, user_id: Option<Uuid>, name: &str, props: Value) -> Result<(), sqlx::Error> {
    let ok = match user_id {
        Some(_) => analytics_consent(state, user_id, None).await?,
        None => false,
    };
    let _ = sqlx::query("INSERT INTO analytics_events(user_id, name, props) VALUES ($1,$2,$3)")
        .bind(if ok { user_id } else { None })
        .bind(name)
        .bind(sqlx::types::Json(&props))
        .execute(&state.pool)
        .await;
    Ok(())
}
