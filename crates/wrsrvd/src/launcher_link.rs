//! Compositor → session-launcher link.
//!
//! When wrsrvd is started with `--launcher-socket <path>` it notifies the
//! session launcher daemon (wrsessd, or a custom replacement) about session
//! lifecycle events so the launcher can spawn the greeter/desktop into the
//! session and tear it down on logout:
//!
//! - session created  → [`LauncherRequest::SessionRequested`] (token + Wayland
//!   display, so the launcher knows where to spawn clients)
//! - session destroyed → [`LauncherRequest::SessionLogout`]
//!
//! Delivery is strictly best-effort: all launcher IO happens on a dedicated
//! worker thread fed by a channel, so a slow or unreachable launcher can never
//! block or crash the compositor. Failures are logged at `warn` and dropped.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use tracing::{info, warn};
use wayray_protocol::launcher::{LauncherRequest, LauncherResponse};
use wayray_protocol::transport;

/// Session lifecycle events forwarded to the launcher.
#[derive(Debug)]
enum LauncherEvent {
    /// A new session was created; the launcher should prepare the environment
    /// and start the greeter on the given Wayland display.
    SessionCreated { token: String },
    /// A session was destroyed; the launcher should clean up its processes.
    SessionDestroyed { token: String, session_id: u64 },
}

/// Handle to the launcher notification worker. Cheap to call from the
/// compositor loop; sends never block on launcher IO.
pub struct LauncherLink {
    tx: mpsc::Sender<LauncherEvent>,
}

impl LauncherLink {
    /// Start the launcher link worker. `socket_path` is the launcher IPC
    /// endpoint (Unix socket, or a door on illumos); `wayland_display` is the
    /// compositor's Wayland socket name, passed to the launcher so it can
    /// spawn clients into this compositor.
    pub fn start(socket_path: PathBuf, wayland_display: String) -> Self {
        let (tx, rx) = mpsc::channel::<LauncherEvent>();

        let spawned = thread::Builder::new()
            .name("wrsrvd-launcher-link".to_string())
            .spawn(move || worker(socket_path, wayland_display, rx));
        if let Err(e) = spawned {
            // Extremely unlikely; the link degrades to a no-op (events are
            // dropped because the receiver never existed).
            warn!(error = %e, "failed to spawn launcher link worker; launcher events disabled");
        }

        Self { tx }
    }

    /// Notify the launcher that a new session was created.
    pub fn session_created(&self, token: String) {
        let _ = self.tx.send(LauncherEvent::SessionCreated { token });
    }

    /// Notify the launcher that a session was destroyed.
    pub fn session_destroyed(&self, token: String, session_id: u64) {
        let _ = self
            .tx
            .send(LauncherEvent::SessionDestroyed { token, session_id });
    }
}

/// Worker loop: deliver each event to the launcher over the shared transport.
/// Runs until the compositor drops its [`LauncherLink`] (channel closes).
fn worker(socket_path: PathBuf, wayland_display: String, rx: mpsc::Receiver<LauncherEvent>) {
    info!(endpoint = %socket_path.display(), "launcher link active");

    while let Ok(event) = rx.recv() {
        match event {
            LauncherEvent::SessionCreated { token } => {
                let request = LauncherRequest::SessionRequested {
                    token,
                    wayland_display: wayland_display.clone(),
                };
                // The token is a credential; never logged.
                match transport::send_request_sync(&socket_path, &request) {
                    Ok(LauncherResponse::SessionReady { .. }) => {
                        info!("launcher acknowledged session (greeter starting)");
                    }
                    Ok(LauncherResponse::Error { message, .. }) => {
                        warn!(%message, "launcher failed to prepare session");
                    }
                    Ok(other) => {
                        warn!(?other, "unexpected launcher response to session_requested");
                    }
                    Err(e) => {
                        warn!(error = %e, endpoint = %socket_path.display(),
                            "launcher unreachable; session continues without greeter");
                    }
                }
            }
            LauncherEvent::SessionDestroyed { token, session_id } => {
                let request = LauncherRequest::SessionLogout { token, session_id };
                // Logout is a one-way notification (the launcher sends no
                // response), so do not wait for a reply.
                if let Err(e) = transport::send_notification_sync(&socket_path, &request) {
                    warn!(error = %e, session_id, endpoint = %socket_path.display(),
                        "failed to notify launcher of session logout");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use wayray_protocol::transport::RequestHandler;

    use super::*;

    /// Mock launcher: records every request it receives, answering like
    /// wrsessd (SessionReady for requests, silence for logout).
    struct RecordingLauncher(Arc<Mutex<Vec<LauncherRequest>>>);

    impl RequestHandler for RecordingLauncher {
        fn handle(&mut self, request: LauncherRequest) -> Option<LauncherResponse> {
            let response = match &request {
                LauncherRequest::SessionRequested { token, .. } => {
                    Some(LauncherResponse::SessionReady {
                        token: token.clone(),
                    })
                }
                LauncherRequest::SessionLogout { .. } => None,
                other => Some(LauncherResponse::Error {
                    token: String::new(),
                    message: format!("unexpected: {other:?}"),
                }),
            };
            self.0.lock().unwrap().push(request);
            response
        }
    }

    fn wait_for_socket(path: &std::path::Path) {
        for _ in 0..100 {
            if path.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Session create/destroy on the link reach the launcher as the right
    /// protocol events, carrying token, display, and session id.
    #[test]
    fn create_and_destroy_events_reach_mock_launcher() {
        let received: Arc<Mutex<Vec<LauncherRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let path = std::env::temp_dir().join(format!(
            "wayray-launcher-link-test-{}.sock",
            std::process::id()
        ));

        let server_path = path.clone();
        let server_received = received.clone();
        std::thread::spawn(move || {
            let _ = wayray_protocol::transport::serve(
                &server_path,
                Box::new(RecordingLauncher(server_received)),
            );
        });
        wait_for_socket(&path);

        let link = LauncherLink::start(path.clone(), "wayland-7".to_string());
        link.session_created("tok-abc".to_string());
        link.session_destroyed("tok-abc".to_string(), 42);

        // The worker delivers asynchronously; poll until both events landed.
        let mut events = Vec::new();
        for _ in 0..200 {
            events = received.lock().unwrap().clone();
            if events.len() >= 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(events.len(), 2, "expected both events, got {events:?}");

        match &events[0] {
            LauncherRequest::SessionRequested {
                token,
                wayland_display,
            } => {
                assert_eq!(token, "tok-abc");
                assert_eq!(wayland_display, "wayland-7");
            }
            other => panic!("expected SessionRequested first, got {other:?}"),
        }
        match &events[1] {
            LauncherRequest::SessionLogout { token, session_id } => {
                assert_eq!(token, "tok-abc");
                assert_eq!(*session_id, 42);
            }
            other => panic!("expected SessionLogout second, got {other:?}"),
        }

        let _ = std::fs::remove_file(&path);
    }

    /// An unreachable launcher must not panic or block: events are dropped
    /// with a warning and the link keeps accepting notifications.
    #[test]
    fn unreachable_launcher_is_tolerated() {
        let path = std::env::temp_dir().join("wayray-launcher-link-nonexistent.sock");
        let link = LauncherLink::start(path, "wayland-9".to_string());
        link.session_created("tok".to_string());
        link.session_destroyed("tok".to_string(), 1);
        // Give the worker a moment to attempt (and fail) delivery.
        std::thread::sleep(Duration::from_millis(50));
        // Still callable afterwards — the worker survived the IO errors.
        link.session_created("tok2".to_string());
    }
}
