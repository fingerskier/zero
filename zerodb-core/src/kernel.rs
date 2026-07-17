//! Executable CRDT semantic kernel per doc/KERNEL.md §6.
//!
//! Implements pipeline steps 4 (dedup by op id) and 7 (per-type apply),
//! plus the §4.5 total order, for the five M1 CRDT types. Pipeline steps
//! 1–3 and 5–6 (decode, signature verification, authorization, equivocation,
//! causal buffering) land with their owning contracts (M0a codec, M0d, M0e).
//!
//! Experimental until the M0a crdt-apply vectors are promoted; supersedes
//! the removed `crdt::lww` scaffold (whose strictly-greater-local-keep rule
//! was the A-03 divergence the §4.5 total order fixes).

use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Raw identifier bytes (author, op id, dot). The kernel orders these
/// bytewise, matching KERNEL §4.5.
pub type Id = Vec<u8>;

/// Kernel value subset (KERNEL §4.3). Floats are carried as opaque
/// binary64 bytes and never participate in ordering.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
    Bytes(Vec<u8>),
}

/// Total-order key per KERNEL §4.5: (physical_ms, logical, author, op_id).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OrderKey {
    pub physical_ms: u64,
    pub logical: u16,
    pub author: Id,
    pub op_id: Id,
}

/// One operation as seen by the kernel (the §4.1 envelope reduced to the
/// fields steps 4/7 consume).
#[derive(Debug, Clone)]
pub struct KernelOp {
    pub op_id: Id,
    pub author: Id,
    pub physical_ms: u64,
    pub logical: u16,
    pub payload: Payload,
}

impl KernelOp {
    pub fn order_key(&self) -> OrderKey {
        OrderKey {
            physical_ms: self.physical_ms,
            logical: self.logical,
            author: self.author.clone(),
            op_id: self.op_id.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Payload {
    LwwSet(Value),
    CounterInc(u64),
    CounterDec(u64),
    SetAdd(Value),
    SetRemove { element: Value, observed: Vec<Id> },
    FlagEnable,
    FlagDisable { observed: Vec<Id> },
}

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("payload is not valid for this CRDT type")]
    WrongPayload,
    #[error("counter amount must be positive")]
    NonPositiveAmount,
}

/// Per-type apply rules (KERNEL §6 table). All rules are order-independent
/// given step-4 dedup — the property the permutation vectors falsify.
pub trait CrdtState: Default {
    fn apply(&mut self, op: &KernelOp) -> Result<(), KernelError>;
}

/// Dedup wrapper: pipeline step 4 in front of any CRDT state.
#[derive(Debug, Default)]
pub struct Replica<S: CrdtState> {
    seen: BTreeSet<Id>,
    pub state: S,
}

impl<S: CrdtState> Replica<S> {
    /// Apply an operation exactly once. Returns `false` for a duplicate
    /// (same op id already applied — a no-op per I-3).
    pub fn apply(&mut self, op: &KernelOp) -> Result<bool, KernelError> {
        if self.seen.contains(&op.op_id) {
            return Ok(false);
        }
        self.state.apply(op)?;
        self.seen.insert(op.op_id.clone());
        Ok(true)
    }
}

/// LWW register: the operation greater in the §4.5 total order wins.
#[derive(Debug, Default)]
pub struct Lww {
    value: Option<Value>,
    winner: Option<OrderKey>,
}

impl Lww {
    pub fn value(&self) -> Option<&Value> {
        self.value.as_ref()
    }
}

impl CrdtState for Lww {
    fn apply(&mut self, op: &KernelOp) -> Result<(), KernelError> {
        let Payload::LwwSet(value) = &op.payload else {
            return Err(KernelError::WrongPayload);
        };
        let key = op.order_key();
        if self.winner.as_ref().is_none_or(|w| key > *w) {
            self.value = Some(value.clone());
            self.winner = Some(key);
        }
        Ok(())
    }
}

/// Grow-only counter: per-author sums; saturates at u64::MAX.
#[derive(Debug, Default)]
pub struct GCounter {
    per_peer: BTreeMap<Id, u64>,
}

impl GCounter {
    pub fn value(&self) -> u64 {
        self.per_peer
            .values()
            .fold(0u64, |a, b| a.saturating_add(*b))
    }
}

impl CrdtState for GCounter {
    fn apply(&mut self, op: &KernelOp) -> Result<(), KernelError> {
        let Payload::CounterInc(n) = op.payload else {
            return Err(KernelError::WrongPayload);
        };
        if n == 0 {
            return Err(KernelError::NonPositiveAmount);
        }
        let entry = self.per_peer.entry(op.author.clone()).or_insert(0);
        *entry = entry.saturating_add(n);
        Ok(())
    }
}

/// Positive-negative counter: two grow-only maps; value = pos − neg.
#[derive(Debug, Default)]
pub struct PnCounter {
    pos: BTreeMap<Id, u64>,
    neg: BTreeMap<Id, u64>,
}

impl PnCounter {
    pub fn value(&self) -> i128 {
        let sum = |m: &BTreeMap<Id, u64>| m.values().map(|v| *v as i128).sum::<i128>();
        sum(&self.pos) - sum(&self.neg)
    }
}

impl CrdtState for PnCounter {
    fn apply(&mut self, op: &KernelOp) -> Result<(), KernelError> {
        let (map, n) = match op.payload {
            Payload::CounterInc(n) => (&mut self.pos, n),
            Payload::CounterDec(n) => (&mut self.neg, n),
            _ => return Err(KernelError::WrongPayload),
        };
        if n == 0 {
            return Err(KernelError::NonPositiveAmount);
        }
        let entry = map.entry(op.author.clone()).or_insert(0);
        *entry = entry.saturating_add(n);
        Ok(())
    }
}

/// Observed-remove set. A dot is the adding operation's op id; removes
/// tombstone exactly the dots they observed. Tombstones defeat their own
/// dot under any arrival order; unobserved concurrent adds survive.
#[derive(Debug, Default)]
pub struct OrSet {
    add_dots: BTreeMap<Id, Value>,
    tombstones: BTreeSet<Id>,
}

impl OrSet {
    pub fn elements(&self) -> BTreeSet<&Value> {
        self.add_dots
            .iter()
            .filter(|(dot, _)| !self.tombstones.contains(*dot))
            .map(|(_, v)| v)
            .collect()
    }

    pub fn contains(&self, value: &Value) -> bool {
        self.elements().contains(value)
    }
}

impl CrdtState for OrSet {
    fn apply(&mut self, op: &KernelOp) -> Result<(), KernelError> {
        match &op.payload {
            Payload::SetAdd(value) => {
                self.add_dots.insert(op.op_id.clone(), value.clone());
            }
            Payload::SetRemove { observed, .. } => {
                for dot in observed {
                    self.tombstones.insert(dot.clone());
                }
            }
            _ => return Err(KernelError::WrongPayload),
        }
        Ok(())
    }
}

/// Enable-wins flag: an OR-set of unit dots.
#[derive(Debug, Default)]
pub struct Flag {
    enable_dots: BTreeSet<Id>,
    tombstones: BTreeSet<Id>,
}

impl Flag {
    pub fn enabled(&self) -> bool {
        self.enable_dots
            .iter()
            .any(|d| !self.tombstones.contains(d))
    }
}

impl CrdtState for Flag {
    fn apply(&mut self, op: &KernelOp) -> Result<(), KernelError> {
        match &op.payload {
            Payload::FlagEnable => {
                self.enable_dots.insert(op.op_id.clone());
            }
            Payload::FlagDisable { observed } => {
                for dot in observed {
                    self.tombstones.insert(dot.clone());
                }
            }
            _ => return Err(KernelError::WrongPayload),
        }
        Ok(())
    }
}
