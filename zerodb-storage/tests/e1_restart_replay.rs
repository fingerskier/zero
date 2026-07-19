//! E1-style: CRUD, restart, fresh replay equivalence.

use std::path::PathBuf;

use rusqlite::{Connection, params};
use zerodb_storage::LocalStore;

fn tmp_db(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("e1-{name}.sqlite"));
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn e1_restart_and_replay_match() {
    let path = tmp_db("main");
    let mut store = LocalStore::init(&path).unwrap();
    let node = store.create_node("Todo").unwrap();
    store.set_lww(&node, "title", "milk").unwrap();
    store.set_lww(&node, "title", "oat milk").unwrap();
    store.gcounter_inc(&node, "views", 2).unwrap();
    store.gcounter_inc(&node, "views", 3).unwrap();
    store.counter_inc(&node, "score", 5).unwrap();
    store.counter_dec(&node, "score", 2).unwrap();
    store.set_add(&node, "tags", "food").unwrap();
    store.set_add(&node, "tags", "urgent").unwrap();
    store.set_remove(&node, "tags", "urgent").unwrap();
    store.flag_enable(&node, "done").unwrap();
    store.flag_disable(&node, "done").unwrap();
    store.flag_enable(&node, "done").unwrap();

    let title = store.get_lww(&node, "title").unwrap();
    let views = store.get_prop(&node, "views").unwrap();
    let score = store.get_prop(&node, "score").unwrap();
    let tags = store.get_prop(&node, "tags").unwrap();
    let done = store.get_prop(&node, "done").unwrap();
    let ops = store.op_count().unwrap();

    // Restart
    drop(store);
    let store2 = LocalStore::open(&path).unwrap();
    assert_eq!(store2.get_lww(&node, "title").unwrap(), title);
    assert_eq!(store2.get_prop(&node, "views").unwrap(), views);
    assert_eq!(store2.get_prop(&node, "score").unwrap(), score);
    assert_eq!(store2.get_prop(&node, "tags").unwrap(), tags);
    assert_eq!(store2.get_prop(&node, "done").unwrap(), done);
    assert_eq!(store2.op_count().unwrap(), ops);

    // Fresh replay on empty store via export/import
    let bundle = store2.export_all().unwrap();
    let path2 = tmp_db("replay");
    let mut fresh = LocalStore::init(&path2).unwrap();
    let (a, _) = fresh.import_bundle(&bundle).unwrap();
    assert!(a >= 1);
    assert_eq!(fresh.get_lww(&node, "title").unwrap(), title);
    assert_eq!(fresh.get_prop(&node, "views").unwrap(), views);
    assert_eq!(fresh.get_prop(&node, "score").unwrap(), score);
    assert_eq!(fresh.get_prop(&node, "tags").unwrap(), tags);
    assert_eq!(fresh.get_prop(&node, "done").unwrap(), done);

    // Local replay_all
    let mut store3 = LocalStore::open(&path).unwrap();
    store3.replay_all().unwrap();
    assert_eq!(store3.get_lww(&node, "title").unwrap(), title);
    assert_eq!(store3.get_prop(&node, "views").unwrap(), views);
    assert_eq!(store3.get_prop(&node, "score").unwrap(), score);
    assert_eq!(store3.get_prop(&node, "tags").unwrap(), tags);
    assert_eq!(store3.get_prop(&node, "done").unwrap(), done);

    // Expected semantics
    assert_eq!(title.as_deref(), Some("oat milk"));
    assert_eq!(views, Some(serde_json::json!(5)));
    assert_eq!(score, Some(serde_json::json!(3)));
    assert_eq!(tags, Some(serde_json::json!(["food"])));
    assert_eq!(done, Some(serde_json::json!(true)));
}

#[test]
fn e9_delete_hides_props() {
    let path = tmp_db("del");
    let mut store = LocalStore::init(&path).unwrap();
    let node = store.create_node("Todo").unwrap();
    store.set_lww(&node, "title", "gone").unwrap();
    store.delete_node(&node).unwrap();
    assert!(store.is_deleted(&node).unwrap());
    assert_eq!(store.get_lww(&node, "title").unwrap(), None);
    let nodes = store.list_nodes().unwrap();
    assert!(nodes.iter().any(|(id, _, d)| id == &node && *d));
}

#[test]
fn multi_peer_lww_converges() {
    let pa = tmp_db("peer-a");
    let pb = tmp_db("peer-b");
    let mut a = LocalStore::init(&pa).unwrap();
    let mut b = LocalStore::init(&pb).unwrap();
    let node = a.create_node("Todo").unwrap();
    a.set_lww(&node, "title", "from-a").unwrap();
    let bundle = a.export_all().unwrap();
    b.import_bundle(&bundle).unwrap();
    b.set_lww(&node, "title", "from-b").unwrap();
    a.set_lww(&node, "title", "from-a2").unwrap();
    let ba = a.export_all().unwrap();
    let bb = b.export_all().unwrap();
    a.import_bundle(&bb).unwrap();
    b.import_bundle(&ba).unwrap();
    assert_eq!(
        a.get_lww(&node, "title").unwrap(),
        b.get_lww(&node, "title").unwrap()
    );
}

#[test]
fn replay_all_rebuilds_materialized_tables_exactly_from_the_oplog() {
    let path = tmp_db("exact-rebuild");
    let mut store = LocalStore::init(&path).unwrap();
    let node = store.create_node("FlowProbe").unwrap();
    store.set_lww(&node, "title", "fresh").unwrap();
    drop(store);

    let orphan = "ffffffffffffffffffffffffffffffff";
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE nodes SET label = 'Stale', deleted = 1 WHERE id = ?1",
        params![node],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO nodes (id, label, deleted) VALUES (?1, 'Orphan', 1)",
        params![orphan],
    )
    .unwrap();
    conn.execute(
        "UPDATE props SET crdt = 'flag', value_json = 'true' WHERE entity = ?1 AND path = 'title'",
        params![node],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO props (entity, path, crdt, value_json) VALUES (?1, 'ghost', 'lww', '\"stale\"')",
        params![orphan],
    )
    .unwrap();
    drop(conn);

    let mut reopened = LocalStore::open(&path).unwrap();
    reopened.replay_all().unwrap();
    assert_eq!(
        reopened.list_nodes().unwrap(),
        vec![(node.clone(), "FlowProbe".to_string(), false)],
        "replay must discard materialized nodes with no CreateNode in the oplog",
    );
    assert_eq!(
        reopened.get_prop(&node, "title").unwrap(),
        Some(serde_json::json!("fresh")),
    );
    drop(reopened);

    let conn = Connection::open(&path).unwrap();
    let mut stmt = conn
        .prepare("SELECT entity, path, crdt, value_json FROM props ORDER BY entity, path")
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![(
            node,
            "title".to_string(),
            "lww".to_string(),
            "\"fresh\"".to_string(),
        )],
        "replay must derive the complete property table from property operations",
    );
}
