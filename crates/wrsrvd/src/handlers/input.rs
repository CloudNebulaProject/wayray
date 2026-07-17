use smithay::{
    delegate_data_device, delegate_primary_selection, delegate_seat,
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::selection::{
        SelectionHandler, SelectionSource, SelectionTarget,
        data_device::{
            ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
        },
        primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
    },
};
use tracing::{debug, trace};

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

    /// A Wayland client took a selection. Queue the offered mime types for
    /// server→client clipboard forwarding: the payload itself is requested on
    /// the next event-loop turn (via `WayRay::process_pending_clipboard`)
    /// because Smithay calls this handler *before* it stores the new selection
    /// on the seat, so requesting the contents here would read the old one.
    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        // Only the clipboard selection is synced to the thin client; the
        // primary (middle-click) selection changes on every text sweep and
        // would flood the control stream.
        if ty != SelectionTarget::Clipboard {
            return;
        }
        let Some(source) = source else {
            // Selection cleared — drop any queued forward.
            self.pending_selection = None;
            return;
        };
        if self.clipboard_tx.is_none() {
            return; // no network path (winit dev backend)
        }
        let mime_types = source.mime_types();
        debug!(?mime_types, "Wayland client set clipboard selection");
        self.pending_selection = Some(mime_types);
    }

    /// A Wayland client wants to paste the compositor-side selection — the
    /// clipboard payload received from the remote thin client. Serve the
    /// stored bytes for whichever advertised flavor was requested, writing on
    /// a short-lived thread so a slow reader can never block the compositor.
    fn send_selection(
        &mut self,
        _ty: SelectionTarget,
        mime_type: String,
        fd: std::os::unix::io::OwnedFd,
        _seat: Seat<Self>,
        _user_data: &(),
    ) {
        let Some((_, data)) = &self.remote_clipboard else {
            trace!("send_selection with no remote clipboard stored");
            return;
        };
        let data = data.clone();
        std::thread::Builder::new()
            .name("wayray-clipboard-write".into())
            .spawn(move || {
                use std::io::Write;
                let mut file = std::fs::File::from(fd);
                if let Err(e) = file.write_all(&data) {
                    trace!(error = %e, mime_type = %mime_type, "clipboard paste write failed");
                }
            })
            .map_err(|e| debug!(error = %e, "failed to spawn clipboard writer thread"))
            .ok();
    }
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
