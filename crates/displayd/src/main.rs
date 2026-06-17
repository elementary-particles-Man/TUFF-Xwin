use std::{
    collections::HashMap,
    env, fs,
    io::BufReader,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
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
            "service=displayd op=vulkan_init event=success driver={} device={}",
            caps.driver_name, caps.device_name
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
            CaptureMethod::Portal => Box::new(PortalCaptureBackendStub),
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
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();
        config.session_instance_id = DEFAULT_SESSION_INSTANCE_ID.to_string();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--once" => config.serve_once = true,
                "--fail-resume" => config.fail_resume = true,
                "--vulkan" => config.use_vulkan = true,
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
                "--help" | "-h" => {
                    println!(
                        "usage: displayd [--once] [--fail-resume] [--vulkan] [--session-instance-id ID] [--socket PATH] [--capture-backend fake|real] [--allow-real-capture] [--capture-method stub|x11|portal] [--x11-display DISPLAY] [--allow-portal-capture]"
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
        }

        if config.allow_portal_capture && config.capture_method != CaptureMethod::Portal {
            bail!("--allow-portal-capture requires --capture-method portal");
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
            if let Some(vulkan) = vulkan {
                let handle = vulkan.submit_batch(VulkanBatchSubmission {
                    workload: VulkanWorkloadClass::MaintenanceHashing,
                    payload_len: surfaces.len() * 512, // シミュレート
                    surface_words: None,
                    timeout: Duration::from_millis(50),
                    requires_zeroize: false,
                    allows_gpu: true,
                });
                let result = vulkan.wait_for_completion(handle).await;
                println!(
                    "service=displayd op=vulkan_hashing event=completed workload={:?} path={:?}",
                    result.workload, result.path
                );
            }

            let commit_id = state.next_commit_id;
            let snapshot = CommittedSceneState {
                source,
                target: target.clone(),
                focus: focus.clone(),
                selection: selection.clone(),
                surfaces,
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

            println!(
                "service=displayd op=commit_scene event=success commit_id={} surfaces={} path={} session_instance_id={}",
                commit_id,
                surface_count,
                state.snapshot_path.display(),
                config.session_instance_id
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

trait CaptureBackend {
    fn capture(&self, output: &str) -> Result<(u32, u32, Vec<u32>)>;
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

impl CaptureBackend for FakeCaptureBackend {
    fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
        let width = 1920;
        let height = 1080;
        Ok((width, height, generate_mock_pixels(width, height)))
    }
}

struct RealCaptureBackendStub;

impl CaptureBackend for RealCaptureBackendStub {
    fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
        bail!(
            "real screen capture is not implemented/supported in this environment (RealCaptureBackendStub)"
        )
    }
}

struct PortalCaptureBackendStub;

impl CaptureBackend for PortalCaptureBackendStub {
    fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
        bail!(
            "PipeWire/portal screen capture is not implemented/supported in this environment (PortalCaptureBackendStub)"
        )
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
impl CaptureBackend for X11CaptureBackend {
    fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
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
impl CaptureBackend for X11CaptureBackend {
    fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
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

    let (width, height, mut pixels) = backend.capture(output)?;
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
    let r = ((pixel >> 16) & 0xFF) as u8;
    let g = ((pixel >> 8) & 0xFF) as u8;
    let b = (pixel & 0xFF) as u8;
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

        let mut state = DisplayState {
            last_scene: None,
            next_commit_id: 1,
            snapshot_path: std::env::temp_dir().join("scene-snapshot"),
            active_recordings: HashMap::new(),
            pointer_constraints: HashMap::new(),
            presentation_feedbacks: HashMap::new(),
        };

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
        let mut state = DisplayState {
            last_scene: None,
            next_commit_id: 1,
            snapshot_path: std::env::temp_dir().join("scene-snapshot"),
            active_recordings: HashMap::new(),
            pointer_constraints: HashMap::new(),
            presentation_feedbacks: HashMap::new(),
        };
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
        assert_eq!(u32_to_rgba8888(0xAA112233), expected);
        assert_eq!(u32_to_rgba8888(u32::from_be_bytes([0xAA, 0x11, 0x22, 0x33])), expected);
        assert_eq!(u32_to_rgba8888(u32::from_le_bytes([0x33, 0x22, 0x11, 0xAA])), expected);
    }

    #[test]
    fn displayd_writes_expected_rgba8888_bytes_for_known_pixels() {
        let bytes = encode_rgba8888_artifact_bytes(
            1,
            1,
            &[((0xAAu32) << 24) | ((0x11u32) << 16) | ((0x22u32) << 8) | 0x33],
        )
        .unwrap();
        assert_eq!(bytes, vec![0x11, 0x22, 0x33, 0xAA]);
    }

    #[tokio::test]
    async fn test_handle_capture_output_propagates_backend_error() {
        struct FailingCaptureBackend;

        impl CaptureBackend for FailingCaptureBackend {
            fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
                Err(anyhow::anyhow!("capture failed"))
            }
        }

        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut state = DisplayState {
            last_scene: None,
            next_commit_id: 1,
            snapshot_path: std::env::temp_dir().join("scene-snapshot"),
            active_recordings: HashMap::new(),
            pointer_constraints: HashMap::new(),
            presentation_feedbacks: HashMap::new(),
        };
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

        impl CaptureBackend for FailingCaptureBackend {
            fn capture(&self, _output: &str) -> Result<(u32, u32, Vec<u32>)> {
                Err(anyhow::anyhow!("capture failed"))
            }
        }

        let config = Config { session_instance_id: "test-session".into(), ..Default::default() };
        let mut state = DisplayState {
            last_scene: None,
            next_commit_id: 1,
            snapshot_path: std::env::temp_dir().join("scene-snapshot"),
            active_recordings: HashMap::new(),
            pointer_constraints: HashMap::new(),
            presentation_feedbacks: HashMap::new(),
        };
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
        let mut state = DisplayState {
            last_scene: None,
            next_commit_id: 1,
            snapshot_path: std::env::temp_dir().join("scene-snapshot"),
            active_recordings: HashMap::new(),
            pointer_constraints: HashMap::new(),
            presentation_feedbacks: HashMap::new(),
        };
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
        ]
        .into_iter()
        .skip(1);
        let config = Config::from_args(args).unwrap();
        assert_eq!(config.capture_backend, CaptureBackendType::Real);
        assert_eq!(config.capture_method, CaptureMethod::Portal);
        assert!(config.allow_portal_capture);
    }

    #[test]
    fn test_config_parser_rejects_portal_without_allow_portal() {
        let args = vec![
            "displayd".to_string(),
            "--capture-backend".to_string(),
            "real".to_string(),
            "--allow-real-capture".to_string(),
            "--capture-method".to_string(),
            "portal".to_string(),
        ]
        .into_iter()
        .skip(1);
        let err = Config::from_args(args).unwrap_err();
        assert!(err.to_string().contains("requires --allow-portal-capture"));
    }

    #[test]
    fn test_config_parser_rejects_allow_portal_without_portal_method() {
        let args = vec![
            "displayd".to_string(),
            "--capture-backend".to_string(),
            "real".to_string(),
            "--allow-real-capture".to_string(),
            "--allow-portal-capture".to_string(),
        ]
        .into_iter()
        .skip(1);
        let err = Config::from_args(args).unwrap_err();
        assert!(err.to_string().contains("requires --capture-method portal"));
    }

    #[tokio::test]
    async fn test_portal_capture_backend_stub_returns_error() {
        let backend = PortalCaptureBackendStub;
        let err = backend.capture("fullscreen").unwrap_err();
        assert!(err.to_string().contains("PipeWire/portal screen capture"));
        assert!(err.to_string().contains("PortalCaptureBackendStub"));
    }
}
