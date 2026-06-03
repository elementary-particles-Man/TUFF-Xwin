use wayland_wire::{
    pixel_ops::{
        normalize_bgra_to_rgba_bytes, normalize_bgra_to_rgba_in_place_with_path,
        selected_pixel_path, PixelOpPath,
    },
    screencopy::CaptureScratch,
};

#[test]
fn test_pixel_ops_portable_and_selected_match() {
    let input = [0xff33_2211u32, 0x8044_5566, 0x0001_0203, 0x7f10_2030];
    let mut portable = input;
    let mut selected = input;

    normalize_bgra_to_rgba_in_place_with_path(&mut portable, PixelOpPath::Portable);
    normalize_bgra_to_rgba_in_place_with_path(&mut selected, selected_pixel_path());

    assert_eq!(portable, selected);
}

#[test]
fn test_pixel_ops_bytes_are_swapped_in_place() {
    let mut bytes = vec![0x11, 0x22, 0x33, 0xff, 0x66, 0x55, 0x44, 0x80];

    normalize_bgra_to_rgba_bytes(&mut bytes).expect("normalize");

    assert_eq!(bytes, vec![0x33, 0x22, 0x11, 0xff, 0x44, 0x55, 0x66, 0x80]);
}

#[test]
fn test_pixel_ops_rejects_non_aligned_bytes() {
    let mut bytes = vec![0x11, 0x22, 0x33];

    let err = normalize_bgra_to_rgba_bytes(&mut bytes).expect_err("should reject");
    assert_eq!(err, "pixel buffer must be 4-byte aligned");
}

#[test]
fn test_capture_scratch_reuses_capacity() {
    let mut scratch = CaptureScratch::default();

    let initial_capacity = scratch.capacity();
    let pixels = scratch.prepare_pixels(64);
    assert_eq!(pixels.len(), 64);
    assert!(scratch.capacity() >= 64);

    let after_growth = scratch.capacity();
    assert!(after_growth >= initial_capacity);
    let bytes = scratch.as_bytes();
    assert_eq!(bytes.len(), 64 * 4);

    let smaller = scratch.prepare_pixels(16);
    assert_eq!(smaller.len(), 16);
    assert!(scratch.capacity() >= after_growth);
}
