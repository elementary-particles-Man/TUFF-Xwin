use std::{env, io::BufReader};

use anyhow::{Result, bail};
use waybroker_common::{
    DisplayCommand, DisplayEvent, IpcEnvelope, MessageKind, OutputMode, ServiceBanner, ServiceRole,
    connect_service_socket, read_json_line, send_json_line,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(
        ServiceRole::Waylandd,
        "wayland endpoint, client lifecycle, clipboard core",
    );
    println!("{}", banner.render());

    match query_output_inventory() {
        Ok(outputs) => {
            println!("waylandd displayd_outputs={}", format_outputs(&outputs));
            Ok(())
        }
        Err(err) if config.require_displayd => Err(err),
        Err(err) => {
            println!("waylandd displayd_state=unreachable reason={err}");
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Config {
    require_displayd: bool,
}

impl Config {
    fn from_args(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

        for arg in args {
            match arg.as_str() {
                "--require-displayd" => config.require_displayd = true,
                "--help" | "-h" => {
                    println!("usage: waylandd [--require-displayd]");
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
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
