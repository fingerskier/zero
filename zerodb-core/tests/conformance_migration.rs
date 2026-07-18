//! Rust side of the `migration-transform` conformance vectors (doc/SCHEMA.md
//! §4/§6). Each case feeds a source materialized state through one registry
//! transform in isolation and requires the same output lww value as the JS
//! runner. Reuses `epoch::transform_value` — the exact function the epoch model
//! applies at a change_crdt boundary.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::epoch::{Read, transform_value};
use zerodb_core::kernel::Value;

fn value(v: &Json) -> Value {
    match v {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => Value::Int(n.as_i64().expect("int value")),
        Json::String(s) => Value::Text(s.clone()),
        other => panic!("unsupported vector value {other}"),
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

fn input_read(v: &Json) -> Read {
    let val = &v["value"];
    match v["crdt"].as_str().unwrap() {
        "lww" => Read::Lww(if val.is_null() {
            None
        } else {
            Some(value(val))
        }),
        "gcounter" => Read::GCounter(val.as_u64().unwrap()),
        "pncounter" => Read::PnCounter(val.as_i64().unwrap() as i128),
        "flag" => Read::Flag(val.as_bool().unwrap()),
        "orset" => Read::OrSet(vec![]),
        other => panic!("unknown input crdt {other}"),
    }
}

fn vector_files() -> Vec<PathBuf> {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut files = Vec::new();
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("migration");
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
fn migration_transform_vectors() {
    for path in vector_files() {
        let v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            v["type"], "migration-transform",
            "wrong vector type in {path:?}"
        );
        run_vector(&v, &path);
    }
}

fn run_vector(v: &Json, path: &Path) {
    let vid = v["id"].as_str().unwrap();
    let transform = v["transform"].as_str().unwrap();
    let default = v.get("default").filter(|d| !d.is_null()).map(value);
    for (ci, case) in v["cases"].as_array().unwrap().iter().enumerate() {
        let read = input_read(&case["input"]);
        let got = transform_value(transform, default.as_ref(), &read)
            .unwrap_or_else(|e| panic!("{vid} case {ci} ({path:?}): {}", e.name()));
        assert!(
            value_matches(&got, &case["output"]["value"]),
            "{vid} case {ci}: {got:?} != expect {}",
            case["output"]["value"]
        );
    }
}
