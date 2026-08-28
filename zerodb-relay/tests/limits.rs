//! Advertised WELCOME payload/batch limits are enforced (Stage 1).
//! Connection quotas / TLS / datastore-creation policy stay pinned.

use ed25519_dalek::{Signer, SigningKey};
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::relay::{
    DOMAIN_RELAY_AUTH, ERR_PAYLOAD_TOO_LARGE, MSG_AUTH, MSG_ERROR, MSG_HELLO, MSG_OPS, MSG_WELCOME,
    mint_experimental_relay_op, peer_id_from_pk,
};
use zerodb_relay::Relay;

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
    let key = SigningKey::from_bytes(&SK);
    let mut msg = DOMAIN_RELAY_AUTH.to_vec();
    msg.extend_from_slice(nonce);
    let sig = key.sign(&msg).to_bytes();
    let auth = encode_env(
        MSG_AUTH,
        1,
        Cbor::Map(vec![("signature".into(), Cbor::Bytes(sig.to_vec()))]),
    );
    let out = sess.handle(&auth).unwrap();
    assert_eq!(decode_env(&out[0]).0, MSG_WELCOME);
}

#[test]
fn oversized_frame_rejected_before_decode() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    let huge = vec![0u8; 1_048_576 + 1];
    let out = sess.handle(&huge).unwrap();
    let (ty, _, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_ERROR);
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
    let (ty, _, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_ERROR);
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
