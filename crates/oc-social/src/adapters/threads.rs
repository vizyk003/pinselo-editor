//! Threads API (graph.threads.net). Scopes: threads_basic threads_content_publish.
//!   POST /{user}/threads (IMAGE | VIDEO | CAROUSEL children) → wait for FINISHED → POST /{user}/threads_publish
//! Tokens: long-lived (60 days), refreshed with th_refresh_token.
//!
//! Env: OC_THREADS_CLIENT_ID, OC_THREADS_CLIENT_SECRET.

use std::time::Duration;

use serde_json::Value;

use super::{first_media, kv, opt_str, or_str, str_or_empty, tokens_from};
use crate::http::{Fetched, Http, Req, expires_in, js_string, non_null, truthy, with_query};
use crate::types::{
    AdapterFuture, AuthKind, AuthorizeParams, ConnectedAccount, ExchangeParams, Limits, PostKind, PublishContext, PublishError, PublishResult, SocialAdapter,
    Tokens,
};

const API: &str = "https://graph.threads.net";
const V: &str = "https://graph.threads.net/v1.0";

pub struct Threads;
pub static THREADS: Threads = Threads;

static LIMITS: Limits = Limits {
    max_chars: 500,
    images: 20,
    video: true,
    image_types: &["image/png", "image/jpeg"],
    max_image_bytes: Some(8 * 1024 * 1024),
};

fn id(http: &Http) -> String {
    http.env_or_empty("OC_THREADS_CLIENT_ID")
}
fn secret(http: &Http) -> String {
    http.env_or_empty("OC_THREADS_CLIENT_SECRET")
}

async fn post(
    http: &Http,
    url: &str,
    token: &str,
    data: Vec<(String, String)>,
) -> Result<String, PublishError> {
    let r: Fetched = http.call(url, Req::post().bearer(token).form(data)).await?;
    Ok(js_string(r.body.get("id")))
}

async fn ready(http: &Http, container: &str, token: &str) -> Result<(), PublishError> {
    let url = with_query(
        &format!("{V}/{container}"),
        &[("fields", Some("status,error_message"))],
    );
    http.poll(
        Some("Threads"),
        Duration::from_millis(3000),
        None,
        || async {
            let r = http.call(&url, Req::get().bearer(token)).await?;
            let status = r.body.get("status");
            if !truthy(status) {
                return Ok(Some(()));
            }
            match status.and_then(Value::as_str) {
                Some("FINISHED" | "PUBLISHED") => Ok(Some(())),
                Some("ERROR" | "EXPIRED") => {
                    let why = non_null(r.body.get("error_message"))
                        .or(status)
                        .map(|v| js_string(Some(v)))
                        .unwrap_or_default();
                    Err(PublishError::new(format!(
                        "Threads could not process the media: {why}"
                    )))
                }
                _ => Ok(None),
            }
        },
    )
    .await
}

async fn publish(http: &Http, ctx: &PublishContext) -> Result<PublishResult, PublishError> {
    let user = &ctx.account.external_id;
    let token = &ctx.tokens.access_token;
    let threads_url = format!("{V}/{user}/threads");
    let container = if ctx.kind == PostKind::Video {
        let v = first_media(ctx, "url")?;
        post(
            http,
            &threads_url,
            token,
            kv(&[
                ("media_type", "VIDEO"),
                ("video_url", &v.url),
                ("text", &ctx.text),
            ]),
        )
        .await?
    } else if ctx.media.len() == 1 {
        post(
            http,
            &threads_url,
            token,
            kv(&[
                ("media_type", "IMAGE"),
                ("image_url", &ctx.media[0].url),
                ("text", &ctx.text),
            ]),
        )
        .await?
    } else {
        let mut children = Vec::new();
        for m in ctx.media.iter().take(20) {
            children.push(
                post(
                    http,
                    &threads_url,
                    token,
                    kv(&[
                        ("media_type", "IMAGE"),
                        ("image_url", &m.url),
                        ("is_carousel_item", "true"),
                    ]),
                )
                .await?,
            );
        }
        post(
            http,
            &threads_url,
            token,
            kv(&[
                ("media_type", "CAROUSEL"),
                ("children", &children.join(",")),
                ("text", &ctx.text),
            ]),
        )
        .await?
    };
    ready(http, &container, token).await?;
    let pub_id = post(
        http,
        &format!("{V}/{user}/threads_publish"),
        token,
        kv(&[("creation_id", &container)]),
    )
    .await?;
    let link_url = with_query(&format!("{V}/{pub_id}"), &[("fields", Some("permalink"))]);
    let link = http.call(&link_url, Req::get().bearer(token)).await.ok();
    let url = link
        .as_ref()
        .and_then(|l| non_null(l.body.get("permalink")))
        .map(|v| js_string(Some(v)));
    Ok(PublishResult {
        url,
        external_id: Some(pub_id),
        note: None,
    })
}

fn authorize_url(http: &Http, p: &AuthorizeParams) -> String {
    let cid = id(http);
    with_query(
        "https://threads.net/oauth/authorize",
        &[
            ("client_id", Some(cid.as_str())),
            ("redirect_uri", Some(p.redirect_uri.as_str())),
            ("scope", Some("threads_basic,threads_content_publish")),
            ("response_type", Some("code")),
            ("state", Some(p.state.as_str())),
        ],
    )
}

async fn exchange(http: &Http, p: &ExchangeParams) -> Result<Vec<ConnectedAccount>, PublishError> {
    let (cid, cs) = (id(http), secret(http));
    let short = http
        .call(
            &format!("{API}/oauth/access_token"),
            Req::post().form(kv(&[
                ("client_id", &cid),
                ("client_secret", &cs),
                ("grant_type", "authorization_code"),
                ("redirect_uri", &p.redirect_uri),
                ("code", &p.code),
            ])),
        )
        .await?;
    let short_token = opt_str(short.body.get("access_token"));
    let long_url = with_query(
        &format!("{API}/access_token"),
        &[
            ("grant_type", Some("th_exchange_token")),
            ("client_secret", Some(cs.as_str())),
            ("access_token", short_token.as_deref()),
        ],
    );
    let long = http.call(&long_url, Req::get()).await?;
    let token = str_or_empty(long.body.get("access_token"));
    let me_url = with_query(
        &format!("{V}/me"),
        &[("fields", Some("id,username,name,threads_profile_picture_url"))],
    );
    let me = http.call(&me_url, Req::get().bearer(&token)).await?;
    let external = non_null(me.body.get("id")).or(short.body.get("user_id"));
    Ok(vec![ConnectedAccount {
        external_id: js_string(external),
        display_name: or_str(&[me.body.get("name"), me.body.get("username")], "Threads"),
        handle: opt_str(me.body.get("username")),
        avatar_url: opt_str(me.body.get("threads_profile_picture_url")),
        tokens: Tokens {
            access_token: token,
            refresh_token: None,
            expires_at: expires_in(long.body.get("expires_in")),
            extra: None,
        },
        meta: None,
    }])
}

impl SocialAdapter for Threads {
    fn id(&self) -> &'static str {
        "threads"
    }
    fn label(&self) -> &'static str {
        "Threads"
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
        &["OC_THREADS_CLIENT_ID", "OC_THREADS_CLIENT_SECRET"]
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
            // Long-lived tokens refresh themselves (while still valid) into a new 60-day token.
            let url = with_query(
                &format!("{API}/refresh_access_token"),
                &[
                    ("grant_type", Some("th_refresh_token")),
                    ("access_token", Some(t.access_token.as_str())),
                ],
            );
            let r = http.call(&url, Req::get()).await?;
            let mut next = tokens_from(&r.body);
            next.refresh_token = None;
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
