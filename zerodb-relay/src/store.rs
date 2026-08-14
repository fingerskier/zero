//! Durable validated oplog. The relay hashes this set as `validated_root`.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;
use zerodb_core::merkle::MerkleOp;
use zerodb_core::relay::{HeldOp, root_hex};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(String),
}

#[derive(Debug, Clone)]
pub struct StoredOp {
    pub op_id: [u8; 32],
    pub author: [u8; 32],
    pub physical_ms: u64,
    pub logical: u16,
    pub body: Vec<u8>,
}

impl StoredOp {
    pub fn held(&self) -> HeldOp {
        HeldOp {
            op_id: hex::encode(self.op_id),
            author: hex::encode(self.author),
            physical_ms: self.physical_ms,
            logical: self.logical,
        }
    }

    pub fn merkle(&self) -> MerkleOp {
        MerkleOp {
            op_id: self.op_id,
            author: self.author,
            physical_ms: self.physical_ms,
            logical: self.logical,
        }
    }
}

pub trait OpStore: Send {
    fn insert(&mut self, ds: &str, op: StoredOp) -> Result<bool, StoreError>;
    fn list(&self, ds: &str) -> Result<Vec<StoredOp>, StoreError>;
    fn count(&self, ds: &str) -> Result<u64, StoreError>;
}

pub fn validated_root_hex(ops: &[StoredOp]) -> String {
    let m: Vec<MerkleOp> = ops.iter().map(StoredOp::merkle).collect();
    root_hex(&m)
}

pub struct MemoryStore {
    // (ds, op_id) → op
    ops: BTreeMap<(String, [u8; 32]), StoredOp>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            ops: BTreeMap::new(),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OpStore for MemoryStore {
    fn insert(&mut self, ds: &str, op: StoredOp) -> Result<bool, StoreError> {
        let key = (ds.to_string(), op.op_id);
        if self.ops.contains_key(&key) {
            return Ok(false);
        }
        self.ops.insert(key, op);
        Ok(true)
    }

    fn list(&self, ds: &str) -> Result<Vec<StoredOp>, StoreError> {
        Ok(self
            .ops
            .iter()
            .filter(|((d, _), _)| d == ds)
            .map(|(_, o)| o.clone())
            .collect())
    }

    fn count(&self, ds: &str) -> Result<u64, StoreError> {
        Ok(self.ops.keys().filter(|(d, _)| d == ds).count() as u64)
    }
}

pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS ops (
                ds TEXT NOT NULL,
                op_id BLOB NOT NULL,
                author BLOB NOT NULL,
                physical_ms INTEGER NOT NULL,
                logical INTEGER NOT NULL,
                body BLOB NOT NULL,
                PRIMARY KEY (ds, op_id)
            );",
        )?;
        Ok(Self { conn })
    }
}

impl OpStore for SqliteStore {
    fn insert(&mut self, ds: &str, op: StoredOp) -> Result<bool, StoreError> {
        let exists: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM ops WHERE ds = ?1 AND op_id = ?2",
                params![ds, op.op_id.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO ops (ds, op_id, author, physical_ms, logical, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ds,
                op.op_id.as_slice(),
                op.author.as_slice(),
                op.physical_ms as i64,
                op.logical as i64,
                op.body
            ],
        )?;
        Ok(true)
    }

    fn list(&self, ds: &str) -> Result<Vec<StoredOp>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT op_id, author, physical_ms, logical, body FROM ops WHERE ds = ?1")?;
        let rows = stmt.query_map(params![ds], |r| {
            let id: Vec<u8> = r.get(0)?;
            let author: Vec<u8> = r.get(1)?;
            Ok(StoredOp {
                op_id: id.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?,
                author: author
                    .try_into()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                physical_ms: r.get::<_, i64>(2)? as u64,
                logical: r.get::<_, i64>(3)? as u16,
                body: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn count(&self, ds: &str) -> Result<u64, StoreError> {
        let n: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM ops WHERE ds = ?1", params![ds], |r| {
                    r.get(0)
                })?;
        Ok(n as u64)
    }
}
