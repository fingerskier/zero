//! SQLite implementation of [`StoreBackend`]. All SQL lives here.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::backend::{BackendTxn, EdgeRow, OpRecord, OpScanRow, PropOpRow, StoreBackend};
use crate::{KIND_CREATE_NODE, KIND_SET_PROPERTY, KIND_TOMBSTONE, StoreError};

fn sql_err(e: rusqlite::Error) -> StoreError {
    StoreError::Sqlite(e.to_string())
}

pub struct SqliteBackend {
    conn: Connection,
    /// Armed E4 crash-injection point (test-only; a plain Option check at a
    /// handful of named commit-pipeline stages — zero cost when unarmed).
    failpoint: std::cell::RefCell<Option<String>>,
}

impl SqliteBackend {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(sql_err)?;
        migrate(&conn)?;
        Ok(Self {
            conn,
            failpoint: std::cell::RefCell::new(None),
        })
    }

    /// Test hook (E4 crash matrix): arm a named failpoint, or clear with `None`.
    ///
    /// When the armed point is reached inside a `with_txn` commit pipeline the
    /// backend returns an injected error at exactly that point; the open
    /// `rusqlite::Transaction` is dropped, which is a ROLLBACK — the same
    /// durable outcome as a process death before `COMMIT`. Named points
    /// (mapping to WAL.md §3 layer-1 crash points documented in
    /// `tests/e4_crash_matrix.rs`):
    /// `before-txn`, `after-op-insert`, `before-hlc-persist`,
    /// `after-hlc-persist`, `before-commit`.
    #[doc(hidden)]
    pub fn set_failpoint(&mut self, name: Option<&str>) {
        *self.failpoint.borrow_mut() = name.map(str::to_owned);
    }
}

fn failpoint_check(
    armed: &std::cell::RefCell<Option<String>>,
    point: &str,
) -> Result<(), StoreError> {
    if armed.borrow().as_deref() == Some(point) {
        return Err(StoreError::Invalid(format!("failpoint: {point}")));
    }
    Ok(())
}

fn migrate(conn: &Connection) -> Result<(), StoreError> {
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
          label TEXT NOT NULL,
          deleted INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS props (
          entity TEXT NOT NULL,
          path TEXT NOT NULL,
          crdt TEXT NOT NULL,
          value_json TEXT NOT NULL,
          PRIMARY KEY (entity, path)
        );
        CREATE TABLE IF NOT EXISTS edges (
          id TEXT PRIMARY KEY,
          label TEXT NOT NULL,
          src TEXT NOT NULL,
          dst TEXT NOT NULL,
          deleted INTEGER NOT NULL DEFAULT 0
        );
        ",
    )
    .map_err(sql_err)?;
    // migrate old nodes without deleted column
    let has_deleted: bool = conn
        .prepare("PRAGMA table_info(nodes)")
        .map_err(sql_err)?
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(sql_err)?
        .filter_map(|x| x.ok())
        .any(|c| c == "deleted");
    if !has_deleted {
        let _ =
            conn.execute_batch("ALTER TABLE nodes ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;");
    }
    Ok(())
}

struct SqliteTxn<'a> {
    conn: &'a Connection,
    failpoint: &'a std::cell::RefCell<Option<String>>,
}

impl BackendTxn for SqliteBackend {
    fn meta_get(&self, k: &str) -> Result<Option<Vec<u8>>, StoreError> {
        meta_get(&self.conn, k)
    }
    fn meta_set(&self, k: &str, v: &[u8]) -> Result<(), StoreError> {
        meta_set(&self.conn, k, v)
    }
    fn op_insert(&self, rec: &OpRecord) -> Result<(), StoreError> {
        op_insert(&self.conn, rec)
    }
    fn op_exists(&self, id: &[u8; 32]) -> Result<bool, StoreError> {
        op_exists(&self.conn, id)
    }
    fn op_count(&self) -> Result<u64, StoreError> {
        op_count(&self.conn)
    }
    fn op_ids(&self) -> Result<Vec<Vec<u8>>, StoreError> {
        op_ids(&self.conn)
    }
    fn op_wires(&self) -> Result<Vec<String>, StoreError> {
        op_wires(&self.conn)
    }
    fn op_wire_by_id(&self, id: &[u8; 32]) -> Result<Option<String>, StoreError> {
        op_wire_by_id(&self.conn, id)
    }
    fn op_scan(&self) -> Result<Vec<OpScanRow>, StoreError> {
        op_scan(&self.conn)
    }
    fn op_scan_node_kinds(&self) -> Result<Vec<(u64, String)>, StoreError> {
        op_scan_node_kinds(&self.conn)
    }
    fn op_scan_props(&self) -> Result<Vec<PropOpRow>, StoreError> {
        op_scan_props(&self.conn)
    }
    fn op_max_hlc(&self) -> Result<Option<(u64, u16)>, StoreError> {
        op_max_hlc(&self.conn)
    }
    fn node_upsert(&self, id: &str, label: &str, deleted: bool) -> Result<(), StoreError> {
        node_upsert(&self.conn, id, label, deleted)
    }
    fn node_insert_ignore(&self, id: &str, label: &str) -> Result<(), StoreError> {
        node_insert_ignore(&self.conn, id, label)
    }
    fn node_delete(&self, id: &str) -> Result<(), StoreError> {
        node_delete(&self.conn, id)
    }
    fn node_deleted_state(&self, id: &str) -> Result<Option<bool>, StoreError> {
        node_deleted_state(&self.conn, id)
    }
    fn node_list(&self) -> Result<Vec<(String, String, bool)>, StoreError> {
        node_list(&self.conn)
    }
    fn node_count(&self) -> Result<u64, StoreError> {
        node_count(&self.conn)
    }
    fn prop_upsert(
        &self,
        entity: &str,
        path: &str,
        crdt: &str,
        value_json: &str,
    ) -> Result<(), StoreError> {
        prop_upsert(&self.conn, entity, path, crdt, value_json)
    }
    fn prop_delete(&self, entity: &str, path: &str) -> Result<(), StoreError> {
        prop_delete(&self.conn, entity, path)
    }
    fn prop_get(&self, entity: &str, path: &str) -> Result<Option<String>, StoreError> {
        prop_get(&self.conn, entity, path)
    }
    fn prop_list(&self, entity: &str) -> Result<Vec<(String, String)>, StoreError> {
        prop_list(&self.conn, entity)
    }
    fn edge_upsert(
        &self,
        id: &str,
        label: &str,
        src: &str,
        dst: &str,
        deleted: bool,
    ) -> Result<(), StoreError> {
        edge_upsert(&self.conn, id, label, src, dst, deleted)
    }
    fn edge_delete(&self, id: &str) -> Result<(), StoreError> {
        edge_delete(&self.conn, id)
    }
    fn edge_list(&self) -> Result<Vec<EdgeRow>, StoreError> {
        edge_list(&self.conn)
    }
    fn edge_list_visible(&self) -> Result<Vec<(String, String, String, String)>, StoreError> {
        edge_list_visible(&self.conn)
    }
    fn wipe_materialized(&self) -> Result<(), StoreError> {
        wipe_materialized(&self.conn)
    }
}

impl BackendTxn for SqliteTxn<'_> {
    fn meta_get(&self, k: &str) -> Result<Option<Vec<u8>>, StoreError> {
        meta_get(self.conn, k)
    }
    fn meta_set(&self, k: &str, v: &[u8]) -> Result<(), StoreError> {
        // E4 failpoints around HLC durability: hlc_p is the first HLC meta
        // write in every commit path (materialization already staged);
        // hlc_l is the last (HLC fully staged, commit still pending).
        if k == "hlc_p" {
            failpoint_check(self.failpoint, "before-hlc-persist")?;
        }
        meta_set(self.conn, k, v)?;
        if k == "hlc_l" {
            failpoint_check(self.failpoint, "after-hlc-persist")?;
        }
        Ok(())
    }
    fn op_insert(&self, rec: &OpRecord) -> Result<(), StoreError> {
        op_insert(self.conn, rec)?;
        // E4 failpoint: op appended in-transaction, nothing else staged yet.
        failpoint_check(self.failpoint, "after-op-insert")
    }
    fn op_exists(&self, id: &[u8; 32]) -> Result<bool, StoreError> {
        op_exists(self.conn, id)
    }
    fn op_count(&self) -> Result<u64, StoreError> {
        op_count(self.conn)
    }
    fn op_ids(&self) -> Result<Vec<Vec<u8>>, StoreError> {
        op_ids(self.conn)
    }
    fn op_wires(&self) -> Result<Vec<String>, StoreError> {
        op_wires(self.conn)
    }
    fn op_wire_by_id(&self, id: &[u8; 32]) -> Result<Option<String>, StoreError> {
        op_wire_by_id(self.conn, id)
    }
    fn op_scan(&self) -> Result<Vec<OpScanRow>, StoreError> {
        op_scan(self.conn)
    }
    fn op_scan_node_kinds(&self) -> Result<Vec<(u64, String)>, StoreError> {
        op_scan_node_kinds(self.conn)
    }
    fn op_scan_props(&self) -> Result<Vec<PropOpRow>, StoreError> {
        op_scan_props(self.conn)
    }
    fn op_max_hlc(&self) -> Result<Option<(u64, u16)>, StoreError> {
        op_max_hlc(self.conn)
    }
    fn node_upsert(&self, id: &str, label: &str, deleted: bool) -> Result<(), StoreError> {
        node_upsert(self.conn, id, label, deleted)
    }
    fn node_insert_ignore(&self, id: &str, label: &str) -> Result<(), StoreError> {
        node_insert_ignore(self.conn, id, label)
    }
    fn node_delete(&self, id: &str) -> Result<(), StoreError> {
        node_delete(self.conn, id)
    }
    fn node_deleted_state(&self, id: &str) -> Result<Option<bool>, StoreError> {
        node_deleted_state(self.conn, id)
    }
    fn node_list(&self) -> Result<Vec<(String, String, bool)>, StoreError> {
        node_list(self.conn)
    }
    fn node_count(&self) -> Result<u64, StoreError> {
        node_count(self.conn)
    }
    fn prop_upsert(
        &self,
        entity: &str,
        path: &str,
        crdt: &str,
        value_json: &str,
    ) -> Result<(), StoreError> {
        prop_upsert(self.conn, entity, path, crdt, value_json)
    }
    fn prop_delete(&self, entity: &str, path: &str) -> Result<(), StoreError> {
        prop_delete(self.conn, entity, path)
    }
    fn prop_get(&self, entity: &str, path: &str) -> Result<Option<String>, StoreError> {
        prop_get(self.conn, entity, path)
    }
    fn prop_list(&self, entity: &str) -> Result<Vec<(String, String)>, StoreError> {
        prop_list(self.conn, entity)
    }
    fn edge_upsert(
        &self,
        id: &str,
        label: &str,
        src: &str,
        dst: &str,
        deleted: bool,
    ) -> Result<(), StoreError> {
        edge_upsert(self.conn, id, label, src, dst, deleted)
    }
    fn edge_delete(&self, id: &str) -> Result<(), StoreError> {
        edge_delete(self.conn, id)
    }
    fn edge_list(&self) -> Result<Vec<EdgeRow>, StoreError> {
        edge_list(self.conn)
    }
    fn edge_list_visible(&self) -> Result<Vec<(String, String, String, String)>, StoreError> {
        edge_list_visible(self.conn)
    }
    fn wipe_materialized(&self) -> Result<(), StoreError> {
        wipe_materialized(self.conn)
    }
}

impl StoreBackend for SqliteBackend {
    fn with_txn(
        &mut self,
        f: &mut dyn FnMut(&dyn BackendTxn) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        failpoint_check(&self.failpoint, "before-txn")?;
        let tx = self.conn.transaction().map_err(sql_err)?;
        f(&SqliteTxn {
            conn: &tx,
            failpoint: &self.failpoint,
        })?;
        // E4 failpoint: full pipeline staged; dropping `tx` here rolls back,
        // matching a process death at the instant before COMMIT.
        failpoint_check(&self.failpoint, "before-commit")?;
        tx.commit().map_err(sql_err)?;
        Ok(())
    }
}

fn meta_get(conn: &Connection, k: &str) -> Result<Option<Vec<u8>>, StoreError> {
    conn.query_row("SELECT v FROM meta WHERE k = ?1", params![k], |r| r.get(0))
        .optional()
        .map_err(sql_err)
}

fn meta_set(conn: &Connection, k: &str, v: &[u8]) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![k, v],
    )
    .map_err(sql_err)?;
    Ok(())
}

fn op_insert(conn: &Connection, rec: &OpRecord) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO ops (id, author, author_pk, physical_ms, logical, kind, body_json, sig, wire_json)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            rec.id.as_slice(),
            rec.author.as_slice(),
            rec.author_pk.as_slice(),
            rec.physical_ms as i64,
            rec.logical as i64,
            rec.kind as i64,
            rec.body_json,
            rec.sig.as_slice(),
            rec.wire_json,
        ],
    )
    .map_err(sql_err)?;
    Ok(())
}

fn op_exists(conn: &Connection, id: &[u8; 32]) -> Result<bool, StoreError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM ops WHERE id = ?1",
            params![id.as_slice()],
            |_| Ok(true),
        )
        .optional()
        .map_err(sql_err)?
        .unwrap_or(false))
}

fn op_count(conn: &Connection) -> Result<u64, StoreError> {
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM ops", [], |r| r.get(0))
        .map_err(sql_err)?;
    Ok(n as u64)
}

fn op_ids(conn: &Connection) -> Result<Vec<Vec<u8>>, StoreError> {
    let mut stmt = conn.prepare("SELECT id FROM ops").map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(sql_err)?);
    }
    Ok(out)
}

fn op_wires(conn: &Connection) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT wire_json FROM ops ORDER BY physical_ms, logical, id")
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(sql_err)?);
    }
    Ok(out)
}

fn op_wire_by_id(conn: &Connection, id: &[u8; 32]) -> Result<Option<String>, StoreError> {
    conn.query_row(
        "SELECT wire_json FROM ops WHERE id = ?1",
        params![id.as_slice()],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql_err)
}

fn op_scan(conn: &Connection) -> Result<Vec<OpScanRow>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, body_json, physical_ms, logical FROM ops
             ORDER BY physical_ms, logical, id",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(OpScanRow {
                kind: r.get::<_, i64>(0)? as u64,
                body_json: r.get::<_, String>(1)?,
                physical_ms: r.get::<_, i64>(2)? as u64,
                logical: r.get::<_, i64>(3)? as u16,
            })
        })
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(sql_err)?);
    }
    Ok(out)
}

fn op_scan_node_kinds(conn: &Connection) -> Result<Vec<(u64, String)>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, body_json FROM ops
             WHERE kind IN (?1, ?2)
             ORDER BY physical_ms, logical, id",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(
            params![KIND_CREATE_NODE as i64, KIND_TOMBSTONE as i64],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?)),
        )
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(sql_err)?);
    }
    Ok(out)
}

fn op_scan_props(conn: &Connection) -> Result<Vec<PropOpRow>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, author, physical_ms, logical, body_json FROM ops
             WHERE kind = ?1 ORDER BY physical_ms, logical, id",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(params![KIND_SET_PROPERTY as i64], |r| {
            Ok(PropOpRow {
                id: r.get::<_, Vec<u8>>(0)?,
                author: r.get::<_, Vec<u8>>(1)?,
                physical_ms: r.get::<_, i64>(2)? as u64,
                logical: r.get::<_, i64>(3)? as u16,
                body_json: r.get::<_, String>(4)?,
            })
        })
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(sql_err)?);
    }
    Ok(out)
}

fn op_max_hlc(conn: &Connection) -> Result<Option<(u64, u16)>, StoreError> {
    conn.query_row(
        "SELECT physical_ms, logical FROM ops
         ORDER BY physical_ms DESC, logical DESC LIMIT 1",
        [],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u16)),
    )
    .optional()
    .map_err(sql_err)
}

fn node_upsert(conn: &Connection, id: &str, label: &str, deleted: bool) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO nodes (id, label, deleted) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET label = excluded.label, deleted = excluded.deleted",
        params![id, label, if deleted { 1i64 } else { 0 }],
    )
    .map_err(sql_err)?;
    Ok(())
}

fn node_insert_ignore(conn: &Connection, id: &str, label: &str) -> Result<(), StoreError> {
    conn.execute(
        "INSERT OR IGNORE INTO nodes (id, label, deleted) VALUES (?1, ?2, 0)",
        params![id, label],
    )
    .map_err(sql_err)?;
    Ok(())
}

fn node_delete(conn: &Connection, id: &str) -> Result<(), StoreError> {
    conn.execute("DELETE FROM nodes WHERE id = ?1", params![id])
        .map_err(sql_err)?;
    Ok(())
}

fn node_deleted_state(conn: &Connection, id: &str) -> Result<Option<bool>, StoreError> {
    let d: Option<i64> = conn
        .query_row(
            "SELECT deleted FROM nodes WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql_err)?;
    Ok(d.map(|value| value != 0))
}

fn node_list(conn: &Connection) -> Result<Vec<(String, String, bool)>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT id, label, deleted FROM nodes ORDER BY id")
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? != 0,
            ))
        })
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(sql_err)?);
    }
    Ok(out)
}

fn node_count(conn: &Connection) -> Result<u64, StoreError> {
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))
        .map_err(sql_err)?;
    Ok(n as u64)
}

fn prop_upsert(
    conn: &Connection,
    entity: &str,
    path: &str,
    crdt: &str,
    value_json: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO props (entity, path, crdt, value_json) VALUES (?1,?2,?3,?4)
         ON CONFLICT(entity, path) DO UPDATE SET crdt=excluded.crdt, value_json=excluded.value_json",
        params![entity, path, crdt, value_json],
    )
    .map_err(sql_err)?;
    Ok(())
}

fn prop_delete(conn: &Connection, entity: &str, path: &str) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM props WHERE entity = ?1 AND path = ?2",
        params![entity, path],
    )
    .map_err(sql_err)?;
    Ok(())
}

fn prop_get(conn: &Connection, entity: &str, path: &str) -> Result<Option<String>, StoreError> {
    conn.query_row(
        "SELECT value_json FROM props WHERE entity = ?1 AND path = ?2",
        params![entity, path],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql_err)
}

fn prop_list(conn: &Connection, entity: &str) -> Result<Vec<(String, String)>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT path, value_json FROM props WHERE entity = ?1 ORDER BY path")
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(params![entity], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(sql_err)?);
    }
    Ok(out)
}

fn edge_upsert(
    conn: &Connection,
    id: &str,
    label: &str,
    src: &str,
    dst: &str,
    deleted: bool,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO edges (id, label, src, dst, deleted) VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(id) DO UPDATE SET label=excluded.label, src=excluded.src,
           dst=excluded.dst, deleted=excluded.deleted",
        params![id, label, src, dst, if deleted { 1i64 } else { 0 }],
    )
    .map_err(sql_err)?;
    Ok(())
}

fn edge_delete(conn: &Connection, id: &str) -> Result<(), StoreError> {
    conn.execute("DELETE FROM edges WHERE id = ?1", params![id])
        .map_err(sql_err)?;
    Ok(())
}

fn edge_list(conn: &Connection) -> Result<Vec<EdgeRow>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT id, label, src, dst, deleted FROM edges ORDER BY id")
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(EdgeRow {
                id: r.get::<_, String>(0)?,
                label: r.get::<_, String>(1)?,
                src: r.get::<_, String>(2)?,
                dst: r.get::<_, String>(3)?,
                deleted: r.get::<_, i64>(4)? != 0,
            })
        })
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(sql_err)?);
    }
    Ok(out)
}

fn edge_list_visible(
    conn: &Connection,
) -> Result<Vec<(String, String, String, String)>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT e.id, e.label, e.src, e.dst FROM edges e
             JOIN nodes s ON s.id = e.src AND s.deleted = 0
             JOIN nodes d ON d.id = e.dst AND d.deleted = 0
             WHERE e.deleted = 0
             ORDER BY e.id",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(sql_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(sql_err)?);
    }
    Ok(out)
}

fn wipe_materialized(conn: &Connection) -> Result<(), StoreError> {
    conn.execute("DELETE FROM props", []).map_err(sql_err)?;
    conn.execute("DELETE FROM nodes", []).map_err(sql_err)?;
    conn.execute("DELETE FROM edges", []).map_err(sql_err)?;
    Ok(())
}
