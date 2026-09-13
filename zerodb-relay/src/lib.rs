//! Experimental L2 relay process (RELAY-SPEC 0.2.2-draft).
//!
//! Handshake, SIGNAL (0x42) mailbox + live WS fanout, durable validated oplog, dual-root catch-up, resume cursor,
//! reject-ack, frozen-snapshot Merkle subtree/leaf walk, and M3b-sig
//! operation admission (signature + OpId + datastore bind), plus the durable
//! AUTH membership grant cache, token-gated SUBSCRIBE, and grant-op write filter.
//!
//! Transport hardening ([`ListenConfig`]): pre-decode WebSocket message
//! ceiling, handshake deadline, idle timeout, global connection cap, and
//! optional in-process TLS (`wss://`) via rustls. PING/PONG keepalive and
//! GOODBYE are handled by the session.

mod session;
mod store;
mod ws;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub use session::{
    ConnectionSlot, MAX_FRAME_BYTES, Relay, RelayError, RelaySession, RelayStats,
    RelayStatsSnapshot,
};
pub use ws::{serve_connection, serve_connection_with, spawn_listener, spawn_listener_with};

/// Listener / per-connection hardening knobs. `Default` is the binary's
/// default; tests tighten the timeouts.
#[derive(Clone)]
pub struct ListenConfig {
    /// HELLO/AUTH must complete (WELCOME sent) within this window, measured
    /// from TCP accept — covers the TLS + WebSocket upgrade as well.
    pub handshake_timeout: Duration,
    /// Close a session with no inbound WebSocket message (binary frame or
    /// WebSocket ping) for this long. `None` disables. RELAY §4.6 PING
    /// (protocol) and WebSocket pings both count as activity.
    pub idle_timeout: Option<Duration>,
    /// Global cap on simultaneous transport connections (all peers).
    pub max_connections: usize,
    /// Terminate TLS in-process when set (`wss://`). `None` is plaintext
    /// (`ws://`; loopback-only unless `--allow-insecure`).
    pub tls: Option<Arc<rustls::ServerConfig>>,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(10),
            idle_timeout: Some(Duration::from_secs(300)),
            max_connections: 1024,
            tls: None,
        }
    }
}

/// Build a rustls server config from PEM files (certificate chain + PKCS#8 /
/// SEC1 / PKCS#1 private key). No CA, no minting — the operator supplies both.
pub fn load_tls_config(
    cert_pem: &Path,
    key_pem: &Path,
) -> Result<Arc<rustls::ServerConfig>, String> {
    use rustls_pki_types::pem::PemObject;
    use rustls_pki_types::{CertificateDer, PrivateKeyDer};

    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_pem)
        .map_err(|e| format!("{}: {e}", cert_pem.display()))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", cert_pem.display()))?;
    if certs.is_empty() {
        return Err(format!("{}: no certificates", cert_pem.display()));
    }
    let key =
        PrivateKeyDer::from_pem_file(key_pem).map_err(|e| format!("{}: {e}", key_pem.display()))?;
    tls_config_from_der(certs, key)
}

/// Build a rustls server config from DER material (tests / embedders).
pub fn tls_config_from_der(
    certs: Vec<rustls_pki_types::CertificateDer<'static>>,
    key: rustls_pki_types::PrivateKeyDer<'static>,
) -> Result<Arc<rustls::ServerConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(cfg))
}

/// Plaintext listen is loopback-only unless `--allow-insecure` is set.
/// A TLS listener may bind anywhere.
pub fn listen_allowed(bind: &str, allow_insecure: bool, tls: bool) -> bool {
    tls || plaintext_listen_allowed(bind, allow_insecure)
}

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
    use super::{listen_allowed, plaintext_listen_allowed};

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

    #[test]
    fn tls_may_bind_anywhere_plaintext_still_gated() {
        assert!(listen_allowed("0.0.0.0:7700", false, true));
        assert!(!listen_allowed("0.0.0.0:7700", false, false));
        assert!(listen_allowed("127.0.0.1:7700", false, false));
    }
}
