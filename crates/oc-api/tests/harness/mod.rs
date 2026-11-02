//! Replays an HTTP parity transcript (tests/golden/api/<group>.json, written by tools/golden/api.ts from the TS
//! Fastify app) against the Rust router and compares every step. This is the Rust half of
//! tools/golden/api/harness.ts; the two must stay in lock-step (templates, cookie jars, fake services, the
//! normalizer, volatile paths).
//!
//! Each group runs serially (one at a time: the local Postgres has little connection headroom) on its own
//! throwaway database (`oc_db::testing::TestDb`, skipped without `OC_DATABASE_URL`) with a pool of 3, its own
//! object store directory, a pinned clock and an in-process fake for the external services. Mismatches are
//! collected and reported together; the database is dropped even when the replay panics.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use axum::body::Body;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http_body_util::BodyExt as _;
use oc_api::http::jsjson;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tower::ServiceExt as _;

pub const CLOCK: i64 = 1_735_732_800_000; // Date.UTC(2025, 0, 1, 12, 0, 0)
pub const SECRET: &str = "api-golden-secret-0123456789abcdef0123";
pub const WEB: &str = "https://app.example";
pub const API: &str = "https://api.example";
pub const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
pub const BOUNDARY: &str = "----ocgoldenboundary";
const SKIP_HEADERS: [&str; 5] = ["date", "connection", "keep-alive", "transfer-encoding", "content-length"];
const FAKE_HEADERS: [&str; 8] = ["content-type", "x-oc-internal", "authorization", "x-api-key", "anthropic-version", "anthropic-beta", "x-stainless-retry-count", "ocp-apim-subscription-key"];

/// One group at a time (and never alongside another DB-heavy test of this binary).
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn golden(group: &str) -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/golden/api/{group}.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).expect("golden json")
}

fn sha(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

// ---------------------------------------------------------------- canonical comparison

/// `JSON.stringify` with object keys sorted (UTF-16 order).
pub fn canon(v: &Value) -> String {
    match v {
        Value::Array(a) => format!("[{}]", a.iter().map(canon).collect::<Vec<_>>().join(",")),
        Value::Object(m) => {
            let mut kv: Vec<(&String, &Value)> = m.iter().collect();
            kv.sort_by(|a, b| oc_js::cmp_utf16(a.0, b.0));
            format!("{{{}}}", kv.iter().map(|(k, x)| format!("{}:{}", oc_js::stringify_str(k), canon(x))).collect::<Vec<_>>().join(","))
        }
        other => jsjson::stringify(other),
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(&serde_json::from_str::<Value>(&canon(v)).unwrap_or(Value::Null)).unwrap_or_default()
}

// ---------------------------------------------------------------- normalizer (tools/golden/api/harness.ts `Normalizer`)

static TS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})").expect("regex"));
static UUID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$").expect("regex"));
static RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z0-9_-]+").expect("regex"));
static ID16: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9A-Za-z]{16}$").expect("regex"));
static HEX64: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{64}$").expect("regex"));
const SEEDED_UUID: &str = "00000000-0000-4000-8000-";

pub struct Normalizer {
    vars: Vec<(String, String)>,
    counters: HashMap<&'static str, HashMap<String, usize>>,
    fixed_ids: HashSet<String>,
    known_sha: HashSet<String>,
}

impl Normalizer {
    pub fn new(fixed_ids: HashSet<String>, known_sha: HashSet<String>) -> Self {
        Normalizer { vars: vec![], counters: HashMap::new(), fixed_ids, known_sha }
    }

    pub fn set_var(&mut self, name: &str, value: &str) {
        self.vars.retain(|(n, _)| n != name);
        if value.encode_utf16().count() >= 6 {
            self.vars.push((name.to_owned(), value.to_owned()));
        }
        self.vars.sort_by(|a, b| b.1.encode_utf16().count().cmp(&a.1.encode_utf16().count()).then_with(|| oc_js::cmp_utf16(&a.0, &b.0)));
    }

    fn num(&mut self, kind: &'static str, v: &str) -> String {
        let m = self.counters.entry(kind).or_default();
        let n = m.len() + 1;
        let i = *m.entry(v.to_owned()).or_insert(n);
        format!("<{kind}:{i}>")
    }

    pub fn str(&mut self, s: &str) -> String {
        let mut s = s.to_owned();
        for (name, v) in self.vars.clone() {
            s = s.replace(&v, &format!("{{{{{name}}}}}"));
        }
        if ID16.is_match(&s) && s.bytes().any(|c| c.is_ascii_digit()) && s.bytes().any(|c| c.is_ascii_alphabetic()) && !self.fixed_ids.contains(&s) {
            return self.num("id", &s);
        }
        let s = TS.replace_all(&s, |c: &regex::Captures| if c[0].starts_with("2025-") { c[0].to_owned() } else { "<ts>".to_owned() }).into_owned();
        let mut out = String::with_capacity(s.len());
        let mut last = 0;
        for m in RUN.find_iter(&s) {
            out.push_str(&s[last..m.start()]);
            let r = m.as_str();
            let rep = if UUID.is_match(r) {
                if r.starts_with(SEEDED_UUID) { r.to_owned() } else { self.num("uuid", &r.to_lowercase()) }
            } else if r.encode_utf16().count() == 43 {
                self.num("token", r)
            } else if HEX64.is_match(r) && !self.known_sha.contains(r) {
                self.num("sha", r)
            } else {
                r.to_owned()
            };
            out.push_str(&rep);
            last = m.end();
        }
        out.push_str(&s[last..]);
        out
    }

    pub fn value(&mut self, v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(self.str(s)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.value(x)).collect()),
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort_by(|a, b| oc_js::cmp_utf16(a, b));
                let mut out = Map::new();
                for k in keys {
                    let nk = self.str(k);
                    let nv = self.value(&m[k.as_str()]);
                    out.insert(nk, nv);
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }
}

/// `blank(rec, path)`: set the value(s) at a dotted path (`*` = every key/item) to `"<volatile>"`.
pub fn blank(rec: &mut Value, path: &str) {
    fn walk(cur: &mut Value, parts: &[&str]) {
        let Some((p, rest)) = parts.split_first() else { return };
        let keys: Vec<String> = match cur {
            Value::Array(a) if *p == "*" => (0..a.len()).map(|i| i.to_string()).collect(),
            Value::Object(m) if *p == "*" => m.keys().cloned().collect(),
            _ => vec![(*p).to_owned()],
        };
        for k in keys {
            let child = match cur {
                Value::Array(a) => k.parse::<usize>().ok().and_then(|i| a.get_mut(i)),
                Value::Object(m) => m.get_mut(&k),
                _ => None,
            };
            if let Some(c) = child {
                if rest.is_empty() {
                    *c = json!("<volatile>");
                } else {
                    walk(c, rest);
                }
            }
        }
    }
    let parts: Vec<&str> = path.split('.').collect();
    walk(rec, &parts);
}

// ---------------------------------------------------------------- templates

static TEMPLATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{(totp:)?([A-Za-z0-9_.]+)\}\}").expect("regex"));

fn totp(secret: &str) -> String {
    let key = oc_api::routes::account::base32_decode(secret).unwrap_or_default();
    oc_api::routes::account::totp_code(&key, (CLOCK / 1000) as u64)
}

pub fn subst(s: &str, vars: &HashMap<String, String>) -> String {
    TEMPLATE
        .replace_all(s, |c: &regex::Captures| match vars.get(&c[2]) {
            Some(v) if c.get(1).is_some() => totp(v),
            Some(v) => v.clone(),
            None => c[0].to_owned(),
        })
        .into_owned()
}

fn subst_json(v: &Value, vars: &HashMap<String, String>) -> Value {
    match v {
        Value::String(s) => Value::String(subst(s, vars)),
        Value::Array(a) => Value::Array(a.iter().map(|x| subst_json(x, vars)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), subst_json(x, vars))).collect()),
        other => other.clone(),
    }
}

/// `fileBytes(m)`: the file of a multipart step (zero-padded to `padTo`).
fn file_bytes(m: &Value) -> Vec<u8> {
    let mut b = hex::decode(m["hex"].as_str().unwrap_or("")).expect("multipart hex");
    if let Some(n) = m.get("padTo").and_then(Value::as_u64) {
        b.resize((n as usize).max(b.len()), 0);
    }
    b
}

fn multipart_body(m: &Value) -> Vec<u8> {
    let s = |k: &str| m[k].as_str().unwrap_or("").to_owned();
    let mut b = format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n", s("field"), s("filename"), s("contentType")).into_bytes();
    b.extend(file_bytes(m));
    b.extend(format!("\r\n--{BOUNDARY}--\r\n").into_bytes());
    b
}

fn pick(rec: &Value, path: &str) -> Option<Value> {
    if let Some(p) = path.strip_prefix("keys:") {
        let o = pick(rec, p)?;
        let mut keys: Vec<String> = o.as_object()?.keys().cloned().collect();
        keys.sort_by(|a, b| oc_js::cmp_utf16(a, b));
        return keys.into_iter().next().map(Value::String);
    }
    let mut cur = rec;
    for part in path.split('.') {
        cur = match cur {
            Value::Object(m) => m.get(part)?,
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur.clone())
}

fn capture(rec: &Value, spec: &Value) -> Option<String> {
    let from = spec.as_str().or_else(|| spec["from"].as_str())?;
    let v = match pick(rec, from)? {
        Value::String(s) => s,
        Value::Number(n) => jsjson::number_text(&n),
        _ => return None,
    };
    match spec.get("regex").and_then(Value::as_str) {
        None => Some(v),
        Some(re) => {
            let c = Regex::new(re).expect("capture regex").captures(&v)?;
            Some(c.get(1).unwrap_or_else(|| c.get(0).expect("match")).as_str().to_owned())
        }
    }
}

// ---------------------------------------------------------------- bodies

fn image_dims(b: &[u8]) -> Value {
    if b.len() > 24 && b[0] == 0x89 && &b[1..4] == b"PNG" {
        let u = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        return json!([u(16), u(20)]);
    }
    if b.len() > 4 && b[0] == 0xff && b[1] == 0xd8 {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xff {
                return Value::Null;
            }
            let marker = b[i + 1];
            let len = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
            if (0xc0..=0xcf).contains(&marker) && marker != 0xc4 && marker != 0xc8 && marker != 0xcc {
                let u = |j: usize| u16::from_be_bytes([b[j], b[j + 1]]);
                return json!([u(i + 7), u(i + 5)]);
            }
            i += 2 + len;
        }
    }
    Value::Null
}

/// `recordBody(contentType, raw)`.
pub fn record_body(content_type: &str, raw: &[u8]) -> Value {
    if raw.is_empty() {
        return Value::Null;
    }
    let text = || String::from_utf8_lossy(raw).into_owned();
    if content_type.contains("json") {
        return jsjson::parse(&text()).unwrap_or_else(|_| json!({ "$text": text() }));
    }
    if content_type.starts_with("text/") || content_type.contains("svg") || content_type.contains("xml") {
        return json!({ "$text": text() });
    }
    json!({ "$binary": { "size": raw.len(), "sha256": sha(raw), "dims": image_dims(raw) } })
}

// ---------------------------------------------------------------- fake external services

#[derive(Default)]
struct FakeState {
    log: Vec<Value>,
    script: HashMap<String, Vec<Value>>,
    defaults: HashMap<String, Value>,
}

pub struct Fake {
    pub base: String,
    state: Arc<Mutex<FakeState>>,
    inflight: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

fn fake_body(ct: &str, raw: &[u8]) -> Value {
    if raw.is_empty() {
        return Value::Null;
    }
    let text = String::from_utf8_lossy(raw).into_owned();
    if ct.contains("json") {
        return jsjson::parse(&text).unwrap_or_else(|_| json!({ "$text": text }));
    }
    if ct.contains("x-www-form-urlencoded") {
        // Object.fromEntries(new URLSearchParams(…)): a repeated key keeps the last value.
        let mut m = Map::new();
        for (k, v) in form_urlencoded::parse(raw) {
            m.insert(k.into_owned(), Value::String(v.into_owned()));
        }
        return Value::Object(m);
    }
    if ct.starts_with("text/") {
        return json!({ "$text": text });
    }
    json!({ "$binary": { "size": raw.len(), "sha256": sha(raw), "dims": Value::Null } })
}

/// A tiny HTTP/1.1 server (one request per connection) that records requests and replays scripted replies.
async fn serve_fake(listener: tokio::net::TcpListener, state: Arc<Mutex<FakeState>>, inflight: Arc<AtomicUsize>) {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else { return };
        let (state, inflight) = (state.clone(), inflight.clone());
        inflight.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move {
            let _ = handle_fake(&mut sock, &state).await;
            inflight.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

async fn handle_fake(sock: &mut tokio::net::TcpStream, state: &Arc<Mutex<FakeState>>) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let method = parts.next().unwrap_or("GET").to_owned();
    let path = parts.next().unwrap_or("/").to_owned();
    let mut headers: HashMap<String, String> = HashMap::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_owned());
        }
    }
    let len: usize = headers.get("content-length").and_then(|l| l.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    let ct = headers.get("content-type").cloned().unwrap_or_default();
    let mut rec_headers = Map::new();
    for h in FAKE_HEADERS {
        if let Some(v) = headers.get(h) {
            rec_headers.insert(h.to_owned(), json!(v));
        }
    }
    let req_body = fake_body(&ct, &body);
    let route = path.split('?').next().unwrap_or("").to_owned();
    let reply = {
        let mut st = state.lock().expect("fake state");
        st.log.push(json!({ "method": method, "path": path, "headers": rec_headers, "body": req_body }));
        let scripted = st.script.get_mut(&route).filter(|q| !q.is_empty()).map(|q| q.remove(0));
        scripted.or_else(|| st.defaults.get(&route).cloned()).unwrap_or_else(|| json!({ "status": 404, "text": "no fake for this path" }))
    };
    if reply.get("close").and_then(Value::as_bool) == Some(true) {
        let _ = sock.shutdown().await;
        return Ok(());
    }
    let mut out_headers: Vec<(String, String)> = reply.get("headers").and_then(Value::as_object).map(|m| m.iter().map(|(k, v)| (k.to_lowercase(), v.as_str().unwrap_or("").to_owned())).collect()).unwrap_or_default();
    let has = |hs: &Vec<(String, String)>, k: &str| hs.iter().any(|(n, _)| n == k);
    let payload: Vec<u8> = if let Some(t) = reply.get("idToken") {
        if !has(&out_headers, "content-type") {
            out_headers.push(("content-type".into(), "application/json".into()));
        }
        let jwt = sign_jwt(t["pkcs8"].as_str().unwrap_or(""), t["kid"].as_str().unwrap_or(""), &t["claims"]);
        jsjson::stringify(&json!({ "access_token": "golden-access-token", "token_type": "Bearer", "id_token": jwt })).into_bytes()
    } else if reply.get("echoExport").and_then(Value::as_bool) == Some(true) {
        if !has(&out_headers, "content-type") {
            out_headers.push(("content-type".into(), "application/octet-stream".into()));
        }
        let s = |k: &str| match req_body.get(k) {
            Some(Value::String(x)) => x.clone(),
            Some(v) => jsjson::stringify(v),
            None => "undefined".into(),
        };
        format!("EXPORT:{}:{}", s("designId"), s("format")).into_bytes()
    } else if let Some(j) = reply.get("json") {
        if !has(&out_headers, "content-type") {
            out_headers.push(("content-type".into(), "application/json".into()));
        }
        jsjson::stringify(j).into_bytes()
    } else if let Some(h) = reply.get("hex").and_then(Value::as_str) {
        hex::decode(h).unwrap_or_default()
    } else {
        reply.get("text").and_then(Value::as_str).unwrap_or("").as_bytes().to_vec()
    };
    let status = reply.get("status").and_then(Value::as_u64).unwrap_or(200);
    let mut resp = format!("HTTP/1.1 {status} X\r\n");
    for (k, v) in &out_headers {
        resp.push_str(&format!("{k}: {v}\r\n"));
    }
    resp.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n", payload.len()));
    sock.write_all(resp.as_bytes()).await?;
    sock.write_all(&payload).await?;
    sock.shutdown().await
}

/// `signJwt(pkcs8, kid, claims)`: RS256 (PKCS#1 v1.5, deterministic).
fn sign_jwt(pkcs8_hex: &str, kid: &str, claims: &Value) -> String {
    let b64 = |b: &[u8]| URL_SAFE_NO_PAD.encode(b);
    let head = format!("{}.{}", b64(jsjson::stringify(&json!({ "alg": "RS256", "kid": kid, "typ": "JWT" })).as_bytes()), b64(jsjson::stringify(claims).as_bytes()));
    let key = ring::signature::RsaKeyPair::from_pkcs8(&hex::decode(pkcs8_hex).expect("pkcs8 hex")).expect("rsa key");
    let mut sig = vec![0u8; key.public().modulus_len()];
    key.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), head.as_bytes(), &mut sig).expect("rsa sign");
    format!("{head}.{}", b64(&sig))
}

impl Fake {
    pub async fn start() -> Fake {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind fake");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let state = Arc::new(Mutex::new(FakeState::default()));
        let inflight = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn(serve_fake(listener, state.clone(), inflight.clone()));
        Fake { base, state, inflight, task }
    }

    /// The reply for `route` from now on (same shape as a transcript's `fake` entries).
    pub fn set_default(&self, route: &str, reply: &Value) {
        let reply = fake_reply(reply, &self.base);
        self.state.lock().expect("fake").defaults.insert(route.to_owned(), reply);
    }
}

fn fake_reply(v: &Value, fake: &str) -> Value {
    serde_json::from_str(&jsjson::stringify(v).replace("{{FAKE}}", fake)).expect("fake reply")
}

/// `fakeSortKey(e)`.
fn fake_sort_key(e: &Value) -> String {
    let path = e["path"].as_str().unwrap_or("");
    let masked = RUN.replace_all(path, |c: &regex::Captures| {
        let r = &c[0];
        if UUID.is_match(r) || r.encode_utf16().count() == 43 || r.encode_utf16().count() == 16 { "*".to_owned() } else { r.to_owned() }
    });
    format!("{} {masked}", e["method"].as_str().unwrap_or(""))
}

// ---------------------------------------------------------------- WebAuthn authenticator emulator (`webauthnBody`)

fn cbor_bytes(b: &[u8]) -> Vec<u8> {
    let n = b.len();
    let mut out = if n < 24 { vec![0x40 | n as u8] } else if n < 256 { vec![0x58, n as u8] } else { vec![0x59, (n >> 8) as u8, (n & 255) as u8] };
    out.extend_from_slice(b);
    out
}

/// The emulated credential's key, seeded by its name: Ed25519 (COSE -8, ring) or ML-DSA-44 (COSE -48, FIPS 204,
/// `ml-dsa`; deterministic signatures, where Node's are hedged: both verify the same way).
enum CredKey {
    Ed(ring::signature::Ed25519KeyPair),
    MlDsa44(Box<ml_dsa::SigningKey<ml_dsa::MlDsa44>>),
}

impl CredKey {
    fn new(cred: &str, alg: i64) -> Self {
        let seed = Sha256::digest(format!("webauthn-{cred}").as_bytes());
        if alg == -48 {
            return CredKey::MlDsa44(Box::new(ml_dsa::SigningKey::from_seed(&ml_dsa::Seed::try_from(seed.as_slice()).expect("seed"))));
        }
        CredKey::Ed(ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed).expect("ed25519 seed"))
    }
    /// The COSE_Key: `{1: 1 (OKP), 3: -8, -1: 6 (Ed25519), -2: x}` or `{1: 7 (AKP), 3: -48, -1: pub}`.
    fn cose(&self) -> Vec<u8> {
        use ml_dsa::{KeyExport as _, Keypair as _};
        use ring::signature::KeyPair as _;
        match self {
            CredKey::Ed(kp) => [hex::decode("a401010327200621").expect("hex"), cbor_bytes(kp.public_key().as_ref())].concat(),
            CredKey::MlDsa44(sk) => [hex::decode("a3010703382f20").expect("hex"), cbor_bytes(&sk.verifying_key().to_bytes())].concat(),
        }
    }
    fn sign(&self, data: &[u8]) -> Vec<u8> {
        use ml_dsa::{SignatureEncoding as _, Signer as _};
        match self {
            CredKey::Ed(kp) => kp.sign(data).as_ref().to_vec(),
            CredKey::MlDsa44(sk) => sk.sign(data).to_vec(),
        }
    }
}

fn webauthn_body(w: &Value, vars: &HashMap<String, String>, rp_id: &str, origin: &str) -> Value {
    let b64u = |b: &[u8]| URL_SAFE_NO_PAD.encode(b);
    let kind = w["kind"].as_str().unwrap_or("");
    let challenge = vars.get(w["challenge"].as_str().unwrap_or("")).cloned().unwrap_or_default();
    let cred = w["credential"].as_str().unwrap_or("");
    let alg = w.get("alg").and_then(Value::as_i64).unwrap_or(-8);
    let key = CredKey::new(cred, alg);
    let cid = Sha256::digest(format!("cred-{cred}").as_bytes())[..16].to_vec();
    let rp_hash = Sha256::digest(rp_id.as_bytes()).to_vec();
    let client_data = jsjson::stringify(&json!({ "type": if kind == "register" { "webauthn.create" } else { "webauthn.get" }, "challenge": challenge, "origin": origin, "crossOrigin": false }));
    let counter = (w.get("counter").and_then(Value::as_u64).unwrap_or(0) as u32).to_be_bytes();
    if kind == "register" {
        let cose = key.cose();
        let mut auth = rp_hash;
        auth.push(0x45);
        auth.extend_from_slice(&counter);
        auth.extend_from_slice(&[0u8; 16]);
        auth.extend_from_slice(&[(cid.len() >> 8) as u8, (cid.len() & 255) as u8]);
        auth.extend_from_slice(&cid);
        auth.extend(cose);
        // {"fmt": "none", "attStmt": {}, "authData": h'…'} or {"fmt": "packed", "attStmt": {"alg": alg, "sig": h'…'}, …}
        let mut att = vec![0xa3];
        if w.get("fmt").and_then(Value::as_str) == Some("packed") {
            let mut data = auth.clone();
            data.extend(Sha256::digest(client_data.as_bytes()));
            att.extend(hex::decode("63666d74667061636b65646761747453746d74a263616c67").expect("hex"));
            att.extend(if alg == -48 { vec![0x38, 0x2f] } else { vec![0x27] });
            att.extend(hex::decode("63736967").expect("hex"));
            att.extend(cbor_bytes(&key.sign(&data)));
        } else {
            att.extend(hex::decode("63666d74646e6f6e656761747453746d74a0").expect("hex"));
        }
        att.extend(hex::decode("68617574684461746159").expect("hex"));
        att.extend_from_slice(&[(auth.len() >> 8) as u8, (auth.len() & 255) as u8]);
        att.extend(auth);
        let mut body = Map::new();
        body.insert(
            "response".into(),
            json!({
                "id": b64u(&cid), "rawId": b64u(&cid), "type": "public-key",
                "response": { "clientDataJSON": b64u(client_data.as_bytes()), "attestationObject": b64u(&att), "transports": ["internal"] },
                "clientExtensionResults": {}, "authenticatorAttachment": "platform",
            }),
        );
        if let Some(n) = w.get("name") {
            body.insert("name".into(), n.clone());
        }
        return Value::Object(body);
    }
    let mut auth = rp_hash;
    auth.push(0x05);
    auth.extend_from_slice(&counter);
    let mut data = auth.clone();
    data.extend(Sha256::digest(client_data.as_bytes()));
    let sig = key.sign(&data);
    json!({ "response": {
        "id": b64u(&cid), "rawId": b64u(&cid), "type": "public-key",
        "response": { "clientDataJSON": b64u(client_data.as_bytes()), "authenticatorData": b64u(&auth), "signature": b64u(&sig) },
        "clientExtensionResults": {}, "authenticatorAttachment": "platform",
    } })
}

// ---------------------------------------------------------------- replay

fn base_env(db_url: &str, data: &Path, fake: &str) -> HashMap<String, String> {
    HashMap::from([
        ("NODE_ENV".into(), "development".into()),
        ("OC_DATABASE_URL".into(), db_url.into()),
        ("OC_DATA_DIR".into(), data.to_string_lossy().into_owned()),
        ("OC_SECRET".into(), SECRET.into()),
        ("OC_WEB_ORIGIN".into(), WEB.into()),
        ("OC_API_URL".into(), API.into()),
        ("OC_RENDER_URL".into(), fake.into()),
    ])
}

async fn rows_json(pool: &sqlx::PgPool, sql: &str) -> Value {
    let rows = sqlx::query(sql).fetch_all(pool).await.unwrap_or_else(|e| panic!("check query failed: {e}\n{sql}"));
    Value::Array(rows.iter().map(|r| oc_api::pg::row(r).expect("row json")).collect())
}

fn set_cookies(headers: &axum::http::HeaderMap) -> Vec<String> {
    headers.get_all("set-cookie").iter().map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned()).collect()
}

/// Replay one group; panics with every mismatch.
pub async fn run(group: &str) {
    let _serial = SERIAL.lock().await;
    for tool in golden(group)["requires"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let ok = std::process::Command::new(tool).arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        if !ok {
            eprintln!("{tool} not on PATH: skipping the {group} API parity transcript");
            return;
        }
    }
    let Some(db) = oc_db::testing::TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping the {group} API parity transcript");
        return;
    };
    let url = db.url.clone();
    let name = group.to_owned();
    let r = tokio::spawn(async move { replay(&name, &url).await }).await;
    db.drop_db().await;
    match r {
        Ok(failures) => assert!(failures.is_empty(), "group {group}: {} mismatch(es)\n\n{}", failures.len(), failures.join("\n\n")),
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

async fn replay(group: &str, db_url: &str) -> Vec<String> {
    let g = golden(group);
    let pool = sqlx::postgres::PgPoolOptions::new().max_connections(3).connect(db_url).await.expect("connect test db");
    let data: PathBuf = std::env::temp_dir().join(format!("oc-api-parity-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&data).expect("data dir");
    let fake = Fake::start().await;
    {
        let mut st = fake.state.lock().expect("fake");
        if let Some(d) = g["fake"].as_object() {
            for (k, v) in d {
                st.defaults.insert(k.clone(), fake_reply(v, &fake.base));
            }
        }
    }
    let mut env = base_env(db_url, &data, &fake.base);
    if let Some(e) = g["env"].as_object() {
        for (k, v) in e {
            env.insert(k.clone(), v.as_str().unwrap_or("").replace("{{FAKE}}", &fake.base));
        }
    }
    for s in g["seed"].as_array().into_iter().flatten() {
        let sql = s.as_str().expect("seed sql").replace("{{FAKE}}", &fake.base);
        sqlx::raw_sql(&sql).execute(&pool).await.unwrap_or_else(|e| panic!("seed failed: {e}\n{sql}"));
    }
    let mut known_sha = HashSet::new();
    for h in g["objects"].as_array().into_iter().flatten() {
        let bytes = hex::decode(h.as_str().unwrap_or("")).expect("object hex");
        let s = sha(&bytes);
        let dir = data.join("objects/a").join(&s[..2]);
        std::fs::create_dir_all(&dir).expect("object dir");
        std::fs::write(dir.join(&s), &bytes).expect("object");
        known_sha.insert(s);
    }
    for st in g["steps"].as_array().into_iter().flatten() {
        if let Some(m) = st.get("multipart") {
            known_sha.insert(sha(&file_bytes(m)));
        }
    }
    static QUOTED_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'([0-9A-Za-z]{16})'").expect("regex"));
    let mut fixed_ids = HashSet::new();
    for s in g["seed"].as_array().into_iter().flatten() {
        for c in QUOTED_ID.captures_iter(s.as_str().unwrap_or("")) {
            fixed_ids.insert(c[1].to_owned());
        }
    }

    let clock: oc_api::state::Clock = Arc::new(|| CLOCK);
    let state = oc_api::AppState::new(oc_api::AppOptions { env, pool: pool.clone(), store: None, clock: Some(clock) }).await.expect("app state");
    let app = oc_api::app(state.clone());

    let mut vars: HashMap<String, String> = g["vars"].as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned())).collect()).unwrap_or_default();
    vars.insert("FAKE".into(), fake.base.clone());
    let mut norm = Normalizer::new(fixed_ids, known_sha);
    let mut sorted_vars: Vec<(&String, &String)> = vars.iter().collect();
    sorted_vars.sort();
    for (k, v) in sorted_vars {
        norm.set_var(k, v);
    }
    let mut jars: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    let mut failures = Vec::new();

    for (i, step) in g["steps"].as_array().into_iter().flatten().enumerate() {
        let sname = format!("#{i} {}", step["name"].as_str().unwrap_or(""));
        if let Some(e) = step.get("env").and_then(Value::as_object) {
            for (k, v) in e {
                state.set_env(k, v.as_str().map(|s| s.replace("{{FAKE}}", &fake.base)).as_deref());
            }
        }
        {
            let mut st = fake.state.lock().expect("fake");
            st.log.clear();
            st.script.clear();
            if let Some(r) = step.get("replies").and_then(Value::as_object) {
                for (path, list) in r {
                    st.script.insert(path.clone(), list.as_array().into_iter().flatten().map(|x| fake_reply(x, &fake.base)).collect());
                }
            }
        }
        let mut raw = Map::new();
        let mut rec = Map::new();
        if let Some(sql) = step.get("sql").and_then(Value::as_str) {
            let rows = rows_json(&pool, &subst(sql, &vars)).await;
            raw.insert("rows".into(), rows.clone());
            rec.insert("rows".into(), rows);
        } else {
            let mut headers: Vec<(String, String)> = vec![("user-agent".into(), UA.into()), ("host".into(), "localhost:80".into())];
            let set = |hs: &mut Vec<(String, String)>, k: &str, v: String| {
                hs.retain(|(n, _)| n != k);
                hs.push((k.to_owned(), v));
            };
            let mut payload: Option<Vec<u8>> = None;
            if let Some(j) = step.get("json") {
                set(&mut headers, "content-type", "application/json".into());
                payload = Some(jsjson::stringify(&subst_json(j, &vars)).into_bytes());
            } else if let Some(w) = step.get("webauthn") {
                set(&mut headers, "content-type", "application/json".into());
                payload = Some(jsjson::stringify(&webauthn_body(w, &vars, "app.example", WEB)).into_bytes());
            } else if let Some(t) = step.get("text").and_then(Value::as_str) {
                payload = Some(subst(t, &vars).into_bytes());
            } else if let Some(n) = step.get("bigBody").and_then(Value::as_u64) {
                payload = Some(vec![b'x'; n as usize]);
            } else if let Some(m) = step.get("multipart") {
                set(&mut headers, "content-type", format!("multipart/form-data; boundary={BOUNDARY}"));
                payload = Some(multipart_body(m));
            }
            if let Some(c) = step.get("client").and_then(Value::as_str)
                && let Some(jar) = jars.get(c).filter(|j| !j.is_empty())
            {
                set(&mut headers, "cookie", jar.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; "));
            }
            if let Some(h) = step.get("headers").and_then(Value::as_object) {
                for (k, v) in h {
                    set(&mut headers, &k.to_lowercase(), subst(v.as_str().unwrap_or(""), &vars));
                }
            }
            if let Some(p) = &payload {
                set(&mut headers, "content-length", p.len().to_string());
            }
            let mut req = axum::http::Request::builder().method(step.get("method").and_then(Value::as_str).unwrap_or("GET")).uri(subst(step.get("url").and_then(Value::as_str).unwrap_or("/"), &vars));
            for (k, v) in &headers {
                req = req.header(k.as_str(), v.as_str());
            }
            let req = req.body(payload.map_or_else(Body::empty, Body::from)).expect("request");
            let res = app.clone().oneshot(req).await.expect("router");
            let status = res.status().as_u16();
            let res_headers = res.headers().clone();
            let bytes = res.into_body().collect().await.map(|b| b.to_bytes()).unwrap_or_default();
            let mut out = Map::new();
            let mut names: Vec<&str> = res_headers.keys().map(|k| k.as_str()).collect();
            names.dedup();
            for k in names {
                if SKIP_HEADERS.contains(&k) {
                    continue;
                }
                if k == "set-cookie" {
                    out.insert(k.into(), json!(set_cookies(&res_headers)));
                } else {
                    let v: Vec<String> = res_headers.get_all(k).iter().map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned()).collect();
                    out.insert(k.into(), json!(v.join(",")));
                }
            }
            let ct = res_headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
            let body = record_body(&ct, &bytes);
            rec.insert("response".into(), json!({ "status": status, "headers": out, "body": body }));
            raw.insert("body".into(), body);
            raw.insert("header".into(), Value::Object(out));
            let mut cookies = Map::new();
            for c in set_cookies(&res_headers) {
                let pair = c.split(';').next().unwrap_or("");
                let (k, v) = pair.split_once('=').map_or((pair.trim(), ""), |(k, v)| (k.trim(), v.trim()));
                cookies.insert(k.to_owned(), json!(v));
                if let Some(cl) = step.get("client").and_then(Value::as_str) {
                    let jar = jars.entry(cl.to_owned()).or_default();
                    if c.to_lowercase().contains("max-age=0") || v.is_empty() {
                        jar.remove(k);
                    } else {
                        jar.insert(k.to_owned(), v.to_owned());
                    }
                }
            }
            raw.insert("cookie".into(), Value::Object(cookies));
        }
        // settle: background tasks of the app and requests still at the fake
        for _ in 0..400 {
            state.idle().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
            if fake.inflight.load(Ordering::SeqCst) == 0 && state.idle_now() {
                break;
            }
        }
        let raw = Value::Object(raw);
        if let Some(c) = step.get("capture").and_then(Value::as_object) {
            for (k, spec) in c {
                match capture(&raw, spec) {
                    Some(v) => {
                        vars.insert(k.clone(), v.clone());
                        norm.set_var(k, &v);
                    }
                    None => failures.push(format!("{sname}: nothing to capture for {k} in\n{}", pretty(&raw))),
                }
            }
        }
        let mut log = fake.state.lock().expect("fake").log.clone();
        log.sort_by_key(fake_sort_key);
        if !log.is_empty() {
            rec.insert("fake".into(), Value::Array(log));
        }
        if let Some(checks) = step.get("checks").and_then(Value::as_array) {
            let mut out = Vec::new();
            for c in checks {
                out.push(json!({ "name": c["name"], "rows": rows_json(&pool, &subst(c["sql"].as_str().unwrap_or(""), &vars)).await }));
            }
            rec.insert("db".into(), Value::Array(out));
        }
        let mut normalized = norm.value(&Value::Object(rec));
        for p in step.get("volatile").and_then(Value::as_array).into_iter().flatten() {
            blank(&mut normalized, p.as_str().unwrap_or(""));
        }
        for key in ["response", "fake", "db", "rows"] {
            let want = step.get(key).cloned().unwrap_or(Value::Null);
            let got = normalized.get(key).cloned().unwrap_or(Value::Null);
            if canon(&want) != canon(&got) {
                failures.push(format!("{sname} — {key}:\n  expected {}\n  actual   {}", pretty(&want), pretty(&got)));
            }
        }
    }
    fake.task.abort();
    pool.close().await;
    let _ = std::fs::remove_dir_all(&data);
    failures
}
