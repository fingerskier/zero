//! Rust side of wal-crash / group-seal vectors (doc/WAL.md).

use std::fs;
use std::path::PathBuf;

use serde_json::Value as Json;
use zerodb_core::wal::{GroupManifest, ModelTs, Step, WalRecord, WalState};

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn arr32(s: &str) -> [u8; 32] {
    hex_to_bytes(s).try_into().expect("32 bytes")
}

fn arr16(s: &str) -> [u8; 16] {
    hex_to_bytes(s).try_into().expect("16 bytes")
}

fn parse_ts(v: &Json) -> ModelTs {
    ModelTs {
        physical_ms: v["p"].as_u64().unwrap(),
        logical: v["l"].as_u64().unwrap() as u16,
    }
}

fn parse_record(v: &Json) -> WalRecord {
    let group = match &v["group"] {
        Json::Null => None,
        other => Some(arr16(other.as_str().unwrap())),
    };
    WalRecord {
        op_id: arr32(v["op_id"].as_str().unwrap()),
        body: v["body"].as_str().unwrap().to_owned(),
        group,
        author_ts: parse_ts(&v["author_ts"]),
    }
}

fn parse_step(v: &Json) -> Step {
    match v["op"].as_str().unwrap() {
        "wal_append" => Step::WalAppend(parse_record(&v["record"])),
        "wal_sync" => Step::WalSync,
        "state_apply" => Step::StateApply(arr32(v["op_id"].as_str().unwrap())),
        "hlc_persist" => Step::HlcPersist(parse_ts(&v["ts"])),
        "group_seal" => {
            let m = &v["manifest"];
            Step::GroupSeal(GroupManifest {
                group_id: arr16(m["group_id"].as_str().unwrap()),
                members: m["members"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| arr32(x.as_str().unwrap()))
                    .collect(),
                abort: m["abort"].as_bool().unwrap_or(false),
            })
        }
        "wal_truncate" => Step::WalTruncate(v["up_to_index"].as_u64().unwrap() as usize),
        other => panic!("unknown step {other}"),
    }
}

fn vector_files() -> Vec<PathBuf> {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut files = Vec::new();
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("wal");
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

fn check_expect(st: &WalState, exp: &Json, path: &std::path::Path) {
    if let Some(n) = exp["wal_len"].as_u64() {
        assert_eq!(st.wal.len() as u64, n, "wal_len {}", path.display());
    }
    if let Some(arr) = exp["applied"].as_array() {
        let mut got: Vec<String> = st
            .applied
            .iter()
            .map(|id| id.iter().map(|b| format!("{b:02x}")).collect())
            .collect();
        got.sort();
        let mut want: Vec<String> = arr.iter().map(|x| x.as_str().unwrap().to_owned()).collect();
        want.sort();
        assert_eq!(got, want, "applied {}", path.display());
    }
    match &exp["hlc"] {
        Json::Null => assert!(st.hlc.is_none(), "hlc {}", path.display()),
        Json::Object(_) => {
            let t = parse_ts(&exp["hlc"]);
            assert_eq!(st.hlc, Some(t), "hlc {}", path.display());
        }
        _ => {}
    }
    if let Some(arr) = exp["sealed"].as_array() {
        let mut got: Vec<String> = st
            .sealed
            .iter()
            .map(|id| id.iter().map(|b| format!("{b:02x}")).collect())
            .collect();
        got.sort();
        let mut want: Vec<String> = arr.iter().map(|x| x.as_str().unwrap().to_owned()).collect();
        want.sort();
        assert_eq!(got, want, "sealed {}", path.display());
    }
}

#[test]
fn wal_vectors() {
    let files = vector_files();
    assert!(!files.is_empty(), "no wal vectors");
    for path in files {
        let raw = fs::read_to_string(&path).unwrap();
        let v: Json = serde_json::from_str(&raw).unwrap();
        match v["type"].as_str().unwrap() {
            "wal-crash" => {
                let schedule: Vec<Step> = v["schedule"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(parse_step)
                    .collect();
                let crash = v["crash_after_step"].as_i64().unwrap() as i32;
                let st = WalState::run_until_crash(&schedule, crash);
                check_expect(&st, &v["expect_after_recover"], &path);
            }
            "group-seal" => {
                let mut st = WalState::default();
                for step_j in v["schedule"].as_array().unwrap() {
                    let step = parse_step(step_j);
                    let res = st.run_step(&step);
                    if let Some(err) = step_j["expect_error"].as_str() {
                        assert!(res.is_err(), "expected error {err} {}", path.display());
                        assert_eq!(format!("{}", res.unwrap_err()), err, "{}", path.display());
                    } else {
                        res.unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    }
                }
                if let Some(arr) = v["expect_sealed"].as_array() {
                    check_expect(&st, &serde_json::json!({ "sealed": arr }), &path);
                }
            }
            other => panic!("{}: unknown type {other}", path.display()),
        }
    }
}
