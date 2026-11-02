//! Sign-up control (`OC_SIGNUP`, plan 15 §5): every way of creating an account, refused and allowed. Same fakes and
//! throwaway databases as the parity transcripts; skipped without `OC_DATABASE_URL`.

mod harness;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::body::Body;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use harness::{API, CLOCK, Fake, SECRET, WEB};
use http_body_util::BodyExt as _;
use oc_api::AppState;
use oc_db::testing::TestDb;
use serde_json::{Value, json};
use tower::ServiceExt as _;

/// One database at a time: the local Postgres has little connection headroom.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// A new client address per request, so the strict sign-in rate limits never trigger.
static IP: AtomicU32 = AtomicU32::new(1);

const INVITE: &[(&str, &str)] = &[("OC_SIGNUP", "invite"), ("OC_SIGNUP_ALLOW", "Friend@Example.test,@allowed.test"), ("OC_STAFF_EMAILS", "root@example.test")];

fn golden(group: &str) -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/golden/api/{group}.json"));
    serde_json::from_str(&std::fs::read_to_string(p).expect("golden")).expect("golden json")
}

struct Res {
    status: u16,
    location: String,
    cookie: String,
    body: Value,
}

struct Ctx {
    url: String,
    pool: sqlx::PgPool,
    fake: Fake,
    data: std::path::PathBuf,
}

impl Ctx {
    fn base_env(&self) -> HashMap<String, String> {
        HashMap::from(
            [
                ("NODE_ENV", "development"),
                ("OC_DATABASE_URL", self.url.as_str()),
                ("OC_DATA_DIR", self.data.to_str().expect("data dir")),
                ("OC_SECRET", SECRET),
                ("OC_WEB_ORIGIN", WEB),
                ("OC_API_URL", API),
                ("OC_RENDER_URL", self.fake.base.as_str()),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
        )
    }

    /// An app on this database with these settings; the settings are read once, so each mode gets its own.
    async fn app(&self, extra: &[(&str, &str)]) -> AppState {
        let mut env = self.base_env();
        env.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), v.replace("{{FAKE}}", &self.fake.base))));
        let clock: oc_api::state::Clock = Arc::new(|| CLOCK);
        AppState::new(oc_api::AppOptions { env, pool: self.pool.clone(), store: None, clock: Some(clock) }).await.expect("app state")
    }

    async fn users(&self, email: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM users WHERE email = $1").bind(email).fetch_one(&self.pool).await.expect("count users")
    }

    async fn magic_tokens(&self, email: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM auth_tokens WHERE kind = 'magic' AND email = $1").bind(email).fetch_one(&self.pool).await.expect("count tokens")
    }
}

async fn send(s: &AppState, method: &str, url: &str, cookie: Option<&str>, body: Option<Value>) -> Res {
    let ip = IP.fetch_add(1, Ordering::SeqCst);
    let mut req = axum::http::Request::builder().method(method).uri(url).header("x-forwarded-for", format!("198.51.{}.{}", ip / 250, ip % 250 + 1));
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    let req = match body {
        Some(b) => req.header("content-type", "application/json").body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .expect("request");
    let res = oc_api::app(s.clone()).oneshot(req).await.expect("response");
    let status = res.status().as_u16();
    let location = res.headers().get("location").map_or(String::new(), |v| v.to_str().expect("location").to_owned());
    let cookie = res.headers().get_all("set-cookie").iter().map(|v| v.to_str().expect("cookie").to_owned()).collect::<Vec<_>>().join("\n");
    let raw = res.into_body().collect().await.expect("body").to_bytes();
    Res { status, location, cookie, body: serde_json::from_slice(&raw).unwrap_or(Value::Null) }
}

async fn post(s: &AppState, url: &str, body: Value) -> Res {
    send(s, "POST", url, None, Some(body)).await
}

fn assert_refused(r: &Res) {
    assert_eq!(r.status, 403, "{:?}", r.body);
    assert_eq!(r.body["error"], "signup_closed");
    assert!(r.body["message"].is_string());
}

/// Runs `f` on a fresh database; `None` without `OC_DATABASE_URL`.
macro_rules! with_db {
    ($ctx:ident, $body:block) => {{
        let _serial = SERIAL.lock().await;
        let Some(db) = TestDb::create().await else {
            eprintln!("OC_DATABASE_URL not set: skipping");
            return;
        };
        let url = db.url.clone();
        let task = tokio::spawn(async move {
            let pool = sqlx::postgres::PgPoolOptions::new().max_connections(3).connect(&url).await.expect("connect test db");
            let data = std::env::temp_dir().join(format!("oc-signup-test-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&data).expect("data dir");
            let $ctx = Ctx { url, pool, fake: Fake::start().await, data: data.clone() };
            $body
            $ctx.pool.close().await;
            let _ = std::fs::remove_dir_all(&data);
        })
        .await;
        db.drop_db().await;
        if let Err(e) = task {
            std::panic::resume_unwind(e.into_panic());
        }
    }};
}

fn signup_body(email: &str) -> Value {
    json!({ "email": email, "password": "correct horse battery 42", "name": "New Person", "birthYear": 1990, "acceptTerms": true })
}

async fn seed_user(ctx: &Ctx, email: &str, verified: bool) {
    let s = ctx.app(&[]).await;
    oc_api::auth::create_user(&s, email, "Existing", oc_api::auth::NewUser { password: Some("correct horse battery 42"), verified, birth_year: Some(1990), ..Default::default() }).await.expect("seed user");
}

/// A pending team invite for `email`, as POST /api/workspaces/{id}/invites issues it.
async fn invite(s: &AppState, email: &str) -> String {
    oc_api::auth::issue_token(s, "invite", None, Some(email), Some(json!({ "workspaceId": uuid::Uuid::new_v4().to_string(), "role": "member" })), 60).await.expect("invite")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn password_signup() {
    with_db!(ctx, {
        let s = ctx.app(INVITE).await;
        assert_refused(&post(&s, "/api/auth/signup", signup_body("stranger@example.test")).await);
        assert_eq!(ctx.users("stranger@example.test").await, 0);
        // A domain entry doesn't match a lookalike, and the refusal names nothing.
        let r = post(&s, "/api/auth/signup", signup_body("x@allowed.test.evil.test")).await;
        assert_refused(&r);
        assert!(!r.body.to_string().contains("allowed.test"));

        // Allowed: exact (any case), domain, staff.
        for email in ["FRIEND@example.test", "someone@ALLOWED.test", "root@example.test"] {
            let r = post(&s, "/api/auth/signup", signup_body(email)).await;
            assert_eq!(r.status, 200, "{email}: {:?}", r.body);
            assert_eq!(ctx.users(&email.to_lowercase()).await, 1);
        }

        // Invited: pending counts; used, cancelled or expired doesn't.
        invite(&s, "Invited@example.test").await;
        assert_eq!(post(&s, "/api/auth/signup", signup_body("invited@example.test")).await.status, 200);
        let used = invite(&s, "used@example.test").await;
        oc_api::auth::consume_token(&s, "invite", &used).await.expect("consume");
        assert_refused(&post(&s, "/api/auth/signup", signup_body("used@example.test")).await);
        invite(&s, "late@example.test").await;
        sqlx::query("UPDATE auth_tokens SET expires_at = now() - interval '1 minute' WHERE email = 'late@example.test'").execute(&ctx.pool).await.expect("expire");
        assert_refused(&post(&s, "/api/auth/signup", signup_body("late@example.test")).await);

        // Existing accounts answer as always, and keep signing in.
        seed_user(&ctx, "old@example.test", true).await;
        assert_eq!(post(&s, "/api/auth/signup", signup_body("old@example.test")).await.status, 409);
        let r = post(&s, "/api/auth/login", json!({ "email": "old@example.test", "password": "correct horse battery 42" })).await;
        assert_eq!(r.status, 200, "{:?}", r.body);

        // Open (the default) is unchanged.
        let open = ctx.app(&[]).await;
        assert_eq!(post(&open, "/api/auth/signup", signup_body("stranger@example.test")).await.status, 200);
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn providers_reports_the_mode_only() {
    with_db!(ctx, {
        let r = send(&ctx.app(INVITE).await, "GET", "/api/auth/providers", None, None).await;
        assert_eq!(r.body["signup"], "invite");
        assert!(!r.body.to_string().contains("allowed.test"));
        let r = send(&ctx.app(&[]).await, "GET", "/api/auth/providers", None, None).await;
        assert_eq!(r.body["signup"], "open");
        let r = send(&ctx.app(&[("OC_SIGNUP", "open")]).await, "GET", "/api/auth/providers", None, None).await;
        assert_eq!(r.body["signup"], "open");
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_typo_in_the_mode_refuses_to_start() {
    let env = HashMap::from([("OC_SIGNUP".to_owned(), "closed".to_owned())]);
    assert!(oc_config::Config::from_map(&env).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dev_login() {
    with_db!(ctx, {
        let s = ctx.app(INVITE).await;
        assert_refused(&post(&s, "/api/auth/login", json!({ "email": "stranger@example.test" })).await);
        assert_eq!(ctx.users("stranger@example.test").await, 0);
        assert_eq!(post(&s, "/api/auth/login", json!({ "email": "pal@allowed.test" })).await.status, 200);
        seed_user(&ctx, "old@example.test", true).await;
        assert_eq!(post(&s, "/api/auth/login", json!({ "email": "old@example.test" })).await.status, 200);
        assert_eq!(post(&ctx.app(&[]).await, "/api/auth/login", json!({ "email": "stranger@example.test" })).await.status, 200);
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn magic_link() {
    with_db!(ctx, {
        let s = ctx.app(INVITE).await;
        let profile = |token: &str| json!({ "token": token, "name": "New Person", "birthYear": 1990, "acceptTerms": true });

        // The request answers the same for an allowed and a refused address, and does the same work.
        let refused = post(&s, "/api/auth/magic", json!({ "email": "stranger@example.test" })).await;
        let allowed = post(&s, "/api/auth/magic", json!({ "email": "pal@allowed.test" })).await;
        assert_eq!((refused.status, &refused.body), (allowed.status, &allowed.body));
        assert_eq!(refused.body, json!({ "ok": true }));
        assert_eq!(ctx.magic_tokens("stranger@example.test").await, 1);
        assert_eq!(ctx.magic_tokens("pal@allowed.test").await, 1);

        // Refused at the first step (no profile form) and when the profile is sent directly.
        let t = oc_api::auth::issue_token(&s, "magic", None, Some("stranger@example.test"), None, 15).await.expect("token");
        assert_refused(&post(&s, "/api/auth/magic/verify", json!({ "token": t })).await);
        assert_refused(&post(&s, "/api/auth/magic/verify", profile(&t)).await);
        assert_eq!(ctx.users("stranger@example.test").await, 0);

        // An invite cancelled after the link was sent no longer helps.
        let inv = invite(&s, "gone@example.test").await;
        let t = oc_api::auth::issue_token(&s, "magic", None, Some("gone@example.test"), None, 15).await.expect("token");
        assert_eq!(post(&s, "/api/auth/magic/verify", json!({ "token": t })).await.body["needsProfile"], true);
        oc_api::auth::consume_token(&s, "invite", &inv).await.expect("consume");
        assert_refused(&post(&s, "/api/auth/magic/verify", profile(&t)).await);

        // Allowed and invited create an account.
        for email in ["pal@allowed.test", "invited@example.test"] {
            if email.starts_with("invited") {
                invite(&s, email).await;
            }
            let t = oc_api::auth::issue_token(&s, "magic", None, Some(email), None, 15).await.expect("token");
            assert_eq!(post(&s, "/api/auth/magic/verify", json!({ "token": t })).await.body["needsProfile"], true);
            let r = post(&s, "/api/auth/magic/verify", profile(&t)).await;
            assert_eq!(r.status, 200, "{email}: {:?}", r.body);
            assert_eq!(ctx.users(email).await, 1);
        }

        // An existing account signs in, with a token issued before or after it existed.
        seed_user(&ctx, "old@example.test", true).await;
        let t = oc_api::auth::issue_token(&s, "magic", None, Some("old@example.test"), None, 15).await.expect("token");
        let r = post(&s, "/api/auth/magic/verify", json!({ "token": t })).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert!(r.cookie.contains("oc_session"));

        // Open is unchanged.
        let open = ctx.app(&[]).await;
        let t = oc_api::auth::issue_token(&open, "magic", None, Some("stranger@example.test"), None, 15).await.expect("token");
        assert_eq!(post(&open, "/api/auth/magic/verify", json!({ "token": t })).await.body["needsProfile"], true);
    });
}

/// The fake identity provider from the OIDC transcript, with `email` in its next id token.
fn oidc_token(ctx: &Ctx, g: &Value, aud: &str, sub: &str, email: &str) {
    let mut reply = g["fake"]["/oidc/token"].clone();
    let c = &mut reply["idToken"]["claims"];
    c["email"] = json!(email);
    c["sub"] = json!(sub);
    c["aud"] = json!(aud);
    ctx.fake.set_default("/oidc/token", &reply);
}

/// Start → provider → callback, as a browser would; returns the callback's answer.
async fn oauth_round(s: &AppState, provider: &str) -> Res {
    let start = send(s, "GET", &format!("/api/auth/oauth/{provider}/start"), None, None).await;
    assert_eq!(start.status, 302, "{:?}", start.body);
    let state = regex::Regex::new("state=([^&]+)").expect("regex").captures(&start.location).expect("state")[1].to_owned();
    let cookie = start.cookie.split(';').next().expect("cookie").to_owned();
    send(s, "GET", &format!("/api/auth/oauth/{provider}/callback?code=c&state={state}"), Some(&cookie), None).await
}

fn fakes(ctx: &Ctx, g: &Value) {
    for (route, reply) in g["fake"].as_object().expect("fake") {
        ctx.fake.set_default(route, reply);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn company_sso_callback() {
    with_db!(ctx, {
        let g = golden("oidc");
        fakes(&ctx, &g);
        let sso = [("OC_OIDC_ISSUER", "{{FAKE}}/oidc"), ("OC_OIDC_CLIENT_ID", "golden-client"), ("OC_OIDC_CLIENT_SECRET", "golden-secret")];
        let invite_env: Vec<_> = sso.iter().chain(INVITE).copied().collect();
        let s = ctx.app(&invite_env).await;

        oidc_token(&ctx, &g, "golden-client", "sub-stranger", "stranger@example.test");
        let r = oauth_round(&s, "oidc").await;
        assert_eq!((r.status, r.location.as_str()), (302, "https://app.example/login?error=signup_closed"));
        assert!(!r.cookie.contains("oc_session="));
        assert_eq!(ctx.users("stranger@example.test").await, 0);

        oidc_token(&ctx, &g, "golden-client", "sub-pal", "pal@allowed.test");
        let r = oauth_round(&s, "oidc").await;
        assert_eq!((r.status, r.location.as_str()), (302, "https://app.example/"));
        assert!(r.cookie.contains("oc_session="));
        assert_eq!(ctx.users("pal@allowed.test").await, 1);

        invite(&s, "invited@example.test").await;
        oidc_token(&ctx, &g, "golden-client", "sub-invited", "invited@example.test");
        assert_eq!(oauth_round(&s, "oidc").await.location, "https://app.example/");

        // Existing, by email and then by its now linked identity.
        seed_user(&ctx, "old@example.test", true).await;
        oidc_token(&ctx, &g, "golden-client", "sub-old", "old@example.test");
        assert_eq!(oauth_round(&s, "oidc").await.location, "https://app.example/");
        assert_eq!(oauth_round(&s, "oidc").await.location, "https://app.example/");

        let open = ctx.app(&sso).await;
        oidc_token(&ctx, &g, "golden-client", "sub-stranger", "stranger@example.test");
        assert_eq!(oauth_round(&open, "oidc").await.location, "https://app.example/");
        assert_eq!(ctx.users("stranger@example.test").await, 1);
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn google_callback_refuses_before_the_profile_token() {
    with_db!(ctx, {
        let g = golden("oidc");
        fakes(&ctx, &g);
        let google = [("OC_GOOGLE_CLIENT_ID", "google-client"), ("OC_GOOGLE_CLIENT_SECRET", "google-secret")];
        let invite_env: Vec<_> = google.iter().chain(INVITE).copied().collect();
        let discovery = serde_json::from_str::<Value>(&g["fake"]["/oidc/.well-known/openid-configuration"]["json"].to_string().replace("{{FAKE}}", &ctx.fake.base)).expect("discovery");
        let s = ctx.app(&invite_env).await;
        oc_api::routes::account::seed_oidc_discovery(&s, "https://accounts.google.com", discovery.clone()).await;

        oidc_token(&ctx, &g, "google-client", "g-stranger", "stranger@example.test");
        let r = oauth_round(&s, "google").await;
        assert_eq!(r.location, "https://app.example/login?error=signup_closed");
        assert_eq!(ctx.magic_tokens("stranger@example.test").await, 0, "no token for the profile step");
        assert_eq!(ctx.users("stranger@example.test").await, 0);

        // Allowed: on to the profile step, which then creates the account.
        oidc_token(&ctx, &g, "google-client", "g-pal", "pal@allowed.test");
        let r = oauth_round(&s, "google").await;
        assert!(r.location.starts_with("https://app.example/auth/magic?token="), "{}", r.location);
        assert_eq!(ctx.magic_tokens("pal@allowed.test").await, 1);
        let t = r.location.split("token=").nth(1).expect("token").to_owned();
        let r = post(&s, "/api/auth/magic/verify", json!({ "token": t, "name": "Pal", "birthYear": 1990, "acceptTerms": true })).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(ctx.users("pal@allowed.test").await, 1);

        // A token issued while the address was allowed is still refused once it isn't.
        oidc_token(&ctx, &g, "google-client", "g-was", "was@allowed.test");
        let t = oauth_round(&s, "google").await.location.split("token=").nth(1).expect("token").to_owned();
        let closed = ctx.app(&[("OC_SIGNUP", "invite")]).await;
        assert_refused(&post(&closed, "/api/auth/magic/verify", json!({ "token": t, "name": "Was", "birthYear": 1990, "acceptTerms": true })).await);

        // Existing accounts go straight in; open mode still reaches the profile step.
        seed_user(&ctx, "old@example.test", true).await;
        oidc_token(&ctx, &g, "google-client", "g-old", "old@example.test");
        assert_eq!(oauth_round(&s, "google").await.location, "https://app.example/");
        let open = ctx.app(&google).await;
        oc_api::routes::account::seed_oidc_discovery(&open, "https://accounts.google.com", discovery).await;
        oidc_token(&ctx, &g, "google-client", "g-stranger", "stranger@example.test");
        assert!(oauth_round(&open, "google").await.location.contains("/auth/magic?token="));
    });
}

/// The `sub` claim of a transcript's access token.
fn token_sub(token: &str) -> String {
    let payload = URL_SAFE_NO_PAD.decode(token.split('.').nth(1).expect("payload")).expect("base64");
    serde_json::from_slice::<Value>(&payload).expect("claims")["sub"].as_str().expect("sub").to_owned()
}

/// What Supabase answers for the signed-in user of the next bridge call.
fn supabase_user(ctx: &Ctx, id: &str, email: &str) {
    ctx.fake.set_default(
        "/auth/v1/user",
        &json!({ "json": {
            "app_metadata": { "provider": "email", "providers": ["email"] }, "aud": "authenticated", "email": email, "email_confirmed_at": "2025-01-01T00:00:00Z",
            "factors": [], "id": id, "role": "authenticated", "user_metadata": {},
        } }),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supabase_bridge() {
    with_db!(ctx, {
        let g = golden("supabase");
        fakes(&ctx, &g);
        let vars: Vec<(String, String)> = g["env"].as_object().expect("env").iter().map(|(k, v)| (k.clone(), v.as_str().expect("env").to_owned())).collect();
        let sb: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let invite_env: Vec<_> = sb.iter().chain(INVITE).copied().collect();
        let s = ctx.app(&invite_env).await;
        let tok = |name: &str| g["vars"][name].as_str().expect("token").to_owned();
        let session = |t: &str| json!({ "accessToken": t, "profile": { "name": "New Person", "birthYear": 1990, "acceptTerms": true } });

        // No account: refused with and without a profile, so no profile form first. The token stays unused.
        let (t, id) = (tok("tok_samRs"), token_sub(&tok("tok_samRs")));
        supabase_user(&ctx, &id, "stranger@example.test");
        assert_refused(&post(&s, "/api/auth/supabase/session", json!({ "accessToken": t })).await);
        assert_refused(&post(&s, "/api/auth/supabase/session", session(&t)).await);
        assert_eq!(ctx.users("stranger@example.test").await, 0);

        // An unverified account with the address stays as it was: no tombstone for a sign-in that creates nothing.
        seed_user(&ctx, "squat@example.test", false).await;
        supabase_user(&ctx, &id, "squat@example.test");
        assert_refused(&post(&s, "/api/auth/supabase/session", session(&t)).await);
        assert_eq!(ctx.users("squat@example.test").await, 1);

        // Allowed creates; the same identity then signs in even off the list.
        supabase_user(&ctx, &id, "pal@allowed.test");
        let r = post(&s, "/api/auth/supabase/session", session(&t)).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(ctx.users("pal@allowed.test").await, 1);
        let t2 = tok("tok_samOld");
        assert_eq!(token_sub(&t2), id);
        let closed: Vec<_> = sb.iter().copied().chain([("OC_SIGNUP", "invite")]).collect();
        let r = post(&ctx.app(&closed).await, "/api/auth/supabase/session", json!({ "accessToken": t2 })).await;
        assert_eq!(r.status, 200, "{:?}", r.body);

        // Invited creates.
        let (t, id) = (tok("tok_nina"), token_sub(&tok("tok_nina")));
        invite(&s, "invited@example.test").await;
        supabase_user(&ctx, &id, "invited@example.test");
        let r = post(&s, "/api/auth/supabase/session", session(&t)).await;
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(ctx.users("invited@example.test").await, 1);

        // A verified account links by email, off the list.
        let (t, id) = (tok("tok_vic"), token_sub(&tok("tok_vic")));
        seed_user(&ctx, "old@example.test", true).await;
        supabase_user(&ctx, &id, "old@example.test");
        let r = post(&s, "/api/auth/supabase/session", json!({ "accessToken": t })).await;
        assert_eq!(r.status, 200, "{:?}", r.body);

        // Open is unchanged: a new address reaches the profile step.
        let (t, id) = (tok("tok_dis"), token_sub(&tok("tok_dis")));
        supabase_user(&ctx, &id, "stranger@example.test");
        let r = post(&ctx.app(&sb).await, "/api/auth/supabase/session", json!({ "accessToken": t })).await;
        assert_eq!(r.body["needsProfile"], true, "{:?}", r.body);
    });
}

#[tokio::test]
async fn staff_emails_only_promote_verified_accounts() {
    with_db!(ctx, {
        let s = ctx.app(INVITE).await;
        let add = async |email: &str, verified: bool| {
            sqlx::query("INSERT INTO users(id, email, name, color, email_verified_at) VALUES ($1, $2, 'x', '#000000', CASE WHEN $3 THEN now() END)")
                .bind(uuid::Uuid::new_v4())
                .bind(email)
                .bind(verified)
                .execute(&ctx.pool)
                .await
                .expect("insert user");
        };
        add("root@example.test", false).await;
        let staff = async || -> bool { sqlx::query_scalar("SELECT is_staff FROM users WHERE email = 'root@example.test'").fetch_one(&ctx.pool).await.expect("is_staff") };
        oc_api::routes::safety::bootstrap_staff(&s).await.expect("bootstrap");
        assert!(!staff().await, "someone who registered a listed address without proving it");
        sqlx::query("UPDATE users SET email_verified_at = now() WHERE email = 'root@example.test'").execute(&ctx.pool).await.expect("verify");
        oc_api::routes::safety::bootstrap_staff(&s).await.expect("bootstrap");
        assert!(staff().await);
    });
}

#[tokio::test]
async fn wrong_two_factor_codes_cancel_the_ticket() {
    with_db!(ctx, {
        let s = ctx.app(&[]).await;
        let id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO users(id, email, name, color, totp_secret) VALUES ($1, 'mfa@example.test', 'x', '#000000', 'unused')").bind(id).execute(&ctx.pool).await.expect("insert user");
        for data in [None, Some(json!({ "method": "password" }))] {
            let ticket = oc_api::auth::issue_token(&s, "mfa", Some(id), None, data, 10).await.expect("ticket");
            let attempt = async || post(&s, "/api/auth/mfa", json!({ "ticket": ticket, "code": "not-a-code" })).await;
            for _ in 0..4 {
                let r = attempt().await;
                assert_eq!((r.status, r.body["error"].as_str()), (401, Some("bad_code")), "{:?}", r.body);
            }
            for _ in 0..2 {
                let r = attempt().await;
                assert_eq!((r.status, r.body["error"].as_str()), (401, Some("expired")), "{:?}", r.body);
            }
        }
    });
}

#[tokio::test]
async fn wrong_two_factor_codes_lock_the_account_across_tickets() {
    with_db!(ctx, {
        let s = ctx.app(&[]).await;
        let id = uuid::Uuid::new_v4();
        let recovery = oc_api::auth::sha("ABCD-EFGH");
        sqlx::query("INSERT INTO users(id, email, name, color, totp_secret, recovery_codes) VALUES ($1, 'lock@example.test', 'x', '#000000', 'unused', $2)")
            .bind(id)
            .bind(vec![recovery])
            .execute(&ctx.pool)
            .await
            .expect("insert user");
        for _ in 0..2 {
            let ticket = oc_api::auth::issue_token(&s, "mfa", Some(id), None, None, 10).await.expect("ticket");
            for _ in 0..5 {
                post(&s, "/api/auth/mfa", json!({ "ticket": ticket, "code": "not-a-code" })).await;
            }
        }
        let ticket = oc_api::auth::issue_token(&s, "mfa", Some(id), None, None, 10).await.expect("ticket");
        let r = post(&s, "/api/auth/mfa", json!({ "ticket": ticket, "code": "not-a-code" })).await;
        assert_eq!((r.status, r.body["error"].as_str()), (423, Some("too_many_codes")), "{:?}", r.body);
        let r = post(&s, "/api/auth/mfa", json!({ "ticket": ticket, "code": "abcd-efgh" })).await;
        assert_eq!(r.status, 200, "a recovery code still works: {:?}", r.body);
        let left: i64 = sqlx::query_scalar("SELECT sum(failures)::bigint FROM auth_tokens WHERE user_id = $1").bind(id).fetch_one(&ctx.pool).await.expect("failures");
        assert_eq!(left, 0, "signing in clears the count");
    });
}
