use std::{env, fs, io::BufReader, os::unix::net::UnixStream, path::PathBuf};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    DisplayCommand, DisplayEvent, IpcEnvelope, MessageKind, OutputMode, ServiceBanner, ServiceRole,
    SurfaceRegistrySnapshot, WaylandCommand, WaylandEvent, WaylandSurfaceRole, WaylandSurfaceState,
    bind_service_socket, connect_service_socket, now_unix_timestamp, read_json_line,
    send_json_line,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(
        ServiceRole::Waylandd,
        "wayland endpoint, client lifecycle, clipboard core",
    );
    println!("{}", banner.render());

    if config.serve_ipc {
        let registry = load_surface_registry(config.registry_path.as_ref())?;
        log_surface_registry(&registry);

        if config.print_registry {
            println!("{}", serde_json::to_string_pretty(&registry)?);
        }

        match query_output_inventory() {
            Ok(outputs) => println!("waylandd displayd_outputs={}", format_outputs(&outputs)),
            Err(err) if config.require_displayd => {
                return Err(err).context("failed to query output inventory before serving IPC");
            }
            Err(err) => println!("waylandd displayd_state=unreachable reason={err}"),
        }

        serve_ipc(&config, &registry)?;
        return Ok(());
    }

    match query_output_inventory() {
        Ok(outputs) => println!("waylandd displayd_outputs={}", format_outputs(&outputs)),
        Err(err) if config.require_displayd => return Err(err),
        Err(err) => println!("waylandd displayd_state=unreachable reason={err}"),
    }

    if config.print_registry {
        let registry = load_surface_registry(config.registry_path.as_ref())?;
        log_surface_registry(&registry);
        println!("{}", serde_json::to_string_pretty(&registry)?);
    }

    Ok(())
}

#[derive(Debug, Default)]
struct Config {
    require_displayd: bool,
    serve_ipc: bool,
    serve_once: bool,
    print_registry: bool,
    registry_path: Option<PathBuf>,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--require-displayd" => config.require_displayd = true,
                "--serve-ipc" => config.serve_ipc = true,
                "--once" => config.serve_once = true,
                "--print-registry" => config.print_registry = true,
                "--registry" => {
                    let path = args.next().context("--registry requires a path")?;
                    config.registry_path = Some(PathBuf::from(path));
                }
                "--help" | "-h" => {
                    println!(
                        "usage: waylandd [--require-displayd] [--serve-ipc] [--once] [--print-registry] [--registry PATH]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

fn serve_ipc(config: &Config, registry: &SurfaceRegistrySnapshot) -> Result<()> {
    let (listener, socket_path) = bind_service_socket(ServiceRole::Waylandd)?;
    let _socket_guard = SocketGuard::new(socket_path.clone());
    println!("service=waylandd op=listen event=socket_bound path={}", socket_path.display());

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = stream?;
        handle_client(stream, registry)?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("service=waylandd op=terminate event=finished served_requests={served}");
    Ok(())
}

fn handle_client(mut stream: UnixStream, registry: &SurfaceRegistrySnapshot) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let response = build_response(request, registry);
    send_json_line(&mut stream, &response)?;
    Ok(())
}

fn build_response(request: IpcEnvelope, registry: &SurfaceRegistrySnapshot) -> IpcEnvelope {
    let source = request.source;
    let response_kind = match request.kind {
        MessageKind::WaylandCommand(command) if request.destination == ServiceRole::Waylandd => {
            MessageKind::WaylandEvent(handle_wayland_command(command, registry))
        }
        MessageKind::WaylandCommand(_) => MessageKind::WaylandEvent(WaylandEvent::Rejected {
            reason: format!(
                "waylandd received message addressed to {}",
                request.destination.as_str()
            ),
        }),
        other => MessageKind::WaylandEvent(WaylandEvent::Rejected {
            reason: format!("waylandd does not handle {other:?}"),
        }),
    };

    IpcEnvelope::new(ServiceRole::Waylandd, source, response_kind)
}

fn handle_wayland_command(
    command: WaylandCommand,
    registry: &SurfaceRegistrySnapshot,
) -> WaylandEvent {
    match command {
        WaylandCommand::GetSurfaceRegistry => {
            println!(
                "service=waylandd op=get_surface_registry event=success generation={} surfaces={}",
                registry.generation,
                registry.surfaces.len()
            );
            WaylandEvent::SurfaceRegistry { snapshot: registry.clone() }
        }
    }
}

fn query_output_inventory() -> Result<Vec<OutputMode>> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)?;
    let request = IpcEnvelope::new(
        ServiceRole::Waylandd,
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(DisplayCommand::EnumerateOutputs),
    );
    send_json_line(&mut stream, &request)?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;

    if response.source != ServiceRole::Displayd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::Waylandd {
        bail!("unexpected response destination: {}", response.destination.as_str());
    }

    match response.kind {
        MessageKind::DisplayEvent(DisplayEvent::OutputInventory { outputs }) => Ok(outputs),
        MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
            bail!("displayd rejected request: {reason}")
        }
        other => bail!("unexpected displayd response: {other:?}"),
    }
}

fn load_surface_registry(path: Option<&PathBuf>) -> Result<SurfaceRegistrySnapshot> {
    match path {
        Some(path) => {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read surface registry {}", path.display()))?;
            serde_json::from_str(&raw)
                .with_context(|| format!("failed to decode surface registry {}", path.display()))
        }
        None => Ok(mock_surface_registry()),
    }
}

fn mock_surface_registry() -> SurfaceRegistrySnapshot {
    SurfaceRegistrySnapshot {
        generation: 1,
        surfaces: vec![
            WaylandSurfaceState {
                id: "konsole-1".into(),
                app_id: "org.kde.konsole".into(),
                role: WaylandSurfaceRole::Toplevel,
                mapped: true,
                buffer_attached: true,
            },
            WaylandSurfaceState {
                id: "background-1".into(),
                app_id: "org.kde.plasmashell.wallpaper".into(),
                role: WaylandSurfaceRole::Background,
                mapped: true,
                buffer_attached: true,
            },
        ],
        unix_timestamp: now_unix_timestamp(),
    }
}

fn log_surface_registry(registry: &SurfaceRegistrySnapshot) {
    println!(
        "service=waylandd op=surface_registry event=loaded generation={} surfaces={} timestamp={}",
        registry.generation,
        registry.surfaces.len(),
        registry.unix_timestamp
    );
}

fn format_outputs(outputs: &[OutputMode]) -> String {
    let mut rendered = Vec::with_capacity(outputs.len());
    for output in outputs {
        rendered.push(format!(
            "{}:{}x{}@{}Hz",
            output.name, output.width, output.height, output.refresh_hz
        ));
    }

    rendered.join(",")
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

#[cfg(test)]
mod tests {
    use super::mock_surface_registry;
    use waybroker_common::WaylandSurfaceRole;

    #[test]
    fn mock_registry_contains_mapped_focusable_surface() {
        let registry = mock_surface_registry();
        assert_eq!(registry.surfaces.len(), 2);
        assert!(registry.surfaces.iter().any(|surface| {
            surface.role == WaylandSurfaceRole::Toplevel
                && surface.mapped
                && surface.buffer_attached
        }));
    }
}
