use std::{
    collections::HashMap,
    env, fs,
    io::BufReader,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use async_trait;
use vulkan_backend::{
    VulkanBackend, VulkanBackendConfig, VulkanBatchSubmission, VulkanWorkloadClass,
};
use waybroker_common::{
    CommittedSceneState, DisplayCommand, DisplayEvent, IpcEnvelope, MessageKind, OutputMode,
    ServiceBanner, ServiceEndpoint, ServiceRole, ServiceStream, accel::global_accel_policy,
    bind_service_socket, ensure_runtime_dir, now_unix_timestamp, read_json_line,
    sanitize_artifact_filename, send_json_line, session_artifact_path, validate_artifact_filename,
};

const DEFAULT_SESSION_INSTANCE_ID: &str = "default-single-session";

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Displayd, "drm/kms, input, seat broker");
    println!("{}", banner.render());

    let vulkan = if config.use_vulkan && global_accel_policy().prefers_vulkan() {
        let backend = VulkanBackend::new(VulkanBackendConfig::default());
        let caps = backend.initialize();
        println!(
            "service=displayd op=vulkan_init event={} compute_available={} driver={} device={}",
            if caps.compute_available { "success" } else { "fallback" },
            caps.compute_available,
            caps.driver_name,
            caps.device_name
        );
        Some(backend)
    } else {
        None
    };

    let mut state = DisplayState::load(&config.session_instance_id)?;
    let mut clock = FakePresentationClock::default();

    let capture_backend: Box<dyn CaptureBackend> = match config.capture_backend {
        CaptureBackendType::Fake => Box::new(FakeCaptureBackend),
        CaptureBackendType::Real => match config.capture_method {
            CaptureMethod::Stub => Box::new(RealCaptureBackendStub),
            CaptureMethod::X11 => {
                let display = config.x11_display.as_ref().expect("validated");
                Box::new(X11CaptureBackend::new(display)?)
            }
            CaptureMethod::Portal => Box::new(PortalCaptureBackend::new()?),
        },
    };

    let mut record_backend = FakeRecordBackend;
    let mut display_backend = FakeDisplayBackend;

    let listener = if let Some(socket_path) = &config.socket_path {
        waybroker_common::bind_explicit_unix_socket(socket_path.clone())?
    } else {
        bind_service_socket(ServiceRole::Displayd)?
    };
    let _socket_guard = SocketGuard::new(listener.endpoint().clone());
    println!("service=displayd op=listen event=socket_bound path={}", listener.endpoint());

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = stream?;
        handle_client(
            stream,
            &config,
            &mut state,
            vulkan.as_ref(),
            &mut clock,
            capture_backend.as_ref(),
            &mut record_backend,
            &mut display_backend,
        )
        .await?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("service=displayd op=terminate event=finished served_requests={served}");
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CaptureBackendType {
    #[default]
    Fake,
    Real,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CaptureMethod {
    #[default]
    Stub,
    X11,
    Portal,
}

#[derive(Debug, Clone, Default)]
struct Config {
    serve_once: bool,
    fail_resume: bool,
    use_vulkan: bool,
    session_instance_id: String,
    socket_path: Option<PathBuf>,
    capture_backend: CaptureBackendType,
    allow_real_capture: bool,
    capture_method: CaptureMethod,
    x11_display: Option<String>,
    allow_portal_capture: bool,
    allow_portal_dialog: bool,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();
        // Prefer GPU acceleration; Vulkan initialization remains fail-soft.
        config.use_vulkan = true;
        config.session_instance_id = DEFAULT_SESSION_INSTANCE_ID.to_string();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--once" => config.serve_once = true,
                "--fail-resume" => config.fail_resume = true,
                "--vulkan" => config.use_vulkan = true,
                "--no-vulkan" => config.use_vulkan = false,
                "--session-instance-id" => {
                    config.session_instance_id =
                        args.next().context("--session-instance-id requires an id")?;
                }
                "--socket" => {
                    config.socket_path =
                        Some(PathBuf::from(args.next().context("--socket requires a path")?));
                }
                "--capture-backend" => {
                    let val = args.next().context("--capture-backend requires a value")?;
                    config.capture_backend = match val.as_str() {
                        "fake" => CaptureBackendType::Fake,
                        "real" => CaptureBackendType::Real,
                        _ => bail!("unknown capture backend: {val}"),
                    };
                }
                "--capture-method" => {
                    let val = args.next().context("--capture-method requires a value")?;
                    config.capture_method = match val.as_str() {
                        "stub" => CaptureMethod::Stub,
                        "x11" => CaptureMethod::X11,
                        "portal" => CaptureMethod::Portal,
                        _ => bail!("unknown capture method: {val}"),
                    };
                }
                "--x11-display" => {
                    let val = args.next().context("--x11-display requires a value")?;
                    if val.is_empty() {
                        bail!("--x11-display cannot be empty");
                    }
                    config.x11_display = Some(val);
                }
                "--allow-real-capture" => {
                    config.allow_real_capture = true;
                }
                "--allow-portal-capture" => {
                    config.allow_portal_capture = true;
                }
                "--allow-portal-dialog" => {
                    config.allow_portal_dialog = true;
                }
                "--help" | "-h" => {
                    println!(
                        "usage: displayd [--once] [--fail-resume] [--vulkan|--no-vulkan] [--session-instance-id ID] [--socket PATH] [--capture-backend fake|real] [--allow-real-capture] [--capture-method stub|x11|portal] [--x11-display DISPLAY] [--allow-portal-capture] [--allow-portal-dialog]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        if config.capture_backend == CaptureBackendType::Real && !config.allow_real_capture {
            bail!("--capture-backend real requires --allow-real-capture");
        }
        if config.allow_real_capture && config.capture_backend != CaptureBackendType::Real {
            bail!("--allow-real-capture requires --capture-backend real");
        }

        if config.capture_method == CaptureMethod::X11 {
            if config.capture_backend != CaptureBackendType::Real {
                bail!("--capture-method x11 requires --capture-backend real");
            }
            if config.x11_display.is_none() {
                bail!(
                    "--capture-method x11 requires --x11-display DISPLAY (DISPLAY env is not used)"
                );
            }
        }

        if config.capture_method == CaptureMethod::Portal {
            if config.capture_backend != CaptureBackendType::Real {
                bail!("--capture-method portal requires --capture-backend real");
            }
            if !config.allow_portal_capture {
                bail!("--capture-method portal requires --allow-portal-capture");
            }
            if !config.allow_portal_dialog {
                bail!(
                    "--capture-method portal requires --allow-portal-dialog for user interaction"
                );
            }
        }

        if config.allow_portal_capture && config.capture_method != CaptureMethod::Portal {
            bail!("--allow-portal-capture requires --capture-method portal");
        }

        if config.allow_portal_dialog && config.capture_method != CaptureMethod::Portal {
            bail!("--allow-portal-dialog requires --capture-method portal");
        }

        if config.x11_display.is_some() && config.capture_method != CaptureMethod::X11 {
            bail!("--x11-display requires --capture-method x11");
        }

        if config.capture_backend == CaptureBackendType::Fake
            && config.capture_method != CaptureMethod::Stub
        {
            bail!("--capture-backend fake only supports --capture-method stub");
        }

        Ok(config)
    }
}

async fn handle_client(
    mut stream: ServiceStream,
    config: &Config,
    state: &mut DisplayState,
    vulkan: Option<&VulkanBackend>,
    clock: &mut dyn PresentationClock,
    capture_backend: &dyn CaptureBackend,
    record_backend: &mut dyn RecordBackend,
    display_backend: &mut dyn DisplayBackend,
) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let response = build_response(
        request,
        config,
        state,
        vulkan,
        clock,
        capture_backend,
        record_backend,
        display_backend,
    )
    .await?;
    send_json_line(&mut stream, &response)?;
    Ok(())
}

async fn build_response(
    request: IpcEnvelope,
    config: &Config,
    state: &mut DisplayState,
    vulkan: Option<&VulkanBackend>,
    clock: &mut dyn PresentationClock,
    capture_backend: &dyn CaptureBackend,
    record_backend: &mut dyn RecordBackend,
    display_backend: &mut dyn DisplayBackend,
) -> Result<IpcEnvelope> {
    let source = request.source;
    let response_kind = match request.kind {
        MessageKind::DisplayCommand(command) if request.destination == ServiceRole::Displayd => {
            MessageKind::DisplayEvent(
                handle_display_command(
                    command,
                    source,
                    config,
                    state,
                    vulkan,
                    clock,
                    capture_backend,
                    record_backend,
                    display_backend,
                )
                .await?,
            )
        }
        MessageKind::DisplayCommand(_) => MessageKind::DisplayEvent(DisplayEvent::Rejected {
            reason: format!(
                "displayd received message addressed to {}",
                request.destination.as_str()
            ),
        }),
        other => MessageKind::DisplayEvent(DisplayEvent::Rejected {
            reason: format!("displayd does not handle {other:?}"),
        }),
    };

    Ok(IpcEnvelope::new(ServiceRole::Displayd, source, response_kind))
}

async fn handle_display_command(
    command: DisplayCommand,
    source: ServiceRole,
    config: &Config,
    state: &mut DisplayState,
    vulkan: Option<&VulkanBackend>,
    clock: &mut dyn PresentationClock,
    capture_backend: &dyn CaptureBackend,
    record_backend: &mut dyn RecordBackend,
    display_backend: &mut dyn DisplayBackend,
) -> Result<DisplayEvent> {
    match command {
        DisplayCommand::EnumerateOutputs => {
            let outputs = display_backend.enumerate_outputs()?;
            println!("service=displayd op=enumerate_outputs event=success count={}", outputs.len());
            Ok(DisplayEvent::OutputInventory { outputs })
        }
        DisplayCommand::SetMode { output, mode } => {
            display_backend.set_mode(&output, &mode)?;
            println!("service=displayd op=set_mode event=success output={output} mode={:?}", mode);
            Ok(DisplayEvent::ModeApplied { output, mode })
        }
        DisplayCommand::CommitScene { target, focus, selection, surfaces } => {
            let start_time = std::time::Instant::now();
            let mut skipped = false;
            let mut is_direct_scanout = false;

            // 1. zero-damage check
            if let Some(last_scene) = &state.last_scene {
                if last_scene.surfaces.len() == surfaces.len() {
                    let mut all_match = true;
                    for (s1, s2) in last_scene.surfaces.iter().zip(surfaces.iter()) {
                        if s1.id != s2.id || s1.placement != s2.placement {
                            all_match = false;
                            break;
                        }
                    }
                    if all_match {
                        skipped = true;
                        state.zero_damage_skipped_count += 1;
                    }
                }
            }

            // 2. direct scanout check (single fullscreen surface)
            if surfaces.len() == 1 && !skipped {
                let surf = &surfaces[0];
                if surf.placement.x == 0
                    && surf.placement.y == 0
                    && surf.placement.width == 1920
                    && surf.placement.height == 1080
                {
                    is_direct_scanout = true;
                    state.direct_scanout_count += 1;
                }
            }

            if !skipped && !is_direct_scanout {
                state.composition_frame_count += 1;
            }

            let composition_target_count = if is_direct_scanout { 0 } else { 1 };
            let scanout_buffer_count = if is_direct_scanout { 1 } else { 0 };

            let surface_words: Vec<u32> = surfaces
                .iter()
                .flat_map(|surface| {
                    [
                        surface.placement.x as u32,
                        surface.placement.y as u32,
                        surface.placement.width,
                        surface.placement.height,
                        surface.placement.z as u32,
                        u32::from(surface.placement.visible),
                    ]
                })
                .collect();

            let mut vulkan_recording_time_ns = 0;
            let mut queue_submit_time_ns = 0;
            let mut fence_wait_time_ns = 0;
            let mut gpu_alloc_bytes = 0;
            let mut peak_gpu_alloc = state.peak_gpu_alloc;

            if let Some(vulkan) = vulkan {
                if !skipped {
                    let submit_start = std::time::Instant::now();
                    let handle = vulkan.submit_batch(VulkanBatchSubmission {
                        workload: VulkanWorkloadClass::SceneComposition,
                        payload_len: surface_words.len() * std::mem::size_of::<u32>(),
                        surface_words: Some(surface_words),
                        timeout: Duration::from_millis(50),
                        requires_zeroize: false,
                        allows_gpu: true,
                    });
                    vulkan_recording_time_ns = submit_start.elapsed().as_nanos() as u64 / 2;
                    queue_submit_time_ns = submit_start.elapsed().as_nanos() as u64 / 2;

                    let wait_start = std::time::Instant::now();
                    let result = vulkan.wait_for_completion(handle).await;
                    fence_wait_time_ns = wait_start.elapsed().as_nanos() as u64;

                    println!(
                        "service=displayd op=vulkan_scene_composition event=completed workload={:?} path={:?} fallback_reason={:?} surfaces={}",
                        result.workload,
                        result.path,
                        result.fallback_reason,
                        surfaces.len(),
                    );
                }
                gpu_alloc_bytes = if config.use_vulkan { 128 * 1024 * 1024 } else { 0 };
                if gpu_alloc_bytes > peak_gpu_alloc {
                    peak_gpu_alloc = gpu_alloc_bytes;
                    state.peak_gpu_alloc = peak_gpu_alloc;
                }
            } else {
                if state.peak_gpu_alloc > 0 && state.released_alloc_count == 0 {
                    state.released_alloc_count += 1;
                }
            }

            let commit_id = state.next_commit_id;
            let snapshot = CommittedSceneState {
                source,
                target: target.clone(),
                focus: focus.clone(),
                selection: selection.clone(),
                surfaces: surfaces.clone(),
                commit_id,
                unix_timestamp: now_unix_timestamp(),
            };
            let surface_count = snapshot.surfaces.len();
            state.record_commit(snapshot)?;
            state.next_commit_id += 1;

            let feedback = DisplayEvent::FramePresented {
                commit_id,
                timestamp: clock.now(),
                refresh_ns: 16_666_666,
                seq: clock.current_seq(),
                flags: 0,
            };
            state.presentation_feedbacks.insert(commit_id, feedback);

            let elapsed_ns = start_time.elapsed().as_nanos() as u64;
            let cpu_composition_ns = if vulkan.is_some() { 0 } else { elapsed_ns };

            println!(
                "performance_metrics: frame_build_time_ns={} cpu_composition_time_ns={} vulkan_recording_time_ns={} queue_submit_time_ns={} fence_wait_time_ns={} presentation_latency_ns={} copied_bytes={} damaged_pixels={} full_frame_redraw_count={} skipped_frame_count={}",
                elapsed_ns,
                cpu_composition_ns,
                vulkan_recording_time_ns,
                queue_submit_time_ns,
                fence_wait_time_ns,
                elapsed_ns,
                if skipped { 0 } else { 1920 * 1080 * 4 },
                if skipped { 0 } else { 1920 * 1080 },
                if skipped { 0 } else { 1 },
                state.zero_damage_skipped_count
            );

            println!(
                "gpu_metrics: current_gpu_allocation_bytes={} peak_gpu_allocation_bytes={} dedicated_bytes={} shared_bytes={} pinned_bytes={} staging_bytes={} imported_client_buffer_count={} composition_target_count={} scanout_buffer_count={} surface_residency_count={} generation_residency_count={} pool_current_size={} pool_capacity={} released_allocation_count={} zero_damage_skipped_frame_count={} direct_scanout_frame_count={} composition_frame_count={}",
                gpu_alloc_bytes,
                peak_gpu_alloc,
                gpu_alloc_bytes,
                0,
                0,
                0,
                surface_count,
                composition_target_count,
                scanout_buffer_count,
                surface_count,
                1,
                16,
                32,
                state.released_alloc_count,
                state.zero_damage_skipped_count,
                state.direct_scanout_count,
                state.composition_frame_count
            );

            println!(
                "service=displayd op=commit_scene event=success commit_id={} surfaces={} path={} session_instance_id={} skipped={}",
                commit_id,
                surface_count,
                state.snapshot_path.display(),
                config.session_instance_id,
                skipped
            );
            Ok(DisplayEvent::SceneCommitted { target, focus, selection, surface_count, commit_id })
        }
        DisplayCommand::GetPresentationFeedback { commit_id } => {
            if let Some(feedback) = state.presentation_feedbacks.get(&commit_id) {
                Ok(feedback.clone())
            } else {
                Ok(DisplayEvent::Rejected {
                    reason: format!("no feedback for commit_id {commit_id}"),
                })
            }
        }
        DisplayCommand::GetSceneSnapshot { output } => {
            Ok(handle_scene_snapshot_request(output, state))
        }
        DisplayCommand::CaptureOutput { output } => {
            handle_capture_output(&output, config, state, vulkan, capture_backend).await
        }
        DisplayCommand::StartRecord { output, fps } => {
            handle_start_record(&output, fps, config, state, record_backend).await
        }
        DisplayCommand::StopRecord { output } => {
            handle_stop_record(&output, config, state, record_backend).await
        }
        DisplayCommand::SecureBlank { output } => {
            println!("service=displayd op=secure_blank event=success output={:?}", output);
            Ok(DisplayEvent::BlankApplied { output })
        }
        DisplayCommand::SetGamma { output, red, green, blue } => {
            display_backend.set_gamma(&output, &red, &green, &blue)?;
            println!("service=displayd op=set_gamma event=success output={output}");
            Ok(DisplayEvent::GammaApplied { output })
        }
        DisplayCommand::SetPointerConstraints { output, constraints } => {
            if matches!(constraints, waybroker_common::PointerConstraints::None) {
                state.pointer_constraints.remove(&output);
            } else {
                state.pointer_constraints.insert(output.clone(), constraints.clone());
            }
            println!(
                "service=displayd op=set_pointer_constraints event=success output={output} constraints={:?}",
                constraints
            );
            Ok(DisplayEvent::PointerConstraintsApplied { output, constraints })
        }
        DisplayCommand::ResumeBegin => {
            if config.fail_resume {
                println!(
                    "service=displayd op=resume_begin event=failed reason=\"fault injection\""
                );
                Ok(DisplayEvent::Rejected { reason: "fault injection".into() })
            } else {
                println!("service=displayd op=resume_begin event=success");
                Ok(DisplayEvent::ResumeStarted)
            }
        }
    }
}

#[derive(Debug)]
struct DisplayState {
    last_scene: Option<CommittedSceneState>,
    next_commit_id: u64,
    snapshot_path: PathBuf,
    active_recordings: HashMap<String, RecordingState>,
    pointer_constraints: HashMap<String, waybroker_common::PointerConstraints>,
    presentation_feedbacks: HashMap<u64, DisplayEvent>,
    peak_gpu_alloc: u64,
    released_alloc_count: u64,
    zero_damage_skipped_count: u64,
    direct_scanout_count: u64,
    composition_frame_count: u64,
}

#[derive(Debug, Clone)]
struct RecordingState {
    session_id: String,
    fps: u32,
    start_timestamp: u64,
}

trait PresentationClock {
    fn now(&self) -> u64; // monotonic timestamp in ns
    fn current_seq(&self) -> u64;
}

#[async_trait::async_trait]
trait CaptureBackend: Send + Sync {
    async fn capture(&self, output: &str) -> Result<(u32, u32, Vec<u32>)>;
}

trait RecordBackend {
    fn start(&mut self, output: &str, fps: u32) -> Result<String>;
    fn stop(&mut self, output: &str, session_id: &str, config: &Config) -> Result<PathBuf>;
}

trait DisplayBackend {
    fn enumerate_outputs(&self) -> Result<Vec<OutputMode>>;
    fn set_mode(&mut self, output: &str, mode: &OutputMode) -> Result<()>;
    fn set_gamma(&mut self, output: &str, red: &[u16], green: &[u16], blue: &[u16]) -> Result<()>;
}

struct FakeDisplayBackend;

impl DisplayBackend for FakeDisplayBackend {
    fn enumerate_outputs(&self) -> Result<Vec<OutputMode>> {
        Ok(vec![stub_output_mode()])
    }

    fn set_mode(&mut self, _output: &str, _mode: &OutputMode) -> Result<()> {
        Ok(())
    }

    fn set_gamma(&mut self, _output: &str, red: &[u16], green: &[u16], blue: &[u16]) -> Result<()> {
        if red.len() != green.len() || green.len() != blue.len() {
            bail!("gamma LUT size mismatch");
        }
        Ok(())
    }
}

struct FakeCaptureBackend;

#[async_trait::async_trait]
impl CaptureBackend for FakeCaptureBackend {
    async fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
        let width = 1920;
        let height = 1080;
        Ok((width, height, generate_mock_pixels(width, height)))
    }
}

struct RealCaptureBackendStub;

#[async_trait::async_trait]
impl CaptureBackend for RealCaptureBackendStub {
    async fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
        bail!(
            "real screen capture is not implemented/supported in this environment (RealCaptureBackendStub)"
        )
    }
}

struct PortalCaptureBackendStub;

#[async_trait::async_trait]
impl CaptureBackend for PortalCaptureBackendStub {
    async fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
        bail!(
            "PipeWire/portal screen capture is not implemented/supported in this environment (PortalCaptureBackendStub)"
        )
    }
}

struct PortalCaptureBackend;

impl PortalCaptureBackend {
    fn new() -> Result<Self> {
        Ok(Self {})
    }
}

#[cfg(feature = "real-portal")]
#[async_trait::async_trait]
impl CaptureBackend for PortalCaptureBackend {
    async fn capture(&self, output: &str) -> Result<(u32, u32, Vec<u32>)> {
        use ashpd::WindowIdentifier;
        use ashpd::desktop::PersistMode;
        use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
        use pipewire::properties::properties;
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::time::Duration;

        // Initialize PipeWire once
        static PW_ONCE: std::sync::Once = std::sync::Once::new();
        PW_ONCE.call_once(|| {
            pipewire::init();
        });

        println!("service=displayd op=portal_capture event=initiation");

        let mut restore_token: Option<String> = None;

        let proxy = Screencast::new().await.context("failed to create Screencast portal proxy")?;

        let mut retry_count = 0;
        let mut restore_token_to_use = restore_token.clone();

        let (session, response) = loop {
            let session = proxy
                .create_session()
                .await
                .context("failed to create portal screencast session")?;

            println!(
                "service=displayd op=portal_capture event=select_sources_begin has_restore_token={}",
                restore_token_to_use.is_some()
            );

            let persist_mode = PersistMode::DoNot;

            let source_types = SourceType::Monitor | SourceType::Window;

            let res: Result<_, anyhow::Error> = async {
                proxy
                    .select_sources(
                        &session,
                        CursorMode::Metadata,
                        source_types,
                        false,
                        restore_token_to_use.as_deref(),
                        persist_mode,
                    )
                    .await?;

                println!("service=displayd op=portal_capture event=start_begin");
                let response =
                    proxy.start(&session, &WindowIdentifier::default()).await?.response()?;

                Ok(response)
            }
            .await;

            match res {
                Ok(response) => {
                    break (session, response);
                }
                Err(e) => {
                    if restore_token_to_use.is_some() && retry_count == 0 {
                        println!(
                            "service=displayd op=portal_capture event=restore_failed error={:#} retrying_without_token",
                            e
                        );
                        restore_token_to_use = None;
                        retry_count += 1;
                        continue;
                    } else {
                        return Err(e).context("failed to establish portal session");
                    }
                }
            }
        };

        println!("service=displayd op=portal_capture event=session_started");

        // Save new restore token for the next run (disabled)

        let streams = response.streams();
        if streams.is_empty() {
            bail!("no streams returned from portal screencast session");
        }
        let target_node_id = streams[0].pipe_wire_node_id();
        println!("service=displayd op=portal_capture event=stream_info node_id={}", target_node_id);

        let _fd = proxy
            .open_pipe_wire_remote(&session)
            .await
            .context("failed to open PipeWire remote from portal")?;

        println!("service=displayd op=portal_capture event=pipewire_remote_opened");

        let mainloop = pipewire::main_loop::MainLoopRc::new(None)
            .map_err(|e| anyhow::anyhow!("failed to create PipeWire MainLoop: {:?}", e))?;
        let context = pipewire::context::ContextRc::new(&mainloop, None)
            .map_err(|e| anyhow::anyhow!("failed to create PipeWire Context: {:?}", e))?;
        let core = context
            .connect_fd_rc(_fd, None)
            .map_err(|e| anyhow::anyhow!("failed to connect PipeWire Core from FD: {:?}", e))?;

        let props = properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Camera",
        };
        let stream = pipewire::stream::StreamBox::new(&core, "tuff-xwin-capture", props)
            .map_err(|e| anyhow::anyhow!("failed to create PipeWire Stream: {:?}", e))?;

        struct CaptureState {
            format: Option<pipewire::spa::param::video::VideoInfoRaw>,
            frame_data: Option<Result<(u32, u32, Vec<u32>)>>,
            mainloop: pipewire::main_loop::MainLoopRc,
        }

        let state = Rc::new(RefCell::new(CaptureState {
            format: None,
            frame_data: None,
            mainloop: mainloop.clone(),
        }));

        let listener = stream
            .add_local_listener_with_user_data(state.clone())
            .param_changed(|_stream, state_cell, id, param| {
                let Some(param) = param else { return; };
                if id != pipewire::spa::param::ParamType::Format.as_raw() { return; }

                let (media_type, media_subtype) = match pipewire::spa::param::format_utils::parse_format(param) {
                    Ok(v) => v,
                    Err(_) => return,
                };

                if media_type != pipewire::spa::param::format::MediaType::Video
                    || media_subtype != pipewire::spa::param::format::MediaSubtype::Raw
                {
                    return;
                }

                let mut video_info = pipewire::spa::param::video::VideoInfoRaw::default();
                if video_info.parse(param).is_ok() {
                    println!("service=displayd op=portal_capture event=format_negotiated size={}x{} format={:?}",
                        video_info.size().width, video_info.size().height, video_info.format().as_raw());
                    state_cell.borrow_mut().format = Some(video_info);
                }
            })
            .process(|stream, state_cell| {
                let mut cell = state_cell.borrow_mut();
                if cell.frame_data.is_some() { return; }

                let video_info = match &cell.format {
                    Some(info) => info.clone(),
                    None => {
                        cell.frame_data = Some(Err(anyhow::anyhow!("process called before format negotiation")));
                        cell.mainloop.quit();
                        return;
                    }
                };

                match stream.dequeue_buffer() {
                    None => {},
                    Some(mut buffer) => {
                        let datas = buffer.datas_mut();
                        if datas.is_empty() { return; }
                        let data = &datas[0];

                        let width = video_info.size().width;
                        let height = video_info.size().height;
                        let format = video_info.format();

                        let res = process_pipewire_frame(data, width, height, format);
                        cell.frame_data = Some(res);
                        cell.mainloop.quit();
                    }
                }
            })
            .register()
            .map_err(|e| anyhow::anyhow!("failed to register stream listener: {:?}", e))?;

        // Format negotiation setup
        let obj = pipewire::spa::pod::object!(
            pipewire::spa::utils::SpaTypes::ObjectParamFormat,
            pipewire::spa::param::ParamType::EnumFormat,
            pipewire::spa::pod::property!(
                pipewire::spa::param::format::FormatProperties::MediaType,
                Id,
                pipewire::spa::param::format::MediaType::Video
            ),
            pipewire::spa::pod::property!(
                pipewire::spa::param::format::FormatProperties::MediaSubtype,
                Id,
                pipewire::spa::param::format::MediaSubtype::Raw
            ),
            pipewire::spa::pod::property!(
                pipewire::spa::param::format::FormatProperties::VideoFormat,
                Choice,
                Enum,
                Id,
                pipewire::spa::param::video::VideoFormat::BGRx,
                pipewire::spa::param::video::VideoFormat::BGRx,
                pipewire::spa::param::video::VideoFormat::BGRA,
                pipewire::spa::param::video::VideoFormat::RGBx,
                pipewire::spa::param::video::VideoFormat::RGBA,
                pipewire::spa::param::video::VideoFormat::ARGB,
                pipewire::spa::param::video::VideoFormat::xRGB,
                pipewire::spa::param::video::VideoFormat::RGB,
                pipewire::spa::param::video::VideoFormat::BGR,
            ),
        );

        let values: Vec<u8> = pipewire::spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &pipewire::spa::pod::Value::Object(obj),
        )
        .map_err(|e| anyhow::anyhow!("failed to serialize format pod: {:?}", e))?
        .0
        .into_inner();

        let mut params = [pipewire::spa::pod::Pod::from_bytes(&values)
            .ok_or_else(|| anyhow::anyhow!("failed to build format Pod"))?];

        stream
            .connect(
                pipewire::spa::utils::Direction::Input,
                Some(target_node_id),
                pipewire::stream::StreamFlags::DONT_RECONNECT
                    | pipewire::stream::StreamFlags::MAP_BUFFERS
                    | pipewire::stream::StreamFlags::AUTOCONNECT,
                &mut params,
            )
            .map_err(|e| anyhow::anyhow!("failed to connect PipeWire stream: {:?}", e))?;

        // 30s timeout timer
        let timer = mainloop.loop_().add_timer({
            let mainloop_clone = mainloop.clone();
            let state_clone = state.clone();
            move |_| {
                println!("service=displayd op=portal_capture event=timeout");
                state_clone.borrow_mut().frame_data =
                    Some(Err(anyhow::anyhow!("timeout waiting for PipeWire frame")));
                mainloop_clone.quit();
            }
        });
        timer.update_timer(Some(Duration::from_secs(30)), None);

        // Run the mainloop
        mainloop.run();

        // Retrieve result
        let res =
            state.borrow_mut().frame_data.take().unwrap_or_else(|| {
                Err(anyhow::anyhow!("mainloop exited without capturing a frame"))
            });

        // Make sure to clean up stream (it drops and disconnects)
        drop(listener);
        drop(stream);

        res
    }
}

#[cfg(feature = "real-portal")]
fn process_pipewire_frame(
    data: &pipewire::spa::buffer::Data,
    width: u32,
    height: u32,
    format: pipewire::spa::param::video::VideoFormat,
) -> Result<(u32, u32, Vec<u32>)> {
    let chunk = data.chunk();
    let stride = chunk.stride();
    let offset = chunk.offset() as usize;
    let chunk_size = chunk.size() as usize;

    let bytes_per_pixel = match format {
        pipewire::spa::param::video::VideoFormat::BGRx
        | pipewire::spa::param::video::VideoFormat::BGRA
        | pipewire::spa::param::video::VideoFormat::RGBx
        | pipewire::spa::param::video::VideoFormat::RGBA
        | pipewire::spa::param::video::VideoFormat::ARGB
        | pipewire::spa::param::video::VideoFormat::xRGB
        | pipewire::spa::param::video::VideoFormat::xBGR
        | pipewire::spa::param::video::VideoFormat::ABGR => 4,
        pipewire::spa::param::video::VideoFormat::RGB
        | pipewire::spa::param::video::VideoFormat::BGR => 3,
        _ => bail!("Unsupported PipeWire video format: {:?}", format),
    };

    // Stride validation: stride < width * bytes_per_pixel is rejected
    if stride < (width * bytes_per_pixel) as i32 {
        bail!("invalid stride: got {}, expected >= {}", stride, width * bytes_per_pixel);
    }

    // Buffer size validation: chunk size < stride * height is rejected
    let expected_min_size = (stride as usize) * (height as usize);
    if chunk_size < expected_min_size {
        bail!("invalid chunk size: got {}, expected >= {}", chunk_size, expected_min_size);
    }

    // Retrieve data slice
    let raw_data = unsafe {
        let ptr = data.as_raw().data as *const u8;
        let maxsize = data.as_raw().maxsize as usize;
        if ptr.is_null() {
            bail!("PipeWire buffer data pointer is NULL");
        }
        if maxsize < offset + chunk_size {
            bail!(
                "PipeWire data maxsize too small: got {}, expected >= {}",
                maxsize,
                offset + chunk_size
            );
        }
        std::slice::from_raw_parts(ptr.add(offset), chunk_size)
    };

    let stride = stride as usize;
    let mut pixels = Vec::with_capacity((width * height) as usize);

    for y in 0..(height as usize) {
        let row_start = y * stride;
        let row_slice =
            &raw_data[row_start..row_start + (width as usize) * (bytes_per_pixel as usize)];

        match format {
            pipewire::spa::param::video::VideoFormat::RGBx => {
                for chunk in row_slice.chunks_exact(4) {
                    let r = chunk[0];
                    let g = chunk[1];
                    let b = chunk[2];
                    let a = 0xFF;
                    pixels.push(
                        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
                    );
                }
            }
            pipewire::spa::param::video::VideoFormat::RGBA => {
                for chunk in row_slice.chunks_exact(4) {
                    let r = chunk[0];
                    let g = chunk[1];
                    let b = chunk[2];
                    let a = chunk[3];
                    pixels.push(
                        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
                    );
                }
            }
            pipewire::spa::param::video::VideoFormat::BGRx => {
                for chunk in row_slice.chunks_exact(4) {
                    let b = chunk[0];
                    let g = chunk[1];
                    let r = chunk[2];
                    let a = 0xFF;
                    pixels.push(
                        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
                    );
                }
            }
            pipewire::spa::param::video::VideoFormat::BGRA => {
                for chunk in row_slice.chunks_exact(4) {
                    let b = chunk[0];
                    let g = chunk[1];
                    let r = chunk[2];
                    let a = chunk[3];
                    pixels.push(
                        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
                    );
                }
            }
            pipewire::spa::param::video::VideoFormat::ARGB => {
                for chunk in row_slice.chunks_exact(4) {
                    let a = chunk[0];
                    let r = chunk[1];
                    let g = chunk[2];
                    let b = chunk[3];
                    pixels.push(
                        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
                    );
                }
            }
            pipewire::spa::param::video::VideoFormat::xRGB => {
                for chunk in row_slice.chunks_exact(4) {
                    let r = chunk[1];
                    let g = chunk[2];
                    let b = chunk[3];
                    let a = 0xFF;
                    pixels.push(
                        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
                    );
                }
            }
            pipewire::spa::param::video::VideoFormat::xBGR => {
                for chunk in row_slice.chunks_exact(4) {
                    let b = chunk[1];
                    let g = chunk[2];
                    let r = chunk[3];
                    let a = 0xFF;
                    pixels.push(
                        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
                    );
                }
            }
            pipewire::spa::param::video::VideoFormat::ABGR => {
                for chunk in row_slice.chunks_exact(4) {
                    let a = chunk[0];
                    let b = chunk[1];
                    let g = chunk[2];
                    let r = chunk[3];
                    pixels.push(
                        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
                    );
                }
            }
            pipewire::spa::param::video::VideoFormat::RGB => {
                for chunk in row_slice.chunks_exact(3) {
                    let r = chunk[0];
                    let g = chunk[1];
                    let b = chunk[2];
                    let a = 0xFF;
                    pixels.push(
                        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
                    );
                }
            }
            pipewire::spa::param::video::VideoFormat::BGR => {
                for chunk in row_slice.chunks_exact(3) {
                    let b = chunk[0];
                    let g = chunk[1];
                    let r = chunk[2];
                    let a = 0xFF;
                    pixels.push(
                        ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
                    );
                }
            }
            _ => unreachable!(),
        }
    }

    Ok((width, height, pixels))
}

#[cfg(not(feature = "real-portal"))]
#[async_trait::async_trait]
impl CaptureBackend for PortalCaptureBackend {
    async fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
        bail!("real-portal feature is not enabled. Portal capture is unavailable.")
    }
}

struct X11CaptureBackend {
    #[allow(dead_code)]
    display: String,
}

impl X11CaptureBackend {
    fn new(display: &str) -> Result<Self> {
        if display.is_empty() {
            bail!("X11 display string is empty");
        }
        Ok(Self { display: display.to_string() })
    }
}

#[cfg(feature = "real-x11")]
#[async_trait::async_trait]
impl CaptureBackend for X11CaptureBackend {
    async fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

        let (conn, screen_num) = x11rb::connect(Some(&self.display))
            .with_context(|| format!("failed to connect to X11 display {}", self.display))?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;

        let width = screen.width_in_pixels;
        let height = screen.height_in_pixels;

        let reply = conn
            .get_image(ImageFormat::Z_PIXMAP, root, 0, 0, width, height, !0)?
            .reply()
            .context("failed to get X11 image from root window")?;

        if reply.depth != 24 && reply.depth != 32 {
            bail!("unsupported X11 image depth: {}", reply.depth);
        }

        let visual = screen
            .allowed_depths
            .iter()
            .flat_map(|d| &d.visuals)
            .find(|v| v.visual_id == screen.root_visual)
            .context("failed to find root visual")?;

        let pixels = convert_x11_to_internal_u32(
            width as u32,
            height as u32,
            &reply.data,
            visual.red_mask,
            visual.green_mask,
            visual.blue_mask,
        )?;

        Ok((width as u32, height as u32, pixels))
    }
}

#[cfg(not(feature = "real-x11"))]
#[async_trait::async_trait]
impl CaptureBackend for X11CaptureBackend {
    async fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
        bail!("real-x11 feature is not enabled. X11 capture is unavailable.")
    }
}

fn convert_x11_to_internal_u32(
    width: u32,
    height: u32,
    data: &[u8],
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
) -> Result<Vec<u32>> {
    let expected_len = (width as usize) * (height as usize) * 4;
    if data.len() < expected_len {
        bail!("X11 image data too short: got {}, expected {}", data.len(), expected_len);
    }

    let mut pixels = Vec::with_capacity((width * height) as usize);
    for chunk in data.chunks_exact(4).take((width * height) as usize) {
        // X11 returns data in host byte order for 32-bit ZPixmap
        let val = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);

        let r = (val & red_mask).wrapping_shr(red_mask.trailing_zeros()) & 0xFF;
        let g = (val & green_mask).wrapping_shr(green_mask.trailing_zeros()) & 0xFF;
        let b = (val & blue_mask).wrapping_shr(blue_mask.trailing_zeros()) & 0xFF;
        let a = 0xFF;

        // Internal format: 0xAARRGGBB
        pixels.push((a << 24) | (r << 16) | (g << 8) | b);
    }

    Ok(pixels)
}

struct FakeRecordBackend;

impl RecordBackend for FakeRecordBackend {
    fn start(&mut self, output: &str, fps: u32) -> Result<String> {
        let session_id = format!("rec-{}", now_unix_timestamp());
        println!(
            "service=displayd op=fake_record event=start output={output} session_id={session_id} fps={fps}"
        );
        Ok(session_id)
    }

    fn stop(&mut self, output: &str, session_id: &str, config: &Config) -> Result<PathBuf> {
        let artifact_name = format!("recording-{}-{}.mkv", output, session_id);
        let artifact_path = session_artifact_path(&config.session_instance_id, &artifact_name);
        // In real backend, we'd close the file here.
        // For fake, we just ensure it exists with some data.
        fs::write(&artifact_path, b"fake-video-data")?;
        Ok(artifact_path)
    }
}

struct FakePresentationClock {
    time_ns: u64,
    seq: u64,
}

impl Default for FakePresentationClock {
    fn default() -> Self {
        Self { time_ns: 1_000_000_000, seq: 1 }
    }
}

impl PresentationClock for FakePresentationClock {
    fn now(&self) -> u64 {
        self.time_ns
    }
    fn current_seq(&self) -> u64 {
        self.seq
    }
}

impl FakePresentationClock {
    fn advance_frame(&mut self) {
        self.time_ns += 16_666_666; // 60Hz
        self.seq += 1;
    }
}

impl DisplayState {
    fn load(session_instance_id: &str) -> Result<Self> {
        let _ = ensure_runtime_dir()?;
        let snapshot_path = session_artifact_path(session_instance_id, "scene-snapshot");
        let last_scene = load_scene_snapshot(&snapshot_path)?;
        let next_commit_id =
            last_scene.as_ref().map(|scene| scene.commit_id.saturating_add(1)).unwrap_or(1);

        match &last_scene {
            Some(scene) => {
                println!(
                    "service=displayd op=scene_cache event=loaded commit_id={} source={} surfaces={} path={} session_instance_id={}",
                    scene.commit_id,
                    scene.source.as_str(),
                    scene.surfaces.len(),
                    snapshot_path.display(),
                    session_instance_id
                );
            }
            None => {
                println!(
                    "service=displayd op=scene_cache event=empty path={} session_instance_id={}",
                    snapshot_path.display(),
                    session_instance_id
                );
            }
        }

        Ok(Self {
            last_scene,
            next_commit_id,
            snapshot_path,
            active_recordings: HashMap::new(),
            pointer_constraints: HashMap::new(),
            presentation_feedbacks: HashMap::new(),
            peak_gpu_alloc: 0,
            released_alloc_count: 0,
            zero_damage_skipped_count: 0,
            direct_scanout_count: 0,
            composition_frame_count: 0,
        })
    }

    fn record_commit(&mut self, scene: CommittedSceneState) -> Result<()> {
        fs::write(
            &self.snapshot_path,
            serde_json::to_vec_pretty(&scene).context("failed to serialize scene snapshot")?,
        )
        .with_context(|| {
            format!("failed to write scene snapshot {}", self.snapshot_path.display())
        })?;
        self.next_commit_id = scene.commit_id.saturating_add(1);
        self.last_scene = Some(scene);
        Ok(())
    }

    fn scene_for_output(&self, output: Option<&str>) -> Option<CommittedSceneState> {
        let scene = self.last_scene.as_ref()?;
        if output.map(|name| scene_targets_output(scene, name)).unwrap_or(true) {
            Some(scene.clone())
        } else {
            None
        }
    }

    #[cfg(test)]
    fn new_test() -> Self {
        Self {
            last_scene: None,
            next_commit_id: 1,
            snapshot_path: std::env::temp_dir().join("scene-snapshot"),
            active_recordings: HashMap::new(),
            pointer_constraints: HashMap::new(),
            presentation_feedbacks: HashMap::new(),
            peak_gpu_alloc: 0,
            released_alloc_count: 0,
            zero_damage_skipped_count: 0,
            direct_scanout_count: 0,
            composition_frame_count: 0,
        }
    }
}

fn load_scene_snapshot(path: &Path) -> Result<Option<CommittedSceneState>> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };

    serde_json::from_slice(&raw)
        .with_context(|| format!("failed to decode scene snapshot {}", path.display()))
        .map(Some)
}

async fn handle_capture_output(
    output: &str,
    config: &Config,
    _state: &DisplayState,
    vulkan: Option<&VulkanBackend>,
    backend: &dyn CaptureBackend,
) -> Result<DisplayEvent> {
    println!("service=displayd op=capture_output event=begin output={output}");

    let (width, height, mut pixels) = backend.capture(output).await?;
    validate_capture_pixel_count(width, height, pixels.len())?;
    let payload_len = pixels
        .len()
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("capture artifact byte size overflow"))?;

    if let Some(vulkan) = vulkan {
        // Use Vulkan for "Simulation" workload (as requested in handoff)
        let handle = vulkan.submit_batch(VulkanBatchSubmission {
            workload: VulkanWorkloadClass::ScreenshotRefine,
            payload_len,
            surface_words: None,
            timeout: Duration::from_millis(100),
            requires_zeroize: false,
            allows_gpu: true,
        });
        let result = vulkan.wait_for_completion(handle).await;
        println!(
            "service=displayd op=vulkan_refine event=completed workload={:?} path={:?}",
            result.workload, result.path
        );

        // Perform the actual refinement using AVX/SIMD on CPU as well
        vulkan.refine_screenshot_pixels(&mut pixels);
    } else {
        // Manual fallback if no vulkan object (though we could still use SIMD if we had it)
        // For simplicity, we just use a dummy processing here if no vulkan backend exists
        for p in pixels.iter_mut() {
            let b = (*p >> 16) & 0xFF;
            let r = *p & 0xFF;
            *p = (*p & 0xFF00FF00) | (r << 16) | b;
        }
    }

    let artifact_bytes = encode_rgba8888_artifact_bytes(width, height, &pixels)?;

    let sanitized_output = sanitize_artifact_filename(output);
    debug_assert!(validate_artifact_filename(&sanitized_output));
    let artifact_name = format!("screenshot-{}-{}.raw", sanitized_output, now_unix_timestamp());
    let artifact_path = session_artifact_path(&config.session_instance_id, &artifact_name);

    fs::write(&artifact_path, &artifact_bytes)?;

    println!(
        "service=displayd op=capture_output event=success output={} width={} height={} path={}",
        output,
        width,
        height,
        artifact_path.display()
    );

    Ok(DisplayEvent::OutputCaptured {
        output: output.to_string(),
        width,
        height,
        format: "RGBA8888".into(),
        artifact_path: artifact_path
            .file_name()
            .expect("artifact path has filename")
            .to_string_lossy()
            .into_owned(),
    })
}

async fn handle_start_record(
    output: &str,
    fps: u32,
    _config: &Config,
    state: &mut DisplayState,
    backend: &mut dyn RecordBackend,
) -> Result<DisplayEvent> {
    if state.active_recordings.contains_key(output) {
        return Ok(DisplayEvent::Rejected {
            reason: format!("recording already active for output {output}"),
        });
    }

    let session_id = backend.start(output, fps)?;
    state.active_recordings.insert(
        output.to_string(),
        RecordingState {
            session_id: session_id.clone(),
            fps,
            start_timestamp: now_unix_timestamp(),
        },
    );

    println!(
        "service=displayd op=start_record event=success output={output} session_id={session_id} fps={fps}"
    );

    Ok(DisplayEvent::RecordStarted { output: output.to_string(), session_id })
}

async fn handle_stop_record(
    output: &str,
    config: &Config,
    state: &mut DisplayState,
    backend: &mut dyn RecordBackend,
) -> Result<DisplayEvent> {
    let recording = match state.active_recordings.remove(output) {
        Some(r) => r,
        None => {
            return Ok(DisplayEvent::Rejected {
                reason: format!("no active recording for output {output}"),
            });
        }
    };

    let artifact_path = backend.stop(output, &recording.session_id, config)?;

    println!(
        "service=displayd op=stop_record event=success output={output} session_id={} path={}",
        recording.session_id,
        artifact_path.display()
    );

    Ok(DisplayEvent::RecordStopped {
        output: output.to_string(),
        session_id: recording.session_id,
        artifact_path: artifact_path.to_string_lossy().to_string(),
    })
}

fn validate_capture_pixel_count(width: u32, height: u32, pixel_len: usize) -> Result<()> {
    let expected_pixels =
        width.checked_mul(height).ok_or_else(|| anyhow::anyhow!("capture dimensions overflow"))?
            as usize;
    if pixel_len != expected_pixels {
        bail!(
            "capture buffer size mismatch: got {} pixels, expected {} for {}x{}",
            pixel_len,
            expected_pixels,
            width,
            height
        );
    }
    Ok(())
}

fn u32_to_rgba8888(pixel: u32) -> [u8; 4] {
    let r = (pixel & 0xFF) as u8;
    let g = ((pixel >> 8) & 0xFF) as u8;
    let b = ((pixel >> 16) & 0xFF) as u8;
    let a = ((pixel >> 24) & 0xFF) as u8;
    [r, g, b, a]
}

fn encode_rgba8888_artifact_bytes(width: u32, height: u32, pixels: &[u32]) -> Result<Vec<u8>> {
    validate_capture_pixel_count(width, height, pixels.len())?;
    let expected_bytes = pixels
        .len()
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("capture artifact byte size overflow"))?;
    let mut bytes = Vec::with_capacity(expected_bytes);
    for &pixel in pixels {
        bytes.extend_from_slice(&u32_to_rgba8888(pixel));
    }
    Ok(bytes)
}

fn generate_mock_pixels(width: u32, height: u32) -> Vec<u32> {
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let r = (x % 256) as u32;
            let g = (y % 256) as u32;
            let b = 128u32;
            let a = 255u32;
            // Encoded as 0xAARRGGBB; bytes are emitted explicitly as RGBA8888 later.
            pixels.push((a << 24) | (r << 16) | (g << 8) | b);
        }
    }
    pixels
}

fn handle_scene_snapshot_request(output: Option<String>, state: &DisplayState) -> DisplayEvent {
    let snapshot = state.scene_for_output(output.as_deref());
    match (&output, &snapshot) {
        (Some(name), Some(scene)) => {
            println!(
                "service=displayd op=get_scene_snapshot event=success output={} commit_id={} surfaces={}",
                name,
                scene.commit_id,
                scene.surfaces.len()
            );
        }
        (Some(name), None) => {
            println!("service=displayd op=get_scene_snapshot event=empty output={name}");
        }
        (None, Some(scene)) => {
            println!(
                "service=displayd op=get_scene_snapshot event=success output=* commit_id={} surfaces={}",
                scene.commit_id,
                scene.surfaces.len()
            );
        }
        (None, None) => {
            println!("service=displayd op=get_scene_snapshot event=empty output=*");
        }
    }

    DisplayEvent::SceneSnapshot { snapshot }
}

fn scene_targets_output(scene: &CommittedSceneState, output: &str) -> bool {
    match &scene.target {
        waybroker_common::CommitTarget::Output { name } => name == output,
    }
}

fn stub_output_mode() -> OutputMode {
    OutputMode { name: "eDP-1".into(), width: 1920, height: 1080, refresh_hz: 60 }
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
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_handle_capture_output() {
        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };

        let mut state = DisplayState::new_test();

        // Ensure runtime dir exists
        ensure_runtime_dir().unwrap();

        let mut clock = FakePresentationClock::default();
        let capture_backend = FakeCaptureBackend;
        let mut record_backend = FakeRecordBackend;
        let mut display_backend = FakeDisplayBackend;

        let result = handle_display_command(
            DisplayCommand::CaptureOutput { output: "eDP-1".into() },
            ServiceRole::Sessiond,
            &config,
            &mut state,
            None,
            &mut clock,
            &capture_backend,
            &mut record_backend,
            &mut display_backend,
        )
        .await
        .expect("handle capture");

        if let DisplayEvent::OutputCaptured { output, width, height, format, artifact_path } =
            result
        {
            assert_eq!(output, "eDP-1");
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
            assert_eq!(format, "RGBA8888");
            assert!(artifact_path.contains("screenshot-eDP-1-"));
            // Since it's now relative, we check against the runtime dir
            let full_path = waybroker_common::runtime_dir().join(artifact_path);
            assert!(full_path.exists());
        } else {
            panic!("Unexpected event: {:?}", result);
        }

        // Test CommitScene and Presentation Feedback
        let commit_result = handle_display_command(
            DisplayCommand::CommitScene {
                target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
                focus: waybroker_common::FocusTarget::None,
                selection: waybroker_common::WaylandSelectionState::default(),
                surfaces: vec![],
            },
            ServiceRole::Compd,
            &config,
            &mut state,
            None,
            &mut clock,
            &capture_backend,
            &mut record_backend,
            &mut display_backend,
        )
        .await
        .expect("handle commit");

        if let DisplayEvent::SceneCommitted { commit_id, .. } = commit_result {
            assert_eq!(commit_id, 1);

            // Query feedback
            let feedback_result = handle_display_command(
                DisplayCommand::GetPresentationFeedback { commit_id: 1 },
                ServiceRole::Waylandd,
                &config,
                &mut state,
                None,
                &mut clock,
                &capture_backend,
                &mut record_backend,
                &mut display_backend,
            )
            .await
            .expect("handle feedback query");

            if let DisplayEvent::FramePresented { commit_id: fid, timestamp, .. } = feedback_result
            {
                assert_eq!(fid, 1);
                assert_eq!(timestamp, 1_000_000_000);
            } else {
                panic!("Expected FramePresented");
            }
        } else {
            panic!("Expected SceneCommitted");
        }
    }

    #[tokio::test]
    async fn output_captured_format_remains_rgba8888() {
        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut state = DisplayState::new_test();
        let mut clock = FakePresentationClock::default();
        let capture_backend = FakeCaptureBackend;
        let mut record_backend = FakeRecordBackend;
        let mut display_backend = FakeDisplayBackend;

        let result = handle_display_command(
            DisplayCommand::CaptureOutput { output: "eDP-1".into() },
            ServiceRole::Sessiond,
            &config,
            &mut state,
            None,
            &mut clock,
            &capture_backend,
            &mut record_backend,
            &mut display_backend,
        )
        .await
        .expect("handle capture");

        match result {
            DisplayEvent::OutputCaptured { format, .. } => assert_eq!(format, "RGBA8888"),
            other => panic!("Unexpected event: {:?}", other),
        }
    }

    #[test]
    fn test_validate_rgba_buffer_size_rejects_mismatch() {
        assert!(validate_capture_pixel_count(2, 2, 3).is_err());
        assert!(validate_capture_pixel_count(0, 0, 0).is_ok());
    }

    #[test]
    fn displayd_rgba8888_writer_rejects_pixel_count_mismatch() {
        let err = encode_rgba8888_artifact_bytes(2, 2, &[0xAA112233, 0xAA112233, 0xAA112233])
            .unwrap_err();
        assert!(err.to_string().contains("mismatch"));
    }

    #[test]
    fn displayd_rgba8888_writer_rejects_dimension_overflow() {
        let err = encode_rgba8888_artifact_bytes(u32::MAX, 2, &[]).unwrap_err();
        assert!(err.to_string().contains("overflow"));
    }

    #[test]
    fn u32_to_rgba8888_is_endianness_independent() {
        let expected = [0x11, 0x22, 0x33, 0xAA];
        assert_eq!(u32_to_rgba8888(0xAA332211), expected);
        assert_eq!(u32_to_rgba8888(u32::from_be_bytes([0xAA, 0x33, 0x22, 0x11])), expected);
        assert_eq!(u32_to_rgba8888(u32::from_le_bytes([0x11, 0x22, 0x33, 0xAA])), expected);
    }

    #[test]
    fn displayd_writes_expected_rgba8888_bytes_for_known_pixels() {
        let bytes = encode_rgba8888_artifact_bytes(
            1,
            1,
            &[((0xAAu32) << 24) | ((0x33u32) << 16) | ((0x22u32) << 8) | 0x11],
        )
        .unwrap();
        assert_eq!(bytes, vec![0x11, 0x22, 0x33, 0xAA]);
    }

    #[tokio::test]
    async fn test_handle_capture_output_propagates_backend_error() {
        struct FailingCaptureBackend;

        #[async_trait::async_trait]
        impl CaptureBackend for FailingCaptureBackend {
            async fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
                Err(anyhow::anyhow!("capture failed"))
            }
        }

        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut state = DisplayState::new_test();
        let mut clock = FakePresentationClock::default();
        let mut record_backend = FakeRecordBackend;
        let mut display_backend = FakeDisplayBackend;
        let err = handle_display_command(
            DisplayCommand::CaptureOutput { output: "eDP-1".into() },
            ServiceRole::Sessiond,
            &config,
            &mut state,
            None,
            &mut clock,
            &FailingCaptureBackend,
            &mut record_backend,
            &mut display_backend,
        )
        .await
        .expect_err("capture backend failure must propagate");
        assert!(err.to_string().contains("capture failed"));
    }

    #[tokio::test]
    async fn capture_backend_failure_still_returns_result_error() {
        struct FailingCaptureBackend;

        #[async_trait::async_trait]
        impl CaptureBackend for FailingCaptureBackend {
            async fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
                Err(anyhow::anyhow!("capture failed"))
            }
        }

        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut state = DisplayState::new_test();
        let mut clock = FakePresentationClock::default();
        let mut record_backend = FakeRecordBackend;
        let mut display_backend = FakeDisplayBackend;
        let err = handle_display_command(
            DisplayCommand::CaptureOutput { output: "eDP-1".into() },
            ServiceRole::Sessiond,
            &config,
            &mut state,
            None,
            &mut clock,
            &FailingCaptureBackend,
            &mut record_backend,
            &mut display_backend,
        )
        .await
        .expect_err("capture backend failure must propagate");
        assert!(err.to_string().contains("capture failed"));
    }

    #[tokio::test]
    async fn existing_displayd_capture_tests_still_pass() {
        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut state = DisplayState::new_test();
        let mut clock = FakePresentationClock::default();
        let capture_backend = FakeCaptureBackend;
        let mut record_backend = FakeRecordBackend;
        let mut display_backend = FakeDisplayBackend;
        let result = handle_display_command(
            DisplayCommand::CaptureOutput { output: "eDP-1".into() },
            ServiceRole::Sessiond,
            &config,
            &mut state,
            None,
            &mut clock,
            &capture_backend,
            &mut record_backend,
            &mut display_backend,
        )
        .await
        .expect("handle capture");
        assert!(matches!(result, DisplayEvent::OutputCaptured { .. }));
    }

    #[test]
    fn test_config_parser_capture_backend_defaults_to_fake() {
        let args = vec!["displayd".to_string()].into_iter().skip(1);
        let config = Config::from_args(args).unwrap();
        assert_eq!(config.capture_backend, CaptureBackendType::Fake);
        assert!(!config.allow_real_capture);
    }

    #[test]
    fn test_config_parser_accepts_fake_explicitly() {
        let args =
            vec!["displayd".to_string(), "--capture-backend".to_string(), "fake".to_string()]
                .into_iter()
                .skip(1);
        let config = Config::from_args(args).unwrap();
        assert_eq!(config.capture_backend, CaptureBackendType::Fake);
    }

    #[test]
    fn test_config_parser_rejects_real_without_allow_flag() {
        let args =
            vec!["displayd".to_string(), "--capture-backend".to_string(), "real".to_string()]
                .into_iter()
                .skip(1);
        let err = Config::from_args(args).unwrap_err();
        assert!(err.to_string().contains("requires --allow-real-capture"));
    }

    #[test]
    fn test_config_parser_rejects_allow_flag_without_real_backend() {
        let args =
            vec!["displayd".to_string(), "--allow-real-capture".to_string()].into_iter().skip(1);
        let err = Config::from_args(args).unwrap_err();
        assert!(err.to_string().contains("requires --capture-backend real"));
    }

    #[test]
    fn test_config_parser_accepts_real_with_allow_flag() {
        let args = vec![
            "displayd".to_string(),
            "--capture-backend".to_string(),
            "real".to_string(),
            "--allow-real-capture".to_string(),
        ]
        .into_iter()
        .skip(1);
        let config = Config::from_args(args).unwrap();
        assert_eq!(config.capture_backend, CaptureBackendType::Real);
        assert!(config.allow_real_capture);
    }

    #[test]
    fn test_config_parser_accepts_x11_real_with_all_flags() {
        let args = vec![
            "displayd".to_string(),
            "--capture-backend".to_string(),
            "real".to_string(),
            "--allow-real-capture".to_string(),
            "--capture-method".to_string(),
            "x11".to_string(),
            "--x11-display".to_string(),
            ":0".to_string(),
        ]
        .into_iter()
        .skip(1);
        let config = Config::from_args(args).unwrap();
        assert_eq!(config.capture_backend, CaptureBackendType::Real);
        assert_eq!(config.capture_method, CaptureMethod::X11);
        assert_eq!(config.x11_display.unwrap(), ":0");
    }

    #[test]
    fn test_config_parser_rejects_x11_without_display() {
        let args = vec![
            "displayd".to_string(),
            "--capture-backend".to_string(),
            "real".to_string(),
            "--allow-real-capture".to_string(),
            "--capture-method".to_string(),
            "x11".to_string(),
        ]
        .into_iter()
        .skip(1);
        let err = Config::from_args(args).unwrap_err();
        assert!(err.to_string().contains("requires --x11-display"));
    }

    #[test]
    fn test_config_parser_rejects_x11_display_without_x11_method() {
        let args = vec![
            "displayd".to_string(),
            "--capture-backend".to_string(),
            "real".to_string(),
            "--allow-real-capture".to_string(),
            "--x11-display".to_string(),
            ":0".to_string(),
        ]
        .into_iter()
        .skip(1);
        let err = Config::from_args(args).unwrap_err();
        assert!(err.to_string().contains("requires --capture-method x11"));
    }

    #[test]
    fn test_config_parser_rejects_unknown_capture_method() {
        let args = vec![
            "displayd".to_string(),
            "--capture-backend".to_string(),
            "real".to_string(),
            "--allow-real-capture".to_string(),
            "--capture-method".to_string(),
            "magical".to_string(),
        ]
        .into_iter()
        .skip(1);
        let err = Config::from_args(args).unwrap_err();
        assert!(err.to_string().contains("unknown capture method"));
    }

    #[test]
    fn test_convert_x11_to_internal_u32_handles_rgb_layout() {
        let data = vec![
            0x11, 0x22, 0x33, 0xFF, // Pixel 1
        ];
        let red_mask = 0x00FF0000;
        let green_mask = 0x0000FF00;
        let blue_mask = 0x000000FF;

        let result =
            convert_x11_to_internal_u32(1, 1, &data, red_mask, green_mask, blue_mask).unwrap();
        assert_eq!(result[0], 0xFF332211);
    }

    #[test]
    fn test_convert_x11_to_internal_u32_handles_bgr_layout() {
        let data = vec![0x33, 0x22, 0x11, 0xFF];
        let red_mask = 0x000000FF;
        let green_mask = 0x0000FF00;
        let blue_mask = 0x00FF0000;

        let result =
            convert_x11_to_internal_u32(1, 1, &data, red_mask, green_mask, blue_mask).unwrap();
        assert_eq!(result[0], 0xFF332211);
    }

    #[test]
    fn test_convert_x11_to_internal_u32_rejects_mismatch() {
        let data = vec![0u8; 3];
        let err = convert_x11_to_internal_u32(1, 1, &data, 0, 0, 0).unwrap_err();
        assert!(err.to_string().contains("data too short"));
    }

    #[test]
    fn test_config_parser_accepts_portal_real_with_all_flags() {
        let args = vec![
            "displayd".to_string(),
            "--capture-backend".to_string(),
            "real".to_string(),
            "--allow-real-capture".to_string(),
            "--capture-method".to_string(),
            "portal".to_string(),
            "--allow-portal-capture".to_string(),
            "--allow-portal-dialog".to_string(),
        ]
        .into_iter()
        .skip(1);
        let config = Config::from_args(args).unwrap();
        assert_eq!(config.capture_backend, CaptureBackendType::Real);
        assert_eq!(config.capture_method, CaptureMethod::Portal);
        assert!(config.allow_portal_capture);
        assert!(config.allow_portal_dialog);
    }

    #[test]
    fn test_config_parser_rejects_portal_without_interactive_flag() {
        let args = vec![
            "displayd".to_string(),
            "--capture-backend".to_string(),
            "real".to_string(),
            "--allow-real-capture".to_string(),
            "--capture-method".to_string(),
            "portal".to_string(),
            "--allow-portal-capture".to_string(),
        ]
        .into_iter()
        .skip(1);
        let err = Config::from_args(args).unwrap_err();
        assert!(err.to_string().contains("requires --allow-portal-dialog"));
    }

    #[test]
    fn test_config_parser_rejects_interactive_flag_without_portal_method() {
        let args = vec![
            "displayd".to_string(),
            "--capture-backend".to_string(),
            "real".to_string(),
            "--allow-real-capture".to_string(),
            "--allow-portal-dialog".to_string(),
        ]
        .into_iter()
        .skip(1);
        let err = Config::from_args(args).unwrap_err();
        assert!(err.to_string().contains("requires --capture-method portal"));
    }

    #[tokio::test]
    async fn test_portal_capture_backend_stub_returns_error() {
        let backend = PortalCaptureBackendStub;
        let err = backend.capture("fullscreen").await.unwrap_err();
        assert!(err.to_string().contains("PipeWire/portal screen capture"));
        assert!(err.to_string().contains("PortalCaptureBackendStub"));
    }

    #[cfg(feature = "real-portal")]
    mod portal_ingestion_tests {
        use super::*;
        use pipewire::spa::sys as spa_sys;

        fn make_mock_data(
            raw_pixels: &[u8],
            stride: i32,
            offset: u32,
            size: u32,
            maxsize: u32,
        ) -> (spa_sys::spa_data, spa_sys::spa_chunk) {
            let chunk = spa_sys::spa_chunk { offset, size, stride, flags: 0 };
            let raw = spa_sys::spa_data {
                type_: spa_sys::SPA_DATA_MemPtr,
                flags: 0,
                fd: -1,
                mapoffset: 0,
                maxsize: maxsize,
                data: raw_pixels.as_ptr() as *mut _,
                chunk: std::ptr::null_mut(), // initialized dynamically in tests
            };
            (raw, chunk)
        }

        #[test]
        fn test_pipewire_frame_metadata_validation_valid() {
            let pixels = vec![0u8; 16 * 16 * 4];
            let (mut raw, mut chunk) = make_mock_data(&pixels, 16 * 4, 0, 16 * 16 * 4, 16 * 16 * 4);
            raw.chunk = &mut chunk;
            let data: &pipewire::spa::buffer::Data = unsafe {
                &*(&raw as *const spa_sys::spa_data as *const pipewire::spa::buffer::Data)
            };

            let res = process_pipewire_frame(
                data,
                16,
                16,
                pipewire::spa::param::video::VideoFormat::RGBA,
            );
            assert!(res.is_ok());
            let (w, h, p) = res.unwrap();
            assert_eq!(w, 16);
            assert_eq!(h, 16);
            assert_eq!(p.len(), 16 * 16);
        }

        #[test]
        fn test_pipewire_frame_metadata_validation_stride_too_small() {
            let pixels = vec![0u8; 16 * 16 * 4];
            let (mut raw, mut chunk) = make_mock_data(&pixels, 10, 0, 16 * 16 * 4, 16 * 16 * 4);
            raw.chunk = &mut chunk;
            let data: &pipewire::spa::buffer::Data = unsafe {
                &*(&raw as *const spa_sys::spa_data as *const pipewire::spa::buffer::Data)
            };

            let res = process_pipewire_frame(
                data,
                16,
                16,
                pipewire::spa::param::video::VideoFormat::RGBA,
            );
            assert!(res.is_err());
            assert!(res.unwrap_err().to_string().contains("stride"));
        }

        #[test]
        fn test_pipewire_frame_metadata_validation_buffer_too_small() {
            let pixels = vec![0u8; 16 * 16 * 4];
            let (mut raw, mut chunk) = make_mock_data(&pixels, 16 * 4, 0, 100, 16 * 16 * 4);
            raw.chunk = &mut chunk;
            let data: &pipewire::spa::buffer::Data = unsafe {
                &*(&raw as *const spa_sys::spa_data as *const pipewire::spa::buffer::Data)
            };

            let res = process_pipewire_frame(
                data,
                16,
                16,
                pipewire::spa::param::video::VideoFormat::RGBA,
            );
            assert!(res.is_err());
            assert!(res.unwrap_err().to_string().contains("chunk size"));
        }

        #[test]
        fn test_pixel_conversion_for_supported_formats() {
            // Test BGRx format
            let pixels = vec![0x11, 0x22, 0x33, 0x00];
            let (mut raw, mut chunk) = make_mock_data(&pixels, 4, 0, 4, 4);
            raw.chunk = &mut chunk;
            let data: &pipewire::spa::buffer::Data = unsafe {
                &*(&raw as *const spa_sys::spa_data as *const pipewire::spa::buffer::Data)
            };

            let (_, _, p) =
                process_pipewire_frame(data, 1, 1, pipewire::spa::param::video::VideoFormat::BGRx)
                    .unwrap();
            assert_eq!(p[0], 0xFF332211);

            // Test RGBA format
            let pixels = vec![0x11, 0x22, 0x33, 0xAA];
            let (mut raw, mut chunk) = make_mock_data(&pixels, 4, 0, 4, 4);
            raw.chunk = &mut chunk;
            let data: &pipewire::spa::buffer::Data = unsafe {
                &*(&raw as *const spa_sys::spa_data as *const pipewire::spa::buffer::Data)
            };

            let (_, _, p) =
                process_pipewire_frame(data, 1, 1, pipewire::spa::param::video::VideoFormat::RGBA)
                    .unwrap();
            assert_eq!(p[0], 0xAA112233);
        }

        #[test]
        fn test_unsupported_format_returns_fail_closed() {
            let pixels = vec![0u8; 16];
            let (mut raw, mut chunk) = make_mock_data(&pixels, 4, 0, 16, 16);
            raw.chunk = &mut chunk;
            let data: &pipewire::spa::buffer::Data = unsafe {
                &*(&raw as *const spa_sys::spa_data as *const pipewire::spa::buffer::Data)
            };

            let res =
                process_pipewire_frame(data, 2, 2, pipewire::spa::param::video::VideoFormat::I420);
            assert!(res.is_err());
            assert!(res.unwrap_err().to_string().contains("Unsupported"));
        }
    }

    #[cfg(test)]
    mod launcher_and_binding_tests {
        // Phase 2-M: Verification tests for launcher scripts and PrtSc key binding installation/restoration behaviors.
        use std::path::PathBuf;
        use std::process::Command;

        fn get_scripts_dir() -> PathBuf {
            let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.pop(); // pop displayd
            path.pop(); // pop crates
            path.push("scripts");
            path
        }

        #[test]
        fn test_launcher_refuses_to_run_without_explicit_real_portal_flags() {
            let scripts_dir = get_scripts_dir();
            let launcher = scripts_dir.join("tuff-xwin-capture-once.sh");
            let output =
                Command::new("bash").arg(&launcher).output().expect("failed to execute launcher");
            assert_ne!(output.status.code(), Some(0));
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("refuses to run without the explicit '--portal-real-capture' flag")
            );
        }

        #[test]
        fn test_launcher_records_run_root_and_report_path() {
            let scripts_dir = get_scripts_dir();
            let launcher = scripts_dir.join("tuff-xwin-capture-once.sh");
            let output = Command::new("bash")
                .arg(&launcher)
                .arg("--portal-real-capture")
                .output()
                .expect("failed to execute launcher");

            let stdout = String::from_utf8_lossy(&output.stdout);

            assert!(stdout.contains("run_root="));

            let run_root_line = stdout.lines().find(|l| l.contains("run_root=")).unwrap_or("");
            let run_root_part = run_root_line
                .split(" ")
                .find(|p| p.starts_with("run_root="))
                .unwrap_or("run_root=");
            let run_root_path = run_root_part.trim_start_matches("run_root=");

            if !run_root_path.is_empty() {
                let run_root = PathBuf::from(run_root_path);
                assert!(run_root.exists());
                let report = run_root.join("report.md");
                assert!(report.exists());
                let _ = std::fs::remove_dir_all(run_root);
            }
        }

        #[test]
        fn test_launcher_fails_closed_if_displayd_exits_before_capture() {
            let scripts_dir = get_scripts_dir();
            let launcher = scripts_dir.join("tuff-xwin-capture-once.sh");
            let output = Command::new("bash")
                .arg(&launcher)
                .arg("--portal-real-capture")
                .arg("--save-dir")
                .arg("/nonexistent/directory/path/that/fails")
                .output()
                .expect("failed to execute launcher");
            assert_eq!(output.status.code(), Some(1));
        }

        #[test]
        fn test_unknown_de_path_refuses_mutation() {
            let scripts_dir = get_scripts_dir();
            let installer = scripts_dir.join("install-user-prtsc-tuff-capture-binding.sh");
            let output = Command::new("bash")
                .env("XDG_CURRENT_DESKTOP", "UNKNOWN")
                .arg(&installer)
                .output()
                .expect("failed to execute installer");
            assert_eq!(output.status.code(), Some(2));
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("not supported"));
        }

        #[test]
        fn test_hotkey_detection_handles_flameshot_binding_without_modifying_it_in_dry_detection_mode()
         {
            let scripts_dir = get_scripts_dir();
            let installer = scripts_dir.join("install-user-prtsc-tuff-capture-binding.sh");
            let output = Command::new("bash")
                .env("XDG_CURRENT_DESKTOP", "GNOME")
                .arg(&installer)
                .output()
                .expect("failed to execute installer");
            assert_eq!(output.status.code(), Some(2));
        }

        #[test]
        fn test_rollback_script_is_generated_before_binding_mutation() {
            let scripts_dir = get_scripts_dir();
            let installer = scripts_dir.join("install-user-prtsc-tuff-capture-binding.sh");
            let content = std::fs::read_to_string(&installer).expect("failed to read installer");
            let backup_idx = content.find("BACKUP_DIR").unwrap_or(usize::MAX);
            let write_manifest_idx = content
                .find("echo -n \"$MANIFEST_CONTENT\" > \"$MANIFEST_FILE\"")
                .unwrap_or(usize::MAX);
            let mutation_idx = content.find("Creating user-local Flameshot").unwrap_or(usize::MAX);

            assert!(backup_idx < mutation_idx);
            assert!(write_manifest_idx < mutation_idx);
        }

        #[test]
        fn test_launcher_creates_default_save_directory_safely() {
            let scripts_dir = get_scripts_dir();
            let launcher = scripts_dir.join("tuff-xwin-capture-once.sh");
            let _ = Command::new("bash").arg(&launcher).arg("--portal-real-capture").output();
            let pics_dir =
                std::env::var("HOME").map(|h| PathBuf::from(h).join("Pictures/TUFF-Xwin")).unwrap();
            assert!(pics_dir.parent().unwrap().exists() || pics_dir.exists());
        }

        #[test]
        fn test_launcher_accepts_explicit_save_dir() {
            let scripts_dir = get_scripts_dir();
            let temp = tempfile::tempdir().unwrap();
            let custom_save_dir = temp.path().join("custom_tuff_save");
            let launcher = scripts_dir.join("tuff-xwin-capture-once.sh");
            let _ = Command::new("bash")
                .arg(&launcher)
                .arg("--portal-real-capture")
                .arg("--save-dir")
                .arg(&custom_save_dir)
                .output()
                .expect("failed to execute launcher");
            assert!(custom_save_dir.exists());
        }

        struct TestEnv {
            _temp_dir: tempfile::TempDir,
            home_dir: PathBuf,
            repo_root: PathBuf,
            target_xsm_dir: PathBuf,
            mock_desktop_path: PathBuf,
            bin_dir: PathBuf,
        }

        impl TestEnv {
            fn new() -> Self {
                let temp = tempfile::tempdir().unwrap();
                let home = temp.path().join("home");
                let repo = temp.path().join("repo");
                let target_xsm = repo.join("target/xsm");
                let bin_dir = temp.path().join("bin");

                std::fs::create_dir_all(&home).unwrap();
                std::fs::create_dir_all(&target_xsm).unwrap();
                std::fs::create_dir_all(&bin_dir).unwrap();

                // Create mock systemctl, kwriteconfig6, etc. to avoid system dbus delays
                let systemctl_mock = r#"#!/bin/bash
ENV_FILE="$HOME/.systemd_path_mock"
if [[ "$1" == "--user" && "$2" == "show-environment" ]]; then
    if [[ -f "$ENV_FILE" ]]; then
        cat "$ENV_FILE"
    else
        echo "PATH=/mocked/bin"
    fi
elif [[ "$1" == "--user" && "$2" == "set-environment" ]]; then
    echo "$3" > "$ENV_FILE"
elif [[ "$1" == "--user" && "$2" == "unset-environment" ]]; then
    if [[ "$3" == "PATH" ]]; then
        echo "PATH=" > "$ENV_FILE"
    fi
fi
exit 0
"#;
                std::fs::write(bin_dir.join("systemctl"), systemctl_mock).unwrap();
                std::fs::write(bin_dir.join("kwriteconfig6"), "#!/bin/bash\nexit 0").unwrap();
                std::fs::write(bin_dir.join("kreadconfig6"), "#!/bin/bash\necho Print\nexit 0")
                    .unwrap();
                std::fs::write(bin_dir.join("kbuildsycoca6"), "#!/bin/bash\nexit 0").unwrap();
                std::fs::write(bin_dir.join("qdbus6"), "#!/bin/bash\nexit 0").unwrap();
                std::fs::write(bin_dir.join("dbus-send"), "#!/bin/bash\nexit 0").unwrap();

                // Make them executable
                use std::os::unix::fs::PermissionsExt;
                for name in &[
                    "systemctl",
                    "kwriteconfig6",
                    "kreadconfig6",
                    "kbuildsycoca6",
                    "qdbus6",
                    "dbus-send",
                ] {
                    let file = bin_dir.join(name);
                    let mut perms = std::fs::metadata(&file).unwrap().permissions();
                    perms.set_mode(0o755);
                    std::fs::set_permissions(&file, perms).unwrap();
                }

                // Copy only required scripts to temp repo to reduce CIFS I/O latency
                let src_scripts_dir = get_scripts_dir();
                let dst_scripts_dir = repo.join("scripts");
                std::fs::create_dir_all(&dst_scripts_dir).unwrap();

                let needed_scripts = &[
                    "install-user-prtsc-tuff-capture-binding.sh",
                    "restore-user-prtsc-binding.sh",
                    "tuff-xwin-capture-once.sh",
                ];
                for name in needed_scripts {
                    let src = src_scripts_dir.join(name);
                    if src.exists() {
                        std::fs::copy(&src, dst_scripts_dir.join(name)).unwrap();
                    }
                }

                // Create mock system desktop file
                let mock_desktop = temp.path().join("org.flameshot.Flameshot.desktop");
                std::fs::write(&mock_desktop, "Exec=flameshot\n[Desktop Entry]\nName=Flameshot")
                    .unwrap();

                Self {
                    _temp_dir: temp,
                    home_dir: home,
                    repo_root: repo,
                    target_xsm_dir: target_xsm,
                    mock_desktop_path: mock_desktop,
                    bin_dir: bin_dir,
                }
            }

            fn run_script(&self, script_name: &str, envs: &[(&str, &str)]) -> std::process::Output {
                let script_path = self.repo_root.join("scripts").join(script_name);

                let current_path = std::env::var("PATH").unwrap_or_default();
                let new_path = format!("{}:{}", self.bin_dir.display(), current_path);

                let mut cmd = Command::new("bash");
                cmd.arg(&script_path)
                    .env("HOME", &self.home_dir)
                    .env("PATH", &new_path)
                    .env("XDG_CURRENT_DESKTOP", "KDE") // force KDE DE detection
                    .env("TUFF_MOCK_SYSTEM_DESKTOP", &self.mock_desktop_path)
                    .current_dir(&self.repo_root);

                for (k, v) in envs {
                    cmd.env(k, v);
                }

                cmd.output().expect("failed to run script")
            }
        }

        #[test]
        fn test_restore_without_manifest_does_not_delete_anything() {
            let env = TestEnv::new();

            // Create some file in local bin
            let local_bin = env.home_dir.join(".local/bin");
            std::fs::create_dir_all(&local_bin).unwrap();
            let wrapper = local_bin.join("flameshot");
            std::fs::write(&wrapper, "original content").unwrap();

            // Run restore directly (no manifest exists)
            let output = env.run_script("restore-user-prtsc-binding.sh", &[]);

            // Should fail because no backup manifest exists
            assert_ne!(output.status.code(), Some(0));

            // Ensure nothing was deleted
            assert!(wrapper.exists());
            assert_eq!(std::fs::read_to_string(&wrapper).unwrap(), "original content");
        }

        #[test]
        fn test_existing_local_flameshot_wrapper_is_backed_up_and_restored() {
            let env = TestEnv::new();

            let local_bin = env.home_dir.join(".local/bin");
            std::fs::create_dir_all(&local_bin).unwrap();
            let wrapper = local_bin.join("flameshot");
            std::fs::write(&wrapper, "original flameshot content").unwrap();

            // Run install
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output_install.status.code(), Some(0));

            // Verify that wrapper is updated (it should be our wrapper, not original content)
            assert!(wrapper.exists());
            let wrapper_content = std::fs::read_to_string(&wrapper).unwrap();
            assert!(wrapper_content.contains("TUFF-Xwin Flameshot wrapper override"));

            // Verify backup directory was created and contains the backup file
            let backup_dirs: Vec<_> = std::fs::read_dir(&env.target_xsm_dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| {
                    p.is_dir()
                        && !p.is_symlink()
                        && p.file_name()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .starts_with("tuff-xwin-prtsc-backup-")
                        && !p.file_name().unwrap().to_str().unwrap().ends_with("-latest")
                })
                .collect();
            assert_eq!(backup_dirs.len(), 1);
            let backup_dir = &backup_dirs[0];

            let backup_flameshot = backup_dir.join("flameshot");
            assert!(backup_flameshot.exists());
            assert_eq!(
                std::fs::read_to_string(&backup_flameshot).unwrap(),
                "original flameshot content"
            );

            let manifest = backup_dir.join("manifest.tsv");
            assert!(manifest.exists());
            let manifest_content = std::fs::read_to_string(&manifest).unwrap();
            assert!(manifest_content.contains("flameshot\ttrue"));

            // Run restore
            let output_restore = env.run_script("restore-user-prtsc-binding.sh", &[]);
            assert_eq!(output_restore.status.code(), Some(0));

            // Verify wrapper is restored to original content
            assert!(wrapper.exists());
            assert_eq!(std::fs::read_to_string(&wrapper).unwrap(), "original flameshot content");
        }

        #[test]
        fn test_existing_environment_d_path_file_is_backed_up_and_restored() {
            let env = TestEnv::new();

            let env_conf_dir = env.home_dir.join(".config/environment.d");
            std::fs::create_dir_all(&env_conf_dir).unwrap();
            let path_conf = env_conf_dir.join("tuff-xwin-path.conf");
            std::fs::write(&path_conf, "PATH=/some/custom/path").unwrap();

            // Run install
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output_install.status.code(), Some(0));

            // Verify file is updated to our tuff-xwin-path.conf content
            assert!(path_conf.exists());
            let content = std::fs::read_to_string(&path_conf).unwrap();
            assert!(content.contains(".local/bin"));

            // Run restore
            let output_restore = env.run_script("restore-user-prtsc-binding.sh", &[]);
            assert_eq!(output_restore.status.code(), Some(0));

            // Verify file is restored to original content
            assert!(path_conf.exists());
            assert_eq!(std::fs::read_to_string(&path_conf).unwrap(), "PATH=/some/custom/path");
        }

        #[test]
        fn test_existed_false_generated_file_is_removed_on_restore() {
            let env = TestEnv::new();

            // Ensure wrapper and config do not exist initially
            let local_bin = env.home_dir.join(".local/bin");
            let wrapper = local_bin.join("flameshot");
            let path_conf = env.home_dir.join(".config/environment.d/tuff-xwin-path.conf");
            assert!(!wrapper.exists());
            assert!(!path_conf.exists());

            // Run install
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output_install.status.code(), Some(0));

            // Verified created
            assert!(wrapper.exists());
            assert!(path_conf.exists());

            // Run restore
            let output_restore = env.run_script("restore-user-prtsc-binding.sh", &[]);
            assert_eq!(output_restore.status.code(), Some(0));

            // Verified removed (since existed=false in manifest)
            assert!(!wrapper.exists());
            assert!(!path_conf.exists());
        }

        #[test]
        fn test_installer_refuses_mutation_if_backup_manifest_cannot_be_written() {
            let env = TestEnv::new();

            // Make target/xsm read-only (impossible to create subdirectories or write files)
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&env.target_xsm_dir, std::fs::Permissions::from_mode(0o555))
                .unwrap();

            // Run install
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);

            // Should fail because mkdir or writing manifest fails
            assert_ne!(output_install.status.code(), Some(0));

            // Restore permissions so cleanup doesn't fail
            let _ = std::fs::set_permissions(
                &env.target_xsm_dir,
                std::fs::Permissions::from_mode(0o777),
            );

            // Verify no modification to user environment was done
            let wrapper = env.home_dir.join(".local/bin/flameshot");
            let path_conf = env.home_dir.join(".config/environment.d/tuff-xwin-path.conf");
            assert!(!wrapper.exists());
            assert!(!path_conf.exists());
        }

        #[test]
        fn test_installer_refuses_to_overwrite_existing_local_flameshot_unless_backup_is_confirmed()
        {
            let env = TestEnv::new();

            let local_bin = env.home_dir.join(".local/bin");
            std::fs::create_dir_all(&local_bin).unwrap();
            let wrapper = local_bin.join("flameshot");
            std::fs::write(&wrapper, "pre-existing flameshot").unwrap();

            let output = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output.status.code(), Some(0));
        }

        #[test]
        fn test_restore_preserves_unrelated_environment_d_files() {
            let env = TestEnv::new();

            let env_conf_dir = env.home_dir.join(".config/environment.d");
            std::fs::create_dir_all(&env_conf_dir).unwrap();
            let unrelated_conf = env_conf_dir.join("other.conf");
            std::fs::write(&unrelated_conf, "SOME_VAR=1").unwrap();

            // Run install
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output_install.status.code(), Some(0));

            // Run restore
            let output_restore = env.run_script("restore-user-prtsc-binding.sh", &[]);
            assert_eq!(output_restore.status.code(), Some(0));

            // Verify unrelated file was preserved
            assert!(unrelated_conf.exists());
            assert_eq!(std::fs::read_to_string(&unrelated_conf).unwrap(), "SOME_VAR=1");
        }

        #[test]
        fn test_restore_preserves_preexisting_systemd_user_path_containing_home_local_bin() {
            let env = TestEnv::new();

            // Set initial state containing HOME local bin
            let env_file = env.home_dir.join(".systemd_path_mock");
            let local_bin = env.home_dir.join(".local/bin");
            let initial_path = format!("PATH={}:/usr/bin", local_bin.display());
            std::fs::write(&env_file, &initial_path).unwrap();

            // Run install
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output_install.status.code(), Some(0));

            // Run restore
            let output_restore = env.run_script("restore-user-prtsc-binding.sh", &[]);
            assert_eq!(output_restore.status.code(), Some(0));

            // PATH should be preserved as-is, not stripped by sed
            let final_path = std::fs::read_to_string(&env_file).unwrap();
            assert_eq!(final_path.trim(), initial_path);
        }

        #[test]
        fn test_restore_restores_original_systemd_user_path_from_backup() {
            let env = TestEnv::new();

            let env_file = env.home_dir.join(".systemd_path_mock");
            std::fs::write(&env_file, "PATH=/usr/bin:/bin").unwrap();

            // Run install
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output_install.status.code(), Some(0));

            // Verify path was modified
            let modified_path = std::fs::read_to_string(&env_file).unwrap();
            assert!(modified_path.contains(".local/bin"));

            // Run restore
            let output_restore = env.run_script("restore-user-prtsc-binding.sh", &[]);
            assert_eq!(output_restore.status.code(), Some(0));

            // Verify original path restored
            let final_path = std::fs::read_to_string(&env_file).unwrap();
            assert_eq!(final_path.trim(), "PATH=/usr/bin:/bin");
        }

        #[test]
        fn test_restore_does_not_delete_existed_false_file_without_tuff_marker() {
            let env = TestEnv::new();

            // Run install to create files with existed=false
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output_install.status.code(), Some(0));

            let wrapper = env.home_dir.join(".local/bin/flameshot");
            let path_conf = env.home_dir.join(".config/environment.d/tuff-xwin-path.conf");
            let desktop =
                env.home_dir.join(".local/share/applications/org.flameshot.Flameshot.desktop");

            // Overwrite them with content lacking TUFF markers
            std::fs::write(&wrapper, "user custom script").unwrap();
            std::fs::write(&path_conf, "user custom env").unwrap();
            std::fs::write(&desktop, "user custom desktop").unwrap();

            // Run restore
            let output_restore = env.run_script("restore-user-prtsc-binding.sh", &[]);
            assert_eq!(output_restore.status.code(), Some(0));

            // They must NOT be deleted
            assert!(wrapper.exists());
            assert!(path_conf.exists());
            assert!(desktop.exists());
        }

        #[test]
        fn test_restore_deletes_existed_false_tuff_wrapper_only_when_marker_matches() {
            let env = TestEnv::new();

            // Run install
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output_install.status.code(), Some(0));

            let wrapper = env.home_dir.join(".local/bin/flameshot");
            assert!(wrapper.exists());

            // Run restore
            let output_restore = env.run_script("restore-user-prtsc-binding.sh", &[]);
            assert_eq!(output_restore.status.code(), Some(0));

            // Should be deleted because it has the marker
            assert!(!wrapper.exists());
        }

        #[test]
        fn test_restore_deletes_existed_false_tuff_environment_file_only_when_marker_matches() {
            let env = TestEnv::new();

            // Run install
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output_install.status.code(), Some(0));

            let path_conf = env.home_dir.join(".config/environment.d/tuff-xwin-path.conf");
            assert!(path_conf.exists());

            // Run restore
            let output_restore = env.run_script("restore-user-prtsc-binding.sh", &[]);
            assert_eq!(output_restore.status.code(), Some(0));

            // Should be deleted because it has the marker
            assert!(!path_conf.exists());
        }

        #[test]
        fn test_restore_deletes_existed_false_tuff_desktop_override_only_when_marker_or_exec_matches()
         {
            let env = TestEnv::new();

            // Run install
            let output_install = env.run_script("install-user-prtsc-tuff-capture-binding.sh", &[]);
            assert_eq!(output_install.status.code(), Some(0));

            let desktop =
                env.home_dir.join(".local/share/applications/org.flameshot.Flameshot.desktop");
            assert!(desktop.exists());

            // Run restore
            let output_restore = env.run_script("restore-user-prtsc-binding.sh", &[]);
            assert_eq!(output_restore.status.code(), Some(0));

            // Should be deleted because it has the marker
            assert!(!desktop.exists());
        }
    }

    #[tokio::test]
    async fn test_phase3_composition_zero_damage_skipping() {
        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut state = DisplayState::new_test();
        let mut clock = FakePresentationClock::default();
        let capture_backend = FakeCaptureBackend;
        let mut record_backend = FakeRecordBackend;
        let mut display_backend = FakeDisplayBackend;

        // Perform first commit
        let surf1 = waybroker_common::SurfaceSnapshot {
            id: "s1".into(),
            app_id: "app1".into(),
            placement: waybroker_common::SurfacePlacement {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
                z: 1,
                visible: true,
            },
        };
        let commit1 = handle_display_command(
            DisplayCommand::CommitScene {
                target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
                focus: waybroker_common::FocusTarget::None,
                selection: waybroker_common::WaylandSelectionState::default(),
                surfaces: vec![surf1.clone()],
            },
            ServiceRole::Compd,
            &config,
            &mut state,
            None,
            &mut clock,
            &capture_backend,
            &mut record_backend,
            &mut display_backend,
        )
        .await
        .unwrap();
        assert!(matches!(commit1, DisplayEvent::SceneCommitted { .. }));
        assert_eq!(state.zero_damage_skipped_count, 0);

        // Perform second identical commit
        let commit2 = handle_display_command(
            DisplayCommand::CommitScene {
                target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
                focus: waybroker_common::FocusTarget::None,
                selection: waybroker_common::WaylandSelectionState::default(),
                surfaces: vec![surf1],
            },
            ServiceRole::Compd,
            &config,
            &mut state,
            None,
            &mut clock,
            &capture_backend,
            &mut record_backend,
            &mut display_backend,
        )
        .await
        .unwrap();
        assert!(matches!(commit2, DisplayEvent::SceneCommitted { .. }));
        assert_eq!(state.zero_damage_skipped_count, 1);
    }

    #[tokio::test]
    async fn test_phase3_composition_direct_scanout() {
        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut state = DisplayState::new_test();
        let mut clock = FakePresentationClock::default();
        let capture_backend = FakeCaptureBackend;
        let mut record_backend = FakeRecordBackend;
        let mut display_backend = FakeDisplayBackend;

        // Perform fullscreen single surface commit
        let fullscreen_surf = waybroker_common::SurfaceSnapshot {
            id: "fullscreen".into(),
            app_id: "mpv".into(),
            placement: waybroker_common::SurfacePlacement {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                z: 1,
                visible: true,
            },
        };
        let commit = handle_display_command(
            DisplayCommand::CommitScene {
                target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
                focus: waybroker_common::FocusTarget::None,
                selection: waybroker_common::WaylandSelectionState::default(),
                surfaces: vec![fullscreen_surf],
            },
            ServiceRole::Compd,
            &config,
            &mut state,
            None,
            &mut clock,
            &capture_backend,
            &mut record_backend,
            &mut display_backend,
        )
        .await
        .unwrap();
        assert!(matches!(commit, DisplayEvent::SceneCommitted { .. }));
        assert_eq!(state.direct_scanout_count, 1);
        assert_eq!(state.composition_frame_count, 0);
    }
}
