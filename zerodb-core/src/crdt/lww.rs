use crate::hlc::HLCTimestamp;
use serde::{Deserialize, Serialize};

/// Last-Writer-Wins register.
///
/// Stores a value tagged with the HLC timestamp of its last write.
/// Concurrent writes are resolved by picking the higher timestamp;
/// ties are broken deterministically by HLCTimestamp's total order
/// (physical_ms → logical → peer_id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LWW<T> {
    value: T,
    timestamp: HLCTimestamp,
}

impl<T> LWW<T> {
    /// Create a new register with an initial value and timestamp.
    pub fn new(value: T, timestamp: HLCTimestamp) -> Self {
        Self { value, timestamp }
    }

    /// Current value.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Timestamp of the current value.
    pub fn timestamp(&self) -> &HLCTimestamp {
        &self.timestamp
    }

    /// Set a new value. Only takes effect if `timestamp` is strictly
    /// greater than the current timestamp. Returns `true` if updated.
    pub fn set(&mut self, value: T, timestamp: HLCTimestamp) -> bool {
        if timestamp > self.timestamp {
            self.value = value;
            self.timestamp = timestamp;
            true
        } else {
            false
        }
    }

    /// Merge with another LWW register. The higher timestamp wins.
    pub fn merge(&mut self, other: LWW<T>) {
        if other.timestamp > self.timestamp {
            self.value = other.value;
            self.timestamp = other.timestamp;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PeerId;

    fn peer(byte: u8) -> PeerId {
        let mut id = [0u8; 16];
        id[0] = byte;
        PeerId(id)
    }

    fn ts(physical_ms: u64, logical: u16, peer_byte: u8) -> HLCTimestamp {
        HLCTimestamp {
            physical_ms,
            logical,
            peer_id: peer(peer_byte),
        }
    }

    #[test]
    fn basic_set_and_get() {
        let mut reg = LWW::new("hello", ts(100, 0, 1));
        assert_eq!(*reg.value(), "hello");

        assert!(reg.set("world", ts(200, 0, 1)));
        assert_eq!(*reg.value(), "world");
    }

    #[test]
    fn stale_write_rejected() {
        let mut reg = LWW::new("current", ts(200, 0, 1));
        assert!(!reg.set("old", ts(100, 0, 1)));
        assert_eq!(*reg.value(), "current");
    }

    #[test]
    fn merge_picks_higher_timestamp() {
        let mut a = LWW::new("a-value", ts(100, 0, 1));
        let b = LWW::new("b-value", ts(200, 0, 2));
        a.merge(b);
        assert_eq!(*a.value(), "b-value");
    }

    #[test]
    fn merge_commutativity() {
        let ts_a = ts(100, 5, 1);
        let ts_b = ts(100, 5, 2); // same physical+logical, higher peer_id wins

        let mut a1 = LWW::new(10i32, ts_a);
        let b1 = LWW::new(20i32, ts_b);
        a1.merge(b1);

        let mut a2 = LWW::new(20i32, ts_b);
        let b2 = LWW::new(10i32, ts_a);
        a2.merge(b2);

        // Both orderings produce the same winner.
        assert_eq!(*a1.value(), *a2.value());
        assert_eq!(a1.timestamp(), a2.timestamp());
    }

    #[test]
    fn merge_idempotency() {
        let t = ts(300, 0, 1);
        let mut reg = LWW::new("x", t);
        let dup = LWW::new("x", t);
        reg.merge(dup);
        assert_eq!(*reg.value(), "x");
        assert_eq!(*reg.timestamp(), t);
    }
}
