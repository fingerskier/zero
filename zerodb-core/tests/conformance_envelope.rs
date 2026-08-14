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
use zerodb_core::cbor::Cbor;
use zerodb_core::envelope::{EnvelopeError, ValueContext, open, seal};
use zerodb_core::op::{OpEnvelope, OpTs};

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
        author: fixed::<32>(v["author"].as_str().unwrap()),
        physical_ms: v["physical_ms"].as_u64().unwrap(),
        logical: v["logical"].as_u64().unwrap_or(0) as u16,
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
    c.author[0] ^= 1;
    assert_eq!(
        open(&key, &envelope, &c),
        Err(EnvelopeError::DecryptFailed),
        "{id}: author flip"
    );
    let mut c = ctx.clone();
    c.physical_ms ^= 1;
    assert_eq!(
        open(&key, &envelope, &c),
        Err(EnvelopeError::DecryptFailed),
        "{id}: physical_ms flip"
    );
    let mut c = ctx.clone();
    c.logical ^= 1;
    assert_eq!(
        open(&key, &envelope, &c),
        Err(EnvelopeError::DecryptFailed),
        "{id}: logical flip"
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

    // Complete-operation construction (CX-03): OpId is hashed *after* seal.
    if let Some(want_id) = v["expect_op_id_hex"].as_str() {
        let op = OpEnvelope {
            v: 1,
            ds: ctx.ds,
            ep: ctx.ep,
            author: ctx.author,
            ts: OpTs {
                physical_ms: ctx.physical_ms,
                logical: ctx.logical,
            },
            deps: vec![],
            grp: None,
            kind: 3,
            body: Cbor::Map(vec![
                ("crdt".into(), Cbor::Text("lww".into())),
                ("encrypted".into(), Cbor::Bytes(envelope.clone())),
                ("path".into(), Cbor::Text(ctx.path.clone())),
            ]),
        };
        assert_eq!(bytes_to_hex(&op.op_id().unwrap()), want_id, "{id}: op_id");
        assert_eq!(
            open(&key, &envelope, &ctx).unwrap(),
            plaintext,
            "{id}: decrypt after OpId"
        );
    }
}

/// Authoring helper: fills TBD envelope_hex fields in place.
#[test]
#[ignore]
fn generate_envelope_hex() {
    for path in vector_files() {
        let mut v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let need_env = v["envelope_hex"] == "TBD";
        let need_id = v.get("expect_op_id_hex").map(|x| x == "TBD").unwrap_or(false);
        if !need_env && !need_id {
            continue;
        }
        let key = fixed::<32>(v["key_hex"].as_str().unwrap());
        let nonce = fixed::<24>(v["nonce_hex"].as_str().unwrap());
        let ctx = context(&v);
        let plaintext = hex_to_bytes(v["plaintext_hex"].as_str().unwrap());
        v["envelope_hex"] = Json::String(bytes_to_hex(&seal(&key, &nonce, &ctx, &plaintext)));
        if v.get("expect_op_id_hex").is_some() {
            let env = hex_to_bytes(v["envelope_hex"].as_str().unwrap());
            let op = OpEnvelope {
                v: 1,
                ds: ctx.ds,
                ep: ctx.ep,
                author: ctx.author,
                ts: OpTs {
                    physical_ms: ctx.physical_ms,
                    logical: ctx.logical,
                },
                deps: vec![],
                grp: None,
                kind: 3,
                body: Cbor::Map(vec![
                    ("crdt".into(), Cbor::Text("lww".into())),
                    ("encrypted".into(), Cbor::Bytes(env)),
                    ("path".into(), Cbor::Text(ctx.path.clone())),
                ]),
            };
            v["expect_op_id_hex"] = Json::String(bytes_to_hex(&op.op_id().unwrap()));
        }
        let mut pretty = serde_json::to_string_pretty(&v).unwrap();
        pretty.push('\n');
        fs::write(&path, pretty).unwrap();
        println!("filled {}", path.display());
    }
}
