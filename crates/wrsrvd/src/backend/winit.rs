use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use miette::Result;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ExportMem,
            damage::OutputDamageTracker,
            element::texture::TextureRenderElement,
            gles::{GlesRenderer, GlesTexture},
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    desktop::{Window, space::render_output},
    output::Output,
    reexports::wayland_server::Display,
    utils::{Buffer as BufferCoord, Rectangle, Size},
    wayland::{compositor::CompositorClientState, socket::ListeningSocketSource},
};
use tracing::{info, warn};

use crate::errors::WayRayError;
use crate::handlers::ClientState;
use crate::state::WayRay;

/// Dark grey clear color for the compositor background.
const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

/// Data accessible from calloop event callbacks in the Winit backend.
struct CalloopData {
    state: WayRay,
    display: Display<WayRay>,
    backend: WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: OutputDamageTracker,
}

/// Run the compositor with the Winit backend (opens a window for development).
pub fn run(display: Display<WayRay>, mut state: WayRay, output: Output) -> Result<()> {
    // Initialize the Winit backend (opens a window, creates a GlesRenderer).
    let (backend, winit_event_loop) = winit::init::<GlesRenderer>().map_err(|e| {
        WayRayError::BackendInit(Box::<dyn std::error::Error + Send + Sync>::from(
            e.to_string(),
        ))
    })?;
    info!("winit backend initialized");

    // Update the output mode to match the actual Winit window size.
    let window_size = backend.window_size();
    let mode = smithay::output::Mode {
        size: window_size,
        refresh: 60_000,
    };
    output.change_current_state(Some(mode), None, None, None);
    output.set_preferred(mode);
    info!(?window_size, "output mode updated to match winit window");

    // Create a Wayland listening socket for clients.
    let listening_socket =
        ListeningSocketSource::new_auto().map_err(|e| WayRayError::DisplayInit(Box::new(e)))?;
    let socket_name = listening_socket.socket_name().to_os_string();
    info!(?socket_name, "wayland socket created");

    // Set WAYLAND_DISPLAY so child processes can find us.
    // SAFETY: This is called early in main before any other threads are spawned,
    // so there are no concurrent readers of the environment.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &socket_name) };

    // Create the calloop event loop.
    let mut event_loop: smithay::reexports::calloop::EventLoop<CalloopData> =
        smithay::reexports::calloop::EventLoop::try_new()
            .map_err(|e| WayRayError::EventLoop(Box::new(e)))?;

    let loop_handle = event_loop.handle();

    // Insert the Wayland listening socket as a calloop source.
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
        .map_err(|e| WayRayError::EventLoop(Box::new(e.error)))?;

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
            WinitEvent::Input(event) => {
                data.state.process_input_event(event);
            }
            WinitEvent::Redraw => {
                render_winit_frame(&mut data.state, &mut data.backend, &mut data.damage_tracker);
            }
            WinitEvent::CloseRequested => {
                info!("close requested, shutting down");
                running_clone.store(false, Ordering::SeqCst);
            }
        })
        .map_err(|e| WayRayError::EventLoop(Box::new(e.error)))?;

    // Create a damage tracker for efficient rendering.
    let damage_tracker = OutputDamageTracker::from_output(&output);

    // Map the output into the compositor space.
    state.space.map_output(&output, (0, 0));

    let mut calloop_data = CalloopData {
        state,
        display,
        backend,
        damage_tracker,
    };

    info!("entering winit main event loop");

    while running.load(Ordering::SeqCst) {
        // Dispatch Wayland clients.
        calloop_data
            .display
            .dispatch_clients(&mut calloop_data.state)
            .map_err(|e| WayRayError::EventLoop(Box::new(e)))?;

        calloop_data
            .display
            .flush_clients()
            .map_err(|e| WayRayError::EventLoop(Box::new(e)))?;

        // Dispatch calloop sources (Winit events + Wayland socket) with ~16ms timeout.
        event_loop
            .dispatch(Duration::from_millis(16), &mut calloop_data)
            .map_err(|e| WayRayError::EventLoop(Box::new(e)))?;
    }

    info!("winit backend shutting down");
    Ok(())
}

/// Render the compositor space to the Winit backend window.
///
/// Uses `OutputDamageTracker` for efficient re-rendering: only
/// damaged regions are redrawn each frame.
fn render_winit_frame(
    state: &mut WayRay,
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: &mut OutputDamageTracker,
) {
    let output = state.output.clone();

    // Get buffer age before bind (avoids borrow conflict).
    let age = backend.buffer_age().unwrap_or(0);

    // Render within a block so framebuffer is dropped before submit.
    let render_result = {
        let (renderer, mut framebuffer) = match backend.bind() {
            Ok(pair) => pair,
            Err(err) => {
                warn!(?err, "failed to bind winit backend for rendering");
                return;
            }
        };

        let custom_elements: &[TextureRenderElement<GlesTexture>] = &[];

        let render_result = render_output::<_, _, Window, _>(
            &output,
            renderer,
            &mut framebuffer,
            1.0,
            age,
            [&state.space],
            custom_elements,
            damage_tracker,
            CLEAR_COLOR,
        );

        match render_result {
            Ok(result) => {
                let damage = result.damage.cloned();

                // Verify framebuffer capture path works (will be consumed
                // by network transport in Phase 1).
                let output_size = state.output.current_mode().unwrap().size;
                let region: Rectangle<i32, BufferCoord> =
                    Rectangle::from_size(Size::from((output_size.w, output_size.h)));

                match renderer.copy_framebuffer(&framebuffer, region, Fourcc::Argb8888) {
                    Ok(mapping) => match renderer.map_texture(&mapping) {
                        Ok(pixels) => {
                            let damage_rects = damage.as_ref().map(|d| d.len()).unwrap_or(0);
                            tracing::debug!(
                                width = output_size.w,
                                height = output_size.h,
                                bytes = pixels.len(),
                                damage_rects,
                                "framebuffer captured"
                            );
                        }
                        Err(err) => {
                            tracing::warn!(?err, "failed to map framebuffer");
                        }
                    },
                    Err(err) => {
                        tracing::warn!(?err, "failed to copy framebuffer");
                    }
                }

                Ok(damage)
            }
            Err(err) => {
                warn!(?err, "damage tracker render failed");
                Err(())
            }
        }
    };

    if let Ok(damage) = render_result {
        let has_damage = damage.is_some();

        let submit_result = if let Some(ref rects) = damage {
            backend.submit(Some(rects))
        } else {
            backend.submit(None)
        };

        if let Err(err) = submit_result {
            warn!(?err, "failed to submit frame");
            return;
        }

        // Send frame callbacks to all mapped surfaces so clients
        // know they can draw the next frame.
        if has_damage {
            let time = state.clock.now();
            for window in state.space.elements() {
                window.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                    Some(output.clone())
                });
            }
        }
    }
}
