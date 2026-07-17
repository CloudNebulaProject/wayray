#[cfg(feature = "winit")]
use smithay::backend::input::{
    AbsolutePositionEvent, Event as InputEventTrait, InputBackend, InputEvent, KeyboardKeyEvent,
    PointerAxisEvent as PointerAxisEventTrait, PointerButtonEvent as PointerButtonEventTrait,
};
use smithay::{
    backend::input::{Axis, AxisSource, ButtonState},
    desktop::{Space, WindowSurfaceType},
    input::{
        Seat, SeatState,
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    output::Output,
    reexports::wayland_server::{Display, DisplayHandle},
    utils::{Clock, Monotonic, SERIAL_COUNTER},
    wayland::{
        compositor::CompositorState,
        output::OutputManagerState,
        selection::{
            data_device::{
                DataDeviceState, request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::PrimarySelectionState,
        },
        shell::xdg::{XdgShellState, decoration::XdgDecorationState},
        shm::ShmState,
    },
};
use tracing::{debug, info, warn};
use wayray_protocol::messages::{
    ClipboardData, ClipboardOffer, ControlMessage, InputMessage, cap_clipboard_payload,
    clipboard_offer_mimes, preferred_mime_type,
};

use crate::network::CompositorToNet;

use crate::wm::{self, WmState};

/// Central compositor state holding all Smithay subsystem states.
///
/// This is the "god struct" pattern required by Smithay — a single type that
/// implements all handler traits and holds all protocol global state.
pub struct WayRay {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub space: Space<smithay::desktop::Window>,
    pub seat: Seat<Self>,
    pub clock: Clock<Monotonic>,
    pub output: Output,
    /// Window management state — delegates to built-in or external WM.
    pub wm_state: WmState,
    /// Maps Smithay Window to WindowId for WM communication.
    pub window_ids: Vec<(wm::types::WindowId, smithay::desktop::Window)>,
    /// Counter for allocating WindowIds.
    next_window_id: u64,
    /// evdev keycodes currently held down via the network input stream. The
    /// compositor's keyboard state lives on the seat and outlives any single
    /// client connection, so a key still down when a client drops would stay
    /// "pressed" into the resumed session. We track held keys here and release
    /// them on disconnect so a (re)connecting client always starts clean.
    pressed_keys: std::collections::HashSet<u32>,
    /// Channel to the network thread for forwarding clipboard control
    /// messages to the connected remote client. `None` when the backend has
    /// no network path (e.g. the winit development backend).
    pub clipboard_tx: Option<std::sync::mpsc::Sender<CompositorToNet>>,
    /// Mime types of a selection freshly set by a Wayland client, awaiting a
    /// read on the next event-loop turn. Deferred because Smithay invokes
    /// `SelectionHandler::new_selection` *before* the seat's selection state
    /// is updated, so the data cannot be requested from within the handler.
    pub pending_selection: Option<Vec<String>>,
    /// Clipboard payload received from the remote client, re-offered to
    /// Wayland apps as the compositor-side selection. Served byte-for-byte to
    /// any of the advertised text flavors in `send_selection`.
    pub remote_clipboard: Option<(String, Vec<u8>)>,
    // Kept alive to maintain their Wayland globals — not accessed directly.
    _output_manager_state: OutputManagerState,
    _xdg_decoration_state: XdgDecorationState,
}

impl WayRay {
    /// Create a new WayRay compositor state, initializing all Smithay subsystems.
    pub fn new(display: &mut Display<Self>, output: Output) -> Self {
        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "wayray");

        seat.add_keyboard(Default::default(), 200, 25)
            .expect("failed to add keyboard to seat");
        seat.add_pointer();

        info!("all Smithay subsystem states initialized");

        let output_mode = output.current_mode().unwrap();
        let mut wm_state = WmState::new(output_mode.size.w, output_mode.size.h);

        // Register the WM protocol global for external window managers.
        let wm_protocol = wm::protocol::WmProtocolState::new::<Self>(&dh);
        wm_state.init_protocol(wm_protocol);

        Self {
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            space: Space::default(),
            seat,
            clock: Clock::new(),
            output,
            wm_state,
            window_ids: Vec::new(),
            next_window_id: 1,
            pressed_keys: std::collections::HashSet::new(),
            clipboard_tx: None,
            pending_selection: None,
            remote_clipboard: None,
            _output_manager_state: OutputManagerState::new_with_xdg_output::<Self>(&dh),
            _xdg_decoration_state: XdgDecorationState::new::<Self>(&dh),
        }
    }

    /// Allocate a new WindowId and associate it with a Smithay Window.
    pub fn register_window(&mut self, window: smithay::desktop::Window) -> wm::types::WindowId {
        let id = wm::types::WindowId::from_raw(self.next_window_id);
        self.next_window_id += 1;
        self.window_ids.push((id, window));
        id
    }

    /// Find the WindowId for a Smithay Window.
    pub fn window_id_for(&self, window: &smithay::desktop::Window) -> Option<wm::types::WindowId> {
        self.window_ids
            .iter()
            .find(|(_, w)| w == window)
            .map(|(id, _)| *id)
    }

    /// Find the Smithay Window for a WindowId.
    #[allow(dead_code)]
    pub fn window_for_id(&self, id: wm::types::WindowId) -> Option<&smithay::desktop::Window> {
        self.window_ids
            .iter()
            .find(|(wid, _)| *wid == id)
            .map(|(_, w)| w)
    }

    /// Remove a window from the id mapping.
    #[allow(dead_code)]
    pub fn unregister_window(&mut self, window: &smithay::desktop::Window) {
        if let Some(id) = self.window_id_for(window) {
            self.wm_state.active_wm().on_close_toplevel(id);
            self.window_ids.retain(|(_, w)| w != window);
        }
    }

    /// Apply WM render commands to the Space before frame rendering.
    pub fn apply_wm_render_commands(&mut self) {
        let output_name = self.output.name();
        let ids: Vec<_> = self.window_ids.iter().map(|(id, _)| *id).collect();
        let commands = self.wm_state.active_wm().on_render(&ids);

        for cmd in commands {
            // Apply workspace/tag visibility filtering when a protocol WM state
            // exists. The built-in WM assigns no workspaces/tags, so unassigned
            // windows stay visible (is_visible returns true) and behavior is
            // unchanged.
            let visible = cmd.visible
                && self
                    .wm_state
                    .protocol
                    .as_ref()
                    .is_none_or(|p| p.workspace_visible(cmd.id, &output_name));

            if let Some(window) = self
                .window_ids
                .iter()
                .find(|(id, _)| *id == cmd.id)
                .map(|(_, w)| w.clone())
            {
                if visible {
                    self.space.map_element(window, cmd.position, false);
                } else {
                    self.space.unmap_elem(&window);
                }
            }
        }
    }

    /// Process an input event from the backend and forward it to the appropriate
    /// Smithay seat device (keyboard or pointer).
    /// Only used by the Winit backend for local input processing.
    #[cfg(feature = "winit")]
    pub fn process_input_event<B: InputBackend>(&mut self, event: InputEvent<B>) {
        match event {
            InputEvent::Keyboard { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let keyboard = self.seat.get_keyboard().unwrap();
                keyboard.input::<(), _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    event.time_msec(),
                    |_, _, _| FilterResult::Forward,
                );
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let output_size = self.output.current_mode().unwrap().size;
                let pos = event.position_transformed(output_size.to_logical(1));

                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();

                // Find the element (window) under the pointer and get its surface.
                let under = self.space.element_under(pos).and_then(|(window, loc)| {
                    window
                        .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                        .map(|(surface, surf_loc)| (surface, (surf_loc + loc).to_f64()))
                });

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();

                // On button press, focus the window under the pointer via WM.
                if event.state() == ButtonState::Pressed {
                    let pos = pointer.current_location();
                    if let Some(focus_window) = self
                        .space
                        .element_under(pos)
                        .map(|(w, _)| w.clone())
                        .and_then(|w| self.window_id_for(&w))
                        .and_then(|wid| self.wm_state.active_wm().on_pointer_focus(wid))
                        .and_then(|fid| {
                            self.window_ids
                                .iter()
                                .find(|(id, _)| *id == fid)
                                .map(|(_, w)| w.clone())
                        })
                    {
                        self.space.raise_element(&focus_window, true);
                        let keyboard = self.seat.get_keyboard().unwrap();
                        let wl_surface = focus_window.toplevel().map(|t| t.wl_surface().clone());
                        keyboard.set_focus(self, wl_surface, serial);
                    }
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        serial,
                        time: event.time_msec(),
                        button: event.button_code(),
                        state: event.state(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event } => {
                let pointer = self.seat.get_pointer().unwrap();

                let source = event.source();

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 3.0 / 120.0
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 3.0 / 120.0
                });

                let mut frame = AxisFrame::new(event.time_msec()).source(source);

                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(v120) = event.amount_v120(Axis::Horizontal) {
                        frame = frame.v120(Axis::Horizontal, v120 as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(v120) = event.amount_v120(Axis::Vertical) {
                        frame = frame.v120(Axis::Vertical, v120 as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if horizontal_amount == 0.0 {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if vertical_amount == 0.0 {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {} // Ignore other events
        }
    }

    /// Release every key currently held via the network input stream.
    ///
    /// Call this when a client disconnects: the seat keyboard state outlives the
    /// connection, so any key still down (a modifier mid-chord, an abrupt drop)
    /// would otherwise stay "pressed" into the resumed session — shifting or
    /// repeating subsequent input. Releasing them returns the keyboard to a
    /// clean state that matches a freshly-(re)connecting client, which holds
    /// nothing.
    pub fn release_all_keys(&mut self) {
        if self.pressed_keys.is_empty() {
            return;
        }
        let held: Vec<u32> = self.pressed_keys.drain().collect();
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        for keycode in held {
            let serial = SERIAL_COUNTER.next_serial();
            let time = self.clock.now().as_millis();
            keyboard.input::<(), _>(
                self,
                (keycode + 8).into(),
                smithay::backend::input::KeyState::Released,
                serial,
                time,
                |_, _, _| FilterResult::Forward,
            );
        }
    }

    /// Forward a Wayland-app selection to the remote client (server → client
    /// clipboard sync). Called once per event-loop turn by the backend, after
    /// client dispatch — by then the seat's selection state reflects the
    /// `new_selection` that queued `pending_selection`.
    ///
    /// The selection payload is read from the data-source fd on a short-lived
    /// bounded reader thread (a Wayland client controls when — and whether —
    /// it writes, so the compositor thread must never block on the pipe) and
    /// then sent to the network thread over the compositor→net channel.
    pub fn process_pending_clipboard(&mut self) {
        let Some(mime_types) = self.pending_selection.take() else {
            return;
        };
        let Some(tx) = self.clipboard_tx.clone() else {
            return; // no network path (winit dev backend)
        };
        let Some(mime_type) = preferred_mime_type(&mime_types).map(str::to_string) else {
            debug!("selection offers no mime types; nothing to forward");
            return;
        };

        let (reader, writer) = match std::io::pipe() {
            Ok(pair) => pair,
            Err(e) => {
                warn!(error = %e, "failed to create clipboard pipe");
                return;
            }
        };

        if let Err(e) = request_data_device_client_selection::<Self>(
            &self.seat,
            mime_type.clone(),
            writer.into(),
        ) {
            // E.g. the selection was replaced by a compositor-side one in the
            // meantime — nothing to forward.
            debug!(error = %e, "could not request selection contents");
            return;
        }

        std::thread::Builder::new()
            .name("wayray-clipboard-read".into())
            .spawn(move || read_selection_and_forward(reader, mime_type, mime_types, tx))
            .map_err(|e| warn!(error = %e, "failed to spawn clipboard reader thread"))
            .ok();
    }

    /// Re-offer clipboard data received from the remote client as the Wayland
    /// selection, so applications in the session can paste it. The payload is
    /// stored on the compositor and served from `send_selection` when an app
    /// asks for it.
    pub fn set_remote_clipboard(&mut self, dh: &DisplayHandle, mut clip: ClipboardData) {
        if cap_clipboard_payload(&mut clip.data, &clip.mime_type) {
            warn!(
                mime_type = %clip.mime_type,
                "remote clipboard payload exceeded the size cap and was truncated"
            );
        }
        let offer_mimes = clipboard_offer_mimes(&clip.mime_type);
        // Log only metadata: clipboard contents are user data.
        info!(
            mime_type = %clip.mime_type,
            bytes = clip.data.len(),
            "publishing remote clipboard as Wayland selection"
        );
        self.remote_clipboard = Some((clip.mime_type, clip.data));
        set_data_device_selection(dh, &self.seat, offer_mimes, ());
    }

    /// Inject an input event received from a network client into the
    /// compositor's seat, following the same patterns as `process_input_event`.
    pub fn inject_network_input(&mut self, msg: InputMessage) {
        match msg {
            InputMessage::Keyboard(ev) => {
                let pressed = matches!(ev.state, wayray_protocol::messages::KeyState::Pressed);

                // Check if an external WM wants this key.
                if let Some(proto) = &self.wm_state.protocol
                    && proto.check_key_binding(ev.keycode, 0, pressed)
                {
                    return;
                }

                // Check if the built-in WM wants this key (only on press).
                if pressed && self.wm_state.active_wm().on_key_binding(ev.keycode, 0) {
                    // Alt+F4: close the focused window.
                    if ev.keycode == crate::wm::floating::KEY_F4
                        && let Some(focused) = self.wm_state.builtin.focused()
                        && let Some(window) = self.window_for_id(focused).cloned()
                        && let Some(toplevel) = window.toplevel()
                    {
                        toplevel.send_close();
                    }
                    // Alt+Tab: focus the next window.
                    if ev.keycode == crate::wm::floating::KEY_TAB
                        && let Some(new_focus) = self.wm_state.builtin.focused()
                        && let Some(window) = self.window_for_id(new_focus).cloned()
                    {
                        self.space.raise_element(&window, true);
                        let serial = SERIAL_COUNTER.next_serial();
                        let keyboard = self.seat.get_keyboard().unwrap();
                        let wl_surface = window.toplevel().map(|t| t.wl_surface().clone());
                        keyboard.set_focus(self, wl_surface, serial);
                    }
                    return;
                }

                let serial = SERIAL_COUNTER.next_serial();
                let keyboard = self.seat.get_keyboard().unwrap();
                let state = match ev.state {
                    wayray_protocol::messages::KeyState::Pressed => {
                        smithay::backend::input::KeyState::Pressed
                    }
                    wayray_protocol::messages::KeyState::Released => {
                        smithay::backend::input::KeyState::Released
                    }
                };
                // XKB keycodes = evdev scancode + 8
                keyboard.input::<(), _>(
                    self,
                    (ev.keycode + 8).into(),
                    state,
                    serial,
                    ev.time,
                    |_, _, _| FilterResult::Forward,
                );
                // Track held keys so we can release them if the client drops.
                if pressed {
                    self.pressed_keys.insert(ev.keycode);
                } else {
                    self.pressed_keys.remove(&ev.keycode);
                }
            }
            InputMessage::PointerMotion(ev) => {
                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();

                let pos = (ev.x, ev.y).into();

                let under = self.space.element_under(pos).and_then(|(window, loc)| {
                    window
                        .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                        .map(|(surface, surf_loc)| (surface, (surf_loc + loc).to_f64()))
                });

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: ev.time,
                    },
                );
                pointer.frame(self);
            }
            InputMessage::PointerButton(ev) => {
                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();

                let state = match ev.state {
                    wayray_protocol::messages::ButtonState::Pressed => ButtonState::Pressed,
                    wayray_protocol::messages::ButtonState::Released => ButtonState::Released,
                };

                // Click-to-focus on button press — delegate to WM.
                if state == ButtonState::Pressed {
                    let pos = pointer.current_location();
                    if let Some(focus_window) = self
                        .space
                        .element_under(pos)
                        .map(|(w, _)| w.clone())
                        .and_then(|w| self.window_id_for(&w))
                        .and_then(|wid| self.wm_state.active_wm().on_pointer_focus(wid))
                        .and_then(|fid| {
                            self.window_ids
                                .iter()
                                .find(|(id, _)| *id == fid)
                                .map(|(_, w)| w.clone())
                        })
                    {
                        self.space.raise_element(&focus_window, true);
                        let keyboard = self.seat.get_keyboard().unwrap();
                        let wl_surface = focus_window.toplevel().map(|t| t.wl_surface().clone());
                        keyboard.set_focus(self, wl_surface, serial);
                    }
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        serial,
                        time: ev.time,
                        button: ev.button,
                        state,
                    },
                );
                pointer.frame(self);
            }
            InputMessage::PointerAxis(ev) => {
                let pointer = self.seat.get_pointer().unwrap();

                let axis = match ev.axis {
                    wayray_protocol::messages::Axis::Horizontal => Axis::Horizontal,
                    wayray_protocol::messages::Axis::Vertical => Axis::Vertical,
                };

                let mut frame = AxisFrame::new(ev.time).source(AxisSource::Wheel);

                if ev.value != 0.0 {
                    frame = frame.value(axis, ev.value);
                }

                pointer.axis(self, frame);
                pointer.frame(self);
            }
        }
    }
}

/// Read a selection payload from the data-source pipe (bounded by the
/// protocol clipboard cap) and forward it to the network thread as a
/// `ClipboardOffer` + `ClipboardData` pair. Runs on a dedicated short-lived
/// thread; the read ends when the source finishes writing and closes its end
/// of the pipe (or once the cap is exceeded).
fn read_selection_and_forward(
    mut reader: std::io::PipeReader,
    mime_type: String,
    offered_mime_types: Vec<String>,
    tx: std::sync::mpsc::Sender<CompositorToNet>,
) {
    use std::io::Read;
    use wayray_protocol::messages::MAX_CLIPBOARD_DATA;

    // Read at most cap + 1 bytes: the extra byte tells truncation from an
    // exactly-cap-sized payload. Dropping the reader early closes the pipe,
    // so an oversized source gets EPIPE instead of filling the kernel buffer.
    let mut data = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    while data.len() <= MAX_CLIPBOARD_DATA {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                debug!(error = %e, "clipboard pipe read failed");
                return;
            }
        }
    }

    if data.is_empty() {
        debug!("selection source wrote no data; nothing to forward");
        return;
    }
    if cap_clipboard_payload(&mut data, &mime_type) {
        warn!(
            mime_type = %mime_type,
            "selection exceeded the clipboard size cap and was truncated"
        );
    }

    // Contents are user data — log only metadata.
    debug!(mime_type = %mime_type, bytes = data.len(), "forwarding selection to client");
    let offer = ControlMessage::ClipboardOffer(ClipboardOffer {
        mime_types: offered_mime_types,
    });
    let payload = ControlMessage::ClipboardData(ClipboardData { mime_type, data });
    if tx.send(CompositorToNet::SendControl(offer)).is_err()
        || tx.send(CompositorToNet::SendControl(payload)).is_err()
    {
        debug!("network thread gone; clipboard forward dropped");
    }
}
