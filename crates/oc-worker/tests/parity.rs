//! Cross-language parity for the worker and the database side of `oc-social` (ADR-0001): replays every phase of
//! tools/golden/worker.ts against the Rust implementation and compares with tests/golden/worker/<phase>.json.
//!
//! * `store`    oc_social::store (ensureLocalAccount, saveAccounts, visibleAccounts, publicAccount, tokensFor)
//! * `sweep`    oc_social::publish (duePosts, sweep)
//! * `gc-empty` / `gc`  oc_worker::gc (the sanity brake, the advisory lock, dry run, real runs)
//! * `worker`   the `oc-worker` BINARY draining the seeded jobs of every queue (email, webhook, account-export,
//!   account-delete, publish) plus its start-up maintenance run and heartbeat, exactly as the generator ran
//!   `services/worker` (`tsx src/main.ts`)
//!
//! Each phase gets the same seed SQL, object-store files and a Rust copy of the generator's fake HTTP server
//! (render service, Bluesky, webhook endpoints), on its own throwaway database (`oc_db::testing::TestDb`,
//! skipped without `OC_DATABASE_URL`). Compared: the phase result (normalized the same way), every check query's
//! rows as node-postgres would return them (`oc_worker::pgjson`, key order included), the requests the fake server
//! received, the worker's log lines and the entries of the account-export zip.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use axum::Router;
use axum::extract::{Request, State};
use axum::response::Response;
use hmac::{Hmac, Mac};
use oc_db::testing::TestDb;
use oc_js::JsValue;
use oc_social::store::{self, AccountRow};
use oc_social::{ConnectedAccount, Http, Tokens};
use oc_worker::gc::{self, GcOptions};
use oc_worker::pgjson;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::Sha256;
use sqlx::PgPool;
use tokio::io::AsyncBufReadExt as _;
use uuid::Uuid;

const SECRET: &str = "worker-golden-secret-0123456789abcdef";
const WEB: &str = "https://app.example";
const WEBHOOK_SECRET: &str = "whsec-golden";

fn golden(name: &str) -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/golden/worker/{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).expect("golden json")
}

/// `uid(n)` of the generator.
fn uid(n: u64) -> Uuid {
    Uuid::parse_str(&format!("00000000-0000-4000-8000-{n:012}")).expect("uuid")
}

/// `did(name)`: a 16-character design id.
fn did(name: &str) -> String {
    let mut s = format!("g{name}");
    while s.len() < 16 {
        s.push('0');
    }
    s[..16].to_string()
}

// ---------------------------------------------------------------- comparison

/// `JSON.stringify` with object keys sorted: equality regardless of key order.
fn canon(v: &JsValue) -> String {
    match v {
        JsValue::Array(a) => format!("[{}]", a.iter().map(canon).collect::<Vec<_>>().join(",")),
        JsValue::Object(m) => {
            let mut kv: Vec<&(String, JsValue)> = m.iter().collect();
            kv.sort_by(|a, b| oc_js::cmp_utf16(&a.0, &b.0));
            format!("{{{}}}", kv.iter().map(|(k, x)| format!("{}:{}", oc_js::stringify_str(k), canon(x))).collect::<Vec<_>>().join(","))
        }
        other => other.stringify(),
    }
}

/// Collects mismatches of one phase and reports them all at the end.
#[derive(Default)]
struct Report {
    failures: Vec<String>,
}

impl Report {
    /// Exact comparison, key order included (what `JSON.stringify` of each side prints).
    fn exact(&mut self, what: &str, actual: &JsValue, expected: &JsValue) {
        if actual.stringify() != expected.stringify() {
            self.failures.push(format!(
                "{what}:\n  expected {}\n  actual   {}",
                pgjson::stringify_pretty(expected),
                pgjson::stringify_pretty(actual)
            ));
        }
    }

    /// Comparison of values the test harness itself assembles (object key order is the harness's, not the
    /// implementation's).
    fn same(&mut self, what: &str, actual: &JsValue, expected: &JsValue) {
        if canon(actual) != canon(expected) {
            self.failures.push(format!(
                "{what}:\n  expected {}\n  actual   {}",
                pgjson::stringify_pretty(expected),
                pgjson::stringify_pretty(actual)
            ));
        }
    }

    fn finish(self, phase: &str) {
        assert!(self.failures.is_empty(), "phase {phase}: {} mismatch(es)\n{}", self.failures.len(), self.failures.join("\n\n"));
    }
}

fn js(v: &Value) -> JsValue {
    JsValue::from(v)
}

static SIG: LazyLock<Regex> = LazyLock::new(|| Regex::new("exp=[0-9]+&sig=[A-Za-z0-9_-]+").expect("regex"));

/// `norm(v, fake)`: the fake server's address → `{{FAKE}}`, signed media links → `exp=E&sig=S`.
fn norm(v: &JsValue, fake: &str) -> JsValue {
    match v {
        JsValue::String(s) => JsValue::String(SIG.replace_all(&s.replace(fake, "{{FAKE}}"), "exp=E&sig=S").into_owned()),
        JsValue::Array(a) => JsValue::Array(a.iter().map(|x| norm(x, fake)).collect()),
        JsValue::Object(m) => JsValue::Object(m.iter().map(|(k, x)| (k.clone(), norm(x, fake))).collect()),
        other => other.clone(),
    }
}

/// The fake server's log, normalized and in a stable order (both sides are sorted the same way).
fn sorted(entries: &[JsValue]) -> JsValue {
    let mut v: Vec<JsValue> = entries.to_vec();
    v.sort_by_key(JsValue::stringify);
    JsValue::Array(v)
}

// ---------------------------------------------------------------- fake render / Bluesky / webhook server

#[derive(Clone)]
struct FakeState {
    base: String,
    log: Arc<Mutex<Vec<Value>>>,
}

struct Fake {
    base: String,
    log: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Fake {
    async fn start() -> Fake {
        // Like the generator (`listen(0, '::')`, base `http://localhost:<port>`): webhooks only allow plain http
        // for the hostname `localhost`. Dual-stack where IPv6 exists, else IPv4 only.
        let listener = match tokio::net::TcpListener::bind("[::]:0").await {
            Ok(l) => l,
            Err(_) => tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind fake server"),
        };
        let base = format!("http://localhost:{}", listener.local_addr().expect("addr").port());
        let st = FakeState { base: base.clone(), log: Arc::default() };
        let app = Router::new().fallback(fake_handler).with_state(st.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Fake { base, log: st.log, task }
    }

    /// `sortedLog(fake.log, fake.base)`.
    fn sorted_log(&self) -> JsValue {
        let log: Vec<JsValue> = self.log.lock().expect("log").iter().map(|e| norm(&js(e), &self.base)).collect();
        sorted(&log)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 1×1 PNG with its IHDR patched to (100 + page) × 60.
fn png_for(page: u32) -> Vec<u8> {
    use base64::Engine as _;
    let mut b = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==")
        .expect("png");
    b[16..20].copy_from_slice(&(100 + page).to_be_bytes());
    b[20..24].copy_from_slice(&60u32.to_be_bytes());
    b
}

/// A minimal JPEG header (SOI, APP0, SOF0 640×360) plus a page marker byte.
fn jpg_for(page: u8) -> Vec<u8> {
    let mut b = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
    b.extend_from_slice(&[0; 14]);
    b.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08, 0x01, 0x68, 0x02, 0x80, 0x03]);
    b.extend_from_slice(&[0; 9]);
    b.push(page);
    b
}

fn send(status: u16, body: impl Into<axum::body::Body>, ty: &str) -> Response {
    Response::builder().status(status).header("content-type", ty).body(body.into()).expect("response")
}

fn send_json(status: u16, v: Value) -> Response {
    send(status, v.to_string(), "application/json")
}

static ISO_MS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$").expect("regex"));
static SIGNATURE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^t=(\d+),v1=([0-9a-f]{64})$").expect("regex"));

async fn fake_handler(State(st): State<FakeState>, req: Request) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path_and_query().map_or("/".to_string(), |p| p.as_str().to_string());
    let headers = req.headers().clone();
    let header = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let ct = header("content-type").unwrap_or_default();
    let raw = axum::body::to_bytes(req.into_body(), usize::MAX).await.unwrap_or_default();
    let mut body = if raw.is_empty() { Value::Null } else { json!(format!("<bytes {}>", raw.len())) };
    if ct.contains("json") {
        body = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    }
    let mut entry = Map::new();
    entry.insert("method".into(), json!(method));
    entry.insert("path".into(), json!(path));
    entry.insert("contentType".into(), if ct.is_empty() { Value::Null } else { json!(ct) });
    entry.insert("authorization".into(), header("authorization").map_or(Value::Null, Value::from));
    entry.insert("body".into(), body.clone());
    let push = |e: Map<String, Value>| st.log.lock().expect("log").push(Value::Object(e));
    let session = json!({
        "did": "did:plc:golden", "handle": "golden.bsky.social", "accessJwt": "A2", "refreshJwt": "R2",
        "didDoc": { "service": [{ "id": "#atproto_pds", "type": "AtprotoPersonalDataServer", "serviceEndpoint": format!("{}/bsky", st.base) }] },
    });
    match path.as_str() {
        "/internal/export" => {
            entry.insert("internal".into(), json!(header("x-oc-internal").as_deref() == Some(oc_social::crypto::internal_token(SECRET).as_str())));
            push(entry);
            if body.get("designId").and_then(Value::as_str) == Some(did("fail").as_str()) {
                return send(503, "render busy", "text/plain");
            }
            let page = body.get("pages").and_then(|p| p.get(0)).and_then(Value::as_u64).unwrap_or(0);
            match body.get("format").and_then(Value::as_str) {
                Some("png") => send(200, png_for(page as u32), "image/png"),
                Some("jpg") => send(200, jpg_for(page as u8), "image/jpeg"),
                _ => send(200, format!("MP4:{}", body.get("designId").and_then(Value::as_str).unwrap_or("undefined")), "video/mp4"),
            }
        }
        "/bsky/xrpc/com.atproto.server.refreshSession" => {
            push(entry);
            if header("authorization").as_deref() == Some("Bearer R-bad") {
                return send_json(400, json!({ "error": "ExpiredToken", "message": "Token has been revoked" }));
            }
            send_json(200, session)
        }
        "/bsky/xrpc/com.atproto.server.createSession" => {
            push(entry);
            send_json(200, session)
        }
        "/bsky/xrpc/com.atproto.repo.uploadBlob" => {
            push(entry);
            send_json(200, json!({ "blob": { "$type": "blob", "ref": { "$link": format!("bafy{}", raw.len()) }, "mimeType": ct, "size": raw.len() } }))
        }
        "/bsky/xrpc/com.atproto.repo.createRecord" => {
            let mut body = body;
            let created = body.get("record").and_then(|r| r.get("createdAt")).map(|v| oc_social::http::js_string(Some(v))).unwrap_or_else(|| "undefined".into());
            if let Some(Value::Object(rec)) = body.get_mut("record") {
                rec.shift_remove("createdAt");
            }
            entry.insert("body".into(), body);
            entry.insert("createdAtIso".into(), json!(ISO_MS.is_match(&created)));
            push(entry);
            send_json(200, json!({ "uri": "at://did:plc:golden/app.bsky.feed.post/3kgolden", "cid": "c" }))
        }
        "/hook/ok" | "/hook/fail" => {
            let sig = header("oc-signature").unwrap_or_default();
            let valid = SIGNATURE.captures(&sig).is_some_and(|m| {
                let mut mac = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes()).expect("hmac");
                mac.update(format!("{}.{}", &m[1], String::from_utf8_lossy(&raw)).as_bytes());
                hex::encode(mac.finalize().into_bytes()) == m[2]
            });
            entry.insert("signatureValid".into(), json!(valid));
            entry.insert("delivery".into(), header("oc-delivery").map_or(Value::Null, Value::from));
            entry.insert("userAgent".into(), header("user-agent").map_or(Value::Null, Value::from));
            push(entry);
            if path == "/hook/ok" { send(200, "ok", "text/plain") } else { send(500, "nope", "text/plain") }
        }
        _ => {
            push(entry);
            send_json(404, json!({ "error": { "message": format!("unmocked {path}") } }))
        }
    }
}

// ---------------------------------------------------------------- phase plumbing

/// Phases run one at a time: each holds several Postgres connections plus a maintenance connection, and the dev
/// servers of other worktrees often leave little headroom under `max_connections` ("too many clients already").
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Phase {
    _serial: tokio::sync::MutexGuard<'static, ()>,
    name: &'static str,
    golden: Value,
    db: TestDb,
    data: PathBuf,
    fake: Fake,
    report: Report,
}

impl Phase {
    /// Throwaway database + data dir + fake server, seeded like the generator did. `None` without a database.
    async fn start(name: &'static str) -> Option<Phase> {
        let serial = SERIAL.lock().await;
        let Some(db) = TestDb::create().await else {
            eprintln!("OC_DATABASE_URL not set: skipping the {name} parity phase");
            return None;
        };
        let golden = golden(name);
        let data = std::env::temp_dir().join(format!("oc-worker-parity-{name}-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&data).expect("data dir");
        let fake = Fake::start().await;
        for o in golden["objects"].as_array().expect("objects") {
            let bytes = hex::decode(o["hex"].as_str().expect("hex")).expect("hex");
            let age = Duration::from_secs_f64(o["ageHours"].as_f64().expect("ageHours") * 3600.0);
            place_object(&data, &bytes, SystemTime::now() - age);
        }
        for s in golden["seed"].as_array().expect("seed") {
            let sql = s.as_str().expect("sql").replace("{{FAKE}}", &fake.base);
            sqlx::raw_sql(&sql).execute(&db.pool).await.unwrap_or_else(|e| panic!("seed {sql}: {e}"));
        }
        Some(Phase { _serial: serial, name, golden, db, data, fake, report: Report::default() })
    }

    fn pool(&self) -> &PgPool {
        &self.db.pool
    }

    fn store(&self) -> oc_storage::ObjectStore {
        oc_storage::ObjectStore::Local(oc_storage::LocalStore::new(self.data.join("objects")))
    }

    /// The environment the generator gave its child processes (`env(url, data, fake)`).
    fn env(&self) -> HashMap<String, String> {
        let mut e = HashMap::new();
        for k in ["PATH", "HOME", "USER", "LOGNAME", "TMPDIR"] {
            if let Ok(v) = std::env::var(k) {
                e.insert(k.to_string(), v);
            }
        }
        e.insert("NODE_ENV".into(), "development".into());
        e.insert("OC_DATABASE_URL".into(), self.db.url.clone());
        e.insert("OC_DATA_DIR".into(), self.data.to_string_lossy().into_owned());
        e.insert("OC_SECRET".into(), SECRET.into());
        e.insert("OC_WEB_ORIGIN".into(), WEB.into());
        e.insert("OC_RENDER_URL".into(), self.fake.base.clone());
        e.insert("OC_BLUESKY_SERVICE".into(), format!("{}/bsky", self.fake.base));
        e
    }

    fn http(&self) -> Http {
        Http::with_env_map(self.env())
    }

    fn compare_result(&mut self, actual: Value) {
        let expected = js(&self.golden["result"]);
        let actual = norm(&js(&actual), &self.fake.base);
        self.report.same("result", &actual, &expected);
    }

    /// Every check query, rows as node-postgres returns them (column and jsonb key order included).
    async fn compare_checks(&mut self) {
        let checks = self.golden["checks"].as_array().expect("checks").clone();
        for c in checks {
            let sql = c["sql"].as_str().expect("sql");
            let rows = sqlx::query(sql).fetch_all(&self.db.pool).await.unwrap_or_else(|e| panic!("check {sql}: {e}"));
            let actual = norm(&pgjson::rows(&rows).expect("rows"), &self.fake.base);
            self.report.exact(&format!("check `{}`", c["name"].as_str().unwrap_or("?")), &actual, &js(&c["rows"]));
        }
    }

    fn compare_fake(&mut self) {
        let expected: Vec<JsValue> = self.golden["fake"].as_array().expect("fake").iter().map(js).collect();
        let actual = self.fake.sorted_log();
        self.report.exact("fake server requests", &actual, &sorted(&expected));
    }

    async fn finish(mut self) {
        self.compare_checks().await;
        self.compare_fake();
        let Phase { name, db, data, fake, report, .. } = self;
        drop(fake);
        db.drop_db().await;
        let _ = std::fs::remove_dir_all(&data);
        report.finish(name);
    }
}

/// `placeObjects`: a content-addressed file under `<data>/objects/a/<sha[0..2]>/<sha>` with the given mtime.
fn place_object(data: &Path, bytes: &[u8], modified: SystemTime) {
    let sha = oc_storage::sha256_hex(bytes);
    let dir = data.join("objects/a").join(&sha[..2]);
    std::fs::create_dir_all(&dir).expect("object dir");
    let f = dir.join(&sha);
    std::fs::write(&f, bytes).expect("object");
    std::fs::File::options()
        .write(true)
        .open(&f)
        .and_then(|h| h.set_times(std::fs::FileTimes::new().set_accessed(modified).set_modified(modified)))
        .expect("mtime");
}

fn now_ms() -> f64 {
    oc_js::now_ms() as f64
}

/// `Math.round`.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

// ---------------------------------------------------------------- phase: store

#[tokio::test(flavor = "multi_thread")]
async fn store_phase() {
    let Some(mut ph) = Phase::start("store").await else { return };
    let (pool, http) = (ph.pool().clone(), ph.http());
    let st = uid(101);
    let mut label: HashMap<Uuid, String> = [(uid(301), "seed:team"), (uid(302), "seed:stranger"), (uid(303), "seed:noTokens")]
        .into_iter()
        .map(|(k, v)| (k, v.to_string()))
        .collect();
    store::ensure_local_account(&pool, st).await.expect("ensureLocalAccount");
    store::ensure_local_account(&pool, st).await.expect("ensureLocalAccount (again)");
    let extra = |pairs: &[(&str, &str)]| Some(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
    let a1 = ConnectedAccount {
        external_id: "did:plc:store".into(),
        display_name: "Store Test".into(),
        handle: Some("s.bsky.social".into()),
        avatar_url: None,
        tokens: Tokens {
            access_token: "SECRET-ACCESS".into(),
            refresh_token: Some("R1".into()),
            expires_at: Some(4_102_444_800_000.0),
            extra: extra(&[("identifier", "s.bsky.social"), ("appPassword", "pw-123"), ("did", "did:plc:store")]),
        },
        meta: Some(json!({ "a": 1 }).as_object().cloned().unwrap_or_default()),
    };
    let ids1 = store::save_accounts(&pool, SECRET, st, "bluesky", std::slice::from_ref(&a1)).await.expect("saveAccounts");
    let renamed = ConnectedAccount {
        display_name: "Renamed ".repeat(40),
        avatar_url: Some("https://cdn/a.png".into()),
        meta: Some(json!({ "b": 2 }).as_object().cloned().unwrap_or_default()),
        ..a1.clone()
    };
    let ids2 = store::save_accounts(&pool, SECRET, st, "bluesky", &[renamed]).await.expect("saveAccounts (update)");
    let save = |provider: &'static str, external: &'static str, tokens: Tokens| {
        let pool = pool.clone();
        async move {
            let a = ConnectedAccount { external_id: external.into(), display_name: external.into(), tokens, ..Default::default() };
            store::save_accounts(&pool, SECRET, st, provider, &[a]).await.expect("saveAccounts")[0]
        }
    };
    let t = |access: &str, refresh: Option<&str>, expires: f64| Tokens {
        access_token: access.into(),
        refresh_token: refresh.map(str::to_string),
        expires_at: Some(expires),
        extra: None,
    };
    let ok = save("bluesky", "did:plc:exp-ok", t("OLD", Some("R1"), 1000.0)).await;
    let bad = save("bluesky", "did:plc:exp-bad", t("OLD", Some("R-bad"), 1000.0)).await;
    let pw = save(
        "bluesky",
        "did:plc:exp-pw",
        Tokens { extra: extra(&[("identifier", "pw.bsky.social"), ("appPassword", "app-pw")]), ..t("OLD", Some("R-bad"), 1000.0) },
    )
    .await;
    let x_no = save("x", "x-expired", t("XA", None, 1000.0)).await;
    let zero = save("x", "x-zero", t("XZ", None, 0.0)).await;
    for (id, n) in [(ids1[0], "a1"), (ok, "ok"), (bad, "bad"), (pw, "pw"), (x_no, "xNo"), (zero, "zero")] {
        label.insert(id, format!("saved:{n}"));
    }
    let local: Uuid = sqlx::query_scalar("SELECT id FROM social_accounts WHERE user_id = $1 AND provider = $2")
        .bind(st)
        .bind("local")
        .fetch_one(&pool)
        .await
        .expect("local account");
    label.insert(local, "saved:local".into());
    let visible = store::visible_accounts(&pool, st).await.expect("visibleAccounts");
    let public = |r: &AccountRow, me: Option<Uuid>| {
        let mut p = store::public_account(r, me);
        let l = label.get(&r.id).cloned().unwrap_or_else(|| "unknown".into());
        let m = p.as_object_mut().expect("object");
        if !l.starts_with("seed:") {
            m.insert("createdAt".into(), json!("now"));
        }
        m.insert("id".into(), json!(l));
        p
    };
    let tokens = |id: Uuid, force: bool| {
        let (pool, http) = (pool.clone(), http.clone());
        async move {
            let row: AccountRow = sqlx::query_as("SELECT * FROM social_accounts WHERE id = $1").bind(id).fetch_one(&pool).await.expect("row");
            match store::tokens_for(&pool, &http, SECRET, &row, force, now_ms()).await {
                Ok(t) => {
                    let mut v = serde_json::to_value(&t).expect("tokens json");
                    if let Some(exp) = t.expires_at {
                        let now = now_ms();
                        let shown = if exp > now + 365.0 * 86_400_000.0 || exp < now {
                            v["expiresAt"].clone()
                        } else {
                            json!(format!("+{}min", js_round((exp - now) / 60_000.0)))
                        };
                        v["expiresAt"] = shown;
                    }
                    json!({ "tokens": v })
                }
                Err(e) => json!({ "error": { "message": e.message, "transient": e.transient, "status": e.status, "reauth": e.reauth } }),
            }
        }
    };
    let result = json!({
        "sameId": ids1[0] == ids2[0],
        "visible": visible.iter().map(|r| public(r, Some(st))).collect::<Vec<_>>(),
        "publicWithoutMe": public(&visible[0], None),
        "tokens": {
            "local": tokens(local, false).await,
            "a1": tokens(ids1[0], false).await,
            "a1Forced": tokens(ids1[0], true).await,
            "refreshOk": tokens(ok, false).await,
            "refreshRejected": tokens(bad, false).await,
            "refreshFallsBackToPassword": tokens(pw, false).await,
            "noRefreshToken": tokens(x_no, false).await,
            "zeroExpiry": tokens(zero, false).await,
            "noStoredTokens": tokens(uid(303), false).await,
        },
    });
    ph.compare_result(result);
    ph.finish().await;
}

// ---------------------------------------------------------------- phase: sweep

#[tokio::test(flavor = "multi_thread")]
async fn sweep_phase() {
    let Some(mut ph) = Phase::start("sweep").await else { return };
    let pool = ph.pool().clone();
    let ids = |v: Vec<oc_social::publish::DuePost>| v.into_iter().map(|p| p.id.to_string()).collect::<Vec<_>>();
    let due = ids(oc_social::publish::due_posts(&pool, 1000).await.expect("duePosts"));
    let due_limited = ids(oc_social::publish::due_posts(&pool, 1).await.expect("duePosts(1)"));
    let swept = oc_social::publish::sweep(&pool, oc_js::now_ms()).await.expect("sweep");
    let after = ids(oc_social::publish::due_posts(&pool, 1000).await.expect("duePosts"));
    ph.compare_result(json!({ "due": due, "dueLimited": due_limited, "swept": swept, "after": after }));
    ph.finish().await;
}

// ---------------------------------------------------------------- phases: gc-empty, gc

/// Object names of the phase (from the golden's object list) by sha.
fn names_by_sha(golden: &Value) -> Vec<(String, String)> {
    golden["objects"]
        .as_array()
        .expect("objects")
        .iter()
        .map(|o| (o["name"].as_str().expect("name").to_string(), oc_storage::sha256_hex(&hex::decode(o["hex"].as_str().expect("hex")).expect("hex"))))
        .collect()
}

/// Names whose object file still exists, in the generator's order.
fn present(data: &Path, names: &[(String, String)]) -> Value {
    json!(names.iter().filter(|(_, sha)| data.join("objects/a").join(&sha[..2]).join(sha).exists()).map(|(n, _)| n.clone()).collect::<Vec<_>>())
}

async fn collect(pool: &PgPool, store: &oc_storage::ObjectStore, grace_hours: f64, dry_run: bool) -> Value {
    let r = gc::collect_garbage(pool, store, GcOptions { grace_hours, dry_run }, SystemTime::now()).await.expect("collectGarbage");
    serde_json::to_value(r).expect("report json")
}

#[tokio::test(flavor = "multi_thread")]
async fn gc_empty_phase() {
    let Some(mut ph) = Phase::start("gc-empty").await else { return };
    let names = names_by_sha(&ph.golden);
    let run = collect(ph.pool(), &ph.store(), 48.0, false).await;
    let result = json!({ "run": run, "present": present(&ph.data, &names) });
    ph.compare_result(result);
    ph.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn gc_phase() {
    let Some(mut ph) = Phase::start("gc").await else { return };
    let names = names_by_sha(&ph.golden);
    let (pool, store) = (ph.pool().clone(), ph.store());
    let mut marked: Vec<String> = gc::mark_referenced(&pool)
        .await
        .expect("markReferenced")
        .into_iter()
        .map(|sha| names.iter().find(|(_, s)| *s == sha).map_or(sha, |(n, _)| n.clone()))
        .collect();
    marked.sort_by(|a, b| oc_js::cmp_utf16(a, b));
    let locked = {
        let mut c = pool.acquire().await.expect("connection");
        sqlx::query("SELECT pg_advisory_lock($1)").bind(gc::LOCK).execute(&mut *c).await.expect("lock");
        let r = collect(&pool, &store, 48.0, false).await;
        sqlx::query("SELECT pg_advisory_unlock($1)").bind(gc::LOCK).execute(&mut *c).await.expect("unlock");
        r
    };
    let dry = collect(&pool, &store, 48.0, true).await;
    let present_after_dry = present(&ph.data, &names);
    let real = collect(&pool, &store, 48.0, false).await;
    let again = collect(&pool, &store, 48.0, false).await;
    let tiny = collect(&pool, &store, 0.0, false).await;
    let result = json!({
        "marked": marked, "locked": locked, "dry": dry, "presentAfterDry": present_after_dry, "real": real, "again": again,
        "tiny": tiny, "present": present(&ph.data, &names),
    });
    ph.compare_result(result);
    ph.finish().await;
}

// ---------------------------------------------------------------- phase: worker (the oc-worker binary)

/// Wait until no due job is pending or running, maintenance has finished and the heartbeat is written.
async fn settle(pool: &PgPool) {
    let sql = "SELECT
        (SELECT count(*)::int FROM jobs WHERE queue <> 'maint-none' AND (status = 'running' OR (status = 'queued' AND run_at <= now()))) AS pending,
        (SELECT count(*) = 0 FROM oauth_clients WHERE id = 'm-oc-stale') AS maintained,
        (SELECT count(*)::int FROM service_heartbeats WHERE service = 'worker') AS beat";
    let end = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut quiet = 0;
    while quiet < 5 {
        assert!(tokio::time::Instant::now() < end, "worker did not settle");
        let (pending, maintained, beat): (i32, Option<bool>, i32) = sqlx::query_as(sql).fetch_one(pool).await.expect("settle query");
        quiet = if pending == 0 && maintained == Some(true) && beat == 1 { quiet + 1 } else { 0 };
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The account-export zip: every entry with its content (designs/*.json parsed, uploads as hex).
async fn export_entries(pool: &PgPool, data: &Path) -> JsValue {
    let r: Value = sqlx::query_scalar("SELECT result FROM jobs WHERE id = $1").bind(uid(605)).fetch_one(pool).await.expect("export job");
    let sha = r["sha"].as_str().expect("export sha");
    let bytes = std::fs::read(data.join("objects/a").join(&sha[..2]).join(sha)).expect("export zip");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).expect("zip");
    let mut entries = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).expect("entry");
        let name = f.name().to_string();
        if f.is_dir() {
            entries.push(json!({ "name": name, "dir": true }));
            continue;
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).expect("read entry");
        let e = if name.starts_with("uploads/") {
            json!({ "name": name, "dir": false, "hex": hex::encode(&buf) })
        } else if name.starts_with("designs/") {
            json!({ "name": name, "dir": false, "json": serde_json::from_slice::<Value>(&buf).expect("design json") })
        } else {
            json!({ "name": name, "dir": false, "text": String::from_utf8_lossy(&buf) })
        };
        entries.push(e);
    }
    let size = r["size"].as_u64() == Some(bytes.len() as u64);
    js(&json!({ "entries": entries, "size": size, "sha": oc_storage::sha256_hex(&bytes) == sha }))
}

/// The export zip: entry names and order, folders, text files and uploads byte for byte. The design documents
/// (`designs/*.json`) must hold the same JSON, but their object keys are compared without order: TS `decodeDoc`
/// emits Yjs insertion order, which yrs does not retain (its map entries and `Any` objects are hash maps), so
/// `oc_crdt::decode_doc_value` emits keys in its own canonical order (documented deviation; JSON object order is
/// not significant to the re-import).
fn compare_export(report: &mut Report, actual: &JsValue, expected: &JsValue) {
    let field = |v: &JsValue, k: &str| match v {
        JsValue::Object(m) => m.iter().find(|(n, _)| n == k).map(|(_, x)| x.clone()),
        _ => None,
    };
    for k in ["size", "sha"] {
        report.exact(&format!("account export {k}"), &field(actual, k).unwrap_or(JsValue::Null), &field(expected, k).unwrap_or(JsValue::Null));
    }
    let entries = |v: &JsValue| match field(v, "entries") {
        Some(JsValue::Array(a)) => a,
        _ => Vec::new(),
    };
    let (a, e) = (entries(actual), entries(expected));
    let names = |l: &[JsValue]| JsValue::Array(l.iter().map(|x| field(x, "name").unwrap_or(JsValue::Null)).collect());
    report.exact("account export entry names", &names(&a), &names(&e));
    for (x, y) in a.iter().zip(&e) {
        let name = field(y, "name").map(|n| n.stringify()).unwrap_or_default();
        if field(y, "json").is_some() {
            report.same(&format!("account export {name}"), x, y);
        } else {
            report.exact(&format!("account export {name}"), x, y);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_phase() {
    let Some(mut ph) = Phase::start("worker").await else { return };
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_oc-worker"))
        .env_clear()
        .envs(ph.env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn oc-worker");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let lines = tokio::spawn(async move {
        let mut out = Vec::new();
        let mut r = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(l)) = r.next_line().await {
            out.push(l);
        }
        out
    });
    tokio::spawn(async move {
        let mut r = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(l)) = r.next_line().await {
            eprintln!("  [worker] {l}");
        }
    });
    settle(ph.pool()).await;
    child.kill().await.expect("kill oc-worker");
    let mut log: Vec<String> = lines.await.expect("stdout reader").into_iter().filter(|l| l.starts_with("[worker]")).collect();
    log.sort_by(|a, b| oc_js::cmp_utf16(a, b));
    let expected_log = js(&ph.golden["log"]);
    ph.report.exact("worker log", &js(&json!(log)), &expected_log);
    let export = export_entries(ph.pool(), &ph.data).await;
    let expected_export = js(&ph.golden["export"]);
    compare_export(&mut ph.report, &export, &expected_export);
    ph.finish().await;
}
