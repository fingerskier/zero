//! Rust side of `peer-ingest` vectors (named rejects + SchemaEpoch n=1).
//! Independent of the TypeScript PeerStore; both must agree.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use zerodb_core::cbor::decode;
use zerodb_core::op::{OpEnvelope, OpTs, json_to_cbor_body};
use zerodb_core::schema::{parse_ir, schema_id};
use zerodb_core::sign::verify_op;

const AUTH_SIG_INVALID: &str = "AUTH_SIG_INVALID";
const AUTH_WRONG_DATASTORE: &str = "AUTH_WRONG_DATASTORE";
const CLOCK_DRIFT: &str = "CLOCK_DRIFT";
const EPOCH_UNKNOWN: &str = "EPOCH_UNKNOWN";
const APPLY_INVALID: &str = "APPLY_INVALID";
const MAX_DRIFT_MS: u64 = 60_000;
const ZERO_DS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn arr32(s: &str) -> [u8; 32] {
    hex_to_bytes(s).try_into().expect("32 bytes")
}

fn arr64(s: &str) -> [u8; 64] {
    hex_to_bytes(s).try_into().expect("64 bytes")
}

fn bytes_to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn is_hex32(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn validate_schema_epoch_body(body: &Json) -> Result<(), &'static str> {
    let obj = body.as_object().ok_or(APPLY_INVALID)?;
    let epoch = obj
        .get("epoch")
        .and_then(Json::as_u64)
        .ok_or(APPLY_INVALID)?;
    if epoch == 0 {
        return Err(APPLY_INVALID);
    }
    let schema = obj
        .get("schema")
        .and_then(Json::as_str)
        .ok_or(APPLY_INVALID)?;
    if !is_hex32(schema) {
        return Err(APPLY_INVALID);
    }
    let ir_hex = obj.get("ir").and_then(Json::as_str).ok_or(APPLY_INVALID)?;
    let ir_bytes = hex::decode(ir_hex).map_err(|_| APPLY_INVALID)?;
    if ir_bytes.is_empty() {
        return Err(APPLY_INVALID);
    }
    let decoded = decode(&ir_bytes).map_err(|_| APPLY_INVALID)?;
    parse_ir(&decoded).map_err(|_| APPLY_INVALID)?;
    if schema_id(&ir_bytes) != arr32(schema) {
        return Err(APPLY_INVALID);
    }
    match obj.get("prev") {
        None | Some(Json::Null) => {
            if epoch != 1 {
                return Err(APPLY_INVALID);
            }
        }
        Some(Json::String(h)) => {
            if epoch == 1 || !is_hex32(h) {
                return Err(APPLY_INVALID);
            }
        }
        _ => return Err(APPLY_INVALID),
    }
    match obj.get("migration") {
        Some(Json::Array(a)) if a.is_empty() => Ok(()),
        _ => Err(APPLY_INVALID),
    }
}

fn verify_wire(wire: &Json) -> Result<(), &'static str> {
    let id = wire
        .get("id")
        .and_then(Json::as_str)
        .ok_or(AUTH_SIG_INVALID)?;
    let author = wire
        .get("author")
        .and_then(Json::as_str)
        .ok_or(AUTH_SIG_INVALID)?;
    let pk_hex = wire
        .get("author_pk")
        .and_then(Json::as_str)
        .ok_or(AUTH_SIG_INVALID)?;
    let sig_hex = wire
        .get("sig")
        .and_then(Json::as_str)
        .ok_or(AUTH_SIG_INVALID)?;
    let pk = arr32(pk_hex);
    if bytes_to_hex(blake3::hash(&pk).as_bytes()) != author {
        return Err(AUTH_SIG_INVALID);
    }
    let ts = wire.get("ts").ok_or(AUTH_SIG_INVALID)?;
    let deps = match wire.get("deps") {
        Some(Json::Array(items)) => {
            let mut out = Vec::new();
            for d in items {
                let s = d.as_str().ok_or(AUTH_SIG_INVALID)?;
                out.push(arr32(s));
            }
            out
        }
        _ => Vec::new(),
    };
    let grp = match wire.get("grp") {
        None | Some(Json::Null) => None,
        Some(Json::String(h)) => {
            let b = hex::decode(h).map_err(|_| AUTH_SIG_INVALID)?;
            Some(<[u8; 16]>::try_from(b).map_err(|_| AUTH_SIG_INVALID)?)
        }
        _ => return Err(AUTH_SIG_INVALID),
    };
    let envelope = OpEnvelope {
        v: wire
            .get("v")
            .and_then(Json::as_u64)
            .ok_or(AUTH_SIG_INVALID)?,
        ds: arr32(
            wire.get("ds")
                .and_then(Json::as_str)
                .ok_or(AUTH_SIG_INVALID)?,
        ),
        ep: wire
            .get("ep")
            .and_then(Json::as_u64)
            .ok_or(AUTH_SIG_INVALID)?,
        author: arr32(author),
        ts: OpTs {
            physical_ms: ts.get("p").and_then(Json::as_u64).ok_or(AUTH_SIG_INVALID)?,
            logical: ts.get("l").and_then(Json::as_u64).ok_or(AUTH_SIG_INVALID)? as u16,
        },
        deps,
        grp,
        kind: wire
            .get("kind")
            .and_then(Json::as_u64)
            .ok_or(AUTH_SIG_INVALID)?,
        body: json_to_cbor_body(wire.get("body").unwrap_or(&Json::Null))
            .map_err(|_| AUTH_SIG_INVALID)?,
    };
    let computed = envelope.op_id().map_err(|_| AUTH_SIG_INVALID)?;
    if bytes_to_hex(&computed) != id {
        return Err(AUTH_SIG_INVALID);
    }
    if !verify_op(&pk, &computed, &arr64(sig_hex)) {
        return Err(AUTH_SIG_INVALID);
    }
    Ok(())
}

fn belongs(wire: &Json, expected: &str) -> bool {
    let ds = wire["ds"].as_str().unwrap_or("");
    if wire["kind"].as_u64() == Some(0) {
        ds == ZERO_DS
    } else {
        ds == expected
    }
}

struct PeerModel {
    wall: u64,
    schema_epoch: u64,
    seen: BTreeSet<String>,
    lww: BTreeMap<(String, String), String>,
}

impl PeerModel {
    fn ingest(&mut self, wire: &Json, expected: &str) -> String {
        if let Err(reason) = verify_wire(wire) {
            return reason.to_string();
        }
        let id = wire["id"].as_str().unwrap().to_string();
        if self.seen.contains(&id) {
            return "duplicate".into();
        }
        if !belongs(wire, expected) {
            return AUTH_WRONG_DATASTORE.to_string();
        }
        let ts_p = wire["ts"]["p"].as_u64().unwrap();
        if ts_p > self.wall + MAX_DRIFT_MS {
            return CLOCK_DRIFT.to_string();
        }
        if wire["kind"].as_u64() == Some(5)
            && let Err(err) = validate_schema_epoch_body(&wire["body"])
        {
            return err.to_string();
        }
        let ep = wire["ep"].as_u64().unwrap_or(0);
        if ep > self.schema_epoch {
            return EPOCH_UNKNOWN.to_string();
        }
        self.seen.insert(id);
        if wire["kind"].as_u64() == Some(5) {
            let epoch = wire["body"]["epoch"].as_u64().unwrap();
            if self.schema_epoch == 0 || epoch >= self.schema_epoch {
                self.schema_epoch = epoch;
            }
        }
        if wire["kind"].as_u64() == Some(3) && wire["body"]["crdt"] == "lww" {
            let node = wire["body"]["node"].as_str().unwrap().to_string();
            let path = wire["body"]["path"].as_str().unwrap().to_string();
            let value = wire["body"]["value"].as_str().unwrap().to_string();
            self.lww.insert((node, path), value);
        }
        "applied".into()
    }
}

/// Same genesis / SchemaEpoch / data ranking as TS `epochFirst` (`store.mjs`).
fn batch_apply_rank(kind: u64) -> u8 {
    match kind {
        0 => 0,
        5 => 1,
        _ => 2,
    }
}

fn epoch_first(ops: &[Json]) -> Vec<Json> {
    let mut indexed: Vec<(usize, Json)> = ops.iter().cloned().enumerate().collect();
    indexed.sort_by(|a, b| {
        let ra = batch_apply_rank(a.1["kind"].as_u64().unwrap_or(u64::MAX));
        let rb = batch_apply_rank(b.1["kind"].as_u64().unwrap_or(u64::MAX));
        ra.cmp(&rb).then(a.0.cmp(&b.0))
    });
    indexed.into_iter().map(|(_, op)| op).collect()
}

fn run_vector(v: &Json, path: &Path) {
    assert_eq!(v["type"], "peer-ingest", "{}", path.display());
    let wall = v["clock"].as_u64().unwrap_or(1_700_000_000_000);
    let ds = v["datastore"].as_str().unwrap();
    let expected = v
        .get("expected_ds")
        .and_then(Json::as_str)
        .unwrap_or(ds)
        .to_string();
    let mut peer = PeerModel {
        wall,
        schema_epoch: 0,
        seen: BTreeSet::new(),
        lww: BTreeMap::new(),
    };
    if let Some(setup) = v["setup"].as_array() {
        for w in epoch_first(setup) {
            let r = peer.ingest(w, &expected);
            assert!(
                r == "applied" || r == "duplicate",
                "{} setup {}: {r}",
                path.display(),
                w["id"]
            );
        }
    }
    let mut got = Vec::new();
    if let Some(ingest) = v["ingest"].as_array() {
        for w in ingest {
            let r = peer.ingest(w, &expected);
            got.push((w["id"].as_str().unwrap().to_string(), r));
        }
    }
    let expect = &v["expect"];
    if let Some(want_ep) = expect.get("schema_epoch").and_then(Json::as_u64) {
        assert_eq!(
            peer.schema_epoch,
            want_ep,
            "{} schema_epoch",
            path.display()
        );
    }
    if let Some(want) = expect.get("applied").and_then(Json::as_array) {
        let applied: Vec<String> = got
            .iter()
            .filter(|(_, r)| r == "applied")
            .map(|(id, _)| id.clone())
            .collect();
        let want: Vec<String> = want
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        assert_eq!(applied, want, "{} applied", path.display());
    }
    if let Some(want) = expect.get("rejects").and_then(Json::as_array) {
        let rejects: Vec<Json> = got
            .iter()
            .filter(|(_, r)| r != "applied" && r != "duplicate")
            .map(|(id, r)| serde_json::json!({ "op_id": id, "reason": r }))
            .collect();
        assert_eq!(rejects, *want, "{} rejects", path.display());
    }
    if let Some(rows) = expect.get("lww").and_then(Json::as_array) {
        for row in rows {
            let key = (
                row["node"].as_str().unwrap().to_string(),
                row["path"].as_str().unwrap().to_string(),
            );
            let got = peer.lww.get(&key).map(String::as_str);
            assert_eq!(
                got,
                row["value"].as_str(),
                "{} lww {:?}",
                path.display(),
                key
            );
        }
    }
}

#[test]
fn epoch_first_ranks_genesis_then_schema_then_data() {
    let ops = vec![
        serde_json::json!({"id": "d", "kind": 3}),
        serde_json::json!({"id": "s", "kind": 5}),
        serde_json::json!({"id": "g", "kind": 0}),
        serde_json::json!({"id": "d2", "kind": 3}),
    ];
    let ordered: Vec<&str> = epoch_first(&ops)
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert_eq!(ordered, vec!["g", "s", "d", "d2"]);
}

#[test]
fn peer_ingest_vectors() {
    // Blocking lane only. Demonstrated-red xfail is the TS `--lane xfail` job
    // (exit 0); a red fixture here would fail `cargo test`.
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let dir = vectors.join("required").join("peer");
    let mut ran = 0;
    for entry in fs::read_dir(&dir).expect("required/peer") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let vector: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        run_vector(&vector, &path);
        ran += 1;
    }
    assert!(ran > 0, "no peer-ingest vectors under {dir:?}");
}
