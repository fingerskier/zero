//! Regression coverage for authenticated, atomic bundle ingress.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use zerodb_core::cbor::Cbor;
use zerodb_core::op::{OpEnvelope, OpTs};
use zerodb_core::sign::sign_op;
use zerodb_storage::{ExportBundle, LocalStore, WireOp, WireTs};

#[derive(Debug, PartialEq)]
struct StoreSnapshot {
    datastore_id: String,
    op_count: u64,
    nodes: Vec<(String, String, bool)>,
}

fn tmp_db(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("ingress-validation-{name}.sqlite"));
    remove_sqlite_files(&path);
    path
}

fn remove_sqlite_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

fn source_bundle(name: &str, with_property: bool) -> ExportBundle {
    let path = tmp_db(name);
    let mut source = LocalStore::init(&path).unwrap();
    let node = source.create_node("Todo").unwrap();
    if with_property {
        source.set_lww(&node, "title", "milk").unwrap();
    }
    source.export_all().unwrap()
}

fn snapshot(store: &LocalStore) -> StoreSnapshot {
    StoreSnapshot {
        datastore_id: store.datastore_id_hex(),
        op_count: store.op_count().unwrap(),
        nodes: store.list_nodes().unwrap(),
    }
}

fn signed_create_wire(datastore_id: &str, physical_ms: u64) -> WireOp {
    let ds: [u8; 32] = hex::decode(datastore_id).unwrap().try_into().unwrap();
    let node = [0x44; 16];
    let seed = [0x55; 32];
    let body = Cbor::Map(vec![
        ("label".into(), Cbor::Text("Future".into())),
        ("node".into(), Cbor::Bytes(node.to_vec())),
    ]);
    let (author_pk, _) = sign_op(&seed, &[0; 32]);
    let author = *blake3::hash(&author_pk).as_bytes();
    let envelope = OpEnvelope {
        v: 1,
        ds,
        ep: 0,
        author,
        ts: OpTs {
            physical_ms,
            logical: 0,
        },
        deps: vec![],
        grp: None,
        kind: 1,
        body,
    };
    let id = envelope.op_id().unwrap();
    let (_, sig) = sign_op(&seed, &id);
    WireOp {
        id: hex::encode(id),
        v: 1,
        ds: datastore_id.to_owned(),
        ep: 0,
        author: hex::encode(author),
        author_pk: hex::encode(author_pk),
        ts: WireTs {
            p: physical_ms,
            l: 0,
        },
        deps: vec![],
        kind: 1,
        body: serde_json::json!({
            "label": "Future",
            "node": hex::encode(node),
        }),
        sig: hex::encode(sig),
    }
}

fn assert_rejected_without_mutation(store: &mut LocalStore, bundle: &ExportBundle) {
    let before = snapshot(store);
    let result = store.import_bundle(bundle);
    assert!(result.is_err(), "invalid bundle was accepted: {result:?}");
    assert_eq!(snapshot(store), before, "rejected import mutated the store");
}

#[test]
fn tampered_wire_body_is_rejected_without_mutation() {
    let mut bundle = source_bundle("tampered-source", false);
    bundle.ops[0].body["label"] = serde_json::json!("Tampered");

    let destination_path = tmp_db("tampered-destination");
    let mut destination = LocalStore::init(&destination_path).unwrap();
    assert_rejected_without_mutation(&mut destination, &bundle);
}

#[test]
fn wrong_per_op_datastore_is_rejected_without_mutation() {
    let mut bundle = source_bundle("wrong-ds-source", false);
    let wrong_datastore = "00".repeat(32);
    assert_ne!(bundle.datastore_id, wrong_datastore);
    bundle.ops[0].ds = wrong_datastore;

    let destination_path = tmp_db("wrong-ds-destination");
    let mut destination = LocalStore::init(&destination_path).unwrap();
    assert_rejected_without_mutation(&mut destination, &bundle);
}

#[test]
fn invalid_signature_is_rejected_without_mutation() {
    let mut bundle = source_bundle("bad-signature-source", false);
    bundle.ops[0].sig = "00".repeat(64);

    let destination_path = tmp_db("bad-signature-destination");
    let mut destination = LocalStore::init(&destination_path).unwrap();
    assert_rejected_without_mutation(&mut destination, &bundle);
}

#[test]
fn failed_bootstrap_does_not_adopt_or_partially_ingest() {
    let mut bundle = source_bundle("partial-bootstrap-source", true);
    assert_eq!(bundle.ops.len(), 2);
    bundle.ops[1].sig = "00".repeat(64);

    let destination_path = tmp_db("partial-bootstrap-destination");
    let mut destination = LocalStore::init(&destination_path).unwrap();
    assert_rejected_without_mutation(&mut destination, &bundle);
}

#[test]
fn zero_op_mismatched_bundle_does_not_adopt_datastore() {
    let source_path = tmp_db("empty-source");
    let source = LocalStore::init(&source_path).unwrap();
    let bundle = source.export_all().unwrap();
    assert!(bundle.ops.is_empty());

    let destination_path = tmp_db("empty-destination");
    let mut destination = LocalStore::init(&destination_path).unwrap();
    assert_ne!(destination.datastore_id_hex(), bundle.datastore_id);
    let before = snapshot(&destination);

    let _ = destination.import_bundle(&bundle);

    assert_eq!(
        snapshot(&destination),
        before,
        "empty bundle adopted datastore"
    );
}

#[test]
fn semantic_cross_op_failure_does_not_adopt_or_commit_a_valid_prefix() {
    let source_path = tmp_db("semantic-source");
    let mut source = LocalStore::init(&source_path).unwrap();
    let node = source.create_node("Todo").unwrap();
    let seed_bundle = source.export_all().unwrap();

    let divergent_path = tmp_db("semantic-divergent");
    let mut divergent = LocalStore::init(&divergent_path).unwrap();
    divergent.import_bundle(&seed_bundle).unwrap();

    source.gcounter_inc(&node, "mixed", 1).unwrap();
    let source_bundle = source.export_all().unwrap();
    let create = source_bundle
        .ops
        .iter()
        .find(|op| op.kind == 1)
        .unwrap()
        .clone();
    let gcounter = source_bundle
        .ops
        .iter()
        .find(|op| op.body["crdt"] == "gcounter")
        .unwrap()
        .clone();

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        if wall > gcounter.ts.p {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "wall clock did not advance beyond the first operation timestamp",
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    divergent.counter_dec(&node, "mixed", 1).unwrap();
    let divergent_bundle = divergent.export_all().unwrap();
    let pncounter = divergent_bundle
        .ops
        .iter()
        .find(|op| op.body["crdt"] == "pncounter")
        .unwrap()
        .clone();
    assert!(gcounter.ts.p < pncounter.ts.p);
    let invalid_bundle = ExportBundle {
        format: 1,
        datastore_id: source_bundle.datastore_id,
        ops: vec![create, gcounter, pncounter],
    };

    let destination_path = tmp_db("semantic-destination");
    let mut destination = LocalStore::init(&destination_path).unwrap();
    assert_rejected_without_mutation(&mut destination, &invalid_bundle);
}

#[test]
fn future_timestamp_is_rejected_without_poisoning_local_hlc() {
    let path = tmp_db("future-timestamp");
    let mut store = LocalStore::init(&path).unwrap();
    let before = snapshot(&store);
    let wall_before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let future = wall_before + 10 * 60 * 1000;
    let bundle = ExportBundle {
        format: 1,
        datastore_id: store.datastore_id_hex(),
        ops: vec![signed_create_wire(&store.datastore_id_hex(), future)],
    };

    let result = store.import_bundle(&bundle);
    let after_import = snapshot(&store);
    let local_node = store.create_node("Local").unwrap();
    let local_author = store.author_hex();
    let local_op = store
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.author == local_author && op.body["node"] == local_node)
        .unwrap();
    let wall_after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    assert!(
        local_op.ts.p <= wall_after + 60_000,
        "future ingress poisoned local HLC: local={} wall={wall_after}",
        local_op.ts.p,
    );
    assert!(result.is_err(), "future operation was accepted: {result:?}");
    assert_eq!(
        after_import, before,
        "future operation mutated durable store state"
    );
}
