mod backend;
mod errors;
mod handlers;
pub mod network;
mod state;

use crate::network::{ServerConfig, start_server};
use crate::state::WayRay;
use miette::Result;
use smithay::{
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::Display,
    utils::Transform,
};
use tracing::info;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("wrsrvd starting");

    // Parse backend selection from CLI args.
    let args: Vec<String> = std::env::args().collect();
    let use_winit = args
        .windows(2)
        .any(|w| w[0] == "--backend" && w[1] == "winit");

    // Create the Wayland display.
    let mut display =
        Display::<WayRay>::new().map_err(|e| errors::WayRayError::DisplayInit(Box::new(e)))?;

    // Create a virtual output.
    let output = Output::new(
        "wayray-0".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "WayRay".to_string(),
            model: "Virtual".to_string(),
        },
    );

    // Default to 1280x720 for headless; Winit will use its window size
    // but we still need an initial mode for state setup.
    let mode = Mode {
        size: (1280, 720).into(),
        refresh: 60_000,
    };
    output.change_current_state(Some(mode), Some(Transform::Normal), None, None);
    output.set_preferred(mode);

    // Create the global output for Wayland clients to bind to.
    output.create_global::<WayRay>(&display.handle());

    // Create compositor state.
    let state = WayRay::new(&mut display, output.clone());

    // Start the QUIC network server for remote client connections.
    let output_size = mode.size;
    let net_handle = start_server(ServerConfig {
        output_width: output_size.w as u32,
        output_height: output_size.h as u32,
        ..ServerConfig::default()
    });
    info!("QUIC network server started");

    info!(
        backend = if use_winit { "winit" } else { "headless" },
        "dispatching to backend"
    );

    if use_winit {
        let result = backend::winit::run(display, state, output);
        net_handle.shutdown();
        result
    } else {
        backend::headless::run(display, state, output, net_handle)
    }
}
