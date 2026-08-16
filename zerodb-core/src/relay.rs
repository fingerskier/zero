//! RELAY 0.2.2-draft wire-transcript model (M3a) plus M3b-sig admission.
//!
//! Handshake (HELLO/AUTH/WELCOME capabilities), dual-root catch-up
//! invariant (CX-08), resume cursor, reject-ack, and ordered envelope
//! frames. `admit_experimental_op` is the relay-side signature / OpId /
//! datastore-binding check (M3b-sig). AUTH membership grants remain later.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::cbor::Cbor;
use crate::merkle::{MerkleOp, merkle_root};
use crate::op::{OpEnvelope, OpTs, json_to_cbor_body};
use crate::sign::verify_op;

/// Domain-separation prefix (registry `domain_separation.relay_auth`).
pub const DOMAIN_RELAY_AUTH: &[u8] = b"zerodb-relay-auth-v1";

/// Negotiable session capabilities (sorted).
pub const RELAY_CAPS: &[&str] = &["dual-root", "merkle-walk-v1", "reject-ack", "resume-cursor"];

/// RELAY §10 `AUTH_FAILED`.
pub const ERR_AUTH_FAILED: u16 = 0x201;
/// RELAY §9.1 unsigned / bad signature (frame-level; per-op uses `REJECT`/`SIG`).
pub const ERR_SIG_INVALID: u16 = 0x301;

/// `OP_ACK` reject reasons (RELAY §4.4).
pub const REJECT_DECODE: &str = "DECODE";
pub const REJECT_SIG: &str = "SIG";
pub const REJECT_AUTHZ: &str = "AUTHZ";

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

/// Header fields the relay persists after admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedOp {
    pub op_id: [u8; 32],
    pub author: [u8; 32],
    pub physical_ms: u64,
    pub logical: u16,
}

/// Per-op admission failure (maps to `OP_ACK` `REJECT.reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitReject {
    Decode,
    Sig,
    Authz,
}

impl AdmitReject {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Decode => REJECT_DECODE,
            Self::Sig => REJECT_SIG,
            Self::Authz => REJECT_AUTHZ,
        }
    }
}

fn map_get<'a>(c: &'a Cbor, k: &str) -> &'a Cbor {
    static NULL: Cbor = Cbor::Null;
    match c {
        Cbor::Map(ents) => ents
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v)
            .unwrap_or(&NULL),
        _ => &NULL,
    }
}

fn take_bytes<const N: usize>(c: &Cbor) -> Option<[u8; N]> {
    match c {
        Cbor::Bytes(b) if b.len() == N => b.as_slice().try_into().ok(),
        _ => None,
    }
}

fn hex32(s: &str) -> Result<[u8; 32], AdmitReject> {
    let b = hex::decode(s).map_err(|_| AdmitReject::Decode)?;
    b.try_into().map_err(|_| AdmitReject::Decode)
}

fn hex64(s: &str) -> Result<[u8; 64], AdmitReject> {
    let b = hex::decode(s).map_err(|_| AdmitReject::Decode)?;
    b.try_into().map_err(|_| AdmitReject::Decode)
}

/// Admit an experimental relay op (`wire` JSON + header).
///
/// Checks: decode, datastore bind, author = BLAKE3(pk), OpId = preimage hash,
/// Ed25519 signature over the OpId. Transport sender MAY differ from author.
pub fn admit_experimental_op(op: &Cbor, datastore: &str) -> Result<AdmittedOp, AdmitReject> {
    if !matches!(op, Cbor::Map(_)) {
        return Err(AdmitReject::Decode);
    }
    let header_id = take_bytes::<32>(map_get(op, "op_id")).ok_or(AdmitReject::Decode)?;
    let header_author = take_bytes::<32>(map_get(op, "author")).ok_or(AdmitReject::Decode)?;
    let physical_ms = match map_get(op, "physical_ms") {
        Cbor::Uint(n) => *n,
        _ => return Err(AdmitReject::Decode),
    };
    let logical = match map_get(op, "logical") {
        Cbor::Uint(n) => *n as u16,
        _ => return Err(AdmitReject::Decode),
    };
    let wire_txt = match map_get(op, "wire") {
        Cbor::Text(s) => s,
        Cbor::Null => return Err(AdmitReject::Sig),
        _ => return Err(AdmitReject::Decode),
    };
    let wire: serde_json::Value =
        serde_json::from_str(wire_txt).map_err(|_| AdmitReject::Decode)?;
    let get_str = |k: &str| {
        wire.get(k)
            .and_then(|v| v.as_str())
            .ok_or(AdmitReject::Decode)
    };
    if get_str("ds")? != datastore {
        return Err(AdmitReject::Authz);
    }
    let claimed_id = hex32(get_str("id")?)?;
    let author = hex32(get_str("author")?)?;
    let author_pk = hex32(get_str("author_pk")?)?;
    let sig = hex64(get_str("sig")?)?;
    if claimed_id != header_id || author != header_author {
        return Err(AdmitReject::Sig);
    }
    if peer_id_from_pk(&author_pk) != author {
        return Err(AdmitReject::Sig);
    }
    let ts = wire.get("ts").ok_or(AdmitReject::Decode)?;
    let ts_p = ts
        .get("p")
        .and_then(|v| v.as_u64())
        .ok_or(AdmitReject::Decode)?;
    let ts_l = ts
        .get("l")
        .and_then(|v| v.as_u64())
        .ok_or(AdmitReject::Decode)? as u16;
    if ts_p != physical_ms || ts_l != logical {
        return Err(AdmitReject::Sig);
    }
    let v = wire
        .get("v")
        .and_then(|x| x.as_u64())
        .ok_or(AdmitReject::Decode)?;
    let ep = wire
        .get("ep")
        .and_then(|x| x.as_u64())
        .ok_or(AdmitReject::Decode)?;
    let kind = wire
        .get("kind")
        .and_then(|x| x.as_u64())
        .ok_or(AdmitReject::Decode)?;
    let deps = match wire.get("deps") {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|d| hex32(d.as_str().ok_or(AdmitReject::Decode)?))
            .collect::<Result<Vec<_>, _>>()?,
        Some(serde_json::Value::Null) | None => Vec::new(),
        _ => return Err(AdmitReject::Decode),
    };
    let grp = match wire.get("grp") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(h)) => {
            let b = hex::decode(h).map_err(|_| AdmitReject::Decode)?;
            Some(<[u8; 16]>::try_from(b).map_err(|_| AdmitReject::Decode)?)
        }
        _ => return Err(AdmitReject::Decode),
    };
    let ds = hex32(get_str("ds")?).unwrap_or([0u8; 32]);
    // Experimental LocalStore stamps hex datastore ids. Protocol tests may use
    // a non-hex label (`app:main`); bind by string above and hash as bytes only
    // when the field is 32-byte hex so OpId still covers a stable ds slot.
    let ds_bytes = if get_str("ds")?.len() == 64 {
        hex32(get_str("ds")?)?
    } else {
        ds
    };
    let body = json_to_cbor_body(wire.get("body").unwrap_or(&serde_json::Value::Null))
        .map_err(|_| AdmitReject::Decode)?;
    let envelope = OpEnvelope {
        v,
        ds: ds_bytes,
        ep,
        author,
        ts: OpTs {
            physical_ms,
            logical,
        },
        deps,
        grp,
        kind,
        body,
    };
    let computed = envelope.op_id().map_err(|_| AdmitReject::Decode)?;
    if computed != claimed_id {
        return Err(AdmitReject::Sig);
    }
    if !verify_op(&author_pk, &claimed_id, &sig) {
        return Err(AdmitReject::Sig);
    }
    Ok(AdmittedOp {
        op_id: claimed_id,
        author,
        physical_ms,
        logical,
    })
}

/// Mint a signed experimental CreateNode relay op (tests / protocol fixtures).
pub fn mint_experimental_relay_op(
    seed: &[u8; 32],
    datastore: &str,
    physical_ms: u64,
    logical: u16,
    tag: u16,
) -> Cbor {
    let key = SigningKey::from_bytes(seed);
    let author_pk = key.verifying_key().to_bytes();
    let author = peer_id_from_pk(&author_pk);
    let mut node = [0u8; 16];
    node[14] = (tag >> 8) as u8;
    node[15] = tag as u8;
    let body_json = serde_json::json!({
        "label": format!("n{tag}"),
        "node": hex::encode(node),
    });
    let ds_bytes = if datastore.len() == 64 {
        hex32(datastore).unwrap_or([0u8; 32])
    } else {
        [0u8; 32]
    };
    let envelope = OpEnvelope {
        v: 1,
        ds: ds_bytes,
        ep: 0,
        author,
        ts: OpTs {
            physical_ms,
            logical,
        },
        deps: vec![],
        grp: None,
        kind: 1,
        body: json_to_cbor_body(&body_json).expect("body"),
    };
    let op_id = envelope.op_id().expect("op_id");
    let (_, sig) = crate::sign::sign_op(seed, &op_id);
    let wire = serde_json::json!({
        "id": hex::encode(op_id),
        "v": 1,
        "ds": datastore,
        "ep": 0,
        "author": hex::encode(author),
        "author_pk": hex::encode(author_pk),
        "ts": { "p": physical_ms, "l": logical },
        "deps": [],
        "kind": 1,
        "body": body_json,
        "sig": hex::encode(sig),
    });
    Cbor::Map(vec![
        ("op_id".into(), Cbor::Bytes(op_id.to_vec())),
        ("author".into(), Cbor::Bytes(author.to_vec())),
        ("physical_ms".into(), Cbor::Uint(physical_ms)),
        ("logical".into(), Cbor::Uint(logical as u64)),
        ("wire".into(), Cbor::Text(wire.to_string())),
    ])
}

#[cfg(test)]
mod admit_tests {
    use super::*;

    const SEED: [u8; 32] = [7u8; 32];

    fn take_map(c: &Cbor) -> &[(String, Cbor)] {
        match c {
            Cbor::Map(m) => m,
            _ => panic!("map"),
        }
    }

    fn set_field(op: &Cbor, key: &str, val: Cbor) -> Cbor {
        let ents: Vec<(String, Cbor)> = take_map(op)
            .iter()
            .map(|(k, v)| {
                if k == key {
                    (k.clone(), val.clone())
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect();
        Cbor::Map(ents)
    }

    #[test]
    fn minted_op_is_admitted() {
        let op = mint_experimental_relay_op(&SEED, "app:main", 10, 0, 1);
        admit_experimental_op(&op, "app:main").expect("admit");
    }

    #[test]
    fn unsigned_is_sig() {
        let op = mint_experimental_relay_op(&SEED, "app:main", 10, 0, 1);
        let op = set_field(&op, "wire", Cbor::Null);
        assert_eq!(
            admit_experimental_op(&op, "app:main"),
            Err(AdmitReject::Sig)
        );
    }

    #[test]
    fn wrong_datastore_is_authz() {
        let op = mint_experimental_relay_op(&SEED, "app:main", 10, 0, 1);
        assert_eq!(admit_experimental_op(&op, "other"), Err(AdmitReject::Authz));
    }

    #[test]
    fn flipped_signature_is_sig() {
        let op = mint_experimental_relay_op(&SEED, "app:main", 10, 0, 1);
        let wire = match map_get(&op, "wire") {
            Cbor::Text(s) => s.clone(),
            _ => panic!("wire"),
        };
        let mut v: serde_json::Value = serde_json::from_str(&wire).unwrap();
        let mut sig = hex::decode(v["sig"].as_str().unwrap()).unwrap();
        sig[0] ^= 1;
        v["sig"] = serde_json::Value::String(hex::encode(sig));
        let op = set_field(&op, "wire", Cbor::Text(v.to_string()));
        assert_eq!(
            admit_experimental_op(&op, "app:main"),
            Err(AdmitReject::Sig)
        );
    }

    #[test]
    fn flipped_body_breaks_opid() {
        let op = mint_experimental_relay_op(&SEED, "app:main", 10, 0, 1);
        let wire = match map_get(&op, "wire") {
            Cbor::Text(s) => s.clone(),
            _ => panic!("wire"),
        };
        let mut v: serde_json::Value = serde_json::from_str(&wire).unwrap();
        v["body"]["label"] = serde_json::Value::String("tampered".into());
        let op = set_field(&op, "wire", Cbor::Text(v.to_string()));
        assert_eq!(
            admit_experimental_op(&op, "app:main"),
            Err(AdmitReject::Sig)
        );
    }
}
