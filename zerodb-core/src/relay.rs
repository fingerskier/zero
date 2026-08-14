//! RELAY 0.2.2-draft wire-transcript model (M3a).
//!
//! Handshake (HELLO/AUTH/WELCOME capabilities), dual-root catch-up
//! invariant (CX-08), resume cursor, and reject-ack. No relay process.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::merkle::{merkle_root, MerkleOp};

/// Domain-separation prefix (registry `domain_separation.relay_auth`).
pub const DOMAIN_RELAY_AUTH: &[u8] = b"zerodb-relay-auth-v1";

/// Negotiable session capabilities (sorted).
pub const RELAY_CAPS: &[&str] = &["dual-root", "reject-ack", "resume-cursor"];

/// RELAY §10 `AUTH_FAILED`.
pub const ERR_AUTH_FAILED: u16 = 0x201;

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
