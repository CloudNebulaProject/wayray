//! Wayland protocol server for the WayRay WM protocol.
//!
//! Implements `GlobalDispatch` and `Dispatch` for the four custom WM interfaces,
//! allowing external WM clients to connect and control window layout.

use std::collections::{HashMap, HashSet};

use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::{ClientId, GlobalId},
};
use tracing::{info, warn};
use wayray_wm_protocol::server::{
    wayray_wm_manager_v1::{self, WayrayWmManagerV1},
    wayray_wm_seat_v1::{self, WayrayWmSeatV1},
    wayray_wm_window_v1::{self, WayrayWmWindowV1},
    wayray_wm_workspace_v1::{self, WayrayWmWorkspaceV1},
};

use super::types::{DecorationMode, RenderCommand, WindowId, ZOrder};

/// Window info tuple for sending to a newly connected WM.
/// (window_id, title, app_id, width, height)
pub type WindowSnapshot = (WindowId, Option<String>, Option<String>, i32, i32);

/// Per-window data associated with a `wayray_wm_window_v1` protocol object.
#[derive(Debug, Clone)]
pub struct WmWindowData {
    pub window_id: WindowId,
}

/// Data associated with the WM manager global.
pub struct WmGlobalData;

/// State for the WM protocol server.
///
/// Tracks the currently connected WM, pending phase operations, and
/// the mapping between WindowIds and protocol objects.
#[allow(dead_code)]
pub struct WmProtocolState {
    global: GlobalId,
    /// The currently bound WM manager resource (only one allowed).
    wm_client: Option<WayrayWmManagerV1>,
    /// Mapping from WindowId to the protocol window object sent to the WM.
    window_objects: HashMap<WindowId, WayrayWmWindowV1>,
    /// Pending render commands collected during the render phase.
    pending_render_commands: Vec<RenderCommand>,
    /// Whether a manage phase is currently in progress.
    manage_phase_active: bool,
    /// Whether a render phase is currently in progress.
    render_phase_active: bool,
    /// Display handle for creating resources.
    dh: DisplayHandle,
    /// Registered keybindings: (key, modifiers, mode) -> active.
    keybindings: HashSet<(u32, u32, String)>,
    /// Available binding modes.
    binding_modes: HashSet<String>,
    /// Currently active binding mode (empty string = default).
    active_mode: String,
    /// The WM's seat object for sending binding events.
    wm_seat: Option<WayrayWmSeatV1>,
}

impl std::fmt::Debug for WmProtocolState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WmProtocolState")
            .field("wm_connected", &self.wm_client.is_some())
            .field("windows", &self.window_objects.len())
            .finish()
    }
}

#[allow(dead_code)]
impl WmProtocolState {
    /// Create the WM global and register it with the display.
    pub fn new<D>(dh: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<WayrayWmManagerV1, WmGlobalData>
            + Dispatch<WayrayWmManagerV1, ()>
            + Dispatch<WayrayWmWindowV1, WmWindowData>
            + Dispatch<WayrayWmSeatV1, ()>
            + Dispatch<WayrayWmWorkspaceV1, ()>
            + 'static,
    {
        let global = dh.create_global::<D, WayrayWmManagerV1, _>(1, WmGlobalData);

        Self {
            global,
            wm_client: None,
            window_objects: HashMap::new(),
            pending_render_commands: Vec::new(),
            manage_phase_active: false,
            render_phase_active: false,
            dh: dh.clone(),
            keybindings: HashSet::new(),
            binding_modes: HashSet::new(),
            active_mode: String::new(),
            wm_seat: None,
        }
    }

    /// Whether an external WM is currently connected.
    pub fn is_wm_connected(&self) -> bool {
        self.wm_client.is_some()
    }

    /// Send a `window_new` event to the connected WM for a new toplevel.
    pub fn notify_new_window<D>(
        &mut self,
        window_id: WindowId,
        title: Option<&str>,
        app_id: Option<&str>,
        width: i32,
        height: i32,
    ) where
        D: Dispatch<WayrayWmWindowV1, WmWindowData> + Dispatch<WayrayWmManagerV1, ()> + 'static,
    {
        let Some(manager) = &self.wm_client else {
            return;
        };

        let Ok(client) = self.dh.get_client(manager.id()) else {
            return;
        };

        let data = WmWindowData { window_id };

        let Ok(window_obj) =
            client.create_resource::<WayrayWmWindowV1, _, D>(&self.dh, manager.version(), data)
        else {
            warn!("failed to create WM window resource");
            return;
        };

        // Send window_new event with the new protocol object.
        manager.window_new(&window_obj);

        // Send initial properties.
        if let Some(title) = title {
            window_obj.title(title.to_string());
        }
        if let Some(app_id) = app_id {
            window_obj.app_id(app_id.to_string());
        }
        window_obj.dimensions(width, height);
        window_obj.done();

        self.window_objects.insert(window_id, window_obj);
    }

    /// Send a `window_closed` event to the connected WM.
    pub fn notify_window_closed(&mut self, window_id: WindowId) {
        let Some(manager) = &self.wm_client else {
            return;
        };

        if let Some(window_obj) = self.window_objects.remove(&window_id) {
            manager.window_closed(&window_obj);
        }
    }

    /// Send `manage_start` to begin the manage phase.
    pub fn start_manage_phase(&mut self) {
        if let Some(manager) = &self.wm_client {
            manager.manage_start();
            self.manage_phase_active = true;
        }
    }

    /// Send `render_start` to begin the render phase.
    pub fn start_render_phase(&mut self) {
        if let Some(manager) = &self.wm_client {
            self.pending_render_commands.clear();
            manager.render_start();
            self.render_phase_active = true;
        }
    }

    /// Take the collected render commands from the last render phase.
    pub fn take_render_commands(&mut self) -> Vec<RenderCommand> {
        std::mem::take(&mut self.pending_render_commands)
    }

    /// Check if a key+modifiers combination is registered as a WM binding.
    /// If so, send the binding_pressed event to the WM and return true.
    pub fn check_key_binding(&self, key: u32, modifiers: u32, pressed: bool) -> bool {
        // Check default mode bindings and active mode bindings.
        let default_match = self.keybindings.contains(&(key, modifiers, String::new()));
        let mode_match = !self.active_mode.is_empty()
            && self
                .keybindings
                .contains(&(key, modifiers, self.active_mode.clone()));

        if !default_match && !mode_match {
            return false;
        }

        // Send binding event to the WM's seat object.
        if let Some(seat) = &self.wm_seat {
            if pressed {
                seat.binding_pressed(key, modifiers);
            } else {
                seat.binding_released(key, modifiers);
            }
        }

        true
    }

    /// Send the full window list to a newly connected WM.
    fn send_full_window_list<D>(
        &mut self,
        manager: &WayrayWmManagerV1,
        existing_windows: &[WindowSnapshot],
    ) where
        D: Dispatch<WayrayWmWindowV1, WmWindowData> + Dispatch<WayrayWmManagerV1, ()> + 'static,
    {
        let Ok(client) = self.dh.get_client(manager.id()) else {
            return;
        };

        for (window_id, title, app_id, width, height) in existing_windows {
            let data = WmWindowData {
                window_id: *window_id,
            };

            let Ok(window_obj) =
                client.create_resource::<WayrayWmWindowV1, _, D>(&self.dh, manager.version(), data)
            else {
                continue;
            };

            manager.window_new(&window_obj);

            if let Some(title) = title {
                window_obj.title(title.clone());
            }
            if let Some(app_id) = app_id {
                window_obj.app_id(app_id.clone());
            }
            window_obj.dimensions(*width, *height);
            window_obj.done();

            self.window_objects.insert(*window_id, window_obj);
        }
    }

    /// Look up a WindowId from a protocol window object.
    fn window_id_for_resource(&self, resource: &WayrayWmWindowV1) -> Option<WindowId> {
        self.window_objects
            .iter()
            .find(|(_, obj)| *obj == resource)
            .map(|(id, _)| *id)
    }
}

// =============================================================================
// Dispatch implementations
// =============================================================================

/// Helper trait for the WayRay compositor state to provide WM protocol state.
pub trait WmProtocolHandler:
    GlobalDispatch<WayrayWmManagerV1, WmGlobalData>
    + Dispatch<WayrayWmManagerV1, ()>
    + Dispatch<WayrayWmWindowV1, WmWindowData>
    + Dispatch<WayrayWmSeatV1, ()>
    + Dispatch<WayrayWmWorkspaceV1, ()>
    + 'static
{
    fn wm_protocol_state(&mut self) -> &mut WmProtocolState;

    /// Return the list of existing windows for sending to a newly connected WM.
    /// Each tuple is (window_id, title, app_id, width, height).
    fn existing_windows(&self) -> Vec<WindowSnapshot>;

    // -- Compositor actions: let protocol dispatch reach back into Smithay --

    /// Send a configure with proposed dimensions to the window's toplevel.
    fn configure_window(&mut self, id: WindowId, width: i32, height: i32);

    /// Move keyboard focus to the specified window.
    fn focus_window(&mut self, id: WindowId);

    /// Ask the client to close the specified window.
    fn close_window(&mut self, id: WindowId);

    /// Set or unset fullscreen state on a window.
    fn set_fullscreen(&mut self, id: WindowId, granted: bool);

    /// Set the decoration mode (server-side or client-side) for a window.
    fn set_decoration(&mut self, id: WindowId, mode: DecorationMode);
}

// --- Manager ---

impl<D: WmProtocolHandler> GlobalDispatch<WayrayWmManagerV1, WmGlobalData, D> for WmProtocolState {
    fn bind(
        state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WayrayWmManagerV1>,
        _global_data: &WmGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let instance = data_init.init(resource, ());

        // Collect existing windows before mutating protocol state.
        let existing = state.existing_windows();

        let proto = state.wm_protocol_state();

        // Enforce single-WM: replace old WM if present.
        if let Some(old_manager) = proto.wm_client.take() {
            old_manager.replaced();
            proto.window_objects.clear();
            info!("external WM replaced by new connection");
        }

        proto.wm_client = Some(instance.clone());

        // Send the full window list so the new WM can reconstruct state.
        if !existing.is_empty() {
            proto.send_full_window_list::<D>(&instance, &existing);
            info!(
                window_count = existing.len(),
                "sent existing window list to new WM"
            );
        }

        info!("external WM connected");
    }
}

impl<D: WmProtocolHandler> Dispatch<WayrayWmManagerV1, (), D> for WmProtocolState {
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &WayrayWmManagerV1,
        request: wayray_wm_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let proto = state.wm_protocol_state();
        match request {
            wayray_wm_manager_v1::Request::ManageDone => {
                proto.manage_phase_active = false;
            }
            wayray_wm_manager_v1::Request::RenderDone => {
                proto.render_phase_active = false;
            }
            wayray_wm_manager_v1::Request::GetSeat { id } => {
                data_init.init(id, ());
            }
            wayray_wm_manager_v1::Request::GetWorkspaceManager { id } => {
                data_init.init(id, ());
            }
            wayray_wm_manager_v1::Request::Destroy => {
                // WM disconnecting gracefully.
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, resource: &WayrayWmManagerV1, _data: &()) {
        let proto = state.wm_protocol_state();
        if proto.wm_client.as_ref().is_some_and(|wm| wm == resource) {
            proto.wm_client = None;
            proto.window_objects.clear();
            proto.manage_phase_active = false;
            proto.render_phase_active = false;
            proto.keybindings.clear();
            proto.wm_seat = None;
            proto.active_mode.clear();
            info!("external WM disconnected");
        }
    }
}

// --- Window ---

impl<D: WmProtocolHandler> Dispatch<WayrayWmWindowV1, WmWindowData, D> for WmProtocolState {
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &WayrayWmWindowV1,
        request: wayray_wm_window_v1::Request,
        data: &WmWindowData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let window_id = data.window_id;

        // Manage-phase requests: call compositor actions directly on state.
        // These need &mut access to the full compositor, not just the protocol state.
        match request {
            wayray_wm_window_v1::Request::ProposeDimensions { width, height } => {
                state.configure_window(window_id, width, height);
                return;
            }
            wayray_wm_window_v1::Request::SetFocus => {
                state.focus_window(window_id);
                return;
            }
            wayray_wm_window_v1::Request::UseSsd => {
                state.set_decoration(window_id, DecorationMode::ServerSide);
                return;
            }
            wayray_wm_window_v1::Request::UseCsd => {
                state.set_decoration(window_id, DecorationMode::ClientSide);
                return;
            }
            wayray_wm_window_v1::Request::GrantFullscreen => {
                state.set_fullscreen(window_id, true);
                return;
            }
            wayray_wm_window_v1::Request::DenyFullscreen => {
                state.set_fullscreen(window_id, false);
                return;
            }
            wayray_wm_window_v1::Request::Close => {
                state.close_window(window_id);
                return;
            }
            _ => {}
        }

        // Render-phase requests: accumulate into pending render commands.
        let proto = state.wm_protocol_state();

        match request {
            wayray_wm_window_v1::Request::SetPosition { x, y } => {
                proto.pending_render_commands.push(RenderCommand {
                    id: window_id,
                    position: (x, y),
                    z_order: ZOrder::Preserve,
                    visible: true,
                });
            }
            wayray_wm_window_v1::Request::SetZTop => {
                if let Some(cmd) = proto
                    .pending_render_commands
                    .iter_mut()
                    .find(|c| c.id == window_id)
                {
                    cmd.z_order = ZOrder::Top;
                } else {
                    proto.pending_render_commands.push(RenderCommand {
                        id: window_id,
                        position: (0, 0),
                        z_order: ZOrder::Top,
                        visible: true,
                    });
                }
            }
            wayray_wm_window_v1::Request::SetZBottom => {
                if let Some(cmd) = proto
                    .pending_render_commands
                    .iter_mut()
                    .find(|c| c.id == window_id)
                {
                    cmd.z_order = ZOrder::Bottom;
                } else {
                    proto.pending_render_commands.push(RenderCommand {
                        id: window_id,
                        position: (0, 0),
                        z_order: ZOrder::Bottom,
                        visible: true,
                    });
                }
            }
            wayray_wm_window_v1::Request::Show => {
                if let Some(cmd) = proto
                    .pending_render_commands
                    .iter_mut()
                    .find(|c| c.id == window_id)
                {
                    cmd.visible = true;
                } else {
                    proto.pending_render_commands.push(RenderCommand {
                        id: window_id,
                        position: (0, 0),
                        z_order: ZOrder::Preserve,
                        visible: true,
                    });
                }
            }
            wayray_wm_window_v1::Request::Hide => {
                if let Some(cmd) = proto
                    .pending_render_commands
                    .iter_mut()
                    .find(|c| c.id == window_id)
                {
                    cmd.visible = false;
                } else {
                    proto.pending_render_commands.push(RenderCommand {
                        id: window_id,
                        position: (0, 0),
                        z_order: ZOrder::Preserve,
                        visible: false,
                    });
                }
            }
            wayray_wm_window_v1::Request::SetZAbove { .. }
            | wayray_wm_window_v1::Request::SetZBelow { .. } => {
                // TODO: relative z-ordering (needs sibling window lookup)
            }
            wayray_wm_window_v1::Request::SetBorders { .. } => {
                // TODO: border rendering
            }
            wayray_wm_window_v1::Request::SetOutput { .. } => {
                // TODO: multi-output support
            }
            wayray_wm_window_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        _resource: &WayrayWmWindowV1,
        data: &WmWindowData,
    ) {
        state
            .wm_protocol_state()
            .window_objects
            .remove(&data.window_id);
    }
}

// --- Seat ---

impl<D: WmProtocolHandler> Dispatch<WayrayWmSeatV1, (), D> for WmProtocolState {
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &WayrayWmSeatV1,
        request: wayray_wm_seat_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let proto = state.wm_protocol_state();

        // Store the seat object for sending binding events later.
        if proto.wm_seat.is_none() {
            proto.wm_seat = Some(resource.clone());
        }

        match request {
            wayray_wm_seat_v1::Request::BindKey {
                key,
                modifiers,
                mode,
            } => {
                proto.keybindings.insert((key, modifiers, mode.clone()));
                info!(key, modifiers, mode, "WM registered keybinding");
            }
            wayray_wm_seat_v1::Request::UnbindKey {
                key,
                modifiers,
                mode,
            } => {
                proto.keybindings.remove(&(key, modifiers, mode));
            }
            wayray_wm_seat_v1::Request::CreateMode { name } => {
                proto.binding_modes.insert(name);
            }
            wayray_wm_seat_v1::Request::ActivateMode { name } => {
                proto.active_mode = name;
            }
            wayray_wm_seat_v1::Request::StartMove { .. } => {
                // TODO: interactive move
            }
            wayray_wm_seat_v1::Request::StartResize { .. } => {
                // TODO: interactive resize
            }
            wayray_wm_seat_v1::Request::Destroy => {
                proto.wm_seat = None;
                proto.keybindings.clear();
            }
            _ => {}
        }
    }
}

// --- Workspace ---

impl<D: WmProtocolHandler> Dispatch<WayrayWmWorkspaceV1, (), D> for WmProtocolState {
    fn request(
        _state: &mut D,
        _client: &Client,
        resource: &WayrayWmWorkspaceV1,
        request: wayray_wm_workspace_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wayray_wm_workspace_v1::Request::CreateWorkspace { name } => {
                // TODO: create workspace
                resource.workspace_created(name);
            }
            wayray_wm_workspace_v1::Request::DestroyWorkspace { name } => {
                // TODO: destroy workspace
                resource.workspace_destroyed(name);
            }
            wayray_wm_workspace_v1::Request::SetActiveWorkspace { .. } => {
                // TODO: set active workspace
            }
            wayray_wm_workspace_v1::Request::AssignWindow { .. } => {
                // TODO: assign window to workspace
            }
            wayray_wm_workspace_v1::Request::SetWindowTags { .. } => {
                // TODO: set window tags
            }
            wayray_wm_workspace_v1::Request::Destroy => {}
            _ => {}
        }
    }
}
