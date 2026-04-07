use smithay::{delegate_output, output::Output, wayland::output::OutputHandler};

use crate::state::WayRay;

impl OutputHandler for WayRay {
    fn output_bound(
        &mut self,
        _output: Output,
        _wl_output: smithay::reexports::wayland_server::protocol::wl_output::WlOutput,
    ) {
    }
}

delegate_output!(WayRay);
