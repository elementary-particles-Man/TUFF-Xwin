use std::{
    env, fs,
    io::{BufReader, ErrorKind},
    os::unix::fs::PermissionsExt,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    DesktopComponent, DesktopComponentState, DesktopProfile, DesktopRecoveryAction, IpcEnvelope,
    MessageKind, ServiceBanner, ServiceRole, SessionCommand, SessionLaunchComponentState,
    SessionLaunchState, SessionProfileTransition, SessionWatchdogReport, bind_service_socket,
    ensure_runtime_dir, read_json_line, runtime_dir, send_json_line,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let profiles_dir = config.profiles_dir();
    let profiles = load_profiles(&profiles_dir)?;

    let banner = ServiceBanner::new(
        ServiceRole::Sessiond,
        "lid, idle, suspend, session policy, desktop profile manager",
    );
    println!("{}", banner.render());
    println!("sessiond profiles_dir={} loaded_profiles={}", profiles_dir.display(), profiles.len());

    if config.list_profiles || config.selected_profile_id.is_none() {
        for profile in &profiles {
            println!(
                "sessiond profile id={} protocol={} name={} summary={}",
                profile.id,
                profile.protocol.as_str(),
                profile.display_name,
                profile.summary
            );
        }
    }

    if let Some(profile_id) = config.selected_profile_id.as_deref() {
        let profile = profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .with_context(|| format!("unknown profile id: {profile_id}"))?;
        let plan = profile.launch_plan();

        println!(
            "sessiond selected_profile id={} protocol={} components={}",
            profile.id,
            profile.protocol.as_str(),
            profile.session_components.len()
        );

        if config.print_launch_plan {
            for service in &plan.broker_services {
                println!("sessiond broker service={}", service.as_str());
            }

            for component in &plan.session_components {
                println!(
                    "sessiond component id={} role={:?} critical={} command={}",
                    component.id,
                    component.role,
                    component.critical,
                    component.command.join(" ")
                );
            }
        }

        if config.write_selection {
            let state_path = write_active_profile(profile)?;
            println!("sessiond wrote_active_profile={}", state_path.display());
        }
    }

    if let Some(profile_id) = config.launch_profile_id.as_deref() {
        let profile = profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .with_context(|| format!("unknown launch profile id: {profile_id}"))?;
        let launch_state = launch_state_for_profile(profile, &config)?;
        let state_path = write_launch_state(&launch_state)?;

        print_launch_state(&launch_state);
        println!("sessiond wrote_launch_state={}", state_path.display());
    }

    if config.launch_active {
        let profile = read_active_profile()?;
        let launch_state = launch_state_for_profile(&profile, &config)?;
        let state_path = write_launch_state(&launch_state)?;

        print_launch_state(&launch_state);
        println!("sessiond wrote_launch_state={}", state_path.display());
    }

    if config.apply_watchdog_active {
        let active_profile = read_active_profile()?;
        let report = read_watchdog_report(&config, &active_profile)?;
        let outcome = apply_watchdog_report(&active_profile, &profiles, &report)?;
        let _ = persist_watchdog_apply_outcome(&outcome, &config, None)?;
    }

    if config.serve_ipc {
        serve_ipc(&config, &profiles)?;
    }

    Ok(())
}

#[derive(Debug)]
struct Config {
    profiles_dir: Option<PathBuf>,
    list_profiles: bool,
    selected_profile_id: Option<String>,
    print_launch_plan: bool,
    write_selection: bool,
    launch_profile_id: Option<String>,
    launch_active: bool,
    spawn_components: bool,
    supervise_seconds: u64,
    restart_limit: u32,
    apply_watchdog_active: bool,
    watchdog_report_path: Option<PathBuf>,
    serve_ipc: bool,
    serve_once: bool,
    manage_active: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profiles_dir: None,
            list_profiles: false,
            selected_profile_id: None,
            print_launch_plan: false,
            write_selection: false,
            launch_profile_id: None,
            launch_active: false,
            spawn_components: false,
            supervise_seconds: 0,
            restart_limit: 2,
            apply_watchdog_active: false,
            watchdog_report_path: None,
            serve_ipc: false,
            serve_once: false,
            manage_active: false,
        }
    }
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--profiles-dir" => {
                    let dir = args.next().context("--profiles-dir requires a path")?;
                    config.profiles_dir = Some(PathBuf::from(dir));
                }
                "--list-profiles" => config.list_profiles = true,
                "--select-profile" => {
                    let id = args.next().context("--select-profile requires an id")?;
                    config.selected_profile_id = Some(id);
                }
                "--print-launch-plan" => config.print_launch_plan = true,
                "--write-selection" => config.write_selection = true,
                "--launch-profile" => {
                    let id = args.next().context("--launch-profile requires an id")?;
                    config.launch_profile_id = Some(id);
                }
                "--launch-active" => config.launch_active = true,
                "--spawn-components" => config.spawn_components = true,
                "--apply-watchdog-active" => config.apply_watchdog_active = true,
                "--watchdog-report" => {
                    let path = args.next().context("--watchdog-report requires a path")?;
                    config.watchdog_report_path = Some(PathBuf::from(path));
                }
                "--serve-ipc" => config.serve_ipc = true,
                "--once" => config.serve_once = true,
                "--manage-active" => config.manage_active = true,
                "--supervise-seconds" => {
                    let value = args.next().context("--supervise-seconds requires a number")?;
                    config.supervise_seconds = value
                        .parse()
                        .with_context(|| format!("invalid --supervise-seconds value: {value}"))?;
                }
                "--restart-limit" => {
                    let value = args.next().context("--restart-limit requires a number")?;
                    config.restart_limit = value
                        .parse()
                        .with_context(|| format!("invalid --restart-limit value: {value}"))?;
                }
                "--help" | "-h" => {
                    println!(
                        "usage: sessiond [--profiles-dir PATH] [--list-profiles] [--select-profile ID] [--print-launch-plan] [--write-selection] [--launch-profile ID] [--launch-active] [--spawn-components] [--supervise-seconds N] [--restart-limit N] [--apply-watchdog-active] [--watchdog-report PATH] [--serve-ipc] [--once] [--manage-active]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }

    fn profiles_dir(&self) -> PathBuf {
        self.profiles_dir.clone().unwrap_or_else(default_profiles_dir)
    }
}

fn default_profiles_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../profiles")
}

fn load_profiles(dir: &Path) -> Result<Vec<DesktopProfile>> {
    let mut profiles = Vec::new();
    let entries = fs::read_dir(dir)
        .with_context(|| format!("failed to read profiles dir {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read profile {}", path.display()))?;
        let profile: DesktopProfile = serde_json::from_str(&raw)
            .with_context(|| format!("failed to decode profile {}", path.display()))?;
        profiles.push(profile);
    }

    profiles.sort_by(|left, right| left.id.cmp(&right.id));

    if profiles.is_empty() {
        bail!("no desktop profiles found in {}", dir.display());
    }

    Ok(profiles)
}

fn write_active_profile(profile: &DesktopProfile) -> Result<PathBuf> {
    let dir = ensure_runtime_dir()?;
    let path = dir.join("active-profile.json");
    let json =
        serde_json::to_string_pretty(profile).context("failed to serialize active profile")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn active_profile_path() -> PathBuf {
    runtime_dir().join("active-profile.json")
}

fn launch_state_path(profile_id: &str) -> PathBuf {
    runtime_dir().join(format!("launch-state-{profile_id}.json"))
}

fn watchdog_report_path(profile_id: &str) -> PathBuf {
    runtime_dir().join(format!("watchdog-report-{profile_id}.json"))
}

fn read_active_profile() -> Result<DesktopProfile> {
    let path = active_profile_path();
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read active profile {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to decode active profile {}", path.display()))
}

fn read_watchdog_report(
    config: &Config,
    active_profile: &DesktopProfile,
) -> Result<SessionWatchdogReport> {
    let path = config
        .watchdog_report_path
        .clone()
        .unwrap_or_else(|| watchdog_report_path(&active_profile.id));
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read watchdog report {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to decode watchdog report {}", path.display()))
}

fn serve_ipc(config: &Config, profiles: &[DesktopProfile]) -> Result<()> {
    let (listener, socket_path) = bind_service_socket(ServiceRole::Sessiond)?;
    let _socket_guard = SocketGuard::new(socket_path.clone());
    listener
        .set_nonblocking(true)
        .with_context(|| format!("failed to set nonblocking mode on {}", socket_path.display()))?;
    println!("sessiond listening socket={}", socket_path.display());

    let mut supervisor =
        if config.manage_active { Some(SessionSupervisor::bootstrap(config)?) } else { None };

    let mut served = 0usize;
    loop {
        if let Some(supervisor) = supervisor.as_mut() {
            let _ = supervisor.poll(config)?;
        }

        match listener.accept() {
            Ok((stream, _addr)) => {
                handle_client(stream, profiles, config, supervisor.as_mut())?;
                served += 1;

                if config.serve_once {
                    break;
                }
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(200));
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to accept on {}", socket_path.display()));
            }
        }
    }

    println!("sessiond served_requests={served}");
    Ok(())
}

fn handle_client(
    mut stream: UnixStream,
    profiles: &[DesktopProfile],
    config: &Config,
    supervisor: Option<&mut SessionSupervisor>,
) -> Result<()> {
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let response = build_response(request, profiles, config, supervisor);
    send_json_line(&mut stream, &response)?;
    Ok(())
}

fn build_response(
    request: IpcEnvelope,
    profiles: &[DesktopProfile],
    config: &Config,
    supervisor: Option<&mut SessionSupervisor>,
) -> IpcEnvelope {
    let source = request.source;
    let response_kind = match request.kind {
        MessageKind::SessionCommand(command) if request.destination == ServiceRole::Sessiond => {
            match handle_session_command(command, profiles, config, supervisor) {
                Ok(command) => MessageKind::SessionCommand(command),
                Err(err) => MessageKind::SessionCommand(SessionCommand::ProfileUnchanged {
                    profile_id: "unknown".into(),
                    reason: format!("sessiond rejected watchdog apply request: {err:#}"),
                }),
            }
        }
        MessageKind::SessionCommand(_) => {
            MessageKind::SessionCommand(SessionCommand::ProfileUnchanged {
                profile_id: "unknown".into(),
                reason: format!(
                    "sessiond received message addressed to {}",
                    request.destination.as_str()
                ),
            })
        }
        other => MessageKind::SessionCommand(SessionCommand::ProfileUnchanged {
            profile_id: "unknown".into(),
            reason: format!("sessiond does not handle {other:?}"),
        }),
    };

    IpcEnvelope::new(ServiceRole::Sessiond, source, response_kind)
}

fn handle_session_command(
    command: SessionCommand,
    profiles: &[DesktopProfile],
    config: &Config,
    supervisor: Option<&mut SessionSupervisor>,
) -> Result<SessionCommand> {
    match command {
        SessionCommand::ApplyWatchdogReport { report } => {
            let active_profile = match supervisor.as_ref() {
                Some(supervisor) => supervisor.profile(),
                None => read_active_profile()?,
            };
            let outcome = apply_watchdog_report(&active_profile, profiles, &report)?;
            persist_watchdog_apply_outcome(&outcome, config, supervisor)
        }
        other => Ok(SessionCommand::ProfileUnchanged {
            profile_id: "unknown".into(),
            reason: format!("sessiond IPC does not apply {other:?}"),
        }),
    }
}

fn launch_state_for_profile(
    profile: &DesktopProfile,
    config: &Config,
) -> Result<SessionLaunchState> {
    if config.supervise_seconds > 0 {
        return supervise_launch_state(profile, config.supervise_seconds, config.restart_limit);
    }

    build_launch_state(profile, config.spawn_components)
}

fn build_launch_state(
    profile: &DesktopProfile,
    spawn_components: bool,
) -> Result<SessionLaunchState> {
    let mut components = Vec::with_capacity(profile.session_components.len());

    for component in &profile.session_components {
        components.push(resolve_component_state(component, &profile.id, spawn_components)?);
    }

    Ok(SessionLaunchState {
        profile_id: profile.id.clone(),
        display_name: profile.display_name.clone(),
        protocol: profile.protocol,
        broker_services: profile.broker_services.clone(),
        components,
    })
}

fn supervise_launch_state(
    profile: &DesktopProfile,
    supervise_seconds: u64,
    restart_limit: u32,
) -> Result<SessionLaunchState> {
    let mut components = Vec::with_capacity(profile.session_components.len());

    for component in &profile.session_components {
        components.push(RuntimeComponent::new(component)?);
    }

    for component in &mut components {
        component.spawn(&profile.id)?;
    }

    let deadline = Instant::now() + Duration::from_secs(supervise_seconds);
    while Instant::now() < deadline {
        let mut had_event = false;

        for component in &mut components {
            if component.poll_and_restart(&profile.id, restart_limit)? {
                had_event = true;
            }
        }

        if !had_event {
            thread::sleep(Duration::from_millis(200));
        }
    }

    Ok(SessionLaunchState {
        profile_id: profile.id.clone(),
        display_name: profile.display_name.clone(),
        protocol: profile.protocol,
        broker_services: profile.broker_services.clone(),
        components: components.into_iter().map(|component| component.state).collect(),
    })
}

fn resolve_component_state(
    component: &DesktopComponent,
    profile_id: &str,
    spawn_components: bool,
) -> Result<SessionLaunchComponentState> {
    let mut state = base_component_state(component);

    if spawn_components {
        if let Some(command_path) = state.resolved_command.as_ref() {
            let child = Command::new(command_path)
                .args(component.command.iter().skip(1))
                .env("WAYBROKER_PROFILE_ID", profile_id)
                .env("WAYBROKER_COMPONENT_ID", &component.id)
                .spawn();

            match child {
                Ok(child) => {
                    state.state = DesktopComponentState::Spawned;
                    state.pid = Some(child.id());
                }
                Err(_) => {
                    state.state = DesktopComponentState::Failed;
                    state.last_exit_status = Some(-1);
                }
            }
        }
    }

    Ok(state)
}

fn base_component_state(component: &DesktopComponent) -> SessionLaunchComponentState {
    let resolved_command =
        resolve_command_path(&component.command).map(|path| path.display().to_string());
    let state = if resolved_command.is_some() {
        DesktopComponentState::Ready
    } else {
        DesktopComponentState::Missing
    };

    SessionLaunchComponentState {
        id: component.id.clone(),
        role: component.role,
        critical: component.critical,
        command: component.command.clone(),
        resolved_command,
        state,
        pid: None,
        restart_count: 0,
        last_exit_status: None,
    }
}

fn resolve_command_path(command: &[String]) -> Option<PathBuf> {
    let executable = command.first()?;
    let candidate = PathBuf::from(executable);

    if candidate.components().count() > 1 {
        return is_executable(&candidate).then_some(candidate);
    }

    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        let candidate = dir.join(executable);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn is_executable(path: &Path) -> bool {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };

    metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0)
}

fn write_launch_state(state: &SessionLaunchState) -> Result<PathBuf> {
    let dir = ensure_runtime_dir()?;
    let path = dir.join(
        launch_state_path(&state.profile_id)
            .file_name()
            .expect("launch state path should have a file name"),
    );
    let json = serde_json::to_string_pretty(state).context("failed to serialize launch state")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn write_profile_transition(transition: &SessionProfileTransition) -> Result<PathBuf> {
    let dir = ensure_runtime_dir()?;
    let path = dir.join(format!(
        "profile-transition-{}-to-{}.json",
        transition.source_profile_id, transition.target_profile_id
    ));
    let json = serde_json::to_string_pretty(transition)
        .context("failed to serialize profile transition")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn print_launch_state(state: &SessionLaunchState) {
    println!(
        "sessiond launch_state profile={} protocol={} components={}",
        state.profile_id,
        state.protocol.as_str(),
        state.components.len()
    );

    for component in &state.components {
        let resolved = component.resolved_command.as_deref().unwrap_or("missing");
        println!(
            "sessiond launch component id={} role={} critical={} state={} resolved={} pid={} restarts={} last_exit={}",
            component.id,
            component.role.as_str(),
            component.critical,
            component.state.as_str(),
            resolved,
            component.pid.map(|pid| pid.to_string()).as_deref().unwrap_or("none"),
            component.restart_count,
            component
                .last_exit_status
                .map(|status| status.to_string())
                .as_deref()
                .unwrap_or("none")
        );
    }
}

fn print_profile_transition(transition: &SessionProfileTransition) {
    println!(
        "sessiond profile_transition from={} to={} reason={} triggers={}",
        transition.source_profile_id,
        transition.target_profile_id,
        transition.reason,
        transition.trigger_component_ids.join(",")
    );
}

fn apply_watchdog_report(
    active_profile: &DesktopProfile,
    profiles: &[DesktopProfile],
    report: &SessionWatchdogReport,
) -> Result<WatchdogApplyOutcome> {
    if report.profile_id != active_profile.id {
        bail!(
            "watchdog report profile {} does not match active profile {}",
            report.profile_id,
            active_profile.id
        );
    }

    let trigger_component_ids = degraded_trigger_component_ids(report);

    if trigger_component_ids.is_empty() {
        return Ok(WatchdogApplyOutcome::Unchanged {
            profile_id: active_profile.id.clone(),
            reason: "watchdog report did not request degraded profile switch".into(),
        });
    }

    let Some(target_profile_id) = active_profile.degraded_profile_id.as_deref() else {
        return Ok(WatchdogApplyOutcome::Unchanged {
            profile_id: active_profile.id.clone(),
            reason: format!("active profile {} has no degraded fallback", active_profile.id),
        });
    };

    if target_profile_id == active_profile.id {
        bail!("active profile {} cannot degrade to itself", active_profile.id);
    }

    let target_profile = profiles
        .iter()
        .find(|profile| profile.id == target_profile_id)
        .with_context(|| format!("unknown degraded profile id: {target_profile_id}"))?
        .clone();

    let transition = SessionProfileTransition {
        source_profile_id: active_profile.id.clone(),
        source_display_name: active_profile.display_name.clone(),
        target_profile_id: target_profile.id.clone(),
        target_display_name: target_profile.display_name.clone(),
        reason: "watchdog requested degraded profile switch".into(),
        trigger_component_ids,
    };

    Ok(WatchdogApplyOutcome::Transition { target_profile, transition })
}

fn degraded_trigger_component_ids(report: &SessionWatchdogReport) -> Vec<String> {
    report
        .components
        .iter()
        .filter(|component| component.action == DesktopRecoveryAction::DegradedProfile)
        .map(|component| component.id.clone())
        .collect()
}

fn persist_watchdog_apply_outcome(
    outcome: &WatchdogApplyOutcome,
    config: &Config,
    supervisor: Option<&mut SessionSupervisor>,
) -> Result<SessionCommand> {
    match outcome {
        WatchdogApplyOutcome::Transition { target_profile, transition } => {
            let active_path = write_active_profile(target_profile)?;
            let transition_path = write_profile_transition(transition)?;

            print_profile_transition(transition);
            println!("sessiond wrote_active_profile={}", active_path.display());
            println!("sessiond wrote_profile_transition={}", transition_path.display());

            if let Some(supervisor) = supervisor {
                supervisor.switch_to(target_profile.clone(), config)?;
            } else if should_auto_launch_transition(config) {
                let launch_state = launch_state_for_profile(target_profile, config)?;
                let state_path = write_launch_state(&launch_state)?;
                print_launch_state(&launch_state);
                println!("sessiond auto_launched_profile={}", target_profile.id);
                println!("sessiond wrote_launch_state={}", state_path.display());
            }

            Ok(SessionCommand::ProfileTransition { transition: transition.clone() })
        }
        WatchdogApplyOutcome::Unchanged { profile_id, reason } => {
            println!(
                "sessiond watchdog_action=none active_profile={} reason={}",
                profile_id, reason
            );
            Ok(SessionCommand::ProfileUnchanged {
                profile_id: profile_id.clone(),
                reason: reason.clone(),
            })
        }
    }
}

fn should_auto_launch_transition(config: &Config) -> bool {
    config.spawn_components || config.supervise_seconds > 0
}

enum WatchdogApplyOutcome {
    Transition { target_profile: DesktopProfile, transition: SessionProfileTransition },
    Unchanged { profile_id: String, reason: String },
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

struct SessionSupervisor {
    profile: DesktopProfile,
    components: Vec<RuntimeComponent>,
}

impl SessionSupervisor {
    fn bootstrap(config: &Config) -> Result<Self> {
        let profile = read_active_profile()?;
        let mut supervisor = Self::new(profile)?;
        supervisor.activate(config)?;
        Ok(supervisor)
    }

    fn new(profile: DesktopProfile) -> Result<Self> {
        let mut components = Vec::with_capacity(profile.session_components.len());
        for component in &profile.session_components {
            components.push(RuntimeComponent::new(component)?);
        }

        Ok(Self { profile, components })
    }

    fn profile(&self) -> DesktopProfile {
        self.profile.clone()
    }

    fn activate(&mut self, config: &Config) -> Result<()> {
        if should_auto_launch_transition(config) {
            for component in &mut self.components {
                component.spawn(&self.profile.id)?;
            }
        }

        self.write_snapshot("managed_active_profile")
    }

    fn switch_to(&mut self, profile: DesktopProfile, config: &Config) -> Result<()> {
        self.stop_all()?;
        *self = Self::new(profile)?;
        self.activate(config)?;
        println!("sessiond auto_launched_profile={}", self.profile.id);
        Ok(())
    }

    fn poll(&mut self, config: &Config) -> Result<bool> {
        let mut had_event = false;

        for component in &mut self.components {
            if component.poll_and_restart(&self.profile.id, config.restart_limit)? {
                had_event = true;
            }
        }

        if had_event {
            self.write_snapshot("managed_runtime_update")?;
        }

        Ok(had_event)
    }

    fn stop_all(&mut self) -> Result<()> {
        for component in &mut self.components {
            component.stop()?;
        }

        Ok(())
    }

    fn snapshot(&self) -> SessionLaunchState {
        SessionLaunchState {
            profile_id: self.profile.id.clone(),
            display_name: self.profile.display_name.clone(),
            protocol: self.profile.protocol,
            broker_services: self.profile.broker_services.clone(),
            components: self.components.iter().map(|component| component.state.clone()).collect(),
        }
    }

    fn write_snapshot(&self, label: &str) -> Result<()> {
        let launch_state = self.snapshot();
        let state_path = write_launch_state(&launch_state)?;
        print_launch_state(&launch_state);
        println!("sessiond {}={}", label, self.profile.id);
        println!("sessiond wrote_launch_state={}", state_path.display());
        Ok(())
    }
}

struct RuntimeComponent {
    component: DesktopComponent,
    state: SessionLaunchComponentState,
    child: Option<Child>,
}

impl RuntimeComponent {
    fn new(component: &DesktopComponent) -> Result<Self> {
        Ok(Self {
            component: component.clone(),
            state: base_component_state(component),
            child: None,
        })
    }

    fn spawn(&mut self, profile_id: &str) -> Result<()> {
        let Some(command_path) = self.state.resolved_command.as_ref() else {
            self.state.state = DesktopComponentState::Missing;
            self.state.pid = None;
            return Ok(());
        };

        let child = Command::new(command_path)
            .args(self.component.command.iter().skip(1))
            .env("WAYBROKER_PROFILE_ID", profile_id)
            .env("WAYBROKER_COMPONENT_ID", &self.component.id)
            .spawn();

        match child {
            Ok(child) => {
                self.state.state = DesktopComponentState::Spawned;
                self.state.pid = Some(child.id());
                self.child = Some(child);
            }
            Err(_) => {
                self.state.state = DesktopComponentState::Failed;
                self.state.pid = None;
                self.state.last_exit_status = Some(-1);
                self.child = None;
            }
        }

        Ok(())
    }

    fn poll_and_restart(&mut self, profile_id: &str, restart_limit: u32) -> Result<bool> {
        let Some(child) = self.child.as_mut() else {
            return Ok(false);
        };

        let Some(status) = child.try_wait()? else {
            return Ok(false);
        };

        self.state.pid = None;
        self.state.last_exit_status = status.code();
        self.state.restart_count += 1;
        self.child = None;

        if self.component.critical && self.state.restart_count <= restart_limit {
            self.spawn(profile_id)?;
        } else {
            self.state.state = DesktopComponentState::Failed;
        }

        Ok(true)
    }

    fn stop(&mut self) -> Result<()> {
        let Some(mut child) = self.child.take() else {
            self.state.pid = None;
            return Ok(());
        };

        let _ = child.kill();
        let _ = child.wait();
        self.state.pid = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_watchdog_report, build_launch_state, is_executable, resolve_command_path};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };
    use waybroker_common::{
        DesktopComponent, DesktopComponentRole, DesktopHealthStatus, DesktopProfile,
        DesktopProtocol, DesktopRecoveryAction, ServiceRole, SessionWatchdogComponentReport,
        SessionWatchdogReport,
    };

    #[test]
    fn resolves_absolute_executable_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sessiond-test-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).expect("time").as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("create temp dir");

        let executable = temp_dir.join("component");
        fs::write(&executable, "#!/bin/sh\nexit 0\n").expect("write executable");
        let mut permissions = fs::metadata(&executable).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("chmod");

        assert!(is_executable(&executable));
        assert_eq!(
            resolve_command_path(&[executable.display().to_string()]).as_deref(),
            Some(executable.as_path())
        );

        let _ = fs::remove_file(&executable);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn builds_missing_launch_state_for_unknown_command() {
        let profile = DesktopProfile {
            id: "test-x11".into(),
            display_name: "Test".into(),
            protocol: DesktopProtocol::LayerX11,
            summary: "test".into(),
            degraded_profile_id: None,
            broker_services: vec![ServiceRole::Sessiond],
            session_components: vec![DesktopComponent {
                id: "missing".into(),
                role: DesktopComponentRole::Shell,
                command: vec!["definitely-not-a-real-command-waybroker".into()],
                critical: true,
            }],
        };

        let state = build_launch_state(&profile, false).expect("build launch state");

        assert_eq!(state.components.len(), 1);
        assert_eq!(state.components[0].state, waybroker_common::DesktopComponentState::Missing);
        assert_eq!(state.components[0].restart_count, 0);
    }

    #[test]
    fn evaluates_degraded_profile_transition_from_watchdog_report() {
        let active_profile = DesktopProfile {
            id: "demo-x11-crashy".into(),
            display_name: "Crashy Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            summary: "crash test".into(),
            degraded_profile_id: Some("demo-x11-degraded".into()),
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            session_components: Vec::new(),
        };
        let degraded_profile = DesktopProfile {
            id: "demo-x11-degraded".into(),
            display_name: "Degraded Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            summary: "fallback".into(),
            degraded_profile_id: None,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            session_components: Vec::new(),
        };
        let report = SessionWatchdogReport {
            profile_id: "demo-x11-crashy".into(),
            display_name: "Crashy Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            healthy_components: 1,
            unhealthy_components: 1,
            inactive_components: 0,
            components: vec![SessionWatchdogComponentReport {
                id: "crashy-wm".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                status: DesktopHealthStatus::Unhealthy,
                pid: None,
                crash_loop_count: 3,
                action: DesktopRecoveryAction::DegradedProfile,
                reason: "component spawn failed or supervisor gave up".into(),
            }],
        };

        let outcome = apply_watchdog_report(
            &active_profile,
            &[active_profile.clone(), degraded_profile.clone()],
            &report,
        )
        .expect("evaluate transition");

        match outcome {
            super::WatchdogApplyOutcome::Transition { target_profile, transition } => {
                assert_eq!(target_profile.id, "demo-x11-degraded");
                assert_eq!(transition.source_profile_id, "demo-x11-crashy");
                assert_eq!(transition.target_profile_id, "demo-x11-degraded");
                assert_eq!(transition.trigger_component_ids, vec!["crashy-wm".to_string()]);
            }
            super::WatchdogApplyOutcome::Unchanged { .. } => {
                panic!("transition should be present");
            }
        }
    }

    #[test]
    fn ignores_watchdog_transition_when_no_degraded_action_exists() {
        let active_profile = DesktopProfile {
            id: "demo-x11".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            summary: "demo".into(),
            degraded_profile_id: Some("demo-x11-degraded".into()),
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            session_components: Vec::new(),
        };
        let report = SessionWatchdogReport {
            profile_id: "demo-x11".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            healthy_components: 1,
            unhealthy_components: 0,
            inactive_components: 0,
            components: vec![SessionWatchdogComponentReport {
                id: "demo-wm".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                status: DesktopHealthStatus::Healthy,
                pid: Some(42),
                crash_loop_count: 0,
                action: DesktopRecoveryAction::None,
                reason: "component process is alive".into(),
            }],
        };

        let outcome = apply_watchdog_report(&active_profile, &[active_profile.clone()], &report)
            .expect("evaluate transition");

        match outcome {
            super::WatchdogApplyOutcome::Transition { .. } => {
                panic!("transition should not be present");
            }
            super::WatchdogApplyOutcome::Unchanged { profile_id, reason } => {
                assert_eq!(profile_id, "demo-x11");
                assert_eq!(reason, "watchdog report did not request degraded profile switch");
            }
        }
    }
}
