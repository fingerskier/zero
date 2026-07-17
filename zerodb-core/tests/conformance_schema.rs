//! Rust side of the `schema-ir` conformance vectors (doc/SCHEMA.md §2/§6).
//!
//! Encodes the vector's tagged IR description with the canonical CBOR
//! codec, requires byte agreement with canonical_hex plus round-trip
//! identity, and derives `SchemaId = BLAKE3(domain ‖ bytes)`. Structural
//! IR validation (unknown keys, `unique` smuggling, encrypted-on-counter)
//! lands with the IR validator and its negative vectors.
//!
//! Run with `-- --ignored` to fill TBD hex fields in place.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::cbor::{Cbor, decode, encode};

const DOMAIN_SCHEMA_IR: &[u8] = b"zerodb-schema-ir-v1";

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn bytes_to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn tagged_to_cbor(v: &Json) -> Cbor {
    match v["t"].as_str().unwrap() {
        "null" => Cbor::Null,
        "bool" => Cbor::Bool(v["v"].as_bool().unwrap()),
        "uint" => Cbor::Uint(v["v"].as_u64().unwrap()),
        "text" => Cbor::Text(v["v"].as_str().unwrap().to_owned()),
        "bytes" => Cbor::Bytes(hex_to_bytes(v["hex"].as_str().unwrap())),
        "array" => Cbor::Array(
            v["v"]
                .as_array()
                .unwrap()
                .iter()
                .map(tagged_to_cbor)
                .collect(),
        ),
        "map" => Cbor::Map(
            v["v"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, tv)| (k.clone(), tagged_to_cbor(tv)))
                .collect(),
        ),
        other => panic!("unknown tagged type {other}"),
    }
}

fn schema_id(ir_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN_SCHEMA_IR);
    hasher.update(ir_bytes);
    *hasher.finalize().as_bytes()
}

fn vector_files() -> Vec<PathBuf> {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut files = Vec::new();
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("schema");
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
fn schema_ir_vectors() {
    let files = vector_files();
    assert!(!files.is_empty(), "no schema-ir vectors found");
    for path in files {
        let v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        check_vector(&v, &path);
    }
}

fn check_vector(v: &Json, path: &Path) {
    assert_eq!(v["type"], "schema-ir", "wrong vector type in {path:?}");
    let id = v["id"].as_str().unwrap();
    let bytes = encode(&tagged_to_cbor(&v["ir"])).unwrap();
    assert_eq!(
        bytes_to_hex(&bytes),
        v["canonical_hex"].as_str().unwrap(),
        "{id} ({path:?}): canonical IR bytes"
    );
    let reencoded = encode(&decode(&bytes).unwrap()).unwrap();
    assert_eq!(reencoded, bytes, "{id}: round trip");
    assert_eq!(
        bytes_to_hex(&schema_id(&bytes)),
        v["schema_id_hex"].as_str().unwrap(),
        "{id}: schema id"
    );
}

/// Authoring helper: fills TBD hex fields in place.
#[test]
#[ignore]
fn generate_schema_hex() {
    for path in vector_files() {
        let mut v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if v["canonical_hex"] != "TBD" && v["schema_id_hex"] != "TBD" {
            continue;
        }
        let bytes = encode(&tagged_to_cbor(&v["ir"])).unwrap();
        v["canonical_hex"] = Json::String(bytes_to_hex(&bytes));
        v["schema_id_hex"] = Json::String(bytes_to_hex(&schema_id(&bytes)));
        let mut pretty = serde_json::to_string_pretty(&v).unwrap();
        pretty.push('\n');
        fs::write(&path, pretty).unwrap();
        println!("filled {}", path.display());
    }
}
