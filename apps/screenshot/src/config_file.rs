use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    artifact::FileCaptureArtifactReader,
    cli::{CaptureBackendKind, CliOptions, parse_format, parse_target, parse_u8_in_range},
    displayd_ipc::DisplaydUnixSocketTransport,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotConfigFile {
    pub backend: Option<String>,
    pub displayd_socket: Option<PathBuf>,
    pub artifact_root: Option<PathBuf>,
    pub target: Option<String>,
    pub format: Option<String>,
    pub save_dir: Option<PathBuf>,
    pub png_compression: Option<u8>,
    pub jpeg_quality: Option<u8>,
}

impl ScreenshotConfigFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            bail!("config file path must not be empty");
        }
        if path.is_dir() {
            bail!("config file path must be a file, not a directory");
        }

        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        if contents.trim().is_empty() {
            bail!("config file must not be empty");
        }

        let file: Self = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;
        file.validate()?;
        Ok(file)
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(backend) = self.backend.as_deref() {
            parse_backend(backend)?;
        } else if self.displayd_socket.is_some() || self.artifact_root.is_some() {
            bail!(
                "displayd socket and artifact root require an explicit isolated-displayd backend"
            );
        }

        if let Some(target) = self.target.as_deref() {
            parse_target(target)?;
        }
        if let Some(format) = self.format.as_deref() {
            parse_format(format)?;
        }
        if let Some(compression) = self.png_compression {
            parse_u8_in_range(&compression.to_string(), 0..=9, "png-compression")?;
        }
        if let Some(quality) = self.jpeg_quality {
            parse_u8_in_range(&quality.to_string(), 1..=100, "jpeg-quality")?;
        }

        match self.backend.as_deref() {
            Some("fake") | None => {
                if self.displayd_socket.is_some() {
                    bail!("fake backend must not take a displayd socket");
                }
                if self.artifact_root.is_some() {
                    bail!("fake backend must not take an artifact root");
                }
            }
            Some("isolated-displayd") => {
                let socket = self
                    .displayd_socket
                    .as_ref()
                    .ok_or_else(|| anyhow!("isolated-displayd backend requires displayd_socket"))?;
                DisplaydUnixSocketTransport::new(socket.clone())?;
                let artifact_root = self
                    .artifact_root
                    .as_ref()
                    .ok_or_else(|| anyhow!("isolated-displayd backend requires artifact_root"))?;
                FileCaptureArtifactReader::new(artifact_root.clone())?;
            }
            Some(other) => bail!("unknown screenshot backend: {other}"),
        }

        Ok(())
    }

    pub fn apply_to(&self, options: &mut CliOptions) -> Result<()> {
        if let Some(backend) = self.backend.as_deref() {
            options.backend = parse_backend(backend)?;
        }
        if let Some(displayd_socket) = &self.displayd_socket {
            options.displayd_socket = Some(displayd_socket.clone());
        }
        if let Some(artifact_root) = &self.artifact_root {
            options.artifact_root = Some(artifact_root.clone());
        }
        if let Some(target) = self.target.as_deref() {
            options.target = parse_target(target)?;
        }
        if let Some(format) = self.format.as_deref() {
            options.format = parse_format(format)?;
        }
        if let Some(save_dir) = &self.save_dir {
            options.save_dir = save_dir.clone();
        }
        if let Some(compression) = self.png_compression {
            options.png_compression = compression;
        }
        if let Some(quality) = self.jpeg_quality {
            options.jpeg_quality = quality;
        }

        options.validate()
    }
}

fn parse_backend(value: &str) -> Result<CaptureBackendKind> {
    match value {
        "fake" => Ok(CaptureBackendKind::Fake),
        "isolated-displayd" => Ok(CaptureBackendKind::IsolatedDisplayd),
        other => bail!("unknown screenshot backend: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_config(contents: &str) -> Result<(tempfile::TempDir, PathBuf)> {
        let dir = tempdir()?;
        let path = dir.path().join("config.toml");
        fs::write(&path, contents)?;
        Ok((dir, path))
    }

    #[test]
    fn config_file_rejects_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.toml");
        fs::write(&path, "").unwrap();
        let err = ScreenshotConfigFile::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("must not be empty"));
    }

    #[test]
    fn config_file_rejects_invalid_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        fs::write(&path, "backend = [").unwrap();
        let err = ScreenshotConfigFile::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("failed to parse config file"));
    }

    #[test]
    fn config_file_rejects_unknown_field() {
        let (_dir, path) = write_config("backend = \"fake\"\nunknown_field = 1\n").unwrap();
        let err = ScreenshotConfigFile::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("unknown field"));
    }

    #[test]
    fn config_file_loads_fake_backend_png() {
        let (_dir, path) = write_config(
            r#"
backend = "fake"
format = "png"
save_dir = "screenshots"
png_compression = 6
jpeg_quality = 90
"#,
        )
        .unwrap();
        let config = ScreenshotConfigFile::load(&path).unwrap();
        assert_eq!(config.backend.as_deref(), Some("fake"));
        assert_eq!(config.format.as_deref(), Some("png"));
    }

    #[test]
    fn config_file_loads_fake_backend_jpeg() {
        let (_dir, path) = write_config(
            r#"
backend = "fake"
format = "jpeg"
save_dir = "shots"
png_compression = 6
jpeg_quality = 80
"#,
        )
        .unwrap();
        let config = ScreenshotConfigFile::load(&path).unwrap();
        assert_eq!(config.format.as_deref(), Some("jpeg"));
    }

    #[test]
    fn config_file_loads_isolated_displayd_backend() {
        let dir = tempdir().unwrap();
        let (_config_dir, path) = write_config(&format!(
            r#"
backend = "isolated-displayd"
displayd_socket = "{}"
artifact_root = "{}"
save_dir = "shots"
format = "png"
"#,
            dir.path().join("displayd.sock").display(),
            dir.path().display(),
        ))
        .unwrap();
        let config = ScreenshotConfigFile::load(&path).unwrap();
        assert_eq!(config.backend.as_deref(), Some("isolated-displayd"));
    }

    #[test]
    fn config_file_rejects_unknown_backend() {
        let (_dir, path) = write_config("backend = \"weird\"\n").unwrap();
        let err = ScreenshotConfigFile::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("unknown screenshot backend"));
    }

    #[test]
    fn config_file_rejects_invalid_png_compression() {
        let (_dir, path) = write_config("backend = \"fake\"\npng_compression = 10\n").unwrap();
        let err = ScreenshotConfigFile::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("png-compression"));
    }

    #[test]
    fn config_file_rejects_invalid_jpeg_quality() {
        let (_dir, path) = write_config("backend = \"fake\"\njpeg_quality = 0\n").unwrap();
        let err = ScreenshotConfigFile::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("jpeg-quality"));
    }

    #[test]
    fn config_file_isolated_displayd_requires_socket_path() {
        let (_dir, path) = write_config(
            r#"
backend = "isolated-displayd"
artifact_root = "shots"
"#,
        )
        .unwrap();
        let err = ScreenshotConfigFile::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("displayd_socket"));
    }

    #[test]
    fn config_file_isolated_displayd_requires_artifact_root() {
        let (_dir, path) = write_config(
            r#"
backend = "isolated-displayd"
displayd_socket = "/tmp/displayd.sock"
"#,
        )
        .unwrap();
        let err = ScreenshotConfigFile::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("artifact_root"));
    }

    #[test]
    fn config_file_rejects_run_user_socket_path() {
        let (_dir, path) = write_config(
            r#"
backend = "isolated-displayd"
displayd_socket = "/run/user/1000/displayd.sock"
artifact_root = "/tmp/artifacts"
"#,
        )
        .unwrap();
        let err = ScreenshotConfigFile::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("/run/user"));
    }

    #[test]
    fn config_file_rejects_run_user_artifact_root() {
        let (_dir, path) = write_config(
            r#"
backend = "isolated-displayd"
displayd_socket = "/tmp/displayd.sock"
artifact_root = "/run/user/1000/artifacts"
"#,
        )
        .unwrap();
        let err = ScreenshotConfigFile::load(&path).unwrap_err();
        assert!(format!("{err:#}").contains("/run/user"));
    }

    #[test]
    fn config_file_does_not_expand_environment_variables() {
        let (_dir, path) = write_config(
            r#"
backend = "fake"
save_dir = "$HOME/screenshots"
"#,
        )
        .unwrap();
        let config = ScreenshotConfigFile::load(&path).unwrap();
        assert_eq!(config.save_dir.as_deref(), Some(Path::new("$HOME/screenshots")));
    }

    #[test]
    fn config_file_does_not_expand_tilde() {
        let (_dir, path) = write_config(
            r#"
backend = "fake"
save_dir = "~/screenshots"
"#,
        )
        .unwrap();
        let config = ScreenshotConfigFile::load(&path).unwrap();
        assert_eq!(config.save_dir.as_deref(), Some(Path::new("~/screenshots")));
    }
}
