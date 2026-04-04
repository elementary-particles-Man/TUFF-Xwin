use std::{
    env, fs,
    io::{BufRead, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};

use crate::ServiceRole;

pub fn runtime_dir() -> PathBuf {
    if let Some(path) = env::var_os("WAYBROKER_RUNTIME_DIR") {
        return PathBuf::from(path);
    }

    if let Some(path) = env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(path).join("waybroker");
    }

    env::temp_dir().join("waybroker")
}

pub fn ensure_runtime_dir() -> Result<PathBuf> {
    let dir = runtime_dir();
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create runtime dir {}", dir.display()))?;
    Ok(dir)
}

pub fn service_socket_path(role: ServiceRole) -> PathBuf {
    runtime_dir().join(format!("{}.sock", role.as_str()))
}

pub fn session_artifact_path(session_instance_id: &str, artifact_name: &str) -> PathBuf {
    let safe_id = sanitize_session_instance_id(session_instance_id);
    runtime_dir().join(format!("session-{}-{}.json", safe_id, artifact_name))
}

pub fn validate_session_instance_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }

    // path-safe な文字のみ許可: [A-Za-z0-9._-]
    // また、"." や ".." などの特殊ディレクトリ指定を防ぐ
    if id == "." || id == ".." {
        return false;
    }

    id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

pub fn sanitize_session_instance_id(id: &str) -> String {
    if id.is_empty() {
        return "default".to_string();
    }

    let mut sanitized: String =
        id.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' }
            })
            .collect();

    if sanitized == "." || sanitized == ".." {
        sanitized = format!("_{}", sanitized);
    }

    if sanitized.len() > 128 {
        sanitized.truncate(128);
    }

    sanitized
}

pub fn bind_service_socket(role: ServiceRole) -> Result<(UnixListener, PathBuf)> {
    let _ = ensure_runtime_dir()?;
    let socket_path = service_socket_path(role);

    match fs::remove_file(&socket_path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!("failed to remove stale socket {}", socket_path.display())
            });
        }
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind {}", socket_path.display()))?;

    Ok((listener, socket_path))
}

pub fn connect_service_socket(role: ServiceRole) -> Result<UnixStream> {
    let socket_path = service_socket_path(role);
    UnixStream::connect(&socket_path)
        .with_context(|| format!("failed to connect to {}", socket_path.display()))
}

pub fn send_json_line<T: Serialize>(writer: &mut impl Write, message: &T) -> Result<()> {
    serde_json::to_writer(&mut *writer, message).context("failed to serialize IPC message")?;
    writer.write_all(b"\n").context("failed to write IPC newline delimiter")?;
    writer.flush().context("failed to flush IPC message")?;
    Ok(())
}

pub fn read_json_line<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<T> {
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).context("failed to read IPC message")?;

    if bytes == 0 {
        bail!("unexpected EOF while reading IPC message");
    }

    serde_json::from_str(line.trim_end()).context("failed to decode IPC message")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{read_json_line, send_json_line};
    use crate::{DisplayCommand, IpcEnvelope, MessageKind, ServiceRole};

    #[test]
    fn validates_session_instance_id() {
        use super::validate_session_instance_id;
        assert!(validate_session_instance_id("default-single-session"));
        assert!(validate_session_instance_id("abc-123"));
        assert!(validate_session_instance_id("sess.demo_01"));
        assert!(!validate_session_instance_id("../evil"));
        assert!(!validate_session_instance_id("a/b"));
        assert!(!validate_session_instance_id("a\\b"));
        assert!(!validate_session_instance_id(""));
        assert!(!validate_session_instance_id("."));
        assert!(!validate_session_instance_id(".."));
        assert!(!validate_session_instance_id(&"a".repeat(129)));
        assert!(!validate_session_instance_id("hello\0world"));
    }

    #[test]
    fn sanitizes_session_instance_id() {
        use super::sanitize_session_instance_id;
        assert_eq!(
            sanitize_session_instance_id("default-single-session"),
            "default-single-session"
        );
        assert_eq!(sanitize_session_instance_id("../evil"), ".._evil");
        assert_eq!(sanitize_session_instance_id("a/b"), "a_b");
        assert_eq!(sanitize_session_instance_id("."), "_.");
        assert_eq!(sanitize_session_instance_id(".."), "_..");
        assert_eq!(sanitize_session_instance_id(""), "default");
        assert_eq!(sanitize_session_instance_id("hello\x01world"), "hello_world");
    }

    #[test]
    fn session_artifact_path_stays_within_runtime_dir() {
        use super::{runtime_dir, session_artifact_path};
        let runtime = runtime_dir();
        let path = session_artifact_path("../evil/path", "test");
        assert!(path.starts_with(&runtime));
        // Verify no directory traversal
        assert!(!path.to_string_lossy().contains("/evil"));
        assert!(path.to_string_lossy().contains(".._evil_path"));
    }
}
