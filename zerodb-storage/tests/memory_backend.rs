//! MemoryBackend runs the representative store behaviors used by the wasm
//! browser peer: init/open with an injected backend, CRUD + CRDT merge,
//! export/import round-trip, fresh replay, create/tombstone permutation,
//! and transactional rollback.

use zerodb_storage::{BackendTxn, LocalStore, MemoryBackend, StoreBackend, StoreError};

fn mem_store() -> LocalStore<MemoryBackend> {
    LocalStore::init_with_backend(MemoryBackend::new()).unwrap()
}

#[test]
fn crud_and_crdts() {
    let mut s = mem_store();
    let node = s.create_node("Todo").unwrap();
    s.set_lww(&node, "title", "hello").unwrap();
    assert_eq!(s.get_lww(&node, "title").unwrap().as_deref(), Some("hello"));
    s.counter_inc(&node, "count", 3).unwrap();
    s.counter_dec(&node, "count", 1).unwrap();
    assert_eq!(
        s.get_prop(&node, "count").unwrap(),
        Some(serde_json::json!(2))
    );
    s.set_add(&node, "tags", "x").unwrap();
    s.set_add(&node, "tags", "y").unwrap();
    s.set_remove(&node, "tags", "x").unwrap();
    assert_eq!(
        s.get_prop(&node, "tags").unwrap(),
        Some(serde_json::json!(["y"]))
    );
    s.flag_enable(&node, "done").unwrap();
    assert_eq!(
        s.get_prop(&node, "done").unwrap(),
        Some(serde_json::json!(true))
    );
    let nodes = s.list_nodes().unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].1, "Todo");
    assert_eq!(s.op_count().unwrap(), 8);
}

#[test]
fn export_import_round_trip() {
    let mut a = mem_store();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "from-a").unwrap();

    // Fresh empty store adopts A's datastore from the bundle.
    let mut b = mem_store();
    let bundle = a.export_all().unwrap();
    let (accepted, skipped) = b.import_bundle(&bundle).unwrap();
    assert_eq!((accepted, skipped), (2, 0));
    assert_eq!(b.datastore_id_hex(), a.datastore_id_hex());
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );

    // Re-import is idempotent.
    let (accepted, skipped) = b.import_bundle(&bundle).unwrap();
    assert_eq!((accepted, skipped), (0, 2));
}

#[test]
fn replay_rebuilds_materialization() {
    let mut s = mem_store();
    let node = s.create_node("Todo").unwrap();
    s.set_lww(&node, "title", "kept").unwrap();
    let gone = s.create_node("Gone").unwrap();
    s.delete_node(&gone).unwrap();
    s.replay_all().unwrap();
    assert_eq!(s.get_lww(&node, "title").unwrap().as_deref(), Some("kept"));
    assert!(s.is_deleted(&gone).unwrap());
    let nodes = s.list_nodes().unwrap();
    assert_eq!(nodes.len(), 2);
}

/// Tombstone-before-create arrival still deletes (set-derived node state).
#[test]
fn create_tombstone_permutation() {
    let mut a = mem_store();
    let node = a.create_node("Todo").unwrap();
    a.delete_node(&node).unwrap();
    let bundle = a.export_all().unwrap();
    assert_eq!(bundle.ops.len(), 2);

    let mut reversed = bundle.clone();
    reversed.ops.reverse();
    let mut b = mem_store();
    b.import_bundle(&reversed).unwrap();
    assert!(b.is_deleted(&node).unwrap());
}

#[test]
fn with_txn_rolls_back_on_error() {
    let mut backend = MemoryBackend::new();
    backend.meta_set("k", b"before").unwrap();
    let err = backend.with_txn(&mut |tx| {
        tx.meta_set("k", b"after")?;
        tx.node_upsert("n1", "Todo", false)?;
        Err(StoreError::Invalid("boom".into()))
    });
    assert!(err.is_err());
    assert_eq!(
        backend.meta_get("k").unwrap().as_deref(),
        Some(&b"before"[..])
    );
    assert_eq!(backend.node_count().unwrap(), 0);
}

#[test]
fn init_from_seed_restores_identity() {
    let mut a = mem_store();
    let node = a.create_node("Todo").unwrap();
    let seed = a.identity_seed();
    let ds = a.export_all().unwrap().datastore_id;
    let peer = a.author_hex();

    // "Reload": same seed + ds on a fresh backend, then re-import the bundle.
    let mut b = LocalStore::init_with_backend_from_seed(
        MemoryBackend::new(),
        &seed,
        &hex::decode(&ds).unwrap().try_into().unwrap(),
    )
    .unwrap();
    assert_eq!(b.author_hex(), peer);
    assert_eq!(b.datastore_id_hex(), ds);
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    assert_eq!(b.list_nodes().unwrap()[0].0, node);

    // Fail-closed: refuse re-init on a nonempty backend.
    let backend = into_backend_with_meta();
    assert!(LocalStore::init_with_backend_from_seed(backend, &seed, &[0u8; 32]).is_err());
}

fn into_backend_with_meta() -> MemoryBackend {
    let backend = MemoryBackend::new();
    backend.meta_set("seed", &[1u8; 32]).unwrap();
    backend
}
