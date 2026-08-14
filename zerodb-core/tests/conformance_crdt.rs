//! Rust side of the `crdt-apply` conformance vectors (doc/KERNEL.md §9).
//!
//! Delegates to `zerodb_core::apply_crdt_vector` so the NAPI binding and
//! this harness cannot drift.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::apply_crdt_vector;

#[test]
fn crdt_apply_vectors() {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut ran = 0;
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("crdt");
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
    assert!(ran > 0, "no crdt-apply vectors found under {vectors:?}");
}

fn check_vector(v: &Json, path: &Path) {
    let result = apply_crdt_vector(v).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(result["id"], v["id"], "{}", path.display());
    let orders = result["orders"].as_array().expect("orders");
    let want_orders = v["orders"].as_array().unwrap();
    assert_eq!(orders.len(), want_orders.len(), "{}", path.display());
    let expect_error = v["expect_error"].as_str();
    let expect_equivocation = v["expect_equivocation"].as_bool().unwrap_or(false);
    for (oi, got) in orders.iter().enumerate() {
        let ctx = format!("{} order {oi}", v["id"]);
        if let Some(want) = expect_error {
            assert_eq!(got["error"].as_str(), Some(want), "{ctx}");
            assert!(got["state"].is_null(), "{ctx}");
        } else {
            assert!(got["error"].is_null(), "{ctx}: {}", got["error"]);
            assert_eq!(got["state"], v["expect"], "{ctx}");
            assert_eq!(
                got["equivocation"].as_bool().unwrap(),
                expect_equivocation,
                "{ctx}: equivocation signal"
            );
        }
    }
}
