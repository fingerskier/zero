//! R0.1 stabilization: fail-closed init + set-derived create/tombstone materialization.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use zerodb_storage::{ExportBundle, LocalStore};

const KIND_CREATE_NODE: u64 = 1;
const KIND_TOMBSTONE: u64 = 4;

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("r0-stabilize-{name}-{nonce}.sqlite"));
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
    path
}

fn normalized_nodes(store: &LocalStore, path: &Path) -> Value {
    let report = store.inspect(path).unwrap();
    let mut nodes = report.nodes;
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    serde_json::json!(
        nodes
            .into_iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "label": n.label,
                    "deleted": n.deleted,
                })
            })
            .collect::<Vec<_>>()
    )
}

fn order_create_first(mut bundle: ExportBundle) -> ExportBundle {
    bundle.ops.sort_by_key(|op| match op.kind {
        KIND_CREATE_NODE => 0u8,
        KIND_TOMBSTONE => 1,
        _ => 2,
    });
    bundle
}

fn order_tombstone_first(mut bundle: ExportBundle) -> ExportBundle {
    bundle.ops.sort_by_key(|op| match op.kind {
        KIND_TOMBSTONE => 0u8,
        KIND_CREATE_NODE => 1,
        _ => 2,
    });
    bundle
}

// --- Fail-closed init ---

#[test]
fn init_rejects_already_initialized_empty_db() {
    let path = tmp_db("init-empty");
    let store = LocalStore::init(&path).unwrap();
    let ds = store.datastore_id_hex();
    drop(store);

    let err = match LocalStore::init(&path) {
        Ok(_) => panic!("re-init of initialized empty DB must fail"),
        Err(e) => e,
    };
    assert!(
        err.to_string().to_lowercase().contains("already")
            || err.to_string().to_lowercase().contains("initialized"),
        "error should mention already initialized, got: {err}"
    );

    let reopened = LocalStore::open(&path).unwrap();
    assert_eq!(reopened.datastore_id_hex(), ds);
    assert_eq!(reopened.op_count().unwrap(), 0);
}

#[test]
fn init_rejects_nonempty_db_without_rekeying() {
    let path = tmp_db("init-nonempty");
    let mut store = LocalStore::init(&path).unwrap();
    let ds = store.datastore_id_hex();
    let node = store.create_node("Todo").unwrap();
    store.set_lww(&node, "title", "keep").unwrap();
    let ops = store.op_count().unwrap();
    drop(store);

    let err = match LocalStore::init(&path) {
        Ok(_) => panic!("re-init of nonempty DB must fail"),
        Err(e) => e,
    };
    assert!(
        err.to_string().to_lowercase().contains("already")
            || err.to_string().to_lowercase().contains("initialized"),
        "error should mention already initialized, got: {err}"
    );

    let reopened = LocalStore::open(&path).unwrap();
    assert_eq!(
        reopened.datastore_id_hex(),
        ds,
        "fail-closed init must not replace datastore identity"
    );
    assert_eq!(reopened.op_count().unwrap(), ops);
    assert_eq!(
        reopened.get_lww(&node, "title").unwrap().as_deref(),
        Some("keep")
    );
}

// --- Create/tombstone arrival order (SEC / I-1) ---

fn source_with_deleted_node(path: &Path) -> (LocalStore, String) {
    let mut source = LocalStore::init(path).unwrap();
    let node = source.create_node("FlowProbe").unwrap();
    source.set_lww(&node, "title", "gone").unwrap();
    source.delete_node(&node).unwrap();
    (source, node)
}

#[test]
fn create_then_tombstone_import_marks_node_deleted() {
    let source_path = tmp_db("ct-source");
    let dest_path = tmp_db("ct-dest");
    let (source, node) = source_with_deleted_node(&source_path);
    let bundle = order_create_first(source.export_all().unwrap());
    let create_pos = bundle
        .ops
        .iter()
        .position(|o| o.kind == KIND_CREATE_NODE)
        .unwrap();
    let tomb_pos = bundle
        .ops
        .iter()
        .position(|o| o.kind == KIND_TOMBSTONE)
        .unwrap();
    assert!(create_pos < tomb_pos, "setup: create before tombstone");

    let mut dest = LocalStore::init(&dest_path).unwrap();
    dest.import_bundle(&bundle).unwrap();
    assert!(
        dest.is_deleted(&node).unwrap(),
        "create-then-tombstone must materialize deleted"
    );
    assert_eq!(
        normalized_nodes(&dest, &dest_path),
        normalized_nodes(&source, &source_path)
    );
}

#[test]
fn tombstone_then_create_import_marks_node_deleted() {
    let source_path = tmp_db("tc-source");
    let dest_path = tmp_db("tc-dest");
    let (source, node) = source_with_deleted_node(&source_path);
    let bundle = order_tombstone_first(source.export_all().unwrap());
    let create_pos = bundle
        .ops
        .iter()
        .position(|o| o.kind == KIND_CREATE_NODE)
        .unwrap();
    let tomb_pos = bundle
        .ops
        .iter()
        .position(|o| o.kind == KIND_TOMBSTONE)
        .unwrap();
    assert!(
        tomb_pos < create_pos,
        "test setup must deliver tombstone before create"
    );

    let mut dest = LocalStore::init(&dest_path).unwrap();
    dest.import_bundle(&bundle).unwrap();
    assert!(
        dest.is_deleted(&node).unwrap(),
        "tombstone-before-create must still materialize deleted (not resurrect)"
    );
    assert_eq!(
        normalized_nodes(&dest, &dest_path),
        normalized_nodes(&source, &source_path),
        "materialized state must depend only on the op set, not arrival order"
    );
}

#[test]
fn create_tombstone_permutations_converge_and_survive_replay() {
    let source_path = tmp_db("perm-source");
    let dest_ct = tmp_db("perm-ct");
    let dest_tc = tmp_db("perm-tc");
    let (source, _node) = source_with_deleted_node(&source_path);
    let full = source.export_all().unwrap();

    let mut a = LocalStore::init(&dest_ct).unwrap();
    a.import_bundle(&order_create_first(full.clone())).unwrap();
    let mut b = LocalStore::init(&dest_tc).unwrap();
    b.import_bundle(&order_tombstone_first(full)).unwrap();

    let expected = normalized_nodes(&source, &source_path);
    assert_eq!(normalized_nodes(&a, &dest_ct), expected);
    assert_eq!(normalized_nodes(&b, &dest_tc), expected);

    a.replay_all().unwrap();
    b.replay_all().unwrap();
    assert_eq!(
        normalized_nodes(&a, &dest_ct),
        expected,
        "replay must preserve set-derived deleted state (create-first path)"
    );
    assert_eq!(
        normalized_nodes(&b, &dest_tc),
        expected,
        "replay must preserve set-derived deleted state (tombstone-first path)"
    );
    assert_eq!(
        normalized_nodes(&a, &dest_ct),
        normalized_nodes(&b, &dest_tc)
    );
}
