//! Rust side of the `epoch-replay` conformance vectors (doc/SCHEMA.md §4.1).
//!
//! Parses the vector into the `epoch` model's records/ops and materializes one
//! property, requiring the same read, quarantine set, and error outcome as the
//! JS runner for every ingestion order.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::epoch::{EpochOp, EpochRecord, Migration, Read, materialize};
use zerodb_core::kernel::{Id, KernelOp, OrderKey, Payload, Value};

fn id(s: &str) -> Id {
    s.as_bytes().to_vec()
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

fn order_key(v: &Json) -> OrderKey {
    OrderKey {
        physical_ms: v["physical_ms"].as_u64().unwrap(),
        logical: v["logical"].as_u64().unwrap() as u16,
        author: id(v["author"].as_str().unwrap()),
        op_id: id(v["op_id"].as_str().unwrap()),
    }
}

fn migration(v: &Json) -> Migration {
    Migration {
        transform: v["transform"].as_str().unwrap().to_owned(),
        default: v.get("default").map(value),
    }
}

fn epoch_record(v: &Json) -> EpochRecord {
    EpochRecord {
        record: v["record"].as_str().unwrap().to_owned(),
        epoch: v["epoch"].as_u64().unwrap(),
        prev: v["prev"].as_str().map(|s| s.to_owned()),
        order_key: order_key(&v["order_key"]),
        crdt: v["crdt"].as_str().unwrap().to_owned(),
        migration: v.get("migration").filter(|m| !m.is_null()).map(migration),
    }
}

fn payload(p: &Json) -> Payload {
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
            observed: p["observed"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| id(d.as_str().unwrap()))
                .collect(),
        }
    } else if obj.contains_key("enable") {
        Payload::FlagEnable
    } else if obj.contains_key("disable") {
        Payload::FlagDisable {
            observed: p["observed"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| id(d.as_str().unwrap()))
                .collect(),
        }
    } else {
        panic!("unknown payload {p}");
    }
}

fn epoch_op(v: &Json) -> EpochOp {
    EpochOp {
        op: KernelOp {
            op_id: id(v["op_id"].as_str().unwrap()),
            author: id(v["author"].as_str().unwrap()),
            physical_ms: v["ts"]["physical_ms"].as_u64().unwrap(),
            logical: v["ts"]["logical"].as_u64().unwrap() as u16,
            payload: payload(&v["payload"]),
        },
        ep: v["ep"].as_u64().unwrap(),
        ep_record: v["ep_record"].as_str().map(|s| s.to_owned()),
    }
}

fn read_matches(read: &Read, expect: &Json) -> bool {
    let want = &expect["value"];
    match read {
        Read::Lww(Some(v)) => value_matches(v, want),
        Read::Lww(None) => want.is_null(),
        Read::GCounter(n) => want.as_u64() == Some(*n),
        Read::PnCounter(n) => want.as_i64().map(|x| x as i128) == Some(*n),
        Read::Flag(b) => want.as_bool() == Some(*b),
        Read::OrSet(els) => {
            let Some(arr) = expect["elements"].as_array() else {
                return false;
            };
            let mut want: Vec<Value> = arr.iter().map(value).collect();
            want.sort();
            *els == want
        }
    }
}

fn value_matches(v: &Value, want: &Json) -> bool {
    match v {
        Value::Null => want.is_null(),
        Value::Bool(b) => want.as_bool() == Some(*b),
        Value::Int(i) => want.as_i64() == Some(*i),
        Value::Text(s) => want.as_str() == Some(s.as_str()),
        Value::Bytes(_) | Value::BlobRef { .. } => false,
    }
}

fn vector_files() -> Vec<PathBuf> {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut files = Vec::new();
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("epoch");
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                files.push(path);
            }
        }
    }
    files
}

#[test]
fn epoch_replay_vectors() {
    let files = vector_files();
    for path in &files {
        let v: Json = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(v["type"], "epoch-replay", "wrong vector type in {path:?}");
        run_vector(&v, path);
    }
}

fn run_vector(v: &Json, path: &Path) {
    let vid = v["id"].as_str().unwrap();
    let records: Vec<EpochRecord> = v["epochs"]
        .as_array()
        .unwrap()
        .iter()
        .map(epoch_record)
        .collect();
    let ops: Vec<EpochOp> = v["ops"].as_array().unwrap().iter().map(epoch_op).collect();

    let mut expect_q: Vec<String> = v["expect_quarantine"]
        .as_array()
        .map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect())
        .unwrap_or_default();
    expect_q.sort();

    for (oi, order) in v["orders"].as_array().unwrap().iter().enumerate() {
        let ctx = format!("{vid} order {oi} ({path:?})");
        let sequence: Vec<EpochOp> = order
            .as_array()
            .unwrap()
            .iter()
            .map(|i| ops[i.as_u64().unwrap() as usize].clone())
            .collect();

        let result = materialize(&sequence, &records);
        match v["expect_error"].as_str() {
            Some(want) => {
                let err = result
                    .err()
                    .unwrap_or_else(|| panic!("{ctx}: expected error {want}, got success"));
                assert_eq!(err.name(), want, "{ctx}: error kind");
            }
            None => {
                let (read, mut got_q) =
                    result.unwrap_or_else(|e| panic!("{ctx}: unexpected error {}", e.name()));
                assert!(
                    read_matches(&read, &v["expect"]),
                    "{ctx}: read {read:?} != expect {}",
                    v["expect"]
                );
                got_q.sort();
                assert_eq!(got_q, expect_q, "{ctx}: quarantine set");
            }
        }
    }
}
