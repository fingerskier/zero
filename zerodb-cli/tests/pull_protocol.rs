use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;
use zerodb_storage::LocalStore;

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("cli-{name}-{}-{nonce}.sqlite", std::process::id()))
}

fn read_frame(stream: &mut impl Read) -> serde_json::Value {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).unwrap();
    let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
    stream.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn write_frame(stream: &mut impl Write, value: &serde_json::Value) {
    let body = serde_json::to_vec(value).unwrap();
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&body).unwrap();
    stream.flush().unwrap();
}

#[test]
fn pull_rejects_unsupported_hello_ok_version_before_reading_ops() {
    let path = tmp_db("pull-bad-hello-version");
    let store = LocalStore::init(&path).unwrap();
    let datastore_id = store.datastore_id_hex();
    drop(store);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server_datastore_id = datastore_id.clone();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "pull CLI did not connect before the test deadline",
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("test listener accept failed: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let hello = read_frame(&mut stream);
        assert_eq!(hello["v"], 2);
        write_frame(
            &mut stream,
            &json!({
                "v": 3,
                "datastore_id": server_datastore_id,
                "peer": "11".repeat(32),
                "need": [],
            }),
        );
        // Deliberately close without an OpsMsg. A conforming pull rejects the
        // HelloOk version immediately instead of advancing to the ops phase.
    });

    let output = Command::new(env!("CARGO_BIN_EXE_zerodb"))
        .args([
            "pull",
            "--path",
            path.to_str().unwrap(),
            "--from",
            &address.to_string(),
        ])
        .output()
        .unwrap();
    server.join().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "pull accepted HelloOk v3");
    assert!(
        stderr.contains("hello version"),
        "pull failed for the wrong reason: {stderr}",
    );

    let reopened = LocalStore::open(&path).unwrap();
    assert_eq!(reopened.op_count().unwrap(), 0);
    assert_eq!(reopened.datastore_id_hex(), datastore_id);
}
