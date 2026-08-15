//! Experimental L2 relay process (RELAY-SPEC 0.2.2-draft).
//!
//! Handshake, durable validated oplog, dual-root catch-up, resume cursor,
//! reject-ack, and frozen-snapshot Merkle subtree/leaf walk. No
//! membership/authz (M3b).

mod session;
mod store;

pub use session::{Relay, RelayError, RelaySession};
