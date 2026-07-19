//! M1 Wave 1 correctness: CSPRNG seed, HLC recover, storage_format_version.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use zerodb_storage::LocalStore;

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("m1-wave1-{name}-{nonce}.sqlite"));
    remove_sqlite_files(&path);
    path
}

fn remove_sqlite_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

fn meta_blob(path: &Path, key: &str) -> Option<Vec<u8>> {
    let conn = Connection::open(path).unwrap();
    conn.query_row("SELECT v FROM meta WHERE k = ?1", params![key], |r| {
        r.get(0)
    })
    .ok()
}

fn meta_u64(path: &Path, key: &str) -> Option<u64> {
    let conn = Connection::open(path).unwrap();
    let v: Option<Vec<u8>> = conn
        .query_row("SELECT v FROM meta WHERE k = ?1", params![key], |r| {
            r.get(0)
        })
        .ok();
    v.and_then(|b| {
        if b.len() != 8 {
            return None;
        }
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&b);
        Some(u64::from_le_bytes(arr))
    })
}

fn set_meta_u64(path: &Path, key: &str, value: u64) {
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "INSERT INTO meta (k, v) VALUES (?1, ?2)
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![key, value.to_le_bytes().as_slice()],
    )
    .unwrap();
}

fn max_op_ts(path: &Path) -> (u64, u16) {
    let conn = Connection::open(path).unwrap();
    conn.query_row(
        "SELECT physical_ms, logical FROM ops
         ORDER BY physical_ms DESC, logical DESC LIMIT 1",
        [],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u16)),
    )
    .unwrap()
}

fn decode_hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Broken UUID-tiled fill implies seed[i] ^ seed[i+16] == 240 for every i in 0..16.
#[test]
fn init_seed_is_not_uuid_tiled_entropy() {
    let path = tmp_db("seed-entropy");
    let _store = LocalStore::init(&path).unwrap();
    let seed = meta_blob(&path, "seed").expect("seed meta");
    assert_eq!(seed.len(), 32, "seed must be 32 bytes");

    // Broken fill: seed[i] = uuid[i%16] ^ (i*31) ^ clock[i%8] for i in 0..32.
    // Then seed[i] ^ seed[i+16] equals the deterministic (i*31)^( (i+16)*31 ) (u8 wrap).
    let looks_tiled = (0..16).all(|i| {
        let expected = (i as u8).wrapping_mul(31) ^ ((i + 16) as u8).wrapping_mul(31);
        seed[i] ^ seed[i + 16] == expected
    });
    assert!(
        !looks_tiled,
        "Ed25519 seed must not be derived by tiling a 16-byte UUID (G-01)"
    );

    let salt = meta_blob(&path, "salt").expect("salt meta");
    assert_eq!(salt.len(), 16);
}

#[test]
fn open_recovers_hlc_from_oplog_when_meta_is_stale() {
    let path = tmp_db("hlc-recover");
    let mut store = LocalStore::init(&path).unwrap();
    let node = store.create_node("Todo").unwrap();
    store.set_lww(&node, "title", "first").unwrap();
    store.set_lww(&node, "title", "second").unwrap();
    let (max_p, max_l) = max_op_ts(&path);
    assert!(max_p > 0);
    drop(store);

    // Simulate meta cache desync / zeroed HLC while oplog remains.
    set_meta_u64(&path, "hlc_p", 0);
    set_meta_u64(&path, "hlc_l", 0);

    // Open alone (no write) must rewrite durable HLC from the oplog (DQ-7).
    let _reopened = LocalStore::open(&path).unwrap();
    drop(_reopened);
    let meta_p = meta_u64(&path, "hlc_p").unwrap_or(0);
    let meta_l = meta_u64(&path, "hlc_l").unwrap_or(0) as u16;
    assert_eq!(
        (meta_p, meta_l),
        (max_p, max_l),
        "open must persist HLC high-water from oplog when meta is stale"
    );

    let mut reopened = LocalStore::open(&path).unwrap();
    let op = reopened.set_lww(&node, "title", "after-recover").unwrap();
    drop(reopened);

    let conn = Connection::open(&path).unwrap();
    let (p, l): (i64, i64) = conn
        .query_row(
            "SELECT physical_ms, logical FROM ops WHERE id = ?1",
            params![decode_hex(&op)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let p = p as u64;
    let l = l as u16;
    assert!(
        (p, l) > (max_p, max_l),
        "post-recover op HLC ({p},{l}) must strictly exceed durable oplog max ({max_p},{max_l})"
    );
}

#[test]
fn replay_all_restores_hlc_high_water_from_oplog() {
    let path = tmp_db("hlc-replay");
    let mut store = LocalStore::init(&path).unwrap();
    let node = store.create_node("Todo").unwrap();
    store.set_lww(&node, "title", "a").unwrap();
    store.gcounter_inc(&node, "views", 1).unwrap();
    let (max_p, max_l) = max_op_ts(&path);

    // Corrupt durable meta while the store is open; replay_all must re-derive HLC.
    set_meta_u64(&path, "hlc_p", 0);
    set_meta_u64(&path, "hlc_l", 0);
    store.replay_all().unwrap();
    drop(store);

    let meta_p = meta_u64(&path, "hlc_p").unwrap_or(0);
    let meta_l = meta_u64(&path, "hlc_l").unwrap_or(0) as u16;
    assert_eq!(
        (meta_p, meta_l),
        (max_p, max_l),
        "replay_all must persist HLC high-water from oplog"
    );

    let mut store = LocalStore::open(&path).unwrap();
    let op = store.set_lww(&node, "title", "b").unwrap();
    drop(store);

    let conn = Connection::open(&path).unwrap();
    let (p, l): (i64, i64) = conn
        .query_row(
            "SELECT physical_ms, logical FROM ops WHERE id = ?1",
            params![decode_hex(&op)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(
        (p as u64, l as u16) > (max_p, max_l),
        "HLC after replay_all must not regress below oplog max"
    );
}

#[test]
fn init_writes_storage_format_version() {
    let path = tmp_db("fmtver");
    let _store = LocalStore::init(&path).unwrap();
    let v = meta_u64(&path, "storage_format_version")
        .expect("storage_format_version meta must be written at init (G-07)");
    assert_eq!(v, 1, "experimental local layout is version 1");
}

#[test]
fn open_backfills_storage_format_version_on_legacy_db() {
    let path = tmp_db("fmtver-legacy");
    let _store = LocalStore::init(&path).unwrap();
    drop(_store);

    // Simulate pre-fmtver DB.
    let conn = Connection::open(&path).unwrap();
    conn.execute("DELETE FROM meta WHERE k = 'storage_format_version'", [])
        .unwrap();
    drop(conn);

    let _reopened = LocalStore::open(&path).unwrap();
    let v = meta_u64(&path, "storage_format_version")
        .expect("open must backfill storage_format_version");
    assert_eq!(v, 1);
}
