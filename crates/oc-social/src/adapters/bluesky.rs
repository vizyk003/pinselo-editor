//! Bluesky (AT Protocol) with an app password; no OAuth app registration needed.
//!   com.atproto.server.createSession → uploadBlob (per image, ≤ 1 MB) → createRecord app.bsky.feed.post
//! Hashtags become richtext tag facets (UTF-8 byte offsets). Images only (up to 4).
//!
//! Env: optional OC_BLUESKY_SERVICE (default https://bsky.social); OC_BLUESKY_DISABLED=1 hides it.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::{opt_str, or_str, str_or_empty};
use crate::http::{Http, Req, js_string, non_null, prop, slice_utf16};
use crate::text::tag_facets;
use crate::types::{
    AdapterFuture, AuthKind, ConnectedAccount, Limits, PostKind, PublishContext, PublishError, PublishResult, SocialAdapter,
    Tokens,
};

pub struct Bluesky;
pub static BLUESKY: Bluesky = Bluesky;

static LIMITS: Limits = Limits {
    max_chars: 300,
    images: 4,
    video: false,
    image_types: &["image/jpeg", "image/png"],
    max_image_bytes: Some(1_000_000),
};

/// Session lifetime assumed for the access JWT (90 minutes).
const SESSION_MS: f64 = 90.0 * 60_000.0;

fn strip_slash(s: &str) -> String {
    s.strip_suffix('/').unwrap_or(s).to_string()
}

/// The entryway service (`OC_BLUESKY_SERVICE`, default https://bsky.social), without a trailing slash.
pub fn service(http: &Http) -> String {
    strip_slash(
        &http
            .env("OC_BLUESKY_SERVICE")
            .unwrap_or_else(|| "https://bsky.social".into()),
    )
}

/// The account's PDS from the session's DID document, else the entryway.
fn pds_of(http: &Http, s: &Value) -> String {
    let found = s
        .get("didDoc")
        .and_then(|d| d.get("service"))
        .and_then(Value::as_array)
        .and_then(|svcs| {
            svcs.iter().find(|x| {
                x.get("id").and_then(Value::as_str) == Some("#atproto_pds")
                    || x.get("type").and_then(Value::as_str) == Some("AtprotoPersonalDataServer")
            })
        })
        .and_then(|x| x.get("serviceEndpoint"))
        .and_then(Value::as_str);
    match found {
        Some(ep) => strip_slash(ep),
        None => service(http),
    }
}

async fn create_session(
    http: &Http,
    identifier: &str,
    password: &str,
) -> Result<Value, PublishError> {
    let url = format!("{}/xrpc/com.atproto.server.createSession", service(http));
    Ok(http
        .call(
            &url,
            Req::post().json(&json!({ "identifier": identifier, "password": password })),
        )
        .await?
        .body)
}

fn to_tokens(http: &Http, s: &Value, identifier: &str, password: &str) -> Tokens {
    let field = |k: &str| non_null(s.get(k)).map(|v| js_string(Some(v)));
    let mut extra = BTreeMap::new();
    extra.insert("identifier".to_string(), identifier.to_string());
    extra.insert("appPassword".to_string(), password.to_string());
    extra.insert("pds".to_string(), pds_of(http, s));
    if let Some(did) = field("did") {
        extra.insert("did".to_string(), did);
    }
    Tokens {
        access_token: field("accessJwt").unwrap_or_default(),
        refresh_token: field("refreshJwt"),
        expires_at: Some(oc_js::now_ms() as f64 + SESSION_MS),
        extra: Some(extra),
    }
}

fn extra<'a>(t: &'a Tokens, k: &str) -> Option<&'a str> {
    t.extra.as_ref().and_then(|e| e.get(k)).map(String::as_str)
}

async fn refresh(http: &Http, t: &Tokens) -> Result<Option<Tokens>, PublishError> {
    let pds = extra(t, "pds")
        .map(String::from)
        .unwrap_or_else(|| service(http));
    let url = format!("{pds}/xrpc/com.atproto.server.refreshSession");
    match http
        .call(
            &url,
            Req::post().bearer(t.refresh_token.as_deref().unwrap_or("")),
        )
        .await
    {
        Ok(r) => Ok(Some(to_tokens(
            http,
            &r.body,
            extra(t, "identifier").unwrap_or(""),
            extra(t, "appPassword").unwrap_or(""),
        ))),
        Err(_) => {
            // Refresh tokens expire after a while; the stored app password opens a new session.
            let (Some(id), Some(pw)) = (
                extra(t, "identifier").filter(|s| !s.is_empty()),
                extra(t, "appPassword").filter(|s| !s.is_empty()),
            ) else {
                return Ok(None);
            };
            let s = create_session(http, id, pw).await?;
            Ok(Some(to_tokens(http, &s, id, pw)))
        }
    }
}

async fn publish(http: &Http, ctx: &PublishContext) -> Result<PublishResult, PublishError> {
    if ctx.kind == PostKind::Video {
        return Err(PublishError::new(
            "Bluesky posts from Pinselo support images only — schedule this design as PNG or JPG.",
        ));
    }
    let token = &ctx.tokens.access_token;
    let pds = extra(&ctx.tokens, "pds")
        .map(String::from)
        .unwrap_or_else(|| service(http));
    let did = extra(&ctx.tokens, "did")
        .map(String::from)
        .unwrap_or_else(|| ctx.account.external_id.clone());
    let mut images = Vec::new();
    for m in ctx.media.iter().take(4) {
        if m.bytes.len() > 1_000_000 {
            return Err(PublishError::new(
                "Bluesky images must be under 1 MB — choose JPG or fewer details.",
            ));
        }
        let r = http
            .call(
                &format!("{pds}/xrpc/com.atproto.repo.uploadBlob"),
                Req::post()
                    .bytes(m.bytes.clone())
                    .bearer(token)
                    .header("content-type", m.mime.clone()),
            )
            .await?;
        let mut img = Map::new();
        img.insert("alt".into(), json!(slice_utf16(&ctx.title, 1000)));
        if let Some(blob) = r.body.get("blob") {
            img.insert("image".into(), blob.clone());
        }
        img.insert(
            "aspectRatio".into(),
            json!({ "width": m.width, "height": m.height }),
        );
        images.push(Value::Object(img));
    }
    let facets = tag_facets(&ctx.text);
    let mut record = Map::new();
    record.insert("$type".into(), json!("app.bsky.feed.post"));
    record.insert("text".into(), json!(ctx.text));
    record.insert(
        "createdAt".into(),
        json!(oc_js::iso_string(&chrono::Utc::now())),
    );
    if !facets.is_empty() {
        record.insert(
            "facets".into(),
            serde_json::to_value(&facets).expect("facets serialize"),
        );
    }
    if !images.is_empty() {
        record.insert(
            "embed".into(),
            json!({ "$type": "app.bsky.embed.images", "images": images }),
        );
    }
    let r = http
        .call(
            &format!("{pds}/xrpc/com.atproto.repo.createRecord"),
            Req::post()
                .json(&json!({ "repo": did, "collection": "app.bsky.feed.post", "record": record }))
                .bearer(token),
        )
        .await?;
    let uri = js_string(prop(Some(&r.body), "uri")?);
    let rkey = uri.rsplit('/').next().unwrap_or("");
    let who = ctx.account.handle.as_deref().unwrap_or(&did);
    Ok(PublishResult {
        url: Some(format!("https://bsky.app/profile/{who}/post/{rkey}")),
        external_id: Some(uri.clone()),
        note: None,
    })
}

async fn connect_with_password(
    http: &Http,
    identifier: &str,
    password: &str,
) -> Result<Vec<ConnectedAccount>, PublishError> {
    let s = create_session(http, identifier.strip_prefix('@').unwrap_or(identifier), password).await?;
    let did = js_string(s.get("did"));
    let access = str_or_empty(s.get("accessJwt"));
    let url = format!(
        "{}/xrpc/app.bsky.actor.getProfile?actor={}",
        pds_of(http, &s),
        oc_js::encode_uri_component(&did)
    );
    let p = http.call(&url, Req::get().bearer(&access)).await.ok();
    let profile = p.as_ref().map(|r| &r.body);
    Ok(vec![ConnectedAccount {
        external_id: str_or_empty(s.get("did")),
        display_name: or_str(
            &[profile.and_then(|b| b.get("displayName")), s.get("handle")],
            "",
        ),
        handle: opt_str(s.get("handle")),
        avatar_url: opt_str(profile.and_then(|b| b.get("avatar"))),
        tokens: to_tokens(http, &s, identifier, password),
        meta: None,
    }])
}

impl SocialAdapter for Bluesky {
    fn id(&self) -> &'static str {
        "bluesky"
    }
    fn label(&self) -> &'static str {
        "Bluesky"
    }
    fn auth(&self) -> AuthKind {
        AuthKind::AppPassword
    }
    fn connect_with_password<'a>(
        &'a self,
        http: &'a Http,
        identifier: &'a str,
        password: &'a str,
    ) -> Option<AdapterFuture<'a, Vec<ConnectedAccount>>> {
        Some(Box::pin(connect_with_password(http, identifier, password)))
    }
    fn env(&self) -> &'static [&'static str] {
        &[]
    }
    fn configured(&self, http: &Http) -> bool {
        http.env("OC_BLUESKY_DISABLED").as_deref() != Some("1")
    }
    fn limits(&self) -> &Limits {
        &LIMITS
    }
    fn has_refresh(&self) -> bool {
        true
    }
    fn refresh<'a>(&'a self, http: &'a Http, t: &'a Tokens) -> AdapterFuture<'a, Option<Tokens>> {
        Box::pin(refresh(http, t))
    }
    fn publish<'a>(
        &'a self,
        http: &'a Http,
        ctx: &'a PublishContext,
    ) -> AdapterFuture<'a, PublishResult> {
        Box::pin(publish(http, ctx))
    }
}
