//! Shared `crdt-apply` vector runner (KERNEL §9).
//!
//! Author/op-id strings in vectors become raw UTF-8 bytes; bytewise
//! comparison then matches the JS runner's string comparison exactly
//! (ASCII), so both runners and the NAPI binding share one total order.

use serde_json::Value as Json;

use crate::kernel::{
    CrdtState, Flag, GCounter, Id, KernelError, KernelOp, Lww, OrSet, Payload, PnCounter, Replica,
    Value,
};

/// Apply one `crdt-apply` vector. Returns
/// `{ id, orders: [{ equivocation, error, state }] }`.
pub fn apply_crdt_vector(v: &Json) -> Result<Json, String> {
    if v["type"].as_str() != Some("crdt-apply") {
        return Err(format!(
            "expected type crdt-apply, got {:?}",
            v["type"].as_str()
        ));
    }
    let id = v["id"].as_str().ok_or("vector missing id")?.to_string();
    let crdt = v["crdt"].as_str().ok_or("vector missing crdt")?;
    let ops_json = v["ops"].as_array().ok_or("vector missing ops")?;
    let ops: Result<Vec<KernelOp>, String> = ops_json.iter().map(|o| kernel_op(o, crdt)).collect();
    let ops = ops?;
    let orders = v["orders"].as_array().ok_or("vector missing orders")?;

    let mut out = Vec::with_capacity(orders.len());
    for order in orders {
        let sequence: Result<Vec<usize>, String> = order
            .as_array()
            .ok_or("order must be an array")?
            .iter()
            .map(|i| {
                i.as_u64()
                    .map(|n| n as usize)
                    .ok_or_else(|| "order index must be a uint".to_string())
            })
            .collect();
        out.push(run_order(crdt, &ops, &sequence?)?);
    }

    Ok(serde_json::json!({
        "id": id,
        "orders": out,
    }))
}

fn run_order(crdt: &str, ops: &[KernelOp], sequence: &[usize]) -> Result<Json, String> {
    for &index in sequence {
        if index >= ops.len() {
            return Err(format!("order index {index} out of range"));
        }
    }
    match crdt {
        "lww" => order_state::<Lww>(ops, sequence, |s| match s.value() {
            Some(v) => Ok(serde_json::json!({ "value": value_json(v)? })),
            None => Ok(serde_json::json!({ "value": Json::Null })),
        }),
        "gcounter" => order_state::<GCounter>(ops, sequence, |s| {
            Ok(serde_json::json!({ "value": s.value() }))
        }),
        "pncounter" => order_state::<PnCounter>(ops, sequence, |s| {
            let n = s.value();
            let num = i64::try_from(n).map_err(|_| format!("pncounter value {n} out of i64"))?;
            Ok(serde_json::json!({ "value": num }))
        }),
        "orset" => order_state::<OrSet>(ops, sequence, |s| {
            let mut els: Vec<Json> = s
                .elements()
                .into_iter()
                .map(value_json)
                .collect::<Result<Vec<_>, _>>()?;
            els.sort_by_key(|a| a.to_string());
            Ok(serde_json::json!({ "elements": els }))
        }),
        "flag" => order_state::<Flag>(ops, sequence, |s| {
            Ok(serde_json::json!({ "enabled": s.enabled() }))
        }),
        other => Err(format!("unknown crdt {other}")),
    }
}

fn order_state<S: CrdtState>(
    ops: &[KernelOp],
    sequence: &[usize],
    read: impl FnOnce(&S) -> Result<Json, String>,
) -> Result<Json, String> {
    let mut replica = Replica::<S>::default();
    for &index in sequence {
        replica.ingest(&ops[index]);
    }
    let equivocation = replica.has_equivocation();
    match replica.state() {
        Ok(state) => Ok(serde_json::json!({
            "equivocation": equivocation,
            "error": Json::Null,
            "state": read(&state)?,
        })),
        Err(KernelError::BlobUnsupported) => Ok(serde_json::json!({
            "equivocation": equivocation,
            "error": "BLOB_UNSUPPORTED",
            "state": Json::Null,
        })),
        Err(e) => Err(e.to_string()),
    }
}

fn kernel_op(v: &Json, crdt: &str) -> Result<KernelOp, String> {
    Ok(KernelOp {
        op_id: id(v["op_id"].as_str().ok_or("op missing op_id")?),
        author: id(v["author"].as_str().ok_or("op missing author")?),
        physical_ms: v["ts"]["physical_ms"]
            .as_u64()
            .ok_or("op missing ts.physical_ms")?,
        logical: v["ts"]["logical"].as_u64().ok_or("op missing ts.logical")? as u16,
        payload: parse_payload(&v["payload"], crdt)?,
    })
}

fn id(s: &str) -> Id {
    s.as_bytes().to_vec()
}

fn value(v: &Json) -> Result<Value, String> {
    match v {
        Json::Null => Ok(Value::Null),
        Json::Bool(b) => Ok(Value::Bool(*b)),
        Json::Number(n) => Ok(Value::Int(
            n.as_i64().ok_or_else(|| format!("int value {n}"))?,
        )),
        Json::String(s) => Ok(Value::Text(s.clone())),
        Json::Object(o) if o.contains_key("blobref") => {
            let b = &v["blobref"];
            let s = b["hash"].as_str().ok_or("blobref.hash")?;
            if s.len() != 64 {
                return Err("blobref.hash must be 32-byte hex".into());
            }
            let mut hash = [0u8; 32];
            for i in 0..32 {
                hash[i] =
                    u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| "blobref.hash hex")?;
            }
            Ok(Value::BlobRef {
                hash,
                size: b["size"].as_u64().ok_or("blobref.size")?,
                codec: b["codec"].as_u64().ok_or("blobref.codec")? as u16,
            })
        }
        other => Err(format!("unsupported vector value {other}")),
    }
}

fn value_json(v: &Value) -> Result<Json, String> {
    match v {
        Value::Null => Ok(Json::Null),
        Value::Bool(b) => Ok(Json::Bool(*b)),
        Value::Int(i) => Ok(Json::Number((*i).into())),
        Value::Text(s) => Ok(Json::String(s.clone())),
        Value::Bytes(b) => Ok(Json::String(
            b.iter().map(|x| format!("{x:02x}")).collect::<String>(),
        )),
        Value::BlobRef { .. } => Err("BlobRef cannot be read as a materialized value".into()),
    }
}

fn observed(v: &Json) -> Result<Vec<Id>, String> {
    v.as_array()
        .ok_or("observed must be an array")?
        .iter()
        .map(|d| {
            d.as_str()
                .map(id)
                .ok_or_else(|| "observed dot must be a string".to_string())
        })
        .collect()
}

fn parse_payload(p: &Json, crdt: &str) -> Result<Payload, String> {
    let obj = p
        .as_object()
        .ok_or_else(|| format!("payload must be an object for {crdt}"))?;
    if let Some(v) = obj.get("set") {
        Ok(Payload::LwwSet(value(v)?))
    } else if let Some(n) = obj.get("inc") {
        Ok(Payload::CounterInc(
            n.as_u64().ok_or("inc must be a positive uint")?,
        ))
    } else if let Some(n) = obj.get("dec") {
        Ok(Payload::CounterDec(
            n.as_u64().ok_or("dec must be a positive uint")?,
        ))
    } else if let Some(v) = obj.get("add") {
        Ok(Payload::SetAdd(value(v)?))
    } else if let Some(v) = obj.get("remove") {
        Ok(Payload::SetRemove {
            element: value(v)?,
            observed: observed(&p["observed"])?,
        })
    } else if obj.contains_key("enable") {
        Ok(Payload::FlagEnable)
    } else if obj.contains_key("disable") {
        Ok(Payload::FlagDisable {
            observed: observed(&p["observed"])?,
        })
    } else {
        Err(format!("unknown payload for crdt {crdt}: {p}"))
    }
}
