use std::{
    env, fs,
    io::BufReader,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    CommittedSceneState, DisplayCommand, DisplayEvent, IpcEnvelope, MessageKind, OutputMode,
    ServiceBanner, ServiceRole, bind_service_socket, ensure_runtime_dir, now_unix_timestamp,
    read_json_line, send_json_line,
};

const SCENE_SNAPSHOT_FILE: &str = "displayd-last-scene.json";

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Displayd, "drm/kms, input, seat broker");
    println!("{}", banner.render());

    let mut state = DisplayState::load()?;

    let (listener, socket_path) = bind_service_socket(ServiceRole::Displayd)?;
    let _socket_guard = SocketGuard::new(socket_path.clone());
    println!("service=displayd op=listen event=socket_bound path={}", socket_path.display());

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = stream?;
        handle_client(stream, &config, &mut state)?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("service=displayd op=terminate event=finished served_requests={served}");
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct Config {
    serve_once: bool,
    fail_resume: bool,
}

impl Config {
    fn from_args(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

        for arg in args {
            match arg.as_str() {
                "--once" => config.serve_once = true,
                "--fail-resume" => config.fail_resume = true,
                "--help" | "-h" => {
                    println!("usage: displayd [--once] [--fail-resume]");
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

fn handle_client(mut stream: UnixStream, config: &Config, state: &mut DisplayState) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let response = build_response(request, config, state)?;
    send_json_line(&mut stream, &response)?;
    Ok(())
}

fn build_response(
    request: IpcEnvelope,
    config: &Config,
    state: &mut DisplayState,
) -> Result<IpcEnvelope> {
    let source = request.source;
    let response_kind = match request.kind {
        MessageKind::DisplayCommand(command) if request.destination == ServiceRole::Displayd => {
            MessageKind::DisplayEvent(handle_display_command(command, source, config, state)?)
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

fn handle_display_command(
    command: DisplayCommand,
    source: ServiceRole,
    config: &Config,
    state: &mut DisplayState,
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
        DisplayCommand::CommitScene { target, focus, surfaces } => {
            let commit_id = state.next_commit_id;
            let snapshot = CommittedSceneState {
                source,
                target: target.clone(),
                focus: focus.clone(),
                surfaces,
                commit_id,
                unix_timestamp: now_unix_timestamp(),
            };
            let surface_count = snapshot.surfaces.len();
            state.record_commit(snapshot)?;
            println!(
                "service=displayd op=commit_scene event=success commit_id={} surfaces={} path={}",
                commit_id,
                surface_count,
                state.snapshot_path.display()
            );
            Ok(DisplayEvent::SceneCommitted { target, focus, surface_count, commit_id })
        }
        DisplayCommand::GetSceneSnapshot { output } => {
            Ok(handle_scene_snapshot_request(output, state))
        }
        DisplayCommand::SecureBlank { output } => {
            println!("service=displayd op=secure_blank event=success output={:?}", output);
            Ok(DisplayEvent::BlankApplied { output })
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
}

impl DisplayState {
    fn load() -> Result<Self> {
        let snapshot_path = ensure_runtime_dir()?.join(SCENE_SNAPSHOT_FILE);
        let last_scene = load_scene_snapshot(&snapshot_path)?;
        let next_commit_id =
            last_scene.as_ref().map(|scene| scene.commit_id.saturating_add(1)).unwrap_or(1);

        match &last_scene {
            Some(scene) => {
                println!(
                    "service=displayd op=scene_cache event=loaded commit_id={} source={} surfaces={} path={}",
                    scene.commit_id,
                    scene.source.as_str(),
                    scene.surfaces.len(),
                    snapshot_path.display()
                );
            }
            None => {
                println!(
                    "service=displayd op=scene_cache event=empty path={}",
                    snapshot_path.display()
                );
            }
        }

        Ok(Self { last_scene, next_commit_id, snapshot_path })
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
    path: PathBuf,
}

impl SocketGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
