//! WayRay wire protocol definitions.
//!
//! Shared between wrsrvd (server) and wrclient (client).
//! Messages are serialized with postcard and framed with a 4-byte
//! length prefix for transmission over QUIC streams.

pub mod cluster;
pub mod codec;
pub mod encoding;
pub mod launcher;
pub mod messages;
pub mod session_config;
pub mod tls;
pub mod transport;

/// Current protocol version. Incremented on breaking changes.
pub const PROTOCOL_VERSION: u32 = 1;
