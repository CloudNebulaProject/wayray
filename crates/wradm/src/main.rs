//! wradm -- WayRay administration CLI.
//!
//! Provides session management commands following the illumos `zoneadm`/`svcadm`
//! pattern. Communicates with the session launcher (wrsessd) via Unix socket.
//!
//! ## Commands
//!
//! - `wradm list` — List all managed sessions
//! - `wradm kill <token>` — Kill a session by token
//! - `wradm show <token>` — Show details for a session

use std::path::PathBuf;

use miette::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use wayray_protocol::launcher::{LauncherRequest, LauncherResponse};

/// Default launcher socket path.
fn default_socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("wayray-launcher.sock")
}

/// Send a request to the launcher and read the response.
async fn send_request(request: &LauncherRequest) -> Result<LauncherResponse> {
    let socket_path = std::env::var("WAYRAY_LAUNCHER_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_socket_path());

    let stream = UnixStream::connect(&socket_path).await.map_err(|e| {
        miette::miette!(
            "failed to connect to launcher at {}: {}\n\nIs wrsessd running?",
            socket_path.display(),
            e
        )
    })?;

    let (reader, mut writer) = stream.into_split();

    let mut json = serde_json::to_string(request)
        .map_err(|e| miette::miette!("serialization error: {}", e))?;
    json.push('\n');

    writer
        .write_all(json.as_bytes())
        .await
        .map_err(|e| miette::miette!("failed to send request: {}", e))?;

    let mut reader = BufReader::new(reader);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .await
        .map_err(|e| miette::miette!("failed to read response: {}", e))?;

    serde_json::from_str(response_line.trim())
        .map_err(|e| miette::miette!("failed to parse response: {}", e))
}

fn print_usage() {
    eprintln!("Usage: wradm <command> [args]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  list            List all managed sessions");
    eprintln!("  kill <token>    Kill a session by token");
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    let command = args.get(1).map(String::as_str).unwrap_or_else(|| {
        print_usage();
        std::process::exit(1);
    });

    match command {
        "list" => {
            let response = send_request(&LauncherRequest::ListSessions).await?;
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
            let response = send_request(&LauncherRequest::KillSession {
                token: token.clone(),
            })
            .await?;
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
