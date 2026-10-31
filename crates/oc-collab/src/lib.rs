//! Real-time sync server — the Rust port of `services/collab` (ADR-0001). It speaks the Hocuspocus 3
//! WebSocket protocol exactly as `@hocuspocus/provider` 3.4.4 expects it, keeps documents as yrs docs
//! (wire-compatible with the browser's yjs), and persists to the same Postgres tables:
//!
//! * Auth: 5-minute collab ticket (`oc-auth`); view/comment roles get read-only connections.
//! * Durability (CO-002): a client update is appended to `doc_updates` *before* it is applied and
//!   acknowledged (SyncStatus), so an acknowledged edit is always on disk. A debounced store (1.5 s, max
//!   8 s) folds the log into `doc_state` and truncates it; load = snapshot + log tail, then migrations
//!   and the structural repair pass (CO-004).
//! * Revocation (CO-006) and server-side document changes arrive over `LISTEN oc_revoke` / `oc_doc_update`, on a
//!   reconnecting connection; after a reconnect every client socket is closed (4403) and older tickets are refused.
//! * An automatic version snapshot at most every 10 minutes; a thumbnail refresh after each store.

pub mod awareness;
pub mod persistence;
pub mod protocol;
pub mod server;
pub mod socket;
pub mod ydoc;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{WebSocketUpgrade, rejection::WebSocketUpgradeRejection};
use axum::response::{IntoResponse, Response};

pub use server::{Server, Settings};

/// The HTTP side: every WebSocket upgrade on any path is a Hocuspocus connection; plain requests get the
/// Hocuspocus banner (used as the health check).
pub fn router(server: Arc<Server>) -> Router {
    Router::new().fallback(handler).with_state(server)
}

async fn handler(State(server): State<Arc<Server>>, ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>) -> Response {
    match ws {
        // ws's default maxPayload is 100 MiB.
        Ok(ws) => ws
            .max_message_size(100 * 1024 * 1024)
            .max_frame_size(100 * 1024 * 1024)
            .on_upgrade(move |socket| socket::run(server, socket))
            .into_response(),
        Err(_) => ([("content-type", "text/plain")], "Welcome to Hocuspocus!").into_response(),
    }
}

/// Listen for `oc_revoke` / `oc_doc_update` on a reconnecting LISTEN connection (`oc_db::listener`: backoff,
/// re-LISTEN, ping). Notifications sent while it is down are lost, so a reconnect closes every client socket
/// ([`Server::on_listener_reconnect`]). Resolves once the first LISTEN is in place (startup waits for it).
pub async fn listen_notifications(server: Arc<Server>) -> Result<oc_db::Listener, oc_db::listener::ListenerClosed> {
    listen_notifications_with(server, oc_db::ListenerTiming::default()).await
}

/// [`listen_notifications`] with explicit deadlines (tests).
pub async fn listen_notifications_with(server: Arc<Server>, timing: oc_db::ListenerTiming) -> Result<oc_db::Listener, oc_db::listener::ListenerClosed> {
    let on_notify = {
        let server = server.clone();
        move |n: sqlx::postgres::PgNotification| {
            // A malformed payload is ignored.
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(n.payload()) else { return };
            let s = server.clone();
            let channel = n.channel().to_owned();
            tokio::spawn(async move {
                let r = if channel == "oc_doc_update" { s.merge_server_update(&payload).await } else { s.revoke(&payload).await.map(|_| ()) };
                if let Err(e) = r {
                    eprintln!("[collab] {channel}: {e}");
                }
            });
        }
    };
    let weak = Arc::downgrade(&server);
    let opts = oc_db::ListenerOptions::named("collab")
        .on_reconnect(move || {
            if let Some(s) = weak.upgrade() {
                s.on_listener_reconnect();
            }
        })
        .timing(timing);
    let l = oc_db::create_listener((*server.pool.connect_options()).clone(), &["oc_revoke", "oc_doc_update"], on_notify, opts);
    l.ready().await?;
    Ok(l)
}

/// Run the server on `listener` until `shutdown` resolves, then close every connection, flush pending
/// stores (like Hocuspocus `destroy()`), and return.
pub async fn serve(server: Arc<Server>, listener: tokio::net::TcpListener, shutdown: impl Future<Output = ()> + Send + 'static) -> std::io::Result<()> {
    let listen = listen_notifications(server.clone()).await.map_err(std::io::Error::other)?;
    let background = {
        let s = server.clone();
        tokio::spawn(async move {
            let mut prune = tokio::time::interval(Duration::from_secs(60));
            let mut aw = tokio::time::interval(Duration::from_secs(3));
            loop {
                tokio::select! {
                    _ = prune.tick() => s.prune_revocations(oc_js::now_ms()),
                    _ = aw.tick() => s.expire_awareness(),
                }
            }
        })
    };
    let hb = {
        let s = server.clone();
        oc_jobs::heartbeat(server.pool.clone(), "collab", move || serde_json::json!({ "documents": s.documents_count(), "connections": s.connections_count() }))
    };
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let srv = axum::serve(listener, router(server.clone())).with_graceful_shutdown(async move {
        let _ = stop_rx.await;
    });
    let http = tokio::spawn(async move { srv.await });
    shutdown.await;
    // Stop accepting, close every connection (stores run as documents lose their last connection).
    server.close_all();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while server.resident() > 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = stop_tx.send(());
    hb.stop();
    background.abort();
    listen.close().await;
    // Open sockets keep axum's graceful shutdown waiting; give it a moment, then return regardless.
    let _ = tokio::time::timeout(Duration::from_millis(500), http).await;
    Ok(())
}
