//! Experimental M2 Node/NAPI binding over `zerodb-storage` LocalStore.
//!
//! Not a format freeze. API mirrors the M1 local slice for open/mutate/inspect.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use zerodb_storage::LocalStore;

fn map_err(e: impl ToString) -> Error {
    Error::from_reason(e.to_string())
}

type Subscriber =
    ThreadsafeFunction<serde_json::Value, (), serde_json::Value, Status, false, true>;

/// SQLite-backed ZeroDB handle for Node (M2 vertical).
#[napi]
pub struct Database {
    inner: Mutex<Option<LocalStore>>,
    subs: Mutex<HashMap<u32, Subscriber>>,
    next_sub: Mutex<u32>,
}

impl Database {
    fn emit(&self, event: serde_json::Value) {
        if let Ok(subs) = self.subs.lock() {
            for tsfn in subs.values() {
                tsfn.call(event.clone(), ThreadsafeFunctionCallMode::NonBlocking);
            }
        }
    }

    fn emit_op(&self, method: &str, node: &str, key: Option<&str>, op_id: Option<&str>) {
        self.emit(serde_json::json!({
            "kind": "op",
            "method": method,
            "node": node,
            "key": key,
            "opId": op_id,
        }));
    }

    fn with_store<R>(&self, f: impl FnOnce(&LocalStore) -> Result<R>) -> Result<R> {
        let guard = self.inner.lock().map_err(|e| map_err(e.to_string()))?;
        let store = guard
            .as_ref()
            .ok_or_else(|| Error::from_reason("database is closed"))?;
        f(store)
    }

    fn with_store_mut<R>(&self, f: impl FnOnce(&mut LocalStore) -> Result<R>) -> Result<R> {
        let mut guard = self.inner.lock().map_err(|e| map_err(e.to_string()))?;
        let store = guard
            .as_mut()
            .ok_or_else(|| Error::from_reason("database is closed"))?;
        f(store)
    }
}

#[napi]
impl Database {
    /// Create a new database file and open it.
    #[napi(factory)]
    pub fn init(path: String) -> Result<Self> {
        let store = LocalStore::init(Path::new(&path)).map_err(map_err)?;
        Ok(Self {
            inner: Mutex::new(Some(store)),
            subs: Mutex::new(HashMap::new()),
            next_sub: Mutex::new(0),
        })
    }

    /// Open an existing database file.
    #[napi(factory)]
    pub fn open(path: String) -> Result<Self> {
        let store = LocalStore::open(Path::new(&path)).map_err(map_err)?;
        Ok(Self {
            inner: Mutex::new(Some(store)),
            subs: Mutex::new(HashMap::new()),
            next_sub: Mutex::new(0),
        })
    }

    /// Release the SQLite connection (required before deleting the file on Windows).
    #[napi]
    pub fn close(&self) -> Result<()> {
        let mut guard = self.inner.lock().map_err(|e| map_err(e.to_string()))?;
        *guard = None;
        if let Ok(mut subs) = self.subs.lock() {
            subs.clear();
        }
        Ok(())
    }

    /// Register a change callback. Returns a subscription id for `unsubscribe`.
    ///
    /// Events (JSON): `{kind:'op', method, node, key?, opId}` for local
    /// mutations, `{kind:'import', accepted, skipped}`, `{kind:'replay'}`.
    /// Delivery is asynchronous (next event-loop tick). Experimental surface;
    /// wire/event shapes are versioned-experimental and may change pre-freeze.
    #[napi]
    pub fn subscribe(&self, callback: Function<serde_json::Value, ()>) -> Result<u32> {
        let tsfn = callback
            .build_threadsafe_function()
            .weak::<true>()
            .build_callback(|ctx| Ok(ctx.value))?;
        let mut next = self.next_sub.lock().map_err(|e| map_err(e.to_string()))?;
        let id = *next;
        *next += 1;
        self.subs
            .lock()
            .map_err(|e| map_err(e.to_string()))?
            .insert(id, tsfn);
        Ok(id)
    }

    /// Remove a subscription; unknown ids are a no-op.
    #[napi]
    pub fn unsubscribe(&self, id: u32) -> Result<()> {
        self.subs
            .lock()
            .map_err(|e| map_err(e.to_string()))?
            .remove(&id);
        Ok(())
    }

    #[napi]
    pub fn datastore_id(&self) -> Result<String> {
        self.with_store(|store| Ok(store.datastore_id_hex()))
    }

    #[napi]
    pub fn peer_id(&self) -> Result<String> {
        self.with_store(|store| Ok(store.author_hex()))
    }

    #[napi]
    pub fn op_count(&self) -> Result<u32> {
        self.with_store(|store| Ok(store.op_count().map_err(map_err)? as u32))
    }

    #[napi]
    pub fn create_node(&self, label: String) -> Result<String> {
        let (id, op) =
            self.with_store_mut(|store| store.create_node_with_op(&label).map_err(map_err))?;
        self.emit_op("createNode", &id, None, Some(&op));
        Ok(id)
    }

    #[napi]
    pub fn delete_node(&self, node: String) -> Result<String> {
        let op = self.with_store_mut(|store| store.delete_node(&node).map_err(map_err))?;
        self.emit_op("deleteNode", &node, None, Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn set_lww(&self, node: String, key: String, value: String) -> Result<String> {
        let op = self.with_store_mut(|store| store.set_lww(&node, &key, &value).map_err(map_err))?;
        self.emit_op("setLww", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn get_lww(&self, node: String, key: String) -> Result<Option<String>> {
        self.with_store(|store| store.get_lww(&node, &key).map_err(map_err))
    }

    /// Get a materialized property as JSON (string, number, bool, array, or null).
    #[napi]
    pub fn get_prop(&self, node: String, key: String) -> Result<serde_json::Value> {
        self.with_store(
            |store| match store.get_prop(&node, &key).map_err(map_err)? {
                Some(v) => Ok(v),
                None => Ok(serde_json::Value::Null),
            },
        )
    }

    #[napi]
    pub fn gcounter_inc(&self, node: String, key: String, n: u32) -> Result<String> {
        let op = self
            .with_store_mut(|store| store.gcounter_inc(&node, &key, n as u64).map_err(map_err))?;
        self.emit_op("gcounterInc", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn counter_inc(&self, node: String, key: String, n: u32) -> Result<String> {
        let op = self
            .with_store_mut(|store| store.counter_inc(&node, &key, n as u64).map_err(map_err))?;
        self.emit_op("counterInc", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn counter_dec(&self, node: String, key: String, n: u32) -> Result<String> {
        let op = self
            .with_store_mut(|store| store.counter_dec(&node, &key, n as u64).map_err(map_err))?;
        self.emit_op("counterDec", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn set_add(&self, node: String, key: String, value: String) -> Result<String> {
        let op =
            self.with_store_mut(|store| store.set_add(&node, &key, &value).map_err(map_err))?;
        self.emit_op("setAdd", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn set_remove(&self, node: String, key: String, value: String) -> Result<String> {
        let op =
            self.with_store_mut(|store| store.set_remove(&node, &key, &value).map_err(map_err))?;
        self.emit_op("setRemove", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn flag_enable(&self, node: String, key: String) -> Result<String> {
        let op = self.with_store_mut(|store| store.flag_enable(&node, &key).map_err(map_err))?;
        self.emit_op("flagEnable", &node, Some(&key), Some(&op));
        Ok(op)
    }

    #[napi]
    pub fn flag_disable(&self, node: String, key: String) -> Result<String> {
        let op = self.with_store_mut(|store| store.flag_disable(&node, &key).map_err(map_err))?;
        self.emit_op("flagDisable", &node, Some(&key), Some(&op));
        Ok(op)
    }

    /// List nodes as JSON array of `{ id, label, deleted }`.
    #[napi]
    pub fn list_nodes(&self) -> Result<serde_json::Value> {
        self.with_store(|store| {
            let nodes = store.list_nodes().map_err(map_err)?;
            let arr: Vec<serde_json::Value> = nodes
                .into_iter()
                .map(|(id, label, deleted)| {
                    serde_json::json!({ "id": id, "label": label, "deleted": deleted })
                })
                .collect();
            Ok(serde_json::Value::Array(arr))
        })
    }

    /// Run an O3 minimal query (`MATCH/WHERE/RETURN/ORDER BY/LIMIT`).
    /// Returns a JSON array of row objects keyed by return item (e.g. `"t.title"`).
    #[napi]
    pub fn query(&self, q: String) -> Result<serde_json::Value> {
        self.with_store(|store| store.query(&q).map_err(map_err))
    }

    /// Full inspect report as JSON (path is reported as the given string).
    #[napi]
    pub fn inspect(&self, path: String) -> Result<serde_json::Value> {
        self.with_store(|store| {
            let report = store.inspect(Path::new(&path)).map_err(map_err)?;
            serde_json::to_value(report).map_err(|e| map_err(e.to_string()))
        })
    }

    #[napi]
    pub fn replay(&self) -> Result<()> {
        self.with_store_mut(|store| store.replay_all().map_err(map_err))?;
        self.emit(serde_json::json!({ "kind": "replay" }));
        Ok(())
    }

    /// Export all ops as a JSON bundle string (format 1).
    #[napi]
    pub fn export_json(&self) -> Result<String> {
        self.with_store(|store| {
            let bundle = store.export_all().map_err(map_err)?;
            serde_json::to_string(&bundle).map_err(|e| map_err(e.to_string()))
        })
    }

    /// Import a JSON export bundle. Returns `{ accepted, skipped }`.
    #[napi]
    pub fn import_json(&self, json: String) -> Result<serde_json::Value> {
        self.with_store_mut(|store| {
            let bundle: zerodb_storage::ExportBundle =
                serde_json::from_str(&json).map_err(|e| map_err(e.to_string()))?;
            let (accepted, skipped) = store.import_bundle(&bundle).map_err(map_err)?;
            Ok(serde_json::json!({ "accepted": accepted, "skipped": skipped }))
        })
        .inspect(|result| self.emit(serde_json::json!({ "kind": "import", "accepted": result["accepted"], "skipped": result["skipped"] })))
    }
}
