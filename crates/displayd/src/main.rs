use std::{env, fs, io::BufReader, os::unix::net::UnixStream, path::PathBuf};

use anyhow::{Result, bail};
use waybroker_common::{
    DisplayCommand, DisplayEvent, IpcEnvelope, MessageKind, OutputMode, ServiceBanner, ServiceRole,
    bind_service_socket, read_json_line, send_json_line,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Displayd, "drm/kms, input, seat broker");
    println!("{}", banner.render());

    let (listener, socket_path) = bind_service_socket(ServiceRole::Displayd)?;
    let _socket_guard = SocketGuard::new(socket_path.clone());
    println!("displayd listening socket={}", socket_path.display());

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = stream?;
        handle_client(stream)?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("displayd served_requests={served}");
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct Config {
    serve_once: bool,
}

impl Config {
    fn from_args(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

        for arg in args {
            match arg.as_str() {
                "--once" => config.serve_once = true,
                "--help" | "-h" => {
                    println!("usage: displayd [--once]");
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

fn handle_client(mut stream: UnixStream) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let response = build_response(request);
    send_json_line(&mut stream, &response)?;
    Ok(())
}

fn build_response(request: IpcEnvelope) -> IpcEnvelope {
    let response_kind = match request.kind {
        MessageKind::DisplayCommand(command) if request.destination == ServiceRole::Displayd => {
            MessageKind::DisplayEvent(handle_display_command(command))
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

    IpcEnvelope::new(ServiceRole::Displayd, request.source, response_kind)
}

fn handle_display_command(command: DisplayCommand) -> DisplayEvent {
    match command {
        DisplayCommand::EnumerateOutputs => {
            DisplayEvent::OutputInventory { outputs: vec![stub_output_mode()] }
        }
        DisplayCommand::SetMode { output, mode } => DisplayEvent::ModeApplied { output, mode },
        DisplayCommand::CommitScene { target, focus, surfaces } => {
            DisplayEvent::SceneCommitted { target, focus, surface_count: surfaces.len() }
        }
        DisplayCommand::SecureBlank { output } => DisplayEvent::BlankApplied { output },
        DisplayCommand::ResumeBegin => DisplayEvent::ResumeStarted,
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
