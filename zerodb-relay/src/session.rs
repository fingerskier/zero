//! RELAY 0.2.2 session: handshake → persist / sync / subscribe.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use thiserror::Error;
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::relay::{
    authenticate, negotiate_capabilities, retransmit, FrontierTip, HeldOp, ERR_AUTH_FAILED,
    MSG_AUTH, MSG_CHALLENGE, MSG_ERROR, MSG_HELLO, MSG_OPS, MSG_OP_ACK, MSG_SUBSCRIBE,
    MSG_SUBSCRIBED, MSG_SYNC_REQUEST, MSG_SYNC_RESPONSE, MSG_WELCOME, RELAY_CAPS,
};

use crate::store::{validated_root_hex, OpStore, StoredOp};

#[derive(Debug, Error)]
pub enum RelayError {
    #[error("{0}")]
    Protocol(String),
    #[error("store: {0}")]
    Store(#[from] crate::store::StoreError),
    #[error("lock poisoned")]
    Poison,
}

const MAX_BATCH_OPS: usize = 64;
const MAX_BATCH_BYTES: usize = 16_777_216;

pub struct Inner {
    pub store: Box<dyn OpStore>,
    pub next_nonce: Option<[u8; 32]>,
    next_session: u64,
    subscribers: HashMap<String, HashSet<u64>>,
}

pub struct Relay {
    inner: Arc<Mutex<Inner>>,
}

impl Relay {
    pub fn memory() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                store: Box::new(crate::store::MemoryStore::new()),
                next_nonce: None,
                next_session: 0,
                subscribers: HashMap::new(),
            })),
        }
    }

    pub fn open(path: &std::path::Path) -> Result<Self, RelayError> {
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                store: Box::new(crate::store::SqliteStore::open(path)?),
                next_nonce: None,
                next_session: 0,
                subscribers: HashMap::new(),
            })),
        })
    }

    pub fn set_next_nonce(&self, nonce: [u8; 32]) {
        if let Ok(mut g) = self.inner.lock() {
            g.next_nonce = Some(nonce);
        }
    }

    pub fn accept(&self) -> RelaySession {
        let (nonce, session_id) = match self.inner.lock() {
            Ok(mut g) => {
                g.next_session = g.next_session.wrapping_add(1);
                let nonce = g.next_nonce.take().unwrap_or_else(random_nonce);
                (nonce, g.next_session)
            }
            Err(_) => (random_nonce(), 0),
        };
        RelaySession {
            inner: self.inner.clone(),
            phase: Phase::New,
            nonce,
            session_id,
            subscriptions: HashSet::new(),
        }
    }

    pub fn op_count(&self, ds: &str) -> Result<u64, RelayError> {
        self.inner
            .lock()
            .map_err(|_| RelayError::Poison)?
            .store
            .count(ds)
            .map_err(Into::into)
    }
}

fn random_nonce() -> [u8; 32] {
    let mut n = [0u8; 32];
    let _ = getrandom::getrandom(&mut n);
    n
}

enum Phase {
    New,
    Hello {
        claimed: [u8; 32],
        pk: [u8; 32],
        hello_caps: Vec<String>,
    },
    Authed,
    Closed,
}

pub struct RelaySession {
    inner: Arc<Mutex<Inner>>,
    phase: Phase,
    nonce: [u8; 32],
    session_id: u64,
    subscriptions: HashSet<String>,
}

impl RelaySession {
    pub fn is_authed(&self) -> bool {
        matches!(self.phase, Phase::Authed)
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.phase, Phase::Closed)
    }

    fn close(&mut self) {
        self.unregister();
        self.phase = Phase::Closed;
    }

    fn unregister(&mut self) {
        if self.subscriptions.is_empty() {
            return;
        }
        if let Ok(mut g) = self.inner.lock() {
            for ds in self.subscriptions.drain() {
                if let Some(set) = g.subscribers.get_mut(&ds) {
                    set.remove(&self.session_id);
                    if set.is_empty() {
                        g.subscribers.remove(&ds);
                    }
                }
            }
        }
    }

    pub fn handle(&mut self, frame: &[u8]) -> Result<Vec<Vec<u8>>, RelayError> {
        let env = decode_env(frame)?;
        match env.ty {
            MSG_HELLO => self.on_hello(&env),
            MSG_AUTH => self.on_auth(&env),
            MSG_OPS => self.require_auth(&env, |s, e| s.on_ops(e)),
            MSG_SYNC_REQUEST => self.require_auth(&env, |s, e| s.on_sync(e)),
            MSG_SUBSCRIBE => self.require_auth(&env, |s, e| s.on_subscribe(e)),
            _ => Ok(vec![error_frame(
                env.request_id,
                0x400,
                "UNSUPPORTED",
                false,
            )]),
        }
    }

    fn require_auth(
        &mut self,
        env: &Envelope,
        f: impl FnOnce(&mut Self, &Envelope) -> Result<Vec<Vec<u8>>, RelayError>,
    ) -> Result<Vec<Vec<u8>>, RelayError> {
        if !self.is_authed() {
            self.close();
            return Ok(vec![error_frame(
                env.request_id,
                0x201,
                "AUTH_FAILED",
                true,
            )]);
        }
        f(self, env)
    }

    fn on_hello(&mut self, env: &Envelope) -> Result<Vec<Vec<u8>>, RelayError> {
        let version = match map_get(&env.payload, "protocol_version") {
            Cbor::Uint(n) => *n,
            _ => 0,
        };
        if version != 1 {
            self.close();
            return Ok(vec![error_frame(
                env.request_id,
                0x102,
                "VERSION_MISMATCH",
                true,
            )]);
        }
        let claimed = take32(map_get(&env.payload, "peer_id"))?;
        let pk = take32(map_get(&env.payload, "public_key"))?;
        let hello_caps = text_array(map_get(&env.payload, "capabilities"));
        self.phase = Phase::Hello {
            claimed,
            pk,
            hello_caps,
        };
        Ok(vec![encode_env(
            MSG_CHALLENGE,
            env.request_id,
            Cbor::Map(vec![("nonce".into(), Cbor::Bytes(self.nonce.to_vec()))]),
        )])
    }

    fn on_auth(&mut self, env: &Envelope) -> Result<Vec<Vec<u8>>, RelayError> {
        let Phase::Hello {
            claimed,
            pk,
            hello_caps,
        } = &self.phase
        else {
            self.close();
            return Ok(vec![error_frame(
                env.request_id,
                ERR_AUTH_FAILED,
                "AUTH_FAILED",
                true,
            )]);
        };
        let claimed = *claimed;
        let pk = *pk;
        let hello_caps = hello_caps.clone();
        let request_id = env.request_id;
        let sig = take64(map_get(&env.payload, "signature"))?;
        if authenticate(&claimed, &pk, &self.nonce, &sig).is_err() {
            self.close();
            return Ok(vec![error_frame(
                request_id,
                ERR_AUTH_FAILED,
                "AUTH_FAILED",
                true,
            )]);
        }
        let offered: Vec<&str> = hello_caps.iter().map(|s| s.as_str()).collect();
        let caps = negotiate_capabilities(&offered, RELAY_CAPS);
        self.phase = Phase::Authed;
        Ok(vec![encode_env(
            MSG_WELCOME,
            request_id,
            Cbor::Map(vec![
                ("protocol_version".into(), Cbor::Uint(1)),
                ("relay_level".into(), Cbor::Uint(2)),
                (
                    "capabilities".into(),
                    Cbor::Array(caps.iter().map(|c| Cbor::Text((*c).into())).collect()),
                ),
                ("limits".into(), default_limits()),
            ]),
        )])
    }

    fn on_ops(&mut self, env: &Envelope) -> Result<Vec<Vec<u8>>, RelayError> {
        let ds = text(map_get(&env.payload, "datastore"))?;
        let operations = match map_get(&env.payload, "operations") {
            Cbor::Array(a) => a,
            _ => {
                return Ok(vec![error_frame(env.request_id, 0x400, "BAD_OPS", false)]);
            }
        };
        let mut outcomes = Vec::new();
        let mut guard = self.inner.lock().map_err(|_| RelayError::Poison)?;
        for op in operations {
            let (outcome, reason, parsed) = parse_stored(op);
            if outcome == "REJECT" {
                let mut m = vec![
                    ("op_id".into(), op_id_cbor(op)),
                    ("outcome".into(), Cbor::Text("REJECT".into())),
                ];
                if let Some(r) = reason {
                    m.push(("reason".into(), Cbor::Text(r.into())));
                }
                outcomes.push(Cbor::Map(m));
                continue;
            }
            let parsed = parsed.expect("parsed");
            let id = parsed.op_id;
            let inserted = guard.store.insert(&ds, parsed)?;
            let tag = if inserted { "ACCEPT" } else { "DUPLICATE" };
            outcomes.push(Cbor::Map(vec![
                ("op_id".into(), Cbor::Bytes(id.to_vec())),
                ("outcome".into(), Cbor::Text(tag.into())),
            ]));
        }
        drop(guard);
        Ok(vec![encode_env(
            MSG_OP_ACK,
            env.request_id,
            Cbor::Map(vec![("outcomes".into(), Cbor::Array(outcomes))]),
        )])
    }

    fn on_sync(&mut self, env: &Envelope) -> Result<Vec<Vec<u8>>, RelayError> {
        let ds = text(map_get(&env.payload, "datastore"))?;
        let cursor = map_get(&env.payload, "cursor");
        let frontier = parse_frontier(cursor);
        let guard = self.inner.lock().map_err(|_| RelayError::Poison)?;
        let stored = guard.store.list(&ds)?;
        let root = hex::decode(validated_root_hex(&stored)).unwrap_or(vec![0; 32]);
        drop(guard);
        let mut out = vec![encode_env(
            MSG_SYNC_RESPONSE,
            env.request_id,
            Cbor::Map(vec![
                ("datastore".into(), Cbor::Text(ds.clone())),
                ("validated_root".into(), Cbor::Bytes(root)),
            ]),
        )];
        let held: Vec<HeldOp> = stored.iter().map(StoredOp::held).collect();
        let want: HashSet<String> = retransmit(&held, &frontier, &[]).into_iter().collect();
        let send: Vec<StoredOp> = stored
            .into_iter()
            .filter(|o| want.contains(&hex::encode(o.op_id)))
            .collect();
        let operations: Vec<Cbor> = send
            .into_iter()
            .map(|o| {
                cbor::decode(&o.body).unwrap_or_else(|_| {
                    Cbor::Map(vec![
                        ("op_id".into(), Cbor::Bytes(o.op_id.to_vec())),
                        ("author".into(), Cbor::Bytes(o.author.to_vec())),
                        ("physical_ms".into(), Cbor::Uint(o.physical_ms)),
                        ("logical".into(), Cbor::Uint(o.logical as u64)),
                    ])
                })
            })
            .collect();
        out.extend(chunk_ops_frames(&ds, operations));
        Ok(out)
    }

    fn on_subscribe(&mut self, env: &Envelope) -> Result<Vec<Vec<u8>>, RelayError> {
        let list = match map_get(&env.payload, "datastores") {
            Cbor::Array(a) => a,
            _ => {
                return Ok(vec![error_frame(
                    env.request_id,
                    0x400,
                    "BAD_SUBSCRIBE",
                    false,
                )]);
            }
        };
        let mut entries = Vec::new();
        let mut guard = self.inner.lock().map_err(|_| RelayError::Poison)?;
        for item in list {
            let ds = text(item)?;
            self.subscriptions.insert(ds.clone());
            guard
                .subscribers
                .entry(ds.clone())
                .or_default()
                .insert(self.session_id);
            let peer_count = guard
                .subscribers
                .get(&ds)
                .map(|s| s.len() as u64)
                .unwrap_or(1);
            let stored = guard.store.list(&ds)?;
            let root = hex::decode(validated_root_hex(&stored)).unwrap_or(vec![0; 32]);
            entries.push(Cbor::Map(vec![
                ("id".into(), Cbor::Text(ds)),
                ("peer_count".into(), Cbor::Uint(peer_count)),
                ("validated_root".into(), Cbor::Bytes(root)),
            ]));
        }
        Ok(vec![encode_env(
            MSG_SUBSCRIBED,
            env.request_id,
            Cbor::Map(vec![("datastores".into(), Cbor::Array(entries))]),
        )])
    }
}

impl Drop for RelaySession {
    fn drop(&mut self) {
        self.unregister();
    }
}

struct Envelope {
    ty: u8,
    request_id: u32,
    payload: Cbor,
}

fn decode_env(bytes: &[u8]) -> Result<Envelope, RelayError> {
    let c = cbor::decode(bytes).map_err(|e| RelayError::Protocol(e.to_string()))?;
    Ok(Envelope {
        ty: uint(map_get(&c, "type"))? as u8,
        request_id: uint(map_get(&c, "request_id"))? as u32,
        payload: map_get(&c, "payload").clone(),
    })
}

fn encode_env(ty: u8, request_id: u32, payload: Cbor) -> Vec<u8> {
    cbor::encode(&Cbor::Map(vec![
        ("type".into(), Cbor::Uint(ty as u64)),
        ("request_id".into(), Cbor::Uint(request_id as u64)),
        ("payload".into(), payload),
    ]))
    .expect("encode")
}

fn error_frame(request_id: u32, code: u16, message: &str, fatal: bool) -> Vec<u8> {
    encode_env(
        MSG_ERROR,
        request_id,
        Cbor::Map(vec![
            ("code".into(), Cbor::Uint(code as u64)),
            ("message".into(), Cbor::Text(message.into())),
            ("fatal".into(), Cbor::Bool(fatal)),
        ]),
    )
}

fn ops_forward_frame(ds: &str, operations: Vec<Cbor>) -> Vec<u8> {
    encode_env(
        MSG_OPS,
        0,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(ds.into())),
            ("operations".into(), Cbor::Array(operations)),
        ]),
    )
}

fn chunk_ops_frames(ds: &str, operations: Vec<Cbor>) -> Vec<Vec<u8>> {
    if operations.is_empty() {
        return Vec::new();
    }
    let mut frames = Vec::new();
    let mut chunk: Vec<Cbor> = Vec::new();
    for op in operations {
        if !chunk.is_empty() {
            let over_count = chunk.len() >= MAX_BATCH_OPS;
            let mut trial = chunk.clone();
            trial.push(op.clone());
            let over_bytes = ops_forward_frame(ds, trial).len() > MAX_BATCH_BYTES;
            if over_count || over_bytes {
                frames.push(ops_forward_frame(ds, std::mem::take(&mut chunk)));
            }
        }
        chunk.push(op);
    }
    if !chunk.is_empty() {
        frames.push(ops_forward_frame(ds, chunk));
    }
    frames
}

fn default_limits() -> Cbor {
    Cbor::Map(vec![
        ("max_payload_bytes".into(), Cbor::Uint(1_048_576)),
        ("max_batch_ops".into(), Cbor::Uint(MAX_BATCH_OPS as u64)),
        ("max_batch_bytes".into(), Cbor::Uint(MAX_BATCH_BYTES as u64)),
        ("max_subscriptions".into(), Cbor::Uint(64)),
        ("ops_per_second".into(), Cbor::Uint(100)),
        ("bytes_per_second".into(), Cbor::Uint(10_485_760)),
    ])
}

fn map_get<'a>(c: &'a Cbor, k: &str) -> &'a Cbor {
    static NULL: Cbor = Cbor::Null;
    match c {
        Cbor::Map(ents) => ents
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v)
            .unwrap_or(&NULL),
        _ => &NULL,
    }
}

fn uint(c: &Cbor) -> Result<u64, RelayError> {
    match c {
        Cbor::Uint(n) => Ok(*n),
        _ => Err(RelayError::Protocol("uint".into())),
    }
}

fn text(c: &Cbor) -> Result<String, RelayError> {
    match c {
        Cbor::Text(s) => Ok(s.clone()),
        _ => Err(RelayError::Protocol("text".into())),
    }
}

fn take32(c: &Cbor) -> Result<[u8; 32], RelayError> {
    match c {
        Cbor::Bytes(b) if b.len() == 32 => Ok(b.as_slice().try_into().unwrap()),
        _ => Err(RelayError::Protocol("b32".into())),
    }
}

fn take64(c: &Cbor) -> Result<[u8; 64], RelayError> {
    match c {
        Cbor::Bytes(b) if b.len() == 64 => Ok(b.as_slice().try_into().unwrap()),
        _ => Err(RelayError::Protocol("b64".into())),
    }
}

fn text_array(c: &Cbor) -> Vec<String> {
    match c {
        Cbor::Array(a) => a
            .iter()
            .filter_map(|v| match v {
                Cbor::Text(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_stored(op: &Cbor) -> (&'static str, Option<&'static str>, Option<StoredOp>) {
    let Cbor::Map(_) = op else {
        return ("REJECT", Some("DECODE"), None);
    };
    let id = match take32(map_get(op, "op_id")) {
        Ok(id) => id,
        Err(_) => return ("REJECT", Some("DECODE"), None),
    };
    let author = match take32(map_get(op, "author")) {
        Ok(a) => a,
        Err(_) => return ("REJECT", Some("DECODE"), None),
    };
    let physical_ms = uint(map_get(op, "physical_ms")).unwrap_or_default();
    let logical = uint(map_get(op, "logical")).unwrap_or_default() as u16;
    let body = cbor::encode(op).unwrap_or_default();
    (
        "ACCEPT",
        None,
        Some(StoredOp {
            op_id: id,
            author,
            physical_ms,
            logical,
            body,
        }),
    )
}

fn op_id_cbor(op: &Cbor) -> Cbor {
    match take32(map_get(op, "op_id")) {
        Ok(id) => Cbor::Bytes(id.to_vec()),
        Err(_) => Cbor::Bytes(vec![0; 32]),
    }
}

fn parse_frontier(cursor: &Cbor) -> Vec<FrontierTip> {
    let front = map_get(cursor, "frontier");
    let Cbor::Map(ents) = front else {
        return Vec::new();
    };
    ents.iter()
        .filter_map(|(author, tip)| {
            let op_id = match take32(map_get(tip, "op_id")) {
                Ok(id) => hex::encode(id),
                Err(_) => return None,
            };
            Some(FrontierTip {
                author: author.clone(),
                op_id,
                physical_ms: uint(map_get(tip, "physical_ms")).unwrap_or(0),
                logical: uint(map_get(tip, "logical")).unwrap_or(0) as u16,
            })
        })
        .collect()
}
