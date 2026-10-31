//! Port of services/api/src/mcp/live.ts — AI assistants edit designs as live collaborators. For each (design,
//! connected app) the API keeps a Hocuspocus connection to the collab server for a minute after the last change
//! (a small client of the Hocuspocus 3 protocol over tokio-tungstenite, standing in for `@hocuspocus/provider`):
//! the assistant appears in the editor's presence list with its name and selection, its changes reach open editors
//! as normal remote edits, and a change counts as done only when the collab server has acknowledged it
//! (SyncStatus, sent after the update is persisted), so a screenshot or export taken next includes it.
//!
//! Before an assistant's first change in a session a checkpoint version is saved ("Before Claude's changes"):
//! "Undo AI changes" in the editor, or the `undo_ai_changes` tool, restores it.
//!
//! If the collab server cannot be reached (5 s), changes are applied to the stored document and announced through
//! the op log instead (the same path as brand-template updates); everything still works, without presence.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use oc_auth::Role;
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, mpsc};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;
use yrs::{Doc, ReadTxn, Transact, Update};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;

use crate::http::error::ApiError;
use crate::state::AppState;

const IDLE_MS: u64 = 60_000;
const CONNECT_MS: u64 = 5_000;
const ACK_MS: u64 = 10_000;
/// A new checkpoint once an assistant has been quiet on a design this long.
const SESSION_GAP_MS: i64 = 30 * 60_000;
/// Presence colour for assistants (one colour, so people learn to recognise it).
pub const AI_COLOR: &str = "#d9480f";
const REMOTE: &str = "oc-api-collab-remote";

#[derive(Debug, Clone)]
pub struct Agent {
    pub user_id: Uuid,
    pub user_name: String,
    /// Display name of the connected app ("Claude", "Codex", a token's name).
    pub name: String,
    /// api_tokens id: one live session per app connection.
    pub token_id: Uuid,
    pub role: Role,
    pub brand_admin: bool,
}

// ---------------------------------------------------------------- Hocuspocus wire format (lib0 encoding)

mod msg {
    pub const SYNC: u64 = 0;
    pub const AWARENESS: u64 = 1;
    pub const AUTH: u64 = 2;
    pub const QUERY_AWARENESS: u64 = 3;
    pub const SYNC_REPLY: u64 = 4;
    pub const CLOSE: u64 = 7;
    pub const SYNC_STATUS: u64 = 8;
}

fn var_uint(buf: &mut Vec<u8>, mut n: u64) {
    while n > 0x7f {
        buf.push(0x80 | (n & 0x7f) as u8);
        n >>= 7;
    }
    buf.push(n as u8);
}

fn var_bytes(buf: &mut Vec<u8>, b: &[u8]) {
    var_uint(buf, b.len() as u64);
    buf.extend_from_slice(b);
}

fn message(doc: &str, ty: u64, body: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut b = Vec::new();
    var_bytes(&mut b, doc.as_bytes());
    var_uint(&mut b, ty);
    body(&mut b);
    b
}

struct Reader<'a>(&'a [u8], usize);

impl<'a> Reader<'a> {
    fn uint(&mut self) -> Option<u64> {
        let (mut n, mut shift) = (0u64, 0);
        loop {
            let b = *self.0.get(self.1)?;
            self.1 += 1;
            n |= u64::from(b & 0x7f) << shift;
            if b < 0x80 {
                return Some(n);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.uint()? as usize;
        let s = self.0.get(self.1..self.1 + n)?;
        self.1 += n;
        Some(s)
    }
}

// ---------------------------------------------------------------- sessions

struct Session {
    doc: Arc<Doc>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    name: String,
    synced: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    unsynced: Arc<AtomicI64>,
    acked: Arc<Notify>,
    awareness: Mutex<(u64, Map<String, Value>)>,
    last_change: AtomicI64,
    queue: tokio::sync::Mutex<()>,
    touched: AtomicU64,
}

impl Session {
    fn ok(&self) -> bool {
        self.synced.load(Ordering::SeqCst) && self.alive.load(Ordering::SeqCst)
    }

    /// `provider.setAwarenessField(key, value)`.
    fn set_awareness(&self, key: &str, value: Value) {
        let mut a = self.awareness.lock().expect("awareness");
        a.0 += 1;
        a.1.insert(key.into(), value);
        let state = crate::http::jsjson::stringify(&Value::Object(a.1.clone()));
        let clock = a.0;
        drop(a);
        let client = self.doc.client_id();
        let update = {
            let mut u = Vec::new();
            var_uint(&mut u, 1);
            var_uint(&mut u, client.get());
            var_uint(&mut u, clock);
            var_bytes(&mut u, state.as_bytes());
            u
        };
        let _ = self.out.send(message(&self.name, msg::AWARENESS, |b| var_bytes(b, &update)));
    }

    fn awareness_message(&self) -> Vec<u8> {
        let a = self.awareness.lock().expect("awareness");
        let state = crate::http::jsjson::stringify(&Value::Object(a.1.clone()));
        let mut u = Vec::new();
        var_uint(&mut u, 1);
        var_uint(&mut u, self.doc.client_id().get());
        var_uint(&mut u, a.0);
        var_bytes(&mut u, state.as_bytes());
        message(&self.name, msg::AWARENESS, |b| var_bytes(b, &u))
    }
}

#[derive(Default)]
struct Sessions(Mutex<HashMap<String, Arc<Session>>>);

fn key_of(design_id: &str, a: &Agent) -> String {
    format!("{design_id}:{}", a.token_id)
}

fn collab_url(s: &AppState) -> String {
    s.env_nonempty("OC_COLLAB_URL").unwrap_or_else(|| format!("ws://localhost:{}", s.cfg.collab_port))
}

fn ticket(s: &AppState, design_id: &str, a: &Agent) -> String {
    let claims = oc_auth::TicketClaims {
        sub: a.user_id.to_string(),
        name: a.name.clone(),
        color: AI_COLOR.into(),
        aud: oc_auth::Audience::Collab,
        design: design_id.into(),
        role: a.role,
        brand_locked: if a.brand_admin { None } else { Some(true) },
    };
    oc_auth::sign_ticket_with(&s.cfg.secret, &claims, 300)
}

/// Open a connection and wait until it is authenticated and synced; `None` when that does not happen in 5 s or
/// the ticket is refused.
async fn connect(s: &AppState, design_id: &str, a: &Agent) -> Option<Arc<Session>> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(CONNECT_MS);
    let url = collab_url(s);
    let name = format!("design:{design_id}");
    // The provider keeps retrying a refused connection until the timeout.
    let ws = loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        match tokio::time::timeout(left, tokio_tungstenite::connect_async(url.as_str())).await {
            Ok(Ok((ws, _))) => break ws,
            Ok(Err(_)) => tokio::time::sleep(Duration::from_millis(250).min(deadline.saturating_duration_since(tokio::time::Instant::now()))).await,
            Err(_) => return None,
        }
    };
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let doc = Arc::new(oc_crdt::new_doc());
    let mut aw = Map::new();
    aw.insert("user".into(), json!({ "id": format!("ai:{}", a.token_id), "name": a.name, "color": AI_COLOR, "ai": true, "onBehalfOf": a.user_name }));
    let session = Arc::new(Session {
        doc: doc.clone(),
        out: tx.clone(),
        name: name.clone(),
        synced: Arc::new(AtomicBool::new(false)),
        alive: Arc::new(AtomicBool::new(true)),
        unsynced: Arc::new(AtomicI64::new(0)),
        acked: Arc::new(Notify::new()),
        awareness: Mutex::new((0, aw)),
        last_change: AtomicI64::new(0),
        queue: tokio::sync::Mutex::new(()),
        touched: AtomicU64::new(0),
    });
    // Local changes go out as Sync updates and count as unsynced until the server's SyncStatus.
    {
        let (tx, name, unsynced) = (tx.clone(), name.clone(), session.unsynced.clone());
        let _ = doc.observe_update_v1("oc-api-mcp-live", move |txn: &yrs::TransactionMut, e: &yrs::UpdateEvent| {
            let remote = txn.origin().is_some_and(|o| o.as_ref() == REMOTE.as_bytes());
            if !remote {
                unsynced.fetch_add(1, Ordering::SeqCst);
                let _ = tx.send(message(&name, msg::SYNC, |b| {
                    var_uint(b, 2);
                    var_bytes(b, &e.update);
                }));
            }
        });
    }
    // Writer.
    let alive_w = session.alive.clone();
    tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(Message::Binary(m.into())).await.is_err() {
                break;
            }
        }
        alive_w.store(false, Ordering::SeqCst);
        let _ = sink.close().await;
    });
    // Authenticate.
    let _ = tx.send(message(&name, msg::AUTH, |b| {
        var_uint(b, 0);
        var_bytes(b, ticket(s, design_id, a).as_bytes());
    }));
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<bool>();
    let sess = session.clone();
    tokio::spawn(async move {
        let mut ready = Some(ready_tx);
        while let Some(Ok(m)) = stream.next().await {
            let data = match m {
                Message::Binary(b) => b.to_vec(),
                Message::Close(_) => break,
                _ => continue,
            };
            let mut r = Reader(&data, 0);
            if r.bytes().is_none() {
                continue;
            }
            match r.uint() {
                Some(msg::AUTH) => match r.uint() {
                    Some(2) => {
                        let sv = sess.doc.transact().state_vector().encode_v1();
                        let _ = sess.out.send(message(&sess.name, msg::SYNC, |b| {
                            var_uint(b, 0);
                            var_bytes(b, &sv);
                        }));
                        let _ = sess.out.send(sess.awareness_message());
                    }
                    Some(1) => {
                        if let Some(t) = ready.take() {
                            let _ = t.send(false);
                        }
                        break;
                    }
                    _ => {}
                },
                Some(msg::SYNC | msg::SYNC_REPLY) => match r.uint() {
                    Some(0) => {
                        if let Some(sv) = r.bytes().and_then(|b| yrs::StateVector::decode_v1(b).ok()) {
                            let diff = sess.doc.transact().encode_diff_v1(&sv);
                            let _ = sess.out.send(message(&sess.name, msg::SYNC_REPLY, |b| {
                                var_uint(b, 1);
                                var_bytes(b, &diff);
                            }));
                        }
                    }
                    Some(step @ (1 | 2)) => {
                        if let Some(u) = r.bytes().and_then(|b| Update::decode_v1(b).ok()) {
                            let mut txn = sess.doc.transact_mut_with(REMOTE);
                            let _ = txn.apply_update(u);
                        }
                        if step == 1 && !sess.synced.swap(true, Ordering::SeqCst)
                            && let Some(t) = ready.take()
                        {
                            let _ = t.send(true);
                        }
                    }
                    _ => {}
                },
                Some(msg::SYNC_STATUS) => {
                    if r.uint() == Some(1) {
                        let left = sess.unsynced.fetch_sub(1, Ordering::SeqCst) - 1;
                        if left <= 0 {
                            sess.unsynced.store(0, Ordering::SeqCst);
                            sess.acked.notify_waiters();
                        }
                    }
                }
                Some(msg::QUERY_AWARENESS) => {
                    let _ = sess.out.send(sess.awareness_message());
                }
                Some(msg::CLOSE) => break,
                _ => {}
            }
        }
        sess.alive.store(false, Ordering::SeqCst);
        sess.acked.notify_waiters();
        if let Some(t) = ready.take() {
            let _ = t.send(false);
        }
    });
    let left = deadline.saturating_duration_since(tokio::time::Instant::now());
    match tokio::time::timeout(left, ready_rx).await {
        Ok(Ok(true)) => Some(session),
        _ => {
            session.alive.store(false, Ordering::SeqCst);
            None
        }
    }
}

fn close(s: &AppState, key: &str) {
    let sessions = s.local::<Sessions>();
    let removed = sessions.0.lock().expect("sessions").remove(key);
    if let Some(sess) = removed {
        sess.alive.store(false, Ordering::SeqCst);
        // Dropping the last sender ends the writer, which closes the socket.
        let _ = sess.out.send(message(&sess.name, msg::CLOSE, |b| var_bytes(b, b"")));
    }
}

/// Close the session after a minute without changes.
fn touch(s: &AppState, key: &str, sess: &Arc<Session>) {
    let n = sess.touched.fetch_add(1, Ordering::SeqCst) + 1;
    let (s2, key, sess) = (s.clone(), key.to_owned(), sess.clone());
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(IDLE_MS)).await;
        if sess.touched.load(Ordering::SeqCst) == n {
            close(&s2, &key);
        }
    });
}

async fn session_for(s: &AppState, design_id: &str, a: &Agent) -> Option<Arc<Session>> {
    let key = key_of(design_id, a);
    let existing = s.local::<Sessions>().0.lock().expect("sessions").get(&key).cloned();
    if let Some(e) = existing {
        if e.ok() {
            touch(s, &key, &e);
            return Some(e);
        }
        close(s, &key);
    }
    let sess = connect(s, design_id, a).await?;
    s.local::<Sessions>().0.lock().expect("sessions").insert(key.clone(), sess.clone());
    touch(s, &key, &sess);
    Some(sess)
}

async fn wait_for_ack(sess: &Session) -> bool {
    let wait = async {
        loop {
            let notified = sess.acked.notified();
            if sess.unsynced.load(Ordering::SeqCst) <= 0 {
                return true;
            }
            if !sess.alive.load(Ordering::SeqCst) {
                return false;
            }
            notified.await;
        }
    };
    tokio::time::timeout(Duration::from_millis(ACK_MS), wait).await.unwrap_or(false)
}

/// Save "Before {name}'s changes" unless this assistant already has a recent checkpoint on the design.
async fn checkpoint(s: &AppState, design_id: &str, a: &Agent, ydoc: &Doc, last_change: i64) -> Result<(), ApiError> {
    if s.now_ms() - last_change < SESSION_GAP_MS {
        return Ok(());
    }
    let recent: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM design_versions WHERE design_id = $1 AND agent = $2 AND created_by = $3 AND created_at > now() - make_interval(mins => $4)",
    )
    .bind(design_id)
    .bind(&a.name)
    .bind(a.user_id)
    .bind((SESSION_GAP_MS / 60_000) as i32)
    .fetch_optional(&s.pool)
    .await?;
    if recent.is_some() {
        return Ok(());
    }
    let state = ydoc.transact().encode_state_as_update_v1(&yrs::StateVector::default());
    sqlx::query("INSERT INTO design_versions(id, design_id, state, label, created_by, agent) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(oc_db::uuid())
        .bind(design_id)
        .bind(state)
        .bind(format!("Before {}’s changes", a.name))
        .bind(a.user_id)
        .bind(&a.name)
        .execute(&s.pool)
        .await?;
    Ok(())
}

pub struct ChangeResult<T> {
    pub value: T,
    /// Applied through the collab server (false: the op-log fallback).
    #[allow(dead_code)]
    pub live: bool,
    pub acknowledged: bool,
}

/// What collaborators see the assistant select after a change.
#[derive(Default)]
pub struct Presence {
    pub selection: Option<Vec<String>>,
    pub page_id: Option<String>,
}

/// Run `f` against the design's live document as the assistant (one change at a time per app connection).
pub async fn change<T>(
    s: &AppState,
    design_id: &str,
    a: &Agent,
    f: impl FnOnce(&Doc) -> T,
    presence: impl FnOnce(&T) -> Presence,
    save_checkpoint: bool,
) -> Result<ChangeResult<T>, ApiError> {
    let Some(sess) = session_for(s, design_id, a).await else {
        // Collab unreachable: apply to the stored document and announce through the op log.
        let Some(ydoc) = crate::brandhub::load_ydoc(s, design_id).await? else {
            return Err(ApiError::internal("This design has no document yet."));
        };
        if save_checkpoint {
            checkpoint(s, design_id, a, &ydoc, 0).await?;
        }
        let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
        let c = captured.clone();
        ydoc.observe_update_v1("oc-api-mcp", move |_: &yrs::TransactionMut, e: &yrs::UpdateEvent| c.lock().expect("updates").push(e.update.clone()))
            .map_err(ApiError::internal)?;
        let value = f(&ydoc);
        let _ = ydoc.unobserve_update_v1("oc-api-mcp");
        let updates = std::mem::take(&mut *captured.lock().expect("updates"));
        crate::brandhub::persist_server_updates(s, design_id, &updates, Some(a.user_id)).await?;
        return Ok(ChangeResult { value, live: false, acknowledged: true });
    };
    let _turn = sess.queue.lock().await;
    if save_checkpoint {
        checkpoint(s, design_id, a, &sess.doc, sess.last_change.load(Ordering::SeqCst)).await?;
    }
    sess.last_change.store(s.now_ms(), Ordering::SeqCst);
    let value = f(&sess.doc);
    let p = presence(&value);
    if let Some(sel) = p.selection {
        sess.set_awareness("selection", json!(sel));
    }
    if let Some(pid) = p.page_id {
        sess.set_awareness("pageId", json!(pid));
    }
    let acknowledged = wait_for_ack(&sess).await;
    sqlx::query("UPDATE designs SET updated_at = now() WHERE id = $1").bind(design_id).execute(&s.pool).await?;
    Ok(ChangeResult { value, live: true, acknowledged })
}

/// The freshest copy of a design for reading: the assistant's live document when one is open, else the stored one.
pub async fn read_doc(s: &AppState, design_id: &str, token_id: Uuid) -> Result<Option<Doc>, ApiError> {
    let key = format!("{design_id}:{token_id}");
    let sess = s.local::<Sessions>().0.lock().expect("sessions").get(&key).cloned();
    if let Some(sess) = sess.filter(|x| x.ok()) {
        let state = sess.doc.transact().encode_state_as_update_v1(&yrs::StateVector::default());
        let copy = oc_crdt::new_doc();
        oc_crdt::apply_update(&copy, &state, None)?;
        return Ok(Some(copy));
    }
    crate::brandhub::load_ydoc(s, design_id).await
}

/// Latest checkpoint an assistant saved on a design (for "Undo AI changes").
pub struct Checkpoint {
    pub id: Uuid,
    pub label: Option<String>,
    pub agent: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub state: Vec<u8>,
}

pub async fn latest_checkpoint(s: &AppState, design_id: &str) -> Result<Option<Checkpoint>, ApiError> {
    type Row = (Uuid, Option<String>, String, chrono::DateTime<chrono::Utc>, Vec<u8>);
    let r: Option<Row> = sqlx::query_as(
        "SELECT id, label, agent, created_at, state FROM design_versions WHERE design_id = $1 AND agent IS NOT NULL ORDER BY created_at DESC LIMIT 1",
    )
    .bind(design_id)
    .fetch_optional(&s.pool)
    .await?;
    Ok(r.map(|(id, label, agent, created_at, state)| Checkpoint { id, label, agent, created_at, state }))
}
