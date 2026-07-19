//! M1 remainder acceptance: E4 crash rollback, E9 edges/H3, schema pin, O3 query.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use zerodb_core::cbor::Cbor;
use zerodb_core::op::{OpEnvelope, OpTs};
use zerodb_core::sign::sign_op;
use zerodb_storage::{LocalStore, WireOp, WireTs};

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("m1-rem-{name}-{nonce}.sqlite"));
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
    path
}

fn decode_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn signed_create(
    ds_hex: &str,
    seed: [u8; 32],
    node: [u8; 16],
    label: &str,
    physical_ms: u64,
) -> WireOp {
    let ds: [u8; 32] = decode_hex(ds_hex).try_into().unwrap();
    let body = Cbor::Map(vec![
        ("label".into(), Cbor::Text(label.into())),
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
        ds: ds_hex.to_owned(),
        ep: 0,
        author: hex::encode(author),
        author_pk: hex::encode(author_pk),
        ts: WireTs {
            p: physical_ms,
            l: 0,
        },
        deps: vec![],
        grp: None,
        kind: 1,
        body: serde_json::json!({
            "label": label,
            "node": hex::encode(node),
        }),
        sig: hex::encode(sig),
    }
}

// --- E4: mid-batch failure rolls back entire atomic commit ---

#[test]
fn e4_commit_wires_atomic_rolls_back_prefix_on_bad_op() {
    let path = tmp_db("e4-crash");
    let mut store = LocalStore::init(&path).unwrap();
    let ds = store.datastore_id_hex();
    let seed = [0x42u8; 32];
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let good = signed_create(
        &ds,
        seed,
        [1u8; 16],
        "Todo",
        wall.saturating_sub(100).max(1),
    );
    let mut bad = signed_create(&ds, seed, [2u8; 16], "Todo", wall.saturating_sub(50).max(1));
    bad.sig = "00".repeat(64); // invalid signature after valid prefix

    let before = store.op_count().unwrap();
    let err = store
        .commit_wires_atomic(&[good, bad])
        .expect_err("bad signature must fail batch");
    assert!(
        err.to_string().to_lowercase().contains("sig")
            || err.to_string().contains("Crypto")
            || err.to_string().contains("bad")
    );
    assert_eq!(
        store.op_count().unwrap(),
        before,
        "failed atomic batch must leave zero durable prefix ops"
    );
    assert!(store.list_nodes().unwrap().is_empty());
}

// --- E9 / H3: edges + derived visibility ---

#[test]
fn e9_delete_node_hides_incident_edges_without_cascade() {
    let path = tmp_db("e9-hide");
    let mut store = LocalStore::init(&path).unwrap();
    let a = store.create_node("Todo").unwrap();
    let b = store.create_node("Note").unwrap();
    let edge = store.create_edge("NOTATES", &b, &a).unwrap();
    assert_eq!(store.list_edges_visible().unwrap().len(), 1);

    store.delete_node(&a).unwrap();
    assert!(store.list_edges_visible().unwrap().is_empty());

    // Edge row still in inspect but not visible (derived, no cascade tombstone op).
    let report = store.inspect(Path::new(&path)).unwrap();
    let e = report.edges.iter().find(|e| e.id == edge).unwrap();
    assert!(!e.visible);
    assert!(
        !e.deleted,
        "H3 derived visibility does not emit edge tombstones"
    );
}

#[test]
fn e9_late_edge_to_deleted_node_is_not_visible() {
    let path = tmp_db("e9-late");
    let mut store = LocalStore::init(&path).unwrap();
    let a = store.create_node("Todo").unwrap();
    let b = store.create_node("Note").unwrap();
    store.delete_node(&a).unwrap();
    // Late edge after delete (same peer sequence; models late arrival once imported).
    let edge = store.create_edge("NOTATES", &b, &a).unwrap();
    assert!(store.list_edges_visible().unwrap().is_empty());
    let report = store.inspect(Path::new(&path)).unwrap();
    let e = report.edges.iter().find(|e| e.id == edge).unwrap();
    assert!(!e.visible);
}

// --- Schema pin ---

#[test]
fn schema_pin_rejects_wrong_crdt() {
    let path = tmp_db("schema-pin");
    let mut store = LocalStore::init(&path).unwrap();
    store
        .apply_schema_json(
            r#"{ "nodes": { "Todo": { "props": { "title": "lww", "views": "gcounter" } } } }"#,
        )
        .unwrap();
    let node = store.create_node("Todo").unwrap();
    store.set_lww(&node, "title", "ok").unwrap();
    let err = store
        .gcounter_inc(&node, "title", 1)
        .expect_err("title is pinned lww");
    assert!(err.to_string().contains("schema pin"));
    store.gcounter_inc(&node, "views", 1).unwrap();
}

// --- O3 query ---

#[test]
fn query_match_return_title() {
    let path = tmp_db("query");
    let mut store = LocalStore::init(&path).unwrap();
    let n = store.create_node("Todo").unwrap();
    store.set_lww(&n, "title", "milk").unwrap();
    store.flag_enable(&n, "done").unwrap();
    let n2 = store.create_node("Todo").unwrap();
    store.set_lww(&n2, "title", "bread").unwrap();

    let rows = store
        .query(r#"MATCH (t:Todo) WHERE t.done = true RETURN t.title"#)
        .unwrap();
    let arr = rows.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["t.title"], "milk");
}
