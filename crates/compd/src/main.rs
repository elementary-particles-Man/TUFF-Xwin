use std::{
    collections::BTreeMap, env, fs, io::BufReader, os::unix::net::UnixStream, path::PathBuf,
};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    CommitTarget, CommittedSceneState, DisplayCommand, DisplayEvent, FocusTarget, IpcEnvelope,
    MessageKind, ServiceBanner, ServiceRole, SurfacePlacement, SurfaceRegistrySnapshot,
    SurfaceSnapshot, WaylandCommand, WaylandEvent, WaylandSurfaceRole, WaylandSurfaceState,
    bind_service_socket, connect_service_socket, read_json_line, send_json_line,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Compd, "scene, focus, composition policy");
    println!("{}", banner.render());

    if config.restore_from_displayd && config.scene_path.is_some() {
        bail!("--scene cannot be combined with --restore-from-displayd");
    }

    let scene = prepare_scene(&config)?;

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
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

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
                "--help" | "-h" => {
                    println!(
                        "usage: compd [--scene PATH] [--print-scene] [--commit-demo] [--require-displayd] [--require-waylandd] [--restore-from-displayd] [--reconcile-waylandd] [--serve-ipc] [--once] [--fail-resume]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

fn prepare_scene(config: &Config) -> Result<Option<CompdScene>> {
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
        scene = reconcile_scene(config, scene)?;
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

fn reconcile_scene(config: &Config, scene: Option<CompdScene>) -> Result<Option<CompdScene>> {
    let Some(scene) = scene else {
        println!("service=compd op=scene_reconcile event=skipped reason=no-scene");
        return Ok(None);
    };

    match query_surface_registry_from_waylandd() {
        Ok(snapshot) => {
            println!(
                "service=compd op=surface_registry event=success generation={} surfaces={} timestamp={}",
                snapshot.generation,
                snapshot.surfaces.len(),
                snapshot.unix_timestamp
            );
            let reconciled = reconcile_scene_with_registry(scene, &snapshot);
            println!(
                "service=compd op=scene_reconcile event=success kept={} dropped={} app_id_updates={} focus={:?}",
                reconciled.scene.surfaces.len(),
                reconciled.dropped_surface_ids.len(),
                reconciled.updated_app_ids,
                reconciled.scene.focus
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
    let (listener, socket_path) = bind_service_socket(ServiceRole::Compd)?;
    let _socket_guard = SocketGuard::new(socket_path.clone());
    println!("service=compd op=listen event=socket_bound path={}", socket_path.display());

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = stream?;
        handle_client(stream, config)?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("service=compd op=terminate event=finished served_requests={served}");
    Ok(())
}

fn handle_client(mut stream: UnixStream, config: &Config) -> Result<()> {
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CompdScene {
    target_output: String,
    focus: FocusTarget,
    surfaces: Vec<SurfaceSnapshot>,
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
            },
        ],
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
            surfaces: scene.surfaces.clone(),
        }),
    );
    send_json_line(&mut stream, &request).context("failed to send commit-scene to displayd")?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope =
        read_json_line(&mut reader).context("failed to read response from displayd")?;

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
        }) => Ok(SceneCommitReceipt { surface_count, commit_id }),
        MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
            bail!("displayd rejected scene: {reason}")
        }
        other => bail!("unexpected displayd response: {other:?}"),
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

fn scene_from_snapshot(snapshot: &CommittedSceneState) -> CompdScene {
    CompdScene {
        target_output: match &snapshot.target {
            CommitTarget::Output { name } => name.clone(),
        },
        focus: snapshot.focus.clone(),
        surfaces: snapshot.surfaces.clone(),
    }
}

#[derive(Debug)]
struct SceneReconcileResult {
    scene: CompdScene,
    dropped_surface_ids: Vec<String>,
    updated_app_ids: usize,
}

fn reconcile_scene_with_registry(
    scene: CompdScene,
    registry: &SurfaceRegistrySnapshot,
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
                kept_surfaces.push(surface);
            }
            None => dropped_surface_ids.push(surface.id),
        }
    }

    let focus = reconcile_focus(&scene.focus, &kept_surfaces, &active_registry);
    SceneReconcileResult {
        scene: CompdScene { target_output: scene.target_output, focus, surfaces: kept_surfaces },
        dropped_surface_ids,
        updated_app_ids,
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
                .is_some_and(|surface| is_focusable_role(surface.role)) =>
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
                is_focusable_role(registry_surface.role)
                    .then_some((surface.placement.z, &surface.id))
            })
        })
        .max_by_key(|(z, _)| *z)
        .map(|(_, id)| FocusTarget::Surface { id: id.clone() })
        .unwrap_or(FocusTarget::None)
}

fn is_focusable_role(role: WaylandSurfaceRole) -> bool {
    matches!(
        role,
        WaylandSurfaceRole::Toplevel | WaylandSurfaceRole::Popup | WaylandSurfaceRole::Lock
    )
}

#[cfg(test)]
mod tests {
    use super::{CompdScene, mock_demo_scene, reconcile_scene_with_registry, scene_from_snapshot};
    use waybroker_common::{
        CommitTarget, CommittedSceneState, FocusTarget, ServiceRole, SurfacePlacement,
        SurfaceRegistrySnapshot, SurfaceSnapshot, WaylandSurfaceRole, WaylandSurfaceState,
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
                },
            ],
        };
        assert_eq!(scene.surfaces.len(), 2);
    }

    #[test]
    fn rebuilds_scene_from_displayd_snapshot() {
        let scene = scene_from_snapshot(&CommittedSceneState {
            source: ServiceRole::X11Bridge,
            target: CommitTarget::Output { name: "HDMI-1".into() },
            focus: FocusTarget::Surface { id: "xterm-1".into() },
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
            }],
            commit_id: 3,
            unix_timestamp: 1_778_000_100,
        });

        assert_eq!(scene.target_output, "HDMI-1");
        assert_eq!(scene.focus, FocusTarget::Surface { id: "xterm-1".into() });
        assert_eq!(scene.surfaces.len(), 1);
    }

    #[test]
    fn drops_surfaces_missing_from_wayland_registry() {
        let reconciled = reconcile_scene_with_registry(
            CompdScene {
                target_output: "eDP-1".into(),
                focus: FocusTarget::Surface { id: "panel-1".into() },
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
                }],
                unix_timestamp: 1,
            },
        );

        assert_eq!(reconciled.scene.surfaces.len(), 1);
        assert_eq!(reconciled.scene.surfaces[0].id, "terminal-1");
        assert_eq!(reconciled.scene.surfaces[0].app_id, "org.kde.konsole");
        assert_eq!(reconciled.scene.focus, FocusTarget::Surface { id: "terminal-1".into() });
        assert_eq!(reconciled.dropped_surface_ids, vec!["panel-1"]);
        assert_eq!(reconciled.updated_app_ids, 1);
    }

    #[test]
    fn falls_back_to_no_focus_when_only_background_survives() {
        let reconciled = reconcile_scene_with_registry(
            CompdScene {
                target_output: "eDP-1".into(),
                focus: FocusTarget::Surface { id: "terminal-1".into() },
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
                }],
                unix_timestamp: 1,
            },
        );

        assert_eq!(reconciled.scene.focus, FocusTarget::None);
        assert!(reconciled.dropped_surface_ids.is_empty());
    }
}
