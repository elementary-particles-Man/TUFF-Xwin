mod artifact;
mod capture;
mod config;
mod displayd_ipc;
mod encode;
mod hotkey;
mod tray;
mod ui;

use anyhow::Result;
use std::path::PathBuf;

use capture::FakeCaptureClient;
use config::ScreenshotConfig;
use displayd_ipc::{
    DisplaydIpcCaptureClient, DisplaydUnixSocketTransport, FakeDisplaydTransport,
    screenshot_user_policy_context,
};
use hotkey::FakeHotkeyController;
use tray::FakeTrayController;
use ui::ScreenshotApp;
use xwin_sec::BrowserSecurityPolicy;

fn main() -> Result<()> {
    let config = ScreenshotConfig::default();
    config.validate()?;
    let capture_client = FakeCaptureClient::default();
    let tray = FakeTrayController::default();
    let hotkey = FakeHotkeyController::default();
    let _app = ScreenshotApp::new(config, capture_client, tray, hotkey)?;
    let displayd_socket_path =
        std::env::args().skip_while(|arg| arg != "--displayd-socket").nth(1).map(PathBuf::from);
    if let Some(socket_path) = displayd_socket_path {
        let _displayd_ipc = DisplaydIpcCaptureClient::new(
            DisplaydUnixSocketTransport::new(socket_path)?,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
    } else {
        let _displayd_ipc = DisplaydIpcCaptureClient::new(
            FakeDisplaydTransport::default(),
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
    }
    println!("xwin-screenshot scaffold ready");
    Ok(())
}
