//! Frame encoding and decoding: XOR diff + zstd compression.
//!
//! The encoder (server-side) produces `DamageRegion` messages from
//! framebuffer changes. The decoder (client-side) applies them to
//! reconstruct the framebuffer.

use crate::messages::DamageRegion;

// ── Encoder (server side) ───────────────────────────────────────────

/// Compute XOR diff between two ARGB8888 framebuffers.
///
/// Returns a new buffer where unchanged pixels are zero.
/// Both buffers must have the same length.
pub fn xor_diff(current: &[u8], previous: &[u8]) -> Vec<u8> {
    assert_eq!(
        current.len(),
        previous.len(),
        "framebuffer size mismatch: {} vs {}",
        current.len(),
        previous.len()
    );
    current
        .iter()
        .zip(previous.iter())
        .map(|(a, b)| a ^ b)
        .collect()
}

/// Extract a rectangular region from an XOR diff buffer and compress it.
///
/// `stride` is the number of bytes per row in the full framebuffer.
/// The region is compressed with zstd at level 1 (fast).
pub fn encode_region(
    diff: &[u8],
    stride: usize,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> DamageRegion {
    let bpp = 4; // ARGB8888
    let region_stride = width as usize * bpp;
    let mut region_data = Vec::with_capacity(region_stride * height as usize);

    for row in 0..height as usize {
        let src_offset = (y as usize + row) * stride + x as usize * bpp;
        region_data.extend_from_slice(&diff[src_offset..src_offset + region_stride]);
    }

    let compressed = zstd::encode_all(region_data.as_slice(), 1).expect("zstd encode failed");

    DamageRegion {
        x,
        y,
        width,
        height,
        data: compressed,
    }
}

// ── Decoder (client side) ───────────────────────────────────────────

/// Apply a compressed XOR diff region onto a framebuffer.
///
/// Decompresses the region data with zstd, then XOR-applies it at the
/// specified position. Modifies `framebuffer` in place.
///
/// `stride` is the number of bytes per row in the full framebuffer.
pub fn apply_region(framebuffer: &mut [u8], stride: usize, region: &DamageRegion) {
    let bpp = 4;
    let decompressed = zstd::decode_all(region.data.as_slice()).expect("zstd decode failed");
    let region_stride = region.width as usize * bpp;

    for row in 0..region.height as usize {
        let dst_offset = (region.y as usize + row) * stride + region.x as usize * bpp;
        let src_offset = row * region_stride;
        let dst_row = &mut framebuffer[dst_offset..dst_offset + region_stride];
        let src_row = &decompressed[src_offset..src_offset + region_stride];
        for (d, s) in dst_row.iter_mut().zip(src_row.iter()) {
            *d ^= s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_diff_identical_frames() {
        let frame = vec![100u8; 400];
        let diff = xor_diff(&frame, &frame);
        assert!(diff.iter().all(|&b| b == 0));
    }

    #[test]
    fn xor_diff_changed_pixel() {
        let prev = vec![0u8; 16];
        let mut curr = vec![0u8; 16];
        curr[0..4].copy_from_slice(&[255, 128, 64, 255]);
        let diff = xor_diff(&curr, &prev);
        assert_eq!(&diff[0..4], &[255, 128, 64, 255]);
        assert!(diff[4..].iter().all(|&b| b == 0));
    }

    #[test]
    fn encode_region_compresses_zeros() {
        let width = 100usize;
        let height = 100usize;
        let stride = width * 4;
        let diff = vec![0u8; stride * height];
        let region = encode_region(&diff, stride, 0, 0, width as u32, height as u32);
        assert!(region.data.len() < 1000);
    }

    #[test]
    fn apply_region_zeros_is_noop() {
        let stride = 40; // 10 pixels * 4 bpp
        let mut framebuffer = vec![42u8; stride * 10];
        let original = framebuffer.clone();
        let zero_data = vec![0u8; stride * 10];
        let compressed = zstd::encode_all(zero_data.as_slice(), 1).unwrap();
        let region = DamageRegion {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
            data: compressed,
        };
        apply_region(&mut framebuffer, stride, &region);
        assert_eq!(framebuffer, original);
    }
}
