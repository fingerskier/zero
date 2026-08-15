//! M3a RELAY 0.2.2 client: handshake, push signed LocalStore ops, catch-up.

use zerodb_core::cbor::{self, Cbor};
use zerodb_core::merkle::{MerkleOp, merkle_root};
use zerodb_core::relay::{MSG_OPS, MSG_SYNC_REQUEST};
use zerodb_relay::Relay;
use zerodb_storage::relay_client;
use zerodb_storage::{LocalStore, MemoryBackend, StoreError};

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

fn merkle_of_ds(store: &LocalStore<MemoryBackend>, ds: &str) -> [u8; 32] {
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

fn drive(
    store: &mut LocalStore<MemoryBackend>,
    sess: &mut zerodb_relay::RelaySession,
    join_ds: Option<&str>,
) -> relay_client::RelaySyncSummary {
    relay_client::sync(store, join_ds, |frame| {
        sess.handle(frame)
            .map_err(|e| StoreError::Invalid(e.to_string()))
    })
    .expect("relay client session")
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
