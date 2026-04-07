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
    reexports::wayland_server::Display,
    utils::{Clock, Monotonic, SERIAL_COUNTER},
    wayland::{
        compositor::CompositorState,
        output::OutputManagerState,
        selection::{data_device::DataDeviceState, primary_selection::PrimarySelectionState},
        shell::xdg::{XdgShellState, decoration::XdgDecorationState},
        shm::ShmState,
    },
};
use tracing::info;
use wayray_protocol::messages::InputMessage;

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
        let ids: Vec<_> = self.window_ids.iter().map(|(id, _)| *id).collect();
        let commands = self.wm_state.active_wm().on_render(&ids);

        for cmd in commands {
            if let Some(window) = self
                .window_ids
                .iter()
                .find(|(id, _)| *id == cmd.id)
                .map(|(_, w)| w.clone())
            {
                if cmd.visible {
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

    /// Inject an input event received from a network client into the
    /// compositor's seat, following the same patterns as `process_input_event`.
    pub fn inject_network_input(&mut self, msg: InputMessage) {
        match msg {
            InputMessage::Keyboard(ev) => {
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
