//! Admin control socket for wrsrvd.
//!
//! Serves the [`wayray_protocol::admin`] protocol (JSON lines over a Unix
//! socket) so `wradm sessions` / `wradm status` can inspect the compositor's
//! session registry.
//!
//! Threading model mirrors the network bridge: a dedicated thread owns the
//! socket and blocking IO, and forwards each request over an `mpsc` channel
//! into the compositor's calloop iteration, which answers from the live
//! session registry via a bounded reply channel. The compositor is therefore
//! never blocked on admin IO, and the admin thread never touches compositor
//! state directly.

use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tracing::{info, warn};
use wayray_protocol::admin::{AdminRequest, AdminResponse};
use wayray_protocol::transport::unix_transport;

/// How long the admin thread waits for the compositor loop to answer one
/// request. The loop iterates every ~16ms, so this is generous slack for a
/// briefly-stalled loop (e.g. a bounded peer probe) without hanging wradm.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// One admin request in flight from the socket thread to the compositor.
pub struct AdminQuery {
    pub request: AdminRequest,
    /// Bounded reply slot; the compositor sends exactly one response.
    pub reply: mpsc::SyncSender<AdminResponse>,
}

/// Receiving side of the admin bridge, drained by the compositor loop.
pub struct AdminHandle {
    pub rx: mpsc::Receiver<AdminQuery>,
}

/// Start the admin control socket at `path`. Returns the handle the
/// compositor loop drains. Socket errors after startup are logged and the
/// admin channel simply becomes unavailable — never fatal to the compositor.
pub fn start(path: &Path) -> std::io::Result<AdminHandle> {
    let (tx, rx) = mpsc::channel::<AdminQuery>();
    let socket_path = path.to_path_buf();

    thread::Builder::new()
        .name("wrsrvd-admin".to_string())
        .spawn(move || {
            info!(path = %socket_path.display(), "admin control socket listening");
            let result = unix_transport::serve_json::<AdminRequest, AdminResponse>(
                &socket_path,
                move |request| Some(dispatch(&tx, request)),
            );
            if let Err(e) = result {
                warn!(error = %e, path = %socket_path.display(),
                    "admin control socket failed; admin channel unavailable");
            }
        })?;

    Ok(AdminHandle { rx })
}

/// Forward one request to the compositor loop and wait (bounded) for its
/// answer. Every failure mode degrades to an error response for the client.
fn dispatch(tx: &mpsc::Sender<AdminQuery>, request: AdminRequest) -> AdminResponse {
    let (reply_tx, reply_rx) = mpsc::sync_channel::<AdminResponse>(1);
    if tx
        .send(AdminQuery {
            request,
            reply: reply_tx,
        })
        .is_err()
    {
        return AdminResponse::Error {
            message: "compositor is shutting down".to_string(),
        };
    }
    match reply_rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(response) => response,
        Err(_) => AdminResponse::Error {
            message: "compositor did not answer in time".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for_socket(path: &Path) {
        for _ in 0..100 {
            if path.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Full bridge roundtrip: a client request on the socket reaches the
    /// (mock) compositor loop through the channel, and the loop's response
    /// travels back to the client.
    #[test]
    fn admin_request_bridges_to_handler_and_back() {
        use wayray_protocol::admin::{ServerStatus, SessionCounts, SessionEntry};

        let path =
            std::env::temp_dir().join(format!("wrsrvd-admin-test-{}.sock", std::process::id()));
        let handle = start(&path).expect("admin socket should start");

        // Mock compositor loop: answer queries from a canned registry.
        std::thread::spawn(move || {
            while let Ok(query) = handle.rx.recv() {
                let response = match query.request {
                    AdminRequest::ListSessions => AdminResponse::SessionList {
                        sessions: vec![SessionEntry {
                            id: 1,
                            token_prefix: "deadbeef\u{2026}".to_string(),
                            user: None,
                            state: "active".to_string(),
                            created_at_epoch_secs: 1_700_000_000,
                            last_active_epoch_secs: 1_700_000_000,
                            uptime_secs: 5,
                            client_addr: Some("198.51.100.2:9000".to_string()),
                        }],
                    },
                    AdminRequest::ServerStatus => AdminResponse::ServerStatus(ServerStatus {
                        version: "0.0.0-test".to_string(),
                        uptime_secs: 12,
                        sessions: SessionCounts {
                            active: 1,
                            ..Default::default()
                        },
                        cluster_peers: 2,
                    }),
                };
                let _ = query.reply.send(response);
            }
        });
        wait_for_socket(&path);

        let resp: AdminResponse =
            unix_transport::send_json(&path, &AdminRequest::ListSessions).expect("list roundtrip");
        match resp {
            AdminResponse::SessionList { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].token_prefix, "deadbeef\u{2026}");
                assert_eq!(
                    sessions[0].client_addr.as_deref(),
                    Some("198.51.100.2:9000")
                );
            }
            other => panic!("expected SessionList, got {other:?}"),
        }

        let resp: AdminResponse = unix_transport::send_json(&path, &AdminRequest::ServerStatus)
            .expect("status roundtrip");
        match resp {
            AdminResponse::ServerStatus(status) => {
                assert_eq!(status.version, "0.0.0-test");
                assert_eq!(status.sessions.active, 1);
                assert_eq!(status.cluster_peers, 2);
            }
            other => panic!("expected ServerStatus, got {other:?}"),
        }

        let _ = std::fs::remove_file(&path);
    }

    /// When the compositor side is gone (receiver dropped), the socket still
    /// answers with a structured error instead of hanging or crashing.
    #[test]
    fn dropped_compositor_yields_error_response() {
        let path = std::env::temp_dir().join(format!(
            "wrsrvd-admin-dropped-test-{}.sock",
            std::process::id()
        ));
        let handle = start(&path).expect("admin socket should start");
        drop(handle); // Compositor never drains queries.
        wait_for_socket(&path);

        let resp: AdminResponse =
            unix_transport::send_json(&path, &AdminRequest::ServerStatus).expect("send");
        assert!(matches!(resp, AdminResponse::Error { .. }));

        let _ = std::fs::remove_file(&path);
    }
}
