//! E4 layer-2 crash matrix: deterministic named failpoints at every SQLite
//! commit-pipeline boundary, for every commit path.
//!
//! WAL.md §3 names layer-1 crash points; this suite maps them onto the real
//! SQLite transaction boundaries (`SqliteBackend::with_txn`). In layer 2 the
//! whole pipeline (op append, materialization, HLC persist) is one SQLite
//! transaction, so a crash at ANY interior point must recover to "nothing
//! happened" — an injected error drops the open `rusqlite::Transaction`,
//! which is a ROLLBACK, the same durable outcome as a process death before
//! COMMIT.
//!
//! Failpoint → WAL.md §3 mapping:
//!
//! | failpoint          | WAL layer-1 point                | pipeline position                       |
//! |--------------------|----------------------------------|-----------------------------------------|
//! | `before-txn`       | `CRASH_START`                    | before the transaction opens            |
//! | `after-op-insert`  | `CRASH_AFTER_APPEND`/`AFTER_SYNC`| op row staged, nothing else             |
//! | `before-hlc-persist`| `CRASH_AFTER_APPLY`             | materialization staged, HLC not (a.k.a. after-materialize) |
//! | `after-hlc-persist`| `CRASH_AFTER_HLC`                | HLC staged, COMMIT still pending        |
//! | `before-commit`    | instant before durable COMMIT    | full pipeline staged                    |
//!
//! For every point × path (single-op `commit_local`, multi-op `atomic_group`,
//! `import_bundle` ingest) the matrix asserts: the operation errors; op count
//! is unchanged; materialized state is identical to the pre-attempt state, to
//! the state after reopening from disk, and to a fresh `replay_all`; the HLC
//! is not corrupted (the next normal commit succeeds).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use zerodb_storage::LocalStore;

const POINTS: [&str; 5] = [
    "before-txn",
    "after-op-insert",
    "before-hlc-persist",
    "after-hlc-persist",
    "before-commit",
];

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("e4-matrix-{name}-{nonce}.sqlite"));
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
    path
}

/// Full observable projection: op count + nodes (with props) + edges.
fn snapshot(store: &LocalStore, path: &Path) -> Value {
    let report = store.inspect(path).unwrap();
    let mut nodes = report.nodes;
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let mut edges = report.edges;
    edges.sort_by(|a, b| a.id.cmp(&b.id));
    serde_json::json!({
        "ops": report.ops,
        "nodes": nodes.iter().map(|n| serde_json::json!({
            "id": n.id, "label": n.label, "deleted": n.deleted, "props": n.props,
        })).collect::<Vec<_>>(),
        "edges": edges.iter().map(|e| serde_json::json!({
            "id": e.id, "label": e.label, "src": e.src, "dst": e.dst,
            "deleted": e.deleted, "visible": e.visible,
        })).collect::<Vec<_>>(),
    })
}

/// After a failed injection: full invariant recovery for `store` at `path`.
/// Returns the store (reopened from disk) for the next iteration.
fn assert_recovered(path: &Path, pre: &Value, point: &str) -> LocalStore {
    // Reopen from disk: nothing from the failed attempt may be durable.
    let mut store = LocalStore::open(path).unwrap();
    assert_eq!(
        &snapshot(&store, path),
        pre,
        "state after reopen must equal pre-attempt state (point {point})"
    );
    // Fresh replay from the oplog materializes the identical state.
    store.replay_all().unwrap();
    assert_eq!(
        &snapshot(&store, path),
        pre,
        "fresh replay must equal pre-attempt state (point {point})"
    );
    store
}

#[test]
fn e4_commit_local_crash_matrix() {
    let path = tmp_db("local");
    let mut store = LocalStore::init(&path).unwrap();
    let node = store.create_node("Todo").unwrap();
    store.set_lww(&node, "title", "baseline").unwrap();

    for point in POINTS {
        let pre = snapshot(&store, &path);
        store.backend_mut().set_failpoint(Some(point));
        let err = store
            .set_lww(&node, "title", &format!("crash-{point}"))
            .expect_err("armed failpoint must fail the commit");
        assert!(
            err.to_string().contains("failpoint"),
            "unexpected error at {point}: {err}"
        );
        store.backend_mut().set_failpoint(None);
        assert_eq!(store.op_count().unwrap(), pre["ops"].as_u64().unwrap());
        assert_eq!(
            snapshot(&store, &path),
            pre,
            "in-memory view (point {point})"
        );
        drop(store);
        store = assert_recovered(&path, &pre, point);
        // HLC not corrupted: next normal commit succeeds and is durable.
        store
            .set_lww(&node, "title", &format!("ok-{point}"))
            .unwrap();
        assert_eq!(
            store.get_lww(&node, "title").unwrap().as_deref(),
            Some(format!("ok-{point}").as_str())
        );
    }
}

#[test]
fn e4_atomic_group_crash_matrix() {
    let path = tmp_db("group");
    let mut store = LocalStore::init(&path).unwrap();
    let anchor = store.create_node("Todo").unwrap();
    store.set_lww(&anchor, "title", "baseline").unwrap();

    for point in POINTS {
        let pre = snapshot(&store, &path);
        store.backend_mut().set_failpoint(Some(point));
        let err = store
            .atomic_group(|g| {
                let n = g.create_node("Todo")?;
                g.set_lww(&n, "title", "grouped")?;
                g.set_add(&n, "tags", "a")?;
                g.flag_enable(&n, "done")?;
                g.counter_inc(&n, "voteScore", 1)?;
                Ok(n)
            })
            .expect_err("armed failpoint must fail the group commit");
        assert!(err.to_string().contains("failpoint"), "at {point}: {err}");
        store.backend_mut().set_failpoint(None);
        // Group seal: mid-group failure rolls back the WHOLE group — no
        // member op, node, or prop survives.
        assert_eq!(store.op_count().unwrap(), pre["ops"].as_u64().unwrap());
        assert_eq!(
            snapshot(&store, &path),
            pre,
            "in-memory view (point {point})"
        );
        // No partial group members in the export either.
        let grouped: usize = store
            .export_all()
            .unwrap()
            .ops
            .iter()
            .filter(|op| op.grp.is_some())
            .count();
        assert_eq!(
            grouped, 0,
            "no partial group members durable (point {point})"
        );
        drop(store);
        store = assert_recovered(&path, &pre, point);
    }

    // After the whole matrix, a normal group still commits atomically.
    let n = store
        .atomic_group(|g| {
            let n = g.create_node("Todo")?;
            g.set_lww(&n, "title", "finally")?;
            Ok(n)
        })
        .unwrap();
    assert_eq!(
        store.get_lww(&n, "title").unwrap().as_deref(),
        Some("finally")
    );
}

#[test]
fn e4_import_bundle_crash_matrix() {
    // Source peer with a representative op mix (create/prop/edge/tombstone).
    let source_path = tmp_db("import-src");
    let mut source = LocalStore::init(&source_path).unwrap();
    let a = source.create_node("Todo").unwrap();
    let b = source.create_node("Note").unwrap();
    source.set_lww(&a, "title", "milk").unwrap();
    source.create_edge("NOTATES", &b, &a).unwrap();
    source.delete_node(&b).unwrap();
    let bundle = source.export_all().unwrap();

    let dest_path = tmp_db("import-dst");
    let mut dest = LocalStore::init(&dest_path).unwrap();
    let dest_ds = dest.datastore_id_hex();

    for point in POINTS {
        let pre = snapshot(&dest, &dest_path);
        dest.backend_mut().set_failpoint(Some(point));
        let err = dest
            .import_bundle(&bundle)
            .expect_err("armed failpoint must fail the ingest");
        assert!(err.to_string().contains("failpoint"), "at {point}: {err}");
        dest.backend_mut().set_failpoint(None);
        assert_eq!(
            dest.op_count().unwrap(),
            0,
            "no partial ingest (point {point})"
        );
        assert_eq!(
            dest.datastore_id_hex(),
            dest_ds,
            "failed adopt must roll back datastore identity (point {point})"
        );
        assert_eq!(
            snapshot(&dest, &dest_path),
            pre,
            "in-memory view (point {point})"
        );
        drop(dest);
        dest = assert_recovered(&dest_path, &pre, point);
    }

    // Next normal ingest succeeds and converges with the source.
    let (accepted, skipped) = dest.import_bundle(&bundle).unwrap();
    assert_eq!(accepted as usize, bundle.ops.len());
    assert_eq!(skipped, 0);
    assert_eq!(
        snapshot(&dest, &dest_path)["nodes"],
        snapshot(&source, &source_path)["nodes"]
    );
    assert_eq!(
        snapshot(&dest, &dest_path)["edges"],
        snapshot(&source, &source_path)["edges"]
    );
}
