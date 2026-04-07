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
use winit::event_loop::{ActiveEventLoop, EventLoop};
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Use Poll mode so the event loop continuously checks for new frames
        // instead of blocking until user input arrives.
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

        // Drain any pending frames from the network thread.
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

fn main() {
    // Initialize tracing with RUST_LOG env filtering.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: wrclient <host>:<port>");
        std::process::exit(1);
    }

    let (server_addr, server_name) = match network::resolve_server_addr(&args[1]) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("Invalid server address '{}': {}", args[1], e);
            std::process::exit(1);
        }
    };

    info!(server = %server_addr, name = %server_name, "connecting to server");

    let (frame_tx, frame_rx) = mpsc::channel::<FrameData>();
    let (input_tx, input_rx) = mpsc::channel::<InputMessage>();

    // Use a std::sync::mpsc oneshot pattern: the network thread sends
    // dimensions back before entering its frame-receive loop.
    let (dim_tx, dim_rx) = mpsc::channel::<(u32, u32)>();

    // Spawn the network thread with its own tokio runtime.
    std::thread::Builder::new()
        .name("wrclient-network".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create tokio runtime");

            rt.block_on(async move {
                let config = ClientConfig {
                    server_addr,
                    server_name,
                    capabilities: vec!["display".to_string()],
                };

                let (_endpoint, mut conn) = match network::connect(&config).await {
                    Ok(c) => c,
                    Err(e) => {
                        error!(error = %e, "failed to connect to server");
                        // Send zero dimensions to unblock the main thread.
                        let _ = dim_tx.send((0, 0));
                        return;
                    }
                };

                let width = conn.server_hello.output_width;
                let height = conn.server_hello.output_height;
                info!(
                    width,
                    height,
                    session_id = conn.server_hello.session_id,
                    "connected to server"
                );

                // Send dimensions to the main thread so it can create the window.
                if dim_tx.send((width, height)).is_err() {
                    error!("main thread not listening for dimensions");
                    return;
                }

                // Accept the display stream from the server.
                let mut display_recv = match conn.accept_display_stream().await {
                    Ok(s) => s,
                    Err(e) => {
                        error!(error = %e, "failed to accept display stream");
                        return;
                    }
                };

                // Maintain a persistent framebuffer for XOR-diff decoding.
                let stride = width as usize * 4;
                let mut framebuffer = vec![0u8; stride * height as usize];

                // Read frames and forward input in a select loop.
                loop {
                    // Drain any pending input messages before blocking on frame read.
                    while let Ok(input_msg) = input_rx.try_recv() {
                        if let Err(e) = conn.send_input(&input_msg).await {
                            warn!(error = %e, "failed to send input");
                        }
                    }

                    // Use a short timeout so we can keep draining input even
                    // when no frames are arriving.
                    let frame_result = tokio::time::timeout(
                        std::time::Duration::from_millis(5),
                        network::read_display_message(&mut display_recv),
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
                                wayray_protocol::encoding::apply_region(
                                    &mut framebuffer,
                                    stride,
                                    region,
                                );
                            }

                            if frame_tx
                                .send(FrameData {
                                    pixels: framebuffer.clone(),
                                })
                                .is_err()
                            {
                                info!("render thread closed, stopping network loop");
                                break;
                            }

                            // Acknowledge the frame.
                            if let Err(e) = conn.send_frame_ack(update.sequence).await {
                                warn!(error = %e, "failed to send frame ack");
                            }
                        }
                        Ok(Err(e)) => {
                            error!(error = %e, "display stream error");
                            break;
                        }
                        Err(_) => {
                            // Timeout -- no frame available, loop back to drain input.
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
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = App::new(width, height, frame_rx, input_tx);
    event_loop.run_app(&mut app).expect("event loop error");
}
