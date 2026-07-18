//! Cross-package composite smoke (composite M0 gate).

use std::fs;
use std::path::PathBuf;

use serde_json::Value as Json;
use zerodb_core::delivery::{DelivStep, run_schedule};
use zerodb_core::frontier::is_late_against_ops;
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

#[test]
fn composite_vectors() {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors/required/composite");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    for path in files {
        let v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["type"], "composite-smoke");
        let o = &v["op"];
        let op = MerkleOp {
            op_id: arr32(o["op_id"].as_str().unwrap()),
            physical_ms: o["physical_ms"].as_u64().unwrap(),
            logical: o["logical"].as_u64().unwrap_or(0) as u16,
            author: arr32(o["author"].as_str().unwrap()),
        };
        let root = merkle_root(std::slice::from_ref(&op));
        assert_eq!(
            bytes_to_hex(&root),
            v["expect_merkle_root_hex"].as_str().unwrap(),
            "merkle {}",
            path.display()
        );
        let del = run_schedule(&[DelivStep::Deliver {
            op_id: op.op_id,
            reject: false,
        }]);
        assert_eq!(
            del.last_outcomes,
            vec![v["expect_delivery"].as_str().unwrap()],
            "delivery {}",
            path.display()
        );
        let late = is_late_against_ops(&op, std::slice::from_ref(&op));
        assert_eq!(
            late,
            v["expect_late"].as_bool().unwrap(),
            "late {}",
            path.display()
        );
    }
}
