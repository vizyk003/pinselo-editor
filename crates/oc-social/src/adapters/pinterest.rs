//! Pinterest API v5. Scopes: boards:read pins:read pins:write user_accounts:read.
//!   image pin:     POST /v5/pins media_source {source_type: image_url}
//!   carousel:      media_source {source_type: multiple_image_urls, items[]}
//!   video pin:     POST /v5/media → multipart upload to upload_url → poll /v5/media/{id} → pin with video_id
//! Pins need a board: the post's options.pinterestBoardId, else the account's first board.
//!
//! Env: OC_PINTEREST_CLIENT_ID, OC_PINTEREST_CLIENT_SECRET.

use std::time::Duration;

use serde_json::{Map, Value, json};

use super::{first_media, opt_str, or_str, str_or_empty, tokens_from};
use crate::http::{
    Http, Part, Req, form, js_string, non_null, prop, slice_utf16, truthy, with_query,
};
use crate::types::{
    AdapterFuture, AuthKind, AuthorizeParams, ConnectedAccount, ExchangeParams, Limits, PostKind, PublishContext, PublishError, PublishResult, SocialAdapter,
    Tokens,
};

const API: &str = "https://api.pinterest.com/v5";

pub struct Pinterest;
pub static PINTEREST: Pinterest = Pinterest;

static LIMITS: Limits = Limits {
    max_chars: 500,
    images: 5,
    video: true,
    image_types: &["image/png", "image/jpeg"],
    max_image_bytes: Some(20 * 1024 * 1024),
};

fn id(http: &Http) -> String {
    http.env_or_empty("OC_PINTEREST_CLIENT_ID")
}
fn secret(http: &Http) -> String {
    http.env_or_empty("OC_PINTEREST_CLIENT_SECRET")
}

async fn token_request(http: &Http, data: &[(&str, &str)]) -> Result<Tokens, PublishError> {
    let pairs: Vec<(&str, Option<&str>)> = data.iter().map(|(k, v)| (*k, Some(*v))).collect();
    let req = Req::post()
        .basic(&id(http), &secret(http))
        .form(form(&pairs));
    let r = http.call(&format!("{API}/oauth/token"), req).await?;
    Ok(tokens_from(&r.body))
}

fn is_http_link(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

async fn publish(http: &Http, ctx: &PublishContext) -> Result<PublishResult, PublishError> {
    let token = &ctx.tokens.access_token;
    let h = || {
        Req::post()
            .bearer(token)
            .header("content-type", "application/json")
    };
    let first_board = match ctx.account.meta.get("boards") {
        Some(Value::Array(b)) => b.first().and_then(|x| x.get("id")).cloned(),
        _ => None,
    };
    let board_id = ctx
        .options
        .get("pinterestBoardId")
        .filter(|v| truthy(Some(v)))
        .cloned()
        .or(first_board)
        .filter(|v| truthy(Some(v)));
    let Some(board_id) = board_id else {
        return Err(PublishError::new(
            "Create a Pinterest board first — pins must be saved to a board.",
        ));
    };
    let media_source = if ctx.kind == PostKind::Video {
        let v = first_media(ctx, "bytes")?;
        let mut req = h();
        req = req.json(&json!({ "media_type": "video" }));
        let reg = http.call(&format!("{API}/media"), req).await?;
        let media_id = js_string(reg.body.get("media_id"));
        let upload_url = js_string(reg.body.get("upload_url"));
        let mut parts = Vec::new();
        if let Some(Value::Object(params)) = reg.body.get("upload_parameters") {
            for (k, val) in params {
                parts.push(Part::Text {
                    name: k.clone(),
                    value: js_string(Some(val)),
                });
            }
        }
        parts.push(Part::File {
            name: "file".into(),
            bytes: v.bytes.clone(),
            mime: v.mime.clone(),
            filename: v.filename.clone(),
        });
        http.call(
            &upload_url,
            Req::post()
                .multipart(parts)
                .timeout(Duration::from_secs(600)),
        )
        .await?;
        let status_url = format!("{API}/media/{media_id}");
        http.poll(
            Some("Pinterest"),
            Duration::from_millis(3000),
            None,
            || async {
                let r = http.call(&status_url, Req::get().bearer(token)).await?;
                match r.body.get("status").and_then(Value::as_str) {
                    Some("succeeded") => Ok(Some(())),
                    Some("failed") => {
                        Err(PublishError::new("Pinterest could not process the video"))
                    }
                    _ => Ok(None),
                }
            },
        )
        .await?;
        json!({ "source_type": "video_id", "media_id": reg.body.get("media_id"), "cover_image_key_frame_time": 0 })
    } else if ctx.media.len() == 1 {
        json!({ "source_type": "image_url", "url": ctx.media[0].url })
    } else {
        let title = slice_utf16(&ctx.title, 100);
        let items: Vec<Value> = ctx
            .media
            .iter()
            .take(5)
            .map(|m| json!({ "url": m.url, "title": title }))
            .collect();
        json!({ "source_type": "multiple_image_urls", "items": items })
    };
    let mut body = Map::new();
    body.insert("board_id".into(), board_id);
    body.insert("title".into(), json!(slice_utf16(&ctx.title, 100)));
    body.insert("description".into(), json!(slice_utf16(&ctx.text, 500)));
    if let Some(Value::String(link)) = ctx.options.get("link")
        && is_http_link(link)
    {
        body.insert("link".into(), json!(link));
    }
    body.insert("alt_text".into(), json!(slice_utf16(&ctx.title, 500)));
    body.insert("media_source".into(), media_source);
    let r = http
        .call(&format!("{API}/pins"), h().json(&Value::Object(body)))
        .await?;
    let pin = js_string(prop(Some(&r.body), "id")?);
    Ok(PublishResult {
        url: Some(format!("https://www.pinterest.com/pin/{pin}/")),
        external_id: Some(pin),
        note: None,
    })
}

fn authorize_url(http: &Http, p: &AuthorizeParams) -> String {
    let cid = id(http);
    with_query(
        "https://www.pinterest.com/oauth/",
        &[
            ("client_id", Some(cid.as_str())),
            ("redirect_uri", Some(p.redirect_uri.as_str())),
            ("response_type", Some("code")),
            ("scope", Some("boards:read,pins:read,pins:write,user_accounts:read")),
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
            ("continuous_refresh", "true"),
        ],
    )
    .await?;
    let token = tokens.access_token.clone();
    let me = http
        .call(&format!("{API}/user_account"), Req::get().bearer(&token))
        .await?;
    let boards_url = with_query(&format!("{API}/boards"), &[("page_size", Some("100"))]);
    let boards = http.call(&boards_url, Req::get().bearer(&token)).await?;
    let items: Vec<Value> = match non_null(boards.body.get("items")) {
        Some(Value::Array(a)) => a
            .iter()
            .map(|b| {
                let mut m = Map::new();
                for k in ["id", "name"] {
                    if let Some(v) = b.get(k) {
                        m.insert(k.into(), v.clone());
                    }
                }
                Value::Object(m)
            })
            .collect(),
        _ => Vec::new(),
    };
    let username = me.body.get("username");
    let mut meta = Map::new();
    meta.insert("boards".into(), Value::Array(items));
    Ok(vec![ConnectedAccount {
        external_id: str_or_empty(username),
        display_name: or_str(&[me.body.get("business_name"), username], ""),
        handle: opt_str(username),
        avatar_url: opt_str(me.body.get("profile_image")),
        tokens,
        meta: Some(meta),
    }])
}

impl SocialAdapter for Pinterest {
    fn id(&self) -> &'static str {
        "pinterest"
    }
    fn label(&self) -> &'static str {
        "Pinterest"
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
        &["OC_PINTEREST_CLIENT_ID", "OC_PINTEREST_CLIENT_SECRET"]
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
