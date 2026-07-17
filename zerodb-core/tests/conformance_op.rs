//! Rust side of the `op-encoding` / `op-decode-negative` conformance
//! vectors (doc/KERNEL.md §9).
//!
//! `op-encoding`: build the envelope from the vector's logical description,
//! encode canonically, compare bytes; derive OpId, compare. The JS runner
//! re-encodes the same description with an independent CBOR encoder —
//! byte agreement is the I-6 check.
//!
//! Run `cargo test --test conformance_op -- --ignored --nocapture` to print
//! the canonical/op-id hex for authoring new vectors.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::cbor::{Cbor, CborError, decode};
use zerodb_core::op::{OpEnvelope, OpTs};
use zerodb_core::sign::{sign_op, verify_op};

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn bytes_to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn fixed<const N: usize>(s: &str) -> [u8; N] {
    hex_to_bytes(s).try_into().expect("fixed-length hex")
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

fn envelope_from_json(op: &Json) -> OpEnvelope {
    OpEnvelope {
        v: op["v"].as_u64().unwrap(),
        ds: fixed::<32>(op["ds"].as_str().unwrap()),
        ep: op["ep"].as_u64().unwrap(),
        author: fixed::<32>(op["author"].as_str().unwrap()),
        ts: OpTs {
            physical_ms: op["ts"]["p"].as_u64().unwrap(),
            logical: op["ts"]["l"].as_u64().unwrap() as u16,
        },
        deps: op["deps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| fixed::<32>(d.as_str().unwrap()))
            .collect(),
        grp: match &op["grp"] {
            Json::Null => None,
            Json::String(s) => Some(fixed::<16>(s)),
            other => panic!("bad grp {other}"),
        },
        kind: op["kind"].as_u64().unwrap(),
        body: tagged_to_cbor(&op["body"]),
    }
}

fn vector_files(subdir: &str) -> Vec<PathBuf> {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut files = Vec::new();
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join(subdir);
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

fn error_name(e: &CborError) -> &'static str {
    match e {
        CborError::DuplicateKey => "DuplicateKey",
        CborError::KeyOrder => "KeyOrder",
        CborError::Depth => "Depth",
        CborError::NonCanonical => "NonCanonical",
        CborError::Unsupported => "Unsupported",
        CborError::Truncated => "Truncated",
        CborError::Trailing => "Trailing",
        CborError::Utf8 => "Utf8",
    }
}

#[test]
fn op_vectors() {
    let files = vector_files("op");
    assert!(!files.is_empty(), "no op vectors found");
    for path in files {
        let v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        match v["type"].as_str().unwrap() {
            "op-encoding" => check_encoding(&v, &path),
            "op-decode-negative" => check_negative(&v, &path),
            "op-signature" => check_signature(&v, &path),
            other => panic!("{path:?}: unknown vector type {other}"),
        }
    }
}

fn check_encoding(v: &Json, path: &Path) {
    let id = v["id"].as_str().unwrap();
    let envelope = envelope_from_json(&v["op"]);
    let bytes = envelope.id_preimage_body().unwrap();
    assert_eq!(
        bytes_to_hex(&bytes),
        v["canonical_hex"].as_str().unwrap(),
        "{id} ({path:?}): canonical bytes"
    );
    // Canonical means decode → re-encode is identity.
    let reencoded = zerodb_core::cbor::encode(&decode(&bytes).unwrap()).unwrap();
    assert_eq!(reencoded, bytes, "{id}: round trip");
    assert_eq!(
        bytes_to_hex(&envelope.op_id().unwrap()),
        v["op_id_hex"].as_str().unwrap(),
        "{id}: op id"
    );
}

fn check_negative(v: &Json, path: &Path) {
    let id = v["id"].as_str().unwrap();
    let raw = hex_to_bytes(v["raw_hex"].as_str().unwrap());
    match decode(&raw) {
        Ok(_) => panic!("{id} ({path:?}): expected decode error, got success"),
        Err(e) => assert_eq!(
            error_name(&e),
            v["expect_error"].as_str().unwrap(),
            "{id}: error kind"
        ),
    }
}

fn check_signature(v: &Json, path: &Path) {
    let id = v["id"].as_str().unwrap();
    let envelope = envelope_from_json(&v["op"]);
    let op_id = envelope.op_id().unwrap();
    assert_eq!(
        bytes_to_hex(&op_id),
        v["op_id_hex"].as_str().unwrap(),
        "{id} ({path:?}): op id"
    );

    let seed = fixed::<32>(v["seed_hex"].as_str().unwrap());
    let (pubkey, sig) = sign_op(&seed, &op_id);
    assert_eq!(
        bytes_to_hex(&pubkey),
        v["pub_hex"].as_str().unwrap(),
        "{id}: public key"
    );
    assert_eq!(
        bytes_to_hex(&sig),
        v["sig_hex"].as_str().unwrap(),
        "{id}: signature"
    );
    assert!(
        verify_op(&pubkey, &op_id, &sig),
        "{id}: golden signature must verify"
    );

    let mut bad_sig = sig;
    bad_sig[0] ^= 1;
    assert!(
        !verify_op(&pubkey, &op_id, &bad_sig),
        "{id}: tampered signature verified"
    );
    let mut bad_id = op_id;
    bad_id[0] ^= 1;
    assert!(
        !verify_op(&pubkey, &bad_id, &sig),
        "{id}: signature verified over tampered op id"
    );
}

/// Authoring helper: prints canonical/op-id hex for every op-encoding vector.
#[test]
#[ignore]
fn generate_op_vector_hex() {
    for path in vector_files("op") {
        let mut v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let kind = v["type"].as_str().unwrap().to_owned();
        if kind != "op-encoding" && kind != "op-signature" {
            continue;
        }
        if v["canonical_hex"] != "TBD" && v["op_id_hex"] != "TBD" && v["sig_hex"] != "TBD" {
            continue;
        }
        let envelope = envelope_from_json(&v["op"]);
        let bytes = envelope.id_preimage_body().unwrap();
        let op_id = envelope.op_id().unwrap();
        if kind == "op-encoding" {
            v["canonical_hex"] = Json::String(bytes_to_hex(&bytes));
        }
        v["op_id_hex"] = Json::String(bytes_to_hex(&op_id));
        if kind == "op-signature" {
            let seed = fixed::<32>(v["seed_hex"].as_str().unwrap());
            let (pubkey, sig) = sign_op(&seed, &op_id);
            v["pub_hex"] = Json::String(bytes_to_hex(&pubkey));
            v["sig_hex"] = Json::String(bytes_to_hex(&sig));
        }
        let mut pretty = serde_json::to_string_pretty(&v).unwrap();
        pretty.push('\n');
        fs::write(&path, pretty).unwrap();
        println!("filled {}", path.display());
    }
}
