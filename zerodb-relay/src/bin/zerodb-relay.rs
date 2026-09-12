//! `zerodb-relay` — experimental L2 WebSocket process (RELAY 0.2.2-draft).
//!
//! Binary WebSocket frames; each message is one CBOR envelope.
//! Loopback by default. Optional in-process TLS (`--tls-cert` + `--tls-key`,
//! PEM) serves `wss://` and may bind non-loopback without `--allow-insecure`.
//! Not a format freeze.

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use zerodb_relay::{ListenConfig, Relay, load_tls_config, serve_connection_with};

#[derive(Parser)]
#[command(
    name = "zerodb-relay",
    about = "ZeroDB experimental L2 relay (RELAY 0.2.2-draft)"
)]
struct Args {
    #[arg(long, default_value = "./relay.sqlite")]
    path: PathBuf,
    /// Bind address. Default loopback only.
    #[arg(long, default_value = "127.0.0.1:7700")]
    bind: String,
    /// Permit a non-loopback plaintext bind. No certificates are minted;
    /// use only on a trusted LAN.
    #[arg(long, default_value_t = false)]
    allow_insecure: bool,
    /// PEM certificate chain; with --tls-key serves wss:// in-process.
    #[arg(long, requires = "tls_key")]
    tls_cert: Option<PathBuf>,
    /// PEM private key (PKCS#8 / SEC1 / PKCS#1) for --tls-cert.
    #[arg(long, requires = "tls_cert")]
    tls_key: Option<PathBuf>,
    /// Global cap on simultaneous connections (all peers).
    #[arg(long, default_value_t = 1024)]
    max_connections: usize,
    /// TLS + WebSocket upgrade + HELLO/AUTH must finish within this many seconds.
    #[arg(long, default_value_t = 10)]
    handshake_timeout_secs: u64,
    /// Close a session idle (no inbound frame / WS ping) this long; 0 disables.
    #[arg(long, default_value_t = 300)]
    idle_timeout_secs: u64,
}

fn main() {
    let args = Args::parse();
    let tls = match (&args.tls_cert, &args.tls_key) {
        (Some(cert), Some(key)) => match load_tls_config(cert, key) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                eprintln!("zerodb-relay: cannot load TLS material: {e}");
                std::process::exit(1);
            }
        },
        _ => None,
    };
    if !zerodb_relay::listen_allowed(&args.bind, args.allow_insecure, tls.is_some()) {
        eprintln!(
            "zerodb-relay refuses non-loopback plaintext bind {:?} without --allow-insecure \
             (pass --tls-cert/--tls-key to serve wss://, use loopback, or pass the flag for \
             disposable LAN tests only)",
            args.bind
        );
        std::process::exit(1);
    }
    let cfg = ListenConfig {
        handshake_timeout: Duration::from_secs(args.handshake_timeout_secs.max(1)),
        idle_timeout: (args.idle_timeout_secs > 0)
            .then(|| Duration::from_secs(args.idle_timeout_secs)),
        max_connections: args.max_connections.max(1),
        tls,
    };
    let scheme = if cfg.tls.is_some() { "wss" } else { "ws" };
    let cfg = Arc::new(cfg);
    let relay = Arc::new(Relay::open(&args.path).expect("open relay store"));
    let listener = TcpListener::bind(&args.bind).expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    // Always print IPv4 loopback: Windows Display of mapped/IPv6 addrs is not
    // a host tungstenite can resolve (`No such host is known`).
    eprintln!(
        "zerodb-relay listening on {scheme}://127.0.0.1:{}",
        addr.port()
    );
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let relay = relay.clone();
                let cfg = cfg.clone();
                std::thread::spawn(move || serve_connection_with(s, &relay, &cfg));
            }
            Err(e) => eprintln!("accept: {e}"),
        }
    }
}
