use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use miette::Result;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, ExportMem, Offscreen,
            damage::OutputDamageTracker,
            element::texture::TextureRenderElement,
            pixman::{PixmanRenderer, PixmanTexture},
        },
    },
    desktop::{Window, space::render_output},
    output::Output,
    reexports::pixman as pixman_lib,
    reexports::{
        calloop::{self, EventLoop},
        wayland_server::Display,
    },
    utils::{Buffer as BufferCoord, Rectangle, Size},
    wayland::{compositor::CompositorClientState, socket::ListeningSocketSource},
};
use tracing::{info, warn};

use crate::errors::WayRayError;
use crate::handlers::ClientState;
use crate::state::WayRay;

/// Dark grey clear color for the compositor background.
const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

/// Data accessible from calloop event callbacks in the headless backend.
struct CalloopData {
    state: WayRay,
    display: Display<WayRay>,
    renderer: PixmanRenderer,
    render_buffer: pixman_lib::Image<'static, 'static>,
    damage_tracker: OutputDamageTracker,
}

/// Run the compositor with the headless PixmanRenderer backend.
///
/// This creates a CPU-only software renderer suitable for headless servers
/// with no display hardware.
pub fn run(display: Display<WayRay>, mut state: WayRay, output: Output) -> Result<()> {
    // Create the PixmanRenderer (CPU software renderer).
    let mut renderer = PixmanRenderer::new().map_err(|e| {
        WayRayError::BackendInit(Box::<dyn std::error::Error + Send + Sync>::from(
            e.to_string(),
        ))
    })?;
    info!("pixman headless backend initialized");

    // Create an in-memory render buffer matching the output size.
    let output_size = output.current_mode().unwrap().size;
    let buffer_size: Size<i32, BufferCoord> = Size::from((output_size.w, output_size.h));
    let render_buffer: pixman_lib::Image<'static, 'static> = renderer
        .create_buffer(Fourcc::Argb8888, buffer_size)
        .map_err(|e| {
            WayRayError::BackendInit(Box::<dyn std::error::Error + Send + Sync>::from(
                e.to_string(),
            ))
        })?;
    info!(
        width = output_size.w,
        height = output_size.h,
        "headless render buffer created"
    );

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
    let mut event_loop: EventLoop<CalloopData> =
        EventLoop::try_new().map_err(|e| WayRayError::EventLoop(Box::new(e)))?;

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

    // Set up a timer to drive the render loop at ~60fps.
    let timer = calloop::timer::Timer::from_duration(Duration::from_millis(16));
    loop_handle
        .insert_source(timer, |_, _, data| {
            render_headless_frame(data);
            calloop::timer::TimeoutAction::ToDuration(Duration::from_millis(16))
        })
        .map_err(|e| WayRayError::EventLoop(Box::new(e.error)))?;

    // Create a damage tracker for efficient rendering.
    let damage_tracker = OutputDamageTracker::from_output(&output);

    // Map the output into the compositor space.
    state.space.map_output(&output, (0, 0));

    let mut calloop_data = CalloopData {
        state,
        display,
        renderer,
        render_buffer,
        damage_tracker,
    };

    let running = Arc::new(AtomicBool::new(true));

    // Handle SIGINT/SIGTERM for graceful shutdown.
    let running_clone = running.clone();
    ctrlc_handler(&running_clone);

    info!("entering headless main event loop");

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

        // Dispatch calloop sources (timer + Wayland socket) with ~16ms timeout.
        event_loop
            .dispatch(Duration::from_millis(16), &mut calloop_data)
            .map_err(|e| WayRayError::EventLoop(Box::new(e)))?;
    }

    info!("headless backend shutting down");
    Ok(())
}

/// Render a single frame using the headless PixmanRenderer.
fn render_headless_frame(data: &mut CalloopData) {
    let output = data.state.output.clone();

    // Bind the in-memory buffer as the render target.
    let mut target = match data.renderer.bind(&mut data.render_buffer) {
        Ok(target) => target,
        Err(err) => {
            warn!(?err, "failed to bind headless render buffer");
            return;
        }
    };

    let custom_elements: &[TextureRenderElement<PixmanTexture>] = &[];

    let render_result = render_output::<_, _, Window, _>(
        &output,
        &mut data.renderer,
        &mut target,
        1.0,
        0, // buffer age: 0 means full redraw (no swap chain in headless)
        [&data.state.space],
        custom_elements,
        &mut data.damage_tracker,
        CLEAR_COLOR,
    );

    match render_result {
        Ok(result) => {
            let damage = result.damage.cloned();

            // Read pixels from the CPU buffer for network transport.
            let output_size = data.state.output.current_mode().unwrap().size;
            let region: Rectangle<i32, BufferCoord> =
                Rectangle::from_size(Size::from((output_size.w, output_size.h)));

            match data
                .renderer
                .copy_framebuffer(&target, region, Fourcc::Argb8888)
            {
                Ok(mapping) => match data.renderer.map_texture(&mapping) {
                    Ok(pixels) => {
                        let damage_rects = damage.as_ref().map(|d| d.len()).unwrap_or(0);
                        tracing::debug!(
                            width = output_size.w,
                            height = output_size.h,
                            bytes = pixels.len(),
                            damage_rects,
                            "headless framebuffer captured"
                        );
                    }
                    Err(err) => {
                        tracing::warn!(?err, "failed to map headless framebuffer");
                    }
                },
                Err(err) => {
                    tracing::warn!(?err, "failed to copy headless framebuffer");
                }
            }

            // Send frame callbacks to all mapped surfaces.
            let time = data.state.clock.now();
            for window in data.state.space.elements() {
                window.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                    Some(output.clone())
                });
            }
        }
        Err(err) => {
            warn!(?err, "headless damage tracker render failed");
        }
    }
}

/// Install a Ctrl-C handler that sets the running flag to false.
fn ctrlc_handler(running: &Arc<AtomicBool>) {
    let r = running.clone();
    // Ignore error if handler can't be set (e.g., in tests).
    let _ = ctrlc::set_handler(move || {
        info!("received shutdown signal");
        r.store(false, Ordering::SeqCst);
    });
}
