//! Experimental M4a browser peer: wasm-bindgen wrapper over
//! `LocalStore<MemoryBackend>`. Mirrors the NAPI `Database` surface where it
//! makes sense, plus sync-driver hooks (`opIds`, `exportOpsByIds`) so a JS
//! WebSocket driver can speak sync protocol v2 as a client.
//!
//! Identity/persistence model (honest about the risk): the store lives in
//! memory; the embedder persists `seedHex()` + `datastoreId()` + the export
//! bundle (e.g. to IndexedDB) and restores via `ZeroDb.fromSeed(...)` +
//! `importJson(...)`. The ed25519 seed therefore sits in browser storage —
//! any script with origin access can sign as this peer. Acceptable for this
//! experimental slice only.

use wasm_bindgen::prelude::*;
use zerodb_storage::{ExportBundle, LocalStore, MemoryBackend, StoreError};

#[wasm_bindgen]
pub struct ZeroDb {
    store: LocalStore<MemoryBackend>,
    subs: Vec<(u32, js_sys::Function)>,
    next_sub: u32,
}

fn err(e: StoreError) -> JsError {
    JsError::new(&e.to_string())
}

fn to_js(v: &serde_json::Value) -> Result<JsValue, JsError> {
    js_sys::JSON::parse(&v.to_string()).map_err(|_| JsError::new("json parse"))
}

fn decode32(s: &str) -> Result<[u8; 32], JsError> {
    let b = hex::decode(s).map_err(|e| JsError::new(&e.to_string()))?;
    b.try_into()
        .map_err(|_| JsError::new("expected 32 bytes hex"))
}

impl ZeroDb {
    fn wrap(store: LocalStore<MemoryBackend>) -> ZeroDb {
        ZeroDb {
            store,
            subs: Vec::new(),
            next_sub: 0,
        }
    }

    /// Invoke every registered change callback with `event` (as a JS value).
    /// Callbacks MUST NOT synchronously call back into this ZeroDb (the
    /// wasm-bindgen borrow is still held) — defer with `queueMicrotask`.
    fn notify(&self, event: serde_json::Value) {
        if self.subs.is_empty() {
            return;
        }
        let Ok(val) = js_sys::JSON::parse(&event.to_string()) else {
            return;
        };
        for (_, cb) in &self.subs {
            let _ = cb.call1(&JsValue::NULL, &val);
        }
    }

    fn notify_op(&self, method: &str, node: &str, key: Option<&str>, op_id: &str) {
        self.notify(serde_json::json!({
            "kind": "op",
            "method": method,
            "node": node,
            "key": key,
            "opId": op_id,
        }));
    }
}

#[wasm_bindgen]
impl ZeroDb {
    /// Fresh in-memory store with a new random identity + datastore id.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<ZeroDb, JsError> {
        let store = LocalStore::init_with_backend(MemoryBackend::new()).map_err(err)?;
        Ok(ZeroDb::wrap(store))
    }

    /// Register a change callback, mirroring the NAPI `subscribe` shapes:
    /// `{kind:'op', method, node, key?, opId}` after each local mutation,
    /// `{kind:'import', accepted, skipped}` after `importJson`,
    /// `{kind:'replay'}` after `replay`. Delivery is synchronous — the
    /// callback must not re-enter this ZeroDb directly (use `queueMicrotask`).
    /// Returns an id for `offChange`.
    #[wasm_bindgen(js_name = onChange)]
    pub fn on_change(&mut self, callback: js_sys::Function) -> u32 {
        let id = self.next_sub;
        self.next_sub += 1;
        self.subs.push((id, callback));
        id
    }

    /// Remove a change callback; unknown ids are a no-op.
    #[wasm_bindgen(js_name = offChange)]
    pub fn off_change(&mut self, id: u32) {
        self.subs.retain(|(i, _)| *i != id);
    }

    /// Restore a persisted identity: same seed + datastore id as a previous
    /// session. Re-import the persisted bundle afterwards to restore ops.
    #[wasm_bindgen(js_name = fromSeed)]
    pub fn from_seed(seed_hex: &str, ds_hex: &str) -> Result<ZeroDb, JsError> {
        let seed = decode32(seed_hex)?;
        let ds = decode32(ds_hex)?;
        let store = LocalStore::init_with_backend_from_seed(MemoryBackend::new(), &seed, &ds)
            .map_err(err)?;
        Ok(ZeroDb::wrap(store))
    }

    /// Raw identity seed (hex). Whoever holds this can sign as this peer.
    #[wasm_bindgen(js_name = seedHex)]
    pub fn seed_hex(&self) -> String {
        hex::encode(self.store.identity_seed())
    }

    #[wasm_bindgen(js_name = datastoreId)]
    pub fn datastore_id(&self) -> String {
        self.store.datastore_id_hex()
    }

    #[wasm_bindgen(js_name = peerId)]
    pub fn peer_id(&self) -> String {
        self.store.author_hex()
    }

    #[wasm_bindgen(js_name = opCount)]
    pub fn op_count(&self) -> Result<u32, JsError> {
        Ok(self.store.op_count().map_err(err)? as u32)
    }

    #[wasm_bindgen(js_name = createNode)]
    pub fn create_node(&mut self, label: &str) -> Result<String, JsError> {
        let (id, op) = self.store.create_node_with_op(label).map_err(err)?;
        self.notify_op("createNode", &id, None, &op);
        Ok(id)
    }

    #[wasm_bindgen(js_name = deleteNode)]
    pub fn delete_node(&mut self, node: &str) -> Result<String, JsError> {
        let op = self.store.delete_node(node).map_err(err)?;
        self.notify_op("deleteNode", node, None, &op);
        Ok(op)
    }

    #[wasm_bindgen(js_name = setLww)]
    pub fn set_lww(&mut self, node: &str, key: &str, value: &str) -> Result<String, JsError> {
        let op = self.store.set_lww(node, key, value).map_err(err)?;
        self.notify_op("setLww", node, Some(key), &op);
        Ok(op)
    }

    #[wasm_bindgen(js_name = getLww)]
    pub fn get_lww(&self, node: &str, key: &str) -> Result<Option<String>, JsError> {
        self.store.get_lww(node, key).map_err(err)
    }

    /// Materialized property as a JS value (string, number, bool, array, null).
    #[wasm_bindgen(js_name = getProp)]
    pub fn get_prop(&self, node: &str, key: &str) -> Result<JsValue, JsError> {
        match self.store.get_prop(node, key).map_err(err)? {
            Some(v) => to_js(&v),
            None => Ok(JsValue::NULL),
        }
    }

    #[wasm_bindgen(js_name = counterInc)]
    pub fn counter_inc(&mut self, node: &str, key: &str, n: u32) -> Result<String, JsError> {
        let op = self.store.counter_inc(node, key, n as u64).map_err(err)?;
        self.notify_op("counterInc", node, Some(key), &op);
        Ok(op)
    }

    #[wasm_bindgen(js_name = counterDec)]
    pub fn counter_dec(&mut self, node: &str, key: &str, n: u32) -> Result<String, JsError> {
        let op = self.store.counter_dec(node, key, n as u64).map_err(err)?;
        self.notify_op("counterDec", node, Some(key), &op);
        Ok(op)
    }

    #[wasm_bindgen(js_name = gcounterInc)]
    pub fn gcounter_inc(&mut self, node: &str, key: &str, n: u32) -> Result<String, JsError> {
        let op = self.store.gcounter_inc(node, key, n as u64).map_err(err)?;
        self.notify_op("gcounterInc", node, Some(key), &op);
        Ok(op)
    }

    #[wasm_bindgen(js_name = setAdd)]
    pub fn set_add(&mut self, node: &str, key: &str, value: &str) -> Result<String, JsError> {
        let op = self.store.set_add(node, key, value).map_err(err)?;
        self.notify_op("setAdd", node, Some(key), &op);
        Ok(op)
    }

    #[wasm_bindgen(js_name = setRemove)]
    pub fn set_remove(&mut self, node: &str, key: &str, value: &str) -> Result<String, JsError> {
        let op = self.store.set_remove(node, key, value).map_err(err)?;
        self.notify_op("setRemove", node, Some(key), &op);
        Ok(op)
    }

    #[wasm_bindgen(js_name = flagEnable)]
    pub fn flag_enable(&mut self, node: &str, key: &str) -> Result<String, JsError> {
        let op = self.store.flag_enable(node, key).map_err(err)?;
        self.notify_op("flagEnable", node, Some(key), &op);
        Ok(op)
    }

    #[wasm_bindgen(js_name = flagDisable)]
    pub fn flag_disable(&mut self, node: &str, key: &str) -> Result<String, JsError> {
        let op = self.store.flag_disable(node, key).map_err(err)?;
        self.notify_op("flagDisable", node, Some(key), &op);
        Ok(op)
    }

    /// Nodes as a JS array of `{ id, label, deleted }`.
    #[wasm_bindgen(js_name = listNodes)]
    pub fn list_nodes(&self) -> Result<JsValue, JsError> {
        let listed = self.store.list_nodes().map_err(err)?;
        let mut props_by = self.store.list_props_all().unwrap_or_default();
        let arr: Vec<serde_json::Value> = listed
            .into_iter()
            .map(|(id, label, deleted)| {
                let props: serde_json::Map<String, serde_json::Value> = if deleted {
                    serde_json::Map::new()
                } else {
                    props_by
                        .remove(&id)
                        .unwrap_or_default()
                        .into_iter()
                        .collect()
                };
                serde_json::json!({ "id": id, "label": label, "deleted": deleted, "props": props })
            })
            .collect();
        to_js(&serde_json::Value::Array(arr))
    }

    #[wasm_bindgen(js_name = createEdge)]
    pub fn create_edge(&mut self, label: &str, src: &str, dst: &str) -> Result<String, JsError> {
        self.store.create_edge(label, src, dst).map_err(err)
    }

    #[wasm_bindgen(js_name = deleteEdge)]
    pub fn delete_edge(&mut self, edge: &str) -> Result<String, JsError> {
        self.store.delete_edge(edge).map_err(err)
    }

    #[wasm_bindgen(js_name = applySchema)]
    pub fn apply_schema(&mut self, schema_json: &str) -> Result<JsValue, JsError> {
        self.store.apply_schema_json(schema_json).map_err(err)?;
        to_js(&serde_json::json!({
            "schemaId": self.store.schema_id_hex().map_err(err)?,
            "epoch": self.store.schema_epoch().map_err(err)?,
        }))
    }

    /// O3 minimal query; JS array of row objects.
    pub fn query(&self, q: &str) -> Result<JsValue, JsError> {
        to_js(&self.store.query(q).map_err(err)?)
    }

    #[wasm_bindgen(js_name = queryWith)]
    pub fn query_with(&self, q: &str, params_json: &str) -> Result<JsValue, JsError> {
        let params: serde_json::Value =
            serde_json::from_str(params_json).map_err(|e| JsError::new(&e.to_string()))?;
        to_js(&self.store.query_with(q, &params).map_err(err)?)
    }

    pub fn replay(&mut self) -> Result<(), JsError> {
        self.store.replay_all().map_err(err)?;
        self.notify(serde_json::json!({ "kind": "replay" }));
        Ok(())
    }

    /// Export all ops as a JSON bundle string (format 1).
    #[wasm_bindgen(js_name = exportJson)]
    pub fn export_json(&self) -> Result<String, JsError> {
        let bundle = self.store.export_all().map_err(err)?;
        serde_json::to_string(&bundle).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Import a JSON export bundle. Returns `{ accepted, skipped }`.
    #[wasm_bindgen(js_name = importJson)]
    pub fn import_json(&mut self, json: &str) -> Result<JsValue, JsError> {
        let bundle: ExportBundle =
            serde_json::from_str(json).map_err(|e| JsError::new(&e.to_string()))?;
        let (accepted, skipped) = self.store.import_bundle(&bundle).map_err(err)?;
        self.notify(
            serde_json::json!({ "kind": "import", "accepted": accepted, "skipped": skipped }),
        );
        to_js(&serde_json::json!({ "accepted": accepted, "skipped": skipped }))
    }

    /// All local op ids (hex), sorted — the sync driver's Hello payload.
    #[wasm_bindgen(js_name = opIds)]
    pub fn op_ids(&self) -> Result<Vec<String>, JsError> {
        Ok(self
            .store
            .list_op_ids()
            .map_err(err)?
            .into_iter()
            .map(hex::encode)
            .collect())
    }

    /// JSON array of the wire ops with the given hex ids (unknown ids are
    /// skipped) — the sync driver's answer to HelloOk.need.
    #[wasm_bindgen(js_name = exportOpsByIds)]
    pub fn export_ops_by_ids(&self, ids: Vec<String>) -> Result<String, JsError> {
        let ids: Vec<[u8; 32]> = ids
            .iter()
            .filter_map(|h| hex::decode(h).ok()?.try_into().ok())
            .collect();
        let ops = self.store.export_ops_by_id(&ids).map_err(err)?;
        serde_json::to_string(&ops).map_err(|e| JsError::new(&e.to_string()))
    }
}
