#[allow(dead_code)]
pub mod floating;
pub mod protocol;
#[allow(dead_code)]
pub mod types;

use types::{ManageResponse, RenderCommand, WindowId, WindowInfo};

use self::floating::BuiltinFloatingWm;
use self::protocol::WmProtocolState;

/// Trait abstracting window management behavior.
///
/// Both the built-in floating WM and the external protocol adapter implement
/// this trait, so the compositor core doesn't care which is active.
#[allow(dead_code)]
pub trait WindowManager {
    /// Called when a new toplevel surface is mapped. Returns policy decisions
    /// (size, position, focus, decorations) for the window.
    fn on_new_toplevel(&mut self, id: WindowId, info: WindowInfo) -> ManageResponse;

    /// Called when a toplevel is destroyed.
    fn on_close_toplevel(&mut self, id: WindowId);

    /// Called when a client requests fullscreen. Returns true to grant.
    fn on_fullscreen_request(&mut self, id: WindowId) -> bool;

    /// Called each frame before rendering. Returns placement commands for all
    /// managed windows so the compositor can apply positions/z-order atomically.
    fn on_render(&mut self, windows: &[WindowId]) -> Vec<RenderCommand>;

    /// Called when a key press occurs. Returns true if the WM consumed the
    /// binding (preventing delivery to the focused client).
    fn on_key_binding(&mut self, key: u32, modifiers: u32) -> bool;

    /// Called when the user clicks on a window. The WM should update focus
    /// and z-order. Returns the id of the window that should receive focus,
    /// if any.
    fn on_pointer_focus(&mut self, id: WindowId) -> Option<WindowId>;
}

/// Coordinator holding both the built-in WM and the protocol server state.
/// Routes calls to whichever is active.
pub struct WmState {
    pub builtin: BuiltinFloatingWm,
    pub protocol: Option<WmProtocolState>,
}

impl WmState {
    /// Create a new WmState with the built-in floating WM active.
    pub fn new(output_width: i32, output_height: i32) -> Self {
        Self {
            builtin: BuiltinFloatingWm::new(output_width, output_height),
            protocol: None,
        }
    }

    /// Initialize the protocol server (call after Display is set up).
    pub fn init_protocol(&mut self, protocol_state: WmProtocolState) {
        self.protocol = Some(protocol_state);
    }

    /// Whether an external WM is connected via the protocol.
    #[allow(dead_code)]
    pub fn external_connected(&self) -> bool {
        self.protocol.as_ref().is_some_and(|p| p.is_wm_connected())
    }

    /// Get a mutable reference to the currently active WM.
    /// Returns the built-in WM when no external WM is connected.
    pub fn active_wm(&mut self) -> &mut dyn WindowManager {
        // When an external WM is connected, we still use the built-in WM
        // for the trait interface. The external WM's render commands are
        // applied separately via the protocol state.
        // TODO: In a future iteration, create a ProtocolAdapter that
        // implements WindowManager and bridges to the protocol.
        &mut self.builtin
    }
}
