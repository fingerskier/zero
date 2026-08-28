//! Persistence backend abstraction (M2 prep).
//!
//! All durable state — meta KV, oplog, materialized nodes/props/edges — goes
//! through [`StoreBackend`] so alternative backends (e.g. browser OPFS or
//! IndexedDB) can be implemented without forking the store logic. The trait is
//! object-safe; transactions are closure-based via [`StoreBackend::with_txn`].

use crate::StoreError;

/// Full oplog row as persisted.
pub struct OpRecord {
    pub id: [u8; 32],
    pub author: [u8; 32],
    pub author_pk: [u8; 32],
    pub physical_ms: u64,
    pub logical: u16,
    pub kind: u64,
    pub body_json: String,
    pub sig: [u8; 64],
    pub wire_json: String,
}

/// Oplog scan row for replay (HLC order).
pub struct OpScanRow {
    pub kind: u64,
    pub body_json: String,
    pub physical_ms: u64,
    pub logical: u16,
}

/// Oplog scan row for property rematerialization (kind = SetProperty, HLC order).
pub struct PropOpRow {
    pub id: Vec<u8>,
    pub author: Vec<u8>,
    pub physical_ms: u64,
    pub logical: u16,
    pub body_json: String,
}

/// Materialized edge row.
pub struct EdgeRow {
    pub id: String,
    pub label: String,
    pub src: String,
    pub dst: String,
    pub deleted: bool,
}

/// The persistence surface, usable both outside and inside a transaction.
/// All scans return rows ordered by (physical_ms, logical, id).
pub trait BackendTxn {
    // meta KV
    fn meta_get(&self, k: &str) -> Result<Option<Vec<u8>>, StoreError>;
    fn meta_set(&self, k: &str, v: &[u8]) -> Result<(), StoreError>;

    // oplog
    fn op_insert(&self, rec: &OpRecord) -> Result<(), StoreError>;
    fn op_exists(&self, id: &[u8; 32]) -> Result<bool, StoreError>;
    fn op_count(&self) -> Result<u64, StoreError>;
    fn op_ids(&self) -> Result<Vec<Vec<u8>>, StoreError>;
    fn op_wires(&self) -> Result<Vec<String>, StoreError>;
    fn op_wire_by_id(&self, id: &[u8; 32]) -> Result<Option<String>, StoreError>;
    fn op_scan(&self) -> Result<Vec<OpScanRow>, StoreError>;
    /// CreateNode + Tombstone ops only: (kind, body_json).
    fn op_scan_node_kinds(&self) -> Result<Vec<(u64, String)>, StoreError>;
    /// SetProperty ops only.
    fn op_scan_props(&self) -> Result<Vec<PropOpRow>, StoreError>;
    fn op_max_hlc(&self) -> Result<Option<(u64, u16)>, StoreError>;

    // nodes
    fn node_upsert(&self, id: &str, label: &str, deleted: bool) -> Result<(), StoreError>;
    fn node_insert_ignore(&self, id: &str, label: &str) -> Result<(), StoreError>;
    fn node_delete(&self, id: &str) -> Result<(), StoreError>;
    fn node_deleted_state(&self, id: &str) -> Result<Option<bool>, StoreError>;
    fn node_list(&self) -> Result<Vec<(String, String, bool)>, StoreError>;
    fn node_count(&self) -> Result<u64, StoreError>;

    // props
    fn prop_upsert(
        &self,
        entity: &str,
        path: &str,
        crdt: &str,
        value_json: &str,
    ) -> Result<(), StoreError>;
    fn prop_delete(&self, entity: &str, path: &str) -> Result<(), StoreError>;
    fn prop_get(&self, entity: &str, path: &str) -> Result<Option<String>, StoreError>;
    /// (path, value_json) pairs for an entity, ordered by path.
    fn prop_list(&self, entity: &str) -> Result<Vec<(String, String)>, StoreError>;
    /// All properties as `(entity, path, value_json)`, ordered by entity then path.
    /// One scan so `to_query_graph` / `list_nodes` are not N+1.
    fn prop_list_all(&self) -> Result<Vec<(String, String, String)>, StoreError>;

    // edges
    /// Write the full derived edge projection (including its set-derived
    /// `deleted` flag — any edge Tombstone in the op set deletes, order-free).
    fn edge_upsert(
        &self,
        id: &str,
        label: &str,
        src: &str,
        dst: &str,
        deleted: bool,
    ) -> Result<(), StoreError>;
    /// Remove an edge row entirely (orphan tombstone: no CreateEdge in the op set).
    fn edge_delete(&self, id: &str) -> Result<(), StoreError>;
    fn edge_list(&self) -> Result<Vec<EdgeRow>, StoreError>;
    /// Visible edges only (both endpoints live; edge not deleted), ordered by id.
    fn edge_list_visible(&self) -> Result<Vec<(String, String, String, String)>, StoreError>;

    /// Clear all materialized state (nodes, props, edges) for fresh replay.
    fn wipe_materialized(&self) -> Result<(), StoreError>;
}

/// A backend adds atomic transactions on top of the plain surface.
/// On `Err` from the closure nothing is persisted.
pub trait StoreBackend: BackendTxn {
    fn with_txn(
        &mut self,
        f: &mut dyn FnMut(&dyn BackendTxn) -> Result<(), StoreError>,
    ) -> Result<(), StoreError>;
}
