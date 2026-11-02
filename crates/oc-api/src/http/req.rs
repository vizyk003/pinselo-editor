//! The request as a Fastify handler sees it, extracted in Fastify's order:
//!
//! 1. `onRequest`: the route's rate limit (see [`crate::http::ratelimit`]);
//! 2. content-type parsing (`main.ts` replaces the JSON parser; `text/plain` and `multipart/form-data` are the
//!    other registered types): a body on a GET/HEAD is ignored; no content type and no body → `undefined`;
//!    a body without a content type, an invalid or unknown content type → 415 `Unsupported Media Type`;
//!    `Content-Length` over 10 MiB → 413; an empty JSON body → `{}`; malformed JSON → 400 with V8's message;
//! 3. the handler, which then runs `requireUser` (`preHandler`) and the zod schemas.
//!
//! Every handler takes [`Req`] as its LAST argument (it consumes the body): that is also what applies the
//! route's rate limit, so a handler without it would not be limited.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body as AxumBody;
use axum::extract::{ConnectInfo, FromRequest, MatchedPath, Request};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderName, HeaderValue, TRANSFER_ENCODING};
use axum::http::{HeaderMap, Method};
use serde_json::{Map, Value};

use crate::http::error::ApiError;
use crate::http::{cookie, jsjson, ratelimit};
use crate::state::AppState;

/// Fastify's `bodyLimit` (main.ts: 10 MiB).
pub const BODY_LIMIT: usize = 10 * 1024 * 1024;

/// Headers set before the handler ran (the rate limiter's), merged into whatever response goes out.
#[derive(Clone, Default)]
pub struct Sink(Arc<Mutex<Vec<(HeaderName, HeaderValue)>>>);

impl Sink {
    pub fn set(&self, name: &'static str, value: impl ToString) {
        if let Ok(v) = HeaderValue::from_str(&value.to_string()) {
            self.0.lock().expect("sink").push((HeaderName::from_static(name), v));
        }
    }

    pub fn apply(&self, headers: &mut HeaderMap) {
        for (k, v) in self.0.lock().expect("sink").drain(..) {
            headers.insert(k, v);
        }
    }
}

pub enum Body {
    /// `request.body === undefined`
    None,
    Json(Value),
    Text(String),
    /// `multipart/form-data`: left unread for [`crate::http::multipart`] (behind a mutex so `Req` is `Sync`
    /// and `&Req` can be held across `.await` in handlers).
    Multipart { content_type: String, body: Mutex<Option<AxumBody>> },
}

pub struct Req {
    pub method: Method,
    /// `request.url`: path and query string as received.
    pub url: String,
    pub path: String,
    pub headers: HeaderMap,
    /// `request.ip`: see [`client_ip`].
    pub ip: String,
    /// `request.cookies`.
    pub cookies: HashMap<String, String>,
    /// `request.query`.
    pub query: Value,
    pub body: Body,
}

impl Req {
    /// `request.body` (`None` = `undefined`).
    pub fn body(&self) -> Option<&Value> {
        match &self.body {
            Body::Json(v) => Some(v),
            Body::Text(_) | Body::None | Body::Multipart { .. } => None,
        }
    }

    /// `request.body ?? {}` (routes that tolerate a missing body: `z.object(...).parse(req.body ?? {})`).
    pub fn body_or_empty(&self) -> Value {
        match &self.body {
            Body::Json(Value::Null) | Body::None => Value::Object(Map::new()),
            Body::Json(v) => v.clone(),
            Body::Text(s) => Value::String(s.clone()),
            Body::Multipart { .. } => Value::Object(Map::new()),
        }
    }

    /// The body as zod input: a text body is a string, a missing one `undefined`.
    pub fn body_input(&self) -> Option<Value> {
        match &self.body {
            Body::Json(v) => Some(v.clone()),
            Body::Text(s) => Some(Value::String(s.clone())),
            Body::None | Body::Multipart { .. } => None,
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// `String(req.headers['user-agent'] ?? '')`.
    pub fn user_agent(&self) -> String {
        self.header("user-agent").unwrap_or("").to_owned()
    }

    /// `request.query[name]` as a string (the first value of a repeated parameter is NOT taken: Fastify gives
    /// an array, which TS code compares as a string and fails).
    pub fn query_str(&self, name: &str) -> Option<&str> {
        self.query.get(name).and_then(Value::as_str)
    }

    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }
}

/// An address a proxy of ours may have: the ranges of Caddy's `trusted_proxies static private_ranges`.
fn is_proxy_addr(ip: std::net::IpAddr) -> bool {
    match ip.to_canonical() {
        std::net::IpAddr::V4(a) => a.is_private() || a.is_loopback(),
        std::net::IpAddr::V6(a) => a.is_loopback() || a.segments()[0] & 0xfe00 == 0xfc00,
    }
}

/// `request.ip`: `X-Forwarded-For` counts only when a proxy sent it, and is read from the right, because the client
/// writes the left end. The nearest hop that isn't a proxy of ours is the client; if all are private, the furthest.
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    if peer.is_none_or(|p| is_proxy_addr(p.ip()))
        && let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
    {
        let hops: Vec<(&str, std::net::IpAddr)> = xff.split(',').filter_map(|h| Some((h.trim(), h.trim().parse().ok()?))).collect();
        if let Some((ip, _)) = hops.iter().rev().find(|(_, a)| !is_proxy_addr(*a)).or(hops.first()) {
            return (*ip).to_owned();
        }
    }
    match peer {
        Some(a) => match a.ip() {
            std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().map(|v4| format!("::ffff:{v4}")).unwrap_or_else(|| v6.to_string()),
            ip => ip.to_string(),
        },
        // `app.inject` / tower `oneshot`: Fastify's light-my-request uses 127.0.0.1.
        None => "127.0.0.1".to_owned(),
    }
}

/// Fastify's `querystringParser` (fast-querystring): `+` is a space, repeated keys become arrays.
pub fn parse_query(q: &str) -> Value {
    let mut out = Map::new();
    for (k, v) in form_urlencoded::parse(q.as_bytes()) {
        let (k, v) = (k.into_owned(), Value::String(v.into_owned()));
        match out.get_mut(&k) {
            None => {
                out.insert(k, v);
            }
            Some(Value::Array(a)) => a.push(v),
            Some(prev) => {
                let p = prev.take();
                *prev = Value::Array(vec![p, v]);
            }
        }
    }
    Value::Object(out)
}

/// Fastify `ContentType`: `Some(media type)` (lower-cased `type/subtype`) or `None` when invalid.
pub fn media_type(header: &str) -> Option<String> {
    let token = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&c));
    if header.is_empty() || header == "undefined" {
        return None;
    }
    let (mt, _params) = match header.find(';') {
        Some(i) => (&header[..i], Some(&header[i + 1..])),
        None => (header, None),
    };
    let slash = mt.find('/')?;
    let ty = mt[..slash].trim_start().to_lowercase();
    let sub = mt[slash + 1..].trim_end().to_lowercase();
    (token(&ty) && token(&sub)).then(|| format!("{ty}/{sub}"))
}

fn too_large() -> ApiError {
    ApiError::Status { status: 413, message: "Request body is too large".into(), close: true }
}

fn unsupported() -> ApiError {
    ApiError::status(415, "Unsupported Media Type")
}

impl FromRequest<AppState> for Req {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, ApiError> {
        let (parts, body) = req.into_parts();
        let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
        let ip = client_ip(&parts.headers, peer);

        // 1. onRequest: rate limit.
        if let Some(mp) = parts.extensions.get::<MatchedPath>() {
            let limiter = state.limits.for_route(parts.method.as_str(), mp.as_str());
            let c = limiter.incr(&ratelimit::normalize_ip(&ip), state.now_ms());
            if let Some(sink) = parts.extensions.get::<Sink>() {
                sink.set("x-ratelimit-limit", c.max);
                sink.set("x-ratelimit-remaining", if c.exceeded { 0 } else { c.remaining });
                sink.set("x-ratelimit-reset", c.ttl_seconds);
                if c.exceeded {
                    sink.set("retry-after", c.ttl_seconds);
                }
            }
            if c.exceeded {
                return Err(ApiError::status(429, format!("Rate limit exceeded, retry in {}", ratelimit::format_ms(c.ttl_seconds * 1000))));
            }
        }

        let url = parts.uri.path_and_query().map_or_else(|| parts.uri.path().to_owned(), |p| p.as_str().to_owned());
        let path = parts.uri.path().to_owned();
        let query = parse_query(parts.uri.query().unwrap_or(""));
        let cookies = parts.headers.get("cookie").and_then(|v| v.to_str().ok()).map(cookie::parse).unwrap_or_default();

        // 2. content-type parsing.
        let body = if parts.method == Method::GET || parts.method == Method::HEAD {
            Body::None
        } else {
            let ct = parts.headers.get(CONTENT_TYPE).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
            let len = parts.headers.get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()).map(str::to_owned);
            let empty = !parts.headers.contains_key(TRANSFER_ENCODING) && len.as_deref().is_none_or(|l| l == "0");
            match ct {
                None if empty => Body::None,
                None => return Err(unsupported()),
                Some(ct) => {
                    let Some(media) = media_type(&ct) else { return Err(unsupported()) };
                    match media.as_str() {
                        "application/json" | "text/plain" => {
                            if len.as_deref().and_then(|l| l.parse::<f64>().ok()).is_some_and(|n| n > BODY_LIMIT as f64) {
                                return Err(too_large());
                            }
                            let bytes = axum::body::to_bytes(body, BODY_LIMIT).await.map_err(|_| too_large())?;
                            let text = String::from_utf8_lossy(&bytes).into_owned();
                            if media == "text/plain" {
                                Body::Text(text)
                            } else if text.is_empty() {
                                Body::Json(Value::Object(Map::new()))
                            } else {
                                Body::Json(jsjson::parse(&text).map_err(|m| ApiError::Status { status: 400, message: m, close: true })?)
                            }
                        }
                        "multipart/form-data" => Body::Multipart { content_type: ct, body: Mutex::new(Some(body)) },
                        _ => return Err(unsupported()),
                    }
                }
            }
        };

        Ok(Req { method: parts.method, url, path, headers: parts.headers, ip, cookies, query, body })
    }
}

#[cfg(test)]
mod client_ip_tests {
    use super::*;

    fn ip(xff: &str, peer: &str) -> String {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", xff.parse().unwrap());
        client_ip(&h, Some(peer.parse().unwrap()))
    }

    #[test]
    fn the_client_cannot_choose_its_address() {
        let proxy = "172.18.0.5:4000";
        assert_eq!(ip("203.0.113.9", proxy), "203.0.113.9");
        assert_eq!(ip("1.2.3.4, 203.0.113.9", proxy), "203.0.113.9"); // spoofed left entry
        assert_eq!(ip("1.2.3.4, 203.0.113.9, 10.0.0.2", proxy), "203.0.113.9"); // a load balancer in front of Caddy
        assert_eq!(ip("10.0.0.1, 192.168.0.1", proxy), "10.0.0.1"); // a client on the private network
        assert_eq!(ip("junk, 203.0.113.9, also-junk", proxy), "203.0.113.9");
        assert_eq!(ip("1.2.3.4", "203.0.113.50:4000"), "203.0.113.50"); // no proxy in between: the header is the client's own
        assert_eq!(ip("junk", proxy), "172.18.0.5");
    }
}
