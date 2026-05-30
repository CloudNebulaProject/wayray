//! wrclient -- WayRay thin client viewer.
//!
//! Connects to a wrsrvd server via QUIC, receives framebuffer updates,
//! and renders them in a native window using wgpu. Input events are
//! captured and will be forwarded to the server in a future task.

pub mod display;
pub mod input;
pub mod network;

use std::sync::Arc;
use std::sync::mpsc;

use tracing::{error, info, warn};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
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
        }
    }

    /// Send an input message to the network thread, logging on failure.
    fn send_input(&self, msg: InputMessage) {
        if self.input_tx.send(msg).is_err() {
            warn!("network thread closed, cannot send input");
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

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Drain any pending frames from the network thread.
        // The network thread wakes us via EventLoopProxy when frames arrive,
        // so we don't need to busy-poll.
        let mut got_frame = false;
        while let Ok(frame) = self.frame_rx.try_recv() {
            if let Some(display) = &self.display {
                display.update_frame(&frame.pixels);
                got_frame = true;
            }
        }

        if got_frame && let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        // Woken by the network thread — new frame available.
        // The actual frame processing happens in about_to_wait.
        // Just request a redraw check.
        let mut got_frame = false;
        while let Ok(frame) = self.frame_rx.try_recv() {
            if let Some(display) = &self.display {
                display.update_frame(&frame.pixels);
                got_frame = true;
            }
        }
        if got_frame && let Some(window) = &self.window {
            window.request_redraw();
        }
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
                if let Some(display) = &mut self.display {
                    match display.render() {
                        Ok(()) => {}
                        Err(wgpu::SurfaceError::OutOfMemory) => {
                            error!("GPU out of memory, exiting");
                            event_loop.exit();
                        }
                        Err(e) => {
                            // Lost/outdated surfaces are handled inside render(),
                            // just request another redraw.
                            warn!(error = %e, "render error, will retry");
                            if let Some(window) = &self.window {
                                window.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(msg) = input::convert_keyboard(&event) {
                    self.send_input(msg);
                }
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

    // Persist it.
    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        warn!(error = %e, "failed to create config dir");
    } else if let Err(e) = std::fs::write(&token_path, &token) {
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

    // Parse: wrclient <host>:<port> [--token <token>]
    let addr_arg = args.get(1).cloned().unwrap_or_else(|| {
        eprintln!("Usage: wrclient <host>:<port> [--token <token>]");
        std::process::exit(1);
    });

    let token = if let Some(pos) = args.iter().position(|a| a == "--token") {
        args.get(pos + 1).cloned()
    } else {
        Some(load_or_generate_token())
    };

    let (server_addr, server_name) = match network::resolve_server_addr(&addr_arg) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("Invalid server address '{}': {}", addr_arg, e);
            std::process::exit(1);
        }
    };

    info!(server = %server_addr, name = %server_name, token = ?token, "connecting to server");

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
                let config = ClientConfig {
                    server_addr,
                    server_name,
                    capabilities: vec!["display".to_string()],
                    token,
                };

                // Reconnect loop: on any disconnection we immediately retry with
                // the SAME token so the server resumes the existing session
                // (hot-desking). Backoff starts tight to hit the <500ms target.
                let mut backoff = std::time::Duration::from_millis(INITIAL_BACKOFF_MS);
                let mut dims_reported = false;
                let mut first_attempt = true;

                loop {
                    // Time from the start of a reconnect attempt to ServerHello.
                    let attempt_start = std::time::Instant::now();
                    let connect_result = network::connect(&config).await;

                    let (_endpoint, mut conn) = match connect_result {
                        Ok(c) => c,
                        Err(e) => {
                            if first_attempt {
                                error!(error = %e, "failed to connect to server");
                                // Unblock the main thread on the very first try.
                                let _ = dim_tx.send((0, 0));
                                return;
                            }
                            warn!(error = %e, backoff_ms = backoff.as_millis() as u64,
                                "reconnect failed, retrying");
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
