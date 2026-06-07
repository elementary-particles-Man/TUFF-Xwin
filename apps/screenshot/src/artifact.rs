use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    capture::{CaptureClient, CapturedFrame, DisplaydCaptureArtifact, validate_rgba_buffer_size},
    config::CaptureTarget,
    displayd_ipc::{DisplaydIpcCaptureClient, DisplaydIpcTransport},
};
use xwin_sec::SecurityPolicy;

pub trait CaptureArtifactReader {
    fn read_frame(&self, artifact: &DisplaydCaptureArtifact) -> Result<CapturedFrame>;
}

#[derive(Debug, Clone)]
pub struct ArtifactRoot {
    path: PathBuf,
}

impl ArtifactRoot {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            bail!("artifact root must not be empty");
        }
        if !path.is_absolute() {
            bail!("artifact root must be absolute");
        }
        Ok(Self { path })
    }

    pub fn as_path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone)]
pub struct FileCaptureArtifactReader {
    allowed_root: ArtifactRoot,
}

impl FileCaptureArtifactReader {
    pub fn new(allowed_root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self { allowed_root: ArtifactRoot::new(allowed_root)? })
    }

    fn resolve_artifact_path(&self, artifact_path: &Path) -> Result<PathBuf> {
        if artifact_path.as_os_str().is_empty() {
            bail!("artifact path must not be empty");
        }
        if artifact_path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        {
            bail!("artifact path must not contain traversal segments");
        }

        if artifact_path.is_absolute() {
            if !artifact_path.starts_with(self.allowed_root.as_path()) {
                bail!(
                    "artifact path must stay within allowed root {}",
                    self.allowed_root.as_path().display()
                );
            }
            Ok(artifact_path.to_path_buf())
        } else {
            Ok(self.allowed_root.as_path().join(artifact_path))
        }
    }
}

impl CaptureArtifactReader for FileCaptureArtifactReader {
    fn read_frame(&self, artifact: &DisplaydCaptureArtifact) -> Result<CapturedFrame> {
        if artifact.width == 0 || artifact.height == 0 {
            bail!("displayd artifact dimensions must be non-zero");
        }
        let path = self.resolve_artifact_path(&artifact.artifact_path)?;
        let rgba = fs::read(&path)
            .with_context(|| format!("failed to read displayd artifact {}", path.display()))?;
        validate_rgba_buffer_size(artifact.width, artifact.height, rgba.len())?;
        CapturedFrame::new(artifact.width, artifact.height, rgba)
    }
}

#[derive(Debug)]
pub struct DisplaydArtifactCaptureClient<T, R, P = xwin_sec::BrowserSecurityPolicy> {
    ipc: DisplaydIpcCaptureClient<T, P>,
    reader: R,
}

impl<T, R, P> DisplaydArtifactCaptureClient<T, R, P>
where
    T: DisplaydIpcTransport,
    R: CaptureArtifactReader,
    P: SecurityPolicy,
{
    pub fn new(ipc: DisplaydIpcCaptureClient<T, P>, reader: R) -> Self {
        Self { ipc, reader }
    }

    pub fn with_source_role(mut self, source_role: waybroker_common::ServiceRole) -> Self {
        self.ipc = self.ipc.with_source_role(source_role);
        self
    }
}

impl<T, R, P> CaptureClient for DisplaydArtifactCaptureClient<T, R, P>
where
    T: DisplaydIpcTransport,
    R: CaptureArtifactReader,
    P: SecurityPolicy,
{
    fn capture(&self, target: CaptureTarget) -> Result<CapturedFrame> {
        let artifact = self.ipc.request_capture(target)?;
        self.reader.read_frame(&artifact)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use waybroker_common::{DisplayEvent, IpcEnvelope, MessageKind, ServiceRole};
    use xwin_sec::BrowserSecurityPolicy;

    use crate::{
        config::{CaptureTarget, ScreenshotConfig, ScreenshotFormat},
        displayd_ipc::{FakeDisplaydTransport, screenshot_user_policy_context},
        ui::ScreenshotApp,
    };

    fn write_rgba(path: &Path, width: u32, height: u32) -> Result<()> {
        let pixels =
            width.checked_mul(height).and_then(|value| value.checked_mul(4)).expect("valid size")
                as usize;
        let mut file = fs::File::create(path)?;
        file.write_all(&vec![0x7f; pixels])?;
        Ok(())
    }

    fn success_response(path: impl Into<String>, width: u32, height: u32) -> IpcEnvelope {
        IpcEnvelope::new(
            ServiceRole::Displayd,
            ServiceRole::Sessiond,
            MessageKind::DisplayEvent(DisplayEvent::OutputCaptured {
                output: "fullscreen".into(),
                width,
                height,
                format: "RGBA8888".into(),
                artifact_path: path.into(),
            }),
        )
    }

    #[test]
    fn artifact_reader_reads_valid_rgba_under_allowed_root() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", &artifact_path).unwrap();
        let frame = reader.read_frame(&artifact).unwrap();
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.rgba.len(), 16);
    }

    #[test]
    fn artifact_reader_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact = DisplaydCaptureArtifact {
            output: "fullscreen".into(),
            width: 2,
            height: 2,
            format: "RGBA8888".into(),
            artifact_path: PathBuf::from("../evil.rgba"),
        };
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn artifact_reader_rejects_absolute_path_outside_allowed_root() {
        let dir = tempdir().unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let outside = PathBuf::from("/tmp/xwin-artifact-outside.rgba");
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", &outside).unwrap();
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn artifact_reader_rejects_size_mismatch() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        fs::write(&artifact_path, vec![0x55; 12]).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", &artifact_path).unwrap();
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn artifact_reader_rejects_zero_dimensions() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        fs::write(&artifact_path, vec![0x55; 4]).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact = DisplaydCaptureArtifact {
            output: "fullscreen".into(),
            width: 0,
            height: 2,
            format: "RGBA8888".into(),
            artifact_path,
        };
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn displayd_artifact_capture_client_converts_output_captured_to_frame() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = success_response(artifact_path.to_string_lossy().to_string(), 2, 2);
        let ipc = DisplaydIpcCaptureClient::new(
            FakeDisplaydTransport::with_response(response),
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let client = DisplaydArtifactCaptureClient::new(
            ipc,
            FileCaptureArtifactReader::new(dir.path()).unwrap(),
        );

        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.rgba.len(), 16);
    }

    #[test]
    fn displayd_artifact_capture_client_propagates_reader_error() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        fs::write(&artifact_path, vec![0x11; 12]).unwrap();
        let response = success_response(artifact_path.to_string_lossy().to_string(), 2, 2);
        let ipc = DisplaydIpcCaptureClient::new(
            FakeDisplaydTransport::with_response(response),
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let client = DisplaydArtifactCaptureClient::new(
            ipc,
            FileCaptureArtifactReader::new(dir.path()).unwrap(),
        );

        assert!(client.capture(CaptureTarget::Fullscreen).is_err());
    }

    #[test]
    fn displayd_artifact_capture_client_propagates_ipc_rejected() {
        let response = IpcEnvelope::new(
            ServiceRole::Displayd,
            ServiceRole::Sessiond,
            MessageKind::DisplayEvent(DisplayEvent::Rejected { reason: "no capture".into() }),
        );
        let dir = tempdir().unwrap();
        let client = DisplaydArtifactCaptureClient::new(
            DisplaydIpcCaptureClient::new(
                FakeDisplaydTransport::with_response(response),
                BrowserSecurityPolicy,
                screenshot_user_policy_context(),
            ),
            FileCaptureArtifactReader::new(dir.path()).unwrap(),
        );

        assert!(client.capture(CaptureTarget::Fullscreen).is_err());
    }

    #[test]
    fn displayd_artifact_capture_client_png_encode_writes_file() {
        let artifact_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let artifact_path = artifact_dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = success_response(artifact_path.to_string_lossy().to_string(), 2, 2);
        let ipc = DisplaydIpcCaptureClient::new(
            FakeDisplaydTransport::with_response(response),
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let client = DisplaydArtifactCaptureClient::new(
            ipc,
            FileCaptureArtifactReader::new(artifact_dir.path()).unwrap(),
        );
        let config = ScreenshotConfig {
            save_dir: save_dir.path().to_path_buf(),
            ..ScreenshotConfig::default()
        };
        let mut app = ScreenshotApp::new(
            config,
            client,
            crate::tray::FakeTrayController::default(),
            crate::hotkey::FakeHotkeyController::default(),
        )
        .unwrap();

        let saved = app.handle_command(crate::ui::ScreenshotUiCommand::Capture).unwrap().unwrap();
        assert_eq!(saved.extension().and_then(|s| s.to_str()), Some("png"));
        assert!(saved.exists());
    }

    #[test]
    fn displayd_artifact_capture_client_jpeg_encode_writes_file() {
        let artifact_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let artifact_path = artifact_dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = success_response(artifact_path.to_string_lossy().to_string(), 2, 2);
        let ipc = DisplaydIpcCaptureClient::new(
            FakeDisplaydTransport::with_response(response),
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let client = DisplaydArtifactCaptureClient::new(
            ipc,
            FileCaptureArtifactReader::new(artifact_dir.path()).unwrap(),
        );
        let config = ScreenshotConfig {
            save_dir: save_dir.path().to_path_buf(),
            format: ScreenshotFormat::Jpeg,
            ..ScreenshotConfig::default()
        };
        let mut app = ScreenshotApp::new(
            config,
            client,
            crate::tray::FakeTrayController::default(),
            crate::hotkey::FakeHotkeyController::default(),
        )
        .unwrap();

        let saved = app.handle_command(crate::ui::ScreenshotUiCommand::Capture).unwrap().unwrap();
        assert_eq!(saved.extension().and_then(|s| s.to_str()), Some("jpg"));
        assert!(saved.exists());
    }

    #[test]
    fn existing_fake_capture_tests_still_pass() {
        let client = crate::capture::FakeCaptureClient::default();
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(frame.rgba.len(), 16);
    }
}
