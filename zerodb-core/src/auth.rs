//! Identity, genesis, and authorization models per doc/AUTH.md (M0d).
//!
//! - §1 device certificates: `PrincipalId`/`PeerId`, cert-preimage, root Ed25519
//! - §2 genesis: `DatastoreId = BLAKE3(domain("genesis") ‖ id-preimage)`
//! - §4 authz predicate: causal grant-time membership (executable reference model)

use std::collections::{HashMap, HashSet, VecDeque};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use thiserror::Error;

use crate::cbor::{Cbor, CborError, encode};
use crate::op::{DOMAIN_OP_ID, OpEnvelope, OpTs};

/// Domain-separation prefix (registry `domain_separation.device_cert`).
pub const DOMAIN_DEVICE_CERT: &[u8] = b"zerodb-device-cert-v1";
/// Domain-separation prefix (registry `domain_separation.genesis`).
pub const DOMAIN_GENESIS: &[u8] = b"zerodb-genesis-v1";
/// Domain-separation prefix (registry `domain_separation.relay_auth`).
pub const DOMAIN_RELAY_AUTH: &[u8] = b"zerodb-relay-auth-v1";

/// KeyRecord subtype tags (AUTH.md §1.2).
pub const KR_DEVICE_CERT: u64 = 0;
pub const KR_DEVICE_REVOKE: u64 = 1;

/// Scope tags (AUTH.md §3.1).
pub const SCOPE_WRITE: u64 = 0;
pub const SCOPE_ADMIN: u64 = 1;
pub const SCOPE_READ: u64 = 2;
pub const SCOPE_SYNC: u64 = 3;

/// Operation kind tags (KERNEL §4.2).
pub const KIND_GENESIS: u64 = 0;
pub const KIND_CREATE_NODE: u64 = 1;
pub const KIND_CREATE_EDGE: u64 = 2;
pub const KIND_SET_PROPERTY: u64 = 3;
pub const KIND_TOMBSTONE: u64 = 4;
pub const KIND_SCHEMA_EPOCH: u64 = 5;
pub const KIND_CAP_GRANT: u64 = 6;
pub const KIND_CAP_REVOKE: u64 = 7;
pub const KIND_KEY_RECORD: u64 = 8;
pub const KIND_CHECKPOINT: u64 = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCert {
    pub kr: u64,
    pub device_pk: [u8; 32],
    pub principal_id: [u8; 32],
    pub root_pk: [u8; 32],
    pub issued: u64,
    pub expiry: Option<u64>,
    pub revoke_of: Option<[u8; 32]>,
    pub cert_sig: [u8; 64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenesisBody {
    pub founder: [u8; 32],
    pub salt: [u8; 16],
    pub init_ep: u64,
    pub fmt_v: u64,
}

/// Logical control/data op for the authz reference model (AUTH.md §4).
/// Not a full signed envelope — fixtures supply resolved principal + ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthzOp {
    pub id: [u8; 32],
    pub kind: u64,
    pub author: [u8; 32],
    pub principal: [u8; 32],
    pub ds: [u8; 32],
    pub deps: Vec<[u8; 32]>,
    pub ts_physical_ms: u64,
    /// Kind-specific fields (grant/revoke/genesis).
    pub body: AuthzBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthzBody {
    Genesis {
        founder: [u8; 32],
    },
    Grant {
        subject: [u8; 32],
        scopes: Vec<u64>,
        expiry: Option<u64>,
        delegable: bool,
        ds_bind: [u8; 32],
    },
    Revoke {
        grant: [u8; 32],
        reason: u64,
    },
    /// Data / other control ops — body not needed for authz.
    Other,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("AUTH_SIG_INVALID")]
    SigInvalid,
    #[error("CAP_INVALID")]
    CapInvalid,
    #[error("AUTH_WRONG_DATASTORE")]
    WrongDatastore,
    #[error("AUTH_NO_MEMBERSHIP")]
    NoMembership,
    #[error("AUTH_REVOKED")]
    Revoked,
    #[error("AUTH_EXPIRED")]
    Expired,
    #[error("AUTH_NOT_ADMIN")]
    NotAdmin,
    #[error("CAP_SCOPE_UNKNOWN")]
    ScopeUnknown,
    #[error("cbor: {0}")]
    Cbor(#[from] CborError),
}

/// Outcome string for conformance vectors (AUTH.md §4.4).
pub fn auth_error_tag(e: &AuthError) -> &'static str {
    match e {
        AuthError::SigInvalid => "AUTH_SIG_INVALID",
        AuthError::CapInvalid => "CAP_INVALID",
        AuthError::WrongDatastore => "AUTH_WRONG_DATASTORE",
        AuthError::NoMembership => "AUTH_NO_MEMBERSHIP",
        AuthError::Revoked => "AUTH_REVOKED",
        AuthError::Expired => "AUTH_EXPIRED",
        AuthError::NotAdmin => "AUTH_NOT_ADMIN",
        AuthError::ScopeUnknown => "CAP_SCOPE_UNKNOWN",
        AuthError::Cbor(_) => "CAP_INVALID",
    }
}

/// `PrincipalId = BLAKE3(root public key)`.
pub fn principal_id(root_pk: &[u8; 32]) -> [u8; 32] {
    *blake3::hash(root_pk).as_bytes()
}

/// `PeerId = BLAKE3(device public key)`.
pub fn peer_id(device_pk: &[u8; 32]) -> [u8; 32] {
    *blake3::hash(device_pk).as_bytes()
}

fn pubkey_from_seed(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

// ---------------------------------------------------------------- device cert

/// Canonical CBOR of the cert map without `cert_sig` (AUTH.md §1.2).
pub fn cert_preimage_body(cert: &DeviceCert) -> Result<Vec<u8>, CborError> {
    let entries = vec![
        ("device".into(), Cbor::Bytes(cert.device_pk.to_vec())),
        (
            "expiry".into(),
            match cert.expiry {
                Some(e) => Cbor::Uint(e),
                None => Cbor::Null,
            },
        ),
        ("issued".into(), Cbor::Uint(cert.issued)),
        ("kr".into(), Cbor::Uint(cert.kr)),
        ("principal".into(), Cbor::Bytes(cert.principal_id.to_vec())),
        ("root_pk".into(), Cbor::Bytes(cert.root_pk.to_vec())),
        (
            "revoke_of".into(),
            match &cert.revoke_of {
                Some(r) => Cbor::Bytes(r.to_vec()),
                None => Cbor::Null,
            },
        ),
    ];
    encode(&Cbor::Map(entries))
}

pub fn cert_preimage(cert: &DeviceCert) -> Result<Vec<u8>, CborError> {
    let body = cert_preimage_body(cert)?;
    let mut out = Vec::with_capacity(DOMAIN_DEVICE_CERT.len() + body.len());
    out.extend_from_slice(DOMAIN_DEVICE_CERT);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Issue a device cert (`kr = 0`) signed by the root seed.
pub fn issue_device_cert(
    root_seed: &[u8; 32],
    device_pk: [u8; 32],
    issued: u64,
    expiry: Option<u64>,
) -> Result<DeviceCert, AuthError> {
    let root = SigningKey::from_bytes(root_seed);
    let root_pk = root.verifying_key().to_bytes();
    let principal = principal_id(&root_pk);
    let mut cert = DeviceCert {
        kr: KR_DEVICE_CERT,
        device_pk,
        principal_id: principal,
        root_pk,
        issued,
        expiry,
        revoke_of: None,
        cert_sig: [0u8; 64],
    };
    let pre = cert_preimage(&cert)?;
    cert.cert_sig = root.sign(&pre).to_bytes();
    Ok(cert)
}

/// Issue a device revoke (`kr = 1`) for `revoke_peer` signed by the root seed.
pub fn issue_device_revoke(
    root_seed: &[u8; 32],
    device_pk: [u8; 32],
    revoke_peer: [u8; 32],
    issued: u64,
) -> Result<DeviceCert, AuthError> {
    let root = SigningKey::from_bytes(root_seed);
    let root_pk = root.verifying_key().to_bytes();
    let principal = principal_id(&root_pk);
    let mut cert = DeviceCert {
        kr: KR_DEVICE_REVOKE,
        device_pk,
        principal_id: principal,
        root_pk,
        issued,
        expiry: None,
        revoke_of: Some(revoke_peer),
        cert_sig: [0u8; 64],
    };
    let pre = cert_preimage(&cert)?;
    cert.cert_sig = root.sign(&pre).to_bytes();
    Ok(cert)
}

/// Verify a device certificate (AUTH.md §1.2).
pub fn verify_device_cert(cert: &DeviceCert) -> Result<(), AuthError> {
    if cert.kr != KR_DEVICE_CERT && cert.kr != KR_DEVICE_REVOKE {
        return Err(AuthError::CapInvalid);
    }
    if principal_id(&cert.root_pk) != cert.principal_id {
        return Err(AuthError::CapInvalid);
    }
    match cert.kr {
        KR_DEVICE_CERT if cert.revoke_of.is_some() => return Err(AuthError::CapInvalid),
        KR_DEVICE_REVOKE if cert.revoke_of.is_none() => return Err(AuthError::CapInvalid),
        _ => {}
    }
    let Ok(vk) = VerifyingKey::from_bytes(&cert.root_pk) else {
        return Err(AuthError::SigInvalid);
    };
    let pre = cert_preimage(cert)?;
    let sig = Signature::from_bytes(&cert.cert_sig);
    vk.verify(&pre, &sig).map_err(|_| AuthError::SigInvalid)?;
    Ok(())
}

/// Convenience: device public key from a 32-byte seed (test/fixture helper).
pub fn device_pk_from_seed(seed: &[u8; 32]) -> [u8; 32] {
    pubkey_from_seed(seed)
}

// ---------------------------------------------------------------- genesis

pub fn genesis_body_cbor(g: &GenesisBody) -> Cbor {
    Cbor::Map(vec![
        ("fmt_v".into(), Cbor::Uint(g.fmt_v)),
        ("founder".into(), Cbor::Bytes(g.founder.to_vec())),
        ("init_ep".into(), Cbor::Uint(g.init_ep)),
        ("salt".into(), Cbor::Bytes(g.salt.to_vec())),
    ])
}

/// Build a genesis envelope (ds all-zero, kind 0).
pub fn genesis_envelope(author: [u8; 32], ts: OpTs, body: &GenesisBody) -> OpEnvelope {
    OpEnvelope {
        v: body.fmt_v,
        ds: [0u8; 32],
        ep: body.init_ep,
        author,
        ts,
        deps: vec![],
        grp: None,
        kind: KIND_GENESIS,
        body: genesis_body_cbor(body),
    }
}

/// `DatastoreId = BLAKE3(domain("genesis") ‖ id-preimage body)` (AUTH.md §2 / KERNEL §4.6).
///
/// Requires `kind == 0` and `ds` all-zero; otherwise `AUTH_WRONG_DATASTORE`.
pub fn datastore_id_from_genesis(env: &OpEnvelope) -> Result<[u8; 32], AuthError> {
    if env.kind != KIND_GENESIS {
        return Err(AuthError::CapInvalid);
    }
    if env.ds != [0u8; 32] {
        return Err(AuthError::WrongDatastore);
    }
    let body = env.id_preimage_body()?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN_GENESIS);
    hasher.update(&body);
    Ok(*hasher.finalize().as_bytes())
}

/// OpId of any envelope (re-export convenience via DOMAIN_OP_ID).
pub fn op_id_of(env: &OpEnvelope) -> Result<[u8; 32], AuthError> {
    let body = env.id_preimage_body()?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN_OP_ID);
    hasher.update(&body);
    Ok(*hasher.finalize().as_bytes())
}

// ---------------------------------------------------------------- admission token (AUTH.md §3.3)

/// Relay-side membership capability token (SUBSCRIBE admission filter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionToken {
    pub ds: [u8; 32],
    pub subject: [u8; 32],
    pub grant: [u8; 32],
    pub scopes: Vec<u64>,
    pub device: [u8; 32],
    /// Inline device_cert body (kr=0) used for resolution in the model.
    pub cert: DeviceCert,
    pub sig: [u8; 64],
}

/// Known grants available to the relay cache for admission checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownGrant {
    pub id: [u8; 32],
    pub ds: [u8; 32],
    pub subject: [u8; 32],
    pub scopes: Vec<u64>,
    pub expiry: Option<u64>,
    pub revoked: bool,
}

pub fn admission_token_preimage_body(tok: &AdmissionToken) -> Result<Vec<u8>, CborError> {
    // cert is carried as nested map without cert_sig for the token preimage? AUTH says:
    // "cert: <device_cert KeyRecord body or OpId of applied cert>"
    // For the model we bind the cert body *including* cert_sig (full verified cert)
    // as opaque bytes via nested map fields — actually simpler: cert OpId-style is
    // not used; we encode a map of cert fields WITH cert_sig so the token binds the
    // exact cert presented.
    let cert_map = Cbor::Map(vec![
        ("cert_sig".into(), Cbor::Bytes(tok.cert.cert_sig.to_vec())),
        ("device".into(), Cbor::Bytes(tok.cert.device_pk.to_vec())),
        (
            "expiry".into(),
            match tok.cert.expiry {
                Some(e) => Cbor::Uint(e),
                None => Cbor::Null,
            },
        ),
        ("issued".into(), Cbor::Uint(tok.cert.issued)),
        ("kr".into(), Cbor::Uint(tok.cert.kr)),
        (
            "principal".into(),
            Cbor::Bytes(tok.cert.principal_id.to_vec()),
        ),
        ("root_pk".into(), Cbor::Bytes(tok.cert.root_pk.to_vec())),
        (
            "revoke_of".into(),
            match &tok.cert.revoke_of {
                Some(r) => Cbor::Bytes(r.to_vec()),
                None => Cbor::Null,
            },
        ),
    ]);
    let entries = vec![
        ("cert".into(), cert_map),
        ("device".into(), Cbor::Bytes(tok.device.to_vec())),
        ("ds".into(), Cbor::Bytes(tok.ds.to_vec())),
        ("grant".into(), Cbor::Bytes(tok.grant.to_vec())),
        (
            "scopes".into(),
            Cbor::Array(tok.scopes.iter().map(|s| Cbor::Uint(*s)).collect()),
        ),
        ("subject".into(), Cbor::Bytes(tok.subject.to_vec())),
    ];
    encode(&Cbor::Map(entries))
}

pub fn admission_token_preimage(tok: &AdmissionToken) -> Result<Vec<u8>, CborError> {
    let body = admission_token_preimage_body(tok)?;
    let mut out = Vec::with_capacity(DOMAIN_RELAY_AUTH.len() + body.len());
    out.extend_from_slice(DOMAIN_RELAY_AUTH);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Sign an admission token with the device seed (fixture helper).
pub fn sign_admission_token(
    device_seed: &[u8; 32],
    mut tok: AdmissionToken,
) -> Result<AdmissionToken, AuthError> {
    let key = SigningKey::from_bytes(device_seed);
    let pre = admission_token_preimage(&tok)?;
    tok.sig = key.sign(&pre).to_bytes();
    Ok(tok)
}

/// Relay-side admission verification (AUTH.md §3.3) — filter only, not peer integrity.
pub fn verify_admission_token(
    tok: &AdmissionToken,
    known_grants: &[KnownGrant],
) -> Result<(), AuthError> {
    // 1. device cert
    verify_device_cert(&tok.cert)?;
    if tok.cert.kr != KR_DEVICE_CERT {
        return Err(AuthError::CapInvalid);
    }
    if tok.device != tok.cert.device_pk {
        return Err(AuthError::CapInvalid);
    }
    if tok.subject != tok.cert.principal_id {
        return Err(AuthError::NoMembership);
    }
    // 2. grant lookup
    let grant = known_grants
        .iter()
        .find(|g| g.id == tok.grant)
        .ok_or(AuthError::NoMembership)?;
    if grant.ds != tok.ds || grant.subject != tok.subject {
        return Err(AuthError::NoMembership);
    }
    if grant.revoked {
        return Err(AuthError::Revoked);
    }
    for s in &tok.scopes {
        if !is_known_scope(*s) {
            return Err(AuthError::ScopeUnknown);
        }
        if !grant.scopes.contains(s) {
            return Err(AuthError::NoMembership);
        }
    }
    // 3. device signature over token preimage
    let Ok(vk) = VerifyingKey::from_bytes(&tok.device) else {
        return Err(AuthError::SigInvalid);
    };
    let pre = admission_token_preimage(tok)?;
    let sig = Signature::from_bytes(&tok.sig);
    vk.verify(&pre, &sig).map_err(|_| AuthError::SigInvalid)?;
    Ok(())
}

/// Admission verification with relay wall-clock expiry and binding to the
/// authenticated transport peer. This remains a relay filter; peers enforce
/// the causal per-operation predicate independently.
pub fn verify_admission_token_at(
    tok: &AdmissionToken,
    known_grants: &[KnownGrant],
    connected_peer: &[u8; 32],
    now_ms: u64,
) -> Result<(), AuthError> {
    verify_admission_token(tok, known_grants)?;
    if peer_id(&tok.device) != *connected_peer {
        return Err(AuthError::SigInvalid);
    }
    if tok.cert.expiry.is_some_and(|expiry| now_ms >= expiry) {
        return Err(AuthError::Expired);
    }
    let grant = known_grants
        .iter()
        .find(|grant| grant.id == tok.grant)
        .ok_or(AuthError::NoMembership)?;
    if grant.expiry.is_some_and(|expiry| now_ms >= expiry) {
        return Err(AuthError::Expired);
    }
    Ok(())
}

// ---------------------------------------------------------------- authz predicate

fn is_known_scope(s: u64) -> bool {
    matches!(s, SCOPE_WRITE | SCOPE_ADMIN | SCOPE_READ | SCOPE_SYNC)
}

fn required_scope(kind: u64) -> Result<u64, AuthError> {
    match kind {
        KIND_CREATE_NODE | KIND_CREATE_EDGE | KIND_SET_PROPERTY | KIND_TOMBSTONE => Ok(SCOPE_WRITE),
        KIND_SCHEMA_EPOCH | KIND_CAP_GRANT | KIND_CAP_REVOKE | KIND_CHECKPOINT => Ok(SCOPE_ADMIN),
        // KeyRecord device cert publication: model as write-equivalent self-attestation;
        // full root-sig path is covered by device-cert vectors. Treat as write for authz.
        KIND_KEY_RECORD => Ok(SCOPE_WRITE),
        KIND_GENESIS => Ok(SCOPE_ADMIN), // not used for genesis path
        _ => Err(AuthError::CapInvalid),
    }
}

/// Transitive causal past of `op` within `by_id` (deps closure, not including self).
pub fn causal_past(op: &AuthzOp, by_id: &HashMap<[u8; 32], &AuthzOp>) -> HashSet<[u8; 32]> {
    let mut seen = HashSet::new();
    let mut q = VecDeque::new();
    for d in &op.deps {
        q.push_back(*d);
    }
    while let Some(id) = q.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(dep) = by_id.get(&id) {
            for d in &dep.deps {
                q.push_back(*d);
            }
        }
    }
    seen
}

/// Per-operation authorization predicate (AUTH.md §4).
///
/// `datastore_id` is the store's self-certifying id (from genesis). `applied` is the set of
/// already-accepted ops (must include genesis for non-genesis candidates). Signature
/// verification is assumed done (model step 1 stubbed via supplied `principal`).
pub fn authorize(
    datastore_id: &[u8; 32],
    applied: &[AuthzOp],
    candidate: &AuthzOp,
) -> Result<(), AuthError> {
    let by_id: HashMap<[u8; 32], &AuthzOp> = applied.iter().map(|o| (o.id, o)).collect();

    // --- genesis ---
    if candidate.kind == KIND_GENESIS {
        if candidate.ds != [0u8; 32] {
            return Err(AuthError::WrongDatastore);
        }
        let AuthzBody::Genesis { founder } = &candidate.body else {
            return Err(AuthError::CapInvalid);
        };
        if candidate.principal != *founder {
            return Err(AuthError::NoMembership);
        }
        return Ok(());
    }

    // --- datastore bind ---
    if candidate.ds != *datastore_id {
        return Err(AuthError::WrongDatastore);
    }
    for o in applied {
        if o.kind != KIND_GENESIS && o.ds != *datastore_id {
            return Err(AuthError::WrongDatastore);
        }
    }

    let genesis = applied
        .iter()
        .find(|o| o.kind == KIND_GENESIS)
        .ok_or(AuthError::CapInvalid)?;
    let AuthzBody::Genesis { founder } = &genesis.body else {
        return Err(AuthError::CapInvalid);
    };

    let past = causal_past(candidate, &by_id);
    if !past.contains(&genesis.id) {
        return Err(AuthError::NoMembership);
    }

    let need = required_scope(candidate.kind)?;

    #[derive(Clone)]
    struct GrantView {
        id: [u8; 32],
        scopes: Vec<u64>,
        expiry: Option<u64>,
    }

    let mut grants: Vec<GrantView> = Vec::new();

    if candidate.principal == *founder {
        grants.push(GrantView {
            id: genesis.id,
            scopes: vec![SCOPE_WRITE, SCOPE_ADMIN, SCOPE_READ, SCOPE_SYNC],
            expiry: None,
        });
    }

    for id in &past {
        let Some(op) = by_id.get(id) else { continue };
        if op.kind != KIND_CAP_GRANT {
            continue;
        }
        let AuthzBody::Grant {
            subject,
            scopes,
            expiry,
            ds_bind,
            ..
        } = &op.body
        else {
            continue;
        };
        if *subject != candidate.principal {
            continue;
        }
        if *ds_bind != *datastore_id {
            continue;
        }
        for s in scopes {
            if !is_known_scope(*s) {
                return Err(AuthError::ScopeUnknown);
            }
        }
        grants.push(GrantView {
            id: op.id,
            scopes: scopes.clone(),
            expiry: *expiry,
        });
    }

    let mut revoked: HashSet<[u8; 32]> = HashSet::new();
    for id in &past {
        let Some(op) = by_id.get(id) else { continue };
        if op.kind != KIND_CAP_REVOKE {
            continue;
        }
        let AuthzBody::Revoke { grant, .. } = &op.body else {
            continue;
        };
        revoked.insert(*grant);
    }

    let mut saw_expired = false;
    let mut saw_scope_but_not_admin = false;

    for g in &grants {
        if revoked.contains(&g.id) {
            continue;
        }
        if g.expiry.is_some_and(|exp| candidate.ts_physical_ms >= exp) {
            saw_expired = true;
            continue;
        }
        if !g.scopes.contains(&need) {
            if need == SCOPE_ADMIN && g.scopes.contains(&SCOPE_WRITE) {
                saw_scope_but_not_admin = true;
            }
            continue;
        }
        return Ok(());
    }

    if grants
        .iter()
        .any(|g| g.scopes.contains(&need) && revoked.contains(&g.id))
        && !grants
            .iter()
            .any(|g| g.scopes.contains(&need) && !revoked.contains(&g.id))
    {
        return Err(AuthError::Revoked);
    }
    if saw_expired {
        return Err(AuthError::Expired);
    }
    if need == SCOPE_ADMIN || saw_scope_but_not_admin {
        return Err(AuthError::NotAdmin);
    }
    Err(AuthError::NoMembership)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn issue_and_verify_device_cert() {
        let root_seed = [1u8; 32];
        let device_seed = [2u8; 32];
        let device_pk = device_pk_from_seed(&device_seed);
        let cert = issue_device_cert(&root_seed, device_pk, 1_000_000, None).unwrap();
        assert_eq!(cert.kr, KR_DEVICE_CERT);
        assert_eq!(peer_id(&device_pk), peer_id(&cert.device_pk));
        verify_device_cert(&cert).unwrap();
    }

    #[test]
    fn tampered_sig_fails() {
        let root_seed = [1u8; 32];
        let device_pk = device_pk_from_seed(&[2u8; 32]);
        let mut cert = issue_device_cert(&root_seed, device_pk, 1, None).unwrap();
        cert.cert_sig[0] ^= 1;
        assert_eq!(verify_device_cert(&cert), Err(AuthError::SigInvalid));
    }

    #[test]
    fn principal_mismatch_fails() {
        let root_seed = [1u8; 32];
        let device_pk = device_pk_from_seed(&[2u8; 32]);
        let mut cert = issue_device_cert(&root_seed, device_pk, 1, None).unwrap();
        cert.principal_id[0] ^= 1;
        assert_eq!(verify_device_cert(&cert), Err(AuthError::CapInvalid));
    }

    #[test]
    fn device_cert_must_not_carry_revoke_of() {
        let root_seed = [1u8; 32];
        let device_pk = device_pk_from_seed(&[2u8; 32]);
        let mut cert = issue_device_cert(&root_seed, device_pk, 1, None).unwrap();
        cert.revoke_of = Some([9u8; 32]);
        assert_eq!(verify_device_cert(&cert), Err(AuthError::CapInvalid));
    }

    #[test]
    fn genesis_id_stable_and_salt_sensitive() {
        let author = [0xAAu8; 32];
        let founder = [0xBBu8; 32];
        let body = GenesisBody {
            founder,
            salt: [0x11; 16],
            init_ep: 0,
            fmt_v: 1,
        };
        let env = genesis_envelope(
            author,
            OpTs {
                physical_ms: 1_000,
                logical: 0,
            },
            &body,
        );
        let id1 = datastore_id_from_genesis(&env).unwrap();
        let id2 = datastore_id_from_genesis(&env).unwrap();
        assert_eq!(id1, id2);

        let mut body2 = body;
        body2.salt[0] ^= 1;
        let env2 = genesis_envelope(
            author,
            OpTs {
                physical_ms: 1_000,
                logical: 0,
            },
            &body2,
        );
        assert_ne!(id1, datastore_id_from_genesis(&env2).unwrap());
    }

    #[test]
    fn genesis_nonzero_ds_rejected() {
        let mut env = genesis_envelope(
            [1u8; 32],
            OpTs {
                physical_ms: 1,
                logical: 0,
            },
            &GenesisBody {
                founder: [2u8; 32],
                salt: [3u8; 16],
                init_ep: 0,
                fmt_v: 1,
            },
        );
        env.ds[0] = 1;
        assert_eq!(
            datastore_id_from_genesis(&env),
            Err(AuthError::WrongDatastore)
        );
    }

    #[test]
    fn authz_founder_write_ok() {
        let g_id = [1u8; 32];
        let founder = [0xF0; 32];
        let ds = [0xD0; 32];
        let genesis = AuthzOp {
            id: g_id,
            kind: KIND_GENESIS,
            author: [0xA0; 32],
            principal: founder,
            ds: [0u8; 32],
            deps: vec![],
            ts_physical_ms: 1000,
            body: AuthzBody::Genesis { founder },
        };
        let write = AuthzOp {
            id: [2u8; 32],
            kind: KIND_CREATE_NODE,
            author: [0xA0; 32],
            principal: founder,
            ds,
            deps: vec![g_id],
            ts_physical_ms: 2000,
            body: AuthzBody::Other,
        };
        assert_eq!(authorize(&ds, &[genesis], &write), Ok(()));
    }

    #[test]
    fn authz_stranger_no_membership() {
        let g_id = [1u8; 32];
        let founder = [0xF0; 32];
        let stranger = [0xEE; 32];
        let ds = [0xD0; 32];
        let genesis = AuthzOp {
            id: g_id,
            kind: KIND_GENESIS,
            author: [0xA0; 32],
            principal: founder,
            ds: [0u8; 32],
            deps: vec![],
            ts_physical_ms: 1000,
            body: AuthzBody::Genesis { founder },
        };
        let write = AuthzOp {
            id: [2u8; 32],
            kind: KIND_CREATE_NODE,
            author: [0xA1; 32],
            principal: stranger,
            ds,
            deps: vec![g_id],
            ts_physical_ms: 2000,
            body: AuthzBody::Other,
        };
        assert_eq!(
            authorize(&ds, &[genesis], &write),
            Err(AuthError::NoMembership)
        );
    }

    /// Regenerates AUTH-CERT-001..005 under `conformance/vectors/required/auth/`.
    #[test]
    #[ignore]
    fn write_golden_cert_vectors() {
        use std::fs;
        use std::path::PathBuf;

        let root_seed = [1u8; 32];
        let device_seed = [2u8; 32];
        let device_pk = device_pk_from_seed(&device_seed);
        let cert = issue_device_cert(&root_seed, device_pk, 1_000_000, None).unwrap();
        let revoke =
            issue_device_revoke(&root_seed, device_pk, peer_id(&device_pk), 2_000_000).unwrap();

        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors/required/auth");
        fs::create_dir_all(&dir).unwrap();

        let base = |id: &str, desc: &str, c: &DeviceCert, expect: &str, body: &str| {
            let derived_principal = hex(&principal_id(&c.root_pk));
            format!(
                r#"{{
  "description": "{desc}",
  "id": "{id}",
  "type": "device-cert",
  "invariants": ["I-8"],
  "root_seed_hex": "{rs}",
  "device_seed_hex": "{ds}",
  "principal_id_hex": "{dpr}",
  "peer_id_hex": "{pe}",
  "preimage_body_hex": "{body}",
  "expect": "{expect}",
  "cert": {{
    "kr": {kr},
    "device": "{dev}",
    "principal": "{pr}",
    "root_pk": "{root}",
    "issued": {issued},
    "expiry": null,
    "revoke_of": {rev},
    "cert_sig": "{sig}"
  }}
}}
"#,
                desc = desc,
                id = id,
                rs = hex(&root_seed),
                ds = hex(&device_seed),
                dpr = derived_principal,
                pe = hex(&peer_id(&c.device_pk)),
                body = body,
                expect = expect,
                kr = c.kr,
                dev = hex(&c.device_pk),
                pr = hex(&c.principal_id),
                root = hex(&c.root_pk),
                issued = c.issued,
                rev = c
                    .revoke_of
                    .map(|r| format!("\"{}\"", hex(&r)))
                    .unwrap_or_else(|| "null".into()),
                sig = hex(&c.cert_sig),
            )
        };

        let body = hex(&cert_preimage_body(&cert).unwrap());
        fs::write(
            dir.join("AUTH-CERT-001-valid.json"),
            base(
                "AUTH-CERT-001",
                "Valid device_cert (kr=0) signed by principal root; PeerId/PrincipalId bind.",
                &cert,
                "ok",
                &body,
            ),
        )
        .unwrap();

        let mut bad_sig = cert.clone();
        bad_sig.cert_sig[0] ^= 1;
        fs::write(
            dir.join("AUTH-CERT-002-bad-sig.json"),
            base(
                "AUTH-CERT-002",
                "Tampered cert_sig must fail AUTH_SIG_INVALID.",
                &bad_sig,
                "AUTH_SIG_INVALID",
                &body,
            ),
        )
        .unwrap();

        let mut bad_pr = cert.clone();
        bad_pr.principal_id[0] ^= 1;
        let body_pr = hex(&cert_preimage_body(&bad_pr).unwrap());
        fs::write(
            dir.join("AUTH-CERT-003-principal-mismatch.json"),
            base(
                "AUTH-CERT-003",
                "principal field not equal to BLAKE3(root_pk) → CAP_INVALID.",
                &bad_pr,
                "CAP_INVALID",
                &body_pr,
            ),
        )
        .unwrap();

        let rbody = hex(&cert_preimage_body(&revoke).unwrap());
        fs::write(
            dir.join("AUTH-CERT-004-revoke.json"),
            base(
                "AUTH-CERT-004",
                "Valid device_revoke (kr=1) with revoke_of = PeerId.",
                &revoke,
                "ok",
                &rbody,
            ),
        )
        .unwrap();

        let mut bad_struct = cert.clone();
        bad_struct.revoke_of = Some([9u8; 32]);
        let body_s = hex(&cert_preimage_body(&bad_struct).unwrap());
        fs::write(
            dir.join("AUTH-CERT-005-cert-with-revoke-of.json"),
            base(
                "AUTH-CERT-005",
                "device_cert (kr=0) must not carry revoke_of → CAP_INVALID.",
                &bad_struct,
                "CAP_INVALID",
                &body_s,
            ),
        )
        .unwrap();
    }

    /// Writes AUTH-GEN-* and AUTH-AUTHZ-* vectors (run with --ignored).
    #[test]
    #[ignore]
    fn write_genesis_and_authz_vectors() {
        use std::fs;
        use std::path::PathBuf;

        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors/required/auth");
        fs::create_dir_all(&dir).unwrap();

        let founder = [0xF0u8; 32];
        let author = [0xA0u8; 32];
        let body = GenesisBody {
            founder,
            salt: [
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
                0x0f, 0x10,
            ],
            init_ep: 0,
            fmt_v: 1,
        };
        let env = genesis_envelope(
            author,
            OpTs {
                physical_ms: 1_700_000_000_000,
                logical: 0,
            },
            &body,
        );
        let ds_id = datastore_id_from_genesis(&env).unwrap();
        let g_id = op_id_of(&env).unwrap();
        let preimage = env.id_preimage_body().unwrap();
        let zds = hex(&[0u8; 32]);

        let write_gen = |name: &str, json: String| {
            fs::write(dir.join(name), json).unwrap();
        };

        write_gen(
            "AUTH-GEN-001-valid.json",
            format!(
                r#"{{
  "description": "Valid genesis: zero ds, DatastoreId = BLAKE3(genesis domain ‖ id-preimage).",
  "id": "AUTH-GEN-001",
  "type": "genesis-id",
  "invariants": ["I-9"],
  "expect": "ok",
  "datastore_id_hex": "{ds}",
  "op_id_hex": "{oid}",
  "preimage_body_hex": "{pre}",
  "op": {{
    "v": 1, "ds": "{zds}", "ep": 0, "author": "{auth}",
    "ts": {{ "p": 1700000000000, "l": 0 }}, "deps": [], "grp": null, "kind": 0,
    "body": {{ "t": "map", "v": {{
      "founder": {{ "t": "bytes", "hex": "{founder}" }},
      "salt": {{ "t": "bytes", "hex": "{salt}" }},
      "init_ep": {{ "t": "uint", "v": 0 }},
      "fmt_v": {{ "t": "uint", "v": 1 }}
    }} }}
  }}
}}
"#,
                ds = hex(&ds_id),
                oid = hex(&g_id),
                pre = hex(&preimage),
                zds = zds,
                auth = hex(&author),
                founder = hex(&founder),
                salt = hex(&body.salt),
            ),
        );

        let mut env_bad = env.clone();
        env_bad.ds[0] = 0xff;
        write_gen(
            "AUTH-GEN-002-nonzero-ds.json",
            format!(
                r#"{{
  "description": "Genesis with non-zero ds must fail AUTH_WRONG_DATASTORE.",
  "id": "AUTH-GEN-002",
  "type": "genesis-id",
  "invariants": ["I-9"],
  "expect": "AUTH_WRONG_DATASTORE",
  "op": {{
    "v": 1, "ds": "{ds}", "ep": 0, "author": "{auth}",
    "ts": {{ "p": 1700000000000, "l": 0 }}, "deps": [], "grp": null, "kind": 0,
    "body": {{ "t": "map", "v": {{
      "founder": {{ "t": "bytes", "hex": "{founder}" }},
      "salt": {{ "t": "bytes", "hex": "{salt}" }},
      "init_ep": {{ "t": "uint", "v": 0 }},
      "fmt_v": {{ "t": "uint", "v": 1 }}
    }} }}
  }}
}}
"#,
                ds = hex(&env_bad.ds),
                auth = hex(&author),
                founder = hex(&founder),
                salt = hex(&body.salt),
            ),
        );

        let mut body_salt = body;
        body_salt.salt[0] ^= 0xff;
        let env_salt = genesis_envelope(
            author,
            OpTs {
                physical_ms: 1_700_000_000_000,
                logical: 0,
            },
            &body_salt,
        );
        let ds_salt = datastore_id_from_genesis(&env_salt).unwrap();
        assert_ne!(ds_id, ds_salt);
        write_gen(
            "AUTH-GEN-003-salt-sensitive.json",
            format!(
                r#"{{
  "description": "Salt flip yields a different DatastoreId (self-certifying id).",
  "id": "AUTH-GEN-003",
  "type": "genesis-id",
  "invariants": ["I-9"],
  "expect": "ok",
  "datastore_id_hex": "{ds}",
  "op_id_hex": "{oid}",
  "preimage_body_hex": "{pre}",
  "different_from_hex": "{other}",
  "op": {{
    "v": 1, "ds": "{zds}", "ep": 0, "author": "{auth}",
    "ts": {{ "p": 1700000000000, "l": 0 }}, "deps": [], "grp": null, "kind": 0,
    "body": {{ "t": "map", "v": {{
      "founder": {{ "t": "bytes", "hex": "{founder}" }},
      "salt": {{ "t": "bytes", "hex": "{salt}" }},
      "init_ep": {{ "t": "uint", "v": 0 }},
      "fmt_v": {{ "t": "uint", "v": 1 }}
    }} }}
  }}
}}
"#,
                ds = hex(&ds_salt),
                oid = hex(&op_id_of(&env_salt).unwrap()),
                pre = hex(&env_salt.id_preimage_body().unwrap()),
                other = hex(&ds_id),
                zds = zds,
                auth = hex(&author),
                founder = hex(&founder),
                salt = hex(&body_salt.salt),
            ),
        );

        let ds = ds_id;
        let member = [0x11u8; 32];
        let stranger = [0x22u8; 32];
        let grant_id = [0x33u8; 32];
        let revoke_id = [0x44u8; 32];
        let write_id = [0x55u8; 32];
        let ds_h = hex(&ds);
        let gid = hex(&g_id);
        let auth_h = hex(&author);
        let founder_h = hex(&founder);
        let member_h = hex(&member);
        let grid = hex(&grant_id);
        let rid = hex(&revoke_id);
        let wid = hex(&write_id);

        write_gen(
            "AUTH-AUTHZ-001-founder-write.json",
            format!(
                r#"{{
  "description": "Founder synthetic grant covers CreateNode (write).",
  "id": "AUTH-AUTHZ-001", "type": "authz-predicate",
  "invariants": ["I-8", "I-9"], "expect": "ok",
  "datastore_id_hex": "{ds}",
  "applied": [{{ "id": "{gid}", "kind": 0, "author": "{auth}", "principal": "{founder}",
    "ds": "{zds}", "deps": [], "ts_p": 1000,
    "body": {{ "type": "genesis", "founder": "{founder}" }} }}],
  "candidate": {{ "id": "{wid}", "kind": 1, "author": "{auth}", "principal": "{founder}",
    "ds": "{ds}", "deps": ["{gid}"], "ts_p": 2000, "body": {{ "type": "other" }} }}
}}
"#,
                ds = ds_h,
                gid = gid,
                auth = auth_h,
                founder = founder_h,
                zds = zds,
                wid = wid
            ),
        );

        write_gen(
            "AUTH-AUTHZ-002-no-membership.json",
            format!(
                r#"{{
  "description": "Stranger without grant → AUTH_NO_MEMBERSHIP.",
  "id": "AUTH-AUTHZ-002", "type": "authz-predicate",
  "invariants": ["I-8", "I-9"], "expect": "AUTH_NO_MEMBERSHIP",
  "datastore_id_hex": "{ds}",
  "applied": [{{ "id": "{gid}", "kind": 0, "author": "{auth}", "principal": "{founder}",
    "ds": "{zds}", "deps": [], "ts_p": 1000,
    "body": {{ "type": "genesis", "founder": "{founder}" }} }}],
  "candidate": {{ "id": "{wid}", "kind": 1, "author": "{sauth}", "principal": "{stranger}",
    "ds": "{ds}", "deps": ["{gid}"], "ts_p": 2000, "body": {{ "type": "other" }} }}
}}
"#,
                ds = ds_h,
                gid = gid,
                auth = auth_h,
                founder = founder_h,
                zds = zds,
                wid = wid,
                sauth = hex(&[0xA1u8; 32]),
                stranger = hex(&stranger),
            ),
        );

        write_gen(
            "AUTH-AUTHZ-003-grant-write.json",
            format!(
                r#"{{
  "description": "Explicit write grant in causal past authorizes member CreateNode.",
  "id": "AUTH-AUTHZ-003", "type": "authz-predicate",
  "invariants": ["I-8", "I-9"], "expect": "ok",
  "datastore_id_hex": "{ds}",
  "applied": [
    {{ "id": "{gid}", "kind": 0, "author": "{auth}", "principal": "{founder}",
      "ds": "{zds}", "deps": [], "ts_p": 1000,
      "body": {{ "type": "genesis", "founder": "{founder}" }} }},
    {{ "id": "{grid}", "kind": 6, "author": "{auth}", "principal": "{founder}",
      "ds": "{ds}", "deps": ["{gid}"], "ts_p": 1500,
      "body": {{ "type": "grant", "subject": "{member}", "scopes": [0],
        "expiry": null, "delegable": false, "ds_bind": "{ds}" }} }}
  ],
  "candidate": {{ "id": "{wid}", "kind": 1, "author": "{mauth}", "principal": "{member}",
    "ds": "{ds}", "deps": ["{gid}", "{grid}"], "ts_p": 2000, "body": {{ "type": "other" }} }}
}}
"#,
                ds = ds_h,
                gid = gid,
                auth = auth_h,
                founder = founder_h,
                zds = zds,
                grid = grid,
                member = member_h,
                mauth = hex(&[0xA2u8; 32]),
                wid = wid,
            ),
        );

        write_gen(
            "AUTH-AUTHZ-004-revoked.json",
            format!(
                r#"{{
  "description": "Revoke in causal past of write → AUTH_REVOKED.",
  "id": "AUTH-AUTHZ-004", "type": "authz-predicate",
  "invariants": ["I-8", "I-9"], "expect": "AUTH_REVOKED",
  "datastore_id_hex": "{ds}",
  "applied": [
    {{ "id": "{gid}", "kind": 0, "author": "{auth}", "principal": "{founder}",
      "ds": "{zds}", "deps": [], "ts_p": 1000,
      "body": {{ "type": "genesis", "founder": "{founder}" }} }},
    {{ "id": "{grid}", "kind": 6, "author": "{auth}", "principal": "{founder}",
      "ds": "{ds}", "deps": ["{gid}"], "ts_p": 1500,
      "body": {{ "type": "grant", "subject": "{member}", "scopes": [0],
        "expiry": null, "delegable": false, "ds_bind": "{ds}" }} }},
    {{ "id": "{rid}", "kind": 7, "author": "{auth}", "principal": "{founder}",
      "ds": "{ds}", "deps": ["{gid}", "{grid}"], "ts_p": 1800,
      "body": {{ "type": "revoke", "grant": "{grid}", "reason": 2 }} }}
  ],
  "candidate": {{ "id": "{wid}", "kind": 1, "author": "{mauth}", "principal": "{member}",
    "ds": "{ds}", "deps": ["{gid}", "{grid}", "{rid}"], "ts_p": 2000,
    "body": {{ "type": "other" }} }}
}}
"#,
                ds = ds_h,
                gid = gid,
                auth = auth_h,
                founder = founder_h,
                zds = zds,
                grid = grid,
                rid = rid,
                member = member_h,
                mauth = hex(&[0xA2u8; 32]),
                wid = wid,
            ),
        );

        write_gen(
            "AUTH-AUTHZ-005-concurrent-revoke.json",
            format!(
                r#"{{
  "description": "Revoke concurrent (not in candidate deps) does not defeat write.",
  "id": "AUTH-AUTHZ-005", "type": "authz-predicate",
  "invariants": ["I-1", "I-8", "I-9"], "expect": "ok",
  "datastore_id_hex": "{ds}",
  "applied": [
    {{ "id": "{gid}", "kind": 0, "author": "{auth}", "principal": "{founder}",
      "ds": "{zds}", "deps": [], "ts_p": 1000,
      "body": {{ "type": "genesis", "founder": "{founder}" }} }},
    {{ "id": "{grid}", "kind": 6, "author": "{auth}", "principal": "{founder}",
      "ds": "{ds}", "deps": ["{gid}"], "ts_p": 1500,
      "body": {{ "type": "grant", "subject": "{member}", "scopes": [0],
        "expiry": null, "delegable": false, "ds_bind": "{ds}" }} }},
    {{ "id": "{rid}", "kind": 7, "author": "{auth}", "principal": "{founder}",
      "ds": "{ds}", "deps": ["{gid}", "{grid}"], "ts_p": 1800,
      "body": {{ "type": "revoke", "grant": "{grid}", "reason": 2 }} }}
  ],
  "candidate": {{ "id": "{wid}", "kind": 1, "author": "{mauth}", "principal": "{member}",
    "ds": "{ds}", "deps": ["{gid}", "{grid}"], "ts_p": 2000, "body": {{ "type": "other" }} }}
}}
"#,
                ds = ds_h,
                gid = gid,
                auth = auth_h,
                founder = founder_h,
                zds = zds,
                grid = grid,
                rid = rid,
                member = member_h,
                mauth = hex(&[0xA2u8; 32]),
                wid = wid,
            ),
        );

        write_gen(
            "AUTH-AUTHZ-006-wrong-ds.json",
            format!(
                r#"{{
  "description": "Candidate ds mismatch → AUTH_WRONG_DATASTORE.",
  "id": "AUTH-AUTHZ-006", "type": "authz-predicate",
  "invariants": ["I-9"], "expect": "AUTH_WRONG_DATASTORE",
  "datastore_id_hex": "{ds}",
  "applied": [{{ "id": "{gid}", "kind": 0, "author": "{auth}", "principal": "{founder}",
    "ds": "{zds}", "deps": [], "ts_p": 1000,
    "body": {{ "type": "genesis", "founder": "{founder}" }} }}],
  "candidate": {{ "id": "{wid}", "kind": 1, "author": "{auth}", "principal": "{founder}",
    "ds": "{badds}", "deps": ["{gid}"], "ts_p": 2000, "body": {{ "type": "other" }} }}
}}
"#,
                ds = ds_h,
                gid = gid,
                auth = auth_h,
                founder = founder_h,
                zds = zds,
                wid = wid,
                badds = hex(&[0xFFu8; 32]),
            ),
        );

        // --- admission tokens ---
        let root_seed = [1u8; 32];
        let device_seed = [2u8; 32];
        let device_pk = device_pk_from_seed(&device_seed);
        let cert = issue_device_cert(&root_seed, device_pk, 1_000_000, None).unwrap();
        let subject = cert.principal_id;
        // founder synthetic grant id = g_id, scopes all
        let grant = KnownGrant {
            id: g_id,
            ds: ds_id,
            subject,
            scopes: vec![SCOPE_WRITE, SCOPE_ADMIN, SCOPE_READ, SCOPE_SYNC],
            expiry: None,
            revoked: false,
        };
        let tok = AdmissionToken {
            ds: ds_id,
            subject,
            grant: g_id,
            scopes: vec![SCOPE_READ, SCOPE_SYNC],
            device: device_pk,
            cert: cert.clone(),
            sig: [0u8; 64],
        };
        let tok = sign_admission_token(&device_seed, tok).unwrap();
        verify_admission_token(&tok, std::slice::from_ref(&grant)).unwrap();
        let pre_body = hex(&admission_token_preimage_body(&tok).unwrap());

        let cert_json = |c: &DeviceCert| {
            format!(
                r#"{{ "kr": {kr}, "device": "{dev}", "principal": "{pr}", "root_pk": "{root}",
    "issued": {issued}, "expiry": null, "revoke_of": null, "cert_sig": "{sig}" }}"#,
                kr = c.kr,
                dev = hex(&c.device_pk),
                pr = hex(&c.principal_id),
                root = hex(&c.root_pk),
                issued = c.issued,
                sig = hex(&c.cert_sig),
            )
        };

        write_gen(
            "AUTH-ADM-001-valid.json",
            format!(
                r#"{{
  "description": "Valid SUBSCRIBE admission token: cert + founder grant + device sig.",
  "id": "AUTH-ADM-001", "type": "admission-token",
  "invariants": ["I-8", "I-9"], "expect": "ok",
  "preimage_body_hex": "{pre}",
  "known_grants": [{{
    "id": "{gid}", "ds": "{ds}", "subject": "{sub}",
    "scopes": [0,1,2,3], "revoked": false
  }}],
  "token": {{
    "ds": "{ds}", "subject": "{sub}", "grant": "{gid}",
    "scopes": [2,3], "device": "{dev}",
    "cert": {cert},
    "sig": "{sig}"
  }}
}}
"#,
                pre = pre_body,
                gid = hex(&g_id),
                ds = hex(&ds_id),
                sub = hex(&subject),
                dev = hex(&device_pk),
                cert = cert_json(&cert),
                sig = hex(&tok.sig),
            ),
        );

        let mut bad = tok.clone();
        bad.sig[0] ^= 1;
        write_gen(
            "AUTH-ADM-002-bad-sig.json",
            format!(
                r#"{{
  "description": "Tampered admission token sig → AUTH_SIG_INVALID.",
  "id": "AUTH-ADM-002", "type": "admission-token",
  "invariants": ["I-8"], "expect": "AUTH_SIG_INVALID",
  "known_grants": [{{
    "id": "{gid}", "ds": "{ds}", "subject": "{sub}",
    "scopes": [0,1,2,3], "revoked": false
  }}],
  "token": {{
    "ds": "{ds}", "subject": "{sub}", "grant": "{gid}",
    "scopes": [2,3], "device": "{dev}",
    "cert": {cert},
    "sig": "{sig}"
  }}
}}
"#,
                gid = hex(&g_id),
                ds = hex(&ds_id),
                sub = hex(&subject),
                dev = hex(&device_pk),
                cert = cert_json(&cert),
                sig = hex(&bad.sig),
            ),
        );

        let mut bad_ds = tok.clone();
        bad_ds.ds = [0xFFu8; 32];
        // re-sign so only grant/ds check fails (or resign would bind wrong ds)
        let bad_ds = sign_admission_token(&device_seed, bad_ds).unwrap();
        write_gen(
            "AUTH-ADM-003-wrong-ds.json",
            format!(
                r#"{{
  "description": "Token ds not matching known grant → AUTH_NO_MEMBERSHIP.",
  "id": "AUTH-ADM-003", "type": "admission-token",
  "invariants": ["I-9"], "expect": "AUTH_NO_MEMBERSHIP",
  "known_grants": [{{
    "id": "{gid}", "ds": "{ds}", "subject": "{sub}",
    "scopes": [0,1,2,3], "revoked": false
  }}],
  "token": {{
    "ds": "{badds}", "subject": "{sub}", "grant": "{gid}",
    "scopes": [2,3], "device": "{dev}",
    "cert": {cert},
    "sig": "{sig}"
  }}
}}
"#,
                gid = hex(&g_id),
                ds = hex(&ds_id),
                badds = hex(&bad_ds.ds),
                sub = hex(&subject),
                dev = hex(&device_pk),
                cert = cert_json(&cert),
                sig = hex(&bad_ds.sig),
            ),
        );

        write_gen(
            "AUTH-ADM-004-revoked.json",
            format!(
                r#"{{
  "description": "Known grant marked revoked → AUTH_REVOKED.",
  "id": "AUTH-ADM-004", "type": "admission-token",
  "invariants": ["I-8", "I-9"], "expect": "AUTH_REVOKED",
  "known_grants": [{{
    "id": "{gid}", "ds": "{ds}", "subject": "{sub}",
    "scopes": [0,1,2,3], "revoked": true
  }}],
  "token": {{
    "ds": "{ds}", "subject": "{sub}", "grant": "{gid}",
    "scopes": [2,3], "device": "{dev}",
    "cert": {cert},
    "sig": "{sig}"
  }}
}}
"#,
                gid = hex(&g_id),
                ds = hex(&ds_id),
                sub = hex(&subject),
                dev = hex(&device_pk),
                cert = cert_json(&cert),
                sig = hex(&tok.sig),
            ),
        );
    }
}
