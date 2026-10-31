//! The job handlers of services/worker/src/main.ts: `email`, `webhook`, `account-export`, `account-delete`,
//! `publish`, plus the `maintenance` and `social-sweep` schedules.

use std::io::Write as _;

use chrono::{Datelike, Local, Timelike};
use oc_js::JsValue;
use oc_social::publish::{JobInfo, Render};
use oc_social::{Social, db_message};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::pgjson::{self, stringify_pretty};

/// A failed job attempt; the message becomes the job's `error` (TS: `error.message`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct JobError(pub String);

impl From<sqlx::Error> for JobError {
    fn from(e: sqlx::Error) -> Self {
        JobError(db_message(&e))
    }
}

impl From<oc_social::PublishError> for JobError {
    fn from(e: oc_social::PublishError) -> Self {
        JobError(e.message)
    }
}

impl From<oc_mail::MailError> for JobError {
    fn from(e: oc_mail::MailError) -> Self {
        match e {
            oc_mail::MailError::Db(e) => e.into(),
            e => JobError(e.to_string()),
        }
    }
}

/// A uuid given as a string in a payload: Postgres would refuse anything else with this message.
pub fn parse_uuid(s: &str) -> Result<Uuid, JobError> {
    Uuid::parse_str(s).map_err(|_| JobError(format!("invalid input syntax for type uuid: \"{s}\"")))
}

/// A payload string field (`undefined` / `null` → `None`).
pub fn field<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(Value::as_str)
}

/// `ctx.progress(fraction, label)`.
pub async fn progress(pool: &sqlx::PgPool, job: Uuid, fraction: f64, label: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE jobs SET progress = $2, label = COALESCE($3, label) WHERE id = $1")
        .bind(job)
        .bind(fraction.clamp(0.0, 1.0) as f32)
        .bind(label)
        .execute(pool)
        .await
        .map(|_| ())
}

// ---------------------------------------------------------------- email / webhook

/// `email`: deliver one outbox message (`deliver(p.id)`). A missing id finds no row, like `id = NULL`.
pub async fn email(s: &Social, payload: &Value) -> Result<Value, JobError> {
    if let Some(id) = field(payload, "id") {
        oc_mail::deliver(&s.pool, parse_uuid(id)?).await?;
    }
    Ok(Value::Null)
}

/// `webhook`: one delivery attempt (`deliverWebhook(p.deliveryId)`).
pub async fn webhook(s: &Social, payload: &Value) -> Result<Value, JobError> {
    if let Some(id) = field(payload, "deliveryId") {
        oc_webhooks::deliver_webhook(&s.pool, parse_uuid(id)?).await.map_err(|e| match e {
            oc_webhooks::WebhookError::Db(e) => JobError::from(e),
            e => JobError(e.to_string()),
        })?;
    }
    Ok(Value::Null)
}

// ---------------------------------------------------------------- publish

/// `publish`: `runPublish(p, ctx)`.
pub async fn publish(s: &Social, payload: &Value, job: JobInfo, render: &dyn Render) -> Result<Value, JobError> {
    Ok(oc_social::publish::run_publish(s, payload, job, render).await?)
}

/// `social-sweep` (every minute).
pub async fn social_sweep(s: &Social) -> Result<usize, JobError> {
    let n = oc_social::publish::sweep(&s.pool, s.now_ms()).await?;
    if n > 0 {
        println!("[worker] queued {n} overdue scheduled post(s)");
    }
    Ok(n)
}

// ---------------------------------------------------------------- account export (GDPR Art. 15/20)

/// One zip entry as JSZip stores it (folders are created for nested paths).
struct Zip {
    w: zip::ZipWriter<std::io::Cursor<Vec<u8>>>,
    dirs: Vec<String>,
    opts: zip::write::SimpleFileOptions,
}

impl Zip {
    fn new(now_ms: i64) -> Zip {
        // JSZip stamps entries with the local time of `new Date()`.
        let t = chrono::DateTime::from_timestamp_millis(now_ms).unwrap_or_default().with_timezone(&Local);
        let dos = zip::DateTime::from_date_and_time(
            t.year().clamp(1980, 2107) as u16,
            t.month() as u8,
            t.day() as u8,
            t.hour() as u8,
            t.minute() as u8,
            t.second() as u8,
        )
        .unwrap_or_default();
        Zip {
            w: zip::ZipWriter::new(std::io::Cursor::new(Vec::new())),
            dirs: Vec::new(),
            opts: zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .last_modified_time(dos),
        }
    }

    fn file(&mut self, name: &str, data: &[u8]) -> Result<(), JobError> {
        let err = |e: zip::result::ZipError| JobError(e.to_string());
        // JSZip `createFolders`: every parent folder gets its own entry before the file.
        let mut at = 0;
        while let Some(i) = name[at..].find('/') {
            let dir = &name[..at + i + 1];
            if !self.dirs.iter().any(|d| d == dir) {
                self.dirs.push(dir.to_string());
                self.w.add_directory(dir, self.opts).map_err(err)?;
            }
            at += i + 1;
        }
        self.w.start_file(name, self.opts).map_err(err)?;
        self.w.write_all(data).map_err(|e| JobError(e.to_string()))
    }

    fn finish(self) -> Result<Vec<u8>, JobError> {
        Ok(self.w.finish().map_err(|e| JobError(e.to_string()))?.into_inner())
    }
}

/// `a.name.replace(/[^\w.\- ]+/g, '_').slice(0, 100) || a.id`.
pub fn upload_name(name: &str, id: &str) -> String {
    let mut out = String::new();
    let mut in_run = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ' ') {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    let cut: String = out.chars().take(100).collect();
    if cut.is_empty() { id.to_string() } else { cut }
}

async fn json_file(s: &Social, sql: &str, user: Uuid, single: bool) -> Result<String, JobError> {
    let rows = sqlx::query(sql).bind(user).fetch_all(&s.pool).await?;
    let v = if single {
        rows.first().map(pgjson::row).transpose()?.unwrap_or(JsValue::Null)
    } else {
        pgjson::rows(&rows)?
    };
    Ok(stringify_pretty(&v))
}

/// `account-export`: profile, email preferences, sessions, teams, comments, designs (JSON) and uploads →
/// a zip in the object store; the owner is emailed a download link.
pub async fn account_export(s: &Social, payload: &Value, job: JobInfo) -> Result<Value, JobError> {
    let user_id = parse_uuid(field(payload, "userId").unwrap_or("undefined"))?;
    let user = sqlx::query("SELECT id, email, name, locale, birth_year, email_verified_at, created_at FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&s.pool)
        .await?;
    let Some(user) = user else { return Err(JobError("user not found".into())) };
    let email: String = sqlx::Row::try_get(&user, "email")?;
    let mut zip = Zip::new(s.now_ms());
    zip.file("profile.json", stringify_pretty(&pgjson::row(&user)?).as_bytes())?;
    let prefs = json_file(s, "SELECT mentions, comments, shares, product FROM email_preferences WHERE user_id = $1", user_id, true).await?;
    zip.file("email-preferences.json", prefs.as_bytes())?;
    let sessions = json_file(s, "SELECT user_agent, ip, created_at, last_seen_at FROM sessions WHERE user_id = $1", user_id, false).await?;
    zip.file("sessions.json", sessions.as_bytes())?;
    let teams = json_file(
        s,
        "SELECT w.name, m.role, m.created_at FROM memberships m JOIN workspaces w ON w.id = m.workspace_id WHERE m.user_id = $1",
        user_id,
        false,
    )
    .await?;
    zip.file("teams.json", teams.as_bytes())?;
    let comments = json_file(s, "SELECT design_id, body, created_at FROM comments WHERE author_id = $1 ORDER BY created_at", user_id, false).await?;
    zip.file("comments.json", comments.as_bytes())?;

    let designs: Vec<(String, String, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT d.id, d.title, s.state FROM designs d LEFT JOIN doc_state s ON s.design_id = d.id WHERE d.owner_id = $1")
            .bind(user_id)
            .fetch_all(&s.pool)
            .await?;
    for (i, (id, title, state)) in designs.iter().enumerate() {
        if let Some(state) = state {
            let doc = oc_crdt::decode_doc_value(state).map_err(|e| JobError(e.to_string()))?;
            let body = JsValue::Object(vec![("title".into(), JsValue::String(title.clone())), ("doc".into(), JsValue::from(&doc))]);
            zip.file(&format!("designs/{id}.json"), body.stringify().as_bytes())?;
        }
        progress(&s.pool, job.id, (i + 1) as f64 / (designs.len() + 1) as f64 * 0.6, Some("Collecting designs")).await?;
    }
    let assets: Vec<(String, String, String, String)> =
        sqlx::query_as("SELECT id, name, sha256, mime FROM assets WHERE owner_id = $1 AND trashed_at IS NULL")
            .bind(user_id)
            .fetch_all(&s.pool)
            .await?;
    for (i, (id, name, sha, _mime)) in assets.iter().enumerate() {
        let bytes = s.objects.get(sha).await.unwrap_or_default();
        zip.file(&format!("uploads/{id}-{}", upload_name(name, id)), &bytes)?;
        progress(&s.pool, job.id, 0.6 + (i + 1) as f64 / (assets.len() + 1) as f64 * 0.35, Some("Collecting uploads")).await?;
    }
    zip.file(
        "README.txt",
        b"Your Pinselo data export.\n\ndesigns/*.json are full design documents (they can be imported again).\nuploads/ holds your original files.\n",
    )?;
    let bytes = zip.finish()?;
    let sha = s.objects.put(&bytes, Some("application/zip")).await.map_err(|e| JobError(e.to_string()))?;
    let url = format!("{}/api/account/exports/{}", s.cfg.web_origin, job.id);
    oc_mail::send_email(&s.pool, &email, &oc_mail::Template::ExportReady { url }, None).await?;
    oc_audit::audit(
        &s.pool,
        oc_audit::AuditEvent {
            actor_id: Some(user_id),
            action: "privacy.export_ready".into(),
            meta: Some(json!({ "bytes": bytes.len() })),
            ..Default::default()
        },
    )
    .await?;
    Ok(json!({ "sha": sha, "size": bytes.len(), "filename": "pinselo-export.zip", "type": "application/zip" }))
}

// ---------------------------------------------------------------- account delete (GDPR Art. 17)

/// Tables whose rows are keyed by the erased user.
pub const USER_TABLES: [&str; 8] =
    ["sessions", "auth_tokens", "passkeys", "oauth_accounts", "email_preferences", "design_shares", "stars", "notifications"];

/// `account-delete`: designs, uploads, comments and credentials are erased; team workspaces where the user
/// is the only owner pass to the longest-standing admin, else are deleted; the user row is anonymised.
pub async fn account_delete(s: &Social, payload: &Value) -> Result<Value, JobError> {
    let user_id = parse_uuid(field(payload, "userId").unwrap_or("undefined"))?;
    let email = field(payload, "email").unwrap_or("undefined").to_string();
    let mut t = s.pool.begin().await?;
    let owned: Vec<(Uuid, bool)> = sqlx::query_as(
        "SELECT w.id, w.personal FROM workspaces w JOIN memberships m ON m.workspace_id = w.id AND m.user_id = $1 AND m.role = 'owner'",
    )
    .bind(user_id)
    .fetch_all(&mut *t)
    .await?;
    for (w, personal) in owned {
        if !personal {
            let heir: Option<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM memberships WHERE workspace_id = $1 AND user_id <> $2 ORDER BY CASE role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 ELSE 2 END, created_at LIMIT 1",
            )
            .bind(w)
            .bind(user_id)
            .fetch_optional(&mut *t)
            .await?;
            if let Some(heir) = heir {
                sqlx::query("UPDATE memberships SET role = 'owner' WHERE workspace_id = $1 AND user_id = $2").bind(w).bind(heir).execute(&mut *t).await?;
                sqlx::query("UPDATE workspaces SET owner_id = $2 WHERE id = $1").bind(w).bind(heir).execute(&mut *t).await?;
                // Their designs in a surviving team stay with the team.
                sqlx::query("UPDATE designs SET owner_id = $3 WHERE workspace_id = $1 AND owner_id = $2").bind(w).bind(user_id).bind(heir).execute(&mut *t).await?;
                sqlx::query("UPDATE assets SET owner_id = $3 WHERE workspace_id = $1 AND owner_id = $2").bind(w).bind(user_id).bind(heir).execute(&mut *t).await?;
                continue;
            }
        }
        sqlx::query("DELETE FROM workspaces WHERE id = $1").bind(w).execute(&mut *t).await?;
    }
    for sql in [
        "DELETE FROM designs WHERE owner_id = $1",
        "DELETE FROM assets WHERE owner_id = $1",
        "DELETE FROM folders WHERE owner_id = $1",
        "DELETE FROM comments WHERE author_id = $1",
        "UPDATE design_versions SET created_by = NULL WHERE created_by = $1",
        "UPDATE workspaces SET owner_id = NULL WHERE owner_id = $1",
        "DELETE FROM memberships WHERE user_id = $1",
    ] {
        sqlx::query(sql).bind(user_id).execute(&mut *t).await?;
    }
    for table in USER_TABLES {
        sqlx::query(&format!("DELETE FROM {table} WHERE user_id = $1")).bind(user_id).execute(&mut *t).await?;
    }
    // Keep a tombstone row so audit/report foreign keys stay valid; no personal data remains.
    sqlx::query(
        "UPDATE users SET email = $2, name = 'Deleted user', password_hash = NULL, totp_secret = NULL, totp_pending = NULL,
         recovery_codes = NULL, birth_year = NULL, deleted_at = now(), disabled_at = COALESCE(disabled_at, now()) WHERE id = $1",
    )
    .bind(user_id)
    .bind(format!("deleted+{user_id}@invalid"))
    .execute(&mut *t)
    .await?;
    t.commit().await?;
    oc_mail::send_email(&s.pool, &email, &oc_mail::Template::AccountDeleted, None).await?;
    oc_audit::audit(
        &s.pool,
        oc_audit::AuditEvent { actor_id: None, action: "privacy.erasure_completed".into(), target: Some(user_id.to_string()), ..Default::default() },
    )
    .await?;
    Ok(Value::Null)
}

// ---------------------------------------------------------------- maintenance

/// What a maintenance run purged from the trash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Purged {
    pub designs: usize,
    pub assets: usize,
}

/// Hourly maintenance: purge trash after 30 days, expire tokens/sessions, prune finished jobs, old mail,
/// notifications and analytics, remove stale OAuth clients; lapse/renew local-provider subscriptions.
pub async fn maintenance(pool: &sqlx::PgPool) -> Result<Purged, JobError> {
    // Held items stay: designs taken down by staff and uploads that are reported ('pending' / 'quarantined') or
    // removed by a moderator are evidence until a moderator decides.
    let designs: Vec<String> =
        sqlx::query_scalar("DELETE FROM designs WHERE trashed_at < now() - interval '30 days' AND taken_down_at IS NULL RETURNING id").fetch_all(pool).await?;
    let assets: Vec<String> =
        sqlx::query_scalar("DELETE FROM assets WHERE trashed_at < now() - interval '30 days' AND moderation = 'ok' RETURNING id").fetch_all(pool).await?;
    sqlx::query("DELETE FROM auth_tokens WHERE expires_at < now() - interval '1 day'").execute(pool).await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at < now()").execute(pool).await?;
    sqlx::query("DELETE FROM jobs WHERE status IN ('done','failed','cancelled') AND finished_at < now() - interval '7 days'").execute(pool).await?;
    sqlx::query("DELETE FROM email_outbox WHERE created_at < now() - interval '90 days'").execute(pool).await?;
    let _ = sqlx::query("DELETE FROM notifications WHERE created_at < now() - interval '180 days'").execute(pool).await;
    sqlx::query("DELETE FROM analytics_events WHERE created_at < now() - interval '400 days'").execute(pool).await?;
    let clients = purge_stale_oauth_clients(pool).await?;
    // Local billing provider: trials convert, cancelled plans lapse at period end (Stripe does this by webhook).
    sqlx::query(
        "UPDATE subscriptions SET plan = 'free', status = 'canceled', updated_at = now()
               WHERE provider = 'local' AND cancel_at_period_end AND current_period_end < now() AND status <> 'canceled'",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE subscriptions SET status = 'active', current_period_end = current_period_end + CASE interval WHEN 'year' THEN interval '365 days' ELSE interval '30 days' END, updated_at = now()
               WHERE provider = 'local' AND NOT cancel_at_period_end AND current_period_end < now() AND status IN ('trialing','active')",
    )
    .execute(pool)
    .await?;
    if !designs.is_empty() || !assets.is_empty() {
        println!("[worker] purged {} designs, {} uploads from trash", designs.len(), assets.len());
    }
    if clients > 0 {
        println!("[worker] removed {clients} stale OAuth client(s)");
    }
    Ok(Purged { designs: designs.len(), assets: assets.len() })
}

/// services/worker/src/oauthClients.ts `purgeStaleOAuthClients`: AI connector OAuth clients (dynamically registered,
/// or cached metadata documents) with no live grant or pending code that nobody has used, registered or refreshed
/// for 30 days. Apps register again when they reconnect; their revoked tokens and spent codes go with them (CASCADE).
pub async fn purge_stale_oauth_clients(pool: &sqlx::PgPool) -> Result<usize, JobError> {
    let ids: Vec<String> = sqlx::query_scalar(
        "DELETE FROM oauth_clients c
     WHERE GREATEST(c.created_at, c.last_used_at, c.fetched_at) < now() - interval '30 days'
       AND NOT EXISTS (SELECT 1 FROM api_tokens t WHERE t.client_id = c.id AND t.revoked_at IS NULL AND t.refresh_expires_at > now())
       AND NOT EXISTS (SELECT 1 FROM oauth_codes o WHERE o.client_id = c.id AND o.used_at IS NULL AND o.expires_at > now())
     RETURNING c.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_names() {
        assert_eq!(upload_name("photo (1).png", "x"), "photo _1_.png");
        assert_eq!(upload_name("Été.jpg", "x"), "_t_.jpg");
        assert_eq!(upload_name("", "a1"), "a1");
        assert_eq!(upload_name("日本", "a1"), "_");
        assert_eq!(upload_name(&"a".repeat(150), "x").len(), 100);
    }
}
