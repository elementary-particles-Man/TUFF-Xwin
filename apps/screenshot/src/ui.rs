use anyhow::Result;
use std::path::PathBuf;

use crate::capture::CaptureClient;
use crate::config::{CaptureTarget, ScreenshotConfig, ScreenshotFormat};
use crate::encode::save_captured_frame;
use crate::hotkey::HotkeyController;
use crate::tray::TrayController;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenshotUiCommand {
    UpdateSaveDir(PathBuf),
    SelectFormat(ScreenshotFormat),
    SetPngCompression(u8),
    SetJpegQuality(u8),
    SelectTarget(CaptureTarget),
    Capture,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenshotUiAction {
    None,
    RequestCapture,
    RequestCancel,
}

#[derive(Debug, Clone)]
pub struct ScreenshotUiState {
    pub popup_visible: bool,
    pub last_error: Option<String>,
    pub last_saved_artifact: Option<PathBuf>,
    pub config: ScreenshotConfig,
}

impl ScreenshotUiState {
    pub fn new(config: ScreenshotConfig) -> Self {
        Self { popup_visible: false, last_error: None, last_saved_artifact: None, config }
    }

    pub fn apply_command(&mut self, command: ScreenshotUiCommand) -> Result<ScreenshotUiAction> {
        match command {
            ScreenshotUiCommand::UpdateSaveDir(save_dir) => {
                self.update_config(|config| config.save_dir = save_dir)?;
                Ok(ScreenshotUiAction::None)
            }
            ScreenshotUiCommand::SelectFormat(format) => {
                self.update_config(|config| config.format = format)?;
                Ok(ScreenshotUiAction::None)
            }
            ScreenshotUiCommand::SetPngCompression(compression_level) => {
                self.update_config(|config| config.png.compression_level = compression_level)?;
                Ok(ScreenshotUiAction::None)
            }
            ScreenshotUiCommand::SetJpegQuality(quality) => {
                self.update_config(|config| config.jpeg.quality = quality)?;
                Ok(ScreenshotUiAction::None)
            }
            ScreenshotUiCommand::SelectTarget(target) => {
                self.update_config(|config| config.capture_target = target)?;
                Ok(ScreenshotUiAction::None)
            }
            ScreenshotUiCommand::Capture => {
                self.popup_visible = true;
                Ok(ScreenshotUiAction::RequestCapture)
            }
            ScreenshotUiCommand::Cancel => {
                self.popup_visible = false;
                Ok(ScreenshotUiAction::RequestCancel)
            }
        }
    }

    fn update_config<F>(&mut self, mutate: F) -> Result<()>
    where
        F: FnOnce(&mut ScreenshotConfig),
    {
        let mut next = self.config.clone();
        mutate(&mut next);
        next.validate()?;
        self.config = next;
        Ok(())
    }
}

#[derive(Debug)]
pub struct ScreenshotApp<C, T, H>
where
    C: CaptureClient,
    T: TrayController,
    H: HotkeyController,
{
    pub state: ScreenshotUiState,
    pub capture_client: C,
    pub tray: T,
    pub hotkey: H,
    next_sequence: u64,
}

impl<C, T, H> ScreenshotApp<C, T, H>
where
    C: CaptureClient,
    T: TrayController,
    H: HotkeyController,
{
    pub fn new(
        config: ScreenshotConfig,
        capture_client: C,
        tray: T,
        mut hotkey: H,
    ) -> Result<Self> {
        config.validate()?;
        hotkey.register_hotkey(&config.hotkey)?;
        Ok(Self {
            state: ScreenshotUiState::new(config),
            capture_client,
            tray,
            hotkey,
            next_sequence: 0,
        })
    }

    pub fn on_hotkey_pressed(&mut self, hotkey_name: &str) -> Result<()> {
        if hotkey_name == self.state.config.hotkey {
            self.state.popup_visible = true;
            self.tray.request_popup()?;
        }
        Ok(())
    }

    pub fn handle_command(&mut self, command: ScreenshotUiCommand) -> Result<Option<PathBuf>> {
        match self.state.apply_command(command)? {
            ScreenshotUiAction::None => Ok(None),
            ScreenshotUiAction::RequestCancel => {
                self.tray.dismiss_popup()?;
                Ok(None)
            }
            ScreenshotUiAction::RequestCapture => self.capture_and_save(),
        }
    }

    fn capture_and_save(&mut self) -> Result<Option<PathBuf>> {
        let frame = self.capture_client.capture(self.state.config.capture_target)?;
        let artifact = save_captured_frame(&self.state.config, &frame, self.next_sequence)?;
        self.next_sequence += 1;
        self.state.last_saved_artifact = Some(artifact.clone());
        self.state.popup_visible = false;
        Ok(Some(artifact))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    use crate::capture::FakeCaptureClient;
    use crate::config::{
        CaptureTarget, FilenameTemplate, JpegOptions, PngOptions, ScreenshotConfig,
        ScreenshotFormat,
    };
    use crate::hotkey::FakeHotkeyController;
    use crate::tray::FakeTrayController;

    #[test]
    fn ui_state_updates_target_save_dir_format_compression() {
        let mut state = ScreenshotUiState::new(ScreenshotConfig::default());
        let dir = tempdir().unwrap();
        let save_dir = dir.path().to_path_buf();
        state.apply_command(ScreenshotUiCommand::UpdateSaveDir(save_dir.clone())).unwrap();
        state
            .apply_command(ScreenshotUiCommand::SelectTarget(CaptureTarget::ActiveWindow))
            .unwrap();
        state.apply_command(ScreenshotUiCommand::SelectFormat(ScreenshotFormat::Jpeg)).unwrap();
        state.apply_command(ScreenshotUiCommand::SetPngCompression(2)).unwrap();
        state.apply_command(ScreenshotUiCommand::SetJpegQuality(80)).unwrap();
        state.config.validate().unwrap();
        assert_eq!(state.config.save_dir, save_dir);
        assert_eq!(state.config.capture_target, CaptureTarget::ActiveWindow);
        assert_eq!(state.config.format, ScreenshotFormat::Jpeg);
        assert_eq!(state.config.png, PngOptions { compression_level: 2 });
        assert_eq!(state.config.jpeg, JpegOptions { quality: 80 });
    }

    #[test]
    fn fake_printscreen_requests_popup() {
        let config = ScreenshotConfig::default();
        let capture = FakeCaptureClient::default();
        let tray = FakeTrayController::default();
        let hotkey = FakeHotkeyController::default();
        let mut app = ScreenshotApp::new(config, capture, tray, hotkey).unwrap();
        app.on_hotkey_pressed("PrintScreen").unwrap();
        assert!(app.state.popup_visible);
        assert_eq!(app.tray.popup_requests, 1);
    }

    #[test]
    fn capture_command_saves_expected_extension() {
        let dir = tempdir().unwrap();
        let config = ScreenshotConfig {
            save_dir: dir.path().to_path_buf(),
            filename_template: FilenameTemplate("capture-{sequence}".to_owned()),
            ..ScreenshotConfig::default()
        };
        let capture = FakeCaptureClient::default();
        let tray = FakeTrayController::default();
        let hotkey = FakeHotkeyController::default();
        let mut app = ScreenshotApp::new(config, capture, tray, hotkey).unwrap();
        let saved = app.handle_command(ScreenshotUiCommand::Capture).unwrap().unwrap();
        assert_eq!(saved.extension().and_then(|s| s.to_str()), Some("png"));
        assert!(saved.exists());
        assert_eq!(app.state.last_saved_artifact.as_ref(), Some(&saved));
    }

    #[test]
    fn fake_capture_backend_failure_returns_result_error_without_panic() {
        let config = ScreenshotConfig::default();
        let capture = FakeCaptureClient::with_failure(CaptureTarget::Fullscreen);
        let tray = FakeTrayController::default();
        let hotkey = FakeHotkeyController::default();
        let mut app = ScreenshotApp::new(config, capture, tray, hotkey).unwrap();
        let err = app.handle_command(ScreenshotUiCommand::Capture).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("fake capture backend failure"));
    }

    #[test]
    fn capture_target_and_filename_template_are_still_valid() {
        let mut state = ScreenshotUiState::new(ScreenshotConfig::default());
        state
            .update_config(|config| {
                config.filename_template = FilenameTemplate("shot-{target}-{sequence}".into())
            })
            .unwrap();
        assert!(matches!(state.config.capture_target, CaptureTarget::Fullscreen));
    }
}
