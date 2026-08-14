//! Rust side of `relay-transcript` vectors (doc/RELAY-SPEC.md 0.2.2-draft).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::merkle::MerkleOp;
use zerodb_core::relay::{
    negotiate_capabilities, peer_id_from_pk, retransmit, root_hex, sign_auth, verify_auth,
    FrontierTip, HeldOp, ERR_AUTH_FAILED,
};

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

fn arr64(s: &str) -> [u8; 64] {
    hex_to_bytes(s).try_into().expect("64 bytes")
}

fn parse_merkle_op(v: &Json) -> MerkleOp {
    MerkleOp {
        op_id: arr32(v["op_id"].as_str().unwrap()),
        physical_ms: v["physical_ms"].as_u64().unwrap(),
        logical: v["logical"].as_u64().unwrap_or(0) as u16,
        author: arr32(v["author"].as_str().unwrap()),
    }
}

fn parse_held(v: &Json) -> HeldOp {
    HeldOp {
        op_id: v["op_id"].as_str().unwrap().to_string(),
        author: v["author"].as_str().unwrap().to_string(),
        physical_ms: v["physical_ms"].as_u64().unwrap(),
        logical: v["logical"].as_u64().unwrap_or(0) as u16,
    }
}

fn parse_frontier(v: &Json) -> Vec<FrontierTip> {
    let Some(map) = v["frontier"].as_object() else {
        return Vec::new();
    };
    map.iter()
        .map(|(author, tip)| FrontierTip {
            author: author.clone(),
            op_id: tip["op_id"].as_str().unwrap().to_string(),
            physical_ms: tip["physical_ms"].as_u64().unwrap(),
            logical: tip["logical"].as_u64().unwrap_or(0) as u16,
        })
        .collect()
}

fn strs(v: &Json) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect()
}

#[test]
fn relay_transcript_vectors() {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut ran = 0;
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("relay");
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let vector: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            check_vector(&vector, &path);
            ran += 1;
        }
    }
    assert!(ran > 0, "no relay-transcript vectors under {vectors:?}");
}

fn check_vector(v: &Json, path: &Path) {
    assert_eq!(v["type"], "relay-transcript", "{}", path.display());
    match v["kind"].as_str().unwrap() {
        "handshake" => check_handshake(v, path),
        "dual-root" => check_dual_root(v, path),
        "resume" => check_resume(v, path),
        "reject-ack" => check_reject(v, path),
        other => panic!("{}: unknown kind {other}", path.display()),
    }
}

fn check_handshake(v: &Json, path: &Path) {
    let pk = arr32(v["public_key"].as_str().unwrap());
    let seed = arr32(v["secret_key"].as_str().unwrap());
    let nonce = arr32(v["nonce"].as_str().unwrap());
    let pid = bytes_to_hex(&peer_id_from_pk(&pk));
    let honest = sign_auth(&seed, &nonce);
    let sig = if let Some(s) = v["auth_signature"].as_str() {
        arr64(s)
    } else {
        honest
    };
    let auth_ok = verify_auth(&pk, &nonce, &sig);
    let expect = &v["expect"];
    assert_eq!(
        auth_ok,
        expect["auth_ok"].as_bool().unwrap(),
        "{} auth_ok",
        path.display()
    );
    if !auth_ok {
        assert_eq!(
            expect["error_code"].as_u64().unwrap(),
            ERR_AUTH_FAILED as u64,
            "{} error_code",
            path.display()
        );
        return;
    }
    assert_eq!(
        pid,
        expect["peer_id"].as_str().unwrap(),
        "{}",
        path.display()
    );
    let hello = strs(&v["hello_capabilities"]);
    let relay = strs(&v["relay_capabilities"]);
    let caps = negotiate_capabilities(&hello, &relay);
    let want = strs(&expect["welcome_capabilities"]);
    assert_eq!(caps, want, "{} capabilities", path.display());
    if let Some(want_sig) = expect["signature"].as_str() {
        assert_eq!(
            bytes_to_hex(&honest),
            want_sig,
            "{} signature",
            path.display()
        );
    }
}

fn check_dual_root(v: &Json, path: &Path) {
    let validated: Vec<MerkleOp> = v["validated"]
        .as_array()
        .unwrap()
        .iter()
        .map(parse_merkle_op)
        .collect();
    let a: Vec<MerkleOp> = v["accepted_a"]
        .as_array()
        .unwrap()
        .iter()
        .map(parse_merkle_op)
        .collect();
    let b: Vec<MerkleOp> = v["accepted_b"]
        .as_array()
        .unwrap()
        .iter()
        .map(parse_merkle_op)
        .collect();
    let vr = root_hex(&validated);
    let ar = root_hex(&a);
    let br = root_hex(&b);
    assert_eq!(
        vr == ar,
        v["expect"]["roots_equal"].as_bool().unwrap(),
        "{} roots_equal validated={vr} accepted={ar}",
        path.display()
    );
    assert_eq!(
        ar == br,
        v["expect"]["peer_accepted_equal"].as_bool().unwrap(),
        "{} peer_accepted_equal a={ar} b={br}",
        path.display()
    );
}

fn check_resume(v: &Json, path: &Path) {
    let held: Vec<HeldOp> = v["held"]
        .as_array()
        .unwrap()
        .iter()
        .map(parse_held)
        .collect();
    let frontier = parse_frontier(&v["cursor"]);
    let rejected: Vec<String> = v
        .get("rejected")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
        .unwrap_or_default();
    let mut got = retransmit(&held, &frontier, &rejected);
    let mut want: Vec<String> = v["expect"]["retransmit"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    got.sort();
    want.sort();
    assert_eq!(got, want, "{}", path.display());
}

fn check_reject(v: &Json, path: &Path) {
    let held: Vec<HeldOp> = v["held"]
        .as_array()
        .unwrap()
        .iter()
        .map(parse_held)
        .collect();
    let frontier = parse_frontier(&v["cursor"]);
    let rejected: Vec<String> = v["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["outcome"] == "REJECT")
        .map(|o| o["op_id"].as_str().unwrap().to_string())
        .collect();
    let mut got = retransmit(&held, &frontier, &rejected);
    let mut want: Vec<String> = v["expect"]["retransmit"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    got.sort();
    want.sort();
    assert_eq!(got, want, "{}", path.display());
}
