//! WayRay wire protocol message types.
//!
//! Organized by channel: control (bidirectional), display (server→client),
//! and input (client→server). Serialized with postcard over QUIC streams.

use serde::{Deserialize, Serialize};

// ── Control channel (bidirectional) ─────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientHello {
    pub version: u32,
    pub capabilities: Vec<String>,
    /// Session token for session lookup/creation.
    /// If None, the server creates a new session with a generated token.
    pub token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerHello {
    pub version: u32,
    pub session_id: u64,
    pub output_width: u32,
    pub output_height: u32,
    /// Whether this is a resumed session or a new one.
    pub resumed: bool,
    /// The token bound to this session (echoed back to the client).
    pub token: String,
}

/// Session lifecycle state as seen by the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// New session being created.
    Creating,
    /// Session is active.
    Active,
    /// Session was suspended and is being resumed.
    Resuming,
}

/// Session-related control events sent by the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionEvent {
    /// Session state changed.
    StateChanged {
        session_id: u64,
        status: SessionStatus,
    },
    /// Session is being suspended (client should prepare for disconnect).
    Suspending { session_id: u64 },
    /// Session has been destroyed (client should disconnect).
    Destroyed { session_id: u64 },
    /// Session was resumed on reconnect (hot-desking). The client should
    /// clear any frame cache and expect a full redraw.
    Resumed { session_id: u64 },
    /// Session lives on (or has been moved to) another server. The client
    /// should reconnect to the indicated server with the same token.
    Redirected {
        /// Target server id (matches the cluster config).
        server_id: String,
        /// Target server address as `host:port`.
        addr: String,
    },
}

/// Lightweight server-info query/response used for cross-server load
/// balancing. A new variant on the control channel; appended to keep
/// postcard back-compat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfoMsg {
    /// The responding server's id.
    pub server_id: String,
    /// Number of currently active sessions on the server.
    pub active_sessions: u32,
    /// Maximum number of sessions the server is willing to host.
    pub capacity: u32,
}

/// Response to a [`ControlMessage::SessionLookupRequest`]: whether the queried
/// server currently hosts a resumable session for the given token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLookupResponse {
    /// The responding server's id (so the asker can record the home server).
    pub server_id: String,
    /// `true` if this server hosts a resumable session for the queried token.
    pub hosts: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ping {
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pong {
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameAck {
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlMessage {
    ClientHello(ClientHello),
    ServerHello(ServerHello),
    Ping(Ping),
    Pong(Pong),
    FrameAck(FrameAck),
    SessionEvent(SessionEvent),
    /// Lightweight server-info request used by a peer to probe load before
    /// placing a new session. Carries the cluster shared secret so the server
    /// only answers authenticated peers (not arbitrary network clients).
    ServerInfoRequest {
        /// Cluster shared secret authenticating the probing peer.
        auth: String,
    },
    /// Response to a `ServerInfoRequest`.
    ServerInfo(ServerInfoMsg),
    /// Cross-server affinity probe: a peer asks whether this server hosts a
    /// resumable session for `token`. Used so a server that receives a client
    /// for an unknown token can discover the session's home server and redirect
    /// the client there instead of creating a duplicate.
    ///
    /// Carries the cluster shared secret: answering this for an arbitrary caller
    /// would turn the control channel into a token-existence oracle, so only
    /// authenticated peers are answered.
    SessionLookupRequest {
        /// Session token to look up.
        token: String,
        /// Cluster shared secret authenticating the probing peer.
        auth: String,
    },
    /// Response to a `SessionLookupRequest`.
    SessionLookupResponse(SessionLookupResponse),
    /// Client → server: request a full keyframe. Sent when the client detects
    /// its reconstructed framebuffer has drifted out of sync (a frame-checksum
    /// mismatch), so the server resends a complete frame to resynchronize.
    RequestKeyframe,
    /// Sent in place of (or alongside) `ServerHello` when the client's session
    /// lives on a different server. The client reconnects to `addr`.
    SessionRedirect {
        /// Target server id (matches the cluster config).
        server_id: String,
        /// Target server address as `host:port`.
        addr: String,
    },
}

// ── Display channel (server → client, unidirectional) ───────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DamageRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// zstd-compressed *absolute* BGRA8 pixels for this region (not a diff), so
    /// applying it is an idempotent copy — a missed or out-of-order frame can
    /// never permanently corrupt the reconstruction, only briefly stale a
    /// region until it is redrawn or a keyframe arrives.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameUpdate {
    pub sequence: u64,
    pub regions: Vec<DamageRegion>,
    /// Checksum of the server's *full* framebuffer after this frame. The client
    /// recomputes it over its reconstructed framebuffer and, on mismatch,
    /// requests a keyframe — detecting and recovering from any drift.
    pub checksum: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DisplayMessage {
    FrameUpdate(FrameUpdate),
}

// ── Input channel (client → server, unidirectional) ─────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyboardEvent {
    pub keycode: u32,
    pub state: KeyState,
    pub time: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointerMotion {
    pub x: f64,
    pub y: f64,
    pub time: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointerButton {
    pub button: u32,
    pub state: ButtonState,
    pub time: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointerAxis {
    pub axis: Axis,
    pub value: f64,
    pub time: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputMessage {
    Keyboard(KeyboardEvent),
    PointerMotion(PointerMotion),
    PointerButton(PointerButton),
    PointerAxis(PointerAxis),
}
