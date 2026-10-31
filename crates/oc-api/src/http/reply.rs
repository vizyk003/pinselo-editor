//! Replies shaped like Fastify's: `reply.code(n).header(k, v).send(payload)`.
//!
//! * an object payload is serialized with `JSON.stringify` semantics as `application/json; charset=utf-8`;
//! * `send()` without a payload is an empty body (no content type);
//! * bytes keep the content type the route set (`reply.type(...)`), default `application/octet-stream`;
//! * a string payload without a type is `text/plain; charset=utf-8`.

use axum::body::Body;
use axum::http::header::{CONTENT_TYPE, HeaderName, HeaderValue, LOCATION, SET_COOKIE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::http::jsjson;

pub const JSON_TYPE: &str = "application/json; charset=utf-8";

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Body,
}

impl Reply {
    /// `return { ... }` from a handler (200, JSON).
    pub fn json(v: Value) -> Reply {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_TYPE));
        Reply { status: StatusCode::OK, headers, body: Body::from(jsjson::stringify(&v)) }
    }

    /// `reply.code(status).send()`: no body.
    pub fn empty(status: u16) -> Reply {
        Reply { status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), headers: HeaderMap::new(), body: Body::empty() }
    }

    /// `reply.type(content_type).send(buffer)`.
    pub fn bytes(content_type: &str, bytes: impl Into<Body>) -> Reply {
        let mut headers = HeaderMap::new();
        if let Ok(v) = HeaderValue::from_str(content_type) {
            headers.insert(CONTENT_TYPE, v);
        }
        Reply { status: StatusCode::OK, headers, body: bytes.into() }
    }

    /// `reply.send(string)` with no type set.
    pub fn text(s: String) -> Reply {
        Reply::bytes("text/plain; charset=utf-8", s)
    }

    /// `reply.redirect(url)` (302).
    pub fn redirect(url: &str) -> Reply {
        let mut r = Reply::empty(302);
        if let Ok(v) = HeaderValue::from_str(url) {
            r.headers.insert(LOCATION, v);
        }
        r
    }

    /// `reply.code(status)`.
    pub fn status(mut self, status: u16) -> Reply {
        self.status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        self
    }

    /// `reply.header(name, value)` (replaces).
    pub fn header(mut self, name: &str, value: impl AsRef<str>) -> Reply {
        if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value.as_ref())) {
            self.headers.insert(n, v);
        }
        self
    }

    /// Append a `Set-Cookie` header (already serialized, see [`crate::http::cookie`]).
    pub fn cookie(mut self, set_cookie: String) -> Reply {
        if let Ok(v) = HeaderValue::from_str(&set_cookie) {
            self.headers.append(SET_COOKIE, v);
        }
        self
    }

    pub fn cookies(mut self, set_cookies: impl IntoIterator<Item = String>) -> Reply {
        for c in set_cookies {
            self = self.cookie(c);
        }
        self
    }
}

impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        let mut res = Response::new(self.body);
        *res.status_mut() = self.status;
        *res.headers_mut() = self.headers;
        res
    }
}

/// JSON reply with a status: `reply.code(status).send(body)`.
pub fn json(status: u16, v: Value) -> Reply {
    Reply::json(v).status(status)
}
