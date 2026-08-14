//! M2-schema: canonical IR + SchemaId, ep stamp, dep limits, query params.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use zerodb_core::cbor::Cbor;
use zerodb_core::op::{OpEnvelope, OpTs};
use zerodb_core::schema::{parse_ir, schema_id};
use zerodb_core::sign::sign_op;
use zerodb_storage::{LocalStore, WireOp, WireTs};

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("m2-schema-{name}-{nonce}.sqlite"));
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
    path
}

fn signed_create_with_deps(store: &LocalStore, deps: Vec<[u8; 32]>) -> WireOp {
    let ds_hex = store.datastore_id_hex();
    let ds: [u8; 32] = hex::decode(&ds_hex).unwrap().try_into().unwrap();
    let seed = store.identity_seed();
    let node = [0xABu8; 16];
    let body = Cbor::Map(vec![
        ("label".into(), Cbor::Text("Todo".into())),
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
            physical_ms: 9_000_000,
            logical: 0,
        },
        deps,
        grp: None,
        kind: 1,
        body,
    };
    let id = envelope.op_id().unwrap();
    let (_, sig) = sign_op(&seed, &id);
    WireOp {
        id: hex::encode(id),
        v: 1,
        ds: ds_hex,
        ep: 0,
        author: hex::encode(author),
        author_pk: hex::encode(author_pk),
        ts: WireTs { p: 9_000_000, l: 0 },
        deps: envelope.deps.iter().map(hex::encode).collect(),
        grp: None,
        kind: 1,
        body: serde_json::json!({
            "label": "Todo",
            "node": hex::encode(node),
        }),
        sig: hex::encode(sig),
    }
}

#[test]
fn apply_pin_persists_schema_id_and_stamps_ep() {
    let path = tmp_db("pin-ep");
    let mut store = LocalStore::init(&path).unwrap();
    store
        .apply_schema_json(r#"{"nodes":{"Todo":{"props":{"title":"lww","done":"flag"}}}}"#)
        .unwrap();
    let sid = store.schema_id_hex().unwrap().expect("schema id");
    assert_eq!(sid.len(), 64);
    assert_eq!(store.schema_epoch().unwrap(), 1);

    let node = store.create_node("Todo").unwrap();
    store.set_lww(&node, "title", "milk").unwrap();
    let bundle = store.export_all().unwrap();
    assert!(
        bundle.ops.iter().all(|op| op.ep == 1),
        "local ops after apply must carry ep=1, got {:?}",
        bundle.ops.iter().map(|o| o.ep).collect::<Vec<_>>()
    );
    let ir = store.schema_ir_bytes().unwrap().expect("ir bytes");
    let parsed = parse_ir(&zerodb_core::cbor::decode(&ir).unwrap()).unwrap();
    assert!(parsed.nodes.contains_key("Todo"));
    assert_eq!(hex::encode(schema_id(&ir)), sid);
}

#[test]
fn apply_ir_json_round_trips_schema_id() {
    let path = tmp_db("ir-json");
    let mut store = LocalStore::init(&path).unwrap();
    store
        .apply_schema_json(
            r#"{
              "v": 1,
              "name": "todo",
              "nodes": {
                "Todo": {
                  "props": {
                    "title": { "crdt": 0, "type": 4, "nullable": true, "encrypted": false }
                  }
                }
              },
              "edges": {}
            }"#,
        )
        .unwrap();
    assert_eq!(store.schema_epoch().unwrap(), 1);
    let sid = store.schema_id_hex().unwrap().unwrap();
    let ir = store.schema_ir_bytes().unwrap().unwrap();
    assert_eq!(hex::encode(schema_id(&ir)), sid);
}

#[test]
fn import_rejects_more_than_64_deps() {
    let path = tmp_db("deps-cap");
    let mut store = LocalStore::init(&path).unwrap();
    let deps = vec![[0x11u8; 32]; 65];
    let mut bundle = store.export_all().unwrap();
    bundle.ops.push(signed_create_with_deps(&store, deps));
    let err = store.import_bundle(&bundle).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("dep"), "got {err}");
}

#[test]
fn import_rejects_unknown_dep() {
    let path = tmp_db("dep-miss");
    let mut store = LocalStore::init(&path).unwrap();
    let mut bundle = store.export_all().unwrap();
    bundle
        .ops
        .push(signed_create_with_deps(&store, vec![[0xFFu8; 32]]));
    let err = store.import_bundle(&bundle).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("dep"), "got {err}");
}

#[test]
fn query_binds_parameters() {
    let path = tmp_db("qparam");
    let mut store = LocalStore::init(&path).unwrap();
    let a = store.create_node("Todo").unwrap();
    store.set_lww(&a, "title", "milk").unwrap();
    let b = store.create_node("Todo").unwrap();
    store.set_lww(&b, "title", "bread").unwrap();
    let rows = store
        .query_with(
            "MATCH (t:Todo) WHERE t.title = $want RETURN t.title",
            &serde_json::json!({ "want": "milk" }),
        )
        .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["t.title"], "milk");
}
