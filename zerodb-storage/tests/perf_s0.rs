//! Stage 0 minimum fixtures: phase counters for 1k-op hot write, import,
//! replay, and one AUTH ingest. Prints timings — do not copy these into
//! README as claims.

use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use zerodb_storage::{LocalStore, MemoryBackend};

const N: usize = 1_000;

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("perf-s0-{name}-{nonce}.sqlite"));
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
    path
}

#[test]
fn phase_counters_1k_hot_write_import_replay() {
    let path = tmp_db("hot");
    let mut store = LocalStore::init(&path).unwrap();
    let node = store.create_node("Todo").unwrap();

    let t0 = Instant::now();
    for i in 0..N {
        store.set_lww(&node, "title", &format!("v{i}")).unwrap();
    }
    let hot_write_ms = t0.elapsed().as_millis();
    let ops = store.op_count().unwrap();
    assert!(ops as usize >= N);

    let bundle = store.export_all().unwrap();
    let dest_path = tmp_db("import");
    let mut dest = LocalStore::init(&dest_path).unwrap();
    let t1 = Instant::now();
    let (accepted, _skipped) = dest.import_bundle(&bundle).unwrap();
    let import_ms = t1.elapsed().as_millis();
    assert!(accepted as usize >= N);

    let t2 = Instant::now();
    dest.replay_all().unwrap();
    let replay_ms = t2.elapsed().as_millis();
    assert_eq!(dest.list_nodes().unwrap().len(), 1);

    eprintln!(
        "perf_s0 sqlite hot_write_1k_ms={hot_write_ms} import_ms={import_ms} replay_ms={replay_ms} ops={ops}"
    );
}

#[test]
fn phase_counters_1k_memory_and_one_auth_ingest() {
    let mut a = LocalStore::init_auth_with_backend(MemoryBackend::new()).unwrap();
    let node = a.create_node("Todo").unwrap();
    let t0 = Instant::now();
    for i in 0..N {
        a.set_lww(&node, "title", &format!("v{i}")).unwrap();
    }
    let hot_write_ms = t0.elapsed().as_millis();

    let mut b = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    a.grant_write_access(&b.principal_hex()).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();

    let t1 = Instant::now();
    let op = b.set_lww(&node, "title", "auth-one").unwrap();
    let auth_ingest_ms = t1.elapsed().as_millis();
    assert!(!op.is_empty());

    let bundle = a.export_all().unwrap();
    let mut c = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let t2 = Instant::now();
    c.import_bundle(&bundle).unwrap();
    let import_ms = t2.elapsed().as_millis();
    let t3 = Instant::now();
    c.replay_all().unwrap();
    let replay_ms = t3.elapsed().as_millis();

    eprintln!(
        "perf_s0 memory hot_write_1k_ms={hot_write_ms} auth_ingest_ms={auth_ingest_ms} import_ms={import_ms} replay_ms={replay_ms}"
    );
}
