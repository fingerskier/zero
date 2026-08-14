//! M3a L2 relay process: handshake, persist, dual-root, resume, catch-up.
//! Speaks RELAY 0.2.2-draft envelopes. No membership/authz (M3b).

use ed25519_dalek::{Signer, SigningKey};
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::relay::{
    authenticate, peer_id_from_pk, DIR_RELAY_TO_PEER, DOMAIN_RELAY_AUTH, ERR_AUTH_FAILED, MSG_AUTH,
    MSG_CHALLENGE, MSG_ERROR, MSG_HELLO, MSG_OPS, MSG_OP_ACK, MSG_SUBSCRIBE, MSG_SUBSCRIBED,
    MSG_SYNC_REQUEST, MSG_SYNC_RESPONSE, MSG_WELCOME,
};
use zerodb_relay::{Relay, RelaySession};

const PK: [u8; 32] = [
    0x26, 0xb7, 0x07, 0x2d, 0x6b, 0x2b, 0x0e, 0x99, 0x27, 0xbe, 0x59, 0xf4, 0x7b, 0x3b, 0x9a, 0xb7,
    0xd1, 0x7c, 0x79, 0x67, 0x25, 0xc2, 0x5f, 0x82, 0x69, 0x88, 0x2a, 0xf8, 0x6a, 0x13, 0x06, 0xe1,
];
const SK: [u8; 32] = [
    0x56, 0x02, 0x95, 0x41, 0x1c, 0xb3, 0x77, 0x1a, 0x48, 0x92, 0xc5, 0x3f, 0xab, 0x03, 0x2a, 0xba,
    0xa0, 0xdc, 0x96, 0xb7, 0xa6, 0xed, 0x7b, 0xe6, 0xc6, 0x48, 0x65, 0x55, 0x1d, 0x06, 0x2d, 0xfa,
];
const NONCE: [u8; 32] = [7u8; 32];

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

fn hello(claimed: [u8; 32]) -> Vec<u8> {
    encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            ("peer_id".into(), Cbor::Bytes(claimed.to_vec())),
            ("public_key".into(), Cbor::Bytes(PK.to_vec())),
            ("protocol_version".into(), Cbor::Uint(1)),
            (
                "capabilities".into(),
                Cbor::Array(vec![
                    Cbor::Text("dual-root".into()),
                    Cbor::Text("resume-cursor".into()),
                    Cbor::Text("reject-ack".into()),
                ]),
            ),
        ]),
    )
}

fn auth_for(nonce: &[u8; 32]) -> Vec<u8> {
    auth_for_rid(nonce, 1)
}

fn auth_for_rid(nonce: &[u8; 32], request_id: u32) -> Vec<u8> {
    let key = SigningKey::from_bytes(&SK);
    let mut msg = DOMAIN_RELAY_AUTH.to_vec();
    msg.extend_from_slice(nonce);
    let sig = key.sign(&msg).to_bytes();
    encode_env(
        MSG_AUTH,
        request_id,
        Cbor::Map(vec![("signature".into(), Cbor::Bytes(sig.to_vec()))]),
    )
}

fn handshake(sess: &mut RelaySession) {
    let out = sess.handle(&hello(peer_id_from_pk(&PK))).unwrap();
    assert_eq!(out.len(), 1);
    let (ty, _, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_CHALLENGE);
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(as_bytes(map_get(&pl, "nonce")));
    let out = sess.handle(&auth_for(&nonce)).unwrap();
    assert_eq!(out.len(), 1);
    let (ty, _, _) = decode_env(&out[0]);
    assert_eq!(ty, MSG_WELCOME);
}

fn op_map(id: u8, author: u8, ms: u64) -> Cbor {
    Cbor::Map(vec![
        ("op_id".into(), Cbor::Bytes(vec![id; 32])),
        ("author".into(), Cbor::Bytes(vec![author; 32])),
        ("physical_ms".into(), Cbor::Uint(ms)),
        ("logical".into(), Cbor::Uint(0)),
    ])
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "zerodb-relay-{name}-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn hello_challenge_auth_welcome() {
    let relay = Relay::memory();
    relay.set_next_nonce(NONCE);
    let mut sess = relay.accept();
    let out = sess.handle(&hello(peer_id_from_pk(&PK))).unwrap();
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_CHALLENGE);
    assert_eq!(rid, 1);
    assert_eq!(as_bytes(map_get(&pl, "nonce")), &NONCE);

    let out = sess.handle(&auth_for(&NONCE)).unwrap();
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_WELCOME);
    assert_eq!(rid, 1);
    assert_eq!(as_u64(map_get(&pl, "relay_level")), 2);
    let caps = match map_get(&pl, "capabilities") {
        Cbor::Array(a) => a
            .iter()
            .map(|c| match c {
                Cbor::Text(s) => s.as_str(),
                _ => panic!(),
            })
            .collect::<Vec<_>>(),
        _ => panic!(),
    };
    assert_eq!(caps, ["dual-root", "reject-ack", "resume-cursor"]);
    assert!(sess.is_authed());
}

#[test]
fn welcome_correlates_with_auth_request_id() {
    let relay = Relay::memory();
    relay.set_next_nonce(NONCE);
    let mut sess = relay.accept();
    let out = sess.handle(&hello(peer_id_from_pk(&PK))).unwrap();
    let (ty, rid, _) = decode_env(&out[0]);
    assert_eq!(ty, MSG_CHALLENGE);
    assert_eq!(rid, 1);

    let out = sess.handle(&auth_for_rid(&NONCE, 9)).unwrap();
    let (ty, rid, _) = decode_env(&out[0]);
    assert_eq!(ty, MSG_WELCOME);
    assert_eq!(rid, 9);
    assert!(sess.is_authed());
}

#[test]
fn hello_unsupported_version_is_fatal() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    let frame = encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            ("peer_id".into(), Cbor::Bytes(peer_id_from_pk(&PK).to_vec())),
            ("public_key".into(), Cbor::Bytes(PK.to_vec())),
            ("protocol_version".into(), Cbor::Uint(2)),
            ("capabilities".into(), Cbor::Array(vec![])),
        ]),
    );
    let out = sess.handle(&frame).unwrap();
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_ERROR);
    assert_eq!(rid, 1);
    assert_eq!(as_u64(map_get(&pl, "code")), 0x102);
    assert_eq!(as_text(map_get(&pl, "message")), "VERSION_MISMATCH");
    assert!(matches!(map_get(&pl, "fatal"), Cbor::Bool(true)));
    assert!(sess.is_closed());
    assert!(!sess.is_authed());
}

#[test]
fn unauthenticated_ops_emits_fatal_and_closes() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    let ops = encode_env(
        MSG_OPS,
        7,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text("app:main".into())),
            ("operations".into(), Cbor::Array(vec![op_map(1, 0xaa, 10)])),
        ]),
    );
    let out = sess.handle(&ops).unwrap();
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_ERROR);
    assert_eq!(rid, 7);
    assert_eq!(as_u64(map_get(&pl, "code")), ERR_AUTH_FAILED as u64);
    assert!(matches!(map_get(&pl, "fatal"), Cbor::Bool(true)));
    assert!(sess.is_closed());
    assert!(!sess.is_authed());
}

#[test]
fn bad_signature_is_auth_failed() {
    let relay = Relay::memory();
    relay.set_next_nonce(NONCE);
    let mut sess = relay.accept();
    sess.handle(&hello(peer_id_from_pk(&PK))).unwrap();
    let bad = encode_env(
        MSG_AUTH,
        1,
        Cbor::Map(vec![("signature".into(), Cbor::Bytes(vec![0u8; 64]))]),
    );
    let out = sess.handle(&bad).unwrap();
    let (ty, _, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_ERROR);
    assert_eq!(as_u64(map_get(&pl, "code")), ERR_AUTH_FAILED as u64);
    assert!(!sess.is_authed());
    assert!(sess.is_closed());
}

#[test]
fn claimed_peer_id_mismatch_is_auth_failed() {
    let relay = Relay::memory();
    relay.set_next_nonce(NONCE);
    let mut sess = relay.accept();
    sess.handle(&hello([0xff; 32])).unwrap();
    let out = sess.handle(&auth_for(&NONCE)).unwrap();
    let (ty, _, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_ERROR);
    assert_eq!(as_u64(map_get(&pl, "code")), ERR_AUTH_FAILED as u64);
    let _ = authenticate(&[0xff; 32], &PK, &NONCE, &[0u8; 64]);
}

#[test]
fn persist_ops_and_duplicate() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let ops = encode_env(
        MSG_OPS,
        7,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text("app:main".into())),
            ("operations".into(), Cbor::Array(vec![op_map(1, 0xaa, 10)])),
        ]),
    );
    let out = sess.handle(&ops).unwrap();
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_OP_ACK);
    assert_eq!(rid, 7);
    let outcomes = match map_get(&pl, "outcomes") {
        Cbor::Array(a) => a,
        _ => panic!(),
    };
    assert_eq!(as_text(map_get(&outcomes[0], "outcome")), "ACCEPT");

    let out = sess.handle(&ops).unwrap();
    let (_, _, pl) = decode_env(&out[0]);
    let outcomes = match map_get(&pl, "outcomes") {
        Cbor::Array(a) => a,
        _ => panic!(),
    };
    assert_eq!(as_text(map_get(&outcomes[0], "outcome")), "DUPLICATE");
}

#[test]
fn reject_op_without_id() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let ops = encode_env(
        MSG_OPS,
        2,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text("app:main".into())),
            (
                "operations".into(),
                Cbor::Array(vec![Cbor::Map(vec![(
                    "author".into(),
                    Cbor::Bytes(vec![1; 32]),
                )])]),
            ),
        ]),
    );
    let (_, _, pl) = decode_env(&sess.handle(&ops).unwrap()[0]);
    let outcomes = match map_get(&pl, "outcomes") {
        Cbor::Array(a) => a,
        _ => panic!(),
    };
    assert_eq!(as_text(map_get(&outcomes[0], "outcome")), "REJECT");
    assert_eq!(as_text(map_get(&outcomes[0], "reason")), "DECODE");
}

#[test]
fn sync_response_carries_validated_root_not_accepted() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    sess.handle(&encode_env(
        MSG_OPS,
        3,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text("app:main".into())),
            (
                "operations".into(),
                Cbor::Array(vec![op_map(1, 0xaa, 1000), op_map(2, 0xaa, 2000)]),
            ),
        ]),
    ))
    .unwrap();
    let sync = encode_env(
        MSG_SYNC_REQUEST,
        4,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text("app:main".into())),
            ("accepted_root".into(), Cbor::Bytes(vec![0u8; 32])),
        ]),
    );
    let out = sess.handle(&sync).unwrap();
    let (ty, _, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_SYNC_RESPONSE);
    assert!(matches!(map_get(&pl, "validated_root"), Cbor::Bytes(b) if b.len() == 32));
    if let Cbor::Map(ents) = &pl {
        assert!(
            !ents.iter().any(|(k, _)| k == "accepted_root"),
            "relay must not publish accepted_root"
        );
    }
}

#[test]
fn catch_up_sends_ops_not_covered_by_cursor() {
    let relay = Relay::memory();
    let mut a = relay.accept();
    handshake(&mut a);
    a.handle(&encode_env(
        MSG_OPS,
        5,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text("app:main".into())),
            (
                "operations".into(),
                Cbor::Array(vec![
                    op_map(1, 0xaa, 50),
                    op_map(2, 0xaa, 200),
                    op_map(3, 0xcc, 80),
                ]),
            ),
        ]),
    ))
    .unwrap();

    let mut b = relay.accept();
    handshake(&mut b);
    let frontier = Cbor::Map(vec![(
        "aa".repeat(32),
        Cbor::Map(vec![
            ("op_id".into(), Cbor::Bytes(vec![2; 32])),
            ("physical_ms".into(), Cbor::Uint(200)),
            ("logical".into(), Cbor::Uint(0)),
        ]),
    )]);
    let sync = encode_env(
        MSG_SYNC_REQUEST,
        8,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text("app:main".into())),
            ("accepted_root".into(), Cbor::Bytes(vec![0u8; 32])),
            (
                "cursor".into(),
                Cbor::Map(vec![
                    ("epoch".into(), Cbor::Uint(1)),
                    ("frontier".into(), frontier),
                ]),
            ),
        ]),
    );
    let out = b.handle(&sync).unwrap();
    assert!(
        out.iter().any(|f| decode_env(f).0 == MSG_SYNC_RESPONSE),
        "SYNC_RESPONSE"
    );
    let ops = out
        .iter()
        .find(|f| decode_env(f).0 == MSG_OPS)
        .expect("relay must push uncovered ops");
    let (_, rid, pl) = decode_env(ops);
    assert_eq!(rid, 0, "unsolicited forward");
    let operations = match map_get(&pl, "operations") {
        Cbor::Array(a) => a,
        _ => panic!(),
    };
    assert_eq!(operations.len(), 1);
    assert_eq!(as_bytes(map_get(&operations[0], "op_id")), &[3u8; 32]);
}

#[test]
fn durable_reopen_keeps_validated_ops() {
    let path = tmp_path("durable");
    {
        let relay = Relay::open(&path).unwrap();
        let mut sess = relay.accept();
        handshake(&mut sess);
        sess.handle(&encode_env(
            MSG_OPS,
            9,
            Cbor::Map(vec![
                ("datastore".into(), Cbor::Text("app:main".into())),
                ("operations".into(), Cbor::Array(vec![op_map(9, 0xaa, 1)])),
            ]),
        ))
        .unwrap();
    }
    let relay = Relay::open(&path).unwrap();
    assert_eq!(relay.op_count("app:main").unwrap(), 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn e3_lite_offline_peer_catchup_from_relay_only() {
    let relay = Relay::memory();
    let mut a = relay.accept();
    handshake(&mut a);
    a.handle(&encode_env(
        MSG_OPS,
        10,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text("app:main".into())),
            (
                "operations".into(),
                Cbor::Array(vec![op_map(1, 0xaa, 1), op_map(2, 0xaa, 2)]),
            ),
        ]),
    ))
    .unwrap();

    let mut c = relay.accept();
    handshake(&mut c);
    let out = c
        .handle(&encode_env(
            MSG_SYNC_REQUEST,
            11,
            Cbor::Map(vec![
                ("datastore".into(), Cbor::Text("app:main".into())),
                ("accepted_root".into(), Cbor::Bytes(vec![0u8; 32])),
            ]),
        ))
        .unwrap();
    let ops = out
        .iter()
        .find(|f| decode_env(f).0 == MSG_OPS)
        .expect("C must catch up from R alone");
    let env = decode_env(ops);
    let operations = match map_get(&env.2, "operations") {
        Cbor::Array(a) => a,
        _ => panic!(),
    };
    assert_eq!(operations.len(), 2);
}

#[test]
fn subscribe_reports_validated_root() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let out = sess
        .handle(&encode_env(
            MSG_SUBSCRIBE,
            12,
            Cbor::Map(vec![(
                "datastores".into(),
                Cbor::Array(vec![Cbor::Text("app:main".into())]),
            )]),
        ))
        .unwrap();
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_SUBSCRIBED);
    assert_eq!(rid, 12);
    let ds = match map_get(&pl, "datastores") {
        Cbor::Array(a) => &a[0],
        _ => panic!(),
    };
    assert!(matches!(map_get(ds, "validated_root"), Cbor::Bytes(b) if b.len() == 32));
    let _ = DIR_RELAY_TO_PEER;
    let _ = MSG_SUBSCRIBED;
}
