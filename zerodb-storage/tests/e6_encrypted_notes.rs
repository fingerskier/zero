//! EXEMPLAR E6: encrypted private notes (I-10).
//!
//! A and B share list L; `Note.body` is schema-encrypted LWW. B reads
//! plaintext. Relay-captured artifacts and a full replica handed to
//! non-recipient C do not permit recovery (decrypt oracle included).
//! After A removes B and rotates the group key, B cannot decrypt notes
//! written post-rotation. A's keyring survives SQLite reopen.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use zerodb_core::auth::{SCOPE_READ, SCOPE_SYNC, SCOPE_WRITE};
use zerodb_relay::Relay;
use zerodb_storage::relay_client;
use zerodb_storage::{LocalStore, MemoryBackend, StoreBackend, StoreError};

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
