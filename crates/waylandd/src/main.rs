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
    DisplayCommand, DisplayEvent, FocusTarget, ImeBridgeMode, ImeCommand, ImeEvent, ImeStatus,
    IpcEnvelope, MessageKind, OutputMode, ServiceBanner, ServiceEndpoint, ServiceRole,
    ServiceStream, SurfaceRegistrySnapshot, WaylandCommand, WaylandEvent, WaylandSelectionHandoff,
    WaylandSelectionState, WaylandSurfaceRole, WaylandSurfaceState, bind_service_socket,
    connect_service_socket, ensure_runtime_dir, now_unix_timestamp, read_json_line, send_json_line,
    session_artifact_path,
};

fn run_wire_headless_test() -> Result<()> {
    use byteorder::{ByteOrder, LittleEndian};
    use wayland_wire::{WaylandMessage, WaylandObjectId, WaylandOpcode, core::HeadlessWireCore};

    println!("service=waylandd op=wire_headless_test event=begin");
    let mut core = HeadlessWireCore::default();

    // Simulate Client: wl_display.get_registry(new_id=2)
    let mut payload = vec![0u8; 4];
    LittleEndian::write_u32(&mut payload[0..4], 2);
    let msg = WaylandMessage::new(WaylandObjectId::DISPLAY, WaylandOpcode(1), payload);

    println!(
        "service=waylandd op=wire_headless_test event=dispatch_request object_id=1 opcode=1 info=\"get_registry\""
    );
    core.dispatch(msg).map_err(|e| anyhow::anyhow!(e))?;

    // Check events
    let mut event_count = 0;
    while let Some(ev) = core.pop_event() {
        println!(
            "service=waylandd op=wire_headless_test event=pop_event object_id={} opcode={} size={}",
            ev.header.object_id.0, ev.header.opcode.0, ev.header.size
        );
        event_count += 1;
    }

    if event_count == 3 {
        println!(
            "service=waylandd op=wire_headless_test event=success info=\"received 3 global advertisements\""
        );
    } else {
        bail!("expected 3 global events, got {}", event_count);
    }

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

    if config.wire_headless_test {
        run_wire_headless_test()?;
        return Ok(());
    }

    if config.serve_ipc {
        let mut registry = load_surface_registry(config.registry_path.as_ref())?;
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
    wire_headless_test: bool,
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
                "--wire-headless-test" => config.wire_headless_test = true,
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
                        "usage: waylandd [--require-displayd] [--serve-ipc] [--once] [--print-registry] [--registry PATH] [--bind-wayland-display NAME] [--vulkan] [--session-instance-id ID] [--wire-headless-test]"
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
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let (response, registry_changed) =
        build_response(request, registry, ime_state, ime_backend, data_payloads, vulkan, config)
            .await;
    send_json_line(&mut stream, &response)?;
    if registry_changed {
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
        assert_eq!(state.preedit_active, true);

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
        assert_eq!(state.preedit_active, false);
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
}
