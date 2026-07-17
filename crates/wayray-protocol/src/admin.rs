//! Compositor admin control protocol.
//!
//! Defines the JSON-over-Unix-socket messages exchanged between the WayRay
//! compositor (wrsrvd) and the administration CLI (wradm). Unlike the launcher
//! protocol, which talks to wrsessd about *managed processes*, this channel
//! inspects the compositor's own session registry.
//!
//! Wire format matches the launcher protocol: each message is a single JSON
//! line (newline-delimited) over a Unix domain socket.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Default path of the wrsrvd admin control socket.
pub fn default_admin_socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("wrsrvd-admin.sock")
}

/// How many leading characters of a session token are exposed over the admin
/// channel. The token is the sole session credential; the prefix is enough to
/// correlate with launcher-side listings without being replayable.
const TOKEN_PREFIX_LEN: usize = 8;

/// Redact a session token to a short prefix for display. The full token is a
/// bearer credential and must never leave the compositor via admin queries.
pub fn redact_token(token: &str) -> String {
    let prefix: String = token.chars().take(TOKEN_PREFIX_LEN).collect();
    if token.chars().count() > TOKEN_PREFIX_LEN {
        format!("{prefix}\u{2026}")
    } else {
        prefix
    }
}

/// Requests sent from wradm to the compositor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AdminRequest {
    /// List all sessions in the compositor's session registry.
    #[serde(rename = "list_sessions")]
    ListSessions,

    /// Report overall server status.
    #[serde(rename = "server_status")]
    ServerStatus,
}

/// Responses sent from the compositor back to wradm.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AdminResponse {
    /// All sessions known to the registry.
    #[serde(rename = "session_list")]
    SessionList { sessions: Vec<SessionEntry> },

    /// Overall server status.
    #[serde(rename = "server_status")]
    ServerStatus(ServerStatus),

    /// The request could not be served.
    #[serde(rename = "error")]
    Error { message: String },
}

/// One session as seen by the compositor's registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    /// Compositor-assigned session id.
    pub id: u64,
    /// Redacted token prefix (see [`redact_token`]); never the full credential.
    pub token_prefix: String,
    /// Authenticated user, when known.
    pub user: Option<String>,
    /// Lifecycle state (`creating`, `active`, `suspended`, `destroyed`).
    pub state: String,
    /// Unix epoch seconds when the session was created (approximate; derived
    /// from the session's monotonic age at snapshot time).
    pub created_at_epoch_secs: u64,
    /// Unix epoch seconds of the session's last activity (state-derived:
    /// now for active sessions, suspension time for suspended ones).
    pub last_active_epoch_secs: u64,
    /// Seconds since the session was created.
    pub uptime_secs: u64,
    /// Remote address of the connected client endpoint, if one is attached.
    pub client_addr: Option<String>,
}

/// Session counts broken down by lifecycle state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionCounts {
    pub creating: usize,
    pub active: usize,
    pub suspended: usize,
    /// Destroyed but not yet purged from memory.
    pub destroyed: usize,
}

/// Overall server status report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
    /// wrsrvd crate version.
    pub version: String,
    /// Seconds since the compositor entered its main loop.
    pub uptime_secs: u64,
    /// Session counts by state.
    pub sessions: SessionCounts,
    /// Number of configured cluster peers (0 in single-server mode).
    pub cluster_peers: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_socket_path_is_reasonable() {
        let path = default_admin_socket_path();
        assert!(path.to_str().unwrap().contains("wrsrvd-admin"));
    }

    #[test]
    fn redact_token_truncates_long_tokens() {
        let redacted = redact_token("0123456789abcdef");
        assert_eq!(redacted, "01234567\u{2026}");
        // The redacted form must not contain the full token.
        assert!(!redacted.contains("89abcdef"));
    }

    #[test]
    fn redact_token_keeps_short_tokens() {
        assert_eq!(redact_token("abc"), "abc");
        assert_eq!(redact_token(""), "");
    }

    #[test]
    fn roundtrip_admin_request() {
        for req in [AdminRequest::ListSessions, AdminRequest::ServerStatus] {
            let json = serde_json::to_string(&req).unwrap();
            let parsed: AdminRequest = serde_json::from_str(&json).unwrap();
            assert_eq!(
                std::mem::discriminant(&req),
                std::mem::discriminant(&parsed)
            );
        }
    }

    #[test]
    fn roundtrip_session_list() {
        let resp = AdminResponse::SessionList {
            sessions: vec![SessionEntry {
                id: 7,
                token_prefix: "abcd1234\u{2026}".to_string(),
                user: Some("alice".to_string()),
                state: "active".to_string(),
                created_at_epoch_secs: 1_700_000_000,
                last_active_epoch_secs: 1_700_000_100,
                uptime_secs: 100,
                client_addr: Some("192.0.2.1:4433".to_string()),
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"session_list\""));
        let parsed: AdminResponse = serde_json::from_str(&json).unwrap();
        match parsed {
            AdminResponse::SessionList { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].id, 7);
                assert_eq!(sessions[0].user.as_deref(), Some("alice"));
            }
            other => panic!("expected SessionList, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_server_status() {
        let resp = AdminResponse::ServerStatus(ServerStatus {
            version: "0.1.0".to_string(),
            uptime_secs: 3600,
            sessions: SessionCounts {
                creating: 0,
                active: 2,
                suspended: 1,
                destroyed: 0,
            },
            cluster_peers: 3,
        });
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"server_status\""));
        let parsed: AdminResponse = serde_json::from_str(&json).unwrap();
        match parsed {
            AdminResponse::ServerStatus(status) => {
                assert_eq!(status.sessions.active, 2);
                assert_eq!(status.cluster_peers, 3);
            }
            other => panic!("expected ServerStatus, got {other:?}"),
        }
    }
}
