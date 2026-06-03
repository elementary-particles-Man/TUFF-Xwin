mod capture;
mod config;
mod encode;
mod hotkey;
mod tray;
mod ui;

use anyhow::Result;

use capture::FakeCaptureClient;
use config::ScreenshotConfig;
use hotkey::FakeHotkeyController;
use tray::FakeTrayController;
use ui::ScreenshotApp;

fn main() -> Result<()> {
    let config = ScreenshotConfig::default();
    config.validate()?;
    let capture_client = FakeCaptureClient::default();
    let tray = FakeTrayController::default();
    let hotkey = FakeHotkeyController::default();
    let _app = ScreenshotApp::new(config, capture_client, tray, hotkey)?;
    println!("xwin-screenshot scaffold ready");
    Ok(())
}
