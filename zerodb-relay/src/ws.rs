//! Live WebSocket accept loop: drain the SIGNAL mailbox onto the target socket.
//!
//! Hardening (see [`ListenConfig`]): tungstenite rejects any message above
//! [`MAX_FRAME_BYTES`] before it is buffered (no CBOR decode of oversized
//! input), the TLS + WebSocket upgrade and HELLO/AUTH must finish inside
//! `handshake_timeout`, a session with no inbound traffic for `idle_timeout`
//! is told GOODBYE and closed, and at most `max_connections` sockets hold a
//! [`ConnectionSlot`] at once. Optional in-process TLS via rustls.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket, accept_with_config};
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::relay::{ERR_PAYLOAD_TOO_LARGE, ERR_RATE_EXCEEDED, MSG_ERROR, MSG_GOODBYE};

use crate::session::ConnectionSlot;
use crate::{ListenConfig, MAX_FRAME_BYTES, Relay};

/// How long a blocked `read` waits before we drain forwarded SIGNAL frames.
/// The target session is otherwise stuck in `ws.read()` and would never write.
const OUTBOUND_POLL: Duration = Duration::from_millis(10);

/// RELAY §10.2 `PROTOCOL_ERROR` (fatal) — used for the handshake deadline.
const ERR_PROTOCOL_ERROR: u16 = 0x100;

fn ws_config() -> WebSocketConfig {
    // Same ceiling `RelaySession::handle` enforces, applied before buffering.
    WebSocketConfig {
        max_message_size: Some(MAX_FRAME_BYTES),
        max_frame_size: Some(MAX_FRAME_BYTES),
        ..Default::default()
    }
}

fn frame(ty: u8, request_id: u32, payload: Cbor) -> Vec<u8> {
    cbor::encode(&Cbor::Map(vec![
        ("type".into(), Cbor::Uint(ty as u64)),
        ("request_id".into(), Cbor::Uint(request_id as u64)),
        ("payload".into(), payload),
    ]))
    .expect("frame cbor")
}

fn error_frame(code: u16, message: &str, fatal: bool) -> Vec<u8> {
    frame(
        MSG_ERROR,
        0,
        Cbor::Map(vec![
            ("code".into(), Cbor::Uint(code as u64)),
            ("message".into(), Cbor::Text(message.into())),
            ("fatal".into(), Cbor::Bool(fatal)),
        ]),
    )
}

fn goodbye_frame(reason: u16, message: &str) -> Vec<u8> {
    frame(
        MSG_GOODBYE,
        0,
        Cbor::Map(vec![
            ("reason".into(), Cbor::Uint(reason as u64)),
            ("message".into(), Cbor::Text(message.into())),
        ]),
    )
}

/// Serve one accepted TCP stream with the default [`ListenConfig`].
pub fn serve_connection(stream: TcpStream, relay: &Relay) {
    serve_connection_with(stream, relay, &ListenConfig::default())
}

/// Serve one accepted TCP stream as a RELAY 0.2 WebSocket session (plaintext
/// or TLS per `cfg.tls`).
pub fn serve_connection_with(stream: TcpStream, relay: &Relay, cfg: &ListenConfig) {
    let _ = stream.set_nodelay(true);
    // Bound the TLS + HTTP upgrade: a silent TCP client must not pin a thread.
    let _ = stream.set_read_timeout(Some(cfg.handshake_timeout));
    let slot = relay.acquire_connection(cfg.max_connections);
    match cfg.tls.clone() {
        None => {
            let Ok(ws) = accept_with_config(stream, Some(ws_config())) else {
                return;
            };
            let _ = ws.get_ref().set_read_timeout(Some(OUTBOUND_POLL));
            serve_ws(ws, relay, cfg, slot);
        }
        Some(tls) => {
            let Ok(conn) = rustls::ServerConnection::new(tls) else {
                return;
            };
            let tls_stream = rustls::StreamOwned::new(conn, stream);
            let Ok(ws) = accept_with_config(tls_stream, Some(ws_config())) else {
                return;
            };
            let _ = ws.get_ref().sock.set_read_timeout(Some(OUTBOUND_POLL));
            serve_ws(ws, relay, cfg, slot);
        }
    }
}

fn send_and_close<S: Read + Write>(ws: &mut WebSocket<S>, frame: Vec<u8>) {
    let _ = ws.send(Message::Binary(frame));
    let _ = ws.close(None);
    let _ = ws.flush();
}

fn serve_ws<S: Read + Write>(
    mut ws: WebSocket<S>,
    relay: &Relay,
    cfg: &ListenConfig,
    slot: Option<ConnectionSlot>,
) {
    let Some(_slot) = slot else {
        // Over the global cap: say why, then go. The slot is never held.
        send_and_close(
            &mut ws,
            error_frame(ERR_RATE_EXCEEDED, "TOO_MANY_CONNECTIONS", true),
        );
        return;
    };
    let mut sess = relay.accept();
    let accepted = Instant::now();
    let mut last_inbound = accepted;
    loop {
        match ws.read() {
            Ok(Message::Binary(frame)) => {
                last_inbound = Instant::now();
                let replies = match sess.handle(&frame) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("session: {e}");
                        break;
                    }
                };
                let mut dead = false;
                for r in replies {
                    if ws.send(Message::Binary(r)).is_err() {
                        dead = true;
                        break;
                    }
                }
                if dead {
                    break;
                }
            }
            Ok(Message::Ping(p)) => {
                last_inbound = Instant::now();
                if ws.send(Message::Pong(p)).is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => break,
            Ok(_) => last_inbound = Instant::now(),
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(tungstenite::Error::Capacity(_)) => {
                // Above MAX_FRAME_BYTES: never buffered, never decoded.
                send_and_close(
                    &mut ws,
                    error_frame(ERR_PAYLOAD_TOO_LARGE, "PAYLOAD_TOO_LARGE", true),
                );
                break;
            }
            Err(_) => break,
        }
        if !sess.is_authed() && accepted.elapsed() > cfg.handshake_timeout {
            send_and_close(
                &mut ws,
                error_frame(ERR_PROTOCOL_ERROR, "HANDSHAKE_TIMEOUT", true),
            );
            break;
        }
        if let Some(idle) = cfg.idle_timeout
            && last_inbound.elapsed() > idle
        {
            send_and_close(&mut ws, goodbye_frame(0, "IDLE_TIMEOUT"));
            break;
        }
        let mut dead = false;
        for frame in sess.take_outbound() {
            if ws.send(Message::Binary(frame)).is_err() {
                dead = true;
                break;
            }
        }
        if dead || sess.is_closed() {
            break;
        }
    }
}

/// Bind `addr` and accept WebSocket sessions on background threads (default config).
pub fn spawn_listener(addr: &str, relay: Arc<Relay>) -> std::io::Result<std::net::SocketAddr> {
    spawn_listener_with(addr, relay, ListenConfig::default())
}

/// Bind `addr` and accept sessions on background threads with `cfg`.
pub fn spawn_listener_with(
    addr: &str,
    relay: Arc<Relay>,
    cfg: ListenConfig,
) -> std::io::Result<std::net::SocketAddr> {
    let listener = TcpListener::bind(addr)?;
    let local = listener.local_addr()?;
    let cfg = Arc::new(cfg);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    let relay = relay.clone();
                    let cfg = cfg.clone();
                    std::thread::spawn(move || serve_connection_with(s, &relay, &cfg));
                }
                Err(_) => break,
            }
        }
    });
    Ok(local)
}
