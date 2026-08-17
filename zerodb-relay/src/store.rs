//! Durable validated oplog. The relay hashes this set as `validated_root`.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;
use zerodb_core::auth::KnownGrant;
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
    fn upsert_grant(&mut self, grant: KnownGrant) -> Result<(), StoreError>;
    fn grants(&self, ds: &str) -> Result<Vec<KnownGrant>, StoreError>;
    fn revoke_grant(&mut self, ds: &str, id: &[u8; 32]) -> Result<bool, StoreError>;
}

pub fn validated_root_hex(ops: &[StoredOp]) -> String {
    let m: Vec<MerkleOp> = ops.iter().map(StoredOp::merkle).collect();
    root_hex(&m)
}

pub struct MemoryStore {
    // (ds, op_id) → op
    ops: BTreeMap<(String, [u8; 32]), StoredOp>,
    grants: BTreeMap<(String, [u8; 32]), KnownGrant>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            ops: BTreeMap::new(),
            grants: BTreeMap::new(),
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

    fn upsert_grant(&mut self, grant: KnownGrant) -> Result<(), StoreError> {
        self.grants.insert((hex::encode(grant.ds), grant.id), grant);
        Ok(())
    }

    fn grants(&self, ds: &str) -> Result<Vec<KnownGrant>, StoreError> {
        Ok(self
            .grants
            .iter()
            .filter(|((d, _), _)| d == ds)
            .map(|(_, g)| g.clone())
            .collect())
    }

    fn revoke_grant(&mut self, ds: &str, id: &[u8; 32]) -> Result<bool, StoreError> {
        if let Some(grant) = self.grants.get_mut(&(ds.to_string(), *id)) {
            grant.revoked = true;
            return Ok(true);
        }
        Ok(false)
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
            );
            CREATE TABLE IF NOT EXISTS membership_grants (
                ds TEXT NOT NULL,
                grant_id BLOB NOT NULL,
                subject BLOB NOT NULL,
                scopes TEXT NOT NULL,
                expiry INTEGER,
                revoked INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (ds, grant_id)
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

    fn upsert_grant(&mut self, grant: KnownGrant) -> Result<(), StoreError> {
        let scopes =
            serde_json::to_string(&grant.scopes).map_err(|e| StoreError::Io(e.to_string()))?;
        self.conn.execute(
            "INSERT INTO membership_grants (ds, grant_id, subject, scopes, expiry, revoked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(ds, grant_id) DO UPDATE SET subject=excluded.subject,
             scopes=excluded.scopes, expiry=excluded.expiry, revoked=excluded.revoked",
            params![
                hex::encode(grant.ds),
                grant.id.as_slice(),
                grant.subject.as_slice(),
                scopes,
                grant.expiry.map(|v| v as i64),
                grant.revoked as i64
            ],
        )?;
        Ok(())
    }

    fn grants(&self, ds: &str) -> Result<Vec<KnownGrant>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT grant_id, subject, scopes, expiry, revoked FROM membership_grants WHERE ds=?1",
        )?;
        let ds_bytes: [u8; 32] = hex::decode(ds)
            .map_err(|e| StoreError::Io(e.to_string()))?
            .try_into()
            .map_err(|_| StoreError::Io("datastore id length".into()))?;
        let rows = stmt.query_map(params![ds], |r| {
            let id: Vec<u8> = r.get(0)?;
            let subject: Vec<u8> = r.get(1)?;
            let scopes_json: String = r.get(2)?;
            let scopes = serde_json::from_str(&scopes_json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(KnownGrant {
                id: id.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?,
                ds: ds_bytes,
                subject: subject
                    .try_into()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                scopes,
                expiry: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                revoked: r.get::<_, i64>(4)? != 0,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn revoke_grant(&mut self, ds: &str, id: &[u8; 32]) -> Result<bool, StoreError> {
        Ok(self.conn.execute(
            "UPDATE membership_grants SET revoked=1 WHERE ds=?1 AND grant_id=?2",
            params![ds, id.as_slice()],
        )? != 0)
    }
}
