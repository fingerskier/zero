//! Operation envelope per doc/KERNEL.md §4.1 and `OpId` derivation per §4.4.
//!
//! The envelope encodes as one canonical CBOR map (keys "v", "ds", "ep",
//! "author", "ts", "deps", "grp", "kind", "body" [+ "id", "sig" in the full
//! form]). The id-preimage is the map WITHOUT "id"/"sig", prefixed by the
//! domain-separation string; `OpId = BLAKE3(id-preimage)`. Signatures
//! (Ed25519 over the domain-separated OpId) land with the crypto layer.

use crate::cbor::{Cbor, CborError, encode};

/// Domain-separation prefix for the id-preimage (registry `domain_separation.op_id`).
pub const DOMAIN_OP_ID: &[u8] = b"zerodb-op-id-v1";

/// HLC timestamp as carried in the envelope ("ts": {"l": .., "p": ..});
/// the peer component is implied by "author".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpTs {
    pub physical_ms: u64,
    pub logical: u16,
}

/// KERNEL §4.1 envelope, minus "id"/"sig" (which are derived/attached).
#[derive(Debug, Clone)]
pub struct OpEnvelope {
    pub v: u64,
    pub ds: [u8; 32],
    pub ep: u64,
    pub author: [u8; 32],
    pub ts: OpTs,
    pub deps: Vec<[u8; 32]>,
    pub grp: Option<[u8; 16]>,
    pub kind: u64,
    pub body: Cbor,
}

impl OpEnvelope {
    fn as_cbor(&self) -> Cbor {
        Cbor::Map(vec![
            ("v".into(), Cbor::Uint(self.v)),
            ("ds".into(), Cbor::Bytes(self.ds.to_vec())),
            ("ep".into(), Cbor::Uint(self.ep)),
            ("author".into(), Cbor::Bytes(self.author.to_vec())),
            (
                "ts".into(),
                Cbor::Map(vec![
                    ("l".into(), Cbor::Uint(self.ts.logical as u64)),
                    ("p".into(), Cbor::Uint(self.ts.physical_ms)),
                ]),
            ),
            (
                "deps".into(),
                Cbor::Array(self.deps.iter().map(|d| Cbor::Bytes(d.to_vec())).collect()),
            ),
            (
                "grp".into(),
                match &self.grp {
                    Some(g) => Cbor::Bytes(g.to_vec()),
                    None => Cbor::Null,
                },
            ),
            ("kind".into(), Cbor::Uint(self.kind)),
            ("body".into(), self.body.clone()),
        ])
    }

    /// Canonical bytes of the envelope map without "id"/"sig" (KERNEL §4.4).
    pub fn id_preimage_body(&self) -> Result<Vec<u8>, CborError> {
        encode(&self.as_cbor())
    }

    /// `OpId = BLAKE3(domain ‖ canonical bytes)`.
    pub fn op_id(&self) -> Result<[u8; 32], CborError> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(DOMAIN_OP_ID);
        hasher.update(&self.id_preimage_body()?);
        Ok(*hasher.finalize().as_bytes())
    }
}

/// Experimental WireOp JSON body → CBOR (same rules as LocalStore).
/// Hex strings on `node` / `edge` / `src` / `dst` become bytes.
/// AUTH control bodies also encode `founder` / `salt` / `subject` / `ds_bind` /
/// `grant` as bytes, and numeric arrays (grant `scopes`) as uints.
/// Group-key wraps (`kr = 2`) and KERNEL §7 envelopes recurse into maps.
pub fn json_to_cbor_body(v: &serde_json::Value) -> Result<Cbor, String> {
    match v {
        serde_json::Value::Object(_) => json_to_cbor(v, None),
        _ => Ok(Cbor::Map(vec![])),
    }
}

fn json_to_cbor(val: &serde_json::Value, key: Option<&str>) -> Result<Cbor, String> {
    match val {
        serde_json::Value::String(s) => {
            if key.is_some_and(is_hex_body_key) {
                let b = hex::decode(s).map_err(|e| e.to_string())?;
                Ok(Cbor::Bytes(b))
            } else {
                Ok(Cbor::Text(s.clone()))
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Ok(Cbor::Uint(u))
            } else {
                Ok(Cbor::Text(n.to_string()))
            }
        }
        serde_json::Value::Bool(b) => Ok(Cbor::Bool(*b)),
        serde_json::Value::Null => Ok(Cbor::Null),
        serde_json::Value::Array(a) => {
            let mut items = Vec::new();
            for x in a {
                items.push(json_to_cbor(x, None)?);
            }
            Ok(Cbor::Array(items))
        }
        serde_json::Value::Object(o) => {
            let mut entries = Vec::new();
            for (k, v) in o {
                entries.push((k.clone(), json_to_cbor(v, Some(k))?));
            }
            Ok(Cbor::Map(entries))
        }
    }
}

fn is_hex_body_key(k: &str) -> bool {
    matches!(
        k,
        "node"
            | "edge"
            | "src"
            | "dst"
            | "founder"
            | "salt"
            | "subject"
            | "ds_bind"
            | "grant"
            | "encrypted"
            | "key_id"
            | "recipient"
            | "eph_pk"
            | "nonce"
            | "wrapped"
            | "device"
            | "principal"
            | "root_pk"
            | "cert_sig"
            | "revoke_of"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> OpEnvelope {
        OpEnvelope {
            v: 1,
            ds: [0x11; 32],
            ep: 0,
            author: [0x22; 32],
            ts: OpTs {
                physical_ms: 1_000_000,
                logical: 3,
            },
            deps: vec![[0x33; 32]],
            grp: None,
            kind: 1,
            body: Cbor::Map(vec![
                ("label".into(), Cbor::Text("User".into())),
                ("node".into(), Cbor::Bytes(vec![0x44; 16])),
            ]),
        }
    }

    #[test]
    fn op_id_is_stable_and_field_sensitive() {
        let a = sample();
        let id1 = a.op_id().unwrap();
        assert_eq!(id1, a.op_id().unwrap());

        let mut b = sample();
        b.ep = 1;
        assert_ne!(id1, b.op_id().unwrap(), "epoch must be in the preimage");

        let mut c = sample();
        c.ds[0] ^= 1;
        assert_ne!(id1, c.op_id().unwrap(), "datastore must be in the preimage");
    }

    #[test]
    fn preimage_round_trips_canonically() {
        let bytes = sample().id_preimage_body().unwrap();
        let decoded = crate::cbor::decode(&bytes).unwrap();
        assert_eq!(crate::cbor::encode(&decoded).unwrap(), bytes);
    }
}
