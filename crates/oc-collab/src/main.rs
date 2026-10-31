//! `oc-collab`: the real-time sync server (drop-in for services/collab, same port and env).

use oc_collab::{Server, Settings, serve};

#[tokio::main]
async fn main() {
    let cfg = oc_config::config();
    cfg.assert_production();
    let pool = oc_db::db().clone();
    let server = Server::new(pool, Settings::from_config(cfg));
    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", cfg.collab_port as u16)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[collab] cannot listen on :{}: {e}", cfg.collab_port);
            std::process::exit(1);
        }
    };
    println!("[collab] listening on :{}", cfg.collab_port);
    if let Err(e) = serve(server, listener, shutdown_signal()).await {
        eprintln!("[collab] {e}");
        std::process::exit(1);
    }
    oc_db::close_db().await;
}

/// SIGINT / SIGTERM / SIGQUIT (Hocuspocus `stopOnSignals`).
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("signal handler");
    let mut quit = signal(SignalKind::quit()).expect("signal handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
        _ = quit.recv() => {}
    }
}
