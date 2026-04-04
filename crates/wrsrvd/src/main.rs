mod errors;
mod handlers;
mod state;

use crate::state::WayRay;
use miette::Result;
use smithay::reexports::wayland_server::Display;
use tracing::info;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("wrsrvd starting");

    let mut display = Display::<WayRay>::new()
        .map_err(|e| errors::WayRayError::DisplayInit(Box::new(e)))?;
    let _state = WayRay::new(&mut display);
    info!("compositor state initialized");

    Ok(())
}
