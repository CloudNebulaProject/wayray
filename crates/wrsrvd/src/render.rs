use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker,
            element::texture::TextureRenderElement,
            gles::{GlesRenderer, GlesTexture},
        },
        winit::WinitGraphicsBackend,
    },
    desktop::{Window, space::render_output},
};
use tracing::warn;

use crate::state::WayRay;

/// Dark grey clear color for the compositor background.
const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

/// Render the compositor space to the Winit backend window.
///
/// Uses `OutputDamageTracker` for efficient re-rendering: only
/// damaged regions are redrawn each frame.
///
/// Returns `true` if any damage was present and submitted.
pub fn render_output_frame(
    state: &mut WayRay,
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: &mut OutputDamageTracker,
) -> bool {
    let output = state.output.clone();

    // Get buffer age before bind (avoids borrow conflict).
    let age = backend.buffer_age().unwrap_or(0);

    // Render within a block so framebuffer is dropped before submit.
    let render_damage = {
        let (renderer, mut framebuffer) = match backend.bind() {
            Ok(pair) => pair,
            Err(err) => {
                warn!(?err, "failed to bind winit backend for rendering");
                return false;
            }
        };

        // The empty custom elements slice needs a concrete type.
        let custom_elements: &[TextureRenderElement<GlesTexture>] = &[];

        let render_result = render_output::<_, _, Window, _>(
            &output,
            renderer,
            &mut framebuffer,
            1.0, // alpha
            age,
            [&state.space],
            custom_elements,
            damage_tracker,
            CLEAR_COLOR,
        );

        match render_result {
            Ok(result) => {
                // Clone the damage rectangles so we can use them after
                // the framebuffer is dropped.
                Ok(result.damage.cloned())
            }
            Err(err) => {
                warn!(?err, "damage tracker render failed");
                Err(())
            }
        }
    };
    // framebuffer is now dropped, backend is no longer borrowed.

    match render_damage {
        Ok(damage) => {
            let has_damage = damage.is_some();

            let submit_result = if let Some(ref rects) = damage {
                backend.submit(Some(rects))
            } else {
                backend.submit(None)
            };

            if let Err(err) = submit_result {
                warn!(?err, "failed to submit frame");
                return false;
            }

            // Send frame callbacks to all mapped surfaces so clients
            // know they can draw the next frame.
            let time = state.clock.now();
            for window in state.space.elements() {
                window.send_frame(&output, time, Some(std::time::Duration::ZERO), |_, _| {
                    Some(output.clone())
                });
            }

            has_damage
        }
        Err(()) => false,
    }
}
