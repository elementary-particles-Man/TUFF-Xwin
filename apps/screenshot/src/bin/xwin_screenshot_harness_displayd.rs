use anyhow::{Result, anyhow, bail};
use std::{env, path::PathBuf, process};

use xwin_screenshot::{
    artifact::ArtifactRoot,
    harness_displayd::{HarnessDisplayd, HarnessDisplaydResponse, validate_socket_path},
};

fn main() {
    if let Err(error) = run_from_env() {
        eprintln!("{error:#}");
        process::exit(1);
    }
}

pub fn run_from_env() -> Result<()> {
    run_with_args(env::args())
}

pub fn run_with_args<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let options = HarnessDisplaydArgs::parse_from(args)?;
    options.run()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HarnessDisplaydArgs {
    socket: PathBuf,
    artifact_root: PathBuf,
    width: u32,
    height: u32,
    serve_once: bool,
    reject: Option<String>,
}

impl HarnessDisplaydArgs {
    fn parse_from<I, S>(args: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut iter = args.into_iter();
        let _program = iter.next();
        let mut socket = None;
        let mut artifact_root = None;
        let mut width = None;
        let mut height = None;
        let mut serve_once = false;
        let mut reject = None;

        while let Some(arg) = iter.next() {
            match arg.as_ref() {
                "--socket" => socket = Some(parse_path_flag("--socket", iter.next())?),
                "--artifact-root" => {
                    artifact_root = Some(parse_path_flag("--artifact-root", iter.next())?)
                }
                "--width" => width = Some(parse_positive_u32("--width", iter.next())?),
                "--height" => height = Some(parse_positive_u32("--height", iter.next())?),
                "--serve-once" => serve_once = true,
                "--reject" => reject = Some(parse_string_flag("--reject", iter.next())?),
                "--help" | "-h" => bail!("{}", Self::usage()),
                other => bail!("unknown harness displayd argument: {other}"),
            }
        }

        let socket = socket.ok_or_else(|| anyhow!("--socket is required"))?;
        let artifact_root = artifact_root.ok_or_else(|| anyhow!("--artifact-root is required"))?;
        let width = width.ok_or_else(|| anyhow!("--width is required"))?;
        let height = height.ok_or_else(|| anyhow!("--height is required"))?;
        if !serve_once {
            bail!("--serve-once is required");
        }

        let options = Self { socket, artifact_root, width, height, serve_once, reject };
        options.validate()?;
        Ok(options)
    }

    fn validate(&self) -> Result<()> {
        validate_socket_path(&self.socket)?;
        ArtifactRoot::new(&self.artifact_root)?;
        if self.width == 0 {
            bail!("--width must be greater than zero");
        }
        if self.height == 0 {
            bail!("--height must be greater than zero");
        }
        if self.serve_once && self.reject.is_none() {
            return Ok(());
        }
        Ok(())
    }

    fn run(self) -> Result<()> {
        self.validate()?;
        let response = match self.reject {
            Some(reason) => HarnessDisplaydResponse::rejected(reason),
            None => HarnessDisplaydResponse::output_captured(
                "fullscreen",
                "frame.rgba",
                self.width,
                self.height,
            ),
        };
        let server = HarnessDisplayd::spawn(self.socket, self.artifact_root, response)?;
        server.join()
    }

    fn usage() -> &'static str {
        "Usage: xwin-screenshot-harness-displayd --socket <absolute-tempdir-socket-path> --artifact-root <absolute-tempdir-artifact-root> --width <positive-u32> --height <positive-u32> --serve-once [--reject <reason>]"
    }
}

fn parse_path_flag(flag: &str, value: Option<impl AsRef<str>>) -> Result<PathBuf> {
    let value = value.ok_or_else(|| anyhow!("{flag} requires a value"))?;
    let path = PathBuf::from(value.as_ref());
    if path.as_os_str().is_empty() {
        bail!("{flag} must not be empty");
    }
    Ok(path)
}

fn parse_string_flag(flag: &str, value: Option<impl AsRef<str>>) -> Result<String> {
    let value = value.ok_or_else(|| anyhow!("{flag} requires a value"))?;
    let value = value.as_ref().to_owned();
    if value.trim().is_empty() {
        bail!("{flag} must not be empty");
    }
    Ok(value)
}

fn parse_positive_u32(flag: &str, value: Option<impl AsRef<str>>) -> Result<u32> {
    let value = value.ok_or_else(|| anyhow!("{flag} requires a value"))?;
    let value = value.as_ref();
    let parsed: u32 = value.parse().map_err(|_| anyhow!("{flag} must be a positive integer"))?;
    if parsed == 0 {
        bail!("{flag} must be greater than zero");
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{thread, time::Duration};
    use tempfile::tempdir;
    use xwin_screenshot::{
        artifact::FileCaptureArtifactReader,
        capture::CaptureClient,
        capture::DISPLAYD_SCREENSHOT_FORMAT_RGBA8888,
        config::CaptureTarget,
        displayd_ipc::{
            DisplaydIpcCaptureClient, DisplaydUnixSocketTransport, screenshot_user_policy_context,
        },
        ui::ScreenshotApp,
    };
    use xwin_sec::BrowserSecurityPolicy;

    fn base_args(socket: &std::path::Path, artifact_root: &std::path::Path) -> Vec<String> {
        vec![
            "xwin-screenshot-harness-displayd".to_owned(),
            "--socket".to_owned(),
            socket.display().to_string(),
            "--artifact-root".to_owned(),
            artifact_root.display().to_string(),
            "--width".to_owned(),
            "2".to_owned(),
            "--height".to_owned(),
            "2".to_owned(),
            "--serve-once".to_owned(),
        ]
    }

    fn wait_for_socket(path: &std::path::Path) {
        for _ in 0..100 {
            if path.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("socket did not appear: {}", path.display());
    }

    #[test]
    fn dev_harness_binary_arg_parser_rejects_missing_socket() {
        let dir = tempdir().unwrap();
        let err = HarnessDisplaydArgs::parse_from([
            "xwin-screenshot-harness-displayd",
            "--artifact-root",
            dir.path().to_str().unwrap(),
            "--width",
            "2",
            "--height",
            "2",
            "--serve-once",
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("--socket"));
    }

    #[test]
    fn dev_harness_binary_arg_parser_rejects_relative_socket_path() {
        let dir = tempdir().unwrap();
        let err = HarnessDisplaydArgs::parse_from([
            "xwin-screenshot-harness-displayd",
            "--socket",
            "relative.sock",
            "--artifact-root",
            dir.path().to_str().unwrap(),
            "--width",
            "2",
            "--height",
            "2",
            "--serve-once",
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("absolute"));
    }

    #[test]
    fn dev_harness_binary_arg_parser_rejects_run_user_socket_path() {
        let dir = tempdir().unwrap();
        let err = HarnessDisplaydArgs::parse_from([
            "xwin-screenshot-harness-displayd",
            "--socket",
            "/run/user/1000/harness.sock",
            "--artifact-root",
            dir.path().to_str().unwrap(),
            "--width",
            "2",
            "--height",
            "2",
            "--serve-once",
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("/run/user"));
    }

    #[test]
    fn dev_harness_binary_arg_parser_rejects_missing_artifact_root() {
        let dir = tempdir().unwrap();
        let err = HarnessDisplaydArgs::parse_from([
            "xwin-screenshot-harness-displayd",
            "--socket",
            dir.path().join("displayd.sock").to_str().unwrap(),
            "--width",
            "2",
            "--height",
            "2",
            "--serve-once",
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("--artifact-root"));
    }

    #[test]
    fn dev_harness_binary_arg_parser_rejects_run_user_artifact_root() {
        let socket_dir = tempdir().unwrap();
        let err = HarnessDisplaydArgs::parse_from([
            "xwin-screenshot-harness-displayd",
            "--socket",
            socket_dir.path().join("displayd.sock").to_str().unwrap(),
            "--artifact-root",
            "/run/user/1000/harness-artifacts",
            "--width",
            "2",
            "--height",
            "2",
            "--serve-once",
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("/run/user"));
    }

    #[test]
    fn dev_harness_binary_rejects_zero_width() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let err = HarnessDisplaydArgs::parse_from([
            "xwin-screenshot-harness-displayd",
            "--socket",
            socket_dir.path().join("displayd.sock").to_str().unwrap(),
            "--artifact-root",
            artifact_dir.path().to_str().unwrap(),
            "--width",
            "0",
            "--height",
            "2",
            "--serve-once",
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("--width"));
    }

    #[test]
    fn dev_harness_binary_rejects_zero_height() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let err = HarnessDisplaydArgs::parse_from([
            "xwin-screenshot-harness-displayd",
            "--socket",
            socket_dir.path().join("displayd.sock").to_str().unwrap(),
            "--artifact-root",
            artifact_dir.path().to_str().unwrap(),
            "--width",
            "2",
            "--height",
            "0",
            "--serve-once",
        ])
        .unwrap_err();
        assert!(format!("{err:#}").contains("--height"));
    }

    #[test]
    fn dev_harness_binary_serve_once_returns_output_captured_for_capture_output() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let args = base_args(&socket_path, artifact_dir.path());
        let handle = thread::spawn(move || run_with_args(args));

        wait_for_socket(&socket_path);
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let artifact = ipc.request_capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(artifact.output, "fullscreen");
        assert_eq!(artifact.format, DISPLAYD_SCREENSHOT_FORMAT_RGBA8888);
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn dev_harness_binary_serve_once_writes_rgba8888_artifact_under_root() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let args = base_args(&socket_path, artifact_dir.path());
        let handle = thread::spawn(move || run_with_args(args));

        wait_for_socket(&socket_path);
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let client = xwin_screenshot::artifact::DisplaydArtifactCaptureClient::new(
            ipc,
            FileCaptureArtifactReader::new(artifact_dir.path()).unwrap(),
        );
        let frame = client.capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(frame.rgba.len(), 16);
        assert!(artifact_dir.path().join("frame.rgba").exists());
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn dev_harness_binary_reject_mode_returns_display_event_rejected() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let mut args = base_args(&socket_path, artifact_dir.path());
        args.push("--reject".to_owned());
        args.push("policy denied".to_owned());
        let handle = thread::spawn(move || run_with_args(args));

        wait_for_socket(&socket_path);
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let err = ipc.request_capture(CaptureTarget::Fullscreen).unwrap_err();
        assert!(format!("{err:#}").contains("displayd rejected capture request"));
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn isolated_displayd_client_can_capture_png_through_dev_harness_serve_once() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let args = base_args(&socket_path, artifact_dir.path());
        let handle = thread::spawn(move || run_with_args(args));

        wait_for_socket(&socket_path);
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let client = xwin_screenshot::artifact::DisplaydArtifactCaptureClient::new(
            ipc,
            FileCaptureArtifactReader::new(artifact_dir.path()).unwrap(),
        );
        let config = xwin_screenshot::config::ScreenshotConfig {
            save_dir: save_dir.path().to_path_buf(),
            ..Default::default()
        };
        let mut app = ScreenshotApp::new(
            config,
            client,
            xwin_screenshot::tray::FakeTrayController::default(),
            xwin_screenshot::hotkey::FakeHotkeyController::default(),
        )
        .unwrap();
        let saved =
            app.handle_command(xwin_screenshot::ui::ScreenshotUiCommand::Capture).unwrap().unwrap();
        assert_eq!(saved.extension().and_then(|s| s.to_str()), Some("png"));
        assert!(saved.exists());
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn isolated_displayd_client_can_capture_jpeg_through_dev_harness_serve_once() {
        let socket_dir = tempdir().unwrap();
        let artifact_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let socket_path = socket_dir.path().join("displayd.sock");
        let args = base_args(&socket_path, artifact_dir.path());
        let handle = thread::spawn(move || run_with_args(args));

        wait_for_socket(&socket_path);
        let transport = DisplaydUnixSocketTransport::new(&socket_path).unwrap();
        let ipc = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let client = xwin_screenshot::artifact::DisplaydArtifactCaptureClient::new(
            ipc,
            FileCaptureArtifactReader::new(artifact_dir.path()).unwrap(),
        );
        let config = xwin_screenshot::config::ScreenshotConfig {
            save_dir: save_dir.path().to_path_buf(),
            format: xwin_screenshot::config::ScreenshotFormat::Jpeg,
            ..Default::default()
        };
        let mut app = ScreenshotApp::new(
            config,
            client,
            xwin_screenshot::tray::FakeTrayController::default(),
            xwin_screenshot::hotkey::FakeHotkeyController::default(),
        )
        .unwrap();
        let saved =
            app.handle_command(xwin_screenshot::ui::ScreenshotUiCommand::Capture).unwrap().unwrap();
        assert_eq!(saved.extension().and_then(|s| s.to_str()), Some("jpg"));
        assert!(saved.exists());
        handle.join().unwrap().unwrap();
    }
}
