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

use crate::wm::{
    self, WmState,
    floating::{MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER},
};

// Linux evdev keycodes for the modifier keys tracked by [`ModifierTracker`].
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_RIGHTSHIFT: u32 = 54;
const KEY_LEFTALT: u32 = 56;
const KEY_RIGHTCTRL: u32 = 97;
const KEY_RIGHTALT: u32 = 100;
const KEY_LEFTMETA: u32 = 125;
const KEY_RIGHTMETA: u32 = 126;

/// Tracks which modifier keys are currently held on the network input stream
/// and derives the X11-style modifier bitmask (`MOD_*` in [`wm::floating`])
/// that WM keybindings — both the protocol's `bind_key` and the built-in WM —
/// are registered against.
///
/// Left and right variants are tracked as distinct keys, so releasing one
/// while the other is still down keeps the modifier bit set.
#[derive(Debug, Default)]
pub struct ModifierTracker {
    /// Held modifier keycodes (evdev).
    held: std::collections::HashSet<u32>,
}

impl ModifierTracker {
    /// The modifier bit an evdev keycode contributes, if it is a modifier key.
    fn modifier_bit(keycode: u32) -> Option<u32> {
        match keycode {
            KEY_LEFTSHIFT | KEY_RIGHTSHIFT => Some(MOD_SHIFT),
            KEY_LEFTCTRL | KEY_RIGHTCTRL => Some(MOD_CTRL),
            KEY_LEFTALT | KEY_RIGHTALT => Some(MOD_ALT),
            KEY_LEFTMETA | KEY_RIGHTMETA => Some(MOD_SUPER),
            _ => None,
        }
    }

    /// Record a key press or release. Non-modifier keys are ignored.
    pub fn on_key(&mut self, keycode: u32, pressed: bool) {
        if Self::modifier_bit(keycode).is_none() {
            return;
        }
        if pressed {
            self.held.insert(keycode);
        } else {
            self.held.remove(&keycode);
        }
    }

    /// The bitmask of currently held modifiers.
    pub fn mask(&self) -> u32 {
        self.held
            .iter()
            .filter_map(|&key| Self::modifier_bit(key))
            .fold(0, |mask, bit| mask | bit)
    }

    /// Forget all held modifiers (e.g. when the client disconnects).
    pub fn reset(&mut self) {
        self.held.clear();
    }
}

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
    /// Held-modifier bitmask derived from network key events, passed to WM
    /// keybinding checks so chords like Super+Arrow work.
    modifiers: ModifierTracker,
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
            modifiers: ModifierTracker::default(),
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
        self.apply_render_commands(commands);
    }

    /// Apply a batch of WM render commands to the Space: position, visibility
    /// (with workspace/tag/output filtering) and z-order (absolute and
    /// relative). Used for both the built-in WM and external WM commands.
    pub fn apply_render_commands(&mut self, commands: Vec<wm::types::RenderCommand>) {
        let output_name = self.output.name();

        for cmd in &commands {
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

        // Apply z-order directives in a second pass: compute the desired
        // stacking order from the Space's current one, then raise the windows
        // bottom-to-top so the Space ends up in exactly that order.
        if commands
            .iter()
            .any(|cmd| cmd.z_order != wm::types::ZOrder::Preserve)
        {
            let current: Vec<wm::types::WindowId> = self
                .space
                .elements()
                .filter_map(|window| self.window_id_for(window))
                .collect();
            let desired = wm::types::restack(&current, &commands);
            if desired != current {
                for id in desired {
                    if let Some(window) = self.window_for_id(id).cloned() {
                        self.space.raise_element(&window, false);
                    }
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
        // The departing client's modifiers are no longer held either.
        self.modifiers.reset();
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

    /// Inject an input event received from a network client into the
    /// compositor's seat, following the same patterns as `process_input_event`.
    pub fn inject_network_input(&mut self, msg: InputMessage) {
        match msg {
            InputMessage::Keyboard(ev) => {
                let pressed = matches!(ev.state, wayray_protocol::messages::KeyState::Pressed);

                // Track held modifiers before the binding checks so a chorded
                // key (e.g. Super+Left) sees the modifier already held down.
                self.modifiers.on_key(ev.keycode, pressed);
                let modifiers = self.modifiers.mask();

                // Check if an external WM wants this key.
                if let Some(proto) = &self.wm_state.protocol
                    && proto.check_key_binding(ev.keycode, modifiers, pressed)
                {
                    return;
                }

                // Check if the built-in WM wants this key (only on press).
                if pressed
                    && self
                        .wm_state
                        .active_wm()
                        .on_key_binding(ev.keycode, modifiers)
                {
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
                    // Super+Arrow: the snap changed the focused window's size —
                    // send a configure (the position flows via on_render).
                    if matches!(
                        ev.keycode,
                        crate::wm::floating::KEY_LEFT
                            | crate::wm::floating::KEY_RIGHT
                            | crate::wm::floating::KEY_UP
                            | crate::wm::floating::KEY_DOWN
                    ) && let Some(focused) = self.wm_state.builtin.focused()
                        && let Some((_, size)) = self.wm_state.builtin.geometry_of(focused)
                        && let Some(window) = self.window_for_id(focused).cloned()
                        && let Some(toplevel) = window.toplevel()
                    {
                        toplevel.with_pending_state(|state| {
                            state.size = Some(size.into());
                        });
                        toplevel.send_pending_configure();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tracker_has_no_modifiers() {
        let tracker = ModifierTracker::default();
        assert_eq!(tracker.mask(), 0);
    }

    #[test]
    fn press_and_release_toggle_modifier_bits() {
        // (keycode, expected bit) for every tracked modifier key.
        let cases = [
            (KEY_LEFTSHIFT, MOD_SHIFT),
            (KEY_RIGHTSHIFT, MOD_SHIFT),
            (KEY_LEFTCTRL, MOD_CTRL),
            (KEY_RIGHTCTRL, MOD_CTRL),
            (KEY_LEFTALT, MOD_ALT),
            (KEY_RIGHTALT, MOD_ALT),
            (KEY_LEFTMETA, MOD_SUPER),
            (KEY_RIGHTMETA, MOD_SUPER),
        ];

        for (keycode, bit) in cases {
            let mut tracker = ModifierTracker::default();
            tracker.on_key(keycode, true);
            assert_eq!(tracker.mask(), bit, "keycode {keycode}");
            tracker.on_key(keycode, false);
            assert_eq!(tracker.mask(), 0, "keycode {keycode}");
        }
    }

    #[test]
    fn combined_modifiers_accumulate() {
        let mut tracker = ModifierTracker::default();
        tracker.on_key(KEY_LEFTCTRL, true);
        tracker.on_key(KEY_LEFTALT, true);
        assert_eq!(tracker.mask(), MOD_CTRL | MOD_ALT);

        tracker.on_key(KEY_LEFTALT, false);
        assert_eq!(tracker.mask(), MOD_CTRL);
    }

    #[test]
    fn releasing_one_of_two_shifts_keeps_bit_set() {
        let mut tracker = ModifierTracker::default();
        tracker.on_key(KEY_LEFTSHIFT, true);
        tracker.on_key(KEY_RIGHTSHIFT, true);
        assert_eq!(tracker.mask(), MOD_SHIFT);

        // Left released while right is still held: Shift stays active.
        tracker.on_key(KEY_LEFTSHIFT, false);
        assert_eq!(tracker.mask(), MOD_SHIFT);

        tracker.on_key(KEY_RIGHTSHIFT, false);
        assert_eq!(tracker.mask(), 0);
    }

    #[test]
    fn non_modifier_keys_are_ignored() {
        let mut tracker = ModifierTracker::default();
        tracker.on_key(crate::wm::floating::KEY_TAB, true);
        tracker.on_key(crate::wm::floating::KEY_F4, true);
        assert_eq!(tracker.mask(), 0);
    }

    #[test]
    fn reset_clears_held_modifiers() {
        let mut tracker = ModifierTracker::default();
        tracker.on_key(KEY_LEFTMETA, true);
        tracker.on_key(KEY_LEFTSHIFT, true);
        assert_ne!(tracker.mask(), 0);

        tracker.reset();
        assert_eq!(tracker.mask(), 0);
    }
}
