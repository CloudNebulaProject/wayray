//! Dispatch delegation for the WayRay WM protocol.
//!
//! Connects the generated protocol types to our WmProtocolState implementation.

use smithay::reexports::{
    wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as SmithayDecorationMode,
    wayland_protocols::xdg::shell::server::xdg_toplevel,
    wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::ClientId,
    },
};
use smithay::utils::SERIAL_COUNTER;
use tracing::warn;
use wayray_wm_protocol::server::{
    wayray_wm_manager_v1::WayrayWmManagerV1, wayray_wm_seat_v1::WayrayWmSeatV1,
    wayray_wm_window_v1::WayrayWmWindowV1, wayray_wm_workspace_v1::WayrayWmWorkspaceV1,
};

use crate::state::WayRay;
use crate::wm::protocol::{WmGlobalData, WmProtocolHandler, WmProtocolState, WmWindowData};
use crate::wm::types::DecorationMode;

impl WmProtocolHandler for WayRay {
    fn wm_protocol_state(&mut self) -> &mut WmProtocolState {
        self.wm_state
            .protocol
            .as_mut()
            .expect("WM protocol state not initialized")
    }

    fn existing_windows(&self) -> Vec<crate::wm::protocol::WindowSnapshot> {
        self.window_ids
            .iter()
            .filter_map(|(id, window)| {
                let toplevel = window.toplevel()?;
                let size = toplevel.current_state().size?;
                Some((*id, None, None, size.w, size.h))
            })
            .collect()
    }

    fn configure_window(&mut self, id: crate::wm::types::WindowId, width: i32, height: i32) {
        let Some(window) = self.window_for_id(id).cloned() else {
            warn!(?id, "configure_window: unknown window");
            return;
        };
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                state.size = Some((width, height).into());
            });
            toplevel.send_pending_configure();
        }
    }

    fn focus_window(&mut self, id: crate::wm::types::WindowId) {
        let Some(window) = self.window_for_id(id).cloned() else {
            warn!(?id, "focus_window: unknown window");
            return;
        };
        self.space.raise_element(&window, true);
        let serial = SERIAL_COUNTER.next_serial();
        let keyboard = self.seat.get_keyboard().unwrap();
        let wl_surface = window.toplevel().map(|t| t.wl_surface().clone());
        keyboard.set_focus(self, wl_surface, serial);
    }

    fn close_window(&mut self, id: crate::wm::types::WindowId) {
        let Some(window) = self.window_for_id(id).cloned() else {
            warn!(?id, "close_window: unknown window");
            return;
        };
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_close();
        }
    }

    fn set_fullscreen(&mut self, id: crate::wm::types::WindowId, granted: bool) {
        let Some(window) = self.window_for_id(id).cloned() else {
            warn!(?id, "set_fullscreen: unknown window");
            return;
        };
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                if granted {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                } else {
                    state.states.unset(xdg_toplevel::State::Fullscreen);
                }
            });
            toplevel.send_pending_configure();
        }
    }

    fn set_decoration(&mut self, id: crate::wm::types::WindowId, mode: DecorationMode) {
        let Some(window) = self.window_for_id(id).cloned() else {
            warn!(?id, "set_decoration: unknown window");
            return;
        };
        if let Some(toplevel) = window.toplevel() {
            let smithay_mode = match mode {
                DecorationMode::ServerSide => SmithayDecorationMode::ServerSide,
                DecorationMode::ClientSide => SmithayDecorationMode::ClientSide,
            };
            toplevel.with_pending_state(|state| {
                state.decoration_mode = Some(smithay_mode);
            });
            toplevel.send_pending_configure();
        }
    }
}

// Delegate GlobalDispatch for the manager global.
impl GlobalDispatch<WayrayWmManagerV1, WmGlobalData> for WayRay {
    fn bind(
        state: &mut Self,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<WayrayWmManagerV1>,
        global_data: &WmGlobalData,
        data_init: &mut DataInit<'_, Self>,
    ) {
        <WmProtocolState as GlobalDispatch<WayrayWmManagerV1, WmGlobalData, Self>>::bind(
            state,
            dh,
            client,
            resource,
            global_data,
            data_init,
        );
    }
}

// Delegate Dispatch for manager requests.
impl Dispatch<WayrayWmManagerV1, ()> for WayRay {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &WayrayWmManagerV1,
        request: <WayrayWmManagerV1 as Resource>::Request,
        data: &(),
        dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        <WmProtocolState as Dispatch<WayrayWmManagerV1, (), Self>>::request(
            state, client, resource, request, data, dh, data_init,
        );
    }

    fn destroyed(state: &mut Self, client: ClientId, resource: &WayrayWmManagerV1, data: &()) {
        <WmProtocolState as Dispatch<WayrayWmManagerV1, (), Self>>::destroyed(
            state, client, resource, data,
        );
    }
}

// Delegate Dispatch for window requests.
impl Dispatch<WayrayWmWindowV1, WmWindowData> for WayRay {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &WayrayWmWindowV1,
        request: <WayrayWmWindowV1 as Resource>::Request,
        data: &WmWindowData,
        dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        <WmProtocolState as Dispatch<WayrayWmWindowV1, WmWindowData, Self>>::request(
            state, client, resource, request, data, dh, data_init,
        );
    }

    fn destroyed(
        state: &mut Self,
        client: ClientId,
        resource: &WayrayWmWindowV1,
        data: &WmWindowData,
    ) {
        <WmProtocolState as Dispatch<WayrayWmWindowV1, WmWindowData, Self>>::destroyed(
            state, client, resource, data,
        );
    }
}

// Delegate Dispatch for seat requests.
impl Dispatch<WayrayWmSeatV1, ()> for WayRay {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &WayrayWmSeatV1,
        request: <WayrayWmSeatV1 as Resource>::Request,
        data: &(),
        dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        <WmProtocolState as Dispatch<WayrayWmSeatV1, (), Self>>::request(
            state, client, resource, request, data, dh, data_init,
        );
    }
}

// Delegate Dispatch for workspace requests.
impl Dispatch<WayrayWmWorkspaceV1, ()> for WayRay {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &WayrayWmWorkspaceV1,
        request: <WayrayWmWorkspaceV1 as Resource>::Request,
        data: &(),
        dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        <WmProtocolState as Dispatch<WayrayWmWorkspaceV1, (), Self>>::request(
            state, client, resource, request, data, dh, data_init,
        );
    }
}
