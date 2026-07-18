//! Delivery / anti-replay / resume model per doc/DELIVERY.md (M0e.2).

use std::collections::BTreeSet;

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeliveryState {
    pub seen: BTreeSet<[u8; 32]>,
    /// Ops available for retransmit (sender side).
    pub held: BTreeSet<[u8; 32]>,
}

impl DeliveryState {
    pub fn deliver(&mut self, op_id: [u8; 32], reject: bool) -> Outcome {
        if reject {
            return Outcome::Reject;
        }
        if self.seen.contains(&op_id) {
            return Outcome::Duplicate;
        }
        self.seen.insert(op_id);
        Outcome::Accept
    }

    pub fn deliver_batch(&mut self, ids: &[[u8; 32]], rejects: &[bool]) -> Vec<Outcome> {
        ids.iter()
            .zip(rejects.iter().chain(std::iter::repeat(&false)))
            .map(|(id, &rej)| self.deliver(*id, rej))
            .collect()
    }

    /// Resume: return held ops not yet in seen (sorted for determinism).
    pub fn resume_missing(&self) -> Vec<[u8; 32]> {
        self.held.difference(&self.seen).copied().collect()
    }

    pub fn hold(&mut self, op_id: [u8; 32]) {
        self.held.insert(op_id);
    }
}

/// Schedule step for delivery-schedule vectors.
#[derive(Debug, Clone)]
pub enum DelivStep {
    Hold([u8; 32]),
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
            DelivStep::Hold(id) => st.hold(*id),
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
                for id in missing {
                    let _ = st.deliver(id, false);
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
        st.hold(id(1));
        st.hold(id(2));
        st.deliver(id(1), false);
        let miss = st.resume_missing();
        assert_eq!(miss, vec![id(2)]);
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
