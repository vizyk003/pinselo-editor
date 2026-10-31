//! Facebook Pages + Instagram professional accounts (Meta Graph API).
//!
//!   Facebook: POST /{page}/photos (url) · multi-photo: unpublished photos + /{page}/feed attached_media ·
//!             video: POST graph-video /{page}/videos (file_url)
//!   Instagram: POST /{ig}/media (image_url | REELS video_url | CAROUSEL children) → /{ig}/media_publish
//!
//! Env: OC_FACEBOOK_CLIENT_ID, OC_FACEBOOK_CLIENT_SECRET (Meta app), optional OC_FACEBOOK_GRAPH_VERSION.

use std::time::Duration;

use serde_json::{Map, Value, json};

use super::{first_media, kv, opt_str, or_str, str_or_empty};
use crate::http::{Fetched, Http, Req, js_string, js_stringify, non_null, truthy, with_query};
use crate::types::{
    AdapterFuture, AuthKind, AuthorizeParams, ConnectedAccount, ExchangeParams, Limits, PostKind,
    PublishContext, PublishError, PublishResult, SocialAdapter, Tokens,
};

const ENV: &[&str] = &["OC_FACEBOOK_CLIENT_ID", "OC_FACEBOOK_CLIENT_SECRET"];

fn version(http: &Http) -> String {
    http.env("OC_FACEBOOK_GRAPH_VERSION")
        .unwrap_or_else(|| "v23.0".into())
}
fn graph(http: &Http) -> String {
    format!("https://graph.facebook.com/{}", version(http))
}
fn graph_video(http: &Http) -> String {
    format!("https://graph-video.facebook.com/{}", version(http))
}
fn configured(http: &Http) -> bool {
    !id(http).is_empty() && !secret(http).is_empty()
}

const FB_SCOPES: &[&str] = &[
    "pages_show_list",
    "pages_read_engagement",
    "pages_manage_posts",
    "business_management",
];
const IG_SCOPES: &[&str] = &[
    "pages_show_list",
    "business_management",
    "instagram_basic",
    "instagram_content_publish",
];

fn id(http: &Http) -> String {
    http.env_or_empty("OC_FACEBOOK_CLIENT_ID")
}
fn secret(http: &Http) -> String {
    http.env_or_empty("OC_FACEBOOK_CLIENT_SECRET")
}

/// Code → short-lived user token → long-lived user token (~60 days); Page tokens derived from it never
/// expire.
async fn user_token(http: &Http, p: &ExchangeParams) -> Result<Option<String>, PublishError> {
    let (cid, cs) = (id(http), secret(http));
    let url = format!("{}/oauth/access_token", graph(http));
    let short = http
        .call(
            &with_query(
                &url,
                &[
                    ("client_id", Some(cid.as_str())),
                    ("client_secret", Some(cs.as_str())),
                    ("redirect_uri", Some(p.redirect_uri.as_str())),
                    ("code", Some(p.code.as_str())),
                ],
            ),
            Req::get(),
        )
        .await?;
    let short_token = opt_str(short.body.get("access_token"));
    let long = http
        .call(
            &with_query(
                &url,
                &[
                    ("grant_type", Some("fb_exchange_token")),
                    ("client_id", Some(cid.as_str())),
                    ("client_secret", Some(cs.as_str())),
                    ("fb_exchange_token", short_token.as_deref()),
                ],
            ),
            Req::get(),
        )
        .await?;
    Ok(opt_str(long.body.get("access_token")))
}

/// The Pages the user manages (`/me/accounts`), with each Page's linked Instagram account.
async fn pages(http: &Http, token: Option<&str>) -> Result<Vec<Value>, PublishError> {
    let url = with_query(
        &format!("{}/me/accounts", graph(http)),
        &[
            (
                "fields",
                Some(
                    "id,name,access_token,picture{url},instagram_business_account{id,username,name,profile_picture_url}",
                ),
            ),
            ("limit", Some("100")),
        ],
    );
    let r = http
        .call(&url, Req::get().bearer(token.unwrap_or("undefined")))
        .await?;
    Ok(match non_null(r.body.get("data")) {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    })
}

fn authorize(http: &Http, scopes: &[&str], p: &AuthorizeParams) -> String {
    let cid = id(http);
    let scope = scopes.join(",");
    with_query(
        &format!("https://www.facebook.com/{}/dialog/oauth", version(http)),
        &[
            ("client_id", Some(cid.as_str())),
            ("redirect_uri", Some(p.redirect_uri.as_str())),
            ("state", Some(p.state.as_str())),
            ("response_type", Some("code")),
            ("scope", Some(scope.as_str())),
        ],
    )
}

fn page_tokens(pg: &Value) -> Tokens {
    Tokens {
        access_token: str_or_empty(pg.get("access_token")),
        ..Tokens::default()
    }
}

async fn fb_exchange(
    http: &Http,
    p: &ExchangeParams,
) -> Result<Vec<ConnectedAccount>, PublishError> {
    let token = user_token(http, p).await?;
    let list = pages(http, token.as_deref()).await?;
    if list.is_empty() {
        return Err(PublishError::new(
            "No Facebook Pages found. Make sure you manage a Page and granted access to it.",
        ));
    }
    Ok(list
        .iter()
        .map(|pg| {
            // `pg.picture?.data?.url`
            let avatar = non_null(pg.get("picture"))
                .and_then(|x| non_null(x.get("data")))
                .and_then(|x| x.get("url"));
            ConnectedAccount {
                external_id: str_or_empty(pg.get("id")),
                display_name: str_or_empty(pg.get("name")),
                handle: None,
                avatar_url: opt_str(avatar),
                tokens: page_tokens(pg),
                meta: None,
            }
        })
        .collect())
}

async fn ig_exchange(
    http: &Http,
    p: &ExchangeParams,
) -> Result<Vec<ConnectedAccount>, PublishError> {
    let token = user_token(http, p).await?;
    let list: Vec<Value> = pages(http, token.as_deref())
        .await?
        .into_iter()
        .filter(|pg| truthy(pg.get("instagram_business_account")))
        .collect();
    if list.is_empty() {
        return Err(PublishError::new(
            "No Instagram professional account is linked to your Facebook Pages.",
        ));
    }
    Ok(list
        .iter()
        .map(|pg| {
            let ig = &pg["instagram_business_account"];
            let mut meta = Map::new();
            if let Some(page_id) = pg.get("id") {
                meta.insert("pageId".into(), page_id.clone());
            }
            ConnectedAccount {
                external_id: str_or_empty(ig.get("id")),
                display_name: or_str(&[ig.get("name"), ig.get("username"), pg.get("name")], ""),
                handle: opt_str(ig.get("username")),
                avatar_url: opt_str(ig.get("profile_picture_url")),
                tokens: page_tokens(pg),
                meta: Some(meta),
            }
        })
        .collect())
}

async fn post(
    http: &Http,
    url: &str,
    token: &str,
    data: Vec<(String, String)>,
) -> Result<Fetched, PublishError> {
    http.call(url, Req::post().bearer(token).form(data)).await
}

fn id_of(r: &Fetched) -> String {
    js_string(r.body.get("id"))
}

// ---------------------------------------------------------------- Facebook Pages

pub struct Facebook;
pub static FACEBOOK: Facebook = Facebook;

static FB_LIMITS: Limits = Limits {
    max_chars: 63206,
    images: 10,
    video: true,
    image_types: &["image/png", "image/jpeg"],
    max_image_bytes: None,
};

async fn fb_publish(http: &Http, ctx: &PublishContext) -> Result<PublishResult, PublishError> {
    let page = &ctx.account.external_id;
    let token = &ctx.tokens.access_token;
    let g = graph(http);
    if ctx.kind == PostKind::Video {
        let v = first_media(ctx, "url")?;
        let url = format!("{}/{page}/videos", graph_video(http));
        let r = post(
            http,
            &url,
            token,
            kv(&[
                ("file_url", &v.url),
                ("description", &ctx.text),
                ("published", "true"),
            ]),
        )
        .await?;
        let vid = id_of(&r);
        return Ok(PublishResult {
            url: Some(format!("https://www.facebook.com/{page}/videos/{vid}")),
            external_id: Some(vid),
            note: None,
        });
    }
    if ctx.media.len() == 1 {
        let url = format!("{g}/{page}/photos");
        let r = post(
            http,
            &url,
            token,
            kv(&[
                ("url", &ctx.media[0].url),
                ("message", &ctx.text),
                ("published", "true"),
            ]),
        )
        .await?;
        let post_id = js_string(non_null(r.body.get("post_id")).or(r.body.get("id")));
        return Ok(PublishResult {
            url: Some(format!("https://www.facebook.com/{post_id}")),
            external_id: Some(post_id),
            note: None,
        });
    }
    // Multi-photo post: upload each photo unpublished, then one feed post attaching them all.
    let mut ids = Vec::new();
    for m in &ctx.media {
        let r = post(
            http,
            &format!("{g}/{page}/photos"),
            token,
            kv(&[("url", &m.url), ("published", "false")]),
        )
        .await?;
        ids.push(id_of(&r));
    }
    let mut data = vec![("message".to_string(), ctx.text.clone())];
    for (i, fbid) in ids.iter().enumerate() {
        data.push((
            format!("attached_media[{i}]"),
            js_stringify(&json!({ "media_fbid": fbid })),
        ));
    }
    let r = post(http, &format!("{g}/{page}/feed"), token, data).await?;
    let post_id = id_of(&r);
    Ok(PublishResult {
        url: Some(format!("https://www.facebook.com/{post_id}")),
        external_id: Some(post_id),
        note: None,
    })
}

impl SocialAdapter for Facebook {
    fn id(&self) -> &'static str {
        "facebook"
    }
    fn label(&self) -> &'static str {
        "Facebook Page"
    }
    fn auth(&self) -> AuthKind {
        AuthKind::OAuth
    }
    fn authorize_url(&self, http: &Http, p: &AuthorizeParams) -> Option<String> {
        Some(authorize(http, FB_SCOPES, p))
    }
    fn exchange<'a>(
        &'a self,
        http: &'a Http,
        p: &'a ExchangeParams,
    ) -> Option<AdapterFuture<'a, Vec<ConnectedAccount>>> {
        Some(Box::pin(fb_exchange(http, p)))
    }
    fn env(&self) -> &'static [&'static str] {
        ENV
    }
    fn configured(&self, http: &Http) -> bool {
        configured(http)
    }
    fn limits(&self) -> &Limits {
        &FB_LIMITS
    }
    fn publish<'a>(
        &'a self,
        http: &'a Http,
        ctx: &'a PublishContext,
    ) -> AdapterFuture<'a, PublishResult> {
        Box::pin(fb_publish(http, ctx))
    }
}

// ---------------------------------------------------------------- Instagram

pub struct Instagram;
pub static INSTAGRAM: Instagram = Instagram;

// Feed images must be JPEG; aspect ratio 4:5 … 1.91:1. Carousels hold up to 10 items.
static IG_LIMITS: Limits = Limits {
    max_chars: 2200,
    images: 10,
    video: true,
    image_types: &["image/jpeg"],
    max_image_bytes: Some(8 * 1024 * 1024),
};

/// Wait for a media container to finish processing (videos; images are usually instant).
async fn ig_ready(http: &Http, container: &str, token: &str) -> Result<(), PublishError> {
    let url = with_query(
        &format!("{}/{container}", graph(http)),
        &[("fields", Some("status_code,status"))],
    );
    http.poll(
        Some("Instagram"),
        Duration::from_millis(4000),
        None,
        || async {
            let r = http.call(&url, Req::get().bearer(token)).await?;
            match r.body.get("status_code") {
                None => Ok(Some(())),
                Some(Value::String(s)) if s == "FINISHED" || s == "PUBLISHED" => Ok(Some(())),
                Some(Value::String(s)) if s == "ERROR" || s == "EXPIRED" => {
                    let why = non_null(r.body.get("status"))
                        .map(|v| js_string(Some(v)))
                        .unwrap_or_else(|| s.clone());
                    Err(PublishError::new(format!(
                        "Instagram could not process the media ({why})"
                    )))
                }
                _ => Ok(None),
            }
        },
    )
    .await
}

async fn ig_publish(http: &Http, ctx: &PublishContext) -> Result<PublishResult, PublishError> {
    let ig = &ctx.account.external_id;
    let token = &ctx.tokens.access_token;
    let g = graph(http);
    let media_url = format!("{g}/{ig}/media");
    let container = if ctx.kind == PostKind::Video {
        let v = first_media(ctx, "url")?;
        let data = kv(&[
            ("media_type", "REELS"),
            ("video_url", &v.url),
            ("caption", &ctx.text),
            ("share_to_feed", "true"),
        ]);
        let c = id_of(&post(http, &media_url, token, data).await?);
        ig_ready(http, &c, token).await?;
        c
    } else if ctx.media.len() == 1 {
        let data = kv(&[("image_url", &ctx.media[0].url), ("caption", &ctx.text)]);
        id_of(&post(http, &media_url, token, data).await?)
    } else {
        let mut children = Vec::new();
        for m in ctx.media.iter().take(10) {
            let r = post(
                http,
                &media_url,
                token,
                kv(&[("image_url", &m.url), ("is_carousel_item", "true")]),
            )
            .await?;
            children.push(id_of(&r));
        }
        let data = kv(&[
            ("media_type", "CAROUSEL"),
            ("children", &children.join(",")),
            ("caption", &ctx.text),
        ]);
        let c = id_of(&post(http, &media_url, token, data).await?);
        ig_ready(http, &c, token).await?;
        c
    };
    let publish = post(
        http,
        &format!("{g}/{ig}/media_publish"),
        token,
        kv(&[("creation_id", &container)]),
    )
    .await?;
    let media_id = id_of(&publish);
    let link_url = with_query(&format!("{g}/{media_id}"), &[("fields", Some("permalink"))]);
    let link = http.call(&link_url, Req::get().bearer(token)).await.ok();
    let url = link
        .as_ref()
        .and_then(|l| non_null(l.body.get("permalink")))
        .map(|v| js_string(Some(v)));
    Ok(PublishResult {
        url,
        external_id: Some(media_id),
        note: None,
    })
}

impl SocialAdapter for Instagram {
    fn id(&self) -> &'static str {
        "instagram"
    }
    fn label(&self) -> &'static str {
        "Instagram"
    }
    fn auth(&self) -> AuthKind {
        AuthKind::OAuth
    }
    fn authorize_url(&self, http: &Http, p: &AuthorizeParams) -> Option<String> {
        Some(authorize(http, IG_SCOPES, p))
    }
    fn exchange<'a>(
        &'a self,
        http: &'a Http,
        p: &'a ExchangeParams,
    ) -> Option<AdapterFuture<'a, Vec<ConnectedAccount>>> {
        Some(Box::pin(ig_exchange(http, p)))
    }
    fn env(&self) -> &'static [&'static str] {
        ENV
    }
    fn configured(&self, http: &Http) -> bool {
        configured(http)
    }
    fn limits(&self) -> &Limits {
        &IG_LIMITS
    }
    fn publish<'a>(
        &'a self,
        http: &'a Http,
        ctx: &'a PublishContext,
    ) -> AdapterFuture<'a, PublishResult> {
        Box::pin(ig_publish(http, ctx))
    }
}
