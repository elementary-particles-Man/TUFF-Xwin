use std::{
    env, fmt, fs,
    io::{self, BufRead, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    ServiceRole,
    ipc::{DisplayCommand, IpcEnvelope, MessageKind},
    pixel_transport::PixelTransportPayload,
};

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

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match &self.inner {
            #[cfg(unix)]
            ServiceStreamInner::Unix(stream) => stream.set_read_timeout(timeout),
            ServiceStreamInner::Tcp(stream) => stream.set_read_timeout(timeout),
        }
    }

    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match &self.inner {
            #[cfg(unix)]
            ServiceStreamInner::Unix(stream) => stream.set_write_timeout(timeout),
            ServiceStreamInner::Tcp(stream) => stream.set_write_timeout(timeout),
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
pub const MAX_IPC_FRAME_METADATA_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_IPC_BINARY_ATTACHMENTS: usize = 4096;
pub const MAX_IPC_BINARY_ATTACHMENT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_IPC_BINARY_TOTAL_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_ARTIFACT_FILENAME_BYTES: usize = 128;

const IPC_FRAME_MARKER: u8 = 0x1e;
const IPC_FRAME_MAGIC: [u8; 4] = *b"XW3F";
const IPC_FRAME_VERSION: u8 = 1;
const IPC_FRAME_FIXED_HEADER_BYTES: usize = 24;

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

struct BoundedJsonBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl BoundedJsonBuffer {
    fn new(limit: usize) -> Self {
        Self { bytes: Vec::with_capacity(limit.min(4096)), limit }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedJsonBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let next = self
            .bytes
            .len()
            .checked_add(buf.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "IPC size overflow"))?;
        if next > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "IPC JSON serialization exceeds configured bound",
            ));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialize_json_bounded<T: Serialize>(message: &T, limit: usize) -> Result<Vec<u8>> {
    let mut buffer = BoundedJsonBuffer::new(limit);
    serde_json::to_writer(&mut buffer, message).context("failed to serialize IPC message")?;
    Ok(buffer.into_inner())
}

pub fn send_json_line<T: Serialize>(writer: &mut impl Write, message: &T) -> Result<()> {
    let encoded = serialize_json_bounded(message, MAX_IPC_JSON_LINE_BYTES)?;
    writer.write_all(&encoded).context("failed to write IPC message")?;
    writer.write_all(b"\n").context("failed to write IPC newline delimiter")?;
    writer.flush().context("failed to flush IPC message")?;
    Ok(())
}

fn read_bounded_json_line(reader: &mut impl BufRead) -> Result<Vec<u8>> {
    let mut line = Vec::with_capacity(1024);
    loop {
        let available = reader.fill_buf().context("failed to read IPC message")?;
        if available.is_empty() {
            if line.is_empty() {
                bail!("unexpected EOF while reading IPC message");
            }
            bail!("unexpected EOF before IPC newline delimiter");
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.unwrap_or(available.len());
        let next_len = line
            .len()
            .checked_add(take)
            .ok_or_else(|| anyhow::anyhow!("IPC message length overflow"))?;
        if next_len > MAX_IPC_JSON_LINE_BYTES {
            bail!("IPC message exceeds {} bytes", MAX_IPC_JSON_LINE_BYTES);
        }

        line.extend_from_slice(&available[..take]);
        reader.consume(take + if newline.is_some() { 1 } else { 0 });
        if newline.is_some() {
            return Ok(line);
        }
    }
}

pub fn read_json_line<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<T> {
    let line = read_bounded_json_line(reader)?;
    let line = std::str::from_utf8(&line).context("IPC message is not valid UTF-8")?;
    serde_json::from_str(line).context("failed to decode IPC message")
}

fn display_command_pixel_payloads(command: &DisplayCommand) -> Option<&[PixelTransportPayload]> {
    match command {
        DisplayCommand::CommitScene { pixel_payloads, .. }
        | DisplayCommand::ReconcileScene { pixel_payloads, .. } => Some(pixel_payloads),
        _ => None,
    }
}

fn envelope_pixel_payloads_mut(
    envelope: &mut IpcEnvelope,
) -> Option<&mut Vec<PixelTransportPayload>> {
    match &mut envelope.kind {
        MessageKind::DisplayCommand(DisplayCommand::CommitScene { pixel_payloads, .. })
        | MessageKind::DisplayCommand(DisplayCommand::ReconcileScene { pixel_payloads, .. }) => {
            Some(pixel_payloads)
        }
        _ => None,
    }
}

fn metadata_only_payload(payload: &PixelTransportPayload) -> PixelTransportPayload {
    PixelTransportPayload {
        handle: payload.handle.clone(),
        pixels: Vec::new(),
        width: payload.width,
        height: payload.height,
        stride: payload.stride,
        format: payload.format,
    }
}

fn display_command_without_pixel_bytes(command: &DisplayCommand) -> DisplayCommand {
    match command {
        DisplayCommand::CommitScene {
            target,
            focus,
            selection,
            surfaces,
            pixel_payloads,
            scene_epoch,
            scene_generation,
        } => DisplayCommand::CommitScene {
            target: target.clone(),
            focus: focus.clone(),
            selection: selection.clone(),
            surfaces: surfaces.clone(),
            pixel_payloads: pixel_payloads.iter().map(metadata_only_payload).collect(),
            scene_epoch: *scene_epoch,
            scene_generation: *scene_generation,
        },
        DisplayCommand::ReconcileScene {
            epoch,
            scene_epoch,
            scene_generation,
            target,
            focus,
            selection,
            surfaces,
            pixel_payloads,
        } => DisplayCommand::ReconcileScene {
            epoch: *epoch,
            scene_epoch: *scene_epoch,
            scene_generation: *scene_generation,
            target: target.clone(),
            focus: focus.clone(),
            selection: selection.clone(),
            surfaces: surfaces.clone(),
            pixel_payloads: pixel_payloads.iter().map(metadata_only_payload).collect(),
        },
        other => other.clone(),
    }
}

fn expected_pixel_payload_bytes(payload: &PixelTransportPayload) -> Result<usize> {
    if payload.width == 0 || payload.height == 0 {
        bail!("framed PixelTransport payload dimensions must be non-zero");
    }
    let min_stride = payload
        .width
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("framed PixelTransport stride overflow"))?;
    if payload.stride < min_stride {
        bail!("framed PixelTransport stride is smaller than 32-bit pixel width");
    }
    let expected = (payload.stride as usize)
        .checked_mul(payload.height as usize)
        .ok_or_else(|| anyhow::anyhow!("framed PixelTransport byte length overflow"))?;
    if expected > MAX_IPC_BINARY_ATTACHMENT_BYTES {
        bail!("framed PixelTransport attachment exceeds {} bytes", MAX_IPC_BINARY_ATTACHMENT_BYTES);
    }
    Ok(expected)
}

fn write_framed_scene_envelope(
    writer: &mut impl Write,
    metadata: &IpcEnvelope,
    payloads: &[PixelTransportPayload],
) -> Result<()> {
    if payloads.len() > MAX_IPC_BINARY_ATTACHMENTS {
        bail!("IPC binary attachment count exceeds {}", MAX_IPC_BINARY_ATTACHMENTS);
    }

    let mut attachment_lengths = Vec::with_capacity(payloads.len());
    let mut total_bytes = 0usize;
    for payload in payloads {
        let expected = expected_pixel_payload_bytes(payload)?;
        if payload.pixels.len() != expected {
            bail!(
                "framed PixelTransport payload byte length mismatch for {}: {} != {}",
                payload.handle.surface_id,
                payload.pixels.len(),
                expected
            );
        }
        total_bytes = total_bytes
            .checked_add(expected)
            .ok_or_else(|| anyhow::anyhow!("IPC binary attachment byte accounting overflow"))?;
        if total_bytes > MAX_IPC_BINARY_TOTAL_BYTES {
            bail!("IPC binary attachment bytes exceed {}", MAX_IPC_BINARY_TOTAL_BYTES);
        }
        attachment_lengths.push(expected as u64);
    }

    let metadata_bytes = serialize_json_bounded(metadata, MAX_IPC_FRAME_METADATA_BYTES)?;
    let metadata_len = u32::try_from(metadata_bytes.len())
        .map_err(|_| anyhow::anyhow!("IPC frame metadata length does not fit u32"))?;
    let attachment_count = u32::try_from(attachment_lengths.len())
        .map_err(|_| anyhow::anyhow!("IPC attachment count does not fit u32"))?;

    let mut header = [0u8; IPC_FRAME_FIXED_HEADER_BYTES];
    header[0..4].copy_from_slice(&IPC_FRAME_MAGIC);
    header[4] = IPC_FRAME_VERSION;
    header[5] = 0;
    header[6..8].copy_from_slice(&0u16.to_le_bytes());
    header[8..12].copy_from_slice(&metadata_len.to_le_bytes());
    header[12..16].copy_from_slice(&attachment_count.to_le_bytes());
    header[16..24].copy_from_slice(&(total_bytes as u64).to_le_bytes());

    writer.write_all(&[IPC_FRAME_MARKER]).context("failed to write IPC frame marker")?;
    writer.write_all(&header).context("failed to write IPC frame header")?;
    for length in &attachment_lengths {
        writer.write_all(&length.to_le_bytes()).context("failed to write IPC attachment length")?;
    }
    writer.write_all(&metadata_bytes).context("failed to write IPC frame metadata")?;
    for payload in payloads {
        writer.write_all(&payload.pixels).context("failed to write IPC binary attachment")?;
    }
    writer.flush().context("failed to flush IPC frame")?;
    Ok(())
}

pub fn send_ipc_display_command(
    writer: &mut impl Write,
    source: ServiceRole,
    destination: ServiceRole,
    command: &DisplayCommand,
) -> Result<()> {
    let Some(payloads) = display_command_pixel_payloads(command) else {
        let envelope =
            IpcEnvelope::new(source, destination, MessageKind::DisplayCommand(command.clone()));
        return send_json_line(writer, &envelope);
    };

    let metadata = IpcEnvelope::new(
        source,
        destination,
        MessageKind::DisplayCommand(display_command_without_pixel_bytes(command)),
    );
    write_framed_scene_envelope(writer, &metadata, payloads)
}

pub fn send_ipc_envelope(writer: &mut impl Write, message: &IpcEnvelope) -> Result<()> {
    if let MessageKind::DisplayCommand(command) = &message.kind {
        if let Some(payloads) = display_command_pixel_payloads(command) {
            let metadata = IpcEnvelope::new(
                message.source,
                message.destination,
                MessageKind::DisplayCommand(display_command_without_pixel_bytes(command)),
            );
            return write_framed_scene_envelope(writer, &metadata, payloads);
        }
    }
    send_json_line(writer, message)
}

fn read_frame_header(reader: &mut impl BufRead) -> Result<(usize, Vec<usize>, usize)> {
    let mut header = [0u8; IPC_FRAME_FIXED_HEADER_BYTES];
    reader.read_exact(&mut header).context("unexpected EOF while reading IPC frame header")?;
    if &header[0..4] != IPC_FRAME_MAGIC.as_slice() {
        bail!("invalid IPC frame magic");
    }
    if header[4] != IPC_FRAME_VERSION {
        bail!("unsupported IPC frame version {}", header[4]);
    }
    if header[5] != 0 || header[6] != 0 || header[7] != 0 {
        bail!("unsupported IPC frame flags");
    }

    let metadata_len = u32::from_le_bytes(header[8..12].try_into().expect("fixed header")) as usize;
    let attachment_count =
        u32::from_le_bytes(header[12..16].try_into().expect("fixed header")) as usize;
    let total_bytes = u64::from_le_bytes(header[16..24].try_into().expect("fixed header"));

    if metadata_len > MAX_IPC_FRAME_METADATA_BYTES {
        bail!("IPC frame metadata exceeds {} bytes", MAX_IPC_FRAME_METADATA_BYTES);
    }
    if attachment_count > MAX_IPC_BINARY_ATTACHMENTS {
        bail!("IPC binary attachment count exceeds {}", MAX_IPC_BINARY_ATTACHMENTS);
    }
    if total_bytes > MAX_IPC_BINARY_TOTAL_BYTES as u64 {
        bail!("IPC binary attachment bytes exceed {}", MAX_IPC_BINARY_TOTAL_BYTES);
    }

    let mut lengths = Vec::with_capacity(attachment_count);
    let mut sum = 0usize;
    for _ in 0..attachment_count {
        let mut encoded = [0u8; 8];
        reader
            .read_exact(&mut encoded)
            .context("unexpected EOF while reading IPC attachment length")?;
        let length_u64 = u64::from_le_bytes(encoded);
        if length_u64 > MAX_IPC_BINARY_ATTACHMENT_BYTES as u64 {
            bail!("IPC binary attachment exceeds {} bytes", MAX_IPC_BINARY_ATTACHMENT_BYTES);
        }
        let length = usize::try_from(length_u64)
            .map_err(|_| anyhow::anyhow!("IPC binary attachment length does not fit usize"))?;
        sum = sum
            .checked_add(length)
            .ok_or_else(|| anyhow::anyhow!("IPC binary attachment length overflow"))?;
        if sum > MAX_IPC_BINARY_TOTAL_BYTES {
            bail!("IPC binary attachment bytes exceed {}", MAX_IPC_BINARY_TOTAL_BYTES);
        }
        lengths.push(length);
    }
    if sum as u64 != total_bytes {
        bail!("IPC frame attachment total does not match declared total");
    }

    Ok((metadata_len, lengths, sum))
}

fn read_framed_ipc_envelope(reader: &mut impl BufRead) -> Result<IpcEnvelope> {
    let (metadata_len, attachment_lengths, _total_bytes) = read_frame_header(reader)?;

    let mut metadata = vec![0u8; metadata_len];
    reader.read_exact(&mut metadata).context("unexpected EOF while reading IPC frame metadata")?;
    let mut envelope: IpcEnvelope =
        serde_json::from_slice(&metadata).context("failed to decode IPC frame metadata")?;

    let payloads = envelope_pixel_payloads_mut(&mut envelope)
        .ok_or_else(|| anyhow::anyhow!("IPC binary frame is not a scene command"))?;
    if payloads.len() != attachment_lengths.len() {
        bail!(
            "IPC frame attachment count {} does not match scene payload count {}",
            attachment_lengths.len(),
            payloads.len()
        );
    }

    for (payload, declared_len) in payloads.iter_mut().zip(attachment_lengths) {
        if !payload.pixels.is_empty() {
            bail!("IPC frame metadata must not contain inline pixel bytes");
        }
        let expected = expected_pixel_payload_bytes(payload)?;
        if declared_len != expected {
            bail!(
                "IPC frame attachment length mismatch for {}: {} != {}",
                payload.handle.surface_id,
                declared_len,
                expected
            );
        }
        let mut pixels = vec![0u8; declared_len];
        reader.read_exact(&mut pixels).with_context(|| {
            format!("unexpected EOF while reading IPC attachment for {}", payload.handle.surface_id)
        })?;
        payload.pixels = pixels;
    }

    Ok(envelope)
}

pub fn read_ipc_envelope(reader: &mut impl BufRead) -> Result<IpcEnvelope> {
    let first = reader
        .fill_buf()
        .context("failed to inspect IPC message prefix")?
        .first()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("unexpected EOF while reading IPC message"))?;
    if first != IPC_FRAME_MARKER {
        return read_json_line(reader);
    }

    reader.consume(1);
    read_framed_ipc_envelope(reader)
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

    fn framed_scene_envelope(
        scene_generation: u64,
        payload_specs: &[(u64, &str, u32, u32)],
    ) -> crate::IpcEnvelope {
        let mut surfaces = Vec::new();
        let mut pixel_payloads = Vec::new();
        for (client_id, surface_id, width, height) in payload_specs {
            let stride = width.checked_mul(4).unwrap();
            let handle = crate::PixelTransportHandle {
                client_id: *client_id,
                surface_id: (*surface_id).to_string(),
                buffer_generation: scene_generation,
                scene_generation,
            };
            surfaces.push(crate::SurfaceSnapshot {
                id: (*surface_id).to_string(),
                app_id: "transport.test".into(),
                placement: crate::SurfacePlacement {
                    width: *width,
                    height: *height,
                    visible: true,
                    ..Default::default()
                },
                buffer_generation: scene_generation,
                pixel_transport: Some(handle.clone()),
                ..Default::default()
            });
            pixel_payloads.push(crate::PixelTransportPayload {
                handle,
                pixels: vec![0x5a; stride as usize * *height as usize],
                width: *width,
                height: *height,
                stride,
                format: 1,
            });
        }
        crate::IpcEnvelope::new(
            crate::ServiceRole::Waylandd,
            crate::ServiceRole::Compd,
            crate::MessageKind::DisplayCommand(crate::DisplayCommand::CommitScene {
                target: crate::CommitTarget::Output { name: "eDP-1".into() },
                focus: crate::FocusTarget::None,
                selection: crate::WaylandSelectionState::default(),
                surfaces,
                pixel_payloads,
                scene_epoch: 9,
                scene_generation,
            }),
        )
    }

    #[test]
    fn completion_transport_large_pixel_payload_round_trips_as_bounded_binary_frame() {
        use super::{IPC_FRAME_MARKER, read_ipc_envelope, send_ipc_envelope};
        use std::io::{BufReader, Cursor};

        let envelope = framed_scene_envelope(7, &[(1, "surface-a", 256, 256)]);
        let mut wire = Vec::new();
        send_ipc_envelope(&mut wire, &envelope).unwrap();

        assert_eq!(wire.first().copied(), Some(IPC_FRAME_MARKER));
        assert!(wire.len() < 400_000, "binary frame must avoid JSON byte-array expansion");

        let mut reader = BufReader::new(Cursor::new(wire));
        let decoded = read_ipc_envelope(&mut reader).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn completion_transport_multiple_attachments_preserve_payload_order_and_identity() {
        use super::{read_ipc_envelope, send_ipc_envelope};
        use std::io::{BufReader, Cursor};

        let envelope =
            framed_scene_envelope(11, &[(2, "surface-b", 64, 32), (1, "surface-a", 32, 16)]);
        let mut wire = Vec::new();
        send_ipc_envelope(&mut wire, &envelope).unwrap();

        let mut reader = BufReader::new(Cursor::new(wire));
        let decoded = read_ipc_envelope(&mut reader).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn completion_transport_truncated_binary_attachment_fails_closed() {
        use super::{read_ipc_envelope, send_ipc_envelope};
        use std::io::{BufReader, Cursor};

        let envelope = framed_scene_envelope(12, &[(1, "surface-a", 128, 128)]);
        let mut wire = Vec::new();
        send_ipc_envelope(&mut wire, &envelope).unwrap();
        wire.truncate(wire.len() - 17);

        let mut reader = BufReader::new(Cursor::new(wire));
        assert!(read_ipc_envelope(&mut reader).is_err());
    }

    #[test]
    fn completion_transport_rejects_malformed_payload_before_writing_frame() {
        use super::send_ipc_envelope;

        let mut envelope = framed_scene_envelope(13, &[(1, "surface-a", 8, 8)]);
        if let crate::MessageKind::DisplayCommand(crate::DisplayCommand::CommitScene {
            pixel_payloads,
            ..
        }) = &mut envelope.kind
        {
            pixel_payloads[0].pixels.pop();
        } else {
            panic!("expected scene command");
        }

        let mut wire = Vec::new();
        assert!(send_ipc_envelope(&mut wire, &envelope).is_err());
        assert!(wire.is_empty(), "validation must finish before writing a frame prefix");
    }

    #[test]
    fn completion_transport_rejects_oversized_declared_frame_before_payload_allocation() {
        use super::{
            IPC_FRAME_FIXED_HEADER_BYTES, IPC_FRAME_MAGIC, IPC_FRAME_MARKER, IPC_FRAME_VERSION,
            MAX_IPC_BINARY_TOTAL_BYTES, read_ipc_envelope,
        };
        use std::io::{BufReader, Cursor};

        let mut wire = vec![IPC_FRAME_MARKER];
        let mut header = [0u8; IPC_FRAME_FIXED_HEADER_BYTES];
        header[0..4].copy_from_slice(&IPC_FRAME_MAGIC);
        header[4] = IPC_FRAME_VERSION;
        header[8..12].copy_from_slice(&0u32.to_le_bytes());
        header[12..16].copy_from_slice(&0u32.to_le_bytes());
        header[16..24].copy_from_slice(&((MAX_IPC_BINARY_TOTAL_BYTES as u64) + 1).to_le_bytes());
        wire.extend_from_slice(&header);

        let mut reader = BufReader::new(Cursor::new(wire));
        let error = read_ipc_envelope(&mut reader).unwrap_err();
        assert!(error.to_string().contains("attachment bytes exceed"));
    }

    #[test]
    fn completion_transport_legacy_small_json_and_binary_frames_can_be_mixed() {
        use super::{read_ipc_envelope, send_ipc_envelope};
        use std::io::{BufReader, Cursor};

        let small = crate::IpcEnvelope::new(
            crate::ServiceRole::Waylandd,
            crate::ServiceRole::Compd,
            crate::MessageKind::DisplayCommand(crate::DisplayCommand::GetLiveness),
        );
        let large = framed_scene_envelope(14, &[(1, "surface-a", 128, 128)]);

        let mut wire = Vec::new();
        send_ipc_envelope(&mut wire, &small).unwrap();
        send_ipc_envelope(&mut wire, &large).unwrap();
        send_ipc_envelope(&mut wire, &small).unwrap();

        let mut reader = BufReader::new(Cursor::new(wire));
        assert_eq!(read_ipc_envelope(&mut reader).unwrap(), small);
        assert_eq!(read_ipc_envelope(&mut reader).unwrap(), large);
        assert_eq!(read_ipc_envelope(&mut reader).unwrap(), small);
    }

    #[test]
    fn completion_transport_outbound_json_limit_is_checked_before_write() {
        use super::{MAX_IPC_JSON_LINE_BYTES, send_json_line};

        let value = serde_json::json!({
            "message": "x".repeat(MAX_IPC_JSON_LINE_BYTES + 1)
        });
        let mut wire = Vec::new();
        assert!(send_json_line(&mut wire, &value).is_err());
        assert!(wire.is_empty());
    }

    #[test]
    fn completion_transport_10k_small_message_storm_remains_sequential_and_bounded() {
        use super::{read_ipc_envelope, send_ipc_envelope};
        use std::io::{BufReader, Cursor};

        let mut wire = Vec::new();
        for _ in 0..10_000 {
            let envelope = crate::IpcEnvelope::new(
                crate::ServiceRole::Waylandd,
                crate::ServiceRole::Compd,
                crate::MessageKind::DisplayCommand(crate::DisplayCommand::GetLiveness),
            );
            send_ipc_envelope(&mut wire, &envelope).unwrap();
        }

        let mut reader = BufReader::new(Cursor::new(wire));
        for _ in 0..10_000 {
            let envelope = read_ipc_envelope(&mut reader).unwrap();
            assert!(matches!(
                envelope.kind,
                crate::MessageKind::DisplayCommand(crate::DisplayCommand::GetLiveness)
            ));
        }
    }

    #[test]
    fn completion_transport_metadata_only_large_scene_uses_same_bounded_frame_protocol() {
        use super::{IPC_FRAME_MARKER, read_ipc_envelope, send_ipc_envelope};
        use std::io::{BufReader, Cursor};

        let surfaces = (0..4096)
            .map(|index| crate::SurfaceSnapshot {
                id: format!("surface-{index}"),
                app_id: "transport.metadata".into(),
                placement: crate::SurfacePlacement {
                    width: 1,
                    height: 1,
                    visible: true,
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect();
        let envelope = crate::IpcEnvelope::new(
            crate::ServiceRole::Compd,
            crate::ServiceRole::Displayd,
            crate::MessageKind::DisplayCommand(crate::DisplayCommand::ReconcileScene {
                epoch: 9,
                scene_epoch: 8,
                scene_generation: 77,
                target: crate::CommitTarget::Output { name: "eDP-1".into() },
                focus: crate::FocusTarget::None,
                selection: crate::WaylandSelectionState::default(),
                surfaces,
                pixel_payloads: Vec::new(),
            }),
        );

        let mut wire = Vec::new();
        send_ipc_envelope(&mut wire, &envelope).unwrap();
        assert_eq!(wire.first().copied(), Some(IPC_FRAME_MARKER));

        let mut reader = BufReader::new(Cursor::new(wire));
        assert_eq!(read_ipc_envelope(&mut reader).unwrap(), envelope);
    }
}
