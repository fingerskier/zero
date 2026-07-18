//! Epoch-aware replay model per doc/SCHEMA.md §3/§4.1.
//!
//! Materializes one property from its operation set and the canonical epoch
//! chain, reusing the KERNEL §6 CRDT apply rules (`kernel::CrdtState`) so epoch
//! semantics can never drift from the base CRDT semantics. Pipeline: dedup by
//! op id (step 4) + equivocation exclusion (step 5) over the real op set;
//! resolve the `SchemaEpoch` records into one canonical chain (lowest §4.5
//! order key wins a fork, losers + descendants quarantine); split the chain
//! into maximal single-CRDT segments; pool each segment's ops into one replica;
//! carry a `change_crdt` boundary across as one synthetic seed op placed at the
//! `SchemaEpoch` order key. Order-independent by construction (I-1, I-17).

use std::collections::{BTreeMap, BTreeSet};

use crate::kernel::{
    CrdtState, Flag, GCounter, Id, KernelError, KernelOp, Lww, OrSet, OrderKey, Payload, PnCounter,
    Value,
};

/// One `SchemaEpoch` record reduced to the fields the property model needs.
#[derive(Debug, Clone)]
pub struct EpochRecord {
    pub record: String,
    pub epoch: u64,
    pub prev: Option<String>,
    pub order_key: OrderKey,
    pub crdt: String,
    pub migration: Option<Migration>,
}

/// A `change_crdt` migration step (the only step that starts a new segment).
#[derive(Debug, Clone)]
pub struct Migration {
    pub transform: String,
    pub default: Option<Value>,
}

/// A data op plus its epoch binding (`ep` number, optional `ep_record` id).
#[derive(Debug, Clone)]
pub struct EpochOp {
    pub op: KernelOp,
    pub ep: u64,
    pub ep_record: Option<String>,
}

/// Materialized read of the final segment (mirrors the KERNEL §6 read shapes).
#[derive(Debug, Clone, PartialEq)]
pub enum Read {
    Lww(Option<Value>),
    GCounter(u64),
    PnCounter(i128),
    Flag(bool),
    OrSet(Vec<Value>),
}

#[derive(Debug)]
pub enum EpochError {
    /// A surviving op resolves to an epoch absent from the known chain.
    EpochUnknown,
    /// Adjacent same-segment epochs disagree on CRDT type with no `change_crdt`.
    TypeChangeWithoutMigration,
    UnknownCrdt(String),
    UnsupportedTransform(String),
    Kernel(KernelError),
}

impl EpochError {
    pub fn name(&self) -> String {
        match self {
            EpochError::EpochUnknown => "EPOCH_UNKNOWN".into(),
            EpochError::TypeChangeWithoutMigration => "SCHEMA_TYPE_CHANGE_WITHOUT_MIGRATION".into(),
            EpochError::UnknownCrdt(c) => format!("UNKNOWN_CRDT({c})"),
            EpochError::UnsupportedTransform(t) => format!("UNSUPPORTED_TRANSFORM({t})"),
            EpochError::Kernel(e) => format!("{e}"),
        }
    }
}

/// Resolve records into one linear chain. Level 1's winner is the lowest-order-
/// key record with `prev = None`; level n's winner is the lowest-key record
/// whose `prev` is level n-1's winner. Everything else quarantines.
fn resolve_chain(records: &[EpochRecord]) -> (BTreeMap<u64, EpochRecord>, BTreeSet<String>) {
    let mut by_level: BTreeMap<u64, Vec<&EpochRecord>> = BTreeMap::new();
    for r in records {
        by_level.entry(r.epoch).or_default().push(r);
    }
    let mut winners: BTreeMap<u64, EpochRecord> = BTreeMap::new();
    let mut quarantined: BTreeSet<String> = BTreeSet::new();
    let mut prev_winner: Option<String> = None;
    let mut broken = false;
    for (_level, at_level) in by_level {
        if broken {
            for r in at_level {
                quarantined.insert(r.record.clone());
            }
            continue;
        }
        let candidates: Vec<&EpochRecord> = at_level
            .iter()
            .copied()
            .filter(|r| r.prev == prev_winner)
            .collect();
        for r in &at_level {
            if !candidates.iter().any(|c| c.record == r.record) {
                quarantined.insert(r.record.clone());
            }
        }
        let Some(win) = candidates
            .iter()
            .min_by(|a, b| a.order_key.cmp(&b.order_key))
            .copied()
        else {
            broken = true;
            continue;
        };
        for c in &candidates {
            if c.record != win.record {
                quarantined.insert(c.record.clone());
            }
        }
        prev_winner = Some(win.record.clone());
        winners.insert(win.epoch, win.clone());
    }
    (winners, quarantined)
}

/// Op ids excluded by the equivocation rule (author + equal timestamp).
fn equivocated(ops: &[EpochOp]) -> BTreeSet<Id> {
    let mut groups: BTreeMap<(Id, u64, u16), Vec<Id>> = BTreeMap::new();
    for e in ops {
        groups
            .entry((e.op.author.clone(), e.op.physical_ms, e.op.logical))
            .or_default()
            .push(e.op.op_id.clone());
    }
    groups
        .into_values()
        .filter(|ids| ids.len() > 1)
        .flatten()
        .collect()
}

/// Synthetic seed op for a `change_crdt` boundary, placed at the SchemaEpoch
/// order key so post-migration writes outrank it iff genuinely newer.
fn build_seed(mig: &Migration, prev: &Read, key: &OrderKey) -> Result<KernelOp, EpochError> {
    let value = match mig.transform.as_str() {
        "counter_value_to_lww_int" => match prev {
            Read::GCounter(v) => Value::Int(*v as i64),
            Read::PnCounter(v) => Value::Int(*v as i64),
            _ => Value::Null,
        },
        "reset_to" => mig.default.clone().unwrap_or(Value::Null),
        other => return Err(EpochError::UnsupportedTransform(other.into())),
    };
    Ok(KernelOp {
        op_id: key.op_id.clone(),
        author: key.author.clone(),
        physical_ms: key.physical_ms,
        logical: key.logical,
        payload: Payload::LwwSet(value),
    })
}

fn read_segment<S, F>(
    seed: Option<&KernelOp>,
    ops: &[&KernelOp],
    read: F,
) -> Result<Read, EpochError>
where
    S: CrdtState,
    F: Fn(&S) -> Read,
{
    let mut st = S::default();
    if let Some(s) = seed {
        st.apply(s).map_err(EpochError::Kernel)?;
    }
    for o in ops {
        st.apply(o).map_err(EpochError::Kernel)?;
    }
    Ok(read(&st))
}

fn materialize_segment(
    crdt: &str,
    seed: Option<&KernelOp>,
    ops: &[&KernelOp],
) -> Result<Read, EpochError> {
    match crdt {
        "lww" => read_segment::<Lww, _>(seed, ops, |s| Read::Lww(s.value().cloned())),
        "gcounter" => read_segment::<GCounter, _>(seed, ops, |s| Read::GCounter(s.value())),
        "pncounter" => read_segment::<PnCounter, _>(seed, ops, |s| Read::PnCounter(s.value())),
        "flag" => read_segment::<Flag, _>(seed, ops, |s| Read::Flag(s.enabled())),
        "orset" => read_segment::<OrSet, _>(seed, ops, |s| {
            let mut els: Vec<Value> = s.elements().into_iter().cloned().collect();
            els.sort();
            Read::OrSet(els)
        }),
        other => Err(EpochError::UnknownCrdt(other.into())),
    }
}

/// Materialize one property. Returns the final read and the ids of ops
/// quarantined because they are bound to a losing fork record.
pub fn materialize(
    ops: &[EpochOp],
    records: &[EpochRecord],
) -> Result<(Read, Vec<String>), EpochError> {
    // Steps 4-5: dedup by op id, then exclude equivocation groups.
    let mut seen: BTreeSet<Id> = BTreeSet::new();
    let mut deduped: Vec<&EpochOp> = Vec::new();
    for e in ops {
        if seen.insert(e.op.op_id.clone()) {
            deduped.push(e);
        }
    }
    let excluded = equivocated(&deduped.iter().map(|e| (*e).clone()).collect::<Vec<_>>());

    let (winners, quarantined) = resolve_chain(records);
    let rec_by_id: BTreeMap<&str, &EpochRecord> =
        records.iter().map(|r| (r.record.as_str(), r)).collect();

    // Resolve each survivor to a level, or quarantine / EPOCH_UNKNOWN.
    let mut quarantined_ops: Vec<String> = Vec::new();
    let mut active_by_level: BTreeMap<u64, Vec<&KernelOp>> = BTreeMap::new();
    for e in &deduped {
        if excluded.contains(&e.op.op_id) {
            continue;
        }
        let level = if let Some(rid) = &e.ep_record {
            let Some(rec) = rec_by_id.get(rid.as_str()) else {
                return Err(EpochError::EpochUnknown);
            };
            if quarantined.contains(&rec.record) {
                quarantined_ops.push(id_string(&e.op.op_id));
                continue;
            }
            rec.epoch
        } else if e.ep == 0 {
            0
        } else {
            if !winners.contains_key(&e.ep) {
                return Err(EpochError::EpochUnknown);
            }
            e.ep
        };
        active_by_level.entry(level).or_default().push(&e.op);
    }

    // Level walk → single-CRDT segments (a change_crdt migration starts a new
    // segment; add_prop / entity steps do not).
    let max_level = winners.keys().copied().max().unwrap_or(0);
    let has_ep0 = active_by_level.contains_key(&0);
    let start_level = if has_ep0 {
        0
    } else {
        winners.keys().copied().min().unwrap_or(0)
    };

    struct Segment<'a> {
        crdt: String,
        migration: Option<Migration>,
        seed_key: Option<OrderKey>,
        ops: Vec<&'a KernelOp>,
    }
    let mut segments: Vec<Segment> = Vec::new();
    for level in start_level..=max_level {
        if level != 0 && !winners.contains_key(&level) {
            continue;
        }
        let (crdt, migration, seed_key) = if level == 0 {
            ("lww".to_string(), None, None)
        } else {
            let w = &winners[&level];
            (
                w.crdt.clone(),
                w.migration.clone(),
                Some(w.order_key.clone()),
            )
        };
        let level_ops: Vec<&KernelOp> = active_by_level.get(&level).cloned().unwrap_or_default();
        if segments.is_empty() || migration.is_some() {
            segments.push(Segment {
                crdt,
                migration,
                seed_key,
                ops: level_ops,
            });
        } else {
            let seg = segments.last_mut().unwrap();
            if seg.crdt != crdt {
                return Err(EpochError::TypeChangeWithoutMigration);
            }
            seg.ops.extend(level_ops);
        }
    }

    // Materialize each segment; carry a change_crdt boundary as a seed op.
    let mut prev = Read::Lww(None);
    for (si, seg) in segments.iter().enumerate() {
        let seed = if si == 0 {
            None
        } else {
            let mig = seg
                .migration
                .as_ref()
                .expect("non-first segment implies a change_crdt migration");
            let key = seg
                .seed_key
                .as_ref()
                .expect("epoch>0 records carry an order key");
            Some(build_seed(mig, &prev, key)?)
        };
        // Deterministic set order (arrival order is irrelevant by convergence).
        let mut ordered: Vec<&KernelOp> = seg.ops.clone();
        ordered.sort_by(|a, b| a.op_id.cmp(&b.op_id));
        prev = materialize_segment(&seg.crdt, seed.as_ref(), &ordered)?;
    }

    Ok((prev, quarantined_ops))
}

fn id_string(id: &Id) -> String {
    String::from_utf8_lossy(id).into_owned()
}
