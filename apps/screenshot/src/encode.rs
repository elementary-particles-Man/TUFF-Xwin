use anyhow::{Context, Result, anyhow, bail};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::capture::{CapturedFrame, validate_rgba_buffer_size};
use crate::config::{ScreenshotConfig, ScreenshotFormat};

pub fn save_captured_frame(
    config: &ScreenshotConfig,
    frame: &CapturedFrame,
    sequence: u64,
) -> Result<PathBuf> {
    config.validate()?;
    validate_captured_frame(frame)?;
    fs::create_dir_all(&config.save_dir)
        .with_context(|| format!("failed to create save dir {}", config.save_dir.display()))?;
    let output_path = config.artifact_path(sequence)?;
    match config.format {
        ScreenshotFormat::Png => {
            encode_rgba_png(&output_path, frame, config.png.compression_level)?
        }
        ScreenshotFormat::Jpeg => encode_rgba_jpeg(&output_path, frame, config.jpeg.quality)?,
    }
    Ok(output_path)
}

pub fn validate_captured_frame(frame: &CapturedFrame) -> Result<()> {
    validate_rgba_buffer_size(frame.width, frame.height, frame.rgba.len())
}

pub fn encode_rgba_png(path: &Path, frame: &CapturedFrame, compression_level: u8) -> Result<()> {
    validate_captured_frame(frame)?;
    let file = File::create(path)
        .with_context(|| format!("failed to create png file {}", path.display()))?;
    let mut encoder = png::Encoder::new(file, frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(map_png_compression(compression_level));
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&frame.rgba)?;
    Ok(())
}

pub fn encode_rgba_jpeg(path: &Path, frame: &CapturedFrame, quality: u8) -> Result<()> {
    validate_captured_frame(frame)?;
    let file = File::create(path)
        .with_context(|| format!("failed to create jpeg file {}", path.display()))?;
    let rgb = rgba_to_rgb(&frame.rgba)?;
    let width = u16::try_from(frame.width).map_err(|_| anyhow!("jpeg width exceeds u16"))?;
    let height = u16::try_from(frame.height).map_err(|_| anyhow!("jpeg height exceeds u16"))?;
    let encoder = jpeg_encoder::Encoder::new(file, quality);
    encoder.encode(&rgb, width, height, jpeg_encoder::ColorType::Rgb)?;
    Ok(())
}

fn rgba_to_rgb(rgba: &[u8]) -> Result<Vec<u8>> {
    if rgba.len() % 4 != 0 {
        bail!("rgba buffer length must be divisible by 4");
    }
    let mut rgb = Vec::with_capacity((rgba.len() / 4) * 3);
    for pixel in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }
    Ok(rgb)
}

fn map_png_compression(level: u8) -> png::Compression {
    match level {
        0..=2 => png::Compression::Fast,
        3..=6 => png::Compression::Default,
        _ => png::Compression::Best,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    use crate::capture::{CaptureClient, FakeCaptureClient};
    use crate::config::{CaptureTarget, ScreenshotConfig, ScreenshotFormat};

    #[test]
    fn rgba_size_mismatch_is_rejected() {
        let err = validate_rgba_buffer_size(2, 2, 8).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("mismatch"));
    }

    #[test]
    fn png_encode_writes_file_under_tempdir() {
        let dir = tempdir().unwrap();
        let config =
            ScreenshotConfig { save_dir: dir.path().to_path_buf(), ..ScreenshotConfig::default() };
        let client = FakeCaptureClient::default();
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        let path = save_captured_frame(&config, &frame, 1).unwrap();
        assert_eq!(path.extension().and_then(|s| s.to_str()), Some("png"));
        assert!(path.exists());
    }

    #[test]
    fn jpeg_encode_writes_file_under_tempdir() {
        let dir = tempdir().unwrap();
        let config = ScreenshotConfig {
            save_dir: dir.path().to_path_buf(),
            format: ScreenshotFormat::Jpeg,
            ..ScreenshotConfig::default()
        };
        let client = FakeCaptureClient::default();
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        let path = save_captured_frame(&config, &frame, 7).unwrap();
        assert_eq!(path.extension().and_then(|s| s.to_str()), Some("jpg"));
        assert!(path.exists());
    }

    #[test]
    fn existing_png_jpeg_encode_tests_still_pass() {
        let dir = tempdir().unwrap();
        let png_config =
            ScreenshotConfig { save_dir: dir.path().to_path_buf(), ..ScreenshotConfig::default() };
        let jpeg_config = ScreenshotConfig {
            save_dir: dir.path().to_path_buf(),
            format: ScreenshotFormat::Jpeg,
            ..ScreenshotConfig::default()
        };
        let client = FakeCaptureClient::default();
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        let png = save_captured_frame(&png_config, &frame, 1).unwrap();
        let jpeg = save_captured_frame(&jpeg_config, &frame, 2).unwrap();
        assert_eq!(png.extension().and_then(|s| s.to_str()), Some("png"));
        assert_eq!(jpeg.extension().and_then(|s| s.to_str()), Some("jpg"));
        assert!(png.exists());
        assert!(jpeg.exists());
    }
}
