//! M1-e1 / M1-e2-store: fuller E1 and store-boundary E2 acceptance.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde_json::Value;
use zerodb_core::cbor::Cbor;
use zerodb_core::op::{OpEnvelope, OpTs};
use zerodb_core::sign::sign_op;
use zerodb_storage::{ExportBundle, LocalStore, WireOp, WireTs};

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("e1e2-{name}-{nonce}.sqlite"));
    remove_sqlite(&path);
    path
}

fn remove_sqlite(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

fn decode_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn max_op_ts(path: &Path) -> (u64, u16) {
    let conn = Connection::open(path).unwrap();
    conn.query_row(
        "SELECT physical_ms, logical FROM ops
         ORDER BY physical_ms DESC, logical DESC LIMIT 1",
        [],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u16)),
    )
    .unwrap()
}

fn op_ts(path: &Path, op_hex: &str) -> (u64, u16) {
    let conn = Connection::open(path).unwrap();
    conn.query_row(
        "SELECT physical_ms, logical FROM ops WHERE id = ?1",
        [decode_hex(op_hex)],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u16)),
    )
    .unwrap()
}

/// Normalized inspect for byte-stable comparison (drop path/peer).
fn norm_state(store: &LocalStore, path: &Path) -> Value {
    let report = store.inspect(path).unwrap();
    serde_json::json!({
        "datastore_id": report.datastore_id,
        "ops": report.ops,
        "nodes": report.nodes,
    })
}

fn signed_lww(
    ds_hex: &str,
    seed: [u8; 32],
    node_hex: &str,
    path: &str,
    value: &str,
    physical_ms: u64,
    logical: u16,
) -> WireOp {
    let ds: [u8; 32] = decode_hex(ds_hex).try_into().unwrap();
    let node = decode_hex(node_hex);
    let body = Cbor::Map(vec![
        ("node".into(), Cbor::Bytes(node)),
        ("path".into(), Cbor::Text(path.into())),
        ("crdt".into(), Cbor::Text("lww".into())),
        ("value".into(), Cbor::Text(value.into())),
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
            logical,
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
        ds: ds_hex.to_owned(),
        ep: 0,
        author: hex::encode(author),
        author_pk: hex::encode(author_pk),
        ts: WireTs {
            p: physical_ms,
            l: logical,
        },
        deps: vec![],
        grp: None,
        kind: 3,
        body: serde_json::json!({
            "node": node_hex,
            "path": path,
            "crdt": "lww",
            "value": value,
        }),
        sig: hex::encode(sig),
    }
}

// --- E1 ---

#[test]
fn e1_fifty_todos_restart_and_fresh_replay_match() {
    const N: usize = 50;
    let path = tmp_db("e1-fifty");
    let mut store = LocalStore::init(&path).unwrap();

    let mut nodes = Vec::with_capacity(N);
    for i in 0..N {
        let id = store.create_node("Todo").unwrap();
        store.set_lww(&id, "title", &format!("item-{i}")).unwrap();
        if i % 2 == 0 {
            store.flag_enable(&id, "done").unwrap();
        }
        store.set_add(&id, "tags", "groceries").unwrap();
        if i % 3 == 0 {
            store.set_add(&id, "tags", "urgent").unwrap();
            store.set_remove(&id, "tags", "urgent").unwrap();
        }
        store.counter_inc(&id, "voteScore", 1).unwrap();
        if i % 5 == 0 {
            store.counter_dec(&id, "voteScore", 1).unwrap();
        }
        store.gcounter_inc(&id, "views", 1).unwrap();
        nodes.push(id);
    }

    let before = norm_state(&store, &path);
    let ops_before = store.op_count().unwrap();
    assert!(ops_before >= N as u64 * 3);
    drop(store);

    // Restart
    let store2 = LocalStore::open(&path).unwrap();
    assert_eq!(norm_state(&store2, &path), before);
    assert_eq!(store2.op_count().unwrap(), ops_before);

    // Fresh replay via export/import
    let bundle = store2.export_all().unwrap();
    drop(store2);
    let path_fresh = tmp_db("e1-fifty-fresh");
    let mut fresh = LocalStore::init(&path_fresh).unwrap();
    fresh.import_bundle(&bundle).unwrap();
    assert_eq!(
        norm_state(&fresh, &path_fresh),
        before,
        "fresh-import materialization must match restarted store"
    );

    // Local replay_all
    let mut store3 = LocalStore::open(&path).unwrap();
    store3.replay_all().unwrap();
    assert_eq!(norm_state(&store3, &path), before);

    // Spot-check semantics
    assert_eq!(
        store3.get_lww(&nodes[0], "title").unwrap().as_deref(),
        Some("item-0")
    );
    assert_eq!(
        store3.get_prop(&nodes[0], "done").unwrap(),
        Some(serde_json::json!(true))
    );
    // Odd indices never received FlagEnable — property is absent, not false.
    assert_eq!(store3.get_prop(&nodes[1], "done").unwrap(), None);
    assert_eq!(
        store3.get_prop(&nodes[49], "views").unwrap(),
        Some(serde_json::json!(1))
    );
}

#[test]
fn e1_post_restart_op_timestamps_exceed_pre_restart_max() {
    let path = tmp_db("e1-hlc-mono");
    let mut store = LocalStore::init(&path).unwrap();
    let node = store.create_node("Todo").unwrap();
    store.set_lww(&node, "title", "pre").unwrap();
    // Burn logical ticks so HLC is non-trivial.
    for i in 0..5 {
        store.set_lww(&node, "title", &format!("pre-{i}")).unwrap();
    }
    let pre_max = max_op_ts(&path);
    drop(store);

    let mut store = LocalStore::open(&path).unwrap();
    let op = store.set_lww(&node, "title", "post-restart").unwrap();
    drop(store);

    let post = op_ts(&path, &op);
    assert!(
        post > pre_max,
        "I-4: post-restart op {post:?} must exceed pre-restart max {pre_max:?}"
    );
}

// --- E2 store boundary ---

#[test]
fn e2_same_author_equal_ts_lww_equivocation_excludes_both() {
    let path = tmp_db("e2-equiv");
    let mut store = LocalStore::init(&path).unwrap();
    let node = store.create_node("Todo").unwrap();
    let ds = store.datastore_id_hex();
    // Baseline non-conflicting write so node exists and has prior history.
    store.set_lww(&node, "title", "baseline").unwrap();
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    // Far enough in the past that equal-ts pair is accepted under drift, but
    // we use current wall with logical 0 after importing — better: use recent
    // ts with fixed physical shared by both forks.
    let physical = wall.saturating_sub(1_000).max(1);
    drop(store);

    // Same device key for both forks (same-author equivocation).
    let seed = [0xABu8; 32];
    let a = signed_lww(&ds, seed, &node, "title", "fork-a", physical, 7);
    let b = signed_lww(&ds, seed, &node, "title", "fork-b", physical, 7);
    assert_ne!(a.id, b.id);

    let mut store = LocalStore::open(&path).unwrap();
    let bundle = ExportBundle {
        format: 1,
        datastore_id: ds.clone(),
        ops: vec![a, b],
    };
    let (accepted, _) = store.import_bundle(&bundle).unwrap();
    assert_eq!(accepted, 2);

    // Both equal-ts same-author ops excluded → no LWW winner from those forks.
    // Baseline remains if its order key wins over nothing from the pair.
    // Equivocation excludes only the forked equal-ts pair; baseline (different
    // ts) still materializes.
    let title = store.get_lww(&node, "title").unwrap();
    assert_eq!(
        title.as_deref(),
        Some("baseline"),
        "equivocated equal-ts forks must not override baseline LWW"
    );

    // If we import *only* the two forks onto a node with no other LWW ops:
    let path2 = tmp_db("e2-equiv-only");
    let mut s2 = LocalStore::init(&path2).unwrap();
    let node2 = s2.create_node("Todo").unwrap();
    let ds2 = s2.datastore_id_hex();
    drop(s2);
    let seed2 = [0xCDu8; 32];
    let fa = signed_lww(&ds2, seed2, &node2, "title", "only-a", physical, 3);
    let fb = signed_lww(&ds2, seed2, &node2, "title", "only-b", physical, 3);
    let mut s2 = LocalStore::open(&path2).unwrap();
    s2.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: ds2,
        ops: vec![fa, fb],
    })
    .unwrap();
    let only = s2.get_lww(&node2, "title").unwrap();
    // No surviving LWW op → null materialization (not fork-a or fork-b).
    assert!(
        only.is_none() || only.as_deref() == Some(""),
        "equal-ts same-author only: expected no LWW winner, got {only:?}"
    );
    // props may store JSON null
    let prop = s2.get_prop(&node2, "title").unwrap();
    assert!(
        prop.is_none() || prop == Some(Value::Null),
        "expected null/absent title after pure equivocation, got {prop:?}"
    );
}

#[test]
fn e2_cross_peer_equal_ts_lww_total_order_converges() {
    let pa = tmp_db("e2-cross-a");
    let pb = tmp_db("e2-cross-b");
    let mut a = LocalStore::init(&pa).unwrap();
    let node = a.create_node("Todo").unwrap();
    let seed_bundle = a.export_all().unwrap();
    drop(a);

    let mut b = LocalStore::init(&pb).unwrap();
    b.import_bundle(&seed_bundle).unwrap();
    drop(b);

    let ds = seed_bundle.datastore_id.clone();
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let physical = wall.saturating_sub(500).max(1);

    // Two different device keys, same HLC physical+logical → not equivocation;
    // §4.5 total order by (p, l, author, op_id) picks one winner.
    let seed_a = [0x11u8; 32];
    let seed_b = [0x22u8; 32];
    let op_a = signed_lww(&ds, seed_a, &node, "title", "from-a", physical, 1);
    let op_b = signed_lww(&ds, seed_b, &node, "title", "from-b", physical, 1);

    let mut store_a = LocalStore::open(&pa).unwrap();
    let mut store_b = LocalStore::open(&pb).unwrap();
    store_a
        .import_bundle(&ExportBundle {
            format: 1,
            datastore_id: ds.clone(),
            ops: vec![op_a.clone(), op_b.clone()],
        })
        .unwrap();
    store_b
        .import_bundle(&ExportBundle {
            format: 1,
            datastore_id: ds.clone(),
            ops: vec![op_b, op_a], // reverse arrival
        })
        .unwrap();

    let ta = store_a.get_lww(&node, "title").unwrap();
    let tb = store_b.get_lww(&node, "title").unwrap();
    assert_eq!(
        ta, tb,
        "arrival order must not affect LWW equal-ts total order"
    );
    assert!(
        ta.as_deref() == Some("from-a") || ta.as_deref() == Some("from-b"),
        "winner must be one of the two concurrent writes, got {ta:?}"
    );
}
