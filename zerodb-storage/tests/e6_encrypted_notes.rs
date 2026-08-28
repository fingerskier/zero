//! EXEMPLAR E6: encrypted private notes (I-10).
//!
//! A and B share list L; `Note.body` is schema-encrypted LWW. B reads
//! plaintext. Relay-captured artifacts and a full replica handed to
//! non-recipient C do not permit recovery (decrypt oracle included).
//! After A removes B and rotates the group key, B cannot decrypt notes
//! written post-rotation. A's keyring survives SQLite reopen.

use ed25519_dalek::{Signer, SigningKey};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use zerodb_core::auth::{SCOPE_READ, SCOPE_SYNC, SCOPE_WRITE};
use zerodb_core::op::{OpEnvelope, OpTs, json_to_cbor_body};
use zerodb_core::sign::DOMAIN_OP_SIG;
use zerodb_relay::Relay;
use zerodb_storage::relay_client;
use zerodb_storage::{
    ENCRYPTED_PLAINTEXT, ExportBundle, IngestResult, KEY_WRAP_INVALID, LocalStore, MemoryBackend,
    StoreBackend, StoreError, WireOp, WireTs,
};

const NOTE_SCHEMA: &str = r#"{
  "v": 1,
  "nodes": {
    "Note": {
      "props": {
        "body": { "crdt": 0, "type": 4, "nullable": false, "encrypted": true }
      }
    }
  },
  "edges": {}
}"#;

const SECRET: &str = "attack-at-dawn-for-ab-only";
const SECRET_AFTER: &str = "post-rotation-for-a-only";

fn auth_store() -> LocalStore<MemoryBackend> {
    LocalStore::init_auth_with_backend(MemoryBackend::new()).unwrap()
}

fn empty_store() -> LocalStore<MemoryBackend> {
    LocalStore::init_with_backend(MemoryBackend::new()).unwrap()
}

fn pair<'a>(
    a_peer: &'a str,
    a_pk: &'a str,
    b_peer: &'a str,
    b_pk: &'a str,
) -> [(&'a str, &'a str); 2] {
    [(a_peer, a_pk), (b_peer, b_pk)]
}

fn assert_no_plaintext(haystack: &str, secret: &str) {
    assert!(
        !haystack.contains(secret),
        "protocol artifact leaked plaintext"
    );
}

fn drive<B: StoreBackend>(
    store: &mut LocalStore<B>,
    sess: &mut zerodb_relay::RelaySession,
    join: Option<&str>,
) -> relay_client::RelaySyncSummary {
    relay_client::sync(store, join, |frame| {
        Ok::<_, StoreError>(sess.handle(frame).unwrap())
    })
    .unwrap()
}

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("e6-{name}-{nonce}.sqlite"));
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
    path
}

fn e6_share_notes() -> (
    LocalStore<MemoryBackend>,
    LocalStore<MemoryBackend>,
    String,
    String,
) {
    let mut a = auth_store();
    let mut b = empty_store();
    a.apply_schema_json(NOTE_SCHEMA).unwrap();
    let grant = a
        .grant_membership(&b.principal_hex(), &[SCOPE_WRITE, SCOPE_READ, SCOPE_SYNC])
        .unwrap();

    let a_peer = a.principal_hex();
    let a_pk = a.author_pk_hex();
    let b_peer = b.principal_hex();
    let b_pk = b.author_pk_hex();
    a.distribute_group_key(&pair(&a_peer, &a_pk, &b_peer, &b_pk))
        .unwrap();

    // Schema is local meta (not a SchemaEpoch op yet). Recipients must hold
    // the same IR so ep=1 ops are not EPOCH_UNKNOWN; the schema is not secret.
    b.apply_schema_json(NOTE_SCHEMA).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    assert_eq!(b.datastore_id_hex(), a.datastore_id_hex());

    let note = a.create_node("Note").unwrap();
    a.set_lww(&note, "body", SECRET).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();

    assert_eq!(a.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));
    assert_eq!(b.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));
    (a, b, note, grant)
}

#[test]
fn e6_members_read_plaintext_artifacts_and_c_do_not() {
    let (a, b, note, _grant) = e6_share_notes();

    let export = serde_json::to_string(&a.export_all().unwrap()).unwrap();
    assert_no_plaintext(&export, SECRET);
    let set = a
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 3)
        .expect("SetProperty");
    assert!(
        set.body.get("encrypted").and_then(|v| v.as_str()).is_some(),
        "encrypted LWW must carry a KERNEL §7 envelope"
    );
    assert!(set.body.get("value").is_none());

    let mut c = empty_store();
    c.apply_schema_json(NOTE_SCHEMA).unwrap();
    c.import_bundle(&a.export_all().unwrap()).unwrap();
    assert_eq!(c.get_lww(&note, "body").unwrap(), None);
    assert!(
        c.decrypt_oracle().unwrap().is_empty(),
        "C decrypt-oracle recovered plaintext"
    );
    let c_export = serde_json::to_string(&c.export_all().unwrap()).unwrap();
    assert_no_plaintext(&c_export, SECRET);

    let _ = b;
}

#[test]
fn e6_rotate_after_revoke_blinds_b_a_still_reads() {
    let (mut a, mut b, note, grant) = e6_share_notes();
    a.revoke_membership(&grant, 2).unwrap();

    let a_peer = a.principal_hex();
    let a_pk = a.author_pk_hex();
    a.rotate_group_key(&[(a_peer.as_str(), a_pk.as_str())])
        .unwrap();

    let late = a.create_node("Note").unwrap();
    a.set_lww(&late, "body", SECRET_AFTER).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();

    assert_eq!(a.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));
    assert_eq!(
        a.get_lww(&late, "body").unwrap().as_deref(),
        Some(SECRET_AFTER)
    );
    assert_eq!(
        b.get_lww(&note, "body").unwrap().as_deref(),
        Some(SECRET),
        "pre-rotation notes stay readable to a former recipient"
    );
    assert_eq!(
        b.get_lww(&late, "body").unwrap(),
        None,
        "B must not decrypt notes written after rotation"
    );
    assert!(
        !b.decrypt_oracle()
            .unwrap()
            .iter()
            .any(|s| s == SECRET_AFTER),
        "B decrypt-oracle recovered a post-rotation note"
    );
}

#[test]
fn e6_sqlite_reopen_decrypts() {
    let path = tmp_db("reopen");
    let note = {
        let mut a = LocalStore::init_auth(&path).unwrap();
        let b = empty_store();
        a.apply_schema_json(NOTE_SCHEMA).unwrap();
        a.grant_write_access(&b.principal_hex()).unwrap();
        let a_peer = a.principal_hex();
        let a_pk = a.author_pk_hex();
        let b_peer = b.principal_hex();
        let b_pk = b.author_pk_hex();
        a.distribute_group_key(&pair(&a_peer, &a_pk, &b_peer, &b_pk))
            .unwrap();
        let note = a.create_node("Note").unwrap();
        a.set_lww(&note, "body", SECRET).unwrap();
        assert_eq!(a.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));
        note
    };
    let mut a = LocalStore::open(&path).unwrap();
    assert_eq!(a.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));
    a.replay_all().unwrap();
    assert_eq!(
        a.get_lww(&note, "body").unwrap().as_deref(),
        Some(SECRET),
        "keys must persist across reopen + replay"
    );
}

#[test]
fn e6_relay_artifacts_blind() {
    let (mut a, mut b, note, _grant) = e6_share_notes();
    let relay = Relay::memory();
    let mut ra = relay.accept();
    let mut rb = relay.accept();
    let ds = a.datastore_id_hex();
    drive(&mut a, &mut ra, None);
    drive(&mut b, &mut rb, Some(&ds));
    assert_eq!(b.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));

    let captured = relay.captured_artifacts(&ds).unwrap();
    let captured_text = String::from_utf8_lossy(&captured);
    assert!(
        !captured_text.contains(SECRET),
        "relay stored artifacts contained plaintext"
    );
}

fn control_deps(store: &LocalStore<MemoryBackend>) -> Vec<String> {
    store
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .filter(|op| matches!(op.kind, 0 | 6 | 7 | 8))
        .map(|op| op.id)
        .collect()
}

fn last_physical(store: &LocalStore<MemoryBackend>) -> u64 {
    store
        .export_all()
        .unwrap()
        .ops
        .iter()
        .map(|op| op.ts.p)
        .max()
        .unwrap_or(0)
}

fn ds_bytes(store: &LocalStore<MemoryBackend>) -> [u8; 32] {
    hex::decode(store.datastore_id_hex())
        .unwrap()
        .try_into()
        .unwrap()
}

fn sign_wire(
    seed: &[u8; 32],
    ds: &[u8; 32],
    ep: u64,
    deps: &[String],
    physical_ms: u64,
    kind: u64,
    body_json: serde_json::Value,
) -> WireOp {
    let signing = SigningKey::from_bytes(seed);
    let author_pk = signing.verifying_key().to_bytes();
    let author = *blake3::hash(&author_pk).as_bytes();
    let dep_ids = deps
        .iter()
        .map(|dep| hex::decode(dep).unwrap().try_into().unwrap())
        .collect::<Vec<[u8; 32]>>();
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
        kind,
        body,
    };
    let id = envelope.op_id().unwrap();
    let sig = {
        let pre = [DOMAIN_OP_SIG, id.as_slice()].concat();
        signing.sign(&pre).to_bytes()
    };
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
        kind,
        body: body_json,
        sig: hex::encode(sig),
    }
}

fn plaintext_body(node: &str, value: &str) -> serde_json::Value {
    serde_json::json!({
        "node": node,
        "path": "body",
        "crdt": "lww",
        "value": value,
    })
}

fn create_note_body(node: &str) -> serde_json::Value {
    serde_json::json!({
        "label": "Note",
        "node": node,
    })
}

fn signed_set_then_create(
    member: &LocalStore<MemoryBackend>,
    ds: &LocalStore<MemoryBackend>,
    node: &str,
    plaintext: &str,
    physical: u64,
) -> (WireOp, WireOp) {
    let deps = control_deps(ds);
    let ds_b = ds_bytes(ds);
    let set = sign_wire(
        &member.identity_seed(),
        &ds_b,
        1,
        &deps,
        physical,
        3,
        plaintext_body(node, plaintext),
    );
    let create = sign_wire(
        &member.identity_seed(),
        &ds_b,
        1,
        &deps,
        physical.saturating_add(1),
        1,
        create_note_body(node),
    );
    (set, create)
}

#[test]
fn e6_member_plaintext_value_rejected() {
    let (mut a, mut b, note, _grant) = e6_share_notes();
    let attack = "member-plaintext-smuggle";
    let wire = sign_wire(
        &b.identity_seed(),
        &ds_bytes(&a),
        1,
        &control_deps(&a),
        last_physical(&a).saturating_add(1),
        3,
        plaintext_body(&note, attack),
    );

    match a.ingest_op(&wire).unwrap() {
        IngestResult::Rejected { reason } => assert_eq!(reason, ENCRYPTED_PLAINTEXT),
        other => panic!("expected ENCRYPTED_PLAINTEXT, got {other:?}"),
    }
    assert_eq!(a.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));
    let export = serde_json::to_string(&a.export_all().unwrap()).unwrap();
    assert_no_plaintext(&export, attack);
    assert!(
        !a.export_all()
            .unwrap()
            .ops
            .iter()
            .any(|op| op.id == wire.id),
        "plaintext SetProperty must not persist"
    );

    let (accepted, skipped) = b
        .import_bundle(&ExportBundle {
            format: 1,
            datastore_id: a.datastore_id_hex(),
            ops: vec![wire.clone()],
        })
        .unwrap();
    assert_eq!(accepted, 0);
    assert!(skipped >= 1);
    assert!(
        b.take_rejects()
            .iter()
            .any(|r| r.reason == ENCRYPTED_PLAINTEXT)
    );
    assert_eq!(b.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));
    assert_no_plaintext(
        &serde_json::to_string(&b.export_all().unwrap()).unwrap(),
        attack,
    );

    let err = a.commit_wires_atomic(&[wire]).unwrap_err();
    match err {
        StoreError::Invalid(msg) => assert_eq!(msg, ENCRYPTED_PLAINTEXT),
        other => panic!("expected Invalid(ENCRYPTED_PLAINTEXT), got {other}"),
    }
}

#[test]
fn e6_member_set_before_create_plaintext_rejected() {
    let (mut a, mut b, _note, _grant) = e6_share_notes();
    let attack = "set-before-create-plaintext";

    let node_ingest = hex::encode([0xe6u8; 16]);
    let (set, create) = signed_set_then_create(&b, &a, &node_ingest, attack, last_physical(&a) + 1);
    match a.ingest_op(&set).unwrap() {
        IngestResult::Rejected { reason } => assert_eq!(reason, ENCRYPTED_PLAINTEXT),
        other => panic!("expected ENCRYPTED_PLAINTEXT, got {other:?}"),
    }
    assert_eq!(a.ingest_op(&create).unwrap(), IngestResult::Applied);
    assert_eq!(a.get_lww(&node_ingest, "body").unwrap(), None);
    assert!(
        !a.export_all().unwrap().ops.iter().any(|op| op.id == set.id),
        "plaintext SetProperty must not persist"
    );
    assert_no_plaintext(
        &serde_json::to_string(&a.export_all().unwrap()).unwrap(),
        attack,
    );

    let node_import = hex::encode([0xe7u8; 16]);
    let (set_imp, create_imp) =
        signed_set_then_create(&b, &a, &node_import, attack, last_physical(&a) + 3);
    let (accepted, skipped) = b
        .import_bundle(&ExportBundle {
            format: 1,
            datastore_id: a.datastore_id_hex(),
            ops: vec![set_imp.clone(), create_imp.clone()],
        })
        .unwrap();
    assert_eq!(accepted, 1);
    assert!(skipped >= 1);
    assert!(
        b.take_rejects()
            .iter()
            .any(|r| r.reason == ENCRYPTED_PLAINTEXT)
    );
    assert_eq!(b.get_lww(&node_import, "body").unwrap(), None);
    assert!(
        !b.export_all()
            .unwrap()
            .ops
            .iter()
            .any(|op| op.id == set_imp.id),
        "plaintext SetProperty must not persist in import"
    );
    assert!(
        b.export_all()
            .unwrap()
            .ops
            .iter()
            .any(|op| op.id == create_imp.id),
        "later CreateNode must still apply"
    );
    assert_no_plaintext(
        &serde_json::to_string(&b.export_all().unwrap()).unwrap(),
        attack,
    );

    let node_atomic = hex::encode([0xe8u8; 16]);
    let (set_at, create_at) =
        signed_set_then_create(&b, &a, &node_atomic, attack, last_physical(&a) + 5);
    let err = a
        .commit_wires_atomic(&[set_at.clone(), create_at.clone()])
        .unwrap_err();
    match err {
        StoreError::Invalid(msg) => assert_eq!(msg, ENCRYPTED_PLAINTEXT),
        other => panic!("expected Invalid(ENCRYPTED_PLAINTEXT), got {other}"),
    }
    assert_eq!(a.get_lww(&node_atomic, "body").unwrap(), None);
    assert!(
        !a.export_all()
            .unwrap()
            .ops
            .iter()
            .any(|op| op.id == set_at.id || op.id == create_at.id),
        "atomic reverse-order plaintext must roll back"
    );
    assert_no_plaintext(
        &serde_json::to_string(&a.export_all().unwrap()).unwrap(),
        attack,
    );
}

#[test]
fn e6_atomic_group_set_lww_seals() {
    let (mut a, _b, _note, _grant) = e6_share_notes();
    let created = a
        .atomic_group(|g| {
            let n = g.create_node("Note")?;
            g.set_lww(&n, "body", SECRET)?;
            Ok(n)
        })
        .unwrap();
    assert_eq!(
        a.get_lww(&created, "body").unwrap().as_deref(),
        Some(SECRET)
    );
    let set = a
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .rev()
        .find(|op| {
            op.kind == 3 && op.body.get("node").and_then(|v| v.as_str()) == Some(created.as_str())
        })
        .expect("atomic SetProperty");
    assert!(set.body.get("encrypted").and_then(|v| v.as_str()).is_some());
    assert!(set.body.get("value").is_none());
    assert_no_plaintext(&serde_json::to_string(&set).unwrap(), SECRET);
}

#[test]
fn e6_member_kr2_not_adopted_as_current() {
    let (mut a, mut b, note, _grant) = e6_share_notes();
    let old_kr = a
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 8 && op.body.get("kr").and_then(|v| v.as_u64()) == Some(2))
        .expect("admin KeyRecord");

    let hijack = sign_wire(
        &b.identity_seed(),
        &ds_bytes(&a),
        1,
        &control_deps(&a),
        last_physical(&a).saturating_add(1),
        8,
        old_kr.body.clone(),
    );
    match a.ingest_op(&hijack).unwrap() {
        IngestResult::Rejected { reason } => assert_eq!(reason, "AUTH_NOT_ADMIN"),
        other => panic!("expected AUTH_NOT_ADMIN, got {other:?}"),
    }

    let a_peer = a.principal_hex();
    let a_pk = a.author_pk_hex();
    a.rotate_group_key(&[(a_peer.as_str(), a_pk.as_str())])
        .unwrap();

    let republish = sign_wire(
        &b.identity_seed(),
        &ds_bytes(&a),
        1,
        &control_deps(&a),
        last_physical(&a).saturating_add(1),
        8,
        old_kr.body,
    );
    match a.ingest_op(&republish).unwrap() {
        IngestResult::Rejected { reason } => assert_eq!(reason, "AUTH_NOT_ADMIN"),
        other => panic!("expected AUTH_NOT_ADMIN after rotate, got {other:?}"),
    }

    let late = a.create_node("Note").unwrap();
    a.set_lww(&late, "body", "cannot-downgrade-to-A").unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    assert_eq!(a.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));
    assert_eq!(
        a.get_lww(&late, "body").unwrap().as_deref(),
        Some("cannot-downgrade-to-A")
    );
    assert_eq!(
        b.get_lww(&late, "body").unwrap(),
        None,
        "member republish of old key must not make A seal under A"
    );
}

#[test]
fn e6_short_nonce_wrap_does_not_poison_bundle() {
    let (a, mut b, _note, _grant) = e6_share_notes();
    let mut kr = a
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 8 && op.body.get("kr").and_then(|v| v.as_u64()) == Some(2))
        .expect("KeyRecord");
    kr.body["wraps"][0]["nonce"] = serde_json::json!("aa");
    let bad = sign_wire(
        &a.identity_seed(),
        &ds_bytes(&a),
        1,
        &control_deps(&a),
        last_physical(&a).saturating_add(1),
        8,
        kr.body,
    );
    let node = [0x4eu8; 16];
    let create = sign_wire(
        &a.identity_seed(),
        &ds_bytes(&a),
        1,
        &control_deps(&a),
        last_physical(&a).saturating_add(2),
        1,
        serde_json::json!({
            "label": "Note",
            "node": hex::encode(node),
        }),
    );
    let (accepted, skipped) = b
        .import_bundle(&ExportBundle {
            format: 1,
            datastore_id: a.datastore_id_hex(),
            ops: vec![bad, create],
        })
        .unwrap();
    assert!(
        skipped >= 1,
        "short-nonce wrap must be a per-op skip, accepted={accepted} skipped={skipped}"
    );
    assert!(accepted >= 1, "later CreateNode must still apply");
    assert!(
        b.take_rejects()
            .iter()
            .any(|r| r.reason == KEY_WRAP_INVALID)
    );
    assert!(
        b.export_all()
            .unwrap()
            .ops
            .iter()
            .any(|op| op.kind == 1 && op.body["node"] == hex::encode(node)),
        "later create must be materialized"
    );
}

#[test]
fn e6_kind8_device_cert_and_revoke_accepted() {
    let mut a = auth_store();
    a.apply_schema_json(NOTE_SCHEMA).unwrap();
    let device = a.author_pk_hex();
    let principal = a.principal_hex();
    let cert_sig = "00".repeat(64);
    let cert = sign_wire(
        &a.identity_seed(),
        &ds_bytes(&a),
        1,
        &control_deps(&a),
        last_physical(&a).saturating_add(1),
        8,
        serde_json::json!({
            "kr": 0,
            "device": device,
            "principal": principal,
            "root_pk": device,
            "issued": 1,
            "expiry": null,
            "revoke_of": null,
            "cert_sig": cert_sig,
        }),
    );
    assert_eq!(a.ingest_op(&cert).unwrap(), IngestResult::Applied);

    let revoke = sign_wire(
        &a.identity_seed(),
        &ds_bytes(&a),
        1,
        &control_deps(&a),
        last_physical(&a).saturating_add(2),
        8,
        serde_json::json!({
            "kr": 1,
            "device": device,
            "principal": principal,
            "root_pk": device,
            "issued": 2,
            "expiry": null,
            "revoke_of": principal,
            "cert_sig": "11".repeat(64),
        }),
    );
    assert_eq!(a.ingest_op(&revoke).unwrap(), IngestResult::Applied);
}

#[test]
fn e6_offline_revoke_note_without_rotate_stays_closed() {
    let (mut a, mut b, note, grant) = e6_share_notes();
    a.revoke_membership(&grant, 2).unwrap();
    let a_peer = a.principal_hex();
    let a_pk = a.author_pk_hex();
    a.rotate_group_key(&[(a_peer.as_str(), a_pk.as_str())])
        .unwrap();
    let late = a.create_node("Note").unwrap();
    a.set_lww(&late, "body", SECRET_AFTER).unwrap();

    let all = a.export_all().unwrap();
    let late_only: Vec<WireOp> = all
        .ops
        .into_iter()
        .filter(|op| {
            let node = op.body.get("node").and_then(|v| v.as_str());
            (op.kind == 1 || op.kind == 3) && node == Some(late.as_str())
        })
        .collect();
    b.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: a.datastore_id_hex(),
        ops: late_only,
    })
    .unwrap();

    assert_eq!(b.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));
    assert_eq!(
        b.get_lww(&late, "body").unwrap(),
        None,
        "offline B must not open a post-revoke note without the rotate"
    );
    assert!(
        !b.decrypt_oracle()
            .unwrap()
            .iter()
            .any(|s| s == SECRET_AFTER),
        "decrypt oracle recovered post-revoke ciphertext"
    );
}

#[test]
fn e6_membership_at_open_blinds_same_key_after_revoke() {
    let (mut a, mut b, note, grant) = e6_share_notes();
    a.revoke_membership(&grant, 2).unwrap();
    let late = a.create_node("Note").unwrap();
    a.set_lww(&late, "body", SECRET_AFTER).unwrap();

    let all = a.export_all().unwrap();
    let without_revoke: Vec<WireOp> = all.ops.iter().filter(|op| op.kind != 7).cloned().collect();
    let revokes: Vec<WireOp> = all.ops.iter().filter(|op| op.kind == 7).cloned().collect();

    b.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: a.datastore_id_hex(),
        ops: without_revoke,
    })
    .unwrap();
    assert_eq!(
        b.get_lww(&late, "body").unwrap().as_deref(),
        Some(SECRET_AFTER),
        "same-key note is readable while B still believes they are a member"
    );

    b.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: a.datastore_id_hex(),
        ops: revokes,
    })
    .unwrap();
    assert_eq!(
        b.get_lww(&note, "body").unwrap().as_deref(),
        Some(SECRET),
        "pre-revoke notes stay readable"
    );
    assert_eq!(
        b.get_lww(&late, "body").unwrap(),
        None,
        "membership at open must hide post-revoke notes even with a cached key"
    );
    assert!(
        !b.decrypt_oracle()
            .unwrap()
            .iter()
            .any(|s| s == SECRET_AFTER)
    );
}

#[test]
fn e6_encrypted_value_before_key_then_after() {
    let mut a = auth_store();
    let mut b = empty_store();
    a.apply_schema_json(NOTE_SCHEMA).unwrap();
    a.grant_membership(&b.principal_hex(), &[SCOPE_WRITE, SCOPE_READ, SCOPE_SYNC])
        .unwrap();
    let a_peer = a.principal_hex();
    let a_pk = a.author_pk_hex();
    let b_peer = b.principal_hex();
    let b_pk = b.author_pk_hex();
    a.distribute_group_key(&pair(&a_peer, &a_pk, &b_peer, &b_pk))
        .unwrap();
    b.apply_schema_json(NOTE_SCHEMA).unwrap();
    let note = a.create_node("Note").unwrap();
    a.set_lww(&note, "body", SECRET).unwrap();

    let all = a.export_all().unwrap();
    let rest: Vec<WireOp> = all
        .ops
        .iter()
        .filter(|op| op.kind != 8 && op.kind != 3)
        .cloned()
        .collect();
    let keys: Vec<WireOp> = all.ops.iter().filter(|op| op.kind == 8).cloned().collect();
    let sets: Vec<WireOp> = all.ops.iter().filter(|op| op.kind == 3).cloned().collect();

    b.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: a.datastore_id_hex(),
        ops: rest,
    })
    .unwrap();
    b.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: a.datastore_id_hex(),
        ops: sets.clone(),
    })
    .unwrap();
    assert_eq!(
        b.get_lww(&note, "body").unwrap(),
        None,
        "ciphertext before key must not materialize plaintext"
    );
    assert!(
        b.export_all()
            .unwrap()
            .ops
            .iter()
            .any(|op| op.kind == 3 && op.body.get("encrypted").is_some()),
        "encrypted op must be held, not dropped"
    );
    assert_no_plaintext(
        &serde_json::to_string(&b.export_all().unwrap()).unwrap(),
        SECRET,
    );

    b.import_bundle(&ExportBundle {
        format: 1,
        datastore_id: a.datastore_id_hex(),
        ops: keys,
    })
    .unwrap();
    assert_eq!(
        b.get_lww(&note, "body").unwrap().as_deref(),
        Some(SECRET),
        "held ciphertext must open after the matching KeyRecord"
    );

    let mut c = empty_store();
    c.apply_schema_json(NOTE_SCHEMA).unwrap();
    c.import_bundle(&a.export_all().unwrap()).unwrap();
    assert_eq!(
        c.get_lww(&note, "body").unwrap().as_deref(),
        Some(SECRET),
        "key-before-data order must also open"
    );
}

#[test]
fn e6_second_device_of_principal_opens_random_does_not() {
    let mut a = auth_store();
    let mut d1 = empty_store();
    a.apply_schema_json(NOTE_SCHEMA).unwrap();
    a.grant_membership(&d1.principal_hex(), &[SCOPE_WRITE, SCOPE_READ, SCOPE_SYNC])
        .unwrap();
    let d2_seed = [0xD2u8; 32];
    let mut d2 =
        LocalStore::init_with_backend_from_seed(MemoryBackend::new(), &d2_seed, &ds_bytes(&a))
            .unwrap();
    d2.apply_schema_json(NOTE_SCHEMA).unwrap();
    let cert = sign_wire(
        &a.identity_seed(),
        &ds_bytes(&a),
        1,
        &control_deps(&a),
        last_physical(&a).saturating_add(1),
        8,
        serde_json::json!({
            "kr": 0,
            "device": d2.author_pk_hex(),
            "principal": d1.principal_hex(),
            "root_pk": d1.author_pk_hex(),
            "issued": 1,
            "expiry": null,
            "revoke_of": null,
            "cert_sig": "00".repeat(64),
        }),
    );
    assert_eq!(a.ingest_op(&cert).unwrap(), IngestResult::Applied);

    let a_peer = a.principal_hex();
    let a_pk = a.author_pk_hex();
    let p = d1.principal_hex();
    let d1_pk = d1.author_pk_hex();
    let d2_pk = d2.author_pk_hex();
    a.distribute_group_key(&[
        (a_peer.as_str(), a_pk.as_str()),
        (p.as_str(), d1_pk.as_str()),
        (p.as_str(), d2_pk.as_str()),
    ])
    .unwrap();
    let note = a.create_node("Note").unwrap();
    a.set_lww(&note, "body", SECRET).unwrap();

    d1.import_bundle(&a.export_all().unwrap()).unwrap();
    d2.import_bundle(&a.export_all().unwrap()).unwrap();
    assert_eq!(d1.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));
    assert_eq!(
        d2.principal_hex(),
        d1.principal_hex(),
        "kr=0 must bind the second device to the principal"
    );
    assert_eq!(d2.get_lww(&note, "body").unwrap().as_deref(), Some(SECRET));

    let mut stranger =
        LocalStore::init_with_backend_from_seed(MemoryBackend::new(), &[0x99u8; 32], &ds_bytes(&a))
            .unwrap();
    stranger.apply_schema_json(NOTE_SCHEMA).unwrap();
    stranger.import_bundle(&a.export_all().unwrap()).unwrap();
    assert_eq!(stranger.get_lww(&note, "body").unwrap(), None);
    assert!(stranger.decrypt_oracle().unwrap().is_empty());
}

#[test]
fn e6_extra_wrap_field_is_key_wrap_invalid() {
    let (a, mut b, _note, _grant) = e6_share_notes();
    let mut kr = a
        .export_all()
        .unwrap()
        .ops
        .into_iter()
        .find(|op| op.kind == 8 && op.body.get("kr").and_then(|v| v.as_u64()) == Some(2))
        .expect("KeyRecord");
    kr.body["wraps"][0]["extra"] = serde_json::json!("nope");
    let bad = sign_wire(
        &a.identity_seed(),
        &ds_bytes(&a),
        1,
        &control_deps(&a),
        last_physical(&a).saturating_add(1),
        8,
        kr.body,
    );
    let (accepted, skipped) = b
        .import_bundle(&ExportBundle {
            format: 1,
            datastore_id: a.datastore_id_hex(),
            ops: vec![bad],
        })
        .unwrap();
    assert_eq!(accepted, 0);
    assert!(skipped >= 1);
    assert!(
        b.take_rejects()
            .iter()
            .any(|r| r.reason == KEY_WRAP_INVALID)
    );
}
