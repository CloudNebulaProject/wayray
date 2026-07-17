//! Local OS clipboard integration for clipboard sync.
//!
//! Server → client: clipboard data forwarded from the compositor is placed on
//! the local OS clipboard via `arboard`. Client → server: the local clipboard
//! is polled on window focus gain and, when its text changed, pushed to the
//! server as a `ClipboardData` control message.
//!
//! Only text is synced: `arboard` is a text/image clipboard API and text is
//! the SunRay-parity use case. Non-text payloads from the server are
//! discarded with a log.

use tracing::{debug, warn};
use wayray_protocol::messages::{ClipboardData, cap_clipboard_payload, is_text_mime};

/// The mime type used for text captured from the local OS clipboard.
const TEXT_MIME: &str = "text/plain;charset=utf-8";

/// Wrapper around the OS clipboard with echo suppression.
///
/// `last_text` remembers the most recent text observed in either direction so
/// that (a) a focus-gain capture does not re-send content the server just
/// pushed to us, and (b) unchanged local content is not re-sent on every
/// focus change.
pub struct LocalClipboard {
    inner: Option<arboard::Clipboard>,
    last_text: Option<String>,
}

impl Default for LocalClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalClipboard {
    /// Open the OS clipboard. On headless or otherwise clipboard-less
    /// endpoints this degrades gracefully: sync is disabled locally but the
    /// protocol side keeps working.
    pub fn new() -> Self {
        let inner = match arboard::Clipboard::new() {
            Ok(c) => Some(c),
            Err(e) => {
                warn!(error = %e, "local OS clipboard unavailable; clipboard sync disabled on this endpoint");
                None
            }
        };
        Self {
            inner,
            last_text: None,
        }
    }

    /// Capture the local clipboard if its text changed since the last
    /// observation. Called on window focus gain — the moment the user is
    /// plausibly bringing fresh content into the session. Returns a
    /// size-capped `ClipboardData` ready to send to the server.
    pub fn capture_if_changed(&mut self) -> Option<ClipboardData> {
        let clipboard = self.inner.as_mut()?;
        let text = match clipboard.get_text() {
            Ok(t) => t,
            // Empty/non-text clipboard is an error in arboard; nothing to do.
            Err(_) => return None,
        };
        if text.is_empty() || self.last_text.as_deref() == Some(text.as_str()) {
            return None;
        }
        self.last_text = Some(text.clone());

        let mut data = text.into_bytes();
        if cap_clipboard_payload(&mut data, TEXT_MIME) {
            warn!(
                bytes = data.len(),
                "local clipboard truncated to the protocol size cap"
            );
        }
        debug!(
            bytes = data.len(),
            "captured local clipboard for the server"
        );
        Some(ClipboardData {
            mime_type: TEXT_MIME.to_string(),
            data,
        })
    }

    /// Place clipboard data received from the server on the local OS
    /// clipboard. Non-text payloads are discarded; the content is recorded
    /// either way so a later focus-gain capture does not echo it back.
    pub fn apply_remote(&mut self, clip: &ClipboardData) {
        if !is_text_mime(&clip.mime_type) {
            debug!(mime_type = %clip.mime_type, "ignoring non-text remote clipboard");
            return;
        }
        let Ok(text) = std::str::from_utf8(&clip.data) else {
            warn!(mime_type = %clip.mime_type, "remote clipboard is not valid UTF-8; discarding");
            return;
        };
        // Record before applying so the echo guard holds even if the OS
        // clipboard write fails or the OS clipboard is unavailable.
        self.last_text = Some(text.to_string());
        if let Some(clipboard) = self.inner.as_mut() {
            match clipboard.set_text(text.to_string()) {
                // Contents are user data — log only the size.
                Ok(()) => debug!(bytes = clip.data.len(), "applied remote clipboard"),
                Err(e) => warn!(error = %e, "failed to set local OS clipboard"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clipboard with no OS backend, for exercising the pure logic.
    fn detached() -> LocalClipboard {
        LocalClipboard {
            inner: None,
            last_text: None,
        }
    }

    #[test]
    fn capture_without_os_clipboard_is_none() {
        let mut clip = detached();
        assert!(clip.capture_if_changed().is_none());
    }

    #[test]
    fn apply_remote_records_text_for_echo_suppression() {
        let mut clip = detached();
        clip.apply_remote(&ClipboardData {
            mime_type: "text/plain;charset=utf-8".to_string(),
            data: b"from the server".to_vec(),
        });
        assert_eq!(clip.last_text.as_deref(), Some("from the server"));
    }

    #[test]
    fn apply_remote_ignores_non_text_and_invalid_utf8() {
        let mut clip = detached();
        clip.apply_remote(&ClipboardData {
            mime_type: "image/png".to_string(),
            data: vec![1, 2, 3],
        });
        assert!(clip.last_text.is_none());

        clip.apply_remote(&ClipboardData {
            mime_type: "text/plain".to_string(),
            data: vec![0xFF, 0xFE],
        });
        assert!(clip.last_text.is_none());
    }
}
