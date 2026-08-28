//! Import vs import+replay equivalence (Stage 1).
//!
//! `import_bundle` rematerializes accepted ops. `replay_all` remains the
//! oracle and recovery API. Success-path push must not need a second replay.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use zerodb_storage::{ExportBundle, LocalStore, MemoryBackend};

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("import-replay-equiv-{name}-{nonce}.sqlite"));
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
    path
}

fn fill_graph<B: zerodb_storage::StoreBackend>(a: &mut LocalStore<B>) -> String {
    let n1 = a.create_node("Todo").unwrap();
    let n2 = a.create_node("Note").unwrap();
    a.set_lww(&n1, "title", "milk").unwrap();
    a.set_lww(&n1, "title", "oat").unwrap();
    a.set_add(&n1, "tags", "x").unwrap();
    a.counter_inc(&n1, "n", 3).unwrap();
    let _e = a.create_edge("child", &n1, &n2).unwrap();
    a.delete_node(&n2).unwrap();
    n1
}

fn seed_bundle_plain() -> ExportBundle {
    let mut a = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    let n1 = fill_graph(&mut a);
    let mut b = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    b.set_lww(&n1, "title", "from-b").unwrap();
    a.import_bundle(&b.export_all().unwrap()).unwrap();
    a.export_all().unwrap()
}

fn seed_bundle_auth() -> ExportBundle {
    let mut a = LocalStore::init_auth_with_backend(MemoryBackend::new()).unwrap();
    let mut b = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    a.grant_write_access(&b.principal_hex()).unwrap();
    let n1 = fill_graph(&mut a);
    b.import_bundle(&a.export_all().unwrap()).unwrap();
    b.set_lww(&n1, "title", "from-b").unwrap();
    a.import_bundle(&b.export_all().unwrap()).unwrap();
    a.export_all().unwrap()
}

fn permute(mut bundle: ExportBundle, rotate: usize) -> ExportBundle {
    if !bundle.ops.is_empty() {
        let n = bundle.ops.len();
        bundle.ops.rotate_left(rotate % n);
    }
    bundle
}

fn reverse_ops(mut bundle: ExportBundle) -> ExportBundle {
    bundle.ops.reverse();
    bundle
}

#[derive(Debug, PartialEq)]
struct NodeSnap {
    id: String,
    label: String,
    deleted: bool,
    props: Vec<(String, String)>,
}

#[derive(Debug, PartialEq)]
struct EdgeSnap {
    id: String,
    label: String,
    src: String,
    dst: String,
    deleted: bool,
    visible: bool,
}

#[derive(Debug, PartialEq)]
struct Snap {
    nodes: Vec<NodeSnap>,
    edges: Vec<EdgeSnap>,
    quarantine: Vec<String>,
    rejects: Vec<(String, &'static str)>,
    ops: u64,
    ds: String,
}

fn snap<B: zerodb_storage::StoreBackend>(store: &mut LocalStore<B>, path: &Path) -> Snap {
    let report = store.inspect(path).unwrap();
    let mut nodes: Vec<_> = report
        .nodes
        .into_iter()
        .map(|n| {
            let mut props: Vec<_> = n
                .props
                .into_iter()
                .map(|(k, v)| (k, v.to_string()))
                .collect();
            props.sort();
            NodeSnap {
                id: n.id,
                label: n.label,
                deleted: n.deleted,
                props,
            }
        })
        .collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let mut edges: Vec<_> = report
        .edges
        .into_iter()
        .map(|e| EdgeSnap {
            id: e.id,
            label: e.label,
            src: e.src,
            dst: e.dst,
            deleted: e.deleted,
            visible: e.visible,
        })
        .collect();
    edges.sort_by(|a, b| a.id.cmp(&b.id));
    let mut quarantine: Vec<_> = store
        .list_quarantine()
        .unwrap()
        .into_iter()
        .map(|w| w.id)
        .collect();
    quarantine.sort();
    let mut rejects: Vec<_> = store
        .take_rejects()
        .into_iter()
        .map(|r| (r.op_id, r.reason))
        .collect();
    rejects.sort_by(|a, b| a.0.cmp(&b.0));
    Snap {
        nodes,
        edges,
        quarantine,
        rejects,
        ops: store.op_count().unwrap(),
        ds: store.datastore_id_hex(),
    }
}

fn pin_wall<B: zerodb_storage::StoreBackend>(store: &mut LocalStore<B>) {
    // Far enough after seed timestamps to be stable, inside the 60s drift
    // window relative to those ops (ops are in the past).
    store.set_test_clock(|| 1_800_000_000_000);
}

fn import_only<B: zerodb_storage::StoreBackend>(
    mut dest: LocalStore<B>,
    bundle: &ExportBundle,
    path: &Path,
) -> (Snap, (u64, u16)) {
    pin_wall(&mut dest);
    dest.import_bundle(bundle).unwrap();
    let hlc = dest.hlc();
    (snap(&mut dest, path), hlc)
}

fn import_then_replay<B: zerodb_storage::StoreBackend>(
    mut dest: LocalStore<B>,
    bundle: &ExportBundle,
    path: &Path,
) -> (Snap, (u64, u16), (u64, u16)) {
    pin_wall(&mut dest);
    dest.import_bundle(bundle).unwrap();
    let hlc_import = dest.hlc();
    dest.replay_all().unwrap();
    let hlc_replay = dest.hlc();
    (snap(&mut dest, path), hlc_import, hlc_replay)
}

fn assert_state_eq(a: &Snap, b: &Snap) {
    assert_eq!(a.ds, b.ds);
    assert_eq!(a.ops, b.ops);
    assert_eq!(a.nodes, b.nodes);
    assert_eq!(a.edges, b.edges);
    assert_eq!(a.quarantine, b.quarantine);
    assert_eq!(a.rejects, b.rejects);
}

#[test]
fn import_matches_import_plus_replay_memory() {
    let bundle = seed_bundle_auth();
    let path = Path::new("mem");
    let (s_import, hlc_import) = import_only(
        LocalStore::init_with_backend(MemoryBackend::new()).unwrap(),
        &bundle,
        path,
    );
    let (s_replay, hlc_before_replay, hlc_after_replay) = import_then_replay(
        LocalStore::init_with_backend(MemoryBackend::new()).unwrap(),
        &bundle,
        path,
    );
    assert_state_eq(&s_import, &s_replay);
    assert_eq!(hlc_import, hlc_before_replay);
    // replay_all rewrites HLC from oplog max (recovery). Ingest HLC may be
    // logical+1 / wall-advanced. Success-path sync keeps import HLC.
    let _ = hlc_after_replay;
}

#[test]
fn import_matches_import_plus_replay_permutation() {
    let bundle = seed_bundle_plain();
    let path = Path::new("mem");
    let rotated = permute(bundle.clone(), 3);
    let reversed = reverse_ops(bundle.clone());
    let (s0, _) = import_only(
        LocalStore::init_with_backend(MemoryBackend::new()).unwrap(),
        &bundle,
        path,
    );
    let (s1, _) = import_only(
        LocalStore::init_with_backend(MemoryBackend::new()).unwrap(),
        &rotated,
        path,
    );
    let (s2, _) = import_only(
        LocalStore::init_with_backend(MemoryBackend::new()).unwrap(),
        &reversed,
        path,
    );
    let (s1r, _, _) = import_then_replay(
        LocalStore::init_with_backend(MemoryBackend::new()).unwrap(),
        &rotated,
        path,
    );
    assert_state_eq(&s0, &s1);
    assert_state_eq(&s0, &s2);
    assert_state_eq(&s1, &s1r);
}

#[test]
fn import_matches_import_plus_replay_sqlite_and_quarantine() {
    let bundle = seed_bundle_auth();
    let p_import = tmp_db("import");
    let p_replay = tmp_db("replay");
    let (s_import, _) = import_only(LocalStore::init(&p_import).unwrap(), &bundle, &p_import);
    let (s_replay, _, _) =
        import_then_replay(LocalStore::init(&p_replay).unwrap(), &bundle, &p_replay);
    assert_state_eq(&s_import, &s_replay);

    // Far-future LWW is quarantined on import and stays held after replay
    // (replay rebuilds projections from the oplog; quarantine is meta).
    fn clock_plus_30d() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 30 * 24 * 60 * 60 * 1000
    }
    let mut c = LocalStore::init_auth_with_backend(MemoryBackend::new()).unwrap();
    c.set_test_clock(clock_plus_30d);
    let node = c.create_node("Todo").unwrap();
    c.set_lww(&node, "title", "future").unwrap();
    let future = c.export_all().unwrap();

    let mut dest = LocalStore::init_with_backend(MemoryBackend::new()).unwrap();
    dest.import_bundle(&future).unwrap();
    let q_before = dest.list_quarantine().unwrap();
    assert!(!q_before.is_empty(), "far-future op must quarantine");
    dest.replay_all().unwrap();
    let q_after = dest.list_quarantine().unwrap();
    let mut ids_before: Vec<_> = q_before.into_iter().map(|w| w.id).collect();
    let mut ids_after: Vec<_> = q_after.into_iter().map(|w| w.id).collect();
    ids_before.sort();
    ids_after.sort();
    assert_eq!(ids_before, ids_after);
    assert!(dest.get_lww(&node, "title").unwrap().is_none());
}
