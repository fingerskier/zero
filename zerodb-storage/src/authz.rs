//! Peer-side AUTH.md §4 evaluation over experimental WireOps.
//!
//! Solo-device principals: `PrincipalId == PeerId == BLAKE3(device pk)`.
//! AUTH is enforced only when a genesis op is present (or `auth` meta is set).

use zerodb_core::auth::{
    AuthzBody, AuthzOp, KIND_CAP_GRANT, KIND_CAP_REVOKE, KIND_GENESIS, auth_error_tag, authorize,
};

use crate::{StoreError, WireOp, decode32};

pub const META_AUTH: &str = "auth";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthReject {
    pub op_id: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestResult {
    Applied,
    Duplicate,
    Rejected { reason: &'static str },
}

pub fn is_control_kind(kind: u64) -> bool {
    matches!(kind, KIND_GENESIS | KIND_CAP_GRANT | KIND_CAP_REVOKE)
}

pub fn wire_to_authz(wire: &WireOp) -> Result<AuthzOp, StoreError> {
    let author = decode32(&wire.author)?;
    Ok(AuthzOp {
        id: decode32(&wire.id)?,
        kind: wire.kind,
        author,
        principal: author,
        ds: decode32(&wire.ds)?,
        deps: wire
            .deps
            .iter()
            .map(|dep| decode32(dep))
            .collect::<Result<Vec<_>, _>>()?,
        ts_physical_ms: wire.ts.p,
        body: authz_body(wire)?,
    })
}

fn authz_body(wire: &WireOp) -> Result<AuthzBody, StoreError> {
    match wire.kind {
        KIND_GENESIS => Ok(AuthzBody::Genesis {
            founder: body_hex32(&wire.body, "founder")?,
        }),
        KIND_CAP_GRANT => {
            let scopes = match wire.body.get("scopes") {
                Some(serde_json::Value::Array(items)) => items
                    .iter()
                    .map(|item| {
                        item.as_u64()
                            .ok_or_else(|| StoreError::Invalid("grant scopes".into()))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                _ => return Err(StoreError::Authz("CAP_INVALID")),
            };
            let expiry = match wire.body.get("expiry") {
                None | Some(serde_json::Value::Null) => None,
                Some(v) => Some(
                    v.as_u64()
                        .ok_or_else(|| StoreError::Invalid("grant expiry".into()))?,
                ),
            };
            let delegable = wire
                .body
                .get("delegable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(AuthzBody::Grant {
                subject: body_hex32(&wire.body, "subject")?,
                scopes,
                expiry,
                delegable,
                ds_bind: body_hex32(&wire.body, "ds_bind")?,
            })
        }
        KIND_CAP_REVOKE => Ok(AuthzBody::Revoke {
            grant: body_hex32(&wire.body, "grant")?,
            reason: wire
                .body
                .get("reason")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        }),
        _ => Ok(AuthzBody::Other),
    }
}

fn body_hex32(body: &serde_json::Value, key: &str) -> Result<[u8; 32], StoreError> {
    let hex = body
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| StoreError::Invalid(format!("body.{key}")))?;
    decode32(hex)
}

pub fn authorize_wire(
    datastore_id: &[u8; 32],
    applied: &[WireOp],
    candidate: &WireOp,
) -> Result<(), StoreError> {
    let applied_authz = applied
        .iter()
        .map(wire_to_authz)
        .collect::<Result<Vec<_>, _>>()?;
    let candidate_authz = wire_to_authz(candidate)?;
    authorize(datastore_id, &applied_authz, &candidate_authz)
        .map_err(|err| StoreError::Authz(auth_error_tag(&err)))
}

pub fn load_applied_wires(
    backend: &dyn crate::backend::BackendTxn,
) -> Result<Vec<WireOp>, StoreError> {
    let mut out = Vec::new();
    for wire in backend.op_wires()? {
        out.push(serde_json::from_str(&wire).map_err(|e| StoreError::Invalid(e.to_string()))?);
    }
    Ok(out)
}

pub fn control_dep_hex(applied: &[WireOp]) -> Result<Vec<String>, StoreError> {
    let mut deps: Vec<String> = applied
        .iter()
        .filter(|op| is_control_kind(op.kind))
        .map(|op| op.id.clone())
        .collect();
    if deps.len() > 64 {
        let genesis = deps.remove(0);
        let keep = deps.split_off(deps.len().saturating_sub(63));
        deps = std::iter::once(genesis).chain(keep).collect();
    }
    Ok(deps)
}

pub fn bundle_has_genesis(ops: &[WireOp]) -> bool {
    ops.iter().any(|op| op.kind == KIND_GENESIS)
}

/// Datastore id for a catch-up batch: genesis-derived when present, else first `ds`.
pub fn bundle_datastore_id(ops: &[WireOp]) -> Result<String, StoreError> {
    if let Some(genesis) = ops.iter().find(|op| op.kind == KIND_GENESIS) {
        let author = decode32(&genesis.author)?;
        let founder = body_hex32(&genesis.body, "founder")?;
        let salt_hex = genesis
            .body
            .get("salt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StoreError::Invalid("genesis salt".into()))?;
        let salt_bytes = hex::decode(salt_hex).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let salt: [u8; 16] = salt_bytes
            .try_into()
            .map_err(|_| StoreError::Invalid("genesis salt length".into()))?;
        let env = zerodb_core::auth::genesis_envelope(
            author,
            zerodb_core::op::OpTs {
                physical_ms: genesis.ts.p,
                logical: genesis.ts.l,
            },
            &zerodb_core::auth::GenesisBody {
                founder,
                salt,
                init_ep: genesis
                    .body
                    .get("init_ep")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                fmt_v: genesis
                    .body
                    .get("fmt_v")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1),
            },
        );
        let ds = zerodb_core::auth::datastore_id_from_genesis(&env)
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        return Ok(hex::encode(ds));
    }
    ops.iter()
        .find(|op| op.ds != "00".repeat(32))
        .or_else(|| ops.first())
        .map(|op| op.ds.clone())
        .ok_or_else(|| StoreError::Invalid("empty op batch".into()))
}
