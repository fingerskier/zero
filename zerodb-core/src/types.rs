use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 16-byte peer identifier. In production this will be derived from
/// a public key; for now it is an opaque fixed-size ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PeerId(pub [u8; 16]);

impl PeerId {
    pub fn random() -> Self {
        Self(*Uuid::new_v4().as_bytes())
    }
}

impl std::fmt::Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Display first 4 bytes as hex for readability
        write!(
            f,
            "Peer({:02x}{:02x}{:02x}{:02x}…)",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

/// UUIDv7 wrapper for node identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

/// UUIDv7 wrapper for edge identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EdgeId(pub Uuid);

impl EdgeId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for EdgeId {
    fn default() -> Self {
        Self::new()
    }
}

/// Content-addressed operation identifier (BLAKE3 hash).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OpId(pub [u8; 32]);

/// UUIDv7 identifier for operation groups (atomic batches).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GroupId(pub Uuid);

impl GroupId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for GroupId {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_are_unique() {
        let a = NodeId::new();
        let b = NodeId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn peer_id_round_trip_serde() {
        let peer = PeerId::random();
        let json = serde_json::to_string(&peer).unwrap();
        let back: PeerId = serde_json::from_str(&json).unwrap();
        assert_eq!(peer, back);
    }
}
