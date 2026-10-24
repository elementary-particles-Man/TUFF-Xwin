use std::{
    env,
    fs::File,
    io::{BufReader, Read},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use layerx11_common::{X11RootlessScene, sample_rootless_scene};
use waybroker_common::{
    CommitTarget, DisplayCommand, DisplayEvent, IpcEnvelope, MessageKind, ServiceBanner,
    ServiceRole, connect_service_socket, read_ipc_envelope, send_ipc_envelope,
};

const MAX_SCENE_INPUT_BYTES: u64 = 8 * 1024 * 1024;
const IPC_TIMEOUT: Duration = Duration::from_secs(30);

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
    let scene = match path {
        Some(path) => {
            let file = File::open(path)
                .with_context(|| format!("failed to read scene file {}", path.display()))?;
            let mut limited = file.take(MAX_SCENE_INPUT_BYTES + 1);
            let mut raw = Vec::new();
            limited.read_to_end(&mut raw)?;
            if raw.len() as u64 > MAX_SCENE_INPUT_BYTES {
                bail!("scene file exceeds {} bytes", MAX_SCENE_INPUT_BYTES);
            }
            serde_json::from_slice(&raw)
                .with_context(|| format!("failed to decode scene file {}", path.display()))
        }
        None => Ok(sample_rootless_scene()),
    }?;
    scene.validate().map_err(|reason| anyhow::anyhow!("invalid rootless scene: {reason}"))?;
    Ok(scene)
}

fn commit_scene_to_displayd(scene: &X11RootlessScene) -> Result<usize> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    stream.set_read_timeout(Some(IPC_TIMEOUT))?;
    stream.set_write_timeout(Some(IPC_TIMEOUT))?;
    let request = IpcEnvelope::new(
        ServiceRole::X11Bridge,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::CommitScene {
            target: CommitTarget::Output { name: scene.output.name.clone() },
            focus: scene.focus_target(),
            selection: scene.selection_state(),
            surfaces: scene.to_surface_snapshots(),
            pixel_payloads: vec![],
            scene_epoch: 0,
            scene_generation: 0,
        }),
    );
    send_ipc_envelope(&mut stream, &request)?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_ipc_envelope(&mut reader)?;

    if response.source != ServiceRole::Displayd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::X11Bridge {
        bail!("unexpected response destination: {}", response.destination.as_str());
    }

    match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::SceneCommitted {
            surface_count,
            commit_id,
            ..
        }) => {
            if surface_count != scene.mapped_window_count() {
                bail!(
                    "displayd committed {surface_count} surfaces, expected {}",
                    scene.mapped_window_count()
                );
            }
            if commit_id == 0 {
                bail!("displayd returned invalid commit id 0");
            }
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

#[cfg(test)]
mod tests {
    use super::{MAX_SCENE_INPUT_BYTES, load_scene};
    use layerx11_common::sample_rootless_scene;
    use std::{fs, path::PathBuf};

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("x11bridge-{name}-{}", std::process::id()))
    }

    #[test]
    fn rejects_invalid_json_and_oversized_scene_before_commit() {
        let invalid = temp_path("invalid.json");
        fs::write(&invalid, b"not-json").unwrap();
        assert!(load_scene(Some(&invalid)).is_err());
        fs::remove_file(&invalid).unwrap();

        let oversized = temp_path("oversized.json");
        let mut bytes = vec![b' '; MAX_SCENE_INPUT_BYTES as usize + 1];
        bytes[0] = b'{';
        fs::write(&oversized, bytes).unwrap();
        assert!(load_scene(Some(&oversized)).is_err());
        fs::remove_file(&oversized).unwrap();
    }

    #[test]
    fn sample_scene_is_validated_before_use() {
        let scene = sample_rootless_scene();
        assert!(scene.validate().is_ok());
        assert_eq!(scene.mapped_window_count(), 3);
    }
}
