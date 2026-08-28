//! EXEMPLAR E6 at the relay persist gate: R forwards/persists a sealed
//! LWW SetProperty without learning plaintext. I-10 is peer-side;
//! the relay treats ciphertext like any other signed member op.

use ed25519_dalek::SigningKey;
use std::time::{SystemTime, UNIX_EPOCH};
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::envelope::{ValueContext, seal};
use zerodb_core::op::{OpEnvelope, OpTs, json_to_cbor_body};
use zerodb_core::relay::{
    MSG_AUTH, MSG_HELLO, MSG_OP_ACK, MSG_OPS, MSG_WELCOME, peer_id_from_pk, sign_auth_for_hello,
};
use zerodb_core::sign::sign_op;
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
const SECRET: &str = "relay-must-not-see-this-body";

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
    let mut n = [0u8; 32];
    n.copy_from_slice(nonce);
    let sig = sign_auth_for_hello(sk, pk, &[] as &[&str], &n);
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

fn sealed_note_op(physical_ms: u64) -> Cbor {
    let key = SigningKey::from_bytes(&SK);
    let author_pk = key.verifying_key().to_bytes();
    let author = peer_id_from_pk(&author_pk);
    let ds: [u8; 32] = hex::decode(DS).unwrap().try_into().unwrap();
    let group = [0x42u8; 32];
    let ctx = ValueContext {
        ds,
        author,
        physical_ms,
        logical: 0,
        ep: 1,
        path: "body".into(),
    };
    let envelope = seal(&group, &[0x05u8; 24], &ctx, SECRET.as_bytes());
    let node = [0x44u8; 16];
    let body_json = serde_json::json!({
        "node": hex::encode(node),
        "path": "body",
        "crdt": "lww",
        "encrypted": hex::encode(envelope),
    });
    let env = OpEnvelope {
        v: 1,
        ds,
        ep: 1,
        author,
        ts: OpTs {
            physical_ms,
            logical: 0,
        },
        deps: vec![],
        grp: None,
        kind: 3,
        body: json_to_cbor_body(&body_json).unwrap(),
    };
    let op_id = env.op_id().unwrap();
    let (_, sig) = sign_op(&SK, &op_id);
    let wire = serde_json::json!({
        "id": hex::encode(op_id),
        "v": 1,
        "ds": DS,
        "ep": 1,
        "author": hex::encode(author),
        "author_pk": hex::encode(author_pk),
        "ts": { "p": physical_ms, "l": 0 },
        "deps": [],
        "kind": 3,
        "body": body_json,
        "sig": hex::encode(sig),
    });
    Cbor::Map(vec![
        ("op_id".into(), Cbor::Bytes(op_id.to_vec())),
        ("author".into(), Cbor::Bytes(author.to_vec())),
        ("physical_ms".into(), Cbor::Uint(physical_ms)),
        ("logical".into(), Cbor::Uint(0)),
        ("wire".into(), Cbor::Text(wire.to_string())),
    ])
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[test]
fn e6_honest_relay_persists_ciphertext_not_plaintext() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess, &PK, &SK);
    let op = sealed_note_op(now_ms());
    assert_eq!(
        outcome(&submit(&mut sess, DS, vec![op])[0]),
        ("ACCEPT", None)
    );
    assert_eq!(relay.op_count(DS).unwrap(), 1);
    let captured = relay.captured_artifacts(DS).unwrap();
    assert!(
        !String::from_utf8_lossy(&captured).contains(SECRET),
        "honest relay stored plaintext"
    );
}

#[test]
fn e6_colluding_relay_still_blind() {
    let relay = Relay::memory_colluding();
    let mut sess = relay.accept();
    handshake(&mut sess, &PK, &SK);
    let op = sealed_note_op(now_ms() + 1);
    assert_eq!(
        outcome(&submit(&mut sess, DS, vec![op])[0]),
        ("ACCEPT", None)
    );
    let captured = relay.captured_artifacts(DS).unwrap();
    assert!(
        !String::from_utf8_lossy(&captured).contains(SECRET),
        "colluding relay learned plaintext"
    );
}
