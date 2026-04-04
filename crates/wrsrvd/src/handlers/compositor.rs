use smithay::{
    delegate_compositor, delegate_shm,
    reexports::wayland_server::{
        Client,
        protocol::wl_surface::WlSurface,
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        seat::WaylandFocus,
        shm::{ShmHandler, ShmState},
    },
};
use tracing::trace;

use crate::state::WayRay;

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

impl CompositorHandler for WayRay {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        trace!(?surface, "surface commit");
        smithay::backend::renderer::utils::on_commit_buffer_handler::<Self>(surface);

        // If this surface belongs to an xdg toplevel that hasn't received
        // its initial configure yet, send it now so the client can start
        // drawing.
        if let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().map(|s| s.into_owned()) == Some(surface.clone()))
            .cloned()
            && let Some(toplevel) = window.toplevel()
            && !toplevel.is_initial_configure_sent()
        {
            toplevel.send_configure();
        }
    }
}

impl BufferHandler for WayRay {
    fn buffer_destroyed(
        &mut self,
        _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    ) {
    }
}

impl ShmHandler for WayRay {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(WayRay);
delegate_shm!(WayRay);
