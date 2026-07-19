//! M1-fix-serve: refuse non-loopback bind without explicit unsafe flag.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use zerodb_storage::LocalStore;

fn tmp_db(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("cli-serve-{name}-{nonce}.sqlite"))
}

#[test]
fn serve_refuses_non_loopback_without_allow_insecure_lan() {
    let path = tmp_db("non-loopback");
    LocalStore::init(&path).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_zerodb"))
        .args([
            "serve",
            "--path",
            path.to_str().unwrap(),
            "--listen",
            "0.0.0.0:17999",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "serve must refuse 0.0.0.0 without --allow-insecure-lan; stderr={stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("allow-insecure-lan")
            || stderr.to_lowercase().contains("loopback")
            || stderr.to_lowercase().contains("insecure"),
        "error should mention loopback / allow-insecure-lan: {stderr}"
    );
}

#[test]
fn serve_accepts_loopback_without_flag() {
    let path = tmp_db("loopback");
    LocalStore::init(&path).unwrap();

    // Bind an ephemeral port; process should start then we kill it.
    // Use a short-lived approach: spawn, wait briefly, ensure it did not exit immediately with error.
    let mut child = Command::new(env!("CARGO_BIN_EXE_zerodb"))
        .args([
            "serve",
            "--path",
            path.to_str().unwrap(),
            "--listen",
            "127.0.0.1:0",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(200));
    match child.try_wait().unwrap() {
        None => {
            // Still running — success for bind acceptance.
            let _ = child.kill();
            let _ = child.wait();
        }
        Some(status) => {
            let stderr = {
                let mut s = String::new();
                if let Some(mut err) = child.stderr.take() {
                    use std::io::Read;
                    let _ = err.read_to_string(&mut s);
                }
                s
            };
            // Port 0 may not be accepted by TcpListener on all platforms the same way;
            // if the process exited, require it was not the insecure-bind policy.
            assert!(
                !stderr.to_lowercase().contains("allow-insecure-lan"),
                "loopback serve rejected by insecure-lan policy: {stderr}"
            );
            // Binding 127.0.0.1:0 is valid; unexpected early exit is still a soft signal.
            // Prefer a fixed high port if :0 is unsupported by clap path.
            if !status.success() {
                // Retry with fixed loopback port.
                let mut child = Command::new(env!("CARGO_BIN_EXE_zerodb"))
                    .args([
                        "serve",
                        "--path",
                        path.to_str().unwrap(),
                        "--listen",
                        "127.0.0.1:17998",
                    ])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                std::thread::sleep(std::time::Duration::from_millis(200));
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "loopback serve should stay running"
                );
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
