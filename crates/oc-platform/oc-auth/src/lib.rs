//! Short-lived service tickets (ADR-0015): 5-minute HMAC tokens carrying `{sub, aud, design, role}`.
//!
//! Wire format (identical to `packages/auth`): `base64url(JSON) "." base64url(HMAC-SHA256(secret, base64url(JSON)))`,
//! no padding. JSON is `{...ticket, iat, exp}` where `iat` is milliseconds and `exp` is seconds since the epoch.
//! The `*_with` variants take the secret explicitly; the plain ones use [`oc_config::config`]`().secret`.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    View,
    Comment,
    Edit,
    Owner,
}

impl Role {
    fn rank(self) -> u8 {
        match self {
            Role::View => 1,
            Role::Comment => 2,
            Role::Edit => 3,
            Role::Owner => 4,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Role::View => "view",
            Role::Comment => "comment",
            Role::Edit => "edit",
            Role::Owner => "owner",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    Collab,
    Render,
}

/// Ticket claims as passed to [`sign_ticket`] (TS `Omit<Ticket, 'exp' | 'iat'>`). Field order is the JSON order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketClaims {
    pub sub: String,
    pub name: String,
    pub color: String,
    pub aud: Audience,
    pub design: String,
    pub role: Role,
    /// Held to brand-template locks (A8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand_locked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    #[serde(flatten)]
    pub claims: TicketClaims,
    /// Issued-at, milliseconds.
    #[serde(deserialize_with = "js_int")]
    pub iat: i64,
    /// Expiry, seconds.
    #[serde(deserialize_with = "js_int")]
    pub exp: i64,
}

/// Accept any JSON number (JS has only doubles); fractional values are truncated.
fn js_int<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(f64::deserialize(d)? as i64)
}

/// `atLeast(role, min)`.
pub fn at_least(role: Option<Role>, min: Role) -> bool {
    role.is_some_and(|r| r.rank() >= min.rank())
}

/// `maxRole(a, b)`.
pub fn max_role(a: Option<Role>, b: Option<Role>) -> Option<Role> {
    match (a, b) {
        (None, b) => b,
        (a, None) => a,
        (Some(a), Some(b)) => Some(if a.rank() >= b.rank() { a } else { b }),
    }
}

fn mac(secret: &str, data: &[u8]) -> [u8; 32] {
    let mut m = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts any key length");
    m.update(data);
    m.finalize().into_bytes().into()
}

/// `signTicket(t, ttlSec)` with an explicit secret and clock (`now_ms` = `Date.now()`).
pub fn sign_ticket_at(secret: &str, claims: &TicketClaims, ttl_sec: i64, now_ms: i64) -> String {
    let t = Ticket { claims: claims.clone(), iat: now_ms, exp: now_ms.div_euclid(1000) + ttl_sec };
    let json = serde_json::to_string(&t).expect("ticket serializes");
    let body = URL_SAFE_NO_PAD.encode(json.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac(secret, body.as_bytes()));
    format!("{body}.{sig}")
}

pub fn sign_ticket_with(secret: &str, claims: &TicketClaims, ttl_sec: i64) -> String {
    sign_ticket_at(secret, claims, ttl_sec, oc_js::now_ms())
}

/// `signTicket(t, ttlSec = 300)` using the configured secret.
pub fn sign_ticket(claims: &TicketClaims, ttl_sec: Option<i64>) -> String {
    sign_ticket_with(&oc_config::config().secret, claims, ttl_sec.unwrap_or(300))
}

/// `verifyTicket(token, aud)` with an explicit secret and clock.
pub fn verify_ticket_at(secret: &str, token: &str, aud: Audience, now_ms: i64) -> Option<Ticket> {
    let mut parts = token.split('.');
    let body = parts.next().filter(|s| !s.is_empty())?;
    let sig = parts.next().filter(|s| !s.is_empty())?;
    let expect = mac(secret, body.as_bytes());
    let got = oc_js::node_base64(sig);
    if got.len() != expect.len() || !bool::from(got.ct_eq(&expect)) {
        return None;
    }
    let json = String::from_utf8_lossy(&oc_js::node_base64(body)).into_owned();
    let t: Ticket = serde_json::from_str(&json).ok()?;
    if t.claims.aud != aud || (t.exp as f64) < now_ms as f64 / 1000.0 {
        return None;
    }
    Some(t)
}

pub fn verify_ticket_with(secret: &str, token: &str, aud: Audience) -> Option<Ticket> {
    verify_ticket_at(secret, token, aud, oc_js::now_ms())
}

pub fn verify_ticket(token: &str, aud: Audience) -> Option<Ticket> {
    verify_ticket_with(&oc_config::config().secret, token, aud)
}

/// Internal service-to-service header (render ← api/collab).
pub const INTERNAL_HEADER: &str = "x-oc-internal";

pub fn internal_token_with(secret: &str) -> String {
    URL_SAFE_NO_PAD.encode(mac(secret, b"internal"))
}

pub fn internal_token() -> String {
    internal_token_with(&oc_config::config().secret)
}

/// `isInternal(value)`: constant-time comparison against the internal token. For a multi-valued header
/// pass the first value.
pub fn is_internal_with(secret: &str, value: Option<&str>) -> bool {
    let Some(v) = value.filter(|v| !v.is_empty()) else { return false };
    let want = internal_token_with(secret);
    v.len() == want.len() && bool::from(v.as_bytes().ct_eq(want.as_bytes()))
}

pub fn is_internal(value: Option<&str>) -> bool {
    is_internal_with(&oc_config::config().secret, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims() -> TicketClaims {
        TicketClaims { sub: "u1".into(), name: "Ann".into(), color: "#f00".into(), aud: Audience::Collab, design: "d1".into(), role: Role::Edit, brand_locked: None }
    }

    #[test]
    fn roundtrip_and_audience() {
        let tok = sign_ticket_at("s", &claims(), 300, 1_700_000_000_123);
        let t = verify_ticket_at("s", &tok, Audience::Collab, 1_700_000_000_500).unwrap();
        assert_eq!(t.iat, 1_700_000_000_123);
        assert_eq!(t.exp, 1_700_000_300);
        assert!(verify_ticket_at("s", &tok, Audience::Render, 1_700_000_000_500).is_none());
        assert!(verify_ticket_at("other", &tok, Audience::Collab, 1_700_000_000_500).is_none());
        assert!(verify_ticket_at("s", &tok, Audience::Collab, 1_700_000_301_000).is_none());
        assert!(verify_ticket_at("s", "garbage", Audience::Collab, 0).is_none());
    }

    #[test]
    fn roles() {
        assert!(at_least(Some(Role::Owner), Role::Edit));
        assert!(!at_least(Some(Role::Comment), Role::Edit));
        assert!(!at_least(None, Role::View));
        assert_eq!(max_role(Some(Role::View), Some(Role::Comment)), Some(Role::Comment));
        assert_eq!(max_role(None, Some(Role::View)), Some(Role::View));
    }
}
