//! Local durable store for the M1 MVP slice.
//!
//! SQLite: oplog + materialized props + nodes. Each accepted op commits
//! append + rematerialize + HLC in one transaction. Property state is rebuilt
//! from the full op set for that (entity, path) so multi-peer CRDT merges stay
//! consistent with the KERNEL replica rules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use zerodb_core::cbor::Cbor;
use zerodb_core::kernel::{
    Flag, GCounter, KernelOp, Lww, OrSet, Payload, PnCounter, Replica, Value,
};
use zerodb_core::op::{OpEnvelope, OpTs};
use zerodb_core::query::parse as parse_query;
use zerodb_core::queryeval::{self, GEdge, GNode, Graph, QValue};
use zerodb_core::sign::{DOMAIN_OP_SIG, verify_op};

const KIND_CREATE_NODE: u64 = 1;
const KIND_CREATE_EDGE: u64 = 2;
const KIND_SET_PROPERTY: u64 = 3;
const KIND_TOMBSTONE: u64 = 4;
const MAX_CLOCK_DRIFT_MS: u64 = 60_000;
/// Experimental local SQLite layout version (KERNEL `storage_format_version`).
const STORAGE_FORMAT_VERSION: u64 = 1;

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
    /// Optional group id (16-byte hex). Present on atomic_group members (M1-e4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grp: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectReport {
    pub path: String,
    pub datastore_id: String,
    pub peer: String,
    pub ops: u64,
    pub nodes: Vec<InspectNode>,
    #[serde(default)]
    pub edges: Vec<InspectEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectNode {
    pub id: String,
    pub label: String,
    pub deleted: bool,
    pub props: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectEdge {
    pub id: String,
    pub label: String,
    pub src: String,
    pub dst: String,
    pub deleted: bool,
    /// False when an endpoint is missing or tombstoned (H3 derived visibility).
    pub visible: bool,
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

struct ValidatedWire {
    id: [u8; 32],
    author: [u8; 32],
    author_pk: [u8; 32],
    sig: [u8; 64],
}

impl LocalStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        migrate(&conn)?;
        let seed: [u8; 32] = meta_get(&conn, "seed")?
            .ok_or_else(|| {
                StoreError::Invalid("database not initialized — run `zerodb init`".into())
            })?
            .try_into()
            .map_err(|_| StoreError::Invalid("seed length".into()))?;
        let signing = SigningKey::from_bytes(&seed);
        let author_pk = signing.verifying_key().to_bytes();
        let author = *blake3::hash(&author_pk).as_bytes();
        let ds: [u8; 32] = meta_get(&conn, "ds")?
            .ok_or_else(|| StoreError::Invalid("missing ds".into()))?
            .try_into()
            .map_err(|_| StoreError::Invalid("ds length".into()))?;
        ensure_storage_format_version(&conn)?;
        let (hlc_p, hlc_l) = recover_hlc_from_oplog(&conn)?;
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
        migrate(&conn)?;
        let mut seed = [0u8; 32];
        getrandom_fill(&mut seed);
        let signing = SigningKey::from_bytes(&seed);
        let author_pk = signing.verifying_key().to_bytes();
        let author: [u8; 32] = *blake3::hash(&author_pk).as_bytes();
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
        meta_set_u64(&conn, "storage_format_version", STORAGE_FORMAT_VERSION)?;
        drop(conn);
        Self::open(path)
    }

    pub fn datastore_id_hex(&self) -> String {
        hex::encode(self.ds)
    }
    pub fn author_hex(&self) -> String {
        hex::encode(self.author)
    }

    fn next_local_ts(&self) -> Result<OpTs, StoreError> {
        let wall = now_ms();
        let p = wall.max(self.hlc_p);
        let (p, l) = if p == self.hlc_p {
            match self.hlc_l.checked_add(1) {
                Some(l) => (p, l),
                None => (
                    p.checked_add(1)
                        .ok_or_else(|| StoreError::Invalid("HLC physical time overflow".into()))?,
                    0,
                ),
            }
        } else {
            (p, 0)
        };
        Ok(OpTs {
            physical_ms: p,
            logical: l,
        })
    }

    pub fn create_node(&mut self, label: &str) -> Result<String, StoreError> {
        let node = Uuid::now_v7();
        let node_bytes = *node.as_bytes();
        let node_hex = hex::encode(node_bytes);
        let body = Cbor::Map(vec![
            ("label".into(), Cbor::Text(label.into())),
            ("node".into(), Cbor::Bytes(node_bytes.to_vec())),
        ]);
        let body_json = serde_json::json!({ "label": label, "node": node_hex });
        self.commit_local(KIND_CREATE_NODE, body, body_json, |tx, _| {
            tx.execute(
                "INSERT INTO nodes (id, label, deleted) VALUES (?1, ?2, 0)
                 ON CONFLICT(id) DO UPDATE SET label = excluded.label",
                params![node_hex, label],
            )?;
            Ok(())
        })?;
        Ok(node_hex)
    }

    pub fn delete_node(&mut self, node_hex: &str) -> Result<String, StoreError> {
        let node = decode_node(node_hex)?;
        let body = Cbor::Map(vec![
            ("node".into(), Cbor::Bytes(node)),
            ("tombstone".into(), Cbor::Bool(true)),
        ]);
        let body_json = serde_json::json!({ "node": node_hex, "tombstone": true });
        self.commit_local(KIND_TOMBSTONE, body, body_json, |tx, _| {
            tx.execute(
                "UPDATE nodes SET deleted = 1 WHERE id = ?1",
                params![node_hex],
            )?;
            // H3 derived visibility: do not emit cascade edge tombstones.
            Ok(())
        })
    }

    /// Create an edge. Visibility is derived: hidden if either endpoint is
    /// missing or deleted (H3) — no cascade ops.
    pub fn create_edge(
        &mut self,
        label: &str,
        src_hex: &str,
        dst_hex: &str,
    ) -> Result<String, StoreError> {
        let src = decode_node(src_hex)?;
        let dst = decode_node(dst_hex)?;
        let edge = Uuid::now_v7();
        let edge_bytes = *edge.as_bytes();
        let edge_hex = hex::encode(edge_bytes);
        let body = Cbor::Map(vec![
            ("edge".into(), Cbor::Bytes(edge_bytes.to_vec())),
            ("label".into(), Cbor::Text(label.into())),
            ("src".into(), Cbor::Bytes(src)),
            ("dst".into(), Cbor::Bytes(dst)),
        ]);
        let body_json = serde_json::json!({
            "edge": edge_hex,
            "label": label,
            "src": src_hex,
            "dst": dst_hex,
        });
        self.commit_local(KIND_CREATE_EDGE, body, body_json, |tx, _| {
            tx.execute(
                "INSERT INTO edges (id, label, src, dst, deleted) VALUES (?1,?2,?3,?4,0)
                 ON CONFLICT(id) DO UPDATE SET label=excluded.label, src=excluded.src, dst=excluded.dst",
                params![edge_hex, label, src_hex, dst_hex],
            )?;
            Ok(())
        })?;
        Ok(edge_hex)
    }

    /// Visible edges only (both endpoints live and not deleted; edge not deleted).
    pub fn list_edges_visible(&self) -> Result<Vec<(String, String, String, String)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.label, e.src, e.dst FROM edges e
             JOIN nodes s ON s.id = e.src AND s.deleted = 0
             JOIN nodes d ON d.id = e.dst AND d.deleted = 0
             WHERE e.deleted = 0
             ORDER BY e.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Apply a simplified JSON schema pin: `{ "nodes": { "Todo": { "props": { "title": "lww" }}} }`.
    /// Stores in meta; subsequent local mutations must match the pin for known labels.
    pub fn apply_schema_json(&mut self, schema_json: &str) -> Result<(), StoreError> {
        let v: serde_json::Value =
            serde_json::from_str(schema_json).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let nodes = v
            .get("nodes")
            .and_then(|n| n.as_object())
            .ok_or_else(|| StoreError::Invalid("schema.nodes required".into()))?;
        for (label, entity) in nodes {
            let props = entity
                .get("props")
                .and_then(|p| p.as_object())
                .ok_or_else(|| StoreError::Invalid(format!("schema.nodes.{label}.props")))?;
            for (path, crdt) in props {
                let c = crdt
                    .as_str()
                    .ok_or_else(|| StoreError::Invalid("prop crdt must be string".into()))?;
                if !matches!(c, "lww" | "gcounter" | "pncounter" | "orset" | "flag") {
                    return Err(StoreError::Invalid(format!("unknown crdt {c}")));
                }
                let _ = path;
            }
        }
        meta_set(&self.conn, "schema_json", schema_json.as_bytes())?;
        Ok(())
    }

    pub fn schema_json(&self) -> Result<Option<String>, StoreError> {
        Ok(meta_get(&self.conn, "schema_json")?.map(|b| String::from_utf8_lossy(&b).into_owned()))
    }

    /// O3 minimal query over visible graph materialization.
    pub fn query(&self, q: &str) -> Result<serde_json::Value, StoreError> {
        let ast = parse_query(q).map_err(|e| StoreError::Invalid(format!("query parse: {e}")))?;
        let graph = self.to_query_graph()?;
        let rows = queryeval::eval(&ast, &graph, &BTreeMap::new())
            .map_err(|e| StoreError::Invalid(format!("query eval: {e}")))?;
        let cols: Vec<String> = ast
            .items
            .iter()
            .map(|item| match &item.path {
                Some(p) => format!("{}.{}", item.var, p),
                None => item.var.clone(),
            })
            .collect();
        let json_rows: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|row| {
                let mut obj = serde_json::Map::new();
                for (i, v) in row.into_iter().enumerate() {
                    let key = cols.get(i).cloned().unwrap_or_else(|| format!("c{i}"));
                    obj.insert(key, qvalue_to_json(&v));
                }
                serde_json::Value::Object(obj)
            })
            .collect();
        Ok(serde_json::Value::Array(json_rows))
    }

    fn to_query_graph(&self) -> Result<Graph, StoreError> {
        let mut nodes = Vec::new();
        for (id, label, deleted) in self.list_nodes()? {
            if deleted {
                continue;
            }
            let mut props = BTreeMap::new();
            let mut stmt = self
                .conn
                .prepare("SELECT path, value_json FROM props WHERE entity = ?1 ORDER BY path")?;
            let rows = stmt.query_map(params![id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (p, vj) = row?;
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&vj) {
                    props.insert(p, json_to_qvalue(&v));
                }
            }
            nodes.push(GNode { id, label, props });
        }
        let mut edges = Vec::new();
        for (id, label, src, dst) in self.list_edges_visible()? {
            edges.push(GEdge {
                id,
                label,
                src,
                dst,
                props: BTreeMap::new(),
            });
        }
        Ok(Graph { nodes, edges })
    }

    pub fn set_lww(&mut self, node: &str, path: &str, value: &str) -> Result<String, StoreError> {
        self.mutate_prop(node, path, "lww", serde_json::json!({ "value": value }))
    }

    pub fn counter_inc(&mut self, node: &str, path: &str, n: u64) -> Result<String, StoreError> {
        if n == 0 {
            return Err(StoreError::Invalid("n must be > 0".into()));
        }
        self.mutate_prop(
            node,
            path,
            "pncounter",
            serde_json::json!({ "op": "inc", "n": n }),
        )
    }

    pub fn counter_dec(&mut self, node: &str, path: &str, n: u64) -> Result<String, StoreError> {
        if n == 0 {
            return Err(StoreError::Invalid("n must be > 0".into()));
        }
        self.mutate_prop(
            node,
            path,
            "pncounter",
            serde_json::json!({ "op": "dec", "n": n }),
        )
    }

    pub fn gcounter_inc(&mut self, node: &str, path: &str, n: u64) -> Result<String, StoreError> {
        if n == 0 {
            return Err(StoreError::Invalid("n must be > 0".into()));
        }
        self.mutate_prop(
            node,
            path,
            "gcounter",
            serde_json::json!({ "op": "inc", "n": n }),
        )
    }

    pub fn set_add(&mut self, node: &str, path: &str, value: &str) -> Result<String, StoreError> {
        self.mutate_prop(
            node,
            path,
            "orset",
            serde_json::json!({ "op": "add", "value": value }),
        )
    }

    pub fn set_remove(
        &mut self,
        node: &str,
        path: &str,
        value: &str,
    ) -> Result<String, StoreError> {
        // Observe current dots for this element from rematerialized state
        let observed = self.orset_dots_for(node, path, value)?;
        self.mutate_prop(
            node,
            path,
            "orset",
            serde_json::json!({ "op": "remove", "value": value, "observed": observed }),
        )
    }

    pub fn flag_enable(&mut self, node: &str, path: &str) -> Result<String, StoreError> {
        self.mutate_prop(node, path, "flag", serde_json::json!({ "op": "enable" }))
    }

    pub fn flag_disable(&mut self, node: &str, path: &str) -> Result<String, StoreError> {
        let observed = self.flag_dots(node, path)?;
        self.mutate_prop(
            node,
            path,
            "flag",
            serde_json::json!({ "op": "disable", "observed": observed }),
        )
    }

    fn mutate_prop(
        &mut self,
        node_hex: &str,
        path: &str,
        crdt: &str,
        extra: serde_json::Value,
    ) -> Result<String, StoreError> {
        let _ = decode_node(node_hex)?;
        match self.node_deleted_state(node_hex)? {
            None => return Err(StoreError::NotFound(format!("node {node_hex}"))),
            Some(true) => return Err(StoreError::Invalid("node is deleted".into())),
            Some(false) => {}
        }
        self.check_schema_pin(node_hex, path, crdt)?;
        let mut body_json = extra;
        let obj = body_json
            .as_object_mut()
            .ok_or_else(|| StoreError::Invalid("body".into()))?;
        obj.insert("node".into(), serde_json::json!(node_hex));
        obj.insert("path".into(), serde_json::json!(path));
        obj.insert("crdt".into(), serde_json::json!(crdt));

        let body = json_to_cbor_body(&body_json)?;
        let node_s = node_hex.to_string();
        let path_s = path.to_string();
        self.commit_local(KIND_SET_PROPERTY, body, body_json, move |tx, _| {
            tx.execute(
                "INSERT OR IGNORE INTO nodes (id, label, deleted) VALUES (?1, ?2, 0)",
                params![node_s, "Node"],
            )?;
            rematerialize_prop(tx, &node_s, &path_s)?;
            Ok(())
        })
    }

    pub fn get_prop(
        &self,
        node_hex: &str,
        path: &str,
    ) -> Result<Option<serde_json::Value>, StoreError> {
        if self.node_deleted_state(node_hex)? != Some(false) {
            return Ok(None);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT value_json FROM props WHERE entity = ?1 AND path = ?2")?;
        let v: Option<String> = stmt
            .query_row(params![node_hex, path], |r| r.get(0))
            .optional()?;
        Ok(v.and_then(|j| serde_json::from_str(&j).ok()))
    }

    pub fn get_lww(&self, node_hex: &str, path: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .get_prop(node_hex, path)?
            .and_then(|v| v.as_str().map(|s| s.to_string())))
    }

    pub fn list_nodes(&self) -> Result<Vec<(String, String, bool)>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, label, deleted FROM nodes ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? != 0,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn is_deleted(&self, node_hex: &str) -> Result<bool, StoreError> {
        Ok(self.node_deleted_state(node_hex)?.unwrap_or(false))
    }

    fn node_deleted_state(&self, node_hex: &str) -> Result<Option<bool>, StoreError> {
        let d: Option<i64> = self
            .conn
            .query_row(
                "SELECT deleted FROM nodes WHERE id = ?1",
                params![node_hex],
                |r| r.get(0),
            )
            .optional()?;
        Ok(d.map(|value| value != 0))
    }

    pub fn inspect(&self, path: &Path) -> Result<InspectReport, StoreError> {
        let mut nodes = Vec::new();
        for (id, label, deleted) in self.list_nodes()? {
            let mut props = BTreeMap::new();
            if !deleted {
                let mut stmt = self.conn.prepare(
                    "SELECT path, value_json FROM props WHERE entity = ?1 ORDER BY path",
                )?;
                let rows = stmt.query_map(params![id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (p, vj) = row?;
                    if let Ok(v) = serde_json::from_str(&vj) {
                        props.insert(p, v);
                    }
                }
            }
            nodes.push(InspectNode {
                id,
                label,
                deleted,
                props,
            });
        }
        let mut edges = Vec::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT id, label, src, dst, deleted FROM edges ORDER BY id")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)? != 0,
                ))
            })?;
            for row in rows {
                let (id, label, src, dst, deleted) = row?;
                let visible = !deleted
                    && self.node_deleted_state(&src)? == Some(false)
                    && self.node_deleted_state(&dst)? == Some(false);
                edges.push(InspectEdge {
                    id,
                    label,
                    src,
                    dst,
                    deleted,
                    visible,
                });
            }
        }
        Ok(InspectReport {
            path: path.display().to_string(),
            datastore_id: self.datastore_id_hex(),
            peer: self.author_hex(),
            ops: self.op_count()?,
            nodes,
            edges,
        })
    }

    fn check_schema_pin(&self, node_hex: &str, path: &str, crdt: &str) -> Result<(), StoreError> {
        let Some(raw) = self.schema_json()? else {
            return Ok(());
        };
        let v: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let label = self
            .list_nodes()?
            .into_iter()
            .find(|(id, _, del)| id == node_hex && !*del)
            .map(|(_, l, _)| l);
        let Some(label) = label else {
            return Ok(());
        };
        let Some(expected) = v
            .pointer(&format!("/nodes/{label}/props/{path}"))
            .and_then(|x| x.as_str())
        else {
            return Ok(()); // undeclared paths allowed in soft pin mode
        };
        if expected != crdt {
            return Err(StoreError::Invalid(format!(
                "schema pin: {label}.{path} expects crdt {expected}, got {crdt}"
            )));
        }
        Ok(())
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
            let arr: [u8; 32] = row?
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
            ops.push(serde_json::from_str(&row?).map_err(|e| StoreError::Invalid(e.to_string()))?);
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

    pub fn import_bundle(&mut self, bundle: &ExportBundle) -> Result<(u32, u32), StoreError> {
        if bundle.format != 1 {
            return Err(StoreError::Invalid(format!(
                "unsupported bundle format {}",
                bundle.format
            )));
        }
        let candidate_ds = decode32(&bundle.datastore_id)?;
        let adopting = candidate_ds != self.ds;
        if adopting {
            if self.op_count()? != 0 {
                return Err(StoreError::Invalid(format!(
                    "datastore mismatch: local {} remote {}",
                    hex::encode(self.ds),
                    bundle.datastore_id
                )));
            }
            if bundle.ops.is_empty() {
                return Err(StoreError::Invalid(
                    "cannot adopt datastore from an empty bundle".into(),
                ));
            }
        }

        // Validate the complete bundle before datastore adoption or any write.
        // Keep the decoded values so the transaction does not repeat crypto work.
        let validated = bundle
            .ops
            .iter()
            .map(|op| validate_wire_for_ds(op, &candidate_ds))
            .collect::<Result<Vec<_>, _>>()?;

        let mut next_hlc_p = self.hlc_p;
        let mut next_hlc_l = self.hlc_l;
        let mut accepted = 0u32;
        let mut skipped = 0u32;
        let tx = self.conn.transaction()?;
        if adopting {
            meta_set(&tx, "ds", &candidate_ds)?;
        }
        for (op, validated) in bundle.ops.iter().zip(&validated) {
            if wire_exists(&tx, &validated.id)? {
                skipped += 1;
                continue;
            }
            (next_hlc_p, next_hlc_l) =
                next_remote_hlc(next_hlc_p, next_hlc_l, op.ts.p, op.ts.l, now_ms())?;
            apply_wire(&tx, op, validated)?;
            accepted += 1;
        }
        meta_set_u64(&tx, "hlc_p", next_hlc_p)?;
        meta_set_u64(&tx, "hlc_l", next_hlc_l as u64)?;
        tx.commit()?;

        self.ds = candidate_ds;
        self.hlc_p = next_hlc_p;
        self.hlc_l = next_hlc_l;
        Ok((accepted, skipped))
    }

    /// Rebuild all property materialization from oplog (E1 fresh-replay).
    pub fn replay_all(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM props", [])?;
        tx.execute("DELETE FROM nodes", [])?;
        tx.execute("DELETE FROM edges", [])?;
        let mut pairs: BTreeSet<(String, String)> = BTreeSet::new();
        let mut max_p: u64 = 0;
        let mut max_l: u16 = 0;
        {
            let mut stmt = tx.prepare(
                "SELECT kind, body_json, physical_ms, logical FROM ops
                 ORDER BY physical_ms, logical, id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?;
            for row in rows {
                let (kind, body_s, physical_ms, logical) = row?;
                let p = physical_ms as u64;
                let l = logical as u16;
                if (p, l) > (max_p, max_l) {
                    max_p = p;
                    max_l = l;
                }
                let body: serde_json::Value = serde_json::from_str(&body_s)
                    .map_err(|e| StoreError::Invalid(e.to_string()))?;
                match kind as u64 {
                    KIND_CREATE_NODE => {
                        let node = body["node"].as_str().unwrap_or("");
                        let label = body["label"].as_str().unwrap_or("Node");
                        tx.execute(
                            "INSERT INTO nodes (id, label, deleted) VALUES (?1,?2,0)
                             ON CONFLICT(id) DO UPDATE SET label = excluded.label",
                            params![node, label],
                        )?;
                    }
                    KIND_CREATE_EDGE => {
                        let edge = body["edge"].as_str().unwrap_or("");
                        let label = body["label"].as_str().unwrap_or("Edge");
                        let src = body["src"].as_str().unwrap_or("");
                        let dst = body["dst"].as_str().unwrap_or("");
                        tx.execute(
                            "INSERT INTO edges (id, label, src, dst, deleted) VALUES (?1,?2,?3,?4,0)
                             ON CONFLICT(id) DO UPDATE SET label=excluded.label, src=excluded.src, dst=excluded.dst",
                            params![edge, label, src, dst],
                        )?;
                    }
                    KIND_TOMBSTONE => {
                        if let Some(node) = body["node"].as_str() {
                            tx.execute(
                                "UPDATE nodes SET deleted = 1 WHERE id = ?1",
                                params![node],
                            )?;
                        }
                    }
                    KIND_SET_PROPERTY => {
                        if let (Some(n), Some(p)) = (body["node"].as_str(), body["path"].as_str()) {
                            pairs.insert((n.to_string(), p.to_string()));
                        }
                    }
                    _ => {}
                }
            }
        }
        for (n, p) in pairs {
            rematerialize_prop(&tx, &n, &p)?;
        }
        meta_set_u64(&tx, "hlc_p", max_p)?;
        meta_set_u64(&tx, "hlc_l", max_l as u64)?;
        tx.commit()?;
        self.hlc_p = max_p;
        self.hlc_l = max_l;
        Ok(())
    }

    pub fn ingest_wire(&mut self, wire: &WireOp) -> Result<bool, StoreError> {
        let validated = validate_wire_for_ds(wire, &self.ds)?;
        if wire_exists(&self.conn, &validated.id)? {
            return Ok(false);
        }

        let (next_hlc_p, next_hlc_l) =
            next_remote_hlc(self.hlc_p, self.hlc_l, wire.ts.p, wire.ts.l, now_ms())?;
        let tx = self.conn.transaction()?;
        apply_wire(&tx, wire, &validated)?;
        meta_set_u64(&tx, "hlc_p", next_hlc_p)?;
        meta_set_u64(&tx, "hlc_l", next_hlc_l as u64)?;
        tx.commit()?;

        self.hlc_p = next_hlc_p;
        self.hlc_l = next_hlc_l;
        Ok(true)
    }

    fn orset_dots_for(
        &self,
        node: &str,
        path: &str,
        value: &str,
    ) -> Result<Vec<String>, StoreError> {
        let ops = load_prop_ops(&self.conn, node, path)?;
        let target = Value::Text(value.into());
        let live: BTreeSet<Vec<u8>> = {
            let mut add_dots: BTreeMap<Vec<u8>, Value> = BTreeMap::new();
            let mut tombs: BTreeSet<Vec<u8>> = BTreeSet::new();
            for op in &ops {
                match &op.payload {
                    Payload::SetAdd(v) => {
                        add_dots.insert(op.op_id.clone(), v.clone());
                    }
                    Payload::SetRemove { observed, .. } => {
                        for d in observed {
                            tombs.insert(d.clone());
                        }
                    }
                    _ => {}
                }
            }
            add_dots
                .into_iter()
                .filter(|(d, v)| !tombs.contains(d) && v == &target)
                .map(|(d, _)| d)
                .collect()
        };
        Ok(live.into_iter().map(hex::encode).collect())
    }

    fn flag_dots(&self, node: &str, path: &str) -> Result<Vec<String>, StoreError> {
        let ops = load_prop_ops(&self.conn, node, path)?;
        let mut enables: BTreeSet<Vec<u8>> = BTreeSet::new();
        let mut tombs: BTreeSet<Vec<u8>> = BTreeSet::new();
        for op in &ops {
            match &op.payload {
                Payload::FlagEnable => {
                    enables.insert(op.op_id.clone());
                }
                Payload::FlagDisable { observed } => {
                    for d in observed {
                        tombs.insert(d.clone());
                    }
                }
                _ => {}
            }
        }
        Ok(enables
            .into_iter()
            .filter(|d| !tombs.contains(d))
            .map(hex::encode)
            .collect())
    }

    /// Run multiple local mutations in one SQLite transaction sharing a GroupId (M1-e4 / I-13).
    /// On any error from `f`, nothing is persisted.
    pub fn atomic_group<F, T>(&mut self, f: F) -> Result<T, StoreError>
    where
        F: FnOnce(&mut GroupBuilder) -> Result<T, StoreError>,
    {
        let group_id = *Uuid::now_v7().as_bytes();
        let mut builder = GroupBuilder {
            signing: self.signing.clone(),
            author: self.author,
            author_pk: self.author_pk,
            ds: self.ds,
            hlc_p: self.hlc_p,
            hlc_l: self.hlc_l,
            group_id,
            staged: Vec::new(),
        };
        let out = f(&mut builder)?;
        if !builder.staged.is_empty() {
            self.commit_wires_atomic(&builder.staged)?;
        }
        Ok(out)
    }

    /// Apply a batch of already-signed wires in one SQLite transaction (E4 layer 2).
    /// On any validation/apply failure the whole batch is rolled back.
    pub fn commit_wires_atomic(&mut self, wires: &[WireOp]) -> Result<(), StoreError> {
        if wires.is_empty() {
            return Ok(());
        }
        let mut next_p = self.hlc_p;
        let mut next_l = self.hlc_l;
        let tx = self.conn.transaction()?;
        for wire in wires {
            let validated = validate_wire_for_ds(wire, &self.ds)?;
            if wire_exists(&tx, &validated.id)? {
                return Err(StoreError::Duplicate);
            }
            (next_p, next_l) = next_remote_hlc(next_p, next_l, wire.ts.p, wire.ts.l, now_ms())?;
            apply_wire(&tx, wire, &validated)?;
        }
        meta_set_u64(&tx, "hlc_p", next_p)?;
        meta_set_u64(&tx, "hlc_l", next_l as u64)?;
        tx.commit()?;
        self.hlc_p = next_p;
        self.hlc_l = next_l;
        Ok(())
    }

    fn commit_local<F>(
        &mut self,
        kind: u64,
        body: Cbor,
        body_json: serde_json::Value,
        materialize: F,
    ) -> Result<String, StoreError>
    where
        F: FnOnce(&rusqlite::Transaction<'_>, &WireOp) -> Result<(), StoreError>,
    {
        let ts = self.next_local_ts()?;
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
            grp: None,
            kind,
            body: body_json.clone(),
            sig: hex::encode(sig),
        };
        let wire_json =
            serde_json::to_string(&wire).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let tx = self.conn.transaction()?;
        // Insert op first so rematerialize sees the new op in the set.
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
        materialize(&tx, &wire)?;
        meta_set_u64(&tx, "hlc_p", ts.physical_ms)?;
        meta_set_u64(&tx, "hlc_l", ts.logical as u64)?;
        tx.commit()?;
        self.hlc_p = ts.physical_ms;
        self.hlc_l = ts.logical;
        Ok(hex::encode(id))
    }
}

/// Builder for [`LocalStore::atomic_group`] — stages ops, commits as one transaction.
pub struct GroupBuilder {
    signing: SigningKey,
    author: [u8; 32],
    author_pk: [u8; 32],
    ds: [u8; 32],
    hlc_p: u64,
    hlc_l: u16,
    group_id: [u8; 16],
    staged: Vec<WireOp>,
}

impl GroupBuilder {
    fn next_ts(&mut self) -> Result<OpTs, StoreError> {
        let wall = now_ms();
        let p = wall.max(self.hlc_p);
        let (p, l) = if p == self.hlc_p {
            match self.hlc_l.checked_add(1) {
                Some(l) => (p, l),
                None => (
                    p.checked_add(1)
                        .ok_or_else(|| StoreError::Invalid("HLC physical time overflow".into()))?,
                    0,
                ),
            }
        } else {
            (p, 0)
        };
        self.hlc_p = p;
        self.hlc_l = l;
        Ok(OpTs {
            physical_ms: p,
            logical: l,
        })
    }

    fn stage(
        &mut self,
        kind: u64,
        body: Cbor,
        body_json: serde_json::Value,
    ) -> Result<(), StoreError> {
        let ts = self.next_ts()?;
        let env = OpEnvelope {
            v: 1,
            ds: self.ds,
            ep: 0,
            author: self.author,
            ts,
            deps: vec![],
            grp: Some(self.group_id),
            kind,
            body,
        };
        let id = env.op_id().map_err(|e| StoreError::Cbor(e.to_string()))?;
        let sig = {
            let pre = [DOMAIN_OP_SIG, id.as_slice()].concat();
            self.signing.sign(&pre).to_bytes()
        };
        self.staged.push(WireOp {
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
            grp: Some(hex::encode(self.group_id)),
            kind,
            body: body_json,
            sig: hex::encode(sig),
        });
        Ok(())
    }

    pub fn create_node(&mut self, label: &str) -> Result<String, StoreError> {
        let node = Uuid::now_v7();
        let node_bytes = *node.as_bytes();
        let node_hex = hex::encode(node_bytes);
        let body = Cbor::Map(vec![
            ("label".into(), Cbor::Text(label.into())),
            ("node".into(), Cbor::Bytes(node_bytes.to_vec())),
        ]);
        let body_json = serde_json::json!({ "label": label, "node": node_hex });
        self.stage(KIND_CREATE_NODE, body, body_json)?;
        Ok(node_hex)
    }

    pub fn set_lww(&mut self, node: &str, path: &str, value: &str) -> Result<(), StoreError> {
        let body_json = serde_json::json!({
            "node": node, "path": path, "crdt": "lww", "value": value
        });
        let body = json_to_cbor_body(&body_json)?;
        self.stage(KIND_SET_PROPERTY, body, body_json)
    }

    pub fn set_add(&mut self, node: &str, path: &str, value: &str) -> Result<(), StoreError> {
        let body_json = serde_json::json!({
            "node": node, "path": path, "crdt": "orset", "op": "add", "value": value
        });
        let body = json_to_cbor_body(&body_json)?;
        self.stage(KIND_SET_PROPERTY, body, body_json)
    }

    pub fn flag_enable(&mut self, node: &str, path: &str) -> Result<(), StoreError> {
        let body_json = serde_json::json!({
            "node": node, "path": path, "crdt": "flag", "op": "enable"
        });
        let body = json_to_cbor_body(&body_json)?;
        self.stage(KIND_SET_PROPERTY, body, body_json)
    }

    pub fn counter_inc(&mut self, node: &str, path: &str, n: u64) -> Result<(), StoreError> {
        if n == 0 {
            return Err(StoreError::Invalid("n must be > 0".into()));
        }
        let body_json = serde_json::json!({
            "node": node, "path": path, "crdt": "pncounter", "op": "inc", "n": n
        });
        let body = json_to_cbor_body(&body_json)?;
        self.stage(KIND_SET_PROPERTY, body, body_json)
    }
}

fn wire_exists(conn: &Connection, id: &[u8; 32]) -> Result<bool, StoreError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM ops WHERE id = ?1",
            params![id.as_slice()],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false))
}

fn apply_wire(
    tx: &rusqlite::Transaction<'_>,
    wire: &WireOp,
    validated: &ValidatedWire,
) -> Result<(), StoreError> {
    let body_json = wire.body.to_string();
    let wire_json = serde_json::to_string(wire).map_err(|e| StoreError::Invalid(e.to_string()))?;
    tx.execute(
        "INSERT INTO ops (id, author, author_pk, physical_ms, logical, kind, body_json, sig, wire_json)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            validated.id.as_slice(),
            validated.author.as_slice(),
            validated.author_pk.as_slice(),
            wire.ts.p as i64,
            wire.ts.l as i64,
            wire.kind as i64,
            body_json,
            validated.sig.as_slice(),
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
                "INSERT INTO nodes (id, label, deleted) VALUES (?1,?2,0)
                 ON CONFLICT(id) DO UPDATE SET label = excluded.label",
                params![node, label],
            )?;
        }
        KIND_CREATE_EDGE => {
            let edge = wire.body["edge"]
                .as_str()
                .ok_or_else(|| StoreError::Invalid("body.edge".into()))?;
            let label = wire.body["label"].as_str().unwrap_or("Edge");
            let src = wire.body["src"]
                .as_str()
                .ok_or_else(|| StoreError::Invalid("body.src".into()))?;
            let dst = wire.body["dst"]
                .as_str()
                .ok_or_else(|| StoreError::Invalid("body.dst".into()))?;
            tx.execute(
                "INSERT INTO edges (id, label, src, dst, deleted) VALUES (?1,?2,?3,?4,0)
                 ON CONFLICT(id) DO UPDATE SET label=excluded.label, src=excluded.src, dst=excluded.dst",
                params![edge, label, src, dst],
            )?;
        }
        KIND_TOMBSTONE => {
            let node = wire.body["node"]
                .as_str()
                .ok_or_else(|| StoreError::Invalid("body.node".into()))?;
            tx.execute("UPDATE nodes SET deleted = 1 WHERE id = ?1", params![node])?;
        }
        KIND_SET_PROPERTY => {
            let node = wire.body["node"]
                .as_str()
                .ok_or_else(|| StoreError::Invalid("body.node".into()))?;
            let path = wire.body["path"]
                .as_str()
                .ok_or_else(|| StoreError::Invalid("body.path".into()))?;
            rematerialize_prop(tx, node, path)?;
        }
        _ => unreachable!("wire kind was validated"),
    }
    Ok(())
}

fn next_remote_hlc(
    latest_p: u64,
    latest_l: u16,
    remote_p: u64,
    remote_l: u16,
    wall: u64,
) -> Result<(u64, u16), StoreError> {
    let max_accepted = wall.saturating_add(MAX_CLOCK_DRIFT_MS);
    if remote_p > max_accepted {
        return Err(StoreError::Invalid(format!(
            "remote clock drift exceeded: remote {remote_p} local {wall} max {MAX_CLOCK_DRIFT_MS}ms"
        )));
    }
    let physical = wall.max(latest_p).max(remote_p);
    let base = match (physical == latest_p, physical == remote_p) {
        (true, true) => Some(latest_l.max(remote_l)),
        (true, false) => Some(latest_l),
        (false, true) => Some(remote_l),
        (false, false) => None,
    };
    match base {
        None => Ok((physical, 0)),
        Some(logical) => match logical.checked_add(1) {
            Some(next) => Ok((physical, next)),
            None => Ok((
                physical
                    .checked_add(1)
                    .ok_or_else(|| StoreError::Invalid("HLC physical time overflow".into()))?,
                0,
            )),
        },
    }
}

fn validate_wire_for_ds(
    wire: &WireOp,
    expected_ds: &[u8; 32],
) -> Result<ValidatedWire, StoreError> {
    if wire.v != 1 {
        return Err(StoreError::Invalid(format!(
            "unsupported operation version {}",
            wire.v
        )));
    }
    let wire_ds = decode32(&wire.ds)?;
    if &wire_ds != expected_ds {
        return Err(StoreError::Invalid(format!(
            "operation datastore mismatch: expected {} got {}",
            hex::encode(expected_ds),
            wire.ds
        )));
    }
    if wire.ts.p > i64::MAX as u64 {
        return Err(StoreError::Invalid(
            "operation physical time exceeds SQLite range".into(),
        ));
    }
    next_remote_hlc(0, 0, wire.ts.p, wire.ts.l, now_ms())?;

    validate_wire_body(wire.kind, &wire.body)?;
    let author = decode32(&wire.author)?;
    let author_pk = decode32(&wire.author_pk)?;
    let expected_author = *blake3::hash(&author_pk).as_bytes();
    if expected_author != author {
        return Err(StoreError::Crypto("author != BLAKE3(author_pk)".into()));
    }
    let deps = wire
        .deps
        .iter()
        .map(|dep| decode32(dep))
        .collect::<Result<Vec<_>, _>>()?;
    let grp = match &wire.grp {
        None => None,
        Some(h) => {
            let b = decode_node(h)?;
            let arr: [u8; 16] = b
                .try_into()
                .map_err(|_| StoreError::Invalid("grp length".into()))?;
            Some(arr)
        }
    };
    let envelope = OpEnvelope {
        v: wire.v,
        ds: wire_ds,
        ep: wire.ep,
        author,
        ts: OpTs {
            physical_ms: wire.ts.p,
            logical: wire.ts.l,
        },
        deps,
        grp,
        kind: wire.kind,
        body: json_to_cbor_body(&wire.body)?,
    };
    let computed_id = envelope
        .op_id()
        .map_err(|e| StoreError::Cbor(e.to_string()))?;
    let claimed_id = decode32(&wire.id)?;
    if computed_id != claimed_id {
        return Err(StoreError::Crypto(
            "operation id does not match envelope".into(),
        ));
    }

    let sig: [u8; 64] = hex::decode(&wire.sig)
        .map_err(|e| StoreError::Invalid(e.to_string()))?
        .try_into()
        .map_err(|_| StoreError::Invalid("sig length".into()))?;
    if !verify_op(&author_pk, &claimed_id, &sig) {
        return Err(StoreError::Crypto("bad signature".into()));
    }

    Ok(ValidatedWire {
        id: claimed_id,
        author,
        author_pk,
        sig,
    })
}

fn validate_wire_body(kind: u64, body: &serde_json::Value) -> Result<(), StoreError> {
    let object = body
        .as_object()
        .ok_or_else(|| StoreError::Invalid("operation body must be an object".into()))?;

    match kind {
        KIND_CREATE_NODE => {
            let node = object
                .get("node")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Invalid("body.node".into()))?;
            decode_node(node)?;
            let label = object
                .get("label")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Invalid("body.label".into()))?;
            if label.len() > 256 {
                return Err(StoreError::Invalid("node label exceeds 256 bytes".into()));
            }
        }
        KIND_CREATE_EDGE => {
            let label = object
                .get("label")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Invalid("body.label".into()))?;
            if label.len() > 256 {
                return Err(StoreError::Invalid("edge label exceeds 256 bytes".into()));
            }
            let src = object
                .get("src")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Invalid("body.src".into()))?;
            let dst = object
                .get("dst")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Invalid("body.dst".into()))?;
            decode_node(src)?;
            decode_node(dst)?;
            let edge = object
                .get("edge")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Invalid("body.edge".into()))?;
            decode_node(edge)?;
        }
        KIND_TOMBSTONE => {
            let node = object
                .get("node")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Invalid("body.node".into()))?;
            decode_node(node)?;
            if object
                .get("tombstone")
                .is_some_and(|value| value.as_bool() != Some(true))
            {
                return Err(StoreError::Invalid("body.tombstone".into()));
            }
        }
        KIND_SET_PROPERTY => {
            let node = object
                .get("node")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Invalid("body.node".into()))?;
            decode_node(node)?;
            let path = object
                .get("path")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Invalid("body.path".into()))?;
            if path.is_empty() {
                return Err(StoreError::Invalid("body.path must not be empty".into()));
            }
            let crdt = object
                .get("crdt")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Invalid("body.crdt".into()))?;
            match crdt {
                "lww" => {
                    if object
                        .get("value")
                        .and_then(serde_json::Value::as_str)
                        .is_none()
                    {
                        return Err(StoreError::Invalid("lww value".into()));
                    }
                }
                "gcounter" | "pncounter" => {
                    let n = object
                        .get("n")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or_else(|| StoreError::Invalid("counter n".into()))?;
                    if n == 0 {
                        return Err(StoreError::Invalid("counter n must be > 0".into()));
                    }
                    let operation = object
                        .get("op")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| StoreError::Invalid("counter op".into()))?;
                    if !matches!(operation, "inc" | "dec")
                        || (crdt == "gcounter" && operation != "inc")
                    {
                        return Err(StoreError::Invalid("counter op".into()));
                    }
                }
                "orset" => {
                    let operation = object
                        .get("op")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| StoreError::Invalid("orset op".into()))?;
                    if !matches!(operation, "add" | "remove")
                        || object
                            .get("value")
                            .and_then(serde_json::Value::as_str)
                            .is_none()
                    {
                        return Err(StoreError::Invalid("orset payload".into()));
                    }
                    if operation == "remove" {
                        validate_observed(object.get("observed"))?;
                    }
                }
                "flag" => {
                    let operation = object
                        .get("op")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| StoreError::Invalid("flag op".into()))?;
                    if !matches!(operation, "enable" | "disable") {
                        return Err(StoreError::Invalid("flag op".into()));
                    }
                    if operation == "disable" {
                        validate_observed(object.get("observed"))?;
                    }
                }
                other => return Err(StoreError::Invalid(format!("crdt {other}"))),
            }
        }
        other => {
            return Err(StoreError::Invalid(format!(
                "unsupported operation kind {other}"
            )));
        }
    }
    Ok(())
}

fn validate_observed(value: Option<&serde_json::Value>) -> Result<(), StoreError> {
    let observed = value
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| StoreError::Invalid("observed dots".into()))?;
    for dot in observed {
        let dot = dot
            .as_str()
            .ok_or_else(|| StoreError::Invalid("observed dot".into()))?;
        decode32(dot)?;
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
    )?;
    // migrate old nodes without deleted column
    let has_deleted: bool = conn
        .prepare("PRAGMA table_info(nodes)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(|x| x.ok())
        .any(|c| c == "deleted");
    if !has_deleted {
        let _ =
            conn.execute_batch("ALTER TABLE nodes ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;");
    }
    Ok(())
}

fn rematerialize_prop(
    tx: &rusqlite::Transaction<'_>,
    entity: &str,
    path: &str,
) -> Result<(), StoreError> {
    let ops = load_prop_ops(tx, entity, path)?;
    if ops.is_empty() {
        tx.execute(
            "DELETE FROM props WHERE entity = ?1 AND path = ?2",
            params![entity, path],
        )?;
        return Ok(());
    }
    // Determine crdt from first op
    let crdt = match &ops[0].payload {
        Payload::LwwSet(_) => "lww",
        Payload::CounterInc(_) => "gcounter",
        Payload::CounterDec(_) => "pncounter",
        Payload::SetAdd(_) | Payload::SetRemove { .. } => "orset",
        Payload::FlagEnable | Payload::FlagDisable { .. } => "flag",
    };
    // Refine counter type from body_json stored ops if needed
    let crdt = infer_crdt_from_tx(tx, entity, path)?.unwrap_or(crdt);

    let value_json = match crdt {
        "lww" => {
            let mut rep = Replica::<Lww>::default();
            for op in &ops {
                rep.ingest(op);
            }
            let s = rep
                .state()
                .map_err(|e| StoreError::Invalid(e.to_string()))?;
            match s.value() {
                Some(Value::Text(t)) => serde_json::to_string(t).unwrap(),
                Some(Value::Int(i)) => serde_json::to_string(i).unwrap(),
                Some(Value::Bool(b)) => serde_json::to_string(b).unwrap(),
                Some(Value::Null) => "null".into(),
                _ => "null".into(),
            }
        }
        "gcounter" => {
            let mut rep = Replica::<GCounter>::default();
            for op in &ops {
                rep.ingest(op);
            }
            let s = rep
                .state()
                .map_err(|e| StoreError::Invalid(e.to_string()))?;
            serde_json::to_string(&s.value()).unwrap()
        }
        "pncounter" => {
            let mut rep = Replica::<PnCounter>::default();
            for op in &ops {
                rep.ingest(op);
            }
            let s = rep
                .state()
                .map_err(|e| StoreError::Invalid(e.to_string()))?;
            serde_json::to_string(&s.value()).unwrap()
        }
        "orset" => {
            let mut rep = Replica::<OrSet>::default();
            for op in &ops {
                rep.ingest(op);
            }
            let s = rep
                .state()
                .map_err(|e| StoreError::Invalid(e.to_string()))?;
            let mut els: Vec<String> = s
                .elements()
                .into_iter()
                .filter_map(|v| match v {
                    Value::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .collect();
            els.sort();
            serde_json::to_string(&els).unwrap()
        }
        "flag" => {
            let mut rep = Replica::<Flag>::default();
            for op in &ops {
                rep.ingest(op);
            }
            let s = rep
                .state()
                .map_err(|e| StoreError::Invalid(e.to_string()))?;
            serde_json::to_string(&s.enabled()).unwrap()
        }
        other => return Err(StoreError::Invalid(format!("unknown crdt {other}"))),
    };

    tx.execute(
        "INSERT INTO props (entity, path, crdt, value_json) VALUES (?1,?2,?3,?4)
         ON CONFLICT(entity, path) DO UPDATE SET crdt=excluded.crdt, value_json=excluded.value_json",
        params![entity, path, crdt, value_json],
    )?;
    Ok(())
}

fn infer_crdt_from_tx(
    tx: &rusqlite::Transaction<'_>,
    entity: &str,
    path: &str,
) -> Result<Option<&'static str>, StoreError> {
    let mut stmt =
        tx.prepare("SELECT body_json FROM ops WHERE kind = 3 ORDER BY physical_ms, logical, id")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows {
        let s = row?;
        let v: serde_json::Value =
            serde_json::from_str(&s).map_err(|e| StoreError::Invalid(e.to_string()))?;
        if v["node"].as_str() == Some(entity)
            && v["path"].as_str() == Some(path)
            && let Some(c) = v["crdt"].as_str()
        {
            return Ok(Some(match c {
                "lww" => "lww",
                "gcounter" => "gcounter",
                "pncounter" => "pncounter",
                "orset" => "orset",
                "flag" => "flag",
                _ => continue,
            }));
        }
    }
    Ok(None)
}

fn load_prop_ops(conn: &Connection, entity: &str, path: &str) -> Result<Vec<KernelOp>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, author, physical_ms, logical, body_json FROM ops
         WHERE kind = 3 ORDER BY physical_ms, logical, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, Vec<u8>>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, author, p, l, body_s) = row?;
        let body: serde_json::Value =
            serde_json::from_str(&body_s).map_err(|e| StoreError::Invalid(e.to_string()))?;
        if body["node"].as_str() != Some(entity) || body["path"].as_str() != Some(path) {
            continue;
        }
        let payload = body_to_payload(&body)?;
        out.push(KernelOp {
            op_id: id,
            author,
            physical_ms: p as u64,
            logical: l as u16,
            payload,
        });
    }
    Ok(out)
}

fn body_to_payload(body: &serde_json::Value) -> Result<Payload, StoreError> {
    let crdt = body["crdt"].as_str().unwrap_or("lww");
    match crdt {
        "lww" => {
            let v = body["value"]
                .as_str()
                .ok_or_else(|| StoreError::Invalid("lww value".into()))?;
            Ok(Payload::LwwSet(Value::Text(v.into())))
        }
        "gcounter" | "pncounter" => {
            let n = body["n"].as_u64().unwrap_or(1);
            match body["op"].as_str().unwrap_or("inc") {
                "dec" => Ok(Payload::CounterDec(n)),
                _ => Ok(Payload::CounterInc(n)),
            }
        }
        "orset" => match body["op"].as_str().unwrap_or("add") {
            "remove" => {
                let observed = body["observed"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .filter_map(|x| x.as_str())
                    .filter_map(|h| hex::decode(h).ok())
                    .collect();
                let element = body["value"].as_str().unwrap_or("").to_string();
                Ok(Payload::SetRemove {
                    element: Value::Text(element),
                    observed,
                })
            }
            _ => {
                let v = body["value"].as_str().unwrap_or("").to_string();
                Ok(Payload::SetAdd(Value::Text(v)))
            }
        },
        "flag" => match body["op"].as_str().unwrap_or("enable") {
            "disable" => {
                let observed = body["observed"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .filter_map(|x| x.as_str())
                    .filter_map(|h| hex::decode(h).ok())
                    .collect();
                Ok(Payload::FlagDisable { observed })
            }
            _ => Ok(Payload::FlagEnable),
        },
        other => Err(StoreError::Invalid(format!("crdt {other}"))),
    }
}

fn json_to_cbor_body(v: &serde_json::Value) -> Result<Cbor, StoreError> {
    // Store a simplified CBOR map for preimage stability
    let mut entries = Vec::new();
    if let Some(obj) = v.as_object() {
        for (k, val) in obj {
            let c = match val {
                serde_json::Value::String(s) => {
                    if k == "node" || k == "edge" || k == "src" || k == "dst" {
                        let b = hex::decode(s).map_err(|e| StoreError::Invalid(e.to_string()))?;
                        Cbor::Bytes(b)
                    } else {
                        Cbor::Text(s.clone())
                    }
                }
                serde_json::Value::Number(n) => {
                    if let Some(u) = n.as_u64() {
                        Cbor::Uint(u)
                    } else {
                        Cbor::Text(n.to_string())
                    }
                }
                serde_json::Value::Bool(b) => Cbor::Bool(*b),
                serde_json::Value::Array(a) => Cbor::Array(
                    a.iter()
                        .map(|x| match x {
                            serde_json::Value::String(s) => Cbor::Text(s.clone()),
                            _ => Cbor::Text(x.to_string()),
                        })
                        .collect(),
                ),
                serde_json::Value::Null => Cbor::Null,
                other => Cbor::Text(other.to_string()),
            };
            entries.push((k.clone(), c));
        }
    }
    Ok(Cbor::Map(entries))
}

fn meta_get(conn: &Connection, k: &str) -> Result<Option<Vec<u8>>, StoreError> {
    conn.query_row("SELECT v FROM meta WHERE k = ?1", params![k], |r| r.get(0))
        .optional()
        .map_err(Into::into)
}

fn meta_set(conn: &Connection, k: &str, v: &[u8]) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![k, v],
    )?;
    Ok(())
}

fn meta_get_u64(conn: &Connection, k: &str) -> Result<Option<u64>, StoreError> {
    Ok(meta_get(conn, k)?.map(|b| {
        let mut arr = [0u8; 8];
        let n = b.len().min(8);
        arr[..n].copy_from_slice(&b[..n]);
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

fn decode_node(s: &str) -> Result<Vec<u8>, StoreError> {
    let b = hex::decode(s).map_err(|e| StoreError::Invalid(e.to_string()))?;
    if b.len() != 16 {
        return Err(StoreError::Invalid("node id must be 16 bytes hex".into()));
    }
    Ok(b)
}

fn getrandom_fill(buf: &mut [u8]) {
    getrandom::getrandom(buf).expect("OS CSPRNG unavailable for key material");
}

fn json_to_qvalue(v: &serde_json::Value) -> QValue {
    match v {
        serde_json::Value::Null => QValue::Null,
        serde_json::Value::Bool(b) => QValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                QValue::Int(i)
            } else if let Some(u) = n.as_u64() {
                QValue::Int(u as i64)
            } else if let Some(f) = n.as_f64() {
                QValue::Float(f)
            } else {
                QValue::Null
            }
        }
        serde_json::Value::String(s) => QValue::Text(s.clone()),
        serde_json::Value::Array(a) => QValue::Mv(a.iter().map(json_to_qvalue).collect()),
        serde_json::Value::Object(_) => QValue::Text(v.to_string()),
    }
}

fn qvalue_to_json(v: &QValue) -> serde_json::Value {
    match v {
        QValue::Null => serde_json::Value::Null,
        QValue::Bool(b) => serde_json::json!(b),
        QValue::Int(i) => serde_json::json!(i),
        QValue::Float(f) => serde_json::json!(f),
        QValue::Text(s) => serde_json::json!(s),
        QValue::Bytes(b) => serde_json::json!(hex::encode(b)),
        QValue::Mv(xs) => serde_json::Value::Array(xs.iter().map(qvalue_to_json).collect()),
        QValue::Node(id) | QValue::Edge(id) => serde_json::json!(id),
    }
}

/// Durable HLC high-water = max over oplog timestamps; rewrite meta if stale (DQ-7).
fn recover_hlc_from_oplog(conn: &Connection) -> Result<(u64, u16), StoreError> {
    let max: Option<(u64, u16)> = conn
        .query_row(
            "SELECT physical_ms, logical FROM ops
             ORDER BY physical_ms DESC, logical DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u16)),
        )
        .optional()?;
    let (max_p, max_l) = max.unwrap_or((0, 0));
    let meta_p = meta_get_u64(conn, "hlc_p")?.unwrap_or(0);
    let meta_l = meta_get_u64(conn, "hlc_l")?.unwrap_or(0) as u16;
    let (p, l) = if (max_p, max_l) > (meta_p, meta_l) {
        (max_p, max_l)
    } else {
        (meta_p, meta_l)
    };
    if (p, l) != (meta_p, meta_l) {
        meta_set_u64(conn, "hlc_p", p)?;
        meta_set_u64(conn, "hlc_l", l as u64)?;
    }
    Ok((p, l))
}

fn ensure_storage_format_version(conn: &Connection) -> Result<(), StoreError> {
    match meta_get_u64(conn, "storage_format_version")? {
        Some(v) if v == STORAGE_FORMAT_VERSION => Ok(()),
        Some(v) => Err(StoreError::Invalid(format!(
            "unsupported storage_format_version {v} (expected {STORAGE_FORMAT_VERSION})"
        ))),
        None => meta_set_u64(conn, "storage_format_version", STORAGE_FORMAT_VERSION),
    }
}
