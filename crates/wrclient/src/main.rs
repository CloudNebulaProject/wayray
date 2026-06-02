//! wrclient -- WayRay thin client viewer.
//!
//! Connects to a wrsrvd server via QUIC, receives framebuffer updates,
//! and renders them in a native window using wgpu. Input events are
//! captured and will be forwarded to the server in a future task.

pub mod display;
pub mod input;
pub mod network;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use tracing::{error, info, warn};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowId};

use wayray_protocol::messages::InputMessage;

use crate::display::Display;
use crate::network::ClientConfig;

/// Frame data sent from the network thread to the render thread.
pub struct FrameData {
    /// Raw BGRA8 pixel data for the full framebuffer.
    pub pixels: Vec<u8>,
}

/// Main application state for the winit event loop.
struct App {
    /// Server output dimensions.
    width: u32,
    height: u32,
    /// Receiver for frames from the network thread.
    frame_rx: mpsc::Receiver<FrameData>,
    /// Sender for input events to the network thread.
    input_tx: mpsc::Sender<InputMessage>,
    /// Display state, created once the window is available.
    display: Option<Display>,
    /// The window reference.
    window: Option<Arc<Window>>,
    /// Last-known modifier state. winit reports modifier *releases* via
    /// `ModifiersChanged` (and on macOS not always via `KeyboardInput`), so we
    /// treat `ModifiersChanged` as the source of truth and synthesize key
    /// press/release events from it — otherwise a released modifier (e.g.
    /// Shift) can get stuck "down" on the server, shifting all later input.
    modifiers: winit::keyboard::ModifiersState,
}

impl App {
    fn new(
        width: u32,
        height: u32,
        frame_rx: mpsc::Receiver<FrameData>,
        input_tx: mpsc::Sender<InputMessage>,
    ) -> Self {
        Self {
            width,
            height,
            frame_rx,
            input_tx,
            display: None,
            window: None,
            modifiers: winit::keyboard::ModifiersState::empty(),
        }
    }

    /// Send an input message to the network thread, logging on failure.
    fn send_input(&self, msg: InputMessage) {
        if self.input_tx.send(msg).is_err() {
            warn!("network thread closed, cannot send input");
        }
    }

    /// Drain pending frames from the network thread into the GPU texture.
    /// Each [`FrameData`] is a complete framebuffer snapshot, so coalescing
    /// multiple into the last is correct. Returns whether any were applied.
    fn drain_frames(&mut self) -> bool {
        let mut got = false;
        while let Ok(frame) = self.frame_rx.try_recv() {
            if let Some(display) = &self.display {
                display.update_frame(&frame.pixels);
                got = true;
            }
        }
        got
    }

    /// Present the current frame texture to the window. Called from
    /// `RedrawRequested`, which is requested whenever new frames arrive.
    fn present(&mut self, event_loop: &ActiveEventLoop) {
        let Some(display) = &mut self.display else {
            return;
        };
        match display.render() {
            Ok(()) => {}
            Err(wgpu::SurfaceError::OutOfMemory) => {
                error!("GPU out of memory, exiting");
                event_loop.exit();
            }
            Err(e) => {
                // Lost/outdated surfaces are reconfigured inside render().
                warn!(error = %e, "render error, will retry");
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("WayRay Client")
            .with_inner_size(winit::dpi::PhysicalSize::new(self.width, self.height));

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("failed to create window"),
        );

        let w = self.width;
        let h = self.height;
        let win = window.clone();

        // Initialize wgpu display synchronously via pollster since
        // winit's resumed callback cannot be async.
        let display = pollster::block_on(Display::new(win, w, h));

        self.window = Some(window);
        self.display = Some(display);

        info!(
            width = w,
            height = h,
            "window created and display initialized"
        );
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Upload any new frames, then present *every* tick. macOS only reliably
        // composites a CAMetalLayer from a continuous present loop; presenting
        // only when a new frame arrived left the window stale until an OS event
        // forced a redraw. The texture changes only on new frames, so a present
        // is just a cheap fullscreen blit; AutoVsync paces the loop to the
        // display rate (so Poll does not busy-spin).
        self.drain_frames();
        self.present(event_loop);
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        // Woken by the network thread; upload frames now so the next present
        // (in about_to_wait) shows them promptly.
        self.drain_frames();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                info!("close requested, shutting down");
                event_loop.exit();
                // Force exit since the network thread may be blocking.
                std::process::exit(0);
            }
            WindowEvent::Resized(physical_size) => {
                if let Some(display) = &mut self.display {
                    display.resize(physical_size);
                }
            }
            WindowEvent::RedrawRequested => {
                self.present(event_loop);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let real_shift = self.modifiers.shift_key();
                for msg in input::convert_keyboard(&event, real_shift) {
                    self.send_input(msg);
                }
            }
            WindowEvent::ModifiersChanged(new_mods) => {
                // Synthesize modifier key press/release from the authoritative
                // modifier state so a release is never lost (which would leave
                // e.g. Shift stuck down on the server).
                let new_state = new_mods.state();
                for msg in input::convert_modifiers(self.modifiers, new_state) {
                    self.send_input(msg);
                }
                self.modifiers = new_state;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let msg = input::convert_cursor_moved(position);
                self.send_input(msg);
            }
            WindowEvent::MouseInput { button, state, .. } => {
                if let Some(msg) = input::convert_mouse_button(button, state) {
                    self.send_input(msg);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                for msg in input::convert_mouse_wheel(delta) {
                    self.send_input(msg);
                }
            }
            _ => {}
        }
    }
}

/// Initial reconnect backoff in milliseconds. Kept tight so loopback/LAN
/// reconnects land well under the 500ms hot-desk target.
const INITIAL_BACKOFF_MS: u64 = 50;

/// Maximum reconnect backoff in milliseconds (exponential cap).
const MAX_BACKOFF_MS: u64 = 2000;

/// Why the per-connection session loop ended.
enum SessionEnd {
    /// The local render thread closed; the client should shut down.
    RenderThreadClosed,
    /// The server connection dropped; the client should reconnect.
    Disconnected,
}

/// Run one connection's frame-receive / input-forward loop until the stream
/// errors (returns [`SessionEnd::Disconnected`]) or the render thread closes
/// (returns [`SessionEnd::RenderThreadClosed`]).
async fn run_session_loop(
    conn: &mut network::ServerConnection,
    display_recv: &mut quinn::RecvStream,
    framebuffer: &mut [u8],
    stride: usize,
    input_rx: &mpsc::Receiver<InputMessage>,
    frame_tx: &mpsc::Sender<FrameData>,
    proxy: &EventLoopProxy<()>,
) -> SessionEnd {
    // True while we've asked for a keyframe and are waiting for the resync, so
    // we don't flood the server with requests every frame during the round-trip.
    let mut keyframe_pending = false;
    loop {
        // Drain any pending input messages before blocking on frame read.
        while let Ok(input_msg) = input_rx.try_recv() {
            if let Err(e) = conn.send_input(&input_msg).await {
                warn!(error = %e, "failed to send input, reconnecting");
                return SessionEnd::Disconnected;
            }
        }

        // Use a short timeout so we can keep draining input even when no
        // frames are arriving.
        let frame_result = tokio::time::timeout(
            std::time::Duration::from_millis(5),
            network::read_display_message(display_recv),
        )
        .await;

        match frame_result {
            Ok(Ok(wayray_protocol::messages::DisplayMessage::FrameUpdate(update))) => {
                info!(
                    sequence = update.sequence,
                    regions = update.regions.len(),
                    "received frame"
                );

                // Apply damage regions to the persistent framebuffer.
                for region in &update.regions {
                    wayray_protocol::encoding::apply_region(framebuffer, stride, region);
                }

                // Verify the reconstruction against the server's checksum. A
                // mismatch means a frame was missed/reordered and left a region
                // stale; ask for a keyframe to resync (once, until it lands).
                let height = framebuffer.len() / stride;
                let local = wayray_protocol::encoding::checksum(framebuffer, stride, stride, height);
                if local != update.checksum {
                    if !keyframe_pending {
                        warn!(
                            sequence = update.sequence,
                            "framebuffer checksum mismatch; requesting keyframe to resync"
                        );
                        if let Err(e) = conn.request_keyframe().await {
                            warn!(error = %e, "failed to request keyframe, reconnecting");
                            return SessionEnd::Disconnected;
                        }
                        keyframe_pending = true;
                    }
                } else {
                    keyframe_pending = false;
                }

                if frame_tx
                    .send(FrameData {
                        pixels: framebuffer.to_vec(),
                    })
                    .is_err()
                {
                    return SessionEnd::RenderThreadClosed;
                }

                // Wake the winit event loop to process the new frame.
                let _ = proxy.send_event(());

                // Acknowledge the frame.
                if let Err(e) = conn.send_frame_ack(update.sequence).await {
                    warn!(error = %e, "failed to send frame ack, reconnecting");
                    return SessionEnd::Disconnected;
                }
            }
            Ok(Err(e)) => {
                error!(error = %e, "display stream error");
                return SessionEnd::Disconnected;
            }
            Err(_) => {
                // Timeout -- no frame available, loop back to drain input.
            }
        }
    }
}

/// Maximum number of redirect hops before giving up (loop guard).
const MAX_REDIRECTS: u32 = 3;

/// Outcome of one (redirect-following) connection attempt.
enum ConnectResult {
    /// Connected to a server; streams ready. Boxed to keep the enum small.
    Connected(Box<(quinn::Endpoint, network::ServerConnection)>),
    /// Connection failed on the very first attempt — the client should give up.
    FatalFirstAttempt,
    /// Connection failed; the caller should back off and retry.
    Retry,
}

/// Connect to one of the candidate servers (tried in order), following up to
/// [`MAX_REDIRECTS`] `SessionRedirect` responses (multi-server affinity / load
/// balancing). The same token is reused on every hop and every candidate so
/// the home server resumes the session.
///
/// Redirect targets are validated against `allowed` — the set of addresses the
/// client already trusts (its candidates / cluster config). A redirect to any
/// other address is refused: otherwise a compromised or impersonated server
/// could send the client (and its session token) to an attacker-controlled
/// host. The server's certificate is pinned via `pin_store` (TOFU) or a
/// configured per-candidate fingerprint.
async fn connect_following_redirects(
    candidates: &[Candidate],
    token: &Option<String>,
    first_attempt: bool,
    pin_store: &Arc<Mutex<network::PinStore>>,
    allowed: &HashMap<SocketAddr, Candidate>,
) -> ConnectResult {
    let mut last_error: Option<String> = None;

    for (cand_idx, candidate) in candidates.iter().enumerate() {
        // Each candidate gets its own redirect chain.
        let mut hop_target = candidate.clone();

        for hop in 0..=MAX_REDIRECTS {
            let config = ClientConfig {
                server_addr: hop_target.addr,
                server_name: hop_target.name.clone(),
                capabilities: vec!["display".to_string()],
                token: token.clone(),
                expected_fingerprint: hop_target.fingerprint.clone(),
                pin_store: pin_store.clone(),
            };

            match network::connect(&config).await {
                Ok(network::ConnectOutcome::Connected(ep, conn)) => {
                    return ConnectResult::Connected(Box::new((ep, conn)));
                }
                Ok(network::ConnectOutcome::Redirect { server_id, addr }) => {
                    if hop == MAX_REDIRECTS {
                        warn!(%server_id, %addr, "redirect limit reached for candidate");
                        break; // try the next candidate
                    }
                    match network::resolve_server_addr(&addr) {
                        Ok((resolved, name)) => {
                            // Only follow redirects to servers we already trust.
                            // An untrusted target must never receive our token.
                            match allowed.get(&resolved) {
                                Some(known) => {
                                    info!(%server_id, target = %resolved, hop, "following redirect");
                                    hop_target = Candidate {
                                        addr: resolved,
                                        name: known.name.clone(),
                                        fingerprint: known.fingerprint.clone(),
                                    };
                                    let _ = name; // resolved name superseded by trusted entry
                                }
                                None => {
                                    warn!(
                                        %server_id, %addr,
                                        "refusing redirect to address outside the trusted cluster set"
                                    );
                                    break; // try the next candidate
                                }
                            }
                        }
                        Err(e) => {
                            warn!(%server_id, %addr, error = %e, "cannot resolve redirect target");
                            break; // try the next candidate
                        }
                    }
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                    warn!(candidate = cand_idx, error = %e, "connection attempt failed");
                    break; // try the next candidate
                }
            }
        }
    }

    if first_attempt {
        error!(error = ?last_error, "failed to connect to any candidate server");
        return ConnectResult::FatalFirstAttempt;
    }
    ConnectResult::Retry
}

/// A server the client may connect to, with its TLS pin policy.
#[derive(Clone)]
struct Candidate {
    /// Resolved socket address.
    addr: SocketAddr,
    /// TLS SNI / pin identity name.
    name: String,
    /// Configured certificate fingerprint (`sha256:<hex>`), if any. `None`
    /// selects trust-on-first-use for this server.
    fingerprint: Option<String>,
}

/// Parse the server candidate list from CLI args: positional `host:port`
/// entries plus every server listed in a `--cluster <path>` cluster.toml.
/// Cluster entries carry any configured certificate fingerprint so the
/// connection can be pinned strictly. Unresolvable entries are skipped.
///
/// An explicit `--server-fingerprint <sha256:..>` applies to positional
/// candidates (the common single-server case).
fn parse_server_candidates(args: &[String]) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = Vec::new();

    let cli_fingerprint = args
        .iter()
        .position(|a| a == "--server-fingerprint")
        .and_then(|pos| args.get(pos + 1))
        .cloned();

    // Positional args (everything that is not a flag or a flag value).
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        if a == "--token" || a == "--cluster" || a == "--server-fingerprint" {
            i += 2; // skip the flag and its value
            continue;
        }
        if a.starts_with("--") {
            i += 1;
            continue;
        }
        match network::resolve_server_addr(a) {
            Ok((addr, name)) => candidates.push(Candidate {
                addr,
                name,
                fingerprint: cli_fingerprint.clone(),
            }),
            Err(e) => warn!(arg = %a, error = %e, "skipping unresolvable server address"),
        }
        i += 1;
    }

    // --cluster <path>: add every server entry from the cluster config.
    if let Some(pos) = args.iter().position(|a| a == "--cluster")
        && let Some(path) = args.get(pos + 1)
    {
        match wayray_protocol::cluster::ClusterConfig::load(std::path::Path::new(path)) {
            Ok(cluster) => {
                for server in &cluster.servers {
                    match network::resolve_server_addr(&server.addr) {
                        Ok((addr, name)) => {
                            if !candidates.iter().any(|c| c.addr == addr) {
                                let fingerprint = (!server.fingerprint.is_empty())
                                    .then(|| server.fingerprint.clone());
                                candidates.push(Candidate {
                                    addr,
                                    name,
                                    fingerprint,
                                });
                            }
                        }
                        Err(e) => {
                            warn!(server = %server.id, addr = %server.addr, error = %e, "skipping cluster entry")
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to load cluster config '{path}': {e}");
            }
        }
    }

    candidates
}

/// Load or generate a persistent session token.
///
/// Reads from `~/.config/wayray/token`. If the file doesn't exist,
/// generates a random hex token and writes it.
fn load_or_generate_token() -> String {
    let config_dir = dirs_path().join("wayray");
    let token_path = config_dir.join("token");

    if let Ok(token) = std::fs::read_to_string(&token_path) {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return token;
        }
    }

    // Generate a random 16-byte hex token.
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("failed to generate random token");
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();

    // Persist it with owner-only permissions: the token is a session
    // credential and must not be world-readable.
    if let Err(e) = wayray_protocol::tls::write_private(&token_path, token.as_bytes()) {
        warn!(error = %e, "failed to persist token");
    } else {
        info!(path = %token_path.display(), "session token persisted");
    }

    token
}

/// Get the user's config directory base path.
fn dirs_path() -> std::path::PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            std::path::PathBuf::from(home).join(".config")
        })
}

fn main() {
    // Initialize tracing with RUST_LOG env filtering.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    // Parse: wrclient [<host>:<port> ...] [--token <token>] [--cluster <path>]
    //
    // Candidate servers are tried in order until one binds (or redirects to a
    // working server). Sources, in priority order: positional host:port args,
    // then every server in a --cluster <path> cluster.toml.
    let token = if let Some(pos) = args.iter().position(|a| a == "--token") {
        args.get(pos + 1).cloned()
    } else {
        Some(load_or_generate_token())
    };

    let candidates = parse_server_candidates(&args);
    if candidates.is_empty() {
        eprintln!(
            "Usage: wrclient <host>:<port> [<host>:<port> ...] [--token <token>] [--cluster <path>] [--server-fingerprint <sha256:..>]"
        );
        std::process::exit(1);
    }

    // The set of server addresses the client trusts. Redirects are only
    // followed to one of these, so an impersonated server cannot send the
    // session token to an arbitrary host.
    let allowed: HashMap<SocketAddr, Candidate> =
        candidates.iter().map(|c| (c.addr, c.clone())).collect();

    // Shared trust-on-first-use pin store, so the client never blindly accepts
    // an unknown server certificate (MITM protection for the session token).
    let pin_store = Arc::new(Mutex::new(
        network::PinStore::open_default().unwrap_or_else(|e| {
            warn!(error = %e, "failed to open known_hosts pin store; using an empty in-memory store");
            network::PinStore::open(std::env::temp_dir().join("wayray-known_hosts"))
                .expect("temp pin store")
        }),
    ));

    // The token is a session credential; log only whether one is set, never
    // its value.
    info!(
        candidates = candidates.len(),
        has_token = token.is_some(),
        "connecting to server (trying candidates in order)"
    );

    let (frame_tx, frame_rx) = mpsc::channel::<FrameData>();
    let (input_tx, input_rx) = mpsc::channel::<InputMessage>();

    // Use a std::sync::mpsc oneshot pattern: the network thread sends
    // dimensions back before entering its frame-receive loop.
    let (dim_tx, dim_rx) = mpsc::channel::<(u32, u32)>();

    // Create the event loop early so we can get a proxy for cross-thread wake.
    let event_loop = EventLoop::<()>::with_user_event()
        .build()
        .expect("failed to create event loop");
    let proxy: EventLoopProxy<()> = event_loop.create_proxy();

    // Spawn the network thread with its own tokio runtime.
    let net_proxy = proxy.clone();
    std::thread::Builder::new()
        .name("wrclient-network".into())
        .spawn(move || {
            let proxy = net_proxy;
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create tokio runtime");

            rt.block_on(async move {
                // Reconnect loop: on any disconnection we immediately retry with
                // the SAME token so the server resumes the existing session
                // (hot-desking). Backoff starts tight to hit the <500ms target.
                let mut backoff = std::time::Duration::from_millis(INITIAL_BACKOFF_MS);
                let mut dims_reported = false;
                let mut first_attempt = true;

                loop {
                    // Time from the start of a reconnect attempt to ServerHello.
                    let attempt_start = std::time::Instant::now();

                    // Connect, trying candidates in order and following redirects
                    // (capped) to the server that owns our session or a
                    // less-loaded peer.
                    let (_endpoint, mut conn) = match connect_following_redirects(
                        &candidates,
                        &token,
                        first_attempt,
                        &pin_store,
                        &allowed,
                    )
                    .await
                    {
                        ConnectResult::Connected(boxed) => {
                            let (ep, conn) = *boxed;
                            (ep, conn)
                        }
                        ConnectResult::FatalFirstAttempt => {
                            // Unblock the main thread on the very first try.
                            let _ = dim_tx.send((0, 0));
                            return;
                        }
                        ConnectResult::Retry => {
                            warn!(
                                backoff_ms = backoff.as_millis() as u64,
                                "reconnect failed, retrying"
                            );
                            tokio::time::sleep(backoff).await;
                            backoff =
                                (backoff * 2).min(std::time::Duration::from_millis(MAX_BACKOFF_MS));
                            continue;
                        }
                    };

                    first_attempt = false;
                    // Successful connection resets the backoff.
                    backoff = std::time::Duration::from_millis(INITIAL_BACKOFF_MS);

                    let width = conn.server_hello.output_width;
                    let height = conn.server_hello.output_height;
                    let resumed = conn.server_hello.resumed;
                    info!(
                        width,
                        height,
                        session_id = conn.server_hello.session_id,
                        resumed,
                        reconnect_ms = attempt_start.elapsed().as_millis() as u64,
                        "connected to server"
                    );

                    // On a resumed session the server sends a Resumed event on
                    // the control stream right after ServerHello. Consume it so
                    // the stream stays drained and we can log the confirmation.
                    // The framebuffer is reallocated fresh below regardless, so
                    // the resume already triggers a clean full redraw.
                    if resumed {
                        match conn.recv_control().await {
                            Ok(wayray_protocol::messages::ControlMessage::SessionEvent(
                                wayray_protocol::messages::SessionEvent::Resumed { session_id },
                            )) => {
                                info!(session_id, "session resume confirmed by server");
                            }
                            Ok(other) => {
                                warn!(
                                    ?other,
                                    "unexpected control message after resumed ServerHello"
                                );
                            }
                            Err(e) => {
                                warn!(error = %e, "failed to read Resumed event, reconnecting");
                                continue;
                            }
                        }
                    }

                    // Report dimensions to the main thread once (window is
                    // created from these); on resume they are unchanged.
                    if !dims_reported {
                        if dim_tx.send((width, height)).is_err() {
                            error!("main thread not listening for dimensions");
                            return;
                        }
                        dims_reported = true;
                    }

                    // Accept the display stream from the server.
                    let mut display_recv = match conn.accept_display_stream().await {
                        Ok(s) => s,
                        Err(e) => {
                            warn!(error = %e, "failed to accept display stream, reconnecting");
                            continue;
                        }
                    };

                    // Maintain a persistent framebuffer for XOR-diff decoding.
                    // A fresh buffer per (re)connection forces a clean redraw,
                    // matching the server's full-frame send after a resume.
                    let stride = width as usize * 4;
                    let mut framebuffer = vec![0u8; stride * height as usize];

                    // Read frames and forward input in a select loop. On any
                    // stream error we break out to the reconnect loop instead
                    // of exiting the client.
                    let disconnected = run_session_loop(
                        &mut conn,
                        &mut display_recv,
                        &mut framebuffer,
                        stride,
                        &input_rx,
                        &frame_tx,
                        &proxy,
                    )
                    .await;

                    match disconnected {
                        SessionEnd::RenderThreadClosed => {
                            info!("render thread closed, stopping network loop");
                            return;
                        }
                        SessionEnd::Disconnected => {
                            warn!("server connection lost, reconnecting with same token");
                            // Tight retry to hit the sub-500ms reconnect target.
                            tokio::time::sleep(backoff).await;
                        }
                    }
                }
            });
        })
        .expect("failed to spawn network thread");

    // Wait for the network thread to report server dimensions.
    let (width, height) = dim_rx
        .recv()
        .expect("network thread terminated unexpectedly");
    if width == 0 || height == 0 {
        error!("failed to get server dimensions, exiting");
        std::process::exit(1);
    }

    info!(width, height, "starting display");

    // Run the winit event loop on the main thread.
    // The event loop was created earlier (before spawning the network thread)
    // so we could pass an EventLoopProxy to wake it on new frames.
    let mut app = App::new(width, height, frame_rx, input_tx);
    event_loop.run_app(&mut app).expect("event loop error");
}
