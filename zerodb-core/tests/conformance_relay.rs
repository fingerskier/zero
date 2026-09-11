//! Rust side of `relay-transcript` vectors (doc/RELAY-SPEC.md 0.2.2-draft).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::cbor::{Cbor, encode};
use zerodb_core::merkle::{MerkleOp, MerkleTree};
use zerodb_core::relay::{
    AuthTranscript, BYTE_FIELDS, DIR_PEER_TO_RELAY, DIR_RELAY_TO_PEER, ERR_AUTH_FAILED,
    ERR_PAYLOAD_TOO_LARGE, FrontierTip, HeldOp, MSG_AUTH, MSG_CHALLENGE, MSG_DELTA_REQUEST,
    MSG_ERROR, MSG_HELLO, MSG_MERKLE_LEAF_REQUEST, MSG_MERKLE_NODE_REQUEST, MSG_OP_ACK, MSG_OPS,
    MSG_SYNC_REQUEST, MSG_SYNC_RESPONSE, MSG_WELCOME, authenticate, expected_response_types,
    fixed_direction, is_request, is_response, known_message_type, negotiate_capabilities,
    peer_id_from_pk, required_payload_keys, required_sync_root, retransmit, root_hex, sign_auth,
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

fn is_byte_field(k: &str) -> bool {
    BYTE_FIELDS.contains(&k)
}

fn json_to_cbor(v: &Json, field: Option<&str>) -> Cbor {
    if v.is_null() {
        return Cbor::Null;
    }
    if let Some(b) = v.as_bool() {
        return Cbor::Bool(b);
    }
    if let Some(n) = v.as_u64() {
        return Cbor::Uint(n);
    }
    if let Some(s) = v.as_str() {
        if field.is_some_and(is_byte_field) {
            return Cbor::Bytes(hex_to_bytes(s));
        }
        return Cbor::Text(s.to_owned());
    }
    if let Some(arr) = v.as_array() {
        let item_field = match field {
            Some("op_ids") => Some("op_id"),
            _ => None,
        };
        return Cbor::Array(arr.iter().map(|x| json_to_cbor(x, item_field)).collect());
    }
    if let Some(obj) = v.as_object() {
        return Cbor::Map(
            obj.iter()
                .map(|(k, val)| (k.clone(), json_to_cbor(val, Some(k.as_str()))))
                .collect(),
        );
    }
    panic!("unsupported json for envelope cbor: {v}");
}

fn encode_envelope(ty: u8, request_id: u64, payload: &Json) -> Vec<u8> {
    let env = Cbor::Map(vec![
        ("type".into(), Cbor::Uint(ty as u64)),
        ("request_id".into(), Cbor::Uint(request_id)),
        ("payload".into(), json_to_cbor(payload, None)),
    ]);
    encode(&env).expect("encode envelope")
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
        "ops-push" | "merkle-walk" | "limits" => {}
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
    let hello_caps = strs(&v["hello_capabilities"]);
    let version = v["protocol_version"].as_u64().unwrap_or(1) as u8;
    let transcript = AuthTranscript::for_relay_hello(claimed, pk, version, &hello_caps, nonce);
    let honest = sign_auth(&seed, &transcript);
    let sig = if let Some(s) = v["frames"]
        .as_array()
        .and_then(|f| f.get(2))
        .and_then(|f| f["payload"]["signature"].as_str())
    {
        arr64(s)
    } else if let Some(s) = v["auth_signature"].as_str() {
        arr64(s)
    } else {
        honest
    };
    let auth_ok = authenticate(&claimed, &pk, &transcript, &sig).is_ok();
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
        let want_hex = f["cbor_hex"]
            .as_str()
            .unwrap_or_else(|| panic!("{label}: cbor_hex required"));
        assert!(!want_hex.is_empty(), "{label}: cbor_hex required");
        let got = bytes_to_hex(&encode_envelope(ty, rid, &f["payload"]));
        assert_eq!(got, want_hex, "{label}: cbor_hex mismatch");

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
        "ops-push" => check_ops_push_frames(v, frames, path),
        "merkle-walk" => check_merkle_walk_frames(v, frames, path),
        "limits" => check_limits_frames(v, frames, path),
        _ => {}
    }
}

fn check_ops_push_frames(v: &Json, frames: &[Json], path: &Path) {
    let mut saw_ops = false;
    let mut outcomes: Vec<Json> = Vec::new();
    for f in frames {
        if f["type"].as_u64() == Some(MSG_OPS as u64) && f["dir"] == DIR_PEER_TO_RELAY {
            saw_ops = true;
        }
        if f["type"].as_u64() == Some(MSG_OP_ACK as u64) {
            for o in f["payload"]["outcomes"].as_array().unwrap() {
                outcomes.push(o.clone());
            }
        }
    }
    assert!(
        saw_ops,
        "{}: ops-push frames must include peer OPS",
        path.display()
    );
    assert!(
        !outcomes.is_empty(),
        "{}: ops-push frames must include OP_ACK outcomes",
        path.display()
    );
    if let Some(want) = v["expect"]["outcomes"].as_array() {
        assert_eq!(outcomes, *want, "{}: OP_ACK outcomes", path.display());
    } else {
        assert!(
            outcomes.iter().all(|o| o["outcome"] == "ACCEPT"),
            "{}: ops-push golden expects ACCEPT",
            path.display()
        );
    }
}

fn merkle_walk_missing(
    local: &[MerkleOp],
    remote: &[MerkleOp],
    buckets: &[u64],
) -> (Vec<String>, Vec<(u8, usize, usize)>) {
    let local_tree = MerkleTree::build_aligned(local, buckets);
    let remote_tree = MerkleTree::build_aligned(remote, buckets);
    let mut missing: Vec<String> = Vec::new();
    let mut steps: Vec<(u8, usize, usize)> = Vec::new();
    fn hex(b: &[u8; 32]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    fn walk(
        local: &MerkleTree,
        remote: &MerkleTree,
        level: usize,
        index: usize,
        missing: &mut Vec<String>,
        steps: &mut Vec<(u8, usize, usize)>,
    ) {
        if level == 0 {
            let remote_ids: Vec<String> = remote
                .leaves
                .get(index)
                .map(|l| l.op_ids.iter().map(hex).collect())
                .unwrap_or_default();
            let local_ids: std::collections::BTreeSet<String> = local
                .leaves
                .get(index)
                .map(|l| l.op_ids.iter().map(hex).collect())
                .unwrap_or_default();
            steps.push((0, level, index));
            for id in remote_ids {
                if !local_ids.contains(&id) {
                    missing.push(id);
                }
            }
            return;
        }
        steps.push((1, level, index));
        let (rleft, rright) = remote.node_children(level, index).expect("remote node");
        let (lleft, lright) = local
            .node_children(level, index)
            .unwrap_or_else(empty_leaf_pair_fallback);
        if rleft != lleft {
            walk(local, remote, level - 1, index * 2, missing, steps);
        }
        if rright != lright {
            walk(local, remote, level - 1, index * 2 + 1, missing, steps);
        }
    }
    if local_tree.root() != remote_tree.root() {
        let root_level = remote_tree.levels.len() - 1;
        walk(
            &local_tree,
            &remote_tree,
            root_level,
            0,
            &mut missing,
            &mut steps,
        );
    }
    missing.sort();
    missing.dedup();
    (missing, steps)
}

fn empty_leaf_pair_fallback() -> ([u8; 32], [u8; 32]) {
    let e = zerodb_core::merkle::empty_leaf();
    (e, e)
}

fn check_merkle_walk_frames(v: &Json, frames: &[Json], path: &Path) {
    let local: Vec<MerkleOp> = v["local"]
        .as_array()
        .unwrap()
        .iter()
        .map(parse_merkle_op)
        .collect();
    let remote: Vec<MerkleOp> = v["remote"]
        .as_array()
        .unwrap()
        .iter()
        .map(parse_merkle_op)
        .collect();
    let buckets: Vec<u64> = v
        .get("bucket_indices")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().map(|x| x.as_u64().unwrap()).collect())
        .unwrap_or_else(|| {
            let mut s: Vec<u64> = remote.iter().map(|o| o.physical_ms / 60_000).collect();
            s.sort();
            s.dedup();
            s
        });
    let (mut got_missing, steps) = merkle_walk_missing(&local, &remote, &buckets);
    let mut want_missing: Vec<String> = v["expect"]["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_lowercase())
        .collect();
    want_missing.sort();
    got_missing = got_missing.into_iter().map(|s| s.to_lowercase()).collect();
    got_missing.sort();
    assert_eq!(
        got_missing,
        want_missing,
        "{}: merkle-walk missing",
        path.display()
    );

    let node_reqs: Vec<&Json> = frames
        .iter()
        .filter(|f| f["type"].as_u64() == Some(MSG_MERKLE_NODE_REQUEST as u64))
        .collect();
    let leaf_reqs: Vec<&Json> = frames
        .iter()
        .filter(|f| f["type"].as_u64() == Some(MSG_MERKLE_LEAF_REQUEST as u64))
        .collect();
    let node_steps: Vec<_> = steps.iter().filter(|s| s.0 == 1).collect();
    let leaf_steps: Vec<_> = steps.iter().filter(|s| s.0 == 0).collect();
    assert_eq!(
        node_reqs.len(),
        node_steps.len(),
        "{}: merkle-walk NODE count",
        path.display()
    );
    assert_eq!(
        leaf_reqs.len(),
        leaf_steps.len(),
        "{}: merkle-walk LEAF count",
        path.display()
    );
    for (i, step) in node_steps.iter().enumerate() {
        assert_eq!(
            node_reqs[i]["payload"]["level"].as_u64().unwrap() as usize,
            step.1,
            "{} NODE[{i}] level",
            path.display()
        );
        assert_eq!(
            node_reqs[i]["payload"]["index"].as_u64().unwrap() as usize,
            step.2,
            "{} NODE[{i}] index",
            path.display()
        );
    }
    for (i, step) in leaf_steps.iter().enumerate() {
        assert_eq!(
            leaf_reqs[i]["payload"]["leaf_index"].as_u64().unwrap() as usize,
            step.2,
            "{} LEAF[{i}] index",
            path.display()
        );
    }
    if !want_missing.is_empty() {
        let delta = frames
            .iter()
            .find(|f| f["type"].as_u64() == Some(MSG_DELTA_REQUEST as u64))
            .unwrap_or_else(|| {
                panic!(
                    "{}: merkle-walk with missing ops must include DELTA_REQUEST",
                    path.display()
                )
            });
        let mut asked: Vec<String> = delta["payload"]["op_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_lowercase())
            .collect();
        asked.sort();
        assert_eq!(asked, want_missing, "{}: DELTA_REQUEST", path.display());
    }
    let remote_root = {
        let tree = MerkleTree::build_aligned(&remote, &buckets);
        tree.root()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let mut saw_sync = false;
    for f in frames {
        if f["type"].as_u64() == Some(MSG_SYNC_RESPONSE as u64) && f["dir"] == DIR_RELAY_TO_PEER {
            saw_sync = true;
            if let Some(got) = f["payload"]["validated_root"].as_str() {
                assert_eq!(
                    got,
                    remote_root,
                    "{}: SYNC_RESPONSE.validated_root",
                    path.display()
                );
            }
        }
    }
    assert!(
        saw_sync,
        "{}: merkle-walk frames must include relay SYNC_RESPONSE",
        path.display()
    );
}

fn check_limits_frames(v: &Json, frames: &[Json], path: &Path) {
    let limits = &v["limits"];
    let max_ops = limits["max_batch_ops"].as_u64().unwrap_or(64);
    let max_bytes = limits["max_batch_bytes"].as_u64().unwrap_or(16_777_216);
    let max_payload = limits["max_payload_bytes"].as_u64().unwrap_or(1_048_576);
    let mut over = false;
    let mut saw_err = false;
    for f in frames {
        if f["type"].as_u64() == Some(MSG_OPS as u64) {
            let ops = f["payload"]["operations"].as_array().unwrap();
            if ops.len() as u64 > max_ops {
                over = true;
            }
            let encoded = encode_envelope(
                f["type"].as_u64().unwrap() as u8,
                f["request_id"].as_u64().unwrap(),
                &f["payload"],
            );
            if encoded.len() as u64 > max_bytes {
                over = true;
            }
            for op in ops {
                let n = encode(&json_to_cbor(op, None)).expect("op").len() as u64;
                if n > max_payload {
                    over = true;
                }
            }
        }
        if f["type"].as_u64() == Some(MSG_ERROR as u64) {
            saw_err = true;
            assert_eq!(
                f["payload"]["code"].as_u64().unwrap(),
                ERR_PAYLOAD_TOO_LARGE as u64,
                "{}: ERROR.code",
                path.display()
            );
            assert_eq!(
                f["payload"]["fatal"],
                false,
                "{}: PAYLOAD_TOO_LARGE fatal",
                path.display()
            );
        }
    }
    assert!(
        over,
        "{}: limits vector OPS must exceed advertised limits",
        path.display()
    );
    assert!(
        saw_err,
        "{}: limits frames must include ERROR 0x303",
        path.display()
    );
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
