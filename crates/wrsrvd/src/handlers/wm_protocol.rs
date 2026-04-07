//! Dispatch delegation for the WayRay WM protocol.
//!
//! Connects the generated protocol types to our WmProtocolState implementation.

use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::ClientId,
};
use wayray_wm_protocol::server::{
    wayray_wm_manager_v1::WayrayWmManagerV1, wayray_wm_seat_v1::WayrayWmSeatV1,
    wayray_wm_window_v1::WayrayWmWindowV1, wayray_wm_workspace_v1::WayrayWmWorkspaceV1,
};

use crate::state::WayRay;
use crate::wm::protocol::{WmGlobalData, WmProtocolHandler, WmProtocolState, WmWindowData};

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
