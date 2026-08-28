//! Advertised WELCOME limits: payload/batch plus session rate, subscriptions,
//! per-PeerId connection cap, and plaintext-listen policy.

use zerodb_core::cbor::{self, Cbor};
use zerodb_core::relay::{
    AuthTranscript, ERR_AUTH_FAILED, ERR_PAYLOAD_TOO_LARGE, ERR_RATE_EXCEEDED, ERR_TOO_MANY_SUBS,
    MSG_AUTH, MSG_ERROR, MSG_HELLO, MSG_OPS, MSG_SUBSCRIBE, MSG_WELCOME,
    mint_experimental_relay_op, peer_id_from_pk, sign_auth, sign_auth_for_hello,
    sign_auth_v1_nonce_only,
};
use zerodb_relay::{MAX_FRAME_BYTES, Relay};

const PK: [u8; 32] = [
    0x26, 0xb7, 0x07, 0x2d, 0x6b, 0x2b, 0x0e, 0x99, 0x27, 0xbe, 0x59, 0xf4, 0x7b, 0x3b, 0x9a, 0xb7,
    0xd1, 0x7c, 0x79, 0x67, 0x25, 0xc2, 0x5f, 0x82, 0x69, 0x88, 0x2a, 0xf8, 0x6a, 0x13, 0x06, 0xe1,
];
const SK: [u8; 32] = [
    0x56, 0x02, 0x95, 0x41, 0x1c, 0xb3, 0x77, 0x1a, 0x48, 0x92, 0xc5, 0x3f, 0xab, 0x03, 0x2a, 0xba,
    0xa0, 0xdc, 0x96, 0xb7, 0xa6, 0xed, 0x7b, 0xe6, 0xc6, 0x48, 0x65, 0x55, 0x1d, 0x06, 0x2d, 0xfa,
];
const DS: &str = "1111111111111111111111111111111111111111111111111111111111111111";

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

fn handshake(sess: &mut zerodb_relay::RelaySession) {
    let hello = encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            ("peer_id".into(), Cbor::Bytes(peer_id_from_pk(&PK).to_vec())),
            ("public_key".into(), Cbor::Bytes(PK.to_vec())),
            ("protocol_version".into(), Cbor::Uint(1)),
            ("capabilities".into(), Cbor::Array(vec![])),
        ]),
    );
    let out = sess.handle(&hello).unwrap();
    let (_, _, pl) = decode_env(&out[0]);
    let nonce = as_bytes(map_get(&pl, "nonce"));
    let mut n = [0u8; 32];
    n.copy_from_slice(nonce);
    let sig = sign_auth_for_hello(&SK, &PK, &[] as &[&str], &n);
    let auth = encode_env(
        MSG_AUTH,
        1,
        Cbor::Map(vec![("signature".into(), Cbor::Bytes(sig.to_vec()))]),
    );
    let out = sess.handle(&auth).unwrap();
    assert_eq!(decode_env(&out[0]).0, MSG_WELCOME);
}

fn pad_op(op: Cbor, pad: usize) -> Cbor {
    let Cbor::Map(mut ents) = op else {
        panic!("op map");
    };
    ents.push(("pad".into(), Cbor::Bytes(vec![0; pad])));
    Cbor::Map(ents)
}

#[test]
fn oversized_frame_rejected_before_decode() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    let huge = vec![0u8; MAX_FRAME_BYTES + 1];
    let out = sess.handle(&huge).unwrap();
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_ERROR);
    assert_eq!(rid, 0);
    assert_eq!(as_u64(map_get(&pl, "code")), ERR_PAYLOAD_TOO_LARGE as u64);
    assert_eq!(as_text(map_get(&pl, "message")), "PAYLOAD_TOO_LARGE");
    assert_eq!(relay.op_count(DS).unwrap(), 0);
}

#[test]
fn multi_op_over_one_mib_under_batch_is_decoded() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let ops = vec![
        pad_op(mint_experimental_relay_op(&SK, DS, 40, 0, 1), 600_000),
        pad_op(mint_experimental_relay_op(&SK, DS, 41, 0, 2), 600_000),
    ];
    let frame = encode_env(
        MSG_OPS,
        11,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(DS.into())),
            ("operations".into(), Cbor::Array(ops)),
        ]),
    );
    assert!(frame.len() > 1_048_576);
    assert!(frame.len() <= MAX_FRAME_BYTES);
    let out = sess.handle(&frame).unwrap();
    let (ty, rid, _) = decode_env(&out[0]);
    assert_eq!(rid, 11);
    assert_ne!(ty, MSG_ERROR, "aggregate >1 MiB must not be frame-rejected");
    assert_eq!(ty, zerodb_core::relay::MSG_OP_ACK);
}

#[test]
fn single_op_over_payload_rejected_with_request_id() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let op = pad_op(mint_experimental_relay_op(&SK, DS, 50, 0, 1), 1_048_576 + 8);
    let frame = encode_env(
        MSG_OPS,
        12,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(DS.into())),
            ("operations".into(), Cbor::Array(vec![op])),
        ]),
    );
    assert!(frame.len() <= MAX_FRAME_BYTES);
    let out = sess.handle(&frame).unwrap();
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_ERROR);
    assert_eq!(rid, 12);
    assert_eq!(as_u64(map_get(&pl, "code")), ERR_PAYLOAD_TOO_LARGE as u64);
    assert_eq!(as_text(map_get(&pl, "message")), "PAYLOAD_TOO_LARGE");
    assert_eq!(relay.op_count(DS).unwrap(), 0);
}

#[test]
fn batch_over_max_ops_rejected_zero_writes() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let ops: Vec<Cbor> = (0..65)
        .map(|i| mint_experimental_relay_op(&SK, DS, 10 + i, 0, 1))
        .collect();
    let frame = encode_env(
        MSG_OPS,
        9,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(DS.into())),
            ("operations".into(), Cbor::Array(ops)),
        ]),
    );
    assert!(
        frame.len() <= 1_048_576,
        "65 small ops must be under payload cap so the batch-ops check is what fires"
    );
    let out = sess.handle(&frame).unwrap();
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_ERROR);
    assert_eq!(rid, 9);
    assert_eq!(as_u64(map_get(&pl, "code")), ERR_PAYLOAD_TOO_LARGE as u64);
    assert_eq!(relay.op_count(DS).unwrap(), 0);
}

#[test]
fn sqlite_batch_insert_accepts_and_dedups() {
    let path = {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target")
            .join(format!("relay-batch-{nonce}.sqlite"));
        for s in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{s}", p.display()));
        }
        p
    };
    let relay = Relay::open(&path).unwrap();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let ops: Vec<Cbor> = (0..8)
        .map(|i| mint_experimental_relay_op(&SK, DS, 20 + i, 0, 1))
        .collect();
    let frame = encode_env(
        MSG_OPS,
        9,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(DS.into())),
            ("operations".into(), Cbor::Array(ops.clone())),
        ]),
    );
    let out = sess.handle(&frame).unwrap();
    assert_eq!(decode_env(&out[0]).0, zerodb_core::relay::MSG_OP_ACK);
    assert_eq!(relay.op_count(DS).unwrap(), 8);

    let again = sess.handle(&frame).unwrap();
    let (_, _, pl) = decode_env(&again[0]);
    let Cbor::Array(outcomes) = map_get(&pl, "outcomes") else {
        panic!("outcomes");
    };
    for o in outcomes {
        assert_eq!(as_text(map_get(o, "outcome")), "DUPLICATE");
    }
    assert_eq!(relay.op_count(DS).unwrap(), 8);
}

fn subscribe_ids(ids: &[String]) -> Vec<u8> {
    encode_env(
        MSG_SUBSCRIBE,
        20,
        Cbor::Map(vec![(
            "datastores".into(),
            Cbor::Array(ids.iter().map(|id| Cbor::Text(id.clone())).collect()),
        )]),
    )
}

fn error_code_message(frame: &[u8]) -> (u16, String) {
    let (ty, _, pl) = decode_env(frame);
    assert_eq!(ty, MSG_ERROR);
    (
        as_u64(map_get(&pl, "code")) as u16,
        as_text(map_get(&pl, "message")).to_string(),
    )
}

#[test]
fn v1_nonce_only_auth_is_auth_failed() {
    let relay = Relay::memory();
    relay.set_next_nonce([7u8; 32]);
    let mut sess = relay.accept();
    let hello = encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            ("peer_id".into(), Cbor::Bytes(peer_id_from_pk(&PK).to_vec())),
            ("public_key".into(), Cbor::Bytes(PK.to_vec())),
            ("protocol_version".into(), Cbor::Uint(1)),
            ("capabilities".into(), Cbor::Array(vec![])),
        ]),
    );
    sess.handle(&hello).unwrap();
    let sig = sign_auth_v1_nonce_only(&SK, &[7u8; 32]);
    let auth = encode_env(
        MSG_AUTH,
        1,
        Cbor::Map(vec![("signature".into(), Cbor::Bytes(sig.to_vec()))]),
    );
    let out = sess.handle(&auth).unwrap();
    let (code, message) = error_code_message(&out[0]);
    assert_eq!(code, ERR_AUTH_FAILED);
    assert_eq!(message, "AUTH_FAILED");
    assert!(sess.is_closed());
}

#[test]
fn flipped_limits_or_version_auth_fails_honest_welcomes() {
    let relay = Relay::memory();
    relay.set_next_nonce([7u8; 32]);
    let mut sess = relay.accept();
    let hello = encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            ("peer_id".into(), Cbor::Bytes(peer_id_from_pk(&PK).to_vec())),
            ("public_key".into(), Cbor::Bytes(PK.to_vec())),
            ("protocol_version".into(), Cbor::Uint(1)),
            ("capabilities".into(), Cbor::Array(vec![])),
        ]),
    );
    sess.handle(&hello).unwrap();
    let honest =
        AuthTranscript::for_relay_hello(peer_id_from_pk(&PK), PK, 1, &[] as &[&str], [7u8; 32]);
    let mut flipped = honest.clone();
    flipped.limits.ops_per_second ^= 1;
    let auth = encode_env(
        MSG_AUTH,
        1,
        Cbor::Map(vec![(
            "signature".into(),
            Cbor::Bytes(sign_auth(&SK, &flipped).to_vec()),
        )]),
    );
    let out = sess.handle(&auth).unwrap();
    let (code, _) = error_code_message(&out[0]);
    assert_eq!(code, ERR_AUTH_FAILED);

    let mut sess = relay.accept();
    handshake(&mut sess);
    assert!(sess.is_authed());
}

#[test]
fn over_subscribe_is_too_many_subs_zero_new() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let first: Vec<String> = (0..64).map(|i| format!("{i:064x}")).collect();
    let out = sess.handle(&subscribe_ids(&first)).unwrap();
    assert_eq!(decode_env(&out[0]).0, zerodb_core::relay::MSG_SUBSCRIBED);

    let extra = vec![format!("{:064x}", 64u64)];
    let out = sess.handle(&subscribe_ids(&extra)).unwrap();
    let (code, message) = error_code_message(&out[0]);
    assert_eq!(code, ERR_TOO_MANY_SUBS);
    assert_eq!(message, "TOO_MANY_SUBS");

    let sixty_five: Vec<String> = (100..165).map(|i| format!("{i:064x}")).collect();
    let relay2 = Relay::memory();
    let mut sess2 = relay2.accept();
    handshake(&mut sess2);
    let out = sess2.handle(&subscribe_ids(&sixty_five)).unwrap();
    let (code, message) = error_code_message(&out[0]);
    assert_eq!(code, ERR_TOO_MANY_SUBS);
    assert_eq!(message, "TOO_MANY_SUBS");
}

#[test]
fn over_rate_is_rate_exceeded_zero_writes() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let first: Vec<Cbor> = (0..64)
        .map(|i| mint_experimental_relay_op(&SK, DS, 100 + i, 0, 1))
        .collect();
    let frame = encode_env(
        MSG_OPS,
        30,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(DS.into())),
            ("operations".into(), Cbor::Array(first)),
        ]),
    );
    let out = sess.handle(&frame).unwrap();
    assert_eq!(decode_env(&out[0]).0, zerodb_core::relay::MSG_OP_ACK);
    assert_eq!(relay.op_count(DS).unwrap(), 64);

    let second: Vec<Cbor> = (0..64)
        .map(|i| mint_experimental_relay_op(&SK, DS, 200 + i, 0, 1))
        .collect();
    let frame = encode_env(
        MSG_OPS,
        31,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(DS.into())),
            ("operations".into(), Cbor::Array(second)),
        ]),
    );
    let out = sess.handle(&frame).unwrap();
    let (code, message) = error_code_message(&out[0]);
    assert_eq!(code, ERR_RATE_EXCEEDED);
    assert_eq!(message, "RATE_EXCEEDED");
    assert_eq!(relay.op_count(DS).unwrap(), 64);
}

#[test]
fn fourth_connection_same_peer_is_closed() {
    let relay = Relay::memory();
    let mut live = Vec::new();
    for _ in 0..3 {
        let mut sess = relay.accept();
        handshake(&mut sess);
        assert!(sess.is_authed());
        live.push(sess);
    }
    let mut fourth = relay.accept();
    let hello = encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            ("peer_id".into(), Cbor::Bytes(peer_id_from_pk(&PK).to_vec())),
            ("public_key".into(), Cbor::Bytes(PK.to_vec())),
            ("protocol_version".into(), Cbor::Uint(1)),
            ("capabilities".into(), Cbor::Array(vec![])),
        ]),
    );
    let out = fourth.handle(&hello).unwrap();
    let (_, _, pl) = decode_env(&out[0]);
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(as_bytes(map_get(&pl, "nonce")));
    let auth = encode_env(
        MSG_AUTH,
        1,
        Cbor::Map(vec![(
            "signature".into(),
            Cbor::Bytes(sign_auth_for_hello(&SK, &PK, &[] as &[&str], &nonce).to_vec()),
        )]),
    );
    let out = fourth.handle(&auth).unwrap();
    let (code, message) = error_code_message(&out[0]);
    assert_eq!(code, ERR_RATE_EXCEEDED);
    assert_eq!(message, "TOO_MANY_CONNECTIONS");
    assert!(fourth.is_closed());
    drop(live);
}

#[test]
fn plaintext_wildcard_bind_refuses_without_flag() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_zerodb-relay"))
        .args([
            "--path",
            "/tmp/zerodb-relay-insecure-refuse.sqlite",
            "--bind",
            "0.0.0.0:17991",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "plaintext 0.0.0.0 must refuse without --allow-insecure; stderr={stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("allow-insecure")
            || stderr.to_lowercase().contains("loopback"),
        "error should mention --allow-insecure: {stderr}"
    );
}
