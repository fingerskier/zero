//! Group-key wrap for AUTH.md `KeyRecord` `kr = 2` (E6 / H10 lifecycle).
//!
//! The KERNEL §7 envelope stays unchanged: notes are sealed with a 32-byte
//! group symmetric key. This module only distributes that key. Each recipient
//! wrap is X25519 ECDH (Ed25519 key converted per RFC 7748) + XChaCha20-Poly1305.
//! Relays persist the wrapped bytes; they never need the group key.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use curve25519_dalek::edwards::CompressedEdwardsY;
use sha2::{Digest, Sha512};
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::envelope::key_id;

/// Domain-separation prefix for wrap-key derivation and wrap AAD.
pub const DOMAIN_GROUP_WRAP: &[u8] = b"zerodb-group-wrap-v1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GroupKeyError {
    #[error("recipient public key is not a valid Ed25519 point")]
    BadPublicKey,
    #[error("wrapped group key failed to open")]
    UnwrapFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupKeyWrap {
    pub recipient: [u8; 32],
    pub eph_pk: [u8; 32],
    pub nonce: [u8; 24],
    pub wrapped: Vec<u8>,
}

/// `KeyId = BLAKE3(group key)[..16]` (KERNEL §2).
pub fn group_key_id(key: &[u8; 32]) -> [u8; 16] {
    key_id(key)
}

fn ed25519_pk_to_x25519(pk: &[u8; 32]) -> Result<[u8; 32], GroupKeyError> {
    let point = CompressedEdwardsY(*pk)
        .decompress()
        .ok_or(GroupKeyError::BadPublicKey)?;
    Ok(point.to_montgomery().to_bytes())
}

/// Ed25519 seed → X25519 secret (SHA-512(seed)[0..32], then clamp).
fn ed25519_seed_to_x25519(seed: &[u8; 32]) -> StaticSecret {
    let hash = Sha512::digest(seed);
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&hash[..32]);
    StaticSecret::from(bytes)
}

fn wrap_key(ds: &[u8; 32], key_id: &[u8; 16], recipient: &[u8; 32], shared: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN_GROUP_WRAP);
    hasher.update(ds);
    hasher.update(key_id);
    hasher.update(recipient);
    hasher.update(shared);
    *hasher.finalize().as_bytes()
}

fn wrap_aad(ds: &[u8; 32], key_id: &[u8; 16], recipient: &[u8; 32]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(DOMAIN_GROUP_WRAP.len() + 32 + 16 + 32);
    aad.extend_from_slice(DOMAIN_GROUP_WRAP);
    aad.extend_from_slice(ds);
    aad.extend_from_slice(key_id);
    aad.extend_from_slice(recipient);
    aad
}

/// Wrap `group_key` for one recipient. `eph_sk` and `nonce` MUST be fresh random.
pub fn wrap_group_key(
    group_key: &[u8; 32],
    ds: &[u8; 32],
    recipient: &[u8; 32],
    recipient_ed25519_pk: &[u8; 32],
    eph_sk: &[u8; 32],
    nonce: &[u8; 24],
) -> Result<GroupKeyWrap, GroupKeyError> {
    let their_x = ed25519_pk_to_x25519(recipient_ed25519_pk)?;
    let eph = StaticSecret::from(*eph_sk);
    let eph_pk = PublicKey::from(&eph);
    let shared = eph.diffie_hellman(&PublicKey::from(their_x));
    let kid = group_key_id(group_key);
    let key = wrap_key(ds, &kid, recipient, shared.as_bytes());
    let cipher = XChaCha20Poly1305::new((&key).into());
    let wrapped = cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: group_key,
                aad: &wrap_aad(ds, &kid, recipient),
            },
        )
        .expect("XChaCha20-Poly1305 encryption is infallible for in-memory buffers");
    Ok(GroupKeyWrap {
        recipient: *recipient,
        eph_pk: eph_pk.to_bytes(),
        nonce: *nonce,
        wrapped,
    })
}

/// Open a wrap when the KeyRecord's claimed `key_id` is known (AUTH `kr = 2`).
pub fn unwrap_group_key(
    wrap: &GroupKeyWrap,
    ds: &[u8; 32],
    claimed_key_id: &[u8; 16],
    recipient: &[u8; 32],
    recipient_ed25519_seed: &[u8; 32],
) -> Result<[u8; 32], GroupKeyError> {
    if wrap.recipient != *recipient {
        return Err(GroupKeyError::UnwrapFailed);
    }
    let sk = ed25519_seed_to_x25519(recipient_ed25519_seed);
    let shared = sk.diffie_hellman(&PublicKey::from(wrap.eph_pk));
    let key = wrap_key(ds, claimed_key_id, recipient, shared.as_bytes());
    let cipher = XChaCha20Poly1305::new((&key).into());
    let pt = cipher
        .decrypt(
            XNonce::from_slice(&wrap.nonce),
            Payload {
                msg: &wrap.wrapped,
                aad: &wrap_aad(ds, claimed_key_id, recipient),
            },
        )
        .map_err(|_| GroupKeyError::UnwrapFailed)?;
    let group_key: [u8; 32] = pt.try_into().map_err(|_| GroupKeyError::UnwrapFailed)?;
    if group_key_id(&group_key) != *claimed_key_id {
        return Err(GroupKeyError::UnwrapFailed);
    }
    Ok(group_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    #[test]
    fn ed25519_x25519_conversion_agrees() {
        let seed = [0x11u8; 32];
        let pk = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
        let xsk = ed25519_seed_to_x25519(&seed);
        let xpk = PublicKey::from(&xsk);
        assert_eq!(xpk.to_bytes(), ed25519_pk_to_x25519(&pk).unwrap());
    }

    #[test]
    fn wrap_round_trip_and_non_recipient_fails() {
        let group = [0x42u8; 32];
        let ds = [0xAAu8; 32];
        let seed_b = [0x22u8; 32];
        let pk_b = SigningKey::from_bytes(&seed_b).verifying_key().to_bytes();
        let recipient = *blake3::hash(&pk_b).as_bytes();
        let eph = [0x07u8; 32];
        let nonce = [0x08u8; 24];
        let wrap = wrap_group_key(&group, &ds, &recipient, &pk_b, &eph, &nonce).unwrap();
        let kid = group_key_id(&group);
        assert_eq!(
            unwrap_group_key(&wrap, &ds, &kid, &recipient, &seed_b).unwrap(),
            group
        );

        let seed_c = [0x33u8; 32];
        assert_eq!(
            unwrap_group_key(&wrap, &ds, &kid, &recipient, &seed_c),
            Err(GroupKeyError::UnwrapFailed)
        );
        let mut wrong_ds = ds;
        wrong_ds[0] ^= 1;
        assert_eq!(
            unwrap_group_key(&wrap, &wrong_ds, &kid, &recipient, &seed_b),
            Err(GroupKeyError::UnwrapFailed)
        );
    }
}
