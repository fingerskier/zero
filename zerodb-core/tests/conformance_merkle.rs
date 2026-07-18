//! Rust side of merkle-root + merkle-transcript vectors (doc/MERKLE.md).

use std::fs;
use std::path::PathBuf;

use serde_json::Value as Json;
use zerodb_core::merkle::{MerkleMsg, MerkleOp, merkle_root, sync_transcript};

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn bytes_to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn arr32(s: &str) -> [u8; 32] {
    hex_to_bytes(s).try_into().expect("32 bytes")
}

fn parse_op(v: &Json) -> MerkleOp {
    MerkleOp {
        op_id: arr32(v["op_id"].as_str().unwrap()),
        physical_ms: v["physical_ms"].as_u64().unwrap(),
        logical: v["logical"].as_u64().unwrap_or(0) as u16,
        author: arr32(v["author"].as_str().unwrap()),
    }
}

fn vector_files() -> Vec<PathBuf> {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut files = Vec::new();
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("merkle");
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
    files.sort();
    files
}

fn msg_matches(got: &MerkleMsg, exp: &Json) -> bool {
    let t = exp["type"].as_str().unwrap();
    if got.type_name() != t {
        return false;
    }
    match got {
        MerkleMsg::RootOffer { from, root } => exp["from"] == *from && exp["root"] == *root,
        MerkleMsg::NodeRequest { from, level, index } => {
            exp["from"] == *from && exp["level"] == *level as u64 && exp["index"] == *index as u64
        }
        MerkleMsg::NodeResponse {
            from,
            level,
            index,
            hash,
            left,
            right,
        } => {
            exp["from"] == *from
                && exp["level"] == *level as u64
                && exp["index"] == *index as u64
                && exp["hash"] == *hash
                && exp["left"] == *left
                && exp["right"] == *right
        }
        MerkleMsg::LeafRequest {
            from,
            leaf_index,
            bucket_index,
        } => {
            exp["from"] == *from
                && exp["leaf_index"] == *leaf_index as u64
                && match (bucket_index, &exp["bucket_index"]) {
                    (None, Json::Null) => true,
                    (Some(b), j) => j.as_u64() == Some(*b),
                    _ => false,
                }
        }
        MerkleMsg::LeafResponse {
            from,
            leaf_index,
            bucket_index,
            op_ids,
        } => {
            if exp["from"] != *from || exp["leaf_index"] != *leaf_index as u64 {
                return false;
            }
            let bi_ok = match (bucket_index, &exp["bucket_index"]) {
                (None, Json::Null) => true,
                (Some(b), j) => j.as_u64() == Some(*b),
                _ => false,
            };
            if !bi_ok {
                return false;
            }
            let exp_ids: Vec<&str> = exp["op_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap())
                .collect();
            op_ids.iter().map(|s| s.as_str()).collect::<Vec<_>>() == exp_ids
        }
        MerkleMsg::Delta { from, op_ids } => {
            if exp["from"] != *from {
                return false;
            }
            let exp_ids: Vec<&str> = exp["op_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap())
                .collect();
            op_ids.iter().map(|s| s.as_str()).collect::<Vec<_>>() == exp_ids
        }
        MerkleMsg::Converged { root } => exp["root"] == *root,
    }
}

#[test]
fn merkle_vectors() {
    let files = vector_files();
    assert!(!files.is_empty(), "no merkle vectors");
    for path in files {
        let raw = fs::read_to_string(&path).unwrap();
        let v: Json = serde_json::from_str(&raw).unwrap();
        match v["type"].as_str().unwrap() {
            "merkle-root" => {
                let ops: Vec<MerkleOp> =
                    v["ops"].as_array().unwrap().iter().map(parse_op).collect();
                let root = merkle_root(&ops);
                assert_eq!(
                    bytes_to_hex(&root),
                    v["root_hex"].as_str().unwrap(),
                    "{}",
                    path.display()
                );
                if let Some(perm) = v["ops_permuted"].as_array() {
                    let ops2: Vec<MerkleOp> = perm.iter().map(parse_op).collect();
                    assert_eq!(
                        bytes_to_hex(&merkle_root(&ops2)),
                        v["root_hex"].as_str().unwrap(),
                        "permuted {}",
                        path.display()
                    );
                }
            }
            "merkle-transcript" => {
                let a: Vec<MerkleOp> = v["peer_a"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(parse_op)
                    .collect();
                let b: Vec<MerkleOp> = v["peer_b"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(parse_op)
                    .collect();
                let got = sync_transcript(&a, &b);
                let exp = v["transcript"].as_array().unwrap();
                assert_eq!(got.len(), exp.len(), "transcript len {}", path.display());
                for (i, (g, e)) in got.iter().zip(exp.iter()).enumerate() {
                    assert!(
                        msg_matches(g, e),
                        "msg {i} mismatch {}\n got={:?}\n exp={}",
                        path.display(),
                        g,
                        e
                    );
                }
                if let Some(fr) = v["final_root_hex"].as_str() {
                    match got.last() {
                        Some(MerkleMsg::Converged { root }) => {
                            assert_eq!(root, fr, "final root {}", path.display())
                        }
                        _ => panic!("{}: last msg not Converged", path.display()),
                    }
                }
            }
            other => panic!("{}: unknown type {other}", path.display()),
        }
    }
}
