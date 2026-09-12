//! Deterministic handshake transcript (RELAY AUTH / future shared peer sync).
//!
//! H5: AUTH signs this transcript, not the nonce alone. H6 (direct P2P)
//! MUST reuse this helper rather than inventing a second preimage
//! (`conformance/ts/webrtc/`). Handshake *roles* (who issues
//! CHALLENGE/WELCOME) use [`is_handshake_server`] — not a second AUTH
//! domain. Session datastore admission is [`admit_datastore`] (populated
//! A vs offered B is `AUTH_WRONG_DATASTORE` before OPS). Optional
//! `HELLO.datastore` is bound into [`AuthTranscript`] when present
//! (omitted when absent so no-ds goldens stay byte-identical). A
//! signaling MITM that swaps the claim fails AUTH. Reconnect repeats
//! this handshake; already-acked ops resume via `resume-cursor` /
//! DELIVERY §4, not a second AUTH preimage. H6 closed 2026-09-12.
//!
//! H5 slice — DTLS channel binding: on a WebRTC DataChannel the client
//! also binds `HELLO.channel_binding` = [`channel_binding`] of the two
//! DTLS certificate fingerprints (local + remote SDP `a=fingerprint`).
//! The handshake server verifies against its *own* view of both
//! fingerprints, so a signaling MITM that terminates DTLS on each leg
//! and forwards AUTH unchanged fails `0x201` before OPS. Omitted when
//! absent (relay WebSocket profile; no-binding goldens stay
//! byte-identical). Draft-1 / unfrozen — not a format freeze.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::cbor::{self, Cbor};

/// RELAY §10 `AUTH_FAILED`.
const ERR_AUTH_FAILED: u16 = 0x201;
/// RELAY §10 `VERSION_MISMATCH` (HELLO/WELCOME protocol_version).
pub const ERR_VERSION_MISMATCH: u16 = 0x102;

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
/// DTLS channel-binding domain (registry `domain_separation.dc_channel_binding`).
pub const DOMAIN_DC_CHANNEL_BINDING: &[u8] = b"zerodb-dc-binding-v1";

/// `HELLO.channel_binding` for a DataChannel: BLAKE3(domain ‖ min(fp) ‖ max(fp))
/// over the two SHA-256 DTLS certificate fingerprints. Order-independent, so
/// both ends of one DTLS association derive the same value; two different
/// associations (a MITM bridging two legs) derive different values.
pub fn channel_binding(fp_a: &[u8; 32], fp_b: &[u8; 32]) -> [u8; 32] {
    let (lo, hi) = if fp_a <= fp_b {
        (fp_a, fp_b)
    } else {
        (fp_b, fp_a)
    };
    let mut h = blake3::Hasher::new();
    h.update(DOMAIN_DC_CHANNEL_BINDING);
    h.update(lo);
    h.update(hi);
    *h.finalize().as_bytes()
}

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

/// Who issues CHALLENGE/WELCOME on a direct DataChannel.
///
/// Lexicographically smaller PeerId is the handshake server; the other
/// peer sends HELLO/AUTH. Same rule as the TS twin (`isHandshakeServer`).
/// Not a second AUTH preimage — AUTH is still [`AuthTranscript`].
pub fn is_handshake_server(local_peer: &[u8; 32], remote_peer: &[u8; 32]) -> bool {
    local_peer < remote_peer
}

/// Session-level datastore admission (H6).
///
/// A populated (or otherwise bound) store of A MUST fail closed when the
/// other side offers B — one named error before OPS mix graphs. An empty
/// store (`bound == None`) may adopt `offered`. Optional `HELLO.datastore`
/// is in [`AuthTranscript`] when present (omit when absent).
pub fn admit_datastore(bound: Option<&[u8]>, offered: Option<&[u8]>) -> Result<(), &'static str> {
    match (bound, offered) {
        (Some(a), Some(b)) if a != b => Err("AUTH_WRONG_DATASTORE"),
        _ => Ok(()),
    }
}

/// Client WELCOME.protocol_version gate (PR #21). Draft-1 window size 1:
/// only `1` is accepted. Missing/other is `0x102 VERSION_MISMATCH`.
pub fn check_welcome_protocol_version(version: Option<u64>) -> Result<(), u16> {
    match version {
        Some(n) if n == DEFAULT_PROTOCOL_VERSION as u64 => Ok(()),
        _ => Err(ERR_VERSION_MISMATCH),
    }
}

/// Deterministic handshake transcript (HELLO + nonce + intended WELCOME).
///
/// AUTH is sent before WELCOME, so both sides reconstruct the WELCOME the
/// relay is about to send (negotiated caps + advertised limits). Optional
/// `HELLO.datastore` and `HELLO.channel_binding` are in the hello map only
/// when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthTranscript {
    pub peer_id: [u8; 32],
    pub public_key: [u8; 32],
    pub hello_protocol_version: u8,
    pub hello_capabilities: Vec<String>,
    pub hello_datastore: Option<[u8; 32]>,
    /// DTLS channel binding ([`channel_binding`]); DataChannel profile only.
    pub hello_channel_binding: Option<[u8; 32]>,
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
            hello_datastore: None,
            hello_channel_binding: None,
            nonce,
            welcome_protocol_version: DEFAULT_PROTOCOL_VERSION,
            relay_level: DEFAULT_RELAY_LEVEL,
            welcome_capabilities,
            limits: WelcomeLimits::advertised(),
        }
    }

    /// Bind optional `HELLO.datastore` (32-byte id). `None` omits the field.
    pub fn with_hello_datastore(mut self, datastore: Option<[u8; 32]>) -> Self {
        self.hello_datastore = datastore;
        self
    }

    /// Bind optional `HELLO.channel_binding` (32 bytes). `None` omits the field.
    pub fn with_channel_binding(mut self, binding: Option<[u8; 32]>) -> Self {
        self.hello_channel_binding = binding;
        self
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
        let mut hello = vec![
            ("capabilities".into(), Cbor::Array(hello_caps)),
            ("peer_id".into(), Cbor::Bytes(self.peer_id.to_vec())),
            (
                "protocol_version".into(),
                Cbor::Uint(self.hello_protocol_version as u64),
            ),
            ("public_key".into(), Cbor::Bytes(self.public_key.to_vec())),
        ];
        if let Some(ds) = self.hello_datastore {
            hello.push(("datastore".into(), Cbor::Bytes(ds.to_vec())));
        }
        if let Some(cb) = self.hello_channel_binding {
            hello.push(("channel_binding".into(), Cbor::Bytes(cb.to_vec())));
        }
        Cbor::Map(vec![
            ("hello".into(), Cbor::Map(hello)),
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
    fn smaller_peer_id_is_handshake_server() {
        let a = [0u8; 32];
        let mut b = [0u8; 32];
        b[31] = 1;
        assert!(is_handshake_server(&a, &b));
        assert!(!is_handshake_server(&b, &a));
        assert!(!is_handshake_server(&a, &a));
    }

    #[test]
    fn populated_a_rejects_offered_b() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        assert_eq!(
            admit_datastore(Some(&a), Some(&b)),
            Err("AUTH_WRONG_DATASTORE")
        );
        assert!(admit_datastore(Some(&a), Some(&a)).is_ok());
        assert!(admit_datastore(None, Some(&b)).is_ok());
        assert!(admit_datastore(Some(&a), None).is_ok());
        assert!(admit_datastore(None, None).is_ok());
    }

    #[test]
    fn welcome_protocol_version_rejects_other_than_1() {
        assert!(check_welcome_protocol_version(Some(1)).is_ok());
        assert_eq!(
            check_welcome_protocol_version(Some(2)),
            Err(ERR_VERSION_MISMATCH)
        );
        assert_eq!(
            check_welcome_protocol_version(None),
            Err(ERR_VERSION_MISMATCH)
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

    #[test]
    fn omitted_hello_datastore_keeps_no_ds_preimage() {
        let nonce = [7u8; 32];
        let peer = peer_id_from_pk(&PK);
        let none = AuthTranscript::for_relay_hello(peer, PK, 1, &["dual-root"], nonce);
        let explicit_none = none.clone().with_hello_datastore(None);
        assert_eq!(
            auth_transcript_preimage(&none),
            auth_transcript_preimage(&explicit_none)
        );
        assert!(auth_transcript_preimage(&none).starts_with(DOMAIN_RELAY_AUTH));
        let with_ds = none.clone().with_hello_datastore(Some([0xaa; 32]));
        assert_ne!(
            auth_transcript_preimage(&none),
            auth_transcript_preimage(&with_ds)
        );
        assert!(authenticate(&peer, &PK, &none, &sign_auth(&SEED, &none)).is_ok());
        assert!(authenticate(&peer, &PK, &with_ds, &sign_auth(&SEED, &with_ds)).is_ok());
    }

    #[test]
    fn channel_binding_is_order_independent_and_distinct() {
        let a = [0x11u8; 32];
        let b = [0x22u8; 32];
        let c = [0x33u8; 32];
        assert_eq!(channel_binding(&a, &b), channel_binding(&b, &a));
        assert_ne!(channel_binding(&a, &b), channel_binding(&a, &c));
        assert_ne!(channel_binding(&a, &b), channel_binding(&a, &a));
        let mut h = blake3::Hasher::new();
        h.update(DOMAIN_DC_CHANNEL_BINDING);
        h.update(&a);
        h.update(&b);
        assert_eq!(channel_binding(&b, &a), *h.finalize().as_bytes());
    }

    #[test]
    fn omitted_channel_binding_keeps_preimage_swapped_is_auth_failed() {
        let nonce = [7u8; 32];
        let peer = peer_id_from_pk(&PK);
        let none = AuthTranscript::for_relay_hello(peer, PK, 1, &["dual-root"], nonce);
        assert_eq!(
            auth_transcript_preimage(&none),
            auth_transcript_preimage(&none.clone().with_channel_binding(None))
        );
        let honest_cb = channel_binding(&[0x11; 32], &[0x22; 32]);
        let honest = none.clone().with_channel_binding(Some(honest_cb));
        assert_ne!(
            auth_transcript_preimage(&none),
            auth_transcript_preimage(&honest)
        );
        let sig = sign_auth(&SEED, &honest);
        assert!(authenticate(&peer, &PK, &honest, &sig).is_ok());

        // MITM bridging two DTLS legs: the server's own view differs.
        let mitm_cb = channel_binding(&[0x11; 32], &[0x99; 32]);
        let swapped = none.clone().with_channel_binding(Some(mitm_cb));
        assert_eq!(
            authenticate(&peer, &PK, &swapped, &sig),
            Err(ERR_AUTH_FAILED)
        );
        // Stripping the field is not an escape either.
        assert_eq!(authenticate(&peer, &PK, &none, &sig), Err(ERR_AUTH_FAILED));
        // Datastore + binding compose in one hello map.
        let both = honest.clone().with_hello_datastore(Some([0xaa; 32]));
        let sig2 = sign_auth(&SEED, &both);
        assert!(authenticate(&peer, &PK, &both, &sig2).is_ok());
        assert_eq!(
            authenticate(&peer, &PK, &honest, &sig2),
            Err(ERR_AUTH_FAILED)
        );
    }

    #[test]
    fn swapped_hello_datastore_is_auth_failed() {
        let nonce = [7u8; 32];
        let peer = peer_id_from_pk(&PK);
        let honest = AuthTranscript::for_relay_hello(peer, PK, 1, &["dual-root"], nonce)
            .with_hello_datastore(Some([0xaa; 32]));
        let sig = sign_auth(&SEED, &honest);
        assert!(authenticate(&peer, &PK, &honest, &sig).is_ok());

        let swapped = honest.clone().with_hello_datastore(Some([0xbb; 32]));
        assert_eq!(
            authenticate(&peer, &PK, &swapped, &sig),
            Err(ERR_AUTH_FAILED)
        );

        let omitted = AuthTranscript::for_relay_hello(peer, PK, 1, &["dual-root"], nonce);
        assert_eq!(
            authenticate(&peer, &PK, &omitted, &sig),
            Err(ERR_AUTH_FAILED)
        );
    }
}
