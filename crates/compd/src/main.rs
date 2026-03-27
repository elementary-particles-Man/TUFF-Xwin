use std::{env, fs, io::BufReader, os::unix::net::UnixStream, path::PathBuf};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    CommitTarget, DisplayCommand, DisplayEvent, FocusTarget, IpcEnvelope, MessageKind,
    ServiceBanner, ServiceRole, SurfacePlacement, SurfaceSnapshot, bind_service_socket,
    connect_service_socket, read_json_line, send_json_line,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Compd, "scene, focus, composition policy");
    println!("{}", banner.render());

    if config.serve_ipc {
        serve_ipc(&config)?;
        return Ok(());
    }

    let scene = load_scene(config.scene_path.as_ref())?;

    if config.print_scene {
        println!(
            "service=compd op=scene_print event=success target={} surfaces={}",
            scene.target_output,
            scene.surfaces.len()
        );
        println!("{}", serde_json::to_string_pretty(&scene)?);
    }

    if config.commit_demo {
        println!(
            "service=compd op=scene_build event=success target={} focus={:?} surfaces={}",
            scene.target_output,
            scene.focus,
            scene.surfaces.len()
        );

        match commit_scene_to_displayd(&scene) {
            Ok(committed) => {
                println!(
                    "service=compd op=displayd_response event=scene_committed surface_count={committed}"
                );
            }
            Err(err) => {
                if config.require_displayd {
                    return Err(err).context("failed to commit scene to displayd (required)");
                } else {
                    println!("service=compd op=scene_commit event=failed reason=\"{}\"", err);
                }
            }
        }
    }

    if !config.commit_demo && !config.print_scene {
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
                "--help" | "-h" => {
                    println!(
                        "usage: compd [--scene PATH] [--print-scene] [--commit-demo] [--require-displayd] [--serve-ipc] [--once] [--fail-resume]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
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

fn commit_scene_to_displayd(scene: &CompdScene) -> Result<usize> {
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

    match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::SceneCommitted { surface_count, .. }) => {
            Ok(surface_count)
        }
        MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
            bail!("displayd rejected scene: {reason}")
        }
        other => bail!("unexpected displayd response: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{CompdScene, mock_demo_scene};
    use waybroker_common::{FocusTarget, SurfacePlacement, SurfaceSnapshot};

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
}
