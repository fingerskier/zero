//! Merkle sync tree per doc/MERKLE.md (M0c, C3).
//! Pure derived structure over an accepted op set + mismatch-recovery walk.

use std::collections::{BTreeMap, BTreeSet};

/// Domain-separation strings (registry).
pub const DOMAIN_MERKLE_LEAF: &[u8] = b"zerodb-merkle-leaf-v1";
pub const DOMAIN_MERKLE_NODE: &[u8] = b"zerodb-merkle-node-v1";
pub const DOMAIN_MERKLE_EMPTY: &[u8] = b"zerodb-merkle-empty-v1";

pub const MERKLE_FORMAT_VERSION: u8 = 1;
pub const BUCKET_WIDTH_MS: u64 = 60_000;

/// Minimal op fields needed for the Merkle tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleOp {
    pub op_id: [u8; 32],
    pub physical_ms: u64,
    pub logical: u16,
    pub author: [u8; 32],
}

impl MerkleOp {
    fn order_key(&self) -> (u64, u16, [u8; 32], [u8; 32]) {
        (self.physical_ms, self.logical, self.author, self.op_id)
    }

    pub fn bucket_index(&self) -> u64 {
        self.physical_ms / BUCKET_WIDTH_MS
    }

    pub fn op_id_hex(&self) -> String {
        self.op_id.iter().map(|b| format!("{b:02x}")).collect()
    }
}

fn blake3(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

/// Empty leaf / empty-tree root constant.
pub fn empty_leaf() -> [u8; 32] {
    let mut pre = Vec::with_capacity(DOMAIN_MERKLE_EMPTY.len() + 1);
    pre.extend_from_slice(DOMAIN_MERKLE_EMPTY);
    pre.push(MERKLE_FORMAT_VERSION);
    blake3(&pre)
}

fn leaf_hash(bucket_index: u64, op_ids_sorted: &[[u8; 32]]) -> [u8; 32] {
    let mut pre = Vec::with_capacity(DOMAIN_MERKLE_LEAF.len() + 1 + 8 + op_ids_sorted.len() * 32);
    pre.extend_from_slice(DOMAIN_MERKLE_LEAF);
    pre.push(MERKLE_FORMAT_VERSION);
    pre.extend_from_slice(&bucket_index.to_be_bytes());
    for id in op_ids_sorted {
        pre.extend_from_slice(id);
    }
    blake3(&pre)
}

fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut pre = Vec::with_capacity(DOMAIN_MERKLE_NODE.len() + 1 + 64);
    pre.extend_from_slice(DOMAIN_MERKLE_NODE);
    pre.push(MERKLE_FORMAT_VERSION);
    pre.extend_from_slice(left);
    pre.extend_from_slice(right);
    blake3(&pre)
}

fn next_power_of_two(n: usize) -> usize {
    if n <= 1 {
        return 1;
    }
    n.next_power_of_two()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// One padded leaf slot in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafSlot {
    pub hash: [u8; 32],
    /// `None` for empty padding leaves.
    pub bucket_index: Option<u64>,
    pub op_ids: Vec<[u8; 32]>,
}

/// Full tree: level 0 = padded leaves, last level = single root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleTree {
    pub levels: Vec<Vec<[u8; 32]>>,
    pub leaves: Vec<LeafSlot>,
    /// Full op set used to build the tree (for delta materialization).
    pub ops: Vec<MerkleOp>,
}

impl MerkleTree {
    pub fn root(&self) -> [u8; 32] {
        self.levels
            .last()
            .and_then(|l| l.first().copied())
            .unwrap_or_else(empty_leaf)
    }

    pub fn build(ops: &[MerkleOp]) -> Self {
        if ops.is_empty() {
            let e = empty_leaf();
            return Self {
                levels: vec![vec![e]],
                leaves: vec![LeafSlot {
                    hash: e,
                    bucket_index: None,
                    op_ids: vec![],
                }],
                ops: vec![],
            };
        }

        let mut buckets: BTreeMap<u64, Vec<&MerkleOp>> = BTreeMap::new();
        for op in ops {
            buckets.entry(op.bucket_index()).or_default().push(op);
        }

        let mut leaves: Vec<LeafSlot> = Vec::with_capacity(buckets.len());
        for (idx, mut members) in buckets {
            members.sort_by_key(|o| o.order_key());
            let ids: Vec<[u8; 32]> = members.iter().map(|o| o.op_id).collect();
            leaves.push(LeafSlot {
                hash: leaf_hash(idx, &ids),
                bucket_index: Some(idx),
                op_ids: ids,
            });
        }

        let p = next_power_of_two(leaves.len());
        let empty = empty_leaf();
        while leaves.len() < p {
            leaves.push(LeafSlot {
                hash: empty,
                bucket_index: None,
                op_ids: vec![],
            });
        }

        let mut level: Vec<[u8; 32]> = leaves.iter().map(|l| l.hash).collect();
        let mut levels = vec![level.clone()];
        while level.len() > 1 {
            let mut next = Vec::with_capacity(level.len() / 2);
            for pair in level.chunks(2) {
                next.push(node_hash(&pair[0], &pair[1]));
            }
            levels.push(next.clone());
            level = next;
        }

        Self {
            levels,
            leaves,
            ops: ops.to_vec(),
        }
    }

    pub fn node_children(&self, level: usize, index: usize) -> Option<([u8; 32], [u8; 32])> {
        if level == 0 {
            return None;
        }
        let child_level = level - 1;
        let left = *self.levels.get(child_level)?.get(index * 2)?;
        let right = *self.levels.get(child_level)?.get(index * 2 + 1)?;
        Some((left, right))
    }
}

/// Compute Merkle root for an accepted op set (order-independent).
pub fn merkle_root(ops: &[MerkleOp]) -> [u8; 32] {
    MerkleTree::build(ops).root()
}

/// Abstract walk message (MERKLE.md §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MerkleMsg {
    RootOffer {
        from: String,
        root: String,
    },
    NodeRequest {
        from: String,
        level: usize,
        index: usize,
    },
    NodeResponse {
        from: String,
        level: usize,
        index: usize,
        hash: String,
        left: String,
        right: String,
    },
    LeafRequest {
        from: String,
        leaf_index: usize,
        bucket_index: Option<u64>,
    },
    LeafResponse {
        from: String,
        leaf_index: usize,
        bucket_index: Option<u64>,
        op_ids: Vec<String>,
    },
    Delta {
        from: String,
        op_ids: Vec<String>,
    },
    Converged {
        root: String,
    },
}

impl MerkleMsg {
    /// Tag name for the message kind.
    pub fn type_name(&self) -> &'static str {
        match self {
            MerkleMsg::RootOffer { .. } => "RootOffer",
            MerkleMsg::NodeRequest { .. } => "NodeRequest",
            MerkleMsg::NodeResponse { .. } => "NodeResponse",
            MerkleMsg::LeafRequest { .. } => "LeafRequest",
            MerkleMsg::LeafResponse { .. } => "LeafResponse",
            MerkleMsg::Delta { .. } => "Delta",
            MerkleMsg::Converged { .. } => "Converged",
        }
    }
}

/// Deterministic mismatch-recovery walk: requester pulls missing ops from responder.
///
/// Snapshots are frozen at start. Messages are produced in a fixed order so both
/// runners agree on the transcript. After applying the union of ops, roots match.
pub fn sync_transcript(requester_ops: &[MerkleOp], responder_ops: &[MerkleOp]) -> Vec<MerkleMsg> {
    let mut msgs = Vec::new();
    let req_tree = MerkleTree::build(requester_ops);
    let res_tree = MerkleTree::build(responder_ops);

    let req_root = hex(&req_tree.root());
    let res_root = hex(&res_tree.root());

    msgs.push(MerkleMsg::RootOffer {
        from: "A".into(),
        root: req_root.clone(),
    });
    msgs.push(MerkleMsg::RootOffer {
        from: "B".into(),
        root: res_root.clone(),
    });

    if req_root == res_root {
        msgs.push(MerkleMsg::Converged { root: req_root });
        return msgs;
    }

    // Walk from root of B; compare against A's tree. Collect missing op_ids.
    let mut missing: BTreeSet<[u8; 32]> = BTreeSet::new();
    let root_level = res_tree.levels.len() - 1;
    walk_node(&req_tree, &res_tree, root_level, 0, &mut msgs, &mut missing);

    if !missing.is_empty() {
        let mut op_ids: Vec<String> = missing.iter().map(|id| hex(id)).collect();
        op_ids.sort();
        msgs.push(MerkleMsg::Delta {
            from: "B".into(),
            op_ids,
        });
    }

    // Merge and recompute
    let mut by_id: BTreeMap<[u8; 32], MerkleOp> = BTreeMap::new();
    for op in requester_ops {
        by_id.insert(op.op_id, op.clone());
    }
    for op in responder_ops {
        by_id.insert(op.op_id, op.clone());
    }
    let merged: Vec<MerkleOp> = by_id.into_values().collect();
    let final_root = hex(&merkle_root(&merged));
    // Both peers after applying the same set
    msgs.push(MerkleMsg::RootOffer {
        from: "A".into(),
        root: final_root.clone(),
    });
    msgs.push(MerkleMsg::RootOffer {
        from: "B".into(),
        root: final_root.clone(),
    });
    msgs.push(MerkleMsg::Converged { root: final_root });
    msgs
}

fn walk_node(
    local: &MerkleTree,
    remote: &MerkleTree,
    level: usize,
    index: usize,
    msgs: &mut Vec<MerkleMsg>,
    missing: &mut BTreeSet<[u8; 32]>,
) {
    if level == 0 {
        // Leaf
        let remote_leaf = &remote.leaves[index];
        let local_hash = local
            .leaves
            .get(index)
            .map(|l| l.hash)
            .unwrap_or_else(empty_leaf);
        if remote_leaf.hash == local_hash {
            return;
        }
        msgs.push(MerkleMsg::LeafRequest {
            from: "A".into(),
            leaf_index: index,
            bucket_index: remote_leaf.bucket_index,
        });
        let op_ids: Vec<String> = remote_leaf.op_ids.iter().map(|id| hex(id)).collect();
        msgs.push(MerkleMsg::LeafResponse {
            from: "B".into(),
            leaf_index: index,
            bucket_index: remote_leaf.bucket_index,
            op_ids: op_ids.clone(),
        });
        // Local op_ids at this slot
        let local_ids: BTreeSet<[u8; 32]> = local
            .leaves
            .get(index)
            .map(|l| l.op_ids.iter().copied().collect())
            .unwrap_or_default();
        for id in &remote_leaf.op_ids {
            if !local_ids.contains(id) {
                missing.insert(*id);
            }
        }
        return;
    }

    // Internal node: request remote children
    msgs.push(MerkleMsg::NodeRequest {
        from: "A".into(),
        level,
        index,
    });
    let (rleft, rright) = remote.node_children(level, index).expect("remote node");
    msgs.push(MerkleMsg::NodeResponse {
        from: "B".into(),
        level,
        index,
        hash: hex(&remote.levels[level][index]),
        left: hex(&rleft),
        right: hex(&rright),
    });

    let (lleft, lright) = local
        .node_children(level, index)
        .unwrap_or_else(|| (empty_leaf(), empty_leaf()));

    if rleft != lleft {
        walk_node(local, remote, level - 1, index * 2, msgs, missing);
    }
    if rright != lright {
        walk_node(local, remote, level - 1, index * 2 + 1, msgs, missing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(id: u8, p: u64) -> MerkleOp {
        MerkleOp {
            op_id: [id; 32],
            physical_ms: p,
            logical: 0,
            author: [0xAA; 32],
        }
    }

    #[test]
    fn empty_root_is_constant() {
        assert_eq!(merkle_root(&[]), empty_leaf());
        assert_eq!(merkle_root(&[]), merkle_root(&[]));
    }

    #[test]
    fn order_independent() {
        let a = op(1, 0);
        let b = op(2, 0);
        let r1 = merkle_root(&[a.clone(), b.clone()]);
        let r2 = merkle_root(&[b, a]);
        assert_eq!(r1, r2);
    }

    #[test]
    fn multi_bucket_differs_from_single() {
        let same = merkle_root(&[op(1, 0), op(2, 0)]);
        let multi = merkle_root(&[op(1, 0), op(2, 60_000)]);
        assert_ne!(same, multi);
    }

    #[test]
    fn equal_sets_converge_immediately() {
        let ops = vec![op(1, 1000), op(2, 2000)];
        let t = sync_transcript(&ops, &ops);
        assert!(matches!(t.last(), Some(MerkleMsg::Converged { .. })));
        assert_eq!(t.len(), 3); // two RootOffer + Converged
    }

    #[test]
    fn missing_op_produces_delta() {
        let a = vec![op(1, 1000)];
        let b = vec![op(1, 1000), op(2, 120_000)];
        let t = sync_transcript(&a, &b);
        let has_delta = t.iter().any(|m| matches!(m, MerkleMsg::Delta { .. }));
        assert!(has_delta);
        assert!(matches!(t.last(), Some(MerkleMsg::Converged { .. })));
        // final roots equal union
        let union = vec![op(1, 1000), op(2, 120_000)];
        let final_root = hex(&merkle_root(&union));
        match t.last() {
            Some(MerkleMsg::Converged { root }) => assert_eq!(root, &final_root),
            _ => panic!("expected Converged"),
        }
    }

    /// Write MERKLE-T-* transcript vectors under conformance/vectors/required/merkle/.
    #[test]
    #[ignore]
    fn write_transcript_vectors() {
        use std::fs;
        use std::path::{Path, PathBuf};

        fn msg_json(m: &MerkleMsg) -> String {
            match m {
                MerkleMsg::RootOffer { from, root } => {
                    format!(r#"{{ "type": "RootOffer", "from": "{from}", "root": "{root}" }}"#)
                }
                MerkleMsg::NodeRequest { from, level, index } => format!(
                    r#"{{ "type": "NodeRequest", "from": "{from}", "level": {level}, "index": {index} }}"#
                ),
                MerkleMsg::NodeResponse {
                    from,
                    level,
                    index,
                    hash,
                    left,
                    right,
                } => format!(
                    r#"{{ "type": "NodeResponse", "from": "{from}", "level": {level}, "index": {index}, "hash": "{hash}", "left": "{left}", "right": "{right}" }}"#
                ),
                MerkleMsg::LeafRequest {
                    from,
                    leaf_index,
                    bucket_index,
                } => {
                    let bi = match bucket_index {
                        Some(b) => b.to_string(),
                        None => "null".into(),
                    };
                    format!(
                        r#"{{ "type": "LeafRequest", "from": "{from}", "leaf_index": {leaf_index}, "bucket_index": {bi} }}"#
                    )
                }
                MerkleMsg::LeafResponse {
                    from,
                    leaf_index,
                    bucket_index,
                    op_ids,
                } => {
                    let bi = match bucket_index {
                        Some(b) => b.to_string(),
                        None => "null".into(),
                    };
                    let ids = op_ids
                        .iter()
                        .map(|id| format!(r#""{id}""#))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        r#"{{ "type": "LeafResponse", "from": "{from}", "leaf_index": {leaf_index}, "bucket_index": {bi}, "op_ids": [{ids}] }}"#
                    )
                }
                MerkleMsg::Delta { from, op_ids } => {
                    let ids = op_ids
                        .iter()
                        .map(|id| format!(r#""{id}""#))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(r#"{{ "type": "Delta", "from": "{from}", "op_ids": [{ids}] }}"#)
                }
                MerkleMsg::Converged { root } => {
                    format!(r#"{{ "type": "Converged", "root": "{root}" }}"#)
                }
            }
        }

        fn op_json(o: &MerkleOp) -> String {
            format!(
                r#"{{ "op_id": "{}", "physical_ms": {}, "logical": {}, "author": "{}" }}"#,
                hex(&o.op_id),
                o.physical_ms,
                o.logical,
                hex(&o.author)
            )
        }

        fn write_vec(dir: &Path, name: &str, id: &str, desc: &str, a: &[MerkleOp], b: &[MerkleOp]) {
            let t = sync_transcript(a, b);
            let final_root = match t.last() {
                Some(MerkleMsg::Converged { root }) => root.clone(),
                _ => panic!("no converge"),
            };
            let a_json = a.iter().map(op_json).collect::<Vec<_>>().join(",\n    ");
            let b_json = b.iter().map(op_json).collect::<Vec<_>>().join(",\n    ");
            let t_json = t.iter().map(msg_json).collect::<Vec<_>>().join(",\n    ");
            let body = format!(
                r#"{{
  "description": "{desc}",
  "id": "{id}",
  "type": "merkle-transcript",
  "invariants": ["I-11"],
  "peer_a": [
    {a_json}
  ],
  "peer_b": [
    {b_json}
  ],
  "final_root_hex": "{final_root}",
  "transcript": [
    {t_json}
  ]
}}
"#
            );
            fs::write(dir.join(name), body).unwrap();
        }

        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../conformance/vectors/required/merkle");
        fs::create_dir_all(&dir).unwrap();

        // equal non-empty
        let ops = vec![op(1, 1000), op(2, 2000)];
        write_vec(
            &dir,
            "MERKLE-T-001-equal.json",
            "MERKLE-T-001",
            "Equal op sets: RootOffer exchange then Converged without walk.",
            &ops,
            &ops,
        );

        // B has extra bucket
        write_vec(
            &dir,
            "MERKLE-T-002-missing-bucket.json",
            "MERKLE-T-002",
            "B is a superset: A missing an op in another time bucket; walk + Delta then Converged.",
            &[op(1, 1000)],
            &[op(1, 1000), op(2, 120_000)],
        );

        // same bucket missing second op
        write_vec(
            &dir,
            "MERKLE-T-003-same-bucket-delta.json",
            "MERKLE-T-003",
            "B is a superset within the same bucket; leaf mismatch + Delta.",
            &[op(1, 1000)],
            &[op(1, 1000), op(2, 2000)],
        );

        // empty equal
        write_vec(
            &dir,
            "MERKLE-T-004-empty-equal.json",
            "MERKLE-T-004",
            "Both empty: Converged on empty-leaf root.",
            &[],
            &[],
        );
    }
}
