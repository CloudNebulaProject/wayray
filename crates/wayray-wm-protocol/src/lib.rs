//! WayRay pluggable window management Wayland protocol.
//!
//! Provides generated Rust bindings for the `wayray_wm_v1` protocol family:
//! - `wayray_wm_manager_v1` — global WM entry point
//! - `wayray_wm_window_v1` — per-window management
//! - `wayray_wm_seat_v1` — keybindings and interactive operations
//! - `wayray_wm_workspace_v1` — virtual desktop / tag management
//!
//! Enable the `server` feature for compositor-side bindings or
//! `client` feature for WM client-side bindings.

#![allow(
    dead_code,
    non_camel_case_types,
    unused_unsafe,
    unused_variables,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    missing_docs,
    clippy::all
)]

#[cfg(feature = "client")]
pub mod client {
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocols/wayray-wm-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("./protocols/wayray-wm-v1.xml");
}

#[cfg(feature = "server")]
pub mod server {
    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocols/wayray-wm-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_server_code!("./protocols/wayray-wm-v1.xml");
}
