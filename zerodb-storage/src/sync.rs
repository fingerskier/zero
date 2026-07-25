//! Peer sync sessions over any `Read + Write` stream (TCP today, WS later).
//!
//! Protocol v2 (length-prefixed JSON frames, u32 BE lengths):
//! client -> Hello { v, datastore_id, peer, op_ids }
//! server -> HelloOk { v, datastore_id, peer, need }
//! server -> OpsMsg { ops }   (ops the client lacks)
//! client -> OpsMsg { ops }   (ops from `need` the client has)
//! server -> OpsAck { accepted, skipped }
//! Both sides ingest via the prevalidated `import_bundle` path.

use std::collections::BTreeSet;
use std::io::{Read, Write};

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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HelloOk {
    pub v: u32,
    pub datastore_id: String,
    pub peer: String,
    pub need: Vec<String>,
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
    let local: BTreeSet<[u8; 32]> = store.list_op_ids()?.into_iter().collect();
    let remote: BTreeSet<[u8; 32]> = hello
        .op_ids
        .iter()
        .filter_map(|h| hex::decode(h).ok()?.try_into().ok())
        .collect();
    let missing: Vec<[u8; 32]> = local.difference(&remote).copied().collect();
    let need: Vec<String> = remote.difference(&local).map(hex::encode).collect();
    write_msg(
        stream,
        &HelloOk {
            v: SYNC_PROTOCOL_VERSION,
            datastore_id: store.datastore_id_hex(),
            peer: store.author_hex(),
            need,
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
    Ok(ServeSummary {
        peer: hello.peer,
        sent,
        accepted,
        skipped,
    })
}

/// Client side of one session: pull the server's ops, then push back the ops
/// it asked for in `need`.
pub fn pull<S: Read + Write, B: StoreBackend>(
    store: &mut LocalStore<B>,
    stream: &mut S,
) -> Result<PullSummary, SyncError> {
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
            op_ids: local_ids,
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
    Ok(PullSummary {
        accepted,
        skipped,
        sent,
        remote_accepted: ack.accepted,
        remote_skipped: ack.skipped,
    })
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
