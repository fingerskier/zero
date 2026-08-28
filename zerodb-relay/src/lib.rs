//! Experimental L2 relay process (RELAY-SPEC 0.2.2-draft).
//!
//! Handshake, durable validated oplog, dual-root catch-up, resume cursor,
//! reject-ack, frozen-snapshot Merkle subtree/leaf walk, and M3b-sig
//! operation admission (signature + OpId + datastore bind), plus the durable
//! AUTH membership grant cache, token-gated SUBSCRIBE, and grant-op write filter.

mod session;
mod store;

pub use session::{MAX_FRAME_BYTES, Relay, RelayError, RelaySession};

/// Plaintext listen is loopback-only unless `--allow-insecure` is set.
/// The binary does not mint certificates; TLS is the operator's job.
pub fn plaintext_listen_allowed(bind: &str, allow_insecure: bool) -> bool {
    if allow_insecure {
        return true;
    }
    let host = bind
        .rsplit_once(':')
        .map(|(h, _)| h.trim_matches(|c| c == '[' || c == ']'))
        .unwrap_or(bind);
    matches!(host, "127.0.0.1" | "localhost" | "::1" | "0:0:0:0:0:0:0:1")
}

#[cfg(test)]
mod listen_tests {
    use super::plaintext_listen_allowed;

    #[test]
    fn loopback_ok_without_flag() {
        assert!(plaintext_listen_allowed("127.0.0.1:7700", false));
        assert!(plaintext_listen_allowed("localhost:7700", false));
        assert!(plaintext_listen_allowed("[::1]:7700", false));
    }

    #[test]
    fn wildcard_requires_allow_insecure() {
        assert!(!plaintext_listen_allowed("0.0.0.0:7700", false));
        assert!(plaintext_listen_allowed("0.0.0.0:7700", true));
    }
}
