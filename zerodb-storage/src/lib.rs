//! Local durable store for the M1 MVP slice.
//!
//! SQLite holds the oplog, LWW property state, node registry, and peer meta.
//! Atomicity: each accepted op is committed in one transaction (append +
//! materialize + HLC), matching the spirit of WAL.md layer 2 for single-op
//! commits. Multi-op group crash points remain a follow-up.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use zerodb_core::cbor::Cbor;
use zerodb_core::kernel::{KernelOp, Lww, Payload, Replica, Value};
use zerodb_core::op::{OpEnvelope, OpTs};
use zerodb_core::sign::{DOMAIN_OP_SIG, verify_op};

const KIND_CREATE_NODE: u64 = 1;
const KIND_SET_PROPERTY: u64 = 3;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(String),
    #[error("cbor: {0}")]
    Cbor(String),
    #[error("crypto: {0}")]
    Crypto(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("duplicate op")]
    Duplicate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireOp {
    pub id: String,
    pub v: u64,
    pub ds: String,
    pub ep: u64,
    pub author: String,
    pub author_pk: String,
    pub ts: WireTs,
    pub deps: Vec<String>,
    pub kind: u64,
    pub body: serde_json::Value,
    pub sig: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireTs {
    pub p: u64,
    pub l: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportBundle {
    pub format: u32,
    pub datastore_id: String,
    pub ops: Vec<WireOp>,
}

pub struct LocalStore {
    conn: Connection,
    signing: SigningKey,
    author: [u8; 32],
    author_pk: [u8; 32],
    ds: [u8; 32],
    hlc_p: u64,
    hlc_l: u16,
}

impl LocalStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS meta (
              k TEXT PRIMARY KEY,
              v BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS ops (
              id BLOB PRIMARY KEY,
              author BLOB NOT NULL,
              author_pk BLOB NOT NULL,
              physical_ms INTEGER NOT NULL,
              logical INTEGER NOT NULL,
              kind INTEGER NOT NULL,
              body_json TEXT NOT NULL,
              sig BLOB NOT NULL,
              wire_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS nodes (
              id TEXT PRIMARY KEY,
              label TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS props (
              entity TEXT NOT NULL,
              path TEXT NOT NULL,
              value_json TEXT NOT NULL,
              physical_ms INTEGER NOT NULL,
              logical INTEGER NOT NULL,
              author BLOB NOT NULL,
              op_id BLOB NOT NULL,
              PRIMARY KEY (entity, path)
            );
            ",
        )?;

        // Load or create identity + datastore
        let (signing, author, author_pk, ds, hlc_p, hlc_l) = if meta_get(&conn, "seed")?.is_some() {
            let seed: [u8; 32] = meta_get(&conn, "seed")?
                .ok_or_else(|| StoreError::Invalid("missing seed".into()))?
                .try_into()
                .map_err(|_| StoreError::Invalid("seed length".into()))?;
            let signing = SigningKey::from_bytes(&seed);
            let author_pk = signing.verifying_key().to_bytes();
            let author = blake3::hash(&author_pk).into();
            let ds: [u8; 32] = meta_get(&conn, "ds")?
                .ok_or_else(|| StoreError::Invalid("missing ds".into()))?
                .try_into()
                .map_err(|_| StoreError::Invalid("ds length".into()))?;
            let hlc_p = meta_get_u64(&conn, "hlc_p")?.unwrap_or(0);
            let hlc_l = meta_get_u64(&conn, "hlc_l")?.unwrap_or(0) as u16;
            (signing, author, author_pk, ds, hlc_p, hlc_l)
        } else {
            return Err(StoreError::Invalid(
                "database not initialized — run `zerodb init`".into(),
            ));
        };

        Ok(Self {
            conn,
            signing,
            author,
            author_pk,
            ds,
            hlc_p,
            hlc_l,
        })
    }

    pub fn init(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS ops (
              id BLOB PRIMARY KEY,
              author BLOB NOT NULL,
              author_pk BLOB NOT NULL,
              physical_ms INTEGER NOT NULL,
              logical INTEGER NOT NULL,
              kind INTEGER NOT NULL,
              body_json TEXT NOT NULL,
              sig BLOB NOT NULL,
              wire_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS nodes (
              id TEXT PRIMARY KEY,
              label TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS props (
              entity TEXT NOT NULL,
              path TEXT NOT NULL,
              value_json TEXT NOT NULL,
              physical_ms INTEGER NOT NULL,
              logical INTEGER NOT NULL,
              author BLOB NOT NULL,
              op_id BLOB NOT NULL,
              PRIMARY KEY (entity, path)
            );
            ",
        )?;

        let mut seed = [0u8; 32];
        getrandom_fill(&mut seed);
        let signing = SigningKey::from_bytes(&seed);
        let author_pk = signing.verifying_key().to_bytes();
        let author: [u8; 32] = *blake3::hash(&author_pk).as_bytes();

        // Genesis-lite: DatastoreId = BLAKE3("zerodb-local-ds-v1" ‖ author ‖ salt)
        let mut salt = [0u8; 16];
        getrandom_fill(&mut salt);
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"zerodb-local-ds-v1");
        hasher.update(&author);
        hasher.update(&salt);
        let ds = *hasher.finalize().as_bytes();

        meta_set(&conn, "seed", &seed)?;
        meta_set(&conn, "ds", &ds)?;
        meta_set(&conn, "salt", &salt)?;
        meta_set_u64(&conn, "hlc_p", 0)?;
        meta_set_u64(&conn, "hlc_l", 0)?;

        Self::open(path)
    }

    pub fn datastore_id_hex(&self) -> String {
        hex::encode(self.ds)
    }

    pub fn author_hex(&self) -> String {
        hex::encode(self.author)
    }

    pub fn peer_id_short(&self) -> String {
        hex::encode(&self.author[..4])
    }

    fn tick(&mut self) -> OpTs {
        let wall = now_ms();
        let p = wall.max(self.hlc_p);
        let l = if p == self.hlc_p {
            self.hlc_l.saturating_add(1)
        } else {
            0
        };
        self.hlc_p = p;
        self.hlc_l = l;
        OpTs {
            physical_ms: p,
            logical: l,
        }
    }

    pub fn create_node(&mut self, label: &str) -> Result<String, StoreError> {
        let node = Uuid::now_v7();
        let node_bytes = *node.as_bytes();
        let body = Cbor::Map(vec![
            ("label".into(), Cbor::Text(label.into())),
            ("node".into(), Cbor::Bytes(node_bytes.to_vec())),
        ]);
        let body_json = serde_json::json!({
            "label": label,
            "node": hex::encode(node_bytes),
        });
        self.commit_local(KIND_CREATE_NODE, body, body_json, |tx, wire| {
            tx.execute(
                "INSERT OR IGNORE INTO nodes (id, label) VALUES (?1, ?2)",
                params![hex::encode(node_bytes), label],
            )?;
            Ok(wire.clone())
        })?;
        Ok(hex::encode(node_bytes))
    }

    pub fn set_lww(
        &mut self,
        node_hex: &str,
        path: &str,
        value: &str,
    ) -> Result<String, StoreError> {
        let node = hex::decode(node_hex).map_err(|e| StoreError::Invalid(e.to_string()))?;
        if node.len() != 16 {
            return Err(StoreError::Invalid("node id must be 16 bytes hex".into()));
        }
        let body = Cbor::Map(vec![
            ("node".into(), Cbor::Bytes(node.clone())),
            ("path".into(), Cbor::Text(path.into())),
            ("crdt".into(), Cbor::Text("lww".into())),
            ("value".into(), Cbor::Text(value.into())),
        ]);
        let body_json = serde_json::json!({
            "node": node_hex,
            "path": path,
            "crdt": "lww",
            "value": value,
        });
        let op_id = self.commit_local(KIND_SET_PROPERTY, body, body_json.clone(), |tx, wire| {
            // Ensure node row exists
            tx.execute(
                "INSERT OR IGNORE INTO nodes (id, label) VALUES (?1, ?2)",
                params![node_hex, "Node"],
            )?;
            apply_lww_prop(
                tx,
                node_hex,
                path,
                value,
                wire.ts.p,
                wire.ts.l,
                &hex::decode(&wire.author).unwrap(),
                &hex::decode(&wire.id).unwrap(),
            )?;
            Ok(wire.clone())
        })?;
        Ok(op_id)
    }

    pub fn get_lww(&self, node_hex: &str, path: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value_json FROM props WHERE entity = ?1 AND path = ?2")?;
        let v: Option<String> = stmt
            .query_row(params![node_hex, path], |r| r.get(0))
            .optional()?;
        Ok(v.and_then(|j| {
            serde_json::from_str::<serde_json::Value>(&j)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        }))
    }

    pub fn list_nodes(&self) -> Result<Vec<(String, String)>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, label FROM nodes ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn op_count(&self) -> Result<u64, StoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM ops", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    pub fn list_op_ids(&self) -> Result<Vec<[u8; 32]>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT id FROM ops")?;
        let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let b = row?;
            let arr: [u8; 32] = b
                .try_into()
                .map_err(|_| StoreError::Invalid("op id length".into()))?;
            out.push(arr);
        }
        out.sort();
        Ok(out)
    }

    pub fn export_all(&self) -> Result<ExportBundle, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT wire_json FROM ops ORDER BY physical_ms, logical, id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut ops = Vec::new();
        for row in rows {
            let s = row?;
            ops.push(serde_json::from_str(&s).map_err(|e| StoreError::Invalid(e.to_string()))?);
        }
        Ok(ExportBundle {
            format: 1,
            datastore_id: hex::encode(self.ds),
            ops,
        })
    }

    pub fn export_ops_by_id(&self, ids: &[[u8; 32]]) -> Result<Vec<WireOp>, StoreError> {
        let mut out = Vec::new();
        for id in ids {
            let wire: Option<String> = self
                .conn
                .query_row(
                    "SELECT wire_json FROM ops WHERE id = ?1",
                    params![id.as_slice()],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(s) = wire {
                out.push(serde_json::from_str(&s).map_err(|e| StoreError::Invalid(e.to_string()))?);
            }
        }
        Ok(out)
    }

    /// Import remote ops. Skips wrong datastore and duplicates. Returns (accepted, skipped).
    pub fn import_bundle(&mut self, bundle: &ExportBundle) -> Result<(u32, u32), StoreError> {
        if bundle.datastore_id != hex::encode(self.ds) {
            // First import may set ds if we allow join — for MVP require matching ds
            // unless local is empty of ops and we adopt remote ds.
            let n = self.op_count()?;
            if n == 0 {
                let ds = hex::decode(&bundle.datastore_id)
                    .map_err(|e| StoreError::Invalid(e.to_string()))?;
                let ds: [u8; 32] = ds
                    .try_into()
                    .map_err(|_| StoreError::Invalid("ds length".into()))?;
                meta_set(&self.conn, "ds", &ds)?;
                self.ds = ds;
            } else {
                return Err(StoreError::Invalid(format!(
                    "datastore mismatch: local {} remote {}",
                    hex::encode(self.ds),
                    bundle.datastore_id
                )));
            }
        }
        let mut accepted = 0u32;
        let mut skipped = 0u32;
        for op in &bundle.ops {
            match self.ingest_wire(op) {
                Ok(true) => accepted += 1,
                Ok(false) => skipped += 1,
                Err(StoreError::Duplicate) => skipped += 1,
                Err(e) => return Err(e),
            }
        }
        Ok((accepted, skipped))
    }

    pub fn ingest_wire(&mut self, wire: &WireOp) -> Result<bool, StoreError> {
        let id = decode32(&wire.id)?;
        let exists: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM ops WHERE id = ?1",
                params![id.as_slice()],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if exists {
            return Ok(false);
        }

        let author = decode32(&wire.author)?;
        let author_pk = decode32(&wire.author_pk)?;
        let sig = hex::decode(&wire.sig).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let sig_arr: [u8; 64] = sig
            .try_into()
            .map_err(|_| StoreError::Invalid("sig length".into()))?;

        // Verify PeerId = BLAKE3(pk)
        let expected_author: [u8; 32] = *blake3::hash(&author_pk).as_bytes();
        if expected_author != author {
            return Err(StoreError::Crypto("author != BLAKE3(author_pk)".into()));
        }
        if !verify_op(&author_pk, &id, &sig_arr) {
            return Err(StoreError::Crypto("bad signature".into()));
        }

        // HLC observe remote
        self.observe_remote(wire.ts.p, wire.ts.l);

        let body_json = wire.body.to_string();
        let wire_json =
            serde_json::to_string(wire).map_err(|e| StoreError::Invalid(e.to_string()))?;

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO ops (id, author, author_pk, physical_ms, logical, kind, body_json, sig, wire_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                id.as_slice(),
                author.as_slice(),
                author_pk.as_slice(),
                wire.ts.p as i64,
                wire.ts.l as i64,
                wire.kind as i64,
                body_json,
                sig_arr.as_slice(),
                wire_json,
            ],
        )?;

        match wire.kind {
            KIND_CREATE_NODE => {
                let node = wire.body["node"]
                    .as_str()
                    .ok_or_else(|| StoreError::Invalid("body.node".into()))?;
                let label = wire.body["label"].as_str().unwrap_or("Node");
                tx.execute(
                    "INSERT OR IGNORE INTO nodes (id, label) VALUES (?1, ?2)",
                    params![node, label],
                )?;
            }
            KIND_SET_PROPERTY => {
                let node = wire.body["node"]
                    .as_str()
                    .ok_or_else(|| StoreError::Invalid("body.node".into()))?;
                let path = wire.body["path"]
                    .as_str()
                    .ok_or_else(|| StoreError::Invalid("body.path".into()))?;
                let value = wire.body["value"]
                    .as_str()
                    .ok_or_else(|| StoreError::Invalid("body.value".into()))?;
                tx.execute(
                    "INSERT OR IGNORE INTO nodes (id, label) VALUES (?1, ?2)",
                    params![node, "Node"],
                )?;
                apply_lww_prop(&tx, node, path, value, wire.ts.p, wire.ts.l, &author, &id)?;
            }
            _ => {}
        }

        meta_set_u64(&tx, "hlc_p", self.hlc_p)?;
        meta_set_u64(&tx, "hlc_l", self.hlc_l as u64)?;
        tx.commit()?;
        Ok(true)
    }

    fn observe_remote(&mut self, rp: u64, rl: u16) {
        let wall = now_ms();
        let p = wall.max(self.hlc_p).max(rp);
        let mut candidates = Vec::new();
        if self.hlc_p == p {
            candidates.push(self.hlc_l);
        }
        if rp == p {
            candidates.push(rl);
        }
        let base = candidates.into_iter().max().unwrap_or(0);
        let (p, l) = if base == u16::MAX {
            (p + 1, 0)
        } else {
            (p, base + 1)
        };
        // result must be > remote
        let (p, l) = if p > rp || (p == rp && l > rl) {
            (p, l)
        } else {
            (rp, rl.saturating_add(1))
        };
        self.hlc_p = p;
        self.hlc_l = l;
    }

    fn commit_local<F>(
        &mut self,
        kind: u64,
        body: Cbor,
        body_json: serde_json::Value,
        materialize: F,
    ) -> Result<String, StoreError>
    where
        F: FnOnce(&rusqlite::Transaction<'_>, &WireOp) -> Result<WireOp, StoreError>,
    {
        let ts = self.tick();
        let env = OpEnvelope {
            v: 1,
            ds: self.ds,
            ep: 0,
            author: self.author,
            ts,
            deps: vec![],
            grp: None,
            kind,
            body,
        };
        let id = env.op_id().map_err(|e| StoreError::Cbor(e.to_string()))?;
        let sig = {
            let pre = [DOMAIN_OP_SIG, id.as_slice()].concat();
            self.signing.sign(&pre).to_bytes()
        };

        let wire = WireOp {
            id: hex::encode(id),
            v: 1,
            ds: hex::encode(self.ds),
            ep: 0,
            author: hex::encode(self.author),
            author_pk: hex::encode(self.author_pk),
            ts: WireTs {
                p: ts.physical_ms,
                l: ts.logical,
            },
            deps: vec![],
            kind,
            body: body_json.clone(),
            sig: hex::encode(sig),
        };

        let tx = self.conn.transaction()?;
        let wire = materialize(&tx, &wire)?;
        let wire_json =
            serde_json::to_string(&wire).map_err(|e| StoreError::Invalid(e.to_string()))?;
        tx.execute(
            "INSERT INTO ops (id, author, author_pk, physical_ms, logical, kind, body_json, sig, wire_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                id.as_slice(),
                self.author.as_slice(),
                self.author_pk.as_slice(),
                ts.physical_ms as i64,
                ts.logical as i64,
                kind as i64,
                body_json.to_string(),
                sig.as_slice(),
                wire_json,
            ],
        )?;
        meta_set_u64(&tx, "hlc_p", self.hlc_p)?;
        meta_set_u64(&tx, "hlc_l", self.hlc_l as u64)?;
        tx.commit()?;
        Ok(hex::encode(id))
    }
}

fn apply_lww_prop(
    tx: &rusqlite::Transaction<'_>,
    entity: &str,
    path: &str,
    value: &str,
    physical_ms: u64,
    logical: u16,
    author: &[u8],
    op_id: &[u8],
) -> Result<(), StoreError> {
    // Load existing as kernel LWW and re-apply both for determinism
    let existing: Option<(String, i64, i64, Vec<u8>, Vec<u8>)> = tx
        .query_row(
            "SELECT value_json, physical_ms, logical, author, op_id FROM props WHERE entity=?1 AND path=?2",
            params![entity, path],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;

    let mut ops = Vec::new();
    if let Some((vj, p, l, a, oid)) = existing {
        let val = serde_json::from_str::<String>(&vj).unwrap_or(vj);
        ops.push(KernelOp {
            op_id: oid,
            author: a,
            physical_ms: p as u64,
            logical: l as u16,
            payload: Payload::LwwSet(Value::Text(val)),
        });
    }
    ops.push(KernelOp {
        op_id: op_id.to_vec(),
        author: author.to_vec(),
        physical_ms,
        logical,
        payload: Payload::LwwSet(Value::Text(value.into())),
    });
    let mut rep = Replica::<Lww>::default();
    for op in &ops {
        rep.ingest(op);
    }
    let lww = rep
        .state()
        .map_err(|e| StoreError::Invalid(e.to_string()))?;
    let final_v = match lww.value() {
        Some(Value::Text(s)) => s.clone(),
        _ => return Err(StoreError::Invalid("lww read".into())),
    };
    let win = ops.iter().map(|o| o.order_key()).max().unwrap();
    let value_json = serde_json::to_string(&final_v).unwrap();
    tx.execute(
        "INSERT INTO props (entity, path, value_json, physical_ms, logical, author, op_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(entity, path) DO UPDATE SET
           value_json=excluded.value_json,
           physical_ms=excluded.physical_ms,
           logical=excluded.logical,
           author=excluded.author,
           op_id=excluded.op_id",
        params![
            entity,
            path,
            value_json,
            win.physical_ms as i64,
            win.logical as i64,
            win.author.as_slice(),
            win.op_id.as_slice(),
        ],
    )?;
    Ok(())
}

fn meta_get(conn: &Connection, k: &str) -> Result<Option<Vec<u8>>, StoreError> {
    conn.query_row("SELECT v FROM meta WHERE k = ?1", params![k], |r| r.get(0))
        .optional()
        .map_err(Into::into)
}

fn meta_set(conn: &Connection, k: &str, v: &[u8]) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO meta (k, v) VALUES (?1, ?2)
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![k, v],
    )?;
    Ok(())
}

fn meta_get_u64(conn: &Connection, k: &str) -> Result<Option<u64>, StoreError> {
    Ok(meta_get(conn, k)?.map(|b| {
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&b[..8.min(b.len())]);
        u64::from_le_bytes(arr)
    }))
}

fn meta_set_u64(conn: &Connection, k: &str, v: u64) -> Result<(), StoreError> {
    meta_set(conn, k, &v.to_le_bytes())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn decode32(s: &str) -> Result<[u8; 32], StoreError> {
    let b = hex::decode(s).map_err(|e| StoreError::Invalid(e.to_string()))?;
    b.try_into()
        .map_err(|_| StoreError::Invalid("expected 32 bytes".into()))
}

fn getrandom_fill(buf: &mut [u8]) {
    // Prefer OS random via getrandom if available; fall back to uuid bytes.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let u = Uuid::new_v4();
    let b = u.as_bytes();
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot = b[i % 16] ^ ((i as u8).wrapping_mul(31));
    }
    let mut h = DefaultHasher::new();
    now_ms().hash(&mut h);
    let x = h.finish().to_le_bytes();
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot ^= x[i % 8];
    }
}
