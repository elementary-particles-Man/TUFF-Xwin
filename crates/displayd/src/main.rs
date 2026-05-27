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
    ServiceBanner, ServiceEndpoint, ServiceRole, ServiceStream, bind_service_socket,
    ensure_runtime_dir, now_unix_timestamp, read_json_line, send_json_line, session_artifact_path,
};

const DEFAULT_SESSION_INSTANCE_ID: &str = "default-single-session";

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Displayd, "drm/kms, input, seat broker");
    println!("{}", banner.render());

    let vulkan = if config.use_vulkan {
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

    let listener = bind_service_socket(ServiceRole::Displayd)?;
    let _socket_guard = SocketGuard::new(listener.endpoint().clone());
    println!("service=displayd op=listen event=socket_bound path={}", listener.endpoint());

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = stream?;
        handle_client(stream, &config, &mut state, vulkan.as_ref()).await?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("service=displayd op=terminate event=finished served_requests={served}");
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct Config {
    serve_once: bool,
    fail_resume: bool,
    use_vulkan: bool,
    session_instance_id: String,
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
                "--help" | "-h" => {
                    println!(
                        "usage: displayd [--once] [--fail-resume] [--vulkan] [--session-instance-id ID]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

async fn handle_client(
    mut stream: ServiceStream,
    config: &Config,
    state: &mut DisplayState,
    vulkan: Option<&VulkanBackend>,
) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let response = build_response(request, config, state, vulkan).await?;
    send_json_line(&mut stream, &response)?;
    Ok(())
}

async fn build_response(
    request: IpcEnvelope,
    config: &Config,
    state: &mut DisplayState,
    vulkan: Option<&VulkanBackend>,
) -> Result<IpcEnvelope> {
    let source = request.source;
    let response_kind = match request.kind {
        MessageKind::DisplayCommand(command) if request.destination == ServiceRole::Displayd => {
            MessageKind::DisplayEvent(
                handle_display_command(command, source, config, state, vulkan).await?,
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
) -> Result<DisplayEvent> {
    match command {
        DisplayCommand::EnumerateOutputs => {
            println!("service=displayd op=enumerate_outputs event=success");
            Ok(DisplayEvent::OutputInventory { outputs: vec![stub_output_mode()] })
        }
        DisplayCommand::SetMode { output, mode } => {
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
            println!(
                "service=displayd op=commit_scene event=success commit_id={} surfaces={} path={} session_instance_id={}",
                commit_id,
                surface_count,
                state.snapshot_path.display(),
                config.session_instance_id
            );
            Ok(DisplayEvent::SceneCommitted { target, focus, selection, surface_count, commit_id })
        }
        DisplayCommand::GetSceneSnapshot { output } => {
            Ok(handle_scene_snapshot_request(output, state))
        }
        DisplayCommand::CaptureOutput { output } => {
            handle_capture_output(&output, config, state, vulkan).await
        }
        DisplayCommand::StartRecord { output, fps } => {
            handle_start_record(&output, fps, config, state).await
        }
        DisplayCommand::StopRecord { output } => handle_stop_record(&output, config, state).await,
        DisplayCommand::SecureBlank { output } => {
            println!("service=displayd op=secure_blank event=success output={:?}", output);
            Ok(DisplayEvent::BlankApplied { output })
        }
        DisplayCommand::SetGamma { output, .. } => {
            println!("service=displayd op=set_gamma event=success output={output}");
            Ok(DisplayEvent::GammaApplied { output })
        }
        DisplayCommand::SetPointerConstraints { output, constraints } => {
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
}

#[derive(Debug, Clone)]
struct RecordingState {
    session_id: String,
    fps: u32,
    start_timestamp: u64,
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

        Ok(Self { last_scene, next_commit_id, snapshot_path, active_recordings: HashMap::new() })
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
) -> Result<DisplayEvent> {
    println!("service=displayd op=capture_output event=begin output={output}");

    // Mock capture: 1920x1080 BGRA
    let width = 1920;
    let height = 1080;
    let mut pixels = generate_mock_pixels(width, height);

    if let Some(vulkan) = vulkan {
        // Use Vulkan for "Simulation" workload (as requested in handoff)
        let handle = vulkan.submit_batch(VulkanBatchSubmission {
            workload: VulkanWorkloadClass::ScreenshotRefine,
            payload_len: pixels.len() * 4,
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

    let artifact_name = format!("screenshot-{}-{}.raw", output, now_unix_timestamp());
    let artifact_path = session_artifact_path(&config.session_instance_id, &artifact_name);

    fs::write(&artifact_path, unsafe {
        std::slice::from_raw_parts(pixels.as_ptr() as *const u8, pixels.len() * 4)
    })?;

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
        artifact_path: artifact_path.to_string_lossy().to_string(),
    })
}

async fn handle_start_record(
    output: &str,
    fps: u32,
    _config: &Config,
    state: &mut DisplayState,
) -> Result<DisplayEvent> {
    if state.active_recordings.contains_key(output) {
        return Ok(DisplayEvent::Rejected {
            reason: format!("recording already active for output {output}"),
        });
    }

    let session_id = format!("rec-{}", now_unix_timestamp());
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
) -> Result<DisplayEvent> {
    let recording = match state.active_recordings.remove(output) {
        Some(r) => r,
        None => {
            return Ok(DisplayEvent::Rejected {
                reason: format!("no active recording for output {output}"),
            });
        }
    };

    let artifact_name = format!("recording-{}-{}.mkv", output, recording.session_id);
    let artifact_path = session_artifact_path(&config.session_instance_id, &artifact_name);

    // Mock: create an empty file to represent the finished recording
    // TODO: Integrate with PipeWire and real frame capture for actual recording
    fs::write(&artifact_path, b"mock-video-data")?;

    println!(
        "service=displayd op=stop_record event=success_mock output={output} session_id={} path={} info=\"artifact is mock data\"",
        recording.session_id,
        artifact_path.display()
    );

    Ok(DisplayEvent::RecordStopped {
        output: output.to_string(),
        session_id: recording.session_id,
        artifact_path: artifact_path.to_string_lossy().to_string(),
    })
}

fn generate_mock_pixels(width: u32, height: u32) -> Vec<u32> {
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let r = (x % 256) as u32;
            let g = (y % 256) as u32;
            let b = 128u32;
            let a = 255u32;
            // BGRA
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
        let temp_dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", temp_dir.path());
        }

        let session_id = "test-session";
        let config = Config { session_instance_id: session_id.into(), ..Default::default() };

        let state = DisplayState {
            last_scene: None,
            next_commit_id: 1,
            snapshot_path: temp_dir.path().join("scene-snapshot"),
            active_recordings: HashMap::new(),
        };

        // Ensure runtime dir exists
        ensure_runtime_dir().unwrap();
        let session_runtime_dir = temp_dir.path().join("waybroker").join(session_id);
        std::fs::create_dir_all(&session_runtime_dir).unwrap();

        let result = handle_capture_output("eDP-1", &config, &state, None).await.unwrap();

        if let DisplayEvent::OutputCaptured { output, width, height, format, artifact_path } =
            result
        {
            assert_eq!(output, "eDP-1");
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
            assert_eq!(format, "RGBA8888");
            assert!(artifact_path.contains("screenshot-eDP-1-"));
            assert!(std::path::Path::new(&artifact_path).exists());
        } else {
            panic!("Unexpected event: {:?}", result);
        }
    }
}
