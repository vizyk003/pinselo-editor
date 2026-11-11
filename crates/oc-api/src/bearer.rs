//! Port of services/api/src/bearer.ts — bearer tokens for programmatic access, one table (`api_tokens`) for both
//! kinds: `oc_pat_…` personal access tokens (REST API, AI assistants) and `oc_oat_…` OAuth access tokens issued to a
//! connected app (AI connectors), refreshed with `oc_ort_…`. Only SHA-256 hashes are stored.
//!
//! Audience (RFC 8707): every token is checked against the resource it is presented to ([`Audience`]). On `/mcp`:
//! OAuth access tokens whose resource is this server's MCP endpoint (NULL on rows from before resources were always
//! recorded; those were only ever issued for /mcp) and personal access tokens — intended: CLI clients (Claude Code,
//! Codex) connect with a PAT the user created with explicit scopes. On the REST API (`/v1`): personal access tokens
//! only; an OAuth token was granted to an AI app for the MCP endpoint.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use uuid::Uuid;

use crate::http::error::ApiError;
use crate::state::AppState;

/// `sha256(s)`: hex.
pub fn sha256(s: &str) -> String {
    crate::auth::sha(s)
}

/// `newToken(kind)`: `oc_<kind>_` + 32 random bytes, base64url.
pub fn new_token(kind: &str) -> String {
    format!("oc_{kind}_{}", URL_SAFE_NO_PAD.encode(crate::auth::random_bytes(32)))
}

/// The token's user (`BearerUser`).
#[derive(Debug, Clone)]
pub struct BearerUser {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub color: String,
    pub scopes: Vec<String>,
    pub token_id: Uuid,
    /// `'pat' | 'oauth'`
    pub kind: String,
    /// The connected app (OAuth) or the token's own name (personal access token), for attribution.
    pub client_name: String,
    pub client_id: Option<String>,
    /// The grant the token belongs to: the OAuth token family, or the PAT's own id (export links are bound to it).
    pub grant_id: Uuid,
    /// A personal access token limited to the designs of one workspace (REST API only).
    pub workspace_id: Option<Uuid>,
}

/// Where a token is presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// The MCP endpoint (`/mcp`).
    Mcp,
    /// The public REST API (`/v1`).
    Rest,
}

/// `audienceAllows(audience, kind, resource)`.
pub fn audience_allows(s: &AppState, audience: Audience, kind: &str, resource: Option<&str>) -> bool {
    if kind == "pat" {
        return true;
    }
    audience == Audience::Mcp && resource.is_none_or(|r| r == format!("{}/mcp", s.cfg.web_origin))
}

/// `bearerShaped(header)`: whether a header has a token's shape (only those are looked up).
pub fn bearer_shaped(header: Option<&str>) -> bool {
    bearer_token(header.unwrap_or("")).is_some()
}

/// `/^Bearer (oc_(?:pat|oat)_[\w-]{20,})$/` → the token.
pub fn bearer_token(header: &str) -> Option<&str> {
    let t = header.strip_prefix("Bearer ")?;
    let rest = t.strip_prefix("oc_pat_").or_else(|| t.strip_prefix("oc_oat_"))?;
    (rest.len() >= 20 && rest.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')).then_some(t)
}

type Row = (Uuid, Uuid, Vec<String>, String, Option<String>, String, Option<String>, String, String, String, Option<chrono::DateTime<chrono::Utc>>, Option<String>, Option<Uuid>, Option<Uuid>);

/// `resolveBearer(header, audience)`: the token's user, or `None` when missing, unknown, expired, revoked, not meant
/// for `audience` (or the user is disabled). A token limited to one workspace is for the REST API only: the MCP tools
/// do not apply the limit. Touches `last_used_at` at most once a minute, in the background.
pub async fn resolve_bearer(s: &AppState, header: Option<&str>, audience: Audience) -> Result<Option<BearerUser>, ApiError> {
    let Some(token) = bearer_token(header.unwrap_or("")) else { return Ok(None) };
    let t: Option<Row> = sqlx::query_as(
        "SELECT t.id, t.user_id, t.scopes, t.kind, t.client_id, t.name AS token_name, c.name AS client_name, u.email, u.name, u.color, u.disabled_at, t.resource, t.family, t.workspace_id
     FROM api_tokens t JOIN users u ON u.id = t.user_id LEFT JOIN oauth_clients c ON c.id = t.client_id
     WHERE t.token_sha256 = $1 AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > now()) AND u.deleted_at IS NULL",
    )
    .bind(sha256(token))
    .fetch_optional(&s.pool)
    .await?;
    let Some((id, user_id, scopes, kind, client_id, token_name, client_name, email, name, color, disabled, resource, family, workspace_id)) = t else { return Ok(None) };
    if disabled.is_some() || !audience_allows(s, audience, &kind, resource.as_deref()) || (workspace_id.is_some() && audience != Audience::Rest) {
        return Ok(None);
    }
    let pool = s.pool.clone();
    s.spawn(async move {
        let _ = sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')")
            .bind(id)
            .execute(&pool)
            .await;
    });
    Ok(Some(BearerUser { id: user_id, email, name, color, scopes, token_id: id, kind, client_name: client_name.unwrap_or(token_name), client_id, grant_id: family.unwrap_or(id), workspace_id }))
}
