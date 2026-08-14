//! Experimental M2 Node/NAPI binding over `zerodb-storage` LocalStore.
//!
//! Not a format freeze. API mirrors the M1 local slice for open/mutate/inspect.

use std::collections::HashMap;
use std::io::{self, Read as IoRead, Write as IoWrite};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};
use zerodb_storage::{LocalStore, sync};

fn map_err(e: impl ToString) -> Error {
    Error::from_reason(e.to_string())
}

/// Per-session socket timeout: a slow or stalled peer surfaces as a sync
/// error instead of hanging the session thread forever.
const SYNC_TIMEOUT: Duration = Duration::from_secs(30);

fn set_stream_timeouts(stream: &TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(SYNC_TIMEOUT))?;
    stream.set_write_timeout(Some(SYNC_TIMEOUT))
}

/// Connect a client WebSocket with read/write timeouts on the underlying
/// TCP stream so a stalled server cannot hang the session.
fn connect_ws(url: &str) -> std::result::Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let (ws, _resp) = tungstenite::connect(url).map_err(|e| e.to_string())?;
    if let MaybeTlsStream::Plain(stream) = ws.get_ref() {
        set_stream_timeouts(stream).map_err(|e| e.to_string())?;
    }
    Ok(ws)
}

type Subscriber = ThreadsafeFunction<serde_json::Value, (), serde_json::Value, Status, false, true>;

type SharedStore = Arc<Mutex<Option<LocalStore>>>;
type SharedSubs = Arc<Mutex<HashMap<u32, Subscriber>>>;

/// Bridge a WebSocket to `Read + Write` for the sync protocol. Incoming
/// Binary messages are buffered for `read`; `write` sends one Binary message
/// per call. The sync framing is length-prefixed, so message boundaries need
/// not align with protocol frames.
struct WsIo<S: IoRead + IoWrite> {
    ws: WebSocket<S>,
    buf: Vec<u8>,
    pos: usize,
}

impl<S: IoRead + IoWrite> WsIo<S> {
    fn new(ws: WebSocket<S>) -> Self {
        Self {
            ws,
            buf: Vec::new(),
            pos: 0,
        }
    }
}

impl<S: IoRead + IoWrite> IoRead for WsIo<S> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            match self.ws.read() {
                Ok(Message::Binary(data)) => {
                    self.buf = data;
                    self.pos = 0;
                }
                Ok(Message::Close(_)) => return Ok(0),
                Ok(_) => continue, // ignore text/ping/pong
                Err(tungstenite::Error::ConnectionClosed) => return Ok(0),
                // Preserve WouldBlock/TimedOut: the push loop treats them as
                // "no frame yet" (tungstenite keeps partial input buffered).
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(e);
                }
                Err(e) => return Err(io::Error::other(e)),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl<S: IoRead + IoWrite> IoWrite for WsIo<S> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.ws
            .send(Message::Binary(data.to_vec()))
            .map_err(io::Error::other)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.ws.flush().map_err(io::Error::other)
    }
}

struct ServerHandle {
    stop: Arc<AtomicBool>,
    join: JoinHandle<()>,
}

struct ConnHandle {
    stop: Arc<AtomicBool>,
    join: JoinHandle<()>,
}

/// Sleep in small slices so stop/dirty flags stay responsive.
fn wait_until(deadline: Instant, stop: &AtomicBool, dirty: Option<&AtomicBool>) {
    while !stop.load(Ordering::SeqCst)
        && !dirty.is_some_and(|d| d.load(Ordering::SeqCst))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Read timeout during a persistent push session: short, because the push
/// loop treats a timed-out read as "no frame yet" and uses it as its pacing.
const PUSH_POLL: Duration = Duration::from_millis(150);

/// One client session that requests the v2 push capability. If the server
/// acks, this blocks inside the persistent push loop (store locked only per
/// exchange) until the peer closes, the store closes, or `stop` is set.
fn push_session(
    inner: &SharedStore,
    url: &str,
    subs: &SharedSubs,
    gen_ctr: &Arc<AtomicU64>,
    stop: &Arc<AtomicBool>,
) -> std::result::Result<(sync::PullSummary, bool), String> {
    let ws = connect_ws(url)?;
    let mut io = WsIo::new(ws);
    sync::pull_push(
        inner,
        &mut io,
        &|| gen_ctr.load(Ordering::SeqCst),
        &|| stop.load(Ordering::SeqCst),
        |summary, push| {
            emit_to(
                subs,
                serde_json::json!({
                    "kind": "sync",
                    "role": "connect",
                    "url": url,
                    "accepted": summary.accepted,
                    "skipped": summary.skipped,
                    "sent": summary.sent,
                    "remoteAccepted": summary.remote_accepted,
                    "remoteSkipped": summary.remote_skipped,
                    "push": push,
                }),
            )
        },
        // Shorten the read timeout for the persistent phase on the socket
        // handle tungstenite actually reads from (on Windows SO_RCVTIMEO is
        // per handle — a try_clone'd handle would not affect this one).
        |io| {
            if let MaybeTlsStream::Plain(s) = io.ws.get_ref() {
                let _ = s.set_read_timeout(Some(PUSH_POLL));
            }
        },
        |tick| {
            emit_to(
                subs,
                serde_json::json!({
                    "kind": "sync",
                    "role": "connect-push",
                    "url": url,
                    "sent": tick.sent,
                    "accepted": tick.accepted,
                    "skipped": tick.skipped,
                }),
            )
        },
    )
    .map_err(|e| e.to_string())
}

fn emit_to(subs: &SharedSubs, event: serde_json::Value) {
    if let Ok(subs) = subs.lock() {
        for tsfn in subs.values() {
            tsfn.call(event.clone(), ThreadsafeFunctionCallMode::NonBlocking);
        }
    }
}

/// One accepted connection: WS handshake, then a sync serve session. The
/// store lock is taken only per exchange (never while accepting, during the
/// handshake, or across the persistent push loop's waits, so one push peer
/// cannot starve other sessions or local writes). Socket read/write timeouts
/// (30s) bound the handshake and initial exchange; if the client negotiated
/// the v2 push capability the session stays open with a short read timeout
/// (`PUSH_POLL`) as the wake/poll pacing, pushing new local ops whenever the
/// mutation generation counter changes. Errors surface as
/// `{kind:'sync', error}` events.
fn serve_connection(
    stream: TcpStream,
    inner: &SharedStore,
    subs: &SharedSubs,
    gen_ctr: &Arc<AtomicU64>,
    stop: &Arc<AtomicBool>,
    allow_push: bool,
) {
    let peer_addr = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let _ = stream.set_nonblocking(false);
    if set_stream_timeouts(&stream).is_err() {
        return;
    }
    let Ok(ws) = tungstenite::accept(stream) else {
        return;
    };
    let mut io = WsIo::new(ws);
    match sync::serve_push(
        inner,
        &mut io,
        allow_push,
        &|| gen_ctr.load(Ordering::SeqCst),
        &|| stop.load(Ordering::SeqCst),
        |summary, push| {
            emit_to(
                subs,
                serde_json::json!({
                    "kind": "sync",
                    "role": "serve",
                    "peer": summary.peer,
                    "peerAddr": peer_addr.clone(),
                    "sent": summary.sent,
                    "accepted": summary.accepted,
                    "skipped": summary.skipped,
                    "push": push,
                }),
            )
        },
        // Per-handle read timeout (see push_session note) on entering the
        // persistent phase.
        |io| {
            let _ = io.ws.get_ref().set_read_timeout(Some(PUSH_POLL));
        },
        |tick| {
            emit_to(
                subs,
                serde_json::json!({
                    "kind": "sync",
                    "role": "serve-push",
                    "peerAddr": peer_addr.clone(),
                    "sent": tick.sent,
                    "accepted": tick.accepted,
                    "skipped": tick.skipped,
                }),
            )
        },
    ) {
        Ok(_) => {} // summary event already emitted after the exchange
        Err(e) => emit_to(
            subs,
            serde_json::json!({
                "kind": "sync",
                "role": "serve",
                "peerAddr": peer_addr,
                "error": e.to_string(),
            }),
        ),
    }
}

/// SQLite-backed ZeroDB handle for Node (M2 vertical).
#[napi]
pub struct Database {
    inner: SharedStore,
    subs: SharedSubs,
    next_sub: Mutex<u32>,
    server: Mutex<Option<ServerHandle>>,
    dirty: Arc<AtomicBool>,
    /// Mutation generation counter — the push-session wake signal. Bumped on
    /// every local op, import, and replay; push loops diff op ids when it moves.
    gen_ctr: Arc<AtomicU64>,
    conns: Mutex<HashMap<u32, ConnHandle>>,
    next_conn: Mutex<u32>,
}

impl Database {
    fn emit(&self, event: serde_json::Value) {
        emit_to(&self.subs, event);
    }

    fn stop_server(&self) -> Result<()> {
        let handle = self
            .server
            .lock()
            .map_err(|e| map_err(e.to_string()))?
            .take();
        if let Some(handle) = handle {
            handle.stop.store(true, Ordering::SeqCst);
            let _ = handle.join.join();
        }
        Ok(())
    }

    fn stop_conns(&self) -> Result<()> {
        let handles: Vec<ConnHandle> = {
            let mut conns = self.conns.lock().map_err(|e| map_err(e.to_string()))?;
            conns.drain().map(|(_, h)| h).collect()
        };
        for handle in handles {
            handle.stop.store(true, Ordering::SeqCst);
            let _ = handle.join.join();
        }
        Ok(())
    }

    fn emit_op(&self, method: &str, node: &str, key: Option<&str>, op_id: Option<&str>) {
        self.dirty.store(true, Ordering::SeqCst);
        self.gen_ctr.fetch_add(1, Ordering::SeqCst);
        self.emit(serde_json::json!({
            "kind": "op",
            "method": method,
            "node": node,
            "key": key,
            "opId": op_id,
        }));
    }

    fn with_store<R>(&self, f: impl FnOnce(&LocalStore) -> Result<R>) -> Result<R> {
        let guard = self.inner.lock().map_err(|e| map_err(e.to_string()))?;
        let store = guard
            .as_ref()
            .ok_or_else(|| Error::from_reason("database is closed"))?;
        f(store)
    }

    fn with_store_mut<R>(&self, f: impl FnOnce(&mut LocalStore) -> Result<R>) -> Result<R> {
        let mut guard = self.inner.lock().map_err(|e| map_err(e.to_string()))?;
        let store = guard
            .as_mut()
            .ok_or_else(|| Error::from_reason("database is closed"))?;
        f(store)
    }
}

#[napi]
impl Database {
    /// Create a new database file and open it.
    #[napi(factory)]
    pub fn init(path: String) -> Result<Self> {
        let store = LocalStore::init(Path::new(&path)).map_err(map_err)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Some(store))),
            subs: Arc::new(Mutex::new(HashMap::new())),
            next_sub: Mutex::new(0),
            server: Mutex::new(None),
            dirty: Arc::new(AtomicBool::new(false)),
            gen_ctr: Arc::new(AtomicU64::new(0)),
            conns: Mutex::new(HashMap::new()),
            next_conn: Mutex::new(0),
        })
    }

    /// Open an existing database file.
    #[napi(factory)]
    pub fn open(path: String) -> Result<Self> {
        let store = LocalStore::open(Path::new(&path)).map_err(map_err)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Some(store))),
            subs: Arc::new(Mutex::new(HashMap::new())),
            next_sub: Mutex::new(0),
            server: Mutex::new(None),
            dirty: Arc::new(AtomicBool::new(false)),
            gen_ctr: Arc::new(AtomicU64::new(0)),
            conns: Mutex::new(HashMap::new()),
            next_conn: Mutex::new(0),
        })
    }

    /// Release the SQLite connection (required before deleting the file on Windows).
    #[napi]
    pub fn close(&self) -> Result<()> {
        self.stop_server()?;
        self.stop_conns()?;
        let mut guard = self.inner.lock().map_err(|e| map_err(e.to_string()))?;
        *guard = None;
        if let Ok(mut subs) = self.subs.lock() {
            subs.clear();
        }
        Ok(())
    }

    /// Register a change callback. Returns a subscription id for `unsubscribe`.
    ///
    /// Events (JSON): `{kind:'op', method, node, key?, opId}` for local
    /// mutations, `{kind:'import', accepted, skipped}`, `{kind:'replay'}`.
    /// Delivery is asynchronous (next event-loop tick). Experimental surface;
    /// wire/event shapes are versioned-experimental and may change pre-freeze.
    #[napi]
    pub fn subscribe(&self, callback: Function<serde_json::Value, ()>) -> Result<u32> {
        let tsfn = callback
            .build_threadsafe_function()
            .weak::<true>()
            .build_callback(|ctx| Ok(ctx.value))?;
        let mut next = self.next_sub.lock().map_err(|e| map_err(e.to_string()))?;
        let id = *next;
        *next += 1;
        self.subs
            .lock()
            .map_err(|e| map_err(e.to_string()))?
            .insert(id, tsfn);
        Ok(id)
    }

    /// Remove a subscription; unknown ids are a no-op.
    #[napi]
    pub fn unsubscribe(&self, id: u32) -> Result<()> {
        self.subs
            .lock()
            .map_err(|e| map_err(e.to_string()))?
            .remove(&id);
        Ok(())
    }

    #[napi]
    pub fn datastore_id(&self) -> Result<String> {
        self.with_store(|store| Ok(store.datastore_id_hex()))
    }

    #[napi]
    pub fn peer_id(&self) -> Result<String> {
        self.with_store(|store| Ok(store.author_hex()))
    }

    #[napi]
    pub fn op_count(&self) -> Result<u32> {
        self.with_store(|store| Ok(store.op_count().map_err(map_err)? as u32))
    }

    #[napi]
    pub fn create_node(&self, label: String) -> Result<String> {
        let (id, op) =
            self.with_store_mut(|store| store.create_node_with_op(&label).map_err(map_err))?;
        self.emit_op("createNode", &id, None, Some(&op));
        Ok(id)
    }

    #[napi]
    pub fn delete_node(&self, node: String) -> Result<String> {
        let op = self.with_store_mut(|store| store.delete_node(&node).map_err(map_err))?;
        self.emit_op("deleteNode", &node, None, Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn set_lww(&self, node: String, key: String, value: String) -> Result<String> {
        let op =
            self.with_store_mut(|store| store.set_lww(&node, &key, &value).map_err(map_err))?;
        self.emit_op("setLww", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn get_lww(&self, node: String, key: String) -> Result<Option<String>> {
        self.with_store(|store| store.get_lww(&node, &key).map_err(map_err))
    }

    /// Get a materialized property as JSON (string, number, bool, array, or null).
    #[napi]
    pub fn get_prop(&self, node: String, key: String) -> Result<serde_json::Value> {
        self.with_store(
            |store| match store.get_prop(&node, &key).map_err(map_err)? {
                Some(v) => Ok(v),
                None => Ok(serde_json::Value::Null),
            },
        )
    }

    #[napi]
    pub fn gcounter_inc(&self, node: String, key: String, n: u32) -> Result<String> {
        let op = self
            .with_store_mut(|store| store.gcounter_inc(&node, &key, n as u64).map_err(map_err))?;
        self.emit_op("gcounterInc", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn counter_inc(&self, node: String, key: String, n: u32) -> Result<String> {
        let op =
            self.with_store_mut(|store| store.counter_inc(&node, &key, n as u64).map_err(map_err))?;
        self.emit_op("counterInc", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn counter_dec(&self, node: String, key: String, n: u32) -> Result<String> {
        let op =
            self.with_store_mut(|store| store.counter_dec(&node, &key, n as u64).map_err(map_err))?;
        self.emit_op("counterDec", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn set_add(&self, node: String, key: String, value: String) -> Result<String> {
        let op =
            self.with_store_mut(|store| store.set_add(&node, &key, &value).map_err(map_err))?;
        self.emit_op("setAdd", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn set_remove(&self, node: String, key: String, value: String) -> Result<String> {
        let op =
            self.with_store_mut(|store| store.set_remove(&node, &key, &value).map_err(map_err))?;
        self.emit_op("setRemove", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn flag_enable(&self, node: String, key: String) -> Result<String> {
        let op = self.with_store_mut(|store| store.flag_enable(&node, &key).map_err(map_err))?;
        self.emit_op("flagEnable", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn flag_disable(&self, node: String, key: String) -> Result<String> {
        let op = self.with_store_mut(|store| store.flag_disable(&node, &key).map_err(map_err))?;
        self.emit_op("flagDisable", &node, Some(&key), Some(&op));
        Ok(op)
    }

    /// List nodes as JSON array of `{ id, label, deleted, props }`.
    #[napi]
    pub fn list_nodes(&self) -> Result<serde_json::Value> {
        self.with_store(|store| {
            let nodes = store.list_nodes().map_err(map_err)?;
            let mut arr = Vec::new();
            for (id, label, deleted) in nodes {
                let props: serde_json::Map<String, serde_json::Value> = if deleted {
                    serde_json::Map::new()
                } else {
                    store
                        .list_props(&id)
                        .map_err(map_err)?
                        .into_iter()
                        .collect()
                };
                arr.push(serde_json::json!({
                    "id": id, "label": label, "deleted": deleted, "props": props
                }));
            }
            Ok(serde_json::Value::Array(arr))
        })
    }

    #[napi]
    pub fn create_edge(&self, label: String, src: String, dst: String) -> Result<String> {
        let id = self.with_store_mut(|store| {
            store.create_edge(&label, &src, &dst).map_err(map_err)
        })?;
        self.emit_op("createEdge", &id, None, Some(&id));
        Ok(id)
    }

    #[napi]
    pub fn delete_edge(&self, edge: String) -> Result<String> {
        let op = self.with_store_mut(|store| store.delete_edge(&edge).map_err(map_err))?;
        self.emit_op("deleteEdge", &edge, None, Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn list_edges(&self) -> Result<serde_json::Value> {
        self.with_store(|store| {
            let edges = store.list_edges_visible().map_err(map_err)?;
            let arr: Vec<serde_json::Value> = edges
                .into_iter()
                .map(|(id, label, src, dst)| {
                    serde_json::json!({ "id": id, "label": label, "src": src, "dst": dst })
                })
                .collect();
            Ok(serde_json::Value::Array(arr))
        })
    }

    /// Apply a pin or SCHEMA IR JSON. Returns `{ schemaId, epoch }`.
    #[napi]
    pub fn apply_schema(&self, schema_json: String) -> Result<serde_json::Value> {
        self.with_store_mut(|store| {
            store.apply_schema_json(&schema_json).map_err(map_err)?;
            Ok(serde_json::json!({
                "schemaId": store.schema_id_hex().map_err(map_err)?,
                "epoch": store.schema_epoch().map_err(map_err)?,
            }))
        })
    }

    #[napi]
    pub fn schema_id(&self) -> Result<Option<String>> {
        self.with_store(|store| store.schema_id_hex().map_err(map_err))
    }

    /// Run an O3 minimal query (`MATCH/WHERE/RETURN/ORDER BY/LIMIT`).
    /// Optional `params` bind `$name` placeholders.
    #[napi]
    pub fn query(
        &self,
        q: String,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        self.with_store(|store| {
            store
                .query_with(&q, params.as_ref().unwrap_or(&serde_json::json!({})))
                .map_err(map_err)
        })
    }

    /// Full inspect report as JSON (path is reported as the given string).
    #[napi]
    pub fn inspect(&self, path: String) -> Result<serde_json::Value> {
        self.with_store(|store| {
            let report = store.inspect(Path::new(&path)).map_err(map_err)?;
            serde_json::to_value(report).map_err(|e| map_err(e.to_string()))
        })
    }

    #[napi]
    pub fn replay(&self) -> Result<()> {
        self.with_store_mut(|store| store.replay_all().map_err(map_err))?;
        self.gen_ctr.fetch_add(1, Ordering::SeqCst);
        self.emit(serde_json::json!({ "kind": "replay" }));
        Ok(())
    }

    /// Export all ops as a JSON bundle string (format 1).
    #[napi]
    pub fn export_json(&self) -> Result<String> {
        self.with_store(|store| {
            let bundle = store.export_all().map_err(map_err)?;
            serde_json::to_string(&bundle).map_err(|e| map_err(e.to_string()))
        })
    }

    /// Import a JSON export bundle. Returns `{ accepted, skipped }`.
    #[napi]
    pub fn import_json(&self, json: String) -> Result<serde_json::Value> {
        self.with_store_mut(|store| {
            let bundle: zerodb_storage::ExportBundle =
                serde_json::from_str(&json).map_err(|e| map_err(e.to_string()))?;
            let (accepted, skipped) = store.import_bundle(&bundle).map_err(map_err)?;
            Ok(serde_json::json!({ "accepted": accepted, "skipped": skipped }))
        })
        .inspect(|result| {
            self.gen_ctr.fetch_add(1, Ordering::SeqCst);
            self.emit(serde_json::json!({ "kind": "import", "accepted": result["accepted"], "skipped": result["skipped"] }))
        })
    }

    /// Start a WebSocket sync listener. By default binds loopback only
    /// (`ws://127.0.0.1:port`). Pass `allowInsecureLan: true` to bind
    /// `0.0.0.0` (all interfaces) — the wire is **plaintext and
    /// unauthenticated**; anyone on the network can read and write the
    /// store, so only enable this on trusted LANs (mirrors the CLI's
    /// `--allow-insecure-lan`). Port 0 asks the OS for a free port; the
    /// actual port is returned. Each incoming connection runs one serve
    /// session on its own thread (30s socket timeouts; a stalled peer
    /// never blocks the accept loop or the store) and emits a
    /// `{kind:'sync', role:'serve', ...}` event.
    /// `push` (default true) advertises the v2 push capability: clients that
    /// request it get a persistent session that streams new local ops as they
    /// land. Pass `push: false` to behave like an old one-shot-only server.
    #[napi]
    pub fn serve(
        &self,
        port: u32,
        allow_insecure_lan: Option<bool>,
        push: Option<bool>,
    ) -> Result<u32> {
        let mut server = self.server.lock().map_err(|e| map_err(e.to_string()))?;
        if server.is_some() {
            return Err(Error::from_reason("already serving"));
        }
        let host = if allow_insecure_lan.unwrap_or(false) {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };
        let listener = TcpListener::bind((host, port as u16)).map_err(map_err)?;
        let actual = listener.local_addr().map_err(map_err)?.port();
        listener.set_nonblocking(true).map_err(map_err)?;
        let allow_push = push.unwrap_or(true);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let inner = self.inner.clone();
        let subs = self.subs.clone();
        let gen_ctr = self.gen_ctr.clone();
        let join = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // Per-connection thread: a peer that stalls (e.g.
                        // never completes the WS handshake) must not block
                        // accepting further connections. The store mutex
                        // still serializes actual sync exchanges.
                        let inner = inner.clone();
                        let subs = subs.clone();
                        let gen_ctr = gen_ctr.clone();
                        let stop = stop_flag.clone();
                        std::thread::spawn(move || {
                            serve_connection(stream, &inner, &subs, &gen_ctr, &stop, allow_push)
                        });
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        });
        *server = Some(ServerHandle { stop, join });
        Ok(actual as u32)
    }

    /// Stop the sync listener started by `serve`; no-op if not serving.
    #[napi]
    pub fn stop_serve(&self) -> Result<()> {
        self.stop_server()
    }

    /// Connect to a peer's sync listener (`ws://host:port`) and run one
    /// two-way session. Returns `{ accepted, skipped, sent, remoteAccepted,
    /// remoteSkipped }` and emits a `{kind:'sync', role:'connect', ...}` event.
    #[napi]
    pub fn connect_peer(&self, url: String) -> Result<serde_json::Value> {
        let ws = connect_ws(&url).map_err(map_err)?;
        let mut io = WsIo::new(ws);
        let summary = self.with_store_mut(|store| sync::pull(store, &mut io).map_err(map_err))?;
        let result = serde_json::json!({
            "accepted": summary.accepted,
            "skipped": summary.skipped,
            "sent": summary.sent,
            "remoteAccepted": summary.remote_accepted,
            "remoteSkipped": summary.remote_skipped,
        });
        let mut event = result.clone();
        event["kind"] = "sync".into();
        event["role"] = "connect".into();
        event["url"] = url.into();
        self.emit(event);
        Ok(result)
    }

    /// Keep this store converged with a peer (`ws://host:port`) from a
    /// background thread: one session immediately, then re-sync whenever a
    /// local mutation lands (dirty flag) or `intervalMs` elapses (poll floor
    /// for remote changes). Failures emit `{kind:'sync-error', url, error,
    /// retryInMs}` and retry with capped exponential backoff (1s..30s).
    /// Returns a connection id for `disconnect`; `close()` stops all.
    ///
    /// Each attempt requests the v2 push capability. If the server acks,
    /// the session stays open: new ops stream both ways immediately (dirty
    /// flag and interval are then only the reconnect pacing). If the server
    /// is push-unaware (old peer), the one-shot session runs and the dirty
    /// flag + interval poll approximates live sync exactly as before.
    #[napi]
    pub fn auto_connect(&self, url: String, interval_ms: u32) -> Result<u32> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let inner = self.inner.clone();
        let subs = self.subs.clone();
        let dirty = self.dirty.clone();
        let gen_ctr = self.gen_ctr.clone();
        let interval = Duration::from_millis(interval_ms.max(20) as u64);
        let join = std::thread::spawn(move || {
            let mut backoff = Duration::from_secs(1);
            while !stop_flag.load(Ordering::SeqCst) {
                dirty.store(false, Ordering::SeqCst);
                match push_session(&inner, &url, &subs, &gen_ctr, &stop_flag) {
                    Ok((_summary, push)) => {
                        backoff = Duration::from_secs(1);
                        // Summary event already emitted after the exchange.
                        if push {
                            // Persistent session ended (peer closed or store
                            // closed); brief pause, then reconnect.
                            wait_until(
                                Instant::now() + interval.min(Duration::from_secs(1)),
                                &stop_flag,
                                None,
                            );
                        } else {
                            wait_until(Instant::now() + interval, &stop_flag, Some(&dirty));
                        }
                    }
                    Err(e) => {
                        emit_to(
                            &subs,
                            serde_json::json!({
                                "kind": "sync-error",
                                "url": url,
                                "error": e,
                                "retryInMs": backoff.as_millis() as u64,
                            }),
                        );
                        wait_until(Instant::now() + backoff, &stop_flag, None);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        });
        let mut next = self.next_conn.lock().map_err(|e| map_err(e.to_string()))?;
        let id = *next;
        *next += 1;
        self.conns
            .lock()
            .map_err(|e| map_err(e.to_string()))?
            .insert(id, ConnHandle { stop, join });
        Ok(id)
    }

    /// Stop an `autoConnect` background connection; unknown ids are a no-op.
    #[napi]
    pub fn disconnect(&self, id: u32) -> Result<()> {
        let handle = self
            .conns
            .lock()
            .map_err(|e| map_err(e.to_string()))?
            .remove(&id);
        if let Some(handle) = handle {
            handle.stop.store(true, Ordering::SeqCst);
            let _ = handle.join.join();
        }
        Ok(())
    }
}
