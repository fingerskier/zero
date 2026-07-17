//! Rust side of the `envelope` conformance vectors (doc/KERNEL.md §7/§9).
//!
//! Positive: seal with the vector's fixed key/nonce/context and require
//! byte agreement with envelope_hex; open and require the plaintext.
//! Negatives are handler logic (like op-signature tamper checks): every
//! AAD component flipped, truncation, unknown version, and a flipped
//! key id must each fail with the named outcome (I-10).
//!
//! Run with `-- --ignored` to fill TBD envelope_hex fields in place.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::envelope::{EnvelopeError, ValueContext, open, seal};

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

fn context(v: &Json) -> ValueContext {
    ValueContext {
        ds: fixed::<32>(v["ds"].as_str().unwrap()),
        op_id: fixed::<32>(v["op_id"].as_str().unwrap()),
        ep: v["ep"].as_u64().unwrap(),
        path: v["path"].as_str().unwrap().to_owned(),
    }
}

fn vector_files() -> Vec<PathBuf> {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut files = Vec::new();
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("envelope");
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
fn envelope_vectors() {
    let files = vector_files();
    assert!(!files.is_empty(), "no envelope vectors found");
    for path in files {
        let v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        check_vector(&v, &path);
    }
}

fn check_vector(v: &Json, path: &Path) {
    assert_eq!(v["type"], "envelope", "wrong vector type in {path:?}");
    let id = v["id"].as_str().unwrap();
    let key = fixed::<32>(v["key_hex"].as_str().unwrap());
    let nonce = fixed::<24>(v["nonce_hex"].as_str().unwrap());
    let ctx = context(v);
    let plaintext = hex_to_bytes(v["plaintext_hex"].as_str().unwrap());

    let envelope = seal(&key, &nonce, &ctx, &plaintext);
    assert_eq!(
        bytes_to_hex(&envelope),
        v["envelope_hex"].as_str().unwrap(),
        "{id} ({path:?}): envelope bytes"
    );
    assert_eq!(
        open(&key, &envelope, &ctx).unwrap(),
        plaintext,
        "{id}: open"
    );

    // AAD binding negatives — every component (I-10).
    let mut c = ctx.clone();
    c.ds[0] ^= 1;
    assert_eq!(
        open(&key, &envelope, &c),
        Err(EnvelopeError::DecryptFailed),
        "{id}: ds flip"
    );
    let mut c = ctx.clone();
    c.op_id[0] ^= 1;
    assert_eq!(
        open(&key, &envelope, &c),
        Err(EnvelopeError::DecryptFailed),
        "{id}: op_id flip"
    );
    let mut c = ctx.clone();
    c.ep += 1;
    assert_eq!(
        open(&key, &envelope, &c),
        Err(EnvelopeError::DecryptFailed),
        "{id}: ep flip"
    );
    let mut c = ctx.clone();
    c.path.push('x');
    assert_eq!(
        open(&key, &envelope, &c),
        Err(EnvelopeError::DecryptFailed),
        "{id}: path flip"
    );

    // Header negatives.
    let mut bad = envelope.clone();
    bad[0] = 2;
    assert_eq!(
        open(&key, &bad, &ctx),
        Err(EnvelopeError::UnknownVersion),
        "{id}: version"
    );
    let mut bad = envelope.clone();
    bad[1] ^= 1;
    assert_eq!(
        open(&key, &bad, &ctx),
        Err(EnvelopeError::UnknownKeyId),
        "{id}: key id"
    );
    assert_eq!(
        open(&key, &envelope[..40], &ctx),
        Err(EnvelopeError::Truncated),
        "{id}: truncated"
    );
    let mut bad = envelope.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    assert_eq!(
        open(&key, &bad, &ctx),
        Err(EnvelopeError::DecryptFailed),
        "{id}: tag flip"
    );
}

/// Authoring helper: fills TBD envelope_hex fields in place.
#[test]
#[ignore]
fn generate_envelope_hex() {
    for path in vector_files() {
        let mut v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if v["envelope_hex"] != "TBD" {
            continue;
        }
        let key = fixed::<32>(v["key_hex"].as_str().unwrap());
        let nonce = fixed::<24>(v["nonce_hex"].as_str().unwrap());
        let ctx = context(&v);
        let plaintext = hex_to_bytes(v["plaintext_hex"].as_str().unwrap());
        v["envelope_hex"] = Json::String(bytes_to_hex(&seal(&key, &nonce, &ctx, &plaintext)));
        let mut pretty = serde_json::to_string_pretty(&v).unwrap();
        pretty.push('\n');
        fs::write(&path, pretty).unwrap();
        println!("filled {}", path.display());
    }
}
