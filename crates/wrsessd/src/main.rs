//! wrsessd -- WayRay session launcher daemon.
//!
//! Reference implementation of the session launcher interface. Listens on
//! a Unix socket for events from the WayRay compositor:
//!
//! - `session_requested`: Start a greeter for the new session
//! - `session_authenticated`: Launch the user's desktop from session.toml
//! - `session_logout`: Clean up child processes
//!
//! This is a reference implementation. Production deployments may replace
//! it with a custom launcher that integrates PAM, LDAP, NFS mounts, etc.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

use miette::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::process::{Child, Command};
use tracing::{error, info, warn};
use wayray_protocol::launcher::{LauncherRequest, LauncherResponse, SessionInfo};
use wayray_protocol::session_config::SessionConfig;

/// Tracked state for an active session.
struct ManagedSession {
    token: String,
    user: Option<String>,
    /// Child processes launched for this session.
    children: Vec<Child>,
    /// The Wayland display socket name.
    wayland_display: String,
}

/// Session launcher state.
struct Launcher {
    sessions: HashMap<String, ManagedSession>,
    /// Path to the greeter binary.
    greeter_bin: String,
}

impl Launcher {
    fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            greeter_bin: "wrlogin".to_string(),
        }
    }

    /// Handle a session_requested event: launch the greeter.
    async fn handle_session_requested(
        &mut self,
        token: String,
        wayland_display: String,
    ) -> LauncherResponse {
        info!(%token, %wayland_display, "session requested, launching greeter");

        let mut session = ManagedSession {
            token: token.clone(),
            user: None,
            children: Vec::new(),
            wayland_display: wayland_display.clone(),
        };

        // Launch the greeter as a Wayland client.
        match Command::new(&self.greeter_bin)
            .env("WAYLAND_DISPLAY", &wayland_display)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(child) => {
                session.children.push(child);
                self.sessions.insert(token.clone(), session);
                LauncherResponse::SessionReady { token }
            }
            Err(e) => {
                warn!(error = %e, greeter = %self.greeter_bin, "failed to launch greeter");
                // Still register the session even if greeter fails.
                self.sessions.insert(token.clone(), session);
                LauncherResponse::Error {
                    token,
                    message: format!("greeter launch failed: {e}"),
                }
            }
        }
    }

    /// Handle session_authenticated: launch the user's desktop.
    async fn handle_session_authenticated(
        &mut self,
        token: String,
        user: String,
    ) -> LauncherResponse {
        let Some(session) = self.sessions.get_mut(&token) else {
            return LauncherResponse::Error {
                token,
                message: "no session found for token".to_string(),
            };
        };

        session.user = Some(user.clone());
        info!(%token, %user, "session authenticated, launching desktop");

        // Load the user's session config.
        let config = SessionConfig::load_default();

        // Launch each component from the session config.
        for (binary, is_wm) in config.launch_list() {
            match Command::new(binary)
                .env("WAYLAND_DISPLAY", &session.wayland_display)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
            {
                Ok(child) => {
                    info!(binary, is_wm, "launched session component");
                    session.children.push(child);
                }
                Err(e) => {
                    warn!(binary, error = %e, "failed to launch session component");
                }
            }
        }

        LauncherResponse::DesktopStarted { token, user }
    }

    /// Handle session_logout: kill all child processes.
    async fn handle_session_logout(&mut self, token: String, _session_id: u64) {
        if let Some(mut session) = self.sessions.remove(&token) {
            info!(%token, "session logout, cleaning up");
            for child in &mut session.children {
                let _ = child.kill().await;
            }
        }
    }

    /// Admin: list all managed sessions.
    fn list_sessions(&self) -> LauncherResponse {
        let sessions = self
            .sessions
            .values()
            .map(|s| SessionInfo {
                token: s.token.clone(),
                user: s.user.clone(),
                child_count: s.children.len(),
            })
            .collect();
        LauncherResponse::SessionList { sessions }
    }

    /// Admin: kill a session by token.
    async fn kill_session(&mut self, token: String) -> LauncherResponse {
        if let Some(mut session) = self.sessions.remove(&token) {
            info!(%token, "admin: killing session");
            for child in &mut session.children {
                let _ = child.kill().await;
            }
            LauncherResponse::SessionKilled { token }
        } else {
            LauncherResponse::Error {
                token,
                message: "session not found".to_string(),
            }
        }
    }
}

/// Default IPC path for the launcher (uses shared transport default).
fn default_socket_path() -> PathBuf {
    wayray_protocol::transport::default_ipc_path()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let socket_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_socket_path);

    // Remove stale socket file if it exists.
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).map_err(|e| {
        miette::miette!(
            "failed to bind launcher socket at {}: {}",
            socket_path.display(),
            e
        )
    })?;

    info!(socket = %socket_path.display(), "wrsessd listening");

    let mut launcher = Launcher::new();

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|e| miette::miette!("failed to accept connection: {}", e))?;

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        // Read JSON lines from the client (wrsrvd).
        while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                line.clear();
                continue;
            }

            match serde_json::from_str::<LauncherRequest>(trimmed) {
                Ok(request) => {
                    let response = match request {
                        LauncherRequest::SessionRequested {
                            token,
                            wayland_display,
                        } => {
                            launcher
                                .handle_session_requested(token, wayland_display)
                                .await
                        }
                        LauncherRequest::SessionAuthenticated { token, user } => {
                            launcher.handle_session_authenticated(token, user).await
                        }
                        LauncherRequest::SessionLogout { token, session_id } => {
                            launcher.handle_session_logout(token, session_id).await;
                            // No response for logout.
                            line.clear();
                            continue;
                        }
                        LauncherRequest::ListSessions => launcher.list_sessions(),
                        LauncherRequest::KillSession { token } => {
                            launcher.kill_session(token).await
                        }
                    };

                    let mut resp_json = serde_json::to_string(&response).unwrap();
                    resp_json.push('\n');
                    if let Err(e) = writer.write_all(resp_json.as_bytes()).await {
                        warn!(error = %e, "failed to send response");
                        break;
                    }
                }
                Err(e) => {
                    error!(error = %e, line = trimmed, "failed to parse launcher request");
                }
            }

            line.clear();
        }
    }
}
