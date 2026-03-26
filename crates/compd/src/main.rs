use std::{env, io::BufReader};

use anyhow::{Result, bail};
use waybroker_common::{
    CommitTarget, DisplayCommand, DisplayEvent, FocusTarget, IpcEnvelope, MessageKind,
    ServiceBanner, ServiceRole, SurfacePlacement, SurfaceSnapshot, connect_service_socket,
    read_json_line, send_json_line,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Compd, "scene, focus, composition policy");
    println!("{}", banner.render());

    if config.commit_demo {
        let scene = mock_demo_scene();
        println!(
            "compd demo_scene focus={:?} surfaces={}",
            scene.focus,
            scene.surfaces.len()
        );

        let committed = commit_scene_to_displayd(&scene)?;
        println!("compd committed_surfaces={committed}");
    } else {
        println!("compd state=idle (use --commit-demo to trigger mock scene)");
    }

    Ok(())
}

#[derive(Debug, Default)]
struct Config {
    commit_demo: bool,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--commit-demo" => config.commit_demo = true,
                "--help" | "-h" => {
                    println!("usage: compd [--commit-demo]");
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

struct Scene {
    target_output: String,
    focus: FocusTarget,
    surfaces: Vec<SurfaceSnapshot>,
}

fn mock_demo_scene() -> Scene {
    Scene {
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

fn commit_scene_to_displayd(scene: &Scene) -> Result<usize> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let request = IpcEnvelope::new(
        ServiceRole::Compd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::CommitScene {
            target: CommitTarget::Output { name: scene.target_output.clone() },
            focus: scene.focus.clone(),
            surfaces: scene.surfaces.clone(),
        }),
    );
    send_json_line(&mut stream, &request)?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;

    if response.source != ServiceRole::Displayd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::Compd {
        bail!("unexpected response destination: {}", response.destination.as_str());
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
    use super::mock_demo_scene;
    use waybroker_common::FocusTarget;

    #[test]
    fn mock_scene_has_expected_focus_and_surfaces() {
        let scene = mock_demo_scene();
        assert_eq!(scene.target_output, "eDP-1");
        assert_eq!(scene.focus, FocusTarget::Surface { id: "konsole-1".into() });
        assert_eq!(scene.surfaces.len(), 2);
        assert_eq!(scene.surfaces[0].id, "konsole-1");
        assert_eq!(scene.surfaces[1].id, "background-1");
    }
}
