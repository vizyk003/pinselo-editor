//! YouTube Data API v3 (videos and Shorts). Google OAuth 2.0 with PKCE and offline access.
//!   upload: POST upload/youtube/v3/videos?uploadType=resumable → PUT the bytes to the session URL
//! A vertical video of 3 minutes or less is published as a Short automatically.
//!
//! Env: OC_YOUTUBE_CLIENT_ID, OC_YOUTUBE_CLIENT_SECRET (Google Cloud OAuth client).

use std::time::Duration;

use serde_json::json;

use super::{first_media, opt_str, str_or_empty, tokens_from};
use crate::http::{
    Http, Req, form, js_number, js_string, non_null, prop, slice_utf16, truthy, with_query,
};
use crate::types::{
    AdapterFuture, AuthKind, AuthorizeParams, ConnectedAccount, ExchangeParams, Limits, PostKind, PublishContext, PublishError, PublishResult, SocialAdapter,
    Tokens,
};

pub struct YouTube;
pub static YOUTUBE: YouTube = YouTube;

static LIMITS: Limits = Limits {
    max_chars: 5000,
    images: 0,
    video: true,
    image_types: &[],
    max_image_bytes: None,
};

fn id(http: &Http) -> String {
    http.env_or_empty("OC_YOUTUBE_CLIENT_ID")
}
fn secret(http: &Http) -> String {
    http.env_or_empty("OC_YOUTUBE_CLIENT_SECRET")
}

async fn token_request(http: &Http, data: &[(&str, &str)]) -> Result<Tokens, PublishError> {
    let (cid, cs) = (id(http), secret(http));
    let mut pairs: Vec<(&str, Option<&str>)> = data.iter().map(|(k, v)| (*k, Some(*v))).collect();
    pairs.push(("client_id", Some(&cid)));
    pairs.push(("client_secret", Some(&cs)));
    let r = http
        .call(
            "https://oauth2.googleapis.com/token",
            Req::post().form(form(&pairs)),
        )
        .await?;
    Ok(tokens_from(&r.body))
}

/// The video title: options.youtubeTitle, else the first caption line, else the design title.
pub fn video_title(ctx: &PublishContext) -> String {
    let raw = match ctx.options.get("youtubeTitle").filter(|v| truthy(Some(v))) {
        Some(v) => js_string(Some(v)),
        None => {
            let first = ctx.text.split('\n').next().unwrap_or("");
            if !first.is_empty() {
                first.to_string()
            } else {
                ctx.title.clone()
            }
        }
    };
    let cleaned: String = raw.chars().filter(|c| *c != '<' && *c != '>').collect();
    let t = slice_utf16(&cleaned, 100);
    if t.is_empty() {
        "Untitled".into()
    } else {
        t.to_string()
    }
}

async fn publish(http: &Http, ctx: &PublishContext) -> Result<PublishResult, PublishError> {
    if ctx.kind != PostKind::Video {
        return Err(PublishError::new(
            "YouTube only accepts videos — schedule this design as MP4.",
        ));
    }
    let v = first_media(ctx, "mime")?;
    let token = &ctx.tokens.access_token;
    let title = video_title(ctx);
    let wanted = js_string(ctx.options.get("youtubePrivacy"));
    let privacy = if ["public", "unlisted", "private"].contains(&wanted.as_str()) {
        wanted
    } else {
        "public".into()
    };
    let tags: Vec<&String> = ctx.hashtags.iter().take(30).collect();
    let url = with_query(
        "https://www.googleapis.com/upload/youtube/v3/videos",
        &[
            ("uploadType", Some("resumable")),
            ("part", Some("snippet,status")),
        ],
    );
    let session = http
        .call(
            &url,
            Req::post()
                .json(&json!({
                    "snippet": { "title": title, "description": slice_utf16(&ctx.text, 5000), "tags": tags, "categoryId": "22" },
                    "status": { "privacyStatus": privacy, "selfDeclaredMadeForKids": false },
                }))
                .bearer(token)
                .header("content-type", "application/json; charset=UTF-8")
                .header("x-upload-content-type", v.mime.clone())
                .header("x-upload-content-length", v.bytes.len().to_string()),
        )
        .await?;
    let Some(location) = session.header("location") else {
        return Err(PublishError::transient(
            "YouTube did not start an upload session",
        ));
    };
    let r = http
        .call(
            &location,
            Req::put()
                .bytes(v.bytes.clone())
                .bearer(token)
                .header("content-type", v.mime.clone())
                .timeout(Duration::from_secs(30 * 60)),
        )
        .await?;
    let vid = js_string(prop(Some(&r.body), "id")?);
    let duration = js_number(Some(
        non_null(ctx.options.get("durationMs")).unwrap_or(&json!(0)),
    ));
    let short = v.height > v.width && duration <= 180_000.0;
    let url = if short {
        format!("https://www.youtube.com/shorts/{vid}")
    } else {
        format!("https://www.youtube.com/watch?v={vid}")
    };
    Ok(PublishResult {
        url: Some(url),
        external_id: Some(vid),
        note: None,
    })
}

fn authorize_url(http: &Http, p: &AuthorizeParams) -> String {
    let cid = id(http);
    with_query(
        "https://accounts.google.com/o/oauth2/v2/auth",
        &[
            ("client_id", Some(cid.as_str())),
            ("redirect_uri", Some(p.redirect_uri.as_str())),
            ("response_type", Some("code")),
            ("state", Some(p.state.as_str())),
            (
                "scope",
                Some(
                    "https://www.googleapis.com/auth/youtube.upload https://www.googleapis.com/auth/youtube.readonly",
                ),
            ),
            ("access_type", Some("offline")),
            ("prompt", Some("consent")),
            ("include_granted_scopes", Some("true")),
            ("code_challenge", Some(p.code_challenge.as_str())),
            ("code_challenge_method", Some("S256")),
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
            ("code_verifier", &p.code_verifier),
        ],
    )
    .await?;
    let url = with_query(
        "https://www.googleapis.com/youtube/v3/channels",
        &[("part", Some("snippet")), ("mine", Some("true"))],
    );
    let r = http.call(&url, Req::get().bearer(&tokens.access_token)).await?;
    // `r.body.items?.[0]`
    let ch = match non_null(r.body.get("items")) {
        None => None,
        Some(items) => prop(Some(items), "0")?,
    };
    if !truthy(ch) {
        return Err(PublishError::new(
            "This Google account has no YouTube channel yet.",
        ));
    }
    let snippet = prop(ch, "snippet")?;
    // `ch.snippet.thumbnails?.default?.url`
    let thumb = non_null(prop(snippet, "thumbnails")?)
        .and_then(|t| non_null(t.get("default")))
        .and_then(|d| d.get("url"));
    Ok(vec![ConnectedAccount {
        external_id: str_or_empty(prop(ch, "id")?),
        display_name: str_or_empty(prop(snippet, "title")?),
        handle: opt_str(prop(snippet, "customUrl")?),
        avatar_url: opt_str(thumb),
        tokens,
        meta: None,
    }])
}

impl SocialAdapter for YouTube {
    fn id(&self) -> &'static str {
        "youtube"
    }
    fn label(&self) -> &'static str {
        "YouTube"
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
        &["OC_YOUTUBE_CLIENT_ID", "OC_YOUTUBE_CLIENT_SECRET"]
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
