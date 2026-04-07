use smithay::{
    delegate_xdg_decoration, delegate_xdg_shell,
    desktop::Window,
    reexports::{
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
        wayland_server::protocol::wl_seat,
    },
    utils::Serial,
    wayland::shell::xdg::{
        PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        decoration::XdgDecorationHandler,
    },
};
use tracing::info;

use crate::state::WayRay;

impl XdgShellHandler for WayRay {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Send the initial configure so the client can start drawing.
        surface.send_configure();
        let window = Window::new_wayland_window(surface);
        self.space.map_element(window, (0, 0), false);
        info!("new toplevel mapped");
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        // Send the initial configure so the popup can start drawing.
        surface.send_configure().ok();
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO: handle popup grabs
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
        // TODO: handle popup reposition requests
    }
}

impl XdgDecorationHandler for WayRay {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        // Prefer server-side decorations — we don't draw them yet,
        // but telling the client lets it skip its own title bar.
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

delegate_xdg_shell!(WayRay);
delegate_xdg_decoration!(WayRay);
