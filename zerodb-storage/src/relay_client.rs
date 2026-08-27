//! RELAY 0.2.2 client: handshake, submit signed LocalStore ops, apply catch-up.
//!
//! Transport-agnostic: the caller supplies a `handle` that takes one peer
//! envelope and returns the relay's reply frames (in-process `RelaySession`
//! or one WebSocket binary per envelope). Not a format freeze. Implements the
//! negotiated frozen-snapshot Merkle walk. M3b-sig: the relay admits
//! signed ops only (signature, OpId preimage, datastore bind). AUTH
//! membership is enforced by peers on apply.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::SigningKey;
use zerodb_core::cbor::{self, Cbor};
use zerodb_core::merkle::{
    BUCKET_WIDTH_MS, MERKLE_FORMAT_VERSION, MerkleOp, MerkleTree, merkle_root,
};
use zerodb_core::relay::{
    MSG_AUTH, MSG_CHALLENGE, MSG_DELTA_BATCH, MSG_DELTA_REQUEST, MSG_ERROR, MSG_HELLO,
    MSG_MERKLE_LEAF_REQUEST, MSG_MERKLE_LEAF_RESPONSE, MSG_MERKLE_NODE_REQUEST,
    MSG_MERKLE_NODE_RESPONSE, MSG_OP_ACK, MSG_OPS, MSG_SYNC_REQUEST, MSG_SYNC_RESPONSE,
    MSG_WELCOME, RELAY_CAPS, peer_id_from_pk, sign_auth,
};

use crate::authz::bundle_datastore_id;
use crate::{ExportBundle, IngestResult, LocalStore, StoreBackend, StoreError, WireOp};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelaySyncSummary {
    pub sent: u32,
    pub ack_accepted: u32,
    pub ack_duplicate: u32,
    pub ack_rejected: u32,
    pub received: u32,
    pub applied: u32,
    pub skipped: u32,
    pub merkle_nodes: u32,
    pub merkle_leaves: u32,
}

pub fn sync<B, H, E>(
    store: &mut LocalStore<B>,
    join_ds: Option<&str>,
    mut handle: H,
) -> Result<RelaySyncSummary, StoreError>
where
    B: StoreBackend,
    H: FnMut(&[u8]) -> Result<Vec<Vec<u8>>, E>,
    E: std::fmt::Display,
{
    let mut summary = RelaySyncSummary::default();
    let seed = store.identity_seed();
    let pk = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    let peer = peer_id_from_pk(&pk);

    let hello = encode_env(
        MSG_HELLO,
        1,
        Cbor::Map(vec![
            ("peer_id".into(), Cbor::Bytes(peer.to_vec())),
            ("public_key".into(), Cbor::Bytes(pk.to_vec())),
            ("protocol_version".into(), Cbor::Uint(1)),
            (
                "capabilities".into(),
                Cbor::Array(RELAY_CAPS.iter().map(|c| Cbor::Text((*c).into())).collect()),
            ),
        ]),
    );
    let challenge = expect_type(
        first_reply(handle(&hello).map_err(map_h)?)?,
        MSG_CHALLENGE,
        "CHALLENGE",
    )?;
    let (_, _, challenge_pl) = decode_env(&challenge)?;
    let nonce = take32(map_get(&challenge_pl, "nonce"))?;
    let auth = encode_env(
        MSG_AUTH,
        2,
        Cbor::Map(vec![(
            "signature".into(),
            Cbor::Bytes(sign_auth(&seed, &nonce).to_vec()),
        )]),
    );
    let welcome = expect_type(
        first_reply(handle(&auth).map_err(map_h)?)?,
        MSG_WELCOME,
        "WELCOME",
    )?;
    let (_, _, welcome_pl) = decode_env(&welcome)?;
    let (max_batch_ops, max_batch_bytes) = welcome_limits(&welcome_pl);

    let ds = join_ds
        .map(str::to_string)
        .unwrap_or_else(|| store.datastore_id_hex());

    let local = store.export_all()?;
    let zero_ds = "00".repeat(32);
    let to_send: Vec<WireOp> = local
        .ops
        .into_iter()
        .filter(|w| w.ds == ds || (w.kind == 0 && w.ds == zero_ds))
        .collect();
    let mut request_id = 3u32;
    if !to_send.is_empty() {
        summary.sent = to_send.len() as u32;
        let ops_cbor = to_send
            .iter()
            .map(wire_to_relay)
            .collect::<Result<Vec<_>, _>>()?;
        for batch in split_ops_batches(&ds, &ops_cbor, max_batch_ops, max_batch_bytes)? {
            let frame = encode_env(MSG_OPS, request_id, ops_payload(&ds, &batch));
            request_id = request_id.saturating_add(1);
            let ack = expect_type(
                first_reply(handle(&frame).map_err(map_h)?)?,
                MSG_OP_ACK,
                "OP_ACK",
            )?;
            let (_, _, ack_pl) = decode_env(&ack)?;
            tally_ack(&mut summary, &ack_pl);
        }
    }

    let sync_req = encode_env(
        MSG_SYNC_REQUEST,
        request_id,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(ds.clone())),
            (
                "accepted_root".into(),
                Cbor::Bytes(accepted_root(&to_send)?),
            ),
            ("cursor".into(), local_frontier(store, &ds)?),
        ]),
    );
    let replies = handle(&sync_req).map_err(map_h)?;
    let mut incoming: Vec<WireOp> = Vec::new();
    let mut catchup_skipped = 0u32;
    let mut sync_payload = None;
    for frame in &replies {
        let (ty, _, pl) = decode_env(frame)?;
        match ty {
            MSG_SYNC_RESPONSE => sync_payload = Some(pl),
            MSG_OPS => collect_wire_ops(&pl, &mut incoming, &mut catchup_skipped)?,
            MSG_ERROR => return Err(err(&format!("SYNC error: {pl:?}"))),
            _ => {}
        }
    }
    let sync_payload = sync_payload.ok_or_else(|| err("expected SYNC_RESPONSE"))?;
    if let Some(bucket_indices) = parse_bucket_indices(&sync_payload)? {
        let remote_root = take32(map_get(&sync_payload, "validated_root"))?;
        let local_merkle = wire_merkle_ops(&to_send)?;
        let local_tree = MerkleTree::build_aligned(&local_merkle, &bucket_indices);
        if local_tree.root() != remote_root {
            let mut missing = BTreeSet::new();
            let root_level = local_tree.levels.len().saturating_sub(1);
            walk_remote(
                &mut handle,
                &ds,
                &local_tree,
                root_level,
                0,
                &mut request_id,
                &mut missing,
                &mut summary,
            )?;
            if !missing.is_empty() {
                let delta = encode_env(
                    MSG_DELTA_REQUEST,
                    request_id,
                    Cbor::Map(vec![
                        ("datastore".into(), Cbor::Text(ds.clone())),
                        (
                            "op_ids".into(),
                            Cbor::Array(
                                missing.iter().map(|id| Cbor::Bytes(id.to_vec())).collect(),
                            ),
                        ),
                    ]),
                );
                for frame in handle(&delta).map_err(map_h)? {
                    let (ty, _, pl) = decode_env(&frame)?;
                    match ty {
                        MSG_DELTA_BATCH => {
                            collect_wire_ops(&pl, &mut incoming, &mut catchup_skipped)?
                        }
                        MSG_ERROR => return Err(err(&format!("DELTA error: {pl:?}"))),
                        _ => return Err(err("expected DELTA_BATCH")),
                    }
                }
            }
        }
    }
    summary.received = incoming.len() as u32 + catchup_skipped;
    if incoming.is_empty() {
        summary.skipped = catchup_skipped;
        if let Some(join) = join_ds
            && store.op_count()? == 0
        {
            store.adopt_empty_datastore(join)?;
        }
        return Ok(summary);
    }
    let bundle = ExportBundle {
        format: 1,
        datastore_id: bundle_datastore_id(&incoming)?,
        ops: incoming,
    };
    let adopting = store.op_count()? == 0 && bundle.datastore_id != store.datastore_id_hex();
    let (applied, skipped) = if adopting {
        store.import_bundle(&bundle)?
    } else {
        let mut applied = 0u32;
        let mut skipped = 0u32;
        for w in &bundle.ops {
            match store.ingest_op(w)? {
                IngestResult::Applied => applied += 1,
                IngestResult::Duplicate
                | IngestResult::Rejected { .. }
                | IngestResult::Quarantined { .. } => skipped += 1,
            }
        }
        (applied, skipped)
    };
    summary.applied = applied;
    summary.skipped = catchup_skipped.saturating_add(skipped);
    Ok(summary)
}

/// Decode catch-up ops. Missing or malformed `wire` is AUTH.md §4
/// `AUTH_SIG_INVALID` (KERNEL §4.4 cannot be checked). A colluding or
/// buggy relay may persist header-only junk; one entry must not abort
/// the batch.
fn collect_wire_ops(
    payload: &Cbor,
    incoming: &mut Vec<WireOp>,
    skipped: &mut u32,
) -> Result<(), StoreError> {
    if let Cbor::Array(ops) = map_get(payload, "operations") {
        for op in ops {
            match catchup_wire_op(op) {
                Some(wire) => incoming.push(wire),
                None => *skipped += 1,
            }
        }
    }
    Ok(())
}

fn parse_bucket_indices(payload: &Cbor) -> Result<Option<Vec<u64>>, StoreError> {
    let version = map_get(payload, "merkle_format_version");
    if matches!(version, Cbor::Null) {
        return Ok(None);
    }
    if uint(version)? != MERKLE_FORMAT_VERSION as u64 {
        return Err(err("unsupported merkle_format_version"));
    }
    if uint(map_get(payload, "bucket_width_ms"))? != BUCKET_WIDTH_MS {
        return Err(err("unsupported merkle bucket_width_ms"));
    }
    match map_get(payload, "bucket_indices") {
        Cbor::Array(items) => {
            const MAX_WALK_BUCKETS: usize = 65_536;
            if items.len() > MAX_WALK_BUCKETS {
                return Err(err("merkle bucket manifest exceeds limit"));
            }
            let indices = items.iter().map(uint).collect::<Result<Vec<_>, _>>()?;
            if indices.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(err("merkle bucket manifest must be strictly sorted"));
            }
            Ok(Some(indices))
        }
        _ => Err(err("missing bucket_indices")),
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_remote<H, E>(
    handle: &mut H,
    ds: &str,
    local: &MerkleTree,
    level: usize,
    index: usize,
    request_id: &mut u32,
    missing: &mut BTreeSet<[u8; 32]>,
    summary: &mut RelaySyncSummary,
) -> Result<(), StoreError>
where
    H: FnMut(&[u8]) -> Result<Vec<Vec<u8>>, E>,
    E: std::fmt::Display,
{
    if level == 0 {
        let frame = encode_env(
            MSG_MERKLE_LEAF_REQUEST,
            *request_id,
            Cbor::Map(vec![
                ("datastore".into(), Cbor::Text(ds.into())),
                ("leaf_index".into(), Cbor::Uint(index as u64)),
            ]),
        );
        *request_id = request_id.saturating_add(1);
        let response = expect_type(
            first_reply(handle(&frame).map_err(map_h)?)?,
            MSG_MERKLE_LEAF_RESPONSE,
            "MERKLE_LEAF_RESPONSE",
        )?;
        let (_, _, payload) = decode_env(&response)?;
        let local_ids: BTreeSet<[u8; 32]> = local
            .leaves
            .get(index)
            .map(|leaf| leaf.op_ids.iter().copied().collect())
            .unwrap_or_default();
        let Cbor::Array(ids) = map_get(&payload, "op_ids") else {
            return Err(err("leaf response missing op_ids"));
        };
        for id in ids {
            let id = take32(id)?;
            if !local_ids.contains(&id) {
                missing.insert(id);
            }
        }
        summary.merkle_leaves += 1;
        return Ok(());
    }

    let frame = encode_env(
        MSG_MERKLE_NODE_REQUEST,
        *request_id,
        Cbor::Map(vec![
            ("datastore".into(), Cbor::Text(ds.into())),
            ("level".into(), Cbor::Uint(level as u64)),
            ("index".into(), Cbor::Uint(index as u64)),
        ]),
    );
    *request_id = request_id.saturating_add(1);
    let response = expect_type(
        first_reply(handle(&frame).map_err(map_h)?)?,
        MSG_MERKLE_NODE_RESPONSE,
        "MERKLE_NODE_RESPONSE",
    )?;
    let (_, _, payload) = decode_env(&response)?;
    let remote_left = take32(map_get(&payload, "left"))?;
    let remote_right = take32(map_get(&payload, "right"))?;
    let (local_left, local_right) = local
        .node_children(level, index)
        .ok_or_else(|| err("local merkle node missing children"))?;
    summary.merkle_nodes += 1;
    if remote_left != local_left {
        walk_remote(
            handle,
            ds,
            local,
            level - 1,
            index * 2,
            request_id,
            missing,
            summary,
        )?;
    }
    if remote_right != local_right {
        walk_remote(
            handle,
            ds,
            local,
            level - 1,
            index * 2 + 1,
            request_id,
            missing,
            summary,
        )?;
    }
    Ok(())
}

const DEFAULT_MAX_BATCH_OPS: usize = 64;
const DEFAULT_MAX_BATCH_BYTES: usize = 16_777_216;

fn welcome_limits(welcome: &Cbor) -> (usize, usize) {
    let limits = map_get(welcome, "limits");
    let ops = match map_get(limits, "max_batch_ops") {
        Cbor::Uint(n) if *n > 0 => *n as usize,
        _ => DEFAULT_MAX_BATCH_OPS,
    };
    let bytes = match map_get(limits, "max_batch_bytes") {
        Cbor::Uint(n) if *n > 0 => *n as usize,
        _ => DEFAULT_MAX_BATCH_BYTES,
    };
    (ops, bytes)
}

fn ops_payload(ds: &str, ops: &[Cbor]) -> Cbor {
    Cbor::Map(vec![
        ("datastore".into(), Cbor::Text(ds.into())),
        ("operations".into(), Cbor::Array(ops.to_vec())),
    ])
}

fn split_ops_batches(
    ds: &str,
    ops: &[Cbor],
    max_ops: usize,
    max_bytes: usize,
) -> Result<Vec<Vec<Cbor>>, StoreError> {
    let max_ops = max_ops.max(1);
    let mut out = Vec::new();
    let mut cur: Vec<Cbor> = Vec::new();
    for op in ops {
        cur.push(op.clone());
        let over_ops = cur.len() > max_ops;
        let over_bytes = encode_env(MSG_OPS, 0, ops_payload(ds, &cur)).len() > max_bytes;
        if over_ops || over_bytes {
            if cur.len() == 1 {
                return Err(err("single op exceeds WELCOME batch limits"));
            }
            let last = cur.pop().expect("batch had more than one op");
            out.push(std::mem::take(&mut cur));
            cur.push(last);
            if encode_env(MSG_OPS, 0, ops_payload(ds, &cur)).len() > max_bytes {
                return Err(err("single op exceeds max_batch_bytes"));
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

fn tally_ack(summary: &mut RelaySyncSummary, ack_pl: &Cbor) {
    if let Cbor::Array(outcomes) = map_get(ack_pl, "outcomes") {
        for o in outcomes {
            match text(map_get(o, "outcome")) {
                Some("ACCEPT") => summary.ack_accepted += 1,
                Some("DUPLICATE") => summary.ack_duplicate += 1,
                Some("REJECT") => summary.ack_rejected += 1,
                _ => {}
            }
        }
    }
}

fn map_h<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Invalid(e.to_string())
}

fn err(msg: &str) -> StoreError {
    StoreError::Invalid(msg.into())
}

fn first_reply(replies: Vec<Vec<u8>>) -> Result<Vec<u8>, StoreError> {
    replies
        .into_iter()
        .next()
        .ok_or_else(|| err("empty relay reply"))
}

fn expect_type(frame: Vec<u8>, want: u8, name: &str) -> Result<Vec<u8>, StoreError> {
    let (ty, _, pl) = decode_env(&frame)?;
    if ty == MSG_ERROR {
        return Err(err(&format!("{name} got ERROR: {pl:?}")));
    }
    if ty != want {
        return Err(err(&format!("expected {name}, got type {ty}")));
    }
    Ok(frame)
}

fn encode_env(ty: u8, request_id: u32, payload: Cbor) -> Vec<u8> {
    cbor::encode(&Cbor::Map(vec![
        ("type".into(), Cbor::Uint(ty as u64)),
        ("request_id".into(), Cbor::Uint(request_id as u64)),
        ("payload".into(), payload),
    ]))
    .expect("encode envelope")
}

fn decode_env(bytes: &[u8]) -> Result<(u8, u32, Cbor), StoreError> {
    let c = cbor::decode(bytes).map_err(|e| err(&e.to_string()))?;
    let ty = uint(map_get(&c, "type"))? as u8;
    let request_id = uint(map_get(&c, "request_id"))? as u32;
    Ok((ty, request_id, map_get(&c, "payload").clone()))
}

fn map_get<'a>(c: &'a Cbor, k: &str) -> &'a Cbor {
    static NULL: Cbor = Cbor::Null;
    match c {
        Cbor::Map(ents) => ents
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v)
            .unwrap_or(&NULL),
        _ => &NULL,
    }
}

fn uint(c: &Cbor) -> Result<u64, StoreError> {
    match c {
        Cbor::Uint(n) => Ok(*n),
        _ => Err(err("uint")),
    }
}

fn take32(c: &Cbor) -> Result<[u8; 32], StoreError> {
    match c {
        Cbor::Bytes(b) if b.len() == 32 => Ok(b.as_slice().try_into().unwrap()),
        _ => Err(err("b32")),
    }
}

fn text(c: &Cbor) -> Option<&str> {
    match c {
        Cbor::Text(s) => Some(s.as_str()),
        _ => None,
    }
}

fn hex32(s: &str) -> Result<Vec<u8>, StoreError> {
    let b = hex::decode(s).map_err(|e| err(&e.to_string()))?;
    if b.len() != 32 {
        return Err(err("expected 32-byte hex"));
    }
    Ok(b)
}

fn wire_to_relay(w: &WireOp) -> Result<Cbor, StoreError> {
    let json = serde_json::to_string(w).map_err(|e| err(&e.to_string()))?;
    Ok(Cbor::Map(vec![
        ("op_id".into(), Cbor::Bytes(hex32(&w.id)?)),
        ("author".into(), Cbor::Bytes(hex32(&w.author)?)),
        ("physical_ms".into(), Cbor::Uint(w.ts.p)),
        ("logical".into(), Cbor::Uint(w.ts.l as u64)),
        ("wire".into(), Cbor::Text(json)),
    ]))
}

fn catchup_wire_op(op: &Cbor) -> Option<WireOp> {
    match map_get(op, "wire") {
        Cbor::Text(s) => serde_json::from_str(s).ok(),
        _ => None,
    }
}

fn wire_merkle_ops(ops: &[WireOp]) -> Result<Vec<MerkleOp>, StoreError> {
    ops.iter()
        .map(|w| {
            Ok(MerkleOp {
                op_id: hex32(&w.id)?.try_into().map_err(|_| err("op_id"))?,
                author: hex32(&w.author)?.try_into().map_err(|_| err("author"))?,
                physical_ms: w.ts.p,
                logical: w.ts.l,
            })
        })
        .collect()
}

fn accepted_root(ops: &[WireOp]) -> Result<Vec<u8>, StoreError> {
    Ok(merkle_root(&wire_merkle_ops(ops)?).to_vec())
}

fn local_frontier<B: StoreBackend>(store: &LocalStore<B>, ds: &str) -> Result<Cbor, StoreError> {
    let bundle = store.export_all()?;
    let mut tips: BTreeMap<String, (u64, u16, String)> = BTreeMap::new();
    for w in bundle.ops {
        if w.ds != ds {
            continue;
        }
        let key = w.author.clone();
        let tip = (w.ts.p, w.ts.l, w.id.clone());
        match tips.get(&key) {
            Some(prev) if *prev >= tip => {}
            _ => {
                tips.insert(key, tip);
            }
        }
    }
    let mut ents = Vec::new();
    for (author, (p, l, id)) in tips {
        ents.push((
            author,
            Cbor::Map(vec![
                ("op_id".into(), Cbor::Bytes(hex32(&id)?)),
                ("physical_ms".into(), Cbor::Uint(p)),
                ("logical".into(), Cbor::Uint(l as u64)),
            ]),
        ));
    }
    Ok(Cbor::Map(vec![
        ("frontier".into(), Cbor::Map(ents)),
        ("epoch".into(), Cbor::Uint(0)),
    ]))
}
