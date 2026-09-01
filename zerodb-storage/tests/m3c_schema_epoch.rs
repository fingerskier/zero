//! M3c-a: signed KERNEL kind 5 `SchemaEpoch` (SCHEMA.md §3).
//!
//! `apply_schema_json` is a helper that emits n=1 / prev=null / empty
//! migration. Peers materialize schema from the op IR (including
//! `encrypted: true`). A data op whose `ep` is past the applied chain is
//! `EPOCH_UNKNOWN` and does not persist.

use zerodb_core::schema::{parse_ir, schema_id};
use zerodb_storage::{
    EPOCH_UNKNOWN, ExportBundle, IngestResult, LocalStore, MemoryBackend, StoreBackend,
};

const NOTE_SCHEMA: &str = r#"{
  "v": 1,
  "nodes": {
    "Note": {
      "props": {
        "body": { "crdt": 0, "type": 4, "nullable": false, "encrypted": true }
      }
    }
  },
  "edges": {}
}"#;

fn empty_store() -> LocalStore<MemoryBackend> {
    LocalStore::init_with_backend(MemoryBackend::new()).unwrap()
}

fn ds_bytes<B: StoreBackend>(store: &LocalStore<B>) -> [u8; 32] {
    hex::decode(store.datastore_id_hex())
        .unwrap()
        .try_into()
        .unwrap()
}

#[test]
fn schema_epoch_encrypted_lww_rides_kind_5() {
    let mut a = empty_store();
    a.apply_schema_json(NOTE_SCHEMA).unwrap();
    assert_eq!(a.schema_epoch().unwrap(), 1);

    let bundle = a.export_all().unwrap();
    let epoch = bundle
        .ops
        .iter()
        .find(|op| op.kind == 5)
        .expect("kind 5 SchemaEpoch");
    assert_eq!(epoch.ep, 0);
    assert_eq!(epoch.body["epoch"], 1);
    assert!(epoch.body["prev"].is_null());
    assert_eq!(epoch.body["migration"], serde_json::json!([]));
    let sid = a.schema_id_hex().unwrap().expect("schema id");
    assert_eq!(epoch.body["schema"], sid);
    let ir_hex = epoch.body["ir"].as_str().expect("ir");
    let ir_bytes = hex::decode(ir_hex).unwrap();
    assert_eq!(hex::encode(schema_id(&ir_bytes)), sid);
    let parsed = parse_ir(&zerodb_core::cbor::decode(&ir_bytes).unwrap()).unwrap();
    assert!(parsed.nodes["Note"].props["body"].encrypted);

    let mut b = empty_store();
    b.import_bundle(&bundle).unwrap();
    assert_eq!(b.datastore_id_hex(), a.datastore_id_hex());
    assert_eq!(b.schema_epoch().unwrap(), 1);
    assert_eq!(b.schema_id_hex().unwrap().as_deref(), Some(sid.as_str()));
    let b_ir = b.schema_ir_bytes().unwrap().expect("imported ir");
    let b_parsed = parse_ir(&zerodb_core::cbor::decode(&b_ir).unwrap()).unwrap();
    assert!(
        b_parsed.nodes["Note"].props["body"].encrypted,
        "encrypted: true must ride the epoch op, not local meta"
    );

    let note = a.create_node("Note").unwrap();
    a.set_lww(&note, "body", "attack-at-dawn").unwrap();
    let set = a
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 3)
        .expect("SetProperty");
    assert_eq!(set.ep, 1);
    assert!(set.body.get("encrypted").and_then(|v| v.as_str()).is_some());
    assert!(set.body.get("value").is_none());

    b.import_bundle(&a.export_all().unwrap()).unwrap();
    assert!(
        b.export_all()
            .unwrap()
            .ops
            .iter()
            .any(|op| op.kind == 3 && op.body.get("encrypted").is_some()),
        "encrypted LWW must persist on the peer that applied the epoch"
    );

    a.replay_all().unwrap();
    assert_eq!(a.schema_epoch().unwrap(), 1);
    assert_eq!(a.schema_id_hex().unwrap().as_deref(), Some(sid.as_str()));
    assert_eq!(
        a.get_lww(&note, "body").unwrap().as_deref(),
        Some("attack-at-dawn")
    );

    a.apply_schema_json(NOTE_SCHEMA).unwrap();
    assert_eq!(
        a.export_all()
            .unwrap()
            .ops
            .iter()
            .filter(|op| op.kind == 5)
            .count(),
        1,
        "same-IR apply_schema_json is a helper, not a second epoch"
    );
}

#[test]
fn unknown_ep_is_epoch_unknown_and_does_not_persist() {
    let mut a = empty_store();
    a.apply_schema_json(NOTE_SCHEMA).unwrap();
    let note = a.create_node("Note").unwrap();
    a.set_lww(&note, "body", "secret").unwrap();
    let create = a
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 1)
        .expect("CreateNode");
    assert_eq!(create.ep, 1);

    let mut peer =
        LocalStore::init_with_backend_from_seed(MemoryBackend::new(), &[0xE1u8; 32], &ds_bytes(&a))
            .unwrap();
    assert_eq!(peer.schema_epoch().unwrap(), 0);
    match peer.ingest_op(&create).unwrap() {
        IngestResult::Rejected { reason } => assert_eq!(reason, EPOCH_UNKNOWN),
        other => panic!("expected EPOCH_UNKNOWN, got {other:?}"),
    }
    assert_eq!(peer.op_count().unwrap(), 0);
    assert_eq!(peer.schema_epoch().unwrap(), 0);
    assert!(peer.schema_ir_bytes().unwrap().is_none());
    assert!(
        peer.take_rejects()
            .iter()
            .any(|r| r.reason == EPOCH_UNKNOWN && r.op_id == create.id)
    );

    let mut importer = empty_store();
    let before_ds = importer.datastore_id_hex();
    let imported = importer.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: a.datastore_id_hex(),
        ops: vec![create.clone()],
    });
    match imported {
        Ok((accepted, skipped)) => {
            assert_eq!(accepted, 0);
            assert!(skipped >= 1);
        }
        Err(err) => {
            assert!(
                err.to_string().contains("EPOCH_UNKNOWN")
                    || err.to_string().contains("no accepted"),
                "empty importer must fail closed, got {err}"
            );
        }
    }
    assert_eq!(importer.op_count().unwrap(), 0);
    assert_eq!(importer.schema_epoch().unwrap(), 0);
    assert_eq!(importer.datastore_id_hex(), before_ds);
    assert!(importer.list_nodes().unwrap().is_empty());
}

/// P1-1: a catch-up / import batch may list epoch-bound data before the
/// kind-5 that introduces that epoch (OpId order, not causal). The epoch
/// must apply first so the data is not permanently skipped as EPOCH_UNKNOWN.
#[test]
fn same_batch_data_before_epoch_persists() {
    let mut a = empty_store();
    a.apply_schema_json(r#"{"nodes":{"Todo":{"props":{"title":"lww"}}}}"#)
        .unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "milk").unwrap();
    let exported = a.export_all().unwrap();
    let epoch = exported
        .ops
        .iter()
        .find(|op| op.kind == 5)
        .cloned()
        .expect("kind 5");
    let create = exported
        .ops
        .iter()
        .find(|op| op.kind == 1)
        .cloned()
        .expect("CreateNode");
    let set = exported
        .ops
        .iter()
        .find(|op| op.kind == 3)
        .cloned()
        .expect("SetProperty");
    assert_eq!(set.ep, 1);
    assert_eq!(create.ep, 1);
    assert_eq!(epoch.ep, 0);

    let mut b = empty_store();
    let (accepted, skipped) = b
        .import_bundle(&ExportBundle {
            format: 1,
            datastore_id: a.datastore_id_hex(),
            ops: vec![set.clone(), create.clone(), epoch.clone()],
        })
        .unwrap();
    assert!(
        accepted >= 3,
        "same-batch epoch + data must persist, accepted={accepted} skipped={skipped}"
    );
    assert_eq!(b.schema_epoch().unwrap(), 1);
    assert!(
        b.export_all().unwrap().ops.iter().any(|op| op.id == set.id),
        "SetProperty must persist after the in-batch epoch applies"
    );
    assert_eq!(b.get_lww(&node, "title").unwrap().as_deref(), Some("milk"));

    b.replay_all().unwrap();
    assert_eq!(b.get_lww(&node, "title").unwrap().as_deref(), Some("milk"));

    let mut c =
        LocalStore::init_with_backend_from_seed(MemoryBackend::new(), &[0xC0u8; 32], &ds_bytes(&a))
            .unwrap();
    c.commit_wires_atomic(&[set, create, epoch]).unwrap();
    assert_eq!(c.schema_epoch().unwrap(), 1);
    assert_eq!(c.get_lww(&node, "title").unwrap().as_deref(), Some("milk"));
}

/// P1-2: SCHEMA.md §3 own-epoch semantics. After epoch 1 (encrypted / CRDT
/// pin) is applied, a delayed ep=0 schemaless write must validate against
/// epoch 0 (no pin, not encrypted), not the current IR.
#[test]
fn late_ep0_plaintext_applies_under_own_epoch() {
    let mut early = empty_store();
    let note = early.create_node("Note").unwrap();
    early.set_lww(&note, "body", "schemaless").unwrap();
    let early_ops = early.export_all().unwrap();
    let create = early_ops
        .ops
        .iter()
        .find(|op| op.kind == 1)
        .cloned()
        .expect("CreateNode");
    let set = early_ops
        .ops
        .iter()
        .find(|op| op.kind == 3)
        .cloned()
        .expect("SetProperty");
    assert_eq!(create.ep, 0);
    assert_eq!(set.ep, 0);
    assert!(
        set.body.get("value").and_then(|v| v.as_str()) == Some("schemaless"),
        "epoch-0 LWW must be plaintext"
    );

    let mut author = LocalStore::init_with_backend_from_seed(
        MemoryBackend::new(),
        &[0xA1u8; 32],
        &ds_bytes(&early),
    )
    .unwrap();
    author.apply_schema_json(NOTE_SCHEMA).unwrap();
    let epoch = author
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 5)
        .expect("kind 5");

    let mut peer = LocalStore::init_with_backend_from_seed(
        MemoryBackend::new(),
        &[0xB2u8; 32],
        &ds_bytes(&early),
    )
    .unwrap();
    peer.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: early.datastore_id_hex(),
        ops: vec![epoch],
    })
    .unwrap();
    assert_eq!(peer.schema_epoch().unwrap(), 1);

    assert_eq!(peer.ingest_op(&create).unwrap(), IngestResult::Applied);
    match peer.ingest_op(&set).unwrap() {
        IngestResult::Applied => {}
        other => panic!("ep=0 plaintext must apply under own epoch, got {other:?}"),
    }
    assert!(
        peer.export_all()
            .unwrap()
            .ops
            .iter()
            .any(|op| op.id == set.id),
        "late ep=0 SetProperty must persist"
    );
    assert_eq!(
        peer.get_lww(&note, "body").unwrap().as_deref(),
        Some("schemaless")
    );

    let mut pin_early = empty_store();
    let todo = pin_early.create_node("Todo").unwrap();
    pin_early.set_lww(&todo, "title", "plain").unwrap();
    let pin_set = pin_early
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 3)
        .expect("ep=0 lww");
    let pin_create = pin_early
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 1)
        .expect("CreateNode");

    let mut pinned = LocalStore::init_with_backend_from_seed(
        MemoryBackend::new(),
        &[0xC3u8; 32],
        &ds_bytes(&pin_early),
    )
    .unwrap();
    pinned
        .apply_schema_json(r#"{"nodes":{"Todo":{"props":{"title":"flag"}}}}"#)
        .unwrap();
    let pin_epoch = pinned
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 5)
        .expect("kind 5");

    let mut late = LocalStore::init_with_backend_from_seed(
        MemoryBackend::new(),
        &[0xD4u8; 32],
        &ds_bytes(&pin_early),
    )
    .unwrap();
    late.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: pin_early.datastore_id_hex(),
        ops: vec![pin_epoch],
    })
    .unwrap();
    late.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: pin_early.datastore_id_hex(),
        ops: vec![pin_create, pin_set.clone()],
    })
    .unwrap();
    assert!(
        late.export_all()
            .unwrap()
            .ops
            .iter()
            .any(|op| op.id == pin_set.id),
        "ep=0 lww must not be rejected by an epoch-1 flag pin"
    );
}
