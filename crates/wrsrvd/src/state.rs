use smithay::{
    delegate_compositor, delegate_output, delegate_seat, delegate_shm, delegate_xdg_shell,
    desktop::Space,
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    output::Output,
    reexports::wayland_server::{
        Display, DisplayHandle, Client,
        protocol::{wl_seat, wl_surface::WlSurface},
    },
    utils::{Clock, Monotonic, Serial},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        output::{OutputHandler, OutputManagerState},
        shell::xdg::{XdgShellHandler, XdgShellState, ToplevelSurface, PopupSurface, PositionerState},
        shm::{ShmHandler, ShmState},
    },
};
use tracing::info;

/// Per-client state required by Smithay's compositor subsystem.
///
/// Must be stored in the client's `ClientData` so that `CompositorHandler`
/// can retrieve it. Implements `wayland_server::backend::ClientData`.
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl smithay::reexports::wayland_server::backend::ClientData for ClientState {
    fn initialized(&self, _client_id: smithay::reexports::wayland_server::backend::ClientId) {}
    fn disconnected(
        &self,
        _client_id: smithay::reexports::wayland_server::backend::ClientId,
        _reason: smithay::reexports::wayland_server::backend::DisconnectReason,
    ) {
    }
}

/// Central compositor state holding all Smithay subsystem states.
///
/// This is the "god struct" pattern required by Smithay — a single type that
/// implements all handler traits and holds all protocol global state.
pub struct WayRay {
    pub display_handle: DisplayHandle,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub output_manager_state: OutputManagerState,
    pub space: Space<smithay::desktop::Window>,
    pub seat: Seat<Self>,
    pub clock: Clock<Monotonic>,
}

impl WayRay {
    /// Create a new WayRay compositor state, initializing all Smithay subsystems.
    pub fn new(display: &mut Display<Self>) -> Self {
        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);

        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&dh, "wayray");

        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);

        info!("all Smithay subsystem states initialized");

        Self {
            display_handle: dh,
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            output_manager_state,
            space: Space::default(),
            seat,
            clock: Clock::new(),
        }
    }
}

// --- Minimal handler trait impls required by delegate macros ---
// TODO: These will be expanded and moved to handlers/ in Task 4.

impl CompositorHandler for WayRay {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, _surface: &WlSurface) {
        // TODO: handle surface commits (Task 4)
    }
}

impl XdgShellHandler for WayRay {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, _surface: ToplevelSurface) {
        // TODO: handle new toplevel windows (Task 4)
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // TODO: handle new popups (Task 4)
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO: handle popup grabs (Task 4)
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
        // TODO: handle popup reposition requests (Task 4)
    }
}

impl ShmHandler for WayRay {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl BufferHandler for WayRay {
    fn buffer_destroyed(
        &mut self,
        _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    ) {
        // TODO: handle buffer destruction (Task 4)
    }
}

impl SeatHandler for WayRay {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {
        // TODO: handle focus changes (Task 4)
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {
        // TODO: handle cursor image updates (Task 4)
    }
}

impl OutputHandler for WayRay {
    fn output_bound(
        &mut self,
        _output: Output,
        _wl_output: smithay::reexports::wayland_server::protocol::wl_output::WlOutput,
    ) {
        // TODO: handle output binding (Task 4)
    }
}

// --- Delegate macros wiring Smithay's Dispatch impls ---
// TODO: These will be moved to handlers/ in Task 4.

delegate_compositor!(WayRay);
delegate_xdg_shell!(WayRay);
delegate_shm!(WayRay);
delegate_seat!(WayRay);
delegate_output!(WayRay);
