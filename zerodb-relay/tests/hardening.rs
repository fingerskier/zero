//! Relay transport hardening (before network exposure): pre-decode message
//! ceiling, handshake deadline, idle timeout, global connection cap, slot
//! release on disconnect, PING/PONG + GOODBYE, and in-process TLS (wss).
//! Live sockets on loopback; no external services.

use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::handshake::sign_auth_for_hello;
use zerodb_core::relay::{
    ERR_PAYLOAD_TOO_LARGE, ERR_RATE_EXCEEDED, MSG_AUTH, MSG_CHALLENGE, MSG_ERROR, MSG_GOODBYE,
    MSG_HELLO, MSG_PING, MSG_PONG, MSG_WELCOME, peer_id_from_pk,
};
use zerodb_relay::{ListenConfig, MAX_FRAME_BYTES, Relay, spawn_listener_with};

const SEED: [u8; 32] = [
    0x56, 0x02, 0x95, 0x41, 0x1c, 0xb3, 0x77, 0x1a, 0x48, 0x92, 0xc5, 0x3f, 0xab, 0x03, 0x2a, 0xba,
    0xa0, 0xdc, 0x96, 0xb7, 0xa6, 0xed, 0x7b, 0xe6, 0xc6, 0x48, 0x65, 0x55, 0x1d, 0x06, 0x2d, 0xfa,
];

fn map_get<'a>(m: &'a Cbor, k: &str) -> &'a Cbor {
    match m {
        Cbor::Map(ents) => ents
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("missing {k}")),
        _ => panic!("not a map"),
    }
}

fn as_u64(v: &Cbor) -> u64 {
    match v {
        Cbor::Uint(n) => *n,
        _ => panic!("not uint"),
    }
}

fn as_bytes(v: &Cbor) -> &[u8] {
    match v {
        Cbor::Bytes(b) => b,
        _ => panic!("not bytes"),
    }
}

fn as_text(v: &Cbor) -> &str {
    match v {
        Cbor::Text(s) => s,
        _ => panic!("not text"),
    }
}

fn decode_env(bytes: &[u8]) -> (u8, u32, Cbor) {
    let c = cbor::decode(bytes).expect("cbor");
    (
        as_u64(map_get(&c, "type")) as u8,
        as_u64(map_get(&c, "request_id")) as u32,
        map_get(&c, "payload").clone(),
    )
}

fn encode_env(ty: u8, request_id: u32, payload: Cbor) -> Vec<u8> {
    cbor::encode(&Cbor::Map(vec![
        ("type".into(), Cbor::Uint(ty as u64)),
        ("request_id".into(), Cbor::Uint(request_id as u64)),
        ("payload".into(), payload),
    ]))
    .unwrap()
}

fn pk() -> [u8; 32] {
    SigningKey::from_bytes(&SEED).verifying_key().to_bytes()
}

fn peer_id() -> [u8; 32] {
    peer_id_from_pk(&pk())
}

fn hello_frame() -> Vec<u8> {
    encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            ("peer_id".into(), Cbor::Bytes(peer_id().to_vec())),
            ("public_key".into(), Cbor::Bytes(pk().to_vec())),
            ("protocol_version".into(), Cbor::Uint(1)),
            ("capabilities".into(), Cbor::Array(vec![])),
        ]),
    )
}

fn auth_frame(nonce: &[u8]) -> Vec<u8> {
    let mut n = [0u8; 32];
    n.copy_from_slice(nonce);
    let sig = sign_auth_for_hello(&SEED, &pk(), &[] as &[&str], &n);
    encode_env(
        MSG_AUTH,
        1,
        Cbor::Map(vec![("signature".into(), Cbor::Bytes(sig.to_vec()))]),
    )
}

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

fn connect(addr: std::net::SocketAddr, read_timeout: Duration) -> Ws {
    let (ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{}/v1/relay", addr.port()))
        .expect("ws connect");
    if let MaybeTlsStream::Plain(s) = ws.get_ref() {
        let _ = s.set_read_timeout(Some(read_timeout));
    }
    ws
}

/// Next binary frame, skipping WebSocket control frames. `None` on close/EOF.
fn next_binary<S: std::io::Read + std::io::Write>(ws: &mut WebSocket<S>) -> Option<Vec<u8>> {
    loop {
        match ws.read() {
            Ok(Message::Binary(b)) => return Some(b),
            Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => return None,
            Ok(_) => {}
            Err(e) => panic!("read: {e}"),
        }
    }
}

fn handshake<S: std::io::Read + std::io::Write>(ws: &mut WebSocket<S>) {
    ws.send(Message::Binary(hello_frame())).unwrap();
    let (ty, _, pl) = decode_env(&next_binary(ws).expect("challenge"));
    assert_eq!(ty, MSG_CHALLENGE);
    ws.send(Message::Binary(auth_frame(as_bytes(map_get(&pl, "nonce")))))
        .unwrap();
    let (ty, _, _) = decode_env(&next_binary(ws).expect("welcome"));
    assert_eq!(ty, MSG_WELCOME);
}

fn error_of(frame: &[u8]) -> (u16, String, bool) {
    let (ty, _, pl) = decode_env(frame);
    assert_eq!(ty, MSG_ERROR);
    (
        as_u64(map_get(&pl, "code")) as u16,
        as_text(map_get(&pl, "message")).to_string(),
        matches!(map_get(&pl, "fatal"), Cbor::Bool(true)),
    )
}

fn fast_cfg() -> ListenConfig {
    ListenConfig {
        handshake_timeout: Duration::from_secs(5),
        idle_timeout: Some(Duration::from_secs(30)),
        max_connections: 8,
        tls: None,
    }
}

fn wait_until(deadline: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f()
}

#[test]
fn ping_pong_echoes_timestamp_in_any_phase_and_goodbye_closes() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    let ping = encode_env(
        MSG_PING,
        7,
        Cbor::Map(vec![("timestamp".into(), Cbor::Uint(1_700_000_000_123))]),
    );
    let out = sess.handle(&ping).unwrap();
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!((ty, rid), (MSG_PONG, 7));
    assert_eq!(as_u64(map_get(&pl, "timestamp")), 1_700_000_000_123);
    assert!(!sess.is_authed(), "PING must not change phase");

    let bad = encode_env(
        MSG_PING,
        8,
        Cbor::Map(vec![("timestamp".into(), Cbor::Text("x".into()))]),
    );
    let (code, msg, fatal) = error_of(&sess.handle(&bad).unwrap()[0]);
    assert_eq!(
        (code, msg.as_str(), fatal),
        (0x103, "MALFORMED_MESSAGE", false)
    );

    let out = sess.handle(&hello_frame()).unwrap();
    let (_, _, pl) = decode_env(&out[0]);
    let out = sess
        .handle(&auth_frame(as_bytes(map_get(&pl, "nonce"))))
        .unwrap();
    assert_eq!(decode_env(&out[0]).0, MSG_WELCOME);
    let out = sess.handle(&ping).unwrap();
    assert_eq!(decode_env(&out[0]).0, MSG_PONG);

    let bye = encode_env(
        MSG_GOODBYE,
        9,
        Cbor::Map(vec![("reason".into(), Cbor::Uint(0))]),
    );
    assert!(sess.handle(&bye).unwrap().is_empty());
    assert!(sess.is_closed());
    // Per-peer slot released: three fresh sessions may AUTH again.
    for _ in 0..3 {
        let mut s = relay.accept();
        let out = s.handle(&hello_frame()).unwrap();
        let (_, _, pl) = decode_env(&out[0]);
        let out = s
            .handle(&auth_frame(as_bytes(map_get(&pl, "nonce"))))
            .unwrap();
        assert_eq!(decode_env(&out[0]).0, MSG_WELCOME);
        std::mem::forget(s);
    }
}

#[test]
fn global_connection_cap_refuses_then_releases_on_disconnect() {
    let relay = Arc::new(Relay::memory());
    let cfg = ListenConfig {
        max_connections: 1,
        ..fast_cfg()
    };
    let addr = spawn_listener_with("127.0.0.1:0", relay.clone(), cfg).unwrap();

    let mut first = connect(addr, Duration::from_secs(5));
    handshake(&mut first);
    assert_eq!(relay.live_connections(), 1);

    let mut second = connect(addr, Duration::from_secs(5));
    let frame = next_binary(&mut second).expect("refusal frame");
    let (code, msg, fatal) = error_of(&frame);
    assert_eq!(code, ERR_RATE_EXCEEDED);
    assert_eq!(msg, "TOO_MANY_CONNECTIONS");
    assert!(fatal);
    assert!(
        next_binary(&mut second).is_none(),
        "refused socket is closed"
    );
    assert_eq!(
        relay.live_connections(),
        1,
        "refused socket never held a slot"
    );

    drop(first);
    assert!(wait_until(Duration::from_secs(5), || relay
        .live_connections()
        == 0));
    let mut third = connect(addr, Duration::from_secs(5));
    handshake(&mut third);
    assert_eq!(relay.live_connections(), 1);
}

#[test]
fn silent_socket_is_closed_at_handshake_deadline() {
    let relay = Arc::new(Relay::memory());
    let cfg = ListenConfig {
        handshake_timeout: Duration::from_millis(300),
        ..fast_cfg()
    };
    let addr = spawn_listener_with("127.0.0.1:0", relay.clone(), cfg).unwrap();
    let mut ws = connect(addr, Duration::from_secs(5));
    let t0 = Instant::now();
    let frame = next_binary(&mut ws).expect("deadline error");
    let (code, msg, fatal) = error_of(&frame);
    assert_eq!(
        (code, msg.as_str(), fatal),
        (0x100, "HANDSHAKE_TIMEOUT", true)
    );
    assert!(next_binary(&mut ws).is_none());
    assert!(t0.elapsed() < Duration::from_secs(4));
    assert!(wait_until(Duration::from_secs(5), || relay
        .live_connections()
        == 0));
}

#[test]
fn raw_tcp_client_that_never_upgrades_does_not_hold_a_slot_forever() {
    let relay = Arc::new(Relay::memory());
    let cfg = ListenConfig {
        handshake_timeout: Duration::from_millis(300),
        max_connections: 1,
        ..fast_cfg()
    };
    let addr = spawn_listener_with("127.0.0.1:0", relay.clone(), cfg).unwrap();
    let _raw = TcpStream::connect(addr).unwrap();
    // The accept thread takes the only slot while it waits for an upgrade...
    assert!(wait_until(Duration::from_secs(5), || relay
        .live_connections()
        == 1));
    // ...then the upgrade read times out, the thread exits, and the slot is released.
    assert!(wait_until(Duration::from_secs(5), || relay
        .live_connections()
        == 0));
    let mut ws = connect(addr, Duration::from_secs(5));
    handshake(&mut ws);
}

#[test]
fn trickling_upgrade_bytes_cannot_outlive_the_handshake_deadline() {
    use std::io::Write;
    let relay = Arc::new(Relay::memory());
    let cfg = ListenConfig {
        handshake_timeout: Duration::from_millis(400),
        max_connections: 1,
        ..fast_cfg()
    };
    let addr = spawn_listener_with("127.0.0.1:0", relay.clone(), cfg).unwrap();
    let mut loris = TcpStream::connect(addr).unwrap();
    assert!(wait_until(Duration::from_secs(5), || relay
        .live_connections()
        == 1));
    // Slow-loris: one byte of a plausible HTTP upgrade every 50 ms — each
    // read succeeds, so the per-read socket timeout alone would never fire.
    let request = b"GET /v1/relay HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
    let t0 = Instant::now();
    let mut dropped_at = None;
    for byte in request.iter() {
        if loris.write_all(std::slice::from_ref(byte)).is_err() {
            dropped_at = Some(t0.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        if relay.live_connections() == 0 {
            dropped_at = Some(t0.elapsed());
            break;
        }
    }
    let dropped_at =
        dropped_at.expect("relay must drop the trickling client before the request completes");
    assert!(
        dropped_at < Duration::from_secs(3),
        "dropped after {dropped_at:?}"
    );
    assert!(wait_until(Duration::from_secs(5), || relay
        .live_connections()
        == 0));
    // Slot is free again for an honest client.
    let mut ws = connect(addr, Duration::from_secs(5));
    handshake(&mut ws);
}

#[test]
fn idle_session_gets_goodbye_and_close() {
    let relay = Arc::new(Relay::memory());
    let cfg = ListenConfig {
        idle_timeout: Some(Duration::from_millis(400)),
        ..fast_cfg()
    };
    let addr = spawn_listener_with("127.0.0.1:0", relay.clone(), cfg).unwrap();
    let mut ws = connect(addr, Duration::from_secs(5));
    handshake(&mut ws);
    // Keepalive resets the clock: a protocol PING at half the idle window
    // keeps the session up past one full window.
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(200));
        ws.send(Message::Binary(encode_env(
            MSG_PING,
            3,
            Cbor::Map(vec![("timestamp".into(), Cbor::Uint(1))]),
        )))
        .unwrap();
        let (ty, _, _) = decode_env(&next_binary(&mut ws).expect("pong"));
        assert_eq!(ty, MSG_PONG);
    }
    let t0 = Instant::now();
    let frame = next_binary(&mut ws).expect("goodbye");
    let (ty, _, pl) = decode_env(&frame);
    assert_eq!(ty, MSG_GOODBYE);
    assert_eq!(as_text(map_get(&pl, "message")), "IDLE_TIMEOUT");
    assert!(t0.elapsed() >= Duration::from_millis(300));
    assert!(next_binary(&mut ws).is_none());
    assert!(wait_until(Duration::from_secs(5), || relay
        .live_connections()
        == 0));
}

#[test]
fn oversized_ws_message_is_refused_before_decode() {
    let relay = Arc::new(Relay::memory());
    let addr = spawn_listener_with("127.0.0.1:0", relay.clone(), fast_cfg()).unwrap();
    let mut ws = connect(addr, Duration::from_secs(10));
    handshake(&mut ws);
    // One byte over the ceiling: tungstenite refuses the message; the relay
    // answers 0x303 fatal and closes. Nothing reaches `RelaySession::handle`.
    let big = vec![0xa0u8; MAX_FRAME_BYTES + 1];
    let _ = ws.send(Message::Binary(big));
    let _ = ws.flush();
    let mut saw = None;
    for _ in 0..3 {
        match ws.read() {
            Ok(Message::Binary(b)) => {
                saw = Some(error_of(&b));
                break;
            }
            Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(_)) | Err(tungstenite::Error::Protocol(_)) => break,
            Err(e) => panic!("read: {e}"),
        }
    }
    if let Some((code, msg, fatal)) = saw {
        assert_eq!(
            (code, msg.as_str(), fatal),
            (ERR_PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE", true)
        );
    }
    // Either way the socket is gone and the slot is released.
    assert!(wait_until(Duration::from_secs(5), || relay
        .live_connections()
        == 0));
    // The relay is still healthy for the next client.
    let mut again = connect(addr, Duration::from_secs(5));
    handshake(&mut again);
}

fn self_signed() -> (
    rustls_pki_types::CertificateDer<'static>,
    rustls_pki_types::PrivateKeyDer<'static>,
    String,
    String,
) {
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_der = ck.cert.der().clone();
    let key_der = rustls_pki_types::PrivateKeyDer::Pkcs8(ck.key_pair.serialize_der().into());
    (
        cert_der,
        key_der,
        ck.cert.pem(),
        ck.key_pair.serialize_pem(),
    )
}

fn tls_client(
    addr: std::net::SocketAddr,
    root: rustls_pki_types::CertificateDer<'static>,
) -> WebSocket<rustls::StreamOwned<rustls::ClientConnection, TcpStream>> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(root).unwrap();
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let name = rustls_pki_types::ServerName::try_from("localhost").unwrap();
    let conn = rustls::ClientConnection::new(Arc::new(cfg), name).unwrap();
    let tcp = TcpStream::connect(addr).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let stream = rustls::StreamOwned::new(conn, tcp);
    let (ws, _) = tungstenite::client(format!("wss://localhost:{}/v1/relay", addr.port()), stream)
        .expect("wss client");
    ws
}

#[test]
fn tls_listener_serves_wss_and_rejects_plaintext_upgrade() {
    let (cert, key, _, _) = self_signed();
    let tls = zerodb_relay::tls_config_from_der(vec![cert.clone()], key).unwrap();
    let relay = Arc::new(Relay::memory());
    let cfg = ListenConfig {
        tls: Some(tls),
        ..fast_cfg()
    };
    let addr = spawn_listener_with("127.0.0.1:0", relay.clone(), cfg).unwrap();

    let mut ws = tls_client(addr, cert);
    handshake(&mut ws);
    ws.send(Message::Binary(encode_env(
        MSG_PING,
        3,
        Cbor::Map(vec![("timestamp".into(), Cbor::Uint(42))]),
    )))
    .unwrap();
    let (ty, _, pl) = decode_env(&next_binary(&mut ws).unwrap());
    assert_eq!(ty, MSG_PONG);
    assert_eq!(as_u64(map_get(&pl, "timestamp")), 42);
    assert_eq!(relay.live_connections(), 1);

    // A plaintext WebSocket upgrade against the TLS listener fails.
    let plain = tungstenite::connect(format!("ws://127.0.0.1:{}/v1/relay", addr.port()));
    assert!(
        plain.is_err(),
        "plaintext upgrade must not succeed on a wss listener"
    );
}

#[test]
fn load_tls_config_reads_pem_files_and_binary_accepts_flags() {
    let (_, _, cert_pem, key_pem) = self_signed();
    let dir = std::env::temp_dir().join(format!("zerodb-relay-tls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, cert_pem).unwrap();
    std::fs::write(&key_path, key_pem).unwrap();
    zerodb_relay::load_tls_config(&cert_path, &key_path).expect("pem load");
    assert!(zerodb_relay::load_tls_config(&key_path, &cert_path).is_err());

    // The binary: --tls-cert without --tls-key is a usage error; with both,
    // a non-loopback bind no longer needs --allow-insecure (we only check
    // the argument gate, not a live listen on 0.0.0.0).
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_zerodb-relay"))
        .args([
            "--tls-cert",
            cert_path.to_str().unwrap(),
            "--bind",
            "127.0.0.1:0",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("tls-key"));
    let _ = std::fs::remove_dir_all(&dir);
}
