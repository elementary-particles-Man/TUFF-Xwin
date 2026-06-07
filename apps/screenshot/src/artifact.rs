use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    capture::{
        CaptureClient, CapturedFrame, DisplaydCaptureArtifact, validate_displayd_artifact_format,
        validate_rgba_buffer_size,
    },
    config::CaptureTarget,
    displayd_ipc::{DisplaydIpcCaptureClient, DisplaydIpcTransport},
};
use xwin_sec::SecurityPolicy;

pub const MAX_SCREENSHOT_ARTIFACT_BYTES: usize = 256 * 1024 * 1024;

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
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        {
            bail!("artifact root must not contain traversal segments");
        }
        if path.starts_with(Path::new("/run/user")) {
            bail!("artifact root under /run/user is not permitted");
        }
        let metadata = fs::metadata(&path)
            .with_context(|| format!("failed to inspect artifact root {}", path.display()))?;
        if !metadata.is_dir() {
            bail!("artifact root must be an existing directory");
        }
        let canonical_path = fs::canonicalize(&path)
            .with_context(|| format!("failed to canonicalize artifact root {}", path.display()))?;
        if canonical_path.starts_with(Path::new("/run/user")) {
            bail!("artifact root under /run/user is not permitted");
        }
        Ok(Self { path: canonical_path })
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

    fn validate_relative_artifact_path(&self, artifact_path: &Path) -> Result<()> {
        if artifact_path.as_os_str().is_empty() {
            bail!("artifact path must not be empty");
        }
        if artifact_path.is_absolute() {
            bail!("absolute artifact paths are not permitted in Phase2-A");
        }
        for component in artifact_path.components() {
            if !matches!(component, Component::Normal(_)) {
                bail!("artifact path must be a simple relative path without traversal segments");
            }
        }
        Ok(())
    }

    fn resolve_artifact_path(&self, artifact_path: &Path) -> Result<PathBuf> {
        self.validate_relative_artifact_path(artifact_path)?;
        Ok(self.allowed_root.as_path().join(artifact_path))
    }

    fn reject_symlink_components(&self, artifact_path: &Path) -> Result<()> {
        let mut current = self.allowed_root.as_path().to_path_buf();
        for component in artifact_path.components() {
            current.push(component.as_os_str());
            if let Ok(metadata) = fs::symlink_metadata(&current) {
                if metadata.file_type().is_symlink() {
                    bail!("artifact path must not traverse symlinks");
                }
            }
        }
        Ok(())
    }

    fn expected_rgba_byte_len(&self, width: u32, height: u32) -> Result<usize> {
        let expected = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| anyhow!("rgba artifact size overflow"))?;
        if expected > MAX_SCREENSHOT_ARTIFACT_BYTES as u64 {
            bail!(
                "rgba artifact size {expected} exceeds limit of {} bytes",
                MAX_SCREENSHOT_ARTIFACT_BYTES
            );
        }
        Ok(expected as usize)
    }
}

impl CaptureArtifactReader for FileCaptureArtifactReader {
    fn read_frame(&self, artifact: &DisplaydCaptureArtifact) -> Result<CapturedFrame> {
        validate_displayd_artifact_format(&artifact.format)?;
        if artifact.width == 0 || artifact.height == 0 {
            bail!("displayd artifact dimensions must be non-zero");
        }
        let expected_len = self.expected_rgba_byte_len(artifact.width, artifact.height)?;
        let path = self.resolve_artifact_path(&artifact.artifact_path)?;
        self.reject_symlink_components(&artifact.artifact_path)?;
        let metadata = fs::metadata(&path)
            .with_context(|| format!("failed to inspect displayd artifact {}", path.display()))?;
        if !metadata.is_file() {
            bail!("displayd artifact path must point to a file");
        }
        let file_len = metadata.len();
        if file_len != expected_len as u64 {
            bail!("displayd artifact size mismatch: expected {expected_len}, got {file_len}");
        }
        let rgba = fs::read(&path)
            .with_context(|| format!("failed to read displayd artifact {}", path.display()))?;
        if rgba.len() != expected_len {
            bail!(
                "displayd artifact size mismatch after read: expected {expected_len}, got {}",
                rgba.len()
            );
        }
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
    use std::os::unix::net::UnixListener;
    use std::thread;
    use tempfile::tempdir;
    use waybroker_common::{DisplayEvent, IpcEnvelope, MessageKind, ServiceRole};
    use xwin_sec::BrowserSecurityPolicy;

    use crate::{
        config::{CaptureTarget, ScreenshotConfig, ScreenshotFormat},
        displayd_ipc::{
            DisplaydUnixSocketTransport, FakeDisplaydTransport, screenshot_user_policy_context,
        },
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

    fn spawn_loopback_displayd_server(
        socket_path: PathBuf,
        response: IpcEnvelope,
    ) -> (thread::JoinHandle<Result<()>>, std::sync::mpsc::Receiver<()>) {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            if let Some(parent) = socket_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let listener = UnixListener::bind(&socket_path)?;
            ready_tx.send(()).ok();
            let (mut stream, _) = listener.accept()?;
            let mut reader = std::io::BufReader::new(stream.try_clone()?);
            let request: IpcEnvelope = waybroker_common::read_json_line(&mut reader)?;
            assert!(matches!(
                request.kind,
                MessageKind::DisplayCommand(waybroker_common::DisplayCommand::CaptureOutput { .. })
            ));
            waybroker_common::send_json_line(&mut stream, &response)?;
            Ok(())
        });
        (handle, ready_rx)
    }

    #[test]
    fn artifact_reader_reads_valid_rgba_under_allowed_root() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", "frame.rgba").unwrap();
        let frame = reader.read_frame(&artifact).unwrap();
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.rgba.len(), 16);
    }

    #[test]
    fn artifact_reader_reads_exact_expected_rgba_len() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 1, 1).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 1, 1, "RGBA8888", "frame.rgba").unwrap();
        let frame = reader.read_frame(&artifact).unwrap();
        assert_eq!(frame.width, 1);
        assert_eq!(frame.height, 1);
        assert_eq!(frame.rgba.len(), 4);
    }

    #[test]
    fn artifact_root_rejects_relative_root() {
        assert!(ArtifactRoot::new("relative/root").is_err());
    }

    #[test]
    fn artifact_root_rejects_missing_root() {
        let missing = tempdir().unwrap().path().join("missing-root");
        assert!(ArtifactRoot::new(missing).is_err());
    }

    #[test]
    fn artifact_root_rejects_file_root() {
        let dir = tempdir().unwrap();
        let file_root = dir.path().join("artifact-root.txt");
        fs::write(&file_root, b"not a directory").unwrap();
        assert!(ArtifactRoot::new(file_root).is_err());
    }

    #[test]
    fn artifact_root_rejects_run_user_root() {
        assert!(ArtifactRoot::new("/run/user/1000/xwin-artifacts").is_err());
    }

    #[test]
    fn artifact_root_accepts_existing_tempdir_root() {
        let dir = tempdir().unwrap();
        let root = ArtifactRoot::new(dir.path()).unwrap();
        assert_eq!(root.as_path(), &dir.path().canonicalize().unwrap());
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
    fn artifact_path_rejects_parent_dir_component() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact = DisplaydCaptureArtifact {
            output: "fullscreen".into(),
            width: 2,
            height: 2,
            format: "RGBA8888".into(),
            artifact_path: PathBuf::from("../frame.rgba"),
        };
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn artifact_path_rejects_cur_dir_component() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact = DisplaydCaptureArtifact {
            output: "fullscreen".into(),
            width: 2,
            height: 2,
            format: "RGBA8888".into(),
            artifact_path: PathBuf::from("./frame.rgba"),
        };
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn artifact_path_rejects_empty_artifact_path() {
        let dir = tempdir().unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact = DisplaydCaptureArtifact {
            output: "fullscreen".into(),
            width: 2,
            height: 2,
            format: "RGBA8888".into(),
            artifact_path: PathBuf::new(),
        };
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn artifact_reader_rejects_allowed_root_escape() {
        let dir = tempdir().unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", artifact_path).unwrap();
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
    fn artifact_reader_rejects_metadata_size_mismatch_before_read() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        fs::write(&artifact_path, vec![0x55; 12]).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", "frame.rgba").unwrap();
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn artifact_reader_rejects_byte_length_mismatch() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        fs::write(&artifact_path, vec![0x55; 12]).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", "frame.rgba").unwrap();
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn artifact_reader_rejects_oversized_expected_len() {
        let dir = tempdir().unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact = DisplaydCaptureArtifact {
            output: "fullscreen".into(),
            width: 8193,
            height: 8192,
            format: "RGBA8888".into(),
            artifact_path: PathBuf::from("frame.rgba"),
        };
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

    #[cfg(unix)]
    #[test]
    fn artifact_reader_rejects_symlink_artifact() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let symlink_path = dir.path().join("link.rgba");
        symlink(&artifact_path, &symlink_path).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", "link.rgba").unwrap();
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn artifact_reader_rejects_symlink_parent_directory() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let real_dir = root.path().join("real");
        let symlink_dir = root.path().join("link-dir");
        fs::create_dir_all(&real_dir).unwrap();
        symlink(&real_dir, &symlink_dir).unwrap();
        let artifact_path = real_dir.join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let reader = FileCaptureArtifactReader::new(root.path()).unwrap();
        let artifact = DisplaydCaptureArtifact::new(
            "fullscreen",
            2,
            2,
            "RGBA8888",
            PathBuf::from("link-dir/frame.rgba"),
        )
        .unwrap();
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn artifact_reader_rejects_unknown_format() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact = DisplaydCaptureArtifact {
            output: "fullscreen".into(),
            width: 2,
            height: 2,
            format: "PNG".into(),
            artifact_path,
        };
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn displayd_artifact_capture_client_still_converts_valid_artifact() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = success_response("frame.rgba", 2, 2);
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
        let response = success_response("frame.rgba", 2, 2);
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
    fn displayd_ipc_rejected_event_maps_to_error() {
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
    fn displayd_artifact_capture_client_rejects_unknown_format() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = IpcEnvelope::new(
            ServiceRole::Displayd,
            ServiceRole::Sessiond,
            MessageKind::DisplayEvent(DisplayEvent::OutputCaptured {
                output: "fullscreen".into(),
                width: 2,
                height: 2,
                format: "PNG".into(),
                artifact_path: "frame.rgba".into(),
            }),
        );
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
    fn displayd_artifact_capture_client_png_encode_writes_file() {
        let artifact_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let artifact_path = artifact_dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = success_response("frame.rgba", 2, 2);
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
        let response = success_response("frame.rgba", 2, 2);
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
    fn artifact_pipeline_reads_loopback_output_captured_rgba8888() {
        let artifact_dir = tempdir().unwrap();
        let socket_dir = tempdir().unwrap();
        let artifact_path = artifact_dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = success_response("frame.rgba", 2, 2);
        let socket_path = socket_dir.path().join("displayd.sock");
        let (server, ready) = spawn_loopback_displayd_server(socket_path.clone(), response);
        ready.recv().unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let client = DisplaydArtifactCaptureClient::new(
            ipc,
            FileCaptureArtifactReader::new(artifact_dir.path()).unwrap(),
        );

        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.rgba.len(), 16);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn artifact_pipeline_png_encode_writes_file_from_loopback_capture() {
        let artifact_dir = tempdir().unwrap();
        let socket_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let artifact_path = artifact_dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = success_response("frame.rgba", 2, 2);
        let socket_path = socket_dir.path().join("displayd.sock");
        let (server, ready) = spawn_loopback_displayd_server(socket_path.clone(), response);
        ready.recv().unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
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
        server.join().unwrap().unwrap();
    }

    #[test]
    fn artifact_pipeline_jpeg_encode_writes_file_from_loopback_capture() {
        let artifact_dir = tempdir().unwrap();
        let socket_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let artifact_path = artifact_dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = success_response("frame.rgba", 2, 2);
        let socket_path = socket_dir.path().join("displayd.sock");
        let (server, ready) = spawn_loopback_displayd_server(socket_path.clone(), response);
        ready.recv().unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
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
        server.join().unwrap().unwrap();
    }

    #[test]
    fn existing_fake_capture_tests_still_pass() {
        let client = crate::capture::FakeCaptureClient::default();
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(frame.rgba.len(), 16);
    }

    #[test]
    fn existing_artifact_ingest_tests_still_pass() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 1, 1).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 1, 1, "RGBA8888", "frame.rgba").unwrap();
        let frame = reader.read_frame(&artifact).unwrap();
        assert_eq!(frame.rgba.len(), 4);
    }
}
