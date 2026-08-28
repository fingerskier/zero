//! `zerodb-relay` — experimental L2 WebSocket process (RELAY 0.2.2-draft).
//!
//! Binary WebSocket frames; each message is one CBOR envelope.
//! Loopback by default. Not a format freeze.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tungstenite::{Message, accept};
use zerodb_relay::Relay;

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
    /// Permit a non-loopback plaintext bind. The process does not terminate
    /// TLS and does not mint certificates; use only on a trusted LAN.
    #[arg(long, default_value_t = false)]
    allow_insecure: bool,
}

fn main() {
    let args = Args::parse();
    if !zerodb_relay::plaintext_listen_allowed(&args.bind, args.allow_insecure) {
        eprintln!(
            "zerodb-relay refuses non-loopback plaintext bind {:?} without --allow-insecure \
             (no TLS in this binary; use loopback or pass the flag for disposable LAN tests only)",
            args.bind
        );
        std::process::exit(1);
    }
    let relay = Arc::new(Relay::open(&args.path).expect("open relay store"));
    let listener = TcpListener::bind(&args.bind).expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    // Always print IPv4 loopback: Windows Display of mapped/IPv6 addrs is not
    // a host tungstenite can resolve (`No such host is known`).
    eprintln!("zerodb-relay listening on ws://127.0.0.1:{}", addr.port());
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let relay = relay.clone();
                std::thread::spawn(move || serve(s, &relay));
            }
            Err(e) => eprintln!("accept: {e}"),
        }
    }
}

fn serve(stream: TcpStream, relay: &Relay) {
    let mut ws = match accept(stream) {
        Ok(ws) => ws,
        Err(_) => return,
    };
    let mut sess = relay.accept();
    loop {
        let msg = match ws.read() {
            Ok(m) => m,
            Err(_) => break,
        };
        let Message::Binary(frame) = msg else {
            continue;
        };
        let replies = match sess.handle(&frame) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("session: {e}");
                break;
            }
        };
        for r in replies {
            if ws.send(Message::Binary(r)).is_err() {
                return;
            }
        }
        if sess.is_closed() {
            break;
        }
    }
}
