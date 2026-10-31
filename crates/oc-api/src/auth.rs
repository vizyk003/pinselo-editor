//! Port of services/api/src/auth.ts — identity core (PL-001/004): sessions, password hashing, single-use
//! tokens, authorization helpers. Sessions are opaque random tokens in an HttpOnly cookie (`__Host-` in
//! production); only their sha256 is stored.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use oc_auth::Role;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::http::cookie::{self, CookieOpts};
use crate::http::error::ApiError;
use crate::http::reply;
use crate::http::req::Req;
use crate::js;
use crate::pg::{self, Arg};
use crate::state::AppState;

pub const SESSION_DAYS: i64 = 30;

pub fn session_cookie(state: &AppState) -> &'static str {
    if state.prod() { "__Host-oc_session" } else { "oc_session" }
}

const COLORS: [&str; 10] = ["#7c3aed", "#db2777", "#ea580c", "#16a34a", "#0891b2", "#2563eb", "#9333ea", "#dc2626", "#0d9488", "#ca8a04"];

/// `req.user` after `requireUser`.
#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub color: String,
    pub is_staff: bool,
}

/// A `users` row (`SELECT *`).
#[derive(Debug, Clone, FromRow)]
pub struct UserFull {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub color: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub totp_secret: Option<String>,
    pub totp_pending: Option<String>,
    pub is_staff: bool,
    pub disabled_at: Option<DateTime<Utc>>,
    pub locale: String,
    pub password_hash: Option<String>,
    pub recovery_codes: Option<Vec<String>>,
    pub birth_year: Option<i32>,
    pub supabase_id: Option<Uuid>,
    pub disabled_reason: Option<String>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// `sha(s)`: hex sha256.
pub fn sha(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// Cryptographically random bytes.
pub fn random_bytes(n: usize) -> Vec<u8> {
    use ring::rand::SecureRandom as _;
    let mut b = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut b).expect("system randomness");
    b
}

/// `randomToken(bytes = 32)`: base64url.
pub fn random_token(bytes: usize) -> String {
    URL_SAFE_NO_PAD.encode(random_bytes(bytes))
}

// ---------------------------------------------------------------- passwords (scrypt, N=2^15)

fn nfkc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    s.nfkc().collect()
}

/// Node's `crypto.scrypt(pw.normalize('NFKC'), salt, len, { N: 2^log_n, r, p, maxmem: 64 MiB })`.
fn scrypt_key(pw: &str, salt: &[u8], log_n: u8, r: u32, p: u32, len: usize) -> Option<Vec<u8>> {
    if log_n == 0 || log_n >= 64 || len == 0 {
        return None;
    }
    let pw = nfkc(pw);
    let mut out = vec![0u8; len];
    // SAFETY: every pointer/length pair describes a live buffer owned by this function.
    let ok = unsafe {
        aws_lc_sys::EVP_PBE_scrypt(
            pw.as_ptr().cast(),
            pw.len(),
            salt.as_ptr(),
            salt.len(),
            1u64 << log_n,
            u64::from(r),
            u64::from(p),
            64 * 1024 * 1024,
            out.as_mut_ptr(),
            out.len(),
        )
    };
    (ok == 1).then_some(out)
}

/// `hashPassword(pw)`: `scrypt$15$8$1$<salt b64>$<key b64>`.
pub async fn hash_password(pw: &str) -> String {
    let pw = pw.to_owned();
    tokio::task::spawn_blocking(move || {
        let salt = random_bytes(16);
        let key = scrypt_key(&pw, &salt, 15, 8, 1, 64).expect("scrypt");
        format!("scrypt$15$8$1${}${}", STANDARD.encode(&salt), STANDARD.encode(&key))
    })
    .await
    .expect("scrypt task")
}

/// `verifyPassword(pw, stored)`.
pub async fn verify_password(pw: &str, stored: Option<&str>) -> bool {
    let Some(stored) = stored.filter(|s| s.starts_with("scrypt$")) else { return false };
    let parts: Vec<String> = stored.split('$').map(str::to_owned).collect();
    let pw = pw.to_owned();
    tokio::task::spawn_blocking(move || {
        let get = |i: usize| parts.get(i).map(String::as_str).unwrap_or("");
        let expect = lenient_b64(get(5));
        let (Ok(n), Ok(r), Ok(p)) = (get(1).parse::<u8>(), get(2).parse::<u32>(), get(3).parse::<u32>()) else { return false };
        match scrypt_key(&pw, &lenient_b64(get(4)), n, r, p, expect.len()) {
            Some(key) => key.len() == expect.len() && bool::from(key.ct_eq(&expect)),
            None => false,
        }
    })
    .await
    .unwrap_or(false)
}

/// `Buffer.from(s, 'base64')` (padding optional, junk skipped).
fn lenient_b64(s: &str) -> Vec<u8> {
    let clean: String = s.chars().take_while(|c| *c != '=').filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '-' || *c == '_').map(|c| match c {
        '-' => '+',
        '_' => '/',
        c => c,
    }).collect();
    let usable = if clean.len() % 4 == 1 { &clean[..clean.len() - 1] } else { &clean[..] };
    base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true).with_decode_padding_mode(base64::engine::DecodePaddingMode::RequireNone),
    )
    .decode(usable)
    .unwrap_or_default()
}

/// `passwordProblem(pw)`: short or among the most common.
pub fn password_problem(pw: &str) -> Option<&'static str> {
    let len = js::len(pw);
    if len < 10 {
        return Some("Use at least 10 characters.");
    }
    if len > 200 {
        return Some("That password is too long.");
    }
    const WEAK: [&str; 9] = ["password", "password1", "1234567890", "qwertyuiop", "1q2w3e4r5t", "iloveyou12", "letmein123", "opencanvas", "pinselo123"];
    if WEAK.contains(&pw.to_lowercase().as_str()) {
        return Some("That password is too common.");
    }
    // /^(.)\1+$/: one UTF-16 unit (not a line terminator) repeated.
    let u: Vec<u16> = pw.encode_utf16().collect();
    if u.len() >= 2 && ![0x0a, 0x0d, 0x2028, 0x2029].contains(&u[0]) && u.iter().all(|&c| c == u[0]) {
        return Some("That password is too simple.");
    }
    None
}

// ---------------------------------------------------------------- users

#[derive(Default)]
pub struct NewUser<'a> {
    pub password: Option<&'a str>,
    pub verified: bool,
    pub birth_year: Option<i64>,
    /// `users.supabase_id` (plan 12); only written when given, so built-in sign-up never depends on that column.
    pub supabase_id: Option<Uuid>,
}

/// `createUser(email, name, opts)`.
pub async fn create_user(state: &AppState, email: &str, name: &str, opts: NewUser<'_>) -> Result<User, ApiError> {
    let id = oc_db::uuid();
    let h = sha(email);
    let color = COLORS[(u64::from_str_radix(&h[..6], 16).unwrap_or(0) % COLORS.len() as u64) as usize];
    let trimmed = js::take(js::trim(name), 80);
    let name = if trimmed.is_empty() { email.split('@').next().unwrap_or("").to_owned() } else { trimmed };
    let email_l = email.to_lowercase();
    let password_hash = match opts.password {
        Some(p) => Some(hash_password(p).await),
        None => None,
    };
    let verified = opts.verified.then(|| state.now());
    let sql = if opts.supabase_id.is_some() {
        "INSERT INTO users(id, email, name, color, password_hash, email_verified_at, birth_year, supabase_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)"
    } else {
        "INSERT INTO users(id, email, name, color, password_hash, email_verified_at, birth_year) VALUES ($1,$2,$3,$4,$5,$6,$7)"
    };
    let mut q = sqlx::query(sql).bind(id).bind(&email_l).bind(&name).bind(color).bind(password_hash).bind(verified).bind(opts.birth_year.map(|y| y as i32));
    if let Some(sb) = opts.supabase_id {
        q = q.bind(sb);
    }
    q.execute(&state.pool).await?;
    let ws = oc_db::uuid();
    sqlx::query("INSERT INTO workspaces(id, name, personal, owner_id) VALUES ($1,$2,true,$3)").bind(ws).bind(format!("{name}'s workspace")).bind(id).execute(&state.pool).await?;
    sqlx::query("INSERT INTO memberships(workspace_id, user_id, role) VALUES ($1,$2,'owner')").bind(ws).bind(id).execute(&state.pool).await?;
    sqlx::query("INSERT INTO email_preferences(user_id) VALUES ($1) ON CONFLICT DO NOTHING").bind(id).execute(&state.pool).await?;
    Ok(User { id, email: email_l, name, color: color.to_owned(), is_staff: false })
}

/// `findUserByEmail(email)`.
pub async fn find_user_by_email(state: &AppState, email: &str) -> Result<Option<UserFull>, ApiError> {
    Ok(sqlx::query_as("SELECT * FROM users WHERE email = $1 AND deleted_at IS NULL").bind(email.to_lowercase()).fetch_optional(&state.pool).await?)
}

/// `SELECT * FROM users WHERE id = $1`.
pub async fn user_by_id(state: &AppState, id: Uuid) -> Result<Option<UserFull>, ApiError> {
    Ok(sqlx::query_as("SELECT * FROM users WHERE id = $1").bind(id).fetch_optional(&state.pool).await?)
}

/// `findOrCreateUser(email, name)` (dev login).
pub async fn find_or_create_user(state: &AppState, email: &str, name: &str) -> Result<User, ApiError> {
    if let Some(u) = find_user_by_email(state, email).await? {
        return Ok(User { id: u.id, email: u.email, name: u.name, color: u.color, is_staff: false });
    }
    create_user(state, email, name, NewUser { verified: true, ..Default::default() }).await
}

// ---------------------------------------------------------------- sessions

/// `createSession(req, reply, userId, opts)`: returns the `Set-Cookie` value and whether the device is new.
pub async fn create_session(state: &AppState, req: &Req, user_id: Uuid, mfa_verified: bool) -> Result<(String, bool), ApiError> {
    let token = random_token(32);
    let ua = js::take(&req.user_agent(), 300);
    let known = pg::one(&state.pool, "SELECT 1 FROM sessions WHERE user_id = $1 AND user_agent = $2 LIMIT 1", &[Arg::Uuid(Some(user_id)), pg::text(&ua)]).await?;
    sqlx::query("INSERT INTO sessions(id, user_id, expires_at, user_agent, ip, mfa_verified) VALUES ($1, $2, now() + interval '30 days', $3, $4, $5)")
        .bind(sha(&token))
        .bind(user_id)
        .bind(&ua)
        .bind(&req.ip)
        .bind(mfa_verified)
        .execute(&state.pool)
        .await?;
    let c = cookie::serialize(
        session_cookie(state),
        &token,
        &CookieOpts { http_only: true, secure: state.prod(), path: Some("/".into()), max_age: Some(SESSION_DAYS * 86400), ..Default::default() },
    );
    Ok((c, known.is_none()))
}

/// `destroySession(req, reply)`: returns the clearing `Set-Cookie`.
pub async fn destroy_session(state: &AppState, req: &Req) -> Result<String, ApiError> {
    if let Some(token) = req.cookie(session_cookie(state)).filter(|t| !t.is_empty()) {
        pg::exec(&state.pool, "DELETE FROM sessions WHERE id = $1", &[pg::text(sha(token))]).await?;
    }
    Ok(cookie::clear(session_cookie(state), "/"))
}

/// `sessionId(req)`.
pub fn session_id(state: &AppState, req: &Req) -> Option<String> {
    req.cookie(session_cookie(state)).filter(|t| !t.is_empty()).map(sha)
}

/// `currentUser(req)`.
pub async fn current_user(state: &AppState, req: &Req) -> Result<Option<UserFull>, ApiError> {
    let Some(sid) = session_id(state, req) else { return Ok(None) };
    let u: Option<UserFull> = sqlx::query_as(
        "SELECT u.* FROM sessions s JOIN users u ON u.id = s.user_id
     WHERE s.id = $1 AND s.expires_at > now() AND s.mfa_verified AND u.disabled_at IS NULL AND u.deleted_at IS NULL",
    )
    .bind(&sid)
    .fetch_optional(&state.pool)
    .await?;
    if u.is_some() {
        let pool = state.pool.clone();
        state.spawn(async move {
            let _ = sqlx::query("UPDATE sessions SET last_seen_at = now() WHERE id = $1 AND last_seen_at < now() - interval '5 minutes'").bind(sid).execute(&pool).await;
        });
    }
    Ok(u)
}

/// `requireUser` (preHandler): 401 `{error: 'unauthenticated'}` without a valid session.
pub async fn require_user(state: &AppState, req: &Req) -> Result<User, ApiError> {
    match current_user(state, req).await? {
        Some(u) => Ok(User { id: u.id, email: u.email, name: u.name, color: u.color, is_staff: u.is_staff }),
        None => Err(reply::json(401, json!({ "error": "unauthenticated" })).into()),
    }
}

/// `requireStaff`.
pub async fn require_staff(state: &AppState, req: &Req) -> Result<User, ApiError> {
    let u = require_user(state, req).await?;
    if !u.is_staff {
        return Err(reply::json(403, json!({ "error": "forbidden" })).into());
    }
    Ok(u)
}

// ---------------------------------------------------------------- single-use tokens

/// `issueToken(kind, opts)`.
pub async fn issue_token(state: &AppState, kind: &str, user_id: Option<Uuid>, email: Option<&str>, data: Option<Value>, ttl_minutes: i64) -> Result<String, ApiError> {
    let token = random_token(32);
    sqlx::query("INSERT INTO auth_tokens(id, kind, user_id, email, data, expires_at) VALUES ($1,$2,$3,$4,$5, now() + ($6 || ' minutes')::interval)")
        .bind(sha(&token))
        .bind(kind)
        .bind(user_id)
        .bind(email.map(str::to_lowercase))
        .bind(data.map(sqlx::types::Json))
        .bind(ttl_minutes.to_string())
        .execute(&state.pool)
        .await?;
    Ok(token)
}

#[derive(Debug, FromRow)]
pub struct ConsumedToken {
    pub user_id: Option<Uuid>,
    pub email: Option<String>,
    pub data: Option<sqlx::types::Json<Value>>,
}

/// `consumeToken(kind, token)`: atomically mark used; the row, or `None` when unknown/expired/used.
pub async fn consume_token(state: &AppState, kind: &str, token: &str) -> Result<Option<ConsumedToken>, ApiError> {
    Ok(sqlx::query_as(
        "UPDATE auth_tokens SET used_at = now() WHERE id = $1 AND kind = $2 AND used_at IS NULL AND expires_at > now()
     RETURNING user_id, email, data",
    )
    .bind(sha(token))
    .bind(kind)
    .fetch_optional(&state.pool)
    .await?)
}

// ---------------------------------------------------------------- workspaces & authorization

/// `personalWorkspace(userId)`.
pub async fn personal_workspace(state: &AppState, user_id: Uuid) -> Result<Uuid, ApiError> {
    let r: Option<Uuid> = sqlx::query_scalar(
        "SELECT m.workspace_id FROM memberships m JOIN workspaces w ON w.id = m.workspace_id WHERE m.user_id = $1 AND w.personal ORDER BY w.created_at LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    r.ok_or_else(|| ApiError::internal("user has no workspace"))
}

fn rank(r: Role) -> u8 {
    match r {
        Role::View => 1,
        Role::Comment => 2,
        Role::Edit => 3,
        Role::Owner => 4,
    }
}

pub fn parse_role(s: &str) -> Option<Role> {
    match s {
        "view" => Some(Role::View),
        "comment" => Some(Role::Comment),
        "edit" => Some(Role::Edit),
        "owner" => Some(Role::Owner),
        _ => None,
    }
}

/// `designRole(userId, designId)`: owner, team membership (team workspaces grant edit, or the design's explicit
/// team access level; workspace owner/admin get owner-level control), explicit or group share, or link access —
/// highest wins.
pub async fn design_role(state: &AppState, user_id: Option<Uuid>, design_id: &str) -> Result<Option<Role>, ApiError> {
    type DesignAccess = (Uuid, String, Option<DateTime<Utc>>, Uuid, bool, Option<String>);
    let d: Option<DesignAccess> = sqlx::query_as(
        "SELECT d.owner_id, d.link_access, d.trashed_at, d.workspace_id, w.personal, d.team_access FROM designs d JOIN workspaces w ON w.id = d.workspace_id WHERE d.id = $1",
    )
    .bind(design_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((owner_id, link_access, trashed_at, workspace_id, personal, team_access)) = d else { return Ok(None) };
    if user_id == Some(owner_id) {
        return Ok(Some(Role::Owner));
    }
    if trashed_at.is_some() {
        return Ok(None);
    }
    let mut best = if link_access == "restricted" { None } else { parse_role(&link_access) };
    let mut up = |r: Option<Role>| {
        if let Some(r) = r
            && best.is_none_or(|b| rank(r) > rank(b))
        {
            best = Some(r);
        }
    };
    if let Some(uid) = user_id {
        if !personal {
            let m: Option<String> = sqlx::query_scalar("SELECT role FROM memberships WHERE workspace_id = $1 AND user_id = $2").bind(workspace_id).bind(uid).fetch_optional(&state.pool).await?;
            if let Some(m) = m {
                up(Some(crate::share_access::team_member_design_role(&m, team_access.as_deref())));
            }
        }
        // Direct shares and team group shares (g149).
        let shares: Vec<String> = sqlx::query_scalar(crate::share_access::SHARE_ROLES_SQL).bind(design_id).bind(uid).fetch_all(&state.pool).await?;
        for s in shares {
            up(parse_role(&s));
        }
    }
    Ok(best)
}

/// `lockActiveUser(c, id)` (accountLock.ts): the account, when still active, locked FOR SHARE until the
/// transaction ends.
pub async fn lock_active_user(c: &mut sqlx::PgConnection, id: Uuid) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as("SELECT id, email FROM users WHERE id = $1 AND disabled_at IS NULL AND deleted_at IS NULL FOR SHARE").bind(id).fetch_optional(c).await
}

/// `atLeast(role, min)`.
pub fn at_least(role: Option<Role>, min: Role) -> bool {
    oc_auth::at_least(role, min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_rules() {
        assert_eq!(password_problem("short"), Some("Use at least 10 characters."));
        assert_eq!(password_problem("Password1"), Some("Use at least 10 characters."));
        assert_eq!(password_problem("1234567890"), Some("That password is too common."));
        assert_eq!(password_problem("aaaaaaaaaaaa"), Some("That password is too simple."));
        assert_eq!(password_problem("correct horse battery"), None);
    }

    #[tokio::test]
    async fn verifies_a_node_hash() {
        // hashPassword('correct horse battery') from services/api/src/auth.ts
        let h = hash_password("correct horse battery").await;
        assert!(verify_password("correct horse battery", Some(&h)).await);
        assert!(!verify_password("wrong", Some(&h)).await);
        assert!(!verify_password("x", Some("scrypt$15$8$1$AAAAAAAAAAAAAAAAAAAAAA==$AAAA")).await);
        assert!(!verify_password("x", None).await);
    }
}
