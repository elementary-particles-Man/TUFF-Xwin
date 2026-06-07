use anyhow::{Result, anyhow, bail};
use std::path::PathBuf;

use crate::{
    artifact::{DisplaydArtifactCaptureClient, FileCaptureArtifactReader},
    capture::{CaptureClient, FakeCaptureClient},
    config::{CaptureTarget, JpegOptions, PngOptions, ScreenshotConfig, ScreenshotFormat},
    displayd_ipc::{
        DisplaydIpcCaptureClient, DisplaydUnixSocketTransport, screenshot_user_policy_context,
    },
    hotkey::FakeHotkeyController,
    tray::FakeTrayController,
    ui::{ScreenshotApp, ScreenshotUiCommand},
};
use xwin_sec::{BrowserSecurityPolicy, PolicyContext, SecurityPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackendKind {
    Fake,
    IsolatedDisplayd,
}

impl Default for CaptureBackendKind {
    fn default() -> Self {
        Self::Fake
    }
}

impl CaptureBackendKind {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "fake" => Ok(Self::Fake),
            "isolated-displayd" => Ok(Self::IsolatedDisplayd),
            other => bail!("unknown screenshot backend: {other}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOptions {
    pub backend: CaptureBackendKind,
    pub displayd_socket: Option<PathBuf>,
    pub artifact_root: Option<PathBuf>,
    pub target: CaptureTarget,
    pub format: ScreenshotFormat,
    pub save_dir: PathBuf,
    pub png_compression: u8,
    pub jpeg_quality: u8,
    pub help: bool,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            backend: CaptureBackendKind::Fake,
            displayd_socket: None,
            artifact_root: None,
            target: CaptureTarget::default(),
            format: ScreenshotFormat::default(),
            save_dir: PathBuf::from("screenshots"),
            png_compression: PngOptions::default().compression_level,
            jpeg_quality: JpegOptions::default().quality,
            help: false,
        }
    }
}

#[derive(Debug)]
pub enum SelectedCaptureBackend<P = BrowserSecurityPolicy> {
    Fake(FakeCaptureClient),
    IsolatedDisplayd(
        DisplaydArtifactCaptureClient<DisplaydUnixSocketTransport, FileCaptureArtifactReader, P>,
    ),
}

impl<P> CaptureClient for SelectedCaptureBackend<P>
where
    P: SecurityPolicy,
{
    fn capture(&self, target: CaptureTarget) -> Result<crate::capture::CapturedFrame> {
        match self {
            Self::Fake(client) => client.capture(target),
            Self::IsolatedDisplayd(client) => client.capture(target),
        }
    }
}

impl CliOptions {
    pub fn parse() -> Result<Self> {
        Self::parse_from(std::env::args())
    }

    pub fn parse_from<I, S>(args: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut options = Self::default();
        let mut iter = args.into_iter();
        let _program = iter.next();
        while let Some(arg) = iter.next() {
            let arg = arg.as_ref().to_owned();
            match arg.as_str() {
                "--help" | "-h" => {
                    options.help = true;
                }
                "--backend" => {
                    let value = next_value(&mut iter, "--backend")?;
                    options.backend = CaptureBackendKind::parse(&value)?;
                }
                "--displayd-socket" => {
                    let value = next_value(&mut iter, "--displayd-socket")?;
                    options.displayd_socket = Some(PathBuf::from(value));
                }
                "--artifact-root" => {
                    let value = next_value(&mut iter, "--artifact-root")?;
                    options.artifact_root = Some(PathBuf::from(value));
                }
                "--target" => {
                    let value = next_value(&mut iter, "--target")?;
                    options.target = parse_target(&value)?;
                }
                "--format" => {
                    let value = next_value(&mut iter, "--format")?;
                    options.format = parse_format(&value)?;
                }
                "--save-dir" => {
                    let value = next_value(&mut iter, "--save-dir")?;
                    options.save_dir = PathBuf::from(value);
                }
                "--png-compression" => {
                    let value = next_value(&mut iter, "--png-compression")?;
                    options.png_compression =
                        parse_u8_in_range(&value, 0..=9, "--png-compression")?;
                }
                "--jpeg-quality" => {
                    let value = next_value(&mut iter, "--jpeg-quality")?;
                    options.jpeg_quality = parse_u8_in_range(&value, 1..=100, "--jpeg-quality")?;
                }
                other if other.starts_with('-') => {
                    bail!("unknown option: {other}");
                }
                other => {
                    bail!("unexpected positional argument: {other}");
                }
            }
        }
        options.validate()?;
        Ok(options)
    }

    pub fn usage() -> &'static str {
        "Usage: xwin-screenshot [--backend fake|isolated-displayd] [--displayd-socket PATH] [--artifact-root PATH] [--target fullscreen|active-window] [--format png|jpeg] [--save-dir PATH] [--png-compression 0..9] [--jpeg-quality 1..100] [--help]"
    }

    pub fn validate(&self) -> Result<()> {
        if self.help {
            return Ok(());
        }

        match self.backend {
            CaptureBackendKind::Fake => {
                if self.displayd_socket.is_some() {
                    bail!("fake backend must not take a displayd socket");
                }
                if self.artifact_root.is_some() {
                    bail!("fake backend must not take an artifact root");
                }
            }
            CaptureBackendKind::IsolatedDisplayd => {
                let socket = self.displayd_socket.as_ref().ok_or_else(|| {
                    anyhow!("isolated-displayd backend requires --displayd-socket")
                })?;
                DisplaydUnixSocketTransport::new(socket.clone())?;
                let artifact_root = self
                    .artifact_root
                    .as_ref()
                    .ok_or_else(|| anyhow!("isolated-displayd backend requires --artifact-root"))?;
                FileCaptureArtifactReader::new(artifact_root.clone())?;
            }
        }

        self.screenshot_config()?;
        Ok(())
    }

    pub fn screenshot_config(&self) -> Result<ScreenshotConfig> {
        let config = ScreenshotConfig {
            hotkey: ScreenshotConfig::default().hotkey,
            save_dir: self.save_dir.clone(),
            capture_target: self.target,
            format: self.format,
            png: PngOptions { compression_level: self.png_compression },
            jpeg: JpegOptions { quality: self.jpeg_quality },
            filename_template: ScreenshotConfig::default().filename_template,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn build_capture_backend<P>(
        &self,
        policy: P,
        policy_context: PolicyContext,
    ) -> Result<SelectedCaptureBackend<P>>
    where
        P: SecurityPolicy,
    {
        self.validate()?;
        match self.backend {
            CaptureBackendKind::Fake => {
                Ok(SelectedCaptureBackend::Fake(FakeCaptureClient::default()))
            }
            CaptureBackendKind::IsolatedDisplayd => {
                let socket_path = self.displayd_socket.clone().expect("validated");
                let artifact_root = self.artifact_root.clone().expect("validated");
                let transport = DisplaydUnixSocketTransport::new(socket_path)?;
                let reader = FileCaptureArtifactReader::new(artifact_root)?;
                let ipc = DisplaydIpcCaptureClient::new(transport, policy, policy_context);
                Ok(SelectedCaptureBackend::IsolatedDisplayd(DisplaydArtifactCaptureClient::new(
                    ipc, reader,
                )))
            }
        }
    }

    pub fn run(self) -> Result<PathBuf> {
        self.run_with_policy(BrowserSecurityPolicy, screenshot_user_policy_context())
    }

    pub fn run_with_policy<P>(self, policy: P, policy_context: PolicyContext) -> Result<PathBuf>
    where
        P: SecurityPolicy,
    {
        let config = self.screenshot_config()?;
        let backend = self.build_capture_backend(policy, policy_context)?;
        let tray = FakeTrayController::default();
        let hotkey = FakeHotkeyController::default();
        let mut app = ScreenshotApp::new(config, backend, tray, hotkey)?;
        app.handle_command(ScreenshotUiCommand::Capture)?
            .ok_or_else(|| anyhow!("screenshot capture did not produce an artifact"))
    }
}

pub fn run_from_env() -> Result<()> {
    let options = CliOptions::parse()?;
    if options.help {
        println!("{}", CliOptions::usage());
        return Ok(());
    }
    let saved = options.run()?;
    println!("{}", saved.display());
    Ok(())
}

fn parse_target(value: &str) -> Result<CaptureTarget> {
    match value {
        "fullscreen" => Ok(CaptureTarget::Fullscreen),
        "active-window" => Ok(CaptureTarget::ActiveWindow),
        other => bail!("unknown capture target: {other}"),
    }
}

fn parse_format(value: &str) -> Result<ScreenshotFormat> {
    match value {
        "png" => Ok(ScreenshotFormat::Png),
        "jpeg" | "jpg" => Ok(ScreenshotFormat::Jpeg),
        other => bail!("unknown screenshot format: {other}"),
    }
}

fn parse_u8_in_range(value: &str, range: std::ops::RangeInclusive<u8>, name: &str) -> Result<u8> {
    let parsed: u8 = value.parse().map_err(|_| anyhow!("{name} must be a number"))?;
    if !range.contains(&parsed) {
        bail!("{name} must be in {range:?}");
    }
    Ok(parsed)
}

fn next_value<I, S>(iter: &mut I, option: &str) -> Result<String>
where
    I: Iterator<Item = S>,
    S: AsRef<str>,
{
    iter.next()
        .map(|value| value.as_ref().to_owned())
        .ok_or_else(|| anyhow!("missing value for {option}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::CaptureArtifactReader;
    use crate::capture::{CaptureClient, DisplaydCaptureArtifact};
    use crate::{
        artifact::{DisplaydArtifactCaptureClient, FileCaptureArtifactReader},
        displayd_ipc::{DisplaydIpcCaptureClient, FakeDisplaydTransport},
    };
    use std::{fs, io::BufReader, os::unix::net::UnixListener, path::Path, sync::mpsc, thread};
    use tempfile::tempdir;
    use waybroker_common::{DisplayEvent, IpcEnvelope, MessageKind, ServiceRole};
    use xwin_sec::{DecisionReason, SecurityDecision, browser_hostile_client};

    #[test]
    fn cli_defaults_to_fake_backend() {
        let opts = CliOptions::parse_from(["xwin-screenshot"]).unwrap();
        assert!(matches!(opts.backend, CaptureBackendKind::Fake));
    }

    #[test]
    fn cli_accepts_backend_fake() {
        let opts = CliOptions::parse_from(["xwin-screenshot", "--backend", "fake"]).unwrap();
        assert!(matches!(opts.backend, CaptureBackendKind::Fake));
    }

    #[test]
    fn cli_accepts_backend_isolated_displayd() {
        let dir = tempdir().unwrap();
        let opts = CliOptions::parse_from([
            "xwin-screenshot",
            "--backend",
            "isolated-displayd",
            "--displayd-socket",
            dir.path().join("displayd.sock").to_str().unwrap(),
            "--artifact-root",
            dir.path().to_str().unwrap(),
        ])
        .unwrap();
        assert!(matches!(opts.backend, CaptureBackendKind::IsolatedDisplayd));
    }

    #[test]
    fn cli_rejects_unknown_backend() {
        let err = CliOptions::parse_from(["xwin-screenshot", "--backend", "weird"]).unwrap_err();
        assert!(format!("{err:#}").contains("unknown screenshot backend"));
    }

    #[test]
    fn cli_isolated_displayd_requires_socket_path() {
        let dir = tempdir().unwrap();
        let err = CliOptions::parse_from([
            "xwin-screenshot",
            "--backend",
            "isolated-displayd",
            "--artifact-root",
            dir.path().to_str().unwrap(),
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("--displayd-socket"));
    }

    #[test]
    fn cli_isolated_displayd_requires_artifact_root() {
        let dir = tempdir().unwrap();
        let err = CliOptions::parse_from([
            "xwin-screenshot",
            "--backend",
            "isolated-displayd",
            "--displayd-socket",
            dir.path().join("displayd.sock").to_str().unwrap(),
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("--artifact-root"));
    }

    #[test]
    fn cli_rejects_run_user_socket_path() {
        let dir = tempdir().unwrap();
        let err = CliOptions::parse_from([
            "xwin-screenshot",
            "--backend",
            "isolated-displayd",
            "--displayd-socket",
            "/run/user/1000/displayd.sock",
            "--artifact-root",
            dir.path().to_str().unwrap(),
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("/run/user"));
    }

    #[test]
    fn cli_rejects_relative_socket_path() {
        let dir = tempdir().unwrap();
        let err = CliOptions::parse_from([
            "xwin-screenshot",
            "--backend",
            "isolated-displayd",
            "--displayd-socket",
            "displayd.sock",
            "--artifact-root",
            dir.path().to_str().unwrap(),
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("absolute"));
    }

    #[test]
    fn cli_rejects_unknown_option() {
        let err = CliOptions::parse_from(["xwin-screenshot", "--nope"]).unwrap_err();
        assert!(format!("{err:#}").contains("unknown option"));
    }

    #[test]
    fn cli_parses_target_fullscreen() {
        let opts = CliOptions::parse_from(["xwin-screenshot", "--target", "fullscreen"]).unwrap();
        assert_eq!(opts.target, CaptureTarget::Fullscreen);
    }

    #[test]
    fn cli_parses_target_active_window() {
        let opts =
            CliOptions::parse_from(["xwin-screenshot", "--target", "active-window"]).unwrap();
        assert_eq!(opts.target, CaptureTarget::ActiveWindow);
    }

    #[test]
    fn cli_parses_png_format() {
        let opts = CliOptions::parse_from(["xwin-screenshot", "--format", "png"]).unwrap();
        assert_eq!(opts.format, ScreenshotFormat::Png);
    }

    #[test]
    fn cli_parses_jpeg_format() {
        let opts = CliOptions::parse_from(["xwin-screenshot", "--format", "jpeg"]).unwrap();
        assert_eq!(opts.format, ScreenshotFormat::Jpeg);
    }

    #[test]
    fn cli_rejects_invalid_png_compression() {
        let err =
            CliOptions::parse_from(["xwin-screenshot", "--png-compression", "10"]).unwrap_err();
        assert!(format!("{err:#}").contains("png-compression"));
    }

    #[test]
    fn cli_rejects_invalid_jpeg_quality() {
        let err = CliOptions::parse_from(["xwin-screenshot", "--jpeg-quality", "0"]).unwrap_err();
        assert!(format!("{err:#}").contains("jpeg-quality"));
    }

    #[test]
    fn fake_backend_cli_capture_writes_png() {
        let dir = tempdir().unwrap();
        let opts = CliOptions { save_dir: dir.path().to_path_buf(), ..CliOptions::default() };
        let saved = opts.run().unwrap();
        assert_eq!(saved.extension().and_then(|s| s.to_str()), Some("png"));
        assert!(saved.exists());
    }

    #[test]
    fn fake_backend_cli_capture_writes_jpeg() {
        let dir = tempdir().unwrap();
        let opts = CliOptions {
            save_dir: dir.path().to_path_buf(),
            format: ScreenshotFormat::Jpeg,
            ..CliOptions::default()
        };
        let saved = opts.run().unwrap();
        assert_eq!(saved.extension().and_then(|s| s.to_str()), Some("jpg"));
        assert!(saved.exists());
    }

    fn write_rgba(path: &Path, width: u32, height: u32) -> Result<()> {
        let pixels =
            width.checked_mul(height).and_then(|value| value.checked_mul(4)).expect("valid size")
                as usize;
        let mut file = fs::File::create(path)?;
        std::io::Write::write_all(&mut file, &vec![0x7f; pixels])?;
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
        artifact_root: PathBuf,
    ) -> (thread::JoinHandle<Result<()>>, mpsc::Receiver<()>) {
        let (ready_tx, ready_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            if let Some(parent) = socket_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let listener = UnixListener::bind(&socket_path)?;
            ready_tx.send(()).ok();
            let (mut stream, _) = listener.accept()?;
            let mut reader = BufReader::new(stream.try_clone()?);
            let request: IpcEnvelope = waybroker_common::read_json_line(&mut reader)?;
            match request.kind {
                MessageKind::DisplayCommand(waybroker_common::DisplayCommand::CaptureOutput {
                    output,
                }) => {
                    assert!(!output.is_empty());
                }
                other => panic!("unexpected request kind: {other:?}"),
            }
            let artifact_path = artifact_root.join("frame.rgba");
            write_rgba(&artifact_path, 2, 2)?;
            waybroker_common::send_json_line(&mut stream, &response)?;
            Ok(())
        });
        (handle, ready_rx)
    }

    #[test]
    fn isolated_displayd_cli_capture_writes_png_from_loopback_artifact() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let response = success_response("frame.rgba", 2, 2);
        let (server, ready) = spawn_loopback_displayd_server(
            socket_path.clone(),
            response,
            artifact_dir.path().to_path_buf(),
        );
        ready.recv().unwrap();
        let opts = CliOptions {
            backend: CaptureBackendKind::IsolatedDisplayd,
            displayd_socket: Some(socket_path),
            artifact_root: Some(artifact_dir.path().to_path_buf()),
            save_dir: save_dir.path().to_path_buf(),
            ..CliOptions::default()
        };
        let saved = opts.run().unwrap();
        assert_eq!(saved.extension().and_then(|s| s.to_str()), Some("png"));
        assert!(saved.exists());
        server.join().unwrap().unwrap();
    }

    #[test]
    fn isolated_displayd_cli_capture_writes_jpeg_from_loopback_artifact() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let response = success_response("frame.rgba", 2, 2);
        let (server, ready) = spawn_loopback_displayd_server(
            socket_path.clone(),
            response,
            artifact_dir.path().to_path_buf(),
        );
        ready.recv().unwrap();
        let opts = CliOptions {
            backend: CaptureBackendKind::IsolatedDisplayd,
            displayd_socket: Some(socket_path),
            artifact_root: Some(artifact_dir.path().to_path_buf()),
            format: ScreenshotFormat::Jpeg,
            save_dir: save_dir.path().to_path_buf(),
            ..CliOptions::default()
        };
        let saved = opts.run().unwrap();
        assert_eq!(saved.extension().and_then(|s| s.to_str()), Some("jpg"));
        assert!(saved.exists());
        server.join().unwrap().unwrap();
    }

    #[test]
    fn policy_deny_prevents_socket_transport_from_cli_flow() {
        struct DenyPolicy;

        impl SecurityPolicy for DenyPolicy {
            fn decide(
                &self,
                _context: &PolicyContext,
                _capability: xwin_sec::XwinCapability,
            ) -> SecurityDecision {
                SecurityDecision::Deny { reason: DecisionReason::GlobalInputDenied }
            }
        }

        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let opts = CliOptions {
            backend: CaptureBackendKind::IsolatedDisplayd,
            displayd_socket: Some(socket_path),
            artifact_root: Some(artifact_dir.path().to_path_buf()),
            save_dir: save_dir.path().to_path_buf(),
            ..CliOptions::default()
        };
        let err = opts
            .run_with_policy(
                DenyPolicy,
                PolicyContext::new(browser_hostile_client("renderer-1", "org.example.browser")),
            )
            .unwrap_err();
        assert!(format!("{err:#}").contains("screen capture denied by policy"));
    }

    #[test]
    fn existing_isolated_transport_tests_still_pass() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let transport = FakeDisplaydTransport::with_response(IpcEnvelope::new(
            ServiceRole::Displayd,
            ServiceRole::Sessiond,
            MessageKind::DisplayEvent(DisplayEvent::OutputCaptured {
                output: "fullscreen".into(),
                width: 2,
                height: 2,
                format: "RGBA8888".into(),
                artifact_path: "frame.rgba".into(),
            }),
        ));
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let client = DisplaydArtifactCaptureClient::new(ipc, reader);
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.rgba.len(), 16);
    }

    #[test]
    fn existing_artifact_ingest_tests_still_pass() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", artifact_path).unwrap();
        let frame = reader.read_frame(&artifact).unwrap();
        assert_eq!(frame.rgba.len(), 16);
    }

    #[test]
    fn existing_displayd_rgba8888_writer_tests_still_pass() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", artifact_path).unwrap();
        let frame = reader.read_frame(&artifact).unwrap();
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
    }
}
