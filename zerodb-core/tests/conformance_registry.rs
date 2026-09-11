//! Both runners load conformance/registry.json. A constant that disagrees
//! with the H9 protocol definition fails this test.

use std::fs;
use std::path::PathBuf;

use serde_json::Value as Json;
use zerodb_core::handshake::{
    DEFAULT_BYTES_PER_SECOND, DEFAULT_MAX_BATCH_BYTES, DEFAULT_MAX_BATCH_OPS,
    DEFAULT_MAX_CONNECTIONS_PER_PEER, DEFAULT_MAX_PAYLOAD_BYTES, DEFAULT_MAX_SUBSCRIPTIONS,
    DEFAULT_OPS_PER_SECOND, DOMAIN_RELAY_AUTH,
};
use zerodb_core::relay::{
    BYTE_FIELDS, ERR_AUTH_FAILED, ERR_CLOCK_DRIFT, ERR_PAYLOAD_TOO_LARGE, ERR_RATE_EXCEEDED,
    ERR_SIG_INVALID, ERR_TOO_MANY_SUBS, MSG_AUTH, MSG_CHALLENGE, MSG_DELTA_BATCH,
    MSG_DELTA_REQUEST, MSG_ERROR, MSG_HELLO, MSG_MERKLE_LEAF_REQUEST, MSG_MERKLE_LEAF_RESPONSE,
    MSG_MERKLE_NODE_REQUEST, MSG_MERKLE_NODE_RESPONSE, MSG_OP_ACK, MSG_OPS, MSG_SUBSCRIBE,
    MSG_SUBSCRIBED, MSG_SYNC_ACK, MSG_SYNC_REQUEST, MSG_SYNC_RESPONSE, MSG_WELCOME, RELAY_CAPS,
};

fn load_registry() -> Json {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/registry.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn registry_agrees_with_rust_constants() {
    let reg = load_registry();
    let wire = &reg["relay_wire"];
    let caps: Vec<&str> = reg["relay_capabilities"]["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert_eq!(caps, RELAY_CAPS);

    let limits = &wire["welcome_limits"];
    assert_eq!(
        limits["max_payload_bytes"].as_u64().unwrap(),
        DEFAULT_MAX_PAYLOAD_BYTES as u64
    );
    assert_eq!(
        limits["max_batch_ops"].as_u64().unwrap(),
        DEFAULT_MAX_BATCH_OPS as u64
    );
    assert_eq!(
        limits["max_batch_bytes"].as_u64().unwrap(),
        DEFAULT_MAX_BATCH_BYTES as u64
    );
    assert_eq!(
        limits["max_subscriptions"].as_u64().unwrap(),
        DEFAULT_MAX_SUBSCRIPTIONS as u64
    );
    assert_eq!(
        limits["ops_per_second"].as_u64().unwrap(),
        DEFAULT_OPS_PER_SECOND as u64
    );
    assert_eq!(
        limits["bytes_per_second"].as_u64().unwrap(),
        DEFAULT_BYTES_PER_SECOND as u64
    );
    assert_eq!(
        limits["max_connections_per_peer"].as_u64().unwrap(),
        DEFAULT_MAX_CONNECTIONS_PER_PEER as u64
    );

    let messages = &wire["messages"];
    let pairs: &[(&str, u8)] = &[
        ("HELLO", MSG_HELLO),
        ("CHALLENGE", MSG_CHALLENGE),
        ("AUTH", MSG_AUTH),
        ("WELCOME", MSG_WELCOME),
        ("SUBSCRIBE", MSG_SUBSCRIBE),
        ("SUBSCRIBED", MSG_SUBSCRIBED),
        ("SYNC_REQUEST", MSG_SYNC_REQUEST),
        ("SYNC_RESPONSE", MSG_SYNC_RESPONSE),
        ("DELTA_REQUEST", MSG_DELTA_REQUEST),
        ("DELTA_BATCH", MSG_DELTA_BATCH),
        ("SYNC_ACK", MSG_SYNC_ACK),
        ("MERKLE_NODE_REQUEST", MSG_MERKLE_NODE_REQUEST),
        ("MERKLE_NODE_RESPONSE", MSG_MERKLE_NODE_RESPONSE),
        ("MERKLE_LEAF_REQUEST", MSG_MERKLE_LEAF_REQUEST),
        ("MERKLE_LEAF_RESPONSE", MSG_MERKLE_LEAF_RESPONSE),
        ("OPS", MSG_OPS),
        ("OP_ACK", MSG_OP_ACK),
        ("ERROR", MSG_ERROR),
    ];
    for (name, ty) in pairs {
        assert_eq!(
            messages[name]["type"].as_u64().unwrap() as u8,
            *ty,
            "{name}"
        );
    }

    let errors = &wire["errors"];
    assert_eq!(
        errors["AUTH_FAILED"]["code"].as_u64().unwrap() as u16,
        ERR_AUTH_FAILED
    );
    assert_eq!(
        errors["UNSIGNED_OP"]["code"].as_u64().unwrap() as u16,
        ERR_SIG_INVALID
    );
    assert_eq!(
        errors["CLOCK_DRIFT"]["code"].as_u64().unwrap() as u16,
        ERR_CLOCK_DRIFT
    );
    assert_eq!(
        errors["PAYLOAD_TOO_LARGE"]["code"].as_u64().unwrap() as u16,
        ERR_PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        errors["RATE_EXCEEDED"]["code"].as_u64().unwrap() as u16,
        ERR_RATE_EXCEEDED
    );
    assert_eq!(
        errors["TOO_MANY_SUBS"]["code"].as_u64().unwrap() as u16,
        ERR_TOO_MANY_SUBS
    );

    let bytes: Vec<&str> = wire["byte_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    let mut got = BYTE_FIELDS.to_vec();
    got.sort();
    let mut want = bytes;
    want.sort();
    assert_eq!(got, want);

    assert_eq!(
        wire["domain_handshake"].as_str().unwrap().as_bytes(),
        DOMAIN_RELAY_AUTH
    );
    assert_eq!(
        reg["peer_rejects"]["max_drift_ms"].as_u64().unwrap(),
        60_000
    );
}
