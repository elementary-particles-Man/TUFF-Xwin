use crate::accel::{selected_simd_flavor, SimdFlavor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelOpPath {
    Portable,
    Sse42,
    Avx2,
    Avx512f,
    Neon,
}

pub fn selected_pixel_path() -> PixelOpPath {
    match selected_simd_flavor() {
        SimdFlavor::Portable => PixelOpPath::Portable,
        SimdFlavor::Sse42 => PixelOpPath::Sse42,
        SimdFlavor::Avx2 => PixelOpPath::Avx2,
        SimdFlavor::Avx512f => PixelOpPath::Avx512f,
        SimdFlavor::Neon => PixelOpPath::Neon,
    }
}

pub fn normalize_bgra_to_rgba_in_place(pixels: &mut [u32]) {
    match selected_pixel_path() {
        PixelOpPath::Avx2 => normalize_bgra_to_rgba_in_place_avx2(pixels),
        PixelOpPath::Avx512f | PixelOpPath::Sse42 | PixelOpPath::Neon | PixelOpPath::Portable => {
            normalize_bgra_to_rgba_in_place_portable(pixels)
        }
    }
}

pub fn normalize_bgra_to_rgba_in_place_with_path(pixels: &mut [u32], path: PixelOpPath) {
    match path {
        PixelOpPath::Avx2 => normalize_bgra_to_rgba_in_place_avx2(pixels),
        PixelOpPath::Avx512f | PixelOpPath::Sse42 | PixelOpPath::Neon | PixelOpPath::Portable => {
            normalize_bgra_to_rgba_in_place_portable(pixels)
        }
    }
}

pub fn normalize_bgra_to_rgba_bytes(bytes: &mut [u8]) -> Result<(), &'static str> {
    if !bytes.len().is_multiple_of(4) {
        return Err("pixel buffer must be 4-byte aligned");
    }

    for chunk in bytes.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }

    Ok(())
}

pub fn normalize_bgra_to_rgba_in_place_portable(pixels: &mut [u32]) {
    for pixel in pixels.iter_mut() {
        let b = (*pixel >> 16) & 0xff;
        let r = *pixel & 0xff;
        *pixel = (*pixel & 0xff00_ff00) | (r << 16) | b;
    }
}

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(target_arch = "x86_64")]
fn normalize_bgra_to_rgba_in_place_avx2(pixels: &mut [u32]) {
    if std::arch::is_x86_feature_detected!("avx2") {
        unsafe {
            normalize_bgra_to_rgba_in_place_avx2_impl(pixels);
        }
    } else {
        normalize_bgra_to_rgba_in_place_portable(pixels);
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn normalize_bgra_to_rgba_in_place_avx2(pixels: &mut [u32]) {
    normalize_bgra_to_rgba_in_place_portable(pixels);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn normalize_bgra_to_rgba_in_place_avx2_impl(pixels: &mut [u32]) {
    let mut chunks = pixels.chunks_exact_mut(8);
    let mask = _mm256_setr_epi8(
        2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15, 2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11,
        14, 13, 12, 15,
    );

    for chunk in &mut chunks {
        let ptr = chunk.as_mut_ptr() as *mut __m256i;
        let data = _mm256_loadu_si256(ptr);
        let shuffled = _mm256_shuffle_epi8(data, mask);
        _mm256_storeu_si256(ptr, shuffled);
    }

    normalize_bgra_to_rgba_in_place_portable(chunks.into_remainder());
}

#[cfg(test)]
mod tests {
    use super::{normalize_bgra_to_rgba_in_place_with_path, selected_pixel_path, PixelOpPath};

    #[test]
    fn portable_and_selected_paths_match() {
        let input = [0xff33_2211u32, 0x8044_5566, 0x0001_0203, 0x7f10_2030];
        let mut portable = input;
        let mut selected = input;

        normalize_bgra_to_rgba_in_place_with_path(&mut portable, PixelOpPath::Portable);
        normalize_bgra_to_rgba_in_place_with_path(&mut selected, selected_pixel_path());

        assert_eq!(portable, selected);
    }

    #[test]
    fn selected_path_is_stable() {
        let path = selected_pixel_path();
        assert!(matches!(
            path,
            PixelOpPath::Portable
                | PixelOpPath::Sse42
                | PixelOpPath::Avx2
                | PixelOpPath::Avx512f
                | PixelOpPath::Neon
        ));
    }
}
