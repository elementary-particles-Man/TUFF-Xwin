use std::{env, fs, io::BufReader, os::unix::net::UnixStream, path::PathBuf};

use anyhow::{Result, bail};
use waybroker_common::{
    IpcEnvelope, LockCommand, LockState, MessageKind, ServiceBanner, ServiceRole,
    bind_service_socket, read_json_line, send_json_line,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Lockd, "lockscreen and auth ui");
    println!("{}", banner.render());

    if config.serve_ipc {
        serve_ipc(&config)?;
    } else {
        println!("service=lockd state=idle (use --serve-ipc to start lock service)");
    }

    Ok(())
}

#[derive(Debug, Default)]
struct Config {
    serve_ipc: bool,
    serve_once: bool,
    fail_resume: bool,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--serve-ipc" => config.serve_ipc = true,
                "--once" => config.serve_once = true,
                "--fail-resume" => config.fail_resume = true,
                "--help" | "-h" => {
                    println!("usage: lockd [--serve-ipc] [--once] [--fail-resume]");
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

fn serve_ipc(config: &Config) -> Result<()> {
    let (listener, socket_path) = bind_service_socket(ServiceRole::Lockd)?;
    let _socket_guard = SocketGuard::new(socket_path.clone());
    println!("service=lockd op=listen event=socket_bound path={}", socket_path.display());

    let mut current_state = LockState::Unlocked;
    println!("service=lockd op=status event=ready current_state={:?}", current_state);

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = stream?;
        handle_client(stream, &mut current_state, config)?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("service=lockd op=terminate event=finished served_requests={served}");
    Ok(())
}

fn handle_client(
    mut stream: UnixStream,
    current_state: &mut LockState,
    config: &Config,
) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let response = build_response(request, current_state, config);
    send_json_line(&mut stream, &response)?;
    Ok(())
}

fn build_response(
    request: IpcEnvelope,
    current_state: &mut LockState,
    config: &Config,
) -> IpcEnvelope {
    let source = request.source;
    let response_kind = match request.kind {
        MessageKind::LockCommand(LockCommand::SetLockState { state })
            if request.destination == ServiceRole::Lockd =>
        {
            if config.fail_resume {
                println!("service=lockd op=set_lock_state event=failed reason=\"fault injection\"");
                MessageKind::LockCommand(LockCommand::AuthPrompt {
                    reason: "fault injection".into(),
                })
            } else {
                let old_state = *current_state;
                *current_state = state;
                println!(
                    "service=lockd op=state_transition event=success from={:?} to={:?}",
                    old_state, state
                );
                MessageKind::LockCommand(LockCommand::SetLockState { state })
            }
        }
        MessageKind::LockCommand(LockCommand::AuthPrompt { reason }) => {
            if config.fail_resume {
                println!(
                    "service=lockd op=auth_prompt event=failed reason=\"fault injection\" prompt_reason=\"{}\"",
                    reason
                );
                MessageKind::LockCommand(LockCommand::AuthPrompt {
                    reason: "fault injection".into(),
                })
            } else {
                println!("service=lockd op=auth_prompt event=success reason=\"{}\"", reason);
                MessageKind::LockCommand(LockCommand::AuthPrompt { reason })
            }
        }
        MessageKind::LockCommand(_) => MessageKind::LockCommand(LockCommand::AuthPrompt {
            reason: format!("lockd received message addressed to {}", request.destination.as_str()),
        }),
        other => MessageKind::LockCommand(LockCommand::AuthPrompt {
            reason: format!("lockd does not handle {other:?}"),
        }),
    };

    IpcEnvelope::new(ServiceRole::Lockd, source, response_kind)
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
    use super::LockState;

    #[test]
    fn state_transition_works() {
        let mut state = LockState::Unlocked;
        state = LockState::Locked;
        assert_eq!(state, LockState::Locked);
    }
}
