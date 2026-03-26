use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    DesktopProfile, ServiceBanner, ServiceRole, ensure_runtime_dir, runtime_dir,
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

    Ok(())
}

#[derive(Debug, Default)]
struct Config {
    profiles_dir: Option<PathBuf>,
    list_profiles: bool,
    selected_profile_id: Option<String>,
    print_launch_plan: bool,
    write_selection: bool,
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
                "--help" | "-h" => {
                    println!(
                        "usage: sessiond [--profiles-dir PATH] [--list-profiles] [--select-profile ID] [--print-launch-plan] [--write-selection]"
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

#[allow(dead_code)]
fn active_profile_path() -> PathBuf {
    runtime_dir().join("active-profile.json")
}
