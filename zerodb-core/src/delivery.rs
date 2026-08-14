//! Delivery / anti-replay / resume model per doc/DELIVERY.md (M0e.2).

use std::collections::{BTreeMap, BTreeSet};

use crate::frontier::{Frontier, FrontierTip};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Accept,
    Duplicate,
    Reject,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Accept => "ACCEPT",
            Outcome::Duplicate => "DUPLICATE",
            Outcome::Reject => "REJECT",
        }
    }
}

/// One held/seen op with the metadata a frontier resume needs (CX-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownOp {
    pub op_id: [u8; 32],
    pub author: [u8; 32],
    pub physical_ms: u64,
    pub logical: u16,
}

impl KnownOp {
    fn order_key(&self) -> (u64, u16, [u8; 32], [u8; 32]) {
        (self.physical_ms, self.logical, self.author, self.op_id)
    }
}

/// Independent sender + receiver. Resume uses the receiver's frontier
/// cursor, not an in-process set-diff of two sibling sets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeliveryState {
    pub seen: BTreeSet<[u8; 32]>,
    pub seen_meta: BTreeMap<[u8; 32], KnownOp>,
    /// Sender-side hold.
    pub held: BTreeMap<[u8; 32], KnownOp>,
}

/// Receiver cursor: author frontier + epoch (DELIVERY §4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cursor {
    pub frontier: Frontier,
    pub epoch: u64,
}

impl DeliveryState {
    pub fn deliver(&mut self, op_id: [u8; 32], reject: bool) -> Outcome {
        let op = self.held.get(&op_id).copied().unwrap_or(KnownOp {
            op_id,
            author: op_id,
            physical_ms: 0,
            logical: 0,
        });
        self.deliver_known(op, reject)
    }

    pub fn deliver_known(&mut self, op: KnownOp, reject: bool) -> Outcome {
        if reject {
            return Outcome::Reject;
        }
        if self.seen.contains(&op.op_id) {
            return Outcome::Duplicate;
        }
        self.seen.insert(op.op_id);
        self.seen_meta.insert(op.op_id, op);
        Outcome::Accept
    }

    pub fn deliver_batch(&mut self, ids: &[[u8; 32]], rejects: &[bool]) -> Vec<Outcome> {
        ids.iter()
            .zip(rejects.iter().chain(std::iter::repeat(&false)))
            .map(|(id, &rej)| self.deliver(*id, rej))
            .collect()
    }

    pub fn receiver_cursor(&self) -> Cursor {
        let mut best: BTreeMap<[u8; 32], &KnownOp> = BTreeMap::new();
        for op in self.seen_meta.values() {
            best.entry(op.author)
                .and_modify(|cur| {
                    if op.order_key() > cur.order_key() {
                        *cur = op;
                    }
                })
                .or_insert(op);
        }
        Cursor {
            frontier: best
                .into_iter()
                .map(|(author, op)| {
                    (
                        author,
                        FrontierTip {
                            op_id: op.op_id,
                            physical_ms: op.physical_ms,
                            logical: op.logical,
                        },
                    )
                })
                .collect(),
            epoch: 0,
        }
    }

    fn covered_by(cursor: &Cursor, op: &KnownOp) -> bool {
        match cursor.frontier.get(&op.author) {
            None => false,
            Some(tip) => {
                let tip_key = (tip.physical_ms, tip.logical, op.author, tip.op_id);
                op.order_key() <= tip_key
            }
        }
    }

    /// Resume against an independently computed receiver cursor (CX-05).
    pub fn resume_missing(&self) -> Vec<[u8; 32]> {
        self.resume_from(&self.receiver_cursor())
    }

    pub fn resume_from(&self, cursor: &Cursor) -> Vec<[u8; 32]> {
        let mut out: Vec<[u8; 32]> = self
            .held
            .values()
            .filter(|op| !Self::covered_by(cursor, op))
            .map(|op| op.op_id)
            .collect();
        out.sort();
        out
    }

    pub fn hold(&mut self, op_id: [u8; 32]) {
        self.hold_known(KnownOp {
            op_id,
            author: op_id,
            physical_ms: 0,
            logical: 0,
        });
    }

    pub fn hold_known(&mut self, op: KnownOp) {
        self.held.insert(op.op_id, op);
    }
}

/// Schedule step for delivery-schedule vectors.
#[derive(Debug, Clone)]
pub enum DelivStep {
    Hold(KnownOp),
    Deliver {
        op_id: [u8; 32],
        reject: bool,
    },
    DeliverBatch {
        op_ids: Vec<[u8; 32]>,
        rejects: Vec<bool>,
    },
    Resume,
}

#[derive(Debug, Clone, Default)]
pub struct DelivResult {
    pub seen: Vec<[u8; 32]>,
    pub last_outcomes: Vec<String>,
    pub resume_sent: Vec<[u8; 32]>,
}

pub fn run_schedule(steps: &[DelivStep]) -> DelivResult {
    let mut st = DeliveryState::default();
    let mut last_outcomes = Vec::new();
    let mut resume_sent = Vec::new();
    for step in steps {
        match step {
            DelivStep::Hold(op) => st.hold_known(*op),
            DelivStep::Deliver { op_id, reject } => {
                last_outcomes = vec![st.deliver(*op_id, *reject).as_str().into()];
            }
            DelivStep::DeliverBatch { op_ids, rejects } => {
                last_outcomes = st
                    .deliver_batch(op_ids, rejects)
                    .iter()
                    .map(|o| o.as_str().to_string())
                    .collect();
            }
            DelivStep::Resume => {
                let missing = st.resume_missing();
                resume_sent = missing.clone();
                for id in &missing {
                    if let Some(op) = st.held.get(id).copied() {
                        let _ = st.deliver_known(op, false);
                    } else {
                        let _ = st.deliver(*id, false);
                    }
                }
            }
        }
    }
    DelivResult {
        seen: st.seen.into_iter().collect(),
        last_outcomes,
        resume_sent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> [u8; 32] {
        [b; 32]
    }

    #[test]
    fn duplicate_after_accept() {
        let mut st = DeliveryState::default();
        assert_eq!(st.deliver(id(1), false), Outcome::Accept);
        assert_eq!(st.deliver(id(1), false), Outcome::Duplicate);
    }

    #[test]
    fn resume_fills_gap() {
        let mut st = DeliveryState::default();
        st.hold_known(KnownOp {
            op_id: id(1),
            author: [0xAA; 32],
            physical_ms: 100,
            logical: 0,
        });
        st.hold_known(KnownOp {
            op_id: id(2),
            author: [0xAA; 32],
            physical_ms: 200,
            logical: 0,
        });
        st.deliver_known(
            KnownOp {
                op_id: id(1),
                author: [0xAA; 32],
                physical_ms: 100,
                logical: 0,
            },
            false,
        );
        let miss = st.resume_missing();
        assert_eq!(miss, vec![id(2)]);
    }

    #[test]
    fn resume_frontier_does_not_retransmit_covered_late_op() {
        let mut st = DeliveryState::default();
        let late = KnownOp {
            op_id: id(1),
            author: [0xAA; 32],
            physical_ms: 50,
            logical: 0,
        };
        let tip = KnownOp {
            op_id: id(2),
            author: [0xAA; 32],
            physical_ms: 200,
            logical: 0,
        };
        st.hold_known(late);
        st.hold_known(tip);
        st.deliver_known(tip, false);
        // Frontier covers everything ≤ tip, including the unseen late op.
        assert!(st.resume_missing().is_empty());
    }

    #[test]
    fn reorder_same_seen() {
        let r1 = run_schedule(&[
            DelivStep::Deliver {
                op_id: id(1),
                reject: false,
            },
            DelivStep::Deliver {
                op_id: id(2),
                reject: false,
            },
        ]);
        let r2 = run_schedule(&[
            DelivStep::Deliver {
                op_id: id(2),
                reject: false,
            },
            DelivStep::Deliver {
                op_id: id(1),
                reject: false,
            },
        ]);
        assert_eq!(
            r1.seen.iter().collect::<BTreeSet<_>>(),
            r2.seen.iter().collect::<BTreeSet<_>>()
        );
    }
}
