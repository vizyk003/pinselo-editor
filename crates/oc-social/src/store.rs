//! Social account persistence (port of services/api/src/social/store.ts): encrypted tokens,
//! refresh-on-use, visibility (own accounts + accounts shared with my teams).

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::PgPool;
use sqlx::types::Json;
use uuid::Uuid;

use crate::crypto::{open_tokens, seal_tokens};
use crate::http::{Http, slice_utf16};
use crate::types::{ConnectedAccount, PublishError, Tokens};
use crate::{adapter, db_message};

/// A `social_accounts` row (`SELECT a.*`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct AccountRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub provider: String,
    pub external_id: String,
    pub display_name: String,
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
    /// Sealed tokens (`v1.…`); never sent to clients.
    pub tokens: Option<String>,
    pub token_expires_at: Option<DateTime<Utc>>,
    pub meta: Json<Map<String, Value>>,
    /// `'ok' | 'reauth'`.
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `publicAccount(r, me)`: the client-safe shape (never includes tokens). `mine` only when `me` is given,
/// `boards` only when the meta holds an array of them.
pub fn public_account(r: &AccountRow, me: Option<Uuid>) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), Value::from(r.id.to_string()));
    m.insert("provider".into(), Value::from(r.provider.clone()));
    m.insert("displayName".into(), Value::from(r.display_name.clone()));
    m.insert("handle".into(), r.handle.clone().map_or(Value::Null, Value::from));
    m.insert("avatarUrl".into(), r.avatar_url.clone().map_or(Value::Null, Value::from));
    m.insert("status".into(), Value::from(r.status.clone()));
    m.insert(
        "workspaceId".into(),
        r.workspace_id.map_or(Value::Null, |w| Value::from(w.to_string())),
    );
    if let Some(me) = me {
        m.insert("mine".into(), Value::from(r.user_id == me));
    }
    m.insert("createdAt".into(), Value::from(oc_js::iso_string(&r.created_at)));
    if let Some(b @ Value::Array(_)) = r.meta.0.get("boards") {
        m.insert("boards".into(), b.clone());
    }
    Value::Object(m)
}

/// Accounts a user may post to: their own, plus accounts teammates shared with a team they belong to.
pub async fn visible_accounts(pool: &PgPool, user_id: Uuid) -> Result<Vec<AccountRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT a.* FROM social_accounts a
     WHERE a.user_id = $1 OR a.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1)
     ORDER BY (a.provider = 'local') DESC, a.provider, a.display_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Every user has the built-in "Download & remind" target (idempotent).
pub async fn ensure_local_account(pool: &PgPool, user_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO social_accounts(id, user_id, provider, external_id, display_name) VALUES ($1,$2::uuid,'local',$2::text,'Download & remind')
     ON CONFLICT (user_id, provider, external_id) DO NOTHING",
    )
    .bind(oc_db::uuid())
    .bind(user_id.to_string())
    .execute(pool)
    .await
    .map(|_| ())
}

/// `saveAccounts(userId, provider, list)`: upsert each account with sealed tokens; returns the row ids.
pub async fn save_accounts(
    pool: &PgPool,
    secret: &str,
    user_id: Uuid,
    provider: &str,
    list: &[ConnectedAccount],
) -> Result<Vec<Uuid>, SaveAccountsError> {
    // One transaction, refused when the user is no longer active: an OAuth callback finishes after a slow exchange, by
    // which time the account may have been disabled or set aside (`lockActiveUser`, review D1).
    let mut tx = pool.begin().await?;
    let active: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id = $1 AND disabled_at IS NULL AND deleted_at IS NULL FOR SHARE")
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
    if active.is_none() {
        return Err(SaveAccountsError::Refused(PublishError { status: Some(403), ..PublishError::new("This account is disabled.") }));
    }
    let mut ids = Vec::with_capacity(list.len());
    for a in list {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO social_accounts(id, user_id, provider, external_id, display_name, handle, avatar_url, tokens, token_expires_at, meta)
       VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
       ON CONFLICT (user_id, provider, external_id) DO UPDATE SET display_name = EXCLUDED.display_name, handle = EXCLUDED.handle,
         avatar_url = EXCLUDED.avatar_url, tokens = EXCLUDED.tokens, token_expires_at = EXCLUDED.token_expires_at,
         meta = social_accounts.meta || EXCLUDED.meta, status = 'ok', updated_at = now()
       RETURNING id",
        )
        .bind(oc_db::uuid())
        .bind(user_id)
        .bind(provider)
        .bind(&a.external_id)
        .bind(slice_utf16(&a.display_name, 200))
        .bind(&a.handle)
        .bind(&a.avatar_url)
        .bind(seal_tokens(&a.tokens, secret))
        .bind(a.tokens.expires_at.and_then(ms_to_date))
        .bind(Json(Value::Object(a.meta.clone().unwrap_or_default())))
        .fetch_one(&mut *tx)
        .await?;
        ids.push(id);
    }
    tx.commit().await?;
    Ok(ids)
}

/// Why [`save_accounts`] failed: refused (its message is shown, like a TS `PublishError`) or the database.
#[derive(Debug, thiserror::Error)]
pub enum SaveAccountsError {
    #[error("{}", .0.message)]
    Refused(PublishError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// `expiresAt ? new Date(expiresAt) : null` for a timestamptz parameter (0 and NaN are falsy).
pub fn ms_to_date(ms: f64) -> Option<DateTime<Utc>> {
    if ms == 0.0 || !ms.is_finite() {
        return None;
    }
    DateTime::from_timestamp_millis(ms.trunc() as i64)
}

/// Decrypted tokens for a publish, refreshed first when they expire within 5 minutes (or when `force`).
/// A refresh that fails permanently marks the account `reauth`. `now_ms` is `Date.now()`.
pub async fn tokens_for(
    pool: &PgPool,
    http: &Http,
    secret: &str,
    row: &AccountRow,
    force: bool,
    now_ms: f64,
) -> Result<Tokens, PublishError> {
    if row.provider == "local" {
        return Ok(Tokens::default());
    }
    let Some(sealed) = row.tokens.as_deref().filter(|s| !s.is_empty()) else {
        return Err(PublishError {
            message: "This account has no stored credentials — reconnect it.".into(),
            transient: false,
            status: Some(401),
            reauth: true,
        });
    };
    // A decryption failure is a plain Error in TS (wrapped as a transient PublishError by the job).
    let mut t = open_tokens(sealed, secret).map_err(|e| PublishError::transient(e.to_string()))?;
    let Some(a) = adapter(&row.provider) else { return Ok(t) };
    let expiring = t.expires_at.is_some_and(|e| e != 0.0 && e < now_ms + 5.0 * 60_000.0);
    if a.has_refresh() && (force || expiring) {
        let next = match a.refresh(http, &t).await {
            Ok(n) => n,
            Err(e) if e.transient => return Err(e),
            Err(_) => None,
        };
        let Some(next) = next else {
            sqlx::query("UPDATE social_accounts SET status = 'reauth', updated_at = now() WHERE id = $1")
                .bind(row.id)
                .execute(pool)
                .await
                .map_err(|e| PublishError::transient(db_message(&e)))?;
            return Err(PublishError {
                message: format!(
                    "{} access expired — reconnect the account in Settings → Connected accounts.",
                    a.label()
                ),
                transient: false,
                status: Some(401),
                reauth: true,
            });
        };
        t = next;
        sqlx::query("UPDATE social_accounts SET tokens = $2, token_expires_at = $3, status = 'ok', updated_at = now() WHERE id = $1")
            .bind(row.id)
            .bind(seal_tokens(&t, secret))
            .bind(t.expires_at.and_then(ms_to_date))
            .execute(pool)
            .await
            .map_err(|e| PublishError::transient(db_message(&e)))?;
    }
    Ok(t)
}
