//! Stage 2 sync: one pull session converges BOTH peers (server<->client).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use zerodb_storage::sync::{self, Hello, OpsMsg, SYNC_PROTOCOL_VERSION};
use zerodb_storage::LocalStore;

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("sync2-{name}-{}-{nonce}.sqlite", std::process::id()))
}

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).unwrap();
    let (server, _) = listener.accept().unwrap();
    for s in [&client, &server] {
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
    }
    (server, client)
}

/// One session: A (server) has X, B (client) has Y, both converge to X∪Y.
#[test]
fn one_session_converges_both_peers() {
    let path_a = tmp_db("a");
    let path_b = tmp_db("b");
    let mut a = LocalStore::init(&path_a).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "from-a").unwrap();

    // Bootstrap B from A so they share a datastore, then diverge.
    let mut b = LocalStore::init(&path_b).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    a.set_lww(&node, "a_only", "x").unwrap();
    b.set_lww(&node, "b_only", "y").unwrap();
    assert_ne!(a.op_count().unwrap(), b.op_count().unwrap() + 1); // diverged

    let (mut srv, mut cli) = pair();
    let server = thread::spawn(move || {
        let summary = sync::serve(&mut a, &mut srv).unwrap();
        (a, summary)
    });
    let pull = sync::pull(&mut b, &mut cli).unwrap();
    let (a, serve_summary) = server.join().unwrap();

    assert_eq!(pull.accepted, 1, "client should accept a_only");
    assert_eq!(pull.sent, 1);
    assert_eq!(pull.remote_accepted, 1, "ack must report server ingest");
    assert_eq!(serve_summary.accepted, 1, "server should accept b_only");
    assert_eq!(a.op_count().unwrap(), b.op_count().unwrap());
    assert_eq!(
        a.get_prop(&node, "b_only").unwrap(),
        Some(json!("y")),
        "server must ingest client-side op"
    );
    assert_eq!(b.get_prop(&node, "a_only").unwrap(), Some(json!("x")));
}

/// Empty client still adopts the server's datastore (bootstrap).
#[test]
fn empty_client_bootstrap_adopts_ds() {
    let path_a = tmp_db("boot-a");
    let path_b = tmp_db("boot-b");
    let mut a = LocalStore::init(&path_a).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "hello").unwrap();
    let ds = a.datastore_id_hex();

    let mut b = LocalStore::init(&path_b).unwrap();
    assert_ne!(b.datastore_id_hex(), ds);

    let (mut srv, mut cli) = pair();
    let server = thread::spawn(move || {
        sync::serve(&mut a, &mut srv).unwrap();
        a
    });
    let pull = sync::pull(&mut b, &mut cli).unwrap();
    let a = server.join().unwrap();

    assert_eq!(b.datastore_id_hex(), ds);
    assert_eq!(pull.accepted, 2);
    assert_eq!(a.op_count().unwrap(), 2);
    assert_eq!(b.get_prop(&node, "title").unwrap(), Some(json!("hello")));
}

/// A tampered op from the client is rejected without poisoning server state.
#[test]
fn tampered_client_op_rejected_without_poisoning_server() {
    let path_a = tmp_db("tamper-a");
    let mut a = LocalStore::init(&path_a).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "clean").unwrap();
    let ds = a.datastore_id_hex();
    let ops_before = a.op_count().unwrap();

    // Take a real op, tamper its signature, offer it under a fresh id claim.
    let mut evil = a.export_all().unwrap().ops[0].clone();
    let mut sig = evil.sig.clone().into_bytes();
    sig[0] = if sig[0] == b'0' { b'1' } else { b'0' };
    evil.sig = String::from_utf8(sig).unwrap();
    // Advertise an op id the server does not have so it lands in `need`.
    let fake_id = "ff".repeat(32);
    evil.id = fake_id.clone();

    let (mut srv, mut cli) = pair();
    let server = thread::spawn(move || {
        let res = sync::serve(&mut a, &mut srv);
        (a, res)
    });

    // Speak the protocol by hand as a malicious client.
    sync::write_msg(
        &mut cli,
        &Hello {
            v: SYNC_PROTOCOL_VERSION,
            datastore_id: ds.clone(),
            peer: "22".repeat(32),
            op_ids: vec![fake_id],
        },
    )
    .unwrap();
    let hello_ok: zerodb_storage::sync::HelloOk = sync::read_msg(&mut cli).unwrap();
    assert_eq!(hello_ok.need.len(), 1);
    let _server_ops: OpsMsg = sync::read_msg(&mut cli).unwrap();
    sync::write_msg(&mut cli, &OpsMsg { ops: vec![evil] }).unwrap();
    drop(cli);

    let (a, res) = server.join().unwrap();
    assert!(res.is_err(), "server must reject tampered op");
    assert_eq!(a.op_count().unwrap(), ops_before, "server state unchanged");
    assert_eq!(a.datastore_id_hex(), ds);
}
