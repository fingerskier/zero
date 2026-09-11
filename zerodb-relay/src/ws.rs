//! Live WebSocket accept loop: drain the SIGNAL mailbox onto the target socket.

use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use tungstenite::{Message, accept};

use crate::Relay;

/// How long a blocked `read` waits before we drain forwarded SIGNAL frames.
/// The target session is otherwise stuck in `ws.read()` and would never write.
const OUTBOUND_POLL: Duration = Duration::from_millis(10);

/// Serve one accepted TCP stream as a RELAY 0.2 WebSocket session.
pub fn serve_connection(stream: TcpStream, relay: &Relay) {
    let _ = stream.set_nodelay(true);
    let mut ws = match accept(stream) {
        Ok(ws) => ws,
        Err(_) => return,
    };
    // After the HTTP upgrade: poll so a forwarded SIGNAL is written without
    // waiting for the target to send another frame.
    let _ = ws.get_ref().set_read_timeout(Some(OUTBOUND_POLL));
    let mut sess = relay.accept();
    loop {
        match ws.read() {
            Ok(Message::Binary(frame)) => {
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
                if ws.send(Message::Pong(p)).is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(_) => break,
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

/// Bind `addr` and accept WebSocket sessions on background threads.
pub fn spawn_listener(addr: &str, relay: Arc<Relay>) -> std::io::Result<std::net::SocketAddr> {
    let listener = TcpListener::bind(addr)?;
    let local = listener.local_addr()?;
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    let relay = relay.clone();
                    std::thread::spawn(move || serve_connection(s, &relay));
                }
                Err(_) => break,
            }
        }
    });
    Ok(local)
}
