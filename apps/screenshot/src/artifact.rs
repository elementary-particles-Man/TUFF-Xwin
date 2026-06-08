use std::{
    ffi::CString,
    fs,
    io::Read,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    path::{Component, Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

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

    fn read_expected_rgba_bytes(
        &self,
        reader: &mut impl Read,
        expected_len: usize,
        path: &Path,
    ) -> Result<Vec<u8>> {
        let mut rgba = Vec::with_capacity(expected_len);
        reader
            .read_to_end(&mut rgba)
            .with_context(|| format!("failed to read displayd artifact {}", path.display()))?;
        if rgba.len() != expected_len {
            bail!(
                "displayd artifact size mismatch after read: expected {expected_len}, got {}",
                rgba.len()
            );
        }
        Ok(rgba)
    }

    #[cfg(unix)]
    fn open_artifact_fd(&self, artifact_path: &Path) -> Result<OwnedFd> {
        self.validate_relative_artifact_path(artifact_path)?;
        let artifact_label = artifact_path.display().to_string();
        let mut dir_fd = open_directory_fd(self.allowed_root.as_path()).with_context(|| {
            format!("failed to open artifact root {}", self.allowed_root.as_path().display())
        })?;

        let components: Vec<_> = artifact_path.components().collect();
        for (index, component) in components.iter().enumerate() {
            let name = component.as_os_str().to_string_lossy();
            let next = if index + 1 == components.len() {
                open_artifact_file_fd(dir_fd.as_raw_fd(), component.as_os_str()).with_context(
                    || {
                        format!(
                            "failed to open displayd artifact {} under {}",
                            artifact_label,
                            self.allowed_root.as_path().display()
                        )
                    },
                )?
            } else {
                open_artifact_dir_fd(dir_fd.as_raw_fd(), component.as_os_str()).with_context(|| {
                    format!(
                        "failed to descend into artifact directory component {name} for {artifact_label}"
                    )
                })?
            };
            dir_fd = next;
        }

        Ok(dir_fd)
    }

    #[cfg(not(unix))]
    fn open_artifact_fd(&self, _artifact_path: &Path) -> Result<OwnedFd> {
        bail!("artifact reads are only supported on Unix in this build");
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
        let fd = self.open_artifact_fd(&artifact.artifact_path)?;
        let mut file = file_from_owned_fd(fd);
        let metadata = file
            .metadata()
            .with_context(|| format!("failed to inspect displayd artifact {}", path.display()))?;
        if !metadata.is_file() {
            bail!("displayd artifact path must point to a file");
        }
        let file_len = metadata.len();
        if file_len != expected_len as u64 {
            bail!("displayd artifact size mismatch: expected {expected_len}, got {file_len}");
        }
        let rgba = self.read_expected_rgba_bytes(&mut file, expected_len, &path)?;
        validate_rgba_buffer_size(artifact.width, artifact.height, rgba.len())?;
        CapturedFrame::new(artifact.width, artifact.height, rgba)
    }
}

#[cfg(unix)]
fn open_directory_fd(path: &Path) -> Result<OwnedFd> {
    let c_path = path_to_cstring(path)?;
    let fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    owned_fd_from_raw(fd).with_context(|| format!("failed to open directory {}", path.display()))
}

#[cfg(unix)]
fn open_artifact_dir_fd(dir_fd: RawFd, component: &std::ffi::OsStr) -> Result<OwnedFd> {
    let c_component = os_str_to_cstring(component)?;
    let fd = unsafe {
        libc::openat(
            dir_fd,
            c_component.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    owned_fd_from_raw(fd).context("failed to open artifact directory component")
}

#[cfg(unix)]
fn open_artifact_file_fd(dir_fd: RawFd, component: &std::ffi::OsStr) -> Result<OwnedFd> {
    let c_component = os_str_to_cstring(component)?;
    let fd = unsafe {
        libc::openat(
            dir_fd,
            c_component.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    owned_fd_from_raw(fd).context("failed to open artifact file")
}

#[cfg(unix)]
fn owned_fd_from_raw(fd: libc::c_int) -> Result<OwnedFd> {
    if fd < 0 {
        bail!("openat/open failed: {}", std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(unix)]
fn path_to_cstring(path: &Path) -> Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| anyhow!("path contains an embedded NUL byte"))
}

#[cfg(unix)]
fn os_str_to_cstring(component: &std::ffi::OsStr) -> Result<CString> {
    CString::new(component.as_bytes())
        .map_err(|_| anyhow!("path component contains an embedded NUL byte"))
}

#[cfg(not(unix))]
fn file_from_owned_fd(_fd: OwnedFd) -> fs::File {
    unreachable!("file_from_owned_fd is only used on Unix")
}

#[cfg(unix)]
fn file_from_owned_fd(fd: OwnedFd) -> fs::File {
    fd.into()
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
    use tempfile::tempdir;
    use waybroker_common::{DisplayEvent, IpcEnvelope, MessageKind, ServiceRole};
    use xwin_sec::BrowserSecurityPolicy;

    use crate::{
        config::{CaptureTarget, ScreenshotConfig, ScreenshotFormat},
        displayd_ipc::{
            DisplaydUnixSocketTransport, FakeDisplaydTransport, screenshot_user_policy_context,
        },
        harness_displayd::{HarnessDisplayd, HarnessDisplaydResponse},
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
        artifact_root: PathBuf,
        response: HarnessDisplaydResponse,
    ) -> HarnessDisplayd {
        HarnessDisplayd::spawn(socket_path, artifact_root, response).unwrap()
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
    fn openat_reader_accepts_valid_rgba8888_artifact_under_root() {
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

    #[cfg(unix)]
    #[test]
    fn openat_reader_rejects_directory_artifact() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("frame.rgba")).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", "frame.rgba").unwrap();
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
    fn openat_reader_rejects_symlink_artifact() {
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
    fn openat_reader_rejects_symlink_parent_directory() {
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
    fn openat_reader_rejects_absolute_artifact_path() {
        let dir = tempdir().unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact_path = PathBuf::from("/tmp/xwin-artifact-outside.rgba");
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", &artifact_path).unwrap();
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn openat_reader_rejects_empty_or_dot_component() {
        let dir = tempdir().unwrap();
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
    fn openat_reader_rejects_parent_traversal() {
        let dir = tempdir().unwrap();
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
    fn openat_reader_rejects_metadata_length_mismatch_before_read() {
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("frame.rgba");
        fs::write(&artifact_path, vec![0x55; 12]).unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let artifact =
            DisplaydCaptureArtifact::new("fullscreen", 2, 2, "RGBA8888", "frame.rgba").unwrap();
        assert!(reader.read_frame(&artifact).is_err());
    }

    #[test]
    fn openat_reader_rejects_read_length_mismatch_after_read() {
        let dir = tempdir().unwrap();
        let reader = FileCaptureArtifactReader::new(dir.path()).unwrap();
        let mut rgba = std::io::Cursor::new(vec![0x55; 15]);
        let path = Path::new("frame.rgba");
        assert!(reader.read_expected_rgba_bytes(&mut rgba, 16, path).is_err());
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
        let response = HarnessDisplaydResponse::output_captured("fullscreen", "frame.rgba", 2, 2);
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = spawn_loopback_displayd_server(
            socket_path.clone(),
            artifact_dir.path().to_path_buf(),
            response,
        );
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
        server.join().unwrap();
    }

    #[test]
    fn artifact_pipeline_png_encode_writes_file_from_loopback_capture() {
        let artifact_dir = tempdir().unwrap();
        let socket_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let artifact_path = artifact_dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = HarnessDisplaydResponse::output_captured("fullscreen", "frame.rgba", 2, 2);
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = spawn_loopback_displayd_server(
            socket_path.clone(),
            artifact_dir.path().to_path_buf(),
            response,
        );
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
        server.join().unwrap();
    }

    #[test]
    fn artifact_pipeline_jpeg_encode_writes_file_from_loopback_capture() {
        let artifact_dir = tempdir().unwrap();
        let socket_dir = tempdir().unwrap();
        let save_dir = tempdir().unwrap();
        let artifact_path = artifact_dir.path().join("frame.rgba");
        write_rgba(&artifact_path, 2, 2).unwrap();
        let response = HarnessDisplaydResponse::output_captured("fullscreen", "frame.rgba", 2, 2);
        let socket_path = socket_dir.path().join("displayd.sock");
        let server = spawn_loopback_displayd_server(
            socket_path.clone(),
            artifact_dir.path().to_path_buf(),
            response,
        );
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
        server.join().unwrap();
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
