mod backend;
mod errors;
mod handlers;
mod state;

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

    info!(
        backend = if use_winit { "winit" } else { "headless" },
        "dispatching to backend"
    );

    if use_winit {
        backend::winit::run(display, state, output)
    } else {
        backend::headless::run(display, state, output)
    }
}
