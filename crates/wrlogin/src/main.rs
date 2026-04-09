//! wrlogin -- WayRay reference greeter.
//!
//! A minimal CLI-based login screen that reads credentials from stdin
//! and communicates with the session launcher (wrsessd) to authenticate
//! the user and start their desktop session.
//!
//! This is a reference implementation using terminal I/O. Production
//! greeters would use a graphical Wayland client (GTK, iced, etc.)
//! with the same launcher protocol.
//!
//! ## Usage
//!
//! The session launcher (wrsessd) starts wrlogin as the first client
//! in a new session. wrlogin:
//! 1. Prompts for username and password on the terminal
//! 2. Sends `session_authenticated` to wrsessd via the launcher socket
//! 3. Exits on success, allowing the desktop to start

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use miette::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::info;
use wayray_protocol::launcher::{LauncherRequest, LauncherResponse};

/// Read a line from stdin with a prompt.
fn prompt(message: &str) -> io::Result<String> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(message.as_bytes())?;
    stdout.flush()?;

    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Send an authentication request to the session launcher.
async fn authenticate(socket_path: &PathBuf, token: &str, user: &str) -> Result<LauncherResponse> {
    let stream = UnixStream::connect(socket_path).await.map_err(|e| {
        miette::miette!(
            "failed to connect to launcher at {}: {}",
            socket_path.display(),
            e
        )
    })?;

    let (reader, mut writer) = stream.into_split();

    let request = LauncherRequest::SessionAuthenticated {
        token: token.to_string(),
        user: user.to_string(),
    };

    let mut json = serde_json::to_string(&request)
        .map_err(|e| miette::miette!("failed to serialize request: {}", e))?;
    json.push('\n');

    writer
        .write_all(json.as_bytes())
        .await
        .map_err(|e| miette::miette!("failed to send to launcher: {}", e))?;

    let mut reader = tokio::io::BufReader::new(reader);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .await
        .map_err(|e| miette::miette!("failed to read launcher response: {}", e))?;

    let response: LauncherResponse = serde_json::from_str(response_line.trim())
        .map_err(|e| miette::miette!("failed to parse launcher response: {}", e))?;

    Ok(response)
}

/// Default launcher socket path.
fn default_socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("wayray-launcher.sock")
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let socket_path = std::env::var("WAYRAY_LAUNCHER_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_socket_path());

    // The session token is passed via environment variable by the launcher.
    let token = std::env::var("WAYRAY_SESSION_TOKEN").unwrap_or_else(|_| {
        eprintln!("warning: WAYRAY_SESSION_TOKEN not set, using 'unknown'");
        "unknown".to_string()
    });

    println!();
    println!("  WayRay Login");
    println!("  ============");
    println!();

    // Simple login loop.
    loop {
        let user = prompt("  Username: ").map_err(|e| miette::miette!("input error: {}", e))?;
        if user.is_empty() {
            continue;
        }

        let _password =
            prompt("  Password: ").map_err(|e| miette::miette!("input error: {}", e))?;

        // NOTE: This reference greeter does NOT verify the password.
        // A production greeter would authenticate via PAM here.
        // For the reference implementation, any non-empty username succeeds.

        info!(%user, "authenticating with session launcher");

        match authenticate(&socket_path, &token, &user).await {
            Ok(LauncherResponse::DesktopStarted { user, .. }) => {
                println!("  Welcome, {user}! Starting desktop...");
                println!();
                // Exit — the launcher starts the desktop components.
                return Ok(());
            }
            Ok(LauncherResponse::Error { message, .. }) => {
                eprintln!("  Login failed: {message}");
                eprintln!();
            }
            Ok(other) => {
                info!(?other, "unexpected response from launcher");
                eprintln!("  Unexpected response. Try again.");
                eprintln!();
            }
            Err(e) => {
                eprintln!("  Connection error: {e}");
                eprintln!("  (Is the session launcher running?)");
                eprintln!();
            }
        }
    }
}
