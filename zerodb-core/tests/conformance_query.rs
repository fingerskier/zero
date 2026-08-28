//! Rust side of the query conformance vectors (doc/SCHEMA.md §5).
//!
//! `query-parse` cases pin grammar acceptance; `query-eval` cases pin
//! evaluation over a fixture graph. Both must match the JS runner exactly.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::query::parse;
use zerodb_core::queryeval::{GEdge, GNode, Graph, QValue, eval};

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
fn query_vectors() {
    for path in vector_files() {
        let v: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        match v["type"].as_str().unwrap() {
            "query-parse" => run_parse(&v, &path),
            "query-eval" => run_eval(&v, &path),
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

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn qvalue(v: &Json) -> QValue {
    match v["t"].as_str().unwrap() {
        "null" => QValue::Null,
        "bool" => QValue::Bool(v["v"].as_bool().unwrap()),
        "int" => QValue::Int(v["v"].as_i64().unwrap()),
        "float" => QValue::Float(v["v"].as_f64().unwrap()),
        "text" => QValue::Text(v["v"].as_str().unwrap().to_owned()),
        "bytes" => QValue::Bytes(hex_to_bytes(v["hex"].as_str().unwrap())),
        "mv" => QValue::Mv(v["v"].as_array().unwrap().iter().map(qvalue).collect()),
        "node" => QValue::Node(v["id"].as_str().unwrap().to_owned()),
        "edge" => QValue::Edge(v["id"].as_str().unwrap().to_owned()),
        other => panic!("unknown tagged value type {other}"),
    }
}

fn props(v: &Json) -> BTreeMap<String, QValue> {
    v.as_object()
        .map(|o| o.iter().map(|(k, val)| (k.clone(), qvalue(val))).collect())
        .unwrap_or_default()
}

fn graph(v: &Json) -> Graph {
    let nodes = v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| GNode {
            id: n["id"].as_str().unwrap().to_owned(),
            label: n["label"].as_str().unwrap().to_owned(),
            props: props(&n["props"]),
        })
        .collect();
    let edges = v["edges"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|e| GEdge {
                    id: e["id"].as_str().unwrap().to_owned(),
                    label: e["label"].as_str().unwrap().to_owned(),
                    src: e["src"].as_str().unwrap().to_owned(),
                    dst: e["dst"].as_str().unwrap().to_owned(),
                    props: props(&e["props"]),
                })
                .collect()
        })
        .unwrap_or_default();
    Graph::new(nodes, edges)
}

fn run_eval(v: &Json, path: &Path) {
    let vid = v["id"].as_str().unwrap();
    let query = parse(v["query"].as_str().unwrap())
        .unwrap_or_else(|e| panic!("{vid} ({path:?}): query failed to parse: {e}"));
    let g = graph(&v["graph"]);
    let params: BTreeMap<String, QValue> = v["params"]
        .as_object()
        .map(|o| o.iter().map(|(k, val)| (k.clone(), qvalue(val))).collect())
        .unwrap_or_default();

    let got = eval(&query, &g, &params).unwrap_or_else(|e| panic!("{vid}: eval error {e}"));
    let want: Vec<Vec<QValue>> = v["expect"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row.as_array().unwrap().iter().map(qvalue).collect())
        .collect();
    assert_eq!(got, want, "{vid} ({path:?}): rows");
}
