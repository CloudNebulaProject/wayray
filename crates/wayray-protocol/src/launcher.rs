//! Session launcher protocol.
//!
//! Defines the JSON-over-Unix-socket messages exchanged between the
//! WayRay compositor (wrsrvd) and the session launcher daemon (wrsessd).
//!
//! The protocol is intentionally simple: a few event types with JSON
//! serialization. Each message is a single JSON line (newline-delimited).

use serde::{Deserialize, Serialize};

/// Events sent from the compositor to the session launcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LauncherRequest {
    /// A new session is needed for this token.
    /// The launcher should create the user environment and start the greeter.
    #[serde(rename = "session_requested")]
    SessionRequested {
        token: String,
        /// Path to the Wayland display socket (WAYLAND_DISPLAY).
        wayland_display: String,
    },

    /// The user has authenticated successfully.
    /// The launcher should start the user's configured desktop.
    #[serde(rename = "session_authenticated")]
    SessionAuthenticated { token: String, user: String },

    /// The session is being logged out / destroyed.
    /// The launcher should clean up the user environment.
    #[serde(rename = "session_logout")]
    SessionLogout { token: String, session_id: u64 },

    /// Admin: list all managed sessions.
    #[serde(rename = "list_sessions")]
    ListSessions,

    /// Admin: kill a session by token.
    #[serde(rename = "kill_session")]
    KillSession { token: String },
}

/// Responses sent from the session launcher back to the compositor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LauncherResponse {
    /// Session environment is ready; greeter has been launched.
    #[serde(rename = "session_ready")]
    SessionReady { token: String },

    /// User desktop has been started.
    #[serde(rename = "desktop_started")]
    DesktopStarted { token: String, user: String },

    /// An error occurred during session setup.
    #[serde(rename = "error")]
    Error { token: String, message: String },

    /// Admin: list of managed sessions.
    #[serde(rename = "session_list")]
    SessionList { sessions: Vec<SessionInfo> },

    /// Admin: session was killed.
    #[serde(rename = "session_killed")]
    SessionKilled { token: String },
}

/// Session info returned by admin queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub token: String,
    pub user: Option<String>,
    pub child_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_session_requested() {
        let msg = LauncherRequest::SessionRequested {
            token: "abc-123".to_string(),
            wayland_display: "wayland-1".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"session_requested\""));
        assert!(json.contains("\"token\":\"abc-123\""));
    }

    #[test]
    fn roundtrip_launcher_messages() {
        let req = LauncherRequest::SessionAuthenticated {
            token: "tok".to_string(),
            user: "alice".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: LauncherRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            parsed,
            LauncherRequest::SessionAuthenticated { ref user, .. } if user == "alice"
        ));
    }

    #[test]
    fn roundtrip_launcher_response() {
        let resp = LauncherResponse::SessionReady {
            token: "t".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: LauncherResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, LauncherResponse::SessionReady { .. }));
    }
}
