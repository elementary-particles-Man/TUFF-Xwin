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
    runtime_dir().join(format!("session-{}-{}.json", session_instance_id, artifact_name))
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
    fn roundtrips_line_framed_envelope() {
        let envelope = IpcEnvelope::new(
            ServiceRole::Waylandd,
            ServiceRole::Displayd,
            MessageKind::DisplayCommand(DisplayCommand::EnumerateOutputs),
        );

        let mut buffer = Vec::new();
        send_json_line(&mut buffer, &envelope).expect("serialize");

        let mut cursor = Cursor::new(buffer);
        let decoded: IpcEnvelope = read_json_line(&mut cursor).expect("deserialize");

        assert_eq!(decoded, envelope);
    }
}
