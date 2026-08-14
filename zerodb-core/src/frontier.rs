//! Causal frontiers and snapshot identity per doc/FRONTIER.md (M0f).

use std::collections::BTreeMap;

use crate::merkle::{MerkleOp, merkle_root};

pub const DOMAIN_SNAPSHOT: &[u8] = b"zerodb-snapshot-v1";
pub const SNAPSHOT_FORMAT_VERSION: u8 = 1;

/// One author's tip: greatest accepted op under KERNEL §4.5, with the
/// HLC needed to decide late-op from the encoded frontier alone (CX-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontierTip {
    pub op_id: [u8; 32],
    pub physical_ms: u64,
    pub logical: u16,
}

/// Frontier: PeerId (author) → tip.
pub type Frontier = BTreeMap<[u8; 32], FrontierTip>;

fn order_key(op: &MerkleOp) -> (u64, u16, [u8; 32], [u8; 32]) {
    (op.physical_ms, op.logical, op.author, op.op_id)
}

/// Build frontier from accepted ops.
pub fn build_frontier(ops: &[MerkleOp]) -> Frontier {
    let mut best: BTreeMap<[u8; 32], &MerkleOp> = BTreeMap::new();
    for op in ops {
        best.entry(op.author)
            .and_modify(|cur| {
                if order_key(op) > order_key(cur) {
                    *cur = op;
                }
            })
            .or_insert(op);
    }
    best.into_iter()
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
        .collect()
}

/// Tail boundary: greatest OpId under full §4.5 order, or None if empty.
pub fn tail_boundary(ops: &[MerkleOp]) -> Option<[u8; 32]> {
    ops.iter()
        .max_by(|a, b| order_key(a).cmp(&order_key(b)))
        .map(|o| o.op_id)
}

/// Canonical frontier encoding: sorted by author, each
/// author ‖ op_id ‖ physical_ms (u64 BE) ‖ logical (u16 BE).
pub fn frontier_bytes(f: &Frontier) -> Vec<u8> {
    let mut out = Vec::new();
    for (author, tip) in f {
        out.extend_from_slice(author);
        out.extend_from_slice(&tip.op_id);
        out.extend_from_slice(&tip.physical_ms.to_be_bytes());
        out.extend_from_slice(&tip.logical.to_be_bytes());
    }
    out
}

pub fn snapshot_id(
    datastore_id: &[u8; 32],
    frontier: &Frontier,
    merkle: &[u8; 32],
    tail: Option<&[u8; 32]>,
) -> [u8; 32] {
    let mut pre = Vec::new();
    pre.extend_from_slice(DOMAIN_SNAPSHOT);
    pre.push(SNAPSHOT_FORMAT_VERSION);
    pre.extend_from_slice(datastore_id);
    pre.extend_from_slice(&frontier_bytes(frontier));
    pre.extend_from_slice(merkle);
    match tail {
        Some(t) => {
            pre.push(1);
            pre.extend_from_slice(t);
        }
        None => pre.push(0),
    }
    *blake3::hash(&pre).as_bytes()
}

/// Late op: author's encoded tip already dominates this op in §4.5 order.
/// Uses only the frontier map — no extra op list (CX-04).
pub fn is_late_op(op: &MerkleOp, frontier: &Frontier) -> bool {
    match frontier.get(&op.author) {
        None => false,
        Some(tip) => {
            let tip_key = (tip.physical_ms, tip.logical, op.author, tip.op_id);
            order_key(op) < tip_key && op.op_id != tip.op_id
        }
    }
}

/// Late check from a published op set: build the frontier, then `is_late_op`.
pub fn is_late_against_ops(op: &MerkleOp, frontier_ops: &[MerkleOp]) -> bool {
    is_late_op(op, &build_frontier(frontier_ops))
}

pub fn snapshot_for_ops(datastore_id: &[u8; 32], ops: &[MerkleOp]) -> [u8; 32] {
    let f = build_frontier(ops);
    let root = merkle_root(ops);
    let tail = tail_boundary(ops);
    snapshot_id(datastore_id, &f, &root, tail.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(id: u8, author: u8, p: u64) -> MerkleOp {
        MerkleOp {
            op_id: [id; 32],
            physical_ms: p,
            logical: 0,
            author: [author; 32],
        }
    }

    #[test]
    fn frontier_per_author_max() {
        let ops = vec![op(1, 0xAA, 100), op(2, 0xAA, 200), op(3, 0xBB, 150)];
        let f = build_frontier(&ops);
        assert_eq!(f.get(&[0xAA; 32]).map(|t| t.op_id), Some([2; 32]));
        assert_eq!(f.get(&[0xBB; 32]).map(|t| t.op_id), Some([3; 32]));
        assert_eq!(f.get(&[0xAA; 32]).map(|t| t.physical_ms), Some(200));
    }

    #[test]
    fn late_op_detected() {
        let base = vec![op(2, 0xAA, 200)];
        let late = op(1, 0xAA, 100);
        let f = build_frontier(&base);
        assert!(is_late_op(&late, &f));
        assert!(!is_late_op(&op(3, 0xAA, 300), &f));
        assert!(is_late_against_ops(&late, &base));
        assert!(!is_late_against_ops(&op(3, 0xAA, 300), &base));
    }

    #[test]
    fn snapshot_stable() {
        let ds = [0xDDu8; 32];
        let ops = vec![op(1, 0xAA, 1000)];
        assert_eq!(snapshot_for_ops(&ds, &ops), snapshot_for_ops(&ds, &ops));
    }

    #[test]
    #[ignore]
    fn print_snapshot_golden() {
        fn hex(b: &[u8]) -> String {
            b.iter().map(|x| format!("{x:02x}")).collect()
        }
        let ds = [0xDDu8; 32];
        let ops = vec![op(1, 0xAA, 1000)];
        eprintln!("snap={}", hex(&snapshot_for_ops(&ds, &ops)));
        eprintln!("root={}", hex(&merkle_root(&ops)));
        let f = build_frontier(&ops);
        eprintln!("frontier_author={}", hex(&[0xAAu8; 32]));
        eprintln!("frontier_tip={}", hex(&f.get(&[0xAAu8; 32]).unwrap().op_id));
    }
}
