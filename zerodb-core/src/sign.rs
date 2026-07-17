//! Ed25519 operation signatures per doc/KERNEL.md §4.4.
//!
//! sig-preimage = domain("op_signature") ‖ OpId; the signature binds
//! transitively to every signed envelope field through the OpId.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

/// Domain-separation prefix (registry `domain_separation.op_signature`).
pub const DOMAIN_OP_SIG: &[u8] = b"zerodb-op-sig-v1";

pub fn sig_preimage(op_id: &[u8; 32]) -> Vec<u8> {
    [DOMAIN_OP_SIG, op_id.as_slice()].concat()
}

/// Sign an OpId with the device key derived from `seed`.
/// Returns (public key, signature). Ed25519 is deterministic (RFC 8032),
/// so golden signature vectors are stable across implementations.
pub fn sign_op(seed: &[u8; 32], op_id: &[u8; 32]) -> ([u8; 32], [u8; 64]) {
    let key = SigningKey::from_bytes(seed);
    let sig = key.sign(&sig_preimage(op_id));
    (key.verifying_key().to_bytes(), sig.to_bytes())
}

pub fn verify_op(pubkey: &[u8; 32], op_id: &[u8; 32], sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(pubkey) else {
        return false;
    };
    vk.verify(&sig_preimage(op_id), &Signature::from_bytes(sig))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_tamper() {
        let seed = [7u8; 32];
        let op_id = [9u8; 32];
        let (pubkey, sig) = sign_op(&seed, &op_id);
        assert!(verify_op(&pubkey, &op_id, &sig));

        let mut bad_sig = sig;
        bad_sig[0] ^= 1;
        assert!(!verify_op(&pubkey, &op_id, &bad_sig));

        let mut bad_id = op_id;
        bad_id[0] ^= 1;
        assert!(!verify_op(&pubkey, &bad_id, &sig));
    }

    #[test]
    fn deterministic_signature() {
        let seed = [7u8; 32];
        let op_id = [9u8; 32];
        assert_eq!(sign_op(&seed, &op_id), sign_op(&seed, &op_id));
    }
}
