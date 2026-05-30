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

use std::collections::HashMap;

/// The default (and currently only) output name for the single-output compositor.
pub const DEFAULT_OUTPUT: &str = "wayray-0";

/// Per-workspace metadata. Currently a placeholder; the window membership is
/// derived from [`WorkspaceState::window_workspace`] so that destroying a
/// workspace cleanly unassigns its windows.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceInfo {}

/// Workspace and tag visibility model for the WM protocol.
///
/// Supports two mutually-exclusive per-window modes:
/// - **Exclusive workspaces**: a window is assigned to a single named workspace
///   and is visible only when that workspace is the active one on its output.
/// - **dwm-style tags**: a window carries a tag bitmask and is visible when any
///   of its tags intersects the output's active tag bitmask.
///
/// Windows with neither assignment are always visible — this preserves the
/// existing single-output, no-workspace behavior for the built-in WM and for
/// external WMs that never use the workspace interface.
#[derive(Debug, Clone)]
pub struct WorkspaceState {
    /// All known workspaces by name.
    pub workspaces: HashMap<String, WorkspaceInfo>,
    /// Active workspace per output (output_name -> workspace_name).
    pub active: HashMap<String, String>,
    /// The default output name (single-output compositor).
    pub default_output: String,
    /// Window -> assigned workspace name (exclusive-workspace mode).
    pub window_workspace: HashMap<WindowId, String>,
    /// Window -> tag bitmask (dwm-style tag mode).
    pub window_tags: HashMap<WindowId, u32>,
    /// Active tag bitmask per output (output_name -> bitmask). Defaults to 0x1.
    pub active_tags: HashMap<String, u32>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        let mut active_tags = HashMap::new();
        active_tags.insert(DEFAULT_OUTPUT.to_string(), 0x1);
        Self {
            workspaces: HashMap::new(),
            active: HashMap::new(),
            default_output: DEFAULT_OUTPUT.to_string(),
            window_workspace: HashMap::new(),
            window_tags: HashMap::new(),
            active_tags,
        }
    }
}

impl WorkspaceState {
    /// Resolve an output name to a key in our maps, mapping unknown outputs to
    /// the default output (single-output compositor behavior).
    fn output_key<'a>(&'a self, output: &'a str) -> &'a str {
        if self.active.contains_key(output)
            || self.active_tags.contains_key(output)
            || output == self.default_output
        {
            output
        } else {
            &self.default_output
        }
    }

    /// Active tag bitmask for an output (defaults to 0x1).
    fn active_tags_for(&self, output: &str) -> u32 {
        let key = self.output_key(output);
        self.active_tags.get(key).copied().unwrap_or(0x1)
    }

    /// Active workspace name for an output, if any.
    fn active_workspace_for(&self, output: &str) -> Option<&str> {
        let key = self.output_key(output);
        self.active.get(key).map(|s| s.as_str())
    }

    /// Create a named workspace (idempotent).
    pub fn create_workspace(&mut self, name: String) {
        self.workspaces.insert(name, WorkspaceInfo::default());
    }

    /// Destroy a workspace: unassign its windows (they become visible) and, if
    /// it was active on any output, fall back to the first remaining workspace
    /// or clear the active assignment.
    pub fn destroy_workspace(&mut self, name: &str) {
        self.workspaces.remove(name);
        self.window_workspace.retain(|_, ws| ws != name);

        // Deterministic fallback: pick the lexicographically-first remaining
        // workspace rather than relying on HashMap iteration order.
        let fallback = self.workspaces.keys().min().cloned();
        let affected: Vec<String> = self
            .active
            .iter()
            .filter(|(_, ws)| ws.as_str() == name)
            .map(|(out, _)| out.clone())
            .collect();
        for out in affected {
            match &fallback {
                Some(next) => {
                    self.active.insert(out, next.clone());
                }
                None => {
                    self.active.remove(&out);
                }
            }
        }
    }

    /// Set the active workspace on an output.
    pub fn set_active_workspace(&mut self, output_name: String, workspace_name: String) {
        self.active.insert(output_name, workspace_name);
    }

    /// Set the active (visible) tag bitmask on an output. The counterpart to
    /// [`set_window_tags`](Self::set_window_tags): a tag-mode window is visible
    /// when any of its tags intersects this mask. Without this, the active tag
    /// set was stuck at its default and tag-mode windows could never switch.
    pub fn set_active_tags(&mut self, output_name: String, tags: u32) {
        self.active_tags.insert(output_name, tags);
    }

    /// Assign a window to a workspace (exclusive mode). Clears any tag bitmask
    /// to keep workspace and tag modes mutually exclusive.
    pub fn assign_window(&mut self, id: WindowId, workspace_name: String) {
        self.window_workspace.insert(id, workspace_name);
        self.window_tags.remove(&id);
    }

    /// Set a window's tag bitmask (dwm mode). Clears any workspace assignment to
    /// keep workspace and tag modes mutually exclusive.
    pub fn set_window_tags(&mut self, id: WindowId, tags: u32) {
        self.window_tags.insert(id, tags);
        self.window_workspace.remove(&id);
    }

    /// Whether a window should be visible on the given output.
    ///
    /// Rules (in order):
    /// 1. If the window has a tag bitmask, it is visible iff
    ///    `(tags & active_tags(output)) != 0`.
    /// 2. Else if the window is assigned to a workspace, it is visible iff that
    ///    workspace is the active workspace on the output.
    /// 3. Else (no assignment) the window is always visible.
    pub fn is_visible(&self, id: WindowId, output: &str) -> bool {
        if let Some(tags) = self.window_tags.get(&id) {
            return (tags & self.active_tags_for(output)) != 0;
        }
        if let Some(ws) = self.window_workspace.get(&id) {
            return self.active_workspace_for(output) == Some(ws.as_str());
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wid(n: u64) -> WindowId {
        WindowId::from_raw(n)
    }

    #[test]
    fn unassigned_window_is_always_visible() {
        let ws = WorkspaceState::default();
        assert!(ws.is_visible(wid(1), DEFAULT_OUTPUT));
        // Unknown output names fall back to the default output.
        assert!(ws.is_visible(wid(1), "some-other-output"));
    }

    #[test]
    fn workspace_assigned_window_visible_only_when_active() {
        let mut ws = WorkspaceState::default();
        ws.workspaces
            .insert("a".to_string(), WorkspaceInfo::default());
        ws.window_workspace.insert(wid(1), "a".to_string());

        // No active workspace -> not visible.
        assert!(!ws.is_visible(wid(1), DEFAULT_OUTPUT));

        // Active workspace 'a' -> visible.
        ws.active
            .insert(DEFAULT_OUTPUT.to_string(), "a".to_string());
        assert!(ws.is_visible(wid(1), DEFAULT_OUTPUT));

        // Switch to 'b' -> not visible.
        ws.active
            .insert(DEFAULT_OUTPUT.to_string(), "b".to_string());
        assert!(!ws.is_visible(wid(1), DEFAULT_OUTPUT));
    }

    #[test]
    fn tagged_window_visible_when_tag_bit_active() {
        let mut ws = WorkspaceState::default();
        // tags = 0b10 (bit 1).
        ws.window_tags.insert(wid(1), 0b10);

        // Default active tags = 0x1 (bit 0) -> not visible.
        assert!(!ws.is_visible(wid(1), DEFAULT_OUTPUT));

        // Activate bit 1 -> visible.
        ws.active_tags.insert(DEFAULT_OUTPUT.to_string(), 0b10);
        assert!(ws.is_visible(wid(1), DEFAULT_OUTPUT));

        // Activate both bits -> still visible (any-match).
        ws.active_tags.insert(DEFAULT_OUTPUT.to_string(), 0b11);
        assert!(ws.is_visible(wid(1), DEFAULT_OUTPUT));
    }

    #[test]
    fn set_active_tags_switches_visible_window() {
        let mut ws = WorkspaceState::default();
        // win1 on tag bit 0, win2 on tag bit 1.
        ws.set_window_tags(wid(1), 0b01);
        ws.set_window_tags(wid(2), 0b10);

        // Default active tags = 0x1 -> only win1 visible.
        assert!(ws.is_visible(wid(1), DEFAULT_OUTPUT));
        assert!(!ws.is_visible(wid(2), DEFAULT_OUTPUT));

        // Switch active tags to bit 1 -> only win2 visible.
        ws.set_active_tags(DEFAULT_OUTPUT.to_string(), 0b10);
        assert!(!ws.is_visible(wid(1), DEFAULT_OUTPUT));
        assert!(ws.is_visible(wid(2), DEFAULT_OUTPUT));

        // Show both tags -> both visible.
        ws.set_active_tags(DEFAULT_OUTPUT.to_string(), 0b11);
        assert!(ws.is_visible(wid(1), DEFAULT_OUTPUT));
        assert!(ws.is_visible(wid(2), DEFAULT_OUTPUT));
    }

    #[test]
    fn tags_and_workspace_are_mutually_exclusive() {
        let mut ws = WorkspaceState::default();
        // Window has both set, but tags take precedence in is_visible.
        ws.window_workspace.insert(wid(1), "a".to_string());
        ws.window_tags.insert(wid(1), 0b1);
        // Tag bit 0 matches default active tags -> visible regardless of workspace.
        assert!(ws.is_visible(wid(1), DEFAULT_OUTPUT));
    }
}
