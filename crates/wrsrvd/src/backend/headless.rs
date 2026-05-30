use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

use wayray_protocol::cluster::ClusterConfig;
use wayray_protocol::tls::PinStore;
use wayray_protocol::tls::verifier::PinPolicy;

use crate::errors::WayRayError;
use crate::handlers::ClientState;
use crate::network::{
    CompositorToNet, ConnectDecision, NetToCompositor, NetworkHandle, SessionBinding,
};
use crate::session::{SessionLocation, SessionRegistry, SessionState, SessionToken};
use crate::state::WayRay;

/// Local load factor above which the server tries to shed *new* sessions to a
/// less-loaded cluster peer. A guess pending deployment sizing.
const LOAD_REDIRECT_THRESHOLD: f32 = 0.9;

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
    /// Session registry tracking all sessions by token.
    session_registry: SessionRegistry,
    /// The currently active session ID (if any).
    active_session: Option<crate::session::SessionId>,
    /// Static cluster configuration for multi-server redirect / placement.
    cluster: ClusterConfig,
    /// Small tokio runtime used for short-lived peer `ServerInfo` queries
    /// during new-session placement. `None` for single-server deployments.
    peer_rt: Option<tokio::runtime::Runtime>,
    /// Trust-on-first-use pin store for peer certificates, used when a peer has
    /// no fingerprint configured in `cluster.toml`.
    peer_pin_store: Arc<Mutex<PinStore>>,
}

/// Maximum wall-clock the compositor thread will spend probing peers for a
/// single connecting client. Bounds how long the calloop loop (rendering +
/// input) can be blocked by cluster placement, independent of peer count.
const PEER_PROBE_BUDGET: Duration = Duration::from_millis(250);

/// Build the certificate-pin policy for a peer: a configured fingerprint is
/// enforced strictly, otherwise trust-on-first-use against the shared store.
fn peer_pin_policy(data: &CalloopData, server_id: &str, addr: &str) -> PinPolicy {
    match data.cluster.fingerprint_of(server_id) {
        Some(fp) => PinPolicy::Fixed(fp.to_string()),
        None => PinPolicy::Tofu {
            identity: addr.to_string(),
            store: data.peer_pin_store.clone(),
        },
    }
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
    cluster: ClusterConfig,
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

    // Build the session registry knowing this server's cluster identity and
    // capacity. The capacity comes from the local server entry's weight scaling
    // (kept simple here: default capacity unless a local entry exists).
    let local_id = cluster.local_id.clone();
    let session_registry =
        SessionRegistry::with_cluster(local_id.clone(), wayray_protocol::cluster::DEFAULT_CAPACITY);

    // A dedicated tokio runtime for short-lived peer `ServerInfo` probes; only
    // needed in a real cluster. Single-server deployments skip it entirely.
    let peer_rt = if cluster.is_clustered() {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => Some(rt),
            Err(e) => {
                warn!(error = %e, "failed to build peer-query runtime; load balancing disabled");
                None
            }
        }
    } else {
        None
    };

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
        session_registry,
        active_session: None,
        cluster,
        peer_rt,
        peer_pin_store: Arc::new(Mutex::new(PinStore::open_default().unwrap_or_else(|e| {
            warn!(error = %e, "failed to open peer known_hosts; using temp store");
            PinStore::open(std::env::temp_dir().join("wayray-peer-known_hosts"))
                .expect("temp pin store")
        }))),
    };

    let running = Arc::new(AtomicBool::new(true));

    // Handle SIGINT/SIGTERM for graceful shutdown.
    let running_clone = running.clone();
    ctrlc_handler(&running_clone);

    info!("entering headless main event loop");

    let mut cleanup_counter: u32 = 0;

    while running.load(Ordering::SeqCst) {
        // Drain network events (input from remote clients, connection state).
        drain_network_events(&mut calloop_data);

        // Periodically clean up expired suspended sessions (~every 60s at 60fps).
        cleanup_counter += 1;
        if cleanup_counter >= 3600 {
            cleanup_counter = 0;
            let expired = calloop_data.session_registry.cleanup_expired();
            if !expired.is_empty() {
                calloop_data.session_registry.purge_destroyed();
            }
        }

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
            Ok(NetToCompositor::ClientConnected { hello, reply }) => {
                // Time the resolve so we can validate the <500ms hot-desk
                // reconnect target end-to-end (the network thread blocks on
                // this reply before composing the ServerHello).
                let connect_received = std::time::Instant::now();

                let token_str = hello.token.clone().unwrap_or_else(|| {
                    // Generate a cryptographically random token if the client
                    // didn't provide one. The token is the sole session
                    // credential, so it must be unguessable: a sequential or
                    // count-based scheme would let an attacker enumerate and
                    // hijack other clients' tokenless sessions.
                    generate_random_token()
                });
                let token = SessionToken::new(token_str);

                // Multi-server affinity: if this token's session is known to
                // live on another server, redirect the client there instead of
                // creating a duplicate local session. When the token is not
                // known locally and we are clustered, probe peers on demand to
                // discover whether one of them hosts the session (and cache the
                // answer in the registry's remote map).
                let mut location = data.session_registry.locate(&token);
                if location == SessionLocation::Unknown
                    && let Some(server_id) = discover_remote_home(data, &token)
                {
                    data.session_registry
                        .note_remote_session(token.clone(), server_id.clone());
                    location = SessionLocation::Remote(server_id);
                }
                if let SessionLocation::Remote(server_id) = location {
                    if let Some(addr) = data.cluster.addr_of(&server_id) {
                        info!(%token, %server_id, %addr, "redirecting client to home server");
                        let _ = reply.send(ConnectDecision::Redirect {
                            server_id,
                            addr: addr.to_string(),
                        });
                        // Do not touch local session state; client reconnects
                        // elsewhere.
                        continue;
                    } else {
                        warn!(%server_id, "session marked remote but server addr unknown; serving locally");
                    }
                }

                // Resolve the session: resume a suspended one, rebind ("steal")
                // an active one whose old endpoint vanished, or create new.
                let (session_id, resumed) = match data.session_registry.is_resumable(&token) {
                    Some(id) => {
                        let state = data
                            .session_registry
                            .get(id)
                            .map(|s| s.state)
                            .unwrap_or(SessionState::Destroyed);
                        match state {
                            SessionState::Suspended => {
                                if let Err(e) = data.session_registry.activate(id) {
                                    warn!(error = %e, "failed to resume session");
                                } else {
                                    info!(%id, "session resumed");
                                }
                            }
                            SessionState::Active => {
                                // Hot-desk steal: the previous endpoint never
                                // sent a clean disconnect, so a new endpoint
                                // presenting the same token rebinds the live
                                // session. This is authorized purely by
                                // possession of the (unguessable, TLS-protected)
                                // token — the SunRay smart-card model — so log it
                                // as a security-relevant takeover for auditing.
                                warn!(
                                    %id,
                                    "active session rebound to a new endpoint (hot-desk takeover); \
                                     authorized by session token"
                                );
                            }
                            _ => {}
                        }
                        (id, true)
                    }
                    None => {
                        // NEW session: consider load-balancing to a peer before
                        // creating it locally. Only attempts a redirect in a
                        // real cluster when local load is over threshold.
                        if let Some((server_id, addr)) = pick_peer_for_new_session(data) {
                            info!(%token, %server_id, %addr, "load-balancing new session to peer");
                            let _ = reply.send(ConnectDecision::Redirect { server_id, addr });
                            continue;
                        }

                        let id = data.session_registry.create_session(token.clone());
                        if let Err(e) = data.session_registry.activate(id) {
                            warn!(error = %e, "failed to activate new session");
                        }
                        (id, false)
                    }
                };

                data.active_session = Some(session_id);
                data.client_connected = true;

                // Publish the live active-session count and resumable token set
                // so peer ServerInfo / SessionLookup queries see an accurate
                // load and session location.
                publish_cluster_state(data);

                // Reply to the network thread with the resolved binding so it
                // can compose an accurate ServerHello.
                if reply
                    .send(ConnectDecision::Bind(SessionBinding {
                        session_id: session_id.raw(),
                        resumed,
                        token: token.0.clone(),
                    }))
                    .is_err()
                {
                    warn!("network thread dropped session binding reply channel");
                }

                // The token is a credential and is never logged in the clear.
                info!(
                    version = hello.version,
                    %session_id,
                    resumed,
                    resolve_us = connect_received.elapsed().as_micros() as u64,
                    "remote client connected"
                );
            }
            Ok(NetToCompositor::ClientDisconnected) => {
                // Suspend the session instead of destroying it.
                if let Some(session_id) = data.active_session.take() {
                    if let Err(e) = data.session_registry.suspend(session_id) {
                        warn!(error = %e, "failed to suspend session");
                    } else {
                        info!(%session_id, "session suspended, waiting for reconnect");
                    }
                }
                data.client_connected = false;
                // Refresh the published load/location state for peer queries.
                // A suspended session is still resumable, so it stays in the
                // published token set and a reconnecting client elsewhere is
                // redirected home.
                publish_cluster_state(data);
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

/// Publish this server's current load and resumable-token set to the network
/// thread so peers get accurate answers to `ServerInfo` and `SessionLookup`
/// probes. Cheap; called whenever session state changes.
fn publish_cluster_state(data: &CalloopData) {
    data.net_handle
        .set_active_sessions(data.session_registry.count_by_state(SessionState::Active) as u32);
    data.net_handle
        .set_local_tokens(data.session_registry.resumable_tokens());
}

/// Probe cluster peers to find which one (if any) hosts a resumable session for
/// `token`. Returns the home server's id on the first positive answer.
///
/// Only runs in a real cluster; single-server deployments and unconfigured
/// peers short-circuit to `None`. This is the runtime data source that
/// populates the registry's remote-session map so a server receiving a client
/// for a token it does not host can redirect to the session's home server
/// instead of creating a duplicate.
fn discover_remote_home(data: &CalloopData, token: &SessionToken) -> Option<String> {
    if !data.cluster.is_clustered() {
        return None;
    }
    if data.cluster.cluster_secret.is_empty() {
        warn!("cluster configured without cluster_secret; skipping peer session lookup");
        return None;
    }
    let rt = data.peer_rt.as_ref()?;
    let secret = data.cluster.cluster_secret.clone();

    let candidates: Vec<(String, String)> = data
        .cluster
        .peers()
        .map(|s| (s.id.clone(), s.addr.clone()))
        .collect();

    // Bound total probe time so a few dead peers cannot stall the event loop.
    let deadline = Instant::now() + PEER_PROBE_BUDGET;
    for (id, addr) in candidates {
        if Instant::now() >= deadline {
            warn!("peer session-lookup budget exhausted; serving locally");
            break;
        }
        let socket: std::net::SocketAddr = match addr.parse() {
            Ok(s) => s,
            Err(e) => {
                warn!(%id, %addr, error = %e, "invalid peer addr, skipping lookup");
                continue;
            }
        };
        let policy = peer_pin_policy(data, &id, &addr);
        if let Some(server_id) = rt.block_on(crate::network::query_peer_for_token(
            socket, &token.0, &secret, policy,
        )) {
            info!(%server_id, "discovered session home server via peer probe");
            return Some(server_id);
        }
    }
    None
}

/// A peer's advertised load, used to choose a target for a new session.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PeerLoad {
    /// Index into the candidate list (so we can map back to id/addr).
    index: usize,
    /// active_sessions / capacity.
    load_factor: f32,
    /// Placement weight (higher = more preferred). Effective score divides the
    /// load by the weight so heavier servers tolerate more sessions.
    weight: u32,
}

/// Choose the best peer for a new session from advertised loads. Returns the
/// index of the least-loaded (weight-adjusted) peer that is below 1.0, or
/// `None` if every peer is at or above capacity.
///
/// Pure function so the placement policy is unit-testable without QUIC.
fn select_least_loaded_peer(peers: &[PeerLoad]) -> Option<usize> {
    peers
        .iter()
        .filter(|p| p.load_factor < 1.0)
        .min_by(|a, b| {
            let sa = a.load_factor / a.weight.max(1) as f32;
            let sb = b.load_factor / b.weight.max(1) as f32;
            sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| p.index)
}

/// Decide whether a NEW session should be redirected to a cluster peer for
/// load balancing. Returns `Some((server_id, addr))` to redirect, or `None` to
/// create the session locally.
///
/// Only acts when (a) a real cluster is configured, (b) the local load factor
/// exceeds [`LOAD_REDIRECT_THRESHOLD`], and (c) a peer advertises a lower load
/// via a `ServerInfo` query. Single-server deployments always return `None`.
fn pick_peer_for_new_session(data: &mut CalloopData) -> Option<(String, String)> {
    if !data.cluster.is_clustered() {
        return None;
    }
    if data.session_registry.load_factor() <= LOAD_REDIRECT_THRESHOLD {
        return None;
    }
    if data.cluster.cluster_secret.is_empty() {
        warn!("cluster configured without cluster_secret; skipping peer load probe");
        return None;
    }
    let rt = data.peer_rt.as_ref()?;
    let secret = data.cluster.cluster_secret.clone();

    // Snapshot peer (id, addr) candidates.
    let candidates: Vec<(String, String)> = data
        .cluster
        .peers()
        .map(|s| (s.id.clone(), s.addr.clone()))
        .collect();
    if candidates.is_empty() {
        return None;
    }

    // Query each peer's load (short-lived QUIC probes). Skip peers that fail
    // to resolve or do not answer in time, and stop once the probe budget is
    // spent so the event loop is never blocked for long.
    let deadline = Instant::now() + PEER_PROBE_BUDGET;
    let mut loads: Vec<PeerLoad> = Vec::new();
    for (index, (id, addr)) in candidates.iter().enumerate() {
        if Instant::now() >= deadline {
            warn!("peer load-probe budget exhausted; placing locally");
            break;
        }
        let socket: std::net::SocketAddr = match addr.parse() {
            Ok(s) => s,
            Err(e) => {
                warn!(%id, %addr, error = %e, "invalid peer addr, skipping");
                continue;
            }
        };
        let weight = data.cluster.server(id).map(|s| s.weight).unwrap_or(1);
        let policy = peer_pin_policy(data, id, addr);
        if let Some(info) = rt.block_on(crate::network::query_peer_info(socket, &secret, policy)) {
            let cap = info.capacity.max(1) as f32;
            loads.push(PeerLoad {
                index,
                load_factor: info.active_sessions as f32 / cap,
                weight,
            });
        }
    }

    let local_load = data.session_registry.load_factor();
    let best = select_least_loaded_peer(&loads)?;
    let best_load = loads.iter().find(|p| p.index == best)?;
    // Only redirect if the peer is actually less loaded than us.
    if best_load.load_factor < local_load {
        let (id, addr) = candidates[best].clone();
        Some((id, addr))
    } else {
        None
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
    // Output name drives per-output workspace visibility. Computed once here so
    // it can be used while `proto` is mutably borrowed below.
    let output_name = data.state.output.name();
    if let Some(proto) = &mut data.state.wm_state.protocol {
        if proto.is_wm_connected() {
            proto.start_render_phase();
            // Note: The external WM responds via protocol dispatch in the
            // next display.dispatch_clients() call. For this frame, apply
            // any commands accumulated from previous dispatches.
            let commands = proto.take_render_commands();
            // Snapshot workspace visibility per command id while `proto` is
            // borrowed, then apply to the Space.
            let resolved: Vec<_> = commands
                .into_iter()
                .map(|cmd| {
                    let visible = cmd.visible && proto.workspace_visible(cmd.id, &output_name);
                    (cmd, visible)
                })
                .collect();
            for (cmd, visible) in resolved {
                if let Some(window) = data
                    .state
                    .window_ids
                    .iter()
                    .find(|(id, _)| *id == cmd.id)
                    .map(|(_, w)| w.clone())
                {
                    if visible {
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

/// Generate a cryptographically random 16-byte session token, hex-encoded.
/// Used when a client connects without one. The value is the sole session
/// credential, so it must be unguessable.
fn generate_random_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("failed to generate random session token");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

#[cfg(test)]
mod tests {
    use super::{PeerLoad, select_least_loaded_peer};

    #[test]
    fn no_candidates_is_none() {
        assert_eq!(select_least_loaded_peer(&[]), None);
    }

    #[test]
    fn picks_least_loaded() {
        let peers = [
            PeerLoad {
                index: 0,
                load_factor: 0.8,
                weight: 1,
            },
            PeerLoad {
                index: 1,
                load_factor: 0.2,
                weight: 1,
            },
            PeerLoad {
                index: 2,
                load_factor: 0.5,
                weight: 1,
            },
        ];
        assert_eq!(select_least_loaded_peer(&peers), Some(1));
    }

    #[test]
    fn skips_peers_at_capacity() {
        let peers = [
            PeerLoad {
                index: 0,
                load_factor: 1.0,
                weight: 1,
            },
            PeerLoad {
                index: 1,
                load_factor: 1.5,
                weight: 1,
            },
        ];
        // Every peer is at or over capacity → nobody is eligible.
        assert_eq!(select_least_loaded_peer(&peers), None);
    }

    #[test]
    fn weight_biases_toward_heavier_servers() {
        // Both peers carry the same raw load, but peer 1 has twice the weight,
        // so its weight-adjusted score is lower and it should be preferred.
        let peers = [
            PeerLoad {
                index: 0,
                load_factor: 0.6,
                weight: 1,
            },
            PeerLoad {
                index: 1,
                load_factor: 0.6,
                weight: 2,
            },
        ];
        assert_eq!(select_least_loaded_peer(&peers), Some(1));
    }

    #[test]
    fn zero_weight_does_not_divide_by_zero() {
        // A weight of 0 is clamped to 1 internally; the call must not panic and
        // should still pick the only below-capacity peer.
        let peers = [PeerLoad {
            index: 3,
            load_factor: 0.4,
            weight: 0,
        }];
        assert_eq!(select_least_loaded_peer(&peers), Some(3));
    }
}
