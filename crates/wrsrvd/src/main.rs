mod errors;
mod handlers;
mod render;
mod state;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::handlers::ClientState;
use crate::state::WayRay;
use miette::Result;
use smithay::{
    backend::{
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::Display,
    utils::Transform,
    wayland::{compositor::CompositorClientState, socket::ListeningSocketSource},
};
use tracing::info;

/// Data accessible from calloop event callbacks.
struct CalloopData {
    state: WayRay,
    display: Display<WayRay>,
    backend: WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: OutputDamageTracker,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("wrsrvd starting");

    // Create the Wayland display.
    let mut display = Display::<WayRay>::new()
        .map_err(|e| errors::WayRayError::DisplayInit(Box::new(e)))?;

    // Initialize the Winit backend (opens a window, creates a GlesRenderer).
    let (backend, winit_event_loop) = winit::init::<GlesRenderer>().map_err(|e| {
        errors::WayRayError::BackendInit(Box::<dyn std::error::Error + Send + Sync>::from(
            e.to_string(),
        ))
    })?;
    info!("winit backend initialized");

    // Create a virtual output matching the window size.
    let window_size = backend.window_size();
    let output = Output::new(
        "wayray-0".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "WayRay".to_string(),
            model: "Virtual".to_string(),
        },
    );

    let mode = Mode {
        size: window_size,
        refresh: 60_000, // 60 Hz in millihertz
    };
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, None);
    output.set_preferred(mode);

    // Create the global output for Wayland clients to bind to.
    output.create_global::<WayRay>(&display.handle());

    // Create compositor state.
    let mut state = WayRay::new(&mut display, output.clone());

    // Map the output into the compositor space.
    state.space.map_output(&output, (0, 0));
    info!("output mapped: {:?} @ {:?}", mode.size, mode.refresh);

    // Create a Wayland listening socket for clients.
    let listening_socket = ListeningSocketSource::new_auto()
        .map_err(|e| errors::WayRayError::DisplayInit(Box::new(e)))?;
    let socket_name = listening_socket.socket_name().to_os_string();
    info!(?socket_name, "wayland socket created");

    // Set WAYLAND_DISPLAY so child processes can find us.
    // SAFETY: This is called early in main before any other threads are spawned,
    // so there are no concurrent readers of the environment.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &socket_name) };

    // Create the calloop event loop.
    let mut event_loop: smithay::reexports::calloop::EventLoop<CalloopData> =
        smithay::reexports::calloop::EventLoop::try_new()
            .map_err(|e| errors::WayRayError::EventLoop(Box::new(e)))?;

    let loop_handle = event_loop.handle();

    // Insert the Wayland listening socket as a calloop source.
    // When a new client connects, insert it into the display.
    loop_handle
        .insert_source(listening_socket, |client_stream, _, data| {
            data.display
                .handle()
                .insert_client(
                    client_stream,
                    Arc::new(ClientState {
                        compositor_state: CompositorClientState::default(),
                    }),
                )
                .ok();
        })
        .map_err(|e| errors::WayRayError::EventLoop(Box::new(e.error)))?;

    // Shared flag to signal the main loop to exit.
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    // Insert the Winit event loop as a calloop source.
    loop_handle
        .insert_source(winit_event_loop, move |event, _, data| match event {
            WinitEvent::Resized { size, scale_factor } => {
                info!(?size, scale_factor, "window resized");
            }
            WinitEvent::Focus(focused) => {
                info!(focused, "window focus changed");
            }
            WinitEvent::Input(_event) => {
                // Input handling is Task 7 -- ignore for now.
            }
            WinitEvent::Redraw => {
                render::render_output_frame(
                    &mut data.state,
                    &mut data.backend,
                    &mut data.damage_tracker,
                );
            }
            WinitEvent::CloseRequested => {
                info!("close requested, shutting down");
                running_clone.store(false, Ordering::SeqCst);
            }
        })
        .map_err(|e| errors::WayRayError::EventLoop(Box::new(e.error)))?;

    // Create a damage tracker for efficient rendering.
    let damage_tracker = OutputDamageTracker::from_output(&output);

    let mut calloop_data = CalloopData {
        state,
        display,
        backend,
        damage_tracker,
    };

    info!("entering main event loop");

    // Main event loop.
    while running.load(Ordering::SeqCst) {
        // Dispatch Wayland clients.
        calloop_data
            .display
            .dispatch_clients(&mut calloop_data.state)
            .map_err(|e| errors::WayRayError::EventLoop(Box::new(e)))?;

        calloop_data
            .display
            .flush_clients()
            .map_err(|e| errors::WayRayError::EventLoop(Box::new(e)))?;

        // Dispatch calloop sources (Winit events + Wayland socket) with ~16ms timeout.
        event_loop
            .dispatch(Duration::from_millis(16), &mut calloop_data)
            .map_err(|e| errors::WayRayError::EventLoop(Box::new(e)))?;
    }

    info!("wrsrvd shutting down");
    Ok(())
}
