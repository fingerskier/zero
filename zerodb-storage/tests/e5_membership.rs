//! EXEMPLAR E5: membership sharing, denial, revocation, and colluding relay.
//!
//! Peer-side AUTH.md §4 is load-bearing. Rejections are named outcomes
//! (`AUTH_NO_MEMBERSHIP`, `AUTH_REVOKED`, `AUTH_WRONG_DATASTORE`, or
//! `REJECT/AUTHZ` on the wire). A colluding relay that forwards everything
//! still cannot force honest peers to materialize unauthorized ops.

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

fn signed_create(
    seed: &[u8; 32],
    ds: &[u8; 32],
    deps: &[String],
    physical_ms: u64,
    label: &str,
) -> WireOp {
    let (author_pk, _) = sign_op(seed, &[0; 32]);
    let author = *blake3::hash(&author_pk).as_bytes();
    let node = [0x44u8; 16];
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

#[test]
fn e5_member_write_non_member_denied_revoke_and_replay() {
    let mut a = auth_store();
    let mut b = empty_store();

    let grant_id = a.grant_write_access(&b.principal_hex()).unwrap();
    let (accepted, _) = b.import_bundle(&a.export_all().unwrap()).unwrap();
    assert!(accepted >= 2, "B must adopt genesis + grant");
    assert_eq!(b.datastore_id_hex(), a.datastore_id_hex());

    let (node, _) = b.create_node_with_op("Todo").unwrap();
    b.set_lww(&node, "title", "milk").unwrap();
    a.import_bundle(&b.export_all().unwrap()).unwrap();
    assert_eq!(a.get_lww(&node, "title").unwrap().as_deref(), Some("milk"));

    let c_seed = [0xCCu8; 32];
    let c_op = signed_create(
        &c_seed,
        &ds_bytes(&a),
        &[genesis_id(&a)],
        9_000_000,
        "Intruder",
    );
    match a.ingest_op(&c_op).unwrap() {
        IngestResult::Rejected { reason } => assert_eq!(reason, "AUTH_NO_MEMBERSHIP"),
        other => panic!("C write must be a named AUTH reject, got {other:?}"),
    }
    match b.ingest_op(&c_op).unwrap() {
        IngestResult::Rejected { reason } => assert_eq!(reason, "AUTH_NO_MEMBERSHIP"),
        other => panic!("peer B must also reject C: {other:?}"),
    }
    assert!(!has_label(&a, "Intruder"));
    assert!(!has_label(&b, "Intruder"));

    let mut other = auth_store();
    let b_write = b
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 1)
        .unwrap();
    let err = other.ingest_wire(&b_write).unwrap_err();
    assert_eq!(err.to_string(), "AUTH_WRONG_DATASTORE");

    a.revoke_membership(&grant_id, 2).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    let err = b.create_node("AfterRevoke").unwrap_err();
    assert_eq!(err.to_string(), "AUTH_REVOKED");
    assert_eq!(b.get_lww(&node, "title").unwrap().as_deref(), Some("milk"));
}

#[test]
fn e5_colluding_relay_peer_side_reject() {
    let mut a = auth_store();
    let mut b = empty_store();
    a.grant_write_access(&b.principal_hex()).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    let node = b.create_node("Todo").unwrap();
    b.set_lww(&node, "title", "shared").unwrap();

    let relay = Relay::memory_colluding();
    let mut ra = relay.accept();
    let mut rb = relay.accept();
    let mut rc = relay.accept();
    let ds = a.datastore_id_hex();
    drive(&mut a, &mut ra, None);
    drive(&mut b, &mut rb, Some(&ds));

    let c_seed = [0xCDu8; 32];
    handshake(&mut rc, &c_seed);
    let c_op = signed_create(
        &c_seed,
        &ds_bytes(&a),
        &[genesis_id(&a)],
        9_100_000,
        "Forged",
    );
    let outcomes = submit_ops(&mut rc, &ds, vec![wire_to_relay(&c_op)]);
    assert_eq!(
        outcome_of(&outcomes[0]).0,
        "ACCEPT",
        "colluding relay forwards"
    );

    drive(&mut a, &mut ra, None);
    drive(&mut b, &mut rb, None);
    assert!(
        !has_label(&a, "Forged"),
        "A rejects C despite colluding relay"
    );
    assert!(
        !has_label(&b, "Forged"),
        "B rejects C despite colluding relay"
    );
    match a.ingest_op(&c_op).unwrap() {
        IngestResult::Rejected { reason } => assert_eq!(reason, "AUTH_NO_MEMBERSHIP"),
        IngestResult::Duplicate => panic!("C's op must not have been applied on A"),
        other => panic!("expected AUTH_NO_MEMBERSHIP, got {other:?}"),
    }
}

#[test]
fn e5_honest_relay_rejects_non_member_write() {
    let mut a = auth_store();
    let mut b = empty_store();
    a.grant_write_access(&b.principal_hex()).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();

    let relay = Relay::memory();
    let mut ra = relay.accept();
    drive(&mut a, &mut ra, None);

    let mut rc = relay.accept();
    let c_seed = [0xCEu8; 32];
    handshake(&mut rc, &c_seed);
    let c_op = signed_create(&c_seed, &ds_bytes(&a), &[genesis_id(&a)], 9_200_000, "Nope");
    let outcomes = submit_ops(&mut rc, &a.datastore_id_hex(), vec![wire_to_relay(&c_op)]);
    let (outcome, reason) = outcome_of(&outcomes[0]);
    assert_eq!(outcome, "REJECT");
    assert_eq!(reason.as_deref(), Some("AUTHZ"));
    assert!(relay.op_count(&a.datastore_id_hex()).unwrap() >= 2);
}
