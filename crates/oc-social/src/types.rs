//! Publisher adapter interface (port of services/api/src/social/types.ts).

use std::collections::BTreeMap;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::http::Http;

/// OAuth / app-password credentials. JSON shape = TS `Tokens` (camelCase, absent fields omitted).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Epoch ms when the access token expires (absent = long-lived / unknown).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "js_number_opt"
    )]
    pub expires_at: Option<f64>,
    /// Provider-specific secrets (e.g. a Bluesky app password, a PDS url).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<BTreeMap<String, String>>,
}

/// Serialize a number like `JSON.stringify` (integral doubles without a fractional part).
fn js_number_opt<S: serde::Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(n) if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 => {
            s.serialize_i64(*n as i64)
        }
        Some(n) => s.serialize_f64(*n),
        None => s.serialize_none(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaFile {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub filename: String,
    /// Publicly fetchable, signed, time-limited URL.
    pub url: String,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostKind {
    Image,
    Carousel,
    Video,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccountInfo {
    pub external_id: String,
    pub display_name: String,
    pub handle: Option<String>,
    pub meta: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishContext {
    pub account: AccountInfo,
    pub tokens: Tokens,
    pub media: Vec<MediaFile>,
    pub kind: PostKind,
    /// Caption with hashtags appended.
    pub text: String,
    pub hashtags: Vec<String>,
    /// Per-network extras from the post (pinterestBoardId, youtubeTitle, tiktokMode, durationMs, …).
    pub options: Map<String, Value>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PublishResult {
    /// Public URL of the post (None when the network gives none).
    pub url: Option<String>,
    pub external_id: Option<String>,
    pub note: Option<String>,
}

/// How an account is connected (`'oauth' | 'app-password' | 'none'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthKind {
    #[serde(rename = "oauth")]
    OAuth,
    #[serde(rename = "app-password")]
    AppPassword,
    #[serde(rename = "none")]
    None,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthKind::OAuth => "oauth",
            AuthKind::AppPassword => "app-password",
            AuthKind::None => "none",
        }
    }
}

/// `AuthorizeParams`: OAuth 2.0 authorization code (PKCE where supported).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuthorizeParams {
    pub state: String,
    pub redirect_uri: String,
    pub code_challenge: String,
}

/// `ExchangeParams`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExchangeParams {
    pub code: String,
    pub redirect_uri: String,
    pub code_verifier: String,
}

/// An account granted by a connect flow (`ConnectedAccount`). `None` = TS `undefined`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConnectedAccount {
    pub external_id: String,
    pub display_name: String,
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
    pub tokens: Tokens,
    pub meta: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub max_chars: usize,
    /// Max images in one post (0 = images unsupported).
    pub images: usize,
    pub video: bool,
    pub image_types: &'static [&'static str],
    pub max_image_bytes: Option<usize>,
}

impl Limits {
    /// The JSON shape of the TS `limits` object (`maxImageBytes` only when set).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("maxChars".into(), Value::from(self.max_chars));
        m.insert("images".into(), Value::from(self.images));
        m.insert("video".into(), Value::from(self.video));
        m.insert(
            "imageTypes".into(),
            Value::from(self.image_types.iter().map(|s| Value::from(*s)).collect::<Vec<_>>()),
        );
        if let Some(b) = self.max_image_bytes {
            m.insert("maxImageBytes".into(), Value::from(b));
        }
        Value::Object(m)
    }
}

/// A failed provider call. `transient` errors (network, 429, 5xx) are retried with backoff.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct PublishError {
    pub message: String,
    pub transient: bool,
    pub status: Option<u16>,
    pub reauth: bool,
}

impl PublishError {
    /// `new PublishError(message)` (permanent).
    pub fn new(message: impl Into<String>) -> Self {
        PublishError {
            message: message.into(),
            transient: false,
            status: None,
            reauth: false,
        }
    }
    /// `new PublishError(message, true)`.
    pub fn transient(message: impl Into<String>) -> Self {
        PublishError {
            message: message.into(),
            transient: true,
            status: None,
            reauth: false,
        }
    }
}

pub type AdapterFuture<'a, T> = BoxFuture<'a, Result<T, PublishError>>;

/// One social network (`SocialAdapter`). The optional TS methods are `Option`s here: `authorize_url`,
/// `exchange` and `connect_with_password` return `None` when the network does not implement them.
pub trait SocialAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn auth(&self) -> AuthKind;
    /// Env vars that must be set for this adapter to be offered.
    fn env(&self) -> &'static [&'static str];
    fn configured(&self, http: &Http) -> bool;
    fn limits(&self) -> &Limits;
    /// Whether the adapter implements `refresh` (TS: `a.refresh` is defined).
    fn has_refresh(&self) -> bool {
        false
    }
    /// Refresh tokens; `Ok(None)` when the provider has no refresh (the account must be reconnected).
    fn refresh<'a>(
        &'a self,
        _http: &'a Http,
        _tokens: &'a Tokens,
    ) -> AdapterFuture<'a, Option<Tokens>> {
        Box::pin(async { Ok(None) })
    }
    /// Build the provider's authorization URL (OAuth 2.0 authorization code; PKCE where supported).
    fn authorize_url(&self, _http: &Http, _p: &AuthorizeParams) -> Option<String> {
        None
    }
    /// Exchange the code for tokens and return the account(s) it grants (a Facebook login may grant many
    /// Pages).
    fn exchange<'a>(
        &'a self,
        _http: &'a Http,
        _p: &'a ExchangeParams,
    ) -> Option<AdapterFuture<'a, Vec<ConnectedAccount>>> {
        None
    }
    /// App-password connection (Bluesky).
    fn connect_with_password<'a>(
        &'a self,
        _http: &'a Http,
        _identifier: &'a str,
        _password: &'a str,
    ) -> Option<AdapterFuture<'a, Vec<ConnectedAccount>>> {
        None
    }
    fn publish<'a>(
        &'a self,
        http: &'a Http,
        ctx: &'a PublishContext,
    ) -> AdapterFuture<'a, PublishResult>;
}
