//! EXEMPLAR E8: far-future clock abuse quarantined.
//!
//! C is a member whose clock is +30 days. A signed LWW from C must not
//! silently win on honest peers A/B. Named outcome: `CLOCK_DRIFT` (held in
//! AUTH.md §6 quarantine, HLC not advanced). After the window resolves, A, B,
//! and C converge on the same materialized LWW (I-1). I-4/I-5 stay true.
//! Honest and colluding relays both deliver the op; peers enforce H1.

use ed25519_dalek::{Signer, SigningKey};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::op::{OpEnvelope, OpTs, json_to_cbor_body};
use zerodb_core::relay::{
    MSG_AUTH, MSG_HELLO, MSG_OP_ACK, MSG_OPS, peer_id_from_pk, sign_auth_for_hello,
};
use zerodb_core::sign::{DOMAIN_OP_SIG, sign_op};
use zerodb_relay::{Relay, RelaySession};
use zerodb_storage::relay_client;
use zerodb_storage::{
    APPLY_INVALID, CLOCK_DRIFT, ExportBundle, IngestResult, LocalStore, MAX_CLOCK_DRIFT_MS,
    MemoryBackend, StoreBackend, StoreError, WireOp, WireTs,
};

const PLUS_30D_MS: u64 = 30 * 24 * 60 * 60 * 1000;

fn clock_plus_30d() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + PLUS_30D_MS
}

fn auth_store() -> LocalStore<MemoryBackend> {
    LocalStore::init_auth_with_backend(MemoryBackend::new()).unwrap()
}

fn empty_store() -> LocalStore<MemoryBackend> {
    LocalStore::init_with_backend(MemoryBackend::new()).unwrap()
}

fn last_kind(store: &LocalStore<MemoryBackend>, kind: u64) -> WireOp {
    store
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .rev()
        .find(|op| op.kind == kind)
        .expect("kind present")
}

fn ds_bytes(store: &LocalStore<impl StoreBackend>) -> [u8; 32] {
    hex::decode(store.datastore_id_hex())
        .unwrap()
        .try_into()
        .unwrap()
}

fn control_deps(store: &LocalStore<impl StoreBackend>) -> Vec<String> {
    store
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .filter(|op| matches!(op.kind, 0 | 5 | 6 | 7 | 8))
        .map(|op| op.id)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn sign_lww(
    seed: &[u8; 32],
    ds: &[u8; 32],
    ep: u64,
    deps: &[String],
    physical_ms: u64,
    node: &str,
    path: &str,
    value: &str,
) -> WireOp {
    let signing = SigningKey::from_bytes(seed);
    let author_pk = signing.verifying_key().to_bytes();
    let author = *blake3::hash(&author_pk).as_bytes();
    let dep_ids = deps
        .iter()
        .map(|dep| hex::decode(dep).unwrap().try_into().unwrap())
        .collect::<Vec<[u8; 32]>>();
    let body_json = serde_json::json!({
        "node": node, "path": path, "crdt": "lww", "value": value
    });
    let body = json_to_cbor_body(&body_json).unwrap();
    let envelope = OpEnvelope {
        v: 1,
        ds: *ds,
        ep,
        author,
        ts: OpTs {
            physical_ms,
            logical: 0,
        },
        deps: dep_ids,
        grp: None,
        kind: 3,
        body,
    };
    let id = envelope.op_id().unwrap();
    let pre = [DOMAIN_OP_SIG, id.as_slice()].concat();
    let sig = signing.sign(&pre).to_bytes();
    WireOp {
        id: hex::encode(id),
        v: 1,
        ds: hex::encode(ds),
        ep,
        author: hex::encode(author),
        author_pk: hex::encode(author_pk),
        ts: WireTs {
            p: physical_ms,
            l: 0,
        },
        deps: deps.to_vec(),
        grp: None,
        kind: 3,
        body: body_json,
        sig: hex::encode(sig),
    }
}

fn last_set_by(store: &LocalStore<MemoryBackend>, author_hex: &str) -> WireOp {
    store
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .rev()
        .find(|op| op.kind == 3 && op.author == author_hex)
        .expect("author SetProperty")
}

fn assert_quarantined(result: IngestResult) {
    match result {
        IngestResult::Quarantined { reason } => assert_eq!(reason, CLOCK_DRIFT),
        other => panic!("expected CLOCK_DRIFT quarantine, got {other:?}"),
    }
}

fn e8_topology() -> (
    LocalStore<MemoryBackend>,
    LocalStore<MemoryBackend>,
    LocalStore<MemoryBackend>,
    String,
    WireOp,
) {
    let mut a = auth_store();
    let mut b = empty_store();
    let mut c = empty_store();
    a.grant_write_access(&b.principal_hex()).unwrap();
    a.grant_write_access(&c.principal_hex()).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    c.import_bundle(&a.export_all().unwrap()).unwrap();

    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "from-a").unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    c.import_bundle(&a.export_all().unwrap()).unwrap();
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );
    assert_eq!(
        c.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );

    c.set_test_clock(clock_plus_30d);
    c.set_lww(&node, "title", "poison").unwrap();
    assert_eq!(
        c.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
    let poison = last_set_by(&c, &c.author_hex());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert!(
        poison.ts.p >= now.saturating_add(PLUS_30D_MS - MAX_CLOCK_DRIFT_MS),
        "C's write must carry the +30d clock"
    );
    (a, b, c, node, poison)
}

fn wire_to_relay(wire: &WireOp) -> Cbor {
    Cbor::Map(vec![
        ("op_id".into(), Cbor::Bytes(hex::decode(&wire.id).unwrap())),
        (
            "author".into(),
            Cbor::Bytes(hex::decode(&wire.author).unwrap()),
        ),
        ("physical_ms".into(), Cbor::Uint(wire.ts.p)),
        ("logical".into(), Cbor::Uint(wire.ts.l as u64)),
        (
            "wire".into(),
            Cbor::Text(serde_json::to_string(wire).unwrap()),
        ),
    ])
}

fn encode_env(ty: u8, request_id: u32, payload: Cbor) -> Vec<u8> {
    cbor::encode(&Cbor::Map(vec![
        ("type".into(), Cbor::Uint(ty as u64)),
        ("request_id".into(), Cbor::Uint(request_id as u64)),
        ("payload".into(), payload),
    ]))
    .unwrap()
}

fn map_get<'a>(value: &'a Cbor, key: &str) -> Option<&'a Cbor> {
    match value {
        Cbor::Map(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, v)| v),
        _ => None,
    }
}

fn as_u64(value: &Cbor) -> u64 {
    match value {
        Cbor::Uint(n) => *n,
        _ => panic!("not uint"),
    }
}

fn as_bytes(value: &Cbor) -> &[u8] {
    match value {
        Cbor::Bytes(b) => b,
        _ => panic!("not bytes"),
    }
}

fn handshake(session: &mut RelaySession, seed: &[u8; 32]) {
    let key = SigningKey::from_bytes(seed);
    let public_key = key.verifying_key().to_bytes();
    let hello = encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            (
                "peer_id".into(),
                Cbor::Bytes(peer_id_from_pk(&public_key).to_vec()),
            ),
            ("public_key".into(), Cbor::Bytes(public_key.to_vec())),
            ("protocol_version".into(), Cbor::Uint(1)),
            ("capabilities".into(), Cbor::Array(vec![])),
        ]),
    );
    let challenge = session.handle(&hello).unwrap();
    let env = cbor::decode(&challenge[0]).unwrap();
    let nonce =
        as_bytes(map_get(map_get(&env, "payload").expect("payload"), "nonce").expect("nonce"));
    let mut n = [0u8; 32];
    n.copy_from_slice(nonce);
    let signature = sign_auth_for_hello(seed, &public_key, &[] as &[&str], &n);
    let auth = encode_env(
        MSG_AUTH,
        1,
        Cbor::Map(vec![("signature".into(), Cbor::Bytes(signature.to_vec()))]),
    );
    session.handle(&auth).unwrap();
}

fn submit_ops(session: &mut RelaySession, ds: &str, ops: Vec<Cbor>) -> Vec<Cbor> {
    let frame = encode_env(
        MSG_OPS,
        9,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(ds.into())),
            ("operations".into(), Cbor::Array(ops)),
        ]),
    );
    let reply = session.handle(&frame).unwrap();
    let env = cbor::decode(&reply[0]).unwrap();
    assert_eq!(
        as_u64(map_get(&env, "type").expect("type")) as u8,
        MSG_OP_ACK
    );
    match map_get(map_get(&env, "payload").expect("payload"), "outcomes") {
        Some(Cbor::Array(items)) => items.clone(),
        _ => panic!("outcomes"),
    }
}

fn outcome_of(item: &Cbor) -> (String, Option<String>) {
    let outcome = match map_get(item, "outcome") {
        Some(Cbor::Text(s)) => s.clone(),
        _ => panic!("outcome"),
    };
    let reason = match map_get(item, "reason") {
        Some(Cbor::Text(s)) => Some(s.clone()),
        _ => None,
    };
    (outcome, reason)
}

fn drive<B: StoreBackend>(
    store: &mut LocalStore<B>,
    sess: &mut RelaySession,
    join: Option<&str>,
) -> relay_client::RelaySyncSummary {
    relay_client::sync(store, join, |frame| {
        Ok::<_, StoreError>(sess.handle(frame).unwrap())
    })
    .unwrap()
}

fn signed_member_wire(
    store: &LocalStore<MemoryBackend>,
    kind: u64,
    body: serde_json::Value,
    deps: Vec<String>,
    physical_ms: u64,
    logical: u16,
) -> WireOp {
    let seed = store.identity_seed();
    let ds: [u8; 32] = hex::decode(store.datastore_id_hex())
        .unwrap()
        .try_into()
        .unwrap();
    let (author_pk, _) = sign_op(&seed, &[0; 32]);
    let author = *blake3::hash(&author_pk).as_bytes();
    let dep_ids = deps
        .iter()
        .map(|dep| hex::decode(dep).unwrap().try_into().unwrap())
        .collect();
    let envelope = OpEnvelope {
        v: 1,
        ds,
        ep: store.schema_epoch().unwrap(),
        author,
        ts: OpTs {
            physical_ms,
            logical,
        },
        deps: dep_ids,
        grp: None,
        kind,
        body: json_to_cbor_body(&body).unwrap(),
    };
    let id = envelope.op_id().unwrap();
    let (_, sig) = sign_op(&seed, &id);
    WireOp {
        id: hex::encode(id),
        v: 1,
        ds: store.datastore_id_hex(),
        ep: store.schema_epoch().unwrap(),
        author: hex::encode(author),
        author_pk: hex::encode(author_pk),
        ts: WireTs {
            p: physical_ms,
            l: logical,
        },
        deps,
        grp: None,
        kind,
        body,
        sig: hex::encode(sig),
    }
}

fn far_future_create_and_set(
    store: &LocalStore<MemoryBackend>,
    label: &str,
    title: &str,
) -> (String, WireOp, WireOp) {
    let ts = clock_plus_30d();
    let node = hex::encode([0x11u8; 16]);
    let control: Vec<String> = store
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .filter(|op| matches!(op.kind, 0 | 6 | 7))
        .map(|op| op.id)
        .collect();
    let create = signed_member_wire(
        store,
        1,
        serde_json::json!({ "label": label, "node": node }),
        control.clone(),
        ts,
        0,
    );
    let mut set_deps = control;
    set_deps.push(create.id.clone());
    let set = signed_member_wire(
        store,
        3,
        serde_json::json!({
            "node": node,
            "path": "title",
            "crdt": "lww",
            "value": title,
        }),
        set_deps,
        ts,
        1,
    );
    (node, create, set)
}

fn assert_held_pair(store: &LocalStore<MemoryBackend>, create: &WireOp, set: &WireOp) {
    let held: Vec<String> = store
        .list_quarantine()
        .unwrap()
        .into_iter()
        .map(|wire| wire.id)
        .collect();
    assert!(
        held.contains(&create.id),
        "create must be held, got {held:?}"
    );
    assert!(held.contains(&set.id), "set must be held, got {held:?}");
    assert_eq!(held.len(), 2, "exactly the causal pair must be held");
    let node = create.body["node"].as_str().expect("create node");
    assert!(
        !store
            .list_nodes()
            .unwrap()
            .iter()
            .any(|(id, _, _)| id == node),
        "held create must not materialize the node"
    );
    assert!(
        store.get_lww(node, "title").unwrap().is_none(),
        "held set must not materialize the property"
    );
}

fn resolve_window(
    a: &mut LocalStore<MemoryBackend>,
    b: &mut LocalStore<MemoryBackend>,
    poison_id: &str,
) {
    a.set_test_clock(clock_plus_30d);
    b.set_test_clock(clock_plus_30d);
    let released_a = a.release_quarantine().unwrap();
    let released_b = b.release_quarantine().unwrap();
    assert!(
        released_a.iter().any(|id| id == poison_id),
        "A must apply the quarantined op after the window"
    );
    assert!(
        released_b.iter().any(|id| id == poison_id),
        "B must apply the quarantined op after the window"
    );
}

#[test]
fn e8_peer_quarantine_then_converge() {
    let (mut a, mut b, c, node, poison) = e8_topology();
    let hlc_before = last_kind(&a, 3).ts.p;

    assert_quarantined(a.ingest_op(&poison).unwrap());
    assert_quarantined(b.ingest_op(&poison).unwrap());
    assert_eq!(
        a.take_rejects()
            .iter()
            .map(|r| r.reason)
            .collect::<Vec<_>>(),
        vec![CLOCK_DRIFT]
    );
    assert_eq!(a.list_quarantine().unwrap().len(), 1);
    assert_eq!(b.list_quarantine().unwrap().len(), 1);
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );
    assert_eq!(
        c.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );

    assert_quarantined(a.ingest_op(&poison).unwrap());
    assert_eq!(a.list_quarantine().unwrap().len(), 1);

    a.set_lww(&node, "title", "during").unwrap();
    let during = last_set_by(&a, &a.author_hex());
    assert!(
        during.ts.p + MAX_CLOCK_DRIFT_MS < poison.ts.p,
        "I-4: quarantining C must not poison A's issued timestamps"
    );
    assert!(
        during.ts.p >= hlc_before,
        "I-4: A's next local ts is still monotone"
    );
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("during")
    );
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );

    resolve_window(&mut a, &mut b, &poison.id);
    assert!(a.list_quarantine().unwrap().is_empty());
    assert!(b.list_quarantine().unwrap().is_empty());
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
    assert_eq!(
        c.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );

    a.set_lww(&node, "title", "post").unwrap();
    let post = last_set_by(&a, &a.author_hex());
    assert!(
        (post.ts.p, post.ts.l) > (poison.ts.p, poison.ts.l),
        "I-5: after release, next local ts is strictly above C's remote ts"
    );
    assert_eq!(a.ingest_op(&poison).unwrap(), IngestResult::Duplicate);
}

#[test]
fn e8_import_bundle_quarantines_far_future_lww() {
    let (mut a, mut b, c, node, _poison) = e8_topology();
    let (accepted, skipped) = a.import_bundle(&c.export_all().unwrap()).unwrap();
    assert_eq!(accepted, 0, "in-bound ops are already on A");
    assert!(
        skipped >= 1,
        "C's +30d LWW must be quarantined, not applied"
    );
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );
    assert_eq!(a.list_quarantine().unwrap().len(), 1);
    b.import_bundle(&c.export_all().unwrap()).unwrap();
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );

    let poison_id = a.list_quarantine().unwrap()[0].id.clone();
    resolve_window(&mut a, &mut b, &poison_id);
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
}

fn relay_roundtrip(
    relay: Relay,
    a: &mut LocalStore<MemoryBackend>,
    b: &mut LocalStore<MemoryBackend>,
    c: &mut LocalStore<MemoryBackend>,
    poison: &WireOp,
) {
    let ds = a.datastore_id_hex();
    let mut ra = relay.accept();
    let mut rb = relay.accept();
    let mut rc = relay.accept();
    drive(a, &mut ra, None);
    drive(b, &mut rb, Some(&ds));
    handshake(&mut rc, &c.identity_seed());
    let (outcome, reason) = outcome_of(&submit_ops(&mut rc, &ds, vec![wire_to_relay(poison)])[0]);
    assert_eq!(
        outcome, "ACCEPT",
        "relay persists well-formed member ops (clock is not an admission reject); reason={reason:?}"
    );
    drive(a, &mut ra, None);
    drive(b, &mut rb, None);
}

#[test]
fn e8_honest_relay_peers_quarantine_then_converge() {
    let (mut a, mut b, mut c, node, poison) = e8_topology();
    relay_roundtrip(Relay::memory(), &mut a, &mut b, &mut c, &poison);
    assert_eq!(a.list_quarantine().unwrap().len(), 1);
    assert_eq!(b.list_quarantine().unwrap().len(), 1);
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );
    resolve_window(&mut a, &mut b, &poison.id);
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
    assert_eq!(
        c.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
}

#[test]
fn e8_colluding_relay_peers_quarantine_then_converge() {
    let (mut a, mut b, mut c, node, poison) = e8_topology();
    relay_roundtrip(Relay::memory_colluding(), &mut a, &mut b, &mut c, &poison);
    assert_eq!(a.list_quarantine().unwrap().len(), 1);
    assert_eq!(b.list_quarantine().unwrap().len(), 1);
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );
    resolve_window(&mut a, &mut b, &poison.id);
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
    assert_eq!(
        c.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
}

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("e8-{name}-{nonce}.sqlite"));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    path
}

fn remove_sqlite(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

#[test]
fn e8_sqlite_quarantine_survives_reopen() {
    let path = tmp_db("reopen");
    let (node, poison) = {
        let mut a = LocalStore::init_auth(&path).unwrap();
        let mut c = empty_store();
        a.grant_write_access(&c.principal_hex()).unwrap();
        c.import_bundle(&a.export_all().unwrap()).unwrap();
        let node = a.create_node("Todo").unwrap();
        a.set_lww(&node, "title", "from-a").unwrap();
        c.import_bundle(&a.export_all().unwrap()).unwrap();
        c.set_test_clock(clock_plus_30d);
        c.set_lww(&node, "title", "poison").unwrap();
        let poison = last_set_by(&c, &c.author_hex());
        assert_quarantined(a.ingest_op(&poison).unwrap());
        assert_eq!(
            a.get_lww(&node, "title").unwrap().as_deref(),
            Some("from-a")
        );
        (node, poison)
    };

    let mut a = LocalStore::open(&path).unwrap();
    assert_eq!(a.list_quarantine().unwrap().len(), 1);
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("from-a")
    );
    a.set_test_clock(clock_plus_30d);
    let released = a.release_quarantine().unwrap();
    assert_eq!(released, vec![poison.id]);
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("poison")
    );
    remove_sqlite(&path);
}

#[test]
fn e8_unreleasable_schema_pin_does_not_wedge() {
    let mut a = auth_store();
    let mut c = empty_store();
    a.apply_schema_json(r#"{"nodes":{"Todo":{"props":{"title":"flag"}}}}"#)
        .unwrap();
    a.grant_write_access(&c.principal_hex()).unwrap();
    c.import_bundle(&a.export_all().unwrap()).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.flag_enable(&node, "title").unwrap();
    c.import_bundle(&a.export_all().unwrap()).unwrap();

    // Own-epoch pin: ep=1 flag IR rejects a crafted ep=1 lww. An ep=0
    // schemaless write would apply (SCHEMA.md §3 / P1-2).
    let poison = sign_lww(
        &c.identity_seed(),
        &ds_bytes(&a),
        1,
        &control_deps(&c),
        clock_plus_30d(),
        &node,
        "title",
        "poison",
    );
    assert_quarantined(a.ingest_op(&poison).unwrap());
    a.set_test_clock(clock_plus_30d);
    let released = a.release_quarantine().unwrap();
    assert!(released.is_empty(), "unreleasable entry must not apply");
    assert!(a.list_quarantine().unwrap().is_empty());
    assert!(
        a.take_rejects()
            .iter()
            .any(|r| r.op_id == poison.id && r.reason == APPLY_INVALID),
        "own-epoch schema-pin miss must be a named APPLY_INVALID reject"
    );
    assert_eq!(
        a.get_prop(&node, "title").unwrap(),
        Some(serde_json::json!(true))
    );
    let note = a.create_node("Note").unwrap();
    a.set_lww(&note, "body", "ok").unwrap();
    assert_eq!(a.get_lww(&note, "body").unwrap().as_deref(), Some("ok"));
}

#[test]
fn e8_unreleasable_survives_sqlite_reopen_without_wedge() {
    let path = tmp_db("unreleasable");
    let (node, poison) = {
        let mut a = LocalStore::init_auth(&path).unwrap();
        let mut c = empty_store();
        a.apply_schema_json(r#"{"nodes":{"Todo":{"props":{"title":"flag"}}}}"#)
            .unwrap();
        a.grant_write_access(&c.principal_hex()).unwrap();
        c.import_bundle(&a.export_all().unwrap()).unwrap();
        let node = a.create_node("Todo").unwrap();
        a.flag_enable(&node, "title").unwrap();
        c.import_bundle(&a.export_all().unwrap()).unwrap();
        let poison = sign_lww(
            &c.identity_seed(),
            &ds_bytes(&a),
            1,
            &control_deps(&c),
            clock_plus_30d(),
            &node,
            "title",
            "poison",
        );
        assert_quarantined(a.ingest_op(&poison).unwrap());
        (node, poison)
    };

    let mut a = LocalStore::open(&path).unwrap();
    assert_eq!(a.list_quarantine().unwrap().len(), 1);
    a.set_test_clock(clock_plus_30d);
    let released = a.release_quarantine().unwrap();
    assert!(released.is_empty(), "unreleasable entry must not apply");
    assert!(a.list_quarantine().unwrap().is_empty());
    assert!(
        a.take_rejects()
            .iter()
            .any(|r| r.op_id == poison.id && r.reason == APPLY_INVALID),
        "own-epoch schema-pin miss must stay APPLY_INVALID after sqlite reopen"
    );
    assert_eq!(
        a.get_prop(&node, "title").unwrap(),
        Some(serde_json::json!(true))
    );
    let note = a.create_node("Note").unwrap();
    a.set_lww(&note, "body", "ok").unwrap();
    assert_eq!(a.get_lww(&note, "body").unwrap().as_deref(), Some("ok"));
    remove_sqlite(&path);
}

#[test]
fn e8_import_bundle_holds_causal_descendants() {
    let (mut a, mut b, _c, _node, _poison) = e8_topology();
    let (node, create, set) = far_future_create_and_set(&a, "Linked", "linked");
    let bundle = ExportBundle {
        format: 1,
        datastore_id: a.datastore_id_hex(),
        ops: vec![create.clone(), set.clone()],
    };
    a.import_bundle(&bundle).unwrap();
    b.import_bundle(&bundle).unwrap();
    assert_held_pair(&a, &create, &set);
    assert_held_pair(&b, &create, &set);
    resolve_window(&mut a, &mut b, &create.id);
    assert!(a.list_quarantine().unwrap().is_empty());
    assert!(b.list_quarantine().unwrap().is_empty());
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("linked")
    );
    assert_eq!(
        b.get_lww(&node, "title").unwrap().as_deref(),
        Some("linked")
    );
}

#[test]
fn e8_commit_wires_atomic_holds_causal_descendants() {
    let (mut a, _b, _c, _node, _poison) = e8_topology();
    let (node, create, set) = far_future_create_and_set(&a, "Atomic", "atomic");
    a.commit_wires_atomic(&[create.clone(), set.clone()])
        .unwrap();
    assert_held_pair(&a, &create, &set);
    a.set_test_clock(clock_plus_30d);
    let released = a.release_quarantine().unwrap();
    assert!(released.contains(&create.id));
    assert!(released.contains(&set.id));
    assert!(a.list_quarantine().unwrap().is_empty());
    assert_eq!(
        a.get_lww(&node, "title").unwrap().as_deref(),
        Some("atomic")
    );
}
