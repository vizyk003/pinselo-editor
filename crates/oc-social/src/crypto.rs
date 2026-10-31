//! Token encryption at rest and signed public media links (port of services/api/src/social/crypto.ts).
//!
//! AES-256-GCM with a key derived from OC_SECRET (HKDF-SHA256, salt "opencanvas", info "social-tokens/v1"),
//! a random 96-bit IV per value and the 128-bit tag. Format: `"v1." + base64url(iv | tag | ciphertext)`.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use super::types::Tokens;

/// base64url decoder that, like Node's `Buffer.from(s, 'base64url')`, accepts optional padding.
const B64URL_LENIENT: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("unknown token format")]
    UnknownFormat,
    #[error("token ciphertext too short")]
    TooShort,
    /// Node's message for a failed GCM tag check (wrong key or tampered data).
    #[error("Unsupported state or unable to authenticate data")]
    Auth,
    #[error("invalid base64url token")]
    Base64,
    #[error("decrypted token is not UTF-8")]
    Utf8,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn key(secret: &str) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(b"opencanvas"), secret.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(b"social-tokens/v1", &mut okm)
        .expect("32 bytes is a valid HKDF-SHA256 length");
    okm
}

/// `encrypt(plain, secret)`.
pub fn encrypt(plain: &str, secret: &str) -> String {
    let iv: [u8; 12] = rand_bytes();
    let cipher = Aes256Gcm::new_from_slice(&key(secret)).expect("32-byte key");
    // aes-gcm appends the 16-byte tag to the ciphertext; the stored layout is iv | tag | ct.
    let out = cipher
        .encrypt(
            Nonce::from_slice(&iv),
            Payload {
                msg: plain.as_bytes(),
                aad: b"",
            },
        )
        .expect("AES-GCM encryption");
    let (ct, tag) = out.split_at(out.len() - 16);
    let mut raw = Vec::with_capacity(28 + ct.len());
    raw.extend_from_slice(&iv);
    raw.extend_from_slice(tag);
    raw.extend_from_slice(ct);
    format!("v1.{}", URL_SAFE_NO_PAD.encode(raw))
}

/// `decrypt(sealed, secret)`.
pub fn decrypt(sealed: &str, secret: &str) -> Result<String, TokenError> {
    let rest = sealed
        .strip_prefix("v1.")
        .ok_or(TokenError::UnknownFormat)?;
    let raw = B64URL_LENIENT
        .decode(rest)
        .map_err(|_| TokenError::Base64)?;
    if raw.len() < 28 {
        return Err(TokenError::TooShort);
    }
    let (iv, rest) = raw.split_at(12);
    let (tag, ct) = rest.split_at(16);
    let mut buf = Vec::with_capacity(ct.len() + 16);
    buf.extend_from_slice(ct);
    buf.extend_from_slice(tag);
    let cipher = Aes256Gcm::new_from_slice(&key(secret)).expect("32-byte key");
    let plain = cipher
        .decrypt(
            Nonce::from_slice(iv),
            Payload {
                msg: &buf,
                aad: b"",
            },
        )
        .map_err(|_| TokenError::Auth)?;
    String::from_utf8(plain).map_err(|_| TokenError::Utf8)
}

/// `sealTokens(t, secret)`: `encrypt(JSON.stringify(t))`.
pub fn seal_tokens(t: &Tokens, secret: &str) -> String {
    encrypt(&serde_json::to_string(t).expect("Tokens serialize"), secret)
}

/// `openTokens(s, secret)`.
pub fn open_tokens(sealed: &str, secret: &str) -> Result<Tokens, TokenError> {
    Ok(serde_json::from_str(&decrypt(sealed, secret)?)?)
}

/// Same derivation as `@oc/auth` `internalToken()`.
pub fn internal_token(secret: &str) -> String {
    URL_SAFE_NO_PAD.encode(hmac(secret, b"internal"))
}

// ---------------------------------------------------------------- signed public media links

/// HMAC over (sha, exp) so media links can't be forged or enumerated. `exp` is epoch seconds.
pub fn media_sig(sha: &str, exp: f64, secret: &str) -> String {
    let msg = format!("social-media:{sha}:{}", oc_js::number_to_string(exp));
    let mut s = URL_SAFE_NO_PAD.encode(hmac(secret, msg.as_bytes()));
    s.truncate(32);
    s
}

/// `verifyMediaSig(sha, exp, sig, now, secret)` (`now_ms` = `Date.now()`).
pub fn verify_media_sig(sha: &str, exp: f64, sig: &str, now_ms: f64, secret: &str) -> bool {
    if !exp.is_finite() || exp * 1000.0 < now_ms {
        return false;
    }
    let a = media_sig(sha, exp, secret);
    a.len() == sig.len() && bool::from(a.as_bytes().ct_eq(sig.as_bytes()))
}

/// `signedMediaUrl(sha, ext, ttlSec, base)`: `<base>/api/social/media/<sha>.<ext>?exp=…&sig=…`.
pub fn signed_media_url(sha: &str, ext: &str, ttl_sec: i64, base: &str, secret: &str) -> String {
    signed_media_url_at(sha, ext, ttl_sec, base, secret, oc_js::now_ms())
}

/// [`signed_media_url`] at an explicit `Date.now()`.
pub fn signed_media_url_at(
    sha: &str,
    ext: &str,
    ttl_sec: i64,
    base: &str,
    secret: &str,
    now_ms: i64,
) -> String {
    let exp = now_ms.div_euclid(1000) + ttl_sec;
    format!(
        "{base}/api/social/media/{sha}.{ext}?exp={exp}&sig={}",
        media_sig(sha, exp as f64, secret)
    )
}

/// Default media link lifetime (7 days).
pub const MEDIA_TTL_SEC: i64 = 7 * 86400;

// ---------------------------------------------------------------- wrappers over the process config

pub fn encrypt_cfg(plain: &str) -> String {
    encrypt(plain, &oc_config::config().secret)
}
pub fn decrypt_cfg(sealed: &str) -> Result<String, TokenError> {
    decrypt(sealed, &oc_config::config().secret)
}
pub fn seal_tokens_cfg(t: &Tokens) -> String {
    seal_tokens(t, &oc_config::config().secret)
}
pub fn open_tokens_cfg(sealed: &str) -> Result<Tokens, TokenError> {
    open_tokens(sealed, &oc_config::config().secret)
}
pub fn internal_token_cfg() -> String {
    internal_token(&oc_config::config().secret)
}
pub fn media_sig_cfg(sha: &str, exp: f64) -> String {
    media_sig(sha, exp, &oc_config::config().secret)
}
pub fn verify_media_sig_cfg(sha: &str, exp: f64, sig: &str) -> bool {
    verify_media_sig(
        sha,
        exp,
        sig,
        oc_js::now_ms() as f64,
        &oc_config::config().secret,
    )
}
/// `signedMediaUrl(sha, ext, ttlSec ?? 7 days)` with base = usercontent origin, else web origin.
pub fn signed_media_url_cfg(sha: &str, ext: &str, ttl_sec: Option<i64>) -> String {
    let c = oc_config::config();
    let base = if c.usercontent_origin.is_empty() {
        &c.web_origin
    } else {
        &c.usercontent_origin
    };
    signed_media_url(sha, ext, ttl_sec.unwrap_or(MEDIA_TTL_SEC), base, &c.secret)
}

fn hmac(secret: &str, msg: &[u8]) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");
    m.update(msg);
    m.finalize().into_bytes().to_vec()
}

fn rand_bytes<const N: usize>() -> [u8; N] {
    use aes_gcm::aead::rand_core::RngCore;
    let mut b = [0u8; N];
    aes_gcm::aead::OsRng.fill_bytes(&mut b);
    b
}
