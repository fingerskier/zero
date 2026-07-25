//! v2 push capability: a persistent session propagates mutations both ways
//! without a new session, and the capability flags stay wire-compatible with
//! old (push-unaware) peers.

use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zerodb_storage::sync::{self, Hello, HelloOk};
use zerodb_storage::{LocalStore, MemoryBackend};

type Shared = Arc<Mutex<Option<LocalStore<MemoryBackend>>>>;

fn shared(store: LocalStore<MemoryBackend>) -> Shared {
    Arc::new(Mutex::new(Some(store)))
}

fn with<R>(s: &Shared, f: impl FnOnce(&mut LocalStore<MemoryBackend>) -> R) -> R {
    f(s.lock().unwrap().as_mut().unwrap())
}

fn wait_for(mut cond: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timeout waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn push_session_propagates_without_new_session() {
    let a = shared(LocalStore::init_with_backend(MemoryBackend::new()).unwrap());
    let b = shared(LocalStore::init_with_backend(MemoryBackend::new()).unwrap());
    let seed_node = with(&a, |s| s.create_node("Todo").unwrap());
    with(&a, |s| s.set_lww(&seed_node, "title", "from-a").unwrap());

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let gen_a = Arc::new(AtomicU64::new(0));
    let gen_b = Arc::new(AtomicU64::new(0));

    let server = {
        let (a, stop, gen_a) = (a.clone(), stop.clone(), gen_a.clone());
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut stream = stream;
            sync::serve_push(
                &a,
                &mut stream,
                true,
                &|| gen_a.load(Ordering::SeqCst),
                &|| stop.load(Ordering::SeqCst),
                |_, _| {},
                |s: &mut TcpStream| s.set_read_timeout(Some(Duration::from_millis(50))).unwrap(),
                |_| {},
            )
            .unwrap()
        })
    };

    let client = {
        let (b, stop, gen_b) = (b.clone(), stop.clone(), gen_b.clone());
        std::thread::spawn(move || {
            let stream = TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut stream = stream;
            sync::pull_push(
                &b,
                &mut stream,
                &|| gen_b.load(Ordering::SeqCst),
                &|| stop.load(Ordering::SeqCst),
                |_, _| {},
                |s: &mut TcpStream| s.set_read_timeout(Some(Duration::from_millis(50))).unwrap(),
                |_| {},
            )
            .unwrap()
        })
    };

    // Initial exchange: empty B adopts A's datastore and ops.
    wait_for(
        || with(&b, |s| s.get_lww(&seed_node, "title").unwrap()) == Some("from-a".into()),
        "initial bootstrap",
    );

    // Server-side mutation arrives at the client with no new session.
    with(&a, |s| s.set_lww(&seed_node, "title", "pushed").unwrap());
    gen_a.fetch_add(1, Ordering::SeqCst);
    wait_for(
        || with(&b, |s| s.get_lww(&seed_node, "title").unwrap()) == Some("pushed".into()),
        "server->client push",
    );

    // Client-side mutation arrives at the server symmetrically.
    let b_node = with(&b, |s| s.create_node("FromB").unwrap());
    gen_b.fetch_add(1, Ordering::SeqCst);
    wait_for(
        || {
            with(&a, |s| s.list_nodes().unwrap())
                .iter()
                .any(|(id, _, _)| *id == b_node)
        },
        "client->server push",
    );
    assert_eq!(
        with(&a, |s| s.op_count().unwrap()),
        with(&b, |s| s.op_count().unwrap())
    );

    stop.store(true, Ordering::SeqCst);
    let (_summary, push) = server.join().unwrap();
    assert!(push, "server should have negotiated push");
    let (_summary, push) = client.join().unwrap();
    assert!(push, "client should have negotiated push");
}

#[test]
fn push_client_against_plain_serve_falls_back() {
    // Old-style server (plain serve, no push) + push-requesting client:
    // one-shot session still converges; client reports push=false.
    let a = shared(LocalStore::init_with_backend(MemoryBackend::new()).unwrap());
    let b = shared(LocalStore::init_with_backend(MemoryBackend::new()).unwrap());
    let node = with(&a, |s| s.create_node("Todo").unwrap());

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = {
        let a = a.clone();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut guard = a.lock().unwrap();
            sync::serve(guard.as_mut().unwrap(), &mut stream).unwrap()
        })
    };
    let mut stream = TcpStream::connect(addr).unwrap();
    let (summary, push) =
        sync::pull_push(&b, &mut stream, &|| 0, &|| false, |_, _| {}, |_| {}, |_| {}).unwrap();
    server.join().unwrap();
    assert!(!push, "plain serve must not ack push");
    assert_eq!(summary.accepted, 1);
    assert!(
        with(&b, |s| s.list_nodes().unwrap())
            .iter()
            .any(|(id, _, _)| *id == node)
    );
}

#[test]
fn hello_capability_fields_are_compatible() {
    // Old peer's Hello (no push field) deserializes with push=false.
    let old = r#"{"v":2,"datastore_id":"d","peer":"p","op_ids":[]}"#;
    let hello: Hello = serde_json::from_str(old).unwrap();
    assert!(!hello.push);
    // New Hello with unknown extra fields is tolerated (no deny_unknown_fields).
    let newer = r#"{"v":2,"datastore_id":"d","peer":"p","op_ids":[],"push":true,"future":1}"#;
    let hello: Hello = serde_json::from_str(newer).unwrap();
    assert!(hello.push);
    let ok = r#"{"v":2,"datastore_id":"d","peer":"p","need":[]}"#;
    let hello_ok: HelloOk = serde_json::from_str(ok).unwrap();
    assert!(!hello_ok.push);
}
