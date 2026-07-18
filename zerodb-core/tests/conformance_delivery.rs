//! delivery-schedule vectors (doc/DELIVERY.md).

use std::fs;
use std::path::PathBuf;

use serde_json::Value as Json;
use zerodb_core::delivery::{DelivStep, run_schedule};

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn arr32(s: &str) -> [u8; 32] {
    hex_to_bytes(s).try_into().expect("32")
}

fn bytes_to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn parse_step(v: &Json) -> DelivStep {
    match v["op"].as_str().unwrap() {
        "hold" => DelivStep::Hold(arr32(v["op_id"].as_str().unwrap())),
        "deliver" => DelivStep::Deliver {
            op_id: arr32(v["op_id"].as_str().unwrap()),
            reject: v["reject"].as_bool().unwrap_or(false),
        },
        "deliver_batch" => DelivStep::DeliverBatch {
            op_ids: v["op_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| arr32(x.as_str().unwrap()))
                .collect(),
            rejects: v
                .get("rejects")
                .and_then(|r| r.as_array())
                .map(|a| a.iter().map(|x| x.as_bool().unwrap()).collect())
                .unwrap_or_default(),
        },
        "resume" => DelivStep::Resume,
        other => panic!("unknown {other}"),
    }
}

#[test]
fn delivery_vectors() {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors/required/delivery");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    for path in files {
        let v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["type"], "delivery-schedule");
        let steps: Vec<_> = v["schedule"]
            .as_array()
            .unwrap()
            .iter()
            .map(parse_step)
            .collect();
        let got = run_schedule(&steps);
        let exp = &v["expect"];
        if let Some(seen) = exp["seen"].as_array() {
            let mut g: Vec<_> = got.seen.iter().map(|id| bytes_to_hex(id)).collect();
            g.sort();
            let mut w: Vec<_> = seen
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect();
            w.sort();
            assert_eq!(g, w, "{}", path.display());
        }
        if let Some(out) = exp["last_outcomes"].as_array() {
            let w: Vec<_> = out
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect();
            assert_eq!(got.last_outcomes, w, "{}", path.display());
        }
        if let Some(rs) = exp["resume_sent"].as_array() {
            let g: Vec<_> = got.resume_sent.iter().map(|id| bytes_to_hex(id)).collect();
            let w: Vec<_> = rs.iter().map(|x| x.as_str().unwrap().to_string()).collect();
            assert_eq!(g, w, "{}", path.display());
        }
    }
}
