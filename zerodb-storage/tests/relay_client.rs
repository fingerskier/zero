//! M3a RELAY 0.2.2 client: handshake, push signed LocalStore ops, catch-up.

use zerodb_core::cbor::{self, Cbor};
use zerodb_core::relay::MSG_OPS;
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
