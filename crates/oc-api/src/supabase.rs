//! Port of services/api/src/supabase.ts — optional Supabase Auth (plan 12 §B): verifying Supabase access tokens,
//! the public settings (providers), and the few calls Pinselo makes to Supabase Auth (logout scopes, admin
//! delete, admin email). Enabled only when `OC_SUPABASE_URL` and `OC_SUPABASE_PUBLISHABLE_KEY` are set (read once
//! at start-up into [`oc_config::Config::supabase`], like the TS module-level `config`). The secret key and the
//! legacy JWT secret are read only here, never logged, never sent to browsers.
//!
//! Token verification reproduces `jose` 6.2.12 (`jwtVerify` with `createRemoteJWKSet` and the options the TS
//! passes) step by step, including which failures are the token's fault (401 `invalid_token`) and which are ours
//! (503 `supabase_unavailable`), so the HTTP answers match:
//!
//! * alg allow-list: `ES256` / `RS256` / `EdDSA` against the project JWKS, `HS256` only with
//!   `OC_SUPABASE_JWT_SECRET` (or, without it, decoded and left to the online check); anything else (`none`,
//!   `HS384`, `PS256`, …) is refused before any key is looked at, so no algorithm confusion is possible: a JWKS
//!   key only ever verifies its own key type, and the HMAC secret is never a JWKS key;
//! * JWKS: one cache per project URL, refetched after 10 minutes, and on an unknown `kid` at most every 30 s
//!   (jose's `cacheMaxAge` / `cooldownDuration`), concurrent reloads share one fetch, redirects are not followed;
//! * claims: `iss` (= `OC_SUPABASE_JWT_ISSUER` or `<url>/auth/v1`), `aud` contains `authenticated`, `exp` / `nbf`
//!   with 30 s tolerance, then `role`, `sub` (a UUID), `session_id`, `aal`, `is_anonymous`;
//! * every token is also checked online (`GET /auth/v1/user`): the session must still exist, and that answer is
//!   the only source of email confirmation and MFA factors.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::http::jsjson;
use crate::http::reply::{self, Reply};
use crate::http::req::Req;
use crate::state::AppState;

const TIMEOUT: Duration = Duration::from_millis(5_000);
const TOLERANCE_S: f64 = 30.0;
const CACHE_MAX_AGE_MS: i64 = 600_000;
const COOLDOWN_MS: i64 = 30_000;

/// `SupabaseAuthError`: answered as `{ error: code, message }` with `status`.
#[derive(Debug, Clone)]
pub struct SbError {
    pub code: &'static str,
    pub status: u16,
    pub message: &'static str,
}

impl SbError {
    /// `sendSupabaseError(reply, e)`.
    pub fn reply(&self) -> Reply {
        reply::json(self.status, json!({ "error": self.code, "message": self.message }))
    }
}

pub const INVALID_MSG: &str = "That sign-in is not valid. Sign in again.";
const EXPIRED_MSG: &str = "That sign-in has expired. Sign in again.";
pub const UNAVAILABLE_MSG: &str = "The sign-in service is not reachable right now. Try again in a moment.";
const ANON_MSG: &str = "Guest sessions can’t be used here. Sign in with an email address.";

fn invalid() -> SbError {
    SbError { code: "invalid_token", status: 401, message: INVALID_MSG }
}
fn expired() -> SbError {
    SbError { code: "invalid_token", status: 401, message: EXPIRED_MSG }
}
fn unavailable() -> SbError {
    SbError { code: "supabase_unavailable", status: 503, message: UNAVAILABLE_MSG }
}
fn anonymous() -> SbError {
    SbError { code: "anonymous_user", status: 403, message: ANON_MSG }
}

/// `supabaseAuthEnabled()`.
pub fn enabled(s: &AppState) -> bool {
    s.cfg.supabase_auth_enabled()
}

fn base_url(s: &AppState) -> String {
    s.cfg.supabase.url.trim_end_matches('/').to_owned()
}

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

/// JS truthiness of a JSON value (`undefined` = `None`).
pub fn js_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// A JS number value of a claim (`typeof v === 'number'`).
fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|_| v.is_some_and(Value::is_number))
}

// ---------------------------------------------------------------- verified claims and the Supabase user

/// The checked claims of an access token (`SupabaseClaims`).
#[derive(Debug, Clone)]
pub struct Claims {
    pub sub: String,
    pub email: Option<Value>,
    pub aal: String,
    /// `amr` entries with a string method and a number timestamp.
    pub amr: Vec<(String, f64)>,
    pub exp: f64,
    pub iat: f64,
}

impl Claims {
    /// `lastAuthAt(c)`: the latest `amr` timestamp, in seconds.
    pub fn last_auth_at(&self) -> f64 {
        self.amr.iter().fold(0.0, |m, (_, t)| f64::max(m, *t))
    }
}

/// The `GET /auth/v1/user` answer (`SupabaseUser`), with `user_metadata` / `app_metadata` defaulted to `{}`.
#[derive(Debug, Clone)]
pub struct SbUser(pub Value);

impl SbUser {
    pub fn id(&self) -> &str {
        self.0.get("id").and_then(Value::as_str).unwrap_or("")
    }
    /// `user.email` when not null/undefined.
    pub fn email(&self) -> Option<&Value> {
        self.0.get("email").filter(|v| !v.is_null())
    }
    /// `Boolean(user.email_confirmed_at)`.
    pub fn email_confirmed(&self) -> bool {
        js_truthy(self.0.get("email_confirmed_at"))
    }
    pub fn user_metadata(&self) -> &Value {
        &self.0["user_metadata"]
    }
    /// `user.app_metadata.provider ?? 'email'` (as the TS would stringify a non-string into JSON/audit: kept as is).
    pub fn provider(&self) -> Value {
        match self.0["app_metadata"].get("provider") {
            None | Some(Value::Null) => json!("email"),
            Some(v) => v.clone(),
        }
    }
    /// `hasVerifiedFactor(u)`.
    pub fn has_verified_factor(&self) -> bool {
        self.0.get("factors").and_then(Value::as_array).is_some_and(|f| f.iter().any(|x| x.get("status").and_then(Value::as_str) == Some("verified")))
    }
}

// ---------------------------------------------------------------- JWKS (jose createRemoteJWKSet)

#[derive(Default)]
struct JwksEntry {
    keys: Option<Vec<Value>>,
    at: i64,
}

/// `jwksByUrl`: one remote JWKS per project URL. The async mutex is jose's shared `pendingFetch`.
#[derive(Default)]
struct JwksCache(Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<JwksEntry>>>>);

/// A failure while resolving the key, in jose's terms.
enum KeyErr {
    /// `JWKSNoMatchingKey` (may trigger a reload).
    NoMatch,
    /// A `JOSEError` about the token or the key set (→ 401).
    Invalid,
    /// A fetch failure or a non-JOSE error (→ 503).
    Unavailable,
}

/// One HTTP client that does not follow redirects (jose fetches the JWKS with `redirect: 'manual'`).
#[derive(Default)]
struct NoRedirect(std::sync::OnceLock<reqwest::Client>);

fn no_redirect(s: &AppState) -> reqwest::Client {
    s.local::<NoRedirect>().0.get_or_init(|| reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("http client")).clone()
}

async fn fetch_jwks(s: &AppState, url: &str) -> Result<Vec<Value>, KeyErr> {
    let res = no_redirect(s)
        .get(url)
        .header("accept", "application/json, application/jwk-set+json")
        .header("user-agent", "jose/v6.2.12")
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|_| KeyErr::Unavailable)?;
    if res.status().as_u16() != 200 {
        return Err(KeyErr::Unavailable); // JOSEError (ERR_JOSE_GENERIC)
    }
    let bytes = res.bytes().await.map_err(|_| KeyErr::Unavailable)?;
    let json = std::str::from_utf8(&bytes).ok().and_then(|t| jsjson::parse(t).ok()).ok_or(KeyErr::Unavailable)?;
    // createLocalJWKSet: isJwkSet (an object whose `keys` is an array of objects), else JWKSInvalid.
    match json.get("keys").and_then(Value::as_array) {
        Some(keys) if json.is_object() && keys.iter().all(Value::is_object) => Ok(keys.clone()),
        _ => Err(KeyErr::Invalid),
    }
}

/// `isUsableJWK(jwk, entry, alg, kid)`.
fn usable(jwk: &Value, kty: &str, crv: Option<&str>, alg: &str, kid: Option<&Value>) -> bool {
    let ext_ok = matches!(jwk.get("ext"), None | Some(Value::Bool(_)));
    let ops_ok = match jwk.get("key_ops") {
        None => true,
        Some(Value::Array(ops)) => {
            let strs: Vec<&str> = ops.iter().filter_map(Value::as_str).collect();
            strs.len() == ops.len() && strs.iter().enumerate().all(|(i, o)| strs.iter().position(|x| x == o) == Some(i)) && strs.contains(&"verify")
        }
        Some(_) => false,
    };
    let kty_ok = jwk.get("kty").and_then(Value::as_str) == Some(kty);
    let kid_ok = match kid {
        None => true,
        Some(Value::String(k)) => jwk.get("kid").and_then(Value::as_str) == Some(k.as_str()),
        Some(_) => false,
    };
    let alg_ok = match jwk.get("alg") {
        None => true,
        Some(a) => a.as_str() == Some(alg),
    };
    let use_ok = match jwk.get("use") {
        None => true,
        Some(u) => u.as_str() == Some("sig"),
    };
    let crv_ok = crv.is_none_or(|c| jwk.get("crv").and_then(Value::as_str) == Some(c));
    ext_ok && ops_ok && kty_ok && kid_ok && alg_ok && use_ok && crv_ok
}

/// The local key set lookup: exactly one usable key, or NoMatch / Invalid (several matches).
fn select(keys: &[Value], alg: &str, kid: Option<&Value>) -> Result<Value, KeyErr> {
    let (kty, crv) = match alg {
        "ES256" => ("EC", Some("P-256")),
        "RS256" => ("RSA", None),
        _ => ("OKP", Some("Ed25519")),
    };
    let c: Vec<&Value> = keys.iter().filter(|k| usable(k, kty, crv, alg, kid)).collect();
    match c.len() {
        0 => Err(KeyErr::NoMatch),
        1 => Ok(c[0].clone()),
        _ => Err(KeyErr::Invalid), // JWKSMultipleMatchingKeys
    }
}

fn fresh(at: i64, now: i64, dur: i64) -> bool {
    now < at + dur
}

/// The remote JWKS function: reload when there is no key set or it is older than 10 minutes; on an unknown key,
/// reload once unless the last fetch is less than 30 s old.
async fn jwks_key(s: &AppState, alg: &str, kid: Option<&Value>) -> Result<Value, KeyErr> {
    let url = format!("{}/auth/v1/.well-known/jwks.json", base_url(s));
    let entry = {
        let cache = s.local::<JwksCache>();
        let mut m = cache.0.lock().expect("jwks cache");
        m.entry(url.clone()).or_default().clone()
    };
    let mut e = entry.lock().await;
    let reload = |e: &mut JwksEntry, keys: Vec<Value>| {
        e.keys = Some(keys);
        e.at = s.now_ms();
    };
    if e.keys.is_none() || !fresh(e.at, s.now_ms(), CACHE_MAX_AGE_MS) {
        let keys = fetch_jwks(s, &url).await?;
        reload(&mut e, keys);
    }
    match select(e.keys.as_deref().unwrap_or(&[]), alg, kid) {
        Err(KeyErr::NoMatch) if !fresh(e.at, s.now_ms(), COOLDOWN_MS) => {
            let keys = fetch_jwks(s, &url).await?;
            reload(&mut e, keys);
            select(e.keys.as_deref().unwrap_or(&[]), alg, kid)
        }
        r => r,
    }
}

/// `Buffer.from(s, 'base64url')`-style lenient decoding of JWK members.
fn b64_lenient(s: &str) -> Vec<u8> {
    let clean: String = s.chars().take_while(|c| *c != '=').filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '/')).map(|c| match c {
        '+' => '-',
        '/' => '_',
        c => c,
    }).collect();
    let usable = if clean.len() % 4 == 1 { &clean[..clean.len() - 1] } else { &clean[..] };
    base64::engine::GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        base64::engine::GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true).with_decode_padding_mode(base64::engine::DecodePaddingMode::RequireNone),
    )
    .decode(usable)
    .unwrap_or_default()
}

fn jwk_bytes(jwk: &Value, k: &str) -> Option<Vec<u8>> {
    jwk.get(k).and_then(Value::as_str).map(b64_lenient)
}

/// Import a public JWK and verify `sig` over `data`. Err(Unavailable) where WebCrypto's importKey (or jose's
/// TypeError on a short RSA modulus) would throw a non-JOSE error, Err(Invalid) for a private key in the set.
fn verify_with_jwk(jwk: &Value, alg: &str, data: &[u8], sig: &[u8]) -> Result<bool, KeyErr> {
    use ring::signature::{self, UnparsedPublicKey};
    if js_truthy(jwk.get("d")) || js_truthy(jwk.get("priv")) {
        return Err(KeyErr::Invalid); // "JSON Web Key Set members must be public keys"
    }
    match alg {
        "RS256" => {
            let (Some(n), Some(e)) = (jwk_bytes(jwk, "n"), jwk_bytes(jwk, "e")) else { return Err(KeyErr::Unavailable) };
            let n_trim: Vec<u8> = n.iter().copied().skip_while(|b| *b == 0).collect();
            if n_trim.is_empty() || e.is_empty() {
                return Err(KeyErr::Unavailable);
            }
            let bits = (n_trim.len() - 1) * 8 + (8 - n_trim[0].leading_zeros() as usize);
            if bits < 2048 {
                return Err(KeyErr::Unavailable); // checkModulusLength: TypeError
            }
            let key = signature::RsaPublicKeyComponents { n: &n_trim, e: &e };
            Ok(key.verify(&signature::RSA_PKCS1_2048_8192_SHA256, data, sig).is_ok())
        }
        "ES256" => {
            let (Some(x), Some(y)) = (jwk_bytes(jwk, "x"), jwk_bytes(jwk, "y")) else { return Err(KeyErr::Unavailable) };
            if x.len() != 32 || y.len() != 32 {
                return Err(KeyErr::Unavailable);
            }
            let point = [&[4u8][..], &x, &y].concat();
            Ok(UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, point).verify(data, sig).is_ok())
        }
        _ => {
            let Some(x) = jwk_bytes(jwk, "x").filter(|x| x.len() == 32) else { return Err(KeyErr::Unavailable) };
            Ok(UnparsedPublicKey::new(&signature::ED25519, x).verify(data, sig).is_ok())
        }
    }
}

// ---------------------------------------------------------------- token verification

/// `parseJoseHeader` / the payload decoding: strict UTF-8, JSON, a plain object.
fn decode_object(seg: &str) -> Option<Map<String, Value>> {
    let bytes = URL_SAFE_NO_PAD.decode(seg).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    match jsjson::parse(&text).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

/// `validateClaimsSet` with `{ issuer, audience: 'authenticated', clockTolerance: 30 }`.
fn validate_claims(p: &Map<String, Value>, issuer: &str, now: f64) -> Result<(), SbError> {
    if !p.contains_key("iss") || !p.contains_key("aud") {
        return Err(invalid());
    }
    if p.get("iss").and_then(Value::as_str) != Some(issuer) {
        return Err(invalid());
    }
    let aud_ok = match p.get("aud") {
        Some(Value::String(a)) => a == "authenticated",
        Some(Value::Array(a)) => a.iter().any(|x| x.as_str() == Some("authenticated")),
        _ => false,
    };
    if !aud_ok {
        return Err(invalid());
    }
    for k in ["iat", "nbf"] {
        if p.get(k).is_some_and(|v| !v.is_number()) {
            return Err(invalid());
        }
    }
    if let Some(nbf) = num(p.get("nbf"))
        && nbf > now + TOLERANCE_S
    {
        return Err(invalid());
    }
    if p.get("exp").is_some_and(|v| !v.is_number()) {
        return Err(invalid());
    }
    if let Some(exp) = num(p.get("exp"))
        && exp <= now - TOLERANCE_S
    {
        return Err(expired());
    }
    Ok(())
}

/// `checkUnverifiedClaims(p)`: the checks jwtVerify does, for HS256 without the legacy secret.
fn check_unverified(p: &Map<String, Value>, issuer: &str, now: f64) -> Result<(), SbError> {
    let aud_ok = match p.get("aud") {
        Some(Value::Array(a)) => a.iter().any(|x| x.as_str() == Some("authenticated")),
        Some(Value::String(a)) => a == "authenticated",
        _ => false,
    };
    if p.get("iss").and_then(Value::as_str) != Some(issuer) || !aud_ok {
        return Err(invalid());
    }
    match num(p.get("exp")) {
        Some(exp) if exp >= now - TOLERANCE_S => {}
        _ => return Err(expired()),
    }
    if let Some(nbf) = num(p.get("nbf"))
        && nbf > now + TOLERANCE_S
    {
        return Err(invalid());
    }
    Ok(())
}

/// The protected header's `crit` / `b64` rules (validateCrit, validateB64): only `b64` is understood, and an
/// unencoded payload is never a JWT.
fn crit_ok(h: &Map<String, Value>) -> bool {
    match h.get("crit") {
        None => true,
        Some(Value::Array(c)) if !c.is_empty() => {
            let names: Vec<&str> = c.iter().filter_map(Value::as_str).collect();
            names.len() == c.len() && names.iter().all(|n| *n == "b64") && h.get("b64") == Some(&Value::Bool(true))
        }
        Some(_) => false,
    }
}

/// `verifySignature(token)`: the header's `alg` picks the verification; anything off the allow-lists is invalid.
async fn verify_signature(s: &AppState, token: &str) -> Result<Map<String, Value>, SbError> {
    let parts: Vec<&str> = token.split('.').collect();
    let header = decode_object(parts[0]).ok_or_else(invalid)?;
    let alg = header.get("alg").and_then(Value::as_str).unwrap_or("").to_owned();
    let issuer = s.cfg.supabase_issuer();
    let now = (s.now_ms() as f64 / 1000.0).floor();
    let secret = &s.cfg.supabase.jwt_secret;
    let data = format!("{}.{}", parts[0], parts[1]);
    let sig = URL_SAFE_NO_PAD.decode(parts[2]).map_err(|_| invalid())?;
    match alg.as_str() {
        "ES256" | "RS256" | "EdDSA" => {
            if !crit_ok(&header) {
                return Err(invalid());
            }
            let jwk = match jwks_key(s, &alg, header.get("kid")).await {
                Ok(k) => k,
                Err(KeyErr::Unavailable) => return Err(unavailable()),
                Err(_) => return Err(invalid()),
            };
            match verify_with_jwk(&jwk, &alg, data.as_bytes(), &sig) {
                Ok(true) => {}
                Ok(false) | Err(KeyErr::Invalid) | Err(KeyErr::NoMatch) => return Err(invalid()),
                Err(KeyErr::Unavailable) => return Err(unavailable()),
            }
            let p = decode_object(parts[1]).ok_or_else(invalid)?;
            validate_claims(&p, &issuer, now)?;
            Ok(p)
        }
        "HS256" if !secret.is_empty() => {
            if !crit_ok(&header) {
                return Err(invalid());
            }
            let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
            ring::hmac::verify(&key, data.as_bytes(), &sig).map_err(|_| invalid())?;
            let p = decode_object(parts[1]).ok_or_else(invalid)?;
            validate_claims(&p, &issuer, now)?;
            Ok(p)
        }
        "HS256" => {
            // Legacy project without OC_SUPABASE_JWT_SECRET: accepted only because the online check always follows.
            warn_once(s);
            let p = decode_object(parts[1]).ok_or_else(invalid)?;
            check_unverified(&p, &issuer, now)?;
            Ok(p)
        }
        _ => Err(invalid()), // none, HS384/512, anything not on the allow-lists
    }
}

#[derive(Default)]
struct Warned(std::sync::atomic::AtomicBool);

fn warn_once(s: &AppState) {
    if !s.local::<Warned>().0.swap(true, std::sync::atomic::Ordering::SeqCst) {
        tracing::warn!("[supabase] HS256 access tokens are checked online only. Set OC_SUPABASE_JWT_SECRET, or migrate the project to JWT signing keys.");
    }
}

/// `checkClaims(p)`.
fn check_claims(p: &Map<String, Value>) -> Result<Claims, SbError> {
    if p.get("role").and_then(Value::as_str) != Some("authenticated") {
        return Err(invalid());
    }
    let sub = p.get("sub").and_then(Value::as_str).filter(|s| is_uuid(s)).ok_or_else(invalid)?.to_owned();
    if p.get("session_id").and_then(Value::as_str).is_none_or(str::is_empty) {
        return Err(invalid());
    }
    let exp = num(p.get("exp")).ok_or_else(invalid)?;
    let aal = match p.get("aal").and_then(Value::as_str) {
        Some(a @ ("aal1" | "aal2")) => a.to_owned(),
        _ => return Err(invalid()),
    };
    if p.get("is_anonymous") == Some(&Value::Bool(true)) {
        return Err(anonymous());
    }
    let amr = p
        .get("amr")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| match (x.get("method").and_then(Value::as_str), num(x.get("timestamp"))) {
                    (Some(m), Some(t)) => Some((m.to_owned(), t)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Claims { sub, email: p.get("email").cloned(), aal, amr, exp, iat: num(p.get("iat")).unwrap_or(0.0) })
}

/// `fetchUser(token)`: GET /auth/v1/user.
async fn fetch_user(s: &AppState, token: &str) -> Result<SbUser, SbError> {
    let res = s
        .http
        .get(format!("{}/auth/v1/user", base_url(s)))
        .header("apikey", &s.cfg.supabase.publishable_key)
        .header("authorization", format!("Bearer {token}"))
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|_| unavailable())?;
    let status = res.status().as_u16();
    if status >= 500 {
        return Err(unavailable());
    }
    let body = res.bytes().await.ok().and_then(|b| String::from_utf8(b.to_vec()).ok()).and_then(|t| jsjson::parse(&t).ok());
    if status != 200 {
        let b = body.unwrap_or(Value::Null);
        let code = match b.get("error_code") {
            Some(Value::String(c)) => Some(Value::String(c.clone())),
            _ => b.get("code").cloned(),
        };
        if code == Some(json!("session_not_found")) {
            return Err(SbError { code: "session_ended", status: 401, message: "That session has ended. Sign in again." });
        }
        return Err(invalid());
    }
    let Some(mut u) = body.filter(|u| u.get("id").is_some_and(Value::is_string)) else { return Err(unavailable()) };
    if let Some(m) = u.as_object_mut() {
        for k in ["user_metadata", "app_metadata"] {
            if m.get(k).is_none_or(Value::is_null) {
                m.insert(k.into(), json!({}));
            }
        }
    }
    Ok(SbUser(u))
}

/// Each segment must be canonical base64url (one spelling per token).
fn canonical_segments(token: &str) -> bool {
    token.split('.').all(|seg| URL_SAFE_NO_PAD.decode(seg).is_ok_and(|b| URL_SAFE_NO_PAD.encode(b) == seg))
}

fn token_shape(t: &str) -> bool {
    let parts: Vec<&str> = t.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'))
}

/// `authenticateSupabase(accessToken)`: local signature + claim checks, then the online check.
pub async fn authenticate(s: &AppState, token: &str) -> Result<(Claims, SbUser), SbError> {
    if !enabled(s) {
        return Err(SbError { code: "supabase_off", status: 404, message: "Supabase sign-in is not configured on this server." });
    }
    if crate::js::len(token) > 8192 || !token_shape(token) || !canonical_segments(token) {
        return Err(invalid());
    }
    let claims = check_claims(&verify_signature(s, token).await?)?;
    let user = fetch_user(s, token).await?;
    if user.id() != claims.sub {
        return Err(invalid());
    }
    if user.0.get("is_anonymous") == Some(&Value::Bool(true)) {
        return Err(anonymous());
    }
    Ok((claims, user))
}

/// `accessTokenKey(token)`: sha256 of the signed part (`header.payload`), the single-use key (plan 12 §B8).
pub fn access_token_key(token: &str) -> String {
    let signed = token.rfind('.').map_or("", |i| &token[..i]);
    hex::encode(Sha256::digest(signed.as_bytes()))
}

// ---------------------------------------------------------------- public settings

fn label(id: &str) -> String {
    let known = match id {
        "google" => "Google",
        "apple" => "Apple",
        "azure" => "Microsoft",
        "github" => "GitHub",
        "gitlab" => "GitLab",
        "bitbucket" => "Bitbucket",
        "discord" => "Discord",
        "facebook" => "Facebook",
        "figma" => "Figma",
        "linkedin" | "linkedin_oidc" => "LinkedIn",
        "slack" | "slack_oidc" => "Slack",
        "twitter" => "X",
        "notion" => "Notion",
        "spotify" => "Spotify",
        "twitch" => "Twitch",
        "zoom" => "Zoom",
        "kakao" => "Kakao",
        "keycloak" => "Keycloak",
        "workos" => "WorkOS",
        "snapchat" => "Snapchat",
        "fly" => "Fly.io",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    let mut c = id.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// `parseSupabaseSettings(raw)`.
pub fn parse_settings(raw: &Value) -> Value {
    let external: Vec<(String, Value)> = match raw.get("external") {
        Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Some(Value::Array(a)) => a.iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect(),
        _ => vec![],
    };
    let providers: Vec<Value> = external
        .iter()
        .filter(|(id, on)| *on == Value::Bool(true) && !["email", "phone", "anonymous_users"].contains(&id.as_str()))
        .map(|(id, _)| json!({ "id": id, "label": label(id) }))
        .collect();
    let email = external.iter().find(|(k, _)| k == "email").map(|(_, v)| v) != Some(&Value::Bool(false));
    json!({
        "providers": providers,
        "email": email,
        "signup": raw.get("disable_signup") != Some(&Value::Bool(true)),
        "autoconfirm": raw.get("mailer_autoconfirm") == Some(&Value::Bool(true)),
    })
}

fn fallback_settings() -> Value {
    json!({ "providers": [], "email": true, "signup": true, "autoconfirm": false })
}

#[derive(Default)]
struct SettingsCache(tokio::sync::Mutex<SettingsState>);

#[derive(Default)]
struct SettingsState {
    /// (url, value, read, until)
    cache: Option<(String, Value, bool, i64)>,
    last_good: Option<(String, Value)>,
}

/// `loadSettings()`: cached 5 minutes; on failure the last good value (or a safe default) for 30 s.
async fn load_settings(s: &AppState) -> (Value, bool) {
    let url = base_url(s);
    let cache = s.local::<SettingsCache>();
    let mut st = cache.0.lock().await;
    if let Some((u, v, read, until)) = &st.cache
        && *u == url
        && s.now_ms() < *until
    {
        return (v.clone(), *read);
    }
    let fetched = async {
        let res = s.http.get(format!("{url}/auth/v1/settings")).header("apikey", &s.cfg.supabase.publishable_key).timeout(TIMEOUT).send().await.ok()?;
        if !res.status().is_success() {
            return None;
        }
        let b = res.bytes().await.ok()?;
        jsjson::parse(std::str::from_utf8(&b).ok()?).ok()
    }
    .await;
    match fetched {
        Some(raw) => {
            let v = parse_settings(&raw);
            st.last_good = Some((url.clone(), v.clone()));
            st.cache = Some((url, v.clone(), true, s.now_ms() + 5 * 60_000));
            (v, true)
        }
        None => {
            let v = match &st.last_good {
                Some((u, v)) if *u == url => v.clone(),
                _ => fallback_settings(),
            };
            st.cache = Some((url, v.clone(), false, s.now_ms() + 30_000));
            (v, false)
        }
    }
}

/// `supabaseSettings()`.
pub async fn settings(s: &AppState) -> Value {
    load_settings(s).await.0
}

/// `emailConfirmationTrust()`: 'trusted' | 'autoconfirm' | 'unknown'.
pub async fn email_confirmation_trust(s: &AppState) -> &'static str {
    let (v, read) = load_settings(s).await;
    if !read {
        "unknown"
    } else if v["autoconfirm"] == Value::Bool(true) {
        "autoconfirm"
    } else {
        "trusted"
    }
}

// ---------------------------------------------------------------- calls to Supabase Auth

/// `supabaseLogout(accessToken, scope)`.
pub async fn logout(s: &AppState, token: &str, scope: &str) -> Result<(), SbError> {
    let res = s
        .http
        .post(format!("{}/auth/v1/logout?scope={scope}", base_url(s)))
        .header("apikey", &s.cfg.supabase.publishable_key)
        .header("authorization", format!("Bearer {token}"))
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|_| unavailable())?;
    let st = res.status().as_u16();
    if st >= 500 {
        return Err(unavailable());
    }
    if !(200..300).contains(&st) {
        return Err(invalid());
    }
    Ok(())
}

#[derive(Default)]
struct WarnedNoSecret(std::sync::atomic::AtomicBool);

/// `supabaseAdminDelete(id)`: 'deleted' | 'not_found' | 'skipped' | 'failed'.
pub async fn admin_delete(s: &AppState, id: &str) -> &'static str {
    let key = &s.cfg.supabase.secret_key;
    if key.is_empty() {
        if !s.local::<WarnedNoSecret>().0.swap(true, std::sync::atomic::Ordering::SeqCst) {
            tracing::warn!("[supabase] OC_SUPABASE_SECRET_KEY is not set: deleted accounts stay in Supabase Auth.");
        }
        return "skipped";
    }
    if !is_uuid(id) {
        return "failed";
    }
    let r = s
        .http
        .delete(format!("{}/auth/v1/admin/users/{id}", base_url(s)))
        .header("apikey", key)
        .header("authorization", format!("Bearer {key}"))
        .header("content-type", "application/json")
        .body(r#"{"should_soft_delete":false}"#)
        .timeout(TIMEOUT)
        .send()
        .await;
    match r {
        Ok(r) if r.status().is_success() => "deleted",
        Ok(r) if r.status().as_u16() == 404 => "not_found",
        _ => "failed",
    }
}

/// `supabaseAdminEmail(id)`: the linked user's current email at Supabase, or `None` (no secret key, no answer
/// within 2 s, not ok).
pub async fn admin_email(s: &AppState, id: &str) -> Option<String> {
    let key = &s.cfg.supabase.secret_key;
    if key.is_empty() || !is_uuid(id) {
        return None;
    }
    let r = s
        .http
        .get(format!("{}/auth/v1/admin/users/{id}", base_url(s)))
        .header("apikey", key)
        .header("authorization", format!("Bearer {key}"))
        .timeout(Duration::from_millis(2_000))
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v = jsjson::parse(std::str::from_utf8(&r.bytes().await.ok()?).ok()?).ok()?;
    v.get("email").and_then(Value::as_str).filter(|e| !e.is_empty()).map(str::to_lowercase)
}

// ---------------------------------------------------------------- route guards

/// The reply of `builtinOnly` (plan 12 §C3): built-in sign-in routes that would bypass Supabase Auth answer 404.
pub fn builtin_reply() -> Reply {
    reply::json(404, json!({ "error": "supabase_auth", "message": "Sign-in is handled by Supabase on this server." }))
}

/// `{ onRequest: builtinOnly }`: in Supabase mode the 404 comes before the route's rate limit and body parsing
/// (the hook runs ahead of @fastify/rate-limit's own `onRequest`), so a guarded handler takes the raw request
/// and extracts [`Req`] through this.
pub async fn builtin_req(s: &AppState, raw: axum::extract::Request) -> Result<Req, crate::http::error::ApiError> {
    use axum::extract::FromRequest as _;
    if enabled(s) {
        return Err(builtin_reply().into());
    }
    Req::from_request(raw, s).await
}

/// `assertSameOrigin(req, reply)`: a present Origin must be the app's, a present Sec-Fetch-Site must be
/// same-origin or none. `Err` is the 403 `bad_origin` reply.
pub fn assert_same_origin(s: &AppState, req: &Req) -> Result<(), Reply> {
    let origin = req.headers.get("origin").map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    let site = req.headers.get("sec-fetch-site").map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    let ok_origin = origin.as_deref().is_none_or(|o| o == s.cfg.web_origin.trim_end_matches('/'));
    let ok_site = site.as_deref().is_none_or(|v| v == "same-origin" || v == "none");
    if ok_origin && ok_site {
        return Ok(());
    }
    Err(reply::json(403, json!({ "error": "bad_origin", "message": "This request came from another site." })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_key_and_shape() {
        assert_eq!(access_token_key("a.b.c"), hex::encode(Sha256::digest(b"a.b")));
        assert!(token_shape("ab.cd.ef"));
        assert!(!token_shape("ab.cd"));
        assert!(!token_shape("ab..ef"));
        // "QQ" is canonical for 'A'; "QR" decodes to the same byte with non-zero trailing bits.
        assert!(canonical_segments("QQ.QQ.QQ"));
        assert!(!canonical_segments("QR.QQ.QQ"));
        assert!(!canonical_segments("Q.QQ.QQ"));
    }

    #[test]
    fn settings_parse() {
        let v = parse_settings(&json!({ "external": { "email": true, "google": true, "github": false, "phone": true, "custom": true }, "mailer_autoconfirm": true }));
        assert_eq!(v, json!({ "providers": [{ "id": "google", "label": "Google" }, { "id": "custom", "label": "Custom" }], "email": true, "signup": true, "autoconfirm": true }));
    }
}
