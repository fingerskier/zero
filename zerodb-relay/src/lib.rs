//! Experimental L2 relay process (RELAY-SPEC 0.2.2-draft).
//!
//! Handshake, durable validated oplog, dual-root catch-up, resume cursor,
//! reject-ack, frozen-snapshot Merkle subtree/leaf walk, and M3b-sig
//! operation admission (signature + OpId + datastore bind), plus the durable
//! AUTH membership grant cache and token-gated E5 subscription/sync path.

mod session;
mod store;

pub use session::{Relay, RelayError, RelaySession};
