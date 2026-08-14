//! Experimental L2 relay process (RELAY-SPEC 0.2.2-draft).
//!
//! Handshake, durable validated oplog, dual-root catch-up, resume cursor,
//! reject-ack. No membership/authz (M3b). No Merkle walk.

mod session;
mod store;

pub use session::{Relay, RelayError, RelaySession};
