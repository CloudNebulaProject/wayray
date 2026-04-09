//! IPC transport abstraction for the launcher protocol.
//!
//! Provides platform-specific transports for request/response communication
//! between WayRay components (wrsessd, wradm, wrlogin, wrsrvd).
//!
//! - **Linux**: Unix domain sockets (async, via tokio)
//! - **illumos**: Doors IPC (sync, high-speed RPC) with Unix socket fallback
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

// =============================================================================
// Unix socket transport (all platforms, used as default)
// =============================================================================

pub mod unix_transport {
    use std::io::{self, BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    use crate::launcher::{LauncherRequest, LauncherResponse};

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
}

// =============================================================================
// Doors transport (illumos only)
// =============================================================================

#[cfg(all(target_os = "illumos", feature = "doors"))]
pub mod doors_transport {
    use std::io;
    use std::path::Path;

    use crate::launcher::{LauncherRequest, LauncherResponse};

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_is_reasonable() {
        let path = default_ipc_path();
        assert!(path.to_str().unwrap().contains("wayray-launcher"));
    }
}
