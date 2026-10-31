//! Port of services/api/src/print/updates.ts — fulfiller webhook updates → order status (idempotent, state-machine
//! checked; stale updates become notes).

use serde_json::Value;

use crate::http::error::ApiError;
use crate::pg;
use crate::print::states::can_transition;
use crate::print::store::{Order, OrderExt, Patch, StatusChange, StatusError, add_note, get_order_by_number, set_status};
use crate::print::types::ProviderUpdate;
use crate::state::AppState;

/// `applyProviderUpdate(provider, update)` → `applied` | `duplicate` | `unknown_order` | `noted`.
pub async fn apply_provider_update(s: &AppState, provider: &str, u: &ProviderUpdate) -> Result<&'static str, StatusError> {
    let fresh = pg::one(
        &s.pool,
        "INSERT INTO print_webhook_events(id, provider) VALUES ($1,$2) ON CONFLICT DO NOTHING RETURNING id",
        &[pg::text(crate::js::take(&u.event_id, 300)), pg::text(provider)],
    )
    .await
    .map_err(ApiError::from)?;
    if fresh.is_none() {
        return Ok("duplicate");
    }
    let mut o: Option<Order> = None;
    if let Some(n) = u.order_number.as_deref().filter(|n| !n.is_empty()) {
        o = get_order_by_number(s, n).await?;
    }
    if o.is_none()
        && let Some(pid) = u.provider_order_id.as_deref().filter(|n| !n.is_empty())
    {
        o = match pg::json_one(&s.pool, "SELECT * FROM print_orders WHERE provider = $1 AND provider_order_id = $2", &[pg::text(provider), pg::text(pid)]).await.map_err(ApiError::from)? {
            Some(Value::Object(m)) => Some(m),
            _ => None,
        };
    }
    let Some(o) = o else { return Ok("unknown_order") };
    let patch = Patch { tracking: Some(u.tracking.clone()), provider_order_id: u.provider_order_id.clone(), ..Default::default() };
    let cur = o.status();
    if let Some(to) = u.status.filter(|to| *to != cur && can_transition(&cur, to)) {
        let mut patch = patch;
        if to == "failed" {
            patch.error = Some(u.note.clone());
        }
        set_status(s, &o.id(), to, StatusChange { source: "provider", note: Some(u.note.clone()), patch, ..Default::default() }).await?;
        return Ok("applied");
    }
    if !u.tracking.is_empty() {
        set_status(s, &o.id(), &cur, StatusChange { source: "provider", note: Some(u.note.clone()), patch, ..Default::default() }).await?;
    } else {
        add_note(s, &o.id(), &cur, &u.note, "provider", None).await?;
    }
    Ok("noted")
}
