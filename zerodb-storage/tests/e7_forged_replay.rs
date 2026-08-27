//! EXEMPLAR E7: forged and replayed operations rejected.
//!
//! Peer-side KERNEL §4.4 / AUTH.md authenticity is load-bearing. Named
//! outcomes: `AUTH_SIG_INVALID` (flipped payload, C-signed claiming B) and
//! `Duplicate` / wire `DUPLICATE` (byte-exact replay, including after the
//! relay's dedup state is wiped). A colluding relay that forwards everything
//! still cannot change materialized state on honest peers.

use ed25519_dalek::Signer;
use ed25519_dalek::SigningKey;
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::op::{OpEnvelope, OpTs};
use zerodb_core::relay::{
    DOMAIN_RELAY_AUTH, MSG_AUTH, MSG_HELLO, MSG_OP_ACK, MSG_OPS, peer_id_from_pk,
};
use zerodb_core::sign::{DOMAIN_OP_SIG, sign_op};
use zerodb_relay::{Relay, RelaySession};
use zerodb_storage::relay_client;
use zerodb_storage::{
    IngestResult, LocalStore, MemoryBackend, StoreBackend, StoreError, WireOp, WireTs,
};

fn auth_store() -> LocalStore<MemoryBackend> {
    LocalStore::init_auth_with_backend(MemoryBackend::new()).unwrap()
}

fn empty_store() -> LocalStore<MemoryBackend> {
    LocalStore::init_with_backend(MemoryBackend::new()).unwrap()
}

fn ds_bytes(store: &LocalStore<MemoryBackend>) -> [u8; 32] {
    hex::decode(store.datastore_id_hex())
        .unwrap()
        .try_into()
        .unwrap()
}

fn genesis_id(store: &LocalStore<MemoryBackend>) -> String {
    store
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 0)
        .expect("genesis")
        .id
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

fn signed_create(
    seed: &[u8; 32],
    ds: &[u8; 32],
    deps: &[String],
    physical_ms: u64,
    label: &str,
) -> WireOp {
    let (author_pk, _) = sign_op(seed, &[0; 32]);
    let author = *blake3::hash(&author_pk).as_bytes();
    let node = [0x47u8; 16];
    let body_json = serde_json::json!({
        "label": label,
        "node": hex::encode(node),
    });
    let dep_ids = deps
        .iter()
        .map(|dep| hex::decode(dep).unwrap().try_into().unwrap())
        .collect::<Vec<[u8; 32]>>();
    let envelope = OpEnvelope {
        v: 1,
        ds: *ds,
        ep: 0,
        author,
        ts: OpTs {
            physical_ms,
            logical: 0,
        },
        deps: dep_ids,
        grp: None,
        kind: 1,
        body: Cbor::Map(vec![
            ("label".into(), Cbor::Text(label.into())),
            ("node".into(), Cbor::Bytes(node.to_vec())),
        ]),
    };
    let id = envelope.op_id().unwrap();
    let sig = {
        let pre = [DOMAIN_OP_SIG, id.as_slice()].concat();
        SigningKey::from_bytes(seed).sign(&pre).to_bytes()
    };
    WireOp {
        id: hex::encode(id),
        v: 1,
        ds: hex::encode(ds),
        ep: 0,
        author: hex::encode(author),
        author_pk: hex::encode(author_pk),
        ts: WireTs {
            p: physical_ms,
            l: 0,
        },
        deps: deps.to_vec(),
        grp: None,
        kind: 1,
        body: body_json,
        sig: hex::encode(sig),
    }
}

fn flip_payload_byte(wire: &WireOp) -> WireOp {
    let mut tampered = wire.clone();
    let label = tampered
        .body
        .get("label")
        .and_then(|v| v.as_str())
        .or_else(|| tampered.body.get("path").and_then(|v| v.as_str()))
        .expect("text payload field");
    let mut bytes = label.as_bytes().to_vec();
    bytes[0] ^= 0x01;
    let flipped = String::from_utf8(bytes).expect("still utf8");
    if tampered.body.get("label").is_some() {
        tampered.body["label"] = serde_json::json!(flipped);
    } else {
        tampered.body["path"] = serde_json::json!(flipped);
    }
    tampered
}

fn claim_author(mut wire: WireOp, author_hex: &str) -> WireOp {
    wire.author = author_hex.to_string();
    wire
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
    let mut preimage = DOMAIN_RELAY_AUTH.to_vec();
    preimage.extend_from_slice(nonce);
    let signature = key.sign(&preimage).to_bytes();
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

fn has_label(store: &LocalStore<MemoryBackend>, label: &str) -> bool {
    store
        .list_nodes()
        .unwrap()
        .iter()
        .any(|(_, name, deleted)| name == label && !deleted)
}

fn assert_rejected(result: IngestResult, expected: &str) {
    match result {
        IngestResult::Rejected { reason } => assert_eq!(reason, expected),
        other => panic!("expected {expected}, got {other:?}"),
    }
}

fn e5_topology() -> (
    LocalStore<MemoryBackend>,
    LocalStore<MemoryBackend>,
    String,
    WireOp,
) {
    let mut a = auth_store();
    let mut b = empty_store();
    a.grant_write_access(&b.principal_hex()).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    let node = b.create_node("Todo").unwrap();
    b.counter_inc(&node, "voteScore", 3).unwrap();
    a.import_bundle(&b.export_all().unwrap()).unwrap();
    assert_eq!(
        a.get_prop(&node, "voteScore").unwrap(),
        Some(serde_json::json!(3))
    );
    let inc = last_kind(&b, 3);
    (a, b, node, inc)
}

#[test]
fn e7_peer_rejects_four_attacks() {
    let (mut a, mut b, node, inc) = e5_topology();
    let score = a.get_prop(&node, "voteScore").unwrap();

    match a.ingest_op(&flip_payload_byte(&inc)).unwrap() {
        IngestResult::Rejected { reason } => assert_eq!(reason, "AUTH_SIG_INVALID"),
        other => panic!("flipped payload must be AUTH_SIG_INVALID, got {other:?}"),
    }
    match b.ingest_op(&flip_payload_byte(&inc)).unwrap() {
        IngestResult::Rejected { reason } => assert_eq!(reason, "AUTH_SIG_INVALID"),
        other => panic!("peer B must also name AUTH_SIG_INVALID: {other:?}"),
    }

    let forged = claim_author(
        signed_create(
            &[0xC7u8; 32],
            &ds_bytes(&a),
            &[genesis_id(&a)],
            9_300_000,
            "ClaimB",
        ),
        &b.author_hex(),
    );
    assert_rejected(a.ingest_op(&forged).unwrap(), "AUTH_SIG_INVALID");
    assert_rejected(b.ingest_op(&forged).unwrap(), "AUTH_SIG_INVALID");
    assert!(!has_label(&a, "ClaimB"));
    assert!(!has_label(&b, "ClaimB"));

    assert_eq!(a.ingest_op(&inc).unwrap(), IngestResult::Duplicate);
    assert_eq!(b.ingest_op(&inc).unwrap(), IngestResult::Duplicate);
    assert_eq!(a.ingest_op(&inc).unwrap(), IngestResult::Duplicate);
    assert_eq!(a.get_prop(&node, "voteScore").unwrap(), score);
    assert_eq!(b.get_prop(&node, "voteScore").unwrap(), score);
}

#[test]
fn e7_honest_relay_rejects_forged_and_names_duplicate() {
    let (mut a, mut b, node, inc) = e5_topology();
    let score = a.get_prop(&node, "voteScore").unwrap();
    let relay = Relay::memory();
    let mut ra = relay.accept();
    let mut rb = relay.accept();
    let mut rc = relay.accept();
    let ds = a.datastore_id_hex();
    drive(&mut a, &mut ra, None);
    drive(&mut b, &mut rb, Some(&ds));
    let before = relay.op_count(&ds).unwrap();

    handshake(&mut rc, &[0xC7u8; 32]);
    let flipped = flip_payload_byte(&inc);
    let (outcome, reason) = outcome_of(&submit_ops(&mut rc, &ds, vec![wire_to_relay(&flipped)])[0]);
    assert_eq!(outcome, "REJECT");
    assert_eq!(reason.as_deref(), Some("SIG"));

    let forged = claim_author(
        signed_create(
            &[0xC7u8; 32],
            &ds_bytes(&a),
            &[genesis_id(&a)],
            9_400_000,
            "ClaimB",
        ),
        &b.author_hex(),
    );
    let (outcome, reason) = outcome_of(&submit_ops(&mut rc, &ds, vec![wire_to_relay(&forged)])[0]);
    assert_eq!(outcome, "REJECT");
    assert_eq!(reason.as_deref(), Some("SIG"));
    assert_eq!(relay.op_count(&ds).unwrap(), before);

    let (outcome, reason) = outcome_of(&submit_ops(&mut rc, &ds, vec![wire_to_relay(&inc)])[0]);
    assert_eq!(outcome, "DUPLICATE");
    assert_eq!(reason, None);

    relay.wipe_dedup(&ds).unwrap();
    assert_eq!(relay.op_count(&ds).unwrap(), 0);
    let (outcome, _) = outcome_of(&submit_ops(&mut rc, &ds, vec![wire_to_relay(&inc)])[0]);
    assert_eq!(outcome, "ACCEPT", "wipe must forget the OpId");

    drive(&mut a, &mut ra, None);
    drive(&mut b, &mut rb, None);
    assert_eq!(a.get_prop(&node, "voteScore").unwrap(), score);
    assert_eq!(b.get_prop(&node, "voteScore").unwrap(), score);
    assert_eq!(a.ingest_op(&inc).unwrap(), IngestResult::Duplicate);
    assert!(!has_label(&a, "ClaimB"));
}

#[test]
fn e7_colluding_relay_cannot_materialize_attacks() {
    let (mut a, mut b, node, inc) = e5_topology();
    let score = a.get_prop(&node, "voteScore").unwrap();
    let relay = Relay::memory_colluding();
    let mut ra = relay.accept();
    let mut rb = relay.accept();
    let mut rc = relay.accept();
    let ds = a.datastore_id_hex();
    drive(&mut a, &mut ra, None);
    drive(&mut b, &mut rb, Some(&ds));

    handshake(&mut rc, &[0xC8u8; 32]);
    let flipped = flip_payload_byte(&inc);
    let (outcome, _) = outcome_of(&submit_ops(&mut rc, &ds, vec![wire_to_relay(&flipped)])[0]);
    assert_eq!(
        outcome, "DUPLICATE",
        "same OpId as B's live increment is still a relay dedup"
    );

    let forged = claim_author(
        signed_create(
            &[0xC8u8; 32],
            &ds_bytes(&a),
            &[genesis_id(&a)],
            9_500_000,
            "ClaimB",
        ),
        &b.author_hex(),
    );
    let (outcome, _) = outcome_of(&submit_ops(&mut rc, &ds, vec![wire_to_relay(&forged)])[0]);
    assert_eq!(outcome, "ACCEPT", "colluding relay forwards C-as-B");

    drive(&mut a, &mut ra, None);
    drive(&mut b, &mut rb, None);
    assert!(!has_label(&a, "ClaimB"));
    assert!(!has_label(&b, "ClaimB"));
    assert_rejected(a.ingest_op(&forged).unwrap(), "AUTH_SIG_INVALID");
    assert_eq!(a.get_prop(&node, "voteScore").unwrap(), score);

    let (outcome, _) = outcome_of(&submit_ops(&mut rc, &ds, vec![wire_to_relay(&inc)])[0]);
    assert_eq!(outcome, "DUPLICATE");

    relay.wipe_dedup(&ds).unwrap();
    let (outcome, _) = outcome_of(&submit_ops(&mut rc, &ds, vec![wire_to_relay(&inc)])[0]);
    assert_eq!(outcome, "ACCEPT", "duplicate after wipe is re-persisted");
    drive(&mut a, &mut ra, None);
    drive(&mut b, &mut rb, None);
    assert_eq!(a.get_prop(&node, "voteScore").unwrap(), score);
    assert_eq!(b.get_prop(&node, "voteScore").unwrap(), score);
    assert_eq!(a.ingest_op(&inc).unwrap(), IngestResult::Duplicate);

    relay.wipe_dedup(&ds).unwrap();
    let (outcome, _) = outcome_of(&submit_ops(&mut rc, &ds, vec![wire_to_relay(&flipped)])[0]);
    assert_eq!(
        outcome, "ACCEPT",
        "colluding relay forwards flipped payload"
    );
    drive(&mut a, &mut ra, None);
    drive(&mut b, &mut rb, None);
    assert_eq!(a.get_prop(&node, "voteScore").unwrap(), score);
    assert_eq!(b.get_prop(&node, "voteScore").unwrap(), score);
    assert_rejected(a.ingest_op(&flipped).unwrap(), "AUTH_SIG_INVALID");
    assert_rejected(b.ingest_op(&flipped).unwrap(), "AUTH_SIG_INVALID");
}

#[test]
fn e7_colluding_header_only_does_not_poison_catchup() {
    let (mut a, mut b, node, _inc) = e5_topology();
    let score = a.get_prop(&node, "voteScore").unwrap();
    let relay = Relay::memory_colluding();
    let mut ra = relay.accept();
    let mut rb = relay.accept();
    let mut rc = relay.accept();
    let ds = a.datastore_id_hex();
    drive(&mut a, &mut ra, None);
    drive(&mut b, &mut rb, Some(&ds));

    handshake(&mut rc, &[0xC9u8; 32]);
    let junk_id = [0xE7u8; 32];
    let header_only = Cbor::Map(vec![
        ("op_id".into(), Cbor::Bytes(junk_id.to_vec())),
        ("author".into(), Cbor::Bytes(vec![0xB0; 32])),
        ("physical_ms".into(), Cbor::Uint(9_600_000)),
        ("logical".into(), Cbor::Uint(0)),
    ]);
    let (outcome, _) = outcome_of(&submit_ops(&mut rc, &ds, vec![header_only])[0]);
    assert_eq!(
        outcome, "ACCEPT",
        "colluding relay forwards header-only unsigned junk"
    );

    a.create_node("AfterJunk").unwrap();
    drive(&mut a, &mut ra, None);

    let summary = drive(&mut b, &mut rb, None);
    assert!(
        summary.skipped >= 1,
        "missing wire is AUTH_SIG_INVALID skip, not a catch-up abort: {summary:?}"
    );
    assert!(
        summary.applied >= 1,
        "later valid ops must still apply after header-only junk: {summary:?}"
    );
    assert!(has_label(&b, "AfterJunk"));
    assert_eq!(b.get_prop(&node, "voteScore").unwrap(), score);
    let junk_hex = hex::encode(junk_id);
    assert!(
        !b.export_all()
            .unwrap()
            .ops
            .iter()
            .any(|op| op.id == junk_hex),
        "header-only junk must not be applied"
    );
}
