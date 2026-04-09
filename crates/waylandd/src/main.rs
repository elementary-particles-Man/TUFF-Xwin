use std::{env, fs, io::BufReader, path::PathBuf, time::Duration};

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
                "--help" | "-h" => {
                    println!(
                        "usage: waylandd [--require-displayd] [--serve-ipc] [--once] [--print-registry] [--registry PATH] [--vulkan] [--session-instance-id ID]"
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
    let listener = bind_service_socket(ServiceRole::Waylandd)?;
    let _socket_guard = SocketGuard::new(listener.endpoint().clone());
    println!("service=waylandd op=listen event=socket_bound path={}", listener.endpoint());

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = stream?;
        handle_client(stream, registry, vulkan, &config.session_instance_id).await?;
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
    session_instance_id: &str,
) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let (response, registry_changed) = build_response(request, registry, vulkan).await;
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
) -> (IpcEnvelope, bool) {
    let source = request.source;
    let (response_kind, registry_changed) = match request.kind {
        MessageKind::WaylandCommand(command) if request.destination == ServiceRole::Waylandd => {
            let (event, changed) = handle_wayland_command(command, registry, vulkan).await;
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

#[cfg(test)]
mod tests {
    use super::{handle_wayland_command, mock_surface_registry};
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
}
