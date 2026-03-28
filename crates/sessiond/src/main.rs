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
    SessionLaunchDelta, SessionLaunchState, SessionProfileTransition, SessionWatchdogReport,
    WatchdogCommand, bind_service_socket, connect_service_socket, ensure_runtime_dir,
    now_unix_timestamp, read_json_line, runtime_dir, send_json_line,
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
    println!(
        "service=sessiond op=load_profiles dir={} count={}",
        profiles_dir.display(),
        profiles.len()
    );

    if config.list_profiles || config.selected_profile_id.is_none() {
        for profile in &profiles {
            println!(
                "service=sessiond op=profile_entry id={} protocol={} name=\"{}\" summary=\"{}\"",
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
            "service=sessiond op=select_profile id={} protocol={} components={}",
            profile.id,
            profile.protocol.as_str(),
            profile.session_components.len()
        );

        if config.print_launch_plan {
            for service in &plan.broker_services {
                println!("service=sessiond op=broker_service id={}", service.as_str());
            }

            for component in &plan.session_components {
                println!(
                    "service=sessiond op=component_entry id={} role={:?} critical={} command=\"{}\"",
                    component.id,
                    component.role,
                    component.critical,
                    component.command.join(" ")
                );
            }
        }

        if config.write_selection {
            let state_path = write_active_profile(profile)?;
            println!("service=sessiond op=write_active_profile path={}", state_path.display());
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
        println!("service=sessiond op=write_launch_state path={}", state_path.display());
    }

    if config.launch_active {
        let profile = read_active_profile()?;
        let launch_state = launch_state_for_profile(&profile, &config)?;
        let state_path = write_launch_state(&launch_state)?;

        print_launch_state(&launch_state);
        println!("service=sessiond op=write_launch_state path={}", state_path.display());
    }

    if config.apply_watchdog_active {
        let active_profile = read_active_profile()?;
        let report = read_watchdog_report(&config, &active_profile)?;
        let outcome = apply_watchdog_report(&active_profile, &profiles, &report)?;
        let _ = persist_watchdog_apply_outcome(&outcome, &config, None)?;
    }

    if config.resume_demo {
        run_resume_scenario(&config, ResumeScenario::Normal)?;
    }

    if let Some(scenario) = config.resume_scenario {
        run_resume_scenario(&config, scenario)?;
    }

    if config.serve_ipc {
        serve_ipc(&config, &profiles)?;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResumeScenario {
    Normal,
    DisplaydTrouble,
    CompdTrouble,
    LockdTrouble,
}

impl ResumeScenario {
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "normal" => Ok(Self::Normal),
            "displayd-trouble" => Ok(Self::DisplaydTrouble),
            "compd-trouble" => Ok(Self::CompdTrouble),
            "lockd-trouble" => Ok(Self::LockdTrouble),
            _ => bail!("unknown resume scenario: {s}"),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::DisplaydTrouble => "displayd-trouble",
            Self::CompdTrouble => "compd-trouble",
            Self::LockdTrouble => "lockd-trouble",
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct ResumeTrace {
    scenario: String,
    unix_timestamp: u64,
    steps: Vec<ResumeStep>,
    final_state: String,
}

#[derive(Debug, serde::Serialize)]
struct ResumeStep {
    name: String,
    service: String,
    outcome: String,
    detail: Option<String>,
}

#[derive(Debug)]
struct Config {
    repo_root: Option<PathBuf>,
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
    notify_watchdog: bool,
    resume_demo: bool,
    resume_scenario: Option<ResumeScenario>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            repo_root: None,
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
            notify_watchdog: false,
            resume_demo: false,
            resume_scenario: None,
        }
    }
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--repo-root" => {
                    let dir = args.next().context("--repo-root requires a path")?;
                    config.repo_root = Some(PathBuf::from(dir));
                }
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
                "--notify-watchdog" => config.notify_watchdog = true,
                "--resume-demo" => config.resume_demo = true,
                "--resume-scenario" => {
                    let scenario = args.next().context("--resume-scenario requires a name")?;
                    config.resume_scenario = Some(ResumeScenario::from_str(&scenario)?);
                }
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
                        "usage: sessiond [--repo-root PATH] [--profiles-dir PATH] [--list-profiles] [--select-profile ID] [--print-launch-plan] [--write-selection] [--launch-profile ID] [--launch-active] [--spawn-components] [--supervise-seconds N] [--restart-limit N] [--apply-watchdog-active] [--watchdog-report PATH] [--serve-ipc] [--once] [--manage-active] [--notify-watchdog] [--resume-demo] [--resume-scenario NAME]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }

    fn repo_root(&self) -> PathBuf {
        self.repo_root.clone().unwrap_or_else(default_repo_root)
    }

    fn profiles_dir(&self) -> PathBuf {
        self.profiles_dir.clone().unwrap_or_else(|| self.repo_root().join("profiles"))
    }
}

fn default_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
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

fn run_resume_scenario(_config: &Config, scenario: ResumeScenario) -> Result<()> {
    println!("service=sessiond op=resume_sequence event=begin scenario={}", scenario.as_str());

    let mut steps = Vec::new();
    let mut final_state = "normal".to_string();

    // 1. sessiond -> displayd (ResumeBegin)
    let displayd_res = send_ipc_and_wait(
        ServiceRole::Displayd,
        MessageKind::DisplayCommand(waybroker_common::DisplayCommand::ResumeBegin),
    );

    match displayd_res {
        Ok(envelope) => {
            if let MessageKind::DisplayEvent(waybroker_common::DisplayEvent::ResumeStarted) =
                envelope.kind
            {
                println!("service=sessiond op=resume_sequence event=displayd_started");
                steps.push(ResumeStep {
                    name: "resume_begin".into(),
                    service: "displayd".into(),
                    outcome: "success".into(),
                    detail: None,
                });
            } else {
                println!(
                    "service=sessiond op=resume_sequence event=displayd_failed detail={:?}",
                    envelope.kind
                );
                steps.push(ResumeStep {
                    name: "resume_begin".into(),
                    service: "displayd".into(),
                    outcome: "failed".into(),
                    detail: Some(format!("{:?}", envelope.kind)),
                });
                final_state = "hold".into();
            }
        }
        Err(err) => {
            println!(
                "service=sessiond op=resume_sequence event=displayd_unreachable reason=\"{err}\""
            );
            steps.push(ResumeStep {
                name: "resume_begin".into(),
                service: "displayd".into(),
                outcome: "unreachable".into(),
                detail: Some(err.to_string()),
            });
            final_state = "hold".into();
        }
    }

    if final_state == "normal" {
        // 2. sessiond -> lockd (SetLockState Locked)
        let lockd_res = send_ipc_and_wait(
            ServiceRole::Lockd,
            MessageKind::LockCommand(waybroker_common::LockCommand::SetLockState {
                state: waybroker_common::LockState::Locked,
            }),
        );

        match lockd_res {
            Ok(envelope) => {
                if let MessageKind::LockCommand(waybroker_common::LockCommand::SetLockState {
                    state: waybroker_common::LockState::Locked,
                }) = envelope.kind
                {
                    println!("service=sessiond op=resume_sequence event=lockd_locked");
                    steps.push(ResumeStep {
                        name: "set_lock_state".into(),
                        service: "lockd".into(),
                        outcome: "success".into(),
                        detail: None,
                    });
                } else {
                    println!(
                        "service=sessiond op=resume_sequence event=lockd_failed detail={:?}",
                        envelope.kind
                    );
                    steps.push(ResumeStep {
                        name: "set_lock_state".into(),
                        service: "lockd".into(),
                        outcome: "failed".into(),
                        detail: Some(format!("{:?}", envelope.kind)),
                    });
                    final_state = "blank-only".into();
                }
            }
            Err(err) => {
                println!(
                    "service=sessiond op=resume_sequence event=lockd_unreachable reason=\"{err}\""
                );
                steps.push(ResumeStep {
                    name: "set_lock_state".into(),
                    service: "lockd".into(),
                    outcome: "unreachable".into(),
                    detail: Some(err.to_string()),
                });
                final_state = "blank-only".into();
            }
        }
    }

    if final_state == "normal" {
        // 3. sessiond -> compd (ResumeHint OutputsRecovered)
        let compd_res = send_ipc_and_wait(
            ServiceRole::Compd,
            MessageKind::SessionCommand(waybroker_common::SessionCommand::ResumeHint {
                stage: waybroker_common::ResumeStage::OutputsRecovered,
                output: Some(waybroker_common::OutputMode {
                    name: "eDP-1".into(),
                    width: 1920,
                    height: 1080,
                    refresh_hz: 60,
                }),
            }),
        );

        match compd_res {
            Ok(envelope) => {
                if let MessageKind::SessionCommand(waybroker_common::SessionCommand::ResumeHint {
                    ..
                }) = envelope.kind
                {
                    println!("service=sessiond op=resume_sequence event=compd_outputs_recovered");
                    steps.push(ResumeStep {
                        name: "resume_hint_outputs".into(),
                        service: "compd".into(),
                        outcome: "success".into(),
                        detail: None,
                    });
                } else {
                    println!(
                        "service=sessiond op=resume_sequence event=compd_failed detail={:?}",
                        envelope.kind
                    );
                    steps.push(ResumeStep {
                        name: "resume_hint_outputs".into(),
                        service: "compd".into(),
                        outcome: "failed".into(),
                        detail: Some(format!("{:?}", envelope.kind)),
                    });
                    final_state = "restart-request".into();
                }
            }
            Err(err) => {
                println!(
                    "service=sessiond op=resume_sequence event=compd_unreachable reason=\"{err}\""
                );
                steps.push(ResumeStep {
                    name: "resume_hint_outputs".into(),
                    service: "compd".into(),
                    outcome: "unreachable".into(),
                    detail: Some(err.to_string()),
                });
                final_state = "restart-request".into();
            }
        }
    }

    if final_state == "normal" {
        // 4. sessiond -> lockd (AuthPrompt)
        let lockd_res = send_ipc_and_wait(
            ServiceRole::Lockd,
            MessageKind::LockCommand(waybroker_common::LockCommand::AuthPrompt {
                reason: "resume auth required".into(),
            }),
        );

        match lockd_res {
            Ok(envelope) => {
                if let MessageKind::LockCommand(waybroker_common::LockCommand::AuthPrompt {
                    reason,
                }) = envelope.kind
                {
                    if reason == "fault injection" {
                        println!("service=sessiond op=resume_sequence event=lockd_auth_failed");
                        steps.push(ResumeStep {
                            name: "auth_prompt".into(),
                            service: "lockd".into(),
                            outcome: "failed".into(),
                            detail: Some("auth prompt fault injection".into()),
                        });
                        final_state = "blank-only".into();
                    } else {
                        println!("service=sessiond op=resume_sequence event=lockd_auth_prompt");
                        steps.push(ResumeStep {
                            name: "auth_prompt".into(),
                            service: "lockd".into(),
                            outcome: "success".into(),
                            detail: None,
                        });
                    }
                } else {
                    println!(
                        "service=sessiond op=resume_sequence event=lockd_auth_failed detail={:?}",
                        envelope.kind
                    );
                    steps.push(ResumeStep {
                        name: "auth_prompt".into(),
                        service: "lockd".into(),
                        outcome: "failed".into(),
                        detail: Some(format!("{:?}", envelope.kind)),
                    });
                    final_state = "blank-only".into();
                }
            }
            Err(err) => {
                println!(
                    "service=sessiond op=resume_sequence event=lockd_auth_unreachable reason=\"{err}\""
                );
                steps.push(ResumeStep {
                    name: "auth_prompt".into(),
                    service: "lockd".into(),
                    outcome: "unreachable".into(),
                    detail: Some(err.to_string()),
                });
                final_state = "blank-only".into();
            }
        }
    }

    if final_state == "normal" {
        // 5. sessiond -> compd (ResumeHint Complete)
        let _ = send_ipc_and_wait(
            ServiceRole::Compd,
            MessageKind::SessionCommand(waybroker_common::SessionCommand::ResumeHint {
                stage: waybroker_common::ResumeStage::Complete,
                output: None,
            }),
        );
        println!("service=sessiond op=resume_sequence event=compd_complete");
        steps.push(ResumeStep {
            name: "resume_hint_complete".into(),
            service: "compd".into(),
            outcome: "success".into(),
            detail: None,
        });
    }

    if final_state == "restart-request" {
        println!("service=sessiond op=resume_sequence event=watchdog_restart_request role=compd");
        let res = send_watchdog_command(WatchdogCommand::Restart {
            role: ServiceRole::Compd,
            reason: "resume failure (restart-request)".into(),
        });

        match res {
            Ok(envelope) => {
                if let MessageKind::WatchdogCommand(WatchdogCommand::Restart { .. }) = envelope.kind
                {
                    steps.push(ResumeStep {
                        name: "watchdog_restart_request".into(),
                        service: "watchdog".into(),
                        outcome: "accepted".into(),
                        detail: None,
                    });
                } else {
                    steps.push(ResumeStep {
                        name: "watchdog_restart_request".into(),
                        service: "watchdog".into(),
                        outcome: "rejected".into(),
                        detail: Some(format!("{:?}", envelope.kind)),
                    });
                }
            }
            Err(err) => {
                steps.push(ResumeStep {
                    name: "watchdog_restart_request".into(),
                    service: "watchdog".into(),
                    outcome: "unreachable".into(),
                    detail: Some(err.to_string()),
                });
            }
        }
    }

    println!("service=sessiond op=resume_sequence event=finished final_state={}", final_state);

    let trace = ResumeTrace {
        scenario: scenario.as_str().into(),
        unix_timestamp: now_unix_timestamp(),
        steps,
        final_state,
    };

    write_resume_trace(&trace)?;

    Ok(())
}

fn write_resume_trace(trace: &ResumeTrace) -> Result<PathBuf> {
    let dir = ensure_runtime_dir()?;
    let path = dir.join(format!("resume-trace-{}.json", trace.scenario));
    let json = serde_json::to_string_pretty(trace).context("failed to serialize resume trace")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    println!("service=sessiond op=write_resume_trace event=success path={}", path.display());
    Ok(path)
}

fn send_ipc_and_wait(destination: ServiceRole, kind: MessageKind) -> Result<IpcEnvelope> {
    let mut stream = connect_service_socket(destination)?;
    let request = IpcEnvelope::new(ServiceRole::Sessiond, destination, kind);
    send_json_line(&mut stream, &request)?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_json_line(&mut reader)?;
    Ok(response)
}

fn serve_ipc(config: &Config, profiles: &[DesktopProfile]) -> Result<()> {
    let (listener, socket_path) = bind_service_socket(ServiceRole::Sessiond)?;
    let _socket_guard = SocketGuard::new(socket_path.clone());
    listener
        .set_nonblocking(true)
        .with_context(|| format!("failed to set nonblocking mode on {}", socket_path.display()))?;
    println!("sessiond listening socket={}", socket_path.display());

    let mut supervisor = if config.manage_active {
        Some(SessionSupervisor::bootstrap(config, profiles)?)
    } else {
        None
    };

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
        return supervise_launch_state(profile, config);
    }

    build_launch_state(profile, config)
}

fn build_launch_state(profile: &DesktopProfile, config: &Config) -> Result<SessionLaunchState> {
    let mut components = Vec::with_capacity(profile.session_components.len());

    for component in &profile.session_components {
        components.push(resolve_component_state(component, &profile.id, config)?);
    }

    Ok(SessionLaunchState {
        profile_id: profile.id.clone(),
        display_name: profile.display_name.clone(),
        protocol: profile.protocol,
        broker_services: profile.broker_services.clone(),
        generation: 1,
        sequence: 1,
        components,
        unix_timestamp: now_unix_timestamp(),
        service_component_bindings: profile.service_component_bindings.clone(),
    })
}

fn supervise_launch_state(profile: &DesktopProfile, config: &Config) -> Result<SessionLaunchState> {
    let mut components = Vec::with_capacity(profile.session_components.len());

    for component in &profile.session_components {
        components.push(RuntimeComponent::new(component, config)?);
    }

    for component in &mut components {
        component.spawn(&profile.id)?;
    }

    let deadline = Instant::now() + Duration::from_secs(config.supervise_seconds);
    while Instant::now() < deadline {
        let mut had_event = false;

        for component in &mut components {
            if component.poll_and_restart(&profile.id, config.restart_limit)? {
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
        generation: 1,
        sequence: 1,
        components: components.into_iter().map(|component| component.state).collect(),
        unix_timestamp: now_unix_timestamp(),
        service_component_bindings: profile.service_component_bindings.clone(),
    })
}

fn resolve_component_state(
    component: &DesktopComponent,
    profile_id: &str,
    config: &Config,
) -> Result<SessionLaunchComponentState> {
    let spawn_components = config.spawn_components;
    let mut state = base_component_state(component, config);

    if spawn_components {
        if let Some(command_path) = state.resolved_command.as_ref() {
            let child = Command::new(command_path)
                .args(component.command.iter().skip(1))
                .env("WAYBROKER_PROFILE_ID", profile_id)
                .env("WAYBROKER_COMPONENT_ID", &component.id)
                .env("WAYBROKER_REPO_ROOT", config.repo_root())
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

fn base_component_state(
    component: &DesktopComponent,
    config: &Config,
) -> SessionLaunchComponentState {
    let resolved_command =
        resolve_command_path(component, config).map(|path| path.display().to_string());
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

fn resolve_command_path(component: &DesktopComponent, config: &Config) -> Option<PathBuf> {
    let executable = component.command.first()?;

    match component.launcher {
        waybroker_common::DesktopLauncher::System => {
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
        waybroker_common::DesktopLauncher::RepoScript => {
            let repo_root = config.repo_root();
            let candidate = repo_root.join(executable);
            is_executable(&candidate).then_some(candidate)
        }
        waybroker_common::DesktopLauncher::RepoBinary => {
            // 1. Check same directory as current executable (useful for custom target-dir)
            if let Ok(current_exe) = env::current_exe() {
                if let Some(exe_dir) = current_exe.parent() {
                    let candidate = exe_dir.join(executable);
                    if is_executable(&candidate) {
                        return Some(candidate);
                    }
                }
            }

            // 2. Check for cargo target dir env var
            if let Some(path) = env::var_os("CARGO_TARGET_DIR") {
                let candidate = PathBuf::from(path).join("debug").join(executable);
                if is_executable(&candidate) {
                    return Some(candidate);
                }
            }

            // 3. Fallback to project root target/debug
            let repo_root = config.repo_root();
            let candidate = repo_root.join("target").join("debug").join(executable);
            if is_executable(&candidate) {
                return Some(candidate);
            }

            None
        }
    }
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
        "sessiond launch_state profile={} protocol={} generation={} sequence={} components={}",
        state.profile_id,
        state.protocol.as_str(),
        state.generation,
        state.sequence,
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
        "service=sessiond op=profile_transition event=transition_begin from={} to={} reason=\"{}\" triggers={} timestamp={}",
        transition.source_profile_id,
        transition.target_profile_id,
        transition.reason,
        transition.trigger_component_ids.join(","),
        transition.unix_timestamp,
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
        unix_timestamp: now_unix_timestamp(),
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
            println!(
                "service=sessiond op=persist_outcome event=write_active_profile path={}",
                active_path.display()
            );
            println!(
                "service=sessiond op=persist_outcome event=write_profile_transition path={}",
                transition_path.display()
            );

            if let Some(supervisor) = supervisor {
                supervisor.switch_to(target_profile.clone(), config)?;
            } else if should_auto_launch_transition(config) {
                let launch_state = launch_state_for_profile(target_profile, config)?;
                let state_path = write_launch_state(&launch_state)?;
                print_launch_state(&launch_state);
                println!(
                    "service=sessiond op=persist_outcome event=auto_launch_profile id={}",
                    target_profile.id
                );
                println!(
                    "service=sessiond op=persist_outcome event=write_launch_state path={}",
                    state_path.display()
                );
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

fn should_notify_watchdog(config: &Config) -> bool {
    config.notify_watchdog
}

fn notify_watchdog(
    state: &SessionLaunchState,
    previous: Option<&SessionLaunchState>,
) -> Result<SessionWatchdogReport> {
    let command = watchdog_stream_command(state, previous);
    let response = send_watchdog_command(command)?;

    match response.kind {
        MessageKind::WatchdogCommand(WatchdogCommand::InspectionResult { report }) => Ok(report),
        MessageKind::WatchdogCommand(WatchdogCommand::ResyncLaunchState { profile_id, reason }) => {
            if profile_id != state.profile_id {
                bail!(
                    "watchdog requested resync for profile {} while sessiond sent {}",
                    profile_id,
                    state.profile_id
                );
            }

            println!("sessiond watchdog_resync_required profile={} reason={}", profile_id, reason);

            let retry = send_watchdog_command(WatchdogCommand::InspectLaunchState {
                state: state.clone(),
            })?;

            match retry.kind {
                MessageKind::WatchdogCommand(WatchdogCommand::InspectionResult { report }) => {
                    Ok(report)
                }
                other => bail!("watchdog returned unexpected resync response: {other:?}"),
            }
        }
        other => bail!("watchdog returned unexpected response: {other:?}"),
    }
}

fn send_watchdog_command(command: WatchdogCommand) -> Result<IpcEnvelope> {
    let mut stream = connect_service_socket(ServiceRole::Watchdog)?;
    let request = IpcEnvelope::new(
        ServiceRole::Sessiond,
        ServiceRole::Watchdog,
        MessageKind::WatchdogCommand(command),
    );

    send_json_line(&mut stream, &request)?;

    let mut reader = BufReader::new(stream);
    read_json_line(&mut reader)
}

fn watchdog_stream_command(
    state: &SessionLaunchState,
    previous: Option<&SessionLaunchState>,
) -> WatchdogCommand {
    let Some(previous) = previous else {
        return WatchdogCommand::InspectLaunchState { state: state.clone() };
    };

    if previous.profile_id != state.profile_id
        || previous.generation != state.generation
        || previous.components.len() != state.components.len()
    {
        return WatchdogCommand::UpdateLaunchState {
            delta: SessionLaunchDelta {
                profile_id: state.profile_id.clone(),
                display_name: state.display_name.clone(),
                protocol: state.protocol,
                broker_services: state.broker_services.clone(),
                generation: state.generation,
                sequence: state.sequence,
                replace: true,
                components: state.components.clone(),
                unix_timestamp: now_unix_timestamp(),
                service_component_bindings: state.service_component_bindings.clone(),
            },
        };
    }

    let changed_components: Vec<SessionLaunchComponentState> = state
        .components
        .iter()
        .filter(|component| {
            previous
                .components
                .iter()
                .find(|previous_component| previous_component.id == component.id)
                != Some(*component)
        })
        .cloned()
        .collect();

    WatchdogCommand::UpdateLaunchState {
        delta: SessionLaunchDelta {
            profile_id: state.profile_id.clone(),
            display_name: state.display_name.clone(),
            protocol: state.protocol,
            broker_services: state.broker_services.clone(),
            generation: state.generation,
            sequence: state.sequence,
            replace: false,
            components: changed_components,
            unix_timestamp: now_unix_timestamp(),
            service_component_bindings: state.service_component_bindings.clone(),
        },
    }
}

fn print_watchdog_stream_report(report: &SessionWatchdogReport) {
    println!(
        "sessiond watchdog_stream profile={} healthy={} unhealthy={} inactive={}",
        report.profile_id,
        report.healthy_components,
        report.unhealthy_components,
        report.inactive_components
    );
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

#[derive(Debug, serde::Deserialize)]
struct WatchdogRecoveryArtifact {
    #[allow(dead_code)]
    role: String,
    reason: String,
    #[allow(dead_code)]
    requested_by: String,
    unix_timestamp: u64,
    #[allow(dead_code)]
    action: String,
    status: String,
}

#[derive(Debug, serde::Serialize)]
struct WatchdogExecutionArtifact {
    role: String,
    action: String,
    requested_at: u64,
    executed_at: u64,
    result: String,
    component_id: Option<String>,
    previous_pid: Option<u32>,
    new_pid: Option<u32>,
    reason: String,
    resolution_source: String,
    bound_component_id: Option<String>,
}

struct SessionSupervisor {
    profiles: Vec<DesktopProfile>,
    profile: DesktopProfile,
    components: Vec<RuntimeComponent>,
    stream_generation: u64,
    stream_sequence: u64,
    last_streamed_state: Option<SessionLaunchState>,
}

impl SessionSupervisor {
    fn bootstrap(config: &Config, profiles: &[DesktopProfile]) -> Result<Self> {
        let profile = read_active_profile()?;
        let mut supervisor = Self::new(profile, profiles.to_vec(), 1, config)?;
        supervisor.activate(config)?;
        Ok(supervisor)
    }

    fn new(
        profile: DesktopProfile,
        profiles: Vec<DesktopProfile>,
        stream_generation: u64,
        config: &Config,
    ) -> Result<Self> {
        let mut components = Vec::with_capacity(profile.session_components.len());
        for component in &profile.session_components {
            components.push(RuntimeComponent::new(component, config)?);
        }

        Ok(Self {
            profiles,
            profile,
            components,
            stream_generation,
            stream_sequence: 0,
            last_streamed_state: None,
        })
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

        self.write_snapshot("managed_active_profile", config)
    }

    fn switch_to(&mut self, profile: DesktopProfile, config: &Config) -> Result<()> {
        self.stop_all()?;
        let profiles = self.profiles.clone();
        let next_generation = self.stream_generation.saturating_add(1);
        *self = Self::new(profile, profiles, next_generation, config)?;
        self.activate(config)?;
        println!(
            "service=sessiond op=profile_transition event=transition_complete profile={} generation={}",
            self.profile.id, self.stream_generation
        );
        Ok(())
    }

    fn process_recovery_requests(&mut self, config: &Config) -> Result<bool> {
        let runtime = ensure_runtime_dir()?;
        let mut executed_any = false;

        // Support roles
        let supported_roles = [ServiceRole::Compd, ServiceRole::Lockd];

        for role in supported_roles {
            let recovery_path = runtime.join(format!("watchdog-recovery-{}.json", role.as_str()));
            if !recovery_path.exists() {
                continue;
            }

            let raw = fs::read_to_string(&recovery_path)?;
            let recovery: WatchdogRecoveryArtifact = match serde_json::from_str(&raw) {
                Ok(r) => r,
                Err(err) => {
                    println!(
                        "service=sessiond op=recovery_execution event=invalid_artifact role={} error=\"{err}\"",
                        role.as_str()
                    );
                    let _ = fs::remove_file(&recovery_path);
                    continue;
                }
            };

            if recovery.status != "pending" {
                continue;
            }

            println!(
                "service=sessiond op=recovery_execution event=started role={} reason=\"{}\"",
                role.as_str(),
                recovery.reason
            );

            let execution = self.execute_recovery(role, &recovery, config)?;

            let execution_path =
                runtime.join(format!("watchdog-action-execution-{}.json", role.as_str()));
            let json = serde_json::to_string_pretty(&execution)?;
            fs::write(execution_path, json)?;

            println!(
                "service=sessiond op=recovery_execution event=finished role={} result={} component={:?} new_pid={:?}",
                role.as_str(),
                execution.result,
                execution.component_id,
                execution.new_pid
            );

            let _ = fs::remove_file(&recovery_path);
            executed_any = true;
        }

        Ok(executed_any)
    }

    fn execute_recovery(
        &mut self,
        role: ServiceRole,
        recovery: &WatchdogRecoveryArtifact,
        _config: &Config,
    ) -> Result<WatchdogExecutionArtifact> {
        let mut artifact = WatchdogExecutionArtifact {
            role: role.as_str().into(),
            action: "restart".into(),
            requested_at: recovery.unix_timestamp,
            executed_at: now_unix_timestamp(),
            result: "failed".into(),
            component_id: None,
            previous_pid: None,
            new_pid: None,
            reason: "no matching component for role".into(),
            resolution_source: "explicit".into(),
            bound_component_id: None,
        };

        // Try explicit binding first
        let target_id = self
            .profile
            .service_component_bindings
            .iter()
            .find(|b| b.service == role)
            .map(|b| b.component_id.clone());

        let target_component = if let Some(id) = target_id {
            artifact.bound_component_id = Some(id.clone());
            println!(
                "service=sessiond op=recovery_resolution event=bound role={} component_id={}",
                role.as_str(),
                id
            );
            self.components.iter_mut().find(|c| c.component.id == id)
        } else {
            // Legacy fallback
            artifact.resolution_source = "legacy_fallback".into();
            let target_role_str = match role {
                ServiceRole::Compd => "window-manager",
                ServiceRole::Lockd => "lockscreen",
                other => other.as_str(),
            };
            println!(
                "service=sessiond op=recovery_resolution event=legacy_fallback role={} target_role={}",
                role.as_str(),
                target_role_str
            );
            self.components.iter_mut().find(|c| c.component.role.as_str() == target_role_str)
        };

        let Some(component) = target_component else {
            artifact.result = "no-executor".into();
            artifact.reason = "no component found via selected resolution source".into();
            return Ok(artifact);
        };

        artifact.component_id = Some(component.component.id.clone());
        artifact.previous_pid = component.state.pid;

        println!(
            "service=sessiond op=recovery_execution event=stopping_component id={}",
            component.component.id
        );
        component.stop()?;

        println!(
            "service=sessiond op=recovery_execution event=spawning_component id={}",
            component.component.id
        );
        component.spawn(&self.profile.id)?;

        if component.state.state == DesktopComponentState::Spawned {
            artifact.result = "succeeded".into();
            artifact.new_pid = component.state.pid;
            artifact.reason = "component restarted successfully".into();
        } else {
            artifact.result = "failed".into();
            artifact.reason = "failed to spawn component".into();
        }

        Ok(artifact)
    }

    fn poll(&mut self, config: &Config) -> Result<bool> {
        let mut had_event = self.process_recovery_requests(config)?;

        for component in &mut self.components {
            if component.poll_and_restart(&self.profile.id, config.restart_limit)? {
                had_event = true;
            }
        }

        if had_event {
            self.write_snapshot("managed_runtime_update", config)?;
        }

        Ok(had_event)
    }

    fn stop_all(&mut self) -> Result<()> {
        for component in &mut self.components {
            component.stop()?;
        }

        Ok(())
    }

    fn snapshot(&mut self) -> SessionLaunchState {
        self.stream_sequence = self.stream_sequence.saturating_add(1);

        SessionLaunchState {
            profile_id: self.profile.id.clone(),
            display_name: self.profile.display_name.clone(),
            protocol: self.profile.protocol,
            broker_services: self.profile.broker_services.clone(),
            generation: self.stream_generation,
            sequence: self.stream_sequence,
            components: self.components.iter().map(|component| component.state.clone()).collect(),
            unix_timestamp: now_unix_timestamp(),
            service_component_bindings: self.profile.service_component_bindings.clone(),
        }
    }

    fn write_snapshot(&mut self, label: &str, config: &Config) -> Result<()> {
        let launch_state = self.snapshot();
        let state_path = write_launch_state(&launch_state)?;
        print_launch_state(&launch_state);
        println!(
            "service=sessiond op=snapshot event={} profile={} timestamp={}",
            label, self.profile.id, launch_state.unix_timestamp
        );
        println!("service=sessiond op=snapshot path={}", state_path.display());

        if should_notify_watchdog(config) {
            match notify_watchdog(&launch_state, self.last_streamed_state.as_ref()) {
                Ok(report) => {
                    print_watchdog_stream_report(&report);
                    self.last_streamed_state = Some(launch_state.clone());
                    self.apply_watchdog_stream_report(&report, config)?;
                }
                Err(err) => {
                    println!(
                        "sessiond watchdog_stream_failed profile={} reason={err:#}",
                        self.profile.id
                    );
                }
            }
        }

        Ok(())
    }

    fn apply_watchdog_stream_report(
        &mut self,
        report: &SessionWatchdogReport,
        config: &Config,
    ) -> Result<()> {
        let outcome = apply_watchdog_report(&self.profile, &self.profiles, report)?;

        match outcome {
            WatchdogApplyOutcome::Transition { target_profile, transition } => {
                let active_path = write_active_profile(&target_profile)?;
                let transition_path = write_profile_transition(&transition)?;
                print_profile_transition(&transition);
                println!("sessiond wrote_active_profile={}", active_path.display());
                println!("sessiond wrote_profile_transition={}", transition_path.display());
                self.switch_to(target_profile, config)?;
            }
            WatchdogApplyOutcome::Unchanged { .. } => {}
        }

        Ok(())
    }
}

struct RuntimeComponent {
    component: DesktopComponent,
    state: SessionLaunchComponentState,
    child: Option<Child>,
}

impl RuntimeComponent {
    fn new(component: &DesktopComponent, config: &Config) -> Result<Self> {
        Ok(Self {
            component: component.clone(),
            state: base_component_state(component, config),
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
    use super::{
        apply_watchdog_report, build_launch_state, is_executable, resolve_command_path,
        watchdog_stream_command,
    };
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };
    use waybroker_common::{
        DesktopComponent, DesktopComponentRole, DesktopComponentState, DesktopHealthStatus,
        DesktopProfile, DesktopProtocol, DesktopRecoveryAction, ServiceRole,
        SessionLaunchComponentState, SessionLaunchState, SessionWatchdogComponentReport,
        SessionWatchdogReport, WatchdogCommand,
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

        let component = DesktopComponent {
            id: "test".into(),
            role: DesktopComponentRole::WindowManager,
            command: vec![executable.display().to_string()],
            critical: true,
            launcher: waybroker_common::DesktopLauncher::System,
        };
        let config = super::Config::default();

        assert_eq!(
            resolve_command_path(&component, &config).as_deref(),
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
                launcher: waybroker_common::DesktopLauncher::System,
            }],
            service_component_bindings: Vec::new(),
        };

        let config = super::Config::default();
        let state = build_launch_state(&profile, &config).expect("build launch state");

        assert_eq!(state.components.len(), 1);
        assert_eq!(state.components[0].state, waybroker_common::DesktopComponentState::Missing);
        assert_eq!(state.components[0].restart_count, 0);
    }

    #[test]
    fn resolves_repo_script_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sessiond-repo-test-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).expect("time").as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let scripts_dir = temp_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).expect("create scripts dir");

        let script = scripts_dir.join("mock.sh");
        fs::write(&script, "#!/bin/sh\nexit 0\n").expect("write script");
        let mut permissions = fs::metadata(&script).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("chmod");

        let component = DesktopComponent {
            id: "mock".into(),
            role: DesktopComponentRole::WindowManager,
            command: vec!["scripts/mock.sh".into()],
            critical: true,
            launcher: waybroker_common::DesktopLauncher::RepoScript,
        };
        let mut config = super::Config::default();
        config.repo_root = Some(temp_dir.clone());

        assert_eq!(resolve_command_path(&component, &config).as_deref(), Some(script.as_path()));

        let _ = fs::remove_file(&script);
        let _ = fs::remove_dir_all(&temp_dir);
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
            service_component_bindings: Vec::new(),
        };
        let degraded_profile = DesktopProfile {
            id: "demo-x11-degraded".into(),
            display_name: "Degraded Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            summary: "fallback".into(),
            degraded_profile_id: None,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            session_components: Vec::new(),
            service_component_bindings: Vec::new(),
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
            unix_timestamp: 0,
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
            service_component_bindings: Vec::new(),
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
            unix_timestamp: 0,
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

    #[test]
    fn emits_delta_stream_for_component_change() {
        let previous = SessionLaunchState {
            profile_id: "demo-x11".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 1,
            components: vec![SessionLaunchComponentState {
                id: "demo-wm".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["demo-wm".into()],
                resolved_command: Some("/usr/bin/demo-wm".into()),
                state: DesktopComponentState::Spawned,
                pid: Some(42),
                restart_count: 0,
                last_exit_status: None,
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
        };
        let next = SessionLaunchState {
            profile_id: "demo-x11".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
            generation: 1,
            sequence: 2,
            components: vec![SessionLaunchComponentState {
                id: "demo-wm".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["demo-wm".into()],
                resolved_command: Some("/usr/bin/demo-wm".into()),
                state: DesktopComponentState::Failed,
                pid: None,
                restart_count: 3,
                last_exit_status: Some(1),
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
        };

        let command = watchdog_stream_command(&next, Some(&previous));

        match command {
            WatchdogCommand::UpdateLaunchState { delta } => {
                assert!(!delta.replace);
                assert_eq!(delta.generation, 1);
                assert_eq!(delta.sequence, 2);
                assert_eq!(delta.components.len(), 1);
                assert_eq!(delta.components[0].state, DesktopComponentState::Failed);
                assert_eq!(delta.components[0].restart_count, 3);
            }
            other => panic!("expected delta update, got {other:?}"),
        }
    }

    #[test]
    fn emits_replace_delta_when_profile_changes() {
        let previous = SessionLaunchState {
            profile_id: "demo-x11".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond],
            generation: 1,
            sequence: 4,
            components: vec![SessionLaunchComponentState {
                id: "demo-wm".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["demo-wm".into()],
                resolved_command: Some("/usr/bin/demo-wm".into()),
                state: DesktopComponentState::Spawned,
                pid: Some(42),
                restart_count: 0,
                last_exit_status: None,
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
        };
        let next = SessionLaunchState {
            profile_id: "demo-x11-degraded".into(),
            display_name: "Degraded Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Sessiond],
            generation: 2,
            sequence: 1,
            components: vec![SessionLaunchComponentState {
                id: "openbox".into(),
                role: DesktopComponentRole::WindowManager,
                critical: true,
                command: vec!["openbox".into()],
                resolved_command: Some("/usr/bin/openbox".into()),
                state: DesktopComponentState::Spawned,
                pid: Some(84),
                restart_count: 0,
                last_exit_status: None,
            }],
            unix_timestamp: 0,
            service_component_bindings: Vec::new(),
        };

        let command = watchdog_stream_command(&next, Some(&previous));

        match command {
            WatchdogCommand::UpdateLaunchState { delta } => {
                assert!(delta.replace);
                assert_eq!(delta.profile_id, "demo-x11-degraded");
                assert_eq!(delta.generation, 2);
                assert_eq!(delta.sequence, 1);
                assert_eq!(delta.components.len(), 1);
                assert_eq!(delta.components[0].id, "openbox");
            }
            other => panic!("expected replace delta, got {other:?}"),
        }
    }
}
