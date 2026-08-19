//! RELAY 0.2.2 session: handshake → persist / sync / subscribe.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use thiserror::Error;
use zerodb_core::auth::{
    AdmissionToken, DeviceCert, KIND_CAP_GRANT, KIND_CAP_REVOKE, KIND_GENESIS, KnownGrant,
    SCOPE_ADMIN, SCOPE_READ, SCOPE_SYNC, SCOPE_WRITE, verify_admission_token_at,
};
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::merkle::{BUCKET_WIDTH_MS, MERKLE_FORMAT_VERSION, MerkleTree};
use zerodb_core::relay::{
    ERR_AUTH_FAILED, FrontierTip, HeldOp, MSG_AUTH, MSG_CHALLENGE, MSG_DELTA_BATCH,
    MSG_DELTA_REQUEST, MSG_ERROR, MSG_HELLO, MSG_MERKLE_LEAF_REQUEST, MSG_MERKLE_LEAF_RESPONSE,
    MSG_MERKLE_NODE_REQUEST, MSG_MERKLE_NODE_RESPONSE, MSG_OP_ACK, MSG_OPS, MSG_SUBSCRIBE,
    MSG_SUBSCRIBED, MSG_SYNC_REQUEST, MSG_SYNC_RESPONSE, MSG_WELCOME, RELAY_CAPS,
    admit_experimental_op, authenticate, negotiate_capabilities, retransmit,
};

use crate::store::{OpStore, StoredOp, validated_root_hex};

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
    /// When true, skip membership filters (SUBSCRIBE + author write) so peers
    /// can prove AUTH.md §4 independently of the relay (EXEMPLAR E5).
    colluding: bool,
}

pub struct Relay {
    inner: Arc<Mutex<Inner>>,
}

impl Relay {
    pub fn memory() -> Self {
        Self::with_store(Box::new(crate::store::MemoryStore::new()), false)
    }

    /// Forwards signed ops and subscriptions without membership checks.
    pub fn memory_colluding() -> Self {
        Self::with_store(Box::new(crate::store::MemoryStore::new()), true)
    }

    fn with_store(store: Box<dyn OpStore>, colluding: bool) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                store,
                next_nonce: None,
                next_session: 0,
                subscribers: HashMap::new(),
                colluding,
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
                colluding: false,
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
            authorized: HashMap::new(),
            walk_snapshots: HashMap::new(),
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

    /// Persist one relay-side membership grant. The first grant protects the
    /// datastore: subsequent access requires a valid SUBSCRIBE token.
    pub fn upsert_grant(&self, grant: KnownGrant) -> Result<(), RelayError> {
        self.inner
            .lock()
            .map_err(|_| RelayError::Poison)?
            .store
            .upsert_grant(grant)?;
        Ok(())
    }

    pub fn revoke_grant(&self, ds: &str, grant: &[u8; 32]) -> Result<bool, RelayError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| RelayError::Poison)?
            .store
            .revoke_grant(ds, grant)?)
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
    Authed {
        caps: Vec<String>,
        peer_id: [u8; 32],
    },
    Closed,
}

pub struct RelaySession {
    inner: Arc<Mutex<Inner>>,
    phase: Phase,
    nonce: [u8; 32],
    session_id: u64,
    subscriptions: HashSet<String>,
    authorized: HashMap<String, [u8; 32]>,
    walk_snapshots: HashMap<String, Vec<StoredOp>>,
}

impl RelaySession {
    pub fn is_authed(&self) -> bool {
        matches!(self.phase, Phase::Authed { .. })
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
            MSG_MERKLE_NODE_REQUEST => self.require_auth(&env, |s, e| s.on_merkle_node(e)),
            MSG_MERKLE_LEAF_REQUEST => self.require_auth(&env, |s, e| s.on_merkle_leaf(e)),
            MSG_DELTA_REQUEST => self.require_auth(&env, |s, e| s.on_delta(e)),
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
        self.phase = Phase::Authed {
            caps: caps.iter().map(|c| (*c).to_string()).collect(),
            peer_id: claimed,
        };
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
        let colluding = guard.colluding;
        for op in operations {
            let (outcome, reason, parsed) = parse_stored(op, &ds);
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
            if !colluding && !author_write_allowed(&mut *guard.store, &ds, op, parsed.author)? {
                outcomes.push(Cbor::Map(vec![
                    ("op_id".into(), Cbor::Bytes(id.to_vec())),
                    ("outcome".into(), Cbor::Text("REJECT".into())),
                    ("reason".into(), Cbor::Text("AUTHZ".into())),
                ]));
                continue;
            }
            let inserted = guard.store.insert(&ds, parsed)?;
            if inserted {
                apply_membership_from_op(&mut *guard.store, &ds, op, id)?;
            }
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

    fn has_cap(&self, cap: &str) -> bool {
        matches!(&self.phase, Phase::Authed { caps, .. } if caps.iter().any(|c| c == cap))
    }

    fn on_sync(&mut self, env: &Envelope) -> Result<Vec<Vec<u8>>, RelayError> {
        let ds = text(map_get(&env.payload, "datastore"))?;
        if !self.datastore_allowed(&ds)? {
            return Ok(vec![error_frame(
                env.request_id,
                0x202,
                "MEMBERSHIP_DENIED",
                false,
            )]);
        }
        let cursor = map_get(&env.payload, "cursor");
        let frontier = parse_frontier(cursor);
        let guard = self.inner.lock().map_err(|_| RelayError::Poison)?;
        let stored = guard.store.list(&ds)?;
        drop(guard);
        let tree = MerkleTree::build(&stored.iter().map(StoredOp::merkle).collect::<Vec<_>>());
        let root = tree.root().to_vec();
        let merkle_walk = self.has_cap("merkle-walk-v1");
        let mut response = vec![
            ("datastore".into(), Cbor::Text(ds.clone())),
            ("validated_root".into(), Cbor::Bytes(root)),
        ];
        if merkle_walk {
            response.extend([
                (
                    "merkle_format_version".into(),
                    Cbor::Uint(MERKLE_FORMAT_VERSION as u64),
                ),
                ("bucket_width_ms".into(), Cbor::Uint(BUCKET_WIDTH_MS)),
                (
                    "bucket_indices".into(),
                    Cbor::Array(
                        tree.active_bucket_indices()
                            .into_iter()
                            .map(Cbor::Uint)
                            .collect(),
                    ),
                ),
            ]);
            // Freeze the responder view for all node/leaf/delta requests in this walk.
            self.walk_snapshots.insert(ds.clone(), stored);
            return Ok(vec![encode_env(
                MSG_SYNC_RESPONSE,
                env.request_id,
                Cbor::Map(response),
            )]);
        }

        let mut out = vec![encode_env(
            MSG_SYNC_RESPONSE,
            env.request_id,
            Cbor::Map(response),
        )];
        // Compatibility path for clients that did not negotiate merkle-walk-v1.
        let held: Vec<HeldOp> = stored.iter().map(StoredOp::held).collect();
        let want: HashSet<String> = retransmit(&held, &frontier, &[]).into_iter().collect();
        let operations = stored
            .into_iter()
            .filter(|o| want.contains(&hex::encode(o.op_id)))
            .map(stored_to_cbor)
            .collect();
        out.extend(chunk_ops_frames(&ds, operations));
        Ok(out)
    }

    fn on_merkle_node(&mut self, env: &Envelope) -> Result<Vec<Vec<u8>>, RelayError> {
        let ds = text(map_get(&env.payload, "datastore"))?;
        if !self.datastore_allowed(&ds)? {
            return Ok(vec![error_frame(
                env.request_id,
                0x202,
                "MEMBERSHIP_DENIED",
                false,
            )]);
        }
        let level = uint(map_get(&env.payload, "level"))? as usize;
        let index = uint(map_get(&env.payload, "index"))? as usize;
        let stored = self
            .walk_snapshots
            .get(&ds)
            .ok_or_else(|| RelayError::Protocol("no frozen merkle walk".into()))?;
        let tree = MerkleTree::build(&stored.iter().map(StoredOp::merkle).collect::<Vec<_>>());
        let hash = tree
            .levels
            .get(level)
            .and_then(|nodes| nodes.get(index))
            .ok_or_else(|| RelayError::Protocol("merkle node out of range".into()))?;
        let (left, right) = tree
            .node_children(level, index)
            .ok_or_else(|| RelayError::Protocol("merkle node has no children".into()))?;
        Ok(vec![encode_env(
            MSG_MERKLE_NODE_RESPONSE,
            env.request_id,
            Cbor::Map(vec![
                ("datastore".into(), Cbor::Text(ds)),
                ("level".into(), Cbor::Uint(level as u64)),
                ("index".into(), Cbor::Uint(index as u64)),
                ("hash".into(), Cbor::Bytes(hash.to_vec())),
                ("left".into(), Cbor::Bytes(left.to_vec())),
                ("right".into(), Cbor::Bytes(right.to_vec())),
            ]),
        )])
    }

    fn on_merkle_leaf(&mut self, env: &Envelope) -> Result<Vec<Vec<u8>>, RelayError> {
        let ds = text(map_get(&env.payload, "datastore"))?;
        if !self.datastore_allowed(&ds)? {
            return Ok(vec![error_frame(
                env.request_id,
                0x202,
                "MEMBERSHIP_DENIED",
                false,
            )]);
        }
        let leaf_index = uint(map_get(&env.payload, "leaf_index"))? as usize;
        let stored = self
            .walk_snapshots
            .get(&ds)
            .ok_or_else(|| RelayError::Protocol("no frozen merkle walk".into()))?;
        let tree = MerkleTree::build(&stored.iter().map(StoredOp::merkle).collect::<Vec<_>>());
        let leaf = tree
            .leaves
            .get(leaf_index)
            .ok_or_else(|| RelayError::Protocol("merkle leaf out of range".into()))?;
        Ok(vec![encode_env(
            MSG_MERKLE_LEAF_RESPONSE,
            env.request_id,
            Cbor::Map(vec![
                ("datastore".into(), Cbor::Text(ds)),
                ("leaf_index".into(), Cbor::Uint(leaf_index as u64)),
                (
                    "bucket_index".into(),
                    leaf.bucket_index.map(Cbor::Uint).unwrap_or(Cbor::Null),
                ),
                (
                    "op_ids".into(),
                    Cbor::Array(
                        leaf.op_ids
                            .iter()
                            .map(|id| Cbor::Bytes(id.to_vec()))
                            .collect(),
                    ),
                ),
            ]),
        )])
    }

    fn on_delta(&mut self, env: &Envelope) -> Result<Vec<Vec<u8>>, RelayError> {
        let ds = text(map_get(&env.payload, "datastore"))?;
        if !self.datastore_allowed(&ds)? {
            return Ok(vec![error_frame(
                env.request_id,
                0x202,
                "MEMBERSHIP_DENIED",
                false,
            )]);
        }
        let wanted: HashSet<[u8; 32]> = match map_get(&env.payload, "op_ids") {
            Cbor::Array(ids) => ids.iter().filter_map(|id| take32(id).ok()).collect(),
            _ => return Err(RelayError::Protocol("delta op_ids".into())),
        };
        let stored = self
            .walk_snapshots
            .get(&ds)
            .ok_or_else(|| RelayError::Protocol("no frozen merkle walk".into()))?;
        let operations: Vec<Cbor> = stored
            .iter()
            .filter(|op| wanted.contains(&op.op_id))
            .cloned()
            .map(stored_to_cbor)
            .collect();
        Ok(chunk_delta_frames(&ds, env.request_id, operations))
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
            let (ds, token) = match item {
                Cbor::Text(_) => (text(item)?, None),
                Cbor::Map(_) => {
                    let ds = text(map_get(item, "id"))?;
                    let token = match parse_admission_token(map_get(item, "token")) {
                        Ok(token) => token,
                        Err(_) => {
                            return Ok(vec![error_frame(
                                env.request_id,
                                0x202,
                                "MEMBERSHIP_DENIED",
                                false,
                            )]);
                        }
                    };
                    (ds, Some(token))
                }
                _ => {
                    return Ok(vec![error_frame(
                        env.request_id,
                        0x400,
                        "BAD_SUBSCRIBE",
                        false,
                    )]);
                }
            };
            let grants = guard.store.grants(&ds)?;
            if !guard.colluding && !grants.is_empty() {
                let Some(token) = token else {
                    return Ok(vec![error_frame(
                        env.request_id,
                        0x202,
                        "MEMBERSHIP_DENIED",
                        false,
                    )]);
                };
                let peer_id = match &self.phase {
                    Phase::Authed { peer_id, .. } => *peer_id,
                    _ => {
                        return Ok(vec![error_frame(
                            env.request_id,
                            0x201,
                            "AUTH_FAILED",
                            true,
                        )]);
                    }
                };
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| RelayError::Protocol(e.to_string()))?
                    .as_millis() as u64;
                if !token.scopes.contains(&SCOPE_READ)
                    || !token.scopes.contains(&SCOPE_SYNC)
                    || verify_admission_token_at(&token, &grants, &peer_id, now_ms).is_err()
                {
                    return Ok(vec![error_frame(
                        env.request_id,
                        0x202,
                        "MEMBERSHIP_DENIED",
                        false,
                    )]);
                }
                self.authorized.insert(ds.clone(), token.grant);
            }
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

    fn datastore_allowed(&self, ds: &str) -> Result<bool, RelayError> {
        let guard = self.inner.lock().map_err(|_| RelayError::Poison)?;
        if guard.colluding {
            return Ok(true);
        }
        let grants = guard.store.grants(ds)?;
        if grants.is_empty() {
            return Ok(true);
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| RelayError::Protocol(e.to_string()))?
            .as_millis() as u64;
        let live = |grant: &KnownGrant| {
            !grant.revoked && grant.expiry.is_none_or(|expiry| now_ms < expiry)
        };
        if let Some(id) = self.authorized.get(ds) {
            return Ok(grants.iter().any(|grant| grant.id == *id && live(grant)));
        }
        // Solo-device: HELLO/AUTH PeerId == PrincipalId, so a live grant for
        // this subject is enough for SYNC without a separate SUBSCRIBE token.
        let Phase::Authed { peer_id, .. } = &self.phase else {
            return Ok(false);
        };
        Ok(grants
            .iter()
            .any(|grant| grant.subject == *peer_id && live(grant)))
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

fn stored_to_cbor(o: StoredOp) -> Cbor {
    cbor::decode(&o.body).unwrap_or_else(|_| {
        Cbor::Map(vec![
            ("op_id".into(), Cbor::Bytes(o.op_id.to_vec())),
            ("author".into(), Cbor::Bytes(o.author.to_vec())),
            ("physical_ms".into(), Cbor::Uint(o.physical_ms)),
            ("logical".into(), Cbor::Uint(o.logical as u64)),
        ])
    })
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

fn delta_frame(ds: &str, request_id: u32, operations: Vec<Cbor>, remaining: usize) -> Vec<u8> {
    encode_env(
        MSG_DELTA_BATCH,
        request_id,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(ds.into())),
            ("operations".into(), Cbor::Array(operations)),
            ("remaining".into(), Cbor::Uint(remaining as u64)),
        ]),
    )
}

fn chunk_delta_frames(ds: &str, request_id: u32, operations: Vec<Cbor>) -> Vec<Vec<u8>> {
    if operations.is_empty() {
        return vec![delta_frame(ds, request_id, vec![], 0)];
    }
    let mut chunks: Vec<Vec<Cbor>> = Vec::new();
    let mut chunk = Vec::new();
    for op in operations {
        if !chunk.is_empty() {
            let mut trial = chunk.clone();
            trial.push(op.clone());
            if chunk.len() >= MAX_BATCH_OPS
                || delta_frame(ds, request_id, trial, 0).len() > MAX_BATCH_BYTES
            {
                chunks.push(std::mem::take(&mut chunk));
            }
        }
        chunk.push(op);
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    let total = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| delta_frame(ds, request_id, chunk, total - i - 1))
        .collect()
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

fn uint_array(c: &Cbor) -> Result<Vec<u64>, RelayError> {
    match c {
        Cbor::Array(values) => values.iter().map(uint).collect(),
        _ => Err(RelayError::Protocol("uint array".into())),
    }
}

fn optional_uint(c: &Cbor) -> Result<Option<u64>, RelayError> {
    match c {
        Cbor::Null => Ok(None),
        Cbor::Uint(value) => Ok(Some(*value)),
        _ => Err(RelayError::Protocol("optional uint".into())),
    }
}

fn optional_b32(c: &Cbor) -> Result<Option<[u8; 32]>, RelayError> {
    match c {
        Cbor::Null => Ok(None),
        _ => take32(c).map(Some),
    }
}

fn parse_admission_token(c: &Cbor) -> Result<AdmissionToken, RelayError> {
    if !matches!(c, Cbor::Map(_)) {
        return Err(RelayError::Protocol("admission token".into()));
    }
    let cert = map_get(c, "cert");
    if !matches!(cert, Cbor::Map(_)) {
        return Err(RelayError::Protocol("device cert".into()));
    }
    Ok(AdmissionToken {
        ds: take32(map_get(c, "ds"))?,
        subject: take32(map_get(c, "subject"))?,
        grant: take32(map_get(c, "grant"))?,
        scopes: uint_array(map_get(c, "scopes"))?,
        device: take32(map_get(c, "device"))?,
        cert: DeviceCert {
            kr: uint(map_get(cert, "kr"))?,
            device_pk: take32(map_get(cert, "device"))?,
            principal_id: take32(map_get(cert, "principal"))?,
            root_pk: take32(map_get(cert, "root_pk"))?,
            issued: uint(map_get(cert, "issued"))?,
            expiry: optional_uint(map_get(cert, "expiry"))?,
            revoke_of: optional_b32(map_get(cert, "revoke_of"))?,
            cert_sig: take64(map_get(cert, "cert_sig"))?,
        },
        sig: take64(map_get(c, "sig"))?,
    })
}

fn wire_json(op: &Cbor) -> Option<serde_json::Value> {
    match map_get(op, "wire") {
        Cbor::Text(s) => serde_json::from_str(s).ok(),
        _ => None,
    }
}

fn hex_field(value: &serde_json::Value, key: &str) -> Option<[u8; 32]> {
    let hex = value.get(key)?.as_str()?;
    let bytes = hex::decode(hex).ok()?;
    bytes.try_into().ok()
}

fn ds_bytes(ds: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(ds).ok()?;
    bytes.try_into().ok()
}

fn author_write_allowed(
    store: &mut dyn OpStore,
    ds: &str,
    op: &Cbor,
    author: [u8; 32],
) -> Result<bool, RelayError> {
    let grants = store.grants(ds)?;
    if grants.is_empty() {
        return Ok(true);
    }
    let kind = wire_json(op)
        .and_then(|wire| wire.get("kind").and_then(|v| v.as_u64()))
        .unwrap_or(1);
    let need = match kind {
        KIND_GENESIS => return Ok(true),
        KIND_CAP_GRANT | KIND_CAP_REVOKE => SCOPE_ADMIN,
        _ => SCOPE_WRITE,
    };
    let physical_ms = match map_get(op, "physical_ms") {
        Cbor::Uint(n) => *n,
        _ => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    };
    Ok(grants.iter().any(|grant| {
        grant.subject == author
            && !grant.revoked
            && grant.scopes.contains(&need)
            && grant.expiry.is_none_or(|expiry| physical_ms < expiry)
    }))
}

fn apply_membership_from_op(
    store: &mut dyn OpStore,
    ds: &str,
    op: &Cbor,
    op_id: [u8; 32],
) -> Result<(), RelayError> {
    let Some(wire) = wire_json(op) else {
        return Ok(());
    };
    let kind = wire
        .get("kind")
        .and_then(|v| v.as_u64())
        .unwrap_or(u64::MAX);
    let Some(ds_bytes) = ds_bytes(ds) else {
        return Ok(());
    };
    match kind {
        KIND_GENESIS => {
            let Some(founder) = wire.get("body").and_then(|body| hex_field(body, "founder")) else {
                return Ok(());
            };
            store.upsert_grant(KnownGrant {
                id: op_id,
                ds: ds_bytes,
                subject: founder,
                scopes: vec![SCOPE_WRITE, SCOPE_ADMIN, SCOPE_READ, SCOPE_SYNC],
                expiry: None,
                revoked: false,
            })?;
        }
        KIND_CAP_GRANT => {
            let Some(body) = wire.get("body") else {
                return Ok(());
            };
            let Some(subject) = hex_field(body, "subject") else {
                return Ok(());
            };
            let scopes = body
                .get("scopes")
                .and_then(|v| v.as_array())
                .map(|items| items.iter().filter_map(|item| item.as_u64()).collect())
                .unwrap_or_default();
            let expiry = match body.get("expiry") {
                None | Some(serde_json::Value::Null) => None,
                Some(value) => value.as_u64(),
            };
            store.upsert_grant(KnownGrant {
                id: op_id,
                ds: ds_bytes,
                subject,
                scopes,
                expiry,
                revoked: false,
            })?;
        }
        KIND_CAP_REVOKE => {
            if let Some(grant) = wire.get("body").and_then(|body| hex_field(body, "grant")) {
                store.revoke_grant(ds, &grant)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_stored(
    op: &Cbor,
    datastore: &str,
) -> (&'static str, Option<&'static str>, Option<StoredOp>) {
    match admit_experimental_op(op, datastore) {
        Ok(admitted) => {
            let body = cbor::encode(op).unwrap_or_default();
            (
                "ACCEPT",
                None,
                Some(StoredOp {
                    op_id: admitted.op_id,
                    author: admitted.author,
                    physical_ms: admitted.physical_ms,
                    logical: admitted.logical,
                    body,
                }),
            )
        }
        Err(reason) => ("REJECT", Some(reason.reason()), None),
    }
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
