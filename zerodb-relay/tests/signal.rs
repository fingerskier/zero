//! RELAY-SPEC SIGNAL (0x42): opaque forward + 0x307 when the target is down.

use ed25519_dalek::SigningKey;
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::relay::{
    ERR_TARGET_NOT_CONNECTED, MSG_AUTH, MSG_CHALLENGE, MSG_ERROR, MSG_HELLO, MSG_SIGNAL,
    MSG_WELCOME, peer_id_from_pk, sign_auth_for_hello,
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
const SK_B: [u8; 32] = [8u8; 32];

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

fn pk_of(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

fn handshake(sess: &mut zerodb_relay::RelaySession, seed: &[u8; 32], pk: &[u8; 32]) {
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
    let (ty, _, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_CHALLENGE);
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(as_bytes(map_get(&pl, "nonce")));
    let sig = sign_auth_for_hello(seed, pk, &[] as &[&str], &nonce);
    let auth = encode_env(
        MSG_AUTH,
        1,
        Cbor::Map(vec![("signature".into(), Cbor::Bytes(sig.to_vec()))]),
    );
    let out = sess.handle(&auth).unwrap();
    assert_eq!(decode_env(&out[0]).0, MSG_WELCOME);
}

fn signal_frame(target: [u8; 32], payload: &[u8]) -> Vec<u8> {
    encode_env(
        MSG_SIGNAL,
        9,
        Cbor::Map(vec![
            ("target".into(), Cbor::Bytes(target.to_vec())),
            ("payload".into(), Cbor::Bytes(payload.to_vec())),
        ]),
    )
}

#[test]
fn signal_missing_target_is_307() {
    let relay = Relay::memory();
    let mut a = relay.accept();
    handshake(&mut a, &SK, &PK);
    let missing = [0xabu8; 32];
    let out = a.handle(&signal_frame(missing, b"opaque-sdp")).unwrap();
    assert_eq!(out.len(), 1);
    let (ty, rid, pl) = decode_env(&out[0]);
    assert_eq!(ty, MSG_ERROR);
    assert_eq!(rid, 9);
    assert_eq!(
        as_u64(map_get(&pl, "code")),
        ERR_TARGET_NOT_CONNECTED as u64
    );
    assert_eq!(as_text(map_get(&pl, "message")), "TARGET_NOT_CONNECTED");
    assert!(!matches!(map_get(&pl, "fatal"), Cbor::Bool(true)));
}

#[test]
fn signal_forwards_opaque_payload_with_relay_sender() {
    let relay = Relay::memory();
    let mut a = relay.accept();
    let mut b = relay.accept();
    let pk_b = pk_of(&SK_B);
    handshake(&mut a, &SK, &PK);
    handshake(&mut b, &SK_B, &pk_b);

    let blob = b"not-inspected-offer-or-ice";
    let out = a
        .handle(&signal_frame(peer_id_from_pk(&pk_b), blob))
        .unwrap();
    assert!(out.is_empty(), "success has no sender reply");

    let fwd = b.take_outbound();
    assert_eq!(fwd.len(), 1);
    let (ty, rid, pl) = decode_env(&fwd[0]);
    assert_eq!(ty, MSG_SIGNAL);
    assert_eq!(rid, 9);
    assert_eq!(as_bytes(map_get(&pl, "sender")), peer_id_from_pk(&PK));
    assert_eq!(as_bytes(map_get(&pl, "payload")), blob);
    let Cbor::Map(ents) = &pl else {
        panic!("payload map");
    };
    assert!(
        ents.iter().all(|(k, _)| k != "target"),
        "forwarded SIGNAL must drop target"
    );
}
