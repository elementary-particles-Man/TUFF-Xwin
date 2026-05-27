use std::{
    env, fs,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::net::UnixListener;

use anyhow::{Context, Result, bail};
use vulkan_backend::{
    VulkanBackend, VulkanBackendConfig, VulkanBatchSubmission, VulkanWorkloadClass,
};
use waybroker_common::{
    DisplayCommand, DisplayEvent, FocusTarget, IpcEnvelope, MessageKind, OutputMode, ServiceBanner,
    ServiceEndpoint, ServiceRole, ServiceStream, SurfaceRegistrySnapshot, WaylandCommand,
    WaylandEvent, WaylandSelectionHandoff, WaylandSelectionState, WaylandSurfaceRole,
    WaylandSurfaceState, bind_service_socket, connect_service_socket, ensure_runtime_dir,
    now_unix_timestamp, read_json_line, send_json_line, session_artifact_path,
};

const DEFAULT_SESSION_INSTANCE_ID: &str = "default-single-session";

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(
        ServiceRole::Waylandd,
        "wayland endpoint, client lifecycle, clipboard core",
    );
    println!("{}", banner.render());

    let vulkan = if config.use_vulkan {
        let backend = VulkanBackend::new(VulkanBackendConfig::default());
        let caps = backend.initialize();
        println!(
            "service=waylandd op=vulkan_init event=success driver={} device={}",
            caps.driver_name, caps.device_name
        );
        Some(backend)
    } else {
        None
    };

    if config.serve_ipc {
        let mut registry = load_surface_registry(config.registry_path.as_ref())?;
        write_surface_registry_artifact(&registry, &config.session_instance_id)?;
        log_surface_registry(&registry);

        if config.print_registry {
            println!("{}", serde_json::to_string_pretty(&registry)?);
        }

        match query_output_inventory() {
            Ok(outputs) => println!("waylandd displayd_outputs={}", format_outputs(&outputs)),
            Err(err) if config.require_displayd => {
                return Err(err).context("failed to query output inventory before serving IPC");
            }
            Err(err) => println!("waylandd displayd_state=unreachable reason={err}"),
        }

        serve_ipc(&config, &mut registry, vulkan.as_ref()).await?;
        return Ok(());
    }

    match query_output_inventory() {
        Ok(outputs) => println!("waylandd displayd_outputs={}", format_outputs(&outputs)),
        Err(err) if config.require_displayd => return Err(err),
        Err(err) => println!("waylandd displayd_state=unreachable reason={err}"),
    }

    if config.print_registry {
        let registry = load_surface_registry(config.registry_path.as_ref())?;
        log_surface_registry(&registry);
        println!("{}", serde_json::to_string_pretty(&registry)?);
    }

    Ok(())
}

#[derive(Debug, Clone, Default)]
struct Config {
    require_displayd: bool,
    serve_ipc: bool,
    serve_once: bool,
    print_registry: bool,
    registry_path: Option<PathBuf>,
    bind_wayland_display: Option<String>,
    use_vulkan: bool,
    session_instance_id: String,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();
        config.session_instance_id = DEFAULT_SESSION_INSTANCE_ID.to_string();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--require-displayd" => config.require_displayd = true,
                "--serve-ipc" => config.serve_ipc = true,
                "--once" => config.serve_once = true,
                "--print-registry" => config.print_registry = true,
                "--vulkan" => config.use_vulkan = true,
                "--session-instance-id" => {
                    config.session_instance_id =
                        args.next().context("--session-instance-id requires an id")?;
                }
                "--registry" => {
                    let path = args.next().context("--registry requires a path")?;
                    config.registry_path = Some(PathBuf::from(path));
                }
                "--bind-wayland-display" => {
                    config.bind_wayland_display =
                        Some(args.next().context("--bind-wayland-display requires a socket name")?);
                }
                "--help" | "-h" => {
                    println!(
                        "usage: waylandd [--require-displayd] [--serve-ipc] [--once] [--print-registry] [--registry PATH] [--bind-wayland-display NAME] [--vulkan] [--session-instance-id ID]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

async fn serve_ipc(
    config: &Config,
    registry: &mut SurfaceRegistrySnapshot,
    vulkan: Option<&VulkanBackend>,
) -> Result<()> {
    let _wayland_display = match config.bind_wayland_display.as_deref() {
        Some(name) => Some(bind_wayland_display_socket(name)?),
        None => None,
    };

    let listener = bind_service_socket(ServiceRole::Waylandd)?;
    let _socket_guard = SocketGuard::new(listener.endpoint().clone());
    println!("service=waylandd op=listen event=socket_bound path={}", listener.endpoint());

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(err) => {
                println!("service=waylandd op=accept event=failed reason=\"{}\"", err);
                if err.kind() == std::io::ErrorKind::Interrupted
                    || err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::ConnectionAborted
                    || err.kind() == std::io::ErrorKind::ConnectionReset
                {
                    continue;
                }
                return Err(err).context("waylandd IPC accept failed");
            }
        };
        handle_client(stream, registry, vulkan, config, &config.session_instance_id).await?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("service=waylandd op=terminate event=finished served_requests={served}");
    Ok(())
}

async fn handle_client(
    mut stream: ServiceStream,
    registry: &mut SurfaceRegistrySnapshot,
    vulkan: Option<&VulkanBackend>,
    config: &Config,
    session_instance_id: &str,
) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let (response, registry_changed) = build_response(request, registry, vulkan, config).await;
    send_json_line(&mut stream, &response)?;
    if registry_changed {
        write_surface_registry_artifact(registry, session_instance_id)?;
    }
    Ok(())
}

async fn build_response(
    request: IpcEnvelope,
    registry: &mut SurfaceRegistrySnapshot,
    vulkan: Option<&VulkanBackend>,
    config: &Config,
) -> (IpcEnvelope, bool) {
    let source = request.source;
    let (response_kind, registry_changed) = match request.kind {
        MessageKind::WaylandCommand(command) if request.destination == ServiceRole::Waylandd => {
            let (event, changed) = handle_wayland_command(command, registry, vulkan, config).await;
            (MessageKind::WaylandEvent(event), changed)
        }
        MessageKind::WaylandCommand(_) => (
            MessageKind::WaylandEvent(WaylandEvent::Rejected {
                reason: format!(
                    "waylandd received message addressed to {}",
                    request.destination.as_str()
                ),
            }),
            false,
        ),
        other => (
            MessageKind::WaylandEvent(WaylandEvent::Rejected {
                reason: format!("waylandd does not handle {other:?}"),
            }),
            false,
        ),
    };

    (IpcEnvelope::new(ServiceRole::Waylandd, source, response_kind), registry_changed)
}

async fn handle_wayland_command(
    command: WaylandCommand,
    registry: &mut SurfaceRegistrySnapshot,
    vulkan: Option<&VulkanBackend>,
    config: &Config,
) -> (WaylandEvent, bool) {
    match command {
        WaylandCommand::GetSurfaceRegistry => {
            println!(
                "service=waylandd op=get_surface_registry event=success generation={} surfaces={} clipboard_owner={} primary_selection_owner={}",
                registry.generation,
                registry.surfaces.len(),
                format_owner(registry.selection.clipboard_owner.as_deref()),
                format_owner(registry.selection.primary_selection_owner.as_deref())
            );
            (WaylandEvent::SurfaceRegistry { snapshot: registry.clone() }, false)
        }
        WaylandCommand::ApplySelectionHandoff { handoff } => {
            if let Err(reason) = validate_selection_handoff(&handoff, registry) {
                return (WaylandEvent::Rejected { reason }, false);
            }

            if let Some(vulkan) = vulkan {
                let handle = vulkan.submit_batch(VulkanBatchSubmission {
                    workload: VulkanWorkloadClass::AuditScan,
                    payload_len: 1024,
                    surface_words: None,
                    timeout: Duration::from_millis(50),
                    requires_zeroize: true,
                    allows_gpu: true,
                });
                let result = vulkan.wait_for_completion(handle).await;
                println!(
                    "service=waylandd op=vulkan_audit event=completed workload={:?} path={:?}",
                    result.workload, result.path
                );
            }

            registry.selection = handoff.selection.clone();
            registry.generation = registry.generation.saturating_add(1);
            registry.unix_timestamp = now_unix_timestamp();

            println!(
                "service=waylandd op=selection_handoff event=applied generation={} focus={:?} clipboard_owner={} primary_selection_owner={}",
                registry.generation,
                handoff.focus,
                format_owner(registry.selection.clipboard_owner.as_deref()),
                format_owner(registry.selection.primary_selection_owner.as_deref())
            );
            (
                WaylandEvent::SelectionHandoffApplied { generation: registry.generation, handoff },
                true,
            )
        }
        WaylandCommand::CaptureOutput { output } => {
            handle_wayland_capture_request(&output, config, vulkan).await
        }
        WaylandCommand::StartRecord { output, fps } => {
            handle_wayland_record_request(&output, Some(fps), config, vulkan).await
        }
        WaylandCommand::StopRecord { output } => {
            handle_wayland_record_request(&output, None, config, vulkan).await
        }
    }
}

async fn handle_wayland_record_request(
    output: &str,
    fps: Option<u32>,
    _config: &Config,
    _vulkan: Option<&VulkanBackend>,
) -> (WaylandEvent, bool) {
    let op = if fps.is_some() { "start_record" } else { "stop_record" };
    println!("service=waylandd op={op} event=bridge_to_displayd output={output}");

    match request_record_from_displayd(output, fps) {
        Ok(event) => match event {
            DisplayEvent::RecordStarted { output, session_id } => {
                println!(
                    "service=waylandd op=start_record event=success output={output} session_id={session_id}"
                );
                (WaylandEvent::RecordStarted { output, session_id }, false)
            }
            DisplayEvent::RecordStopped { output, session_id, artifact_path } => {
                println!(
                    "service=waylandd op=stop_record event=success output={output} path={artifact_path}"
                );
                (WaylandEvent::RecordStopped { output, session_id, artifact_path }, false)
            }
            DisplayEvent::Rejected { reason } => {
                println!("service=waylandd op={op} event=rejected reason=\"{reason}\"");
                (WaylandEvent::Rejected { reason }, false)
            }
            other => {
                println!("service=waylandd op={op} event=failed reason=\"unexpected response: {other:?}\"");
                (WaylandEvent::Rejected { reason: "unexpected displayd response".into() }, false)
            }
        },
        Err(err) => {
            println!("service=waylandd op={op} event=failed reason=\"{err}\"");
            (WaylandEvent::Rejected { reason: err.to_string() }, false)
        }
    }
}

fn request_record_from_displayd(output: &str, fps: Option<u32>) -> Result<DisplayEvent> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let command = if let Some(fps) = fps {
        DisplayCommand::StartRecord { output: output.to_string(), fps }
    } else {
        DisplayCommand::StopRecord { output: output.to_string() }
    };

    let request = IpcEnvelope::new(ServiceRole::Waylandd, ServiceRole::Displayd, MessageKind::DisplayCommand(command));
    send_json_line(&mut stream, &request)?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let response: IpcEnvelope = read_json_line(&mut reader)?;

    match response.kind {
        MessageKind::DisplayEvent(event) => Ok(event),
        other => bail!("unexpected displayd response kind: {other:?}"),
    }
}

async fn handle_wayland_capture_request(
    output: &str,
    _config: &Config,
    _vulkan: Option<&VulkanBackend>,
) -> (WaylandEvent, bool) {
    println!("service=waylandd op=wayland_capture event=bridge_to_displayd output={output}");

    match request_capture_from_displayd(output) {
        Ok(event) => {
            if let DisplayEvent::OutputCaptured { output, width, height, format, artifact_path } =
                event
            {
                println!(
                    "service=waylandd op=wayland_capture event=success output={output} path={artifact_path}"
                );
                (
                    WaylandEvent::OutputCaptured { output, width, height, format, artifact_path },
                    false,
                )
            } else {
                (WaylandEvent::Rejected { reason: "unexpected displayd response".into() }, false)
            }
        }
        Err(err) => {
            println!("service=waylandd op=wayland_capture event=failed reason=\"{err}\"");
            (WaylandEvent::Rejected { reason: err.to_string() }, false)
        }
    }
}

fn request_capture_from_displayd(output: &str) -> Result<DisplayEvent> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let request = IpcEnvelope::new(
        ServiceRole::Waylandd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::CaptureOutput { output: output.to_string() }),
    );
    send_json_line(&mut stream, &request)?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let response: IpcEnvelope = read_json_line(&mut reader)?;

    if response.source != ServiceRole::Displayd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    match response.kind {
        MessageKind::DisplayEvent(event) => Ok(event),
        other => bail!("unexpected displayd response kind: {other:?}"),
    }
}

fn validate_selection_handoff(
    handoff: &WaylandSelectionHandoff,
    registry: &SurfaceRegistrySnapshot,
) -> std::result::Result<(), String> {
    let active_registry = active_surface_registry(registry);

    if let FocusTarget::Surface { id } = &handoff.focus {
        if !active_registry.contains_key(id.as_str()) {
            return Err(format!("focus target {id} is not active in waylandd registry"));
        }
    }

    for (label, owner) in [
        ("clipboard", handoff.selection.clipboard_owner.as_deref()),
        ("primary-selection", handoff.selection.primary_selection_owner.as_deref()),
    ] {
        if let Some(id) = owner {
            if !active_registry.contains_key(id) {
                return Err(format!("{label} owner {id} is not active in waylandd registry"));
            }
        }
    }

    validate_selection_metadata(
        "clipboard",
        handoff.selection.clipboard_owner.as_deref(),
        handoff.selection.clipboard_payload_id.as_deref(),
        handoff.selection.clipboard_source_serial,
    )?;
    validate_selection_metadata(
        "primary-selection",
        handoff.selection.primary_selection_owner.as_deref(),
        handoff.selection.primary_selection_payload_id.as_deref(),
        handoff.selection.primary_selection_source_serial,
    )?;

    Ok(())
}

fn validate_selection_metadata(
    label: &str,
    owner: Option<&str>,
    payload_id: Option<&str>,
    source_serial: Option<u64>,
) -> std::result::Result<(), String> {
    if owner.is_none() && (payload_id.is_some() || source_serial.is_some()) {
        return Err(format!("{label} metadata requires an active owner"));
    }

    if payload_id.is_some() ^ source_serial.is_some() {
        return Err(format!("{label} payload_id and source_serial must be paired"));
    }

    Ok(())
}

fn active_surface_registry(
    registry: &SurfaceRegistrySnapshot,
) -> std::collections::BTreeMap<&str, &WaylandSurfaceState> {
    registry
        .surfaces
        .iter()
        .filter(|surface| surface.mapped && surface.buffer_attached)
        .map(|surface| (surface.id.as_str(), surface))
        .collect()
}

fn query_output_inventory() -> Result<Vec<OutputMode>> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let request = IpcEnvelope::new(
        ServiceRole::Waylandd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::EnumerateOutputs),
    );
    send_json_line(&mut stream, &request)?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;

    if response.source != ServiceRole::Displayd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::Waylandd {
        bail!("unexpected response destination: {}", response.destination.as_str());
    }

    match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::OutputInventory { outputs }) => Ok(outputs),
        MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
            bail!("displayd rejected request: {reason}")
        }
        other => bail!("unexpected displayd response: {other:?}"),
    }
}

fn load_surface_registry(path: Option<&PathBuf>) -> Result<SurfaceRegistrySnapshot> {
    match path {
        Some(path) => {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read surface registry {}", path.display()))?;
            serde_json::from_str(&raw)
                .with_context(|| format!("failed to decode surface registry {}", path.display()))
        }
        None => Ok(mock_surface_registry()),
    }
}

fn mock_surface_registry() -> SurfaceRegistrySnapshot {
    SurfaceRegistrySnapshot {
        generation: 1,
        surfaces: vec![
            WaylandSurfaceState {
                id: "konsole-1".into(),
                app_id: "org.kde.konsole".into(),
                role: WaylandSurfaceRole::Toplevel,
                mapped: true,
                buffer_attached: true,
            },
            WaylandSurfaceState {
                id: "background-1".into(),
                app_id: "org.kde.plasmashell.wallpaper".into(),
                role: WaylandSurfaceRole::Background,
                mapped: true,
                buffer_attached: true,
            },
        ],
        selection: WaylandSelectionState {
            clipboard_owner: Some("konsole-1".into()),
            clipboard_payload_id: Some("konsole-clipboard-v1".into()),
            clipboard_source_serial: Some(11),
            primary_selection_owner: None,
            primary_selection_payload_id: None,
            primary_selection_source_serial: None,
        },
        unix_timestamp: now_unix_timestamp(),
    }
}

fn log_surface_registry(registry: &SurfaceRegistrySnapshot) {
    println!(
        "service=waylandd op=surface_registry event=loaded generation={} surfaces={} clipboard_owner={} primary_selection_owner={} timestamp={}",
        registry.generation,
        registry.surfaces.len(),
        format_owner(registry.selection.clipboard_owner.as_deref()),
        format_owner(registry.selection.primary_selection_owner.as_deref()),
        registry.unix_timestamp
    );
}

fn write_surface_registry_artifact(
    registry: &SurfaceRegistrySnapshot,
    session_instance_id: &str,
) -> Result<PathBuf> {
    let _ = ensure_runtime_dir()?;
    let path = session_artifact_path(session_instance_id, "surface-registry");
    fs::write(&path, serde_json::to_string_pretty(registry)?)
        .with_context(|| format!("failed to write runtime surface registry {}", path.display()))?;
    Ok(path)
}

fn format_owner(owner: Option<&str>) -> &str {
    owner.unwrap_or("none")
}

fn format_outputs(outputs: &[OutputMode]) -> String {
    let mut rendered = Vec::with_capacity(outputs.len());
    for output in outputs {
        rendered.push(format!(
            "{}:{}x{}@{}Hz",
            output.name, output.width, output.height, output.refresh_hz
        ));
    }

    rendered.join(",")
}

#[cfg(unix)]
fn bind_wayland_display_socket(name: &str) -> Result<WaylandDisplaySocket> {
    let mut candidates = vec![name.to_string()];
    if let Some((prefix, display_num)) = split_wayland_display_name(name) {
        candidates.push(format!("{prefix}{}", display_num + 1));
        candidates.push(format!("{prefix}{}", display_num + 2));
    }

    let mut last_err = None;
    for candidate in candidates {
        let path = resolve_wayland_display_path(&candidate)?;
        let lock_path = wayland_lock_path(&path);
        match bind_single_wayland_display_socket(&path, &lock_path) {
            Ok(socket) => return Ok(socket),
            Err(err) => {
                println!(
                    "service=waylandd op=wayland_display event=bind_failed name={} path={} reason={}",
                    candidate,
                    path.display(),
                    err
                );
                last_err = Some(err);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("failed to bind Wayland display socket")))
}

#[cfg(unix)]
fn bind_single_wayland_display_socket(
    path: &Path,
    lock_path: &Path,
) -> Result<WaylandDisplaySocket> {
    if path.exists() {
        bail!("Wayland display socket already exists: {}", path.display());
    }
    if lock_path.exists() {
        if socket_lock_is_stale(path, lock_path) {
            println!(
                "service=waylandd op=wayland_display event=stale_lock_removed path={} lock={}",
                path.display(),
                lock_path.display()
            );
            fs::remove_file(lock_path).with_context(|| {
                format!("failed to remove stale Wayland display lock {}", lock_path.display())
            })?;
        } else {
            bail!("Wayland display lock already exists: {}", lock_path.display());
        }
    }

    let _listener = UnixListener::bind(path)
        .with_context(|| format!("failed to bind Wayland display socket {}", path.display()))?;
    fs::write(lock_path, format!("{}\n", std::process::id()))
        .with_context(|| format!("failed to write Wayland display lock {}", lock_path.display()))?;

    let log_path = path.to_path_buf();
    thread::Builder::new()
        .name("wayland-display-listener".to_string())
        .spawn(move || {
            for stream in _listener.incoming() {
                match stream {
                    Ok(_stream) => {
                        println!(
                            "service=waylandd op=wayland_display event=client_connected path={} info=\"connection accepted by diagnostic listener (no data read)\"",
                            log_path.display()
                        );
                    }
                    Err(err) => {
                        println!(
                            "service=waylandd op=wayland_display event=accept_failed path={} reason={}",
                            log_path.display(),
                            err
                        );
                        break;
                    }
                }
            }
        })
        .context("failed to spawn Wayland display listener")?;

    println!(
        "service=waylandd op=wayland_display event=diagnostic_listener_bound path={} info=\"this is a minimal listener for connection observation only\"",
        path.display()
    );
    Ok(WaylandDisplaySocket { path: path.to_path_buf(), lock_path: lock_path.to_path_buf() })
}

#[cfg(not(unix))]
fn bind_wayland_display_socket(_name: &str) -> Result<WaylandDisplaySocket> {
    bail!("--bind-wayland-display is supported only on Unix platforms")
}

#[cfg(unix)]
fn resolve_wayland_display_path(name: &str) -> Result<PathBuf> {
    if name.is_empty() {
        bail!("Wayland display socket name must not be empty");
    }

    let path = Path::new(name);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        bail!("Wayland display socket name must be a basename or absolute path: {name}");
    }

    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("wayland-runtime"));
    Ok(runtime_dir.join(name))
}

#[cfg(unix)]
fn wayland_lock_path(path: &Path) -> PathBuf {
    let mut lock = path.as_os_str().to_owned();
    lock.push(".lock");
    PathBuf::from(lock)
}

#[cfg(unix)]
fn split_wayland_display_name(name: &str) -> Option<(&str, u32)> {
    let suffix = name.strip_prefix("wayland-")?;
    let display_num = suffix.parse::<u32>().ok()?;
    Some(("wayland-", display_num))
}

#[cfg(unix)]
fn socket_lock_is_stale(path: &Path, lock_path: &Path) -> bool {
    !path.exists() && lock_path.exists()
}

struct SocketGuard {
    endpoint: ServiceEndpoint,
}

impl SocketGuard {
    fn new(endpoint: ServiceEndpoint) -> Self {
        Self { endpoint }
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = self.endpoint.cleanup_stale();
    }
}

struct WaylandDisplaySocket {
    path: PathBuf,
    lock_path: PathBuf,
}

impl Drop for WaylandDisplaySocket {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(&self.lock_path);
    }
}

#[cfg(test)]
mod tests {
    use super::{handle_wayland_command, mock_surface_registry, socket_lock_is_stale};
    use std::fs;
    use waybroker_common::{
        FocusTarget, WaylandCommand, WaylandEvent, WaylandSelectionHandoff, WaylandSelectionState,
        WaylandSurfaceRole,
    };

    #[test]
    fn mock_registry_contains_mapped_focusable_surface() {
        let registry = mock_surface_registry();
        assert_eq!(registry.surfaces.len(), 2);
        assert!(registry.surfaces.iter().any(|surface| {
            surface.role == WaylandSurfaceRole::Toplevel
                && surface.mapped
                && surface.buffer_attached
        }));
    }

    #[test]
    fn applies_selection_handoff_to_active_surface() {
        let mut registry = mock_surface_registry();
        let (event, changed) =
            tokio::runtime::Runtime::new().unwrap().block_on(handle_wayland_command(
                WaylandCommand::ApplySelectionHandoff {
                    handoff: WaylandSelectionHandoff {
                        focus: FocusTarget::Surface { id: "konsole-1".into() },
                        selection: WaylandSelectionState {
                            clipboard_owner: Some("konsole-1".into()),
                            clipboard_payload_id: Some("konsole-clipboard-v2".into()),
                            clipboard_source_serial: Some(12),
                            primary_selection_owner: Some("konsole-1".into()),
                            primary_selection_payload_id: Some("konsole-primary-v1".into()),
                            primary_selection_source_serial: Some(13),
                        },
                    },
                },
                &mut registry,
                None,
                &super::Config::default(),
            ));

        assert!(changed);
        match event {
            WaylandEvent::SelectionHandoffApplied { generation, .. } => {
                assert_eq!(generation, 2);
            }
            other => panic!("expected handoff applied event, got {other:?}"),
        }
        assert_eq!(registry.selection.clipboard_owner.as_deref(), Some("konsole-1"));
        assert_eq!(
            registry.selection.clipboard_payload_id.as_deref(),
            Some("konsole-clipboard-v2")
        );
        assert_eq!(registry.selection.clipboard_source_serial, Some(12));
        assert_eq!(registry.selection.primary_selection_owner.as_deref(), Some("konsole-1"));
        assert_eq!(
            registry.selection.primary_selection_payload_id.as_deref(),
            Some("konsole-primary-v1")
        );
        assert_eq!(registry.selection.primary_selection_source_serial, Some(13));
    }

    #[test]
    fn stale_lock_without_socket_is_treated_as_stale() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("tuff-xwin-waylandd-test-{unique}"));
        let path = base.join("wayland-1");
        let lock_path = base.join("wayland-1.lock");
        fs::create_dir_all(&base).unwrap();
        fs::write(&lock_path, b"123\n").unwrap();

        assert!(socket_lock_is_stale(&path, &lock_path));

        let _ = fs::remove_file(&lock_path);
        let _ = fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn test_handle_wayland_capture_request_rejection() {
        // Since we can't easily mock displayd socket here without more boilerplate,
        // we at least test that it fails gracefully when displayd is not running.
        let config = super::Config::default();
        let (event, _) = super::handle_wayland_capture_request("eDP-1", &config, None).await;

        if let WaylandEvent::Rejected { reason } = event {
            assert!(
                reason.contains("No such file or directory")
                    || reason.contains("connection refused")
                    || reason.contains("failed to connect")
            );
        } else {
            panic!("Expected rejection when displayd is missing, got {:?}", event);
        }
    }
}
