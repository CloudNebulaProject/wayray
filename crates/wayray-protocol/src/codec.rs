//! Length-prefixed message framing for QUIC streams.
//!
//! Each message is encoded as: 4-byte LE length prefix + postcard payload.

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("serialization failed: {0}")]
    Serialize(#[from] postcard::Error),

    #[error("incomplete message: need {needed} bytes, have {available}")]
    Incomplete { needed: usize, available: usize },
}

/// Encode a message to a length-prefixed byte vector.
///
/// Format: [4-byte LE length][postcard payload]
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, CodecError> {
    let payload = postcard::to_allocvec(msg)?;
    let len = (payload.len() as u32).to_le_bytes();
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Decode a message from a payload byte slice (after length prefix is removed).
pub fn decode<T: DeserializeOwned>(data: &[u8]) -> Result<T, CodecError> {
    Ok(postcard::from_bytes(data)?)
}

/// Read a length prefix from a byte slice.
///
/// Returns `Some((length, remaining_slice))` if at least 4 bytes are available,
/// `None` otherwise.
pub fn read_length_prefix(data: &[u8]) -> Option<(u32, &[u8])> {
    if data.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    Some((len, &data[4..]))
}

/// Try to extract a complete framed message from a buffer.
///
/// Returns `Some((message, bytes_consumed))` if a complete message is available,
/// `None` if more data is needed.
pub fn try_decode_framed<T: DeserializeOwned>(
    buf: &[u8],
) -> Result<Option<(T, usize)>, CodecError> {
    let Some((len, rest)) = read_length_prefix(buf) else {
        return Ok(None);
    };
    let len = len as usize;
    if rest.len() < len {
        return Ok(None);
    }
    let msg = decode(&rest[..len])?;
    Ok(Some((msg, 4 + len)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::*;

    #[test]
    fn control_message_roundtrip() {
        let msg = ControlMessage::ClientHello(ClientHello {
            version: 1,
            capabilities: vec!["display".to_string()],
            token: Some("test".to_string()),
        });
        let encoded = encode(&msg).unwrap();
        let (len, payload) = read_length_prefix(&encoded).unwrap();
        let decoded: ControlMessage = decode(&payload[..len as usize]).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn display_message_roundtrip() {
        let msg = DisplayMessage::FrameUpdate(FrameUpdate {
            sequence: 42,
            regions: vec![DamageRegion {
                x: 10,
                y: 20,
                width: 100,
                height: 50,
                data: vec![0, 1, 2, 3],
            }],
        });
        let encoded = encode(&msg).unwrap();
        let (len, payload) = read_length_prefix(&encoded).unwrap();
        let decoded: DisplayMessage = decode(&payload[..len as usize]).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn input_message_roundtrip() {
        let msg = InputMessage::Keyboard(KeyboardEvent {
            keycode: 38,
            state: KeyState::Pressed,
            time: 12345,
        });
        let encoded = encode(&msg).unwrap();
        let (len, payload) = read_length_prefix(&encoded).unwrap();
        let decoded: InputMessage = decode(&payload[..len as usize]).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn try_decode_framed_complete() {
        let msg = ControlMessage::Ping(Ping { timestamp: 999 });
        let encoded = encode(&msg).unwrap();
        let result: Option<(ControlMessage, usize)> = try_decode_framed(&encoded).unwrap();
        let (decoded, consumed) = result.unwrap();
        assert_eq!(decoded, msg);
        assert_eq!(consumed, encoded.len());
    }

    #[test]
    fn try_decode_framed_incomplete() {
        let msg = ControlMessage::Ping(Ping { timestamp: 999 });
        let encoded = encode(&msg).unwrap();
        // Only provide half the data
        let partial = &encoded[..encoded.len() / 2];
        let result: Option<(ControlMessage, usize)> = try_decode_framed(partial).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn empty_frame_update() {
        let msg = DisplayMessage::FrameUpdate(FrameUpdate {
            sequence: 0,
            regions: vec![],
        });
        let encoded = encode(&msg).unwrap();
        let (len, payload) = read_length_prefix(&encoded).unwrap();
        let decoded: DisplayMessage = decode(&payload[..len as usize]).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn pointer_motion_roundtrip() {
        let msg = InputMessage::PointerMotion(PointerMotion {
            x: 100.5,
            y: 200.75,
            time: 54321,
        });
        let encoded = encode(&msg).unwrap();
        let (len, payload) = read_length_prefix(&encoded).unwrap();
        let decoded: InputMessage = decode(&payload[..len as usize]).unwrap();
        assert_eq!(decoded, msg);
    }
}
