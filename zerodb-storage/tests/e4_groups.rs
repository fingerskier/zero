//! M1-e4 first slice: multi-op local group atomicity (SQLite transaction).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use zerodb_storage::{LocalStore, StoreError};

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("e4-{name}-{nonce}.sqlite"));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    path
}

#[test]
fn e4_atomic_group_rolls_back_all_ops_on_error() {
    let path = tmp_db("rollback");
    let mut store = LocalStore::init(&path).unwrap();
    assert_eq!(store.op_count().unwrap(), 0);

    let err = store
        .atomic_group(|g| -> Result<(), StoreError> {
            let node = g.create_node("Todo")?;
            g.set_lww(&node, "title", "should-not-persist")?;
            g.set_add(&node, "tags", "a")?;
            g.flag_enable(&node, "done")?;
            Err(StoreError::Invalid("injected failure".into()))
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("injected"),
        "unexpected error: {err}"
    );

    assert_eq!(
        store.op_count().unwrap(),
        0,
        "no ops durable after group failure"
    );
    assert!(
        store.list_nodes().unwrap().is_empty(),
        "no nodes durable after group failure"
    );
}

#[test]
fn e4_atomic_group_commits_all_ops_together() {
    let path = tmp_db("commit");
    let mut store = LocalStore::init(&path).unwrap();

    let node = store
        .atomic_group(|g| {
            let node = g.create_node("Todo")?;
            g.set_lww(&node, "title", "milk")?;
            g.set_add(&node, "tags", "food")?;
            g.set_add(&node, "tags", "urgent")?;
            g.flag_enable(&node, "done")?;
            g.counter_inc(&node, "voteScore", 2)?;
            Ok(node)
        })
        .unwrap();

    assert!(store.op_count().unwrap() >= 6);
    assert_eq!(
        store.get_lww(&node, "title").unwrap().as_deref(),
        Some("milk")
    );
    assert_eq!(
        store.get_prop(&node, "tags").unwrap(),
        Some(serde_json::json!(["food", "urgent"]))
    );
    assert_eq!(
        store.get_prop(&node, "done").unwrap(),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        store.get_prop(&node, "voteScore").unwrap(),
        Some(serde_json::json!(2))
    );

    // Restart + replay preserve group effects.
    drop(store);
    let mut store = LocalStore::open(&path).unwrap();
    store.replay_all().unwrap();
    assert_eq!(
        store.get_lww(&node, "title").unwrap().as_deref(),
        Some("milk")
    );
    assert_eq!(
        store.get_prop(&node, "done").unwrap(),
        Some(serde_json::json!(true))
    );
}

#[test]
fn e4_grouped_ops_share_group_id_in_export() {
    let path = tmp_db("grp-field");
    let mut store = LocalStore::init(&path).unwrap();
    store
        .atomic_group(|g| {
            let node = g.create_node("Todo")?;
            g.set_lww(&node, "title", "x")?;
            Ok(())
        })
        .unwrap();

    let bundle = store.export_all().unwrap();
    // All ops from this group must carry the same non-null group id once WireOp exposes it.
    // Until the export surface includes grp, at least op count is 2 and state is consistent.
    assert_eq!(bundle.ops.len(), 2);
    let grps: Vec<Option<String>> = bundle.ops.iter().map(|op| op.grp.clone()).collect();
    assert!(
        grps.iter().all(|g| g.is_some()),
        "grouped ops must export grp: {grps:?}"
    );
    assert_eq!(
        grps[0], grps[1],
        "members of one atomic group share GroupId"
    );
}
