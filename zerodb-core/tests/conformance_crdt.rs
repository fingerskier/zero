//! Rust side of the `crdt-apply` conformance vectors (doc/KERNEL.md §9).
//!
//! Author/op-id strings in vectors become raw UTF-8 bytes; bytewise
//! comparison then matches the JS runner's string comparison exactly
//! (ASCII), so both runners share one total order.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::kernel::{
    CrdtState, Flag, GCounter, Id, KernelOp, Lww, OrSet, Payload, PnCounter, Replica, Value,
};

fn id(s: &str) -> Id {
    s.as_bytes().to_vec()
}

fn kernel_op(v: &Json, crdt: &str) -> KernelOp {
    let payload = parse_payload(&v["payload"], crdt);
    KernelOp {
        op_id: id(v["op_id"].as_str().unwrap()),
        author: id(v["author"].as_str().unwrap()),
        physical_ms: v["ts"]["physical_ms"].as_u64().unwrap(),
        logical: v["ts"]["logical"].as_u64().unwrap() as u16,
        payload,
    }
}

fn value(v: &Json) -> Value {
    match v {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Int(n.as_i64().expect("int value")),
        Json::String(s) => Value::Text(s.clone()),
        other => panic!("unsupported vector value {other}"),
    }
}

fn observed(v: &Json) -> Vec<Id> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|d| id(d.as_str().unwrap()))
        .collect()
}

fn parse_payload(p: &Json, crdt: &str) -> Payload {
    let obj = p.as_object().unwrap();
    if let Some(v) = obj.get("set") {
        Payload::LwwSet(value(v))
    } else if let Some(n) = obj.get("inc") {
        Payload::CounterInc(n.as_u64().expect("positive inc"))
    } else if let Some(n) = obj.get("dec") {
        Payload::CounterDec(n.as_u64().expect("positive dec"))
    } else if let Some(v) = obj.get("add") {
        Payload::SetAdd(value(v))
    } else if let Some(v) = obj.get("remove") {
        Payload::SetRemove {
            element: value(v),
            observed: observed(&p["observed"]),
        }
    } else if obj.contains_key("enable") {
        Payload::FlagEnable
    } else if obj.contains_key("disable") {
        Payload::FlagDisable {
            observed: observed(&p["observed"]),
        }
    } else {
        panic!("unknown payload for crdt {crdt}: {p}");
    }
}

#[test]
fn crdt_apply_vectors() {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut ran = 0;
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("crdt");
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let vector: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            run_vector(&vector, &path);
            ran += 1;
        }
    }
    assert!(ran > 0, "no crdt-apply vectors found under {vectors:?}");
}

fn run_vector(v: &Json, path: &Path) {
    assert_eq!(v["type"], "crdt-apply", "wrong vector type in {path:?}");
    let vector_id = v["id"].as_str().unwrap();
    let crdt = v["crdt"].as_str().unwrap();
    let ops: Vec<KernelOp> = v["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| kernel_op(o, crdt))
        .collect();

    for (oi, order) in v["orders"].as_array().unwrap().iter().enumerate() {
        let sequence: Vec<usize> = order
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i.as_u64().unwrap() as usize)
            .collect();
        check_order(crdt, &ops, &sequence, &v["expect"], vector_id, oi);
    }
}

fn apply_all<S: CrdtState>(ops: &[KernelOp], sequence: &[usize]) -> Replica<S> {
    let mut replica = Replica::<S>::default();
    for &index in sequence {
        replica.apply(&ops[index]).expect("apply failed");
    }
    replica
}

fn check_order(
    crdt: &str,
    ops: &[KernelOp],
    sequence: &[usize],
    expect: &Json,
    vector_id: &str,
    oi: usize,
) {
    let ctx = format!("{vector_id} order {oi} {sequence:?}");
    match crdt {
        "lww" => {
            let replica = apply_all::<Lww>(ops, sequence);
            let got = replica.state.value().cloned();
            assert_eq!(got, Some(value(&expect["value"])), "{ctx}");
        }
        "gcounter" => {
            let replica = apply_all::<GCounter>(ops, sequence);
            assert_eq!(
                replica.state.value(),
                expect["value"].as_u64().unwrap(),
                "{ctx}"
            );
        }
        "pncounter" => {
            let replica = apply_all::<PnCounter>(ops, sequence);
            assert_eq!(
                replica.state.value(),
                expect["value"].as_i64().unwrap() as i128,
                "{ctx}"
            );
        }
        "orset" => {
            let replica = apply_all::<OrSet>(ops, sequence);
            let mut got: Vec<Value> = replica.state.elements().into_iter().cloned().collect();
            got.sort();
            let mut want: Vec<Value> = expect["elements"]
                .as_array()
                .unwrap()
                .iter()
                .map(value)
                .collect();
            want.sort();
            assert_eq!(got, want, "{ctx}");
        }
        "flag" => {
            let replica = apply_all::<Flag>(ops, sequence);
            assert_eq!(
                replica.state.enabled(),
                expect["enabled"].as_bool().unwrap(),
                "{ctx}"
            );
        }
        other => panic!("{ctx}: unknown crdt {other}"),
    }
}
