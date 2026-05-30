//! IPC transport abstraction for the launcher protocol.
//!
//! Provides platform-specific transports for request/response communication
//! between WayRay components (wrsessd, wradm, wrlogin, wrsrvd).
//!
//! - **Linux / default**: Unix domain sockets (synchronous, blocking I/O)
//! - **illumos**: Doors IPC (synchronous, high-speed RPC) with the `doors` feature
//!
//! Both the client ([`send_request_sync`]) and the server ([`serve`]) select the
//! same transport, and both are synchronous, sharing one [`RequestHandler`] core.
//!
//! The transport moves JSON bytes — serialization is handled by the caller
//! using [`LauncherRequest`] and [`LauncherResponse`].

use std::io;
use std::path::{Path, PathBuf};

use crate::launcher::{LauncherRequest, LauncherResponse};

/// Default IPC endpoint path.
///
/// On illumos with doors enabled, this is a door file.
/// On Linux, this is a Unix socket.
pub fn default_ipc_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("wayray-launcher.sock")
}

/// Send a request and receive a response (synchronous, blocking).
///
/// Automatically selects the transport based on the platform:
/// - illumos with `doors` feature: uses doors IPC
/// - everywhere else: uses Unix socket with blocking I/O
pub fn send_request_sync(path: &Path, request: &LauncherRequest) -> io::Result<LauncherResponse> {
    #[cfg(all(target_os = "illumos", feature = "doors"))]
    {
        doors_transport::send_request(path, request)
    }

    #[cfg(not(all(target_os = "illumos", feature = "doors")))]
    {
        unix_transport::send_request(path, request)
    }
}

/// Synchronous handler for launcher requests, shared by every server transport.
///
/// Returning `None` means "no response" (e.g. a logout notification). Keeping
/// the handler synchronous lets the doors server — whose procedure is a plain
/// function invoked from the doors thread pool — and the Unix-socket server use
/// the exact same request-processing core, so the two transports can never
/// drift apart (the bug this fixes: a doors *client* with no doors *server*).
pub trait RequestHandler: Send + 'static {
    /// Process one request, optionally producing a response.
    fn handle(&mut self, request: LauncherRequest) -> Option<LauncherResponse>;
}

/// Serve launcher requests at `path` until the process exits, using the
/// platform-appropriate transport (doors on illumos with the `doors` feature,
/// Unix socket otherwise). This is the server counterpart of
/// [`send_request_sync`]: both ends select the same transport, so a doors
/// client always has a doors server to talk to.
pub fn serve(path: &Path, handler: Box<dyn RequestHandler>) -> io::Result<()> {
    #[cfg(all(target_os = "illumos", feature = "doors"))]
    {
        doors_transport::serve(path, handler)
    }

    #[cfg(not(all(target_os = "illumos", feature = "doors")))]
    {
        unix_transport::serve(path, handler)
    }
}

// =============================================================================
// Unix socket transport (all platforms, used as default)
// =============================================================================

pub mod unix_transport {
    use std::io::{self, BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;

    use crate::launcher::{LauncherRequest, LauncherResponse};

    use super::RequestHandler;

    /// Send a request over a Unix socket and read the response.
    pub fn send_request(path: &Path, request: &LauncherRequest) -> io::Result<LauncherResponse> {
        let stream = UnixStream::connect(path)?;
        let mut writer = io::BufWriter::new(&stream);
        let mut reader = BufReader::new(&stream);

        let mut json = serde_json::to_string(request)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        json.push('\n');
        writer.write_all(json.as_bytes())?;
        writer.flush()?;

        let mut response_line = String::new();
        reader.read_line(&mut response_line)?;

        serde_json::from_str(response_line.trim())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Serve newline-delimited JSON requests over a Unix socket, dispatching
    /// each to `handler`. Connections are handled sequentially (the launcher is
    /// low-traffic); each request line yields at most one response line.
    pub fn serve(path: &Path, mut handler: Box<dyn RequestHandler>) -> io::Result<()> {
        // Remove any stale socket file from a previous run.
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;

        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to accept launcher connection");
                    continue;
                }
            };
            if let Err(e) = serve_connection(&stream, handler.as_mut()) {
                tracing::warn!(error = %e, "launcher connection ended with error");
            }
        }
        Ok(())
    }

    /// Handle one client connection: read request lines until EOF.
    fn serve_connection(stream: &UnixStream, handler: &mut dyn RequestHandler) -> io::Result<()> {
        let mut reader = BufReader::new(stream);
        let mut writer = io::BufWriter::new(stream);
        let mut line = String::new();

        while reader.read_line(&mut line)? > 0 {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                match serde_json::from_str::<LauncherRequest>(trimmed) {
                    Ok(request) => {
                        if let Some(response) = handler.handle(request) {
                            let mut json = serde_json::to_string(&response)
                                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                            json.push('\n');
                            writer.write_all(json.as_bytes())?;
                            writer.flush()?;
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, line = trimmed, "failed to parse launcher request");
                    }
                }
            }
            line.clear();
        }
        Ok(())
    }
}

// =============================================================================
// Doors transport (illumos only)
// =============================================================================

#[cfg(all(target_os = "illumos", feature = "doors"))]
pub mod doors_transport {
    use std::io;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    use crate::launcher::{LauncherRequest, LauncherResponse};

    use super::RequestHandler;

    /// Send a request via illumos doors IPC.
    ///
    /// Doors are synchronous RPC: call_with_data sends bytes and blocks
    /// until the server procedure returns a response.
    pub fn send_request(path: &Path, request: &LauncherRequest) -> io::Result<LauncherResponse> {
        let json = serde_json::to_vec(request)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let client = doors::Client::open(path)
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, format!("{e:?}")))?;

        let response_bytes = client
            .call_with_data(&json)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e:?}")))?;

        serde_json::from_slice(&response_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// The door procedure is a plain function invoked by the doors thread pool
    /// and cannot capture state, so the active handler is stored in a
    /// process-global slot installed by [`serve`].
    static HANDLER: OnceLock<Mutex<Box<dyn RequestHandler>>> = OnceLock::new();

    /// Server door procedure: decode the request JSON, dispatch to the installed
    /// handler, and return the response JSON (empty when there is no response).
    #[doors::server_procedure]
    fn launcher_door(request: doors::server::Request) -> doors::server::Response<Vec<u8>> {
        let response = serde_json::from_slice::<LauncherRequest>(&request.data)
            .ok()
            .and_then(|req| {
                HANDLER
                    .get()
                    .and_then(|h| h.lock().ok())
                    .and_then(|mut h| h.handle(req))
            });
        let bytes = response
            .and_then(|r| serde_json::to_vec(&r).ok())
            .unwrap_or_default();
        doors::server::Response::new(bytes)
    }

    /// Install a door at `path` serving `handler`, then block forever while the
    /// doors thread pool services calls. Mirrors [`super::unix_transport::serve`]
    /// so the same synchronous handler backs both transports.
    pub fn serve(path: &Path, handler: Box<dyn RequestHandler>) -> io::Result<()> {
        HANDLER.set(Mutex::new(handler)).map_err(|_| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "doors handler already installed",
            )
        })?;

        let door = doors::server::Door::create(launcher_door)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e:?}")))?;
        door.force_install(path)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{e:?}")))?;

        tracing::info!(path = %path.display(), "doors launcher server installed");
        // The door is serviced by its own thread pool; park this thread.
        loop {
            std::thread::park();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::LauncherResponse;

    #[test]
    fn default_path_is_reasonable() {
        let path = default_ipc_path();
        assert!(path.to_str().unwrap().contains("wayray-launcher"));
    }

    /// The Unix-socket server and `send_request_sync` client agree end-to-end:
    /// a request reaches the handler and its response comes back. This is the
    /// same handler the doors server uses, so both transports are symmetric.
    #[test]
    fn unix_serve_and_client_round_trip() {
        struct EchoHandler;
        impl RequestHandler for EchoHandler {
            fn handle(&mut self, request: LauncherRequest) -> Option<LauncherResponse> {
                match request {
                    LauncherRequest::SessionRequested { token, .. } => {
                        Some(LauncherResponse::SessionReady { token })
                    }
                    LauncherRequest::SessionLogout { .. } => None,
                    other => Some(LauncherResponse::Error {
                        token: String::new(),
                        message: format!("unexpected: {other:?}"),
                    }),
                }
            }
        }

        let path =
            std::env::temp_dir().join(format!("wayray-serve-test-{}.sock", std::process::id()));
        let server_path = path.clone();
        let server = std::thread::spawn(move || {
            // serve() blocks; the client closes the listener by dropping its
            // connection and the test ends when the thread is detached. We run
            // a single accept here by binding directly.
            let _ = unix_transport::serve(&server_path, Box::new(EchoHandler));
        });

        // Wait for the socket to appear.
        for _ in 0..100 {
            if path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let resp = send_request_sync(
            &path,
            &LauncherRequest::SessionRequested {
                token: "tok-123".to_string(),
                wayland_display: "wayland-1".to_string(),
            },
        )
        .expect("request should succeed");
        match resp {
            LauncherResponse::SessionReady { token } => assert_eq!(token, "tok-123"),
            other => panic!("expected SessionReady, got {other:?}"),
        }

        let _ = std::fs::remove_file(&path);
        drop(server); // detach; process exit reclaims the parked thread
    }
}
