use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ExportMem,
            damage::OutputDamageTracker,
            element::texture::TextureRenderElement,
            gles::{GlesRenderer, GlesTexture},
        },
        winit::WinitGraphicsBackend,
    },
    desktop::{Window, space::render_output},
    utils::{Buffer as BufferCoord, Rectangle, Size},
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
    let render_result = {
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
                let damage = result.damage.cloned();

                // Verify framebuffer capture path works (will be consumed
                // by network transport in Phase 1).
                let output_size = state.output.current_mode().unwrap().size;
                let region: Rectangle<i32, BufferCoord> =
                    Rectangle::from_size(Size::from((output_size.w, output_size.h)));

                match renderer.copy_framebuffer(&framebuffer, region, Fourcc::Argb8888) {
                    Ok(mapping) => match renderer.map_texture(&mapping) {
                        Ok(pixels) => {
                            let damage_rects = damage.as_ref().map(|d| d.len()).unwrap_or(0);
                            tracing::debug!(
                                width = output_size.w,
                                height = output_size.h,
                                bytes = pixels.len(),
                                damage_rects,
                                "framebuffer captured"
                            );
                        }
                        Err(err) => {
                            tracing::warn!(?err, "failed to map framebuffer");
                        }
                    },
                    Err(err) => {
                        tracing::warn!(?err, "failed to copy framebuffer");
                    }
                }

                Ok(damage)
            }
            Err(err) => {
                warn!(?err, "damage tracker render failed");
                Err(())
            }
        }
    };
    // framebuffer is now dropped, backend is no longer borrowed.

    match render_result {
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
