//! Experimental M2 Node/NAPI binding over `zerodb-storage` LocalStore.
//!
//! Not a format freeze. API mirrors the M1 local slice for open/mutate/inspect.

use std::path::Path;
use std::sync::Mutex;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use zerodb_storage::LocalStore;

fn map_err(e: impl ToString) -> Error {
    Error::from_reason(e.to_string())
}

/// SQLite-backed ZeroDB handle for Node (M2 vertical).
#[napi]
pub struct Database {
    inner: Mutex<Option<LocalStore>>,
}

impl Database {
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
        })
    }

    /// Open an existing database file.
    #[napi(factory)]
    pub fn open(path: String) -> Result<Self> {
        let store = LocalStore::open(Path::new(&path)).map_err(map_err)?;
        Ok(Self {
            inner: Mutex::new(Some(store)),
        })
    }

    /// Release the SQLite connection (required before deleting the file on Windows).
    #[napi]
    pub fn close(&self) -> Result<()> {
        let mut guard = self.inner.lock().map_err(|e| map_err(e.to_string()))?;
        *guard = None;
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
        self.with_store_mut(|store| store.create_node(&label).map_err(map_err))
    }

    #[napi]
    pub fn delete_node(&self, node: String) -> Result<String> {
        self.with_store_mut(|store| store.delete_node(&node).map_err(map_err))
    }

    #[napi]
    pub fn set_lww(&self, node: String, key: String, value: String) -> Result<String> {
        self.with_store_mut(|store| store.set_lww(&node, &key, &value).map_err(map_err))
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
        self.with_store_mut(|store| store.gcounter_inc(&node, &key, n as u64).map_err(map_err))
    }

    #[napi]
    pub fn counter_inc(&self, node: String, key: String, n: u32) -> Result<String> {
        self.with_store_mut(|store| store.counter_inc(&node, &key, n as u64).map_err(map_err))
    }

    #[napi]
    pub fn counter_dec(&self, node: String, key: String, n: u32) -> Result<String> {
        self.with_store_mut(|store| store.counter_dec(&node, &key, n as u64).map_err(map_err))
    }

    #[napi]
    pub fn set_add(&self, node: String, key: String, value: String) -> Result<String> {
        self.with_store_mut(|store| store.set_add(&node, &key, &value).map_err(map_err))
    }

    #[napi]
    pub fn set_remove(&self, node: String, key: String, value: String) -> Result<String> {
        self.with_store_mut(|store| store.set_remove(&node, &key, &value).map_err(map_err))
    }

    #[napi]
    pub fn flag_enable(&self, node: String, key: String) -> Result<String> {
        self.with_store_mut(|store| store.flag_enable(&node, &key).map_err(map_err))
    }

    #[napi]
    pub fn flag_disable(&self, node: String, key: String) -> Result<String> {
        self.with_store_mut(|store| store.flag_disable(&node, &key).map_err(map_err))
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
        self.with_store_mut(|store| store.replay_all().map_err(map_err))
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
    }
}
