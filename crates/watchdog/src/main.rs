use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{BufReader, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    DesktopComponentState, DesktopHealthStatus, DesktopRecoveryAction, IpcEnvelope, MessageKind,
    ServiceBanner, ServiceEndpoint, ServiceRole, ServiceStream, SessionCommand,
    SessionLaunchComponentState, SessionLaunchDelta, SessionLaunchState,
    SessionWatchdogComponentReport, SessionWatchdogReport, WatchdogCommand, bind_service_socket,
    connect_service_socket, ensure_runtime_dir, is_recoverable_accept_error, now_unix_timestamp,
    read_json_line, runtime_dir, send_json_line, session_artifact_path,
    validate_session_instance_id,
};

const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CACHED_STATES: usize = 128;
const MAX_COMPONENTS_PER_STATE: usize = 256;
const MAX_PROFILE_ID_BYTES: usize = 128;
const MAX_RESTART_REASON_BYTES: usize = 1024;

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
    session_instance_id: Option<String>,
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
                "--session-instance-id" => {
                    let id = args.next().context("--session-instance-id requires an id")?;
                    if !validate_session_instance_id(&id) {
                        bail!(
                            "invalid session-instance-id: \"{}\". Must be 1-128 chars of [A-Za-z0-9._-]",
                            id
                        );
                    }
                    config.session_instance_id = Some(id);
                }
                "--inspect-all" => config.inspect_all = true,
                "--write-reports" => config.write_reports = true,
                "--notify-sessiond" => config.notify_sessiond = true,
                "--serve-ipc" => config.serve_ipc = true,
                "--once" => config.serve_once = true,
                "--help" | "-h" => {
                    println!(
                        "usage: watchdog [--launch-state PATH] [--profile-id ID] [--session-instance-id ID] [--inspect-all] [--write-reports] [--notify-sessiond] [--serve-ipc] [--once]"
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
    let listener = bind_service_socket(ServiceRole::Watchdog)?;
    let _socket_guard = SocketGuard::new(listener.endpoint().clone());
    println!("watchdog listening socket={}", listener.endpoint());
    let mut server = WatchdogServer::default();

    let mut served = 0usize;
    loop {
        let stream = match listener.accept() {
            Ok((stream, _peer)) => stream,
            Err(err) if is_recoverable_accept_error(&err) => {
                eprintln!("watchdog accept recoverable error: {err}");
                continue;
            }
            Err(err) => return Err(err).context("watchdog listener accept failed"),
        };

        match handle_client(stream, config, &mut server) {
            Ok(()) => {
                served += 1;
                if config.serve_once {
                    break;
                }
            }
            Err(err) => eprintln!("watchdog client failed: {err:#}"),
        }
    }

    println!("watchdog served_requests={served}");
    Ok(())
}

fn handle_client(
    mut stream: ServiceStream,
    config: &Config,
    server: &mut WatchdogServer,
) -> Result<()> {
    stream.set_read_timeout(Some(CLIENT_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(CLIENT_IO_TIMEOUT))?;
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
            let response = if let Some(reason) = validate_watchdog_request(&command, source, config)
            {
                WatchdogCommand::Escalate { level: 1, reason }
            } else {
                handle_watchdog_command(command, source, config, server)?
            };
            MessageKind::WatchdogCommand(response)
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

fn validate_watchdog_request(
    command: &WatchdogCommand,
    source: ServiceRole,
    config: &Config,
) -> Option<String> {
    if source != ServiceRole::Sessiond {
        return Some(format!("watchdog command source {} is not authorized", source.as_str()));
    }

    match command {
        WatchdogCommand::Restart { role, session_instance_id, reason } => {
            if *role != ServiceRole::Compd && *role != ServiceRole::Lockd {
                return Some(format!("watchdog does not support restarting {}", role.as_str()));
            }
            if !validate_session_instance_id(session_instance_id) {
                return Some("invalid session-instance-id".into());
            }
            if let Some(expected) = config.session_instance_id.as_deref()
                && expected != session_instance_id
            {
                return Some("session-instance-id does not match watchdog configuration".into());
            }
            if !valid_reason(reason) {
                return Some(
                    "restart reason must be 1-1024 UTF-8 bytes without control characters".into(),
                );
            }
            None
        }
        WatchdogCommand::InspectLaunchState { state } => validate_launch_state(state, config).err(),
        WatchdogCommand::UpdateLaunchState { delta } => validate_delta(delta, config).err(),
        _ => Some("watchdog IPC command is not accepted from a client".into()),
    }
}

fn valid_reason(reason: &str) -> bool {
    !reason.is_empty()
        && reason.len() <= MAX_RESTART_REASON_BYTES
        && !reason.chars().any(char::is_control)
}

fn valid_profile_id(profile_id: &str) -> bool {
    !profile_id.is_empty()
        && profile_id.len() <= MAX_PROFILE_ID_BYTES
        && !profile_id.chars().any(char::is_control)
}

fn validate_session_for_config(session_instance_id: &str, config: &Config) -> Result<(), String> {
    if !validate_session_instance_id(session_instance_id) {
        return Err("invalid session-instance-id".into());
    }
    if let Some(expected) = config.session_instance_id.as_deref()
        && expected != session_instance_id
    {
        return Err("session-instance-id does not match watchdog configuration".into());
    }
    Ok(())
}

fn validate_components(components: &[SessionLaunchComponentState]) -> Result<(), String> {
    if components.len() > MAX_COMPONENTS_PER_STATE {
        return Err(format!(
            "component count {} exceeds {}",
            components.len(),
            MAX_COMPONENTS_PER_STATE
        ));
    }

    let mut ids = HashSet::with_capacity(components.len());
    for component in components {
        if component.id.is_empty() || !ids.insert(component.id.as_str()) {
            return Err(format!("component id is empty or duplicated: {:?}", component.id));
        }
    }
    Ok(())
}

fn validate_launch_state(state: &SessionLaunchState, config: &Config) -> Result<(), String> {
    if !valid_profile_id(&state.profile_id) {
        return Err("invalid profile id".into());
    }
    validate_session_for_config(&state.session_instance_id, config)?;
    validate_components(&state.components)
}

fn validate_delta(delta: &SessionLaunchDelta, config: &Config) -> Result<(), String> {
    if !valid_profile_id(&delta.profile_id) {
        return Err("invalid profile id".into());
    }
    validate_session_for_config(&delta.session_instance_id, config)?;
    validate_components(&delta.components)
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
    if let Some(reason) = validate_watchdog_request(&command, source, config) {
        return Ok(WatchdogCommand::Escalate { level: 1, reason });
    }

    match command {
        WatchdogCommand::Restart { role, session_instance_id, reason } => {
            if role == ServiceRole::Compd || role == ServiceRole::Lockd {
                println!(
                    "service=watchdog op=recovery_request event=accepted role={} reason=\"{}\" requested_by={} session_instance_id={}",
                    role.as_str(),
                    reason,
                    source.as_str(),
                    session_instance_id
                );

                let artifact = WatchdogRecoveryArtifact {
                    role: role.as_str().into(),
                    reason: reason.clone(),
                    requested_by: source.as_str().into(),
                    unix_timestamp: now_unix_timestamp(),
                    action: "restart-request-accepted".into(),
                    status: "pending".into(),
                };

                let artifact_path = write_recovery_artifact(&artifact, &session_instance_id)?;
                println!(
                    "service=watchdog op=write_recovery_artifact path={}",
                    artifact_path.display()
                );

                Ok(WatchdogCommand::Restart { role, session_instance_id, reason })
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
                    "watchdog ignored_launch_state profile={} session_instance={} generation={} sequence={} reason={}",
                    state.profile_id,
                    state.session_instance_id,
                    state.generation,
                    state.sequence,
                    reason
                );

                let report = inspect_launch_state(&state);
                print_report(&state, &report);

                Ok(WatchdogCommand::InspectionResult { report })
            }
            StateUpdateOutcome::Invalid { reason } => {
                Ok(WatchdogCommand::Escalate { level: 1, reason })
            }
            StateUpdateOutcome::Resync { profile_id, session_instance_id, reason } => {
                Ok(WatchdogCommand::ResyncLaunchState { profile_id, session_instance_id, reason })
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
                    "watchdog ignored_launch_state profile={} session_instance={} generation={} sequence={} reason={}",
                    state.profile_id,
                    state.session_instance_id,
                    state.generation,
                    state.sequence,
                    reason
                );

                let report = inspect_launch_state(&state);
                print_report(&state, &report);

                Ok(WatchdogCommand::InspectionResult { report })
            }
            StateUpdateOutcome::Invalid { reason } => {
                Ok(WatchdogCommand::Escalate { level: 1, reason })
            }
            StateUpdateOutcome::Resync { profile_id, session_instance_id, reason } => {
                Ok(WatchdogCommand::ResyncLaunchState { profile_id, session_instance_id, reason })
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
    cached_states: HashMap<LaunchStateCacheKey, SessionLaunchState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LaunchStateCacheKey {
    profile_id: String,
    session_instance_id: String,
}

impl LaunchStateCacheKey {
    fn from_state(state: &SessionLaunchState) -> Self {
        Self {
            profile_id: state.profile_id.clone(),
            session_instance_id: state.session_instance_id.clone(),
        }
    }

    fn from_delta(profile_id: String, session_instance_id: String) -> Self {
        Self { profile_id, session_instance_id }
    }
}

#[derive(Debug)]
enum StateUpdateOutcome {
    Accepted(SessionLaunchState),
    Ignored { state: SessionLaunchState, reason: String },
    Invalid { reason: String },
    Resync { profile_id: String, session_instance_id: String, reason: String },
}

impl WatchdogServer {
    fn cache_full_state(&mut self, state: &SessionLaunchState) -> StateUpdateOutcome {
        if let Err(reason) = validate_launch_state(state, &Config::default()) {
            return StateUpdateOutcome::Invalid { reason };
        }

        let cache_key = LaunchStateCacheKey::from_state(state);

        if let Some(current) = self.cached_states.get(&cache_key).cloned() {
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

            if state.generation == current.generation && state.sequence == current.sequence {
                if state == &current {
                    return StateUpdateOutcome::Ignored {
                        state: current,
                        reason: "duplicate full state".into(),
                    };
                }
                return StateUpdateOutcome::Resync {
                    profile_id: state.profile_id.clone(),
                    session_instance_id: state.session_instance_id.clone(),
                    reason: "same generation and sequence carried different full state".into(),
                };
            }
        } else if self.cached_states.len() >= MAX_CACHED_STATES {
            return StateUpdateOutcome::Invalid {
                reason: format!("watchdog cache capacity {} reached", MAX_CACHED_STATES),
            };
        }

        self.cached_states.insert(cache_key, state.clone());
        StateUpdateOutcome::Accepted(state.clone())
    }

    fn apply_delta(&mut self, delta: SessionLaunchDelta) -> StateUpdateOutcome {
        let SessionLaunchDelta {
            profile_id,
            session_instance_id,
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

        let validation = if !valid_profile_id(&profile_id) {
            Err("invalid profile id".into())
        } else if let Err(reason) =
            validate_session_for_config(&session_instance_id, &Config::default())
        {
            Err(reason)
        } else {
            validate_components(&components)
        };
        if let Err(reason) = validation {
            return StateUpdateOutcome::Invalid { reason };
        }

        let cache_key =
            LaunchStateCacheKey::from_delta(profile_id.clone(), session_instance_id.clone());

        let Some(current) = self.cached_states.get(&cache_key).cloned() else {
            return StateUpdateOutcome::Resync {
                profile_id,
                session_instance_id,
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
                    session_instance_id,
                    reason: format!(
                        "watchdog observed generation jump {} -> {} without full replace",
                        current.generation, generation
                    ),
                };
            }

            if sequence != 1 {
                return StateUpdateOutcome::Resync {
                    profile_id,
                    session_instance_id,
                    reason: format!(
                        "watchdog expected sequence 1 for new generation {} but got {}",
                        generation, sequence
                    ),
                };
            }
        } else {
            let Some(expected_sequence) = current.sequence.checked_add(1) else {
                return StateUpdateOutcome::Resync {
                    profile_id,
                    session_instance_id,
                    reason: "watchdog sequence overflow requires full launch-state resend".into(),
                };
            };
            if sequence != expected_sequence {
                return StateUpdateOutcome::Resync {
                    profile_id,
                    session_instance_id,
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
                session_instance_id: session_instance_id.clone(),
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

        if next_state.components.len() > MAX_COMPONENTS_PER_STATE {
            return StateUpdateOutcome::Invalid {
                reason: format!(
                    "merged component count {} exceeds {}",
                    next_state.components.len(),
                    MAX_COMPONENTS_PER_STATE
                ),
            };
        }

        self.cached_states.insert(cache_key, next_state.clone());
        StateUpdateOutcome::Accepted(next_state)
    }
}

fn load_launch_states(config: &Config) -> Result<Vec<SessionLaunchState>> {
    if let Some(path) = config.launch_state_path.as_ref() {
        return Ok(vec![load_launch_state(path)?]);
    }

    let runtime = runtime_dir();
    let mut launch_states = Vec::new();
    let entries = fs::read_dir(&runtime)
        .with_context(|| format!("failed to read runtime dir {}", runtime.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();

        // session-<instance_id>-launch-state.json 形式をサポート
        if (file_name.starts_with("session-") && file_name.ends_with("-launch-state.json"))
            || (file_name.starts_with("launch-state-") && file_name.ends_with(".json"))
        {
            launch_states.push(load_launch_state(&path)?);
        }
    }

    if let Some(id) = config.session_instance_id.as_deref() {
        launch_states.retain(|state| state.session_instance_id == id);
    } else if let Some(id) = config.profile_id.as_deref() {
        launch_states.retain(|state| state.profile_id == id);
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
        session_instance_id: state.session_instance_id.clone(),
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

fn write_recovery_artifact(
    artifact: &WatchdogRecoveryArtifact,
    session_instance_id: &str,
) -> Result<PathBuf> {
    let _ = ensure_runtime_dir()?;
    let path =
        session_artifact_path(session_instance_id, &format!("watchdog-recovery-{}", artifact.role));
    let json = serde_json::to_string_pretty(artifact)
        .context("failed to serialize watchdog recovery artifact")?;
    atomic_write_json(&path, &json)?;
    Ok(path)
}

fn write_report(report: &SessionWatchdogReport) -> Result<PathBuf> {
    let _ = ensure_runtime_dir()?;
    let path = session_artifact_path(&report.session_instance_id, "watchdog-report");
    let json =
        serde_json::to_string_pretty(report).context("failed to serialize watchdog report")?;
    atomic_write_json(&path, &json)?;
    Ok(path)
}

fn atomic_write_json(path: &Path, json: &str) -> Result<()> {
    let parent = path.parent().context("artifact path has no parent directory")?;
    let file_name = path.file_name().context("artifact path has no file name")?.to_string_lossy();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let temp_path = parent.join(format!(".{file_name}.tmp-{}-{nonce}", std::process::id()));
    let mut created = false;

    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = options.open(&temp_path).with_context(|| {
            format!("failed to create temporary artifact {}", temp_path.display())
        })?;
        created = true;
        file.write_all(json.as_bytes()).with_context(|| {
            format!("failed to write temporary artifact {}", temp_path.display())
        })?;
        file.sync_all().with_context(|| {
            format!("failed to sync temporary artifact {}", temp_path.display())
        })?;
        fs::rename(&temp_path, path)
            .with_context(|| format!("failed to replace artifact {}", path.display()))?;
        Ok(())
    })();

    if result.is_err() && created {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn print_report(state: &SessionLaunchState, report: &SessionWatchdogReport) {
    println!(
        "service=watchdog op=report profile={} session_instance={} generation={} sequence={} healthy={} unhealthy={} inactive={} timestamp={}",
        report.profile_id,
        state.session_instance_id,
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

struct SocketGuard {
    endpoint: ServiceEndpoint,
}

impl SocketGuard {
    fn new(endpoint: ServiceEndpoint) -> Self {
        Self { endpoint }
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = self.endpoint.cleanup_stale();
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
            session_instance_id: "demo-instance-a".into(),
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
            session_instance_id: "demo-instance-a".into(),
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
            session_instance_id: "demo-instance-a".into(),
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
            session_instance_id: "demo-instance-a".into(),
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
                    session_instance_id: "demo-instance-a".into(),
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
            WatchdogCommand::ResyncLaunchState { profile_id, session_instance_id, reason } => {
                assert_eq!(profile_id, "demo");
                assert_eq!(session_instance_id, "demo-instance-a");
                assert!(reason.contains("cache miss"));
            }
            other => panic!("expected resync request, got {other:?}"),
        }
    }

    #[test]
    fn ignores_stale_delta_when_sequence_goes_backwards() {
        let state = SessionLaunchState {
            profile_id: "demo".into(),
            session_instance_id: "demo-instance-a".into(),
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
            session_instance_id: "demo-instance-a".into(),
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
            session_instance_id: "demo-instance-a".into(),
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
            session_instance_id: "demo-instance-a".into(),
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
            super::StateUpdateOutcome::Resync { profile_id, session_instance_id, reason } => {
                assert_eq!(profile_id, "demo");
                assert_eq!(session_instance_id, "demo-instance-a");
                assert!(reason.contains("sequence gap"));
            }
            other => panic!("expected resync for sequence gap, got {other:?}"),
        }
    }

    #[test]
    fn separates_cached_states_by_session_instance_id() {
        let mut server = WatchdogServer::default();

        let state_a = SessionLaunchState {
            profile_id: "demo".into(),
            session_instance_id: "demo-instance-a".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 1,
            components: vec![SessionLaunchComponentState {
                id: "wm-a".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["wm-a".into()],
                resolved_command: Some("/usr/bin/wm-a".into()),
                state: DesktopComponentState::Spawned,
                pid: Some(10),
                restart_count: 0,
                last_exit_status: None,
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        };
        let state_b = SessionLaunchState {
            profile_id: "demo".into(),
            session_instance_id: "demo-instance-b".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 1,
            components: vec![SessionLaunchComponentState {
                id: "wm-b".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["wm-b".into()],
                resolved_command: Some("/usr/bin/wm-b".into()),
                state: DesktopComponentState::Spawned,
                pid: Some(20),
                restart_count: 0,
                last_exit_status: None,
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        };

        match server.cache_full_state(&state_a) {
            StateUpdateOutcome::Accepted(_) => {}
            other => panic!("expected first state to be cached, got {other:?}"),
        }
        match server.cache_full_state(&state_b) {
            StateUpdateOutcome::Accepted(_) => {}
            other => panic!("expected second state to be cached, got {other:?}"),
        }

        let merged = match server.apply_delta(SessionLaunchDelta {
            profile_id: "demo".into(),
            session_instance_id: "demo-instance-a".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 2,
            replace: false,
            components: vec![SessionLaunchComponentState {
                id: "wm-a".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["wm-a".into()],
                resolved_command: Some("/usr/bin/wm-a".into()),
                state: DesktopComponentState::Failed,
                pid: None,
                restart_count: 1,
                last_exit_status: Some(1),
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        }) {
            StateUpdateOutcome::Accepted(state) => state,
            other => panic!("expected delta for instance a to merge, got {other:?}"),
        };

        assert_eq!(merged.session_instance_id, "demo-instance-a");
        assert_eq!(merged.components[0].id, "wm-a");
        assert_eq!(merged.components[0].state, DesktopComponentState::Failed);

        let cached_other = server
            .cached_states
            .values()
            .find(|state| state.session_instance_id == "demo-instance-b")
            .expect("cached state for second session");
        assert_eq!(cached_other.components[0].id, "wm-b");
        assert_eq!(cached_other.components[0].state, DesktopComponentState::Spawned);
    }

    #[test]
    fn recovery_request_is_session_aware() {
        use crate::{
            Config, ServiceRole, WatchdogCommand, WatchdogServer, handle_watchdog_command,
            session_artifact_path,
        };
        use std::fs;

        let mut server = WatchdogServer::default();
        let config = Config::default();
        let source = ServiceRole::Sessiond;

        let command_a = WatchdogCommand::Restart {
            role: ServiceRole::Compd,
            session_instance_id: "session-a".into(),
            reason: "test-a".into(),
        };

        // session-a 向けのリクエスト
        let res_a =
            handle_watchdog_command(command_a, source, &config, &mut server).expect("handle a");
        match res_a {
            WatchdogCommand::Restart { ref session_instance_id, .. } => {
                assert_eq!(session_instance_id, "session-a");
            }
            _ => panic!("expected restart command"),
        }

        let path_a = session_artifact_path("session-a", "watchdog-recovery-compd");
        assert!(path_a.exists(), "artifact for session-a should exist at {}", path_a.display());
        let _ = fs::remove_file(&path_a);

        let path_b = session_artifact_path("session-b", "watchdog-recovery-compd");
        assert!(!path_b.exists(), "artifact for session-b should not exist");
    }

    #[test]
    fn test_phase6_degraded_fallback_policy_evaluation() {
        let state = SessionLaunchState {
            profile_id: "demo-x11".into(),
            session_instance_id: "test-session".into(),
            display_name: "Demo X11".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 1,
            components: vec![SessionLaunchComponentState {
                id: "demo-shell".into(),
                role: DesktopComponentRole::Shell,
                critical: true,
                command: vec!["demo-shell".into()],
                resolved_command: Some("/usr/bin/demo-shell".into()),
                state: DesktopComponentState::Failed,
                pid: None,
                restart_count: 5,
                last_exit_status: Some(1),
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        };

        let report = inspect_launch_state(&state);
        assert_eq!(report.unhealthy_components, 1);
        assert_eq!(report.components[0].action, DesktopRecoveryAction::DegradedProfile);
        assert!(report.components[0].reason.contains("component spawn failed"));
    }
}
