use std::{
    env, fmt, fs,
    io::{BufRead, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};

use crate::ServiceRole;

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

const DEFAULT_TCP_BASE_PORT: u16 = 47000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceTransport {
    Unix,
    Tcp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceEndpoint {
    Unix(PathBuf),
    Tcp(SocketAddr),
}

impl ServiceEndpoint {
    pub fn cleanup_stale(&self) -> Result<()> {
        match self {
            Self::Unix(path) => match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err)
                    .with_context(|| format!("failed to remove stale socket {}", path.display())),
            },
            Self::Tcp(_) => Ok(()),
        }
    }
}

impl fmt::Display for ServiceEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix(path) => write!(f, "{}", path.display()),
            Self::Tcp(addr) => write!(f, "{addr}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServicePeer;

pub struct ServiceListener {
    inner: ServiceListenerInner,
    endpoint: ServiceEndpoint,
}

enum ServiceListenerInner {
    #[cfg(unix)]
    Unix(UnixListener),
    Tcp(TcpListener),
}

pub struct ServiceIncoming<'a> {
    listener: &'a ServiceListener,
}

impl<'a> Iterator for ServiceIncoming<'a> {
    type Item = std::io::Result<ServiceStream>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(self.listener.accept().map(|(stream, _peer)| stream))
    }
}

pub fn is_recoverable_accept_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::NotConnected
    )
}

pub struct ServiceStream {
    inner: ServiceStreamInner,
}

enum ServiceStreamInner {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl ServiceListener {
    pub fn accept(&self) -> std::io::Result<(ServiceStream, ServicePeer)> {
        match &self.inner {
            #[cfg(unix)]
            ServiceListenerInner::Unix(listener) => {
                let (stream, _addr) = listener.accept()?;
                Ok((ServiceStream { inner: ServiceStreamInner::Unix(stream) }, ServicePeer))
            }
            ServiceListenerInner::Tcp(listener) => {
                let (stream, _addr) = listener.accept()?;
                Ok((ServiceStream { inner: ServiceStreamInner::Tcp(stream) }, ServicePeer))
            }
        }
    }

    pub fn incoming(&self) -> ServiceIncoming<'_> {
        ServiceIncoming { listener: self }
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> std::io::Result<()> {
        match &self.inner {
            #[cfg(unix)]
            ServiceListenerInner::Unix(listener) => listener.set_nonblocking(nonblocking),
            ServiceListenerInner::Tcp(listener) => listener.set_nonblocking(nonblocking),
        }
    }

    pub fn endpoint(&self) -> &ServiceEndpoint {
        &self.endpoint
    }
}

impl ServiceStream {
    pub fn try_clone(&self) -> std::io::Result<Self> {
        match &self.inner {
            #[cfg(unix)]
            ServiceStreamInner::Unix(stream) => {
                Ok(Self { inner: ServiceStreamInner::Unix(stream.try_clone()?) })
            }
            ServiceStreamInner::Tcp(stream) => {
                Ok(Self { inner: ServiceStreamInner::Tcp(stream.try_clone()?) })
            }
        }
    }
}

impl Read for ServiceStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match &mut self.inner {
            #[cfg(unix)]
            ServiceStreamInner::Unix(stream) => stream.read(buf),
            ServiceStreamInner::Tcp(stream) => stream.read(buf),
        }
    }
}

impl Write for ServiceStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match &mut self.inner {
            #[cfg(unix)]
            ServiceStreamInner::Unix(stream) => stream.write(buf),
            ServiceStreamInner::Tcp(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.inner {
            #[cfg(unix)]
            ServiceStreamInner::Unix(stream) => stream.flush(),
            ServiceStreamInner::Tcp(stream) => stream.flush(),
        }
    }
}

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

pub const MAX_IPC_JSON_LINE_BYTES: usize = 64 * 1024;
pub const MAX_ARTIFACT_FILENAME_BYTES: usize = 128;

pub fn validate_runtime_socket_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }
    let rendered = path.to_string_lossy();
    if rendered.contains("/run/user/") {
        return false;
    }
    !path.components().any(|component| {
        matches!(component, std::path::Component::ParentDir | std::path::Component::Prefix(_))
    })
}

pub fn validate_artifact_filename(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_ARTIFACT_FILENAME_BYTES {
        return false;
    }
    if name == "." || name == ".." {
        return false;
    }
    !name.contains('/') && !name.contains('\\') && !name.contains('\0') && !name.starts_with('.')
}

pub fn sanitize_artifact_filename(name: &str) -> String {
    let mut sanitized: String =
        name.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' }
            })
            .collect();
    if sanitized.is_empty() {
        sanitized = "artifact".to_string();
    }
    if sanitized == "." || sanitized == ".." {
        sanitized = format!("_{}", sanitized);
    }
    if sanitized.len() > MAX_ARTIFACT_FILENAME_BYTES {
        sanitized.truncate(MAX_ARTIFACT_FILENAME_BYTES);
    }
    sanitized
}

pub fn service_socket_path(role: ServiceRole) -> PathBuf {
    runtime_dir().join(format!("{}.sock", role.as_str()))
}

fn selected_transport() -> ServiceTransport {
    match env::var("WAYBROKER_TRANSPORT") {
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "tcp" => ServiceTransport::Tcp,
            "unix" => {
                #[cfg(unix)]
                {
                    ServiceTransport::Unix
                }
                #[cfg(not(unix))]
                {
                    ServiceTransport::Tcp
                }
            }
            _ => {
                #[cfg(unix)]
                {
                    ServiceTransport::Unix
                }
                #[cfg(not(unix))]
                {
                    ServiceTransport::Tcp
                }
            }
        },
        Err(_) => {
            #[cfg(unix)]
            {
                ServiceTransport::Unix
            }
            #[cfg(not(unix))]
            {
                ServiceTransport::Tcp
            }
        }
    }
}

fn tcp_base_port() -> u16 {
    env::var("WAYBROKER_TCP_BASE_PORT")
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok())
        .filter(|port| *port <= u16::MAX - 16)
        .unwrap_or(DEFAULT_TCP_BASE_PORT)
}

fn service_port(role: ServiceRole) -> u16 {
    let offset = match role {
        ServiceRole::Displayd => 0,
        ServiceRole::Waylandd => 1,
        ServiceRole::Compd => 2,
        ServiceRole::Lockd => 3,
        ServiceRole::Sessiond => 4,
        ServiceRole::Watchdog => 5,
        ServiceRole::X11Bridge => 6,
    };
    tcp_base_port() + offset
}

fn service_endpoint(role: ServiceRole) -> ServiceEndpoint {
    match selected_transport() {
        ServiceTransport::Unix => ServiceEndpoint::Unix(service_socket_path(role)),
        ServiceTransport::Tcp => ServiceEndpoint::Tcp(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            service_port(role),
        )),
    }
}

pub fn session_artifact_path(session_instance_id: &str, artifact_name: &str) -> PathBuf {
    let safe_id = sanitize_session_instance_id(session_instance_id);
    let safe_artifact_name = sanitize_artifact_filename(artifact_name);
    runtime_dir().join(format!("session-{}-{}.json", safe_id, safe_artifact_name))
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

pub fn bind_service_socket(role: ServiceRole) -> Result<ServiceListener> {
    let _ = ensure_runtime_dir()?;
    let endpoint = service_endpoint(role);
    bind_endpoint(endpoint)
}

pub fn bind_explicit_unix_socket(path: PathBuf) -> Result<ServiceListener> {
    let endpoint = ServiceEndpoint::Unix(path);
    bind_endpoint(endpoint)
}

fn bind_endpoint(endpoint: ServiceEndpoint) -> Result<ServiceListener> {
    endpoint.cleanup_stale()?;

    let inner = match &endpoint {
        #[cfg(unix)]
        ServiceEndpoint::Unix(path) => ServiceListenerInner::Unix(
            UnixListener::bind(path)
                .with_context(|| format!("failed to bind {}", path.display()))?,
        ),
        ServiceEndpoint::Tcp(addr) => ServiceListenerInner::Tcp(
            TcpListener::bind(addr).with_context(|| format!("failed to bind {}", addr))?,
        ),
    };

    Ok(ServiceListener { inner, endpoint })
}

pub fn connect_service_socket(role: ServiceRole) -> Result<ServiceStream> {
    let endpoint = service_endpoint(role);
    match endpoint {
        #[cfg(unix)]
        ServiceEndpoint::Unix(path) => UnixStream::connect(&path)
            .map(|stream| ServiceStream { inner: ServiceStreamInner::Unix(stream) })
            .with_context(|| format!("failed to connect to {}", path.display())),
        ServiceEndpoint::Tcp(addr) => TcpStream::connect(addr)
            .map(|stream| ServiceStream { inner: ServiceStreamInner::Tcp(stream) })
            .with_context(|| format!("failed to connect to {}", addr)),
    }
}

pub fn send_json_line<T: Serialize>(writer: &mut impl Write, message: &T) -> Result<()> {
    serde_json::to_writer(&mut *writer, message).context("failed to serialize IPC message")?;
    writer.write_all(b"\n").context("failed to write IPC newline delimiter")?;
    writer.flush().context("failed to flush IPC message")?;
    Ok(())
}

pub fn read_json_line<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<T> {
    let mut line = Vec::new();
    let bytes = reader.read_until(b'\n', &mut line).context("failed to read IPC message")?;

    if bytes == 0 {
        bail!("unexpected EOF while reading IPC message");
    }

    if bytes > MAX_IPC_JSON_LINE_BYTES || line.len() > MAX_IPC_JSON_LINE_BYTES + 1 {
        bail!("IPC message exceeds {} bytes", MAX_IPC_JSON_LINE_BYTES);
    }
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    let line = std::str::from_utf8(&line).context("IPC message is not valid UTF-8")?;
    serde_json::from_str(line).context("failed to decode IPC message")
}

#[cfg(test)]
mod tests {
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
    fn validates_runtime_socket_path_and_artifact_filename() {
        use super::{
            sanitize_artifact_filename, validate_artifact_filename, validate_runtime_socket_path,
        };
        use std::{env, path::PathBuf};

        assert!(validate_runtime_socket_path(&env::temp_dir().join("waybroker/test.sock")));
        assert!(!validate_runtime_socket_path(&PathBuf::from(
            "/run/user/1000/waybroker/test.sock"
        )));
        assert_eq!(sanitize_artifact_filename("../evil.png"), ".._evil.png");
        assert_eq!(sanitize_artifact_filename(""), "artifact");
        assert_eq!(sanitize_artifact_filename("a/b\\c"), "a_b_c");
        assert!(validate_artifact_filename("screenshot-1.png"));
        assert!(!validate_artifact_filename("../evil.png"));
        assert!(!validate_artifact_filename("/abs/path.png"));
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

    #[test]
    fn rejects_oversized_json_line() {
        use super::{MAX_IPC_JSON_LINE_BYTES, read_json_line};
        use std::io::Cursor;

        let payload = format!("{{\"message\":\"{}\"}}\n", "a".repeat(MAX_IPC_JSON_LINE_BYTES + 1));
        let mut reader = Cursor::new(payload.into_bytes());
        let parsed = read_json_line::<serde_json::Value>(&mut reader);
        assert!(parsed.is_err());
    }

    #[test]
    fn classifies_recoverable_accept_errors() {
        use super::is_recoverable_accept_error;

        assert!(is_recoverable_accept_error(&std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "interrupted"
        )));
        assert!(is_recoverable_accept_error(&std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "connection aborted"
        )));
        assert!(!is_recoverable_accept_error(&std::io::Error::other("fatal")));
    }
}
