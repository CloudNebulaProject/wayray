use smithay::{
    delegate_data_device, delegate_primary_selection, delegate_seat,
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::selection::{
        SelectionHandler,
        data_device::{
            ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
        },
        primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
    },
};

use crate::state::WayRay;

impl SeatHandler for WayRay {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}
}

impl SelectionHandler for WayRay {
    type SelectionUserData = ();
}

impl ClientDndGrabHandler for WayRay {}
impl ServerDndGrabHandler for WayRay {}

impl DataDeviceHandler for WayRay {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl PrimarySelectionHandler for WayRay {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

delegate_seat!(WayRay);
delegate_data_device!(WayRay);
delegate_primary_selection!(WayRay);
