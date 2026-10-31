//! One WebSocket (`ClientConnection`): multiplexes documents by name, queues a document's messages until
//! its Auth message has been handled, keeps the socket alive with ping/pong.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::protocol::{self, CloseEvent, Decoder, msg};
use crate::server::{Connection, Out, Server, lock};

enum Slot {
    /// Waiting for (or processing) the Auth message; other messages are queued.
    Queueing { authenticating: bool, queue: Vec<Vec<u8>> },
    Open(Arc<Connection>),
}

struct SocketState {
    id: Uuid,
    out: mpsc::UnboundedSender<Out>,
    slots: Mutex<HashMap<String, Slot>>,
    pong: AtomicBool,
    closed: AtomicBool,
}

/// Drive one upgraded WebSocket until it closes.
pub async fn run(server: Arc<Server>, ws: WebSocket) {
    let (mut sink, mut stream) = ws.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Out>();
    let st = Arc::new(SocketState { id: Uuid::new_v4(), out: out_tx, slots: Mutex::default(), pong: AtomicBool::new(true), closed: AtomicBool::new(false) });

    let writer = tokio::spawn(async move {
        while let Some(o) = out_rx.recv().await {
            let r = match o {
                Out::Binary(b) => sink.send(Message::Binary(b.into())).await,
                Out::Ping => sink.send(Message::Ping(Default::default())).await,
                Out::Close(code, reason) => {
                    let _ = sink.send(Message::Close(Some(CloseFrame { code, reason: reason.into() }))).await;
                    break;
                }
            };
            if r.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    // Keep-alive: ping every `timeout`; no pong since the last ping closes the connections.
    let pinger = {
        let (st, server) = (st.clone(), server.clone());
        tokio::spawn(async move {
            let period = server.settings.timeout;
            let mut iv = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            loop {
                iv.tick().await;
                if !st.pong.swap(false, Ordering::SeqCst) {
                    close_all(&server, &st, CloseEvent::connection_timeout());
                    let _ = st.out.send(Out::Close(4408, "Connection Timeout".into()));
                    return;
                }
                if st.out.send(Out::Ping).is_err() {
                    return;
                }
            }
        })
    };

    while let Some(frame) = stream.next().await {
        let data = match frame {
            Ok(Message::Binary(b)) => b.to_vec(),
            Ok(Message::Text(t)) => t.as_bytes().to_vec(),
            Ok(Message::Pong(_)) => {
                st.pong.store(true, Ordering::SeqCst);
                continue;
            }
            Ok(Message::Ping(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => break,
        };
        if !on_message(&server, &st, data) {
            break;
        }
    }

    st.closed.store(true, Ordering::SeqCst);
    pinger.abort();
    // `handleWebsocketClose`: close every document connection of this socket.
    close_all(&server, &st, CloseEvent::new(1000, ""));
    let _ = st.out.send(Out::Close(1000, String::new()));
    let _ = writer.await;
}

fn close_all(server: &Server, st: &SocketState, ev: CloseEvent) {
    let conns: Vec<Arc<Connection>> = lock(&st.slots)
        .values()
        .filter_map(|s| match s {
            Slot::Open(c) => Some(c.clone()),
            Slot::Queueing { .. } => None,
        })
        .collect();
    for c in conns {
        server.close_connection(&c, Some(ev.clone()));
    }
}

/// `messageHandler` + `handleQueueingMessage`. Returns false when the socket must be closed.
fn on_message(server: &Arc<Server>, st: &Arc<SocketState>, data: Vec<u8>) -> bool {
    let mut d = Decoder::new(&data);
    let Ok(name) = d.var_string() else {
        // An undecodable payload: close the WebSocket (`Unauthorized`).
        let ev = CloseEvent::unauthorized();
        let _ = st.out.send(Out::Close(ev.code, ev.reason));
        return false;
    };
    let mut slots = lock(&st.slots);
    let slot = slots.entry(name.clone()).or_insert_with(|| Slot::Queueing { authenticating: false, queue: Vec::new() });
    let (authenticating, queue) = match slot {
        Slot::Open(conn) => {
            let _ = conn.inbox.send(Some(data));
            return true;
        }
        Slot::Queueing { authenticating, queue } => (authenticating, queue),
    };
    let ty = match d.var_uint() {
        Ok(t) => t,
        Err(_) => {
            let ev = CloseEvent::reset_connection();
            let _ = st.out.send(Out::Close(ev.code, ev.reason));
            return false;
        }
    };
    if !(ty == msg::AUTH && !*authenticating) {
        queue.push(data);
        return true;
    }
    *authenticating = true;
    // The 2nd integer is the auth submessage type (always Token from client to server).
    let token = d.var_uint().and_then(|_| d.var_string());
    let Ok(token) = token else {
        let ev = CloseEvent::reset_connection();
        let _ = st.out.send(Out::Close(ev.code, ev.reason));
        return false;
    };
    drop(slots);
    let (server, st) = (server.clone(), st.clone());
    tokio::spawn(async move { establish(server, st, name, token).await });
    true
}

/// `onAuthenticate` → Authenticated → `setUpNewConnection`; any failure → PermissionDenied.
async fn establish(server: Arc<Server>, st: Arc<SocketState>, name: String, token: String) {
    let deny = |st: &SocketState, name: &str| {
        // The queue of a refused document is never processed; drop it.
        if let Some(Slot::Queueing { queue, .. }) = lock(&st.slots).get_mut(name) {
            queue.clear();
        }
        let _ = st.out.send(Out::Binary(protocol::permission_denied(name, "permission-denied")));
    };
    let (user, read_only) = match server.authenticate(&name, &token) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[onAuthenticate] {e}");
            return deny(&st, &name);
        }
    };
    let _ = st.out.send(Out::Binary(protocol::authenticated(&name, read_only)));
    let doc = match server.acquire(&name).await {
        Ok(d) => d,
        Err(_) => return deny(&st, &name),
    };
    let (inbox_tx, mut inbox_rx) = mpsc::unbounded_channel::<Option<Vec<u8>>>();
    let conn = server.attach(&doc, st.id, st.out.clone(), inbox_tx, user, read_only);
    {
        let (st2, name2, id) = (Arc::downgrade(&st), name.clone(), conn.id);
        *lock(&conn.on_close) = Some(Box::new(move || {
            if let Some(st) = st2.upgrade() {
                let mut slots = lock(&st.slots);
                if matches!(slots.get(&name2), Some(Slot::Open(c)) if c.id == id) {
                    slots.remove(&name2);
                }
            }
        }));
    }
    // The connection task handles this document's messages one at a time, in arrival order.
    {
        let (server, conn) = (server.clone(), conn.clone());
        tokio::spawn(async move {
            while let Some(Some(m)) = inbox_rx.recv().await {
                if conn.is_closed() {
                    break;
                }
                server.handle_message(&conn, m).await;
            }
        });
    }
    let queued = {
        let mut slots = lock(&st.slots);
        let q = match slots.remove(&name) {
            Some(Slot::Queueing { queue, .. }) => queue,
            _ => Vec::new(),
        };
        slots.insert(name.clone(), Slot::Open(conn.clone()));
        q
    };
    // The WebSocket already went away: clean up immediately.
    if st.closed.load(Ordering::SeqCst) {
        server.close_connection(&conn, None);
        return;
    }
    for m in queued {
        let _ = conn.inbox.send(Some(m));
    }
}
