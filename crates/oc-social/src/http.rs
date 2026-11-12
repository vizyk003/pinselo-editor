//! Small HTTP helpers shared by the adapters (port of services/api/src/social/http.ts).
//!
//! `Http` replaces the global `fetch` + `process.env` of the TS version: a reqwest client, an injectable
//! environment lookup, an optional URL rewrite hook (test seam: point provider hosts at a mock server) and
//! optional poll interval / timeout overrides.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use super::types::PublishError;

pub const UA: &str = "Pinselo/1.0 (+self-hosted)";

/// Default per-request timeout (`AbortSignal.timeout(60_000)`).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// Default `poll` deadline (10 minutes).
pub const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub type EnvLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
pub type UrlRewrite = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// HTTP client + environment used by every adapter.
#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    env: EnvLookup,
    rewrite: Option<UrlRewrite>,
    poll_interval: Option<Duration>,
    poll_timeout: Option<Duration>,
}

impl fmt::Debug for Http {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Http")
            .field("rewrite", &self.rewrite.is_some())
            .field("poll_interval", &self.poll_interval)
            .field("poll_timeout", &self.poll_timeout)
            .finish_non_exhaustive()
    }
}

impl Default for Http {
    fn default() -> Self {
        Http::new()
    }
}

impl Http {
    /// Client backed by the process environment.
    pub fn new() -> Self {
        Http::with_env(|name| std::env::var(name).ok())
    }

    /// Client backed by an arbitrary environment lookup (tests never touch the process env).
    pub fn with_env(lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        Http {
            client: reqwest::Client::builder().build().expect("reqwest client"),
            env: Arc::new(lookup),
            rewrite: None,
            poll_interval: None,
            poll_timeout: None,
        }
    }

    /// Client backed by a fixed map of variables.
    pub fn with_env_map(map: HashMap<String, String>) -> Self {
        Http::with_env(move |name| map.get(name).cloned())
    }

    /// The same client with `vars` taking precedence over its environment (a workspace's own Meta app over
    /// the server-wide one). Every other variable, the URL rewrite and the poll settings are kept.
    pub fn with_env_overlay(mut self, vars: HashMap<String, String>) -> Self {
        let base = self.env.clone();
        self.env = Arc::new(move |name| vars.get(name).cloned().or_else(|| base(name)));
        self
    }

    /// Rewrite every request URL (applied right before sending, also to URLs returned by providers).
    pub fn with_url_rewrite(mut self, f: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
        self.rewrite = Some(Arc::new(f));
        self
    }

    /// Override every poll interval / provider-requested wait (tests).
    pub fn with_poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = Some(d);
        self
    }

    /// Override every poll deadline (tests).
    pub fn with_poll_timeout(mut self, d: Duration) -> Self {
        self.poll_timeout = Some(d);
        self
    }

    /// TS `env(name)`: trimmed value, `None` when unset or blank.
    pub fn env(&self, name: &str) -> Option<String> {
        let v = (self.env)(name)?;
        let t = js_trim(&v);
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    }

    /// `env(name) ?? ""`.
    pub fn env_or_empty(&self, name: &str) -> String {
        self.env(name).unwrap_or_default()
    }

    /// fetch → parsed JSON (or text); `Err(PublishError)` on non-2xx and network failure.
    pub async fn call(&self, url: &str, req: Req) -> Result<Fetched, PublishError> {
        let target = match &self.rewrite {
            Some(f) => f(url),
            None => url.to_string(),
        };
        let mut headers = vec![
            ("user-agent".to_string(), UA.to_string()),
            ("accept".to_string(), "application/json".to_string()),
        ];
        for (k, v) in req.headers {
            set_header(&mut headers, &k, v);
        }
        let mut b = self
            .client
            .request(req.method, &target)
            .timeout(req.timeout.unwrap_or(DEFAULT_TIMEOUT));
        match req.body {
            Body::None => {}
            Body::Text(s) => {
                if !headers.iter().any(|(k, _)| k == "content-type") {
                    headers.push(("content-type".into(), "text/plain;charset=UTF-8".into()));
                }
                b = b.body(s);
            }
            Body::Bytes(bytes) => b = b.body(bytes),
            Body::Form(pairs) => {
                if !headers.iter().any(|(k, _)| k == "content-type") {
                    headers.push((
                        "content-type".into(),
                        "application/x-www-form-urlencoded;charset=UTF-8".into(),
                    ));
                }
                b = b.body(encode_form(&pairs));
            }
            Body::Multipart(parts) => {
                // reqwest sets the multipart content-type with its boundary.
                headers.retain(|(k, _)| k != "content-type");
                let mut form = reqwest::multipart::Form::new();
                for p in parts {
                    form = match p {
                        Part::Text { name, value } => form.text(name, value),
                        Part::File {
                            name,
                            bytes,
                            mime,
                            filename,
                        } => {
                            let part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
                            let part = part.mime_str(&mime).map_err(|e| {
                                PublishError::new(format!("Invalid media type: {e}"))
                            })?;
                            form.part(name, part)
                        }
                    };
                }
                b = b.multipart(form);
            }
        }
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(&v),
            ) else {
                return Err(PublishError::transient(format!(
                    "Network error: Invalid header {k}"
                )));
            };
            map.insert(name, value);
        }
        b = b.headers(map);
        let res = b.send().await.map_err(network_error)?;
        let status = res.status().as_u16();
        let headers = res.headers().clone();
        let text = res.text().await.map_err(network_error)?;
        let ctype = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let body = parse_body(&text, ctype);
        if !(200..300).contains(&status) {
            return Err(classify(status, &error_text(&body, status)));
        }
        Ok(Fetched {
            status,
            headers,
            body,
        })
    }

    /// `sleep(ms)`, shortened by the poll-interval override.
    pub async fn sleep(&self, d: Duration) {
        let d = self.poll_interval.unwrap_or(d);
        if !d.is_zero() {
            tokio::time::sleep(d).await;
        }
    }

    /// Poll until `f` returns a value (or fails); for async media processing on the provider side.
    /// `timeout` defaults to 10 minutes; the timeout error is transient.
    pub async fn poll<T, F, Fut>(
        &self,
        what: Option<&str>,
        interval: Duration,
        timeout: Option<Duration>,
        mut f: F,
    ) -> Result<T, PublishError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<Option<T>, PublishError>>,
    {
        let deadline = Instant::now()
            + self
                .poll_timeout
                .or(timeout)
                .unwrap_or(DEFAULT_POLL_TIMEOUT);
        loop {
            if let Some(v) = f().await? {
                return Ok(v);
            }
            if Instant::now() > deadline {
                return Err(poll_timeout_error(what));
            }
            self.sleep(interval).await;
        }
    }

    /// The deadline `poll` would use (for hand-rolled poll loops).
    pub fn poll_deadline(&self, timeout: Option<Duration>) -> Instant {
        Instant::now()
            + self
                .poll_timeout
                .or(timeout)
                .unwrap_or(DEFAULT_POLL_TIMEOUT)
    }
}

/// `new PublishError("Timed out waiting for … to process the media", true)`.
pub fn poll_timeout_error(what: Option<&str>) -> PublishError {
    PublishError::transient(format!(
        "Timed out waiting for {} to process the media",
        what.unwrap_or("the network")
    ))
}

/// `Network error: <fetch's message>`: undici rejects with "fetch failed" (the cause holds the details) and
/// `AbortSignal.timeout` with "The operation was aborted due to timeout".
fn network_error(e: reqwest::Error) -> PublishError {
    PublishError::transient(format!("Network error: {}", fetch_error_message(&e)))
}

/// The `message` of the error Node's `fetch` rejects with for a failed request.
pub fn fetch_error_message(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "The operation was aborted due to timeout"
    } else {
        "fetch failed"
    }
}

fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    let name = name.to_ascii_lowercase();
    headers.retain(|(k, _)| *k != name);
    headers.push((name, value));
}

/// JSON-or-text body parsing rules of `call`.
pub fn parse_body(text: &str, content_type: &str) -> Value {
    let looks_json =
        content_type.contains("json") || matches!(js_trim(text).chars().next(), Some('[' | '{'));
    if !text.is_empty()
        && looks_json
        && let Ok(v) = serde_json::from_str::<Value>(text)
    {
        return v;
    }
    Value::String(text.to_string())
}

/// Turn a provider error body into a short message.
pub fn error_text(body: &Value, status: u16) -> String {
    if let Value::Object(b) = body {
        match b.get("error") {
            Some(Value::Object(e)) => {
                let v = [e.get("message"), e.get("error_user_msg"), e.get("code")]
                    .into_iter()
                    .flatten()
                    .find(|v| !v.is_null());
                let s = match v {
                    Some(v) => js_string(Some(v)),
                    None => js_stringify(&Value::Object(e.clone())),
                };
                return slice_utf16(&s, 300).to_string();
            }
            Some(Value::String(e)) => {
                let v = [b.get("error_description"), b.get("message")]
                    .into_iter()
                    .flatten()
                    .find(|v| !v.is_null());
                let s = match v {
                    Some(v) => js_string(Some(v)),
                    None => e.clone(),
                };
                return slice_utf16(&s, 300).to_string();
            }
            _ => {}
        }
        if let Some(m) = b.get("message").filter(|v| truthy(Some(v))) {
            return slice_utf16(&js_string(Some(m)), 300).to_string();
        }
        if let Some(d) = b.get("detail").filter(|v| truthy(Some(v))) {
            return slice_utf16(&js_string(Some(d)), 300).to_string();
        }
        if let Some(Value::Array(errs)) = b.get("errors")
            && let Some(first) = errs.first().filter(|v| truthy(Some(v)))
        {
            let s = match first.get("message").filter(|v| !v.is_null()) {
                Some(m) => js_string(Some(m)),
                None => js_stringify(first),
            };
            return slice_utf16(&s, 300).to_string();
        }
    }
    if let Value::String(s) = body
        && !s.is_empty()
    {
        return slice_utf16(s, 300).to_string();
    }
    format!("HTTP {status}")
}

/// HTTP status → PublishError (transient for 408/425/429/5xx, reauth on 401).
pub fn classify(status: u16, message: &str) -> PublishError {
    let transient = status == 408 || status == 425 || status == 429 || status >= 500;
    PublishError {
        message: message.to_string(),
        transient,
        status: Some(status),
        reauth: status == 401,
    }
}

/// A fetched response.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub status: u16,
    pub headers: HeaderMap,
    /// Parsed JSON, or the raw text as `Value::String`.
    pub body: Value,
}

impl Fetched {
    /// `headers.get(name)` (multiple values joined with ", ").
    pub fn header(&self, name: &str) -> Option<String> {
        let vals: Vec<String> = self
            .headers
            .get_all(name)
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect();
        if vals.is_empty() {
            None
        } else {
            Some(vals.join(", "))
        }
    }
}

/// Request body.
#[derive(Debug, Clone, Default)]
pub enum Body {
    #[default]
    None,
    Text(String),
    Bytes(Vec<u8>),
    /// URLSearchParams (application/x-www-form-urlencoded).
    Form(Vec<(String, String)>),
    /// FormData (multipart/form-data).
    Multipart(Vec<Part>),
}

/// One FormData entry.
#[derive(Debug, Clone)]
pub enum Part {
    Text {
        name: String,
        value: String,
    },
    File {
        name: String,
        bytes: Vec<u8>,
        mime: String,
        filename: String,
    },
}

/// `RequestInit` + `timeoutMs`.
#[derive(Debug, Clone)]
pub struct Req {
    pub method: Method,
    /// Lower-case names; later entries override earlier ones.
    pub headers: Vec<(String, String)>,
    pub body: Body,
    pub timeout: Option<Duration>,
}

impl Default for Req {
    fn default() -> Self {
        Req::get()
    }
}

impl Req {
    pub fn new(method: Method) -> Self {
        Req {
            method,
            headers: Vec::new(),
            body: Body::None,
            timeout: None,
        }
    }
    pub fn get() -> Self {
        Req::new(Method::GET)
    }
    pub fn post() -> Self {
        Req::new(Method::POST)
    }
    pub fn put() -> Self {
        Req::new(Method::PUT)
    }
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        set_header(&mut self.headers, name, value.into());
        self
    }
    /// `authorization: Bearer <token>`.
    pub fn bearer(self, token: &str) -> Self {
        let (k, v) = bearer(token);
        self.header(k, v)
    }
    /// `authorization: Basic base64(id:secret)`.
    pub fn basic(self, id: &str, secret: &str) -> Self {
        let (k, v) = basic(id, secret);
        self.header(k, v)
    }
    /// `JSON.stringify(data)` body + `content-type: application/json`.
    pub fn json(mut self, data: &Value) -> Self {
        self.body = Body::Text(js_stringify(data));
        self.header("content-type", "application/json")
    }
    /// URLSearchParams body + `content-type: application/x-www-form-urlencoded`.
    pub fn form(mut self, pairs: Vec<(String, String)>) -> Self {
        self.body = Body::Form(pairs);
        self.header("content-type", "application/x-www-form-urlencoded")
    }
    pub fn bytes(mut self, bytes: Vec<u8>) -> Self {
        self.body = Body::Bytes(bytes);
        self
    }
    pub fn text(mut self, s: impl Into<String>) -> Self {
        self.body = Body::Text(s.into());
        self
    }
    pub fn multipart(mut self, parts: Vec<Part>) -> Self {
        self.body = Body::Multipart(parts);
        self
    }
    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }
}

/// PKCE (RFC 7636): a random verifier (48 bytes, base64url) and its S256 challenge.
pub fn pkce_pair() -> (String, String) {
    use aes_gcm::aead::rand_core::RngCore;
    let mut b = [0u8; 48];
    aes_gcm::aead::OsRng.fill_bytes(&mut b);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    let challenge = pkce_challenge(&verifier);
    (verifier, challenge)
}

/// `pkceChallenge(verifier)`: base64url(sha256(verifier)).
pub fn pkce_challenge(verifier: &str) -> String {
    use sha2::Digest as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

/// `form(data)`: drops `None` values (TS `undefined`), keeps order.
pub fn form<K: AsRef<str>, V: AsRef<str>>(data: &[(K, Option<V>)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, v) in data {
        if let Some(v) = v {
            let k = k.as_ref().to_string();
            // URLSearchParams#set: replace the first occurrence in place.
            if let Some(slot) = out.iter_mut().find(|(x, _)| *x == k) {
                slot.1 = v.as_ref().to_string();
            } else {
                out.push((k, v.as_ref().to_string()));
            }
        }
    }
    out
}

/// `form(...).toString()` (application/x-www-form-urlencoded serializer).
pub fn encode_form(pairs: &[(String, String)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        urlencode_into(&mut out, k);
        out.push('=');
        urlencode_into(&mut out, v);
    }
    out
}

fn urlencode_into(out: &mut String, s: &str) {
    use std::fmt::Write as _;
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            b'*' | b'-' | b'.' | b'_' => out.push(b as char),
            b if b.is_ascii_alphanumeric() => out.push(b as char),
            b => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
}

/// `withQuery(url, data)`.
pub fn with_query<K: AsRef<str>, V: AsRef<str>>(url: &str, data: &[(K, Option<V>)]) -> String {
    let q = encode_form(&form(data));
    if q.is_empty() {
        url.to_string()
    } else {
        format!("{url}{}{q}", if url.contains('?') { '&' } else { '?' })
    }
}

pub fn bearer(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

pub fn basic(id: &str, secret: &str) -> (&'static str, String) {
    (
        "authorization",
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"))
        ),
    )
}

/// `expiresIn(sec)`: seconds-from-now → epoch ms (only for positive numbers).
pub fn expires_in(sec: Option<&Value>) -> Option<f64> {
    match sec {
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|s| *s > 0.0)
            .map(|s| oc_js::now_ms() as f64 + s * 1000.0),
        _ => None,
    }
}

// ---------------------------------------------------------------- JS value semantics

/// `String#trim()`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(oc_js::is_js_whitespace)
}

/// `String(v)` (`None` = `undefined`).
pub fn js_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => oc_js::number_to_string(n.as_f64().unwrap_or(f64::NAN)),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| {
                if x.is_null() {
                    String::new()
                } else {
                    js_string(Some(x))
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `JSON.stringify(v)`.
pub fn js_stringify(v: &Value) -> String {
    oc_js::JsValue::from(v).stringify()
}

/// JS truthiness.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `Number(v)`.
pub fn js_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = js_trim(s);
            if t.is_empty() {
                0.0
            } else if t.contains(|c: char| c.is_ascii_alphabetic() && !matches!(c, 'e' | 'E'))
                && t.trim_start_matches(['+', '-']) != "Infinity"
            {
                f64::NAN
            } else {
                t.parse().unwrap_or(f64::NAN)
            }
        }
        Some(Value::Array(a)) => match a.len() {
            0 => 0.0,
            1 => js_number(Some(&Value::String(js_string(Some(&a[0]))))),
            _ => f64::NAN,
        },
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// JS property access `v.key`: `Err(TypeError)` on `undefined` / `null`, `Ok(None)` (undefined) on
/// primitives or missing keys. The error is transient, as publish.ts wraps non-PublishErrors that way.
pub fn prop<'a>(v: Option<&'a Value>, key: &str) -> Result<Option<&'a Value>, PublishError> {
    match v {
        None => Err(PublishError::transient(format!(
            "Cannot read properties of undefined (reading '{key}')"
        ))),
        Some(Value::Null) => Err(PublishError::transient(format!(
            "Cannot read properties of null (reading '{key}')"
        ))),
        Some(Value::Object(m)) => Ok(m.get(key)),
        Some(Value::Array(a)) => Ok(key.parse::<usize>().ok().and_then(|i| a.get(i))),
        Some(_) => Ok(None),
    }
}

/// `v ?? fallback` helper: `None` for undefined and null.
pub fn non_null(v: Option<&Value>) -> Option<&Value> {
    v.filter(|x| !x.is_null())
}

/// `s.slice(0, n)` in UTF-16 code units (never splits a surrogate pair: a split pair is dropped).
pub fn slice_utf16(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}
