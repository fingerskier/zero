//! Peer sync sessions over any `Read + Write` stream (TCP today, WS later).
//!
//! Protocol v2 (length-prefixed JSON frames, u32 BE lengths):
//! client -> Hello { v, datastore_id, peer, op_ids }
//! server -> HelloOk { v, datastore_id, peer, need }
//! server -> OpsMsg { ops }   (ops the client lacks)
//! client -> OpsMsg { ops }   (ops from `need` the client has)
//! server -> OpsAck { accepted, skipped }
//! Both sides ingest via the prevalidated `import_bundle` path.
//!
//! Push capability (still protocol v2, negotiated — no version bump): when the
//! client's Hello carries `push: true` and the server acks with `push: true`,
//! the session stays open after the one-shot exchange. Either side then sends
//! OpsMsg frames whenever new local ops land; each OpsMsg is answered with an
//! OpsAck. Both fields are `#[serde(default)]`, so old peers (which ignore
//! unknown fields and omit the flag) fall back to the one-shot session.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{ExportBundle, LocalStore, StoreBackend, StoreError, WireOp};

pub const SYNC_PROTOCOL_VERSION: u32 = 2;
const MAX_MSG_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub struct Hello {
    pub v: u32,
    pub datastore_id: String,
    pub peer: String,
    pub op_ids: Vec<String>,
    /// v2 push capability request (absent = false on old peers).
    #[serde(default)]
    pub push: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HelloOk {
    pub v: u32,
    pub datastore_id: String,
    pub peer: String,
    pub need: Vec<String>,
    /// v2 push capability ack (absent = false on old peers).
    #[serde(default)]
    pub push: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpsMsg {
    pub ops: Vec<WireOp>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpsAck {
    pub accepted: u32,
    pub skipped: u32,
}

#[derive(Debug)]
pub struct ServeSummary {
    pub peer: String,
    pub sent: usize,
    pub accepted: u32,
    pub skipped: u32,
}

#[derive(Debug)]
pub struct PullSummary {
    pub accepted: u32,
    pub skipped: u32,
    pub sent: usize,
    pub remote_accepted: u32,
    pub remote_skipped: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("{0}")]
    Protocol(String),
}

fn decode_ids<'a>(ids: impl IntoIterator<Item = &'a String>) -> BTreeSet<[u8; 32]> {
    ids.into_iter()
        .filter_map(|h| hex::decode(h).ok()?.try_into().ok())
        .collect()
}

/// One server-side exchange against an already-read Hello. Returns the
/// summary plus the peer's op set after the exchange (its announced ids
/// plus everything we sent it).
fn serve_exchange<S: Read + Write, B: StoreBackend>(
    store: &mut LocalStore<B>,
    stream: &mut S,
    hello: Hello,
    push: bool,
) -> Result<(ServeSummary, BTreeSet<[u8; 32]>), SyncError> {
    let local: BTreeSet<[u8; 32]> = store.list_op_ids()?.into_iter().collect();
    let remote = decode_ids(&hello.op_ids);
    let missing: Vec<[u8; 32]> = local.difference(&remote).copied().collect();
    let need: Vec<String> = remote.difference(&local).map(hex::encode).collect();
    write_msg(
        stream,
        &HelloOk {
            v: SYNC_PROTOCOL_VERSION,
            datastore_id: store.datastore_id_hex(),
            peer: store.author_hex(),
            need,
            push,
        },
    )?;
    let ops = store.export_ops_by_id(&missing)?;
    let sent = ops.len();
    write_msg(stream, &OpsMsg { ops })?;

    let offered: OpsMsg = read_msg(stream)?;
    let (accepted, skipped) = if offered.ops.is_empty() {
        (0, 0)
    } else {
        store.import_bundle(&ExportBundle {
            format: 1,
            datastore_id: hello.datastore_id,
            ops: offered.ops,
        })?
    };
    write_msg(stream, &OpsAck { accepted, skipped })?;
    let mut known = remote;
    known.extend(missing);
    Ok((
        ServeSummary {
            peer: hello.peer,
            sent,
            accepted,
            skipped,
        },
        known,
    ))
}

/// Server side of one session: answer a connected peer, send what it lacks,
/// then ingest the ops it offers back.
pub fn serve<S: Read + Write, B: StoreBackend>(
    store: &mut LocalStore<B>,
    stream: &mut S,
) -> Result<ServeSummary, SyncError> {
    let hello: Hello = read_msg(stream)?;
    if hello.v != SYNC_PROTOCOL_VERSION {
        return Err(SyncError::Protocol(format!(
            "bad hello version {}",
            hello.v
        )));
    }
    Ok(serve_exchange(store, stream, hello, false)?.0)
}

/// One client-side exchange. Returns the summary, the server's op set after
/// the exchange (our announced ids plus everything it sent us — the server
/// holds at least our announced set minus `need`, and `need` is answered
/// within this exchange), and whether the server acked push.
fn pull_exchange<S: Read + Write, B: StoreBackend>(
    store: &mut LocalStore<B>,
    stream: &mut S,
    push: bool,
) -> Result<(PullSummary, BTreeSet<[u8; 32]>, bool), SyncError> {
    let local_ids = store
        .list_op_ids()?
        .into_iter()
        .map(hex::encode)
        .collect::<Vec<_>>();
    write_msg(
        stream,
        &Hello {
            v: SYNC_PROTOCOL_VERSION,
            datastore_id: store.datastore_id_hex(),
            peer: store.author_hex(),
            op_ids: local_ids.clone(),
            push,
        },
    )?;
    let hello_ok: HelloOk = read_msg(stream)?;
    if hello_ok.v != SYNC_PROTOCOL_VERSION {
        return Err(SyncError::Protocol(format!(
            "bad hello version {}",
            hello_ok.v
        )));
    }
    let ops_msg: OpsMsg = read_msg(stream)?;
    let received_ids = decode_ids(ops_msg.ops.iter().map(|o| &o.id));
    let (accepted, skipped) = if ops_msg.ops.is_empty() {
        (0, 0)
    } else {
        store.import_bundle(&ExportBundle {
            format: 1,
            datastore_id: hello_ok.datastore_id,
            ops: ops_msg.ops,
        })?
    };
    let need_ids: Vec<[u8; 32]> = hello_ok
        .need
        .iter()
        .filter_map(|h| hex::decode(h).ok()?.try_into().ok())
        .collect();
    let ops = store.export_ops_by_id(&need_ids)?;
    let sent = ops.len();
    write_msg(stream, &OpsMsg { ops })?;
    let ack: OpsAck = read_msg(stream)?;
    let mut known = decode_ids(&local_ids);
    known.extend(received_ids);
    Ok((
        PullSummary {
            accepted,
            skipped,
            sent,
            remote_accepted: ack.accepted,
            remote_skipped: ack.skipped,
        },
        known,
        hello_ok.push,
    ))
}

/// Client side of one session: pull the server's ops, then push back the ops
/// it asked for in `need`.
pub fn pull<S: Read + Write, B: StoreBackend>(
    store: &mut LocalStore<B>,
    stream: &mut S,
) -> Result<PullSummary, SyncError> {
    Ok(pull_exchange(store, stream, false)?.0)
}

/// One send/ingest event inside a persistent push session.
#[derive(Debug)]
pub struct PushTick {
    pub sent: usize,
    pub accepted: u32,
    pub skipped: u32,
}

type SharedStore<B> = Mutex<Option<LocalStore<B>>>;

fn lock_err<T>(e: std::sync::PoisonError<T>) -> SyncError {
    SyncError::Protocol(format!("store lock poisoned: {e}"))
}

/// Server side of a session with the v2 push capability. Reads Hello and runs
/// the normal exchange (locking the store only for that exchange); if the
/// client requested push and `allow_push` is set, the session stays open:
/// whenever `wake()` changes (a local-mutation generation counter), ops the
/// peer lacks are pushed as OpsMsg; incoming OpsMsg frames are ingested and
/// acked. `on_push_start` runs before the persistent loop (use it to shorten
/// the transport read timeout — the loop treats WouldBlock/TimedOut reads as
/// "no frame yet"). Returns the initial summary and whether push ran.
#[allow(clippy::too_many_arguments)]
pub fn serve_push<S: Read + Write, B: StoreBackend>(
    store: &SharedStore<B>,
    stream: &mut S,
    allow_push: bool,
    wake: &dyn Fn() -> u64,
    stop: &dyn Fn() -> bool,
    on_summary: impl FnOnce(&ServeSummary, bool),
    on_push_start: impl FnOnce(&mut S),
    on_tick: impl FnMut(&PushTick),
) -> Result<(ServeSummary, bool), SyncError> {
    let hello: Hello = read_msg(stream)?;
    if hello.v != SYNC_PROTOCOL_VERSION {
        return Err(SyncError::Protocol(format!(
            "bad hello version {}",
            hello.v
        )));
    }
    let push = hello.push && allow_push;
    let (summary, known) = {
        let mut guard = store.lock().map_err(lock_err)?;
        let s = guard
            .as_mut()
            .ok_or_else(|| SyncError::Protocol("store closed".into()))?;
        let (summary, known) = serve_exchange(s, stream, hello, push)?;
        if summary.accepted > 0 {
            s.replay_all()?;
        }
        (summary, known)
    };
    on_summary(&summary, push);
    if push {
        on_push_start(stream);
        push_loop(store, stream, known, wake, stop, on_tick)?;
    }
    Ok((summary, push))
}

/// Client side with the v2 push capability: one exchange, then — if the
/// server acked push — the same symmetric persistent loop as [`serve_push`].
/// `on_summary` fires right after the exchange (a push session may then stay
/// open for a long time). Returns the initial summary and whether the server
/// acked push (false ⇒ caller should fall back to polling).
pub fn pull_push<S: Read + Write, B: StoreBackend>(
    store: &SharedStore<B>,
    stream: &mut S,
    wake: &dyn Fn() -> u64,
    stop: &dyn Fn() -> bool,
    on_summary: impl FnOnce(&PullSummary, bool),
    on_push_start: impl FnOnce(&mut S),
    on_tick: impl FnMut(&PushTick),
) -> Result<(PullSummary, bool), SyncError> {
    let (summary, known, push) = {
        let mut guard = store.lock().map_err(lock_err)?;
        let s = guard
            .as_mut()
            .ok_or_else(|| SyncError::Protocol("store closed".into()))?;
        let (summary, known, push) = pull_exchange(s, stream, true)?;
        if summary.accepted > 0 {
            s.replay_all()?;
        }
        (summary, known, push)
    };
    on_summary(&summary, push);
    if push {
        on_push_start(stream);
        push_loop(store, stream, known, wake, stop, on_tick)?;
    }
    Ok((summary, push))
}

enum Frame {
    Msg(serde_json::Value),
    Closed,
    Idle,
}

fn would_block(kind: io::ErrorKind) -> bool {
    matches!(kind, io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

/// Retry-read after the first byte of a frame arrived: the rest is in flight,
/// so WouldBlock here just means "not yet" (bounded by a hard deadline).
fn read_exact_retry<S: Read>(stream: &mut S, buf: &mut [u8]) -> Result<(), SyncError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut at = 0;
    while at < buf.len() {
        match stream.read(&mut buf[at..]) {
            Ok(0) => return Err(SyncError::Protocol("connection closed mid-frame".into())),
            Ok(n) => at += n,
            Err(e) if would_block(e.kind()) || e.kind() == io::ErrorKind::Interrupted => {
                if Instant::now() > deadline {
                    return Err(SyncError::Protocol("frame read timeout".into()));
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Non-blocking-ish frame read: WouldBlock/TimedOut before any byte ⇒ Idle;
/// EOF ⇒ Closed; otherwise a complete frame (retrying mid-frame).
fn try_read_frame<S: Read>(stream: &mut S) -> Result<Frame, SyncError> {
    let mut first = [0u8; 1];
    match stream.read(&mut first) {
        Ok(0) => return Ok(Frame::Closed),
        Ok(_) => {}
        Err(e) if would_block(e.kind()) || e.kind() == io::ErrorKind::Interrupted => {
            return Ok(Frame::Idle);
        }
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(Frame::Closed),
        Err(e) => return Err(e.into()),
    }
    let mut rest = [0u8; 3];
    read_exact_retry(stream, &mut rest)?;
    let len = u32::from_be_bytes([first[0], rest[0], rest[1], rest[2]]) as usize;
    if len > MAX_MSG_BYTES {
        return Err(SyncError::Protocol("message too large".into()));
    }
    let mut buf = vec![0u8; len];
    read_exact_retry(stream, &mut buf)?;
    Ok(Frame::Msg(serde_json::from_slice(&buf)?))
}

/// Symmetric persistent phase. The store lock is held only while diffing/
/// exporting and while ingesting one OpsMsg — never across reads or sleeps —
/// so a persistent session cannot starve other sessions or local writes.
/// `known` tracks the peer's op set and is updated as ops flow either way;
/// pushes are diff-based (local ids − known), so a missed or spurious wake is
/// self-correcting. Returns Ok on peer close, store close, or `stop()`.
fn push_loop<S: Read + Write, B: StoreBackend>(
    store: &SharedStore<B>,
    stream: &mut S,
    mut known: BTreeSet<[u8; 32]>,
    wake: &dyn Fn() -> u64,
    stop: &dyn Fn() -> bool,
    mut on_tick: impl FnMut(&PushTick),
) -> Result<(), SyncError> {
    let mut last_gen: Option<u64> = None;
    loop {
        if stop() {
            return Ok(());
        }
        let g = wake();
        if last_gen != Some(g) {
            last_gen = Some(g);
            let ops = {
                let mut guard = store.lock().map_err(lock_err)?;
                let Some(s) = guard.as_mut() else {
                    return Ok(());
                };
                let local: BTreeSet<[u8; 32]> = s.list_op_ids()?.into_iter().collect();
                let missing: Vec<[u8; 32]> = local.difference(&known).copied().collect();
                s.export_ops_by_id(&missing)?
            };
            if !ops.is_empty() {
                known.extend(decode_ids(ops.iter().map(|o| &o.id)));
                let sent = ops.len();
                write_msg(stream, &OpsMsg { ops })?;
                on_tick(&PushTick {
                    sent,
                    accepted: 0,
                    skipped: 0,
                });
            }
        }
        match try_read_frame(stream)? {
            Frame::Msg(v) if v.get("ops").is_some() => {
                let msg: OpsMsg = serde_json::from_value(v)?;
                known.extend(decode_ids(msg.ops.iter().map(|o| &o.id)));
                let (accepted, skipped) = {
                    let mut guard = store.lock().map_err(lock_err)?;
                    let Some(s) = guard.as_mut() else {
                        return Ok(());
                    };
                    if msg.ops.is_empty() {
                        (0, 0)
                    } else {
                        let ds = s.datastore_id_hex();
                        let (accepted, skipped) = s.import_bundle(&ExportBundle {
                            format: 1,
                            datastore_id: ds,
                            ops: msg.ops,
                        })?;
                        if accepted > 0 {
                            s.replay_all()?;
                        }
                        (accepted, skipped)
                    }
                };
                write_msg(stream, &OpsAck { accepted, skipped })?;
                on_tick(&PushTick {
                    sent: 0,
                    accepted,
                    skipped,
                });
            }
            Frame::Msg(_) => {} // OpsAck (informational) or unknown — ignore
            Frame::Closed => return Ok(()),
            Frame::Idle => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

pub fn write_msg<S: Write, T: Serialize>(stream: &mut S, msg: &T) -> Result<(), SyncError> {
    let bytes = serde_json::to_vec(msg)?;
    let len = (bytes.len() as u32).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

pub fn read_msg<S: Read, T: for<'de> Deserialize<'de>>(stream: &mut S) -> Result<T, SyncError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_MSG_BYTES {
        return Err(SyncError::Protocol("message too large".into()));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
}
