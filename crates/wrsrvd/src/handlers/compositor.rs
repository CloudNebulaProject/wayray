use smithay::{
    delegate_compositor, delegate_shm,
    reexports::wayland_server::{Client, protocol::wl_surface::WlSurface},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        seat::WaylandFocus,
        shm::{ShmHandler, ShmState},
    },
};

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
        smithay::backend::renderer::utils::on_commit_buffer_handler::<Self>(surface);

        // Find the window this surface belongs to and update its state.
        if let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().map(|s| s.into_owned()) == Some(surface.clone()))
            .cloned()
        {
            // Update the window's bounding box from the committed surface tree.
            // Without this, the window stays at 0x0 and never gets rendered.
            window.on_commit();

            if let Some(toplevel) = window.toplevel()
                && !toplevel.is_initial_configure_sent()
            {
                toplevel.send_configure();
            }
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
