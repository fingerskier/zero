//! WAL / group reference model per doc/WAL.md (M0e.1, C8, DQ-4).
//! Pure in-memory state machine — no I/O.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelTs {
    pub physical_ms: u64,
    pub logical: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecord {
    pub op_id: [u8; 32],
    pub body: String,
    pub group: Option<[u8; 16]>,
    pub author_ts: ModelTs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupManifest {
    pub group_id: [u8; 16],
    pub members: Vec<[u8; 32]>,
    /// MUST equal `members.len()` when present (WAL §2).
    pub n: Option<usize>,
    pub abort: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WalState {
    pub wal: Vec<WalRecord>,
    pub wal_buf: Vec<WalRecord>,
    pub applied: BTreeSet<[u8; 32]>,
    pub material: BTreeMap<[u8; 32], String>,
    pub hlc: Option<ModelTs>,
    pub sealed: BTreeSet<[u8; 16]>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WalError {
    #[error("GROUP_INCOMPLETE")]
    GroupIncomplete,
    #[error("GROUP_ABORTED")]
    GroupAborted,
    #[error("UNKNOWN_OP")]
    UnknownOp,
    #[error("TRUNCATE_UNSAFE")]
    TruncateUnsafe,
    #[error("INVALID")]
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    WalAppend(WalRecord),
    WalSync,
    StateApply([u8; 32]),
    HlcPersist(ModelTs),
    GroupSeal(GroupManifest),
    WalTruncate(usize),
}

impl WalState {
    pub fn wal_append(&mut self, rec: WalRecord) {
        self.wal_buf.push(rec);
    }

    pub fn wal_sync(&mut self) {
        self.wal.append(&mut self.wal_buf);
    }

    pub fn state_apply(&mut self, op_id: [u8; 32]) -> Result<(), WalError> {
        if self.applied.contains(&op_id) {
            return Ok(());
        }
        let rec = self
            .wal
            .iter()
            .find(|r| r.op_id == op_id)
            .ok_or(WalError::UnknownOp)?;
        self.material.insert(op_id, rec.body.clone());
        self.applied.insert(op_id);
        Ok(())
    }

    pub fn hlc_persist(&mut self, ts: ModelTs) -> Result<(), WalError> {
        // MUST only persist an HLC that a synced WAL record already carries.
        if !self.wal.iter().any(|r| r.author_ts == ts) {
            return Err(WalError::Invalid);
        }
        self.hlc = Some(ts);
        Ok(())
    }

    pub fn group_seal(&mut self, m: &GroupManifest) -> Result<(), WalError> {
        if m.abort {
            return Err(WalError::GroupAborted);
        }
        if m.members.is_empty() {
            return Err(WalError::Invalid);
        }
        if m.n.is_some_and(|n| n != m.members.len()) {
            return Err(WalError::Invalid);
        }
        let mut uniq = BTreeSet::new();
        for id in &m.members {
            if !uniq.insert(*id) {
                return Err(WalError::Invalid);
            }
        }
        for id in &m.members {
            if !self.applied.contains(id) {
                return Err(WalError::GroupIncomplete);
            }
            let rec = self.wal.iter().find(|r| r.op_id == *id);
            match rec {
                Some(r) if r.group == Some(m.group_id) => {}
                Some(_) => return Err(WalError::Invalid),
                None => return Err(WalError::Invalid),
            }
        }
        self.sealed.insert(m.group_id);
        Ok(())
    }

    /// Application-visible materialization (I-13): unsealed group members
    /// stay hidden even if they sit in `material` for CRDT tests.
    pub fn visible_material(&self) -> BTreeMap<[u8; 32], String> {
        self.material
            .iter()
            .filter(|(id, _)| match self.wal.iter().find(|r| r.op_id == **id) {
                Some(r) => r.group.is_none_or(|g| self.sealed.contains(&g)),
                None => true,
            })
            .map(|(k, v)| (*k, v.clone()))
            .collect()
    }

    pub fn wal_truncate(&mut self, up_to_index: usize) -> Result<(), WalError> {
        if up_to_index > self.wal.len() {
            return Err(WalError::Invalid);
        }
        for rec in &self.wal[..up_to_index] {
            if !self.applied.contains(&rec.op_id) {
                return Err(WalError::TruncateUnsafe);
            }
            if rec.group.is_some_and(|g| !self.sealed.contains(&g)) {
                return Err(WalError::TruncateUnsafe);
            }
        }
        self.wal.drain(..up_to_index);
        Ok(())
    }

    /// Discard buffer; re-apply wal; resume HLC as max author_ts in wal (all authors —
    /// fixture model uses a single device; multi-device filter is M1).
    pub fn recover(&mut self) {
        self.wal_buf.clear();
        let ids: Vec<[u8; 32]> = self.wal.iter().map(|r| r.op_id).collect();
        for id in ids {
            let _ = self.state_apply(id);
        }
        self.hlc = self.wal.iter().map(|r| r.author_ts).max_by(|a, b| {
            a.physical_ms
                .cmp(&b.physical_ms)
                .then(a.logical.cmp(&b.logical))
        });
    }

    pub fn run_step(&mut self, step: &Step) -> Result<(), WalError> {
        match step {
            Step::WalAppend(r) => {
                self.wal_append(r.clone());
                Ok(())
            }
            Step::WalSync => {
                self.wal_sync();
                Ok(())
            }
            Step::StateApply(id) => self.state_apply(*id),
            Step::HlcPersist(ts) => self.hlc_persist(*ts),
            Step::GroupSeal(m) => self.group_seal(m),
            Step::WalTruncate(n) => self.wal_truncate(*n),
        }
    }

    /// Run schedule[0..=crash_after_step], then recover.
    /// `crash_after_step = -1` means crash before any step.
    pub fn run_until_crash(schedule: &[Step], crash_after_step: i32) -> WalState {
        let mut st = WalState::default();
        let last = if crash_after_step < 0 {
            -1
        } else {
            crash_after_step.min(schedule.len() as i32 - 1)
        };
        for (i, step) in schedule.iter().enumerate() {
            if i as i32 > last {
                break;
            }
            // Errors mid-schedule still leave state; crash captures that state.
            let _ = st.run_step(step);
        }
        st.recover();
        st
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(b: u8) -> [u8; 32] {
        [b; 32]
    }

    fn rec(b: u8, p: u64) -> WalRecord {
        WalRecord {
            op_id: oid(b),
            body: format!("b{b}"),
            group: None,
            author_ts: ModelTs {
                physical_ms: p,
                logical: 0,
            },
        }
    }

    #[test]
    fn crash_after_append_loses_buffer() {
        let schedule = vec![
            Step::WalAppend(rec(1, 100)),
            Step::WalSync,
            Step::StateApply(oid(1)),
        ];
        // crash after first step (append only)
        let st = WalState::run_until_crash(&schedule, 0);
        assert!(st.wal.is_empty());
        assert!(st.applied.is_empty());
        assert!(st.hlc.is_none());
    }

    #[test]
    fn crash_after_sync_keeps_wal_reapplies() {
        let schedule = vec![
            Step::WalAppend(rec(1, 100)),
            Step::WalSync,
            Step::StateApply(oid(1)),
            Step::HlcPersist(ModelTs {
                physical_ms: 100,
                logical: 0,
            }),
        ];
        // crash after sync (step index 1)
        let st = WalState::run_until_crash(&schedule, 1);
        assert_eq!(st.wal.len(), 1);
        assert!(st.applied.contains(&oid(1)));
        assert_eq!(
            st.hlc,
            Some(ModelTs {
                physical_ms: 100,
                logical: 0
            })
        );
    }

    #[test]
    fn group_seal_requires_all_members() {
        let mut st = WalState::default();
        st.wal_append(WalRecord {
            op_id: oid(1),
            body: "a".into(),
            group: Some([9u8; 16]),
            author_ts: ModelTs {
                physical_ms: 1,
                logical: 0,
            },
        });
        st.wal_sync();
        st.state_apply(oid(1)).unwrap();
        let m = GroupManifest {
            group_id: [9u8; 16],
            members: vec![oid(1), oid(2)],
            n: None,
            abort: false,
        };
        assert_eq!(st.group_seal(&m), Err(WalError::GroupIncomplete));
    }
}
