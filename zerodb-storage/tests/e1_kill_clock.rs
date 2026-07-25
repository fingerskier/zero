//! E1 hard evidence: kill-not-shutdown durability + HLC monotonicity under
//! wall-clock rollback across restart (doc/EXEMPLAR.md E1; plan/PLAN.md §5.3).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use zerodb_storage::LocalStore;

fn tmp_db(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("e1kc-{name}.sqlite"));
    // remove main db + WAL sidecars from any previous run
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(PathBuf::from(format!("{}{suffix}", p.display())));
    }
    p
}

fn raw_op_count(path: &PathBuf) -> u64 {
    let conn = Connection::open(path).unwrap();
    conn.query_row("SELECT COUNT(*) FROM ops", [], |r| r.get::<_, u64>(0))
        .unwrap()
}

/// Projection of the materialized state: nodes + all visible props, as JSON.
fn projection(store: &LocalStore) -> serde_json::Value {
    let mut nodes = Vec::new();
    for (id, label, deleted) in store.list_nodes().unwrap() {
        let title = store.get_prop(&id, "title").unwrap();
        let count = store.get_prop(&id, "count").unwrap();
        let tags = store.get_prop(&id, "tags").unwrap();
        nodes.push(serde_json::json!({
            "id": id, "label": label, "deleted": deleted,
            "title": title, "count": count, "tags": tags,
        }));
    }
    serde_json::json!({ "ops": store.op_count().unwrap(), "nodes": nodes })
}

/// Kill-not-shutdown: repeatedly hard-kill a child process mid-write, then
/// prove the survivor db opens, replays, re-verifies every op signature, and
/// its materialized state equals a fresh replay projection.
#[test]
fn e1_kill_mid_write_survives_and_replays() {
    let path = tmp_db("kill");
    // Parent initializes; helper only opens (kill during init is out of scope).
    drop(LocalStore::init(&path).unwrap());

    let helper = env!("CARGO_BIN_EXE_e1_writer");
    let mut prev_ops: u64 = 0;

    for iter in 0..5 {
        let before = raw_op_count(&path);
        let mut child = Command::new(helper)
            .arg(&path)
            .spawn()
            .expect("spawn e1_writer helper");

        // Wait until the child is actively writing, then kill it mid-stream.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if raw_op_count(&path) > before + 3 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "iter {iter}: helper produced no ops before deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        // Hard kill (TerminateProcess on Windows) — never a clean shutdown.
        child.kill().expect("kill child");
        child.wait().expect("wait child");

        // 1) Reopen must succeed after every kill.
        let mut store = LocalStore::open(&path).unwrap();

        // 2) Op count monotonically consistent — no rollback, no partial rows.
        let ops = store.op_count().unwrap();
        assert!(
            ops > prev_ops,
            "iter {iter}: op count must grow ({prev_ops} -> {ops})"
        );
        prev_ops = ops;

        // 3) Materialized state must equal a fresh replay of the oplog.
        let before_replay = projection(&store);
        store.replay_all().unwrap();
        let after_replay = projection(&store);
        assert_eq!(
            before_replay, after_replay,
            "iter {iter}: materialized state diverged from oplog replay"
        );

        // 4) Every persisted op must re-verify (signature + id) on ingress
        //    into a brand-new store, and project to identical state.
        let bundle = store.export_all().unwrap();
        assert_eq!(bundle.ops.len() as u64, ops);
        let fresh_path = tmp_db(&format!("kill-fresh-{iter}"));
        let mut fresh = LocalStore::init(&fresh_path).unwrap();
        let (accepted, rejected) = fresh.import_bundle(&bundle).unwrap();
        assert_eq!(
            rejected, 0,
            "iter {iter}: persisted op failed signature/ingress re-verification"
        );
        assert_eq!(accepted as u64, ops, "iter {iter}: op lost on re-import");
        let fresh_proj = projection(&fresh);
        assert_eq!(
            after_replay, fresh_proj,
            "iter {iter}: fresh replay projection differs from survivor state"
        );
    }
    assert!(prev_ops > 3, "kill loop never produced meaningful op volume");
}

// --- clock rollback -------------------------------------------------------

static ROLLED_BACK_CLOCK_MS: AtomicU64 = AtomicU64::new(0);

fn rolled_back_clock() -> u64 {
    ROLLED_BACK_CLOCK_MS.load(Ordering::SeqCst)
}

/// E1: HLC must stay strictly monotone across restart even when the wall
/// clock is rolled back one hour. The logical counter takes over.
#[test]
fn e1_hlc_monotone_across_restart_with_clock_rollback() {
    let path = tmp_db("clock");
    let mut store = LocalStore::init(&path).unwrap();
    let node = store.create_node("ClockProbe").unwrap();
    for i in 0..5 {
        store.set_lww(&node, "title", &format!("pre{i}")).unwrap();
    }
    // Max HLC persisted before "shutdown".
    let pre_max = store
        .export_all()
        .unwrap()
        .ops
        .iter()
        .map(|op| (op.ts.p, op.ts.l))
        .max()
        .unwrap();
    drop(store);

    // Restart with the physical clock rolled back one hour.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    ROLLED_BACK_CLOCK_MS.store(now - 3_600_000, Ordering::SeqCst);

    let mut store = LocalStore::open(&path).unwrap();
    store.set_test_clock(rolled_back_clock);
    let pre_count = store.op_count().unwrap();
    for i in 0..20 {
        store.set_lww(&node, "title", &format!("post{i}")).unwrap();
    }

    // Every post-restart op ts must be strictly greater than the pre-restart
    // max, and the post-restart sequence itself strictly monotone.
    let mut post: Vec<(u64, u16)> = store
        .export_all()
        .unwrap()
        .ops
        .iter()
        .map(|op| (op.ts.p, op.ts.l))
        .filter(|ts| *ts > pre_max)
        .collect();
    assert_eq!(
        post.len() as u64,
        20,
        "all post-restart ops must exceed the pre-restart HLC max despite \
         the wall clock being 1h behind"
    );
    assert_eq!(store.op_count().unwrap(), pre_count + 20);
    post.sort();
    for w in post.windows(2) {
        assert!(w[0] < w[1], "post-restart HLC not strictly monotone: {w:?}");
    }
    // Logical counter took over: physical stayed pinned at the recovered max.
    assert!(
        post.iter().all(|(p, _)| *p >= pre_max.0),
        "physical component regressed below recovered HLC high-water"
    );
}
