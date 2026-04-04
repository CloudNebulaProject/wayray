use smithay::{
    desktop::Space,
    input::{Seat, SeatState},
    output::Output,
    reexports::wayland_server::{Display, DisplayHandle},
    utils::{Clock, Monotonic},
    wayland::{
        compositor::CompositorState,
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::XdgShellState,
        shm::ShmState,
    },
};
use tracing::info;

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
    pub data_device_state: DataDeviceState,
    pub space: Space<smithay::desktop::Window>,
    pub seat: Seat<Self>,
    pub clock: Clock<Monotonic>,
    pub output: Output,
}

impl WayRay {
    /// Create a new WayRay compositor state, initializing all Smithay subsystems.
    pub fn new(display: &mut Display<Self>, output: Output) -> Self {
        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let data_device_state = DataDeviceState::new::<Self>(&dh);

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
            data_device_state,
            space: Space::default(),
            seat,
            clock: Clock::new(),
            output,
        }
    }
}
