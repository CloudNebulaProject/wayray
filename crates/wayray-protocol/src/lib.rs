//! WayRay wire protocol definitions.
//!
//! Shared between wrsrvd (server) and wrclient (client).
//! Messages are serialized with postcard and framed with a 4-byte
//! length prefix for transmission over QUIC streams.

pub mod admin;
pub mod cluster;
pub mod codec;
pub mod encoding;
pub mod launcher;
pub mod messages;
pub mod session_config;
pub mod tls;
pub mod transport;

/// Current protocol version. Incremented on breaking changes.
///
/// Version history:
/// - 1: initial protocol (control/display/input streams).
/// - 2: clipboard sync + audio channel. Control messages gained
///   `ProtocolError`, `ClipboardOffer` and `ClipboardData` (appended variants,
///   postcard-back-compatible); the server opens a dedicated audio stream
///   (a server-initiated bidirectional QUIC stream) when the client advertises
///   the `audio` capability. Clients must not send clipboard messages to a
///   server that reports a `ServerHello.version` below 2.
pub const PROTOCOL_VERSION: u32 = 2;

/// Oldest peer protocol version this build still interoperates with. Version-1
/// peers never advertise the new capabilities, so all v2 features are gated off
/// automatically for them.
pub const MIN_PROTOCOL_VERSION: u32 = 1;

/// Whether a peer speaking `peer_version` is compatible with this build.
///
/// Both sides apply the same rule: the peer's version must fall within
/// `[MIN_PROTOCOL_VERSION, PROTOCOL_VERSION]`. Versions newer than ours are
/// rejected too — we cannot know what a future peer requires, and a typed
/// rejection is a clearer failure than a mid-session decode error.
pub fn version_compatible(peer_version: u32) -> bool {
    (MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&peer_version)
}

/// QUIC application close codes used when a connection is terminated for
/// protocol-level reasons. The close reason carries a human-readable string.
pub mod close_codes {
    /// The peer's protocol version is outside the supported range.
    pub const VERSION_MISMATCH: u32 = 1;
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn current_and_min_versions_are_compatible() {
        assert!(version_compatible(PROTOCOL_VERSION));
        assert!(version_compatible(MIN_PROTOCOL_VERSION));
    }

    #[test]
    fn out_of_range_versions_are_rejected() {
        assert!(!version_compatible(MIN_PROTOCOL_VERSION - 1));
        assert!(!version_compatible(PROTOCOL_VERSION + 1));
        assert!(!version_compatible(u32::MAX));
    }
}
