//! Rust side of `h6-profile` vectors (H6 close candidate).
//! Same contract as `conformance/ts/models/h6.mjs`. Not H6 closed.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::cbor::{self, Cbor, encode};
use zerodb_core::handshake::{
    AuthTranscript, admit_datastore, auth_transcript_preimage, authenticate, is_handshake_server,
    sign_auth, sign_auth_v1_nonce_only,
};
use zerodb_core::relay::{
    ERR_AUTH_FAILED, ERR_TARGET_NOT_CONNECTED, FrontierTip, HeldOp, MSG_ERROR, MSG_SIGNAL,
    retransmit,
};

const AUTH_WRONG_DATASTORE: &str = "AUTH_WRONG_DATASTORE";
const ERR_VERSION_MISMATCH: u16 = 0x102;
const DOMAIN_V2: &[u8] = b"zerodb-relay-auth-v2";
const DOMAIN_V1: &[u8] = b"zerodb-relay-auth-v1";

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn arr32(s: &str) -> [u8; 32] {
    hex_to_bytes(s).try_into().expect("32 bytes")
}

fn strs(v: &Json) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect()
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

fn encode_env(ty: u8, request_id: u32, payload: Cbor) -> Vec<u8> {
    encode(&Cbor::Map(vec![
        ("type".into(), Cbor::Uint(ty as u64)),
        ("request_id".into(), Cbor::Uint(request_id as u64)),
        ("payload".into(), payload),
    ]))
    .unwrap()
}

fn map_get<'a>(m: &'a Cbor, k: &str) -> &'a Cbor {
    match m {
        Cbor::Map(ents) => ents
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("missing {k}")),
        _ => panic!("not a map"),
    }
}

fn as_u64(v: &Cbor) -> u64 {
    match v {
        Cbor::Uint(n) => *n,
        _ => panic!("not uint"),
    }
}

fn decode_env(bytes: &[u8]) -> (u8, Cbor) {
    let c = cbor::decode(bytes).expect("cbor");
    (
        as_u64(map_get(&c, "type")) as u8,
        map_get(&c, "payload").clone(),
    )
}

fn run_signal_forward(v: &Json, path: &Path) {
    let rid = v["request_id"].as_u64().unwrap() as u32;
    let sender = hex_to_bytes(v["sender"].as_str().unwrap());
    let target = hex_to_bytes(v["target"].as_str().unwrap());
    let payload = hex_to_bytes(v["payload_hex"].as_str().unwrap());
    let inbound = encode_env(
        MSG_SIGNAL,
        rid,
        Cbor::Map(vec![
            ("payload".into(), Cbor::Bytes(payload.clone())),
            ("target".into(), Cbor::Bytes(target)),
        ]),
    );
    let forwarded = encode_env(
        MSG_SIGNAL,
        rid,
        Cbor::Map(vec![
            ("payload".into(), Cbor::Bytes(payload)),
            ("sender".into(), Cbor::Bytes(sender)),
        ]),
    );
    let (inn_ty, inn_pl) = decode_env(&inbound);
    let (out_ty, out_pl) = decode_env(&forwarded);
    assert_eq!(inn_ty, MSG_SIGNAL, "{} inbound type", path.display());
    assert_eq!(out_ty, MSG_SIGNAL, "{} forward type", path.display());
    match map_get(&inn_pl, "target") {
        Cbor::Bytes(b) if !b.is_empty() => {}
        _ => panic!("{} inbound must carry target", path.display()),
    }
    match map_get(&out_pl, "sender") {
        Cbor::Bytes(b) if !b.is_empty() => {}
        _ => panic!("{} forward must assert sender", path.display()),
    }
    match &out_pl {
        Cbor::Map(ents) => assert!(
            ents.iter().all(|(k, _)| k != "target"),
            "{} forward must omit target",
            path.display()
        ),
        _ => panic!("forward payload"),
    }
}

fn run_signal_missing(v: &Json, path: &Path) {
    let rid = v["request_id"].as_u64().unwrap_or(1) as u32;
    let frame = encode_env(
        MSG_ERROR,
        rid,
        Cbor::Map(vec![
            ("code".into(), Cbor::Uint(ERR_TARGET_NOT_CONNECTED as u64)),
            ("fatal".into(), Cbor::Bool(false)),
            ("message".into(), Cbor::Text("TARGET_NOT_CONNECTED".into())),
        ]),
    );
    let (ty, pl) = decode_env(&frame);
    assert_eq!(ty, MSG_ERROR, "{}", path.display());
    assert_eq!(
        as_u64(map_get(&pl, "code")) as u16,
        ERR_TARGET_NOT_CONNECTED,
        "{}",
        path.display()
    );
    assert_eq!(
        v["expect"]["code"].as_u64().unwrap() as u16,
        ERR_TARGET_NOT_CONNECTED
    );
    match map_get(&pl, "message") {
        Cbor::Text(s) => assert_eq!(s, v["expect"]["message"].as_str().unwrap()),
        _ => panic!("message"),
    }
}

fn run_peer_role(v: &Json, path: &Path) {
    for c in v["cases"].as_array().unwrap() {
        let local = arr32(c["local"].as_str().unwrap());
        let remote = arr32(c["remote"].as_str().unwrap());
        let want = c["handshake_server"].as_bool().unwrap();
        assert_eq!(
            is_handshake_server(&local, &remote),
            want,
            "{} {:?}",
            path.display(),
            c
        );
    }
}

fn run_auth(v: &Json, path: &Path) {
    let peer = arr32(v["peer_id"].as_str().unwrap());
    let pk = arr32(v["public_key"].as_str().unwrap());
    let seed = arr32(v["secret_key"].as_str().unwrap());
    let nonce = arr32(v["nonce"].as_str().unwrap());
    let caps = strs(&v["hello_capabilities"]);
    let t = AuthTranscript::for_relay_hello(peer, pk, 1, &caps, nonce);
    let pre = auth_transcript_preimage(&t);
    assert!(
        pre.starts_with(DOMAIN_V2),
        "{} preimage must be v2",
        path.display()
    );
    assert!(
        !pre.starts_with(DOMAIN_V1),
        "{} preimage must not be v1",
        path.display()
    );
    if v["kind"] == "auth-v1-reject" {
        let sig = sign_auth_v1_nonce_only(&seed, &nonce);
        assert_eq!(
            authenticate(&peer, &pk, &t, &sig),
            Err(ERR_AUTH_FAILED),
            "{}",
            path.display()
        );
        return;
    }
    let sig = sign_auth(&seed, &t);
    assert!(
        authenticate(&peer, &pk, &t, &sig).is_ok(),
        "{} v2 AUTH",
        path.display()
    );
}

fn run_welcome_version(v: &Json, path: &Path) {
    assert_ne!(
        v["protocol_version"].as_u64().unwrap(),
        1,
        "{}",
        path.display()
    );
    assert_eq!(
        v["expect"]["code"].as_u64().unwrap() as u16,
        ERR_VERSION_MISMATCH,
        "{}",
        path.display()
    );
    assert_eq!(
        v["expect"]["name"].as_str().unwrap(),
        "VERSION_MISMATCH",
        "{}",
        path.display()
    );
}

fn opt_hex(v: &Json) -> Option<Vec<u8>> {
    v.as_str().map(hex_to_bytes)
}

fn run_admit(v: &Json, path: &Path) {
    let bound = opt_hex(&v["bound_ds"]);
    let offered = opt_hex(&v["offered_ds"]);
    let got = admit_datastore(bound.as_deref(), offered.as_deref());
    if v["expect"]["ok"].as_bool().unwrap() {
        assert!(got.is_ok(), "{} {:?}", path.display(), got);
    } else {
        assert_eq!(got, Err(AUTH_WRONG_DATASTORE), "{}", path.display());
        assert_eq!(
            v["expect"]["reason"].as_str().unwrap(),
            AUTH_WRONG_DATASTORE
        );
    }
}

fn run_resume(v: &Json, path: &Path) {
    let held: Vec<HeldOp> = v["held"]
        .as_array()
        .unwrap()
        .iter()
        .map(parse_held)
        .collect();
    let frontier = parse_frontier(&v["cursor"]);
    let rejected: Vec<String> = v
        .get("rejected")
        .and_then(Json::as_array)
        .map(|arr| {
            arr.iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default();
    let got = retransmit(&held, &frontier, &rejected);
    let mut want: Vec<String> = v["expect"]["retransmit"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    want.sort();
    assert_eq!(got, want, "{}", path.display());
}

fn run_vector(v: &Json, path: &Path) {
    assert_eq!(v["type"], "h6-profile", "{}", path.display());
    match v["kind"].as_str().unwrap() {
        "signal-forward" => run_signal_forward(v, path),
        "signal-missing" => run_signal_missing(v, path),
        "peer-role" => run_peer_role(v, path),
        "auth-v2" | "auth-v1-reject" => run_auth(v, path),
        "welcome-version" => run_welcome_version(v, path),
        "admit" => run_admit(v, path),
        "resume-cursor" => run_resume(v, path),
        other => panic!("{} unknown kind {other}", path.display()),
    }
}

#[test]
fn h6_profile_vectors() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors/required/h6");
    let mut ran = 0;
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("required/h6")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    for path in files {
        let vector: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        run_vector(&vector, &path);
        ran += 1;
    }
    assert_eq!(ran, 9, "H6 profile is nine named fixtures under {dir:?}");
}

#[test]
fn registry_names_h6_fixtures() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/registry.json");
    let reg: Json = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let fixtures = reg["h6_profile"]["fixtures"].as_array().unwrap();
    assert_eq!(fixtures.len(), 9);
    assert_eq!(reg["h6_profile"]["admission_error"], AUTH_WRONG_DATASTORE);
    assert_eq!(reg["h6_profile"]["auth_domain"], "zerodb-relay-auth-v2");
}
