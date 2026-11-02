//! Port of services/api/src/routes/account.ts — authentication & account (PL-001..004, PL-008, TS-041/052):
//! signup with clickwrap + age gate, password login with TOTP 2FA and recovery codes, magic links, email
//! verification, password reset, passkeys (WebAuthn, crate::webauthn), OIDC sign-in (crate::jwt), session
//! management, data export and account deletion.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Datelike as _;
use hmac::{Hmac, Mac};
use regex::Regex;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::analytics;
use crate::auth::{
    UserFull, consume_token, create_session, create_user, current_user, destroy_session, find_or_create_user, find_user_by_email, hash_password, issue_token, password_problem,
    session_cookie, session_id, sha, user_by_id, verify_password,
};
pub use crate::login::{TERMS_VERSION, device_label};
use crate::login::{LoginOpts, MIN_AGE, public_user, safe_path};
use crate::routes::supabase::{PendingBridge, finish_supabase_login};
use crate::signup as signup_control;
use crate::supabase as sb;
use axum::extract::Request;
use crate::http::cookie::{self, CookieOpts};
use crate::http::ratelimit::strict;
use crate::prelude::*;
use crate::{jwt, webauthn};


/// Wrong codes after which a sign-in ticket is cancelled and the password has to be entered again.
const MFA_MAX_FAILURES: i32 = 5;
/// Wrong codes across all of an account's tickets from the last hour, after which only a recovery code is accepted.
/// A ticket lives 10 minutes, so whoever has the password could otherwise keep fetching new ones.
const MFA_ACCOUNT_MAX_FAILURES: i64 = 10;

pub const LIMITS: &[RouteLimit] = &[
    strict("POST", "/api/auth/signup"),
    strict("POST", "/api/auth/login"),
    strict("POST", "/api/auth/mfa"),
    strict("POST", "/api/auth/magic"),
    strict("POST", "/api/auth/magic/verify"),
    strict("POST", "/api/auth/verify/resend"),
    strict("POST", "/api/auth/reset/request"),
    strict("POST", "/api/auth/reset"),
    strict("POST", "/api/auth/passkey/login/options"),
    strict("POST", "/api/auth/passkey/login/verify"),
    strict("POST", "/api/account/password"),
    strict("POST", "/api/account/2fa/enable"),
    strict("POST", "/api/account/2fa/disable"),
    strict("DELETE", "/api/account"),
];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/providers", get(providers))
        .route("/api/auth/signup", post(signup))
        .route("/api/auth/login", post(login))
        .route("/api/auth/mfa", post(mfa))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .route("/api/auth/magic", post(magic))
        .route("/api/auth/magic/verify", post(magic_verify))
        .route("/api/auth/verify", post(verify_email))
        .route("/api/auth/verify/resend", post(verify_resend))
        .route("/api/auth/reset/request", post(reset_request))
        .route("/api/auth/reset", post(reset))
        .route("/api/auth/passkey/register/options", post(passkey_register_options))
        .route("/api/auth/passkey/register/verify", post(passkey_register_verify))
        .route("/api/auth/passkey/login/options", post(passkey_login_options))
        .route("/api/auth/passkey/login/verify", post(passkey_login_verify))
        .route("/api/auth/oauth/{provider}/start", get(oauth_start))
        .route("/api/auth/oauth/{provider}/callback", get(oauth_callback))
        .route("/api/account", get(account).patch(update_account).delete(delete_account))
        .route("/api/account/email-preferences", put(email_preferences))
        .route("/api/account/password", post(change_password))
        .route("/api/account/sessions", get(sessions))
        .route("/api/account/sessions/{id}", delete(delete_session))
        .route("/api/account/sessions/revoke-others", post(revoke_others))
        .route("/api/account/2fa/setup", post(totp_setup))
        .route("/api/account/2fa/enable", post(totp_enable))
        .route("/api/account/2fa/disable", post(totp_disable))
        .route("/api/account/passkeys/{id}", delete(delete_passkey))
        .route("/api/account/export", post(request_export))
        .route("/api/account/exports/{job}", get(download_export))
}

fn app_url(s: &AppState, path: &str) -> String {
    format!("{}{path}", s.cfg.web_origin)
}

fn rp_id(s: &AppState) -> String {
    oc_config::hostname(&s.cfg.web_origin).unwrap_or_default()
}

async fn audit(s: &AppState, actor: Option<Uuid>, action: &str, target: Option<&str>, meta: Option<Value>, ip: &str) -> Result<(), ApiError> {
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: actor, action: action.into(), target: target.map(str::to_owned), meta, ip: Some(ip.to_owned()), ..Default::default() },
        s.now(),
    )
    .await?;
    Ok(())
}

async fn mail(s: &AppState, to: &str, t: oc_mail::Template, user: Option<Uuid>) -> Result<(), ApiError> {
    oc_mail::send_email(&s.pool, to, &t, user).await?;
    Ok(())
}

/// `completeLogin(req, reply, u, method, { skipMfa })` (crate::login).
async fn complete_login(s: &AppState, req: &Req, u: &UserFull, method: &str, skip_mfa: bool) -> ApiResult {
    crate::login::complete_login(s, req, u, method, LoginOpts { skip_mfa, ..Default::default() }).await
}

// ---------------------------------------------------------------- TOTP (otpauth: SHA1, 6 digits, 30 s)

const B32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn base32_encode(b: &[u8]) -> String {
    let (mut bits, mut value, mut out) = (0u32, 0u32, String::new());
    for &x in b {
        value = (value << 8) | u32::from(x);
        bits += 8;
        while bits >= 5 {
            out.push(B32[((value >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(B32[((value << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// `base32Decode` (spaces and `=` padding removed, upper-cased); `None` on an invalid character.
pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let s: String = s.replace(' ', "").to_uppercase();
    let s = s.trim_end_matches('=');
    let (mut bits, mut value, mut out) = (0u32, 0u32, Vec::new());
    for c in s.bytes() {
        let idx = B32.iter().position(|&x| x == c)? as u32;
        value = (value << 5) | idx;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((value >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

/// `HOTP.generate({ secret, counter })`.
pub fn hotp(secret: &[u8], counter: u64) -> String {
    let mut m = Hmac::<Sha1>::new_from_slice(secret).expect("hmac key");
    m.update(&counter.to_be_bytes());
    let d = m.finalize().into_bytes();
    let off = (d[d.len() - 1] & 15) as usize;
    let otp = ((u32::from(d[off]) & 127) << 24 | u32::from(d[off + 1]) << 16 | u32::from(d[off + 2]) << 8 | u32::from(d[off + 3])) % 1_000_000;
    format!("{otp:06}")
}

/// `checkTotp(secret, email, code)`: window 1 around the current 30 s step. An undecodable secret throws in TS
/// (→ 500), so it is an error here.
fn check_totp(s: &AppState, secret: &str, code: &str) -> Result<bool, ApiError> {
    let token: String = code.chars().filter(|c| !crate::js::is_ws(*c)).collect();
    let key = base32_decode(secret).ok_or_else(|| ApiError::internal("Invalid character found in TOTP secret"))?;
    if token.encode_utf16().count() != 6 {
        return Ok(false);
    }
    let counter = (s.now_ms() as f64 / 1000.0 / 30.0).floor() as i64;
    for c in [counter, counter - 1, counter + 1] {
        if c >= 0 && hotp(&key, c as u64) == token {
            return Ok(true);
        }
    }
    Ok(false)
}

fn otpauth_url(secret: &str, email: &str) -> String {
    let e = crate::js::encode_uri_component;
    let issuer = oc_config::BRAND.name;
    format!("otpauth://totp/{}:{}?issuer={}&secret={}&algorithm=SHA1&digits=6&period=30", e(issuer), e(email), e(issuer), e(secret))
}

static SIX_DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{6}$").expect("regex"));

// ---------------------------------------------------------------- OIDC providers

#[derive(Clone)]
struct OidcProvider {
    id: &'static str,
    label: String,
    issuer: String,
    client_id: String,
    client_secret: String,
}

fn oidc_providers(s: &AppState) -> Vec<OidcProvider> {
    let mut out = Vec::new();
    let e = |k: &str| s.env_nonempty(k);
    if let (Some(id), Some(secret)) = (e("OC_GOOGLE_CLIENT_ID"), e("OC_GOOGLE_CLIENT_SECRET")) {
        out.push(OidcProvider { id: "google", label: "Google".into(), issuer: "https://accounts.google.com".into(), client_id: id, client_secret: secret });
    }
    if let (Some(id), Some(secret)) = (e("OC_APPLE_CLIENT_ID"), e("OC_APPLE_CLIENT_SECRET")) {
        out.push(OidcProvider { id: "apple", label: "Apple".into(), issuer: "https://appleid.apple.com".into(), client_id: id, client_secret: secret });
    }
    if let (Some(iss), Some(id), Some(secret)) = (e("OC_OIDC_ISSUER"), e("OC_OIDC_CLIENT_ID"), e("OC_OIDC_CLIENT_SECRET")) {
        out.push(OidcProvider { id: "oidc", label: s.env("OC_OIDC_LABEL").unwrap_or_else(|| "SSO".into()), issuer: iss, client_id: id, client_secret: secret });
    }
    out
}

/// The module-level discovery cache (`Map<issuer, Promise<config>>`; a failed fetch stays cached too).
#[derive(Default)]
struct Discovery(tokio::sync::Mutex<HashMap<String, Result<Value, String>>>);

async fn discover(s: &AppState, issuer: &str) -> Result<Value, ApiError> {
    let cache = s.local::<Discovery>();
    let mut map = cache.0.lock().await;
    if !map.contains_key(issuer) {
        let url = format!("{}/.well-known/openid-configuration", issuer.strip_suffix('/').unwrap_or(issuer));
        let r = match s.http.get(url).send().await {
            Ok(res) => match res.bytes().await {
                Ok(b) => serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            },
            Err(e) => Err(e.to_string()),
        };
        map.insert(issuer.to_owned(), r);
    }
    map[issuer].clone().map_err(ApiError::internal)
}

/// Tests: answer discovery for an issuer without fetching it (Google's and Apple's issuers are fixed public URLs).
#[doc(hidden)]
pub async fn seed_oidc_discovery(s: &AppState, issuer: &str, config: Value) {
    s.local::<Discovery>().0.lock().await.insert(issuer.to_owned(), Ok(config));
}

fn str_field<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

// ---------------------------------------------------------------- routes

async fn providers(State(s): State<AppState>, _req: Req) -> ApiResult {
    let on = sb::enabled(&s);
    let list: Vec<Value> = if on { vec![] } else { oidc_providers(&s).into_iter().map(|p| json!({ "id": p.id, "label": p.label })).collect() };
    // Only the URL and the publishable key reach the browser.
    let supabase = if on {
        let mut m = serde_json::Map::new();
        m.insert("url".into(), json!(s.cfg.supabase.url));
        m.insert("publishableKey".into(), json!(s.cfg.supabase.publishable_key));
        if let Value::Object(set) = sb::settings(&s).await {
            m.extend(set);
        }
        Value::Object(m)
    } else {
        Value::Null
    };
    Ok(Reply::json(json!({
        "providers": list,
        "passkeys": !on,
        "devLogin": !s.prod() && s.env("OC_DEV_LOGIN").as_deref() != Some("0"),
        "termsVersion": TERMS_VERSION,
        "signup": s.cfg.signup,
        "supabase": supabase,
    })))
}

async fn signup(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let year = i64::from(s.now().year());
    let b = z::obj([
        ("email", z::str().email().max(200.0)),
        ("password", z::str().max(200.0)),
        ("name", z::str().min(1.0).max(80.0)),
        ("birthYear", z::num().int().min(1900.0).max(year as f64)),
        ("acceptTerms", z::literal(json!(true))),
    ])
    .parse(req.body())?;
    let (email, password, name) = (b["email"].as_str().unwrap_or(""), b["password"].as_str().unwrap_or(""), b["name"].as_str().unwrap_or(""));
    let birth_year = b["birthYear"].as_f64().unwrap_or(0.0) as i64;
    if year - birth_year < MIN_AGE {
        return Ok(reply::json(400, json!({ "error": "age", "message": format!("You need to be at least {MIN_AGE} to create an account.") })));
    }
    if let Some(weak) = password_problem(password) {
        return Ok(reply::json(400, json!({ "error": "weak_password", "message": weak })));
    }
    if find_user_by_email(&s, email).await?.is_some() {
        return Ok(reply::json(409, json!({ "error": "exists", "message": "An account with that email already exists. Sign in instead." })));
    }
    if !signup_control::may_create(&s, email).await? {
        return Ok(signup_control::closed());
    }
    let user = create_user(&s, email, name, auth::NewUser { password: Some(password), verified: false, birth_year: Some(birth_year), ..Default::default() }).await?;
    audit(&s, Some(user.id), "auth.signup", None, Some(json!({ "terms": TERMS_VERSION })), &req.ip).await?;
    analytics::track(&s, Some(user.id), "signup", json!({ "method": "password" })).await?;
    audit(&s, Some(user.id), "consent.terms", Some(TERMS_VERSION), Some(json!({ "accepted": true, "ip": req.ip })), &req.ip).await?;
    let token = issue_token(&s, "verify", Some(user.id), None, None, 60 * 24 * 3).await?;
    mail(&s, &user.email, oc_mail::Template::Verify { url: app_url(&s, &format!("/auth/verify?token={token}")), name: user.name.clone() }, None).await?;
    let (set_cookie, _) = create_session(&s, &req, user.id, true).await?;
    Ok(Reply::json(json!({ "user": {
        "id": user.id.to_string(), "email": user.email, "name": user.name, "color": user.color,
        "verified": false, "mfa": false, "staff": false, "locale": "en",
    } }))
    .cookie(set_cookie))
}

const DUMMY_HASH: &str = "scrypt$15$8$1$AAAAAAAAAAAAAAAAAAAAAA==$AAAA";

async fn login(State(s): State<AppState>, req: Req) -> ApiResult {
    let b = z::obj([("email", z::str().email().max(200.0)), ("password", z::str().max(200.0).optional()), ("name", z::str().max(80.0).optional())]).parse(req.body())?;
    let email = b["email"].as_str().unwrap_or("");
    let password = b.get("password").and_then(Value::as_str).filter(|p| !p.is_empty());
    // Dev shortcut: email + name without a password (never in production).
    let Some(password) = password else {
        if s.prod() || s.env("OC_DEV_LOGIN").as_deref() == Some("0") {
            return Ok(reply::json(400, json!({ "error": "password_required" })));
        }
        let default_name = email.split('@').next().unwrap_or("").to_owned();
        let name = b.get("name").and_then(Value::as_str).map_or(default_name, str::to_owned);
        if !signup_control::may_create(&s, email).await? && find_user_by_email(&s, email).await?.is_none() {
            return Ok(signup_control::closed());
        }
        let u = find_or_create_user(&s, email, &name).await?;
        let full = find_user_by_email(&s, &u.email).await?.ok_or_else(|| ApiError::internal("user vanished"))?;
        return complete_login(&s, &req, &full, "dev", false).await;
    };
    if sb::enabled(&s) {
        return Ok(sb::builtin_reply());
    }
    let u = find_user_by_email(&s, email).await?;
    // Constant work whether or not the account exists.
    let ok = verify_password(password, Some(u.as_ref().and_then(|u| u.password_hash.as_deref()).unwrap_or(DUMMY_HASH))).await;
    match u {
        Some(u) if ok => complete_login(&s, &req, &u, "password", false).await,
        u => {
            audit(&s, u.as_ref().map(|u| u.id), "auth.login_failed", None, Some(json!({ "email": email })), &req.ip).await?;
            Ok(reply::json(401, json!({ "error": "invalid", "message": "Wrong email or password." })))
        }
    }
}

async fn mfa(State(s): State<AppState>, req: Req) -> ApiResult {
    let b = z::obj([("ticket", z::str()), ("code", z::str().max(20.0))]).parse(req.body())?;
    let ticket = b["ticket"].as_str().unwrap_or("");
    let expired = || reply::json(401, json!({ "error": "expired", "message": "That sign-in attempt expired. Start again." }));
    let t: Option<(Option<Uuid>, Option<sqlx::types::Json<Value>>)> = sqlx::query_as("SELECT user_id, data FROM auth_tokens WHERE id = $1 AND kind = $2 AND used_at IS NULL AND expires_at > now()")
        .bind(sha(ticket))
        .bind("mfa")
        .fetch_optional(&s.pool)
        .await?;
    let Some((Some(uid), data)) = t else { return Ok(expired()) };
    let u = user_by_id(&s, uid).await?.ok_or_else(|| ApiError::internal("Cannot read properties of undefined"))?;
    // Two-factor was turned off meanwhile.
    let Some(secret) = u.totp_secret.clone().filter(|t| !t.is_empty()) else { return Ok(expired()) };
    let code = crate::js::trim(b["code"].as_str().unwrap_or("")).to_owned();
    let stripped: String = code.chars().filter(|c| !crate::js::is_ws(*c)).collect();
    let failures: i64 = sqlx::query_scalar("SELECT COALESCE(sum(failures), 0)::bigint FROM auth_tokens WHERE kind = 'mfa' AND user_id = $1 AND expires_at > now() - interval '50 minutes'")
        .bind(u.id)
        .fetch_one(&s.pool)
        .await?;
    let locked = failures >= MFA_ACCOUNT_MAX_FAILURES;
    let mut ok = !locked && SIX_DIGITS.is_match(&stripped) && check_totp(&s, &secret, &code)?;
    let hashed = sha(&code.to_uppercase());
    if !ok && u.recovery_codes.as_ref().is_some_and(|c| c.contains(&hashed)) {
        ok = true;
        sqlx::query("UPDATE users SET recovery_codes = array_remove(recovery_codes, $2) WHERE id = $1").bind(u.id).bind(&hashed).execute(&s.pool).await?;
        audit(&s, Some(u.id), "auth.recovery_code_used", None, None, &req.ip).await?;
    }
    if !ok && locked {
        return Ok(reply::json(429, json!({ "error": "too_many_codes", "message": "Too many wrong codes. Try again in an hour, or use a recovery code." })));
    }
    if !ok {
        // The rate limit is per IP, so the ticket has to run out by itself or six digits can be walked.
        let spent: Option<bool> = sqlx::query_scalar(
            "UPDATE auth_tokens SET failures = failures + 1, used_at = CASE WHEN failures + 1 >= $2 THEN now() END
              WHERE id = $1 AND used_at IS NULL RETURNING used_at IS NOT NULL",
        )
        .bind(sha(ticket))
        .bind(MFA_MAX_FAILURES)
        .fetch_optional(&s.pool)
        .await?;
        if spent != Some(false) {
            return Ok(expired());
        }
        return Ok(reply::json(401, json!({ "error": "bad_code", "message": "That code didn’t work." })));
    }
    if consume_token(&s, "mfa", ticket).await?.is_none() {
        return Ok(expired()); // a concurrent submit of the same ticket won
    }
    sqlx::query("UPDATE auth_tokens SET failures = 0 WHERE kind = 'mfa' AND user_id = $1").bind(u.id).execute(&s.pool).await?;
    // A Supabase bridge waiting for this code (plan 12 §B3): its link, email sync and recovery sign-out happen only now.
    if let Some(p) = data.as_ref().and_then(|d| d.0.get("supabase")).filter(|v| crate::supabase::js_truthy(Some(v))).and_then(PendingBridge::from_json) {
        return finish_supabase_login(&s, &req, u.id, &p, "supabase+totp").await;
    }
    complete_login(&s, &req, &u, "password+totp", true).await
}

async fn logout(State(s): State<AppState>, req: Req) -> ApiResult {
    let c = destroy_session(&s, &req).await?;
    Ok(Reply::json(json!({ "ok": true })).cookie(c))
}

async fn me(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = current_user(&s, &req).await?;
    Ok(Reply::json(json!({ "user": u.as_ref().map(public_user) })))
}

// ---------------------------------------------------------------- magic link

async fn magic(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    // `next` can be a long OAuth consent URL (AI connector).
    let b = z::obj([("email", z::str().email().max(200.0)), ("next", z::str().max(2000.0).optional())]).parse(req.body())?;
    let email = b["email"].as_str().unwrap_or("");
    let u = find_user_by_email(&s, email).await?;
    let next = safe_path(b.get("next"));
    // OC_SIGNUP=invite is enforced in magic_verify, never here: this does the same work for every address, so neither
    // the answer nor its timing tells whether one is allowed.
    let token = issue_token(&s, "magic", u.map(|u| u.id), Some(email), Some(json!({ "next": next })), 15).await?;
    mail(&s, email, oc_mail::Template::Magic { url: app_url(&s, &format!("/auth/magic?token={token}")) }, None).await?;
    // Same response whether or not the account exists (no enumeration).
    Ok(Reply::json(json!({ "ok": true })))
}

async fn magic_verify(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let b = z::obj([
        ("token", z::str().max(200.0)),
        ("name", z::str().max(80.0).optional()),
        ("birthYear", z::num().int().optional()),
        ("acceptTerms", z::bool().optional()),
    ])
    .parse(req.body())?;
    let token = b["token"].as_str().unwrap_or("");
    type Peek = (Option<Uuid>, Option<String>, Option<sqlx::types::Json<Value>>);
    let peek: Option<Peek> = sqlx::query_as("SELECT user_id, email, data FROM auth_tokens WHERE id = $1 AND kind = $2 AND used_at IS NULL AND expires_at > now()")
        .bind(sha(token))
        .bind("magic")
        .fetch_optional(&s.pool)
        .await?;
    let Some((peek_user, peek_email, peek_data)) = peek else {
        return Ok(reply::json(400, json!({ "error": "expired", "message": "This link has expired or was already used." })));
    };
    let peek_email = peek_email.unwrap_or_default();
    let mut u = match peek_user {
        Some(id) => user_by_id(&s, id).await?,
        None => find_user_by_email(&s, &peek_email).await?,
    };
    if u.is_none() {
        if !signup_control::may_create(&s, &peek_email).await? {
            return Ok(signup_control::closed());
        }
        // New account via magic link: needs name, age and terms.
        let accept = b.get("acceptTerms").and_then(Value::as_bool).unwrap_or(false);
        let birth = b.get("birthYear").and_then(Value::as_f64).unwrap_or(0.0);
        let name = b.get("name").and_then(Value::as_str).unwrap_or("");
        if !accept || birth == 0.0 || name.is_empty() {
            return Ok(Reply::json(json!({ "needsProfile": true, "email": peek_email })));
        }
        if i64::from(s.now().year()) - (birth as i64) < MIN_AGE {
            return Ok(reply::json(400, json!({ "error": "age", "message": format!("You need to be at least {MIN_AGE}.") })));
        }
        let created = create_user(&s, &peek_email, name, auth::NewUser { verified: true, birth_year: Some(birth as i64), ..Default::default() }).await?;
        audit(&s, Some(created.id), "consent.terms", Some(TERMS_VERSION), Some(json!({ "accepted": true })), &req.ip).await?;
        u = find_user_by_email(&s, &peek_email).await?;
    }
    let u = u.ok_or_else(|| ApiError::internal("Cannot read properties of null"))?;
    consume_token(&s, "magic", token).await?;
    sqlx::query("UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1").bind(u.id).execute(&s.pool).await?;
    let fresh = user_by_id(&s, u.id).await?.ok_or_else(|| ApiError::internal("user vanished"))?;
    // Where to resume after signing in (the action that asked for an account).
    let mut extra = serde_json::Map::new();
    extra.insert("next".into(), json!(safe_path(peek_data.as_ref().and_then(|d| d.0.get("next")))));
    crate::login::complete_login(&s, &req, &fresh, "magic", LoginOpts { extra, ..Default::default() }).await
}

// ---------------------------------------------------------------- email verification / password reset

async fn verify_email(State(s): State<AppState>, req: Req) -> ApiResult {
    let b = z::obj([("token", z::str().max(200.0))]).parse(req.body())?;
    let t = consume_token(&s, "verify", b["token"].as_str().unwrap_or("")).await?;
    let Some(uid) = t.and_then(|t| t.user_id) else {
        return Ok(reply::json(400, json!({ "error": "expired", "message": "This link has expired. Request a new one from Settings." })));
    };
    sqlx::query("UPDATE users SET email_verified_at = now() WHERE id = $1").bind(uid).execute(&s.pool).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn verify_resend(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let u = require_user(&s, &req).await?;
    let token = issue_token(&s, "verify", Some(u.id), None, None, 60 * 24 * 3).await?;
    mail(&s, &u.email, oc_mail::Template::Verify { url: app_url(&s, &format!("/auth/verify?token={token}")), name: u.name.clone() }, None).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn reset_request(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let b = z::obj([("email", z::str().email().max(200.0))]).parse(req.body())?;
    if let Some(u) = find_user_by_email(&s, b["email"].as_str().unwrap_or("")).await? {
        let token = issue_token(&s, "reset", Some(u.id), None, None, 30).await?;
        mail(&s, &u.email, oc_mail::Template::Reset { url: app_url(&s, &format!("/auth/reset?token={token}")) }, None).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}

async fn reset(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let b = z::obj([("token", z::str().max(200.0)), ("password", z::str().max(200.0))]).parse(req.body())?;
    let password = b["password"].as_str().unwrap_or("");
    if let Some(weak) = password_problem(password) {
        return Ok(reply::json(400, json!({ "error": "weak_password", "message": weak })));
    }
    let t = consume_token(&s, "reset", b["token"].as_str().unwrap_or("")).await?;
    let Some(uid) = t.and_then(|t| t.user_id) else {
        return Ok(reply::json(400, json!({ "error": "expired", "message": "This link has expired. Request a new one." })));
    };
    sqlx::query("UPDATE users SET password_hash = $2, email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1").bind(uid).bind(hash_password(password).await).execute(&s.pool).await?;
    // A reset signs out every session.
    sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(uid).execute(&s.pool).await?;
    audit(&s, Some(uid), "auth.password_reset", None, None, &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- passkeys (WebAuthn)

struct Challenge {
    challenge: String,
    at: i64,
}

/// The routes' `challenges` map (5-minute lifetime, pruned on every insert).
#[derive(Default)]
struct Challenges(Mutex<HashMap<String, Challenge>>);

fn put_challenge(s: &AppState, key: String, challenge: String) {
    let now = s.now_ms();
    let c = s.local::<Challenges>();
    let mut m = c.0.lock().expect("challenges");
    m.insert(key, Challenge { challenge, at: now });
    m.retain(|_, v| now - v.at <= 5 * 60_000);
}

fn take_challenge(s: &AppState, key: &str, remove: bool) -> Option<String> {
    let c = s.local::<Challenges>();
    let mut m = c.0.lock().expect("challenges");
    if remove { m.remove(key).map(|c| c.challenge) } else { m.get(key).map(|c| c.challenge.clone()) }
}

async fn passkey_register_options(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let u = require_user(&s, &req).await?;
    let existing: Vec<(String, Option<Vec<String>>)> = sqlx::query_as("SELECT id, transports FROM passkeys WHERE user_id = $1").bind(u.id).fetch_all(&s.pool).await?;
    let challenge = auth::random_bytes(32);
    let opts = webauthn::registration_options(&challenge, oc_config::BRAND.name, &rp_id(&s), &u.email, &u.name, u.id.to_string().as_bytes(), &existing);
    put_challenge(&s, format!("reg:{}", u.id), webauthn::b64url(&challenge));
    Ok(Reply::json(opts))
}

async fn passkey_register_verify(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let u = require_user(&s, &req).await?;
    let body = req.body().cloned().unwrap_or(Value::Null);
    let Some(challenge) = take_challenge(&s, &format!("reg:{}", u.id), false) else { return Ok(reply::json(400, json!({ "error": "expired" }))) };
    let response = body.get("response").ok_or_else(|| ApiError::internal("Cannot read properties of undefined (reading 'id')"))?;
    let v = webauthn::verify_registration(response, &challenge, &s.cfg.web_origin, &rp_id(&s)).map_err(ApiError::internal)?;
    let Some(cred) = v else { return Ok(reply::json(400, json!({ "error": "not_verified" }))) };
    let name = match body.get("name") {
        None | Some(Value::Null) => device_label(&req.user_agent()),
        Some(Value::String(n)) => n.clone(),
        Some(_) => return Err(ApiError::internal("name.slice is not a function")),
    };
    sqlx::query("INSERT INTO passkeys(id, user_id, public_key, counter, transports, name) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(&cred.id)
        .bind(u.id)
        .bind(&cred.public_key)
        .bind(i64::from(cred.counter))
        .bind(&cred.transports)
        .bind(crate::js::take(&name, 80))
        .execute(&s.pool)
        .await?;
    take_challenge(&s, &format!("reg:{}", u.id), true);
    audit(&s, Some(u.id), "auth.passkey_added", None, None, &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn passkey_login_options(State(s): State<AppState>, raw: Request) -> ApiResult {
    let _req = sb::builtin_req(&s, raw).await?;
    let challenge = auth::random_bytes(32);
    let opts = webauthn::authentication_options(&challenge, &rp_id(&s));
    let c = webauthn::b64url(&challenge);
    put_challenge(&s, format!("auth:{c}"), c);
    Ok(Reply::json(opts))
}

async fn passkey_login_verify(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let body = req.body().cloned().unwrap_or(Value::Null);
    let response = body.get("response").filter(|r| !r.is_null()).ok_or_else(|| ApiError::internal("Cannot read properties of undefined (reading 'id')"))?;
    // `response.id` as node-postgres would bind it (undefined → NULL → no row).
    let id: Option<String> = match response.get("id") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(v) => Some(crate::http::jsjson::stringify(v)),
    };
    let pk: Option<(String, Uuid, Vec<u8>, i64)> = sqlx::query_as("SELECT id, user_id, public_key, counter FROM passkeys WHERE id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some((pk_id, pk_user, public_key, counter)) = pk else {
        return Ok(reply::json(401, json!({ "error": "unknown_passkey", "message": "That passkey isn’t registered here." })));
    };
    let cdj = response.pointer("/response/clientDataJSON").and_then(Value::as_str).ok_or_else(|| ApiError::internal("clientDataJSON missing"))?;
    let client_data: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(cdj.trim_end_matches('=')).unwrap_or_default()).map_err(ApiError::internal)?;
    let key = format!("auth:{}", client_data.get("challenge").and_then(Value::as_str).unwrap_or("undefined"));
    let Some(challenge) = take_challenge(&s, &key, false) else { return Ok(reply::json(400, json!({ "error": "expired" }))) };
    let v = webauthn::verify_authentication(response, &challenge, &s.cfg.web_origin, &rp_id(&s), &public_key, counter);
    take_challenge(&s, &key, true);
    let Some(new_counter) = v.map_err(ApiError::internal)? else {
        return Ok(reply::json(401, json!({ "error": "not_verified" })));
    };
    sqlx::query("UPDATE passkeys SET counter = $2, last_used_at = now() WHERE id = $1").bind(&pk_id).bind(i64::from(new_counter)).execute(&s.pool).await?;
    let u: Option<UserFull> = sqlx::query_as("SELECT * FROM users WHERE id = $1 AND deleted_at IS NULL").bind(pk_user).fetch_optional(&s.pool).await?;
    let u = u.ok_or_else(|| ApiError::internal("Cannot read properties of undefined (reading 'disabled_at')"))?;
    // A passkey is a strong factor on its own: skip TOTP.
    complete_login(&s, &req, &u, "passkey", true).await
}

// ---------------------------------------------------------------- OIDC (Google / Apple / SSO)

fn redirect_uri(s: &AppState, p: &OidcProvider) -> String {
    format!("{}/api/auth/oauth/{}/callback", s.cfg.api_url, p.id)
}

async fn oauth_start(State(s): State<AppState>, Path(provider): Path<String>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let Some(p) = oidc_providers(&s).into_iter().find(|x| x.id == provider) else {
        return Ok(reply::json(404, json!({ "error": "unknown_provider" })));
    };
    let d = discover(&s, &p.issuer).await?;
    let state = URL_SAFE_NO_PAD.encode(auth::random_bytes(16));
    let verifier = URL_SAFE_NO_PAD.encode(auth::random_bytes(32));
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let next = match req.query.get("next") {
        Some(Value::String(n)) => Value::String(n.clone()),
        Some(v) => v.clone(),
        None => json!("/"),
    };
    let st = crate::http::jsjson::stringify(&json!({ "state": state, "verifier": verifier, "p": p.id, "next": next }));
    let set_cookie = cookie::serialize(
        "oc_oauth",
        &cookie::sign(&s.cfg.secret, &st),
        &CookieOpts { http_only: true, secure: s.prod(), path: Some("/".into()), max_age: Some(600), ..Default::default() },
    );
    let mut u = url::Url::parse(str_field(&d, "authorization_endpoint")).map_err(|e| ApiError::internal(format!("Invalid URL: {e}")))?;
    let mut q = form_urlencoded::Serializer::new(String::new());
    q.append_pair("client_id", &p.client_id)
        .append_pair("response_type", "code")
        .append_pair("scope", "openid email profile")
        .append_pair("redirect_uri", &redirect_uri(&s, &p))
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    if p.id == "apple" {
        q.append_pair("response_mode", "query");
    }
    let qs = q.finish();
    u.set_query(if qs.is_empty() { None } else { Some(&qs) });
    Ok(Reply::redirect(u.as_str()).cookie(set_cookie))
}

async fn oauth_callback(State(s): State<AppState>, Path(provider): Path<String>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let q_code = req.query_str("code").map(str::to_owned);
    let q_state = req.query.get("state").cloned();
    let raw = cookie::unsign(&s.cfg.secret, req.cookie("oc_oauth").unwrap_or(""));
    let st: Option<Value> = match raw.filter(|v| !v.is_empty()) {
        Some(v) => Some(crate::http::jsjson::parse(&v).map_err(ApiError::internal)?),
        None => None,
    };
    let p = oidc_providers(&s).into_iter().find(|x| x.id == provider);
    let fail = Reply::redirect(&app_url(&s, "/login?error=oauth"));
    let (Some(p), Some(st), Some(code)) = (p, st, q_code.filter(|c| !c.is_empty())) else { return Ok(fail) };
    if st.get("p").and_then(Value::as_str) != Some(provider.as_str()) || st.get("state") != q_state.as_ref() {
        return Ok(fail);
    }
    let d = discover(&s, &p.issuer).await?;
    let form = form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", &code)
        .append_pair("redirect_uri", &redirect_uri(&s, &p))
        .append_pair("client_id", &p.client_id)
        .append_pair("client_secret", &p.client_secret)
        .append_pair("code_verifier", st.get("verifier").and_then(Value::as_str).unwrap_or("undefined"))
        .finish();
    let tok: Value = {
        let res = s.http.post(str_field(&d, "token_endpoint")).header("content-type", "application/x-www-form-urlencoded").body(form).send().await?;
        serde_json::from_slice(&res.bytes().await?)?
    };
    let Some(id_token) = tok.get("id_token").and_then(Value::as_str).filter(|t| !t.is_empty()) else { return Ok(fail) };
    let payload = jwt::verify(&s.http, id_token, str_field(&d, "jwks_uri"), str_field(&d, "issuer"), &p.client_id, s.now_ms()).await.map_err(ApiError::internal)?;
    let js_string = |v: Option<&Value>| match v {
        None => "undefined".to_owned(),
        Some(Value::String(x)) => x.clone(),
        Some(x) => crate::http::jsjson::stringify(x),
    };
    let subject = js_string(payload.get("sub"));
    let email = match payload.get("email") {
        None | Some(Value::Null) => String::new(),
        v => js_string(v).to_lowercase(),
    };
    if email.is_empty() || payload.get("email_verified") == Some(&Value::Bool(false)) {
        return Ok(Reply::redirect(&app_url(&s, "/login?error=unverified_email")));
    }
    let link: Option<Uuid> = sqlx::query_scalar("SELECT user_id FROM oauth_accounts WHERE provider = $1 AND subject = $2").bind(p.id).bind(&subject).fetch_optional(&s.pool).await?;
    let mut u: Option<UserFull> = match link {
        Some(uid) => sqlx::query_as("SELECT * FROM users WHERE id = $1 AND deleted_at IS NULL").bind(uid).fetch_optional(&s.pool).await?,
        None => find_user_by_email(&s, &email).await?,
    };
    if u.is_none() && !signup_control::may_create(&s, &email).await? {
        return Ok(Reply::redirect(&app_url(&s, "/login?error=signup_closed")).cookie(cookie::clear("oc_oauth", "/")));
    }
    if u.is_none() && p.id != "oidc" {
        // A new consumer account still needs name, age and terms: hand over to the magic-link profile step. Google
        // proved the address, and the next sign-in links this identity by email. Company SSO (`oidc`) has its own admins.
        let data = json!({ "next": safe_path(st.get("next")) });
        let token = issue_token(&s, "magic", None, Some(&email), Some(data), 15).await?;
        return Ok(Reply::redirect(&app_url(&s, &format!("/auth/magic?token={token}"))).cookie(cookie::clear("oc_oauth", "/")));
    }
    if u.is_none() {
        let name = match payload.get("name") {
            None | Some(Value::Null) => email.split('@').next().unwrap_or("").to_owned(),
            v => js_string(v),
        };
        create_user(&s, &email, &name, auth::NewUser { verified: true, ..Default::default() }).await?;
        u = find_user_by_email(&s, &email).await?;
    }
    let u = u.ok_or_else(|| ApiError::internal("Cannot read properties of null (reading 'id')"))?;
    sqlx::query("INSERT INTO oauth_accounts(provider, subject, user_id, email) VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(p.id).bind(&subject).bind(u.id).bind(&email).execute(&s.pool).await?;
    let clear = cookie::clear("oc_oauth", "/");
    let next = safe_path(st.get("next"));
    if u.totp_secret.as_deref().is_some_and(|t| !t.is_empty()) {
        let ticket = issue_token(&s, "mfa", Some(u.id), None, None, 10).await?;
        return Ok(Reply::redirect(&app_url(&s, &format!("/login?mfa={ticket}&next={}", crate::js::encode_uri_component(&next)))).cookie(clear));
    }
    let (set_cookie, _) = create_session(&s, &req, u.id, true).await?;
    audit(&s, Some(u.id), "auth.login", None, Some(json!({ "method": p.id })), &req.ip).await?;
    Ok(Reply::redirect(&app_url(&s, &next)).cookie(clear).cookie(set_cookie))
}

// ---------------------------------------------------------------- signed-in account management

async fn full_user(s: &AppState, id: Uuid) -> Result<UserFull, ApiError> {
    user_by_id(s, id).await?.ok_or_else(|| ApiError::internal("Cannot read properties of undefined"))
}

async fn account(State(s): State<AppState>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    let u = full_user(&s, me.id).await?;
    let passkeys = pg::json_rows(&s.pool, "SELECT id, name, created_at, last_used_at FROM passkeys WHERE user_id = $1 ORDER BY created_at", &[pg::uuid(me.id)]).await?;
    let oauth = pg::json_rows(&s.pool, "SELECT provider, email, created_at FROM oauth_accounts WHERE user_id = $1", &[pg::uuid(me.id)]).await?;
    let prefs = pg::json_one(&s.pool, "SELECT mentions, comments, shares, product FROM email_preferences WHERE user_id = $1", &[pg::uuid(me.id)]).await?;
    Ok(Reply::json(json!({
        "user": public_user(&u),
        "hasPassword": u.password_hash.as_deref().is_some_and(|h| !h.is_empty()),
        "passkeys": passkeys,
        "oauth": oauth,
        "prefs": prefs,
        "recoveryCodesLeft": u.recovery_codes.as_ref().map_or(0, Vec::len),
        // supabaseId: the web compares it with its Supabase session's user, which may use another address (plan 12 §C2).
        // supabaseEmail: that user's address at Supabase, for the re-auth form without a Supabase session (F2).
        "auth": {
            "mode": if sb::enabled(&s) { "supabase" } else { "builtin" },
            "supabaseLinked": u.supabase_id.is_some(),
            "supabaseId": u.supabase_id.map(|x| x.to_string()),
            "supabaseEmail": match u.supabase_id {
                Some(id) if sb::enabled(&s) => sb::admin_email(&s, &id.to_string()).await,
                _ => None,
            },
        },
    })))
}

async fn update_account(State(s): State<AppState>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    let b = z::obj([("name", z::str().min(1.0).max(80.0).optional()), ("locale", z::str().max(10.0).optional())]).parse(req.body())?;
    if let Some(name) = b.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) {
        sqlx::query("UPDATE users SET name = $2 WHERE id = $1").bind(me.id).bind(crate::js::trim(name)).execute(&s.pool).await?;
    }
    if let Some(locale) = b.get("locale").and_then(Value::as_str).filter(|l| !l.is_empty()) {
        sqlx::query("UPDATE users SET locale = $2 WHERE id = $1").bind(me.id).bind(locale).execute(&s.pool).await?;
    }
    Ok(Reply::json(json!({ "ok": true })))
}

async fn email_preferences(State(s): State<AppState>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    let b = z::obj([("mentions", z::bool()), ("comments", z::bool()), ("shares", z::bool()), ("product", z::bool())]).parse(req.body())?;
    let f = |k: &str| b[k].as_bool().unwrap_or(false);
    sqlx::query(
        "INSERT INTO email_preferences(user_id, mentions, comments, shares, product) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (user_id) DO UPDATE SET mentions = $2, comments = $3, shares = $4, product = $5",
    )
    .bind(me.id)
    .bind(f("mentions"))
    .bind(f("comments"))
    .bind(f("shares"))
    .bind(f("product"))
    .execute(&s.pool)
    .await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn change_password(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let me = require_user(&s, &req).await?;
    let b = z::obj([("current", z::str().max(200.0).optional()), ("password", z::str().max(200.0))]).parse(req.body())?;
    let u = full_user(&s, me.id).await?;
    if let Some(h) = u.password_hash.as_deref().filter(|h| !h.is_empty())
        && !verify_password(b.get("current").and_then(Value::as_str).unwrap_or(""), Some(h)).await
    {
        return Ok(reply::json(401, json!({ "error": "invalid", "message": "Your current password is wrong." })));
    }
    let password = b["password"].as_str().unwrap_or("");
    if let Some(weak) = password_problem(password) {
        return Ok(reply::json(400, json!({ "error": "weak_password", "message": weak })));
    }
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1").bind(u.id).bind(hash_password(password).await).execute(&s.pool).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id <> $2").bind(u.id).bind(session_id(&s, &req)).execute(&s.pool).await?;
    audit(&s, Some(u.id), "auth.password_changed", None, None, &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn sessions(State(s): State<AppState>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    let sid = session_id(&s, &req);
    let rows = pg::json_rows(
        &s.pool,
        "SELECT id, user_agent, ip, created_at, last_seen_at FROM sessions WHERE user_id = $1 AND expires_at > now() ORDER BY last_seen_at DESC",
        &[pg::uuid(me.id)],
    )
    .await?;
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            let id = r["id"].as_str().unwrap_or("");
            json!({
                "id": crate::js::take(id, 16),
                "device": device_label(r["user_agent"].as_str().unwrap_or("")),
                "ip": r["ip"],
                "created_at": r["created_at"],
                "last_seen_at": r["last_seen_at"],
                "current": sid.as_deref() == Some(id),
            })
        })
        .collect();
    Ok(Reply::json(json!({ "sessions": out })))
}

async fn delete_session(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND left(id, 16) = $2").bind(me.id).bind(&id).execute(&s.pool).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn revoke_others(State(s): State<AppState>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    let b = z::obj([("accessToken", z::str().min(1.0).max(8192.0).optional())]).parse(Some(&req.body_or_empty()))?;
    let token = b.get("accessToken").and_then(Value::as_str).map(str::to_owned);
    // In Supabase mode with the user's current token, the other Supabase sessions are signed out too (plan 12 §B9).
    let mut authed = false;
    if let Some(t) = token.as_deref().filter(|_| sb::enabled(&s)) {
        if let Err(r) = sb::assert_same_origin(&s, &req) {
            return Ok(r);
        }
        let claims = match sb::authenticate(&s, t).await {
            Ok((c, _)) => c,
            Err(e) => return Ok(e.reply()),
        };
        let mine: Option<Uuid> = sqlx::query_scalar("SELECT supabase_id FROM users WHERE id = $1").bind(me.id).fetch_one(&s.pool).await?;
        if mine.map(|x| x.to_string()).as_deref() != Some(claims.sub.as_str()) {
            return Ok(reply::json(403, json!({ "error": "wrong_account", "message": "That sign-in belongs to a different account." })));
        }
        authed = true;
    }
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id <> $2").bind(me.id).bind(session_id(&s, &req)).execute(&s.pool).await?;
    let mut supabase = "skipped";
    if authed {
        supabase = if sb::logout(&s, token.as_deref().unwrap_or(""), "others").await.is_ok() { "revoked" } else { "failed" };
    }
    audit(&s, Some(me.id), "auth.sessions_revoked", None, authed.then(|| json!({ "supabase": supabase })), &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true, "supabase": supabase })))
}

/// A pending TOTP secret is stored as `SECRET:issued-at-ms` (older rows: just `SECRET`). For 2 minutes after it
/// was issued, setup returns it again instead of a new one (AUTH-01). One UPDATE, so concurrent calls agree: the
/// second waits for the first's row lock and re-evaluates the CASE on the row the first wrote.
const TOTP_SETUP_SQL: &str = "UPDATE users SET totp_pending = CASE
    WHEN totp_pending ~ '^[A-Z2-7]+:[0-9]+$' AND split_part(totp_pending, ':', 2)::bigint > (extract(epoch FROM clock_timestamp()) * 1000)::bigint - 120000
    THEN totp_pending ELSE $2 || ':' || (extract(epoch FROM clock_timestamp()) * 1000)::bigint END
  WHERE id = $1 RETURNING split_part(totp_pending, ':', 1) AS secret";

async fn totp_setup(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let me = require_user(&s, &req).await?;
    // Idempotent for a short while (AUTH-01): a repeated call (React StrictMode runs the setup effect twice, a
    // double click, a retry) gets the pending secret it already showed, not a new one that silently replaces it.
    let fresh = base32_encode(&auth::random_bytes(20));
    let secret: String = sqlx::query_scalar(TOTP_SETUP_SQL).bind(me.id).bind(&fresh).fetch_one(&s.pool).await?;
    Ok(Reply::json(json!({ "secret": secret, "otpauthUrl": otpauth_url(&secret, &me.email) })))
}

async fn totp_enable(State(s): State<AppState>, raw: Request) -> ApiResult {
    let req = sb::builtin_req(&s, raw).await?;
    let me = require_user(&s, &req).await?;
    let b = z::obj([("code", z::str().max(10.0))]).parse(req.body())?;
    let pending: Option<String> = sqlx::query_scalar("SELECT split_part(totp_pending, ':', 1) FROM users WHERE id = $1").bind(me.id).fetch_optional(&s.pool).await?.flatten();
    let ok = match pending.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => check_totp(&s, p, b["code"].as_str().unwrap_or(""))?,
        None => false,
    };
    if !ok {
        return Ok(reply::json(400, json!({ "error": "bad_code", "message": "That code didn’t match. Check your authenticator app’s clock." })));
    }
    let codes: Vec<String> = (0..10)
        .map(|_| {
            let h = hex::encode(auth::random_bytes(5)).to_uppercase();
            format!("{}-{}", &h[..5], &h[5..])
        })
        .collect();
    let hashed: Vec<String> = codes.iter().map(|c| sha(c)).collect();
    sqlx::query("UPDATE users SET totp_secret = split_part(totp_pending, ':', 1), totp_pending = NULL, recovery_codes = $2 WHERE id = $1").bind(me.id).bind(&hashed).execute(&s.pool).await?;
    audit(&s, Some(me.id), "auth.2fa_enabled", None, None, &req.ip).await?;
    Ok(Reply::json(json!({ "recoveryCodes": codes })))
}

async fn totp_disable(State(s): State<AppState>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    let b = z::obj([("code", z::str().max(20.0))]).parse(req.body())?;
    let u = full_user(&s, me.id).await?;
    let ok = match u.totp_secret.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => check_totp(&s, t, b["code"].as_str().unwrap_or(""))?,
        None => false,
    };
    if !ok {
        return Ok(reply::json(400, json!({ "error": "bad_code", "message": "That code didn’t work." })));
    }
    sqlx::query("UPDATE users SET totp_secret = NULL, recovery_codes = NULL WHERE id = $1").bind(u.id).execute(&s.pool).await?;
    audit(&s, Some(u.id), "auth.2fa_disabled", None, None, &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn delete_passkey(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    sqlx::query("DELETE FROM passkeys WHERE id = $1 AND user_id = $2").bind(&id).bind(me.id).execute(&s.pool).await?;
    audit(&s, Some(me.id), "auth.passkey_removed", None, None, &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- data rights (PL-008)

async fn request_export(State(s): State<AppState>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    let id = oc_jobs::enqueue(&s.pool, "account-export", &json!({ "userId": me.id.to_string() }), oc_jobs::EnqueueOpts { owner_id: Some(me.id), label: Some("Preparing your export".into()), ..Default::default() }).await?;
    audit(&s, Some(me.id), "privacy.export_requested", None, None, &req.ip).await?;
    Ok(Reply::json(json!({ "jobId": id.to_string() })))
}

async fn download_export(State(s): State<AppState>, Path(job): Path<String>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    if job.len() != 36 || !job.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || c == b'-') {
        return Ok(Reply::empty(404));
    }
    let j: Option<oc_jobs::Job> = sqlx::query_as("SELECT * FROM jobs WHERE id = $1::uuid").bind(&job).fetch_optional(&s.pool).await?;
    let Some(j) = j.filter(|j| j.queue == "account-export" && j.owner_id == Some(me.id)) else {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    };
    let r = j.result.as_ref().map(|r| r.0.clone()).filter(|r| !r.is_null());
    let Some(r) = r.filter(|_| j.status == oc_jobs::JobStatus::Done) else {
        let status = serde_json::to_value(j.status).unwrap_or(Value::Null);
        let progress = crate::http::jsjson::number_value(j.progress.to_string().parse().unwrap_or(0.0)).unwrap_or(Value::Null);
        return Ok(Reply::json(json!({ "status": status, "progress": progress, "label": j.label })));
    };
    let finished = j.finished_at.map_or(f64::NAN, |t| t.timestamp_millis() as f64);
    if (s.now_ms() as f64) - finished > 7.0 * 86_400_000.0 {
        return Ok(reply::json(410, json!({ "error": "expired", "message": "This export link has expired. Request a new export in Settings." })));
    }
    let text = |k: &str| match r.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(v) => crate::http::jsjson::stringify(v),
        None => "undefined".into(),
    };
    let stream = s.store.stream(&text("sha"), None).await?;
    Ok(Reply::bytes("application/zip", axum::body::Body::from_stream(crate::routes::assets::tokio_util_stream(stream)))
        .header("content-length", text("size"))
        .header("content-disposition", format!("attachment; filename=\"{}\"", text("filename"))))
}

async fn delete_account(State(s): State<AppState>, req: Req) -> ApiResult {
    let me = require_user(&s, &req).await?;
    let b = z::obj([
        ("password", z::str().max(200.0).optional()),
        ("code", z::str().max(20.0).optional()),
        ("confirm", z::literal(json!("DELETE"))),
        ("accessToken", z::str().min(1.0).max(8192.0).optional()),
    ])
    .parse(req.body())?;
    let u = full_user(&s, me.id).await?;
    // A linked account in Supabase mode re-proves itself with a fresh Supabase token instead of a local password (plan 12 §B9).
    let via_supabase = sb::enabled(&s) && u.supabase_id.is_some();
    if via_supabase {
        if let Err(r) = sb::assert_same_origin(&s, &req) {
            return Ok(r);
        }
        let reauth = || reply::json(401, json!({ "error": "reauth_required", "message": "Sign in again to confirm it’s you." }));
        let Some(t) = b.get("accessToken").and_then(Value::as_str) else { return Ok(reauth()) };
        let (claims, user) = match sb::authenticate(&s, t).await {
            Ok(x) => x,
            Err(e) => return Ok(e.reply()),
        };
        if u.supabase_id.map(|x| x.to_string()).as_deref() != Some(claims.sub.as_str()) {
            return Ok(reply::json(403, json!({ "error": "wrong_account", "message": "That sign-in belongs to a different account." })));
        }
        if user.has_verified_factor() && claims.aal != "aal2" {
            return Ok(reply::json(403, json!({ "error": "mfa_required", "message": "Confirm with your authenticator app first." })));
        }
        if s.now_ms() as f64 / 1000.0 - claims.last_auth_at() > 600.0 {
            return Ok(reauth());
        }
    } else if let Some(h) = u.password_hash.as_deref().filter(|h| !h.is_empty())
        && !verify_password(b.get("password").and_then(Value::as_str).unwrap_or(""), Some(h)).await
    {
        return Ok(reply::json(401, json!({ "error": "invalid", "message": "Your password is wrong." })));
    }
    if let Some(t) = u.totp_secret.as_deref().filter(|t| !t.is_empty())
        && !check_totp(&s, t, b.get("code").and_then(Value::as_str).unwrap_or(""))?
    {
        return Ok(reply::json(401, json!({ "error": "bad_code", "message": "Enter a current two-factor code." })));
    }
    // The Supabase user goes first: if that fails, nothing here has changed and the user can retry.
    let mut supabase: Option<&str> = None;
    if via_supabase {
        let r = sb::admin_delete(&s, &u.supabase_id.map(|x| x.to_string()).unwrap_or_default()).await;
        if r == "failed" {
            return Ok(reply::json(503, json!({ "error": "supabase_unavailable", "message": sb::UNAVAILABLE_MSG })));
        }
        supabase = Some(r);
    }
    // The Supabase link goes either way (also with Supabase Auth off): a deleted account keeps no claim on that identity.
    let sql = if u.supabase_id.is_some() {
        "UPDATE users SET supabase_id = NULL, disabled_at = now(), disabled_reason = $2 WHERE id = $1"
    } else {
        "UPDATE users SET disabled_at = now(), disabled_reason = $2 WHERE id = $1"
    };
    sqlx::query(sql).bind(u.id).bind("deletion_requested").execute(&s.pool).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(u.id).execute(&s.pool).await?;
    oc_jobs::enqueue(&s.pool, "account-delete", &json!({ "userId": u.id.to_string(), "email": u.email }), oc_jobs::EnqueueOpts { label: Some("Deleting account".into()), ..Default::default() }).await?;
    audit(&s, Some(u.id), "privacy.account_deleted", None, supabase.map(|r| json!({ "supabase": r })), &req.ip).await?;
    Ok(Reply::json(json!({ "ok": true })).cookie(cookie::clear(session_cookie(&s), "/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_rfc6238_vector() {
        // RFC 6238 SHA1 test secret "12345678901234567890", T = 59 s → counter 1 → 94287082 (8 digits) / 287082.
        assert_eq!(hotp(b"12345678901234567890", 1), "287082");
        assert_eq!(base32_decode(&base32_encode(b"hello world!")).unwrap(), b"hello world!");
        assert_eq!(device_label("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/17.0 Safari/605.1.15"), "Safari on macOS");
    }
}
