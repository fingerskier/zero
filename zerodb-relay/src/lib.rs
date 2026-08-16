//! Experimental L2 relay process (RELAY-SPEC 0.2.2-draft).
//!
//! Handshake, durable validated oplog, dual-root catch-up, resume cursor,
//! reject-ack, frozen-snapshot Merkle subtree/leaf walk, and M3b-sig
//! operation admission (signature + OpId + datastore bind). AUTH
//! membership grants / E5 are not wired yet.

mod session;
mod store;

pub use session::{Relay, RelayError, RelaySession};
