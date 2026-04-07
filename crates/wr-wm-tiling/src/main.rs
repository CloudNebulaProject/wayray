//! Reference tiling window manager for WayRay.
//!
//! Demonstrates the WayRay WM protocol with a simple BSP (binary space
//! partitioning) tiling layout. Windows are tiled into a grid that splits
//! alternately horizontally and vertically.

use tracing::{info, warn};
use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_registry};
use wayray_wm_protocol::client::{
    wayray_wm_manager_v1::{self, WayrayWmManagerV1},
    wayray_wm_seat_v1::WayrayWmSeatV1,
    wayray_wm_window_v1::{self, WayrayWmWindowV1},
    wayray_wm_workspace_v1::WayrayWmWorkspaceV1,
};

/// Per-window state tracked by the tiling WM.
#[derive(Debug)]
struct WindowState {
    obj: WayrayWmWindowV1,
    width: i32,
    height: i32,
}

/// Central state for the tiling WM client.
struct TilingWm {
    manager: Option<WayrayWmManagerV1>,
    windows: Vec<WindowState>,
    /// Output dimensions (set from output_new event).
    output_width: i32,
    output_height: i32,
    /// Whether we're in the manage phase.
    in_manage_phase: bool,
    /// Whether we're in the render phase.
    in_render_phase: bool,
}

impl TilingWm {
    fn new() -> Self {
        Self {
            manager: None,
            windows: Vec::new(),
            output_width: 1280,
            output_height: 720,
            in_manage_phase: false,
            in_render_phase: false,
        }
    }

    /// Calculate BSP tiling positions for all windows.
    /// Returns (x, y, width, height) for each window.
    fn calculate_layout(&self) -> Vec<(i32, i32, i32, i32)> {
        let n = self.windows.len();
        if n == 0 {
            return Vec::new();
        }

        let gap = 4;
        let mut layouts = Vec::with_capacity(n);

        if n == 1 {
            layouts.push((
                gap,
                gap,
                self.output_width - 2 * gap,
                self.output_height - 2 * gap,
            ));
            return layouts;
        }

        bsp_layout(
            gap,
            gap,
            self.output_width - 2 * gap,
            self.output_height - 2 * gap,
            n,
            true,
            &mut layouts,
        );

        layouts
    }
}

/// Recursive BSP layout: split area alternately horizontal/vertical.
fn bsp_layout(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    count: usize,
    horizontal: bool,
    layouts: &mut Vec<(i32, i32, i32, i32)>,
) {
    let gap = 4;
    if count == 1 {
        layouts.push((x, y, w, h));
        return;
    }

    let first_half = count / 2;
    let second_half = count - first_half;

    if horizontal {
        let first_w = w * first_half as i32 / count as i32 - gap / 2;
        let second_w = w - first_w - gap;
        bsp_layout(x, y, first_w, h, first_half, !horizontal, layouts);
        bsp_layout(
            x + first_w + gap,
            y,
            second_w,
            h,
            second_half,
            !horizontal,
            layouts,
        );
    } else {
        let first_h = h * first_half as i32 / count as i32 - gap / 2;
        let second_h = h - first_h - gap;
        bsp_layout(x, y, w, first_h, first_half, !horizontal, layouts);
        bsp_layout(
            x,
            y + first_h + gap,
            w,
            second_h,
            second_half,
            !horizontal,
            layouts,
        );
    }
}

// =============================================================================
// Wayland client dispatch implementations
// =============================================================================

impl Dispatch<wl_registry::WlRegistry, ()> for TilingWm {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "wayray_wm_manager_v1"
        {
            info!("found wayray_wm_manager_v1 global");
            let manager = registry.bind::<WayrayWmManagerV1, _, _>(name, version, qh, ());
            state.manager = Some(manager);
        }
    }
}

impl Dispatch<WayrayWmManagerV1, ()> for TilingWm {
    fn event(
        state: &mut Self,
        manager: &WayrayWmManagerV1,
        event: wayray_wm_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wayray_wm_manager_v1::Event::WindowNew { window } => {
                info!("new window received from compositor");
                state.windows.push(WindowState {
                    obj: window,
                    width: 0,
                    height: 0,
                });
            }
            wayray_wm_manager_v1::Event::WindowClosed { window } => {
                info!("window closed");
                state.windows.retain(|w| w.obj != window);
            }
            wayray_wm_manager_v1::Event::ManageStart => {
                state.in_manage_phase = true;

                // Calculate dimensions for all windows.
                let layouts = state.calculate_layout();
                for (i, (_, _, w, h)) in layouts.iter().enumerate() {
                    if i < state.windows.len() {
                        state.windows[i].obj.propose_dimensions(*w, *h);
                    }
                }

                // Focus the last (most recent) window.
                if let Some(last) = state.windows.last() {
                    last.obj.set_focus();
                }

                manager.manage_done();
                state.in_manage_phase = false;
            }
            wayray_wm_manager_v1::Event::RenderStart => {
                state.in_render_phase = true;

                // Apply BSP tiling positions.
                let layouts = state.calculate_layout();
                for (i, (x, y, _w, _h)) in layouts.iter().enumerate() {
                    if i < state.windows.len() {
                        state.windows[i].obj.set_position(*x, *y);
                        state.windows[i].obj.set_z_top();
                        state.windows[i].obj.show();
                    }
                }

                manager.render_done();
                state.in_render_phase = false;
            }
            wayray_wm_manager_v1::Event::OutputNew {
                output_name,
                width,
                height,
            } => {
                info!(output_name, width, height, "output available");
                state.output_width = width;
                state.output_height = height;
            }
            wayray_wm_manager_v1::Event::OutputRemoved { output_name } => {
                info!(output_name, "output removed");
            }
            wayray_wm_manager_v1::Event::Replaced => {
                warn!("replaced by another WM, shutting down");
                std::process::exit(0);
            }
            _ => {}
        }
    }
}

impl Dispatch<WayrayWmWindowV1, ()> for TilingWm {
    fn event(
        state: &mut Self,
        window: &WayrayWmWindowV1,
        event: wayray_wm_window_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wayray_wm_window_v1::Event::Title { title } => {
                info!(title, "window title");
            }
            wayray_wm_window_v1::Event::AppId { app_id } => {
                info!(app_id, "window app_id");
            }
            wayray_wm_window_v1::Event::Dimensions { width, height } => {
                if let Some(ws) = state.windows.iter_mut().find(|w| w.obj == *window) {
                    ws.width = width;
                    ws.height = height;
                }
            }
            wayray_wm_window_v1::Event::Done => {
                // End of property batch — nothing to do for tiling.
            }
            _ => {}
        }
    }
}

impl Dispatch<WayrayWmSeatV1, ()> for TilingWm {
    fn event(
        _state: &mut Self,
        _proxy: &WayrayWmSeatV1,
        _event: <WayrayWmSeatV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WayrayWmWorkspaceV1, ()> for TilingWm {
    fn event(
        _state: &mut Self,
        _proxy: &WayrayWmWorkspaceV1,
        _event: <WayrayWmWorkspaceV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("wr-wm-tiling starting");

    let conn = Connection::connect_to_env().expect("failed to connect to Wayland display");
    let display = conn.display();

    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let _registry = display.get_registry(&qh, ());

    let mut state = TilingWm::new();

    // Initial roundtrip to discover globals.
    event_queue
        .roundtrip(&mut state)
        .expect("initial roundtrip failed");

    if state.manager.is_none() {
        eprintln!("wayray_wm_manager_v1 global not found — is this a WayRay compositor?");
        std::process::exit(1);
    }

    info!("connected to WayRay compositor, entering event loop");

    loop {
        event_queue
            .blocking_dispatch(&mut state)
            .expect("event dispatch failed");
    }
}
