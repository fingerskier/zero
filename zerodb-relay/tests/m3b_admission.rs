//! M3b-sig: relay rejects unsigned, forged, and wrong-datastore ops.
//! Membership / EXEMPLAR E5 is `e5_membership`.

use zerodb_core::cbor::{self, Cbor};
use zerodb_core::relay::{
    MSG_AUTH, MSG_HELLO, MSG_OP_ACK, MSG_OPS, MSG_WELCOME, mint_experimental_relay_op,
    peer_id_from_pk, sign_auth_for_hello,
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
const OTHER_DS: &str = "2222222222222222222222222222222222222222222222222222222222222222";

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

fn set_field(op: &Cbor, key: &str, value: Cbor) -> Cbor {
    match op {
        Cbor::Map(entries) => Cbor::Map(
            entries
                .iter()
                .map(|(name, current)| {
                    if name == key {
                        (name.clone(), value.clone())
                    } else {
                        (name.clone(), current.clone())
                    }
                })
                .collect(),
        ),
        _ => panic!("not a map"),
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

#[test]
fn signed_op_is_accepted() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let op = mint_experimental_relay_op(&SK, DS, 10, 0, 1);
    let out = submit(&mut sess, DS, vec![op]);
    assert_eq!(outcome(&out[0]), ("ACCEPT", None));
    assert_eq!(relay.op_count(DS).unwrap(), 1);
}

#[test]
fn unsigned_op_is_rejected_sig() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let unsigned = Cbor::Map(vec![
        ("op_id".into(), Cbor::Bytes(vec![1; 32])),
        ("author".into(), Cbor::Bytes(vec![2; 32])),
        ("physical_ms".into(), Cbor::Uint(1)),
        ("logical".into(), Cbor::Uint(0)),
    ]);
    let out = submit(&mut sess, DS, vec![unsigned]);
    assert_eq!(outcome(&out[0]), ("REJECT", Some("SIG")));
    assert_eq!(relay.op_count(DS).unwrap(), 0);
}

#[test]
fn forged_signature_is_rejected_sig() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let op = mint_experimental_relay_op(&SK, DS, 10, 0, 1);
    let wire = match map_get(&op, "wire") {
        Cbor::Text(s) => s.clone(),
        _ => panic!("wire"),
    };
    let mut v: serde_json::Value = serde_json::from_str(&wire).unwrap();
    let mut sig = hex::decode(v["sig"].as_str().unwrap()).unwrap();
    sig[0] ^= 0xff;
    v["sig"] = serde_json::json!(hex::encode(sig));
    let forged = Cbor::Map(vec![
        ("op_id".into(), map_get(&op, "op_id").clone()),
        ("author".into(), map_get(&op, "author").clone()),
        ("physical_ms".into(), Cbor::Uint(10)),
        ("logical".into(), Cbor::Uint(0)),
        ("wire".into(), Cbor::Text(v.to_string())),
    ]);
    let out = submit(&mut sess, DS, vec![forged]);
    assert_eq!(outcome(&out[0]), ("REJECT", Some("SIG")));
    assert_eq!(relay.op_count(DS).unwrap(), 0);
}

#[test]
fn wrong_datastore_is_rejected_authz() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let op = mint_experimental_relay_op(&SK, OTHER_DS, 10, 0, 1);
    let out = submit(&mut sess, DS, vec![op]);
    assert_eq!(outcome(&out[0]), ("REJECT", Some("AUTHZ")));
    assert_eq!(relay.op_count(DS).unwrap(), 0);
    assert_eq!(relay.op_count(OTHER_DS).unwrap(), 0);
}

#[test]
fn tampered_body_is_rejected_sig() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let op = mint_experimental_relay_op(&SK, DS, 10, 0, 1);
    let wire = match map_get(&op, "wire") {
        Cbor::Text(s) => s.clone(),
        _ => panic!("wire"),
    };
    let mut v: serde_json::Value = serde_json::from_str(&wire).unwrap();
    v["body"]["label"] = serde_json::json!("tampered");
    let tampered = Cbor::Map(vec![
        ("op_id".into(), map_get(&op, "op_id").clone()),
        ("author".into(), map_get(&op, "author").clone()),
        ("physical_ms".into(), Cbor::Uint(10)),
        ("logical".into(), Cbor::Uint(0)),
        ("wire".into(), Cbor::Text(v.to_string())),
    ]);
    let out = submit(&mut sess, DS, vec![tampered]);
    assert_eq!(outcome(&out[0]), ("REJECT", Some("SIG")));
    assert_eq!(relay.op_count(DS).unwrap(), 0);
}

#[test]
fn changed_datastore_cannot_reuse_signature() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let op = mint_experimental_relay_op(&SK, DS, 10, 0, 1);
    let wire = as_text(map_get(&op, "wire"));
    let mut v: serde_json::Value = serde_json::from_str(wire).unwrap();
    v["ds"] = serde_json::Value::String(OTHER_DS.into());
    let tampered = set_field(&op, "wire", Cbor::Text(v.to_string()));
    let out = submit(&mut sess, OTHER_DS, vec![tampered]);
    assert_eq!(outcome(&out[0]), ("REJECT", Some("SIG")));
    assert_eq!(relay.op_count(OTHER_DS).unwrap(), 0);
}

#[test]
fn non_hex_datastore_is_rejected_decode() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let op = mint_experimental_relay_op(&SK, DS, 10, 0, 1);
    let wire = as_text(map_get(&op, "wire"));
    let mut v: serde_json::Value = serde_json::from_str(wire).unwrap();
    v["ds"] = serde_json::Value::String("app:main".into());
    let tampered = set_field(&op, "wire", Cbor::Text(v.to_string()));
    let out = submit(&mut sess, "app:main", vec![tampered]);
    assert_eq!(outcome(&out[0]), ("REJECT", Some("DECODE")));
    assert_eq!(relay.op_count("app:main").unwrap(), 0);
}

#[test]
fn logical_overflow_is_rejected_decode() {
    let relay = Relay::memory();
    let mut sess = relay.accept();
    handshake(&mut sess);
    let op = mint_experimental_relay_op(&SK, DS, 10, 0, 1);
    let wire = as_text(map_get(&op, "wire"));
    let mut v: serde_json::Value = serde_json::from_str(wire).unwrap();
    v["ts"]["l"] = serde_json::json!(65536_u64);
    let tampered = set_field(&op, "wire", Cbor::Text(v.to_string()));
    let tampered = set_field(&tampered, "logical", Cbor::Uint(65536));
    let out = submit(&mut sess, DS, vec![tampered]);
    assert_eq!(outcome(&out[0]), ("REJECT", Some("DECODE")));
    assert_eq!(relay.op_count(DS).unwrap(), 0);
}
