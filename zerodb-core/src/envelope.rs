//! Encrypted-value envelope per doc/KERNEL.md §7 (DQ-5).
//!
//! Layout: `version (u8, = 1) ‖ key_id (16B = BLAKE3(key)[..16]) ‖
//! nonce (24B) ‖ ciphertext+tag (XChaCha20-Poly1305)`.
//! SlotId = BLAKE3(domain_slot ‖ ds ‖ ep ‖ path ‖ author ‖ ts);
//! AAD = domain_aad ‖ SlotId. Ciphertext and OpId are not inputs, so
//! seal completes before OpId exists (CX-03). Replay into any other
//! slot fails decryption (INVARIANTS I-10).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use thiserror::Error;

/// Domain-separation prefixes (registry `domain_separation`).
pub const DOMAIN_VALUE_AAD: &[u8] = b"zerodb-value-aad-v1";
pub const DOMAIN_VALUE_SLOT: &[u8] = b"zerodb-value-slot-v1";
pub const ENVELOPE_VERSION: u8 = 1;
const KEY_ID_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

/// Pre-encryption slot context (KERNEL §7). Never includes ciphertext or OpId.
#[derive(Debug, Clone)]
pub struct ValueContext {
    pub ds: [u8; 32],
    pub author: [u8; 32],
    pub physical_ms: u64,
    pub logical: u16,
    pub ep: u64,
    pub path: String,
}

impl ValueContext {
    /// `SlotId = BLAKE3(domain("value_slot") ‖ ds ‖ ep ‖ path ‖ author ‖ ts)`.
    pub fn slot_id(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(DOMAIN_VALUE_SLOT);
        hasher.update(&self.ds);
        hasher.update(&self.ep.to_be_bytes());
        hasher.update(self.path.as_bytes());
        hasher.update(&self.author);
        hasher.update(&self.physical_ms.to_be_bytes());
        hasher.update(&self.logical.to_be_bytes());
        *hasher.finalize().as_bytes()
    }

    pub fn aad(&self) -> Vec<u8> {
        let slot = self.slot_id();
        let mut aad = Vec::with_capacity(DOMAIN_VALUE_AAD.len() + 32);
        aad.extend_from_slice(DOMAIN_VALUE_AAD);
        aad.extend_from_slice(&slot);
        aad
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EnvelopeError {
    #[error("envelope shorter than its fixed header")]
    Truncated,
    #[error("unknown envelope version")]
    UnknownVersion,
    #[error("key id does not match the provided key")]
    UnknownKeyId,
    #[error("decryption failed (wrong key, tampered ciphertext, or AAD mismatch)")]
    DecryptFailed,
}

pub fn key_id(key: &[u8; 32]) -> [u8; KEY_ID_LEN] {
    blake3::hash(key).as_bytes()[..KEY_ID_LEN]
        .try_into()
        .unwrap()
}

/// Encrypt `plaintext` into a §7 envelope. `nonce` is caller-supplied so
/// golden vectors are deterministic; production callers MUST use a fresh
/// random nonce per value.
pub fn seal(
    key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    ctx: &ValueContext,
    plaintext: &[u8],
) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let ct = cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: &ctx.aad(),
            },
        )
        .expect("XChaCha20-Poly1305 encryption is infallible for in-memory buffers");

    let mut out = Vec::with_capacity(1 + KEY_ID_LEN + NONCE_LEN + ct.len());
    out.push(ENVELOPE_VERSION);
    out.extend_from_slice(&key_id(key));
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    out
}

/// Open a §7 envelope bound to `ctx`.
pub fn open(key: &[u8; 32], envelope: &[u8], ctx: &ValueContext) -> Result<Vec<u8>, EnvelopeError> {
    if envelope.len() < 1 + KEY_ID_LEN + NONCE_LEN + TAG_LEN {
        return Err(EnvelopeError::Truncated);
    }
    if envelope[0] != ENVELOPE_VERSION {
        return Err(EnvelopeError::UnknownVersion);
    }
    if envelope[1..1 + KEY_ID_LEN] != key_id(key) {
        return Err(EnvelopeError::UnknownKeyId);
    }
    let nonce = &envelope[1 + KEY_ID_LEN..1 + KEY_ID_LEN + NONCE_LEN];
    let ct = &envelope[1 + KEY_ID_LEN + NONCE_LEN..];

    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: &ctx.aad(),
            },
        )
        .map_err(|_| EnvelopeError::DecryptFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ValueContext {
        ValueContext {
            ds: [0x11; 32],
            author: [0x22; 32],
            physical_ms: 1_000_000,
            logical: 3,
            ep: 2,
            path: "note.body".into(),
        }
    }

    #[test]
    fn round_trip_and_aad_binding() {
        let key = [0x42; 32];
        let nonce = [0x05; 24];
        let envelope = seal(&key, &nonce, &ctx(), b"attack at dawn");
        assert_eq!(open(&key, &envelope, &ctx()).unwrap(), b"attack at dawn");

        // Each slot-context component must bind (I-10).
        let mut c = ctx();
        c.ds[0] ^= 1;
        assert_eq!(open(&key, &envelope, &c), Err(EnvelopeError::DecryptFailed));
        let mut c = ctx();
        c.author[0] ^= 1;
        assert_eq!(open(&key, &envelope, &c), Err(EnvelopeError::DecryptFailed));
        let mut c = ctx();
        c.physical_ms += 1;
        assert_eq!(open(&key, &envelope, &c), Err(EnvelopeError::DecryptFailed));
        let mut c = ctx();
        c.logical += 1;
        assert_eq!(open(&key, &envelope, &c), Err(EnvelopeError::DecryptFailed));
        let mut c = ctx();
        c.ep += 1;
        assert_eq!(open(&key, &envelope, &c), Err(EnvelopeError::DecryptFailed));
        let mut c = ctx();
        c.path.push('x');
        assert_eq!(open(&key, &envelope, &c), Err(EnvelopeError::DecryptFailed));
    }

    #[test]
    fn seal_does_not_require_opid_then_op_hashes_ciphertext() {
        use crate::cbor::Cbor;
        use crate::op::{OpEnvelope, OpTs};

        let key = [0x42; 32];
        let ctx = ctx();
        let envelope = seal(&key, &[0x05; 24], &ctx, b"attack at dawn");
        let op = OpEnvelope {
            v: 1,
            ds: ctx.ds,
            ep: ctx.ep,
            author: ctx.author,
            ts: OpTs {
                physical_ms: ctx.physical_ms,
                logical: ctx.logical,
            },
            deps: vec![],
            grp: None,
            kind: 3,
            body: Cbor::Map(vec![
                ("crdt".into(), Cbor::Text("lww".into())),
                ("encrypted".into(), Cbor::Bytes(envelope.clone())),
                ("path".into(), Cbor::Text(ctx.path.clone())),
            ]),
        };
        let op_id = op.op_id().unwrap();
        // Slot context never included op_id; flipping a hypothetical op_id
        // cannot be part of AAD. Decrypt still works after OpId is derived.
        assert_eq!(open(&key, &envelope, &ctx).unwrap(), b"attack at dawn");
        assert_ne!(op_id, [0u8; 32]);
    }

    #[test]
    fn header_checks() {
        let key = [0x42; 32];
        let envelope = seal(&key, &[0x05; 24], &ctx(), b"x");

        let mut bad = envelope.clone();
        bad[0] = 2;
        assert_eq!(open(&key, &bad, &ctx()), Err(EnvelopeError::UnknownVersion));

        let mut bad = envelope.clone();
        bad[1] ^= 1;
        assert_eq!(open(&key, &bad, &ctx()), Err(EnvelopeError::UnknownKeyId));

        assert_eq!(
            open(&key, &envelope[..40], &ctx()),
            Err(EnvelopeError::Truncated)
        );

        let mut bad = envelope.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert_eq!(open(&key, &bad, &ctx()), Err(EnvelopeError::DecryptFailed));
    }
}
