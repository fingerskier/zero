//! Test helper for E1 kill-not-shutdown acceptance (`tests/e1_kill_clock.rs`).
//!
//! Opens an already-initialized store at argv[1] and writes ops in a tight
//! loop until killed. Intentionally never shuts down cleanly.

use zerodb_storage::LocalStore;

fn main() {
    let path = std::env::args().nth(1).expect("usage: e1_writer <db-path>");
    let path = std::path::PathBuf::from(path);
    let mut store = LocalStore::open(&path).expect("open store");
    let node = store.create_node("KillProbe").expect("create node");
    let mut i: u64 = 0;
    loop {
        store
            .set_lww(&node, "title", &format!("v{i}"))
            .expect("set_lww");
        store.counter_inc(&node, "count", 1).expect("counter_inc");
        store
            .set_add(&node, "tags", &format!("t{i}"))
            .expect("set_add");
        i += 1;
    }
}
