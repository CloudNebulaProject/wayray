//! wrsessd -- WayRay session launcher daemon.
//!
//! Reference implementation of the session launcher interface. Serves launcher
//! requests from the WayRay compositor over the shared transport (a Unix socket
//! everywhere, or illumos doors when built with `--features doors`):
//!
//! - `session_requested`: Start a greeter for the new session
//! - `session_authenticated`: Launch the user's desktop from session.toml
//! - `session_logout`: Clean up child processes
//!
//! This is a reference implementation. Production deployments may replace
//! it with a custom launcher that integrates PAM, LDAP, NFS mounts, etc.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use miette::Result;
use tracing::{info, warn};
use wayray_protocol::launcher::{LauncherRequest, LauncherResponse, SessionInfo};
use wayray_protocol::session_config::SessionConfig;
use wayray_protocol::transport::RequestHandler;

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
    fn handle_session_requested(
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
    fn handle_session_authenticated(&mut self, token: String, user: String) -> LauncherResponse {
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
    fn handle_session_logout(&mut self, token: String, _session_id: u64) {
        if let Some(mut session) = self.sessions.remove(&token) {
            info!(%token, "session logout, cleaning up");
            for child in &mut session.children {
                let _ = child.kill();
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
    fn kill_session(&mut self, token: String) -> LauncherResponse {
        if let Some(mut session) = self.sessions.remove(&token) {
            info!(%token, "admin: killing session");
            for child in &mut session.children {
                let _ = child.kill();
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

impl RequestHandler for Launcher {
    fn handle(&mut self, request: LauncherRequest) -> Option<LauncherResponse> {
        match request {
            LauncherRequest::SessionRequested {
                token,
                wayland_display,
            } => Some(self.handle_session_requested(token, wayland_display)),
            LauncherRequest::SessionAuthenticated { token, user } => {
                Some(self.handle_session_authenticated(token, user))
            }
            LauncherRequest::SessionLogout { token, session_id } => {
                self.handle_session_logout(token, session_id);
                None // logout has no response
            }
            LauncherRequest::ListSessions => Some(self.list_sessions()),
            LauncherRequest::KillSession { token } => Some(self.kill_session(token)),
        }
    }
}

/// Default IPC path for the launcher (uses shared transport default).
fn default_socket_path() -> PathBuf {
    wayray_protocol::transport::default_ipc_path()
}

fn main() -> Result<()> {
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

    info!(endpoint = %socket_path.display(), "wrsessd listening");

    let launcher = Launcher::new();

    // serve() selects the doors transport on illumos (with `--features doors`)
    // and a Unix socket everywhere else — the same transport the client half
    // (`send_request_sync`) selects, so the two always match.
    wayray_protocol::transport::serve(&socket_path, Box::new(launcher)).map_err(|e| {
        miette::miette!(
            "failed to serve launcher at {}: {}",
            socket_path.display(),
            e
        )
    })
}
