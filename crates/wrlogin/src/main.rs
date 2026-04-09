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
//! 2. Sends `session_authenticated` to wrsessd via the launcher IPC
//! 3. Exits on success, allowing the desktop to start

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use miette::Result;
use tracing::info;
use wayray_protocol::launcher::{LauncherRequest, LauncherResponse};
use wayray_protocol::transport;

/// Read a line from stdin with a prompt.
fn prompt(message: &str) -> io::Result<String> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(message.as_bytes())?;
    stdout.flush()?;

    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn ipc_path() -> PathBuf {
    std::env::var("WAYRAY_LAUNCHER_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| transport::default_ipc_path())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // The session token is passed via environment variable by the launcher.
    let token = std::env::var("WAYRAY_SESSION_TOKEN").unwrap_or_else(|_| {
        eprintln!("warning: WAYRAY_SESSION_TOKEN not set, using 'unknown'");
        "unknown".to_string()
    });

    println!();
    println!("  WayRay Login");
    println!("  ============");
    println!();

    let path = ipc_path();

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

        let request = LauncherRequest::SessionAuthenticated {
            token: token.clone(),
            user: user.clone(),
        };

        match transport::send_request_sync(&path, &request) {
            Ok(LauncherResponse::DesktopStarted { user, .. }) => {
                println!("  Welcome, {user}! Starting desktop...");
                println!();
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
