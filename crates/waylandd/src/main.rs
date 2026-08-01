use std::{
    env, fs,
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::{
    io::AsRawFd,
    net::{UnixListener, UnixStream},
};

use anyhow::{Context, Result, bail};
use byteorder::ByteOrder;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{
    LazyLock, Mutex,
    mpsc::{Receiver, SyncSender, sync_channel},
};
static GLOBAL_REGISTRY: Mutex<Option<SurfaceRegistrySnapshot>> = Mutex::new(None);
static CANONICAL_SCENE: LazyLock<Mutex<CanonicalSceneState>> =
    LazyLock::new(|| Mutex::new(CanonicalSceneState::default()));
static PIXEL_TRANSPORT: LazyLock<Mutex<PixelTransportStore>> =
    LazyLock::new(|| Mutex::new(PixelTransportStore::default()));
static TOPOLOGY_RECEIVER: LazyLock<Mutex<Option<Receiver<TopologyInput>>>> =
    LazyLock::new(|| Mutex::new(None));
static CLIENT_COMMANDS: LazyLock<Mutex<BTreeMap<u64, SyncSender<ClientCommand>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static ACTIVE_TOPOLOGY_CONSUMERS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum ClientCommand {
    InitialTopology { epoch: u64, sequence: u64, outputs: Vec<OutputTopologyEntry> },
    AddGlobal { epoch: u64, sequence: u64, output: OutputTopologyEntry },
    RemoveGlobal { epoch: u64, sequence: u64, output_id: String, output_generation: u64 },
    ReconfigureOutput { epoch: u64, sequence: u64, output: OutputTopologyEntry },
    RecalculateMembership { epoch: u64, sequence: u64 },
    TopologyReset { epoch: u64, sequence: u64 },
    Disconnect { epoch: u64, sequence: u64 },
}

const TOPOLOGY_DELTA_BUFFER_LIMIT: usize = 32;
const TOPOLOGY_QUEUE_CAPACITY: usize = 64;
const SNAPSHOT_RETRY_LIMIT: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResyncState {
    Streaming,
    SnapshotPending,
    SnapshotApplying,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResyncReason {
    Reset,
    SnapshotRequired,
    SequenceGap,
    StreamOverflow,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum TopologyInput {
    Event(DisplayEvent),
    Overflow,
}
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);
use vulkan_backend::{
    VulkanBackend, VulkanBackendConfig, VulkanBatchSubmission, VulkanWorkloadClass,
};
use waybroker_common::{
    CommitTarget, DisplayCommand, DisplayEvent, FocusTarget, ImeBridgeMode, ImeCommand, ImeEvent,
    ImeStatus, IpcEnvelope, MessageKind, OutputMode, OutputTopologyDelta, OutputTopologyEntry,
    OutputTopologyTransition, PixelTransportHandle, PixelTransportPayload, PixelTransportStore,
    ServiceBanner, ServiceEndpoint, ServiceRole, ServiceStream, SurfacePlacement,
    SurfaceRegistrySnapshot, SurfaceSnapshot, WaylandCommand, WaylandEvent,
    WaylandSelectionHandoff, WaylandSelectionState, WaylandSurfaceRole, WaylandSurfaceState,
    accel::global_accel_policy, bind_service_socket, connect_service_socket, ensure_runtime_dir,
    now_unix_timestamp, read_json_line, send_json_line, session_artifact_path,
};

fn run_wire_headless_test() -> Result<()> {
    use byteorder::{ByteOrder, LittleEndian};
    use wayland_wire::{WaylandMessage, WaylandObjectId, WaylandOpcode, core::HeadlessWireCore};

    println!("service=waylandd op=wire_headless_test event=begin");
    let mut core = HeadlessWireCore::default();

    // 1. Get Registry
    let mut payload = vec![0u8; 4];
    LittleEndian::write_u32(&mut payload[0..4], 2);
    core.dispatch(WaylandMessage::new(WaylandObjectId::DISPLAY, WaylandOpcode(1), payload))
        .map_err(|e| anyhow::anyhow!(e))?;

    // 2. Bind wl_compositor (assume name 1)
    // Signature: name (u32), interface (string), version (u32), id (new_id)
    let mut bind_payload = vec![0u8; 4 + 4 + 16 + 4 + 4];
    LittleEndian::write_u32(&mut bind_payload[0..4], 1); // name
    LittleEndian::write_u32(&mut bind_payload[4..8], 14); // interface len "wl_compositor\0"
    bind_payload[8..22].copy_from_slice(b"wl_compositor\0");
    LittleEndian::write_u32(&mut bind_payload[24..28], 4); // version
    LittleEndian::write_u32(&mut bind_payload[28..32], 3); // new_id
    core.dispatch(WaylandMessage::new(WaylandObjectId(2), WaylandOpcode(0), bind_payload))
        .map_err(|e| anyhow::anyhow!(e))?;

    // 3. Create Surface
    let mut surf_payload = vec![0u8; 4];
    LittleEndian::write_u32(&mut surf_payload[0..4], 4);
    core.dispatch(WaylandMessage::new(WaylandObjectId(3), WaylandOpcode(0), surf_payload))
        .map_err(|e| anyhow::anyhow!(e))?;

    // 4. Commit
    core.dispatch(WaylandMessage::new(WaylandObjectId(4), WaylandOpcode(6), vec![]))
        .map_err(|e| anyhow::anyhow!(e))?;

    while let Some(ev) = core.pop_event() {
        println!(
            "service=waylandd op=wire_headless_test event=pop_event object_id={} opcode={} size={}",
            ev.header.object_id.0, ev.header.opcode.0, ev.header.size
        );
    }

    println!(
        "service=waylandd op=wire_headless_test event=success surface_count={}",
        core.surfaces.surfaces.len()
    );
    Ok(())
}

const DEFAULT_SESSION_INSTANCE_ID: &str = "default-single-session";

#[derive(Debug, Clone)]
struct ImeRuntimeState {
    bridge_mode: ImeBridgeMode,
    focused_surface_id: Option<String>,
    preedit_active: bool,
    commit_count: u64,
    cursor_rect: Option<waybroker_common::Rect>,
    surrounding_text: Option<String>,
    surrounding_cursor: u32,
    content_purpose: u32,
}

impl Default for ImeRuntimeState {
    fn default() -> Self {
        Self {
            bridge_mode: ImeBridgeMode::Disabled,
            focused_surface_id: None,
            preedit_active: false,
            commit_count: 0,
            cursor_rect: None,
            surrounding_text: None,
            surrounding_cursor: 0,
            content_purpose: 0,
        }
    }
}

impl ImeRuntimeState {
    fn status(&self) -> ImeStatus {
        ImeStatus {
            bridge_mode: self.bridge_mode,
            focused_surface_id: self.focused_surface_id.clone(),
            preedit_active: self.preedit_active,
            commit_count: self.commit_count,
            cursor_rect: self.cursor_rect,
            surrounding_text: self.surrounding_text.clone(),
            surrounding_cursor: self.surrounding_cursor,
            content_purpose: self.content_purpose,
        }
    }
}

trait ImeBackend {
    fn set_cursor_rect(&mut self, rect: waybroker_common::Rect);
    fn set_surrounding_text(&mut self, text: &str, cursor: u32, anchor: u32);
    fn set_content_type(&mut self, hint: u32, purpose: u32);
    fn clear_focus(&mut self);
    fn focus_surface(&mut self, surface_id: &str);
}

struct FakeImeBackend;

impl ImeBackend for FakeImeBackend {
    fn set_cursor_rect(&mut self, _rect: waybroker_common::Rect) {}
    fn set_surrounding_text(&mut self, _text: &str, _cursor: u32, _anchor: u32) {}
    fn set_content_type(&mut self, _hint: u32, _purpose: u32) {}
    fn clear_focus(&mut self) {}
    fn focus_surface(&mut self, _surface_id: &str) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DnDStatus {
    Inactive,
    Dragging,
    Dropped,
}

#[derive(Debug, Clone)]
struct DnDState {
    status: DnDStatus,
    source_id: Option<String>,
    origin_surface_id: Option<String>,
    target_surface_id: Option<String>,
    x: f64,
    y: f64,
    mime_types: Vec<String>,
}

impl Default for DnDState {
    fn default() -> Self {
        Self {
            status: DnDStatus::Inactive,
            source_id: None,
            origin_surface_id: None,
            target_surface_id: None,
            x: 0.0,
            y: 0.0,
            mime_types: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DataPayloadRegistry {
    dnd: DnDState,
    fake_buffers: std::collections::HashMap<(String, String), Vec<u8>>, // (source_id, mime_type) -> data
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(
        ServiceRole::Waylandd,
        "wayland endpoint, client lifecycle, clipboard core",
    );
    println!("{}", banner.render());

    if let Some(check_socket) = config.check_readiness.as_ref() {
        let socket_path = resolve_wayland_display_path(check_socket)?;
        match run_readiness_check(&socket_path) {
            Ok(_) => {
                println!("service=waylandd op=readiness_check event=success");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("service=waylandd op=readiness_check event=failure reason={:?}", e);
                std::process::exit(1);
            }
        }
    }

    if config.smoke_check {
        match run_smoke_check() {
            Ok(_) => {
                println!("service=waylandd op=smoke_check event=success");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("service=waylandd op=smoke_check event=failure reason={:?}", e);
                std::process::exit(1);
            }
        }
    }

    let vulkan = if config.use_vulkan && global_accel_policy().prefers_vulkan() {
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

    if let Some(socket_path) = config.wire_test_socket.as_ref() {
        println!("service=waylandd op=wire_test_socket event=begin path={}", socket_path.display());
        println!(
            "service=waylandd info=\"this is an isolated test socket, NOT for production use\""
        );
        let mut server =
            wayland_wire::server::WireServer::new(wayland_wire::server::WireServerConfig {
                socket_path: socket_path.clone(),
            })?;
        server.run_once()?;
        return Ok(());
    }

    if config.wire_headless_test {
        run_wire_headless_test()?;
        return Ok(());
    }

    if config.serve_ipc {
        let mut registry = load_surface_registry(config.registry_path.as_ref())?;
        {
            let mut global = GLOBAL_REGISTRY.lock().unwrap();
            *global = Some(registry.clone());
        }
        let mut ime_state = ImeRuntimeState::default();
        let mut data_payloads = DataPayloadRegistry::default();
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

        let mut ime_backend = FakeImeBackend;
        serve_ipc(
            &config,
            &mut registry,
            &mut ime_state,
            &mut ime_backend,
            &mut data_payloads,
            vulkan.as_ref(),
        )
        .await?;
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
    scene_epoch: u64,
    wire_headless_test: bool,
    wire_test_socket: Option<PathBuf>,
    diagnostic_only: bool,
    production: bool,
    headless_socket: Option<PathBuf>,
    check_readiness: Option<String>,
    smoke_check: bool,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self {
            session_instance_id: DEFAULT_SESSION_INSTANCE_ID.to_string(),
            scene_epoch: generate_scene_epoch(),
            diagnostic_only: true,
            ..Self::default()
        };
        // waylandd currently tracks protocol/state; GPU scene work is owned by
        // compd/displayd until a real buffer import path is available here.

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--require-displayd" => config.require_displayd = true,
                "--serve-ipc" => config.serve_ipc = true,
                "--once" => config.serve_once = true,
                "--print-registry" => config.print_registry = true,
                "--vulkan" => config.use_vulkan = true,
                "--no-vulkan" => config.use_vulkan = false,
                "--wire-headless-test" => config.wire_headless_test = true,
                "--wire-test-socket" => {
                    let path = args.next().context("--wire-test-socket requires a path")?;
                    let path = PathBuf::from(path);
                    let path_str = path.to_string_lossy();
                    if path_str.contains("/run/user")
                        || path_str.contains("waybroker")
                        || path_str.contains("XDG_RUNTIME_DIR")
                    {
                        bail!("Forbidden socket path for --wire-test-socket: {}", path_str);
                    }
                    config.wire_test_socket = Some(path);
                }
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
                "--diagnostic-only" => {
                    config.diagnostic_only = true;
                    config.production = false;
                }
                "--production" => {
                    config.production = true;
                    config.diagnostic_only = false;
                }
                "--headless-socket" => {
                    config.headless_socket = Some(PathBuf::from(
                        args.next().context("--headless-socket requires a path")?,
                    ));
                }
                "--check-readiness" => {
                    config.check_readiness = Some(
                        args.next().context("--check-readiness requires a socket name or path")?,
                    );
                }
                "--smoke-check" => {
                    config.smoke_check = true;
                }
                "--help" | "-h" => {
                    println!(
                        "usage: waylandd [--require-displayd] [--serve-ipc] [--once] [--print-registry] [--registry PATH] [--bind-wayland-display NAME] [--vulkan|--no-vulkan] [--session-instance-id ID] [--wire-headless-test] [--wire-test-socket PATH] [--diagnostic-only] [--production] [--headless-socket PATH] [--check-readiness SOCKET] [--smoke-check]"
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
    ime_state: &mut ImeRuntimeState,
    ime_backend: &mut dyn ImeBackend,
    data_payloads: &mut DataPayloadRegistry,
    vulkan: Option<&VulkanBackend>,
) -> Result<()> {
    if config.production {
        start_topology_subscription().context("failed to start displayd topology subscription")?;
        start_topology_supervisor();
    }
    let _wayland_display = if let Some(name) = config.bind_wayland_display.as_deref() {
        Some(bind_wayland_display_socket_ext(name, config.production, config.scene_epoch)?)
    } else if let Some(path) = config.headless_socket.as_ref() {
        Some(bind_wayland_display_socket_absolute(path, config.production, config.scene_epoch)?)
    } else {
        None
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
        handle_client(
            stream,
            registry,
            ime_state,
            ime_backend,
            data_payloads,
            vulkan,
            config,
            &config.session_instance_id,
        )
        .await?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("service=waylandd op=terminate event=finished served_requests={served}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_client(
    mut stream: ServiceStream,
    registry: &mut SurfaceRegistrySnapshot,
    ime_state: &mut ImeRuntimeState,
    ime_backend: &mut dyn ImeBackend,
    data_payloads: &mut DataPayloadRegistry,
    vulkan: Option<&VulkanBackend>,
    config: &Config,
    session_instance_id: &str,
) -> Result<()> {
    {
        if let Some(ref global) = *GLOBAL_REGISTRY.lock().unwrap() {
            *registry = global.clone();
        }
    }
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let (response, registry_changed) =
        build_response(request, registry, ime_state, ime_backend, data_payloads, vulkan, config)
            .await;
    send_json_line(&mut stream, &response)?;
    if registry_changed {
        {
            let mut global = GLOBAL_REGISTRY.lock().unwrap();
            *global = Some(registry.clone());
        }
        write_surface_registry_artifact(registry, session_instance_id)?;
    }
    Ok(())
}

async fn build_response(
    request: IpcEnvelope,
    registry: &mut SurfaceRegistrySnapshot,
    ime_state: &mut ImeRuntimeState,
    ime_backend: &mut dyn ImeBackend,
    data_payloads: &mut DataPayloadRegistry,
    vulkan: Option<&VulkanBackend>,
    config: &Config,
) -> (IpcEnvelope, bool) {
    let source = request.source;
    let (response_kind, registry_changed) = match request.kind {
        MessageKind::WaylandCommand(command) if request.destination == ServiceRole::Waylandd => {
            let (event, changed) =
                handle_wayland_command(command, registry, data_payloads, vulkan, config).await;
            (MessageKind::WaylandEvent(event), changed)
        }
        MessageKind::ImeCommand(command) if request.destination == ServiceRole::Waylandd => {
            let event = handle_ime_command(command, ime_state, ime_backend);
            (MessageKind::ImeEvent(event), false)
        }
        MessageKind::WaylandCommand(_) | MessageKind::ImeCommand(_) => (
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

fn handle_ime_command(
    command: ImeCommand,
    state: &mut ImeRuntimeState,
    backend: &mut dyn ImeBackend,
) -> ImeEvent {
    match command {
        ImeCommand::GetImeStatus => ImeEvent::Status { status: state.status() },
        ImeCommand::SetImeBridgeMode { mode } => {
            state.bridge_mode = mode;
            println!("service=waylandd op=ime_bridge_mode event=changed mode={:?}", mode);
            ImeEvent::BridgeModeChanged { mode }
        }
        ImeCommand::FocusTextSurface { surface_id } => {
            state.focused_surface_id = Some(surface_id.clone());
            backend.focus_surface(&surface_id);
            println!("service=waylandd op=ime_focus event=changed surface_id={}", surface_id);
            ImeEvent::TextFocusChanged { surface_id: Some(surface_id) }
        }
        ImeCommand::ClearTextFocus => {
            state.focused_surface_id = None;
            state.preedit_active = false; // also clear preedit on defocus
            backend.clear_focus();
            println!("service=waylandd op=ime_focus event=cleared");
            ImeEvent::TextFocusChanged { surface_id: None }
        }
        ImeCommand::CommitString { text } => {
            state.commit_count = state.commit_count.saturating_add(1);
            state.preedit_active = false;
            println!("service=waylandd op=ime_commit_string event=committed");
            ImeEvent::StringCommitted { text }
        }
        ImeCommand::PreeditString { text, cursor_begin, cursor_end } => {
            state.preedit_active = !text.is_empty();
            println!("service=waylandd op=ime_preedit_string event=updated");
            ImeEvent::PreeditUpdated { text, cursor_begin, cursor_end }
        }
        ImeCommand::DeleteSurroundingText { before_length, after_length } => {
            println!("service=waylandd op=ime_delete_surrounding_text event=deleted");
            ImeEvent::SurroundingTextDeleted { before_length, after_length }
        }
        ImeCommand::SetCursorRect { x, y, width, height } => {
            let rect = waybroker_common::Rect { x, y, width, height };
            state.cursor_rect = Some(rect);
            backend.set_cursor_rect(rect);
            println!("service=waylandd op=ime_cursor_rect event=updated");
            ImeEvent::CursorRectChanged { rect }
        }
        ImeCommand::SetSurroundingText { text, cursor, anchor } => {
            state.surrounding_text = Some(text.clone());
            state.surrounding_cursor = cursor;
            backend.set_surrounding_text(&text, cursor, anchor);
            println!("service=waylandd op=ime_surrounding_text event=updated");
            ImeEvent::SurroundingTextChanged { text, cursor, anchor }
        }
        ImeCommand::SetContentType { hint, purpose } => {
            state.content_purpose = purpose;
            backend.set_content_type(hint, purpose);
            println!("service=waylandd op=ime_content_type event=updated");
            ImeEvent::ContentTypeChanged { hint, purpose }
        }
    }
}

async fn handle_wayland_command(
    command: WaylandCommand,
    registry: &mut SurfaceRegistrySnapshot,
    data_payloads: &mut DataPayloadRegistry,
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
        WaylandCommand::StartDrag { source_id, surface_id, mime_types } => {
            data_payloads.dnd.status = DnDStatus::Dragging;
            data_payloads.dnd.source_id = Some(source_id.clone());
            data_payloads.dnd.origin_surface_id = Some(surface_id);
            data_payloads.dnd.mime_types = mime_types;
            (WaylandEvent::DragStarted { source_id }, false)
        }
        WaylandCommand::DragEnter { surface_id, x, y, mime_types } => {
            data_payloads.dnd.target_surface_id = Some(surface_id.clone());
            data_payloads.dnd.x = x;
            data_payloads.dnd.y = y;
            data_payloads.dnd.mime_types = mime_types; // update or match
            (WaylandEvent::DragEntered { surface_id }, false)
        }
        WaylandCommand::DragMotion { surface_id, x, y, time: _ } => {
            data_payloads.dnd.x = x;
            data_payloads.dnd.y = y;
            (WaylandEvent::DragMotioned { surface_id }, false)
        }
        WaylandCommand::DragDrop => {
            data_payloads.dnd.status = DnDStatus::Dropped;
            (WaylandEvent::DragDropped, false)
        }
        WaylandCommand::DragLeave => {
            data_payloads.dnd.target_surface_id = None;
            (WaylandEvent::DragLeft, false)
        }
        WaylandCommand::DragCancel => {
            data_payloads.dnd = DnDState::default();
            (WaylandEvent::DragCancelled, false)
        }
        WaylandCommand::WriteData { source_id, mime_type, data } => {
            data_payloads.fake_buffers.insert((source_id, mime_type), data);
            // We just return a success acknowledgement using DataRead with the written data, or maybe we don't have a specific Write response.
            // Let's just return a dummy event. We can reuse rejected for missing or just use DataRead as an ack.
            (
                WaylandEvent::DataRead { source_id: "".into(), mime_type: "".into(), data: None },
                false,
            )
        }
        WaylandCommand::ReadData { source_id, mime_type } => {
            let data =
                data_payloads.fake_buffers.get(&(source_id.clone(), mime_type.clone())).cloned();
            (WaylandEvent::DataRead { source_id, mime_type, data }, false)
        }
        WaylandCommand::InjectRelativePointerMotion {
            surface_id,
            dx,
            dy,
            dx_unaccel,
            dy_unaccel,
            timestamp,
        } => (
            WaylandEvent::RelativePointerMotion {
                surface_id,
                dx,
                dy,
                dx_unaccel,
                dy_unaccel,
                timestamp,
            },
            false,
        ),
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
                println!(
                    "service=waylandd op={op} event=failed reason=\"unexpected response: {other:?}\""
                );
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

    let request = IpcEnvelope::new(
        ServiceRole::Waylandd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(command),
    );
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

fn query_output_topology() -> Result<Vec<OutputTopologyEntry>> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let request = IpcEnvelope::new(
        ServiceRole::Waylandd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::GetOutputTopology),
    );
    send_json_line(&mut stream, &request)?;
    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;
    if response.source != ServiceRole::Displayd || response.destination != ServiceRole::Waylandd {
        bail!("invalid displayd topology response envelope");
    }
    match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::OutputTopology { outputs, .. }) => Ok(outputs),
        MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
            bail!("displayd rejected topology: {reason}")
        }
        other => bail!("unexpected displayd topology response: {other:?}"),
    }
}

fn query_output_snapshot() -> Result<(u64, u64, Vec<OutputTopologyEntry>)> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let request = IpcEnvelope::new(
        ServiceRole::Waylandd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::GetOutputTopology),
    );
    send_json_line(&mut stream, &request)?;
    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;
    match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::OutputTopology { epoch, sequence, outputs }) => {
            Ok((epoch, sequence, outputs))
        }
        MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
            bail!("displayd rejected topology snapshot: {reason}")
        }
        other => bail!("unexpected topology snapshot response: {other:?}"),
    }
}

fn query_output_snapshot_with_retry(
    current_epoch: u64,
    current_sequence: u64,
) -> Result<(u64, u64, Vec<OutputTopologyEntry>)> {
    let mut last_error = None;
    for _ in 0..SNAPSHOT_RETRY_LIMIT {
        match query_output_snapshot() {
            Ok((epoch, sequence, outputs)) => {
                validate_output_snapshot(&outputs)?;
                if epoch < current_epoch || (epoch == current_epoch && sequence < current_sequence)
                {
                    bail!("stale topology snapshot");
                }
                return Ok((epoch, sequence, outputs));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("topology snapshot retry exhausted")))
}

fn validate_output_snapshot(outputs: &[OutputTopologyEntry]) -> Result<()> {
    let mut ids = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for output in outputs {
        if output.geometry.output_id.is_empty()
            || !ids.insert(output.geometry.output_id.clone())
            || output.name.is_empty()
            || !names.insert(output.name.clone())
            || output.geometry.width == 0
            || output.geometry.height == 0
            || output.geometry.stride < output.geometry.width.saturating_mul(4)
            || output.scale == 0
            || output.refresh_hz == 0
            || output.geometry.width.checked_mul(output.geometry.height).is_none()
        {
            bail!("malformed or duplicate topology snapshot output");
        }
    }
    Ok(())
}

fn start_topology_subscription() -> Result<()> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let request = IpcEnvelope::new(
        ServiceRole::Waylandd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::GetReconciliation),
    );
    send_json_line(&mut stream, &request)?;
    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;
    let epoch = match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::Reconciliation { epoch, .. }) => epoch,
        other => bail!("unexpected displayd reconciliation response: {other:?}"),
    };
    drop(reader);
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let request = IpcEnvelope::new(
        ServiceRole::Waylandd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::SubscribeOutputTopology { epoch, sequence: 0 }),
    );
    send_json_line(&mut stream, &request)?;
    let mut reader = BufReader::new(stream);
    // Keep one bounded slot reserved for the terminal overflow marker.
    let (sender, receiver): (SyncSender<TopologyInput>, Receiver<TopologyInput>) =
        sync_channel(TOPOLOGY_QUEUE_CAPACITY + 1);
    let initial: IpcEnvelope = read_json_line(&mut reader)?;
    if let MessageKind::DisplayEvent(event) = initial.kind {
        sender
            .try_send(TopologyInput::Event(event))
            .map_err(|_| anyhow::anyhow!("topology receiver overflow"))?;
    } else {
        bail!("invalid topology subscription response");
    }
    std::thread::spawn(move || {
        while let Ok(message) = read_json_line::<IpcEnvelope>(&mut reader) {
            let event = match message.kind {
                MessageKind::DisplayEvent(event) => event,
                _ => break,
            };
            if sender.try_send(TopologyInput::Event(event)).is_err() {
                let _ = sender.try_send(TopologyInput::Overflow);
                break;
            }
        }
    });
    *TOPOLOGY_RECEIVER.lock().unwrap() = Some(receiver);
    Ok(())
}

#[allow(unused_assignments)]
fn start_topology_supervisor() {
    let receiver = TOPOLOGY_RECEIVER.lock().unwrap().take();
    let Some(receiver) = receiver else {
        return;
    };
    ACTIVE_TOPOLOGY_CONSUMERS.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        struct ConsumerGuard;
        impl Drop for ConsumerGuard {
            fn drop(&mut self) {
                ACTIVE_TOPOLOGY_CONSUMERS.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _consumer_guard = ConsumerGuard;
        let mut projection = BTreeMap::<String, OutputTopologyEntry>::new();
        let mut epoch = 0u64;
        let mut sequence = 0u64;
        let mut state = ResyncState::Streaming;
        let mut pending_reason = None;
        let mut buffered_deltas = VecDeque::with_capacity(TOPOLOGY_DELTA_BUFFER_LIMIT);
        while let Ok(input) = receiver.recv() {
            if state == ResyncState::Failed {
                break;
            }
            if state != ResyncState::Streaming {
                match input {
                    TopologyInput::Event(DisplayEvent::OutputTopologyDelta(delta)) => {
                        if buffered_deltas.len() == TOPOLOGY_DELTA_BUFFER_LIMIT {
                            buffered_deltas.clear();
                            state = ResyncState::SnapshotPending;
                            pending_reason = Some(ResyncReason::StreamOverflow);
                        } else {
                            buffered_deltas.push_back(delta);
                        }
                    }
                    TopologyInput::Event(_) => {}
                    TopologyInput::Overflow => {
                        state = ResyncState::Failed;
                        break;
                    }
                }
                continue;
            }
            let event = match input {
                TopologyInput::Event(event) => event,
                TopologyInput::Overflow => {
                    state = ResyncState::Failed;
                    break;
                }
            };
            match event {
                DisplayEvent::OutputTopology {
                    epoch: next_epoch,
                    sequence: next_sequence,
                    outputs,
                } => {
                    if validate_output_snapshot(&outputs).is_err() {
                        continue;
                    }
                    if epoch != 0 && next_epoch < epoch {
                        continue;
                    }
                    if next_sequence < sequence {
                        continue;
                    }
                    let next = outputs
                        .into_iter()
                        .map(|output| (output.geometry.output_id.clone(), output))
                        .collect::<BTreeMap<_, _>>();
                    let previous = std::mem::replace(&mut projection, next.clone());
                    epoch = next_epoch;
                    sequence = next_sequence;
                    if previous.is_empty() {
                        broadcast_client_command(ClientCommand::InitialTopology {
                            epoch,
                            sequence,
                            outputs: projection.values().cloned().collect(),
                        });
                    } else {
                        broadcast_projection_diff(&previous, &projection, epoch, sequence);
                    }
                }
                DisplayEvent::OutputTopologyDelta(delta) => {
                    if delta.epoch != epoch || delta.topology_sequence != sequence.saturating_add(1)
                    {
                        state = ResyncState::SnapshotPending;
                        pending_reason = Some(ResyncReason::SequenceGap);
                        if let Ok((fresh_epoch, fresh_sequence, fresh_outputs)) =
                            query_output_snapshot_with_retry(epoch, sequence)
                        {
                            state = ResyncState::SnapshotApplying;
                            let fresh = fresh_outputs
                                .into_iter()
                                .map(|output| (output.geometry.output_id.clone(), output))
                                .collect::<BTreeMap<_, _>>();
                            let previous = std::mem::replace(&mut projection, fresh);
                            epoch = fresh_epoch;
                            sequence = fresh_sequence;
                            broadcast_projection_diff(&previous, &projection, epoch, sequence);
                            replay_buffered_deltas(
                                &mut buffered_deltas,
                                &mut projection,
                                epoch,
                                &mut sequence,
                            );
                            state = ResyncState::Streaming;
                            pending_reason = None;
                        } else {
                            state = ResyncState::Failed;
                        }
                        continue;
                    }
                    let command = match delta.transition {
                        OutputTopologyTransition::Added
                        | OutputTopologyTransition::Enabled
                        | OutputTopologyTransition::Reconfigured
                        | OutputTopologyTransition::Disabled
                        | OutputTopologyTransition::Removed => {
                            let Some(command) = apply_topology_delta(&mut projection, &delta)
                            else {
                                continue;
                            };
                            command
                        }
                        OutputTopologyTransition::Reset
                        | OutputTopologyTransition::SnapshotRequired => {
                            if pending_reason.is_some() {
                                continue;
                            }
                            state = ResyncState::SnapshotPending;
                            pending_reason = Some(match delta.transition {
                                OutputTopologyTransition::Reset => ResyncReason::Reset,
                                _ => ResyncReason::SnapshotRequired,
                            });
                            if let Ok((fresh_epoch, fresh_sequence, fresh_outputs)) =
                                query_output_snapshot_with_retry(epoch, sequence)
                            {
                                state = ResyncState::SnapshotApplying;
                                let fresh = fresh_outputs
                                    .into_iter()
                                    .map(|output| (output.geometry.output_id.clone(), output))
                                    .collect::<BTreeMap<_, _>>();
                                let previous = std::mem::replace(&mut projection, fresh);
                                epoch = fresh_epoch;
                                sequence = fresh_sequence;
                                broadcast_projection_diff(&previous, &projection, epoch, sequence);
                                replay_buffered_deltas(
                                    &mut buffered_deltas,
                                    &mut projection,
                                    epoch,
                                    &mut sequence,
                                );
                                state = ResyncState::Streaming;
                                pending_reason = None;
                            } else {
                                state = ResyncState::Failed;
                            }
                            continue;
                        }
                    };
                    epoch = delta.epoch;
                    sequence = delta.topology_sequence;
                    broadcast_client_command(command);
                    broadcast_client_command(ClientCommand::RecalculateMembership {
                        epoch,
                        sequence,
                    });
                }
                _ => {}
            }
        }
    });
}

fn apply_topology_delta(
    projection: &mut BTreeMap<String, OutputTopologyEntry>,
    delta: &OutputTopologyDelta,
) -> Option<ClientCommand> {
    match delta.transition {
        OutputTopologyTransition::Added | OutputTopologyTransition::Enabled => {
            let output = delta.output.clone()?;
            projection.insert(output.geometry.output_id.clone(), output.clone());
            Some(ClientCommand::AddGlobal {
                epoch: delta.epoch,
                sequence: delta.topology_sequence,
                output,
            })
        }
        OutputTopologyTransition::Reconfigured => {
            let output = delta.output.clone()?;
            projection.insert(output.geometry.output_id.clone(), output.clone());
            Some(ClientCommand::ReconfigureOutput {
                epoch: delta.epoch,
                sequence: delta.topology_sequence,
                output,
            })
        }
        OutputTopologyTransition::Disabled | OutputTopologyTransition::Removed => {
            let output_id = delta.output_id.clone()?;
            projection.remove(&output_id);
            Some(ClientCommand::RemoveGlobal {
                epoch: delta.epoch,
                sequence: delta.topology_sequence,
                output_id,
                output_generation: delta.output_generation.unwrap_or(0),
            })
        }
        OutputTopologyTransition::Reset | OutputTopologyTransition::SnapshotRequired => None,
    }
}

fn replay_buffered_deltas(
    buffered: &mut VecDeque<OutputTopologyDelta>,
    projection: &mut BTreeMap<String, OutputTopologyEntry>,
    epoch: u64,
    sequence: &mut u64,
) {
    let mut queued = buffered.drain(..).collect::<Vec<_>>();
    queued.sort_by_key(|delta| delta.topology_sequence);
    let mut last_sequence = None;
    for delta in queued {
        if delta.epoch != epoch || delta.topology_sequence <= *sequence {
            continue;
        }
        if last_sequence == Some(delta.topology_sequence)
            || delta.topology_sequence != (*sequence).saturating_add(1)
        {
            continue;
        }
        let Some(command) = apply_topology_delta(projection, &delta) else {
            continue;
        };
        *sequence = delta.topology_sequence;
        last_sequence = Some(*sequence);
        broadcast_client_command(command);
        broadcast_client_command(ClientCommand::RecalculateMembership {
            epoch,
            sequence: *sequence,
        });
    }
}

fn broadcast_client_command(command: ClientCommand) {
    let clients = CLIENT_COMMANDS
        .lock()
        .unwrap()
        .iter()
        .map(|(id, sender)| (*id, sender.clone()))
        .collect::<Vec<_>>();
    let mut failed = Vec::new();
    for (id, sender) in clients {
        if sender.try_send(command.clone()).is_err() {
            failed.push(id);
        }
    }
    if !failed.is_empty() {
        let mut registry = CLIENT_COMMANDS.lock().unwrap();
        for id in failed {
            registry.remove(&id);
        }
    }
}

fn broadcast_projection_diff(
    previous: &BTreeMap<String, OutputTopologyEntry>,
    next: &BTreeMap<String, OutputTopologyEntry>,
    epoch: u64,
    sequence: u64,
) {
    for command in projection_diff_commands(previous, next, epoch, sequence) {
        broadcast_client_command(command);
    }
}

fn projection_diff_commands(
    previous: &BTreeMap<String, OutputTopologyEntry>,
    next: &BTreeMap<String, OutputTopologyEntry>,
    epoch: u64,
    sequence: u64,
) -> Vec<ClientCommand> {
    let mut commands = Vec::new();
    for (output_id, old) in previous {
        if !next.contains_key(output_id) {
            commands.push(ClientCommand::RemoveGlobal {
                epoch,
                sequence,
                output_id: output_id.clone(),
                output_generation: old.geometry.output_generation,
            });
        }
    }
    for (output_id, output) in next {
        match previous.get(output_id) {
            None => {
                commands.push(ClientCommand::AddGlobal { epoch, sequence, output: output.clone() })
            }
            Some(old) if old != output => commands.push(ClientCommand::ReconfigureOutput {
                epoch,
                sequence,
                output: output.clone(),
            }),
            Some(_) => {}
        }
    }
    commands.push(ClientCommand::RecalculateMembership { epoch, sequence });
    commands
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
                ..Default::default()
            },
            WaylandSurfaceState {
                id: "background-1".into(),
                app_id: "org.kde.plasmashell.wallpaper".into(),
                role: WaylandSurfaceRole::Background,
                mapped: true,
                buffer_attached: true,
                ..Default::default()
            },
        ],
        foreign_toplevels: vec![],
        selection: WaylandSelectionState {
            clipboard_owner: Some("konsole-1".into()),
            clipboard_payload_id: Some("konsole-clipboard-v1".into()),
            clipboard_source_serial: Some(11),
            clipboard_offer: None,
            primary_selection_owner: None,
            primary_selection_payload_id: None,
            primary_selection_source_serial: None,
            primary_offer: None,
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
fn bind_wayland_display_socket_ext(
    name: &str,
    production: bool,
    scene_epoch: u64,
) -> Result<WaylandDisplaySocket> {
    let mut candidates = vec![name.to_string()];
    if let Some((prefix, display_num)) = split_wayland_display_name(name) {
        candidates.push(format!("{prefix}{}", display_num + 1));
        candidates.push(format!("{prefix}{}", display_num + 2));
    }

    let mut last_err = None;
    for candidate in candidates {
        let path = resolve_wayland_display_path(&candidate)?;
        let lock_path = wayland_lock_path(&path);
        match bind_single_wayland_display_socket_ext(&path, &lock_path, production, scene_epoch) {
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
fn bind_wayland_display_socket_absolute(
    path: &Path,
    production: bool,
    scene_epoch: u64,
) -> Result<WaylandDisplaySocket> {
    let lock_path = wayland_lock_path(path);
    bind_single_wayland_display_socket_ext(path, &lock_path, production, scene_epoch)
}

#[cfg(unix)]
fn bind_single_wayland_display_socket_ext(
    path: &Path,
    lock_path: &Path,
    production: bool,
    scene_epoch: u64,
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

    let listener = UnixListener::bind(path)
        .with_context(|| format!("failed to bind Wayland display socket {}", path.display()))?;
    fs::write(lock_path, format!("{}\n", std::process::id()))
        .with_context(|| format!("failed to write Wayland display lock {}", lock_path.display()))?;

    let log_path = path.to_path_buf();
    thread::Builder::new()
        .name("wayland-display-listener".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        if production {
                            println!(
                                "service=waylandd op=wayland_display event=client_connected path={} mode=production",
                                log_path.display()
                            );
                            let client_path = log_path.clone();
                            thread::spawn(move || {
                                if let Err(err) = handle_production_client(stream, scene_epoch) {
                                    eprintln!(
                                        "service=waylandd op=wayland_display event=production_client_error path={} reason={:?}",
                                        client_path.display(),
                                        err
                                    );
                                }
                            });
                        } else {
                            println!(
                                "service=waylandd op=wayland_display event=client_connected path={} info=\"connection accepted by diagnostic listener (no data read)\"",
                                log_path.display()
                            );
                        }
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

    if production {
        println!(
            "service=waylandd op=wayland_display event=production_listener_bound path={} info=\"this listener handles real Wayland clients\"",
            path.display()
        );
    } else {
        println!(
            "service=waylandd op=wayland_display event=diagnostic_listener_bound path={} info=\"this is a minimal listener for connection observation only\"",
            path.display()
        );
    }
    Ok(WaylandDisplaySocket { path: path.to_path_buf(), lock_path: lock_path.to_path_buf() })
}

fn handle_production_client(stream: UnixStream, scene_epoch: u64) -> Result<()> {
    let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
    let (command_sender, command_receiver) = sync_channel(64);
    CLIENT_COMMANDS.lock().unwrap().insert(client_id, command_sender.clone());
    let _command_guard = ClientCommandGuard { client_id };
    let mut core = wayland_wire::core::HeadlessWireCore::default();
    core.set_defer_frame_callbacks(true);
    let topology =
        query_output_topology().context("production Wayland requires displayd output inventory")?;
    if topology.is_empty() {
        bail!("production Wayland requires at least one real displayd output");
    }
    let outputs = topology
        .iter()
        .map(|entry| OutputMode {
            name: entry.geometry.output_id.clone(),
            width: entry.geometry.width,
            height: entry.geometry.height,
            refresh_hz: entry.refresh_hz,
        })
        .collect::<Vec<_>>();
    let mut registry_guard =
        ClientRegistryGuard { client_id, outputs: outputs.clone(), scene_epoch, finalized: false };
    let mut received_fds = Vec::new();
    let mut pending_callbacks = PendingFrameCallbacks::default();
    let mut command_epoch = 0u64;
    let mut command_sequence = 0u64;
    command_sender
        .try_send(ClientCommand::InitialTopology {
            epoch: 0,
            sequence: 0,
            outputs: topology.clone(),
        })
        .map_err(|_| anyhow::anyhow!("client command endpoint initialization overflow"))?;
    let mut buffer = Vec::with_capacity(4096);
    let mut read_buf = [0u8; 4096];

    // A production Wayland connection is long-lived; idle clients must not be
    // disconnected merely because no request arrived for a fixed interval.
    stream.set_read_timeout(None)?;

    loop {
        while let Ok(command) = command_receiver.try_recv() {
            if apply_client_command(&mut core, command, &mut command_epoch, &mut command_sequence)?
            {
                return Ok(());
            }
        }
        let (n, fds) = wayland_wire::fd::recv_with_fds(&stream, &mut read_buf)?;
        if n == 0 && fds.is_empty() {
            break;
        }
        buffer.extend_from_slice(&read_buf[..n]);
        received_fds.extend(fds);

        let mut consumed = 0;
        loop {
            let remaining = &buffer[consumed..];
            if remaining.len() < 8 {
                break;
            }

            match wayland_wire::codec::decode_message(remaining) {
                Ok(msg) => {
                    let size = msg.header.size as usize;
                    consumed += size;
                    let is_commit = msg.header.opcode.0 == 6
                        && core
                            .registry
                            .get_object(msg.header.object_id)
                            .map(|object| object.interface == "wl_surface")
                            .unwrap_or(false);
                    let committed_surface = msg.header.object_id;

                    let result = core.dispatch_with_fds(msg, &mut received_fds)?;
                    for ev in result.events {
                        let encoded = wayland_wire::codec::encode_message(&ev)?;
                        use std::io::Write;
                        let mut s = &stream;
                        s.write_all(&encoded)?;
                    }
                    if is_commit {
                        upsert_client_scene(client_id, &core);
                        let scene_generation = current_canonical_scene_generation();
                        let surface = production_scene_surfaces(client_id, &core)
                            .0
                            .into_iter()
                            .find(|surface| {
                                surface.id
                                    == format!("client-{client_id}-surface-{}", committed_surface.0)
                            });
                        let output_viewports = output_viewports(&outputs);
                        for callback_id in core.take_frame_callbacks(committed_surface) {
                            let intersecting_outputs = surface
                                .as_ref()
                                .map(|surface| {
                                    intersecting_output_ids(&surface.placement, &output_viewports)
                                })
                                .unwrap_or_default();
                            pending_callbacks.register(
                                client_id,
                                callback_id,
                                committed_surface.0,
                                scene_generation,
                                intersecting_outputs,
                            )?;
                        }

                        let _presented = commit_canonical_scene(&outputs, scene_epoch)?;
                        let presented_output =
                            outputs.first().map(|output| output.name.as_str()).unwrap_or_default();
                        for callback_id in pending_callbacks.release_for_presented(
                            client_id,
                            scene_generation,
                            presented_output,
                        ) {
                            let mut payload = vec![0u8; 4];
                            byteorder::LittleEndian::write_u32(&mut payload, 0);
                            let event = wayland_wire::WaylandMessage::new(
                                wayland_wire::WaylandObjectId(callback_id),
                                wayland_wire::WaylandOpcode(0),
                                payload,
                            );
                            let mut s = &stream;
                            s.write_all(&wayland_wire::codec::encode_message(&event)?)?;
                        }
                    }
                    if !core.surfaces.surfaces.contains_key(&committed_surface) {
                        pending_callbacks.remove_surface(client_id, committed_surface.0);
                    }
                }
                Err(wayland_wire::WireError::Incomplete) => break,
                Err(e) => return Err(e.into()),
            }
        }

        if consumed > 0 {
            buffer.drain(0..consumed);
            sync_core_to_global_registry(&core, client_id);
        }
    }
    registry_guard.finalize()?;
    Ok(())
}

struct ClientCommandGuard {
    client_id: u64,
}

impl Drop for ClientCommandGuard {
    fn drop(&mut self) {
        CLIENT_COMMANDS.lock().unwrap().remove(&self.client_id);
    }
}

fn apply_client_command(
    core: &mut wayland_wire::core::HeadlessWireCore,
    command: ClientCommand,
    epoch: &mut u64,
    sequence: &mut u64,
) -> Result<bool> {
    let (command_epoch, command_sequence) = match &command {
        ClientCommand::InitialTopology { epoch, sequence, .. }
        | ClientCommand::AddGlobal { epoch, sequence, .. }
        | ClientCommand::RemoveGlobal { epoch, sequence, .. }
        | ClientCommand::ReconfigureOutput { epoch, sequence, .. }
        | ClientCommand::RecalculateMembership { epoch, sequence }
        | ClientCommand::TopologyReset { epoch, sequence }
        | ClientCommand::Disconnect { epoch, sequence } => (*epoch, *sequence),
    };
    if *epoch != 0 && command_epoch != *epoch {
        bail!("client topology command epoch mismatch");
    }
    if command_sequence < *sequence {
        bail!("client topology command sequence moved backwards");
    }
    if command_sequence == *sequence && command_epoch != 0 {
        return Ok(false);
    }
    *epoch = command_epoch;
    *sequence = command_sequence;
    match command {
        ClientCommand::InitialTopology { outputs, .. } => {
            for output in outputs {
                core.add_topology_output(
                    &output.name,
                    output.geometry.origin_x,
                    output.geometry.origin_y,
                    output.geometry.width as i32,
                    output.geometry.height as i32,
                    (1_000_000_000u64 / output.refresh_hz.max(1) as u64) as u32,
                    output.scale,
                );
            }
        }
        ClientCommand::AddGlobal { output, .. } => core.add_topology_output(
            &output.name,
            output.geometry.origin_x,
            output.geometry.origin_y,
            output.geometry.width as i32,
            output.geometry.height as i32,
            (1_000_000_000u64 / output.refresh_hz.max(1) as u64) as u32,
            output.scale,
        ),
        ClientCommand::RemoveGlobal { output_id, .. } => core.remove_topology_output(&output_id),
        ClientCommand::ReconfigureOutput { output, .. } => core
            .reconfigure_topology_output(
                &output.name,
                output.geometry.origin_x,
                output.geometry.origin_y,
                output.geometry.width as i32,
                output.geometry.height as i32,
                (1_000_000_000u64 / output.refresh_hz.max(1) as u64) as u32,
                output.scale,
            )
            .map_err(|error| anyhow::anyhow!(error))?,
        ClientCommand::RecalculateMembership { .. } | ClientCommand::TopologyReset { .. } => {
            core.recalculate_surface_membership().map_err(|error| anyhow::anyhow!(error))?;
        }
        ClientCommand::Disconnect { .. } => return Ok(true),
    }
    Ok(false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingFrameCallback {
    callback_id: u32,
    client_id: u64,
    surface_id: u32,
    scene_generation: u64,
    intersecting_outputs: Vec<String>,
}

#[derive(Debug, Default)]
struct PendingFrameCallbacks {
    callbacks: BTreeMap<(u64, u32), PendingFrameCallback>,
}

impl PendingFrameCallbacks {
    fn register(
        &mut self,
        client_id: u64,
        callback_id: wayland_wire::WaylandObjectId,
        surface_id: u32,
        scene_generation: u64,
        mut intersecting_outputs: Vec<String>,
    ) -> Result<()> {
        intersecting_outputs.sort();
        intersecting_outputs.dedup();
        let key = (client_id, callback_id.0);
        if self.callbacks.contains_key(&key) {
            bail!(
                "duplicate frame callback identity client={client_id} callback={}",
                callback_id.0
            );
        }
        self.callbacks.insert(
            key,
            PendingFrameCallback {
                callback_id: callback_id.0,
                client_id,
                surface_id,
                scene_generation,
                intersecting_outputs,
            },
        );
        Ok(())
    }

    fn release_for_presented(
        &mut self,
        client_id: u64,
        presented_scene_generation: u64,
        output_id: &str,
    ) -> Vec<u32> {
        let keys: Vec<_> = self
            .callbacks
            .iter()
            .filter(|(_, callback)| {
                callback.client_id == client_id
                    && callback.scene_generation <= presented_scene_generation
                    && callback.intersecting_outputs.iter().any(|output| output == output_id)
            })
            .map(|(key, _)| *key)
            .collect();
        keys.into_iter()
            .filter_map(|key| self.callbacks.remove(&key).map(|callback| callback.callback_id))
            .collect()
    }

    fn remove_surface(&mut self, client_id: u64, surface_id: u32) {
        self.callbacks.retain(|_, callback| {
            !(callback.client_id == client_id && callback.surface_id == surface_id)
        });
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.callbacks.len()
    }
}

fn current_canonical_scene_generation() -> u64 {
    CANONICAL_SCENE.lock().unwrap().generation
}

fn output_viewports(outputs: &[OutputMode]) -> Vec<(String, waybroker_common::Rect)> {
    let mut origin_x = 0i32;
    outputs
        .iter()
        .map(|output| {
            let viewport = waybroker_common::Rect {
                x: origin_x,
                y: 0,
                width: output.width,
                height: output.height,
            };
            origin_x = origin_x.saturating_add(output.width.min(i32::MAX as u32) as i32);
            (output.name.clone(), viewport)
        })
        .collect()
}

fn intersecting_output_ids(
    placement: &SurfacePlacement,
    output_viewports: &[(String, waybroker_common::Rect)],
) -> Vec<String> {
    output_viewports
        .iter()
        .filter_map(|(output_id, viewport)| {
            let right = i64::from(placement.x) + i64::from(placement.width);
            let bottom = i64::from(placement.y) + i64::from(placement.height);
            let viewport_right = i64::from(viewport.x) + i64::from(viewport.width);
            let viewport_bottom = i64::from(viewport.y) + i64::from(viewport.height);
            (placement.visible
                && i64::from(placement.x) < viewport_right
                && right > i64::from(viewport.x)
                && i64::from(placement.y) < viewport_bottom
                && bottom > i64::from(viewport.y))
            .then(|| output_id.clone())
        })
        .collect()
}

struct ClientRegistryGuard {
    client_id: u64,
    outputs: Vec<OutputMode>,
    scene_epoch: u64,
    finalized: bool,
}

impl ClientRegistryGuard {
    fn finalize(&mut self) -> Result<()> {
        if self.finalized {
            return Ok(());
        }
        remove_client_from_global_registry(self.client_id);
        remove_client_scene(self.client_id);
        invalidate_client_pixel_payloads(self.client_id);
        self.finalized = true;
        commit_canonical_scene(&self.outputs, self.scene_epoch).map(|_| ()).map_err(|err| {
            eprintln!(
                "service=waylandd op=canonical_recommit event=failed client_id={} reason={:?}",
                self.client_id, err
            );
            err
        })
    }
}

impl Drop for ClientRegistryGuard {
    fn drop(&mut self) {
        if !self.finalized {
            if let Err(err) = self.finalize() {
                eprintln!(
                    "service=waylandd op=canonical_recommit event=aborted_disconnect_failed client_id={} reason={:?}",
                    self.client_id, err
                );
            }
        }
    }
}

fn production_scene_surfaces(
    client_id: u64,
    core: &wayland_wire::core::HeadlessWireCore,
) -> (Vec<SurfaceSnapshot>, Vec<PixelTransportPayload>) {
    let scene_generation = next_canonical_scene_generation();
    let mut payloads = Vec::new();
    let surfaces = core
        .surfaces
        .surfaces
        .iter()
        .filter_map(|(id, surface)| {
            let buffer_id = surface.current.buffer_id?;
            let buffer = core.shm.buffers.get(&buffer_id)?;
            let surface_id = format!("client-{client_id}-surface-{}", id.0);
            let handle = PixelTransportHandle {
                client_id,
                surface_id: surface_id.clone(),
                buffer_generation: buffer_id.0 as u64,
                scene_generation,
            };
            let pixels = read_shm_buffer(core, buffer);
            if !pixels.is_empty() {
                payloads.push(PixelTransportPayload {
                    handle: handle.clone(),
                    pixels,
                    width: buffer.width.max(0) as u32,
                    height: buffer.height.max(0) as u32,
                    stride: buffer.stride.max(0) as u32,
                    format: buffer.format,
                });
            }
            Some(SurfaceSnapshot {
                id: surface_id,
                app_id: "production-app".into(),
                placement: SurfacePlacement {
                    x: surface.current.offset_x,
                    y: surface.current.offset_y,
                    width: buffer.width.max(0) as u32,
                    height: buffer.height.max(0) as u32,
                    z: 0,
                    visible: true,
                },
                buffer_handle: Some(buffer_id.0.to_string()),
                buffer_generation: buffer_id.0 as u64,
                damage_rects: surface
                    .current
                    .damage
                    .iter()
                    .map(|r| waybroker_common::Rect {
                        x: r.x,
                        y: r.y,
                        width: r.width,
                        height: r.height,
                    })
                    .collect(),
                pixel_transport: Some(handle),
                layer_class: 0,
                creation_sequence: id.0 as u64,
            })
        })
        .collect();
    (surfaces, payloads)
}

fn upsert_client_scene(client_id: u64, core: &wayland_wire::core::HeadlessWireCore) {
    let (surfaces, payloads) = production_scene_surfaces(client_id, core);
    submit_pixel_payloads(payloads);
    let mut scene = CANONICAL_SCENE.lock().unwrap();
    scene.generation = scene.generation.saturating_add(1);
    scene.clients.insert(client_id, surfaces);
}

fn next_canonical_scene_generation() -> u64 {
    CANONICAL_SCENE.lock().unwrap().generation.saturating_add(1)
}

fn submit_pixel_payloads(payloads: Vec<PixelTransportPayload>) {
    let mut store = PIXEL_TRANSPORT.lock().unwrap();
    for payload in payloads {
        if let Err(err) = store.submit(payload) {
            eprintln!("service=waylandd op=pixel_transport_submit event=rejected reason={err:?}");
        }
    }
}

fn invalidate_client_pixel_payloads(client_id: u64) {
    PIXEL_TRANSPORT.lock().unwrap().invalidate_client(client_id);
}

fn remove_client_scene(client_id: u64) {
    let mut scene = CANONICAL_SCENE.lock().unwrap();
    if scene.clients.remove(&client_id).is_some() {
        scene.generation = scene.generation.saturating_add(1);
    }
}

#[derive(Default)]
struct CanonicalSceneState {
    generation: u64,
    clients: std::collections::BTreeMap<u64, Vec<SurfaceSnapshot>>,
    pending: Option<PendingCanonicalCommit>,
}

struct PendingCanonicalCommit {
    generation: u64,
    surfaces: Vec<SurfaceSnapshot>,
    pixel_payloads: Vec<PixelTransportPayload>,
    reason: String,
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
struct SceneOrderKey {
    layer_class: u32,
    z_index: i32,
    creation_sequence: u64,
    stable_surface_id: String,
}

fn canonical_scene_surfaces_from(scene: &CanonicalSceneState) -> (u64, Vec<SurfaceSnapshot>) {
    let surfaces: Vec<_> = scene.clients.values().flat_map(|items| items.iter().cloned()).collect();
    (scene.generation, order_scene_surfaces(surfaces))
}

fn order_scene_surfaces(mut surfaces: Vec<SurfaceSnapshot>) -> Vec<SurfaceSnapshot> {
    surfaces.sort_by_key(|surface| SceneOrderKey {
        layer_class: surface.layer_class,
        z_index: surface.placement.z,
        creation_sequence: surface.creation_sequence,
        stable_surface_id: surface.id.clone(),
    });
    for (z, surface) in surfaces.iter_mut().enumerate() {
        surface.placement.z = z as i32;
    }
    surfaces
}

fn commit_canonical_scene(outputs: &[OutputMode], scene_epoch: u64) -> Result<DisplayEvent> {
    let (generation, surfaces, pixel_payloads) = {
        let scene = CANONICAL_SCENE.lock().unwrap();
        match pending_replay_commit(&scene) {
            Some((generation, surfaces, pixel_payloads, reason)) => {
                eprintln!(
                    "service=waylandd op=canonical_recommit event=retry generation={} reason={}",
                    generation, reason
                );
                (generation, surfaces, pixel_payloads)
            }
            None => {
                let (generation, surfaces) = canonical_scene_surfaces_from(&scene);
                (generation, surfaces, Vec::new())
            }
        }
    };
    let pixel_payloads = if pixel_payloads.is_empty() {
        resolve_pixel_payloads_for_surfaces(&surfaces)
    } else {
        pixel_payloads
    };
    match commit_production_scene(&surfaces, &pixel_payloads, generation, scene_epoch, outputs) {
        Ok(event) => {
            let mut scene = CANONICAL_SCENE.lock().unwrap();
            if scene
                .pending
                .as_ref()
                .map(|pending| should_clear_pending_commit(generation, pending.generation))
                .unwrap_or(false)
            {
                scene.pending = None;
            }
            Ok(event)
        }
        Err(err) => {
            let mut scene = CANONICAL_SCENE.lock().unwrap();
            let current_generation = scene.generation;
            let current_pending_generation =
                scene.pending.as_ref().map(|pending| pending.generation).unwrap_or(0);
            if should_store_pending_commit(
                generation,
                current_generation,
                current_pending_generation,
            ) {
                scene.pending = Some(PendingCanonicalCommit {
                    generation,
                    surfaces,
                    pixel_payloads,
                    reason: err.to_string(),
                });
            }
            Err(err)
        }
    }
}

fn pending_replay_commit(
    scene: &CanonicalSceneState,
) -> Option<(u64, Vec<SurfaceSnapshot>, Vec<PixelTransportPayload>, String)> {
    let pending = scene.pending.as_ref()?;
    (pending.generation >= scene.generation).then(|| {
        (
            pending.generation,
            pending.surfaces.clone(),
            pending.pixel_payloads.clone(),
            pending.reason.clone(),
        )
    })
}

fn resolve_pixel_payloads_for_surfaces(surfaces: &[SurfaceSnapshot]) -> Vec<PixelTransportPayload> {
    let store = PIXEL_TRANSPORT.lock().unwrap();
    surfaces
        .iter()
        .filter_map(|surface| {
            surface.pixel_transport.as_ref().and_then(|handle| store.lookup(handle).cloned())
        })
        .collect()
}

fn should_store_pending_commit(
    failed_generation: u64,
    current_generation: u64,
    current_pending_generation: u64,
) -> bool {
    failed_generation >= current_generation && failed_generation >= current_pending_generation
}

fn should_clear_pending_commit(success_generation: u64, pending_generation: u64) -> bool {
    pending_generation <= success_generation
}

fn generate_scene_epoch() -> u64 {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64;
    nanos ^ ((std::process::id() as u64) << 32)
}

fn read_shm_buffer(
    core: &wayland_wire::core::HeadlessWireCore,
    buffer: &wayland_wire::shm::ShmBuffer,
) -> Vec<u8> {
    let Some(pool) = core.shm.pools.get(&buffer.pool_id) else { return Vec::new() };
    let len = buffer.byte_len().unwrap_or(0);
    if len == 0 {
        return Vec::new();
    }
    match &pool.storage {
        wayland_wire::shm::ShmPoolStorage::FakeMemory(bytes) => {
            let start = buffer.offset.max(0) as usize;
            bytes.get(start..start.saturating_add(len)).unwrap_or(&[]).to_vec()
        }
        wayland_wire::shm::ShmPoolStorage::ReceivedFd(fd) => {
            let mut pixels = vec![0u8; len];
            let read = unsafe {
                libc::pread(
                    fd.0.as_raw_fd(),
                    pixels.as_mut_ptr().cast(),
                    pixels.len(),
                    buffer.offset.max(0) as libc::off_t,
                )
            };
            if read == len as isize { pixels } else { Vec::new() }
        }
    }
}

fn commit_production_scene(
    surfaces: &[SurfaceSnapshot],
    pixel_payloads: &[PixelTransportPayload],
    scene_generation: u64,
    scene_epoch: u64,
    outputs: &[OutputMode],
) -> Result<DisplayEvent> {
    let mut stream =
        connect_service_socket(ServiceRole::Compd).context("production Wayland requires compd")?;
    let output = outputs.first().context("displayd returned no output")?;
    let request = IpcEnvelope::new(
        ServiceRole::Waylandd,
        ServiceRole::Compd,
        MessageKind::DisplayCommand(DisplayCommand::CommitScene {
            target: CommitTarget::Output { name: output.name.clone() },
            focus: FocusTarget::None,
            selection: WaylandSelectionState::default(),
            surfaces: surfaces.to_vec(),
            pixel_payloads: pixel_payloads.to_vec(),
            scene_epoch,
            scene_generation,
        }),
    );
    send_json_line(&mut stream, &request)?;
    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;
    let commit_id = match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::SceneCommitted { commit_id, .. }) => commit_id,
        other => bail!("scene commit failed before presentation: {other:?}"),
    };

    let mut feedback_stream = connect_service_socket(ServiceRole::Compd)?;
    let feedback_request = IpcEnvelope::new(
        ServiceRole::Waylandd,
        ServiceRole::Compd,
        MessageKind::DisplayCommand(DisplayCommand::GetPresentationFeedback { commit_id }),
    );
    send_json_line(&mut feedback_stream, &feedback_request)?;
    let mut feedback_reader = BufReader::new(feedback_stream);
    let feedback: IpcEnvelope = read_json_line(&mut feedback_reader)?;
    match feedback.kind {
        MessageKind::DisplayEvent(event @ DisplayEvent::FramePresented { .. }) => Ok(event),
        other => bail!("scene commit did not reach presentation: {other:?}"),
    }
}

fn sync_core_to_global_registry(core: &wayland_wire::core::HeadlessWireCore, client_id: u64) {
    let mut global = GLOBAL_REGISTRY.lock().unwrap();
    if let Some(ref mut reg) = *global {
        let prefix = format!("client-{client_id}-");
        reg.surfaces.retain(|surface| !surface.id.starts_with(&prefix));
        for (id, surf) in &core.surfaces.surfaces {
            let role = match core.surfaces.roles.get(id) {
                Some(wayland_wire::surface::SurfaceRoleKind::XdgSurface) => {
                    WaylandSurfaceRole::Toplevel
                }
                Some(wayland_wire::surface::SurfaceRoleKind::Popup) => WaylandSurfaceRole::Popup,
                Some(wayland_wire::surface::SurfaceRoleKind::LayerSurface) => {
                    WaylandSurfaceRole::Layer(waybroker_common::LayerMetadata::default())
                }
                _ => WaylandSurfaceRole::Toplevel,
            };

            reg.surfaces.push(WaylandSurfaceState {
                id: format!("client-{client_id}-surface-{}", id.0),
                app_id: "production-app".to_string(),
                role,
                mapped: surf.current.buffer_id.is_some(),
                buffer_attached: surf.current.buffer_id.is_some(),
                buffer_handle: surf.current.buffer_id.map(|b| b.0.to_string()),
                buffer_generation: surf.current.buffer_id.map(|b| b.0 as u64).unwrap_or(0),
                damage_rects: surf
                    .current
                    .damage
                    .iter()
                    .map(|r| waybroker_common::Rect {
                        x: r.x,
                        y: r.y,
                        width: r.width,
                        height: r.height,
                    })
                    .collect(),
            });
        }
        reg.generation = reg.generation.saturating_add(1);
        reg.unix_timestamp = now_unix_timestamp();
    }
}

fn remove_client_from_global_registry(client_id: u64) {
    let mut global = GLOBAL_REGISTRY.lock().unwrap();
    if let Some(ref mut reg) = *global {
        let prefix = format!("client-{client_id}-");
        let before = reg.surfaces.len();
        reg.surfaces.retain(|surface| !surface.id.starts_with(&prefix));
        if reg.surfaces.len() != before {
            reg.generation = reg.generation.saturating_add(1);
            reg.unix_timestamp = now_unix_timestamp();
        }
    }
}

fn read_xdg_surface_configure(
    stream: &mut UnixStream,
    buf: &mut Vec<u8>,
    temp_buf: &mut [u8],
    xdg_surface_id: u32,
) -> Result<u32> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if std::time::Instant::now() >= deadline {
            bail!("xdg_surface.configure timed out");
        }
        match stream.read(temp_buf) {
            Ok(0) => bail!("Connection closed while waiting for xdg_surface.configure"),
            Ok(n) => buf.extend_from_slice(&temp_buf[..n]),
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(e) => return Err(e.into()),
        }
        let mut consumed = 0;
        while consumed + 8 <= buf.len() {
            let header = wayland_wire::codec::decode_header(&buf[consumed..])?;
            if consumed + header.size as usize > buf.len() {
                break;
            }
            let msg = wayland_wire::codec::decode_message(
                &buf[consumed..consumed + header.size as usize],
            )?;
            consumed += header.size as usize;
            if msg.header.object_id.0 == xdg_surface_id && msg.header.opcode.0 == 0 {
                if msg.payload.len() < 4 {
                    bail!("invalid xdg_surface.configure payload");
                }
                let serial = byteorder::LittleEndian::read_u32(&msg.payload[..4]);
                buf.drain(..consumed);
                return Ok(serial);
            }
        }
        if consumed > 0 {
            buf.drain(..consumed);
        }
    }
}

fn run_readiness_check(socket_path: &Path) -> Result<()> {
    use byteorder::{ByteOrder, LittleEndian};
    use std::io::{Read, Write};
    use std::time::Duration;
    use wayland_wire::{WaylandMessage, WaylandObjectId, WaylandOpcode};

    println!("service=waylandd op=readiness_check event=begin path={}", socket_path.display());

    let mut stream = UnixStream::connect(socket_path)
        .context("Failed to connect to Wayland socket for readiness check")?;

    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;

    // 1. Send wl_display.get_registry (id: 1, opcode: 1, new_id: 2)
    let mut payload = vec![0u8; 4];
    LittleEndian::write_u32(&mut payload[0..4], 2);
    let msg1 = WaylandMessage::new(WaylandObjectId::DISPLAY, WaylandOpcode(1), payload);
    stream.write_all(&wayland_wire::codec::encode_message(&msg1)?)?;

    // 2. Send wl_display.sync (id: 1, opcode: 0, new_id: 3)
    let mut payload_sync = vec![0u8; 4];
    LittleEndian::write_u32(&mut payload_sync[0..4], 3);
    let msg2 = WaylandMessage::new(WaylandObjectId::DISPLAY, WaylandOpcode(0), payload_sync);
    stream.write_all(&wayland_wire::codec::encode_message(&msg2)?)?;

    let mut buf = Vec::new();
    let mut temp_buf = [0u8; 1024];
    let mut got_callback_done = false;
    let mut output_count = 0;
    let mut compositor_bound = false;
    let mut shm_bound = false;

    let start = std::time::Instant::now();
    while !got_callback_done && start.elapsed() < Duration::from_secs(2) {
        let n = match stream.read(&mut temp_buf) {
            Ok(0) => bail!("Connection closed during readiness check"),
            Ok(n) => n,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        buf.extend_from_slice(&temp_buf[..n]);

        let mut consumed = 0;
        while consumed + 8 <= buf.len() {
            let header = wayland_wire::codec::decode_header(&buf[consumed..])?;
            if consumed + header.size as usize > buf.len() {
                break;
            }
            let msg = wayland_wire::codec::decode_message(
                &buf[consumed..consumed + header.size as usize],
            )?;
            consumed += header.size as usize;

            #[allow(clippy::collapsible_if)]
            if msg.header.object_id.0 == 2 && msg.header.opcode.0 == 0 {
                if msg.payload.len() >= 8 {
                    let interface_len = LittleEndian::read_u32(&msg.payload[4..8]) as usize;
                    if msg.payload.len() >= 8 + interface_len {
                        let interface_name =
                            String::from_utf8_lossy(&msg.payload[8..8 + interface_len - 1])
                                .into_owned();
                        if interface_name == "wl_output" {
                            output_count += 1;
                        } else if interface_name == "wl_compositor" {
                            compositor_bound = true;
                        } else if interface_name == "wl_shm" {
                            shm_bound = true;
                        }
                    }
                }
            }

            if msg.header.object_id.0 == 3 && msg.header.opcode.0 == 0 {
                got_callback_done = true;
            }
        }
        if consumed > 0 {
            buf.drain(0..consumed);
        }
    }

    if !got_callback_done {
        bail!("Registry roundtrip timed out during readiness check");
    }

    // output inventory count >= 1
    // For smoke tests, KWin integration or test, output might not always be there unless displayd is running.
    // If not testing with displayd, we could inject outputs inside the core registry mock.
    // In production, we check output_count >= 1. We'll verify this condition.
    if output_count == 0 {
        bail!("No outputs advertised in registry. At least 1 output is required.");
    }
    if !compositor_bound || !shm_bound {
        bail!("Missing critical globals (wl_compositor/wl_shm)");
    }

    // 3. Bind wl_compositor (new_id 4)
    let mut bind_comp = Vec::new();
    bind_comp.extend_from_slice(&1u32.to_le_bytes()); // name
    let comp_iface = "wl_compositor\0";
    bind_comp.extend_from_slice(&(comp_iface.len() as u32).to_le_bytes());
    bind_comp.extend_from_slice(comp_iface.as_bytes());
    while bind_comp.len() % 4 != 0 {
        bind_comp.push(0);
    }
    bind_comp.extend_from_slice(&4u32.to_le_bytes()); // version
    bind_comp.extend_from_slice(&4u32.to_le_bytes()); // new_id
    let msg_bind_comp = WaylandMessage::new(WaylandObjectId(2), WaylandOpcode(0), bind_comp);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_bind_comp)?)?;

    // Bind wl_shm (new_id 7)
    let mut bind_shm = Vec::new();
    bind_shm.extend_from_slice(&2u32.to_le_bytes()); // name
    let shm_iface = "wl_shm\0";
    bind_shm.extend_from_slice(&(shm_iface.len() as u32).to_le_bytes());
    bind_shm.extend_from_slice(shm_iface.as_bytes());
    while bind_shm.len() % 4 != 0 {
        bind_shm.push(0);
    }
    bind_shm.extend_from_slice(&1u32.to_le_bytes()); // version
    bind_shm.extend_from_slice(&7u32.to_le_bytes()); // new_id
    let msg_bind_shm = WaylandMessage::new(WaylandObjectId(2), WaylandOpcode(0), bind_shm);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_bind_shm)?)?;

    // Bind xdg_wm_base (new_id 10)
    let mut bind_xdg = Vec::new();
    bind_xdg.extend_from_slice(&4u32.to_le_bytes()); // name
    let xdg_iface = "xdg_wm_base\0";
    bind_xdg.extend_from_slice(&(xdg_iface.len() as u32).to_le_bytes());
    bind_xdg.extend_from_slice(xdg_iface.as_bytes());
    while bind_xdg.len() % 4 != 0 {
        bind_xdg.push(0);
    }
    bind_xdg.extend_from_slice(&6u32.to_le_bytes()); // version
    bind_xdg.extend_from_slice(&10u32.to_le_bytes()); // new_id
    let msg_bind_xdg = WaylandMessage::new(WaylandObjectId(2), WaylandOpcode(0), bind_xdg);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_bind_xdg)?)?;

    // 4. Create Surface (id 4 (compositor), opcode 0, new_id 5)
    let mut create_surf = vec![0u8; 4];
    LittleEndian::write_u32(&mut create_surf[0..4], 5);
    let msg_create_surf = WaylandMessage::new(WaylandObjectId(4), WaylandOpcode(0), create_surf);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_create_surf)?)?;

    // Create Shm Pool (id 7 (wl_shm), opcode 0, new_id 8, size 4096)
    use std::os::unix::io::AsRawFd;
    let temp_file = tempfile::tempfile()?;
    let fd = temp_file.as_raw_fd();
    let mut create_pool = vec![0u8; 8];
    LittleEndian::write_u32(&mut create_pool[0..4], 8);
    LittleEndian::write_u32(&mut create_pool[4..8], 4096);
    let msg_create_pool = WaylandMessage::new(WaylandObjectId(7), WaylandOpcode(0), create_pool);
    let encoded_pool = wayland_wire::codec::encode_message(&msg_create_pool)?;
    wayland_wire::fd::send_with_fds(&stream, &encoded_pool, &[fd])?;

    // Create Buffer (id 8 (wl_shm_pool), opcode 0, new_id 9)
    let mut create_buf = vec![0u8; 24];
    LittleEndian::write_u32(&mut create_buf[0..4], 9); // new_id
    LittleEndian::write_u32(&mut create_buf[4..8], 0); // offset
    LittleEndian::write_u32(&mut create_buf[8..12], 1); // width
    LittleEndian::write_u32(&mut create_buf[12..16], 1); // height
    LittleEndian::write_u32(&mut create_buf[16..20], 4); // stride
    LittleEndian::write_u32(&mut create_buf[20..24], 0); // format (ARGB8888)
    let msg_create_buf = WaylandMessage::new(WaylandObjectId(8), WaylandOpcode(0), create_buf);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_create_buf)?)?;

    // Get Xdg Surface (id 10 (xdg_wm_base), opcode 2, new_id 11, surface 5)
    let mut get_xdg = vec![0u8; 8];
    LittleEndian::write_u32(&mut get_xdg[0..4], 11);
    LittleEndian::write_u32(&mut get_xdg[4..8], 5);
    let msg_get_xdg = WaylandMessage::new(WaylandObjectId(10), WaylandOpcode(2), get_xdg);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_get_xdg)?)?;

    // Get Toplevel (id 11 (xdg_surface), opcode 1, new_id 12)
    let mut get_top = vec![0u8; 4];
    LittleEndian::write_u32(&mut get_top[0..4], 12);
    let msg_get_top = WaylandMessage::new(WaylandObjectId(11), WaylandOpcode(1), get_top);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_get_top)?)?;

    // xdg-shell requires ack_configure before the first buffer commit.
    let configure_serial = read_xdg_surface_configure(&mut stream, &mut buf, &mut temp_buf, 11)?;
    let ack = WaylandMessage::new(
        WaylandObjectId(11),
        WaylandOpcode(3),
        configure_serial.to_le_bytes().to_vec(),
    );
    stream.write_all(&wayland_wire::codec::encode_message(&ack)?)?;

    // Attach Buffer (id 5 (wl_surface), opcode 1, buffer 9, x 0, y 0)
    let mut attach = vec![0u8; 12];
    LittleEndian::write_u32(&mut attach[0..4], 9);
    LittleEndian::write_i32(&mut attach[4..8], 0);
    LittleEndian::write_i32(&mut attach[8..12], 0);
    let msg_attach = WaylandMessage::new(WaylandObjectId(5), WaylandOpcode(1), attach);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_attach)?)?;

    // Damage (id 5, opcode 2, x 0, y 0, width 1, height 1)
    let mut damage = vec![0u8; 16];
    LittleEndian::write_i32(&mut damage[0..4], 0);
    LittleEndian::write_i32(&mut damage[4..8], 0);
    LittleEndian::write_i32(&mut damage[8..12], 1);
    LittleEndian::write_i32(&mut damage[12..16], 1);
    let msg_damage = WaylandMessage::new(WaylandObjectId(5), WaylandOpcode(2), damage);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_damage)?)?;

    // Frame callback (id 5, opcode 3, new_id 14)
    let mut frame_cb = vec![0u8; 4];
    LittleEndian::write_u32(&mut frame_cb[0..4], 14);
    let msg_frame = WaylandMessage::new(WaylandObjectId(5), WaylandOpcode(3), frame_cb);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_frame)?)?;

    // 5. Commit (id 5, opcode 6)
    let msg_commit = WaylandMessage::new(WaylandObjectId(5), WaylandOpcode(6), vec![]);
    stream.write_all(&wayland_wire::codec::encode_message(&msg_commit)?)?;

    // Wait for frame callback done (id 14, opcode 0)
    let mut got_frame_done = false;
    let start = std::time::Instant::now();
    while !got_frame_done && start.elapsed() < Duration::from_secs(3) {
        let n = match stream.read(&mut temp_buf) {
            Ok(0) => bail!("Connection closed during frame validation"),
            Ok(n) => n,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        buf.extend_from_slice(&temp_buf[..n]);

        let mut consumed = 0;
        while consumed + 8 <= buf.len() {
            let header = wayland_wire::codec::decode_header(&buf[consumed..])?;
            if consumed + header.size as usize > buf.len() {
                break;
            }
            let msg = wayland_wire::codec::decode_message(
                &buf[consumed..consumed + header.size as usize],
            )?;
            consumed += header.size as usize;

            if msg.header.object_id.0 == 14 && msg.header.opcode.0 == 0 {
                got_frame_done = true;
            }
        }
        if consumed > 0 {
            buf.drain(0..consumed);
        }
    }

    if !got_frame_done {
        bail!("Frame presented callback timed out during readiness check");
    }

    Ok(())
}

fn run_smoke_check() -> Result<()> {
    let temp_dir = std::env::temp_dir();
    let socket_path = temp_dir.join(format!("waybroker-smoke-{}.sock", std::process::id()));

    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    // Start production server in background
    let s_path = socket_path.clone();
    let handle = thread::spawn(move || {
        let display = bind_wayland_display_socket_absolute(&s_path, true, 0).unwrap();
        // Keep the lock alive for the duration of the test
        std::thread::sleep(std::time::Duration::from_secs(5));
        drop(display);
    });

    // Wait for socket
    let mut found = false;
    for _ in 0..50 {
        if socket_path.exists() {
            found = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    if !found {
        bail!("Smoke test socket did not appear");
    }

    // Run readiness check
    let res = run_readiness_check(&socket_path);

    // Clean up
    let _ = std::fs::remove_file(&socket_path);
    let lock_path = wayland_lock_path(&socket_path);
    let _ = std::fs::remove_file(&lock_path);

    let _ = handle.join();
    res
}

#[cfg(not(unix))]
fn bind_wayland_display_socket_ext(
    _name: &str,
    _production: bool,
    _scene_epoch: u64,
) -> Result<WaylandDisplaySocket> {
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
    use super::{
        ACTIVE_TOPOLOGY_CONSUMERS, PendingFrameCallbacks, ResyncState, SNAPSHOT_RETRY_LIMIT,
        TOPOLOGY_DELTA_BUFFER_LIMIT, TOPOLOGY_QUEUE_CAPACITY, handle_wayland_command,
        intersecting_output_ids, mock_surface_registry, output_viewports, projection_diff_commands,
        should_clear_pending_commit, should_store_pending_commit, socket_lock_is_stale,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::atomic::Ordering;
    use waybroker_common::{
        FocusTarget, OutputGeometry, OutputMode, OutputTopologyEntry, SurfacePlacement,
        SurfaceSnapshot, WaylandCommand, WaylandEvent, WaylandSelectionHandoff,
        WaylandSelectionState, WaylandSurfaceRole,
    };

    #[test]
    fn topology_snapshot_validation_rejects_duplicates_and_zero_scale() {
        let entry = OutputTopologyEntry {
            geometry: OutputGeometry {
                output_id: "eDP-1".into(),
                width: 1920,
                height: 1080,
                stride: 7680,
                format: 1,
                origin_x: 0,
                origin_y: 0,
                output_generation: 1,
            },
            refresh_hz: 60,
            scale: 1,
            transform: 0,
            enabled: true,
            name: "eDP-1".into(),
            description: "test".into(),
        };
        assert!(super::validate_output_snapshot(&[entry.clone(), entry.clone()]).is_err());
        let mut invalid = entry;
        invalid.scale = 0;
        assert!(super::validate_output_snapshot(&[invalid]).is_err());
    }

    #[test]
    fn resynchronization_policy_is_explicit_and_bounded() {
        assert_eq!(ResyncState::Streaming, ResyncState::Streaming);
        assert_ne!(ResyncState::SnapshotPending, ResyncState::Streaming);
        assert_eq!(TOPOLOGY_DELTA_BUFFER_LIMIT, 32);
        assert_eq!(TOPOLOGY_QUEUE_CAPACITY, 64);
        assert_eq!(SNAPSHOT_RETRY_LIMIT, 3);
        assert_eq!(ACTIVE_TOPOLOGY_CONSUMERS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn topology_snapshot_validation_rejects_invalid_identity_stride_and_overflow() {
        let mut entry = OutputTopologyEntry {
            geometry: OutputGeometry {
                output_id: "HDMI-A-1".into(),
                width: 1920,
                height: 1080,
                stride: 7680,
                format: 1,
                origin_x: 0,
                origin_y: 0,
                output_generation: 7,
            },
            refresh_hz: 60,
            scale: 1,
            transform: 0,
            enabled: true,
            name: "HDMI-A-1".into(),
            description: "test".into(),
        };
        entry.name.clear();
        assert!(super::validate_output_snapshot(&[entry.clone()]).is_err());
        entry.name = "HDMI-A-1".into();
        entry.geometry.stride = 1;
        assert!(super::validate_output_snapshot(&[entry.clone()]).is_err());
        entry.geometry.stride = u32::MAX;
        entry.geometry.width = u32::MAX;
        assert!(super::validate_output_snapshot(&[entry]).is_err());
    }

    #[test]
    fn snapshot_diff_preserves_stable_outputs_and_has_deterministic_minimal_order() {
        let output = |id: &str, generation: u64| OutputTopologyEntry {
            geometry: OutputGeometry {
                output_id: id.into(),
                width: 1920,
                height: 1080,
                stride: 7680,
                format: 1,
                origin_x: 0,
                origin_y: 0,
                output_generation: generation,
            },
            refresh_hz: 60,
            scale: 1,
            transform: 0,
            enabled: true,
            name: id.into(),
            description: "test".into(),
        };
        let old_a = output("A", 1);
        let old_b = output("B", 1);
        let old_c = output("C", 1);
        let mut previous = BTreeMap::new();
        previous.insert("A".into(), old_a.clone());
        previous.insert("B".into(), old_b);
        previous.insert("C".into(), old_c);
        let mut changed_a = old_a;
        changed_a.geometry.output_generation = 2;
        changed_a.scale = 2;
        let mut next = BTreeMap::new();
        next.insert("A".into(), changed_a);
        next.insert("D".into(), output("D", 4));

        let commands = projection_diff_commands(&previous, &next, 9, 12);
        assert!(
            matches!(&commands[0], super::ClientCommand::RemoveGlobal { output_id, .. } if output_id == "B")
        );
        assert!(
            matches!(&commands[1], super::ClientCommand::RemoveGlobal { output_id, .. } if output_id == "C")
        );
        assert!(
            matches!(&commands[2], super::ClientCommand::ReconfigureOutput { output, .. } if output.geometry.output_id == "A")
        );
        assert!(
            matches!(&commands[3], super::ClientCommand::AddGlobal { output, .. } if output.geometry.output_id == "D")
        );
        assert!(matches!(&commands[4], super::ClientCommand::RecalculateMembership { .. }));
        assert_eq!(commands.len(), 5);
    }

    #[test]
    fn frame_callback_waits_for_presented_and_delivers_once() {
        let mut callbacks = PendingFrameCallbacks::default();
        callbacks
            .register(
                7,
                wayland_wire::WaylandObjectId(42),
                3,
                11,
                vec!["HDMI-A-1".into(), "eDP-1".into()],
            )
            .unwrap();

        assert_eq!(callbacks.len(), 1);
        assert!(callbacks.release_for_presented(7, 10, "eDP-1").is_empty());
        assert_eq!(callbacks.len(), 1);
        assert_eq!(callbacks.release_for_presented(7, 11, "eDP-1"), vec![42]);
        assert!(callbacks.release_for_presented(7, 11, "HDMI-A-1").is_empty());
        assert_eq!(callbacks.len(), 0);
    }

    #[test]
    fn frame_callback_ignores_non_intersecting_output_and_newer_generation() {
        let mut callbacks = PendingFrameCallbacks::default();
        callbacks
            .register(7, wayland_wire::WaylandObjectId(42), 3, 11, vec!["eDP-1".into()])
            .unwrap();

        assert!(callbacks.release_for_presented(7, 11, "DP-1").is_empty());
        assert!(callbacks.release_for_presented(7, 10, "eDP-1").is_empty());
        assert_eq!(callbacks.len(), 1);
    }

    #[test]
    fn frame_callback_cleanup_is_scoped_to_surface_and_client() {
        let mut callbacks = PendingFrameCallbacks::default();
        callbacks
            .register(1, wayland_wire::WaylandObjectId(2), 3, 1, vec!["eDP-1".into()])
            .unwrap();
        callbacks
            .register(2, wayland_wire::WaylandObjectId(2), 3, 1, vec!["eDP-1".into()])
            .unwrap();
        callbacks
            .register(1, wayland_wire::WaylandObjectId(4), 4, 1, vec!["eDP-1".into()])
            .unwrap();

        callbacks.remove_surface(1, 3);
        assert_eq!(callbacks.len(), 2);
        assert_eq!(callbacks.release_for_presented(2, 1, "eDP-1"), vec![2]);
        assert_eq!(callbacks.release_for_presented(1, 1, "eDP-1"), vec![4]);
    }

    #[test]
    fn output_intersection_is_deterministic_and_excludes_empty_viewports() {
        let outputs = vec![
            OutputMode { name: "eDP-1".into(), width: 100, height: 100, refresh_hz: 60 },
            OutputMode { name: "HDMI-A-1".into(), width: 100, height: 100, refresh_hz: 60 },
        ];
        let viewports = output_viewports(&outputs);
        let placement =
            SurfacePlacement { x: 90, y: 10, width: 30, height: 20, z: 0, visible: true };
        assert_eq!(intersecting_output_ids(&placement, &viewports), vec!["eDP-1", "HDMI-A-1"]);
        assert!(
            intersecting_output_ids(&SurfacePlacement { visible: false, ..placement }, &viewports)
                .is_empty()
        );
    }

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
        let mut data_payloads = super::DataPayloadRegistry::default();
        let (event, changed) =
            tokio::runtime::Runtime::new().unwrap().block_on(handle_wayland_command(
                WaylandCommand::ApplySelectionHandoff {
                    handoff: WaylandSelectionHandoff {
                        focus: FocusTarget::Surface { id: "konsole-1".into() },
                        selection: WaylandSelectionState {
                            clipboard_owner: Some("konsole-1".into()),
                            clipboard_payload_id: Some("konsole-clipboard-v2".into()),
                            clipboard_source_serial: Some(12),
                            clipboard_offer: None,
                            primary_selection_owner: Some("konsole-1".into()),
                            primary_selection_payload_id: Some("konsole-primary-v1".into()),
                            primary_selection_source_serial: Some(13),
                            primary_offer: None,
                        },
                    },
                },
                &mut registry,
                &mut data_payloads,
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
    fn pending_replay_only_tracks_latest_generation() {
        assert!(should_store_pending_commit(11, 11, 10));
        assert!(!should_store_pending_commit(10, 11, 10));
        assert!(!should_store_pending_commit(11, 12, 10));
        assert!(!should_store_pending_commit(10, 10, 11));
        assert!(should_clear_pending_commit(11, 10));
        assert!(!should_clear_pending_commit(10, 11));
    }

    #[test]
    fn canonical_scene_metadata_survives_without_transport_payload() {
        let scene = super::CanonicalSceneState {
            generation: 4,
            clients: [(7, {
                vec![SurfaceSnapshot {
                    id: "client-7-surface-3".into(),
                    app_id: "production-app".into(),
                    placement: SurfacePlacement { z: 9, visible: true, ..Default::default() },
                    pixel_transport: Some(waybroker_common::PixelTransportHandle {
                        client_id: 7,
                        surface_id: "client-7-surface-3".into(),
                        buffer_generation: 3,
                        scene_generation: 4,
                    }),
                    creation_sequence: 3,
                    ..Default::default()
                }]
            })]
            .into_iter()
            .collect(),
            pending: None,
        };

        let (generation, surfaces) = super::canonical_scene_surfaces_from(&scene);

        assert_eq!(generation, 4);
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0].id, "client-7-surface-3");
        assert_eq!(surfaces[0].pixel_transport.as_ref().unwrap().scene_generation, 4);
    }

    #[test]
    fn production_scene_splits_shm_pixels_from_canonical_surface_metadata() {
        let mut core = wayland_wire::core::HeadlessWireCore::default();
        let surface_id = wayland_wire::WaylandObjectId(3);
        let pool_id = wayland_wire::WaylandObjectId(4);
        let buffer_id = wayland_wire::WaylandObjectId(5);
        core.shm.create_pool_from_fake(pool_id, 16);
        core.shm.create_buffer(buffer_id, pool_id, 0, 2, 2, 8, 0).unwrap();
        core.surfaces.surfaces.insert(
            surface_id,
            wayland_wire::surface::SurfaceInstance {
                pending: Default::default(),
                current: wayland_wire::surface::SurfaceState {
                    buffer_id: Some(buffer_id),
                    offset_x: 10,
                    offset_y: 20,
                    damage: vec![wayland_wire::surface::Rect { x: 0, y: 0, width: 2, height: 2 }],
                    opaque_region: None,
                    input_region: None,
                },
                callbacks: vec![],
            },
        );

        let (surfaces, payloads) = super::production_scene_surfaces(7, &core);

        assert_eq!(surfaces.len(), 1);
        assert_eq!(payloads.len(), 1);
        assert_eq!(surfaces[0].id, "client-7-surface-3");
        assert_eq!(surfaces[0].placement.x, 10);
        assert_eq!(surfaces[0].placement.y, 20);
        assert_eq!(surfaces[0].buffer_generation, 5);
        assert!(surfaces[0].pixel_transport.is_some());
        assert_eq!(surfaces[0].pixel_transport.as_ref().unwrap(), &payloads[0].handle);
        assert_eq!(payloads[0].pixels.len(), 16);
        assert_eq!(payloads[0].format, 0);
    }

    #[test]
    fn pending_replay_carries_transport_payload_bundle_once() {
        let handle = waybroker_common::PixelTransportHandle {
            client_id: 7,
            surface_id: "client-7-surface-3".into(),
            buffer_generation: 3,
            scene_generation: 4,
        };
        let payload = waybroker_common::PixelTransportPayload {
            handle: handle.clone(),
            pixels: vec![0; 16],
            width: 2,
            height: 2,
            stride: 8,
            format: 1,
        };
        let scene = super::CanonicalSceneState {
            generation: 4,
            clients: Default::default(),
            pending: Some(super::PendingCanonicalCommit {
                generation: 4,
                surfaces: vec![SurfaceSnapshot {
                    id: handle.surface_id.clone(),
                    pixel_transport: Some(handle),
                    ..Default::default()
                }],
                pixel_payloads: vec![payload],
                reason: "displayd unavailable".into(),
            }),
        };

        let (generation, surfaces, payloads, _) = super::pending_replay_commit(&scene).unwrap();

        assert_eq!(generation, 4);
        assert_eq!(surfaces.len(), 1);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].handle.surface_id, surfaces[0].id);
        assert_eq!(payloads[0].format, 1);
    }

    #[test]
    fn pending_replay_retains_surface_damage_with_payload_bundle() {
        let handle = waybroker_common::PixelTransportHandle {
            client_id: 7,
            surface_id: "client-7-surface-3".into(),
            buffer_generation: 3,
            scene_generation: 4,
        };
        let damage = waybroker_common::Rect { x: 1, y: 2, width: 3, height: 4 };
        let scene = super::CanonicalSceneState {
            generation: 4,
            clients: Default::default(),
            pending: Some(super::PendingCanonicalCommit {
                generation: 4,
                surfaces: vec![SurfaceSnapshot {
                    id: handle.surface_id.clone(),
                    pixel_transport: Some(handle.clone()),
                    damage_rects: vec![damage],
                    ..Default::default()
                }],
                pixel_payloads: vec![waybroker_common::PixelTransportPayload {
                    handle,
                    pixels: vec![0; 16],
                    width: 2,
                    height: 2,
                    stride: 8,
                    format: 0,
                }],
                reason: "displayd unavailable".into(),
            }),
        };

        let (_, surfaces, payloads, _) = super::pending_replay_commit(&scene).unwrap();

        assert_eq!(surfaces[0].damage_rects, vec![damage]);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].format, 0);
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

    #[tokio::test]
    async fn test_ime_state_transitions() {
        use waybroker_common::{ImeBridgeMode, ImeCommand, ImeEvent};
        let mut state = super::ImeRuntimeState::default();
        let mut backend = super::FakeImeBackend;
        assert_eq!(state.bridge_mode, ImeBridgeMode::Disabled);

        // Test mode change
        let event = super::handle_ime_command(
            ImeCommand::SetImeBridgeMode { mode: ImeBridgeMode::ProtocolStub },
            &mut state,
            &mut backend,
        );
        assert_eq!(event, ImeEvent::BridgeModeChanged { mode: ImeBridgeMode::ProtocolStub });
        assert_eq!(state.bridge_mode, ImeBridgeMode::ProtocolStub);

        // Test focus change
        let event = super::handle_ime_command(
            ImeCommand::FocusTextSurface { surface_id: "editor-1".into() },
            &mut state,
            &mut backend,
        );
        assert_eq!(event, ImeEvent::TextFocusChanged { surface_id: Some("editor-1".into()) });
        assert_eq!(state.focused_surface_id.as_deref(), Some("editor-1"));

        // Test status query
        let event = super::handle_ime_command(ImeCommand::GetImeStatus, &mut state, &mut backend);
        if let ImeEvent::Status { status } = event {
            assert_eq!(status.bridge_mode, ImeBridgeMode::ProtocolStub);
            assert_eq!(status.focused_surface_id.as_deref(), Some("editor-1"));
        } else {
            panic!("expected status event");
        }

        // Test clear focus
        let event = super::handle_ime_command(ImeCommand::ClearTextFocus, &mut state, &mut backend);
        assert_eq!(event, ImeEvent::TextFocusChanged { surface_id: None });
        assert_eq!(state.focused_surface_id, None);

        // Test preedit
        let event = super::handle_ime_command(
            ImeCommand::PreeditString { text: "hello".into(), cursor_begin: 5, cursor_end: 5 },
            &mut state,
            &mut backend,
        );
        assert_eq!(
            event,
            ImeEvent::PreeditUpdated { text: "hello".into(), cursor_begin: 5, cursor_end: 5 }
        );
        assert!(state.preedit_active);

        // Test cursor rect
        let event = super::handle_ime_command(
            ImeCommand::SetCursorRect { x: 10, y: 20, width: 0, height: 16 },
            &mut state,
            &mut backend,
        );
        assert_eq!(
            event,
            ImeEvent::CursorRectChanged {
                rect: waybroker_common::Rect { x: 10, y: 20, width: 0, height: 16 }
            }
        );
        assert_eq!(state.cursor_rect.unwrap().y, 20);

        // Test commit
        let event = super::handle_ime_command(
            ImeCommand::CommitString { text: "hello".into() },
            &mut state,
            &mut backend,
        );
        assert_eq!(event, ImeEvent::StringCommitted { text: "hello".into() });
        assert!(!state.preedit_active);
        assert_eq!(state.commit_count, 1);

        // Test surrounding text and content type
        let event = super::handle_ime_command(
            ImeCommand::SetSurroundingText { text: "world".into(), cursor: 5, anchor: 5 },
            &mut state,
            &mut backend,
        );
        assert_eq!(
            event,
            ImeEvent::SurroundingTextChanged { text: "world".into(), cursor: 5, anchor: 5 }
        );
        assert_eq!(state.surrounding_text.as_deref(), Some("world"));

        let event = super::handle_ime_command(
            ImeCommand::SetContentType { hint: 1, purpose: 2 },
            &mut state,
            &mut backend,
        );
        assert_eq!(event, ImeEvent::ContentTypeChanged { hint: 1, purpose: 2 });
        assert_eq!(state.content_purpose, 2);
    }

    #[tokio::test]
    async fn test_dnd_and_data_transfer_lifecycle() {
        use waybroker_common::{WaylandCommand, WaylandEvent};
        let mut registry = mock_surface_registry();
        let mut data_payloads = super::DataPayloadRegistry::default();
        let config = super::Config::default();

        // Start Drag
        let (event, _) = super::handle_wayland_command(
            WaylandCommand::StartDrag {
                source_id: "src-1".into(),
                surface_id: "konsole-1".into(),
                mime_types: vec!["text/plain".into()],
            },
            &mut registry,
            &mut data_payloads,
            None,
            &config,
        )
        .await;
        assert_eq!(event, WaylandEvent::DragStarted { source_id: "src-1".into() });
        assert_eq!(data_payloads.dnd.status, super::DnDStatus::Dragging);
        assert_eq!(data_payloads.dnd.source_id.as_deref(), Some("src-1"));

        // Drag Enter
        let (event, _) = super::handle_wayland_command(
            WaylandCommand::DragEnter {
                surface_id: "target-1".into(),
                x: 100.0,
                y: 200.0,
                mime_types: vec!["text/plain".into()],
            },
            &mut registry,
            &mut data_payloads,
            None,
            &config,
        )
        .await;
        assert_eq!(event, WaylandEvent::DragEntered { surface_id: "target-1".into() });
        assert_eq!(data_payloads.dnd.target_surface_id.as_deref(), Some("target-1"));

        // Write Data
        let _ = super::handle_wayland_command(
            WaylandCommand::WriteData {
                source_id: "src-1".into(),
                mime_type: "text/plain".into(),
                data: b"hello drop".to_vec(),
            },
            &mut registry,
            &mut data_payloads,
            None,
            &config,
        )
        .await;

        // Drop
        let (event, _) = super::handle_wayland_command(
            WaylandCommand::DragDrop,
            &mut registry,
            &mut data_payloads,
            None,
            &config,
        )
        .await;
        assert_eq!(event, WaylandEvent::DragDropped);
        assert_eq!(data_payloads.dnd.status, super::DnDStatus::Dropped);

        // Read Data
        let (event, _) = super::handle_wayland_command(
            WaylandCommand::ReadData { source_id: "src-1".into(), mime_type: "text/plain".into() },
            &mut registry,
            &mut data_payloads,
            None,
            &config,
        )
        .await;
        if let WaylandEvent::DataRead { source_id, mime_type, data } = event {
            assert_eq!(source_id, "src-1");
            assert_eq!(mime_type, "text/plain");
            assert_eq!(data.unwrap(), b"hello drop");
        } else {
            panic!("Expected DataRead");
        }
    }

    #[tokio::test]
    async fn test_relative_pointer_motion() {
        use waybroker_common::{WaylandCommand, WaylandEvent};
        let mut registry = mock_surface_registry();
        let mut data_payloads = super::DataPayloadRegistry::default();
        let config = super::Config::default();

        let (event, _) = super::handle_wayland_command(
            WaylandCommand::InjectRelativePointerMotion {
                surface_id: "game-1".into(),
                dx: 1.5,
                dy: -2.0,
                dx_unaccel: 1.0,
                dy_unaccel: -2.0,
                timestamp: 1000,
            },
            &mut registry,
            &mut data_payloads,
            None,
            &config,
        )
        .await;

        if let WaylandEvent::RelativePointerMotion {
            surface_id,
            dx,
            dy,
            dx_unaccel,
            dy_unaccel,
            timestamp,
        } = event
        {
            assert_eq!(surface_id, "game-1");
            assert_eq!(dx, 1.5);
            assert_eq!(dy, -2.0);
            assert_eq!(dx_unaccel, 1.0);
            assert_eq!(dy_unaccel, -2.0);
            assert_eq!(timestamp, 1000);
        } else {
            panic!("Expected RelativePointerMotion");
        }
    }

    #[test]
    fn test_wire_test_socket_path_validation() {
        use super::Config;
        let args = vec![
            "waylandd".to_string(),
            "--wire-test-socket".to_string(),
            "/run/user/1000/test.sock".to_string(),
        ];
        assert!(Config::from_args(args.into_iter().skip(1)).is_err());

        let args2 = vec![
            "waylandd".to_string(),
            "--wire-test-socket".to_string(),
            "/tmp/test.sock".to_string(),
        ];
        assert!(Config::from_args(args2.into_iter().skip(1)).is_ok());
    }

    #[test]
    fn canonical_scene_order_is_deterministic_and_assigns_z_order() {
        let make_surface = |id: &str, creation_sequence: u64| SurfaceSnapshot {
            id: id.into(),
            placement: SurfacePlacement { z: 0, visible: true, ..Default::default() },
            creation_sequence,
            ..Default::default()
        };
        let ordered = super::order_scene_surfaces(vec![
            make_surface("client-2-surface-4", 2),
            make_surface("client-1-surface-7", 1),
        ]);
        assert_eq!(
            ordered.iter().map(|surface| surface.id.as_str()).collect::<Vec<_>>(),
            vec!["client-1-surface-7", "client-2-surface-4"]
        );
        assert_eq!(ordered[0].placement.z, 0);
        assert_eq!(ordered[1].placement.z, 1);

        let reversed = super::order_scene_surfaces(vec![
            make_surface("client-1-surface-7", 1),
            make_surface("client-2-surface-4", 2),
        ]);
        assert_eq!(
            reversed.iter().map(|surface| surface.id.as_str()).collect::<Vec<_>>(),
            ordered.iter().map(|surface| surface.id.as_str()).collect::<Vec<_>>()
        );
    }
}
