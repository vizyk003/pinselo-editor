//! LinkedIn member posts (Posts API, versioned REST). Scopes: openid profile w_member_social.
//!   images: POST /rest/images?action=initializeUpload → PUT bytes to uploadUrl
//!   video:  POST /rest/videos?action=initializeUpload → PUT parts (ETags) → action=finalizeUpload
//!   post:   POST /rest/posts → id in the x-restli-id header
//! Commentary uses LinkedIn "little text" (`text::little_text`).
//!
//! Env: OC_LINKEDIN_CLIENT_ID, OC_LINKEDIN_CLIENT_SECRET, optional OC_LINKEDIN_VERSION (YYYYMM).

use std::time::Duration;

use serde_json::{Value, json};

use super::{first_media, nn_or, opt_str, str_or_empty, tokens_from};
use crate::http::{Http, Req, form, js_string, prop, with_query};
use crate::text::little_text;
use crate::types::{
    AdapterFuture, AuthKind, AuthorizeParams, ConnectedAccount, ExchangeParams, Limits, PostKind, PublishContext, PublishError, PublishResult, SocialAdapter,
    Tokens,
};

const API: &str = "https://api.linkedin.com";

pub struct LinkedIn;
pub static LINKEDIN: LinkedIn = LinkedIn;

static LIMITS: Limits = Limits {
    max_chars: 3000,
    images: 20,
    video: true,
    image_types: &["image/png", "image/jpeg"],
    max_image_bytes: None,
};

fn id(http: &Http) -> String {
    http.env_or_empty("OC_LINKEDIN_CLIENT_ID")
}
fn secret(http: &Http) -> String {
    http.env_or_empty("OC_LINKEDIN_CLIENT_SECRET")
}

/// Bearer + versioned REST headers.
fn headers(http: &Http, token: &str) -> Req {
    Req::new(reqwest::Method::GET)
        .bearer(token)
        .header(
            "linkedin-version",
            http.env("OC_LINKEDIN_VERSION")
                .unwrap_or_else(|| "202507".into()),
        )
        .header("x-restli-protocol-version", "2.0.0")
}

fn with_method(mut r: Req, m: reqwest::Method) -> Req {
    r.method = m;
    r
}

async fn token_request(http: &Http, data: &[(&str, &str)]) -> Result<Tokens, PublishError> {
    let (cid, cs) = (id(http), secret(http));
    let mut pairs: Vec<(&str, Option<&str>)> = data.iter().map(|(k, v)| (*k, Some(*v))).collect();
    pairs.push(("client_id", Some(&cid)));
    pairs.push(("client_secret", Some(&cs)));
    let r = http
        .call(
            "https://www.linkedin.com/oauth/v2/accessToken",
            Req::post().form(form(&pairs)),
        )
        .await?;
    Ok(tokens_from(&r.body))
}

fn str_of(v: Option<&Value>) -> Value {
    v.cloned().unwrap_or(Value::Null)
}

async fn publish(http: &Http, ctx: &PublishContext) -> Result<PublishResult, PublishError> {
    let token = &ctx.tokens.access_token;
    let owner = format!("urn:li:person:{}", ctx.account.external_id);
    let h = || headers(http, token);
    let content = if ctx.kind == PostKind::Video {
        let v = first_media(ctx, "bytes")?;
        let init = http
            .call(
                &format!("{API}/rest/videos?action=initializeUpload"),
                with_method(h(), reqwest::Method::POST).json(&json!({
                    "initializeUploadRequest": { "owner": owner, "fileSizeBytes": v.bytes.len(), "uploadCaptions": false, "uploadThumbnail": false }
                })),
            )
            .await?;
        let value = prop(Some(&init.body), "value")?;
        let video = str_of(prop(value, "video")?);
        let upload_token = prop(value, "uploadToken")?
            .filter(|x| !x.is_null())
            .cloned()
            .unwrap_or_else(|| json!(""));
        let mut etags = Vec::new();
        let parts = prop(value, "uploadInstructions")?;
        let parts: Vec<Value> = match parts {
            Some(Value::Array(a)) => a.clone(),
            None | Some(Value::Null) => {
                return Err(PublishError::transient(
                    "uploadInstructions is not iterable",
                ));
            }
            Some(_) => Vec::new(),
        };
        for part in &parts {
            let url = js_string(prop(Some(part), "uploadUrl")?);
            let first = prop(Some(part), "firstByte")?
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .max(0.0) as usize;
            let last = prop(Some(part), "lastByte")?
                .and_then(Value::as_f64)
                .unwrap_or(-1.0);
            let end = ((last + 1.0).max(0.0) as usize).min(v.bytes.len());
            let chunk = if first < end {
                v.bytes[first..end].to_vec()
            } else {
                Vec::new()
            };
            let r = http
                .call(
                    &url,
                    Req::put()
                        .bytes(chunk)
                        .bearer(token)
                        .header("content-type", "application/octet-stream")
                        .timeout(Duration::from_secs(600)),
                )
                .await?;
            etags.push(r.header("etag").unwrap_or_default());
        }
        http.call(
            &format!("{API}/rest/videos?action=finalizeUpload"),
            with_method(h(), reqwest::Method::POST).json(&json!({
                "finalizeUploadRequest": { "video": video, "uploadToken": upload_token, "uploadedPartIds": etags }
            })),
        )
        .await?;
        let status_url = format!(
            "{API}/rest/videos/{}",
            oc_js::encode_uri_component(&js_string(Some(&video)))
        );
        http.poll(
            Some("LinkedIn"),
            Duration::from_millis(5000),
            None,
            || async {
                let r = http.call(&status_url, h()).await?;
                match r.body.get("status").and_then(Value::as_str) {
                    Some("AVAILABLE") => Ok(Some(())),
                    Some("PROCESSING_FAILED") => {
                        Err(PublishError::new("LinkedIn could not process the video"))
                    }
                    _ => Ok(None),
                }
            },
        )
        .await?;
        json!({ "media": { "id": video, "title": ctx.title } })
    } else {
        let mut ids = Vec::new();
        for m in &ctx.media {
            let init = http
                .call(
                    &format!("{API}/rest/images?action=initializeUpload"),
                    with_method(h(), reqwest::Method::POST)
                        .json(&json!({ "initializeUploadRequest": { "owner": owner } })),
                )
                .await?;
            let value = prop(Some(&init.body), "value")?;
            let upload_url = js_string(prop(value, "uploadUrl")?);
            http.call(
                &upload_url,
                Req::put()
                    .bytes(m.bytes.clone())
                    .bearer(token)
                    .header("content-type", m.mime.clone()),
            )
            .await?;
            ids.push(str_of(prop(value, "image")?));
        }
        if ids.len() == 1 {
            json!({ "media": { "id": ids[0], "altText": ctx.title } })
        } else {
            json!({ "multiImage": { "images": ids.iter().map(|x| json!({ "id": x, "altText": ctx.title })).collect::<Vec<_>>() } })
        }
    };
    let r = http
        .call(
            &format!("{API}/rest/posts"),
            with_method(h(), reqwest::Method::POST).json(&json!({
                "author": owner,
                "commentary": little_text(&ctx.text),
                "visibility": "PUBLIC",
                "distribution": { "feedDistribution": "MAIN_FEED", "targetEntities": [], "thirdPartyDistributionChannels": [] },
                "content": content,
                "lifecycleState": "PUBLISHED",
                "isReshareDisabledByAuthor": false,
            })),
        )
        .await?;
    let urn = r.header("x-restli-id").unwrap_or_default();
    Ok(if urn.is_empty() {
        PublishResult {
            url: None,
            external_id: None,
            note: None,
        }
    } else {
        PublishResult {
            url: Some(format!("https://www.linkedin.com/feed/update/{urn}/")),
            external_id: Some(urn),
            note: None,
        }
    })
}

fn authorize_url(http: &Http, p: &AuthorizeParams) -> String {
    let cid = id(http);
    with_query(
        "https://www.linkedin.com/oauth/v2/authorization",
        &[
            ("response_type", Some("code")),
            ("client_id", Some(cid.as_str())),
            ("redirect_uri", Some(p.redirect_uri.as_str())),
            ("state", Some(p.state.as_str())),
            ("scope", Some("openid profile w_member_social")),
        ],
    )
}

async fn exchange(http: &Http, p: &ExchangeParams) -> Result<Vec<ConnectedAccount>, PublishError> {
    let tokens = token_request(
        http,
        &[
            ("grant_type", "authorization_code"),
            ("code", &p.code),
            ("redirect_uri", &p.redirect_uri),
        ],
    )
    .await?;
    let me = http
        .call(
            &format!("{API}/v2/userinfo"),
            Req::get().bearer(&tokens.access_token),
        )
        .await?;
    Ok(vec![ConnectedAccount {
        external_id: str_or_empty(me.body.get("sub")),
        display_name: nn_or(me.body.get("name"), "LinkedIn member"),
        handle: None,
        avatar_url: opt_str(me.body.get("picture")),
        tokens,
        meta: None,
    }])
}

impl SocialAdapter for LinkedIn {
    fn id(&self) -> &'static str {
        "linkedin"
    }
    fn label(&self) -> &'static str {
        "LinkedIn"
    }
    fn auth(&self) -> AuthKind {
        AuthKind::OAuth
    }
    fn authorize_url(&self, http: &Http, p: &AuthorizeParams) -> Option<String> {
        Some(authorize_url(http, p))
    }
    fn exchange<'a>(
        &'a self,
        http: &'a Http,
        p: &'a ExchangeParams,
    ) -> Option<AdapterFuture<'a, Vec<ConnectedAccount>>> {
        Some(Box::pin(exchange(http, p)))
    }
    fn env(&self) -> &'static [&'static str] {
        &["OC_LINKEDIN_CLIENT_ID", "OC_LINKEDIN_CLIENT_SECRET"]
    }
    fn configured(&self, http: &Http) -> bool {
        !id(http).is_empty() && !secret(http).is_empty()
    }
    fn limits(&self) -> &Limits {
        &LIMITS
    }
    fn has_refresh(&self) -> bool {
        true
    }
    fn refresh<'a>(&'a self, http: &'a Http, t: &'a Tokens) -> AdapterFuture<'a, Option<Tokens>> {
        Box::pin(async move {
            // Refresh tokens are only issued to apps LinkedIn has approved for them; otherwise reconnect after 60 days.
            let Some(rt) = t.refresh_token.as_deref().filter(|s| !s.is_empty()) else {
                return Ok(None);
            };
            let mut next = token_request(
                http,
                &[("grant_type", "refresh_token"), ("refresh_token", rt)],
            )
            .await?;
            if next.refresh_token.is_none() {
                next.refresh_token = t.refresh_token.clone();
            }
            Ok(Some(next))
        })
    }
    fn publish<'a>(
        &'a self,
        http: &'a Http,
        ctx: &'a PublishContext,
    ) -> AdapterFuture<'a, PublishResult> {
        Box::pin(publish(http, ctx))
    }
}
