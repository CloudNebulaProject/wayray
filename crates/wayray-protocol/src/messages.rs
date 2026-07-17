//! WayRay wire protocol message types.
//!
//! Organized by channel: control (bidirectional), display (server→client),
//! and input (client→server). Serialized with postcard over QUIC streams.

use serde::{Deserialize, Serialize};

/// Well-known capability names a client may advertise in [`ClientHello`].
/// The server only opens streams / forwards messages for capabilities the
/// client advertised.
pub mod caps {
    /// The client renders display frames (server opens the display stream).
    pub const DISPLAY: &str = "display";
    /// The client handles clipboard sync control messages.
    pub const CLIPBOARD: &str = "clipboard";
    /// The client accepts the dedicated audio stream.
    pub const AUDIO: &str = "audio";
}

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

/// Machine-readable class of a fatal [`ProtocolError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolErrorCode {
    /// The peer's protocol version is outside the supported range.
    VersionMismatch,
    /// Any other fatal protocol violation.
    Other,
}

/// Fatal protocol-level error sent on the control stream right before the
/// sender closes the connection. Gives the peer a typed, human-readable
/// explanation instead of a bare EOF.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ProtocolErrorCode,
    /// Human-readable description, suitable for surfacing to the user.
    pub reason: String,
}

// ── Clipboard sync (control channel, both directions) ───────────────

/// Maximum clipboard payload carried in a [`ClipboardData`] message. Larger
/// selections are truncated (see [`cap_clipboard_payload`]) — the clipboard
/// channel shares the control stream and must never starve the handshake or
/// acks behind a multi-hundred-megabyte selection.
pub const MAX_CLIPBOARD_DATA: usize = 1024 * 1024;

/// Announces that a new selection with these mime types is available on the
/// sending side. Sent by the server before [`ClipboardData`] when a Wayland
/// application takes the selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardOffer {
    pub mime_types: Vec<String>,
}

/// Clipboard payload in a single mime type. Server→client: pushed when a
/// Wayland app sets a selection. Client→server: pushed when the local OS
/// clipboard changed (captured on window focus gain).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardData {
    pub mime_type: String,
    /// Raw payload, at most [`MAX_CLIPBOARD_DATA`] bytes (truncated beyond).
    pub data: Vec<u8>,
}

/// Pick the mime type to transfer from an offered set, preferring text.
///
/// Preference order: UTF-8 plain text, legacy X11 UTF-8 string, bare plain
/// text, any other `text/*` type, then the first offered type as a fallback.
pub fn preferred_mime_type(offered: &[String]) -> Option<&str> {
    const PREFERRED: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
    for want in PREFERRED {
        if let Some(m) = offered.iter().find(|m| m.eq_ignore_ascii_case(want)) {
            return Some(m);
        }
    }
    offered
        .iter()
        .find(|m| m.to_ascii_lowercase().starts_with("text/"))
        .or_else(|| offered.first())
        .map(String::as_str)
}

/// Whether a mime type carries (UTF-8) text.
pub fn is_text_mime(mime: &str) -> bool {
    let lower = mime.to_ascii_lowercase();
    lower.starts_with("text/") || lower == "utf8_string" || lower == "string" || lower == "text"
}

/// The mime types to advertise when re-offering a text selection, so
/// applications that ask for any common text flavor can paste. Non-text mime
/// types are offered as-is.
pub fn clipboard_offer_mimes(mime: &str) -> Vec<String> {
    if is_text_mime(mime) {
        let mut mimes = vec![
            "text/plain;charset=utf-8".to_string(),
            "text/plain".to_string(),
            "UTF8_STRING".to_string(),
            "STRING".to_string(),
            "TEXT".to_string(),
        ];
        if !mimes.iter().any(|m| m.eq_ignore_ascii_case(mime)) {
            mimes.insert(0, mime.to_string());
        }
        mimes
    } else {
        vec![mime.to_string()]
    }
}

/// Enforce [`MAX_CLIPBOARD_DATA`] on a clipboard payload, truncating in place.
/// For text mime types the cut backs off to a UTF-8 character boundary so the
/// truncated payload stays valid text. Returns `true` if data was truncated —
/// callers should emit a tracing warning so the loss is observable.
pub fn cap_clipboard_payload(data: &mut Vec<u8>, mime: &str) -> bool {
    if data.len() <= MAX_CLIPBOARD_DATA {
        return false;
    }
    data.truncate(MAX_CLIPBOARD_DATA);
    if is_text_mime(mime) {
        // Pop UTF-8 continuation bytes (0b10xxxxxx) plus the leading byte of a
        // split multi-byte sequence.
        while let Some(&last) = data.last() {
            if last & 0xC0 == 0x80 {
                data.pop();
            } else {
                if last >= 0x80 {
                    data.pop(); // leading byte of a now-incomplete sequence
                }
                break;
            }
        }
    }
    true
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
    /// Fatal protocol error (e.g. an incompatible protocol version in the
    /// `ClientHello`). The sender closes the connection right after this
    /// message; the receiver should surface `reason` to the user and not
    /// retry with the same parameters. Appended to keep postcard back-compat.
    ProtocolError(ProtocolError),
    /// Server → client: a Wayland application took the selection with these
    /// mime types; a `ClipboardData` with the preferred type follows. Only
    /// sent when the client advertised the `clipboard` capability.
    ClipboardOffer(ClipboardOffer),
    /// Clipboard payload (both directions). Server → client: forwarded
    /// selection to place on the local OS clipboard. Client → server: local
    /// clipboard contents for the compositor to re-offer as the Wayland
    /// selection. Only sent to protocol-version-2+ peers.
    ClipboardData(ClipboardData),
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

// ── Audio channel (server → client, dedicated stream) ───────────────
//
// Carried on a dedicated *server-initiated bidirectional* QUIC stream, opened
// only when the client advertised the `audio` capability. A bidi stream (the
// only server-initiated one) is used instead of a second unidirectional
// stream so the client can tell it apart from the display stream without
// relying on stream-ID ordering, and so a future backend has a return path
// for audio control (volume, device selection) without a protocol change.
// Wire-format plumbing only for now: no audio backend exists yet.

/// Audio payload codec. Wire-format reservation; no encoder/decoder ships yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCodec {
    /// Uncompressed interleaved signed 16-bit little-endian PCM.
    PcmS16Le,
    /// Opus-compressed frames.
    Opus,
}

/// Declares (or re-declares) the audio stream format. The most recent
/// `AudioStart` on the stream is authoritative for subsequent chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioStart {
    pub sample_rate: u32,
    pub channels: u8,
    pub codec: AudioCodec,
}

/// A timestamped chunk of encoded audio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioChunk {
    /// Presentation timestamp in microseconds since the matching `AudioStart`.
    pub pts_micros: u64,
    /// Encoded payload in the codec declared by the last `AudioStart`.
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioMessage {
    Start(AudioStart),
    Chunk(AudioChunk),
    /// Playback stopped; the stream stays open for a later `Start`.
    Stop,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode, encode, read_length_prefix};

    fn roundtrip<T>(msg: &T) -> T
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let encoded = encode(msg).unwrap();
        let (len, payload) = read_length_prefix(&encoded).unwrap();
        decode(&payload[..len as usize]).unwrap()
    }

    #[test]
    fn protocol_error_roundtrip() {
        let msg = ControlMessage::ProtocolError(ProtocolError {
            code: ProtocolErrorCode::VersionMismatch,
            reason: "client version 0 unsupported (need 1..=2)".to_string(),
        });
        assert_eq!(roundtrip(&msg), msg);
    }

    #[test]
    fn clipboard_offer_roundtrip() {
        let msg = ControlMessage::ClipboardOffer(ClipboardOffer {
            mime_types: vec!["text/plain;charset=utf-8".to_string(), "TEXT".to_string()],
        });
        assert_eq!(roundtrip(&msg), msg);
    }

    #[test]
    fn clipboard_data_roundtrip() {
        let msg = ControlMessage::ClipboardData(ClipboardData {
            mime_type: "text/plain;charset=utf-8".to_string(),
            data: "grüße von wayray".as_bytes().to_vec(),
        });
        assert_eq!(roundtrip(&msg), msg);
    }

    #[test]
    fn audio_message_roundtrips() {
        let start = AudioMessage::Start(AudioStart {
            sample_rate: 48_000,
            channels: 2,
            codec: AudioCodec::PcmS16Le,
        });
        assert_eq!(roundtrip(&start), start);

        let chunk = AudioMessage::Chunk(AudioChunk {
            pts_micros: 1_234_567,
            payload: vec![0, 1, 2, 3, 255],
        });
        assert_eq!(roundtrip(&chunk), chunk);

        let stop = AudioMessage::Stop;
        assert_eq!(roundtrip(&stop), stop);
    }

    /// New variants are appended, so a v1 peer's messages still decode: the
    /// postcard variant indices of the existing messages must be unchanged.
    #[test]
    fn appended_variants_keep_old_encodings_stable() {
        let old = ControlMessage::RequestKeyframe;
        let encoded = encode(&old).unwrap();
        // Variant index of RequestKeyframe within ControlMessage is 10.
        assert_eq!(encoded[4], 10);
        assert_eq!(roundtrip(&old), old);
    }

    #[test]
    fn mime_preference_favors_utf8_text() {
        let offered: Vec<String> = ["image/png", "text/html", "text/plain;charset=utf-8"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            preferred_mime_type(&offered),
            Some("text/plain;charset=utf-8")
        );
    }

    #[test]
    fn mime_preference_falls_back_to_any_text_then_first() {
        let text_only: Vec<String> = vec!["image/png".into(), "text/html".into()];
        assert_eq!(preferred_mime_type(&text_only), Some("text/html"));

        let no_text: Vec<String> = vec!["image/png".into(), "application/pdf".into()];
        assert_eq!(preferred_mime_type(&no_text), Some("image/png"));

        assert_eq!(preferred_mime_type(&[]), None);
    }

    #[test]
    fn mime_preference_is_case_insensitive() {
        let offered: Vec<String> = vec!["UTF8_STRING".into(), "image/png".into()];
        assert_eq!(preferred_mime_type(&offered), Some("UTF8_STRING"));
    }

    #[test]
    fn offer_mimes_expand_text_and_pass_through_binary() {
        let text = clipboard_offer_mimes("text/plain;charset=utf-8");
        assert!(text.contains(&"text/plain".to_string()));
        assert!(text.contains(&"UTF8_STRING".to_string()));

        assert_eq!(clipboard_offer_mimes("image/png"), vec!["image/png"]);
    }

    #[test]
    fn cap_leaves_small_payloads_alone() {
        let mut data = b"hello".to_vec();
        assert!(!cap_clipboard_payload(&mut data, "text/plain"));
        assert_eq!(data, b"hello");
    }

    #[test]
    fn cap_truncates_oversized_binary_at_exact_limit() {
        let mut data = vec![0xFFu8; MAX_CLIPBOARD_DATA + 100];
        assert!(cap_clipboard_payload(&mut data, "image/png"));
        assert_eq!(data.len(), MAX_CLIPBOARD_DATA);
    }

    #[test]
    fn cap_truncates_text_on_utf8_boundary() {
        // Fill with 3-byte characters so the cap lands mid-sequence.
        let s = "€".repeat(MAX_CLIPBOARD_DATA / 3 + 10);
        let mut data = s.into_bytes();
        assert!(cap_clipboard_payload(&mut data, "text/plain;charset=utf-8"));
        assert!(data.len() <= MAX_CLIPBOARD_DATA);
        assert!(String::from_utf8(data).is_ok());
    }

    #[test]
    fn text_mime_detection() {
        assert!(is_text_mime("text/plain"));
        assert!(is_text_mime("TEXT/PLAIN;charset=utf-8"));
        assert!(is_text_mime("UTF8_STRING"));
        assert!(!is_text_mime("image/png"));
    }
}
