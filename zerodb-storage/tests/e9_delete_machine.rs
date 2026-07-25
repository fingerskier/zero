//! E9 — H3 deterministic delete machine on the derived-visibility model.
//!
//! Semantics under test (M1-LOCAL §4/§7):
//! - Edge visibility = NOT edge-tombstoned AND both endpoints exist and are
//!   not deleted. Purely derived on read — node delete emits NO cascade ops.
//! - Edge delete is a kind-4 Tombstone with body `{ edge, tombstone: true }`
//!   (KERNEL kind 4 entity ref: node xor edge), set-derived and
//!   order-independent like node tombstones.
//! - Properties of tombstoned/hidden entities stay in the oplog but are
//!   excluded from materialized visibility and query.
//! - Late-arriving ops (edge to a dead endpoint, tombstone before create)
//!   resolve identically in every permutation, and replay is an identity.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use zerodb_storage::{ExportBundle, LocalStore};

const KIND_CREATE_EDGE: u64 = 2;
const KIND_TOMBSTONE: u64 = 4;

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("e9-{name}-{nonce}.sqlite"));
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", path.display()));
    }
    path
}

fn normalized(store: &LocalStore, path: &Path) -> Value {
    let report = store.inspect(path).unwrap();
    let mut nodes = report.nodes;
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let mut edges = report.edges;
    edges.sort_by(|a, b| a.id.cmp(&b.id));
    serde_json::json!({
        "nodes": nodes.iter().map(|n| serde_json::json!({
            "id": n.id, "label": n.label, "deleted": n.deleted, "props": n.props,
        })).collect::<Vec<_>>(),
        "edges": edges.iter().map(|e| serde_json::json!({
            "id": e.id, "label": e.label, "src": e.src, "dst": e.dst,
            "deleted": e.deleted, "visible": e.visible,
        })).collect::<Vec<_>>(),
    })
}

/// Is a specific edge-tombstone op (kind 4 with body.edge)?
fn is_edge_tombstone(op: &zerodb_storage::WireOp) -> bool {
    op.kind == KIND_TOMBSTONE && op.body.get("edge").is_some()
}

fn reorder<F: Fn(&zerodb_storage::WireOp) -> u8>(mut bundle: ExportBundle, key: F) -> ExportBundle {
    bundle.ops.sort_by_key(|op| key(op));
    bundle
}

fn import_into(name: &str, bundle: &ExportBundle) -> (LocalStore, PathBuf) {
    let path = tmp_db(name);
    let mut dest = LocalStore::init(&path).unwrap();
    dest.import_bundle(bundle).unwrap();
    (dest, path)
}

/// Every permutation must materialize the same state as the source, and
/// replay + restart must be identities.
fn assert_permutation_identity(
    source: &LocalStore,
    source_path: &Path,
    dest_specs: Vec<(LocalStore, PathBuf)>,
) {
    let expected = normalized(source, source_path);
    for (mut dest, path) in dest_specs {
        assert_eq!(
            normalized(&dest, &path),
            expected,
            "arrival order changed state"
        );
        dest.replay_all().unwrap();
        assert_eq!(
            normalized(&dest, &path),
            expected,
            "replay must be identity"
        );
        drop(dest);
        let dest = LocalStore::open(&path).unwrap();
        assert_eq!(
            normalized(&dest, &path),
            expected,
            "restart must be identity"
        );
    }
}

// --- Edge tombstone: local semantics ---

#[test]
fn e9_delete_edge_tombstones_edge_and_survives_replay_restart() {
    let path = tmp_db("edge-del");
    let mut store = LocalStore::init(&path).unwrap();
    let a = store.create_node("Todo").unwrap();
    let b = store.create_node("Note").unwrap();
    let edge = store.create_edge("NOTATES", &b, &a).unwrap();
    assert_eq!(store.list_edges_visible().unwrap().len(), 1);
    let ops_before = store.op_count().unwrap();

    store.delete_edge(&edge).unwrap();
    assert_eq!(
        store.op_count().unwrap(),
        ops_before + 1,
        "exactly one tombstone op"
    );
    assert!(store.list_edges_visible().unwrap().is_empty());
    let report = store.inspect(Path::new(&path)).unwrap();
    let e = report.edges.iter().find(|e| e.id == edge).unwrap();
    assert!(e.deleted, "edge tombstone marks edge deleted");
    assert!(!e.visible);
    // Endpoints stay visible: edge delete does not cascade to nodes.
    assert!(!store.is_deleted(&a).unwrap());
    assert!(!store.is_deleted(&b).unwrap());

    let pre = normalized(&store, &path);
    store.replay_all().unwrap();
    assert_eq!(
        normalized(&store, &path),
        pre,
        "replay identity after edge delete"
    );
    drop(store);
    let store = LocalStore::open(&path).unwrap();
    assert_eq!(
        normalized(&store, &path),
        pre,
        "restart identity after edge delete"
    );
}

// --- Permutations: tombstone-before-edge / edge-before-tombstone ---

#[test]
fn e9_edge_tombstone_order_independent() {
    let source_path = tmp_db("perm-src");
    let mut source = LocalStore::init(&source_path).unwrap();
    let a = source.create_node("Todo").unwrap();
    let b = source.create_node("Note").unwrap();
    let edge = source.create_edge("NOTATES", &b, &a).unwrap();
    source.delete_edge(&edge).unwrap();
    let full = source.export_all().unwrap();

    // Edge create before its tombstone.
    let create_first = reorder(full.clone(), |op| match op.kind {
        KIND_CREATE_EDGE => 1,
        k if k == KIND_TOMBSTONE => 2,
        _ => 0,
    });
    // Tombstone before the edge create (orphan tombstone, then late create).
    let tomb_first = reorder(full, |op| {
        if is_edge_tombstone(op) {
            0
        } else if op.kind == KIND_CREATE_EDGE {
            2
        } else {
            1
        }
    });
    assert!(
        tomb_first.ops.iter().position(is_edge_tombstone).unwrap()
            < tomb_first
                .ops
                .iter()
                .position(|o| o.kind == KIND_CREATE_EDGE)
                .unwrap(),
        "setup: tombstone delivered before edge create"
    );

    let d1 = import_into("perm-ct", &create_first);
    let d2 = import_into("perm-tc", &tomb_first);
    for (dest, dpath) in [&d1, &d2] {
        let report = dest.inspect(dpath).unwrap();
        let e = report.edges.iter().find(|e| e.id == edge).unwrap();
        assert!(
            e.deleted && !e.visible,
            "edge deleted in every arrival order"
        );
    }
    assert_permutation_identity(&source, &source_path, vec![d1, d2]);
}

// --- Late edge to an already-tombstoned endpoint ---

#[test]
fn e9_late_edge_to_dead_endpoint_hidden_in_every_order() {
    let source_path = tmp_db("late-src");
    let mut source = LocalStore::init(&source_path).unwrap();
    let a = source.create_node("Todo").unwrap();
    let b = source.create_node("Note").unwrap();
    source.delete_node(&a).unwrap();
    // Edge created after the endpoint died (models a late/concurrent edge).
    let edge = source.create_edge("NOTATES", &b, &a).unwrap();
    assert!(source.list_edges_visible().unwrap().is_empty());
    let full = source.export_all().unwrap();

    // Permutation 1: edge arrives before the node tombstone.
    let edge_first = reorder(full.clone(), |op| match op.kind {
        KIND_CREATE_EDGE => 1,
        k if k == KIND_TOMBSTONE => 2,
        _ => 0,
    });
    // Permutation 2: tombstone arrives before the edge (and before creates).
    let tomb_first = reorder(full, |op| match op.kind {
        k if k == KIND_TOMBSTONE => 0,
        KIND_CREATE_EDGE => 2,
        _ => 1,
    });

    let d1 = import_into("late-ef", &edge_first);
    let d2 = import_into("late-tf", &tomb_first);
    for (dest, dpath) in [&d1, &d2] {
        assert!(dest.list_edges_visible().unwrap().is_empty());
        let report = dest.inspect(dpath).unwrap();
        let e = report.edges.iter().find(|e| e.id == edge).unwrap();
        assert!(!e.visible, "late edge to dead endpoint is hidden");
        assert!(
            !e.deleted,
            "derived visibility: node delete emits no cascade edge tombstone"
        );
    }
    assert_permutation_identity(&source, &source_path, vec![d1, d2]);
}

// --- Node delete hides incident edges and props; oplog retains everything ---

#[test]
fn e9_node_delete_hides_edges_and_props_but_retains_ops() {
    let path = tmp_db("hide");
    let mut store = LocalStore::init(&path).unwrap();
    let a = store.create_node("Todo").unwrap();
    let b = store.create_node("Note").unwrap();
    store.set_lww(&a, "title", "milk").unwrap();
    store.set_add(&a, "tags", "food").unwrap();
    store.create_edge("NOTATES", &b, &a).unwrap();
    let ops_before = store.op_count().unwrap();

    store.delete_node(&a).unwrap();
    // Retained in oplog: only the one tombstone was added, nothing removed.
    assert_eq!(store.op_count().unwrap(), ops_before + 1);
    // Excluded from materialized visibility.
    assert_eq!(store.get_prop(&a, "title").unwrap(), None);
    assert_eq!(store.get_prop(&a, "tags").unwrap(), None);
    assert!(store.list_edges_visible().unwrap().is_empty());
    let report = store.inspect(Path::new(&path)).unwrap();
    let node = report.nodes.iter().find(|n| n.id == a).unwrap();
    assert!(node.deleted);
    assert!(
        node.props.is_empty(),
        "props of tombstoned node hidden in inspect"
    );
    // Excluded from query.
    let rows = store.query(r#"MATCH (t:Todo) RETURN t.title"#).unwrap();
    assert!(
        rows.as_array().unwrap().is_empty(),
        "query excludes tombstoned node"
    );

    // Re-creating equivalent content under a NEW id does not resurrect
    // the deleted node's edges (no resurrection).
    let a2 = store.create_node("Todo").unwrap();
    store.set_lww(&a2, "title", "milk").unwrap();
    assert!(store.list_edges_visible().unwrap().is_empty());
    let rows = store.query(r#"MATCH (t:Todo) RETURN t.title"#).unwrap();
    assert_eq!(
        rows.as_array().unwrap().len(),
        1,
        "only the new node is visible"
    );
}

// --- Query excludes edge-tombstoned edges ---

#[test]
fn e9_query_excludes_tombstoned_edges() {
    let path = tmp_db("query-edge");
    let mut store = LocalStore::init(&path).unwrap();
    let list = store.create_node("List").unwrap();
    let todo = store.create_node("Todo").unwrap();
    store.set_lww(&todo, "title", "milk").unwrap();
    let edge = store.create_edge("CONTAINS", &list, &todo).unwrap();

    let q = r#"MATCH (l:List)-[:CONTAINS]->(t:Todo) RETURN t.title"#;
    let rows = store.query(q).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);

    store.delete_edge(&edge).unwrap();
    let rows = store.query(q).unwrap();
    assert!(
        rows.as_array().unwrap().is_empty(),
        "query excludes deleted edge"
    );
}
