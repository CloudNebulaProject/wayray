/// Unique identifier for a window managed by the WM.
///
/// Wraps a monotonically increasing u64 assigned when a toplevel is first mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(u64);

impl WindowId {
    /// Create a new WindowId from a raw u64.
    pub fn from_raw(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw u64 value.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// Information about a newly created toplevel, sent to the WM during the manage phase.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub title: Option<String>,
    pub app_id: Option<String>,
    /// Output dimensions the window is being mapped on.
    pub output_width: i32,
    pub output_height: i32,
    /// Size hints from the client (min/max size).
    pub min_size: Option<(i32, i32)>,
    pub max_size: Option<(i32, i32)>,
}

/// The WM's policy response for a new or reconfigured window.
#[derive(Debug, Clone)]
pub struct ManageResponse {
    /// Suggested dimensions (width, height) for the window.
    pub size: (i32, i32),
    /// Position (x, y) to place the window on the output.
    pub position: (i32, i32),
    /// Whether this window should receive keyboard focus.
    pub focus: bool,
    /// Decoration mode preference.
    pub decoration: DecorationMode,
}

/// Decoration mode the WM wants for a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationMode {
    /// Server draws title bar and borders.
    ServerSide,
    /// Client draws its own decorations.
    ClientSide,
}

/// A command from the WM specifying visual placement for a window during the render phase.
#[derive(Debug, Clone)]
pub struct RenderCommand {
    pub id: WindowId,
    pub position: (i32, i32),
    pub z_order: ZOrder,
    pub visible: bool,
}

/// Z-ordering directive for a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZOrder {
    /// Place at the top of the stack.
    Top,
    /// Place at the bottom of the stack.
    Bottom,
    /// Keep current z-order (no change).
    Preserve,
}
