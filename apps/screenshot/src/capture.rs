use anyhow::{Result, anyhow, bail};

use crate::config::CaptureTarget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
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
}
