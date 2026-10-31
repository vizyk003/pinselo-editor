//! oc-api binary: `OC_API_PORT` (default 4000) on 0.0.0.0, like services/api/src/main.ts.

use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    // Internal errors are reported through `tracing`; without this they never reach the logs.
    tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).with_ansi(false).init();
    let env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let cfg = oc_config::Config::from_lookup(|k| env.get(k).cloned()).unwrap_or_else(|e| panic!("invalid configuration: {e}"));
    cfg.assert_production();
    // TLS, transaction-pooler and Supabase guards (oc_db::connection); a refused setup stops here.
    let pool = oc_db::lazy_pool(&cfg.database_url).unwrap_or_else(|e| {
        eprintln!("[api] {e}");
        std::process::exit(1)
    });
    oc_db::startup_checks("api", &pool).await;
    let state = oc_api::AppState::new(oc_api::AppOptions { env, pool, store: None, clock: None }).await.expect("config");
    // routes/safety.ts registration step: staff accounts from OC_STAFF_EMAILS.
    if let Err(e) = oc_api::routes::safety::bootstrap_staff(&state).await {
        eprintln!("[api] OC_STAFF_EMAILS: {e}");
    }
    // In-process job workers of the TS API (`work(...)` in the route plugins).
    oc_api::print::fulfil::start_worker(&state);
    oc_api::routes::imports::start_worker(&state);
    let app = oc_api::app(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.api_port as u16));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap_or_else(|e| panic!("listen on {addr}: {e}"));
    println!("[api] listening on :{}", cfg.api_port);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.expect("server");
}
