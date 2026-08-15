//! M3a RELAY 0.2.2 client: handshake, push signed LocalStore ops, catch-up.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use zerodb_core::cbor::{self, Cbor};
use zerodb_core::merkle::{MerkleOp, merkle_root};
use zerodb_core::relay::{MSG_OPS, MSG_SYNC_REQUEST};
use zerodb_relay::Relay;
use zerodb_storage::relay_client;
use zerodb_storage::{LocalStore, MemoryBackend, StoreBackend, StoreError};

fn frame_type(frame: &[u8]) -> u8 {
    let c = cbor::decode(frame).expect("envelope cbor");
    match &c {
        Cbor::Map(ents) => ents
            .iter()
            .find(|(k, _)| k == "type")
            .and_then(|(_, v)| match v {
                Cbor::Uint(n) => Some(*n as u8),
                _ => None,
            })
            .expect("envelope type"),
        _ => panic!("envelope is not a map"),
    }
}

fn map_get<'a>(c: &'a Cbor, k: &str) -> &'a Cbor {
    static NULL: Cbor = Cbor::Null;
    match c {
        Cbor::Map(ents) => ents
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v)
            .unwrap_or(&NULL),
        _ => &NULL,
    }
}

fn hex32(s: &str) -> [u8; 32] {
    assert_eq!(s.len(), 64, "expected 32-byte hex");
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex");
    }
    out
}

fn sync_accepted_root(frame: &[u8]) -> Vec<u8> {
    assert_eq!(frame_type(frame), MSG_SYNC_REQUEST);
    let c = cbor::decode(frame).expect("envelope cbor");
    match map_get(map_get(&c, "payload"), "accepted_root") {
        Cbor::Bytes(b) => b.clone(),
        other => panic!("accepted_root not bytes: {other:?}"),
    }
}

fn merkle_of_ds<B: StoreBackend>(store: &LocalStore<B>, ds: &str) -> [u8; 32] {
    let bundle = store.export_all().unwrap();
    let ops: Vec<MerkleOp> = bundle
        .ops
        .iter()
        .filter(|w| w.ds == ds)
        .map(|w| MerkleOp {
            op_id: hex32(&w.id),
            author: hex32(&w.author),
            physical_ms: w.ts.p,
            logical: w.ts.l,
        })
        .collect();
    merkle_root(&ops)
}

fn drive<B: StoreBackend>(
    store: &mut LocalStore<B>,
    sess: &mut zerodb_relay::RelaySession,
    join_ds: Option<&str>,
) -> relay_client::RelaySyncSummary {
    relay_client::sync(store, join_ds, |frame| {
        sess.handle(frame)
            .map_err(|e| StoreError::Invalid(e.to_string()))
    })
    .expect("relay client session")
}

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("m3a-e2-{name}-{nonce}.sqlite"));
    remove_sqlite(&path);
    path
}

fn remove_sqlite(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

#[test]
fn two_stores_converge_through_in_process_relay() {
    let mut a = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "milk").unwrap();

    let relay = Relay::memory();
    let mut sess_a = relay.accept();
    let pushed = drive(&mut a, &mut sess_a, None);
    assert!(
        pushed.sent >= 2,
        "A must submit local ops, sent={}",
        pushed.sent
    );
    assert_eq!(pushed.ack_accepted, pushed.sent);

    let mut b = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    assert_ne!(b.datastore_id_hex(), a.datastore_id_hex());
    let mut sess_b = relay.accept();
    let ds = a.datastore_id_hex();
    let caught = drive(&mut b, &mut sess_b, Some(&ds));
    assert!(
        caught.received >= 2,
        "B must receive A's ops, received={}",
        caught.received
    );
    assert_eq!(b.datastore_id_hex(), ds);
    assert_eq!(b.get_lww(&node, "title").unwrap().as_deref(), Some("milk"));
}

#[test]
fn second_peer_write_catches_up_first_peer() {
    let mut a = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "milk").unwrap();

    let relay = Relay::memory();
    let mut sess_a = relay.accept();
    drive(&mut a, &mut sess_a, None);

    let mut b = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let mut sess_b = relay.accept();
    let ds = a.datastore_id_hex();
    drive(&mut b, &mut sess_b, Some(&ds));
    b.set_lww(&node, "title", "oat").unwrap();

    let mut sess_b2 = relay.accept();
    let pushed = drive(&mut b, &mut sess_b2, None);
    assert!(pushed.sent >= 1);
    assert!(
        pushed.ack_accepted >= 1,
        "B must persist the new write, {pushed:?}"
    );
    assert_eq!(
        pushed.ack_accepted + pushed.ack_duplicate,
        pushed.sent,
        "every submitted op must be ACK'd, {pushed:?}"
    );

    let mut sess_a2 = relay.accept();
    let caught = drive(&mut a, &mut sess_a2, None);
    assert!(caught.received >= 1);
    assert_eq!(a.get_lww(&node, "title").unwrap().as_deref(), Some("oat"));
}

#[test]
fn resume_does_not_redeliver_covered_ops() {
    let mut a = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "milk").unwrap();

    let relay = Relay::memory();
    let mut sess = relay.accept();
    drive(&mut a, &mut sess, None);

    let mut b = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let ds = a.datastore_id_hex();
    let mut sess_b = relay.accept();
    let first = drive(&mut b, &mut sess_b, Some(&ds));
    assert!(first.received >= 2);
    assert!(first.applied >= 2);

    let mut sess_b2 = relay.accept();
    let second = drive(&mut b, &mut sess_b2, Some(&ds));
    assert_eq!(second.received, 0);
}

#[test]
fn outgoing_ops_split_to_welcome_batch_limits() {
    let mut a = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let node = a.create_node("Todo").unwrap();
    for i in 0..69 {
        a.set_lww(&node, "title", &format!("v{i}")).unwrap();
    }
    assert_eq!(a.op_count().unwrap(), 70);

    let relay = Relay::memory();
    let mut sess = relay.accept();
    let mut ops_frames = 0u32;
    let summary = relay_client::sync(&mut a, None, |frame| {
        if frame_type(frame) == MSG_OPS {
            ops_frames += 1;
        }
        sess.handle(frame)
            .map_err(|e| StoreError::Invalid(e.to_string()))
    })
    .expect("relay client session");

    assert!(
        ops_frames > 1,
        "70 local ops must split across WELCOME max_batch_ops, ops_frames={ops_frames}"
    );
    assert_eq!(summary.sent, 70);
    assert_eq!(summary.ack_accepted, 70);
    assert_eq!(summary.ack_rejected, 0);
}

#[test]
fn empty_join_adopts_target_ds_when_relay_has_no_ops() {
    let mut a = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let mut b = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let ds = a.datastore_id_hex();
    assert_eq!(a.op_count().unwrap(), 0);
    assert_eq!(b.op_count().unwrap(), 0);
    assert_ne!(b.datastore_id_hex(), ds);

    let relay = Relay::memory();
    let mut sess_b = relay.accept();
    drive(&mut b, &mut sess_b, Some(&ds));
    assert_eq!(
        b.datastore_id_hex(),
        ds,
        "empty join must adopt the target ds even with empty catch-up"
    );

    let node = b.create_node("Todo").unwrap();
    b.set_lww(&node, "title", "seed").unwrap();

    let mut sess_b2 = relay.accept();
    let pushed = drive(&mut b, &mut sess_b2, None);
    assert!(
        pushed.sent >= 2,
        "B must push writes stamped with joined ds"
    );
    assert_eq!(pushed.ack_accepted, pushed.sent);

    let mut sess_a = relay.accept();
    let caught = drive(&mut a, &mut sess_a, None);
    assert!(
        caught.received >= 2,
        "A must catch up B's write, {caught:?}"
    );
    assert_eq!(a.get_lww(&node, "title").unwrap().as_deref(), Some("seed"));
}

#[test]
fn nonempty_store_sends_merkle_accepted_root() {
    let mut a = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "milk").unwrap();
    let ds = a.datastore_id_hex();
    let expected = merkle_of_ds(&a, &ds);
    assert_ne!(
        expected, [0u8; 32],
        "fixture must have a nonzero merkle root"
    );

    let relay = Relay::memory();
    let mut sess = relay.accept();
    let mut seen = None;
    relay_client::sync(&mut a, None, |frame| {
        if frame_type(frame) == MSG_SYNC_REQUEST {
            seen = Some(sync_accepted_root(frame));
        }
        sess.handle(frame)
            .map_err(|e| StoreError::Invalid(e.to_string()))
    })
    .expect("relay client session");

    let root = seen.expect("SYNC_REQUEST");
    assert_eq!(root.len(), 32);
    assert_ne!(
        root,
        vec![0u8; 32],
        "nonempty store must not send the zero sentinel"
    );
    assert_eq!(root.as_slice(), expected.as_slice());
}

#[test]
fn empty_store_sends_zero_accepted_root() {
    let a = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let mut b = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let ds = a.datastore_id_hex();
    assert_eq!(b.op_count().unwrap(), 0);

    let relay = Relay::memory();
    let mut sess = relay.accept();
    let mut seen = None;
    relay_client::sync(&mut b, Some(&ds), |frame| {
        if frame_type(frame) == MSG_SYNC_REQUEST {
            seen = Some(sync_accepted_root(frame));
        }
        sess.handle(frame)
            .map_err(|e| StoreError::Invalid(e.to_string()))
    })
    .expect("relay client session");

    let root = seen.expect("SYNC_REQUEST");
    assert_eq!(
        root,
        vec![0u8; 32],
        "empty join must still send the zero sentinel"
    );
}

/// E2-live (not full EXEMPLAR E2): concurrent LWW / ORSet / Flag / PNCounter
/// through an in-process relay. Equal-ts LWW is still the model-level suite.
#[test]
fn concurrent_crdts_converge_through_in_process_relay() {
    let mut a = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "seed").unwrap();
    a.set_add(&node, "tags", "food").unwrap();
    a.set_add(&node, "tags", "urgent").unwrap();
    a.flag_enable(&node, "done").unwrap();
    a.counter_inc(&node, "voteScore", 2).unwrap();

    let relay = Relay::memory();
    let mut sess_a = relay.accept();
    drive(&mut a, &mut sess_a, None);

    let mut b = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let ds = a.datastore_id_hex();
    let mut sess_b = relay.accept();
    drive(&mut b, &mut sess_b, Some(&ds));
    assert_eq!(b.get_lww(&node, "title").unwrap().as_deref(), Some("seed"));

    // Partition: both write without seeing the other peer's new ops.
    a.set_lww(&node, "title", "race-a").unwrap();
    b.set_lww(&node, "title", "race-b").unwrap();
    a.set_add(&node, "tags", "food").unwrap();
    b.set_remove(&node, "tags", "food").unwrap();
    a.flag_enable(&node, "done").unwrap();
    b.flag_disable(&node, "done").unwrap();
    a.counter_inc(&node, "voteScore", 4).unwrap();
    b.counter_dec(&node, "voteScore", 1).unwrap();
    b.counter_inc(&node, "voteScore", 3).unwrap();

    let mut sess_a2 = relay.accept();
    drive(&mut a, &mut sess_a2, None);
    let mut sess_b2 = relay.accept();
    drive(&mut b, &mut sess_b2, None);
    let mut sess_a3 = relay.accept();
    drive(&mut a, &mut sess_a3, None);

    let title_a = a.get_lww(&node, "title").unwrap();
    let title_b = b.get_lww(&node, "title").unwrap();
    assert_eq!(title_a, title_b, "LWW must agree after both-way merge");
    assert!(
        title_a.as_deref() == Some("race-a") || title_a.as_deref() == Some("race-b"),
        "LWW winner must be one concurrent write, got {title_a:?}"
    );

    let tags_a = a.get_prop(&node, "tags").unwrap();
    let tags_b = b.get_prop(&node, "tags").unwrap();
    assert_eq!(tags_a, tags_b);
    let mut tags: Vec<String> = tags_a
        .expect("tags")
        .as_array()
        .expect("orset array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    tags.sort();
    assert_eq!(
        tags,
        vec!["food".to_string(), "urgent".to_string()],
        "concurrent add vs observed-remove must leave food present"
    );

    assert_eq!(
        a.get_prop(&node, "done").unwrap(),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        b.get_prop(&node, "done").unwrap(),
        Some(serde_json::json!(true))
    );
    assert_eq!(
        a.get_prop(&node, "voteScore").unwrap(),
        Some(serde_json::json!(8))
    );
    assert_eq!(
        b.get_prop(&node, "voteScore").unwrap(),
        Some(serde_json::json!(8))
    );

    let mut sess_a4 = relay.accept();
    let rematch = drive(&mut a, &mut sess_a4, None);
    assert_eq!(rematch.received, 0, "re-merge must be a no-op");
    assert_eq!(merkle_of_ds(&a, &ds), merkle_of_ds(&b, &ds));
}

/// E3-lite (not EXEMPLAR E3): 3 peers, C offline, B sqlite close/reopen
/// (not process death), C catch-up from R alone, roots match, resume is a
/// no-op.
#[test]
fn three_peer_offline_catchup_after_sqlite_reopen() {
    let mut a = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "seed").unwrap();
    for i in 0..8 {
        a.set_lww(&node, "note", &format!("a-seed-{i}")).unwrap();
    }

    let relay = Relay::memory();
    let mut sess_a = relay.accept();
    drive(&mut a, &mut sess_a, None);
    let ds = a.datastore_id_hex();

    let path_b = tmp_db("peer-b");
    let mut b = LocalStore::init(&path_b).unwrap();
    let mut sess_b = relay.accept();
    drive(&mut b, &mut sess_b, Some(&ds));

    let mut c = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let mut sess_c = relay.accept();
    let joined = drive(&mut c, &mut sess_c, Some(&ds));
    assert!(
        joined.received >= 10,
        "C must join the seeded list, {joined:?}"
    );
    assert_eq!(c.datastore_id_hex(), ds);

    // C goes offline. A/B write through R; B closes before pushing the tail.
    for i in 0..10 {
        a.set_lww(&node, "note", &format!("a-live-{i}")).unwrap();
    }
    for i in 0..8 {
        b.set_add(&node, "tags", &format!("b{i}")).unwrap();
    }
    let mut sess_a2 = relay.accept();
    drive(&mut a, &mut sess_a2, None);

    b.counter_inc(&node, "voteScore", 5).unwrap();
    b.set_lww(&node, "pending", "unsynced").unwrap();
    drop(b);

    let mut b = LocalStore::open(&path_b).unwrap();
    assert_eq!(b.datastore_id_hex(), ds);
    assert_eq!(
        b.get_lww(&node, "pending").unwrap().as_deref(),
        Some("unsynced"),
        "sqlite reopen must keep B's unsent tail"
    );
    let mut sess_b2 = relay.accept();
    let pushed = drive(&mut b, &mut sess_b2, None);
    assert!(
        pushed.sent >= 2,
        "reopened B must submit the unsent tail, {pushed:?}"
    );
    assert_eq!(
        pushed.ack_accepted + pushed.ack_duplicate,
        pushed.sent,
        "every reopened B op must ACK, {pushed:?}"
    );

    // A and B stay offline. C catch-up uses only R.
    let mut sess_c2 = relay.accept();
    let caught = drive(&mut c, &mut sess_c2, Some(&ds));
    assert!(
        caught.received >= 20,
        "C must receive the partition window from R alone, {caught:?}"
    );
    assert_eq!(
        c.get_lww(&node, "pending").unwrap().as_deref(),
        Some("unsynced"),
        "C must materialize B's unsent write from R alone"
    );
    assert_eq!(
        c.get_prop(&node, "voteScore").unwrap(),
        Some(serde_json::json!(5))
    );
    assert_eq!(
        merkle_of_ds(&b, &ds),
        merkle_of_ds(&c, &ds),
        "C must match B (who already caught A's live writes) using only R"
    );

    // After C is current, A reconnects so all three accepted sets match.
    let mut sess_a3 = relay.accept();
    let a_caught = drive(&mut a, &mut sess_a3, None);
    assert!(
        a_caught.received >= 10,
        "A must catch B's partition writes, {a_caught:?}"
    );
    assert_eq!(
        a.get_lww(&node, "pending").unwrap().as_deref(),
        Some("unsynced")
    );
    assert_eq!(
        a.get_prop(&node, "voteScore").unwrap(),
        Some(serde_json::json!(5))
    );
    let root_a = merkle_of_ds(&a, &ds);
    let root_b = merkle_of_ds(&b, &ds);
    let root_c = merkle_of_ds(&c, &ds);
    assert_eq!(root_a, root_b, "A/B merkle after reconnection");
    assert_eq!(root_a, root_c, "A/C merkle after reconnection");

    let mut sess_c3 = relay.accept();
    let resume = drive(&mut c, &mut sess_c3, Some(&ds));
    assert_eq!(
        resume.received, 0,
        "C resume must not redeliver, {resume:?}"
    );

    remove_sqlite(&path_b);
}
