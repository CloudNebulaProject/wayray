//! Integration test: encode → compress → decompress → apply.
//!
//! Verifies the full XOR diff → zstd → XOR apply pipeline
//! reconstructs the original framebuffer correctly.

use wayray_protocol::encoding::{apply_region, encode_region, xor_diff};

#[test]
fn roundtrip_with_subregion() {
    let width = 100u32;
    let height = 100u32;
    let stride = width as usize * 4;
    let prev = vec![0u8; stride * height as usize];
    let mut curr = prev.clone();

    // Paint a 10x10 red rectangle at (5, 5)
    for y in 5..15u32 {
        for x in 5..15u32 {
            let offset = y as usize * stride + x as usize * 4;
            curr[offset..offset + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
    }

    let diff = xor_diff(&curr, &prev);
    let region = encode_region(&diff, stride, 5, 5, 10, 10);

    let mut reconstructed = prev;
    apply_region(&mut reconstructed, stride, &region);
    assert_eq!(reconstructed, curr);
}

#[test]
fn roundtrip_unchanged_frame() {
    let width = 50u32;
    let height = 50u32;
    let stride = width as usize * 4;
    let frame = vec![128u8; stride * height as usize];

    let diff = xor_diff(&frame, &frame);
    let region = encode_region(&diff, stride, 0, 0, width, height);

    assert!(
        region.data.len() < 200,
        "all-zero diff should compress well"
    );

    let mut reconstructed = frame.clone();
    apply_region(&mut reconstructed, stride, &region);
    assert_eq!(reconstructed, frame);
}

#[test]
fn roundtrip_multiple_regions() {
    let width = 200u32;
    let height = 200u32;
    let stride = width as usize * 4;
    let prev = vec![0u8; stride * height as usize];
    let mut curr = prev.clone();

    for y in 0..10u32 {
        for x in 0..10u32 {
            let offset = y as usize * stride + x as usize * 4;
            curr[offset..offset + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
    }

    for y in 100..120u32 {
        for x in 150..180u32 {
            let offset = y as usize * stride + x as usize * 4;
            curr[offset..offset + 4].copy_from_slice(&[0, 255, 0, 255]);
        }
    }

    let diff = xor_diff(&curr, &prev);
    let region1 = encode_region(&diff, stride, 0, 0, 10, 10);
    let region2 = encode_region(&diff, stride, 150, 100, 30, 20);

    let mut reconstructed = prev;
    apply_region(&mut reconstructed, stride, &region1);
    apply_region(&mut reconstructed, stride, &region2);
    assert_eq!(reconstructed, curr);
}
