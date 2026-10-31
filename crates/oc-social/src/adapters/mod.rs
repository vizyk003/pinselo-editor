//! The ten publisher adapters (one module per network, `local` = "Download & remind").

pub mod bluesky;
pub mod linkedin;
pub mod local;
pub mod meta;
pub mod pinterest;
pub mod threads;
pub mod tiktok;
pub mod x;
pub mod youtube;

use serde_json::Value;

use super::http::{expires_in, js_string, non_null, truthy};
use super::types::{MediaFile, PublishContext, PublishError, Tokens};

/// `ctx.media[0]!` followed by a property read: TS throws a TypeError (wrapped as a transient error by the
/// publish job) when there is no media.
pub(crate) fn first_media<'a>(
    ctx: &'a PublishContext,
    reading: &str,
) -> Result<&'a MediaFile, PublishError> {
    ctx.media.first().ok_or_else(|| {
        PublishError::transient(format!(
            "Cannot read properties of undefined (reading '{reading}')"
        ))
    })
}

/// Form pairs with every value present.
pub(crate) fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    super::http::form(
        &pairs
            .iter()
            .map(|(k, v)| (*k, Some(*v)))
            .collect::<Vec<_>>(),
    )
}

/// `String(v)` for a value that is used as a string id, `""` for undefined / null.
pub(crate) fn str_or_empty(v: Option<&Value>) -> String {
    non_null(v).map(|x| js_string(Some(x))).unwrap_or_default()
}

/// An optional string field taken from a provider response (`handle: me.body.username`): `None` for
/// undefined / null.
pub(crate) fn opt_str(v: Option<&Value>) -> Option<String> {
    non_null(v).map(|x| js_string(Some(x)))
}

/// `a || b || … || fallback`.
pub(crate) fn or_str(candidates: &[Option<&Value>], fallback: &str) -> String {
    candidates
        .iter()
        .find(|v| truthy(**v))
        .map(|v| js_string(*v))
        .unwrap_or_else(|| fallback.to_string())
}

/// `v ?? fallback`.
pub(crate) fn nn_or(v: Option<&Value>, fallback: &str) -> String {
    non_null(v)
        .map(|x| js_string(Some(x)))
        .unwrap_or_else(|| fallback.to_string())
}

/// `{ accessToken: b.access_token, refreshToken: b.refresh_token, expiresAt: expiresIn(b.expires_in) }`.
pub(crate) fn tokens_from(body: &Value) -> Tokens {
    Tokens {
        access_token: str_or_empty(body.get("access_token")),
        refresh_token: non_null(body.get("refresh_token")).map(|v| js_string(Some(v))),
        expires_at: expires_in(body.get("expires_in")),
        extra: None,
    }
}
