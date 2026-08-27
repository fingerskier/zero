//! EXEMPLAR E8 at the relay persist gate: a +30d well-formed op is ACCEPT,
//! not REJECT/CLOCK_DRIFT. H1 is peer-side quarantine; the relay persists so
//! honest peers receive the same op set.

use ed25519_dalek::{Signer, SigningKey};
use std::time::{SystemTime, UNIX_EPOCH};
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::relay::{
    DOMAIN_RELAY_AUTH, MSG_AUTH, MSG_HELLO, MSG_OP_ACK, MSG_OPS, MSG_WELCOME,
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
const PLUS_30D_MS: u64 = 30 * 24 * 60 * 60 * 1000;

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

fn handshake(sess: &mut zerodb_relay::RelaySession, pk: &[u8; 32], sk: &[u8; 32]) {
    let hello = encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            ("peer_id".into(), Cbor::Bytes(peer_id_from_pk(pk).to_vec())),
            ("public_key".into(), Cbor::Bytes(pk.to_vec())),
            ("protocol_version".into(), Cbor::Uint(1)),
            ("capabilities".into(), Cbor::Array(vec![])),
        ]),
    );
    let out = sess.handle(&hello).unwrap();
    let (_, _, pl) = decode_env(&out[0]);
    let nonce = as_bytes(map_get(&pl, "nonce"));
    let key = SigningKey::from_bytes(sk);
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

fn submit(sess: &mut zerodb_relay::RelaySession, ds: &str, ops: Vec<Cbor>) -> Vec<Cbor> {
    let frame = encode_env(
        MSG_OPS,
        9,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(ds.into())),
            ("operations".into(), Cbor::Array(ops)),
        ]),
    );
    let out = sess.handle(&frame).unwrap();
    let (ty, _, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_OP_ACK);
    match map_get(&pl, "outcomes") {
        Cbor::Array(a) => a.clone(),
        _ => panic!("outcomes"),
    }
}

fn outcome(o: &Cbor) -> (&str, Option<&str>) {
    let tag = as_text(map_get(o, "outcome"));
    let reason = match o {
        Cbor::Map(ents) => ents
            .iter()
            .find(|(n, _)| n == "reason")
            .and_then(|(_, v)| match v {
                Cbor::Text(s) => Some(s.as_str()),
                _ => None,
            }),
        _ => None,
    };
    (tag, reason)
}

fn far_future_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + PLUS_30D_MS
}

#[test]
fn e8_honest_relay_persists_far_future_member_op() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess, &PK, &SK);
    let op = mint_experimental_relay_op(&SK, DS, far_future_ms(), 0, 8);
    assert_eq!(
        outcome(&submit(&mut sess, DS, vec![op])[0]),
        ("ACCEPT", None),
        "H1 clock rule is peer-side; honest relay must not drop the op"
    );
    assert_eq!(relay.op_count(DS).unwrap(), 1);
}

#[test]
fn e8_colluding_relay_forwards_far_future_op() {
    let relay = Relay::memory_colluding();
    let mut sess = relay.accept();
    handshake(&mut sess, &PK, &SK);
    let op = mint_experimental_relay_op(&SK, DS, far_future_ms(), 0, 9);
    assert_eq!(
        outcome(&submit(&mut sess, DS, vec![op])[0]),
        ("ACCEPT", None)
    );
    assert_eq!(relay.op_count(DS).unwrap(), 1);
}
