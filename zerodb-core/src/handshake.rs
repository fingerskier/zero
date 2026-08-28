//! Deterministic handshake transcript (RELAY AUTH / future shared peer sync).
//!
//! H5: AUTH signs this transcript, not the nonce alone. H6 (direct P2P) is
//! parked to M4 and MUST reuse this helper rather than inventing a second
//! preimage. Draft-1 / unfrozen — not a format freeze.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::cbor::{self, Cbor};

/// RELAY §10 `AUTH_FAILED`.
const ERR_AUTH_FAILED: u16 = 0x201;

fn peer_id_from_pk(pk: &[u8; 32]) -> [u8; 32] {
    *blake3::hash(pk).as_bytes()
}

fn negotiate_welcome_caps(hello: &[impl AsRef<str>]) -> Vec<String> {
    const RELAY_CAPS: &[&str] = &["dual-root", "merkle-walk-v1", "reject-ack", "resume-cursor"];
    RELAY_CAPS
        .iter()
        .copied()
        .filter(|c| hello.iter().any(|h| h.as_ref() == *c))
        .map(|c| c.to_string())
        .collect()
}

/// Handshake AUTH domain (draft). v1 nonce-only signatures MUST fail closed.
pub const DOMAIN_RELAY_AUTH: &[u8] = b"zerodb-relay-auth-v2";
/// Legacy nonce-only domain. Verifiers MUST reject it for AUTH.
pub const DOMAIN_RELAY_AUTH_V1: &[u8] = b"zerodb-relay-auth-v1";

/// Advertised experimental WELCOME defaults (RELAY-SPEC §8.1).
pub const DEFAULT_PROTOCOL_VERSION: u8 = 1;
pub const DEFAULT_RELAY_LEVEL: u8 = 2;
pub const DEFAULT_MAX_PAYLOAD_BYTES: u32 = 1_048_576;
pub const DEFAULT_MAX_BATCH_OPS: u16 = 64;
pub const DEFAULT_MAX_BATCH_BYTES: u32 = 16_777_216;
pub const DEFAULT_MAX_SUBSCRIPTIONS: u16 = 64;
pub const DEFAULT_OPS_PER_SECOND: u32 = 100;
pub const DEFAULT_BYTES_PER_SECOND: u32 = 10_485_760;
pub const DEFAULT_MAX_CONNECTIONS_PER_PEER: u16 = 3;

/// WELCOME.limits fields bound into the AUTH transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeLimits {
    pub max_payload_bytes: u32,
    pub max_batch_ops: u16,
    pub max_batch_bytes: u32,
    pub max_subscriptions: u16,
    pub ops_per_second: u32,
    pub bytes_per_second: u32,
}

impl WelcomeLimits {
    pub fn advertised() -> Self {
        Self {
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            max_batch_ops: DEFAULT_MAX_BATCH_OPS,
            max_batch_bytes: DEFAULT_MAX_BATCH_BYTES,
            max_subscriptions: DEFAULT_MAX_SUBSCRIPTIONS,
            ops_per_second: DEFAULT_OPS_PER_SECOND,
            bytes_per_second: DEFAULT_BYTES_PER_SECOND,
        }
    }

    pub fn to_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            (
                "max_payload_bytes".into(),
                Cbor::Uint(self.max_payload_bytes as u64),
            ),
            (
                "max_batch_ops".into(),
                Cbor::Uint(self.max_batch_ops as u64),
            ),
            (
                "max_batch_bytes".into(),
                Cbor::Uint(self.max_batch_bytes as u64),
            ),
            (
                "max_subscriptions".into(),
                Cbor::Uint(self.max_subscriptions as u64),
            ),
            (
                "ops_per_second".into(),
                Cbor::Uint(self.ops_per_second as u64),
            ),
            (
                "bytes_per_second".into(),
                Cbor::Uint(self.bytes_per_second as u64),
            ),
        ])
    }
}

/// Deterministic handshake transcript (HELLO + nonce + intended WELCOME).
///
/// AUTH is sent before WELCOME, so both sides reconstruct the WELCOME the
/// relay is about to send (negotiated caps + advertised limits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthTranscript {
    pub peer_id: [u8; 32],
    pub public_key: [u8; 32],
    pub hello_protocol_version: u8,
    pub hello_capabilities: Vec<String>,
    pub nonce: [u8; 32],
    pub welcome_protocol_version: u8,
    pub relay_level: u8,
    pub welcome_capabilities: Vec<String>,
    pub limits: WelcomeLimits,
}

impl AuthTranscript {
    /// Experimental relay transcript from stored HELLO + challenge nonce.
    pub fn for_relay_hello(
        peer_id: [u8; 32],
        public_key: [u8; 32],
        hello_protocol_version: u8,
        hello_capabilities: &[impl AsRef<str>],
        nonce: [u8; 32],
    ) -> Self {
        let hello_capabilities: Vec<String> = hello_capabilities
            .iter()
            .map(|c| c.as_ref().to_string())
            .collect();
        let welcome_capabilities = negotiate_welcome_caps(&hello_capabilities);
        Self {
            peer_id,
            public_key,
            hello_protocol_version,
            hello_capabilities,
            nonce,
            welcome_protocol_version: DEFAULT_PROTOCOL_VERSION,
            relay_level: DEFAULT_RELAY_LEVEL,
            welcome_capabilities,
            limits: WelcomeLimits::advertised(),
        }
    }

    pub fn to_cbor(&self) -> Cbor {
        let hello_caps = self
            .hello_capabilities
            .iter()
            .map(|c| Cbor::Text(c.clone()))
            .collect();
        let welcome_caps = self
            .welcome_capabilities
            .iter()
            .map(|c| Cbor::Text(c.clone()))
            .collect();
        Cbor::Map(vec![
            (
                "hello".into(),
                Cbor::Map(vec![
                    ("capabilities".into(), Cbor::Array(hello_caps)),
                    ("peer_id".into(), Cbor::Bytes(self.peer_id.to_vec())),
                    (
                        "protocol_version".into(),
                        Cbor::Uint(self.hello_protocol_version as u64),
                    ),
                    ("public_key".into(), Cbor::Bytes(self.public_key.to_vec())),
                ]),
            ),
            ("nonce".into(), Cbor::Bytes(self.nonce.to_vec())),
            (
                "welcome".into(),
                Cbor::Map(vec![
                    ("capabilities".into(), Cbor::Array(welcome_caps)),
                    ("limits".into(), self.limits.to_cbor()),
                    (
                        "protocol_version".into(),
                        Cbor::Uint(self.welcome_protocol_version as u64),
                    ),
                    ("relay_level".into(), Cbor::Uint(self.relay_level as u64)),
                ]),
            ),
        ])
    }
}

/// Domain-separated transcript preimage (draft AUTH).
pub fn auth_transcript_preimage(t: &AuthTranscript) -> Vec<u8> {
    let body = cbor::encode(&t.to_cbor()).expect("transcript cbor");
    let mut out = Vec::with_capacity(DOMAIN_RELAY_AUTH.len() + body.len());
    out.extend_from_slice(DOMAIN_RELAY_AUTH);
    out.extend_from_slice(&body);
    out
}

/// Legacy v1 nonce-only preimage (must fail closed at AUTH).
pub fn auth_preimage_v1(nonce: &[u8]) -> Vec<u8> {
    [DOMAIN_RELAY_AUTH_V1, nonce].concat()
}

pub fn sign_auth(seed: &[u8; 32], transcript: &AuthTranscript) -> [u8; 64] {
    let key = SigningKey::from_bytes(seed);
    key.sign(&auth_transcript_preimage(transcript)).to_bytes()
}

/// Sign the experimental default transcript for a HELLO + nonce.
pub fn sign_auth_for_hello(
    seed: &[u8; 32],
    public_key: &[u8; 32],
    hello_capabilities: &[impl AsRef<str>],
    nonce: &[u8; 32],
) -> [u8; 64] {
    let t = AuthTranscript::for_relay_hello(
        peer_id_from_pk(public_key),
        *public_key,
        DEFAULT_PROTOCOL_VERSION,
        hello_capabilities,
        *nonce,
    );
    sign_auth(seed, &t)
}

pub fn sign_auth_v1_nonce_only(seed: &[u8; 32], nonce: &[u8; 32]) -> [u8; 64] {
    let key = SigningKey::from_bytes(seed);
    key.sign(&auth_preimage_v1(nonce)).to_bytes()
}

pub fn verify_auth(pk: &[u8; 32], transcript: &AuthTranscript, sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(pk) else {
        return false;
    };
    vk.verify(
        &auth_transcript_preimage(transcript),
        &Signature::from_bytes(sig),
    )
    .is_ok()
}

/// RELAY §4.1 / §5.2: transcript signature AND claimed PeerId == BLAKE3(pk).
/// A v1 nonce-only signature is AUTH_FAILED.
pub fn authenticate(
    claimed_peer_id: &[u8; 32],
    public_key: &[u8; 32],
    transcript: &AuthTranscript,
    signature: &[u8; 64],
) -> Result<(), u16> {
    if transcript.peer_id != *claimed_peer_id || transcript.public_key != *public_key {
        return Err(ERR_AUTH_FAILED);
    }
    if verify_auth(public_key, transcript, signature)
        && peer_id_from_pk(public_key) == *claimed_peer_id
    {
        Ok(())
    } else {
        Err(ERR_AUTH_FAILED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [
        0x56, 0x02, 0x95, 0x41, 0x1c, 0xb3, 0x77, 0x1a, 0x48, 0x92, 0xc5, 0x3f, 0xab, 0x03, 0x2a,
        0xba, 0xa0, 0xdc, 0x96, 0xb7, 0xa6, 0xed, 0x7b, 0xe6, 0xc6, 0x48, 0x65, 0x55, 0x1d, 0x06,
        0x2d, 0xfa,
    ];
    const PK: [u8; 32] = [
        0x26, 0xb7, 0x07, 0x2d, 0x6b, 0x2b, 0x0e, 0x99, 0x27, 0xbe, 0x59, 0xf4, 0x7b, 0x3b, 0x9a,
        0xb7, 0xd1, 0x7c, 0x79, 0x67, 0x25, 0xc2, 0x5f, 0x82, 0x69, 0x88, 0x2a, 0xf8, 0x6a, 0x13,
        0x06, 0xe1,
    ];

    #[test]
    fn honest_transcript_welcomes() {
        let nonce = [7u8; 32];
        let peer = peer_id_from_pk(&PK);
        let t = AuthTranscript::for_relay_hello(peer, PK, 1, &["dual-root"], nonce);
        let sig = sign_auth(&SEED, &t);
        assert!(authenticate(&peer, &PK, &t, &sig).is_ok());
    }

    #[test]
    fn v1_nonce_only_is_auth_failed() {
        let nonce = [7u8; 32];
        let peer = peer_id_from_pk(&PK);
        let t = AuthTranscript::for_relay_hello(peer, PK, 1, &["dual-root"] as &[&str], nonce);
        let sig = sign_auth_v1_nonce_only(&SEED, &nonce);
        assert_eq!(authenticate(&peer, &PK, &t, &sig), Err(ERR_AUTH_FAILED));
    }

    #[test]
    fn flipped_limits_or_version_fails() {
        let nonce = [7u8; 32];
        let peer = peer_id_from_pk(&PK);
        let honest = AuthTranscript::for_relay_hello(peer, PK, 1, &["dual-root"], nonce);
        let sig = sign_auth(&SEED, &honest);

        let mut flipped_limits = honest.clone();
        flipped_limits.limits.ops_per_second ^= 1;
        assert_eq!(
            authenticate(&peer, &PK, &flipped_limits, &sig),
            Err(ERR_AUTH_FAILED)
        );

        let mut flipped_ver = honest.clone();
        flipped_ver.welcome_protocol_version = 2;
        assert_eq!(
            authenticate(&peer, &PK, &flipped_ver, &sig),
            Err(ERR_AUTH_FAILED)
        );
    }

    #[test]
    fn relay_hello_001_transcript_matches() {
        let nonce = [7u8; 32];
        let peer = peer_id_from_pk(&PK);
        let hello = ["reject-ack", "dual-root", "resume-cursor", "unknown-cap"];
        let t = AuthTranscript::for_relay_hello(peer, PK, 1, &hello, nonce);
        let sig = sign_auth(&SEED, &t);
        assert!(authenticate(&peer, &PK, &t, &sig).is_ok());
    }
}
