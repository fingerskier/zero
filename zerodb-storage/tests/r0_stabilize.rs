//! R0.1 stabilization: fail-closed init + set-derived create/tombstone materialization.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use zerodb_core::cbor::Cbor;
use zerodb_core::op::{OpEnvelope, OpTs};
use zerodb_core::sign::sign_op;
use zerodb_storage::{ExportBundle, LocalStore, WireOp, WireTs};

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

#[test]
fn same_id_create_after_tombstone_does_not_resurrect() {
    let path = tmp_db("same-id");
    let mut store = LocalStore::init(&path).unwrap();
    let (node, _) = store.create_node_with_op("Todo").unwrap();
    store.set_lww(&node, "title", "v1").unwrap();
    store.delete_node(&node).unwrap();
    assert!(store.is_deleted(&node).unwrap());

    store.create_node_at(&node, "Todo").unwrap();
    assert!(
        store.is_deleted(&node).unwrap(),
        "a second CreateNode of the same id must not clear the tombstone"
    );
    assert!(
        store.get_lww(&node, "title").unwrap().is_none(),
        "deleted node properties stay hidden"
    );

    store.replay_all().unwrap();
    assert!(store.is_deleted(&node).unwrap());
}

#[test]
fn conflicting_create_labels_converge_on_hlc_order() {
    let source_path = tmp_db("label-src");
    let dest_path = tmp_db("label-dest");
    let mut source = LocalStore::init(&source_path).unwrap();
    let (node, _) = source.create_node_with_op("Alpha").unwrap();
    source.create_node_at(&node, "Beta").unwrap();

    let report = source.inspect(&source_path).unwrap();
    let row = report.nodes.iter().find(|n| n.id == node).unwrap();
    assert_eq!(row.label, "Beta", "later CreateNode wins the label");

    let mut bundle = source.export_all().unwrap();
    bundle.ops.reverse();
    let mut dest = LocalStore::init(&dest_path).unwrap();
    dest.import_bundle(&bundle).unwrap();
    dest.replay_all().unwrap();
    let dest_report = dest.inspect(&dest_path).unwrap();
    let dest_row = dest_report.nodes.iter().find(|n| n.id == node).unwrap();
    assert_eq!(
        dest_row.label, "Beta",
        "label must be set-derived from HLC order, not import order"
    );
}

#[test]
fn shuffled_import_matches_source_after_replay() {
    let source_path = tmp_db("shuffle-src");
    let dest_path = tmp_db("shuffle-dest");
    let mut source = LocalStore::init(&source_path).unwrap();
    let a = source.create_node("Todo").unwrap();
    let b = source.create_node("Note").unwrap();
    source.set_lww(&a, "title", "milk").unwrap();
    source.flag_enable(&a, "done").unwrap();
    source.set_add(&a, "tags", "errand").unwrap();
    source.set_lww(&b, "title", "aside").unwrap();
    source.delete_node(&b).unwrap();

    let mut bundle = source.export_all().unwrap();
    // Deterministic shuffle: rotate by 3, then reverse.
    let n = bundle.ops.len();
    bundle.ops.rotate_left(n.min(3));
    bundle.ops.reverse();

    let mut dest = LocalStore::init(&dest_path).unwrap();
    dest.import_bundle(&bundle).unwrap();
    dest.replay_all().unwrap();
    assert_eq!(
        normalized_nodes(&dest, &dest_path),
        normalized_nodes(&source, &source_path)
    );
}

fn signed_gcounter(store: &LocalStore, node_hex: &str) -> WireOp {
    let ds_hex = store.datastore_id_hex();
    let ds: [u8; 32] = hex::decode(&ds_hex).unwrap().try_into().unwrap();
    let seed = store.identity_seed();
    let node = hex::decode(node_hex).unwrap();
    let body = Cbor::Map(vec![
        ("crdt".into(), Cbor::Text("gcounter".into())),
        ("n".into(), Cbor::Uint(1)),
        ("node".into(), Cbor::Bytes(node)),
        ("op".into(), Cbor::Text("inc".into())),
        ("path".into(), Cbor::Text("title".into())),
    ]);
    let (author_pk, _) = sign_op(&seed, &[0; 32]);
    let author = *blake3::hash(&author_pk).as_bytes();
    let envelope = OpEnvelope {
        v: 1,
        ds,
        ep: 1,
        author,
        ts: OpTs {
            physical_ms: 9_000_000,
            logical: 0,
        },
        deps: vec![],
        grp: None,
        kind: 3,
        body,
    };
    let id = envelope.op_id().unwrap();
    let (_, sig) = sign_op(&seed, &id);
    WireOp {
        id: hex::encode(id),
        v: 1,
        ds: ds_hex,
        ep: 1,
        author: hex::encode(author),
        author_pk: hex::encode(author_pk),
        ts: WireTs { p: 9_000_000, l: 0 },
        deps: vec![],
        grp: None,
        kind: 3,
        body: serde_json::json!({
            "node": node_hex,
            "path": "title",
            "crdt": "gcounter",
            "op": "inc",
            "n": 1,
        }),
        sig: hex::encode(sig),
    }
}

#[test]
fn import_rejects_crdt_that_breaks_schema_pin() {
    let dst_path = tmp_db("pin-dst");
    let mut dst = LocalStore::init(&dst_path).unwrap();
    dst.apply_schema_json(r#"{"nodes":{"Todo":{"props":{"title":"lww"}}}}"#)
        .unwrap();
    let node = dst.create_node("Todo").unwrap();
    let mut bundle = dst.export_all().unwrap();
    bundle.ops.push(signed_gcounter(&dst, &node));
    let err = dst
        .import_bundle(&bundle)
        .expect_err("pinned title:lww must reject a remote gcounter on title");
    assert!(err.to_string().contains("schema pin"), "got: {err}");
}
