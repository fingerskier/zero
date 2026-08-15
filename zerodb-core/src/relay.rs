//! RELAY 0.2.2-draft wire-transcript model (M3a).
//!
//! Handshake (HELLO/AUTH/WELCOME capabilities), dual-root catch-up
//! invariant (CX-08), resume cursor, reject-ack, and ordered envelope
//! frames. No relay process.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::merkle::{MerkleOp, merkle_root};

/// Domain-separation prefix (registry `domain_separation.relay_auth`).
pub const DOMAIN_RELAY_AUTH: &[u8] = b"zerodb-relay-auth-v1";

/// Negotiable session capabilities (sorted).
pub const RELAY_CAPS: &[&str] = &["dual-root", "merkle-walk-v1", "reject-ack", "resume-cursor"];

/// RELAY §10 `AUTH_FAILED`.
pub const ERR_AUTH_FAILED: u16 = 0x201;

/// Envelope direction (RELAY §4).
pub const DIR_PEER_TO_RELAY: &str = "P→R";
pub const DIR_RELAY_TO_PEER: &str = "R→P";

/// Envelope `type` codes (RELAY Appendix A).
pub const MSG_HELLO: u8 = 0x01;
pub const MSG_CHALLENGE: u8 = 0x02;
pub const MSG_AUTH: u8 = 0x03;
pub const MSG_WELCOME: u8 = 0x04;
pub const MSG_SUBSCRIBE: u8 = 0x10;
pub const MSG_SUBSCRIBED: u8 = 0x11;
pub const MSG_SYNC_REQUEST: u8 = 0x20;
pub const MSG_SYNC_RESPONSE: u8 = 0x21;
pub const MSG_DELTA_REQUEST: u8 = 0x22;
pub const MSG_DELTA_BATCH: u8 = 0x23;
pub const MSG_SYNC_ACK: u8 = 0x24;
pub const MSG_MERKLE_NODE_REQUEST: u8 = 0x25;
pub const MSG_MERKLE_NODE_RESPONSE: u8 = 0x26;
pub const MSG_MERKLE_LEAF_REQUEST: u8 = 0x27;
pub const MSG_MERKLE_LEAF_RESPONSE: u8 = 0x28;
pub const MSG_OPS: u8 = 0x30;
pub const MSG_OP_ACK: u8 = 0x31;
pub const MSG_ERROR: u8 = 0xff;

pub fn peer_id_from_pk(pk: &[u8; 32]) -> [u8; 32] {
    *blake3::hash(pk).as_bytes()
}

pub fn negotiate_capabilities<'a>(hello: &[&'a str], relay: &[&'a str]) -> Vec<&'a str> {
    RELAY_CAPS
        .iter()
        .copied()
        .filter(|c| hello.contains(c) && relay.contains(c))
        .collect()
}

pub fn auth_preimage(nonce: &[u8]) -> Vec<u8> {
    [DOMAIN_RELAY_AUTH, nonce].concat()
}

pub fn sign_auth(seed: &[u8; 32], nonce: &[u8; 32]) -> [u8; 64] {
    let key = SigningKey::from_bytes(seed);
    key.sign(&auth_preimage(nonce)).to_bytes()
}

pub fn verify_auth(pk: &[u8; 32], nonce: &[u8; 32], sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(pk) else {
        return false;
    };
    vk.verify(&auth_preimage(nonce), &Signature::from_bytes(sig))
        .is_ok()
}

/// RELAY §4.1 / §5.2: signature over domain||nonce AND claimed PeerId == BLAKE3(pk).
pub fn authenticate(
    claimed_peer_id: &[u8; 32],
    public_key: &[u8; 32],
    nonce: &[u8; 32],
    signature: &[u8; 64],
) -> Result<(), u16> {
    if verify_auth(public_key, nonce, signature) && peer_id_from_pk(public_key) == *claimed_peer_id
    {
        Ok(())
    } else {
        Err(ERR_AUTH_FAILED)
    }
}

pub fn known_message_type(ty: u8) -> bool {
    matches!(
        ty,
        MSG_HELLO
            | MSG_CHALLENGE
            | MSG_AUTH
            | MSG_WELCOME
            | MSG_SUBSCRIBE
            | MSG_SUBSCRIBED
            | MSG_ERROR
            | MSG_SYNC_REQUEST
            | MSG_SYNC_RESPONSE
            | MSG_DELTA_REQUEST
            | MSG_DELTA_BATCH
            | MSG_SYNC_ACK
            | MSG_MERKLE_NODE_REQUEST
            | MSG_MERKLE_NODE_RESPONSE
            | MSG_MERKLE_LEAF_REQUEST
            | MSG_MERKLE_LEAF_RESPONSE
            | MSG_OPS
            | MSG_OP_ACK
    )
}

/// Fixed direction for unidirectional types. Bidirectional types return `None`.
pub fn fixed_direction(ty: u8) -> Option<&'static str> {
    match ty {
        MSG_HELLO | MSG_AUTH => Some(DIR_PEER_TO_RELAY),
        MSG_CHALLENGE | MSG_WELCOME | MSG_OP_ACK => Some(DIR_RELAY_TO_PEER),
        _ => None,
    }
}

pub fn required_payload_keys(ty: u8) -> &'static [&'static str] {
    match ty {
        MSG_HELLO => &["peer_id", "public_key", "protocol_version", "capabilities"],
        MSG_CHALLENGE => &["nonce"],
        MSG_AUTH => &["signature"],
        MSG_WELCOME => &["protocol_version", "relay_level", "capabilities", "limits"],
        MSG_ERROR => &["code", "message", "fatal"],
        MSG_SYNC_REQUEST | MSG_SYNC_RESPONSE => &["datastore"],
        MSG_DELTA_REQUEST => &["datastore", "op_ids"],
        MSG_DELTA_BATCH => &["datastore", "operations", "remaining"],
        MSG_MERKLE_NODE_REQUEST => &["datastore", "level", "index"],
        MSG_MERKLE_NODE_RESPONSE => &["datastore", "level", "index", "hash", "left", "right"],
        MSG_MERKLE_LEAF_REQUEST => &["datastore", "leaf_index"],
        MSG_MERKLE_LEAF_RESPONSE => &["datastore", "leaf_index", "bucket_index", "op_ids"],
        MSG_OPS => &["datastore", "operations"],
        MSG_OP_ACK => &["outcomes"],
        _ => &[],
    }
}

/// Direction-dependent required root on `SYNC_REQUEST` / `SYNC_RESPONSE`.
/// Peer messages carry `accepted_root`; relay messages carry `validated_root`.
pub fn required_sync_root(dir: &str) -> Option<&'static str> {
    match dir {
        DIR_PEER_TO_RELAY => Some("accepted_root"),
        DIR_RELAY_TO_PEER => Some("validated_root"),
        _ => None,
    }
}

pub fn is_request(ty: u8, dir: &str, request_id: u32) -> bool {
    matches!(
        ty,
        MSG_HELLO
            | MSG_AUTH
            | MSG_SYNC_REQUEST
            | MSG_DELTA_REQUEST
            | MSG_MERKLE_NODE_REQUEST
            | MSG_MERKLE_LEAF_REQUEST
    ) || (ty == MSG_OPS && dir == DIR_PEER_TO_RELAY && request_id != 0)
}

pub fn is_response(ty: u8, request_id: u32) -> bool {
    matches!(
        ty,
        MSG_CHALLENGE
            | MSG_WELCOME
            | MSG_SYNC_RESPONSE
            | MSG_DELTA_BATCH
            | MSG_MERKLE_NODE_RESPONSE
            | MSG_MERKLE_LEAF_RESPONSE
            | MSG_OP_ACK
    ) || (ty == MSG_ERROR && request_id != 0)
}

pub fn expected_response_types(request_ty: u8) -> &'static [u8] {
    match request_ty {
        MSG_HELLO => &[MSG_CHALLENGE, MSG_ERROR],
        MSG_AUTH => &[MSG_WELCOME, MSG_ERROR],
        MSG_SYNC_REQUEST => &[MSG_SYNC_RESPONSE],
        MSG_DELTA_REQUEST => &[MSG_DELTA_BATCH],
        MSG_MERKLE_NODE_REQUEST => &[MSG_MERKLE_NODE_RESPONSE],
        MSG_MERKLE_LEAF_REQUEST => &[MSG_MERKLE_LEAF_RESPONSE],
        MSG_OPS => &[MSG_OP_ACK],
        _ => &[],
    }
}

#[derive(Debug, Clone)]
pub struct HeldOp {
    pub op_id: String,
    pub author: String,
    pub physical_ms: u64,
    pub logical: u16,
}

#[derive(Debug, Clone)]
pub struct FrontierTip {
    pub op_id: String,
    pub author: String,
    pub physical_ms: u64,
    pub logical: u16,
}

fn cmp_held(a: &HeldOp, tip: &FrontierTip) -> std::cmp::Ordering {
    a.physical_ms
        .cmp(&tip.physical_ms)
        .then(a.logical.cmp(&tip.logical))
        .then(a.author.cmp(&tip.author))
        .then(a.op_id.cmp(&tip.op_id))
}

fn covered(frontier: &[FrontierTip], op: &HeldOp) -> bool {
    frontier
        .iter()
        .find(|t| t.author == op.author)
        .is_some_and(|tip| cmp_held(op, tip) != std::cmp::Ordering::Greater)
}

/// Held ops the sender must retransmit given the receiver cursor.
/// Rejected OpIds are never retried (DELIVERY §3, RELAY §7.4).
pub fn retransmit(held: &[HeldOp], frontier: &[FrontierTip], rejected: &[String]) -> Vec<String> {
    let mut out: Vec<String> = held
        .iter()
        .filter(|op| !rejected.iter().any(|r| r == &op.op_id) && !covered(frontier, op))
        .map(|op| op.op_id.clone())
        .collect();
    out.sort();
    out
}

pub fn root_hex(ops: &[MerkleOp]) -> String {
    merkle_root(ops)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
