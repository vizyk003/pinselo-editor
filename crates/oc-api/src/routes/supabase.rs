//! Port of services/api/src/routes/supabase.ts — the Supabase Auth bridge (plan 12 §B3/B4/B9): the browser signs
//! in with supabase-js, then exchanges the access token once for the normal Pinselo session cookie. Also
//! `sync`, which links a signed-in account and picks up email changes. Every route answers 404 `supabase_off`
//! unless Supabase Auth is on. (Registered from routes/account.ts in TS; merged by `router.rs` here.)

use chrono::Datelike as _;
use serde_json::Map;

use crate::analytics;
use crate::auth::{UserFull, create_user, find_user_by_email, issue_token, user_by_id};
use crate::http::ratelimit::strict;
use crate::login::{DISABLED_MESSAGE, LoginOpts, MIN_AGE, TERMS_VERSION, audit, complete_login, public_user, safe_path};
use crate::prelude::*;
use crate::supabase::{self as sb, Claims, SbUser};

pub const LIMITS: &[RouteLimit] = &[strict("POST", "/api/auth/supabase/session"), strict("POST", "/api/account/supabase/sync")];

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/auth/supabase/session", post(session)).route("/api/account/supabase/sync", post(sync))
}

const CHANGED: &str = "This account changed while signing in. Try again.";
const UNVERIFIABLE: &str = "An account with this email already exists here, but this server can’t confirm the address is yours, so they can’t be linked automatically. Contact support.";

fn unavailable_body() -> Value {
    json!({ "error": "supabase_unavailable", "message": sb::UNAVAILABLE_MSG })
}

fn supabase_off() -> Reply {
    reply::json(404, json!({ "error": "supabase_off", "message": "Supabase sign-in is not configured on this server." }))
}

/// What a bridge still has to do after its last gate (`PendingBridge`); it travels in the built-in TOTP ticket.
#[derive(Debug, Clone)]
pub struct PendingBridge {
    pub supabase_id: String,
    pub link: bool,
    pub email: String,
    pub email_confirmed: bool,
    pub revoke_others: bool,
    pub next: String,
    pub provider: Value,
    pub aal: String,
    pub amr: Vec<String>,
}

impl PendingBridge {
    pub fn to_json(&self) -> Value {
        json!({
            "supabaseId": self.supabase_id, "link": self.link, "email": self.email, "emailConfirmed": self.email_confirmed,
            "revokeOthers": self.revoke_others, "next": self.next, "provider": self.provider, "aal": self.aal, "amr": self.amr,
        })
    }

    /// The ticket's `data.supabase` (None when absent or not an object).
    pub fn from_json(v: &Value) -> Option<PendingBridge> {
        let o = v.as_object()?;
        let st = |k: &str| match o.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(v) => crate::http::jsjson::stringify(v),
        };
        let b = |k: &str| o.get(k).and_then(Value::as_bool).unwrap_or(false);
        Some(PendingBridge {
            supabase_id: st("supabaseId"),
            link: b("link"),
            email: st("email"),
            email_confirmed: b("emailConfirmed"),
            revoke_others: b("revokeOthers"),
            next: st("next"),
            provider: o.get("provider").cloned().unwrap_or(Value::Null),
            aal: st("aal"),
            amr: o.get("amr").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_str().unwrap_or("").to_owned()).collect()).unwrap_or_default(),
        })
    }
}

enum Resolution {
    /// Already linked (or just created).
    User(Box<UserFull>),
    /// A verified local account with the same email: linked once every gate passed.
    Link(Box<UserFull>),
    NeedsProfile(Option<String>),
    /// A squatted address is set aside only after the Supabase step-up.
    StepUp,
    Refuse(u16, Value),
}

fn link_conflict(reason: &str, message: &str) -> Resolution {
    Resolution::Refuse(409, json!({ "error": "link_conflict", "reason": reason, "message": message }))
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505")
}

struct Profile {
    name: String,
    birth_year: i64,
    source: &'static str,
}

/// `metadataProfile(meta)`: `name`, `oc_birth_year` and the current `oc_terms`, as written by our own sign-up form.
fn metadata_profile(meta: &Value, year: i64) -> Option<Profile> {
    let name = match meta.get("name") {
        Some(Value::String(n)) => crate::js::trim(n).to_owned(),
        _ => String::new(),
    };
    if name.is_empty() || crate::js::len(&name) > 80 || meta.get("oc_terms").and_then(Value::as_str) != Some(TERMS_VERSION) {
        return None;
    }
    let by = meta.get("oc_birth_year").filter(|v| v.is_number()).and_then(Value::as_f64)?;
    if by.fract() != 0.0 || by < 1900.0 || by > year as f64 {
        return None;
    }
    Some(Profile { name, birth_year: by as i64, source: "signup_metadata" })
}

/// `suggestedName(meta)`.
fn suggested_name(meta: &Value) -> Option<String> {
    let n = match meta.get("full_name") {
        None | Some(Value::Null) => meta.get("name"),
        v => v,
    };
    match n {
        Some(Value::String(n)) if !crate::js::trim(n).is_empty() => Some(crate::js::take(crate::js::trim(n), 80)),
        _ => None,
    }
}

/// `releaseErasedLink(sub)`.
async fn release_erased_link(s: &AppState, sub: &str) -> Result<(), ApiError> {
    pg::exec(&s.pool, "UPDATE users SET supabase_id = NULL WHERE supabase_id = $1::uuid AND deleted_at IS NOT NULL", &[pg::text(sub)]).await?;
    Ok(())
}

async fn user_by_sub(s: &AppState, sub: &str) -> Result<Option<UserFull>, ApiError> {
    Ok(sqlx::query_as("SELECT * FROM users WHERE supabase_id = $1::uuid").bind(sub).fetch_optional(&s.pool).await?)
}

/// `tombstoneSquatter(...)`: set aside an unverified local account whose address a Supabase user just confirmed.
async fn tombstone_squatter(s: &AppState, req: &Req, local: &UserFull, sub: &str, email: &str, provider: &Value) -> Result<bool, ApiError> {
    let id = local.id;
    let mut t = s.pool.begin().await?;
    let r = sqlx::query(
        "UPDATE users SET email = $3, disabled_at = COALESCE(disabled_at, now()), disabled_reason = 'supabase_squat'
       WHERE id = $1 AND email = $2 AND email_verified_at IS NULL AND supabase_id IS NULL AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(email)
    .bind(format!("squatted+{id}@invalid"))
    .execute(&mut *t)
    .await?;
    if r.rows_affected() == 0 {
        t.commit().await?;
        return Ok(false);
    }
    for sql in [
        "DELETE FROM sessions WHERE user_id = $1",
        "UPDATE api_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
        "UPDATE auth_tokens SET used_at = now() WHERE user_id = $1 AND used_at IS NULL",
        "DELETE FROM passkeys WHERE user_id = $1",
        "DELETE FROM oauth_accounts WHERE user_id = $1",
        "UPDATE webhooks SET active = false WHERE user_id = $1 AND active",
        "DELETE FROM memberships m USING workspaces w WHERE w.id = m.workspace_id AND m.user_id = $1 AND w.owner_id IS DISTINCT FROM $1",
        "DELETE FROM design_shares WHERE user_id = $1",
        "DELETE FROM access_requests WHERE user_id = $1 AND status = 'pending'",
        "UPDATE sites SET published = false, updated_at = now() WHERE published AND design_id IN (SELECT id FROM designs WHERE owner_id = $1)",
        "UPDATE social_accounts SET tokens = NULL, token_expires_at = NULL, status = 'reauth', workspace_id = NULL, updated_at = now() WHERE user_id = $1",
        "DELETE FROM social_oauth_states WHERE user_id = $1",
        "UPDATE social_posts SET status = 'draft', updated_at = now() WHERE owner_id = $1 AND status = 'scheduled'",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *t).await?;
    }
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind("oc_revoke")
        .bind(crate::http::jsjson::stringify(&json!({ "userId": id.to_string(), "at": s.now_ms() })))
        .execute(&mut *t)
        .await?;
    t.commit().await?;
    let mut meta = Map::new();
    meta.insert("userId".into(), json!(id.to_string()));
    meta.insert("supabaseId".into(), json!(sub));
    meta.insert("email".into(), json!(email));
    meta.insert("provider".into(), provider.clone());
    if local.disabled_at.is_some() {
        meta.insert("previousDisabledReason".into(), json!(local.disabled_reason));
    }
    audit(s, Some(id), "auth.supabase_squat_tombstoned", Some(sub), Some(Value::Object(meta)), &req.ip).await?;
    Ok(true)
}

const RESOLVE_RETRIES: u32 = 3;

/// `resolveAccount(...)` (plan 12 §B4): writes only to create a brand-new account (after setting aside an
/// unverified one with the same address); linking by email is returned as a plan.
async fn resolve_account(s: &AppState, req: &Req, claims: &Claims, user: &SbUser, email: &str, profile: Option<(String, i64)>, step_up: bool) -> Result<Resolution, ApiError> {
    let mut retries = RESOLVE_RETRIES;
    loop {
        match resolve_once(s, req, claims, user, email, profile.clone(), step_up).await? {
            Some(r) => return Ok(r),
            None if retries > 0 => retries -= 1,
            None => return Ok(link_conflict("changed_during_signin", CHANGED)),
        }
    }
}

/// One round of [`resolve_account`]; `None` = lost a race, start over.
async fn resolve_once(s: &AppState, req: &Req, claims: &Claims, user: &SbUser, email: &str, profile: Option<(String, i64)>, step_up: bool) -> Result<Option<Resolution>, ApiError> {
    let provider = user.provider();
    if let Some(by_sub) = user_by_sub(s, &claims.sub).await? {
        if by_sub.deleted_at.is_none() {
            return Ok(Some(Resolution::User(Box::new(by_sub))));
        }
        release_erased_link(s, &claims.sub).await?;
    }
    if let Some(local) = find_user_by_email(s, email).await? {
        if local.supabase_id.map(|u| u.to_string()).as_deref() == Some(claims.sub.as_str()) {
            return Ok(Some(Resolution::User(Box::new(local))));
        }
        if local.supabase_id.is_some() {
            return Ok(Some(link_conflict("linked_elsewhere", "This email address belongs to an account that is linked to a different sign-in. Contact support.")));
        }
        if local.disabled_at.is_some() && local.email_verified_at.is_some() {
            return Ok(Some(Resolution::Refuse(403, json!({ "error": "disabled", "message": DISABLED_MESSAGE }))));
        }
        if !user.email_confirmed() {
            audit(s, Some(local.id), "auth.supabase_link_refused", None, Some(json!({ "reason": "unconfirmed_supabase", "supabaseId": claims.sub })), &req.ip).await?;
            return Ok(Some(link_conflict("unconfirmed_supabase", "Confirm your email address first — use the link we emailed you — then sign in again.")));
        }
        let trust = sb::email_confirmation_trust(s).await;
        if trust == "unknown" {
            return Ok(Some(Resolution::Refuse(503, unavailable_body())));
        }
        if trust == "autoconfirm" {
            audit(s, Some(local.id), "auth.supabase_link_refused", None, Some(json!({ "reason": "email_unverifiable", "supabaseId": claims.sub })), &req.ip).await?;
            return Ok(Some(link_conflict("email_unverifiable", UNVERIFIABLE)));
        }
        if local.email_verified_at.is_some() {
            return Ok(Some(Resolution::Link(Box::new(local))));
        }
        if step_up {
            return Ok(Some(Resolution::StepUp));
        }
        // Setting aside an unverified account leads to a new one: refuse first, so a refusal leaves it untouched.
        if !crate::signup::may_create(s, email).await? {
            return Ok(Some(Resolution::Refuse(403, crate::signup::closed_body())));
        }
        if !tombstone_squatter(s, req, &local, &claims.sub, email, &provider).await? {
            return Ok(None);
        }
    }
    if !crate::signup::may_create(s, email).await? {
        return Ok(Some(Resolution::Refuse(403, crate::signup::closed_body())));
    }
    let year = i64::from(s.now().year());
    let p = match profile {
        Some((name, birth_year)) => Some(Profile { name: crate::js::trim(&name).to_owned(), birth_year, source: "form" }),
        None => metadata_profile(user.user_metadata(), year),
    };
    let Some(p) = p else { return Ok(Some(Resolution::NeedsProfile(suggested_name(user.user_metadata())))) };
    if year - p.birth_year < MIN_AGE {
        return Ok(Some(Resolution::Refuse(400, json!({ "error": "age", "message": format!("You need to be at least {MIN_AGE} to create an account.") }))));
    }
    let verified = user.email_confirmed() && sb::email_confirmation_trust(s).await == "trusted";
    let sub = Uuid::parse_str(&claims.sub).map_err(ApiError::internal)?;
    let created = match create_user(s, email, &p.name, auth::NewUser { verified, birth_year: Some(p.birth_year), supabase_id: Some(sub), ..Default::default() }).await {
        Ok(u) => u,
        Err(ApiError::Internal(m)) if m.contains("duplicate key value") => return Ok(None),
        Err(e) => return Err(e),
    };
    let id = created.id;
    audit(s, Some(id), "auth.signup", None, Some(json!({ "method": "supabase", "provider": provider, "terms": TERMS_VERSION, "source": p.source })), &req.ip).await?;
    audit(s, Some(id), "consent.terms", Some(TERMS_VERSION), Some(json!({ "accepted": true, "ip": req.ip })), &req.ip).await?;
    analytics::track(s, Some(id), "signup", json!({ "method": "supabase" })).await?;
    let u = user_by_id(s, id).await?.ok_or_else(|| ApiError::internal("user vanished"))?;
    Ok(Some(Resolution::User(Box::new(u))))
}

/// `commitLink(req, userId, p)`.
async fn commit_link(s: &AppState, req: &Req, user_id: Uuid, p: &PendingBridge) -> Result<bool, ApiError> {
    let r = sqlx::query(
        "UPDATE users SET supabase_id = $2::uuid WHERE id = $1 AND email = $3 AND supabase_id IS NULL AND email_verified_at IS NOT NULL
         AND disabled_at IS NULL AND deleted_at IS NULL RETURNING id",
    )
    .bind(user_id)
    .bind(&p.supabase_id)
    .bind(&p.email)
    .fetch_optional(&s.pool)
    .await;
    match r {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Ok(pg::one(&s.pool, "SELECT 1 FROM users WHERE id = $1 AND supabase_id = $2::uuid", &[pg::uuid(user_id), pg::text(&p.supabase_id)]).await?.is_some());
        }
        Err(e) if is_unique_violation(&e) => return Ok(false),
        Err(e) => return Err(e.into()),
    }
    audit(s, Some(user_id), "auth.supabase_linked", None, Some(json!({ "supabaseId": p.supabase_id, "provider": p.provider })), &req.ip).await?;
    Ok(true)
}

/// `syncSupabaseEmail(req, u, s)`: a changed address is adopted only once Supabase confirmed it and
/// confirmations can be trusted. Returns `emailConflict`.
pub async fn sync_supabase_email(s: &AppState, req: &Req, u: &UserFull, email: Option<&Value>, email_confirmed: bool) -> Result<bool, ApiError> {
    let email = match email {
        Some(Value::String(e)) => e.to_lowercase(),
        _ => String::new(),
    };
    if email.is_empty() {
        return Ok(false);
    }
    let trust = if email_confirmed { sb::email_confirmation_trust(s).await } else { "unknown" };
    let sid = u.supabase_id.map(|x| x.to_string());
    if email != u.email {
        if trust == "autoconfirm" {
            audit(s, Some(u.id), "auth.supabase_email_refused", None, Some(json!({ "reason": "email_unverifiable", "supabaseId": sid })), &req.ip).await?;
        }
        if trust != "trusted" {
            return Ok(false);
        }
        let taken = pg::one(&s.pool, "SELECT 1 FROM users WHERE email = $1 AND id <> $2", &[pg::text(&email), pg::uuid(u.id)]).await?;
        if taken.is_none() {
            match sqlx::query("UPDATE users SET email = $2, email_verified_at = now() WHERE id = $1").bind(u.id).bind(&email).execute(&s.pool).await {
                Ok(_) => {
                    audit(s, Some(u.id), "auth.email_changed", None, Some(json!({ "via": "supabase" })), &req.ip).await?;
                    return Ok(false);
                }
                Err(e) if is_unique_violation(&e) => {}
                Err(e) => return Err(e.into()),
            }
        }
        audit(s, Some(u.id), "auth.supabase_email_conflict", None, Some(json!({ "supabaseId": sid })), &req.ip).await?;
        return Ok(true);
    }
    if trust == "trusted" && u.email_verified_at.is_none() {
        pg::exec(&s.pool, "UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1", &[pg::uuid(u.id)]).await?;
    }
    Ok(false)
}

/// `consumeAccessToken(token, userId, exp)`: single use, keyed on the signed part.
async fn consume_access_token(s: &AppState, token: &str, user_id: Uuid, exp: f64) -> Result<bool, ApiError> {
    let row = sqlx::query("INSERT INTO supabase_token_uses(token_sha256, user_id, expires_at) VALUES ($1, $2, to_timestamp($3)) ON CONFLICT DO NOTHING RETURNING 1")
        .bind(sb::access_token_key(token))
        .bind(user_id)
        .bind(exp)
        .fetch_optional(&s.pool)
        .await?;
    let pool = s.pool.clone();
    s.spawn(async move {
        let _ = sqlx::query("DELETE FROM supabase_token_uses WHERE expires_at < now() - interval '1 hour'").execute(&pool).await;
    });
    Ok(row.is_some())
}

/// `finishSupabaseLogin(req, reply, userId, p, method)`: link, email sync, the recovery sign-out of every other
/// session, then the session. `/api/auth/mfa` calls it with the ticket's PendingBridge.
pub async fn finish_supabase_login(s: &AppState, req: &Req, user_id: Uuid, p: &PendingBridge, method: &str) -> ApiResult {
    let changed = || reply::json(409, json!({ "error": "link_conflict", "reason": "changed_during_signin", "message": CHANGED }));
    if p.link && !commit_link(s, req, user_id, p).await? {
        return Ok(changed());
    }
    let Some(u) = user_by_id(s, user_id).await?.filter(|u| u.supabase_id.map(|x| x.to_string()).as_deref() == Some(p.supabase_id.as_str())) else {
        return Ok(changed());
    };
    sync_supabase_email(s, req, &u, Some(&json!(p.email)), p.email_confirmed).await?;
    if p.revoke_others {
        pg::exec(&s.pool, "DELETE FROM sessions WHERE user_id = $1", &[pg::uuid(u.id)]).await?;
        audit(s, Some(u.id), "auth.sessions_revoked", None, Some(json!({ "via": "supabase_recovery" })), &req.ip).await?;
    }
    let u = user_by_id(s, u.id).await?.ok_or_else(|| ApiError::internal("user vanished"))?;
    let mut extra = Map::new();
    extra.insert("next".into(), json!(p.next));
    let mut audit_meta = Map::new();
    audit_meta.insert("provider".into(), p.provider.clone());
    audit_meta.insert("aal".into(), json!(p.aal));
    audit_meta.insert("amr".into(), json!(p.amr));
    complete_login(s, req, &u, method, LoginOpts { skip_mfa: true, extra, audit_meta }).await
}

// ---------------------------------------------------------------- the bridge

async fn session(State(s): State<AppState>, req: Req) -> ApiResult {
    if !sb::enabled(&s) {
        return Ok(supabase_off());
    }
    if let Err(r) = sb::assert_same_origin(&s, &req) {
        return Ok(r);
    }
    let year = f64::from(s.now().year());
    let b = z::obj([
        ("accessToken", z::str().min(1.0).max(8192.0)),
        ("next", z::str().max(2000.0).optional()),
        ("profile", z::obj([("name", z::str().trim().min(1.0).max(80.0)), ("birthYear", z::num().int().min(1900.0).max(year)), ("acceptTerms", z::literal(json!(true)))]).optional()),
        ("revokeOtherSessions", z::bool().optional()),
    ])
    .parse(req.body())?;
    let next = safe_path(b.get("next"));
    let token = b["accessToken"].as_str().unwrap_or("");
    let (claims, user) = match sb::authenticate(&s, token).await {
        Ok(x) => x,
        Err(e) => return Ok(e.reply()),
    };
    let email = match user.email().or(claims.email.as_ref().filter(|v| !v.is_null())) {
        Some(Value::String(e)) => e.to_lowercase(),
        Some(v) => crate::http::jsjson::stringify(v).to_lowercase(),
        None => String::new(),
    };
    if email.is_empty() {
        return Ok(reply::json(400, json!({ "error": "email_required", "message": "This sign-in has no email address. Sign in with an email address instead." })));
    }
    // Supabase MFA: the step-up produces a new (aal2) token, so this one is not consumed either.
    let step_up = user.has_verified_factor() && claims.aal != "aal2";
    let profile = b.get("profile").and_then(|p| Some((p.get("name")?.as_str()?.to_owned(), p.get("birthYear")?.as_f64()? as i64)));
    let r = resolve_account(&s, &req, &claims, &user, &email, profile, step_up).await?;
    let (u, link) = match r {
        Resolution::Refuse(status, body) => return Ok(reply::json(status, body)),
        Resolution::NeedsProfile(name) => return Ok(Reply::json(json!({ "needsProfile": true, "email": email, "name": name }))),
        Resolution::StepUp => return Ok(Reply::json(json!({ "mfaRequired": true, "supabase": true, "next": next }))),
        Resolution::User(u) => (u, false),
        Resolution::Link(u) => (u, true),
    };
    if u.disabled_at.is_some() {
        return Ok(reply::json(403, json!({ "error": "disabled", "message": DISABLED_MESSAGE })));
    }
    if step_up {
        return Ok(Reply::json(json!({ "mfaRequired": true, "supabase": true, "next": next })));
    }
    // Consumed before the session is written for this sign-in.
    if !consume_access_token(&s, token, u.id, claims.exp).await? {
        return Ok(reply::json(401, json!({ "error": "token_used", "message": "That sign-in was already used. Sign in again." })));
    }
    let pending = PendingBridge {
        supabase_id: claims.sub.clone(),
        link,
        email,
        email_confirmed: user.email_confirmed(),
        revoke_others: b.get("revokeOtherSessions").and_then(Value::as_bool).unwrap_or(false),
        next: next.clone(),
        provider: user.provider(),
        aal: claims.aal.clone(),
        amr: claims.amr.iter().map(|(m, _)| m.clone()).collect(),
    };
    // Built-in TOTP is asked whatever the token's aal.
    if u.totp_secret.as_deref().is_some_and(|t| !t.is_empty()) {
        let ticket = issue_token(&s, "mfa", Some(u.id), None, Some(json!({ "method": "supabase", "supabase": pending.to_json() })), 10).await?;
        return Ok(Reply::json(json!({ "mfaRequired": true, "ticket": ticket, "next": next })));
    }
    finish_supabase_login(&s, &req, u.id, &pending, "supabase").await
}

// ---------------------------------------------------------------- sync (signed in)

async fn sync(State(s): State<AppState>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    if !sb::enabled(&s) {
        return Ok(supabase_off());
    }
    if let Err(r) = sb::assert_same_origin(&s, &req) {
        return Ok(r);
    }
    let b = z::obj([("accessToken", z::str().min(1.0).max(8192.0))]).parse(req.body())?;
    let (claims, user) = match sb::authenticate(&s, b["accessToken"].as_str().unwrap_or("")).await {
        Ok(x) => x,
        Err(e) => return Ok(e.reply()),
    };
    let wrong_account = || reply::json(403, json!({ "error": "wrong_account", "message": "That sign-in belongs to a different account." }));
    let mut u = user_by_id(&s, me.id).await?.ok_or_else(|| ApiError::internal("user vanished"))?;
    match u.supabase_id {
        None => {
            let email = match user.email() {
                Some(Value::String(e)) => e.to_lowercase(),
                _ => String::new(),
            };
            if email != u.email || !user.email_confirmed() {
                return Ok(wrong_account());
            }
            let trust = sb::email_confirmation_trust(&s).await;
            if trust == "unknown" {
                return Ok(reply::json(503, unavailable_body()));
            }
            if trust == "autoconfirm" {
                audit(&s, Some(u.id), "auth.supabase_link_refused", None, Some(json!({ "reason": "email_unverifiable", "supabaseId": claims.sub, "via": "sync" })), &req.ip).await?;
                return Ok(reply::json(409, json!({ "error": "link_conflict", "reason": "email_unverifiable", "message": UNVERIFIABLE })));
            }
            release_erased_link(&s, &claims.sub).await?;
            if pg::one(&s.pool, "SELECT 1 FROM users WHERE supabase_id = $1::uuid", &[pg::text(&claims.sub)]).await?.is_some() {
                return Ok(wrong_account());
            }
            let r = sqlx::query("UPDATE users SET supabase_id = $2::uuid, email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1 AND supabase_id IS NULL RETURNING id")
                .bind(u.id)
                .bind(&claims.sub)
                .fetch_optional(&s.pool)
                .await;
            match r {
                Ok(Some(_)) => {}
                Ok(None) => return Ok(wrong_account()),
                Err(e) if is_unique_violation(&e) => return Ok(wrong_account()),
                Err(e) => return Err(e.into()),
            }
            audit(&s, Some(u.id), "auth.supabase_linked", None, Some(json!({ "supabaseId": claims.sub, "provider": user.provider(), "via": "sync" })), &req.ip).await?;
            u = user_by_id(&s, u.id).await?.ok_or_else(|| ApiError::internal("user vanished"))?;
        }
        Some(sid) if sid.to_string() != claims.sub => return Ok(wrong_account()),
        Some(_) => {}
    }
    let conflict = sync_supabase_email(&s, &req, &u, user.email(), user.email_confirmed()).await?;
    let u = user_by_id(&s, u.id).await?.ok_or_else(|| ApiError::internal("user vanished"))?;
    let mut out = Map::new();
    out.insert("user".into(), public_user(&u));
    if conflict {
        out.insert("emailConflict".into(), json!(true));
    }
    Ok(Reply::json(Value::Object(out)))
}
