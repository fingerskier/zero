//! frontier-build / snapshot-id / late-op vectors (doc/FRONTIER.md).

use std::fs;
use std::path::PathBuf;

use serde_json::Value as Json;
use zerodb_core::frontier::{
    build_frontier, is_late_against_ops, is_late_op, snapshot_for_ops, tail_boundary,
};
use zerodb_core::merkle::{MerkleOp, merkle_root};

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn arr32(s: &str) -> [u8; 32] {
    hex_to_bytes(s).try_into().expect("32")
}

fn bytes_to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn parse_op(v: &Json) -> MerkleOp {
    MerkleOp {
        op_id: arr32(v["op_id"].as_str().unwrap()),
        physical_ms: v["physical_ms"].as_u64().unwrap(),
        logical: v["logical"].as_u64().unwrap_or(0) as u16,
        author: arr32(v["author"].as_str().unwrap()),
    }
}

#[test]
fn frontier_vectors() {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors/required/frontier");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    for path in files {
        let v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        match v["type"].as_str().unwrap() {
            "frontier-build" => {
                let ops: Vec<_> = v["ops"].as_array().unwrap().iter().map(parse_op).collect();
                let f = build_frontier(&ops);
                let exp = v["expect_frontier"].as_object().unwrap();
                assert_eq!(f.len(), exp.len(), "{}", path.display());
                for (k, val) in exp {
                    let author = arr32(k);
                    let want = arr32(val.as_str().unwrap());
                    assert_eq!(
                        f.get(&author).map(|t| t.op_id),
                        Some(want),
                        "{}",
                        path.display()
                    );
                }
            }
            "snapshot-id" => {
                let ops: Vec<_> = v["ops"].as_array().unwrap().iter().map(parse_op).collect();
                let ds = arr32(v["datastore_id_hex"].as_str().unwrap());
                let id = snapshot_for_ops(&ds, &ops);
                assert_eq!(
                    bytes_to_hex(&id),
                    v["expect_snapshot_id_hex"].as_str().unwrap(),
                    "{}",
                    path.display()
                );
                if let Some(mr) = v["expect_merkle_root_hex"].as_str() {
                    assert_eq!(bytes_to_hex(&merkle_root(&ops)), mr, "{}", path.display());
                }
                let _ = tail_boundary(&ops);
            }
            "late-op" => {
                let op = parse_op(&v["op"]);
                let late = if let Some(enc) = v.get("encoded_frontier") {
                    // CX-04: decide late-op from the encoded map alone.
                    let mut f = zerodb_core::frontier::Frontier::new();
                    for (k, val) in enc.as_object().unwrap() {
                        f.insert(
                            arr32(k),
                            zerodb_core::frontier::FrontierTip {
                                op_id: arr32(val["op_id"].as_str().unwrap()),
                                physical_ms: val["physical_ms"].as_u64().unwrap(),
                                logical: val["logical"].as_u64().unwrap_or(0) as u16,
                            },
                        );
                    }
                    is_late_op(&op, &f)
                } else {
                    let base: Vec<_> = v["frontier_ops"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(parse_op)
                        .collect();
                    is_late_against_ops(&op, &base)
                };
                assert_eq!(late, v["expect_late"].as_bool().unwrap(), "{}", path.display());
            }
            other => panic!("unknown {other}"),
        }
    }
}
