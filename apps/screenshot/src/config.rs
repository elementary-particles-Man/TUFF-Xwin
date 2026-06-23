use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureTarget {
    Fullscreen,
    ActiveWindow,
}

impl CaptureTarget {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fullscreen => "fullscreen",
            Self::ActiveWindow => "active-window",
        }
    }
}

impl Default for CaptureTarget {
    fn default() -> Self {
        Self::Fullscreen
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScreenshotFormat {
    Png,
    Jpeg,
}

impl ScreenshotFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }
}

impl Default for ScreenshotFormat {
    fn default() -> Self {
        Self::Png
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PngOptions {
    pub compression_level: u8,
}

impl Default for PngOptions {
    fn default() -> Self {
        Self { compression_level: 6 }
    }
}

impl PngOptions {
    pub fn validate(self) -> Result<()> {
        if self.compression_level > 9 {
            bail!("png compression level must be in 0..=9");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JpegOptions {
    pub quality: u8,
}

impl Default for JpegOptions {
    fn default() -> Self {
        Self { quality: 90 }
    }
}

impl JpegOptions {
    pub fn validate(self) -> Result<()> {
        if !(1..=100).contains(&self.quality) {
            bail!("jpeg quality must be in 1..=100");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilenameTemplate(pub String);

impl Default for FilenameTemplate {
    fn default() -> Self {
        Self("xwin-{target}-{timestamp}".to_owned())
    }
}

impl FilenameTemplate {
    pub fn validate(&self) -> Result<()> {
        validate_filename_component(&self.0)
    }

    pub fn render(
        &self,
        target: CaptureTarget,
        format: ScreenshotFormat,
        sequence: u64,
    ) -> Result<String> {
        self.validate()?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let rendered = self
            .0
            .replace("{target}", target.as_str())
            .replace("{format}", format.as_str())
            .replace("{sequence}", &sequence.to_string())
            .replace("{timestamp}", &timestamp.to_string());
        validate_filename_component(&rendered)?;
        Ok(rendered)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotConfig {
    pub hotkey: String,
    pub save_dir: PathBuf,
    pub capture_target: CaptureTarget,
    pub format: ScreenshotFormat,
    pub png: PngOptions,
    pub jpeg: JpegOptions,
    pub filename_template: FilenameTemplate,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self {
            hotkey: "PrintScreen".to_owned(),
            save_dir: PathBuf::from("screenshots"),
            capture_target: CaptureTarget::Fullscreen,
            format: ScreenshotFormat::Png,
            png: PngOptions::default(),
            jpeg: JpegOptions::default(),
            filename_template: FilenameTemplate::default(),
        }
    }
}

impl ScreenshotConfig {
    pub fn validate(&self) -> Result<()> {
        if self.hotkey.trim().is_empty() {
            bail!("hotkey must not be empty");
        }
        validate_save_dir(&self.save_dir)?;
        self.png.validate()?;
        self.jpeg.validate()?;
        self.filename_template.validate()?;
        Ok(())
    }

    pub fn artifact_filename(&self, sequence: u64) -> Result<String> {
        let stem = self.filename_template.render(self.capture_target, self.format, sequence)?;
        Ok(format!("{stem}.{}", self.format.extension()))
    }

    pub fn artifact_path(&self, sequence: u64) -> Result<PathBuf> {
        self.validate()?;
        let filename = self.artifact_filename(sequence)?;
        Ok(self.save_dir.join(filename))
    }
}

fn validate_save_dir(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("save_dir must not be empty");
    }
    Ok(())
}

fn validate_filename_component(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("filename template must not be empty");
    }
    if value == "." || value == ".." {
        bail!("filename template must not resolve to a traversal segment");
    }
    if Path::new(value).is_absolute() {
        bail!("filename template must not be absolute");
    }
    if value.chars().any(|ch| matches!(ch, '/' | '\\' | '\0')) {
        bail!("filename template must not contain path separators");
    }
    if value.len() > 255 {
        bail!("filename template must be 255 bytes or less");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config = ScreenshotConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn config_roundtrip() {
        let config = ScreenshotConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let decoded: ScreenshotConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, config);
    }

    #[test]
    fn invalid_png_compression_is_rejected() {
        let config = ScreenshotConfig {
            png: PngOptions { compression_level: 10 },
            ..ScreenshotConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_jpeg_quality_is_rejected() {
        let config =
            ScreenshotConfig { jpeg: JpegOptions { quality: 0 }, ..ScreenshotConfig::default() };
        assert!(config.validate().is_err());
    }

    #[test]
    fn filename_template_rejects_traversal() {
        let template = FilenameTemplate("../evil".to_owned());
        assert!(template.validate().is_err());
    }
}
