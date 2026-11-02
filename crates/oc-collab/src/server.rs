//! The Hocuspocus core (documents, connections, debounced store, unload) with the Pinselo hooks of
//! services/collab/src/main.ts wired in.
//!
//! Locking: a document's `inner` mutex guards its connections, awareness and every *write* to the Y.Doc,
//! so updates are applied and broadcast in one order. Reads of the Y.Doc happen outside it. `log_lock`
//! makes "append to the op log → apply" atomic with respect to a store's "max(seq) → encode state", so a
//! store never truncates a logged update that is not yet in the snapshot.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use sqlx::PgPool;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;
use yrs::{Doc, Origin};

use crate::awareness::{Awareness, AwarenessChange};
use crate::persistence;
use crate::protocol::{self, CloseEvent};
use crate::ydoc;

/// Tickets live 5 minutes (`TICKET_TTL_S`).
pub const TICKET_TTL_S: i64 = 300;
/// At most one automatic version snapshot per design every 10 minutes.
pub const VERSION_EVERY_MS: i64 = 10 * 60 * 1000;
/// Clock skew allowed between the API (which stamps tickets) and this server when applying the reconnect watermark.
pub const RECONNECT_SKEW_MS: i64 = 30_000;

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Server settings (`new Server({ port, debounce: 1500, maxDebounce: 8000, timeout: 30000 })`).
#[derive(Debug, Clone)]
pub struct Settings {
    pub secret: String,
    pub render_url: String,
    pub debounce: Duration,
    pub max_debounce: Duration,
    /// Ping interval / pong timeout (`timeout`).
    pub timeout: Duration,
}

impl Settings {
    pub fn from_config(c: &oc_config::Config) -> Self {
        Settings {
            secret: c.secret.clone(),
            render_url: c.render_url.clone(),
            debounce: Duration::from_millis(1500),
            max_debounce: Duration::from_millis(8000),
            timeout: Duration::from_millis(30_000),
        }
    }
}

/// What the WebSocket writer sends.
#[derive(Debug)]
pub enum Out {
    Binary(Vec<u8>),
    Ping,
    /// Close the WebSocket itself (not a Hocuspocus CLOSE message).
    Close(u16, String),
}

/// `context.user` set by `onAuthenticate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub color: String,
    pub role: oc_auth::Role,
}

impl User {
    /// The actor for `doc_updates.actor_id` / `design_versions.created_by` (TS passes the string to pg).
    pub fn actor(&self) -> Result<Uuid, String> {
        Uuid::parse_str(&self.id).map_err(|e| format!("invalid input syntax for type uuid: \"{}\" ({e})", self.id))
    }
}

/// One document opened by one WebSocket (`Connection`).
pub struct Connection {
    pub id: u64,
    pub socket_id: Uuid,
    pub doc: Arc<Document>,
    pub read_only: bool,
    pub user: User,
    pub out: UnboundedSender<Out>,
    /// Inbox of the connection task (`None` stops it).
    pub inbox: UnboundedSender<Option<Vec<u8>>>,
    /// Called once when the connection closes (removes the socket's slot).
    pub on_close: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    closed: AtomicBool,
}

impl Connection {
    pub fn send(&self, msg: Vec<u8>) {
        let _ = self.out.send(Out::Binary(msg));
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
    fn origin(&self) -> Origin {
        Origin::from(format!("oc-collab:conn:{}", self.id).as_str())
    }
}

/// Where a document change came from (the yjs transaction origin).
#[derive(Clone)]
pub enum UpdateOrigin {
    Connection(Arc<Connection>),
    /// `mergeServerUpdate` (`'server'`).
    Server,
}

struct DocInner {
    conns: IndexMap<u64, Arc<Connection>>,
    /// Awareness client ids announced by each connection.
    clients: HashMap<u64, HashSet<u64>>,
    awareness: Awareness,
}

#[derive(Default)]
struct StoreState {
    timer: Option<tokio::task::JoinHandle<()>>,
    start: Option<Instant>,
    /// Context of the latest change (`hookPayload.context.user.id`).
    actor: Option<Uuid>,
}

/// A loaded (or loading) document (`Document`).
pub struct Document {
    pub name: String,
    pub design_id: String,
    pub ydoc: Doc,
    inner: Mutex<DocInner>,
    captured: Arc<Mutex<Vec<Vec<u8>>>>,
    loaded: tokio::sync::OnceCell<Result<(), String>>,
    is_loading: AtomicBool,
    /// Connections attached or being set up (counts toward "has connections" for unloading).
    refs: AtomicUsize,
    log_lock: tokio::sync::RwLock<()>,
    save_lock: tokio::sync::Mutex<()>,
    executing: AtomicBool,
    store: Mutex<StoreState>,
}

impl Document {
    fn new(name: String, design_id: String) -> Arc<Self> {
        let ydoc = oc_crdt::new_doc();
        let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
        let c = captured.clone();
        ydoc.observe_update_v1("oc-collab", move |_: &yrs::TransactionMut, e: &yrs::UpdateEvent| lock(&c).push(e.update.clone()))
            .expect("no open transaction on a new doc");
        Arc::new(Document {
            name,
            design_id,
            ydoc,
            inner: Mutex::new(DocInner { conns: IndexMap::new(), clients: HashMap::new(), awareness: Awareness::default() }),
            captured,
            loaded: tokio::sync::OnceCell::new(),
            is_loading: AtomicBool::new(true),
            refs: AtomicUsize::new(0),
            log_lock: tokio::sync::RwLock::new(()),
            save_lock: tokio::sync::Mutex::new(()),
            executing: AtomicBool::new(false),
            store: Mutex::new(StoreState::default()),
        })
    }

    pub fn connections(&self) -> Vec<Arc<Connection>> {
        lock(&self.inner).conns.values().cloned().collect()
    }

    pub fn connection_count(&self) -> usize {
        lock(&self.inner).conns.len()
    }

    fn is_loaded(&self) -> bool {
        matches!(self.loaded.get(), Some(Ok(())))
    }

    fn is_debounced(&self) -> bool {
        lock(&self.store).timer.is_some()
    }

    fn broadcast_awareness(inner: &DocInner, name: &str, ch: &AwarenessChange) {
        let msg = protocol::awareness(name, &inner.awareness.encode(&ch.changed()));
        for c in inner.conns.values() {
            c.send(msg.clone());
        }
    }

    /// Apply an awareness update from a connection and relay the change to every connection.
    fn apply_awareness(&self, conn: &Connection, update: &[u8]) -> Result<(), String> {
        let mut g = lock(&self.inner);
        let ch = g.awareness.apply(update, oc_js::now_ms()).map_err(|e| e.to_string())?;
        if ch.is_empty() {
            return Ok(());
        }
        if g.conns.contains_key(&conn.id) {
            let set = g.clients.entry(conn.id).or_default();
            set.extend(&ch.added);
            for r in &ch.removed {
                set.remove(r);
            }
        }
        Self::broadcast_awareness(&g, &self.name, &ch);
        Ok(())
    }

    /// Drop awareness states not renewed within the y-protocols timeout.
    pub fn expire_awareness(&self, now: i64) {
        let mut g = lock(&self.inner);
        let outdated = g.awareness.outdated(now);
        if outdated.is_empty() {
            return;
        }
        let ch = g.awareness.remove(&outdated);
        if !ch.is_empty() {
            Self::broadcast_awareness(&g, &self.name, &ch);
        }
    }

    /// Apply an update, broadcast each resulting yjs update to every connection, and return them.
    /// yjs swallows apply errors inside y-protocols (`console.error`); so does this.
    fn apply(&self, update: &[u8], origin: Origin) -> Vec<Vec<u8>> {
        let g = lock(&self.inner);
        if let Err(e) = oc_crdt::apply_update(&self.ydoc, update, Some(origin)) {
            eprintln!("Caught error while handling a Yjs update {e}");
        }
        let updates = std::mem::take(&mut *lock(&self.captured));
        for u in &updates {
            let msg = protocol::sync_update(&self.name, u);
            for c in g.conns.values() {
                c.send(msg.clone());
            }
        }
        updates
    }
}

/// Revocation watermarks: tickets issued before these instants are refused (CO-006).
#[derive(Default)]
pub struct Revocations {
    pub design: HashMap<String, i64>,
    pub user: HashMap<String, i64>,
    pub pair: HashMap<String, i64>,
}

/// The server (`Hocuspocus` + the Pinselo hooks).
pub struct Server {
    pub pool: PgPool,
    pub settings: Settings,
    docs: Mutex<HashMap<String, Arc<Document>>>,
    pub revoked: Mutex<Revocations>,
    /// Set when the LISTEN connection comes back: revocations sent while it was down are unknown, so every older
    /// ticket is refused (less [`RECONNECT_SKEW_MS`]).
    reconnected_at: std::sync::atomic::AtomicI64,
    last_version_at: Mutex<HashMap<String, i64>>,
    next_conn: AtomicU64,
    http: reqwest::Client,
    me: Weak<Server>,
}

/// `designIdOf(documentName)`: room = `design:{16 alphanumerics}`.
pub fn design_id_of(name: &str) -> Option<&str> {
    let id = name.strip_prefix("design:")?;
    (id.len() == 16 && id.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(id)
}

impl Server {
    pub fn new(pool: PgPool, settings: Settings) -> Arc<Server> {
        Arc::new_cyclic(|me| Server {
            pool,
            settings,
            docs: Mutex::default(),
            revoked: Mutex::default(),
            reconnected_at: std::sync::atomic::AtomicI64::new(0),
            last_version_at: Mutex::default(),
            next_conn: AtomicU64::new(1),
            http: reqwest::Client::new(),
            me: me.clone(),
        })
    }

    fn arc(&self) -> Arc<Server> {
        self.me.upgrade().expect("server alive")
    }

    /// Loaded documents (`hocuspocus.documents`).
    pub fn documents(&self) -> Vec<Arc<Document>> {
        lock(&self.docs).values().filter(|d| d.is_loaded()).cloned().collect()
    }

    pub fn document(&self, name: &str) -> Option<Arc<Document>> {
        lock(&self.docs).get(name).filter(|d| d.is_loaded()).cloned()
    }

    pub fn documents_count(&self) -> usize {
        self.documents().len()
    }

    /// `getConnectionsCount()`: unique sockets across documents.
    pub fn connections_count(&self) -> usize {
        let mut s = HashSet::new();
        for d in self.documents() {
            for c in d.connections() {
                s.insert(c.socket_id);
            }
        }
        s.len()
    }

    // ------------------------------------------------------------------ auth

    /// `onAuthenticate`: verify the collab ticket for this room; returns the user and read-only flag.
    pub fn authenticate(&self, name: &str, token: &str) -> Result<(User, bool), String> {
        let design_id = design_id_of(name).ok_or("bad room name")?;
        let t = oc_auth::verify_ticket_with(&self.settings.secret, token, oc_auth::Audience::Collab).ok_or("unauthorized")?;
        if t.claims.design != design_id {
            return Err("unauthorized".into());
        }
        let issued_at = t.iat;
        let revoked = {
            let r = lock(&self.revoked);
            let get = |m: &HashMap<String, i64>, k: &str| m.get(k).copied().unwrap_or(0);
            let reconnected = self.reconnected_at.load(Ordering::SeqCst);
            reconnected.max(get(&r.design, design_id)).max(get(&r.user, &t.claims.sub)).max(get(&r.pair, &format!("{design_id}:{}", t.claims.sub)))
        };
        // Strictly earlier: a ticket fetched in response to the revocation itself must pass.
        if issued_at < revoked {
            return Err("revoked".into());
        }
        // Read-only at the transport layer: document updates from read-only connections are dropped.
        let read_only = !oc_auth::at_least(Some(t.claims.role), oc_auth::Role::Edit);
        let c = t.claims;
        Ok((User { id: c.sub, name: c.name, color: c.color, role: c.role }, read_only))
    }

    // ------------------------------------------------------------------ documents

    /// `createDocument`: get or load the document and take a reference on it for a new connection.
    pub async fn acquire(&self, name: &str) -> Result<Arc<Document>, String> {
        let design_id = design_id_of(name).ok_or("bad room name")?.to_owned();
        let doc = {
            let mut docs = lock(&self.docs);
            let d = docs.entry(name.to_owned()).or_insert_with(|| Document::new(name.to_owned(), design_id)).clone();
            d.refs.fetch_add(1, Ordering::SeqCst);
            d
        };
        let res = doc
            .loaded
            .get_or_init(|| async {
                let r = persistence::load(&self.pool, &doc.design_id, &doc.ydoc).await.map(|_| ());
                // Changes made while loading are not broadcast or logged (hooks attach after load).
                lock(&doc.captured).clear();
                if let Err(e) = &r {
                    eprintln!("[onLoadDocument] {e}");
                }
                r
            })
            .await
            .clone();
        match res {
            Ok(()) => {
                doc.is_loading.store(false, Ordering::SeqCst);
                Ok(doc)
            }
            Err(e) => {
                doc.refs.fetch_sub(1, Ordering::SeqCst);
                // A failed load leaves nothing behind; the next connection tries again.
                let mut docs = lock(&self.docs);
                if docs.get(name).is_some_and(|d| Arc::ptr_eq(d, &doc)) && doc.refs.load(Ordering::SeqCst) == 0 {
                    docs.remove(name);
                }
                Err(e)
            }
        }
    }

    /// Register a connection on a loaded, acquired document (`new Connection(...)`): sends the current
    /// awareness states to the new connection.
    #[allow(clippy::too_many_arguments)]
    pub fn attach(
        &self,
        doc: &Arc<Document>,
        socket_id: Uuid,
        out: UnboundedSender<Out>,
        inbox: UnboundedSender<Option<Vec<u8>>>,
        user: User,
        read_only: bool,
    ) -> Arc<Connection> {
        let conn = Arc::new(Connection {
            id: self.next_conn.fetch_add(1, Ordering::SeqCst),
            socket_id,
            doc: doc.clone(),
            read_only,
            user,
            out,
            inbox,
            on_close: Mutex::new(None),
            closed: AtomicBool::new(false),
        });
        let mut g = lock(&doc.inner);
        g.conns.insert(conn.id, conn.clone());
        if g.awareness.has_states() {
            conn.send(protocol::awareness(&doc.name, &g.awareness.encode(&g.awareness.client_ids())));
        }
        conn
    }

    /// Release an acquired reference that never became a connection.
    pub fn release(&self, doc: &Arc<Document>) {
        doc.refs.fetch_sub(1, Ordering::SeqCst);
        self.after_disconnect(doc.clone());
    }

    /// `Connection.close(event)`: unregister (dropping its awareness states), send the CLOSE message,
    /// then run the last-connection logic.
    pub fn close_connection(&self, conn: &Arc<Connection>, event: Option<CloseEvent>) {
        let doc = conn.doc.clone();
        {
            let mut g = lock(&doc.inner);
            if !g.conns.contains_key(&conn.id) {
                return;
            }
            conn.closed.store(true, Ordering::SeqCst);
            let clients: Vec<u64> = g.clients.remove(&conn.id).map(|s| s.into_iter().collect()).unwrap_or_default();
            let ch = g.awareness.remove(&clients);
            if !ch.is_empty() {
                Document::broadcast_awareness(&g, &doc.name, &ch);
            }
            g.conns.shift_remove(&conn.id);
        }
        doc.refs.fetch_sub(1, Ordering::SeqCst);
        if let Some(f) = lock(&conn.on_close).take() {
            f();
        }
        let _ = conn.inbox.send(None);
        conn.send(protocol::close(&doc.name, event.as_ref().map_or("Server closed the connection", |e| e.reason.as_str())));
        self.after_disconnect(doc);
    }

    /// The `clientConnection.onClose` handler of `Hocuspocus.handleConnection`.
    fn after_disconnect(&self, doc: Arc<Document>) {
        if doc.refs.load(Ordering::SeqCst) > 0 {
            return;
        }
        let me = self.arc();
        if !doc.is_loading.load(Ordering::SeqCst) && doc.is_debounced() {
            // Store now (unloadImmediately); the store unloads afterwards.
            me.execute_now(&doc);
        } else {
            me.unload(&doc);
        }
    }

    fn should_unload(doc: &Document) -> bool {
        !doc.is_debounced() && !doc.executing.load(Ordering::SeqCst) && doc.refs.load(Ordering::SeqCst) == 0
    }

    /// `unloadDocument`.
    pub fn unload(&self, doc: &Arc<Document>) {
        let mut docs = lock(&self.docs);
        if Self::should_unload(doc) && docs.get(&doc.name).is_some_and(|d| Arc::ptr_eq(d, doc)) {
            docs.remove(&doc.name);
        }
    }

    // ------------------------------------------------------------------ changes

    /// Apply a change and run the update hooks (`onChange`, debounced `onStoreDocument`).
    pub fn apply_update(&self, doc: &Arc<Document>, update: &[u8], origin: UpdateOrigin) {
        let yorigin = match &origin {
            UpdateOrigin::Connection(c) => c.origin(),
            UpdateOrigin::Server => Origin::from("server"),
        };
        let updates = doc.apply(update, yorigin);
        if updates.is_empty() {
            return;
        }
        let actor = match &origin {
            UpdateOrigin::Connection(c) => c.user.actor().ok(),
            UpdateOrigin::Server => None,
        };
        for u in updates {
            // onChange: server-side changes are logged here; client updates were logged before they applied.
            if matches!(origin, UpdateOrigin::Server) {
                let (pool, id) = (self.pool.clone(), doc.design_id.clone());
                tokio::spawn(async move {
                    if let Err(e) = persistence::log_update(&pool, &id, &u, None).await {
                        eprintln!("[onChange] {e}");
                    }
                });
            }
        }
        self.debounce_store(doc, actor);
    }

    /// `storeDocumentHooks`: debounce 1.5 s, at most 8 s after the first pending change.
    fn debounce_store(&self, doc: &Arc<Document>, actor: Option<Uuid>) {
        let mut s = lock(&doc.store);
        let now = Instant::now();
        let start = s.start.unwrap_or(now);
        if let Some(t) = s.timer.take() {
            t.abort();
        }
        s.actor = actor;
        if now.duration_since(start) >= self.settings.max_debounce {
            s.start = None;
            drop(s);
            self.spawn_store(doc.clone());
            return;
        }
        s.start = Some(start);
        let (me, d, wait) = (self.arc(), doc.clone(), self.settings.debounce);
        s.timer = Some(tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            me.run_store(d, true).await;
        }));
    }

    /// `debouncer.executeNow`.
    fn execute_now(&self, doc: &Arc<Document>) {
        let mut s = lock(&doc.store);
        if let Some(t) = s.timer.take() {
            t.abort();
            s.start = None;
            drop(s);
            self.spawn_store(doc.clone());
        }
    }

    fn spawn_store(&self, doc: Arc<Document>) {
        let me = self.arc();
        doc.executing.store(true, Ordering::SeqCst);
        tokio::spawn(async move { me.run_store(doc, false).await });
    }

    async fn run_store(&self, doc: Arc<Document>, from_timer: bool) {
        // Cancellation (a newer change re-arming the timer) can only land on this await.
        let save = doc.save_lock.lock().await;
        let actor = {
            let mut s = lock(&doc.store);
            if from_timer {
                // Our own timer fired: it is no longer pending.
                s.timer = None;
                s.start = None;
            }
            s.actor
        };
        doc.executing.store(true, Ordering::SeqCst);
        if let Err(e) = self.store_document(&doc, actor).await {
            eprintln!("Caught error during storeDocumentHooks {e}");
        }
        drop(save);
        doc.executing.store(false, Ordering::SeqCst);
        let me = self.arc();
        tokio::spawn(async move {
            if Server::should_unload(&doc) {
                me.unload(&doc);
            }
        });
    }

    /// `onStoreDocument`.
    async fn store_document(&self, doc: &Arc<Document>, actor: Option<Uuid>) -> Result<(), String> {
        let data = {
            let _log = doc.log_lock.write().await;
            // Snapshot covers every logged update up to `upto`. Apply the ones it doesn't cover yet before encoding (yjs
            // updates are idempotent): a server update whose oc_doc_update notification was lost, or a client update
            // logged but not applied yet, would otherwise be truncated away.
            let upto = persistence::max_seq(&self.pool, &doc.design_id).await.map_err(|e| e.to_string())?;
            for u in persistence::unfolded(&self.pool, &doc.design_id, upto).await.map_err(|e| e.to_string())? {
                self.apply_update(doc, &u, UpdateOrigin::Server);
            }
            persistence::StoreData {
                upto,
                state: oc_crdt::encode_state(&doc.ydoc),
                page_count: persistence::page_count(&doc.ydoc),
                meta: persistence::meta(&doc.ydoc),
            }
        };
        persistence::store(&self.pool, &doc.design_id, &data).await.map_err(|e| e.to_string())?;
        let now = oc_js::now_ms();
        let due = {
            let mut m = lock(&self.last_version_at);
            let due = now - m.get(&doc.design_id).copied().unwrap_or(0) > VERSION_EVERY_MS;
            if due {
                m.insert(doc.design_id.clone(), now);
            }
            due
        };
        if due {
            persistence::insert_version(&self.pool, &doc.design_id, &data.state, actor).await.map_err(|e| e.to_string())?;
        }
        self.refresh_thumbnail(&doc.design_id);
        Ok(())
    }

    /// Ask the render service to refresh the design thumbnail (optional in dev: errors are ignored).
    fn refresh_thumbnail(&self, design_id: &str) {
        let url = format!("{}/internal/thumbnail", self.settings.render_url);
        let body = serde_json::json!({ "designId": design_id }).to_string();
        let req = self
            .http
            .post(url)
            .header("content-type", "application/json")
            .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&self.settings.secret))
            .body(body);
        tokio::spawn(async move {
            let _ = req.send().await;
        });
    }

    // ------------------------------------------------------------------ message handling

    /// `beforeHandleMessage` (ack-after-persist, CO-002): log a client update before it is applied (and
    /// then acknowledged). Returns the op-log guard the caller holds until the update is applied.
    pub async fn before_handle<'a>(&self, conn: &'a Arc<Connection>, raw: &[u8]) -> Result<Option<tokio::sync::RwLockReadGuard<'a, ()>>, CloseEvent> {
        if conn.read_only {
            return Ok(None);
        }
        let Some(update) = protocol::update_of(raw) else { return Ok(None) };
        let doc = &conn.doc;
        let guard = doc.log_lock.read().await;
        // Nothing new (e.g. a sync step 2 the server already has): no log entry.
        // Malformed updates are refused before they reach the log (a bad entry would poison every load).
        match ydoc::contains_update(&doc.ydoc, &update) {
            Ok(true) => return Ok(Some(guard)),
            Ok(false) => {}
            Err(e) => {
                eprintln!("closing connection {} (while handling {}) because of exception {e}", conn.socket_id, doc.name);
                return Err(CloseEvent::reset_connection());
            }
        }
        let actor = match conn.user.actor() {
            Ok(a) => Some(a),
            Err(e) => {
                eprintln!("[beforeHandleMessage] {e}");
                return Err(CloseEvent::reset_connection());
            }
        };
        if let Err(e) = persistence::log_update(&self.pool, &doc.design_id, &update, actor).await {
            // Like node-postgres' `error.message`: the server's message for database errors.
            let msg = e.as_database_error().map_or_else(|| e.to_string(), |d| d.message().to_string());
            eprintln!("[beforeHandleMessage] {msg}");
            return Err(CloseEvent::reset_connection());
        }
        Ok(Some(guard))
    }

    /// `Connection.handleMessage`: the hook, then `MessageReceiver.apply`. Any failure closes the connection.
    pub async fn handle_message(&self, conn: &Arc<Connection>, raw: Vec<u8>) {
        let guard = match self.before_handle(conn, &raw).await {
            Ok(g) => g,
            Err(ev) => return self.close_connection(conn, Some(ev)),
        };
        if conn.is_closed() {
            return;
        }
        let res = self.receive(conn, &raw);
        drop(guard);
        if let Err(e) = res {
            eprintln!("closing connection {} (while handling {}) because of exception {e}", conn.socket_id, conn.doc.name);
            self.close_connection(conn, Some(CloseEvent::reset_connection()));
        }
    }

    /// `MessageReceiver.apply` for a connection.
    fn receive(&self, conn: &Arc<Connection>, raw: &[u8]) -> Result<(), String> {
        use crate::protocol::{Decoder, msg, sync};
        let doc = &conn.doc;
        let mut d = Decoder::new(raw);
        let name = d.var_string().map_err(|e| e.to_string())?;
        if name != doc.name {
            return Ok(());
        }
        let ty = d.var_uint().map_err(|e| e.to_string())?;
        match ty {
            msg::SYNC | msg::SYNC_REPLY => {
                let step = d.var_uint().map_err(|e| e.to_string())?;
                match step {
                    sync::STEP1 => {
                        let sv = d.var_bytes().map_err(|e| e.to_string())?;
                        let reply = ydoc::diff(&doc.ydoc, sv)?;
                        // The server's SyncStep1 goes out first, then the SyncStep2 reply.
                        conn.send(protocol::sync_step1(&doc.name, &ydoc::state_vector(&doc.ydoc)));
                        conn.send(protocol::sync_step2(&doc.name, &reply));
                    }
                    sync::STEP2 => {
                        let update = d.var_bytes().map_err(|e| e.to_string())?;
                        if conn.read_only {
                            // Read-only: ack only when the update carries nothing new.
                            let contained = ydoc::contains_update(&doc.ydoc, update)?;
                            conn.send(protocol::sync_status(&doc.name, contained));
                        } else {
                            self.apply_update(doc, update, UpdateOrigin::Connection(conn.clone()));
                            conn.send(protocol::sync_status(&doc.name, true));
                        }
                    }
                    sync::UPDATE => {
                        if conn.read_only {
                            conn.send(protocol::sync_status(&doc.name, false));
                        } else {
                            let update = d.var_bytes().map_err(|e| e.to_string())?;
                            self.apply_update(doc, update, UpdateOrigin::Connection(conn.clone()));
                            conn.send(protocol::sync_status(&doc.name, true));
                        }
                    }
                    t => return Err(format!("Received a message with an unknown type: {t}")),
                }
            }
            msg::AWARENESS => {
                let update = d.var_bytes().map_err(|e| e.to_string())?;
                doc.apply_awareness(conn, update)?;
            }
            msg::QUERY_AWARENESS => {
                // Hocuspocus answers only through a `reply` callback, which connections don't pass.
            }
            msg::STATELESS => {
                // No onStateless hook is configured.
                d.var_string().map_err(|e| e.to_string())?;
            }
            msg::BROADCAST_STATELESS => {
                let payload = d.var_string().map_err(|e| e.to_string())?;
                let m = protocol::stateless(&doc.name, &payload);
                for c in doc.connections() {
                    c.send(m.clone());
                }
            }
            msg::CLOSE => self.close_connection(conn, Some(CloseEvent::new(1000, "provider_initiated"))),
            msg::AUTH => {
                let sub = d.var_uint().map_err(|e| e.to_string())?;
                if sub == protocol::auth::TOKEN {
                    // onTokenSync: no hook configured.
                    d.var_string().map_err(|e| e.to_string())?;
                } else {
                    eprintln!(
                        "Received an authentication message on a connection that is already fully authenticated. Probably your provider has been destroyed + recreated really fast."
                    );
                }
            }
            t => eprintln!("Unable to handle message of type {t}: no handler defined! Are your provider/server versions aligned?"),
        }
        Ok(())
    }

    // ------------------------------------------------------------------ server-side signals

    /// Live revocation (`oc_revoke`): record watermarks and close the affected connections with 4403.
    pub async fn revoke(&self, scope: &serde_json::Value) -> Result<usize, String> {
        let s = |k: &str| scope.get(k).and_then(serde_json::Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned);
        let (design_id, workspace_id, user_id) = (s("designId"), s("workspaceId"), s("userId"));
        // The API's clock: tickets it issued after sending this signal must still pass.
        let now = scope.get("at").and_then(serde_json::Value::as_f64).map_or_else(oc_js::now_ms, |v| v as i64);
        let mut designs: Option<Vec<String>> = design_id.map(|d| vec![d]);
        if let Some(ws) = workspace_id {
            let ws = Uuid::parse_str(&ws).map_err(|e| e.to_string())?;
            let rows: Vec<(String,)> = sqlx::query_as("SELECT id FROM designs WHERE workspace_id = $1").bind(ws).fetch_all(&self.pool).await.map_err(|e| e.to_string())?;
            designs = Some(rows.into_iter().map(|r| r.0).collect());
        }
        {
            let mut r = lock(&self.revoked);
            match (&designs, &user_id) {
                (Some(ds), Some(u)) => ds.iter().for_each(|d| {
                    r.pair.insert(format!("{d}:{u}"), now);
                }),
                (Some(ds), None) => ds.iter().for_each(|d| {
                    r.design.insert(d.clone(), now);
                }),
                (None, Some(u)) => {
                    r.user.insert(u.clone(), now);
                }
                (None, None) => {}
            }
        }
        let set: Option<HashSet<String>> = designs.map(|ds| ds.iter().map(|d| format!("design:{d}")).collect());
        let mut closed = 0;
        for doc in self.documents() {
            if set.as_ref().is_some_and(|s| !s.contains(&doc.name)) {
                continue;
            }
            for c in doc.connections() {
                if user_id.as_ref().is_some_and(|u| &c.user.id != u) {
                    continue;
                }
                self.close_socket_access_changed(&c);
                closed += 1;
            }
        }
        if closed > 0 {
            println!("[collab] revoked {closed} connection(s) {scope}");
        }
        Ok(closed)
    }

    /// 4403 'access changed' on the WebSocket itself: clients refetch a ticket (their role is checked again); if access
    /// is gone they show "You no longer have access". A per-document CLOSE message alone would leave the provider
    /// connected without ever re-authenticating. The document connection is unregistered at once.
    fn close_socket_access_changed(&self, c: &Arc<Connection>) {
        let _ = c.out.send(Out::Close(4403, "access changed".into()));
        self.close_connection(c, Some(CloseEvent::new(4403, "access changed")));
    }

    /// The LISTEN connection is back after an outage: notifications sent meanwhile are lost. Every client's WebSocket
    /// is closed (4403), so the provider reconnects with a new ticket (covering a missed revocation) and syncs again,
    /// and the next store folds in a missed server update from the op log. Tickets issued before now (less the clock
    /// skew allowance) are refused, so a replayed old one can't skip that check.
    pub fn on_listener_reconnect(&self) {
        self.reconnected_at.store(oc_js::now_ms() - RECONNECT_SKEW_MS, Ordering::SeqCst);
        let mut closed = 0;
        for doc in self.documents() {
            for c in doc.connections() {
                self.close_socket_access_changed(&c);
                closed += 1;
            }
        }
        println!("[collab] LISTEN reconnected: closed {closed} connection(s)");
    }

    /// Drop watermarks older than twice the ticket TTL (every minute).
    pub fn prune_revocations(&self, now: i64) {
        let cutoff = now - TICKET_TTL_S * 1000 * 2;
        let mut g = lock(&self.revoked);
        let r = &mut *g;
        for m in [&mut r.design, &mut r.user, &mut r.pair] {
            m.retain(|_, t| *t >= cutoff);
        }
    }

    /// `oc_doc_update`: merge a server-side change (brand template update, A8) into the live document.
    pub async fn merge_server_update(&self, p: &serde_json::Value) -> Result<(), String> {
        let design_id = p.get("designId").and_then(serde_json::Value::as_str).filter(|s| !s.is_empty());
        let seq = match p.get("seq") {
            Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
            Some(serde_json::Value::Number(n)) if n.as_f64() != Some(0.0) => Some(n.to_string()),
            _ => None,
        };
        let (Some(design_id), Some(seq)) = (design_id, seq) else { return Ok(()) };
        let Some(doc) = self.document(&format!("design:{design_id}")) else { return Ok(()) };
        // Small updates travel in the notification (a store may already have folded the log row away).
        let bytes = match p.get("update").and_then(serde_json::Value::as_str).filter(|s| !s.is_empty()) {
            Some(b64) => Some(node_base64(b64)),
            None => {
                let seq: i64 = seq.parse().map_err(|_| format!("invalid input syntax for type bigint: \"{seq}\""))?;
                sqlx::query_scalar::<_, Vec<u8>>("SELECT update FROM doc_updates WHERE design_id = $1 AND seq = $2")
                    .bind(design_id)
                    .bind(seq)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| e.to_string())?
            }
        };
        if let Some(b) = bytes {
            ydoc::decode_update(&b)?;
            self.apply_update(&doc, &b, UpdateOrigin::Server);
        }
        Ok(())
    }

    /// Close every connection (`closeConnections()` on shutdown) — pending stores run as the last
    /// connection of each document goes away.
    pub fn close_all(&self) {
        let docs: Vec<Arc<Document>> = lock(&self.docs).values().cloned().collect();
        for d in docs {
            for c in d.connections() {
                self.close_connection(&c, Some(CloseEvent::reset_connection()));
            }
        }
    }

    /// Number of documents in memory (loaded or loading).
    pub fn resident(&self) -> usize {
        lock(&self.docs).len()
    }

    /// Awareness timeout sweep over every document.
    pub fn expire_awareness(&self) {
        let now = oc_js::now_ms();
        for d in self.documents() {
            d.expire_awareness(now);
        }
    }
}

pub use oc_js::node_base64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_names() {
        assert_eq!(design_id_of("design:abcdefgh12345678"), Some("abcdefgh12345678"));
        assert_eq!(design_id_of("design:abcdefgh1234567"), None);
        assert_eq!(design_id_of("design:abcdefgh1234567-"), None);
        assert_eq!(design_id_of("doc:abcdefgh12345678"), None);
    }

    #[test]
    fn lenient_base64() {
        assert_eq!(node_base64("aGVsbG8="), b"hello");
        assert_eq!(node_base64("aGVsbG8"), b"hello");
        assert_eq!(node_base64("aGVs\nbG8"), b"hello");
    }
}
