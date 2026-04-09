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
}

// ── Display channel (server → client, unidirectional) ───────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DamageRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// zstd-compressed XOR diff pixel data for this region.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameUpdate {
    pub sequence: u64,
    pub regions: Vec<DamageRegion>,
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
