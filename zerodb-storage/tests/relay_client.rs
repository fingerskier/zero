//! M3a RELAY 0.2.2 client: handshake, push signed LocalStore ops, catch-up.

use zerodb_relay::Relay;
use zerodb_storage::relay_client;
use zerodb_storage::{LocalStore, MemoryBackend, StoreError};

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
