//! Rust side of M0d auth vectors: device-cert, genesis-id, authz-predicate (doc/AUTH.md).

use std::fs;
use std::path::PathBuf;

use serde_json::Value as Json;
use zerodb_core::auth::{
    AdmissionToken, AuthError, AuthzBody, AuthzOp, DeviceCert, KnownGrant,
    admission_token_preimage_body, auth_error_tag, authorize, cert_preimage_body,
    datastore_id_from_genesis, peer_id, principal_id, verify_admission_token, verify_device_cert,
};
use zerodb_core::cbor::Cbor;
use zerodb_core::op::{OpEnvelope, OpTs};

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

fn tagged_to_cbor(v: &Json) -> Cbor {
    match v["t"].as_str().unwrap() {
        "null" => Cbor::Null,
        "bool" => Cbor::Bool(v["v"].as_bool().unwrap()),
        "uint" => Cbor::Uint(v["v"].as_u64().unwrap()),
        "text" => Cbor::Text(v["v"].as_str().unwrap().to_owned()),
        "bytes" => Cbor::Bytes(hex_to_bytes(v["hex"].as_str().unwrap())),
        "array" => Cbor::Array(
            v["v"]
                .as_array()
                .unwrap()
                .iter()
                .map(tagged_to_cbor)
                .collect(),
        ),
        "map" => Cbor::Map(
            v["v"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, tv)| (k.clone(), tagged_to_cbor(tv)))
                .collect(),
        ),
        other => panic!("unknown tagged type {other}"),
    }
}

fn parse_cert(v: &Json) -> DeviceCert {
    let expiry = match &v["expiry"] {
        Json::Null => None,
        other => Some(other.as_u64().expect("expiry uint")),
    };
    let revoke_of = match &v["revoke_of"] {
        Json::Null => None,
        other => Some(arr32(other.as_str().expect("revoke_of hex"))),
    };
    DeviceCert {
        kr: v["kr"].as_u64().unwrap(),
        device_pk: arr32(v["device"].as_str().unwrap()),
        principal_id: arr32(v["principal"].as_str().unwrap()),
        root_pk: arr32(v["root_pk"].as_str().unwrap()),
        issued: v["issued"].as_u64().unwrap(),
        expiry,
        revoke_of,
        cert_sig: arr64(v["cert_sig"].as_str().unwrap()),
    }
}

fn parse_envelope(op: &Json) -> OpEnvelope {
    let deps = op["deps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| arr32(d.as_str().unwrap()))
        .collect();
    let grp = match &op["grp"] {
        Json::Null => None,
        other => Some({
            let b = hex_to_bytes(other.as_str().unwrap());
            let arr: [u8; 16] = b.try_into().expect("16-byte group");
            arr
        }),
    };
    OpEnvelope {
        v: op["v"].as_u64().unwrap(),
        ds: arr32(op["ds"].as_str().unwrap()),
        ep: op["ep"].as_u64().unwrap(),
        author: arr32(op["author"].as_str().unwrap()),
        ts: OpTs {
            physical_ms: op["ts"]["p"].as_u64().unwrap(),
            logical: op["ts"]["l"].as_u64().unwrap() as u16,
        },
        deps,
        grp,
        kind: op["kind"].as_u64().unwrap(),
        body: tagged_to_cbor(&op["body"]),
    }
}

fn parse_authz_body(b: &Json) -> AuthzBody {
    match b["type"].as_str().unwrap() {
        "genesis" => AuthzBody::Genesis {
            founder: arr32(b["founder"].as_str().unwrap()),
        },
        "grant" => {
            let expiry = match &b["expiry"] {
                Json::Null => None,
                other => Some(other.as_u64().unwrap()),
            };
            AuthzBody::Grant {
                subject: arr32(b["subject"].as_str().unwrap()),
                scopes: b["scopes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_u64().unwrap())
                    .collect(),
                expiry,
                delegable: b["delegable"].as_bool().unwrap(),
                ds_bind: arr32(b["ds_bind"].as_str().unwrap()),
            }
        }
        "revoke" => AuthzBody::Revoke {
            grant: arr32(b["grant"].as_str().unwrap()),
            reason: b["reason"].as_u64().unwrap(),
        },
        "other" => AuthzBody::Other,
        other => panic!("unknown authz body type {other}"),
    }
}

fn parse_authz_op(o: &Json) -> AuthzOp {
    AuthzOp {
        id: arr32(o["id"].as_str().unwrap()),
        kind: o["kind"].as_u64().unwrap(),
        author: arr32(o["author"].as_str().unwrap()),
        principal: arr32(o["principal"].as_str().unwrap()),
        ds: arr32(o["ds"].as_str().unwrap()),
        deps: o["deps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| arr32(d.as_str().unwrap()))
            .collect(),
        ts_physical_ms: o["ts_p"].as_u64().unwrap(),
        body: parse_authz_body(&o["body"]),
    }
}

fn vector_files() -> Vec<PathBuf> {
    let vectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../conformance/vectors");
    let mut files = Vec::new();
    for lane in ["required", "xfail"] {
        let dir = vectors.join(lane).join("auth");
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

fn run_device_cert(v: &Json, path: &std::path::Path) {
    let cert = parse_cert(&v["cert"]);
    let body = cert_preimage_body(&cert).unwrap();
    if let Some(exp) = v["preimage_body_hex"].as_str() {
        assert_eq!(bytes_to_hex(&body), exp, "body {}", path.display());
    }
    if let Some(exp) = v["peer_id_hex"].as_str() {
        assert_eq!(
            bytes_to_hex(&peer_id(&cert.device_pk)),
            exp,
            "peer {}",
            path.display()
        );
    }
    if let Some(exp) = v["principal_id_hex"].as_str() {
        assert_eq!(
            bytes_to_hex(&principal_id(&cert.root_pk)),
            exp,
            "principal {}",
            path.display()
        );
    }
    let expect = v["expect"].as_str().unwrap();
    match verify_device_cert(&cert) {
        Ok(()) => assert_eq!(expect, "ok", "{}", path.display()),
        Err(e) => assert_eq!(auth_error_tag(&e), expect, "{}", path.display()),
    }
}

fn run_genesis_id(v: &Json, path: &std::path::Path) {
    let env = parse_envelope(&v["op"]);
    let expect = v["expect"].as_str().unwrap();
    match datastore_id_from_genesis(&env) {
        Ok(ds) => {
            assert_eq!(expect, "ok", "{}", path.display());
            if let Some(exp) = v["datastore_id_hex"].as_str() {
                assert_eq!(bytes_to_hex(&ds), exp, "ds {}", path.display());
            }
            if let Some(exp) = v["preimage_body_hex"].as_str() {
                assert_eq!(
                    bytes_to_hex(&env.id_preimage_body().unwrap()),
                    exp,
                    "pre {}",
                    path.display()
                );
            }
            if let Some(exp) = v["op_id_hex"].as_str() {
                assert_eq!(
                    bytes_to_hex(&env.op_id().unwrap()),
                    exp,
                    "op_id {}",
                    path.display()
                );
            }
            if let Some(other) = v["different_from_hex"].as_str() {
                assert_ne!(
                    bytes_to_hex(&ds),
                    other,
                    "salt-sensitive collision {}",
                    path.display()
                );
            }
        }
        Err(e) => assert_eq!(auth_error_tag(&e), expect, "{}", path.display()),
    }
}

fn run_authz(v: &Json, path: &std::path::Path) {
    let ds = arr32(v["datastore_id_hex"].as_str().unwrap());
    let applied: Vec<AuthzOp> = v["applied"]
        .as_array()
        .unwrap()
        .iter()
        .map(parse_authz_op)
        .collect();
    let candidate = parse_authz_op(&v["candidate"]);
    let expect = v["expect"].as_str().unwrap();
    match authorize(&ds, &applied, &candidate) {
        Ok(()) => assert_eq!(expect, "ok", "{}", path.display()),
        Err(e) => assert_eq!(auth_error_tag(&e), expect, "{}", path.display()),
    }
}

fn run_admission(v: &Json, path: &std::path::Path) {
    let tok_j = &v["token"];
    let cert = parse_cert(&tok_j["cert"]);
    let scopes = tok_j["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_u64().unwrap())
        .collect();
    let tok = AdmissionToken {
        ds: arr32(tok_j["ds"].as_str().unwrap()),
        subject: arr32(tok_j["subject"].as_str().unwrap()),
        grant: arr32(tok_j["grant"].as_str().unwrap()),
        scopes,
        device: arr32(tok_j["device"].as_str().unwrap()),
        cert,
        sig: arr64(tok_j["sig"].as_str().unwrap()),
    };
    if let Some(exp) = v["preimage_body_hex"].as_str() {
        assert_eq!(
            bytes_to_hex(&admission_token_preimage_body(&tok).unwrap()),
            exp,
            "token body {}",
            path.display()
        );
    }
    let grants: Vec<KnownGrant> = v["known_grants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| KnownGrant {
            id: arr32(g["id"].as_str().unwrap()),
            ds: arr32(g["ds"].as_str().unwrap()),
            subject: arr32(g["subject"].as_str().unwrap()),
            scopes: g["scopes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_u64().unwrap())
                .collect(),
            expiry: g.get("expiry").and_then(Json::as_u64),
            revoked: g["revoked"].as_bool().unwrap(),
        })
        .collect();
    let expect = v["expect"].as_str().unwrap();
    match verify_admission_token(&tok, &grants) {
        Ok(()) => assert_eq!(expect, "ok", "{}", path.display()),
        Err(e) => assert_eq!(auth_error_tag(&e), expect, "{}", path.display()),
    }
}

#[test]
fn auth_vectors() {
    let files = vector_files();
    assert!(
        !files.is_empty(),
        "no auth vectors under conformance/vectors/*/auth"
    );
    for path in files {
        let raw = fs::read_to_string(&path).unwrap();
        let v: Json = serde_json::from_str(&raw).unwrap();
        match v["type"].as_str().unwrap() {
            "device-cert" => run_device_cert(&v, &path),
            "genesis-id" => run_genesis_id(&v, &path),
            "authz-predicate" => run_authz(&v, &path),
            "admission-token" => run_admission(&v, &path),
            other => panic!("{}: unknown type {other}", path.display()),
        }
    }
}

// silence unused import warning if AuthError only used via auth_error_tag
#[allow(dead_code)]
fn _touch_auth_error(_: AuthError) {}
