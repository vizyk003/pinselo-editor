//! TikTok Content Posting API. Scopes: user.info.basic video.publish video.upload.
//!   inbox (default): POST /v2/post/publish/inbox/video/init/ → PUT the file → the creator finishes in TikTok
//!   direct:          POST /v2/post/publish/creator_info/query/ → /v2/post/publish/video/init/ → PUT → poll status
//!   photos:          POST /v2/post/publish/content/init/ (PULL_FROM_URL; the media host must be a verified domain)
//!
//! Env: OC_TIKTOK_CLIENT_KEY, OC_TIKTOK_CLIENT_SECRET, optional OC_TIKTOK_MODE=inbox|direct.

use std::time::Duration;

use serde_json::{Map, Value, json};

use super::{first_media, opt_str, or_str, str_or_empty, tokens_from};
use crate::http::{
    Http, Req, form, js_string, non_null, prop, slice_utf16, truthy, with_query,
};
use crate::text::chunk_plan;
use crate::types::{
    AdapterFuture, AuthKind, AuthorizeParams, ConnectedAccount, ExchangeParams, Limits, PostKind, PublishContext, PublishError, PublishResult, SocialAdapter,
    Tokens,
};

const API: &str = "https://open.tiktokapis.com/v2";

pub struct TikTok;
pub static TIKTOK: TikTok = TikTok;

static LIMITS: Limits = Limits {
    max_chars: 2200,
    images: 35,
    video: true,
    image_types: &["image/jpeg"],
    max_image_bytes: None,
};

fn key(http: &Http) -> String {
    http.env_or_empty("OC_TIKTOK_CLIENT_KEY")
}
fn secret(http: &Http) -> String {
    http.env_or_empty("OC_TIKTOK_CLIENT_SECRET")
}

async fn token_request(http: &Http, data: &[(&str, &str)]) -> Result<Tokens, PublishError> {
    let (k, s) = (key(http), secret(http));
    let mut pairs: Vec<(&str, Option<&str>)> = data.iter().map(|(k, v)| (*k, Some(*v))).collect();
    pairs.push(("client_key", Some(&k)));
    pairs.push(("client_secret", Some(&s)));
    let r = http
        .call(
            &format!("{API}/oauth/token/"),
            Req::post().form(form(&pairs)),
        )
        .await?;
    let b = &r.body;
    if !truthy(b.get("access_token")) {
        let msg = non_null(b.get("error_description"))
            .or(non_null(b.get("error")))
            .map(|v| js_string(Some(v)))
            .unwrap_or_else(|| "TikTok did not return a token".into());
        return Err(PublishError::new(msg));
    }
    let mut t = tokens_from(b);
    if let Some(open_id) = b.get("open_id").filter(|v| truthy(Some(v))) {
        t.extra = Some(
            [("openId".to_string(), js_string(Some(open_id)))]
                .into_iter()
                .collect(),
        );
    }
    Ok(t)
}

/// POST a TikTok API call and unwrap `{ data, error }`.
async fn tt(http: &Http, path: &str, token: &str, body: Value) -> Result<Value, PublishError> {
    let req = Req::post()
        .json(&body)
        .bearer(token)
        .header("content-type", "application/json; charset=UTF-8");
    let r = http.call(&format!("{API}{path}"), req).await?;
    if let Some(err) = r.body.get("error").filter(|v| truthy(Some(v))) {
        let code = err.get("code");
        if code.and_then(Value::as_str) != Some("ok") {
            let msg = err.get("message").filter(|m| truthy(Some(m))).or(code);
            return Err(PublishError {
                message: format!("TikTok: {}", js_string(msg)),
                transient: code.and_then(Value::as_str) == Some("rate_limit_exceeded"),
                status: None,
                reauth: false,
            });
        }
    }
    Ok(prop(Some(&r.body), "data")?.cloned().unwrap_or(Value::Null))
}

async fn upload(
    http: &Http,
    upload_url: &str,
    bytes: &[u8],
    mime: &str,
) -> Result<(), PublishError> {
    let plan = chunk_plan(bytes.len());
    for i in 0..plan.count {
        let start = i * plan.chunk_size;
        let end = if i == plan.count - 1 {
            bytes.len()
        } else {
            start + plan.chunk_size
        };
        let range = format!("bytes {start}-{}/{}", end as i64 - 1, bytes.len());
        let req = Req::put()
            .bytes(bytes[start.min(end)..end].to_vec())
            .timeout(Duration::from_secs(600))
            .header("content-type", mime)
            .header("content-range", range);
        http.call(upload_url, req).await?;
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Inbox,
    Direct,
}

fn mode(http: &Http, ctx: &PublishContext) -> Mode {
    match ctx.options.get("tiktokMode").and_then(Value::as_str) {
        Some("direct") => Mode::Direct,
        Some("inbox") => Mode::Inbox,
        _ if http.env("OC_TIKTOK_MODE").as_deref() == Some("direct") => Mode::Direct,
        _ => Mode::Inbox,
    }
}

fn pick_privacy(options: Option<&Value>) -> String {
    let opts: &[Value] = match options {
        Some(Value::Array(a)) => a,
        _ => &[],
    };
    for p in [
        "PUBLIC_TO_EVERYONE",
        "MUTUAL_FOLLOW_FRIENDS",
        "FOLLOWER_OF_CREATOR",
        "SELF_ONLY",
    ] {
        if opts.iter().any(|o| o.as_str() == Some(p)) {
            return p.into();
        }
    }
    non_null(opts.first())
        .map(|v| js_string(Some(v)))
        .unwrap_or_else(|| "SELF_ONLY".into())
}

async fn publish(http: &Http, ctx: &PublishContext) -> Result<PublishResult, PublishError> {
    let token = &ctx.tokens.access_token;
    let m = mode(http, ctx);
    let publish_id: Value = if ctx.kind != PostKind::Video {
        // Photo posts are pulled from our signed media URLs (their domain must be verified in the TikTok app).
        let info = if m == Mode::Direct {
            Some(tt(http, "/post/publish/creator_info/query/", token, json!({})).await?)
        } else {
            None
        };
        let mut post_info = Map::new();
        post_info.insert("title".into(), json!(slice_utf16(&ctx.title, 90)));
        post_info.insert("description".into(), json!(slice_utf16(&ctx.text, 4000)));
        if let Some(info) = &info {
            post_info.insert(
                "privacy_level".into(),
                json!(pick_privacy(prop(Some(info), "privacy_level_options")?)),
            );
        }
        let photos: Vec<&str> = ctx.media.iter().take(35).map(|x| x.url.as_str()).collect();
        let d = tt(
            http,
            "/post/publish/content/init/",
            token,
            json!({
                "post_info": post_info,
                "source_info": { "source": "PULL_FROM_URL", "photo_cover_index": 0, "photo_images": photos },
                "post_mode": if m == Mode::Direct { "DIRECT_POST" } else { "MEDIA_UPLOAD" },
                "media_type": "PHOTO",
            }),
        )
        .await?;
        prop(Some(&d), "publish_id")?
            .cloned()
            .unwrap_or(Value::Null)
    } else {
        let v = first_media(ctx, "bytes")?;
        let plan = chunk_plan(v.bytes.len());
        let source_info = json!({ "source": "FILE_UPLOAD", "video_size": v.bytes.len(), "chunk_size": plan.chunk_size, "total_chunk_count": plan.count });
        let d = if m == Mode::Direct {
            let info = tt(http, "/post/publish/creator_info/query/", token, json!({})).await?;
            let privacy = pick_privacy(prop(Some(&info), "privacy_level_options")?);
            tt(
                http,
                "/post/publish/video/init/",
                token,
                json!({
                    "post_info": {
                        "title": slice_utf16(&ctx.text, 2200), "privacy_level": privacy,
                        "disable_duet": false, "disable_comment": false, "disable_stitch": false,
                    },
                    "source_info": source_info,
                }),
            )
            .await?
        } else {
            tt(
                http,
                "/post/publish/inbox/video/init/",
                token,
                json!({ "source_info": source_info }),
            )
            .await?
        };
        let upload_url = js_string(prop(Some(&d), "upload_url")?);
        upload(http, &upload_url, &v.bytes, &v.mime).await?;
        prop(Some(&d), "publish_id")?
            .cloned()
            .unwrap_or(Value::Null)
    };
    let publish_id_s = js_string(Some(&publish_id));
    if m == Mode::Inbox {
        return Ok(PublishResult {
            url: None,
            external_id: Some(publish_id_s),
            note: Some("Sent to your TikTok inbox — open TikTok to finish posting.".into()),
        });
    }
    let done = http
        .poll(
            Some("TikTok"),
            Duration::from_millis(5000),
            None,
            || async {
                let s = tt(
                    http,
                    "/post/publish/status/fetch/",
                    token,
                    json!({ "publish_id": publish_id }),
                )
                .await?;
                match prop(Some(&s), "status")?.and_then(Value::as_str) {
                    Some("PUBLISH_COMPLETE") => Ok(Some(s)),
                    Some("FAILED") => {
                        let why = non_null(s.get("fail_reason"))
                            .map(|v| js_string(Some(v)))
                            .unwrap_or_else(|| "unknown reason".into());
                        Err(PublishError::new(format!(
                            "TikTok rejected the post: {why}"
                        )))
                    }
                    _ => Ok(None),
                }
            },
        )
        .await?;
    let post_id = match done.get("publicaly_available_post_id") {
        Some(Value::Array(a)) => a.first().cloned(),
        _ => None,
    };
    let post_id = post_id.filter(|p| truthy(Some(p)));
    let handle = ctx.account.handle.as_deref().filter(|h| !h.is_empty());
    Ok(match post_id {
        Some(pid) => {
            let pid = js_string(Some(&pid));
            PublishResult {
                url: handle.map(|h| format!("https://www.tiktok.com/@{h}/video/{pid}")),
                external_id: Some(pid),
                note: None,
            }
        }
        None => PublishResult {
            url: None,
            external_id: Some(publish_id_s),
            note: Some("Posted — TikTok shows unaudited apps’ posts to the creator only.".into()),
        },
    })
}

fn authorize_url(http: &Http, p: &AuthorizeParams) -> String {
    let k = key(http);
    with_query(
        "https://www.tiktok.com/v2/auth/authorize/",
        &[
            ("client_key", Some(k.as_str())),
            ("response_type", Some("code")),
            ("scope", Some("user.info.basic,video.publish,video.upload")),
            ("redirect_uri", Some(p.redirect_uri.as_str())),
            ("state", Some(p.state.as_str())),
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
    let url = with_query(
        &format!("{API}/user/info/"),
        &[("fields", Some("open_id,avatar_url,display_name,username"))],
    );
    let r = http.call(&url, Req::get().bearer(&tokens.access_token)).await?;
    let u = prop(prop(Some(&r.body), "data")?, "user")?;
    let field = |k: &str| prop(u, k);
    Ok(vec![ConnectedAccount {
        external_id: str_or_empty(field("open_id")?),
        display_name: or_str(&[field("display_name")?, field("username")?], "TikTok"),
        handle: opt_str(field("username")?),
        avatar_url: opt_str(field("avatar_url")?),
        tokens,
        meta: None,
    }])
}

impl SocialAdapter for TikTok {
    fn id(&self) -> &'static str {
        "tiktok"
    }
    fn label(&self) -> &'static str {
        "TikTok"
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
        &["OC_TIKTOK_CLIENT_KEY", "OC_TIKTOK_CLIENT_SECRET"]
    }
    fn configured(&self, http: &Http) -> bool {
        !key(http).is_empty() && !secret(http).is_empty()
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
