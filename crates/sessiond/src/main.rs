use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    DesktopComponent, DesktopComponentState, DesktopProfile, ServiceBanner, ServiceRole,
    SessionLaunchComponentState, SessionLaunchState, ensure_runtime_dir, runtime_dir,
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
        let launch_state = build_launch_state(profile, config.spawn_components)?;
        let state_path = write_launch_state(&launch_state)?;

        print_launch_state(&launch_state);
        println!("sessiond wrote_launch_state={}", state_path.display());
    }

    if config.launch_active {
        let profile = read_active_profile()?;
        let launch_state = build_launch_state(&profile, config.spawn_components)?;
        let state_path = write_launch_state(&launch_state)?;

        print_launch_state(&launch_state);
        println!("sessiond wrote_launch_state={}", state_path.display());
    }

    Ok(())
}

#[derive(Debug, Default)]
struct Config {
    profiles_dir: Option<PathBuf>,
    list_profiles: bool,
    selected_profile_id: Option<String>,
    print_launch_plan: bool,
    write_selection: bool,
    launch_profile_id: Option<String>,
    launch_active: bool,
    spawn_components: bool,
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
                "--help" | "-h" => {
                    println!(
                        "usage: sessiond [--profiles-dir PATH] [--list-profiles] [--select-profile ID] [--print-launch-plan] [--write-selection] [--launch-profile ID] [--launch-active] [--spawn-components]"
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

fn read_active_profile() -> Result<DesktopProfile> {
    let path = active_profile_path();
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read active profile {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to decode active profile {}", path.display()))
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

fn resolve_component_state(
    component: &DesktopComponent,
    profile_id: &str,
    spawn_components: bool,
) -> Result<SessionLaunchComponentState> {
    let resolved_command = resolve_command_path(&component.command);
    let mut state = match resolved_command.as_ref() {
        Some(path) => SessionLaunchComponentState {
            id: component.id.clone(),
            role: component.role,
            critical: component.critical,
            command: component.command.clone(),
            resolved_command: Some(path.display().to_string()),
            state: DesktopComponentState::Ready,
            pid: None,
        },
        None => SessionLaunchComponentState {
            id: component.id.clone(),
            role: component.role,
            critical: component.critical,
            command: component.command.clone(),
            resolved_command: None,
            state: DesktopComponentState::Missing,
            pid: None,
        },
    };

    if spawn_components {
        if let Some(command_path) = resolved_command {
            let child = Command::new(&command_path)
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
                }
            }
        }
    }

    Ok(state)
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
            "sessiond launch component id={} role={} critical={} state={} resolved={} pid={}",
            component.id,
            component.role.as_str(),
            component.critical,
            component.state.as_str(),
            resolved,
            component.pid.map(|pid| pid.to_string()).as_deref().unwrap_or("none")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{build_launch_state, is_executable, resolve_command_path};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };
    use waybroker_common::{
        DesktopComponent, DesktopComponentRole, DesktopProfile, DesktopProtocol, ServiceRole,
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
    }
}
