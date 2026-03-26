use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    DesktopComponentState, DesktopHealthStatus, DesktopRecoveryAction, ServiceBanner, ServiceRole,
    SessionLaunchComponentState, SessionLaunchState, SessionWatchdogComponentReport,
    SessionWatchdogReport, ensure_runtime_dir, runtime_dir,
};

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Watchdog, "display stack recovery control");
    println!("{}", banner.render());

    if !config.has_inspection_target() {
        return Ok(());
    }

    let launch_states = load_launch_states(&config)?;

    for launch_state in &launch_states {
        let report = inspect_launch_state(launch_state);
        print_report(&report);

        if config.write_reports {
            let report_path = write_report(&report)?;
            println!("watchdog wrote_report={}", report_path.display());
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
                "--help" | "-h" => {
                    println!(
                        "usage: watchdog [--launch-state PATH] [--profile-id ID] [--inspect-all] [--write-reports]"
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

fn write_report(report: &SessionWatchdogReport) -> Result<PathBuf> {
    let dir = ensure_runtime_dir()?;
    let path = dir.join(format!("watchdog-report-{}.json", report.profile_id));
    let json =
        serde_json::to_string_pretty(report).context("failed to serialize watchdog report")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn print_report(report: &SessionWatchdogReport) {
    println!(
        "watchdog profile={} healthy={} unhealthy={} inactive={}",
        report.profile_id,
        report.healthy_components,
        report.unhealthy_components,
        report.inactive_components
    );

    for component in &report.components {
        println!(
            "watchdog component id={} role={} critical={} status={} pid={} crashes={} action={} reason={}",
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

fn launch_state_path(profile_id: &str) -> PathBuf {
    runtime_dir().join(format!("launch-state-{profile_id}.json"))
}

#[cfg(test)]
mod tests {
    use super::inspect_launch_state;
    use waybroker_common::{
        DesktopComponentRole, DesktopComponentState, DesktopHealthStatus, DesktopProtocol,
        DesktopRecoveryAction, ServiceRole, SessionLaunchComponentState, SessionLaunchState,
    };

    #[test]
    fn marks_missing_component_as_unhealthy() {
        let state = SessionLaunchState {
            profile_id: "demo".into(),
            display_name: "Demo".into(),
            protocol: DesktopProtocol::LayerX11,
            broker_services: vec![ServiceRole::Watchdog],
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
        };

        let report = inspect_launch_state(&state);

        assert_eq!(report.inactive_components, 1);
        assert_eq!(report.components[0].status, DesktopHealthStatus::Inactive);
        assert_eq!(report.components[0].action, DesktopRecoveryAction::None);
    }
}
