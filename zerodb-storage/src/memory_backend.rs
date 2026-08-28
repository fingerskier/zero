//! In-memory implementation of [`StoreBackend`] (always compiled).
//!
//! Primary consumer is the wasm/browser peer, where durability is handled by
//! the embedder (e.g. export bundle + identity seed persisted to IndexedDB).
//! Transactions are snapshot-based: `with_txn` clones the whole state and
//! restores it if the closure errors — correct rollback, fine at MVP scale.

use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::backend::{BackendTxn, EdgeRow, OpRecord, OpScanRow, PropOpRow, StoreBackend};
use crate::{KIND_CREATE_NODE, KIND_SET_PROPERTY, KIND_TOMBSTONE, StoreError};

#[derive(Clone)]
struct MemOp {
    author: [u8; 32],
    kind: u64,
    body_json: String,
    wire_json: String,
}

#[derive(Clone)]
struct MemEdge {
    label: String,
    src: String,
    dst: String,
    deleted: bool,
}

#[derive(Clone, Default)]
struct Inner {
    meta: BTreeMap<String, Vec<u8>>,
    /// Keyed by (physical_ms, logical, id) so scans are naturally in HLC order.
    ops: BTreeMap<(u64, u16, [u8; 32]), MemOp>,
    /// id -> (label, deleted)
    nodes: BTreeMap<String, (String, bool)>,
    /// (entity, path) -> (crdt, value_json)
    props: BTreeMap<(String, String), (String, String)>,
    edges: BTreeMap<String, MemEdge>,
}

/// Plain-HashMap/BTreeMap [`StoreBackend`]; no I/O, no SQLite.
#[derive(Default)]
pub struct MemoryBackend {
    inner: RefCell<Inner>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl BackendTxn for MemoryBackend {
    fn meta_get(&self, k: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.inner.borrow().meta.get(k).cloned())
    }
    fn meta_set(&self, k: &str, v: &[u8]) -> Result<(), StoreError> {
        self.inner.borrow_mut().meta.insert(k.into(), v.to_vec());
        Ok(())
    }

    fn op_insert(&self, rec: &OpRecord) -> Result<(), StoreError> {
        let mut inner = self.inner.borrow_mut();
        if inner.ops.keys().any(|(_, _, id)| id == &rec.id) {
            return Err(StoreError::Duplicate);
        }
        inner.ops.insert(
            (rec.physical_ms, rec.logical, rec.id),
            MemOp {
                author: rec.author,
                kind: rec.kind,
                body_json: rec.body_json.clone(),
                wire_json: rec.wire_json.clone(),
            },
        );
        Ok(())
    }
    fn op_exists(&self, id: &[u8; 32]) -> Result<bool, StoreError> {
        Ok(self.inner.borrow().ops.keys().any(|(_, _, i)| i == id))
    }
    fn op_count(&self) -> Result<u64, StoreError> {
        Ok(self.inner.borrow().ops.len() as u64)
    }
    fn op_ids(&self) -> Result<Vec<Vec<u8>>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .ops
            .keys()
            .map(|(_, _, id)| id.to_vec())
            .collect())
    }
    fn op_wires(&self) -> Result<Vec<String>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .ops
            .values()
            .map(|op| op.wire_json.clone())
            .collect())
    }
    fn op_wire_by_id(&self, id: &[u8; 32]) -> Result<Option<String>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .ops
            .iter()
            .find(|((_, _, i), _)| i == id)
            .map(|(_, op)| op.wire_json.clone()))
    }
    fn op_scan(&self) -> Result<Vec<OpScanRow>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .ops
            .iter()
            .map(|((p, l, _), op)| OpScanRow {
                kind: op.kind,
                body_json: op.body_json.clone(),
                physical_ms: *p,
                logical: *l,
            })
            .collect())
    }
    fn op_scan_node_kinds(&self) -> Result<Vec<(u64, String)>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .ops
            .values()
            .filter(|op| op.kind == KIND_CREATE_NODE || op.kind == KIND_TOMBSTONE)
            .map(|op| (op.kind, op.body_json.clone()))
            .collect())
    }
    fn op_scan_props(&self) -> Result<Vec<PropOpRow>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .ops
            .iter()
            .filter(|(_, op)| op.kind == KIND_SET_PROPERTY)
            .map(|((p, l, id), op)| PropOpRow {
                id: id.to_vec(),
                author: op.author.to_vec(),
                physical_ms: *p,
                logical: *l,
                body_json: op.body_json.clone(),
            })
            .collect())
    }
    fn op_max_hlc(&self) -> Result<Option<(u64, u16)>, StoreError> {
        // Key order is (physical_ms, logical, id); the last key is the max HLC.
        Ok(self
            .inner
            .borrow()
            .ops
            .keys()
            .next_back()
            .map(|(p, l, _)| (*p, *l)))
    }

    fn node_upsert(&self, id: &str, label: &str, deleted: bool) -> Result<(), StoreError> {
        self.inner
            .borrow_mut()
            .nodes
            .insert(id.into(), (label.into(), deleted));
        Ok(())
    }
    fn node_insert_ignore(&self, id: &str, label: &str) -> Result<(), StoreError> {
        self.inner
            .borrow_mut()
            .nodes
            .entry(id.into())
            .or_insert_with(|| (label.into(), false));
        Ok(())
    }
    fn node_delete(&self, id: &str) -> Result<(), StoreError> {
        self.inner.borrow_mut().nodes.remove(id);
        Ok(())
    }
    fn node_deleted_state(&self, id: &str) -> Result<Option<bool>, StoreError> {
        Ok(self.inner.borrow().nodes.get(id).map(|(_, d)| *d))
    }
    fn node_list(&self) -> Result<Vec<(String, String, bool)>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .nodes
            .iter()
            .map(|(id, (label, deleted))| (id.clone(), label.clone(), *deleted))
            .collect())
    }
    fn node_count(&self) -> Result<u64, StoreError> {
        Ok(self.inner.borrow().nodes.len() as u64)
    }

    fn prop_upsert(
        &self,
        entity: &str,
        path: &str,
        crdt: &str,
        value_json: &str,
    ) -> Result<(), StoreError> {
        self.inner.borrow_mut().props.insert(
            (entity.into(), path.into()),
            (crdt.into(), value_json.into()),
        );
        Ok(())
    }
    fn prop_delete(&self, entity: &str, path: &str) -> Result<(), StoreError> {
        self.inner
            .borrow_mut()
            .props
            .remove(&(entity.to_string(), path.to_string()));
        Ok(())
    }
    fn prop_get(&self, entity: &str, path: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .props
            .get(&(entity.to_string(), path.to_string()))
            .map(|(_, v)| v.clone()))
    }
    fn prop_list(&self, entity: &str) -> Result<Vec<(String, String)>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .props
            .iter()
            .filter(|((e, _), _)| e == entity)
            .map(|((_, p), (_, v))| (p.clone(), v.clone()))
            .collect())
    }
    fn prop_list_all(&self) -> Result<Vec<(String, String, String)>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .props
            .iter()
            .map(|((e, p), (_, v))| (e.clone(), p.clone(), v.clone()))
            .collect())
    }

    fn edge_upsert(
        &self,
        id: &str,
        label: &str,
        src: &str,
        dst: &str,
        deleted: bool,
    ) -> Result<(), StoreError> {
        self.inner.borrow_mut().edges.insert(
            id.into(),
            MemEdge {
                label: label.into(),
                src: src.into(),
                dst: dst.into(),
                deleted,
            },
        );
        Ok(())
    }
    fn edge_delete(&self, id: &str) -> Result<(), StoreError> {
        self.inner.borrow_mut().edges.remove(id);
        Ok(())
    }
    fn edge_list(&self) -> Result<Vec<EdgeRow>, StoreError> {
        Ok(self
            .inner
            .borrow()
            .edges
            .iter()
            .map(|(id, e)| EdgeRow {
                id: id.clone(),
                label: e.label.clone(),
                src: e.src.clone(),
                dst: e.dst.clone(),
                deleted: e.deleted,
            })
            .collect())
    }
    fn edge_list_visible(&self) -> Result<Vec<(String, String, String, String)>, StoreError> {
        let inner = self.inner.borrow();
        Ok(inner
            .edges
            .iter()
            .filter(|(_, e)| {
                !e.deleted
                    && inner.nodes.get(&e.src).is_some_and(|(_, d)| !*d)
                    && inner.nodes.get(&e.dst).is_some_and(|(_, d)| !*d)
            })
            .map(|(id, e)| (id.clone(), e.label.clone(), e.src.clone(), e.dst.clone()))
            .collect())
    }

    fn wipe_materialized(&self) -> Result<(), StoreError> {
        let mut inner = self.inner.borrow_mut();
        inner.nodes.clear();
        inner.props.clear();
        inner.edges.clear();
        Ok(())
    }
}

impl StoreBackend for MemoryBackend {
    fn with_txn(
        &mut self,
        f: &mut dyn FnMut(&dyn BackendTxn) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        let snapshot = self.inner.borrow().clone();
        match f(self) {
            Ok(()) => Ok(()),
            Err(e) => {
                *self.inner.borrow_mut() = snapshot;
                Err(e)
            }
        }
    }
}
