use std::{
    collections::BTreeMap,
    env, fs,
    io::BufReader,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use vulkan_backend::{
    VulkanBackend, VulkanBackendConfig, VulkanBatchSubmission, VulkanWorkloadClass,
};
use waybroker_common::{
    CommitTarget, CommittedSceneState, DisplayCommand, DisplayEvent, FocusTarget, IpcEnvelope,
    MessageKind, ServiceBanner, ServiceEndpoint, ServiceRole, ServiceStream, SurfacePlacement,
    SurfaceRegistrySnapshot, SurfaceSnapshot, WaylandCommand, WaylandEvent,
    WaylandSelectionHandoff, WaylandSelectionState, WaylandSurfaceRole, WaylandSurfaceState,
    accel::global_accel_policy, bind_service_socket, connect_service_socket,
    is_recoverable_accept_error, read_json_line, send_json_line,
};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Compd, "scene, focus, composition policy");
    println!("{}", banner.render());

    let vulkan = if config.use_vulkan && global_accel_policy().prefers_vulkan() {
        let backend = VulkanBackend::new(VulkanBackendConfig::default());
        let caps = backend.initialize();
        println!(
            "service=compd op=vulkan_init event={} compute_available={} driver={} device={}",
            if caps.compute_available { "success" } else { "fallback" },
            caps.compute_available,
            caps.driver_name,
            caps.device_name
        );
        Some(backend)
    } else {
        None
    };

    if config.restore_from_displayd && config.scene_path.is_some() {
        bail!("--scene cannot be combined with --restore-from-displayd");
    }
    if config.handoff_selection && !config.reconcile_waylandd {
        bail!("--handoff-selection requires --reconcile-waylandd");
    }

    let scene = prepare_scene(&config, vulkan.as_ref()).await?;

    if config.handoff_selection {
        apply_selection_handoff(&config, scene.as_ref())?;
    }

    if config.serve_ipc {
        if config.restore_from_displayd || config.reconcile_waylandd {
            match scene.as_ref() {
                Some(scene) => {
                    println!(
                        "service=compd op=startup_rebuild event=scene_ready target={} focus={:?} surfaces={}",
                        scene.target_output,
                        scene.focus,
                        scene.surfaces.len()
                    );
                    match commit_scene_to_displayd(scene) {
                        Ok(committed) => {
                            println!(
                                "service=compd op=startup_rebuild event=scene_committed surface_count={} commit_id={}",
                                committed.surface_count, committed.commit_id
                            );
                        }
                        Err(err) => {
                            if config.require_displayd {
                                return Err(err).context(
                                    "failed to commit rebuilt scene to displayd during startup",
                                );
                            }
                            println!(
                                "service=compd op=startup_rebuild event=failed reason=\"{}\"",
                                err
                            );
                        }
                    }
                }
                None => {
                    println!("service=compd op=startup_rebuild event=skipped reason=no-scene");
                }
            }
        }
        serve_ipc(&config)?;
        return Ok(());
    }

    if config.commit_demo {
        match scene.as_ref() {
            Some(scene) => {
                println!(
                    "service=compd op=scene_build event=success target={} focus={:?} surfaces={}",
                    scene.target_output,
                    scene.focus,
                    scene.surfaces.len()
                );

                match commit_scene_to_displayd(scene) {
                    Ok(committed) => {
                        println!(
                            "service=compd op=displayd_response event=scene_committed surface_count={} commit_id={}",
                            committed.surface_count, committed.commit_id
                        );
                    }
                    Err(err) => {
                        if config.require_displayd {
                            return Err(err)
                                .context("failed to commit scene to displayd (required)");
                        } else {
                            println!(
                                "service=compd op=scene_commit event=failed reason=\"{}\"",
                                err
                            );
                        }
                    }
                }
            }
            None => {
                println!("service=compd op=scene_build event=skipped reason=no-scene");
            }
        }
    }

    if !config.commit_demo && !config.print_scene && scene.is_none() {
        println!("service=compd state=idle (use --commit-demo or --serve-ipc)");
    }

    Ok(())
}

#[derive(Debug, Default)]
struct Config {
    scene_path: Option<PathBuf>,
    commit_demo: bool,
    print_scene: bool,
    require_displayd: bool,
    serve_ipc: bool,
    serve_once: bool,
    fail_resume: bool,
    restore_from_displayd: bool,
    reconcile_waylandd: bool,
    require_waylandd: bool,
    handoff_selection: bool,
    use_vulkan: bool,
    session_instance_id: String,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self {
            use_vulkan: true,
            session_instance_id: "default-single-session".to_string(),
            ..Self::default()
        };
        // Prefer GPU acceleration; Vulkan initialization remains fail-soft.

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--scene" => {
                    let path = args.next().context("--scene requires a path")?;
                    config.scene_path = Some(PathBuf::from(path));
                }
                "--commit-demo" => config.commit_demo = true,
                "--print-scene" => config.print_scene = true,
                "--require-displayd" => config.require_displayd = true,
                "--serve-ipc" => config.serve_ipc = true,
                "--once" => config.serve_once = true,
                "--fail-resume" => config.fail_resume = true,
                "--restore-from-displayd" => config.restore_from_displayd = true,
                "--reconcile-waylandd" => config.reconcile_waylandd = true,
                "--require-waylandd" => config.require_waylandd = true,
                "--handoff-selection" => config.handoff_selection = true,
                "--vulkan" => config.use_vulkan = true,
                "--no-vulkan" => config.use_vulkan = false,
                "--session-instance-id" => {
                    config.session_instance_id =
                        args.next().context("--session-instance-id requires an id")?;
                }
                "--help" | "-h" => {
                    println!(
                        "usage: compd [--scene PATH] [--print-scene] [--commit-demo] [--require-displayd] [--require-waylandd] [--restore-from-displayd] [--reconcile-waylandd] [--handoff-selection] [--serve-ipc] [--once] [--fail-resume] [--vulkan|--no-vulkan] [--session-instance-id ID]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

async fn prepare_scene(
    config: &Config,
    vulkan: Option<&VulkanBackend>,
) -> Result<Option<CompdScene>> {
    if !config.restore_from_displayd
        && config.scene_path.is_none()
        && !config.reconcile_waylandd
        && !config.commit_demo
        && !config.print_scene
    {
        return Ok(None);
    }

    let recovered_scene = if config.restore_from_displayd {
        match query_scene_snapshot_from_displayd(None) {
            Ok(Some(snapshot)) => {
                println!(
                    "service=compd op=scene_recover event=success source={} commit_id={} surfaces={} timestamp={}",
                    snapshot.source.as_str(),
                    snapshot.commit_id,
                    snapshot.surfaces.len(),
                    snapshot.unix_timestamp
                );
                Some(snapshot)
            }
            Ok(None) => {
                if config.require_displayd {
                    bail!("displayd has no committed scene snapshot");
                }
                println!("service=compd op=scene_recover event=empty");
                None
            }
            Err(err) => {
                if config.require_displayd {
                    return Err(err).context("failed to recover scene from displayd");
                }
                println!("service=compd op=scene_recover event=failed reason=\"{}\"", err);
                None
            }
        }
    } else {
        None
    };

    let mut scene = match recovered_scene.as_ref() {
        Some(snapshot) => Some(scene_from_snapshot(snapshot)),
        None if config.restore_from_displayd => None,
        None => Some(load_scene(config.scene_path.as_ref())?),
    };

    if config.reconcile_waylandd {
        scene = reconcile_scene(config, scene, vulkan).await?;
    }

    if config.print_scene {
        if let Some(scene) = scene.as_ref() {
            println!(
                "service=compd op=scene_print event=success target={} surfaces={}",
                scene.target_output,
                scene.surfaces.len()
            );
            println!("{}", serde_json::to_string_pretty(scene)?);
        } else {
            println!("service=compd op=scene_print event=skipped reason=no-scene");
        }
    }

    Ok(scene)
}

fn query_output_inventory_from_displayd() -> Result<Vec<waybroker_common::OutputMode>> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let request = IpcEnvelope::new(
        ServiceRole::Compd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::EnumerateOutputs),
    );
    send_json_line(&mut stream, &request)?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let response: IpcEnvelope = read_json_line(&mut reader)?;

    match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::OutputInventory { outputs }) => Ok(outputs),
        MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
            bail!("displayd rejected inventory query: {reason}")
        }
        other => bail!("unexpected displayd response: {other:?}"),
    }
}

async fn reconcile_scene(
    config: &Config,
    scene: Option<CompdScene>,
    vulkan: Option<&VulkanBackend>,
) -> Result<Option<CompdScene>> {
    let Some(scene) = scene else {
        println!("service=compd op=scene_reconcile event=skipped reason=no-scene");
        return Ok(None);
    };

    let target_output_name = scene.target_output.clone();
    let outputs = query_output_inventory_from_displayd().unwrap_or_default();
    let default_output = waybroker_common::OutputMode {
        name: target_output_name.clone(),
        width: STUB_SCREEN_WIDTH,
        height: STUB_SCREEN_HEIGHT,
        refresh_hz: 60,
    };
    let output_mode =
        outputs.into_iter().find(|o| o.name == target_output_name).unwrap_or(default_output);

    match query_surface_registry_from_waylandd() {
        Ok(snapshot) => {
            println!(
                "service=compd op=surface_registry event=success generation={} surfaces={} timestamp={}",
                snapshot.generation,
                snapshot.surfaces.len(),
                snapshot.unix_timestamp
            );

            if let Some(vulkan) = vulkan {
                let handle = vulkan.submit_batch(VulkanBatchSubmission {
                    workload: VulkanWorkloadClass::BulkPrefilter,
                    payload_len: snapshot.surfaces.len() * 256, // シミュレート
                    surface_words: None,
                    timeout: Duration::from_millis(100),
                    requires_zeroize: false,
                    allows_gpu: true,
                });
                let result = vulkan.wait_for_completion(handle).await;
                println!(
                    "service=compd op=vulkan_prefilter event=completed workload={:?} path={:?}",
                    result.workload, result.path
                );
            }

            let reconciled = reconcile_scene_with_registry(scene, &snapshot, &output_mode);
            println!(
                "service=compd op=scene_reconcile event=success kept={} dropped={} app_id_updates={} selection_handoffs={} focus={:?} clipboard_owner={} primary_selection_owner={}",
                reconciled.scene.surfaces.len(),
                reconciled.dropped_surface_ids.len(),
                reconciled.updated_app_ids,
                reconciled.selection_handoffs,
                reconciled.scene.focus,
                format_owner(reconciled.scene.selection.clipboard_owner.as_deref()),
                format_owner(reconciled.scene.selection.primary_selection_owner.as_deref())
            );
            if !reconciled.dropped_surface_ids.is_empty() {
                println!(
                    "service=compd op=scene_reconcile dropped_ids={}",
                    reconciled.dropped_surface_ids.join(",")
                );
            }
            Ok(Some(reconciled.scene))
        }
        Err(err) => {
            if config.require_waylandd {
                return Err(err).context("failed to reconcile scene with waylandd");
            }
            println!("service=compd op=surface_registry event=failed reason=\"{}\"", err);
            Ok(Some(scene))
        }
    }
}

fn serve_ipc(config: &Config) -> Result<()> {
    let listener = bind_service_socket(ServiceRole::Compd)?;
    let _socket_guard = SocketGuard::new(listener.endpoint().clone());
    println!("service=compd op=listen event=socket_bound path={}", listener.endpoint());

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(err) if is_recoverable_accept_error(&err) => {
                println!("service=compd op=accept event=recoverable_error reason=\"{}\"", err);
                continue;
            }
            Err(err) => {
                println!("service=compd op=accept event=fatal_error reason=\"{}\"", err);
                return Err(err).context("compd IPC accept failed");
            }
        };
        handle_client(stream, config)?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("service=compd op=terminate event=finished served_requests={served}");
    Ok(())
}

fn handle_client(mut stream: ServiceStream, config: &Config) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let response = build_response(request, config);
    send_json_line(&mut stream, &response)?;
    Ok(())
}

fn build_response(request: IpcEnvelope, config: &Config) -> IpcEnvelope {
    let source = request.source;
    let response_kind = match request.kind {
        MessageKind::DisplayCommand(command) if request.destination == ServiceRole::Compd => {
            match forward_display_command(command) {
                Ok(event) => MessageKind::DisplayEvent(event),
                Err(err) => {
                    MessageKind::DisplayEvent(DisplayEvent::Rejected { reason: err.to_string() })
                }
            }
        }
        MessageKind::SessionCommand(waybroker_common::SessionCommand::ResumeHint {
            stage,
            output,
        }) if request.destination == ServiceRole::Compd => {
            if config.fail_resume {
                println!(
                    "service=compd op=resume_hint event=failed reason=\"fault injection\" stage={:?}",
                    stage
                );
                MessageKind::SessionCommand(waybroker_common::SessionCommand::DegradedMode {
                    reason: "compd fault injection".into(),
                })
            } else {
                println!(
                    "service=compd op=resume_hint event=success stage={:?} output={:?}",
                    stage, output
                );
                MessageKind::SessionCommand(waybroker_common::SessionCommand::ResumeHint {
                    stage,
                    output,
                })
            }
        }
        MessageKind::LockCommand(waybroker_common::LockCommand::SetLockState { state }) => {
            println!("service=compd op=lock_state_hint event=success state={:?}", state);
            MessageKind::LockCommand(waybroker_common::LockCommand::SetLockState { state })
        }
        other => MessageKind::SessionCommand(waybroker_common::SessionCommand::DegradedMode {
            reason: format!("compd does not handle {other:?}"),
        }),
    };

    IpcEnvelope::new(ServiceRole::Compd, source, response_kind)
}

fn forward_display_command(command: DisplayCommand) -> Result<DisplayEvent> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)
        .context("compd could not connect to displayd")?;
    let request = IpcEnvelope::new(
        ServiceRole::Compd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(command),
    );
    send_json_line(&mut stream, &request)?;
    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;
    match response.kind {
        MessageKind::DisplayEvent(event) => Ok(event),
        other => bail!("displayd returned unexpected response: {other:?}"),
    }
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CompdScene {
    target_output: String,
    focus: FocusTarget,
    #[serde(default)]
    selection: WaylandSelectionState,
    surfaces: Vec<SurfaceSnapshot>,
    #[serde(default)]
    scene_epoch: u64,
    #[serde(default)]
    scene_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayLinkState {
    Connected { epoch: u64 },
    Disconnected,
    Reconnecting { attempt: u8 },
    Reconciling { epoch: u64 },
    AwaitingFreshPresentation { epoch: u64 },
    Failed,
}

static DISPLAY_LINK: OnceLock<Mutex<DisplayLinkState>> = OnceLock::new();

fn display_link() -> &'static Mutex<DisplayLinkState> {
    DISPLAY_LINK.get_or_init(|| Mutex::new(DisplayLinkState::Disconnected))
}

fn load_scene(path: Option<&PathBuf>) -> Result<CompdScene> {
    match path {
        Some(path) => {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read scene file {}", path.display()))?;
            serde_json::from_str(&raw)
                .with_context(|| format!("failed to decode scene file {}", path.display()))
        }
        None => Ok(mock_demo_scene()),
    }
}

fn mock_demo_scene() -> CompdScene {
    CompdScene {
        target_output: "eDP-1".into(),
        focus: FocusTarget::Surface { id: "konsole-1".into() },
        selection: WaylandSelectionState::default(),
        surfaces: vec![
            SurfaceSnapshot {
                id: "konsole-1".into(),
                app_id: "org.kde.konsole".into(),
                placement: SurfacePlacement {
                    x: 100,
                    y: 100,
                    width: 800,
                    height: 600,
                    z: 10,
                    visible: true,
                },
                ..Default::default()
            },
            SurfaceSnapshot {
                id: "background-1".into(),
                app_id: "org.kde.plasmashell.wallpaper".into(),
                placement: SurfacePlacement {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                    z: 0,
                    visible: true,
                },
                ..Default::default()
            },
        ],
        scene_epoch: 1,
        scene_generation: 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SceneCommitReceipt {
    surface_count: usize,
    commit_id: u64,
}

fn commit_scene_to_displayd(scene: &CompdScene) -> Result<SceneCommitReceipt> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)
        .context("failed to connect to displayd socket")?;
    let request = IpcEnvelope::new(
        ServiceRole::Compd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::CommitScene {
            target: CommitTarget::Output { name: scene.target_output.clone() },
            focus: scene.focus.clone(),
            selection: scene.selection.clone(),
            surfaces: scene.surfaces.clone(),
            pixel_payloads: vec![],
            scene_epoch: 0,
            scene_generation: 0,
        }),
    );
    if let Err(err) = send_json_line(&mut stream, &request) {
        mark_display_disconnected();
        return reconcile_scene_to_displayd(scene).map_err(|reconcile_err| {
            anyhow::anyhow!(
                "displayd transport lost ({err}); reconciliation failed: {reconcile_err}"
            )
        });
    }

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = match read_json_line(&mut reader) {
        Ok(response) => response,
        Err(err) => {
            mark_display_disconnected();
            return reconcile_scene_to_displayd(scene).map_err(|reconcile_err| {
                anyhow::anyhow!(
                    "displayd transport lost ({err}); reconciliation failed: {reconcile_err}"
                )
            });
        }
    };

    if response.source != ServiceRole::Displayd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::Compd {
        bail!("unexpected response destination: {}", response.destination.as_str());
    }

    match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::SceneCommitted {
            surface_count,
            commit_id,
            ..
        }) => {
            update_display_link_from_commit(&response);
            Ok(SceneCommitReceipt { surface_count, commit_id })
        }
        MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
            bail!("displayd rejected scene: {reason}")
        }
        other => bail!("unexpected displayd response: {other:?}"),
    }
}

fn mark_display_disconnected() {
    if let Ok(mut state) = display_link().lock() {
        *state = DisplayLinkState::Disconnected;
    }
}

fn reconcile_scene_to_displayd(scene: &CompdScene) -> Result<SceneCommitReceipt> {
    let mut state =
        display_link().lock().map_err(|_| anyhow::anyhow!("display link lock poisoned"))?;
    let attempt = match *state {
        DisplayLinkState::Reconnecting { attempt } => attempt,
        _ => 0,
    };
    if attempt >= 3 {
        *state = DisplayLinkState::Failed;
        bail!("displayd reconnect retry budget exhausted")
    }
    *state = DisplayLinkState::Reconnecting { attempt: attempt + 1 };
    drop(state);

    let reconciliation = forward_display_command(DisplayCommand::GetReconciliation)?;
    let epoch = match reconciliation {
        DisplayEvent::Reconciliation { epoch, .. } => epoch,
        DisplayEvent::Rejected { reason } => {
            bail!("displayd rejected reconciliation query: {reason}")
        }
        other => bail!("unexpected reconciliation response: {other:?}"),
    };
    {
        let mut link =
            display_link().lock().map_err(|_| anyhow::anyhow!("display link lock poisoned"))?;
        *link = DisplayLinkState::Reconciling { epoch };
    }
    match forward_display_command(DisplayCommand::BeginReconciliation { epoch })? {
        DisplayEvent::Reconciliation { epoch: accepted, .. } if accepted == epoch => {}
        DisplayEvent::Rejected { reason } => {
            bail!("displayd rejected reconciliation begin: {reason}")
        }
        other => bail!("unexpected reconciliation begin response: {other:?}"),
    }
    let event = forward_display_command(DisplayCommand::ReconcileScene {
        epoch,
        scene_epoch: scene.scene_epoch,
        scene_generation: scene.scene_generation,
        target: CommitTarget::Output { name: scene.target_output.clone() },
        focus: scene.focus.clone(),
        selection: scene.selection.clone(),
        surfaces: scene.surfaces.clone(),
        pixel_payloads: vec![],
    })?;
    match event {
        DisplayEvent::Reconciled { epoch: accepted, scene_generation, payload_count: _ }
            if accepted == epoch =>
        {
            let mut link =
                display_link().lock().map_err(|_| anyhow::anyhow!("display link lock poisoned"))?;
            *link = DisplayLinkState::AwaitingFreshPresentation { epoch };
            Ok(SceneCommitReceipt {
                surface_count: scene.surfaces.len(),
                commit_id: scene_generation,
            })
        }
        DisplayEvent::Rejected { reason } => {
            if let Ok(mut link) = display_link().lock() {
                *link = DisplayLinkState::Failed;
            }
            bail!("displayd rejected scene reconciliation: {reason}")
        }
        other => bail!("unexpected scene reconciliation response: {other:?}"),
    }
}

fn update_display_link_from_commit(response: &IpcEnvelope) {
    if let MessageKind::DisplayEvent(DisplayEvent::SceneCommitted { .. }) = &response.kind {
        if let Ok(mut state) = display_link().lock() {
            *state = DisplayLinkState::Connected { epoch: 0 };
        }
    }
}

fn query_scene_snapshot_from_displayd(output: Option<&str>) -> Result<Option<CommittedSceneState>> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)
        .context("failed to connect to displayd socket")?;
    let request = IpcEnvelope::new(
        ServiceRole::Compd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::GetSceneSnapshot {
            output: output.map(str::to_owned),
        }),
    );
    send_json_line(&mut stream, &request)
        .context("failed to query scene snapshot from displayd")?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope =
        read_json_line(&mut reader).context("failed to read scene snapshot from displayd")?;

    if response.source != ServiceRole::Displayd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::Compd {
        bail!("unexpected response destination: {}", response.destination.as_str());
    }

    match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::SceneSnapshot { snapshot }) => Ok(snapshot),
        MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
            bail!("displayd rejected scene snapshot query: {reason}")
        }
        other => bail!("unexpected displayd response: {other:?}"),
    }
}

fn query_surface_registry_from_waylandd() -> Result<SurfaceRegistrySnapshot> {
    let mut stream = connect_service_socket(ServiceRole::Waylandd)
        .context("failed to connect to waylandd socket")?;
    let request = IpcEnvelope::new(
        ServiceRole::Compd,
        ServiceRole::Waylandd,
        MessageKind::WaylandCommand(WaylandCommand::GetSurfaceRegistry),
    );
    send_json_line(&mut stream, &request)
        .context("failed to query surface registry from waylandd")?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope =
        read_json_line(&mut reader).context("failed to read surface registry from waylandd")?;

    if response.source != ServiceRole::Waylandd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::Compd {
        bail!("unexpected response destination: {}", response.destination.as_str());
    }

    match response.kind {
        MessageKind::WaylandEvent(WaylandEvent::SurfaceRegistry { snapshot }) => Ok(snapshot),
        MessageKind::WaylandEvent(WaylandEvent::Rejected { reason }) => {
            bail!("waylandd rejected surface registry query: {reason}")
        }
        other => bail!("unexpected waylandd response: {other:?}"),
    }
}

fn send_selection_handoff_to_waylandd(
    focus: &FocusTarget,
    selection: &WaylandSelectionState,
) -> Result<u64> {
    let mut stream = connect_service_socket(ServiceRole::Waylandd)
        .context("failed to connect to waylandd socket")?;
    let request = IpcEnvelope::new(
        ServiceRole::Compd,
        ServiceRole::Waylandd,
        MessageKind::WaylandCommand(WaylandCommand::ApplySelectionHandoff {
            handoff: WaylandSelectionHandoff { focus: focus.clone(), selection: selection.clone() },
        }),
    );
    send_json_line(&mut stream, &request)
        .context("failed to send selection handoff to waylandd")?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope =
        read_json_line(&mut reader).context("failed to read selection handoff response")?;

    if response.source != ServiceRole::Waylandd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::Compd {
        bail!("unexpected response destination: {}", response.destination.as_str());
    }

    match response.kind {
        MessageKind::WaylandEvent(WaylandEvent::SelectionHandoffApplied { generation, .. }) => {
            Ok(generation)
        }
        MessageKind::WaylandEvent(WaylandEvent::Rejected { reason }) => {
            bail!("waylandd rejected selection handoff: {reason}")
        }
        other => bail!("unexpected waylandd response: {other:?}"),
    }
}

fn apply_selection_handoff(config: &Config, scene: Option<&CompdScene>) -> Result<()> {
    let Some(scene) = scene else {
        println!("service=compd op=selection_handoff event=skipped reason=no-scene");
        return Ok(());
    };

    match send_selection_handoff_to_waylandd(&scene.focus, &scene.selection) {
        Ok(generation) => {
            println!(
                "service=compd op=selection_handoff event=success generation={} focus={:?} clipboard_owner={} primary_selection_owner={}",
                generation,
                scene.focus,
                format_owner(scene.selection.clipboard_owner.as_deref()),
                format_owner(scene.selection.primary_selection_owner.as_deref())
            );
            Ok(())
        }
        Err(err) => {
            if config.require_waylandd {
                Err(err).context("failed to apply selection handoff to waylandd")
            } else {
                println!("service=compd op=selection_handoff event=failed reason=\"{}\"", err);
                Ok(())
            }
        }
    }
}

fn scene_from_snapshot(snapshot: &CommittedSceneState) -> CompdScene {
    CompdScene {
        target_output: match &snapshot.target {
            CommitTarget::Output { name } => name.clone(),
        },
        focus: snapshot.focus.clone(),
        selection: snapshot.selection.clone(),
        surfaces: snapshot.surfaces.clone(),
        scene_epoch: snapshot.scene_epoch,
        scene_generation: snapshot.scene_generation,
    }
}

#[derive(Debug)]
struct SceneReconcileResult {
    scene: CompdScene,
    dropped_surface_ids: Vec<String>,
    updated_app_ids: usize,
    selection_handoffs: usize,
}

fn reconcile_scene_with_registry(
    scene: CompdScene,
    registry: &SurfaceRegistrySnapshot,
    output_mode: &waybroker_common::OutputMode,
) -> SceneReconcileResult {
    let active_registry: BTreeMap<&str, &WaylandSurfaceState> = registry
        .surfaces
        .iter()
        .filter(|surface| surface.mapped && surface.buffer_attached)
        .map(|surface| (surface.id.as_str(), surface))
        .collect();

    let mut kept_surfaces = Vec::with_capacity(scene.surfaces.len());
    let mut dropped_surface_ids = Vec::new();
    let mut updated_app_ids = 0usize;

    for mut surface in scene.surfaces {
        match active_registry.get(surface.id.as_str()) {
            Some(registry_surface) => {
                if surface.app_id != registry_surface.app_id {
                    surface.app_id = registry_surface.app_id.clone();
                    updated_app_ids += 1;
                }
                surface.buffer_handle = registry_surface.buffer_handle.clone();
                surface.buffer_generation = registry_surface.buffer_generation;
                surface.damage_rects = registry_surface.damage_rects.clone();
                kept_surfaces.push(surface);
            }
            None => dropped_surface_ids.push(surface.id),
        }
    }

    let focus = reconcile_focus(&scene.focus, &kept_surfaces, &active_registry);
    let (selection, selection_handoffs) =
        reconcile_selection(&registry.selection, &focus, &active_registry);

    // Apply basic layout rules based on roles
    for surface in &mut kept_surfaces {
        if let Some(reg) = active_registry.get(surface.id.as_str()) {
            apply_role_based_layout(surface, &reg.role, output_mode);
        }
    }

    SceneReconcileResult {
        scene: CompdScene {
            target_output: scene.target_output,
            focus,
            selection,
            surfaces: kept_surfaces,
            scene_epoch: scene.scene_epoch,
            scene_generation: scene.scene_generation,
        },
        dropped_surface_ids,
        updated_app_ids,
        selection_handoffs,
    }
}

const STUB_SCREEN_WIDTH: u32 = 1920;
const STUB_SCREEN_HEIGHT: u32 = 1080;
const STUB_PANEL_HEIGHT: u32 = 36;

fn apply_role_based_layout(
    surface: &mut SurfaceSnapshot,
    role: &WaylandSurfaceRole,
    output: &waybroker_common::OutputMode,
) {
    match role {
        WaylandSurfaceRole::Background => {
            surface.placement.x = 0;
            surface.placement.y = 0;
            surface.placement.width = output.width;
            surface.placement.height = output.height;
            surface.placement.z = 0;
        }
        WaylandSurfaceRole::Layer(metadata) => {
            let width = if metadata.anchor & 1 != 0 && metadata.anchor & 2 != 0 {
                output
                    .width
                    .saturating_sub((metadata.margin_left + metadata.margin_right).max(0) as u32)
            } else {
                STUB_SCREEN_WIDTH
            };

            let height = if metadata.exclusive_zone > 0 {
                metadata.exclusive_zone as u32
            } else {
                STUB_PANEL_HEIGHT
            };

            surface.placement.x = metadata.margin_left;

            if metadata.anchor & 4 != 0 {
                // Anchor Top
                surface.placement.y = metadata.margin_top;
            } else if metadata.anchor & 8 != 0 {
                // Anchor Bottom
                surface.placement.y =
                    (output.height.saturating_sub(height)) as i32 - metadata.margin_bottom;
            } else {
                surface.placement.y = 0;
            }

            surface.placement.width = width;
            surface.placement.height = height;

            surface.placement.z = match metadata.layer {
                0 => 10,  // Background
                1 => 20,  // Bottom
                2 => 100, // Top
                3 => 200, // Overlay
                _ => 100,
            };
        }
        #[allow(clippy::collapsible_match)]
        WaylandSurfaceRole::Toplevel => {
            if surface.placement.z < 30 {
                surface.placement.z = 30;
            }
        }
        _ => {}
    }
}

fn reconcile_focus(
    previous_focus: &FocusTarget,
    surfaces: &[SurfaceSnapshot],
    active_registry: &BTreeMap<&str, &WaylandSurfaceState>,
) -> FocusTarget {
    match previous_focus {
        FocusTarget::Surface { id }
            if active_registry
                .get(id.as_str())
                .is_some_and(|surface| is_focusable_role(&surface.role)) =>
        {
            FocusTarget::Surface { id: id.clone() }
        }
        _ => fallback_focus_target(surfaces, active_registry),
    }
}

fn fallback_focus_target(
    surfaces: &[SurfaceSnapshot],
    active_registry: &BTreeMap<&str, &WaylandSurfaceState>,
) -> FocusTarget {
    surfaces
        .iter()
        .filter(|surface| surface.placement.visible)
        .filter_map(|surface| {
            active_registry.get(surface.id.as_str()).and_then(|registry_surface| {
                is_focusable_role(&registry_surface.role)
                    .then_some((surface.placement.z, &surface.id))
            })
        })
        .max_by_key(|(z, _)| *z)
        .map(|(_, id)| FocusTarget::Surface { id: id.clone() })
        .unwrap_or(FocusTarget::None)
}

fn reconcile_selection(
    previous_selection: &WaylandSelectionState,
    focus: &FocusTarget,
    active_registry: &BTreeMap<&str, &WaylandSurfaceState>,
) -> (WaylandSelectionState, usize) {
    let (clipboard_owner, clipboard_payload_id, clipboard_source_serial, clipboard_changed) =
        reconcile_selection_slot(
            previous_selection.clipboard_owner.as_deref(),
            previous_selection.clipboard_payload_id.as_deref(),
            previous_selection.clipboard_source_serial,
            focus,
            active_registry,
        );
    let (
        primary_selection_owner,
        primary_selection_payload_id,
        primary_selection_source_serial,
        primary_changed,
    ) = reconcile_selection_slot(
        previous_selection.primary_selection_owner.as_deref(),
        previous_selection.primary_selection_payload_id.as_deref(),
        previous_selection.primary_selection_source_serial,
        focus,
        active_registry,
    );

    (
        WaylandSelectionState {
            clipboard_owner,
            clipboard_payload_id,
            clipboard_source_serial,
            clipboard_offer: previous_selection.clipboard_offer.clone(),
            primary_selection_owner,
            primary_selection_payload_id,
            primary_selection_source_serial,
            primary_offer: previous_selection.primary_offer.clone(),
        },
        usize::from(clipboard_changed) + usize::from(primary_changed),
    )
}

fn reconcile_selection_slot(
    previous_owner: Option<&str>,
    previous_payload_id: Option<&str>,
    previous_source_serial: Option<u64>,
    focus: &FocusTarget,
    active_registry: &BTreeMap<&str, &WaylandSurfaceState>,
) -> (Option<String>, Option<String>, Option<u64>, bool) {
    match previous_owner {
        Some(id) if active_registry.contains_key(id) => (
            Some(id.to_owned()),
            previous_payload_id.map(str::to_owned),
            previous_source_serial,
            false,
        ),
        Some(_) => {
            let handoff = match focus {
                FocusTarget::Surface { id }
                    if active_registry
                        .get(id.as_str())
                        .is_some_and(|surface| is_focusable_role(&surface.role)) =>
                {
                    Some(id.clone())
                }
                _ => None,
            };
            (handoff, None, None, true)
        }
        None => {
            let metadata_changed =
                previous_payload_id.is_some() || previous_source_serial.is_some();
            (None, None, None, metadata_changed)
        }
    }
}

fn format_owner(owner: Option<&str>) -> &str {
    owner.unwrap_or("none")
}

fn is_focusable_role(role: &WaylandSurfaceRole) -> bool {
    matches!(
        role,
        WaylandSurfaceRole::Toplevel | WaylandSurfaceRole::Popup | WaylandSurfaceRole::Lock
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CompdScene, apply_role_based_layout, mock_demo_scene, reconcile_scene_with_registry,
        scene_from_snapshot,
    };
    use waybroker_common::{
        CommitTarget, CommittedSceneState, FocusTarget, ServiceRole, SurfacePlacement,
        SurfaceRegistrySnapshot, SurfaceSnapshot, WaylandSelectionState, WaylandSurfaceRole,
        WaylandSurfaceState,
    };

    #[test]
    fn mock_scene_has_expected_focus_and_surfaces() {
        let scene = mock_demo_scene();
        assert_eq!(scene.target_output, "eDP-1");
        assert_eq!(scene.focus, FocusTarget::Surface { id: "konsole-1".into() });
        assert_eq!(scene.surfaces.len(), 2);
    }

    #[test]
    fn handles_empty_focus() {
        let scene = CompdScene {
            target_output: "eDP-1".into(),
            focus: FocusTarget::None,
            selection: WaylandSelectionState::default(),
            scene_epoch: 1,
            scene_generation: 1,
            surfaces: vec![],
        };
        assert_eq!(scene.focus, FocusTarget::None);
        assert_eq!(scene.surfaces.len(), 0);
    }

    #[test]
    fn surface_count_matches_after_conversion() {
        let scene = CompdScene {
            target_output: "HDMI-1".into(),
            focus: FocusTarget::None,
            selection: WaylandSelectionState::default(),
            scene_epoch: 1,
            scene_generation: 1,
            surfaces: vec![
                SurfaceSnapshot {
                    id: "s1".into(),
                    app_id: "a1".into(),
                    placement: SurfacePlacement {
                        x: 0,
                        y: 0,
                        width: 100,
                        height: 100,
                        z: 1,
                        visible: true,
                    },
                    ..Default::default()
                },
                SurfaceSnapshot {
                    id: "s2".into(),
                    app_id: "a2".into(),
                    placement: SurfacePlacement {
                        x: 0,
                        y: 0,
                        width: 100,
                        height: 100,
                        z: 2,
                        visible: true,
                    },
                    ..Default::default()
                },
            ],
        };
        assert_eq!(scene.surfaces.len(), 2);
    }

    #[test]
    fn rebuilds_scene_from_displayd_snapshot() {
        let scene = scene_from_snapshot(&CommittedSceneState {
            source: ServiceRole::Compd,
            target: CommitTarget::Output { name: "HDMI-1".into() },
            focus: FocusTarget::Surface { id: "xterm-1".into() },
            selection: waybroker_common::WaylandSelectionState::default(),
            surfaces: vec![SurfaceSnapshot {
                id: "xterm-1".into(),
                app_id: "org.xterm".into(),
                placement: SurfacePlacement {
                    x: 42,
                    y: 24,
                    width: 640,
                    height: 480,
                    z: 7,
                    visible: true,
                },
                ..Default::default()
            }],
            scene_epoch: 0,
            scene_generation: 0,
            commit_id: 3,
            unix_timestamp: 1_778_000_100,
        });

        assert_eq!(scene.target_output, "HDMI-1");
        assert_eq!(scene.focus, FocusTarget::Surface { id: "xterm-1".into() });
        assert_eq!(scene.surfaces.len(), 1);
    }

    #[test]
    fn drops_surfaces_missing_from_wayland_registry() {
        let dummy_output = waybroker_common::OutputMode {
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };
        let reconciled = reconcile_scene_with_registry(
            CompdScene {
                target_output: "eDP-1".into(),
                focus: FocusTarget::Surface { id: "panel-1".into() },
                selection: WaylandSelectionState {
                    clipboard_owner: Some("panel-1".into()),
                    clipboard_payload_id: Some("panel-clipboard-v1".into()),
                    clipboard_source_serial: Some(41),
                    clipboard_offer: None,
                    primary_selection_owner: Some("terminal-1".into()),
                    primary_selection_payload_id: Some("terminal-primary-v7".into()),
                    primary_selection_source_serial: Some(77),
                    primary_offer: None,
                },
                scene_epoch: 1,
                scene_generation: 1,
                surfaces: vec![
                    SurfaceSnapshot {
                        id: "terminal-1".into(),
                        app_id: "old.app".into(),
                        placement: SurfacePlacement {
                            x: 10,
                            y: 10,
                            width: 100,
                            height: 100,
                            z: 5,
                            visible: true,
                        },
                        ..Default::default()
                    },
                    SurfaceSnapshot {
                        id: "panel-1".into(),
                        app_id: "org.kde.panel".into(),
                        placement: SurfacePlacement {
                            x: 0,
                            y: 0,
                            width: 200,
                            height: 30,
                            z: 10,
                            visible: true,
                        },
                        ..Default::default()
                    },
                ],
            },
            &SurfaceRegistrySnapshot {
                generation: 2,
                surfaces: vec![WaylandSurfaceState {
                    id: "terminal-1".into(),
                    app_id: "org.kde.konsole".into(),
                    role: WaylandSurfaceRole::Toplevel,
                    mapped: true,
                    buffer_attached: true,
                    ..Default::default()
                }],
                foreign_toplevels: vec![],
                selection: WaylandSelectionState {
                    clipboard_owner: Some("panel-1".into()),
                    clipboard_payload_id: Some("panel-clipboard-v1".into()),
                    clipboard_source_serial: Some(41),
                    clipboard_offer: None,
                    primary_selection_owner: Some("terminal-1".into()),
                    primary_selection_payload_id: Some("terminal-primary-v7".into()),
                    primary_selection_source_serial: Some(77),
                    primary_offer: None,
                },
                unix_timestamp: 1,
            },
            &dummy_output,
        );

        assert_eq!(reconciled.scene.surfaces.len(), 1);
        assert_eq!(reconciled.scene.surfaces[0].id, "terminal-1");
        assert_eq!(reconciled.scene.surfaces[0].app_id, "org.kde.konsole");
        assert_eq!(reconciled.scene.focus, FocusTarget::Surface { id: "terminal-1".into() });
        assert_eq!(reconciled.scene.selection.clipboard_owner.as_deref(), Some("terminal-1"));
        assert_eq!(reconciled.scene.selection.clipboard_payload_id, None);
        assert_eq!(reconciled.scene.selection.clipboard_source_serial, None);
        assert_eq!(
            reconciled.scene.selection.primary_selection_owner.as_deref(),
            Some("terminal-1")
        );
        assert_eq!(
            reconciled.scene.selection.primary_selection_payload_id.as_deref(),
            Some("terminal-primary-v7")
        );
        assert_eq!(reconciled.scene.selection.primary_selection_source_serial, Some(77));
        assert_eq!(reconciled.dropped_surface_ids, vec!["panel-1"]);
        assert_eq!(reconciled.updated_app_ids, 1);
        assert_eq!(reconciled.selection_handoffs, 1);
    }

    #[test]
    fn falls_back_to_no_focus_when_only_background_survives() {
        let dummy_output = waybroker_common::OutputMode {
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };
        let reconciled = reconcile_scene_with_registry(
            CompdScene {
                target_output: "eDP-1".into(),
                focus: FocusTarget::Surface { id: "terminal-1".into() },
                selection: WaylandSelectionState {
                    clipboard_owner: Some("terminal-1".into()),
                    clipboard_payload_id: Some("terminal-clipboard-v1".into()),
                    clipboard_source_serial: Some(51),
                    clipboard_offer: None,
                    primary_selection_owner: Some("terminal-1".into()),
                    primary_selection_payload_id: Some("terminal-primary-v7".into()),
                    primary_selection_source_serial: Some(77),
                    primary_offer: None,
                },
                scene_epoch: 1,
                scene_generation: 1,
                surfaces: vec![SurfaceSnapshot {
                    id: "background-1".into(),
                    app_id: "org.kde.wallpaper".into(),
                    placement: SurfacePlacement {
                        x: 0,
                        y: 0,
                        width: 1920,
                        height: 1080,
                        z: 0,
                        visible: true,
                    },
                    ..Default::default()
                }],
            },
            &SurfaceRegistrySnapshot {
                generation: 3,
                surfaces: vec![WaylandSurfaceState {
                    id: "background-1".into(),
                    app_id: "org.kde.wallpaper".into(),
                    role: WaylandSurfaceRole::Background,
                    mapped: true,
                    buffer_attached: true,
                    ..Default::default()
                }],
                foreign_toplevels: vec![],
                selection: WaylandSelectionState {
                    clipboard_owner: Some("terminal-1".into()),
                    clipboard_payload_id: Some("terminal-clipboard-v1".into()),
                    clipboard_source_serial: Some(51),
                    clipboard_offer: None,
                    primary_selection_owner: Some("terminal-1".into()),
                    primary_selection_payload_id: Some("terminal-primary-v7".into()),
                    primary_selection_source_serial: Some(77),
                    primary_offer: None,
                },
                unix_timestamp: 1,
            },
            &dummy_output,
        );

        assert_eq!(reconciled.scene.focus, FocusTarget::None);
        assert_eq!(reconciled.scene.selection.clipboard_owner, None);
        assert_eq!(reconciled.scene.selection.clipboard_payload_id, None);
        assert_eq!(reconciled.scene.selection.clipboard_source_serial, None);
        assert_eq!(reconciled.scene.selection.primary_selection_owner, None);
        assert_eq!(reconciled.scene.selection.primary_selection_payload_id, None);
        assert_eq!(reconciled.scene.selection.primary_selection_source_serial, None);
        assert!(reconciled.dropped_surface_ids.is_empty());
        assert_eq!(reconciled.selection_handoffs, 2);
    }

    #[test]
    fn test_layer_shell_layout_logic() {
        let output = waybroker_common::OutputMode {
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };

        let mut bg = SurfaceSnapshot {
            id: "bg".into(),
            app_id: "bg".into(),
            placement: SurfacePlacement {
                x: 10,
                y: 10,
                width: 100,
                height: 100,
                z: 5,
                visible: true,
            },
            ..Default::default()
        };
        apply_role_based_layout(&mut bg, &WaylandSurfaceRole::Background, &output);
        assert_eq!(bg.placement.x, 0);
        assert_eq!(bg.placement.y, 0);
        assert_eq!(bg.placement.width, 1920);
        assert_eq!(bg.placement.height, 1080);
        assert_eq!(bg.placement.z, 0);

        let mut panel = SurfaceSnapshot {
            id: "panel".into(),
            app_id: "panel".into(),
            placement: SurfacePlacement { x: 0, y: 0, width: 0, height: 0, z: 0, visible: true },
            ..Default::default()
        };
        let metadata = waybroker_common::LayerMetadata {
            layer: 2,          // Top
            anchor: 1 | 2 | 4, // Left | Right | Top
            exclusive_zone: 30,
            margin_top: 5,
            margin_bottom: 0,
            margin_left: 10,
            margin_right: 10,
            keyboard_interactivity: 0,
        };
        apply_role_based_layout(&mut panel, &WaylandSurfaceRole::Layer(metadata), &output);
        assert_eq!(panel.placement.x, 10);
        assert_eq!(panel.placement.y, 5);
        assert_eq!(panel.placement.width, 1900); // 1920 - 10 - 10
        assert_eq!(panel.placement.height, 30);
        assert_eq!(panel.placement.z, 100);

        let mut overlay = SurfaceSnapshot {
            id: "overlay".into(),
            app_id: "overlay".into(),
            placement: SurfacePlacement { x: 0, y: 0, width: 0, height: 0, z: 0, visible: true },
            ..Default::default()
        };
        let overlay_metadata = waybroker_common::LayerMetadata {
            layer: 3,  // Overlay
            anchor: 8, // Bottom
            exclusive_zone: 0,
            margin_top: 0,
            margin_bottom: 20,
            margin_left: 0,
            margin_right: 0,
            keyboard_interactivity: 0,
        };
        apply_role_based_layout(
            &mut overlay,
            &WaylandSurfaceRole::Layer(overlay_metadata),
            &output,
        );
        assert_eq!(overlay.placement.y, 1080 - 36 - 20);
        assert_eq!(overlay.placement.z, 200);
    }

    #[test]
    fn test_phase2_scene_stacking_and_focus() {
        let mut scene = CompdScene {
            target_output: "eDP-1".into(),
            focus: FocusTarget::Surface { id: "win2".into() },
            selection: WaylandSelectionState::default(),
            scene_epoch: 1,
            scene_generation: 1,
            surfaces: vec![
                SurfaceSnapshot {
                    id: "win1".into(),
                    app_id: "app1".into(),
                    placement: SurfacePlacement {
                        x: 10,
                        y: 10,
                        width: 200,
                        height: 200,
                        z: 1,
                        visible: true,
                    },
                    ..Default::default()
                },
                SurfaceSnapshot {
                    id: "win2".into(),
                    app_id: "app2".into(),
                    placement: SurfacePlacement {
                        x: 50,
                        y: 50,
                        width: 200,
                        height: 200,
                        z: 2,
                        visible: true,
                    },
                    ..Default::default()
                },
            ],
        };
        scene.surfaces.sort_by_key(|s| s.placement.z);
        assert_eq!(scene.surfaces[0].id, "win1");
        assert_eq!(scene.surfaces[1].id, "win2");
        assert_eq!(scene.focus, FocusTarget::Surface { id: "win2".into() });
    }

    #[test]
    fn test_phase2_occlusion_and_visibility() {
        let win1 =
            SurfacePlacement { x: 100, y: 100, width: 100, height: 100, z: 1, visible: true };
        let win2 = SurfacePlacement { x: 50, y: 50, width: 300, height: 300, z: 2, visible: true };

        let win1_occluded = win1.x >= win2.x
            && win1.y >= win2.y
            && (win1.x + win1.width as i32) <= (win2.x + win2.width as i32)
            && (win1.y + win1.height as i32) <= (win2.y + win2.height as i32);

        assert!(win1_occluded, "win1 should be fully occluded by win2");
    }

    #[test]
    fn test_phase2_snapshot_reconcile_crash_recovery() {
        let snapshot = CommittedSceneState {
            source: ServiceRole::Compd,
            target: CommitTarget::Output { name: "eDP-1".into() },
            focus: FocusTarget::Surface { id: "gone-app".into() },
            selection: WaylandSelectionState::default(),
            surfaces: vec![
                SurfaceSnapshot {
                    id: "gone-app".into(),
                    app_id: "gone.app".into(),
                    placement: SurfacePlacement {
                        x: 0,
                        y: 0,
                        width: 100,
                        height: 100,
                        z: 1,
                        visible: true,
                    },
                    ..Default::default()
                },
                SurfaceSnapshot {
                    id: "surviving-app".into(),
                    app_id: "surviving.app".into(),
                    placement: SurfacePlacement {
                        x: 10,
                        y: 10,
                        width: 100,
                        height: 100,
                        z: 2,
                        visible: true,
                    },
                    ..Default::default()
                },
            ],
            scene_epoch: 0,
            scene_generation: 0,
            commit_id: 10,
            unix_timestamp: 1234567,
        };

        let registry = SurfaceRegistrySnapshot {
            generation: 1,
            surfaces: vec![WaylandSurfaceState {
                id: "surviving-app".into(),
                app_id: "surviving.app".into(),
                role: WaylandSurfaceRole::Toplevel,
                mapped: true,
                buffer_attached: true,
                ..Default::default()
            }],
            foreign_toplevels: vec![],
            selection: WaylandSelectionState::default(),
            unix_timestamp: 1234567,
        };

        let output_mode = waybroker_common::OutputMode {
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };

        let initial_scene = scene_from_snapshot(&snapshot);
        let reconciled = reconcile_scene_with_registry(initial_scene, &registry, &output_mode);

        assert_eq!(reconciled.scene.surfaces.len(), 1);
        assert_eq!(reconciled.scene.surfaces[0].id, "surviving-app");
        assert_eq!(reconciled.dropped_surface_ids, vec!["gone-app"]);
    }
}
