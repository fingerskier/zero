use crate::types::PeerId;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// HLC timestamp: (physical_ms, logical, peer_id).
///
/// Ordering is deterministic: physical_ms first, then logical counter,
/// then peer_id as tiebreaker. This gives a total order over all
/// timestamps in the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HLCTimestamp {
    /// Milliseconds since UNIX epoch.
    pub physical_ms: u64,
    /// Logical counter (0..65535). Disambiguates within the same millisecond.
    pub logical: u16,
    /// Originating peer.
    pub peer_id: PeerId,
}

impl Ord for HLCTimestamp {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.physical_ms
            .cmp(&other.physical_ms)
            .then(self.logical.cmp(&other.logical))
            .then(self.peer_id.cmp(&other.peer_id))
    }
}

impl PartialOrd for HLCTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl HLCTimestamp {
    /// Create a zero timestamp for a given peer (used as initial state).
    pub fn zero(peer_id: PeerId) -> Self {
        Self {
            physical_ms: 0,
            logical: 0,
            peer_id,
        }
    }
}

#[derive(Debug, Error)]
pub enum HLCError {
    #[error(
        "clock drift exceeds {max_drift_ms}ms \
         (remote={remote_ms}, local wall={local_ms})"
    )]
    ExcessiveDrift {
        remote_ms: u64,
        local_ms: u64,
        max_drift_ms: u64,
    },
}

/// Returns current wall-clock time as milliseconds since UNIX epoch.
fn system_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis() as u64
}

/// Hybrid Logical Clock.
///
/// Each peer maintains one HLC instance. It generates causally ordered
/// timestamps that stay close to wall-clock time.
pub struct HLC {
    peer_id: PeerId,
    latest: HLCTimestamp,
    max_drift_ms: u64,
    clock: fn() -> u64,
}

impl HLC {
    /// Create a new HLC using the system clock.
    pub fn new(peer_id: PeerId) -> Self {
        Self {
            peer_id,
            latest: HLCTimestamp::zero(peer_id),
            max_drift_ms: 60_000,
            clock: system_clock_ms,
        }
    }

    /// Create an HLC with an injectable clock source (for testing).
    pub fn with_clock(peer_id: PeerId, clock: fn() -> u64) -> Self {
        Self {
            peer_id,
            latest: HLCTimestamp::zero(peer_id),
            max_drift_ms: 60_000,
            clock,
        }
    }

    /// Set the maximum tolerated clock drift in milliseconds.
    pub fn set_max_drift(&mut self, ms: u64) {
        self.max_drift_ms = ms;
    }

    /// Generate a timestamp for a local event.
    pub fn now(&mut self) -> HLCTimestamp {
        let wall = (self.clock)();
        let physical_ms = wall.max(self.latest.physical_ms);

        let logical = if physical_ms == self.latest.physical_ms {
            // Same millisecond — increment logical counter.
            match self.latest.logical.checked_add(1) {
                Some(l) => {
                    self.latest = HLCTimestamp {
                        physical_ms,
                        logical: l,
                        peer_id: self.peer_id,
                    };
                    return self.latest;
                }
                None => {
                    // Overflow: advance physical by 1ms, reset counter.
                    self.latest = HLCTimestamp {
                        physical_ms: physical_ms + 1,
                        logical: 0,
                        peer_id: self.peer_id,
                    };
                    return self.latest;
                }
            }
        } else {
            0
        };

        self.latest = HLCTimestamp {
            physical_ms,
            logical,
            peer_id: self.peer_id,
        };
        self.latest
    }

    /// Update the clock after receiving a remote timestamp.
    ///
    /// Returns the new local timestamp. Returns an error if the remote
    /// clock drifts beyond `max_drift_ms` ahead of our wall clock
    /// (the caller decides whether to log-and-continue or reject).
    pub fn recv(&mut self, remote: &HLCTimestamp) -> Result<HLCTimestamp, HLCError> {
        let wall = (self.clock)();

        // Drift check: is the remote timestamp too far in the future?
        if remote.physical_ms > wall + self.max_drift_ms {
            return Err(HLCError::ExcessiveDrift {
                remote_ms: remote.physical_ms,
                local_ms: wall,
                max_drift_ms: self.max_drift_ms,
            });
        }

        let max_physical = wall.max(self.latest.physical_ms).max(remote.physical_ms);

        let logical = if max_physical == self.latest.physical_ms
            && max_physical == remote.physical_ms
        {
            // All three tied — take max of the two logical counters + 1.
            self.latest.logical.max(remote.logical).saturating_add(1)
        } else if max_physical == self.latest.physical_ms {
            // Our physical is the max — increment our counter.
            self.latest.logical.saturating_add(1)
        } else if max_physical == remote.physical_ms {
            // Remote physical is the max — continue from remote counter.
            remote.logical.saturating_add(1)
        } else {
            // Wall clock is the max — fresh counter.
            0
        };

        // Handle u16 saturation (extremely unlikely but correct).
        let (physical_ms, logical) = if logical == u16::MAX && max_physical == self.latest.physical_ms {
            (max_physical + 1, 0)
        } else {
            (max_physical, logical)
        };

        self.latest = HLCTimestamp {
            physical_ms,
            logical,
            peer_id: self.peer_id,
        };
        Ok(self.latest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static MOCK_TIME: AtomicU64 = AtomicU64::new(1_000_000);

    fn mock_clock() -> u64 {
        MOCK_TIME.load(Ordering::SeqCst)
    }

    fn set_mock_time(ms: u64) {
        MOCK_TIME.store(ms, Ordering::SeqCst);
    }

    fn test_peer(byte: u8) -> PeerId {
        let mut id = [0u8; 16];
        id[0] = byte;
        PeerId(id)
    }

    #[test]
    fn monotonic_now() {
        let peer = test_peer(1);
        let mut hlc = HLC::new(peer);
        let mut prev = hlc.now();
        for _ in 0..1000 {
            let ts = hlc.now();
            assert!(ts > prev, "timestamps must be strictly increasing");
            prev = ts;
        }
    }

    #[test]
    fn logical_increments_within_same_ms() {
        set_mock_time(5_000_000);
        let peer = test_peer(2);
        let mut hlc = HLC::with_clock(peer, mock_clock);

        let t1 = hlc.now();
        let t2 = hlc.now();
        let t3 = hlc.now();

        assert_eq!(t1.physical_ms, 5_000_000);
        assert_eq!(t2.physical_ms, 5_000_000);
        assert_eq!(t3.physical_ms, 5_000_000);
        assert_eq!(t1.logical, 0);
        assert_eq!(t2.logical, 1);
        assert_eq!(t3.logical, 2);
    }

    #[test]
    fn recv_advances_clock() {
        set_mock_time(1_000_000);
        let peer_a = test_peer(0xA);
        let peer_b = test_peer(0xB);
        let mut hlc = HLC::with_clock(peer_a, mock_clock);

        // Remote timestamp from the future (but within drift limit).
        let remote = HLCTimestamp {
            physical_ms: 1_050_000,
            logical: 5,
            peer_id: peer_b,
        };

        let ts = hlc.recv(&remote).unwrap();
        assert!(ts > remote, "local clock must advance past remote");

        // Subsequent now() must also be past remote.
        let ts2 = hlc.now();
        assert!(ts2 > ts);
    }

    #[test]
    fn recv_detects_excessive_drift() {
        set_mock_time(1_000_000);
        let peer_a = test_peer(0xA);
        let peer_b = test_peer(0xB);
        let mut hlc = HLC::with_clock(peer_a, mock_clock);
        hlc.set_max_drift(60_000);

        let remote = HLCTimestamp {
            physical_ms: 1_000_000 + 60_001, // 1ms over the limit
            logical: 0,
            peer_id: peer_b,
        };

        assert!(hlc.recv(&remote).is_err());
    }

    #[test]
    fn logical_overflow_advances_physical() {
        set_mock_time(2_000_000);
        let peer = test_peer(3);
        let mut hlc = HLC::with_clock(peer, mock_clock);

        // Manually set latest to near-overflow state.
        hlc.latest = HLCTimestamp {
            physical_ms: 2_000_000,
            logical: u16::MAX - 1,
            peer_id: peer,
        };

        let t1 = hlc.now(); // logical = MAX
        assert_eq!(t1.logical, u16::MAX);
        assert_eq!(t1.physical_ms, 2_000_000);

        let t2 = hlc.now(); // overflow -> physical advances
        assert_eq!(t2.physical_ms, 2_000_001);
        assert_eq!(t2.logical, 0);
        assert!(t2 > t1);
    }

    #[test]
    fn timestamp_ordering_physical_then_logical_then_peer() {
        let peer_a = test_peer(0x01);
        let peer_b = test_peer(0x02);

        // Physical time wins.
        let t1 = HLCTimestamp { physical_ms: 100, logical: 99, peer_id: peer_b };
        let t2 = HLCTimestamp { physical_ms: 200, logical: 0, peer_id: peer_a };
        assert!(t2 > t1);

        // Same physical — logical wins.
        let t3 = HLCTimestamp { physical_ms: 100, logical: 1, peer_id: peer_a };
        let t4 = HLCTimestamp { physical_ms: 100, logical: 2, peer_id: peer_a };
        assert!(t4 > t3);

        // Same physical and logical — peer_id wins.
        let t5 = HLCTimestamp { physical_ms: 100, logical: 0, peer_id: peer_a };
        let t6 = HLCTimestamp { physical_ms: 100, logical: 0, peer_id: peer_b };
        assert!(t6 > t5);
    }
}
