//! Frame encoding and decoding: absolute region pixels + zstd compression.
//!
//! The encoder (server-side) extracts changed rectangles of the framebuffer
//! and compresses their *absolute* pixels. The decoder (client-side) copies
//! them into its framebuffer. Because regions carry absolute pixels (not a diff
//! against the previous frame), applying them is idempotent: a missed or
//! reordered frame can never permanently corrupt the reconstruction — at worst
//! a region is briefly stale until it is redrawn or a keyframe resyncs it.

use crate::messages::DamageRegion;

// ── Encoder (server side) ───────────────────────────────────────────

/// Extract a rectangular region of absolute pixels from a framebuffer and
/// compress it.
///
/// `stride` is the number of bytes per row in the full framebuffer. The region
/// is compressed with zstd at level 1 (fast).
pub fn encode_region(
    pixels: &[u8],
    stride: usize,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> DamageRegion {
    let bpp = 4; // BGRA8888
    let region_stride = width as usize * bpp;
    let mut region_data = Vec::with_capacity(region_stride * height as usize);

    for row in 0..height as usize {
        let src_offset = (y as usize + row) * stride + x as usize * bpp;
        region_data.extend_from_slice(&pixels[src_offset..src_offset + region_stride]);
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

/// Fast checksum (FNV-1a, 8 bytes per step) over the *visible* pixels of a
/// framebuffer — `row_bytes` per row, skipping any stride padding — so the
/// server and client agree on the value regardless of buffer padding.
pub fn checksum(data: &[u8], stride: usize, row_bytes: usize, height: usize) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for row in 0..height {
        let start = row * stride;
        let line = &data[start..start + row_bytes];
        for chunk in line.chunks(8) {
            let mut v = 0u64;
            for (i, &b) in chunk.iter().enumerate() {
                v |= (b as u64) << (i * 8);
            }
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    (h ^ (h >> 32)) as u32
}

// ── Decoder (client side) ───────────────────────────────────────────

/// Copy a compressed absolute-pixel region onto a framebuffer.
///
/// Decompresses the region data with zstd, then copies it at the region's
/// position. Idempotent (a plain copy), so re-applying or skipping a frame
/// never corrupts the result. `stride` is the bytes per row in the framebuffer.
pub fn apply_region(framebuffer: &mut [u8], stride: usize, region: &DamageRegion) {
    let bpp = 4;
    let decompressed = zstd::decode_all(region.data.as_slice()).expect("zstd decode failed");
    let region_stride = region.width as usize * bpp;

    for row in 0..region.height as usize {
        let dst_offset = (region.y as usize + row) * stride + region.x as usize * bpp;
        let src_offset = row * region_stride;
        framebuffer[dst_offset..dst_offset + region_stride]
            .copy_from_slice(&decompressed[src_offset..src_offset + region_stride]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_apply_round_trip() {
        // A 4x2 region of distinct pixels placed into a 4x2 framebuffer.
        let width = 4u32;
        let height = 2u32;
        let stride = width as usize * 4;
        let mut pixels = vec![0u8; stride * height as usize];
        for (i, b) in pixels.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let region = encode_region(&pixels, stride, 0, 0, width, height);

        let mut framebuffer = vec![0xAAu8; stride * height as usize];
        apply_region(&mut framebuffer, stride, &region);
        assert_eq!(framebuffer, pixels, "absolute region copy must reproduce source");

        // Re-applying is idempotent (a copy, not an accumulating XOR).
        apply_region(&mut framebuffer, stride, &region);
        assert_eq!(framebuffer, pixels, "apply_region must be idempotent");
    }

    #[test]
    fn encode_region_compresses_zeros() {
        let width = 100usize;
        let height = 100usize;
        let stride = width * 4;
        let pixels = vec![0u8; stride * height];
        let region = encode_region(&pixels, stride, 0, 0, width as u32, height as u32);
        assert!(region.data.len() < 1000);
    }

    #[test]
    fn checksum_detects_change() {
        let stride = 40; // 10 px * 4 bpp
        let row_bytes = stride;
        let height = 10;
        let a = vec![7u8; stride * height];
        let mut b = a.clone();
        let c1 = checksum(&a, stride, row_bytes, height);
        assert_eq!(c1, checksum(&b, stride, row_bytes, height));
        b[123] ^= 0x01;
        assert_ne!(c1, checksum(&b, stride, row_bytes, height));
    }

    #[test]
    fn checksum_ignores_stride_padding() {
        // Two buffers with identical visible pixels but different padding bytes
        // must checksum equal.
        let row_bytes = 12; // 3 px
        let stride = 16; // 4 bytes padding per row
        let height = 3;
        let mut a = vec![0u8; stride * height];
        let mut b = vec![0u8; stride * height];
        for row in 0..height {
            for i in 0..row_bytes {
                let v = (row * 7 + i) as u8;
                a[row * stride + i] = v;
                b[row * stride + i] = v;
            }
            // Differ only in padding.
            a[row * stride + row_bytes] = 0xFF;
            b[row * stride + row_bytes] = 0x00;
        }
        assert_eq!(
            checksum(&a, stride, row_bytes, height),
            checksum(&b, stride, row_bytes, height)
        );
    }
}
