use std::{
    fs,
    io::BufReader,
    os::unix::fs::FileTypeExt,
    os::unix::net::{UnixListener, UnixStream},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::{Context, Result, anyhow, bail};
use waybroker_common::{
    DisplayCommand, DisplayEvent, IpcEnvelope, MessageKind, ServiceRole, read_json_line,
    send_json_line,
};

use crate::{artifact::ArtifactRoot, capture::DISPLAYD_SCREENSHOT_FORMAT_RGBA8888};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessDisplaydResponse {
    OutputCaptured {
        output: String,
        width: u32,
        height: u32,
        format: String,
        artifact_path: PathBuf,
        artifact_byte_len: Option<usize>,
    },
    Rejected {
        reason: String,
    },
}

impl HarnessDisplaydResponse {
    pub fn output_captured(
        output: impl Into<String>,
        artifact_path: impl Into<PathBuf>,
        width: u32,
        height: u32,
    ) -> Self {
        Self::OutputCaptured {
            output: output.into(),
            width,
            height,
            format: DISPLAYD_SCREENSHOT_FORMAT_RGBA8888.into(),
            artifact_path: artifact_path.into(),
            artifact_byte_len: None,
        }
    }

    pub fn output_captured_with_format(
        output: impl Into<String>,
        artifact_path: impl Into<PathBuf>,
        width: u32,
        height: u32,
        format: impl Into<String>,
    ) -> Self {
        Self::OutputCaptured {
            output: output.into(),
            width,
            height,
            format: format.into(),
            artifact_path: artifact_path.into(),
            artifact_byte_len: None,
        }
    }

    pub fn output_captured_with_byte_len(
        output: impl Into<String>,
        artifact_path: impl Into<PathBuf>,
        width: u32,
        height: u32,
        artifact_byte_len: usize,
    ) -> Self {
        Self::OutputCaptured {
            output: output.into(),
            width,
            height,
            format: DISPLAYD_SCREENSHOT_FORMAT_RGBA8888.into(),
            artifact_path: artifact_path.into(),
            artifact_byte_len: Some(artifact_byte_len),
        }
    }

    pub fn rejected(reason: impl Into<String>) -> Self {
        Self::Rejected { reason: reason.into() }
    }

    fn to_envelope(&self) -> IpcEnvelope {
        match self {
            Self::OutputCaptured { output, width, height, format, artifact_path, .. } => {
                IpcEnvelope::new(
                    ServiceRole::Displayd,
                    ServiceRole::Sessiond,
                    MessageKind::DisplayEvent(DisplayEvent::OutputCaptured {
                        output: output.clone(),
                        width: *width,
                        height: *height,
                        format: format.clone(),
                        artifact_path: artifact_path.to_string_lossy().into_owned(),
                    }),
                )
            }
            Self::Rejected { reason } => IpcEnvelope::new(
                ServiceRole::Displayd,
                ServiceRole::Sessiond,
                MessageKind::DisplayEvent(DisplayEvent::Rejected { reason: reason.clone() }),
            ),
        }
    }

    fn artifact_write_path(&self, artifact_root: &Path) -> PathBuf {
        match self {
            Self::OutputCaptured { artifact_path, .. }
                if is_simple_relative_path(artifact_path) =>
            {
                artifact_root.join(artifact_path)
            }
            Self::OutputCaptured { .. } => artifact_root.join("frame.rgba"),
            Self::Rejected { .. } => artifact_root.join("frame.rgba"),
        }
    }

    fn artifact_byte_len(&self) -> Result<Option<usize>> {
        match self {
            Self::OutputCaptured { width, height, artifact_byte_len, .. } => {
                let expected = expected_rgba_byte_len(*width, *height)?;
                Ok(Some(artifact_byte_len.unwrap_or(expected)))
            }
            Self::Rejected { .. } => Ok(None),
        }
    }
}

#[derive(Debug)]
pub struct HarnessDisplayd {
    socket_path: PathBuf,
    accepted_connections: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<Result<()>>>,
}

impl HarnessDisplayd {
    pub fn spawn(
        socket_path: impl Into<PathBuf>,
        artifact_root: impl Into<PathBuf>,
        response: HarnessDisplaydResponse,
    ) -> Result<Self> {
        let socket_path = socket_path.into();
        validate_socket_path(&socket_path)?;
        let artifact_root = ArtifactRoot::new(artifact_root)?.as_path().to_path_buf();
        let accepted_connections = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let socket_path_for_thread = socket_path.clone();
        let artifact_root_for_thread = artifact_root.clone();
        let accepted_connections_for_thread = Arc::clone(&accepted_connections);
        let shutdown_for_thread = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            if let Some(parent) = socket_path_for_thread.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to prepare harness socket directory {}", parent.display())
                })?;
            }
            if socket_path_for_thread.exists() {
                let metadata = fs::metadata(&socket_path_for_thread).with_context(|| {
                    format!("failed to inspect harness socket {}", socket_path_for_thread.display())
                })?;
                if metadata.file_type().is_socket() {
                    fs::remove_file(&socket_path_for_thread).with_context(|| {
                        format!(
                            "failed to remove stale harness socket {}",
                            socket_path_for_thread.display()
                        )
                    })?;
                } else if metadata.is_dir() {
                    bail!(
                        "harness displayd socket path must not point to a directory: {}",
                        socket_path_for_thread.display()
                    );
                }
            }

            let listener = UnixListener::bind(&socket_path_for_thread).with_context(|| {
                format!(
                    "failed to bind harness displayd socket {}",
                    socket_path_for_thread.display()
                )
            })?;
            ready_tx.send(()).context("failed to signal harness displayd readiness")?;

            let (mut stream, _) = listener.accept().with_context(|| {
                format!(
                    "failed to accept harness displayd request on {}",
                    socket_path_for_thread.display()
                )
            })?;
            accepted_connections_for_thread.fetch_add(1, Ordering::SeqCst);
            if shutdown_for_thread.load(Ordering::SeqCst) {
                return Ok(());
            }

            let mut reader = BufReader::new(stream.try_clone()?);
            let request: IpcEnvelope = read_json_line(&mut reader)?;
            match request.kind {
                MessageKind::DisplayCommand(DisplayCommand::CaptureOutput { output }) => {
                    if output.trim().is_empty() {
                        bail!("capture output name must not be empty");
                    }
                }
                other => bail!("unexpected harness displayd request kind: {other:?}"),
            }

            if let HarnessDisplaydResponse::OutputCaptured {
                width, height, artifact_path, ..
            } = &response
            {
                if !is_simple_relative_path(artifact_path) {
                    bail!("harness displayd artifact path must be a simple relative path");
                }
                let write_path = response.artifact_write_path(&artifact_root_for_thread);
                let byte_len = response.artifact_byte_len()?.unwrap();
                write_rgba_artifact(&write_path, *width, *height, byte_len)?;
            }

            let envelope = response.to_envelope();
            send_json_line(&mut stream, &envelope)?;
            Ok(())
        });

        ready_rx.recv().context("harness displayd failed to become ready")?;

        Ok(Self { socket_path, accepted_connections, shutdown, handle: Some(handle) })
    }

    pub fn accepted_connections(&self) -> usize {
        self.accepted_connections.load(Ordering::SeqCst)
    }

    pub fn shutdown(self) -> Result<()> {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket_path);
        self.join()
    }

    pub fn join(mut self) -> Result<()> {
        let handle =
            self.handle.take().ok_or_else(|| anyhow!("harness displayd already joined"))?;
        match handle.join() {
            Ok(result) => result.context("harness displayd thread returned error"),
            Err(_) => Err(anyhow!("harness displayd thread panicked")),
        }
    }
}

fn is_simple_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path.components().all(|component| matches!(component, Component::Normal(_)))
}

pub fn validate_socket_path(socket_path: &Path) -> Result<()> {
    if socket_path.as_os_str().is_empty() {
        bail!("harness displayd socket path must not be empty");
    }
    if !socket_path.is_absolute() {
        bail!("harness displayd socket path must be absolute");
    }
    if socket_path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        bail!("harness displayd socket path must not contain traversal segments");
    }
    if socket_path.to_string_lossy().contains("/run/user/") {
        bail!("harness displayd socket path must not point into /run/user");
    }
    if socket_path.exists() && socket_path.is_dir() {
        bail!("harness displayd socket path must be a socket file, not a directory");
    }
    Ok(())
}

fn expected_rgba_byte_len(width: u32, height: u32) -> Result<usize> {
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow!("rgba artifact size overflow"))?;
    Ok(expected as usize)
}

fn write_rgba_artifact(path: &Path, width: u32, height: u32, byte_len: usize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to prepare artifact directory {}", parent.display())
        })?;
    }
    let expected = expected_rgba_byte_len(width, height)?;
    if byte_len != expected {
        bail!("artifact byte length mismatch: expected {expected}, got {byte_len}");
    }
    let mut bytes = Vec::with_capacity(byte_len);
    bytes.resize(byte_len, 0x7f);
    fs::write(path, &bytes)
        .with_context(|| format!("failed to write harness artifact {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    use crate::{
        artifact::DisplaydArtifactCaptureClient,
        capture::CaptureClient,
        cli::CliOptions,
        config::CaptureTarget,
        displayd_ipc::{
            DisplaydIpcCaptureClient, DisplaydUnixSocketTransport, screenshot_user_policy_context,
        },
    };
    use xwin_sec::{
        BrowserSecurityPolicy, DecisionReason, PolicyContext, SecurityDecision, SecurityPolicy,
        XwinCapability, browser_hostile_client,
    };

    #[test]
    fn harness_displayd_binds_only_tempdir_socket() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            artifact_dir.path(),
            HarnessDisplaydResponse::output_captured("fullscreen", "frame.rgba", 2, 2),
        )
        .unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let reader = crate::artifact::FileCaptureArtifactReader::new(artifact_dir.path()).unwrap();
        let client = DisplaydArtifactCaptureClient::new(ipc, reader);
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(server.accepted_connections(), 1);
        server.join().unwrap();
    }

    #[test]
    fn harness_displayd_rejects_run_user_socket_path() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let err = HarnessDisplayd::spawn(
            "/run/user/1000/displayd.sock",
            artifact_dir.path(),
            HarnessDisplaydResponse::rejected("denied"),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("/run/user"));
        assert!(socket_dir.path().exists());
    }

    #[test]
    fn harness_displayd_writes_rgba8888_artifact_under_root() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            artifact_dir.path(),
            HarnessDisplaydResponse::output_captured("fullscreen", "nested/frame.rgba", 2, 2),
        )
        .unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let reader = crate::artifact::FileCaptureArtifactReader::new(artifact_dir.path()).unwrap();
        let client = DisplaydArtifactCaptureClient::new(ipc, reader);
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(frame.rgba.len(), 16);
        assert!(artifact_dir.path().join("nested/frame.rgba").exists());
        server.join().unwrap();
    }

    #[test]
    fn harness_displayd_returns_output_captured_contract() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            artifact_dir.path(),
            HarnessDisplaydResponse::output_captured("active-window", "frame.rgba", 3, 3),
        )
        .unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let artifact = ipc.request_capture(CaptureTarget::ActiveWindow).unwrap();
        assert_eq!(artifact.output, "active-window");
        assert_eq!(artifact.format, DISPLAYD_SCREENSHOT_FORMAT_RGBA8888);
        assert_eq!(artifact.artifact_path, PathBuf::from("frame.rgba"));
        server.join().unwrap();
    }

    #[test]
    fn harness_rejected_event_maps_to_error() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            artifact_dir.path(),
            HarnessDisplaydResponse::rejected("policy denied"),
        )
        .unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let err = ipc.request_capture(CaptureTarget::Fullscreen).unwrap_err();
        assert!(format!("{err:#}").contains("displayd rejected capture request"));
        server.join().unwrap();
    }

    #[test]
    fn harness_unknown_format_is_rejected() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            artifact_dir.path(),
            HarnessDisplaydResponse::output_captured_with_format(
                "fullscreen",
                "frame.rgba",
                2,
                2,
                "PNG",
            ),
        )
        .unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let reader = crate::artifact::FileCaptureArtifactReader::new(artifact_dir.path()).unwrap();
        let client = DisplaydArtifactCaptureClient::new(ipc, reader);
        let err = client.capture(CaptureTarget::Fullscreen).unwrap_err();
        assert!(format!("{err:#}").contains("format"));
        server.join().unwrap();
    }

    #[test]
    fn harness_empty_artifact_path_is_rejected() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            artifact_dir.path(),
            HarnessDisplaydResponse::output_captured("fullscreen", PathBuf::new(), 2, 2),
        )
        .unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let reader = crate::artifact::FileCaptureArtifactReader::new(artifact_dir.path()).unwrap();
        let client = DisplaydArtifactCaptureClient::new(ipc, reader);
        assert!(client.capture(CaptureTarget::Fullscreen).is_err());
        let err = server.join().unwrap_err();
        assert!(format!("{err:#}").contains("artifact path must be a simple relative path"));
    }

    #[test]
    fn harness_absolute_artifact_path_is_rejected() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            artifact_dir.path(),
            HarnessDisplaydResponse::output_captured(
                "fullscreen",
                "/tmp/harness-escape.rgba",
                2,
                2,
            ),
        )
        .unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let reader = crate::artifact::FileCaptureArtifactReader::new(artifact_dir.path()).unwrap();
        let client = DisplaydArtifactCaptureClient::new(ipc, reader);
        assert!(client.capture(CaptureTarget::Fullscreen).is_err());
        let err = server.join().unwrap_err();
        assert!(format!("{err:#}").contains("artifact path must be a simple relative path"));
    }

    #[test]
    fn harness_path_traversal_artifact_path_is_rejected() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            artifact_dir.path(),
            HarnessDisplaydResponse::output_captured("fullscreen", "../escape.rgba", 2, 2),
        )
        .unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let reader = crate::artifact::FileCaptureArtifactReader::new(artifact_dir.path()).unwrap();
        let client = DisplaydArtifactCaptureClient::new(ipc, reader);
        assert!(client.capture(CaptureTarget::Fullscreen).is_err());
        let err = server.join().unwrap_err();
        assert!(format!("{err:#}").contains("artifact path must be a simple relative path"));
    }

    #[test]
    fn harness_byte_length_mismatch_is_rejected() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            artifact_dir.path(),
            HarnessDisplaydResponse::output_captured_with_byte_len(
                "fullscreen",
                "frame.rgba",
                2,
                2,
                12,
            ),
        )
        .unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let reader = crate::artifact::FileCaptureArtifactReader::new(artifact_dir.path()).unwrap();
        let client = DisplaydArtifactCaptureClient::new(ipc, reader);
        assert!(client.capture(CaptureTarget::Fullscreen).is_err());
        let err = server.join().unwrap_err();
        assert!(format!("{err:#}").contains("artifact byte length mismatch"));
    }

    #[test]
    fn policy_deny_prevents_harness_socket_connection() {
        struct DenyPolicy;

        impl SecurityPolicy for DenyPolicy {
            fn decide(
                &self,
                _context: &PolicyContext,
                _capability: XwinCapability,
            ) -> SecurityDecision {
                SecurityDecision::Deny { reason: DecisionReason::GlobalInputDenied }
            }
        }

        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            artifact_dir.path(),
            HarnessDisplaydResponse::output_captured("fullscreen", "frame.rgba", 2, 2),
        )
        .unwrap();

        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            DenyPolicy,
            PolicyContext::new(browser_hostile_client("renderer-1", "org.example.browser")),
        );
        let reader = crate::artifact::FileCaptureArtifactReader::new(artifact_dir.path()).unwrap();
        let client = DisplaydArtifactCaptureClient::new(ipc, reader);
        assert!(client.capture(CaptureTarget::Fullscreen).is_err());
        assert_eq!(server.accepted_connections(), 0);
        server.shutdown().unwrap();
    }

    #[test]
    fn existing_artifact_root_hardening_tests_still_pass() {
        let dir = tempdir().unwrap();
        let root = ArtifactRoot::new(dir.path()).unwrap();
        assert_eq!(root.as_path(), dir.path().canonicalize().unwrap());
    }

    #[test]
    fn existing_isolated_transport_tests_still_pass() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("displayd.sock");
        let server = HarnessDisplayd::spawn(
            socket_path.clone(),
            dir.path(),
            HarnessDisplaydResponse::output_captured("fullscreen", "frame.rgba", 2, 2),
        )
        .unwrap();
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let reader = crate::artifact::FileCaptureArtifactReader::new(dir.path()).unwrap();
        let client = DisplaydArtifactCaptureClient::new(ipc, reader);
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(frame.rgba.len(), 16);
        server.join().unwrap();
    }

    #[test]
    fn existing_cli_backend_selection_tests_still_pass() {
        let dir = tempdir().unwrap();
        let opts = CliOptions { save_dir: dir.path().to_path_buf(), ..CliOptions::default() };
        let saved = opts.run().unwrap();
        assert!(saved.exists());
    }
}
