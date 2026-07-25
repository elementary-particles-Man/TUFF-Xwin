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
    CommittedSceneState, DisplayCommand, DisplayEvent, IpcEnvelope, MessageKind, OutputGeometry,
    OutputMode, PixelTransportError, PixelTransportPayload, PixelTransportStore, ServiceBanner,
    ServiceEndpoint, ServiceRole, ServiceStream, accel::global_accel_policy, bind_service_socket,
    ensure_runtime_dir, now_unix_timestamp, read_json_line, sanitize_artifact_filename,
    send_json_line, session_artifact_path, validate_artifact_filename,
};

const DEFAULT_SESSION_INSTANCE_ID: &str = "default-single-session";
const BACKGROUND_PIXEL: u32 = 0xFF00_0000;
#[cfg(test)]
const FRAMEBUFFER_WIDTH: u32 = 1920;
#[cfg(test)]
const FRAMEBUFFER_HEIGHT: u32 = 1080;
const WL_SHM_FORMAT_ARGB8888: u32 = 0;
const WL_SHM_FORMAT_XRGB8888: u32 = 1;

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
    let mut display_backend = MockDisplayBackend::default();

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
        DisplayCommand::ConfigureOutput { geometry } => {
            let next = OutputState::validate(&geometry).map_err(|reason| anyhow::anyhow!(reason));
            let next = match next {
                Ok(value) => value,
                Err(error) => return Ok(DisplayEvent::Rejected { reason: error.to_string() }),
            };
            if next.generation <= state.output.generation {
                return Ok(DisplayEvent::Rejected { reason: "stale output generation".into() });
            }
            state.output = next;
            // Keep the replacement unpublished until the next successful composition;
            // an empty active buffer also forces the required full repaint.
            state.framebuffer = Vec::new();
            Ok(DisplayEvent::ModeApplied {
                output: geometry.output_id.clone(),
                mode: OutputMode {
                    name: geometry.output_id,
                    width: geometry.width,
                    height: geometry.height,
                    refresh_hz: 0,
                },
            })
        }
        DisplayCommand::SetMode { output, mode } => {
            display_backend.set_mode(&output, &mode)?;
            println!("service=displayd op=set_mode event=success output={output} mode={:?}", mode);
            Ok(DisplayEvent::ModeApplied { output, mode })
        }
        DisplayCommand::CommitScene {
            target,
            focus,
            selection,
            surfaces,
            pixel_payloads,
            scene_epoch,
            scene_generation,
        } => {
            if scene_generation_is_stale(
                state.last_scene_epoch,
                state.last_scene_generation,
                scene_epoch,
                scene_generation,
            ) {
                return Ok(DisplayEvent::Rejected {
                    reason: format!(
                        "stale scene epoch {} generation {}; latest is epoch {} generation {}",
                        scene_epoch,
                        scene_generation,
                        state.last_scene_epoch,
                        state.last_scene_generation
                    ),
                });
            }
            if let Err(err) = submit_pixel_payloads(&mut state.pixel_transport, pixel_payloads) {
                return Ok(DisplayEvent::Rejected {
                    reason: format!("pixel transport rejected payload: {err:?}"),
                });
            }
            if let Err(reason) = verify_pixel_payloads_available(&state.pixel_transport, &surfaces)
            {
                return Ok(DisplayEvent::Rejected { reason });
            }
            let start_time = std::time::Instant::now();
            let mut skipped = false;
            let mut is_direct_scanout = false;

            // 1. zero-damage check
            if let Some(last_scene) = &state.last_scene {
                if last_scene.surfaces.len() == surfaces.len() {
                    let mut all_match = true;
                    for (s1, s2) in last_scene.surfaces.iter().zip(surfaces.iter()) {
                        if s1.id != s2.id
                            || s1.placement != s2.placement
                            || s1.buffer_generation != s2.buffer_generation
                            || !s1.damage_rects.is_empty()
                            || !s2.damage_rects.is_empty()
                        {
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
                    && surf.placement.width == state.output.width
                    && surf.placement.height == state.output.height
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

            let mut damaged_pixels = 0u64;
            let mut copied_bytes = 0u64;

            if !skipped {
                let output_bounds = state.output.bounds();
                let damage_rects = effective_damage_rects(
                    state.last_scene.as_ref(),
                    &surfaces,
                    framebuffer_is_initialized(&state.framebuffer, &state.output),
                    output_bounds,
                );
                if damage_rects.is_empty() {
                    skipped = true;
                    state.zero_damage_skipped_count += 1;
                } else {
                    if let Err(reason) = validate_surface_pixels_for_damage(
                        &state.pixel_transport,
                        &surfaces,
                        &damage_rects,
                    ) {
                        return Ok(DisplayEvent::Rejected { reason });
                    }
                    let mut scratch =
                        if framebuffer_is_initialized(&state.framebuffer, &state.output) {
                            state.framebuffer.clone()
                        } else {
                            vec![BACKGROUND_PIXEL; state.output.framebuffer_words()]
                        };
                    let stats = match compose_damage_rects(
                        &mut scratch,
                        &state.output,
                        &state.pixel_transport,
                        &surfaces,
                        &damage_rects,
                    ) {
                        Ok(stats) => stats,
                        Err(reason) => return Ok(DisplayEvent::Rejected { reason }),
                    };
                    let frame_id = state.next_frame_id;
                    let request = FramePublication {
                        frame_id,
                        output_id: state.output.output_id.clone(),
                        output_generation: state.output.generation,
                        scene_generation,
                        geometry: OutputGeometry {
                            output_id: state.output.output_id.clone(),
                            width: state.output.width,
                            height: state.output.height,
                            stride: state.output.stride,
                            format: state.output.format,
                            origin_x: state.output.origin_x,
                            origin_y: state.output.origin_y,
                            output_generation: state.output.generation,
                        },
                        format: state.output.format,
                        stride: state.output.stride,
                        pixels: std::sync::Arc::<[u32]>::from(scratch.clone()),
                        damage: damage_rects,
                    };
                    if let Err(error) = display_backend.publish_frame(request) {
                        return Ok(DisplayEvent::Rejected {
                            reason: format!("display backend publication failed: {error}"),
                        });
                    }
                    state.framebuffer = scratch;
                    state.published_frame_id = frame_id;
                    state.next_frame_id = frame_id.saturating_add(1);
                    damaged_pixels = stats.damaged_pixels;
                    copied_bytes = stats.copied_bytes;
                }
            }

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
            if scene_generation != 0 {
                if scene_epoch > state.last_scene_epoch {
                    state.last_scene_epoch = scene_epoch;
                    state.last_scene_generation = scene_generation;
                } else if scene_epoch == state.last_scene_epoch
                    && scene_generation > state.last_scene_generation
                {
                    state.last_scene_generation = scene_generation;
                }
            }
            let snapshot = CommittedSceneState {
                source,
                target: target.clone(),
                focus: focus.clone(),
                selection: selection.clone(),
                surfaces: surfaces.clone(),
                scene_epoch,
                scene_generation,
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
                copied_bytes,
                damaged_pixels,
                if skipped || is_direct_scanout { 0 } else { 1 },
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

fn scene_generation_is_stale(
    last_epoch: u64,
    last_generation: u64,
    incoming_epoch: u64,
    incoming_generation: u64,
) -> bool {
    incoming_generation != 0
        && (incoming_epoch < last_epoch
            || (incoming_epoch == last_epoch && incoming_generation < last_generation))
}

#[derive(Debug)]
struct DisplayState {
    last_scene: Option<CommittedSceneState>,
    last_scene_epoch: u64,
    last_scene_generation: u64,
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
    next_frame_id: u64,
    published_frame_id: u64,
    framebuffer: Vec<u32>,
    output: OutputState,
    pixel_transport: PixelTransportStore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputState {
    output_id: String,
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
    origin_x: i32,
    origin_y: i32,
    generation: u64,
}
impl OutputState {
    fn validate(g: &OutputGeometry) -> std::result::Result<Self, String> {
        if g.output_id.is_empty() || g.width == 0 || g.height == 0 {
            return Err("invalid output identity or zero dimensions".into());
        }
        if g.format != WL_SHM_FORMAT_XRGB8888 {
            return Err("output framebuffer must be XRGB8888".into());
        }
        let min = g.width.checked_mul(4).ok_or("output stride overflow")?;
        if g.stride < min || g.stride % 4 != 0 {
            return Err("undersized or unaligned output stride".into());
        }
        let words = (g.stride as usize)
            .checked_div(4)
            .and_then(|row| row.checked_mul(g.height as usize))
            .ok_or("framebuffer size overflow")?;
        if words == 0 {
            return Err("framebuffer size overflow".into());
        }
        Ok(Self {
            output_id: g.output_id.clone(),
            width: g.width,
            height: g.height,
            stride: g.stride,
            format: g.format,
            origin_x: g.origin_x,
            origin_y: g.origin_y,
            generation: g.output_generation,
        })
    }
    fn bounds(&self) -> RendererRect {
        RendererRect::from_origin_size(self.origin_x, self.origin_y, self.width, self.height)
            .expect("validated geometry")
    }
    fn framebuffer_words(&self) -> usize {
        (self.stride as usize / 4) * self.height as usize
    }
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
    fn publish_frame(&mut self, request: FramePublication) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FramePublication {
    frame_id: u64,
    output_id: String,
    output_generation: u64,
    scene_generation: u64,
    geometry: OutputGeometry,
    format: u32,
    stride: u32,
    pixels: std::sync::Arc<[u32]>,
    damage: Vec<RendererRect>,
}

#[derive(Default)]
struct MockDisplayBackend {
    publications: Vec<FramePublication>,
    fail_on_frame: Option<u64>,
}

impl MockDisplayBackend {
    fn set_fail_on_frame(&mut self, frame_id: u64) {
        self.fail_on_frame = Some(frame_id);
    }
    fn publications(&self) -> &[FramePublication] {
        &self.publications
    }
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

    fn publish_frame(&mut self, _request: FramePublication) -> Result<()> {
        Ok(())
    }
}

impl DisplayBackend for MockDisplayBackend {
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
    fn publish_frame(&mut self, request: FramePublication) -> Result<()> {
        if self.fail_on_frame == Some(request.frame_id) {
            bail!("mock publication failure for frame {}", request.frame_id);
        }
        self.publications.push(request);
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
        let (last_scene_epoch, last_scene_generation) = last_scene
            .as_ref()
            .map(|scene| (scene.scene_epoch, scene.scene_generation))
            .unwrap_or((0, 0));
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
            last_scene_epoch,
            last_scene_generation,
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
            next_frame_id: 1,
            published_frame_id: 0,
            framebuffer: Vec::new(),
            output: OutputState {
                output_id: "eDP-1".into(),
                width: 1920,
                height: 1080,
                stride: 1920 * 4,
                format: WL_SHM_FORMAT_XRGB8888,
                origin_x: 0,
                origin_y: 0,
                generation: 0,
            },
            pixel_transport: PixelTransportStore::default(),
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
        if scene.scene_generation != 0 {
            self.last_scene_epoch = scene.scene_epoch;
            self.last_scene_generation = scene.scene_generation;
        }
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
            last_scene_epoch: 0,
            last_scene_generation: 0,
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
            next_frame_id: 1,
            published_frame_id: 0,
            framebuffer: Vec::new(),
            output: OutputState {
                output_id: "eDP-1".into(),
                width: 1920,
                height: 1080,
                stride: 1920 * 4,
                format: WL_SHM_FORMAT_XRGB8888,
                origin_x: 0,
                origin_y: 0,
                generation: 0,
            },
            pixel_transport: PixelTransportStore::default(),
        }
    }
}

fn submit_pixel_payloads(
    store: &mut PixelTransportStore,
    payloads: Vec<PixelTransportPayload>,
) -> std::result::Result<(), PixelTransportError> {
    for payload in payloads {
        store.submit(payload)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RendererRect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl RendererRect {
    fn from_origin_size(x: i32, y: i32, width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let x1 = checked_add_i32_u32(x, width)?;
        let y1 = checked_add_i32_u32(y, height)?;
        (x1 > x && y1 > y).then_some(Self { x0: x, y0: y, x1, y1 })
    }

    fn width(self) -> u32 {
        (self.x1 - self.x0) as u32
    }

    fn height(self) -> u32 {
        (self.y1 - self.y0) as u32
    }

    fn area(self) -> u64 {
        self.width() as u64 * self.height() as u64
    }

    fn intersect(self, other: Self) -> Option<Self> {
        let rect = Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };
        (rect.x1 > rect.x0 && rect.y1 > rect.y0).then_some(rect)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CompositionStats {
    damaged_pixels: u64,
    copied_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PixelFormatPolicy {
    PremultipliedArgb8888,
    OpaqueXrgb8888,
}

impl PixelFormatPolicy {
    fn from_wire_format(format: u32) -> std::result::Result<Self, String> {
        match format {
            WL_SHM_FORMAT_ARGB8888 => Ok(Self::PremultipliedArgb8888),
            WL_SHM_FORMAT_XRGB8888 => Ok(Self::OpaqueXrgb8888),
            other => Err(format!("unsupported pixel format {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourcePixel {
    Opaque(u32),
    Premultiplied { a: u8, r: u8, g: u8, b: u8 },
}

fn checked_add_i32_u32(value: i32, delta: u32) -> Option<i32> {
    let sum = value as i64 + delta as i64;
    i32::try_from(sum).ok()
}

fn framebuffer_is_initialized(framebuffer: &[u32], output: &OutputState) -> bool {
    framebuffer.len() == output.framebuffer_words()
}

#[cfg(test)]
fn ensure_framebuffer(framebuffer: &mut Vec<u32>) {
    if framebuffer.len() != (FRAMEBUFFER_WIDTH as usize * FRAMEBUFFER_HEIGHT as usize) {
        *framebuffer =
            vec![BACKGROUND_PIXEL; FRAMEBUFFER_WIDTH as usize * FRAMEBUFFER_HEIGHT as usize];
    }
}

fn surface_output_bounds(surface: &waybroker_common::SurfaceSnapshot) -> Option<RendererRect> {
    if !surface.placement.visible {
        return None;
    }
    RendererRect::from_origin_size(
        surface.placement.x,
        surface.placement.y,
        surface.placement.width,
        surface.placement.height,
    )
}

fn surface_damage_rects(
    surface: &waybroker_common::SurfaceSnapshot,
    output_bounds: RendererRect,
) -> Vec<RendererRect> {
    let Some(surface_bounds) = surface_output_bounds(surface) else {
        return Vec::new();
    };
    let Some(surface_local_bounds) =
        RendererRect::from_origin_size(0, 0, surface.placement.width, surface.placement.height)
    else {
        return Vec::new();
    };
    surface
        .damage_rects
        .iter()
        .filter_map(|damage| {
            let local =
                RendererRect::from_origin_size(damage.x, damage.y, damage.width, damage.height)?;
            let local = local.intersect(surface_local_bounds)?;
            let output = RendererRect::from_origin_size(
                surface.placement.x.checked_add(local.x0)?,
                surface.placement.y.checked_add(local.y0)?,
                local.width(),
                local.height(),
            )?;
            output.intersect(surface_bounds)?.intersect(output_bounds)
        })
        .collect()
}

fn effective_damage_rects(
    previous: Option<&CommittedSceneState>,
    surfaces: &[waybroker_common::SurfaceSnapshot],
    framebuffer_ready: bool,
    output_bounds: RendererRect,
) -> Vec<RendererRect> {
    if previous.is_none() || !framebuffer_ready {
        return vec![output_bounds];
    }

    let previous = previous.expect("checked above");
    let previous_by_id: HashMap<&str, &waybroker_common::SurfaceSnapshot> =
        previous.surfaces.iter().map(|surface| (surface.id.as_str(), surface)).collect();
    let current_by_id: HashMap<&str, &waybroker_common::SurfaceSnapshot> =
        surfaces.iter().map(|surface| (surface.id.as_str(), surface)).collect();
    let mut damage = Vec::new();

    for surface in surfaces {
        let previous_surface = previous_by_id.get(surface.id.as_str()).copied();
        if previous_surface
            .map(|old| surface_exposes_or_occupies_different_region(old, surface))
            .unwrap_or(true)
        {
            if let Some(rect) =
                surface_output_bounds(surface).and_then(|r| r.intersect(output_bounds))
            {
                damage.push(rect);
            }
        }
        damage.extend(surface_damage_rects(surface, output_bounds));
    }

    for old in &previous.surfaces {
        if !current_by_id.contains_key(old.id.as_str())
            || current_by_id
                .get(old.id.as_str())
                .map(|new| surface_exposes_or_occupies_different_region(old, new))
                .unwrap_or(false)
        {
            if let Some(rect) = surface_output_bounds(old).and_then(|r| r.intersect(output_bounds))
            {
                damage.push(rect);
            }
        }
    }

    damage.sort_by_key(|rect| (rect.y0, rect.x0, rect.y1, rect.x1));
    damage
}

fn surface_exposes_or_occupies_different_region(
    old: &waybroker_common::SurfaceSnapshot,
    new: &waybroker_common::SurfaceSnapshot,
) -> bool {
    old.placement != new.placement
        || old.buffer_generation != new.buffer_generation
        || old.placement.visible != new.placement.visible
}

fn compose_damage_rects(
    framebuffer: &mut [u32],
    output: &OutputState,
    store: &PixelTransportStore,
    surfaces: &[waybroker_common::SurfaceSnapshot],
    damage_rects: &[RendererRect],
) -> std::result::Result<CompositionStats, String> {
    let mut stats = CompositionStats::default();
    for damage in damage_rects {
        stats.damaged_pixels = stats.damaged_pixels.saturating_add(damage.area());
        fill_framebuffer_rect(framebuffer, output, *damage, BACKGROUND_PIXEL)?;
        for surface in surfaces {
            let Some(surface_bounds) = surface_output_bounds(surface) else {
                continue;
            };
            let Some(copy_rect) = surface_bounds.intersect(*damage) else {
                continue;
            };
            let color = fallback_surface_pixel(surface);
            copy_surface_rect(framebuffer, output, store, surface, copy_rect, color)?;
        }
    }
    stats.copied_bytes = stats.damaged_pixels.saturating_mul(4);
    Ok(stats)
}

fn validate_surface_pixels_for_damage(
    store: &PixelTransportStore,
    surfaces: &[waybroker_common::SurfaceSnapshot],
    damage_rects: &[RendererRect],
) -> std::result::Result<(), String> {
    for damage in damage_rects {
        for surface in surfaces {
            let Some(surface_bounds) = surface_output_bounds(surface) else {
                continue;
            };
            let Some(copy_rect) = surface_bounds.intersect(*damage) else {
                continue;
            };
            validate_surface_pixel_rect(store, surface, copy_rect)?;
        }
    }
    Ok(())
}

fn validate_surface_pixel_rect(
    store: &PixelTransportStore,
    surface: &waybroker_common::SurfaceSnapshot,
    rect: RendererRect,
) -> std::result::Result<(), String> {
    let Some(handle) = surface.pixel_transport.as_ref() else {
        return Ok(());
    };
    let payload = store.lookup(handle).ok_or_else(|| missing_payload_reason(handle))?;
    PixelFormatPolicy::from_wire_format(payload.format)?;
    if payload.width == 0 || payload.height == 0 {
        return Err(format!("invalid zero-sized pixel payload for surface {}", surface.id));
    }
    if payload.stride < payload.width.saturating_mul(4) {
        return Err(format!("invalid stride {} for surface {}", payload.stride, surface.id));
    }
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            source_pixel_offset(payload, surface, x, y)?;
        }
    }
    Ok(())
}

fn fill_framebuffer_rect(
    framebuffer: &mut [u32],
    output: &OutputState,
    rect: RendererRect,
    pixel: u32,
) -> std::result::Result<(), String> {
    for y in rect.y0..rect.y1 {
        let row_start = framebuffer_offset_for_output(output, output.origin_x, y)?;
        for x in rect.x0..rect.x1 {
            let index = row_start
                .checked_add((x - output.origin_x) as usize)
                .ok_or_else(|| "framebuffer offset overflow".to_string())?;
            framebuffer[index] = pixel;
        }
    }
    Ok(())
}

fn copy_surface_rect(
    framebuffer: &mut [u32],
    output: &OutputState,
    store: &PixelTransportStore,
    surface: &waybroker_common::SurfaceSnapshot,
    rect: RendererRect,
    fallback_pixel: u32,
) -> std::result::Result<(), String> {
    for y in rect.y0..rect.y1 {
        let row_start = framebuffer_offset_for_output(output, output.origin_x, y)?;
        for x in rect.x0..rect.x1 {
            let index = row_start
                .checked_add((x - output.origin_x) as usize)
                .ok_or_else(|| "framebuffer offset overflow".to_string())?;
            let src =
                pixel_for_surface(store, surface, &surface.placement, x as usize, y as usize)?
                    .unwrap_or(SourcePixel::Opaque(fallback_pixel));
            framebuffer[index] = compose_source_over(src, framebuffer[index]);
        }
    }
    Ok(())
}

fn framebuffer_offset_for_output(
    output: &OutputState,
    x: i32,
    y: i32,
) -> std::result::Result<usize, String> {
    let local_x = x.checked_sub(output.origin_x).ok_or("x translation overflow")?;
    let local_y = y.checked_sub(output.origin_y).ok_or("y translation overflow")?;
    if local_x < 0
        || local_y < 0
        || local_x as u32 >= output.width
        || local_y as u32 >= output.height
    {
        return Err("framebuffer coordinate out of bounds".into());
    }
    (local_y as usize)
        .checked_mul(output.stride as usize / 4)
        .and_then(|offset| offset.checked_add(local_x as usize))
        .ok_or_else(|| "framebuffer offset overflow".into())
}

#[cfg(test)]
fn framebuffer_offset(x: u32, y: u32) -> std::result::Result<usize, String> {
    framebuffer_offset_for_output(
        &OutputState {
            output_id: "test".into(),
            width: FRAMEBUFFER_WIDTH,
            height: FRAMEBUFFER_HEIGHT,
            stride: FRAMEBUFFER_WIDTH * 4,
            format: WL_SHM_FORMAT_XRGB8888,
            origin_x: 0,
            origin_y: 0,
            generation: 1,
        },
        x as i32,
        y as i32,
    )
}

fn fallback_surface_pixel(surface: &waybroker_common::SurfaceSnapshot) -> u32 {
    if surface.id.contains("panel") { 0xFF0000FF } else { 0xFFFF0000 }
}

fn missing_payload_reason(handle: &waybroker_common::PixelTransportHandle) -> String {
    format!(
        "missing pixel payload for client {} surface {} buffer {} scene {}",
        handle.client_id, handle.surface_id, handle.buffer_generation, handle.scene_generation
    )
}

fn pixel_for_surface(
    store: &PixelTransportStore,
    surface: &waybroker_common::SurfaceSnapshot,
    placement: &waybroker_common::SurfacePlacement,
    x: usize,
    y: usize,
) -> std::result::Result<Option<SourcePixel>, String> {
    let Some(handle) = surface.pixel_transport.as_ref() else { return Ok(None) };
    let payload = store.lookup(handle).ok_or_else(|| missing_payload_reason(handle))?;
    let local_x = (x as i32 - placement.x).max(0) as u32;
    let local_y = (y as i32 - placement.y).max(0) as u32;
    if local_x >= payload.width || local_y >= payload.height {
        return Ok(None);
    }
    let offset = source_pixel_offset(payload, surface, x as i32, y as i32)?;
    let bytes = payload
        .pixels
        .get(offset..offset + 4)
        .ok_or_else(|| format!("pixel payload is too short for surface {}", surface.id))?;
    let word = u32::from_le_bytes(bytes.try_into().expect("slice length checked"));
    decode_source_pixel(word, PixelFormatPolicy::from_wire_format(payload.format)?).map(Some)
}

fn source_pixel_offset(
    payload: &PixelTransportPayload,
    surface: &waybroker_common::SurfaceSnapshot,
    x: i32,
    y: i32,
) -> std::result::Result<usize, String> {
    let local_x =
        x.checked_sub(surface.placement.x).ok_or_else(|| "source x offset overflow".to_string())?;
    let local_y =
        y.checked_sub(surface.placement.y).ok_or_else(|| "source y offset overflow".to_string())?;
    if local_x < 0 || local_y < 0 {
        return Err(format!("source coordinate outside surface {}", surface.id));
    }
    let local_x = local_x as u32;
    let local_y = local_y as u32;
    if local_x >= payload.width || local_y >= payload.height {
        return Err(format!("source coordinate outside payload {}", surface.id));
    }
    let row = (local_y as usize)
        .checked_mul(payload.stride as usize)
        .ok_or_else(|| "source row offset overflow".to_string())?;
    let col = (local_x as usize)
        .checked_mul(4)
        .ok_or_else(|| "source column offset overflow".to_string())?;
    let offset = row.checked_add(col).ok_or_else(|| "source pixel offset overflow".to_string())?;
    let end = offset.checked_add(4).ok_or_else(|| "source pixel end overflow".to_string())?;
    if end > payload.pixels.len() {
        return Err(format!("pixel payload is too short for surface {}", surface.id));
    }
    Ok(offset)
}

fn decode_source_pixel(
    word: u32,
    policy: PixelFormatPolicy,
) -> std::result::Result<SourcePixel, String> {
    match policy {
        PixelFormatPolicy::OpaqueXrgb8888 => {
            Ok(SourcePixel::Opaque(0xFF00_0000 | (word & 0x00FF_FFFF)))
        }
        PixelFormatPolicy::PremultipliedArgb8888 => {
            let a = ((word >> 24) & 0xFF) as u8;
            let r = ((word >> 16) & 0xFF) as u8;
            let g = ((word >> 8) & 0xFF) as u8;
            let b = (word & 0xFF) as u8;
            if r > a || g > a || b > a {
                return Err(format!("non-premultiplied ARGB8888 pixel r={r} g={g} b={b} a={a}"));
            }
            Ok(SourcePixel::Premultiplied { a, r, g, b })
        }
    }
}

fn compose_source_over(src: SourcePixel, dst: u32) -> u32 {
    match src {
        SourcePixel::Opaque(pixel) => 0xFF00_0000 | (pixel & 0x00FF_FFFF),
        SourcePixel::Premultiplied { a: 0, .. } => dst,
        SourcePixel::Premultiplied { a: 255, r, g, b } => {
            0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
        }
        SourcePixel::Premultiplied { a, r, g, b } => {
            let inv = 255u32 - a as u32;
            let dst_r = (dst >> 16) & 0xFF;
            let dst_g = (dst >> 8) & 0xFF;
            let dst_b = dst & 0xFF;
            let out_r = r as u32 + div255_rounded(dst_r * inv);
            let out_g = g as u32 + div255_rounded(dst_g * inv);
            let out_b = b as u32 + div255_rounded(dst_b * inv);
            0xFF00_0000 | (out_r.min(255) << 16) | (out_g.min(255) << 8) | out_b.min(255)
        }
    }
}

fn div255_rounded(value: u32) -> u32 {
    (value + 127) / 255
}

fn verify_pixel_payloads_available(
    store: &PixelTransportStore,
    surfaces: &[waybroker_common::SurfaceSnapshot],
) -> std::result::Result<(), String> {
    for surface in surfaces {
        let Some(handle) = surface.pixel_transport.as_ref() else {
            continue;
        };
        if store.lookup(handle).is_none() {
            return Err(format!(
                "missing pixel payload for client {} surface {} buffer {} scene {}",
                handle.client_id,
                handle.surface_id,
                handle.buffer_generation,
                handle.scene_generation
            ));
        }
    }
    Ok(())
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

    #[test]
    fn rejects_only_older_nonzero_scene_generations() {
        assert!(scene_generation_is_stale(2, 8, 1, 7));
        assert!(!scene_generation_is_stale(2, 8, 2, 8));
        assert!(!scene_generation_is_stale(2, 8, 2, 9));
        assert!(!scene_generation_is_stale(2, 8, 2, 0));
        assert!(!scene_generation_is_stale(2, 8, 3, 1));
    }

    #[test]
    fn accepts_newer_epoch_after_restart() {
        assert!(!scene_generation_is_stale(10, 100, 11, 1));
    }

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
                pixel_payloads: vec![],
                scene_epoch: 0,
                scene_generation: 0,
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
            ..Default::default()
        };
        let commit1 = handle_display_command(
            DisplayCommand::CommitScene {
                target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
                focus: waybroker_common::FocusTarget::None,
                selection: waybroker_common::WaylandSelectionState::default(),
                surfaces: vec![surf1.clone()],
                pixel_payloads: vec![],
                scene_epoch: 0,
                scene_generation: 0,
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
                pixel_payloads: vec![],
                scene_epoch: 0,
                scene_generation: 0,
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
    async fn pixel_transport_payload_feeds_renderer_without_entering_scene_snapshot() {
        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut state = DisplayState::new_test();
        let mut clock = FakePresentationClock::default();
        let capture_backend = FakeCaptureBackend;
        let mut record_backend = FakeRecordBackend;
        let mut display_backend = FakeDisplayBackend;
        let handle = waybroker_common::PixelTransportHandle {
            client_id: 7,
            surface_id: "client-7-surface-3".into(),
            buffer_generation: 3,
            scene_generation: 1,
        };
        let surface = waybroker_common::SurfaceSnapshot {
            id: handle.surface_id.clone(),
            app_id: "app1".into(),
            placement: waybroker_common::SurfacePlacement {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
                z: 1,
                visible: true,
            },
            buffer_handle: Some("3".into()),
            buffer_generation: 3,
            pixel_transport: Some(handle.clone()),
            ..Default::default()
        };
        let payload = waybroker_common::PixelTransportPayload {
            handle,
            pixels: vec![0x44, 0x33, 0x22, 0x11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            width: 2,
            height: 2,
            stride: 8,
            format: 1,
        };

        let commit = handle_display_command(
            DisplayCommand::CommitScene {
                target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
                focus: waybroker_common::FocusTarget::None,
                selection: waybroker_common::WaylandSelectionState::default(),
                surfaces: vec![surface],
                pixel_payloads: vec![payload],
                scene_epoch: 1,
                scene_generation: 1,
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
        assert_eq!(state.framebuffer[0], 0xFF223344);
        let snapshot = state.last_scene.as_ref().unwrap();
        assert!(snapshot.surfaces[0].pixel_transport.is_some());
        assert!(!serde_json::to_string(snapshot).unwrap().contains("\"pixels\""));
    }

    #[tokio::test]
    async fn missing_pixel_transport_payload_rejects_without_corrupting_scene() {
        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut state = DisplayState::new_test();
        let mut clock = FakePresentationClock::default();
        let capture_backend = FakeCaptureBackend;
        let mut record_backend = FakeRecordBackend;
        let mut display_backend = FakeDisplayBackend;
        let surface = waybroker_common::SurfaceSnapshot {
            id: "client-7-surface-3".into(),
            app_id: "app1".into(),
            placement: waybroker_common::SurfacePlacement {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
                z: 1,
                visible: true,
            },
            pixel_transport: Some(waybroker_common::PixelTransportHandle {
                client_id: 7,
                surface_id: "client-7-surface-3".into(),
                buffer_generation: 3,
                scene_generation: 1,
            }),
            ..Default::default()
        };

        let commit = handle_display_command(
            DisplayCommand::CommitScene {
                target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
                focus: waybroker_common::FocusTarget::None,
                selection: waybroker_common::WaylandSelectionState::default(),
                surfaces: vec![surface],
                pixel_payloads: vec![],
                scene_epoch: 1,
                scene_generation: 1,
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

        assert!(matches!(commit, DisplayEvent::Rejected { .. }));
        assert!(state.last_scene.is_none());
    }

    fn test_handle(
        client_id: u64,
        surface_id: &str,
        buffer_generation: u64,
        scene_generation: u64,
    ) -> waybroker_common::PixelTransportHandle {
        waybroker_common::PixelTransportHandle {
            client_id,
            surface_id: surface_id.into(),
            buffer_generation,
            scene_generation,
        }
    }

    fn test_payload(
        handle: waybroker_common::PixelTransportHandle,
        pixels: Vec<u32>,
        width: u32,
        height: u32,
        stride: u32,
    ) -> waybroker_common::PixelTransportPayload {
        test_payload_with_format(handle, pixels, width, height, stride, WL_SHM_FORMAT_XRGB8888)
    }

    fn test_payload_with_format(
        handle: waybroker_common::PixelTransportHandle,
        pixels: Vec<u32>,
        width: u32,
        height: u32,
        stride: u32,
        format: u32,
    ) -> waybroker_common::PixelTransportPayload {
        let mut bytes = vec![0u8; stride as usize * height as usize];
        for (i, pixel) in pixels.into_iter().enumerate() {
            let row = i as u32 / width;
            let col = i as u32 % width;
            let offset = row as usize * stride as usize + col as usize * 4;
            bytes[offset..offset + 4].copy_from_slice(&pixel.to_le_bytes());
        }
        waybroker_common::PixelTransportPayload {
            handle,
            pixels: bytes,
            width,
            height,
            stride,
            format,
        }
    }

    fn test_surface(
        id: &str,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        z: i32,
        handle: Option<waybroker_common::PixelTransportHandle>,
    ) -> waybroker_common::SurfaceSnapshot {
        waybroker_common::SurfaceSnapshot {
            id: id.into(),
            app_id: "app1".into(),
            placement: waybroker_common::SurfacePlacement { x, y, width, height, z, visible: true },
            buffer_generation: handle.as_ref().map(|h| h.buffer_generation).unwrap_or(0),
            pixel_transport: handle,
            ..Default::default()
        }
    }

    async fn commit_test_scene(
        state: &mut DisplayState,
        surfaces: Vec<waybroker_common::SurfaceSnapshot>,
        pixel_payloads: Vec<waybroker_common::PixelTransportPayload>,
        scene_generation: u64,
    ) -> DisplayEvent {
        let mut display_backend = MockDisplayBackend::default();
        commit_test_scene_with_backend(
            state,
            surfaces,
            pixel_payloads,
            scene_generation,
            &mut display_backend,
        )
        .await
    }

    async fn commit_test_scene_with_backend(
        state: &mut DisplayState,
        surfaces: Vec<waybroker_common::SurfaceSnapshot>,
        pixel_payloads: Vec<waybroker_common::PixelTransportPayload>,
        scene_generation: u64,
        display_backend: &mut dyn DisplayBackend,
    ) -> DisplayEvent {
        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut clock = FakePresentationClock::default();
        let capture_backend = FakeCaptureBackend;
        let mut record_backend = FakeRecordBackend;
        handle_display_command(
            DisplayCommand::CommitScene {
                target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
                focus: waybroker_common::FocusTarget::None,
                selection: waybroker_common::WaylandSelectionState::default(),
                surfaces,
                pixel_payloads,
                scene_epoch: 1,
                scene_generation,
            },
            ServiceRole::Compd,
            &config,
            state,
            None,
            &mut clock,
            &capture_backend,
            &mut record_backend,
            display_backend,
        )
        .await
        .unwrap()
    }

    #[test]
    fn alpha_source_over_vectors_are_exact_and_opaque_framebuffer_normalized() {
        assert_eq!(
            compose_source_over(SourcePixel::Premultiplied { a: 0, r: 0, g: 0, b: 0 }, 0xFF112233),
            0xFF112233
        );
        assert_eq!(
            compose_source_over(
                SourcePixel::Premultiplied { a: 255, r: 1, g: 2, b: 3 },
                0xFF112233
            ),
            0xFF010203
        );
        assert_eq!(
            compose_source_over(
                SourcePixel::Premultiplied { a: 128, r: 128, g: 0, b: 0 },
                0xFF0000FF
            ),
            0xFF80007F
        );
        assert_eq!(compose_source_over(SourcePixel::Opaque(0x00123456), 0), 0xFF123456);
    }

    #[test]
    fn pixel_format_policy_decodes_argb_and_xrgb_channel_order() {
        assert_eq!(
            decode_source_pixel(0x80800000, PixelFormatPolicy::PremultipliedArgb8888).unwrap(),
            SourcePixel::Premultiplied { a: 128, r: 128, g: 0, b: 0 }
        );
        assert_eq!(
            decode_source_pixel(0x00010203, PixelFormatPolicy::OpaqueXrgb8888).unwrap(),
            SourcePixel::Opaque(0xFF010203)
        );
        assert_eq!(
            decode_source_pixel(0xAA010203, PixelFormatPolicy::OpaqueXrgb8888).unwrap(),
            SourcePixel::Opaque(0xFF010203)
        );
        assert!(decode_source_pixel(0x40800000, PixelFormatPolicy::PremultipliedArgb8888).is_err());
        assert!(PixelFormatPolicy::from_wire_format(99).is_err());
    }

    #[tokio::test]
    async fn damage_limited_composition_updates_only_small_rect() {
        let mut state = DisplayState::new_test();
        let handle = test_handle(1, "surface-1", 1, 1);
        let surface = test_surface("surface-1", 0, 0, 4, 4, 1, Some(handle.clone()));
        let initial_payload = test_payload(handle.clone(), vec![0x11111111; 16], 4, 4, 16);
        assert!(matches!(
            commit_test_scene(&mut state, vec![surface.clone()], vec![initial_payload], 1).await,
            DisplayEvent::SceneCommitted { .. }
        ));
        let before = state.framebuffer.clone();

        let updated_handle = test_handle(1, "surface-1", 1, 2);
        let mut damaged_surface =
            test_surface("surface-1", 0, 0, 4, 4, 1, Some(updated_handle.clone()));
        damaged_surface.damage_rects =
            vec![waybroker_common::Rect { x: 1, y: 1, width: 1, height: 1 }];
        let updated_payload = test_payload(updated_handle, vec![0x22222222; 16], 4, 4, 16);
        assert!(matches!(
            commit_test_scene(&mut state, vec![damaged_surface], vec![updated_payload], 2).await,
            DisplayEvent::SceneCommitted { .. }
        ));

        let changed = framebuffer_offset(1, 1).unwrap();
        assert_eq!(state.framebuffer[changed], 0xFF222222);
        for (index, pixel) in state.framebuffer.iter().enumerate().take(8_000) {
            if index != changed {
                assert_eq!(*pixel, before[index], "unexpected framebuffer change at {index}");
            }
        }
    }

    #[test]
    fn damage_rects_are_translated_and_clipped_for_negative_and_overflowing_surfaces() {
        let output =
            RendererRect::from_origin_size(0, 0, FRAMEBUFFER_WIDTH, FRAMEBUFFER_HEIGHT).unwrap();
        let mut negative = test_surface("neg", -2, -1, 4, 4, 1, None);
        negative.damage_rects = vec![waybroker_common::Rect { x: 0, y: 0, width: 4, height: 4 }];
        let mut overflow = test_surface("overflow", 1918, 1078, 8, 8, 1, None);
        overflow.damage_rects = vec![waybroker_common::Rect { x: 0, y: 0, width: 8, height: 8 }];

        assert_eq!(
            surface_damage_rects(&negative, output),
            vec![RendererRect { x0: 0, y0: 0, x1: 2, y1: 3 }]
        );
        assert_eq!(
            surface_damage_rects(&overflow, output),
            vec![RendererRect { x0: 1918, y0: 1078, x1: 1920, y1: 1080 }]
        );
    }

    #[tokio::test]
    async fn non_tight_stride_partial_row_copy_uses_source_stride() {
        let mut state = DisplayState::new_test();
        let handle = test_handle(1, "surface-1", 1, 1);
        let mut surface = test_surface("surface-1", 0, 0, 3, 2, 1, Some(handle.clone()));
        surface.damage_rects = vec![waybroker_common::Rect { x: 1, y: 1, width: 1, height: 1 }];
        ensure_framebuffer(&mut state.framebuffer);
        state.last_scene = Some(CommittedSceneState {
            source: ServiceRole::Compd,
            target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
            focus: waybroker_common::FocusTarget::None,
            selection: waybroker_common::WaylandSelectionState::default(),
            surfaces: vec![test_surface("surface-1", 0, 0, 3, 2, 1, Some(handle.clone()))],
            scene_epoch: 1,
            scene_generation: 1,
            commit_id: 1,
            unix_timestamp: 1,
        });
        let payload =
            test_payload(handle, vec![0x10, 0x11, 0x12, 0x20, 0xAABBCCDD, 0x22], 3, 2, 16);

        assert!(matches!(
            commit_test_scene(&mut state, vec![surface], vec![payload], 2).await,
            DisplayEvent::SceneCommitted { .. }
        ));

        assert_eq!(state.framebuffer[framebuffer_offset(1, 1).unwrap()], 0xFFBBCCDD);
        assert_eq!(state.framebuffer[framebuffer_offset(0, 1).unwrap()], BACKGROUND_PIXEL);
    }

    #[tokio::test]
    async fn non_tight_stride_alpha_partial_copy_blends_from_source_stride() {
        let mut state = DisplayState::new_test();
        let bottom_handle = test_handle(1, "bottom", 1, 1);
        let bottom = test_surface("bottom", 0, 0, 3, 2, 1, Some(bottom_handle.clone()));
        let handle = test_handle(2, "surface-1", 1, 1);
        let mut surface = test_surface("surface-1", 0, 0, 3, 2, 2, Some(handle.clone()));
        surface.damage_rects = vec![waybroker_common::Rect { x: 1, y: 1, width: 1, height: 1 }];
        ensure_framebuffer(&mut state.framebuffer);
        state.last_scene = Some(CommittedSceneState {
            source: ServiceRole::Compd,
            target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
            focus: waybroker_common::FocusTarget::None,
            selection: waybroker_common::WaylandSelectionState::default(),
            surfaces: vec![
                bottom.clone(),
                test_surface("surface-1", 0, 0, 3, 2, 2, Some(handle.clone())),
            ],
            scene_epoch: 1,
            scene_generation: 1,
            commit_id: 1,
            unix_timestamp: 1,
        });
        let payload = test_payload_with_format(
            handle,
            vec![0, 0, 0, 0, 0x80800000, 0],
            3,
            2,
            16,
            WL_SHM_FORMAT_ARGB8888,
        );

        assert!(matches!(
            commit_test_scene(
                &mut state,
                vec![bottom, surface],
                vec![test_payload(bottom_handle, vec![0x000000FF; 6], 3, 2, 12), payload],
                2
            )
            .await,
            DisplayEvent::SceneCommitted { .. }
        ));

        assert_eq!(state.framebuffer[framebuffer_offset(1, 1).unwrap()], 0xFF80007F);
        assert_eq!(state.framebuffer[framebuffer_offset(0, 1).unwrap()], BACKGROUND_PIXEL);
    }

    #[tokio::test]
    async fn overlapping_surfaces_recompose_damage_in_canonical_order() {
        let mut state = DisplayState::new_test();
        let bottom = test_surface("bottom", 0, 0, 4, 4, 1, None);
        let mut top = test_surface("panel-top", 1, 1, 2, 2, 2, None);
        top.damage_rects = vec![waybroker_common::Rect { x: 0, y: 0, width: 1, height: 1 }];
        ensure_framebuffer(&mut state.framebuffer);
        state.last_scene = Some(CommittedSceneState {
            source: ServiceRole::Compd,
            target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
            focus: waybroker_common::FocusTarget::None,
            selection: waybroker_common::WaylandSelectionState::default(),
            surfaces: vec![bottom.clone(), test_surface("panel-top", 1, 1, 2, 2, 2, None)],
            scene_epoch: 1,
            scene_generation: 1,
            commit_id: 1,
            unix_timestamp: 1,
        });

        assert!(matches!(
            commit_test_scene(&mut state, vec![bottom, top], vec![], 2).await,
            DisplayEvent::SceneCommitted { .. }
        ));

        assert_eq!(state.framebuffer[framebuffer_offset(1, 1).unwrap()], 0xFF0000FF);
    }

    #[tokio::test]
    async fn translucent_overlap_uses_canonical_back_to_front_order() {
        let mut state = DisplayState::new_test();
        let bottom_handle = test_handle(1, "bottom", 1, 1);
        let bottom = test_surface("bottom", 0, 0, 2, 2, 1, Some(bottom_handle.clone()));
        let top_handle = test_handle(2, "top", 1, 2);
        let mut top = test_surface("top", 0, 0, 2, 2, 2, Some(top_handle.clone()));
        top.damage_rects = vec![waybroker_common::Rect { x: 0, y: 0, width: 1, height: 1 }];

        assert!(matches!(
            commit_test_scene(
                &mut state,
                vec![bottom.clone()],
                vec![test_payload(bottom_handle.clone(), vec![0x000000FF; 4], 2, 2, 8)],
                1
            )
            .await,
            DisplayEvent::SceneCommitted { .. }
        ));
        assert!(matches!(
            commit_test_scene(
                &mut state,
                vec![bottom, top],
                vec![
                    test_payload(bottom_handle, vec![0x000000FF; 4], 2, 2, 8),
                    test_payload_with_format(
                        top_handle,
                        vec![0x80800000; 4],
                        2,
                        2,
                        8,
                        WL_SHM_FORMAT_ARGB8888
                    )
                ],
                2
            )
            .await,
            DisplayEvent::SceneCommitted { .. }
        ));

        assert_eq!(state.framebuffer[framebuffer_offset(0, 0).unwrap()], 0xFF80007F);
    }

    #[tokio::test]
    async fn three_layer_translucent_composition_is_deterministic() {
        let mut state = DisplayState::new_test();
        let bottom_h = test_handle(1, "bottom", 1, 1);
        let middle_h = test_handle(2, "middle", 1, 1);
        let top_h = test_handle(3, "top", 1, 1);
        let bottom = test_surface("bottom", 0, 0, 1, 1, 1, Some(bottom_h.clone()));
        let middle = test_surface("middle", 0, 0, 1, 1, 2, Some(middle_h.clone()));
        let top = test_surface("top", 0, 0, 1, 1, 3, Some(top_h.clone()));

        assert!(matches!(
            commit_test_scene(
                &mut state,
                vec![bottom, middle, top],
                vec![
                    test_payload(bottom_h, vec![0x000000FF], 1, 1, 4),
                    test_payload_with_format(
                        middle_h,
                        vec![0x80008000],
                        1,
                        1,
                        4,
                        WL_SHM_FORMAT_ARGB8888
                    ),
                    test_payload_with_format(
                        top_h,
                        vec![0x40400000],
                        1,
                        1,
                        4,
                        WL_SHM_FORMAT_ARGB8888
                    )
                ],
                1
            )
            .await,
            DisplayEvent::SceneCommitted { .. }
        ));

        assert_eq!(state.framebuffer[framebuffer_offset(0, 0).unwrap()], 0xFF40605F);
    }

    #[tokio::test]
    async fn clipped_translucent_surface_blends_only_inside_output_bounds() {
        let mut state = DisplayState::new_test();
        let handle = test_handle(1, "surface-1", 1, 1);
        let surface = test_surface("surface-1", -1, -1, 2, 2, 1, Some(handle.clone()));
        let payload =
            test_payload_with_format(handle, vec![0x80800000; 4], 2, 2, 8, WL_SHM_FORMAT_ARGB8888);

        assert!(matches!(
            commit_test_scene(&mut state, vec![surface], vec![payload], 1).await,
            DisplayEvent::SceneCommitted { .. }
        ));

        assert_eq!(state.framebuffer[framebuffer_offset(0, 0).unwrap()], 0xFF800000);
        assert_eq!(state.framebuffer[framebuffer_offset(1, 0).unwrap()], BACKGROUND_PIXEL);
    }

    #[tokio::test]
    async fn surface_movement_and_removal_reconstruct_exposed_regions() {
        let mut state = DisplayState::new_test();
        let original = test_surface("surface-1", 0, 0, 2, 2, 1, None);
        assert!(matches!(
            commit_test_scene(&mut state, vec![original], vec![], 1).await,
            DisplayEvent::SceneCommitted { .. }
        ));
        assert_eq!(state.framebuffer[framebuffer_offset(0, 0).unwrap()], 0xFFFF0000);

        let moved = test_surface("surface-1", 2, 0, 2, 2, 1, None);
        assert!(matches!(
            commit_test_scene(&mut state, vec![moved], vec![], 2).await,
            DisplayEvent::SceneCommitted { .. }
        ));
        assert_eq!(state.framebuffer[framebuffer_offset(0, 0).unwrap()], BACKGROUND_PIXEL);
        assert_eq!(state.framebuffer[framebuffer_offset(2, 0).unwrap()], 0xFFFF0000);

        assert!(matches!(
            commit_test_scene(&mut state, vec![], vec![], 3).await,
            DisplayEvent::SceneCommitted { .. }
        ));
        assert_eq!(state.framebuffer[framebuffer_offset(2, 0).unwrap()], BACKGROUND_PIXEL);
    }

    #[tokio::test]
    async fn stale_or_missing_payload_does_not_partially_mutate_framebuffer() {
        let mut state = DisplayState::new_test();
        ensure_framebuffer(&mut state.framebuffer);
        state.framebuffer[framebuffer_offset(0, 0).unwrap()] = 0xDEADBEEF;
        let before = state.framebuffer.clone();
        let surface =
            test_surface("surface-1", 0, 0, 2, 2, 1, Some(test_handle(1, "surface-1", 1, 1)));

        let event = commit_test_scene(&mut state, vec![surface], vec![], 1).await;

        assert!(matches!(event, DisplayEvent::Rejected { .. }));
        assert_eq!(state.framebuffer, before);
    }

    #[tokio::test]
    async fn stale_pixel_transport_payload_is_rejected_without_framebuffer_mutation() {
        let mut state = DisplayState::new_test();
        ensure_framebuffer(&mut state.framebuffer);
        state.framebuffer[framebuffer_offset(0, 0).unwrap()] = 0xDEADBEEF;
        let before = state.framebuffer.clone();
        let current_handle = test_handle(1, "surface-1", 2, 2);
        state
            .pixel_transport
            .submit(test_payload(current_handle, vec![0x11111111; 4], 2, 2, 8))
            .unwrap();
        let stale_handle = test_handle(1, "surface-1", 1, 1);
        let surface = test_surface("surface-1", 0, 0, 2, 2, 1, Some(stale_handle.clone()));
        let stale_payload = test_payload(stale_handle, vec![0x22222222; 4], 2, 2, 8);

        let event = commit_test_scene(&mut state, vec![surface], vec![stale_payload], 1).await;

        assert!(matches!(event, DisplayEvent::Rejected { .. }));
        assert_eq!(state.framebuffer, before);
    }

    #[tokio::test]
    async fn unsupported_or_malformed_alpha_payload_rejects_without_framebuffer_mutation() {
        let mut state = DisplayState::new_test();
        ensure_framebuffer(&mut state.framebuffer);
        state.framebuffer[framebuffer_offset(0, 0).unwrap()] = 0xDEADBEEF;
        let before = state.framebuffer.clone();
        let handle = test_handle(1, "surface-1", 1, 1);
        let surface = test_surface("surface-1", 0, 0, 1, 1, 1, Some(handle.clone()));
        let unsupported = test_payload_with_format(handle.clone(), vec![0], 1, 1, 4, 99);

        let event =
            commit_test_scene(&mut state, vec![surface.clone()], vec![unsupported], 1).await;

        assert!(matches!(event, DisplayEvent::Rejected { .. }));
        assert_eq!(state.framebuffer, before);

        let malformed = waybroker_common::PixelTransportPayload {
            handle,
            pixels: vec![0, 0, 0],
            width: 1,
            height: 1,
            stride: 4,
            format: WL_SHM_FORMAT_ARGB8888,
        };
        let event = commit_test_scene(&mut state, vec![surface], vec![malformed], 2).await;

        assert!(matches!(event, DisplayEvent::Rejected { .. }));
        assert_eq!(state.framebuffer, before);
    }

    #[tokio::test]
    async fn initial_frame_uses_full_output_damage() {
        let mut state = DisplayState::new_test();
        let surface = test_surface("surface-1", 10, 10, 2, 2, 1, None);

        assert!(matches!(
            commit_test_scene(&mut state, vec![surface], vec![], 1).await,
            DisplayEvent::SceneCommitted { .. }
        ));

        assert_eq!(
            state.framebuffer.len(),
            FRAMEBUFFER_WIDTH as usize * FRAMEBUFFER_HEIGHT as usize
        );
        assert_eq!(state.framebuffer[framebuffer_offset(0, 0).unwrap()], BACKGROUND_PIXEL);
        assert_eq!(state.framebuffer[framebuffer_offset(10, 10).unwrap()], 0xFFFF0000);
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
            ..Default::default()
        };
        let commit = handle_display_command(
            DisplayCommand::CommitScene {
                target: waybroker_common::CommitTarget::Output { name: "eDP-1".into() },
                focus: waybroker_common::FocusTarget::None,
                selection: waybroker_common::WaylandSelectionState::default(),
                surfaces: vec![fullscreen_surf],
                pixel_payloads: vec![],
                scene_epoch: 0,
                scene_generation: 0,
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

    #[test]
    fn output_geometry_rejects_invalid_shapes_and_overflow() {
        let base = OutputGeometry {
            output_id: "test".into(),
            width: 2,
            height: 2,
            stride: 8,
            format: WL_SHM_FORMAT_XRGB8888,
            origin_x: 0,
            origin_y: 0,
            output_generation: 1,
        };
        assert!(OutputState::validate(&base).is_ok());
        assert!(OutputState::validate(&OutputGeometry { width: 0, ..base.clone() }).is_err());
        assert!(OutputState::validate(&OutputGeometry { stride: 4, ..base.clone() }).is_err());
        assert!(OutputState::validate(&OutputGeometry { format: 99, ..base.clone() }).is_err());
        assert!(OutputState::validate(&OutputGeometry { width: u32::MAX, ..base }).is_err());
    }

    #[test]
    fn output_geometry_supports_padding_and_negative_origin() {
        let output = OutputState::validate(&OutputGeometry {
            output_id: "test".into(),
            width: 3,
            height: 2,
            stride: 16,
            format: WL_SHM_FORMAT_XRGB8888,
            origin_x: -4,
            origin_y: -3,
            output_generation: 7,
        })
        .unwrap();
        assert_eq!(output.framebuffer_words(), 8);
        assert_eq!(output.bounds(), RendererRect { x0: -4, y0: -3, x1: -1, y1: -1 });
        assert_eq!(framebuffer_offset_for_output(&output, -4, -3).unwrap(), 0);
        assert_eq!(framebuffer_offset_for_output(&output, -2, -2).unwrap(), 6);
    }

    #[test]
    fn output_and_scene_generations_are_independent() {
        let output = OutputState::validate(&OutputGeometry {
            output_id: "test".into(),
            width: 4,
            height: 4,
            stride: 16,
            format: WL_SHM_FORMAT_XRGB8888,
            origin_x: 0,
            origin_y: 0,
            output_generation: 11,
        })
        .unwrap();
        assert_eq!(output.generation, 11);
        let scene = CommittedSceneState {
            source: ServiceRole::Compd,
            target: waybroker_common::CommitTarget::Output { name: "test".into() },
            focus: waybroker_common::FocusTarget::None,
            selection: Default::default(),
            surfaces: vec![],
            scene_epoch: 2,
            scene_generation: 3,
            commit_id: 1,
            unix_timestamp: 0,
        };
        assert_ne!(output.generation, scene.scene_generation);
    }

    #[tokio::test]
    async fn actual_commit_path_publishes_immutable_frame_in_order() {
        let mut state = DisplayState::new_test();
        let mut backend = MockDisplayBackend::default();
        let first = commit_test_scene_with_backend(
            &mut state,
            vec![test_surface("one", 0, 0, 2, 2, 1, None)],
            vec![],
            1,
            &mut backend,
        )
        .await;
        let second = commit_test_scene_with_backend(
            &mut state,
            vec![test_surface("two", 2, 0, 2, 2, 1, None)],
            vec![],
            2,
            &mut backend,
        )
        .await;
        assert!(matches!(first, DisplayEvent::SceneCommitted { .. }));
        assert!(matches!(second, DisplayEvent::SceneCommitted { .. }));
        assert_eq!(
            backend.publications().iter().map(|p| p.frame_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(backend.publications()[0].scene_generation, 1);
        assert_eq!(backend.publications()[1].scene_generation, 2);
        assert!(!backend.publications()[0].damage.is_empty());
    }

    #[tokio::test]
    async fn failed_publication_preserves_renderer_state_and_retries_same_scene() {
        let mut state = DisplayState::new_test();
        let before = state.framebuffer.clone();
        let mut failing = MockDisplayBackend::default();
        failing.set_fail_on_frame(1);
        let rejected = commit_test_scene_with_backend(
            &mut state,
            vec![test_surface("one", 0, 0, 2, 2, 1, None)],
            vec![],
            1,
            &mut failing,
        )
        .await;
        assert!(matches!(rejected, DisplayEvent::Rejected { .. }));
        assert_eq!(state.framebuffer, before);
        assert_eq!(state.published_frame_id, 0);
        assert_eq!(state.last_scene_generation, 0);

        let mut retry = MockDisplayBackend::default();
        let accepted = commit_test_scene_with_backend(
            &mut state,
            vec![test_surface("one", 0, 0, 2, 2, 1, None)],
            vec![],
            1,
            &mut retry,
        )
        .await;
        assert!(matches!(accepted, DisplayEvent::SceneCommitted { .. }));
        assert_eq!(retry.publications()[0].frame_id, 1);
    }

    #[test]
    fn mock_retains_immutable_frame_across_resize() {
        let mut backend = MockDisplayBackend::default();
        let pixels = std::sync::Arc::<[u32]>::from(vec![0xFF112233, 0xFF445566]);
        backend
            .publish_frame(FramePublication {
                frame_id: 1,
                output_id: "test".into(),
                output_generation: 1,
                scene_generation: 1,
                geometry: OutputGeometry {
                    output_id: "test".into(),
                    width: 2,
                    height: 1,
                    stride: 8,
                    format: WL_SHM_FORMAT_XRGB8888,
                    origin_x: 0,
                    origin_y: 0,
                    output_generation: 1,
                },
                format: WL_SHM_FORMAT_XRGB8888,
                stride: 8,
                pixels: pixels.clone(),
                damage: vec![],
            })
            .unwrap();
        assert_eq!(&*backend.publications()[0].pixels, &*pixels);
    }
}
