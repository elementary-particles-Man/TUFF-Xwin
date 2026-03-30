use std::{
    collections::HashMap,
    env, fs,
    io::BufReader,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    DesktopComponentState, DesktopHealthStatus, DesktopRecoveryAction, IpcEnvelope, MessageKind,
    ServiceBanner, ServiceRole, SessionCommand, SessionLaunchComponentState, SessionLaunchDelta,
    SessionLaunchState, SessionWatchdogComponentReport, SessionWatchdogReport, WatchdogCommand,
    bind_service_socket, connect_service_socket, ensure_runtime_dir, now_unix_timestamp,
    read_json_line, runtime_dir, send_json_line,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Watchdog, "display stack recovery control");
    println!("{}", banner.render());

    if config.serve_ipc {
        serve_ipc(&config)?;
        return Ok(());
    }

    if !config.has_inspection_target() {
        return Ok(());
    }

    let launch_states = load_launch_states(&config)?;

    for launch_state in &launch_states {
        let report = inspect_launch_state(launch_state);
        print_report(launch_state, &report);

        if config.write_reports {
            let report_path = write_report(&report)?;
            println!("service=watchdog op=write_report path={}", report_path.display());
        }

        if config.notify_sessiond {
            let response = notify_sessiond(&report)?;
            print_sessiond_response(&response);
        }
    }

    Ok(())
}

#[derive(Debug, Default)]
struct Config {
    launch_state_path: Option<PathBuf>,
    profile_id: Option<String>,
    inspect_all: bool,
    write_reports: bool,
    notify_sessiond: bool,
    serve_ipc: bool,
    serve_once: bool,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--launch-state" => {
                    let path = args.next().context("--launch-state requires a path")?;
                    config.launch_state_path = Some(PathBuf::from(path));
                }
                "--profile-id" => {
                    let id = args.next().context("--profile-id requires an id")?;
                    config.profile_id = Some(id);
                }
                "--inspect-all" => config.inspect_all = true,
                "--write-reports" => config.write_reports = true,
                "--notify-sessiond" => config.notify_sessiond = true,
                "--serve-ipc" => config.serve_ipc = true,
                "--once" => config.serve_once = true,
                "--help" | "-h" => {
                    println!(
                        "usage: watchdog [--launch-state PATH] [--profile-id ID] [--inspect-all] [--write-reports] [--notify-sessiond] [--serve-ipc] [--once]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }

    fn has_inspection_target(&self) -> bool {
        self.launch_state_path.is_some() || self.profile_id.is_some() || self.inspect_all
    }
}

fn serve_ipc(config: &Config) -> Result<()> {
    let (listener, socket_path) = bind_service_socket(ServiceRole::Watchdog)?;
    let _socket_guard = SocketGuard::new(socket_path.clone());
    println!("watchdog listening socket={}", socket_path.display());
    let mut server = WatchdogServer::default();

    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = stream?;
        handle_client(stream, config, &mut server)?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    println!("watchdog served_requests={served}");
    Ok(())
}

fn handle_client(
    mut stream: UnixStream,
    config: &Config,
    server: &mut WatchdogServer,
) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let response = build_response(request, config, server)?;
    send_json_line(&mut stream, &response)?;
    Ok(())
}

fn build_response(
    request: IpcEnvelope,
    config: &Config,
    server: &mut WatchdogServer,
) -> Result<IpcEnvelope> {
    let source = request.source;
    let response_kind = match request.kind {
        MessageKind::WatchdogCommand(command) if request.destination == ServiceRole::Watchdog => {
            MessageKind::WatchdogCommand(handle_watchdog_command(command, source, config, server)?)
        }
        MessageKind::WatchdogCommand(_) => {
            MessageKind::WatchdogCommand(WatchdogCommand::Escalate {
                level: 1,
                reason: format!(
                    "watchdog received message addressed to {}",
                    request.destination.as_str()
                ),
            })
        }
        other => MessageKind::WatchdogCommand(WatchdogCommand::Escalate {
            level: 1,
            reason: format!("watchdog does not handle {other:?}"),
        }),
    };

    Ok(IpcEnvelope::new(ServiceRole::Watchdog, source, response_kind))
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct WatchdogRecoveryArtifact {
    role: String,
    reason: String,
    requested_by: String,
    unix_timestamp: u64,
    action: String,
    status: String,
}

fn handle_watchdog_command(
    command: WatchdogCommand,
    source: ServiceRole,
    config: &Config,
    server: &mut WatchdogServer,
) -> Result<WatchdogCommand> {
    match command {
        WatchdogCommand::Restart { role, reason } => {
            if role == ServiceRole::Compd || role == ServiceRole::Lockd {
                println!(
                    "service=watchdog op=recovery_request event=accepted role={} reason=\"{}\" requested_by={}",
                    role.as_str(),
                    reason,
                    source.as_str()
                );

                let artifact = WatchdogRecoveryArtifact {
                    role: role.as_str().into(),
                    reason: reason.clone(),
                    requested_by: source.as_str().into(),
                    unix_timestamp: now_unix_timestamp(),
                    action: "restart-request-accepted".into(),
                    status: "pending".into(),
                };

                let artifact_path = write_recovery_artifact(&artifact)?;
                println!(
                    "service=watchdog op=write_recovery_artifact path={}",
                    artifact_path.display()
                );

                Ok(WatchdogCommand::Restart { role, reason })
            } else {
                println!(
                    "service=watchdog op=recovery_request event=rejected role={} reason=\"unsupported role\"",
                    role.as_str()
                );
                Ok(WatchdogCommand::Escalate {
                    level: 1,
                    reason: format!("watchdog does not support restarting {}", role.as_str()),
                })
            }
        }
        WatchdogCommand::InspectLaunchState { state } => match server.cache_full_state(&state) {
            StateUpdateOutcome::Accepted(state) => {
                let report = inspect_launch_state(&state);
                print_report(&state, &report);

                if config.write_reports {
                    let report_path = write_report(&report)?;
                    println!("watchdog wrote_report={}", report_path.display());
                }

                if config.notify_sessiond && source != ServiceRole::Sessiond {
                    let response = notify_sessiond(&report)?;
                    print_sessiond_response(&response);
                }

                Ok(WatchdogCommand::InspectionResult { report })
            }
            StateUpdateOutcome::Ignored { state, reason } => {
                println!(
                    "watchdog ignored_launch_state profile={} generation={} sequence={} reason={}",
                    state.profile_id, state.generation, state.sequence, reason
                );

                let report = inspect_launch_state(&state);
                print_report(&state, &report);

                if config.write_reports {
                    let report_path = write_report(&report)?;
                    println!("watchdog wrote_report={}", report_path.display());
                }

                if config.notify_sessiond && source != ServiceRole::Sessiond {
                    let response = notify_sessiond(&report)?;
                    print_sessiond_response(&response);
                }

                Ok(WatchdogCommand::InspectionResult { report })
            }
            StateUpdateOutcome::Resync { profile_id, reason } => {
                Ok(WatchdogCommand::ResyncLaunchState { profile_id, reason })
            }
        },
        WatchdogCommand::UpdateLaunchState { delta } => match server.apply_delta(delta) {
            StateUpdateOutcome::Accepted(state) => {
                let report = inspect_launch_state(&state);
                print_report(&state, &report);

                if config.write_reports {
                    let report_path = write_report(&report)?;
                    println!("watchdog wrote_report={}", report_path.display());
                }

                if config.notify_sessiond && source != ServiceRole::Sessiond {
                    let response = notify_sessiond(&report)?;
                    print_sessiond_response(&response);
                }

                Ok(WatchdogCommand::InspectionResult { report })
            }
            StateUpdateOutcome::Ignored { state, reason } => {
                println!(
                    "watchdog ignored_launch_state profile={} generation={} sequence={} reason={}",
                    state.profile_id, state.generation, state.sequence, reason
                );

                let report = inspect_launch_state(&state);
                print_report(&state, &report);

                if config.write_reports {
                    let report_path = write_report(&report)?;
                    println!("watchdog wrote_report={}", report_path.display());
                }

                if config.notify_sessiond && source != ServiceRole::Sessiond {
                    let response = notify_sessiond(&report)?;
                    print_sessiond_response(&response);
                }

                Ok(WatchdogCommand::InspectionResult { report })
            }
            StateUpdateOutcome::Resync { profile_id, reason } => {
                Ok(WatchdogCommand::ResyncLaunchState { profile_id, reason })
            }
        },
        other => Ok(WatchdogCommand::Escalate {
            level: 1,
            reason: format!("watchdog IPC does not apply {other:?}"),
        }),
    }
}

#[derive(Default)]
struct WatchdogServer {
    cached_states: HashMap<String, SessionLaunchState>,
}

#[derive(Debug)]
enum StateUpdateOutcome {
    Accepted(SessionLaunchState),
    Ignored { state: SessionLaunchState, reason: String },
    Resync { profile_id: String, reason: String },
}

impl WatchdogServer {
    fn cache_full_state(&mut self, state: &SessionLaunchState) -> StateUpdateOutcome {
        if let Some(current) = self.cached_states.get(&state.profile_id).cloned() {
            if state.generation < current.generation
                || (state.generation == current.generation && state.sequence < current.sequence)
            {
                return StateUpdateOutcome::Ignored {
                    state: current.clone(),
                    reason: format!(
                        "incoming full state generation={} sequence={} is older than cached generation={} sequence={}",
                        state.generation, state.sequence, current.generation, current.sequence
                    ),
                };
            }
        }

        self.cached_states.insert(state.profile_id.clone(), state.clone());
        StateUpdateOutcome::Accepted(state.clone())
    }

    fn apply_delta(&mut self, delta: SessionLaunchDelta) -> StateUpdateOutcome {
        let SessionLaunchDelta {
            profile_id,
            display_name,
            protocol,
            broker_services,
            generation,
            sequence,
            replace,
            components,
            unix_timestamp: _,
            service_component_bindings,
            service_recovery_execution_policies,
        } = delta;

        let Some(current) = self.cached_states.get(&profile_id).cloned() else {
            return StateUpdateOutcome::Resync {
                profile_id,
                reason: "watchdog cache miss for profile; full launch-state resend required"
                    .to_string(),
            };
        };

        if generation < current.generation
            || (generation == current.generation && sequence <= current.sequence)
        {
            return StateUpdateOutcome::Ignored {
                state: current.clone(),
                reason: format!(
                    "incoming delta generation={} sequence={} is not newer than cached generation={} sequence={}",
                    generation, sequence, current.generation, current.sequence
                ),
            };
        }

        if generation > current.generation {
            if !replace {
                return StateUpdateOutcome::Resync {
                    profile_id,
                    reason: format!(
                        "watchdog observed generation jump {} -> {} without full replace",
                        current.generation, generation
                    ),
                };
            }

            if sequence != 1 {
                return StateUpdateOutcome::Resync {
                    profile_id,
                    reason: format!(
                        "watchdog expected sequence 1 for new generation {} but got {}",
                        generation, sequence
                    ),
                };
            }
        } else {
            let expected_sequence = current.sequence.saturating_add(1);
            if sequence != expected_sequence {
                return StateUpdateOutcome::Resync {
                    profile_id,
                    reason: format!(
                        "watchdog detected sequence gap: expected {}, got {}",
                        expected_sequence, sequence
                    ),
                };
            }
        }

        let next_state = if replace {
            SessionLaunchState {
                profile_id: profile_id.clone(),
                display_name,
                protocol,
                broker_services,
                generation,
                sequence,
                components,
                unix_timestamp: now_unix_timestamp(),
                service_component_bindings,
                service_recovery_execution_policies,
            }
        } else {
            let mut state = current;
            state.display_name = display_name;
            state.protocol = protocol;
            state.broker_services = broker_services;
            state.generation = generation;
            state.sequence = sequence;
            state.unix_timestamp = now_unix_timestamp();
            state.service_component_bindings = service_component_bindings;
            state.service_recovery_execution_policies = service_recovery_execution_policies;

            for component in components {
                if let Some(existing) =
                    state.components.iter_mut().find(|existing| existing.id == component.id)
                {
                    *existing = component;
                } else {
                    state.components.push(component);
                }
            }

            state
        };

        self.cached_states.insert(profile_id, next_state.clone());
        StateUpdateOutcome::Accepted(next_state)
    }
}

fn load_launch_states(config: &Config) -> Result<Vec<SessionLaunchState>> {
    if let Some(path) = config.launch_state_path.as_ref() {
        return Ok(vec![load_launch_state(path)?]);
    }

    if let Some(profile_id) = config.profile_id.as_deref() {
        return Ok(vec![load_launch_state(&launch_state_path(profile_id))?]);
    }

    let runtime = runtime_dir();
    let mut launch_states = Vec::new();
    let entries = fs::read_dir(&runtime)
        .with_context(|| format!("failed to read runtime dir {}", runtime.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();

        if file_name.starts_with("launch-state-") && file_name.ends_with(".json") {
            launch_states.push(load_launch_state(&path)?);
        }
    }

    if !config.inspect_all && launch_states.len() > 1 {
        launch_states.sort_by(|left, right| left.profile_id.cmp(&right.profile_id));
        launch_states.truncate(1);
    }

    if launch_states.is_empty() {
        bail!("no launch states found in {}", runtime.display());
    }

    Ok(launch_states)
}

fn load_launch_state(path: &Path) -> Result<SessionLaunchState> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read launch state {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to decode launch state {}", path.display()))
}

fn inspect_launch_state(state: &SessionLaunchState) -> SessionWatchdogReport {
    let mut components = Vec::with_capacity(state.components.len());
    let mut healthy_components = 0usize;
    let mut unhealthy_components = 0usize;
    let mut inactive_components = 0usize;

    for component in &state.components {
        let report = inspect_component(component);

        match report.status {
            DesktopHealthStatus::Healthy => healthy_components += 1,
            DesktopHealthStatus::Unhealthy => unhealthy_components += 1,
            DesktopHealthStatus::Inactive => inactive_components += 1,
        }

        components.push(report);
    }

    SessionWatchdogReport {
        profile_id: state.profile_id.clone(),
        display_name: state.display_name.clone(),
        protocol: state.protocol,
        healthy_components,
        unhealthy_components,
        inactive_components,
        components,
        unix_timestamp: now_unix_timestamp(),
    }
}

fn inspect_component(component: &SessionLaunchComponentState) -> SessionWatchdogComponentReport {
    let (status, action, reason) = match component.state {
        DesktopComponentState::Missing => (
            DesktopHealthStatus::Unhealthy,
            if component.critical {
                DesktopRecoveryAction::DegradedProfile
            } else {
                DesktopRecoveryAction::None
            },
            "component command is not installed".to_string(),
        ),
        DesktopComponentState::Failed => (
            DesktopHealthStatus::Unhealthy,
            component_action(component),
            "component spawn failed or supervisor gave up".to_string(),
        ),
        DesktopComponentState::Ready => (
            DesktopHealthStatus::Inactive,
            DesktopRecoveryAction::None,
            "component is installed but not launched".to_string(),
        ),
        DesktopComponentState::Spawned => match component.pid {
            Some(pid) if process_exists(pid) => (
                DesktopHealthStatus::Healthy,
                DesktopRecoveryAction::None,
                "component process is alive".to_string(),
            ),
            Some(_) => (
                DesktopHealthStatus::Unhealthy,
                component_action(component),
                "component process is missing".to_string(),
            ),
            None => (
                DesktopHealthStatus::Unhealthy,
                component_action(component),
                "component was marked spawned without pid".to_string(),
            ),
        },
    };

    SessionWatchdogComponentReport {
        id: component.id.clone(),
        role: component.role,
        critical: component.critical,
        status,
        pid: component.pid,
        crash_loop_count: component.restart_count,
        action,
        reason,
    }
}

fn component_action(component: &SessionLaunchComponentState) -> DesktopRecoveryAction {
    if !component.critical {
        return DesktopRecoveryAction::None;
    }

    if component.restart_count >= 3 {
        DesktopRecoveryAction::DegradedProfile
    } else {
        DesktopRecoveryAction::RestartComponent
    }
}

fn process_exists(pid: u32) -> bool {
    PathBuf::from("/proc").join(pid.to_string()).exists()
}

fn write_recovery_artifact(artifact: &WatchdogRecoveryArtifact) -> Result<PathBuf> {
    let dir = ensure_runtime_dir()?;
    let path = dir.join(format!("watchdog-recovery-{}.json", artifact.role));
    let json = serde_json::to_string_pretty(artifact)
        .context("failed to serialize watchdog recovery artifact")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn write_report(report: &SessionWatchdogReport) -> Result<PathBuf> {
    let dir = ensure_runtime_dir()?;
    let path = dir.join(format!("watchdog-report-{}.json", report.profile_id));
    let json =
        serde_json::to_string_pretty(report).context("failed to serialize watchdog report")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn print_report(state: &SessionLaunchState, report: &SessionWatchdogReport) {
    println!(
        "service=watchdog op=report profile={} generation={} sequence={} healthy={} unhealthy={} inactive={} timestamp={}",
        report.profile_id,
        state.generation,
        state.sequence,
        report.healthy_components,
        report.unhealthy_components,
        report.inactive_components,
        report.unix_timestamp,
    );

    for component in &report.components {
        println!(
            "service=watchdog op=component_status id={} role={} critical={} status={} pid={} crashes={} action={} reason=\"{}\"",
            component.id,
            component.role.as_str(),
            component.critical,
            component.status.as_str(),
            component.pid.map(|pid| pid.to_string()).as_deref().unwrap_or("none"),
            component.crash_loop_count,
            component.action.as_str(),
            component.reason
        );
    }
}

fn notify_sessiond(report: &SessionWatchdogReport) -> Result<IpcEnvelope> {
    let mut stream = connect_service_socket(ServiceRole::Sessiond)?;
    let request = IpcEnvelope::new(
        ServiceRole::Watchdog,
        ServiceRole::Sessiond,
        MessageKind::SessionCommand(SessionCommand::ApplyWatchdogReport { report: report.clone() }),
    );

    send_json_line(&mut stream, &request)?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;
    Ok(response)
}

fn print_sessiond_response(response: &IpcEnvelope) {
    match &response.kind {
        MessageKind::SessionCommand(SessionCommand::ProfileTransition { transition }) => {
            println!(
                "service=watchdog op=sessiond_response event=profile_transition from={} to={} reason=\"{}\" triggers={}",
                transition.source_profile_id,
                transition.target_profile_id,
                transition.reason,
                transition.trigger_component_ids.join(",")
            );
        }
        MessageKind::SessionCommand(SessionCommand::ProfileUnchanged { profile_id, reason }) => {
            println!(
                "service=watchdog op=sessiond_response event=profile_unchanged profile={} reason=\"{}\"",
                profile_id, reason
            );
        }
        other => println!("service=watchdog op=sessiond_response event=unknown kind={:?}", other),
    }
}

fn launch_state_path(profile_id: &str) -> PathBuf {
    runtime_dir().join(format!("launch-state-{profile_id}.json"))
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
    use super::{
        Config, StateUpdateOutcome, WatchdogServer, handle_watchdog_command, inspect_launch_state,
    };
    use waybroker_common::{
        DesktopComponentRole, DesktopComponentState, DesktopHealthStatus, DesktopProtocol,
        DesktopRecoveryAction, ServiceRole, SessionLaunchComponentState, SessionLaunchDelta,
        SessionLaunchState, WatchdogCommand,
    };

    #[test]
    fn marks_missing_component_as_unhealthy() {
        let state = SessionLaunchState {
            profile_id: "demo".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Watchdog],
            generation: 1,
            sequence: 1,
            components: vec![SessionLaunchComponentState {
                id: "missing".into(),
                role: DesktopComponentRole::Shell,
                critical: true,
                command: vec!["missing".into()],
                resolved_command: None,
                state: DesktopComponentState::Missing,
                pid: None,
                restart_count: 0,
                last_exit_status: None,
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        };

        let report = inspect_launch_state(&state);

        assert_eq!(report.unhealthy_components, 1);
        assert_eq!(report.components[0].status, DesktopHealthStatus::Unhealthy);
        assert_eq!(report.components[0].action, DesktopRecoveryAction::DegradedProfile);
    }

    #[test]
    fn marks_ready_component_as_inactive() {
        let state = SessionLaunchState {
            profile_id: "demo".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Watchdog],
            generation: 1,
            sequence: 1,
            components: vec![SessionLaunchComponentState {
                id: "ready".into(),
                role: DesktopComponentRole::Panel,
                critical: false,
                command: vec!["panel".into()],
                resolved_command: Some("/usr/bin/panel".into()),
                state: DesktopComponentState::Ready,
                pid: None,
                restart_count: 0,
                last_exit_status: None,
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        };

        let report = inspect_launch_state(&state);

        assert_eq!(report.inactive_components, 1);
        assert_eq!(report.components[0].status, DesktopHealthStatus::Inactive);
        assert_eq!(report.components[0].action, DesktopRecoveryAction::None);
    }

    #[test]
    fn merges_delta_into_cached_launch_state() {
        let state = SessionLaunchState {
            profile_id: "demo".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 1,
            components: vec![
                SessionLaunchComponentState {
                    id: "wm".into(),
                    role: DesktopComponentRole::WindowManager,
                    critical: true,
                    command: vec!["wm".into()],
                    resolved_command: Some("/usr/bin/wm".into()),
                    state: DesktopComponentState::Spawned,
                    pid: Some(10),
                    restart_count: 0,
                    last_exit_status: None,
                },
                SessionLaunchComponentState {
                    id: "panel".into(),
                    role: DesktopComponentRole::Panel,
                    critical: false,
                    command: vec!["panel".into()],
                    resolved_command: Some("/usr/bin/panel".into()),
                    state: DesktopComponentState::Spawned,
                    pid: Some(11),
                    restart_count: 0,
                    last_exit_status: None,
                },
            ],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        };
        let mut server = WatchdogServer::default();
        server.cache_full_state(&state);

        let merged = match server.apply_delta(SessionLaunchDelta {
            profile_id: "demo".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 2,
            replace: false,
            components: vec![SessionLaunchComponentState {
                id: "wm".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["wm".into()],
                resolved_command: Some("/usr/bin/wm".into()),
                state: DesktopComponentState::Failed,
                pid: None,
                restart_count: 3,
                last_exit_status: Some(1),
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        }) {
            StateUpdateOutcome::Accepted(state) => state,
            other => panic!("expected merged delta state, got {other:?}"),
        };

        assert_eq!(merged.components.len(), 2);
        assert_eq!(merged.components[0].id, "wm");
        assert_eq!(merged.components[0].state, DesktopComponentState::Failed);
        assert_eq!(merged.components[0].restart_count, 3);
        assert_eq!(merged.generation, 1);
        assert_eq!(merged.sequence, 2);
        assert_eq!(merged.components[1].id, "panel");
        assert_eq!(merged.components[1].state, DesktopComponentState::Spawned);
    }

    #[test]
    fn requests_resync_when_delta_arrives_without_cached_state() {
        let mut server = WatchdogServer::default();
        let config = Config::default();

        let response = handle_watchdog_command(
            WatchdogCommand::UpdateLaunchState {
                delta: SessionLaunchDelta {
                    profile_id: "demo".into(),
                    display_name: "Demo".into(),
                    protocol: DesktopProtocol::LayerX11,
                    broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
                    generation: 1,
                    sequence: 2,
                    replace: false,
                    components: vec![SessionLaunchComponentState {
                        id: "wm".into(),
                        role: DesktopComponentRole::WindowManager,
                        critical: true,
                        command: vec!["wm".into()],
                        resolved_command: Some("/usr/bin/wm".into()),
                        state: DesktopComponentState::Failed,
                        pid: None,
                        restart_count: 3,
                        last_exit_status: Some(1),
                    }],
                    unix_timestamp: 0,
                    service_component_bindings: Vec::new(),
                    service_recovery_execution_policies: Vec::new(),
                },
            },
            ServiceRole::Sessiond,
            &config,
            &mut server,
        )
        .expect("handle watchdog delta");

        match response {
            WatchdogCommand::ResyncLaunchState { profile_id, reason } => {
                assert_eq!(profile_id, "demo");
                assert!(reason.contains("cache miss"));
            }
            other => panic!("expected resync request, got {other:?}"),
        }
    }

    #[test]
    fn ignores_stale_delta_when_sequence_goes_backwards() {
        let state = SessionLaunchState {
            profile_id: "demo".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 3,
            components: vec![SessionLaunchComponentState {
                id: "wm".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["wm".into()],
                resolved_command: Some("/usr/bin/wm".into()),
                state: DesktopComponentState::Spawned,
                pid: Some(10),
                restart_count: 0,
                last_exit_status: None,
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        };
        let mut server = WatchdogServer::default();
        server.cache_full_state(&state);

        let response = server.apply_delta(SessionLaunchDelta {
            profile_id: "demo".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 2,
            replace: false,
            components: vec![SessionLaunchComponentState {
                id: "wm".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["wm".into()],
                resolved_command: Some("/usr/bin/wm".into()),
                state: DesktopComponentState::Failed,
                pid: None,
                restart_count: 3,
                last_exit_status: Some(1),
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        });

        match response {
            super::StateUpdateOutcome::Ignored { state, reason } => {
                assert_eq!(state.sequence, 3);
                assert_eq!(state.components[0].state, DesktopComponentState::Spawned);
                assert!(reason.contains("not newer"));
            }
            other => panic!("expected stale delta to be ignored, got {other:?}"),
        }
    }

    #[test]
    fn requests_resync_when_sequence_gap_is_detected() {
        let state = SessionLaunchState {
            profile_id: "demo".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 1,
            components: vec![SessionLaunchComponentState {
                id: "wm".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["wm".into()],
                resolved_command: Some("/usr/bin/wm".into()),
                state: DesktopComponentState::Spawned,
                pid: Some(10),
                restart_count: 0,
                last_exit_status: None,
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        };
        let mut server = WatchdogServer::default();
        server.cache_full_state(&state);

        let response = server.apply_delta(SessionLaunchDelta {
            profile_id: "demo".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 3,
            replace: false,
            components: vec![SessionLaunchComponentState {
                id: "wm".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["wm".into()],
                resolved_command: Some("/usr/bin/wm".into()),
                state: DesktopComponentState::Failed,
                pid: None,
                restart_count: 1,
                last_exit_status: Some(1),
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        });

        match response {
            super::StateUpdateOutcome::Resync { profile_id, reason } => {
                assert_eq!(profile_id, "demo");
                assert!(reason.contains("sequence gap"));
            }
            other => panic!("expected resync for sequence gap, got {other:?}"),
        }
    }
}
