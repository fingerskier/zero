//! Rust side of the `hlc-transition` conformance vectors (doc/KERNEL.md §9).
//!
//! Vectors live under conformance/vectors/{required,xfail}/hlc/. They stay in
//! the xfail lane until BOTH runners pass them (promotion policy in
//! conformance/README.md); this harness is the Rust half of that red→green
//! path. Peer hex strings expand to the implementation's PeerId width
//! (first byte, zero-padded).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use zerodb_core::hlc::{HLC, HLCTimestamp};
use zerodb_core::types::PeerId;

static WALL_MS: AtomicU64 = AtomicU64::new(0);

fn vector_clock() -> u64 {
    WALL_MS.load(Ordering::SeqCst)
}

fn peer_from_hex(s: &str) -> PeerId {
    let byte = u8::from_str_radix(s, 16).expect("peer hex byte");
    let mut id = [0u8; 16];
    id[0] = byte;
    PeerId(id)
}

fn ts(v: &Value, peer: PeerId) -> HLCTimestamp {
    HLCTimestamp {
        physical_ms: v["physical_ms"].as_u64().unwrap(),
        logical: v["logical"].as_u64().unwrap() as u16,
        peer_id: peer,
    }
}

#[test]
fn hlc_transition_vectors() {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut ran = 0;
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("hlc");
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let vector: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            run_vector(&vector, &path);
            ran += 1;
        }
    }
    assert!(ran > 0, "no hlc-transition vectors found under {vectors:?}");
}

fn run_vector(v: &Value, path: &Path) {
    assert_eq!(v["type"], "hlc-transition", "wrong vector type in {path:?}");
    let id = v["id"].as_str().unwrap();
    let peer = peer_from_hex(v["peer"].as_str().unwrap());
    WALL_MS.store(v["wall_ms"].as_u64().unwrap(), Ordering::SeqCst);

    let mut hlc = match v.get("resume_from") {
        Some(last) => HLC::resume_with_clock(peer, ts(last, peer), vector_clock),
        None => HLC::with_clock(peer, vector_clock),
    };
    if let Some(drift) = v.get("max_drift_ms").and_then(Value::as_u64) {
        hlc.set_max_drift(drift);
    }

    for (i, step) in v["steps"].as_array().unwrap().iter().enumerate() {
        let result = match step["action"].as_str().unwrap() {
            "local" => Ok(hlc.now()),
            "recv" => {
                let remote_peer = peer_from_hex(step["remote"]["peer"].as_str().unwrap());
                hlc.recv(&ts(&step["remote"], remote_peer))
            }
            other => panic!("{id} step {i}: unknown action {other}"),
        };

        if let Some(expected_err) = step.get("expect_error") {
            assert_eq!(
                expected_err, "DRIFT_EXCEEDED",
                "{id} step {i}: only DRIFT_EXCEEDED is modeled"
            );
            assert!(
                result.is_err(),
                "{id} step {i}: expected DRIFT_EXCEEDED, got {result:?}"
            );
        } else {
            let got = result.unwrap_or_else(|e| panic!("{id} step {i}: unexpected error {e}"));
            let expect = &step["expect"];
            assert_eq!(
                got.physical_ms,
                expect["physical_ms"].as_u64().unwrap(),
                "{id} step {i}: physical_ms"
            );
            assert_eq!(
                got.logical as u64,
                expect["logical"].as_u64().unwrap(),
                "{id} step {i}: logical"
            );
        }
    }
}
