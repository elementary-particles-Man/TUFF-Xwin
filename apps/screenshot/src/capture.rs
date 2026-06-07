use anyhow::{Result, anyhow, bail};
use std::path::PathBuf;

use crate::config::CaptureTarget;

pub const DISPLAYD_SCREENSHOT_FORMAT_RGBA8888: &str = "RGBA8888";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplaydRawArtifactEncoding {
    Rgba8888,
}

impl DisplaydRawArtifactEncoding {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rgba8888 => DISPLAYD_SCREENSHOT_FORMAT_RGBA8888,
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            DISPLAYD_SCREENSHOT_FORMAT_RGBA8888 => Ok(Self::Rgba8888),
            other => bail!(
                "displayd screenshot artifact format must be {DISPLAYD_SCREENSHOT_FORMAT_RGBA8888}, got {other}"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplaydCaptureArtifact {
    pub output: String,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub artifact_path: PathBuf,
}

impl DisplaydCaptureArtifact {
    pub fn new(
        output: impl Into<String>,
        width: u32,
        height: u32,
        format: impl Into<String>,
        artifact_path: impl Into<PathBuf>,
    ) -> Result<Self> {
        let output = output.into();
        if output.trim().is_empty() {
            bail!("displayd capture artifact output must not be empty");
        }
        if width == 0 || height == 0 {
            bail!("displayd capture artifact dimensions must be non-zero");
        }
        let format = format.into();
        validate_displayd_artifact_format(&format)?;
        let artifact_path = artifact_path.into();
        if artifact_path.as_os_str().is_empty() {
            bail!("displayd capture artifact path must not be empty");
        }
        Ok(Self { output, width, height, format, artifact_path })
    }
}

impl CapturedFrame {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self> {
        validate_rgba_buffer_size(width, height, rgba.len())?;
        Ok(Self { width, height, rgba })
    }

    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Result<Self> {
        let pixels =
            width.checked_mul(height).ok_or_else(|| anyhow!("rgba buffer size overflow"))? as usize;
        validate_rgba_buffer_size(width, height, pixels * 4)?;
        let mut bytes = Vec::with_capacity(pixels * 4);
        for _ in 0..pixels {
            bytes.extend_from_slice(&rgba);
        }
        Ok(Self { width, height, rgba: bytes })
    }
}

pub fn validate_rgba_buffer_size(width: u32, height: u32, byte_len: usize) -> Result<()> {
    if width == 0 || height == 0 {
        bail!("rgba buffer dimensions must be non-zero");
    }
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow!("rgba buffer size overflow"))? as usize;
    if expected != byte_len {
        bail!("rgba buffer size mismatch: expected {expected}, got {byte_len}");
    }
    Ok(())
}

pub fn validate_displayd_artifact_format(format: &str) -> Result<DisplaydRawArtifactEncoding> {
    if format.trim().is_empty() {
        bail!("displayd capture artifact format must not be empty");
    }
    DisplaydRawArtifactEncoding::parse(format)
}

pub trait CaptureClient {
    fn capture(&self, target: CaptureTarget) -> Result<CapturedFrame>;
}

#[derive(Debug, Clone)]
pub struct FakeCaptureClient {
    pub fullscreen: CapturedFrame,
    pub active_window: CapturedFrame,
    pub fail_target: Option<CaptureTarget>,
}

impl Default for FakeCaptureClient {
    fn default() -> Self {
        Self {
            fullscreen: CapturedFrame::solid(2, 2, [0x22, 0x44, 0x88, 0xff]).expect("valid"),
            active_window: CapturedFrame::solid(1, 1, [0xaa, 0x11, 0x55, 0xff]).expect("valid"),
            fail_target: None,
        }
    }
}

impl FakeCaptureClient {
    pub fn with_failure(target: CaptureTarget) -> Self {
        Self { fail_target: Some(target), ..Self::default() }
    }
}

impl CaptureClient for FakeCaptureClient {
    fn capture(&self, target: CaptureTarget) -> Result<CapturedFrame> {
        if self.fail_target == Some(target) {
            return Err(anyhow!("fake capture backend failure for {target:?}"));
        }
        Ok(match target {
            CaptureTarget::Fullscreen => self.fullscreen.clone(),
            CaptureTarget::ActiveWindow => self.active_window.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CaptureTarget;

    #[test]
    fn fake_capture_fullscreen_returns_rgba_buffer() {
        let client = FakeCaptureClient::default();
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.rgba.len(), 16);
    }

    #[test]
    fn fake_capture_backend_failure_returns_error() {
        let client = FakeCaptureClient::with_failure(CaptureTarget::Fullscreen);
        assert!(client.capture(CaptureTarget::Fullscreen).is_err());
    }

    #[test]
    fn output_captured_requires_rgba8888_format() {
        let artifact = DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", "/tmp/a.rgba");
        assert!(artifact.is_ok());
    }

    #[test]
    fn output_captured_rejects_unknown_format() {
        let artifact = DisplaydCaptureArtifact::new("fullscreen", 2, 2, "PNG", "/tmp/a.rgba");
        assert!(artifact.is_err());
    }

    #[test]
    fn output_captured_rejects_empty_artifact_path() {
        let artifact = DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", "");
        assert!(artifact.is_err());
    }

    #[test]
    fn output_captured_rejects_zero_dimensions() {
        let artifact = DisplaydCaptureArtifact::new("fullscreen", 0, 2, "RGBA8888", "/tmp/a.rgba");
        assert!(artifact.is_err());
    }
}
