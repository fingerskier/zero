//! Rust side of `relay-transcript` vectors (doc/RELAY-SPEC.md 0.2.2-draft).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::merkle::MerkleOp;
use zerodb_core::relay::{
    DIR_PEER_TO_RELAY, DIR_RELAY_TO_PEER, ERR_AUTH_FAILED, FrontierTip, HeldOp, MSG_AUTH,
    MSG_CHALLENGE, MSG_ERROR, MSG_HELLO, MSG_OP_ACK, MSG_OPS, MSG_SYNC_REQUEST, MSG_SYNC_RESPONSE,
    MSG_WELCOME, authenticate, expected_response_types, fixed_direction, is_request, is_response,
    known_message_type, negotiate_capabilities, peer_id_from_pk, required_payload_keys,
    required_sync_root, retransmit, root_hex, sign_auth,
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
    check_frames(v, path);
}

fn check_handshake(v: &Json, path: &Path) {
    let pk = arr32(v["public_key"].as_str().unwrap());
    let seed = arr32(v["secret_key"].as_str().unwrap());
    let nonce = arr32(v["nonce"].as_str().unwrap());
    let claimed = arr32(
        v["peer_id"]
            .as_str()
            .unwrap_or_else(|| panic!("{}: claimed HELLO.peer_id required", path.display())),
    );
    let pid = bytes_to_hex(&peer_id_from_pk(&pk));
    let honest = sign_auth(&seed, &nonce);
    let sig = if let Some(s) = v["auth_signature"].as_str() {
        arr64(s)
    } else {
        honest
    };
    let auth_ok = authenticate(&claimed, &pk, &nonce, &sig).is_ok();
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
    if let Some(frames) = v["frames"].as_array() {
        for (i, f) in frames.iter().enumerate() {
            let ty = f["type"].as_u64().unwrap_or(0) as u8;
            if ty != MSG_SYNC_REQUEST && ty != MSG_SYNC_RESPONSE {
                continue;
            }
            let dir = f["dir"].as_str().unwrap_or("");
            if dir == DIR_RELAY_TO_PEER
                && let Some(got) = f["payload"]["validated_root"].as_str()
            {
                assert_eq!(got, vr, "{} frames[{i}] validated_root", path.display());
            }
            if dir == DIR_PEER_TO_RELAY
                && let Some(got) = f["payload"]["accepted_root"].as_str()
            {
                assert_eq!(got, ar, "{} frames[{i}] accepted_root", path.display());
            }
        }
    }
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

fn check_frames(v: &Json, path: &Path) {
    let frames = v["frames"].as_array().unwrap_or_else(|| {
        panic!(
            "{}: frames must be a non-empty array of {{type, request_id, payload}}",
            path.display()
        )
    });
    assert!(
        !frames.is_empty(),
        "{}: frames must be a non-empty array",
        path.display()
    );

    let mut pending: BTreeMap<u64, &'static [u8]> = BTreeMap::new();

    for (i, f) in frames.iter().enumerate() {
        let label = format!("{} frames[{i}]", path.display());
        let ty = f["type"]
            .as_u64()
            .unwrap_or_else(|| panic!("{label}: type must be a number")) as u8;
        let rid = f["request_id"]
            .as_u64()
            .unwrap_or_else(|| panic!("{label}: request_id must be a number"));
        let dir = f["dir"]
            .as_str()
            .unwrap_or_else(|| panic!("{label}: dir required"));
        let payload = f["payload"]
            .as_object()
            .unwrap_or_else(|| panic!("{label}: payload required"));

        assert!(known_message_type(ty), "{label}: unknown type 0x{ty:02x}");
        assert!(
            dir == DIR_PEER_TO_RELAY || dir == DIR_RELAY_TO_PEER,
            "{label}: dir must be {DIR_PEER_TO_RELAY} or {DIR_RELAY_TO_PEER}, got {dir}"
        );
        if let Some(want) = fixed_direction(ty) {
            assert_eq!(dir, want, "{label}: type 0x{ty:02x} direction");
        }
        for key in required_payload_keys(ty) {
            assert!(
                payload.contains_key(*key) && !payload[*key].is_null(),
                "{label}: missing {key}"
            );
        }
        if ty == MSG_SYNC_REQUEST || ty == MSG_SYNC_RESPONSE {
            let root = required_sync_root(dir).unwrap_or_else(|| panic!("{label}: invalid dir"));
            assert!(
                payload.contains_key(root) && !payload[root].is_null(),
                "{label}: {} SYNC must carry {root}",
                if dir == DIR_PEER_TO_RELAY {
                    "peer"
                } else {
                    "relay"
                }
            );
        }

        if is_request(ty, dir, rid as u32) {
            assert_ne!(rid, 0, "{label}: request must have non-zero request_id");
            pending.insert(rid, expected_response_types(ty));
        }
        if is_response(ty, rid as u32) {
            assert_ne!(rid, 0, "{label}: response must echo a request_id");
            let want = pending
                .remove(&rid)
                .unwrap_or_else(|| panic!("{label}: no open request for request_id {rid}"));
            assert!(
                want.contains(&ty),
                "{label}: type 0x{ty:02x} does not correlate with request_id {rid}"
            );
        }
    }
    assert!(
        pending.is_empty(),
        "{}: unmatched request_id(s) {:?}",
        path.display(),
        pending.keys().copied().collect::<Vec<_>>()
    );

    match v["kind"].as_str().unwrap() {
        "handshake" => check_handshake_frames(v, frames, path),
        "dual-root" => check_dual_root_frames(frames, path),
        "resume" => check_resume_frames(v, frames, path),
        "reject-ack" => check_reject_frames(v, frames, path),
        _ => {}
    }
}

fn check_handshake_frames(v: &Json, frames: &[Json], path: &Path) {
    assert!(
        frames.len() >= 4,
        "{}: handshake frames must be HELLO/CHALLENGE/AUTH/final",
        path.display()
    );
    assert_eq!(
        frames[0]["type"].as_u64().unwrap(),
        MSG_HELLO as u64,
        "{} frames[0] HELLO",
        path.display()
    );
    assert_eq!(
        frames[1]["type"].as_u64().unwrap(),
        MSG_CHALLENGE as u64,
        "{} frames[1] CHALLENGE",
        path.display()
    );
    assert_eq!(
        frames[2]["type"].as_u64().unwrap(),
        MSG_AUTH as u64,
        "{} frames[2] AUTH",
        path.display()
    );
    let last = if v["expect"]["auth_ok"].as_bool().unwrap() {
        MSG_WELCOME
    } else {
        MSG_ERROR
    };
    assert_eq!(
        frames[3]["type"].as_u64().unwrap(),
        last as u64,
        "{} frames[3]",
        path.display()
    );
    assert_eq!(
        frames[0]["payload"]["peer_id"],
        v["peer_id"],
        "{} HELLO.peer_id",
        path.display()
    );
    assert_eq!(
        frames[0]["payload"]["public_key"],
        v["public_key"],
        "{} HELLO.public_key",
        path.display()
    );
    assert_eq!(
        frames[1]["payload"]["nonce"],
        v["nonce"],
        "{} CHALLENGE.nonce",
        path.display()
    );
    if !v["expect"]["auth_ok"].as_bool().unwrap() {
        assert_eq!(
            frames[3]["payload"]["code"].as_u64().unwrap(),
            ERR_AUTH_FAILED as u64,
            "{} ERROR.code",
            path.display()
        );
        assert_eq!(
            frames[3]["payload"]["fatal"],
            true,
            "{} ERROR.fatal",
            path.display()
        );
    }
}

fn check_dual_root_frames(frames: &[Json], path: &Path) {
    let mut peer_resp = false;
    let mut relay_resp = false;
    for f in frames {
        if f["type"].as_u64() != Some(MSG_SYNC_RESPONSE as u64) {
            continue;
        }
        match f["dir"].as_str() {
            Some(DIR_PEER_TO_RELAY) => peer_resp = true,
            Some(DIR_RELAY_TO_PEER) => relay_resp = true,
            _ => {}
        }
    }
    assert!(
        peer_resp,
        "{}: dual-root frames must include a peer SYNC_RESPONSE (accepted_root)",
        path.display()
    );
    assert!(
        relay_resp,
        "{}: dual-root frames must include a relay SYNC_RESPONSE (validated_root)",
        path.display()
    );
}

fn check_resume_frames(v: &Json, frames: &[Json], path: &Path) {
    let mut saw_cursor = false;
    let mut ops: Vec<String> = Vec::new();
    for f in frames {
        if f["type"].as_u64() == Some(MSG_SYNC_REQUEST as u64) {
            assert!(
                f["payload"].get("cursor").is_some_and(|c| !c.is_null()),
                "{}: resume SYNC_REQUEST must carry cursor",
                path.display()
            );
            saw_cursor = true;
        }
        if f["type"].as_u64() == Some(MSG_OPS as u64) && f["dir"] == DIR_PEER_TO_RELAY {
            for op in f["payload"]["operations"].as_array().unwrap() {
                ops.push(op["op_id"].as_str().unwrap().to_string());
            }
        }
    }
    assert!(
        saw_cursor,
        "{}: resume frames must include SYNC_REQUEST.cursor",
        path.display()
    );
    ops.sort();
    let mut want: Vec<String> = v["expect"]["retransmit"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    want.sort();
    assert_eq!(
        ops,
        want,
        "{}: OPS must carry the retransmit set",
        path.display()
    );
}

fn check_reject_frames(v: &Json, frames: &[Json], path: &Path) {
    let mut saw_ops = false;
    let mut rejected: Vec<String> = Vec::new();
    for f in frames {
        if f["type"].as_u64() == Some(MSG_OPS as u64) {
            saw_ops = true;
        }
        if f["type"].as_u64() == Some(MSG_OP_ACK as u64) {
            for o in f["payload"]["outcomes"].as_array().unwrap() {
                if o["outcome"] == "REJECT" {
                    rejected.push(o["op_id"].as_str().unwrap().to_string());
                }
            }
        }
    }
    assert!(
        saw_ops,
        "{}: reject-ack frames must include OPS",
        path.display()
    );
    let mut want: Vec<String> = v["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["outcome"] == "REJECT")
        .map(|o| o["op_id"].as_str().unwrap().to_string())
        .collect();
    rejected.sort();
    want.sort();
    assert_eq!(rejected, want, "{}: OP_ACK REJECT set", path.display());
}
