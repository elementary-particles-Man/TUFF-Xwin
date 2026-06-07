mod artifact;
mod capture;
mod config;
mod displayd_ipc;
mod encode;
mod hotkey;
mod tray;
mod ui;

use anyhow::Result;

use capture::FakeCaptureClient;
use config::ScreenshotConfig;
use displayd_ipc::{
    DisplaydIpcCaptureClient, FakeDisplaydTransport, screenshot_user_policy_context,
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
    let _displayd_ipc = DisplaydIpcCaptureClient::new(
        FakeDisplaydTransport::default(),
        BrowserSecurityPolicy,
        screenshot_user_policy_context(),
    );
    println!("xwin-screenshot scaffold ready");
    Ok(())
}
