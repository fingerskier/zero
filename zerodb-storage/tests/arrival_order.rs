//! Arrival-order regressions for node creation and property materialization.

use std::path::{Path, PathBuf};

use serde_json::Value;
use zerodb_storage::{ExportBundle, LocalStore};

const KIND_CREATE_NODE: u64 = 1;
const KIND_SET_PROPERTY: u64 = 3;

fn tmp_db(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("arrival-order-{name}.sqlite"));
    for candidate in [
        path.clone(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        let _ = std::fs::remove_file(candidate);
    }
    path
}

fn seeded_source(path: &Path) -> (LocalStore, String) {
    let mut source = LocalStore::init(path).unwrap();
    let node = source.create_node("FlowProbe").unwrap();
    source.set_lww(&node, "title", "seed").unwrap();
    source.gcounter_inc(&node, "views", 2).unwrap();
    source.set_add(&node, "tags", "base").unwrap();
    (source, node)
}

fn create_last(mut bundle: ExportBundle) -> ExportBundle {
    bundle
        .ops
        .sort_by_key(|op| if op.kind == KIND_CREATE_NODE { 1 } else { 0 });
    assert_eq!(bundle.ops.last().map(|op| op.kind), Some(KIND_CREATE_NODE));
    bundle
}

fn normalized_inspect(store: &LocalStore, path: &Path) -> Value {
    let report = store.inspect(path).unwrap();
    serde_json::json!({
        "datastore_id": report.datastore_id,
        "ops": report.ops,
        "nodes": report.nodes,
    })
}

#[test]
fn property_before_create_import_converges_with_source_label() {
    let source_path = tmp_db("converge-source");
    let destination_path = tmp_db("converge-destination");
    let (source, _) = seeded_source(&source_path);
    let reordered = create_last(source.export_all().unwrap());

    let mut destination = LocalStore::init(&destination_path).unwrap();
    let (accepted, skipped) = destination.import_bundle(&reordered).unwrap();

    assert_eq!(accepted as usize, reordered.ops.len());
    assert_eq!(skipped, 0);
    assert_eq!(
        normalized_inspect(&destination, &destination_path),
        normalized_inspect(&source, &source_path),
        "materialized state, including the CreateNode label, must depend only on the op set",
    );
}

#[test]
fn multiple_properties_remain_shadow_state_until_create_arrives() {
    let source_path = tmp_db("shadow-source");
    let destination_path = tmp_db("shadow-destination");
    let (source, _) = seeded_source(&source_path);
    let full_bundle = source.export_all().unwrap();

    let mut properties = full_bundle.clone();
    properties.ops.retain(|op| op.kind == KIND_SET_PROPERTY);
    assert_eq!(properties.ops.len(), 3);

    let mut create = full_bundle.clone();
    create.ops.retain(|op| op.kind == KIND_CREATE_NODE);
    assert_eq!(create.ops.len(), 1);

    let mut destination = LocalStore::init(&destination_path).unwrap();
    let (accepted, skipped) = destination.import_bundle(&properties).unwrap();
    assert_eq!((accepted, skipped), (3, 0));
    assert!(
        destination
            .inspect(&destination_path)
            .unwrap()
            .nodes
            .is_empty(),
        "remote properties may be retained before CreateNode but must not expose a placeholder node",
    );

    assert_eq!(destination.import_bundle(&create).unwrap(), (1, 0));
    assert_eq!(
        normalized_inspect(&destination, &destination_path),
        normalized_inspect(&source, &source_path),
    );
}

#[test]
fn reopen_and_replay_preserve_create_label_after_reordered_import() {
    let source_path = tmp_db("replay-source");
    let destination_path = tmp_db("replay-destination");
    let (source, _) = seeded_source(&source_path);
    let reordered = create_last(source.export_all().unwrap());

    let mut destination = LocalStore::init(&destination_path).unwrap();
    destination.import_bundle(&reordered).unwrap();
    drop(destination);

    let mut reopened = LocalStore::open(&destination_path).unwrap();
    assert_eq!(
        normalized_inspect(&reopened, &destination_path),
        normalized_inspect(&source, &source_path),
        "restart must retain the authoritative CreateNode label",
    );

    reopened.replay_all().unwrap();
    assert_eq!(
        normalized_inspect(&reopened, &destination_path),
        normalized_inspect(&source, &source_path),
        "fresh replay must retain the authoritative CreateNode label",
    );
}

#[test]
fn local_property_mutation_rejects_an_unknown_node_without_side_effects() {
    let path = tmp_db("unknown-local-node");
    let mut store = LocalStore::init(&path).unwrap();
    let unknown_node = "00000000000000000000000000000001";

    let result = store.set_lww(unknown_node, "title", "must not exist");

    assert!(
        result.is_err(),
        "local property mutation must reject a valid but unknown NodeId",
    );
    assert_eq!(store.op_count().unwrap(), 0);
    assert!(store.list_nodes().unwrap().is_empty());
}
