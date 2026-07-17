//! wradm -- WayRay administration CLI.
//!
//! Provides session management commands following the illumos `zoneadm`/`svcadm`
//! pattern. Talks to two daemons:
//!
//! - the session launcher (wrsessd) via the platform IPC transport
//!   (Unix sockets on Linux, doors on illumos) for `list` / `kill`;
//! - the compositor (wrsrvd) via its admin control socket for
//!   `sessions` / `status`.
//!
//! ## Commands
//!
//! - `wradm list` — List sessions managed by the launcher (wrsessd)
//! - `wradm kill <token>` — Kill a launcher-managed session by token
//! - `wradm sessions` — List sessions in the compositor's registry (wrsrvd)
//! - `wradm status` — Show compositor server status (wrsrvd)
//!
//! ## Options
//!
//! - `--server-socket <path>` — wrsrvd admin socket path (for `sessions` /
//!   `status`); defaults to `$XDG_RUNTIME_DIR/wrsrvd-admin.sock`, overridable
//!   via `WAYRAY_ADMIN_SOCKET`.

use std::path::{Path, PathBuf};

use miette::Result;
use wayray_protocol::admin::{AdminRequest, AdminResponse, ServerStatus, SessionEntry};
use wayray_protocol::launcher::{LauncherRequest, LauncherResponse};
use wayray_protocol::transport;

fn ipc_path() -> PathBuf {
    std::env::var("WAYRAY_LAUNCHER_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| transport::default_ipc_path())
}

fn send(request: &LauncherRequest) -> Result<LauncherResponse> {
    transport::send_request_sync(&ipc_path(), request).map_err(|e| {
        miette::miette!(
            "launcher communication failed: {}\n\nIs wrsessd running?",
            e
        )
    })
}

/// Resolve the wrsrvd admin socket path: `--server-socket` flag, then the
/// `WAYRAY_ADMIN_SOCKET` environment variable, then the default location.
fn admin_path(args: &[String]) -> PathBuf {
    args.windows(2)
        .find(|w| w[0] == "--server-socket")
        .map(|w| PathBuf::from(&w[1]))
        .or_else(|| std::env::var_os("WAYRAY_ADMIN_SOCKET").map(PathBuf::from))
        .unwrap_or_else(wayray_protocol::admin::default_admin_socket_path)
}

fn send_admin(path: &Path, request: &AdminRequest) -> Result<AdminResponse> {
    transport::unix_transport::send_json(path, request).map_err(|e| {
        miette::miette!(
            "compositor admin communication failed at {}: {}\n\n\
             Is wrsrvd running with --admin-socket?",
            path.display(),
            e
        )
    })
}

/// Render seconds as a compact human duration (e.g. `2h15m`, `45s`).
fn human_duration(secs: u64) -> String {
    let (d, h, m, s) = (
        secs / 86_400,
        (secs % 86_400) / 3_600,
        (secs % 3_600) / 60,
        secs % 60,
    );
    if d > 0 {
        format!("{d}d{h}h")
    } else if h > 0 {
        format!("{h}h{m}m")
    } else if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

fn print_sessions(sessions: &[SessionEntry]) {
    if sessions.is_empty() {
        println!("No sessions.");
        return;
    }
    println!(
        "{:<6}  {:<12}  {:<16}  {:<10}  {:<8}  CLIENT",
        "ID", "TOKEN", "USER", "STATE", "UPTIME"
    );
    for s in sessions {
        println!(
            "{:<6}  {:<12}  {:<16}  {:<10}  {:<8}  {}",
            s.id,
            s.token_prefix,
            s.user.as_deref().unwrap_or("-"),
            s.state,
            human_duration(s.uptime_secs),
            s.client_addr.as_deref().unwrap_or("-"),
        );
    }
}

fn print_status(status: &ServerStatus) {
    println!("Server:    wrsrvd {}", status.version);
    println!("Uptime:    {}", human_duration(status.uptime_secs));
    println!(
        "Sessions:  {} active, {} suspended, {} creating, {} destroyed",
        status.sessions.active,
        status.sessions.suspended,
        status.sessions.creating,
        status.sessions.destroyed,
    );
    if status.cluster_peers > 0 {
        println!("Cluster:   {} peer(s)", status.cluster_peers);
    } else {
        println!("Cluster:   single-server");
    }
}

fn print_usage() {
    eprintln!("Usage: wradm <command> [args]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  list            List launcher-managed sessions (wrsessd)");
    eprintln!("  kill <token>    Kill a launcher-managed session by token");
    eprintln!("  sessions        List compositor sessions (wrsrvd)");
    eprintln!("  status          Show compositor server status (wrsrvd)");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --server-socket <path>   wrsrvd admin socket (sessions/status)");
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let command = args.get(1).map(String::as_str).unwrap_or_else(|| {
        print_usage();
        std::process::exit(1);
    });

    match command {
        "list" => {
            let response = send(&LauncherRequest::ListSessions)?;
            match response {
                LauncherResponse::SessionList { sessions } => {
                    if sessions.is_empty() {
                        println!("No active sessions.");
                    } else {
                        println!("{:<36}  {:<16}  PROCESSES", "TOKEN", "USER");
                        for s in sessions {
                            println!(
                                "{:<36}  {:<16}  {}",
                                s.token,
                                s.user.as_deref().unwrap_or("-"),
                                s.child_count,
                            );
                        }
                    }
                }
                other => {
                    eprintln!("Unexpected response: {other:?}");
                }
            }
        }
        "kill" => {
            let token = args
                .get(2)
                .ok_or_else(|| miette::miette!("Usage: wradm kill <token>"))?;
            let response = send(&LauncherRequest::KillSession {
                token: token.clone(),
            })?;
            match response {
                LauncherResponse::SessionKilled { token } => {
                    println!("Session {token} killed.");
                }
                LauncherResponse::Error { message, .. } => {
                    eprintln!("Error: {message}");
                    std::process::exit(1);
                }
                other => {
                    eprintln!("Unexpected response: {other:?}");
                }
            }
        }
        "sessions" => {
            let response = send_admin(&admin_path(&args), &AdminRequest::ListSessions)?;
            match response {
                AdminResponse::SessionList { sessions } => print_sessions(&sessions),
                AdminResponse::Error { message } => {
                    eprintln!("Error: {message}");
                    std::process::exit(1);
                }
                other => {
                    eprintln!("Unexpected response: {other:?}");
                }
            }
        }
        "status" => {
            let response = send_admin(&admin_path(&args), &AdminRequest::ServerStatus)?;
            match response {
                AdminResponse::ServerStatus(status) => print_status(&status),
                AdminResponse::Error { message } => {
                    eprintln!("Error: {message}");
                    std::process::exit(1);
                }
                other => {
                    eprintln!("Unexpected response: {other:?}");
                }
            }
        }
        _ => {
            eprintln!("Unknown command: {command}");
            print_usage();
            std::process::exit(1);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_duration_formats() {
        assert_eq!(human_duration(0), "0s");
        assert_eq!(human_duration(45), "45s");
        assert_eq!(human_duration(125), "2m5s");
        assert_eq!(human_duration(8100), "2h15m");
        assert_eq!(human_duration(90 * 3600), "3d18h");
    }

    #[test]
    fn admin_path_prefers_flag() {
        let args = vec![
            "wradm".to_string(),
            "sessions".to_string(),
            "--server-socket".to_string(),
            "/run/custom.sock".to_string(),
        ];
        assert_eq!(admin_path(&args), PathBuf::from("/run/custom.sock"));
    }
}
