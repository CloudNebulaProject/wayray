use std::collections::HashMap;

use super::WindowManager;
use super::types::{DecorationMode, ManageResponse, RenderCommand, WindowId, WindowInfo, ZOrder};

/// Modifier bitmask for Alt (Mod1 in X11/XKB terms).
pub const MOD_ALT: u32 = 0x08;

/// Linux evdev keycode for F4.
pub const KEY_F4: u32 = 62;
/// Linux evdev keycode for Tab.
pub const KEY_TAB: u32 = 15;

/// State for a single managed window in the floating WM.
#[derive(Debug, Clone)]
struct WindowState {
    position: (i32, i32),
    size: (i32, i32),
    /// Stack index: higher means more on top.
    z_index: u32,
    visible: bool,
}

/// Built-in floating window manager providing sane defaults.
///
/// Active when no external WM is connected. Centers new windows,
/// implements click-to-focus, and provides basic keyboard shortcuts.
pub struct BuiltinFloatingWm {
    windows: HashMap<WindowId, WindowState>,
    /// Focus stack: last element is the focused window.
    focus_stack: Vec<WindowId>,
    /// Counter for z-ordering.
    next_z: u32,
    /// Output dimensions for centering calculations.
    output_width: i32,
    output_height: i32,
}

impl BuiltinFloatingWm {
    pub fn new(output_width: i32, output_height: i32) -> Self {
        Self {
            windows: HashMap::new(),
            focus_stack: Vec::new(),
            next_z: 1,
            output_width,
            output_height,
        }
    }

    /// Get the currently focused window (top of focus stack).
    pub fn focused(&self) -> Option<WindowId> {
        self.focus_stack.last().copied()
    }

    /// Cycle focus to the next window in the stack.
    /// Returns the newly focused window id, if any.
    pub fn focus_next(&mut self) -> Option<WindowId> {
        if self.focus_stack.len() < 2 {
            return self.focus_stack.last().copied();
        }

        // Move the top (focused) window to the bottom of the stack.
        let top = self.focus_stack.pop().unwrap();
        self.focus_stack.insert(0, top);

        // The new top is now focused — raise it.
        let new_focus = *self.focus_stack.last().unwrap();
        if let Some(state) = self.windows.get_mut(&new_focus) {
            state.z_index = self.next_z;
            self.next_z += 1;
        }

        Some(new_focus)
    }

    /// Move window to the top of the focus stack.
    fn raise_to_top(&mut self, id: WindowId) {
        self.focus_stack.retain(|&w| w != id);
        self.focus_stack.push(id);

        if let Some(state) = self.windows.get_mut(&id) {
            state.z_index = self.next_z;
            self.next_z += 1;
        }
    }
}

impl WindowManager for BuiltinFloatingWm {
    fn on_new_toplevel(&mut self, id: WindowId, info: WindowInfo) -> ManageResponse {
        let width = 800.min(info.output_width);
        let height = 600.min(info.output_height);

        // Respect size hints if present.
        let width = match (info.min_size, info.max_size) {
            (Some((min_w, _)), _) if width < min_w => min_w,
            (_, Some((max_w, _))) if width > max_w => max_w,
            _ => width,
        };
        let height = match (info.min_size, info.max_size) {
            (Some((_, min_h)), _) if height < min_h => min_h,
            (_, Some((_, max_h))) if height > max_h => max_h,
            _ => height,
        };

        // Center on output.
        let x = (self.output_width - width) / 2;
        let y = (self.output_height - height) / 2;

        let z_index = self.next_z;
        self.next_z += 1;

        self.windows.insert(
            id,
            WindowState {
                position: (x, y),
                size: (width, height),
                z_index,
                visible: true,
            },
        );
        self.focus_stack.push(id);

        ManageResponse {
            size: (width, height),
            position: (x, y),
            focus: true,
            decoration: DecorationMode::ServerSide,
        }
    }

    fn on_close_toplevel(&mut self, id: WindowId) {
        self.windows.remove(&id);
        self.focus_stack.retain(|&w| w != id);
    }

    fn on_fullscreen_request(&mut self, id: WindowId) -> bool {
        if let Some(state) = self.windows.get_mut(&id) {
            state.position = (0, 0);
            state.size = (self.output_width, self.output_height);
            self.raise_to_top(id);
            true
        } else {
            false
        }
    }

    fn on_render(&mut self, windows: &[WindowId]) -> Vec<RenderCommand> {
        windows
            .iter()
            .filter_map(|&id| {
                let state = self.windows.get(&id)?;
                Some(RenderCommand {
                    id,
                    position: state.position,
                    z_order: ZOrder::Preserve,
                    visible: state.visible,
                })
            })
            .collect()
    }

    fn on_key_binding(&mut self, key: u32, modifiers: u32) -> bool {
        if modifiers & MOD_ALT == 0 {
            return false;
        }

        match key {
            KEY_F4 => {
                // Alt+F4: signal that the focused window should close.
                // The actual close is handled by the compositor via
                // the returned `true` + checking focused().
                true
            }
            KEY_TAB => {
                // Alt+Tab: cycle focus to the next window.
                self.focus_next();
                true
            }
            _ => false,
        }
    }

    fn on_pointer_focus(&mut self, id: WindowId) -> Option<WindowId> {
        if self.windows.contains_key(&id) {
            self.raise_to_top(id);
            Some(id)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_info() -> WindowInfo {
        WindowInfo {
            title: None,
            app_id: None,
            output_width: 1280,
            output_height: 720,
            min_size: None,
            max_size: None,
        }
    }

    #[test]
    fn new_window_is_centered() {
        let mut wm = BuiltinFloatingWm::new(1280, 720);
        let id = WindowId::from_raw(1);
        let resp = wm.on_new_toplevel(id, make_info());

        assert_eq!(resp.size, (800, 600));
        // Centered: (1280 - 800) / 2 = 240, (720 - 600) / 2 = 60
        assert_eq!(resp.position, (240, 60));
        assert!(resp.focus);
    }

    #[test]
    fn small_output_clamps_size() {
        let mut wm = BuiltinFloatingWm::new(640, 480);
        let id = WindowId::from_raw(1);
        let info = WindowInfo {
            output_width: 640,
            output_height: 480,
            ..make_info()
        };
        let resp = wm.on_new_toplevel(id, info);

        assert_eq!(resp.size, (640, 480));
        assert_eq!(resp.position, (0, 0));
    }

    #[test]
    fn focus_stack_ordering() {
        let mut wm = BuiltinFloatingWm::new(1280, 720);
        let id1 = WindowId::from_raw(1);
        let id2 = WindowId::from_raw(2);
        let id3 = WindowId::from_raw(3);

        wm.on_new_toplevel(id1, make_info());
        wm.on_new_toplevel(id2, make_info());
        wm.on_new_toplevel(id3, make_info());

        // id3 is on top (last created)
        assert_eq!(wm.focus_stack.last(), Some(&id3));

        // Click on id1 — should raise it
        wm.on_pointer_focus(id1);
        assert_eq!(wm.focus_stack.last(), Some(&id1));
        assert!(wm.windows[&id1].z_index > wm.windows[&id3].z_index);
    }

    #[test]
    fn close_removes_from_focus_stack() {
        let mut wm = BuiltinFloatingWm::new(1280, 720);
        let id1 = WindowId::from_raw(1);
        let id2 = WindowId::from_raw(2);

        wm.on_new_toplevel(id1, make_info());
        wm.on_new_toplevel(id2, make_info());

        wm.on_close_toplevel(id2);
        assert!(!wm.focus_stack.contains(&id2));
        assert!(!wm.windows.contains_key(&id2));
        assert_eq!(wm.focus_stack.last(), Some(&id1));
    }

    #[test]
    fn fullscreen_covers_output() {
        let mut wm = BuiltinFloatingWm::new(1280, 720);
        let id = WindowId::from_raw(1);
        wm.on_new_toplevel(id, make_info());

        let granted = wm.on_fullscreen_request(id);
        assert!(granted);
        assert_eq!(wm.windows[&id].position, (0, 0));
        assert_eq!(wm.windows[&id].size, (1280, 720));
    }

    #[test]
    fn alt_f4_signals_close() {
        let mut wm = BuiltinFloatingWm::new(1280, 720);
        let id = WindowId::from_raw(1);
        wm.on_new_toplevel(id, make_info());

        // Alt+F4 should be consumed.
        assert!(wm.on_key_binding(KEY_F4, MOD_ALT));
        // The focused window should still exist (compositor handles actual close).
        assert_eq!(wm.focused(), Some(id));
    }

    #[test]
    fn alt_tab_cycles_focus() {
        let mut wm = BuiltinFloatingWm::new(1280, 720);
        let id1 = WindowId::from_raw(1);
        let id2 = WindowId::from_raw(2);
        let id3 = WindowId::from_raw(3);

        wm.on_new_toplevel(id1, make_info());
        wm.on_new_toplevel(id2, make_info());
        wm.on_new_toplevel(id3, make_info());

        assert_eq!(wm.focused(), Some(id3));

        // Alt+Tab should cycle to id2.
        assert!(wm.on_key_binding(KEY_TAB, MOD_ALT));
        assert_eq!(wm.focused(), Some(id2));

        // Again -> id1.
        assert!(wm.on_key_binding(KEY_TAB, MOD_ALT));
        assert_eq!(wm.focused(), Some(id1));

        // Again -> back to id3.
        assert!(wm.on_key_binding(KEY_TAB, MOD_ALT));
        assert_eq!(wm.focused(), Some(id3));
    }

    #[test]
    fn non_alt_keys_not_consumed() {
        let mut wm = BuiltinFloatingWm::new(1280, 720);
        let id = WindowId::from_raw(1);
        wm.on_new_toplevel(id, make_info());

        // F4 without Alt should not be consumed.
        assert!(!wm.on_key_binding(KEY_F4, 0));
        // Random key with Alt should not be consumed.
        assert!(!wm.on_key_binding(42, MOD_ALT));
    }
}
