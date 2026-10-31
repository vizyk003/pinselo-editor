//! X (Twitter) API v2 with OAuth 2.0 authorization code + PKCE.
//!   images: POST /2/media/upload (multipart, media_category=tweet_image), up to 4 per post
//!   video:  POST /2/media/upload/initialize → /{id}/append (chunks) → /{id}/finalize → poll processing_info
//!   post:   POST /2/tweets {text, media.media_ids}
//!
//! Env: OC_X_CLIENT_ID, OC_X_CLIENT_SECRET (confidential client).

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{first_media, opt_str, str_or_empty, tokens_from};
use crate::http::{
    Http, Part, Req, form, js_number, js_string, non_null, poll_timeout_error, prop, truthy,
    with_query,
};
use crate::types::{
    AdapterFuture, AuthKind, AuthorizeParams, ConnectedAccount, ExchangeParams, Limits, PostKind, PublishContext, PublishError, PublishResult, SocialAdapter,
    Tokens,
};

const API: &str = "https://api.x.com/2";
const CHUNK: usize = 4 * 1024 * 1024;

pub struct X;
pub static XCOM: X = X;

static LIMITS: Limits = Limits {
    max_chars: 280,
    images: 4,
    video: true,
    image_types: &["image/png", "image/jpeg"],
    max_image_bytes: Some(5 * 1024 * 1024),
};

fn id(http: &Http) -> String {
    http.env_or_empty("OC_X_CLIENT_ID")
}
fn secret(http: &Http) -> String {
    http.env_or_empty("OC_X_CLIENT_SECRET")
}

async fn token_request(http: &Http, data: &[(&str, &str)]) -> Result<Tokens, PublishError> {
    let cid = id(http);
    let mut pairs: Vec<(&str, Option<&str>)> = data.iter().map(|(k, v)| (*k, Some(*v))).collect();
    pairs.push(("client_id", Some(&cid)));
    let req = Req::post().basic(&cid, &secret(http)).form(form(&pairs));
    let r = http.call(&format!("{API}/oauth2/token"), req).await?;
    Ok(tokens_from(&r.body))
}

/// `r.body.data`.
fn data_of(body: &Value) -> Result<Option<&Value>, PublishError> {
    prop(Some(body), "data")
}

async fn publish(http: &Http, ctx: &PublishContext) -> Result<PublishResult, PublishError> {
    let token = &ctx.tokens.access_token;
    let mut ids: Vec<Value> = Vec::new();
    if ctx.kind == PostKind::Video {
        let v = first_media(ctx, "mime")?;
        let init = http
            .call(
                &format!("{API}/media/upload/initialize"),
                Req::post()
                    .json(&json!({ "media_type": v.mime, "total_bytes": v.bytes.len(), "media_category": "tweet_video" }))
                    .bearer(token),
            )
            .await?;
        let mid_v = prop(data_of(&init.body)?, "id")?
            .cloned()
            .unwrap_or(Value::Null);
        let mid = js_string(Some(&mid_v));
        for (seg, chunk) in v.bytes.chunks(CHUNK).enumerate() {
            let parts = vec![
                Part::Text {
                    name: "segment_index".into(),
                    value: seg.to_string(),
                },
                Part::File {
                    name: "media".into(),
                    bytes: chunk.to_vec(),
                    mime: "application/octet-stream".into(),
                    filename: "chunk".into(),
                },
            ];
            http.call(
                &format!("{API}/media/upload/{mid}/append"),
                Req::post()
                    .multipart(parts)
                    .bearer(token)
                    .timeout(Duration::from_secs(300)),
            )
            .await?;
        }
        let fin = http
            .call(
                &format!("{API}/media/upload/{mid}/finalize"),
                Req::post().bearer(token),
            )
            .await?;
        let mut info = prop(data_of(&fin.body)?, "processing_info")?.cloned();
        if truthy(info.as_ref()) {
            // Hand-rolled `poll(…, { what: 'X', intervalMs: 0 })`: the wait comes from check_after_secs.
            let deadline: Instant = http.poll_deadline(None);
            loop {
                let state = info
                    .as_ref()
                    .and_then(|i| i.get("state"))
                    .and_then(Value::as_str);
                if !truthy(info.as_ref()) || state == Some("succeeded") {
                    break;
                }
                let i = info.as_ref().expect("truthy");
                if state == Some("failed") {
                    let msg = non_null(i.get("error").and_then(|e| e.get("message")))
                        .map(|m| js_string(Some(m)));
                    return Err(PublishError::new(format!(
                        "X could not process the video: {}",
                        msg.as_deref().unwrap_or("unknown error")
                    )));
                }
                let after = non_null(i.get("check_after_secs"))
                    .map(|x| js_number(Some(x)))
                    .unwrap_or(3.0);
                let secs = if after.is_nan() {
                    f64::NAN
                } else {
                    after.min(30.0)
                };
                http.sleep(Duration::from_secs_f64(if secs.is_finite() && secs > 0.0 {
                    secs
                } else {
                    0.0
                }))
                .await;
                let st_url = with_query(
                    &format!("{API}/media/upload"),
                    &[
                        ("command", Some("STATUS")),
                        ("media_id", Some(mid.as_str())),
                    ],
                );
                let st = http.call(&st_url, Req::get().bearer(token)).await?;
                info = prop(data_of(&st.body)?, "processing_info")?.cloned();
                if Instant::now() > deadline {
                    return Err(poll_timeout_error(Some("X")));
                }
            }
        }
        ids.push(mid_v);
    } else {
        for m in ctx.media.iter().take(4) {
            let parts = vec![
                Part::Text {
                    name: "media_category".into(),
                    value: "tweet_image".into(),
                },
                Part::Text {
                    name: "media_type".into(),
                    value: m.mime.clone(),
                },
                Part::File {
                    name: "media".into(),
                    bytes: m.bytes.clone(),
                    mime: m.mime.clone(),
                    filename: m.filename.clone(),
                },
            ];
            let r = http
                .call(
                    &format!("{API}/media/upload"),
                    Req::post().multipart(parts).bearer(token),
                )
                .await?;
            ids.push(
                prop(data_of(&r.body)?, "id")?
                    .cloned()
                    .unwrap_or(Value::Null),
            );
        }
    }
    let r = http
        .call(
            &format!("{API}/tweets"),
            Req::post()
                .json(&json!({ "text": ctx.text, "media": { "media_ids": ids } }))
                .bearer(token),
        )
        .await?;
    let tid = js_string(prop(data_of(&r.body)?, "id")?);
    let who = ctx.account.handle.as_deref().unwrap_or("i/web");
    Ok(PublishResult {
        url: Some(format!("https://x.com/{who}/status/{tid}")),
        external_id: Some(tid),
        note: None,
    })
}

fn authorize_url(http: &Http, p: &AuthorizeParams) -> String {
    let cid = id(http);
    with_query(
        "https://x.com/i/oauth2/authorize",
        &[
            ("response_type", Some("code")),
            ("client_id", Some(cid.as_str())),
            ("redirect_uri", Some(p.redirect_uri.as_str())),
            ("state", Some(p.state.as_str())),
            ("scope", Some("tweet.read tweet.write users.read media.write offline.access")),
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
        &format!("{API}/users/me"),
        &[("user.fields", Some("profile_image_url"))],
    );
    let me = http.call(&url, Req::get().bearer(&tokens.access_token)).await?;
    let u = data_of(&me.body)?;
    Ok(vec![ConnectedAccount {
        external_id: str_or_empty(prop(u, "id")?),
        display_name: str_or_empty(prop(u, "name")?),
        handle: opt_str(prop(u, "username")?),
        avatar_url: opt_str(prop(u, "profile_image_url")?),
        tokens,
        meta: None,
    }])
}

impl SocialAdapter for X {
    fn id(&self) -> &'static str {
        "x"
    }
    fn label(&self) -> &'static str {
        "X (Twitter)"
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
        &["OC_X_CLIENT_ID", "OC_X_CLIENT_SECRET"]
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
            Ok(Some(
                token_request(
                    http,
                    &[("grant_type", "refresh_token"), ("refresh_token", rt)],
                )
                .await?,
            ))
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
