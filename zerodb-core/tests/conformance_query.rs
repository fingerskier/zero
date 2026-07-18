//! Rust side of the `query-parse` conformance vectors (doc/SCHEMA.md §5).
//!
//! Each case is a query string with an expected accept/reject; the Rust parser
//! must agree with the JS parser on every one. Evaluation over a fixture graph
//! lands with the `query-eval` vectors.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::query::parse;

fn vector_files() -> Vec<PathBuf> {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut files = Vec::new();
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("query");
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
fn query_parse_vectors() {
    for path in vector_files() {
        let v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        match v["type"].as_str().unwrap() {
            "query-parse" => run_parse(&v, &path),
            other => panic!("{path:?}: unhandled query vector type {other}"),
        }
    }
}

fn run_parse(v: &Json, path: &Path) {
    let vid = v["id"].as_str().unwrap();
    for (ci, case) in v["cases"].as_array().unwrap().iter().enumerate() {
        let query = case["query"].as_str().unwrap();
        let accept = case["accept"].as_bool().unwrap();
        let got = parse(query).is_ok();
        assert_eq!(
            got,
            accept,
            "{vid} case {ci} ({path:?}): query {query:?} — expected accept={accept}, got {got} ({:?})",
            parse(query).err()
        );
    }
}
