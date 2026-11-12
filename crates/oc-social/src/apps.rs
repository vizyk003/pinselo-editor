//! A team workspace's own Meta app (`workspace_social_apps`): the app of the business that owns the workspace's
//! Pages and Instagram accounts, used instead of the server-wide one (`OC_FACEBOOK_CLIENT_ID` / `_SECRET`).
//!
//! The adapters read the app from their [`Http`] environment, so a workspace app is applied by overlaying those
//! variables ([`MetaApp::http`]): connecting (Facebook login or a pasted system-user token), publishing and token
//! refresh all go through the same adapter code. The secret is sealed with `OC_SECRET` like account tokens and
//! never leaves the server.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::adapters::meta;
use crate::crypto::{decrypt, encrypt};
use crate::http::{Http, Req, js_string, with_query};
use crate::store::AccountRow;
use crate::types::PublishError;
use crate::db_message;

/// A workspace's Meta app, with its secret opened.
#[derive(Clone, PartialEq, Eq)]
pub struct MetaApp {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub app_id: String,
    pub app_secret: String,
    pub app_name: Option<String>,
    pub graph_version: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for MetaApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetaApp")
            .field("id", &self.id)
            .field("workspace_id", &self.workspace_id)
            .field("app_id", &self.app_id)
            .field("graph_version", &self.graph_version)
            .finish_non_exhaustive() // never the secret
    }
}

impl MetaApp {
    /// The adapter variables this app replaces.
    pub fn env(&self) -> HashMap<String, String> {
        let mut m = HashMap::from([
            ("OC_FACEBOOK_CLIENT_ID".to_owned(), self.app_id.clone()),
            ("OC_FACEBOOK_CLIENT_SECRET".to_owned(), self.app_secret.clone()),
        ]);
        if let Some(v) = &self.graph_version {
            m.insert("OC_FACEBOOK_GRAPH_VERSION".to_owned(), v.clone());
        }
        m
    }

    /// `base` with this app in place of the server-wide one.
    pub fn http(&self, base: &Http) -> Http {
        base.clone().with_env_overlay(self.env())
    }

    /// The client-safe shape (no secret).
    pub fn public(&self) -> Value {
        json!({
            "provider": "meta",
            "appId": self.app_id,
            "appName": self.app_name,
            "graphVersion": self.graph_version,
            "updatedAt": oc_js::iso_string(&self.updated_at),
        })
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    workspace_id: Uuid,
    app_id: String,
    app_secret: String,
    app_name: Option<String>,
    graph_version: Option<String>,
    updated_at: DateTime<Utc>,
}

fn open(r: Row, secret: &str) -> Result<MetaApp, PublishError> {
    let app_secret = decrypt(&r.app_secret, secret)
        .map_err(|_| PublishError::new("The workspace’s Meta app secret can’t be read on this server; enter it again."))?;
    Ok(MetaApp {
        id: r.id,
        workspace_id: r.workspace_id,
        app_id: r.app_id,
        app_secret,
        app_name: r.app_name,
        graph_version: r.graph_version,
        updated_at: r.updated_at,
    })
}

const COLS: &str = "id, workspace_id, app_id, app_secret, app_name, graph_version, updated_at";

fn db(e: sqlx::Error) -> PublishError {
    PublishError::transient(db_message(&e))
}

/// The workspace's Meta app, if it has one.
pub async fn workspace_meta_app(pool: &PgPool, secret: &str, workspace_id: Uuid) -> Result<Option<MetaApp>, PublishError> {
    let r: Option<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM workspace_social_apps WHERE workspace_id = $1 AND provider = 'meta'"))
        .bind(workspace_id)
        .fetch_optional(pool)
        .await
        .map_err(db)?;
    r.map(|r| open(r, secret)).transpose()
}

/// The [`Http`] to publish or refresh an account with: its workspace app when it was connected through one,
/// otherwise `base` (the server-wide app).
pub async fn http_for_account(pool: &PgPool, base: &Http, secret: &str, row: &AccountRow) -> Result<Http, PublishError> {
    let Some(app_id) = row.social_app_id else { return Ok(base.clone()) };
    let r: Option<Row> = sqlx::query_as(&format!("SELECT {COLS} FROM workspace_social_apps WHERE id = $1"))
        .bind(app_id)
        .fetch_optional(pool)
        .await
        .map_err(db)?;
    match r {
        Some(r) => Ok(open(r, secret)?.http(base)),
        // The FK sets social_app_id to NULL when the app goes; a row read just before that falls back too.
        None => Ok(base.clone()),
    }
}

/// Checks an app ID and secret with Meta (`GET /{app-id}` with the app access token); returns the app's name.
pub async fn verify_meta_app(base: &Http, app_id: &str, app_secret: &str, graph_version: Option<&str>) -> Result<String, PublishError> {
    let mut vars = HashMap::new();
    if let Some(v) = graph_version {
        vars.insert("OC_FACEBOOK_GRAPH_VERSION".to_owned(), v.to_owned());
    }
    let http = base.clone().with_env_overlay(vars);
    let app_token = format!("{app_id}|{app_secret}");
    let url = with_query(
        &format!("{}/{app_id}", meta::graph_base(&http)),
        &[("fields", Some("id,name")), ("access_token", Some(app_token.as_str()))],
    );
    let r = http.call(&url, Req::get()).await.map_err(|e| PublishError {
        message: format!("Meta did not accept that app ID and secret: {}", e.message),
        ..e
    })?;
    if js_string(r.body.get("id")) != app_id {
        return Err(PublishError::new("Meta did not confirm that app ID."));
    }
    Ok(js_string(r.body.get("name")))
}

/// Creates or replaces the workspace's Meta app. `app_secret: None` keeps the stored secret (changing only
/// the Graph version or name); that is refused when the app ID changes.
#[allow(clippy::too_many_arguments)]
pub async fn save_meta_app(
    pool: &PgPool,
    secret: &str,
    workspace_id: Uuid,
    user_id: Uuid,
    app_id: &str,
    app_secret: Option<&str>,
    app_name: Option<&str>,
    graph_version: Option<&str>,
) -> Result<MetaApp, PublishError> {
    let current = workspace_meta_app(pool, secret, workspace_id).await?;
    let plain = match (app_secret, &current) {
        (Some(s), _) => s.to_owned(),
        (None, Some(c)) if c.app_id == app_id => c.app_secret.clone(),
        _ => return Err(PublishError::new("Enter the app secret.")),
    };
    let r: Row = sqlx::query_as(&format!(
        "INSERT INTO workspace_social_apps(id, workspace_id, provider, app_id, app_secret, app_name, graph_version, created_by)
         VALUES ($1, $2, 'meta', $3, $4, $5, $6, $7)
         ON CONFLICT (workspace_id, provider) DO UPDATE SET app_id = EXCLUDED.app_id, app_secret = EXCLUDED.app_secret,
           app_name = EXCLUDED.app_name, graph_version = EXCLUDED.graph_version, updated_at = now()
         RETURNING {COLS}"
    ))
    .bind(oc_db::uuid())
    .bind(workspace_id)
    .bind(app_id)
    .bind(encrypt(&plain, secret))
    .bind(app_name)
    .bind(graph_version)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(db)?;
    // Accounts connected through a different app can't use this one: they need reconnecting.
    if let Some(c) = current.filter(|c| c.app_id != app_id) {
        sqlx::query("UPDATE social_accounts SET status = 'reauth', updated_at = now() WHERE social_app_id = $1")
            .bind(c.id)
            .execute(pool)
            .await
            .map_err(db)?;
    }
    open(r, secret)
}

/// Removes the workspace's Meta app. Its accounts are set to `reauth` (they fall back to the server-wide app,
/// which did not issue their tokens). Returns whether there was one.
pub async fn delete_meta_app(pool: &PgPool, workspace_id: Uuid) -> Result<bool, PublishError> {
    let mut tx = pool.begin().await.map_err(db)?;
    // Before the delete: the FK then sets social_app_id to NULL and the accounts could no longer be found.
    sqlx::query(
        "UPDATE social_accounts SET status = 'reauth', updated_at = now()
         WHERE social_app_id = (SELECT id FROM workspace_social_apps WHERE workspace_id = $1 AND provider = 'meta')",
    )
    .bind(workspace_id)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    let id: Option<Uuid> = sqlx::query_scalar("DELETE FROM workspace_social_apps WHERE workspace_id = $1 AND provider = 'meta' RETURNING id")
        .bind(workspace_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(id.is_some())
}

/// After `save_accounts`: share the accounts with `workspace_id` (when given) and record how they were connected.
pub async fn link_accounts(
    pool: &PgPool,
    ids: &[Uuid],
    workspace_id: Option<Uuid>,
    app: Option<&MetaApp>,
    connected_via: &str,
) -> Result<(), PublishError> {
    sqlx::query(
        "UPDATE social_accounts SET workspace_id = COALESCE($2, workspace_id), social_app_id = $3, connected_via = $4, updated_at = now()
         WHERE id = ANY($1)",
    )
    .bind(ids)
    .bind(workspace_id)
    .bind(app.map(|a| a.id))
    .bind(connected_via)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(())
}
