use std::{env, fs, io::BufReader, path::PathBuf};

use anyhow::{Context, Result, bail};
use layerx11_common::{X11RootlessScene, sample_rootless_scene};
use waybroker_common::{
    CommitTarget, DisplayCommand, DisplayEvent, IpcEnvelope, MessageKind, ServiceBanner,
    ServiceRole, connect_service_socket, read_json_line, send_json_line,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let scene = load_scene(config.scene_path.as_ref())?;

    let banner = ServiceBanner::new(
        ServiceRole::X11Bridge,
        "rootless x11 compatibility island for legacy apps",
    );
    println!("{}", banner.render());
    println!(
        "x11bridge output={} windows={} mapped={} focus={}",
        scene.output.name,
        scene.windows.len(),
        scene.mapped_window_count(),
        focus_label(&scene),
    );

    if config.print_scene {
        println!("{}", serde_json::to_string_pretty(&scene)?);
    }

    if config.commit_demo {
        let committed = commit_scene_to_displayd(&scene)?;
        println!("x11bridge committed_surfaces={committed}");
    }

    Ok(())
}

#[derive(Debug, Default)]
struct Config {
    scene_path: Option<PathBuf>,
    commit_demo: bool,
    print_scene: bool,
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
                "--help" | "-h" => {
                    println!("usage: x11bridge [--scene PATH] [--print-scene] [--commit-demo]");
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

fn load_scene(path: Option<&PathBuf>) -> Result<X11RootlessScene> {
    match path {
        Some(path) => {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read scene file {}", path.display()))?;
            serde_json::from_str(&raw)
                .with_context(|| format!("failed to decode scene file {}", path.display()))
        }
        None => Ok(sample_rootless_scene()),
    }
}

fn commit_scene_to_displayd(scene: &X11RootlessScene) -> Result<usize> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let request = IpcEnvelope::new(
        ServiceRole::X11Bridge,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::CommitScene {
            target: CommitTarget::Output { name: scene.output.name.clone() },
            focus: scene.focus_target(),
            surfaces: scene.to_surface_snapshots(),
        }),
    );
    send_json_line(&mut stream, &request)?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;

    if response.source != ServiceRole::Displayd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::X11Bridge {
        bail!("unexpected response destination: {}", response.destination.as_str());
    }

    match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::SceneCommitted { surface_count, .. }) => {
            Ok(surface_count)
        }
        MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
            bail!("displayd rejected x11 scene: {reason}")
        }
        other => bail!("unexpected displayd response: {other:?}"),
    }
}

fn focus_label(scene: &X11RootlessScene) -> &str {
    scene.focus_window.as_deref().unwrap_or("none")
}
