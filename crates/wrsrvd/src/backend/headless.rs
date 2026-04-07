use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use miette::Result;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Offscreen,
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
use wayray_protocol::encoding;
use wayray_protocol::messages::FrameUpdate;

use crate::errors::WayRayError;
use crate::handlers::ClientState;
use crate::network::{CompositorToNet, NetToCompositor, NetworkHandle};
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
    /// Network handle for sending frames and receiving input.
    net_handle: NetworkHandle,
    /// Previous frame's pixel data for XOR diff encoding.
    previous_frame: Vec<u8>,
    /// Frame sequence counter.
    frame_sequence: u64,
    /// Whether a remote client is currently connected.
    client_connected: bool,
}

/// Run the compositor with the headless PixmanRenderer backend.
///
/// This creates a CPU-only software renderer suitable for headless servers
/// with no display hardware.
pub fn run(
    display: Display<WayRay>,
    mut state: WayRay,
    output: Output,
    net_handle: NetworkHandle,
) -> Result<()> {
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

    // Initialize previous frame buffer (all zeros = black).
    // Use the pixman Image's stride for correct row alignment.
    let frame_stride = render_buffer.stride();
    let frame_size = frame_stride * output_size.h as usize;
    let previous_frame = vec![0u8; frame_size];

    let mut calloop_data = CalloopData {
        state,
        display,
        renderer,
        render_buffer,
        damage_tracker,
        net_handle,
        previous_frame,
        frame_sequence: 0,
        client_connected: false,
    };

    let running = Arc::new(AtomicBool::new(true));

    // Handle SIGINT/SIGTERM for graceful shutdown.
    let running_clone = running.clone();
    ctrlc_handler(&running_clone);

    info!("entering headless main event loop");

    while running.load(Ordering::SeqCst) {
        // Drain network events (input from remote clients, connection state).
        drain_network_events(&mut calloop_data);

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
    // Force exit — the network thread may be blocking on accept().
    // Proper graceful shutdown with tokio CancellationToken is a future improvement.
    std::process::exit(0);
}

/// Drain all pending network events (input, connection changes).
fn drain_network_events(data: &mut CalloopData) {
    use std::sync::mpsc::TryRecvError;

    loop {
        match data.net_handle.rx.try_recv() {
            Ok(NetToCompositor::Input(input_msg)) => {
                data.state.inject_network_input(input_msg);
            }
            Ok(NetToCompositor::ClientConnected(hello)) => {
                info!(
                    version = hello.version,
                    capabilities = ?hello.capabilities,
                    "remote client connected"
                );
                data.client_connected = true;
            }
            Ok(NetToCompositor::ClientDisconnected) => {
                info!("remote client disconnected");
                data.client_connected = false;
            }
            Ok(NetToCompositor::Control(ctrl)) => {
                tracing::debug!(?ctrl, "received control message");
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                warn!("network channel disconnected");
                data.client_connected = false;
                break;
            }
        }
    }
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

    // Refresh the space — updates output-to-element mappings.
    // Must be called each frame before rendering.
    data.state.space.refresh();

    // Apply WM render phase — positions/z-order before frame capture.
    // If an external WM is connected, trigger the render phase protocol
    // and apply its commands instead of the built-in WM's.
    if let Some(proto) = &mut data.state.wm_state.protocol {
        if proto.is_wm_connected() {
            proto.start_render_phase();
            // Note: The external WM responds via protocol dispatch in the
            // next display.dispatch_clients() call. For this frame, apply
            // any commands accumulated from previous dispatches.
            let commands = proto.take_render_commands();
            for cmd in commands {
                if let Some(window) = data
                    .state
                    .window_ids
                    .iter()
                    .find(|(id, _)| *id == cmd.id)
                    .map(|(_, w)| w.clone())
                {
                    if cmd.visible {
                        data.state.space.map_element(window, cmd.position, false);
                    } else {
                        data.state.space.unmap_elem(&window);
                    }
                }
            }
        } else {
            data.state.apply_wm_render_commands();
        }
    } else {
        data.state.apply_wm_render_commands();
    }

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
            let has_damage = result.damage.is_some();
            let damage = result.damage.cloned();
            tracing::debug!(has_damage, "render complete");

            // Drop the render target to release the borrow on the buffer.
            drop(target);

            // Read pixels directly from the pixman Image's CPU memory.
            // Use the Image's stride (bytes per row) which may include padding.
            let output_size = data.state.output.current_mode().unwrap().size;
            let stride = data.render_buffer.stride();
            let frame_bytes = stride * output_size.h as usize;
            let pixels = unsafe {
                let ptr = data.render_buffer.data() as *const u8;
                std::slice::from_raw_parts(ptr, frame_bytes)
            };

            // Send frame over network if a client is connected.
            if data.client_connected {
                send_frame_to_network(data, pixels, &damage, output_size.w, output_size.h, stride);
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

/// Encode the current frame as XOR diff against the previous frame and
/// send it to the connected client via the network channel.
fn send_frame_to_network(
    data: &mut CalloopData,
    current_pixels: &[u8],
    damage: &Option<Vec<Rectangle<i32, smithay::utils::Physical>>>,
    width: i32,
    height: i32,
    stride: usize,
) {
    // Compute XOR diff.
    let diff = encoding::xor_diff(current_pixels, &data.previous_frame);

    // Encode damaged regions. If damage tracking reports specific rects, use those;
    // otherwise encode the full frame as a single region.
    let regions: Vec<_> = match damage {
        Some(rects) if !rects.is_empty() => rects
            .iter()
            .map(|rect| {
                encoding::encode_region(
                    &diff,
                    stride,
                    rect.loc.x.max(0) as u32,
                    rect.loc.y.max(0) as u32,
                    (rect.size.w.min(width - rect.loc.x.max(0))).max(0) as u32,
                    (rect.size.h.min(height - rect.loc.y.max(0))).max(0) as u32,
                )
            })
            .filter(|r| r.width > 0 && r.height > 0)
            .collect(),
        _ => {
            // Full-frame update.
            vec![encoding::encode_region(
                &diff,
                stride,
                0,
                0,
                width as u32,
                height as u32,
            )]
        }
    };

    data.frame_sequence += 1;
    let frame_update = FrameUpdate {
        sequence: data.frame_sequence,
        regions,
    };

    tracing::debug!(
        sequence = data.frame_sequence,
        regions = frame_update.regions.len(),
        "sending frame to client"
    );

    if data
        .net_handle
        .tx
        .send(CompositorToNet::SendFrame(frame_update))
        .is_err()
    {
        warn!("failed to send frame to network thread");
        data.client_connected = false;
    }

    // Store current frame as previous for next diff.
    data.previous_frame.clear();
    data.previous_frame.extend_from_slice(current_pixels);
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
