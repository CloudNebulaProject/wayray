//! wradm -- WayRay administration CLI.
//!
//! Provides session management commands following the illumos `zoneadm`/`svcadm`
//! pattern. Communicates with the session launcher (wrsessd) via the platform
//! IPC transport (Unix sockets on Linux, doors on illumos).
//!
//! ## Commands
//!
//! - `wradm list` — List all managed sessions
//! - `wradm kill <token>` — Kill a session by token

use std::path::PathBuf;

use miette::Result;
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

fn print_usage() {
    eprintln!("Usage: wradm <command> [args]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  list            List all managed sessions");
    eprintln!("  kill <token>    Kill a session by token");
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
        _ => {
            eprintln!("Unknown command: {command}");
            print_usage();
            std::process::exit(1);
        }
    }

    Ok(())
}
