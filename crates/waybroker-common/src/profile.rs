use serde::{Deserialize, Serialize};

use crate::ServiceRole;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopProtocol {
    LayerX11,
    WaylandNative,
}

impl DesktopProtocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LayerX11 => "layer-x11",
            Self::WaylandNative => "wayland-native",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopComponentRole {
    CompatLayer,
    WindowManager,
    Shell,
    Panel,
    SettingsDaemon,
    Applet,
    Portal,
}

impl DesktopComponentRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CompatLayer => "compat-layer",
            Self::WindowManager => "window-manager",
            Self::Shell => "shell",
            Self::Panel => "panel",
            Self::SettingsDaemon => "settings-daemon",
            Self::Applet => "applet",
            Self::Portal => "portal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopComponent {
    pub id: String,
    pub role: DesktopComponentRole,
    pub command: Vec<String>,
    pub critical: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopProfile {
    pub id: String,
    pub display_name: String,
    pub protocol: DesktopProtocol,
    pub summary: String,
    pub broker_services: Vec<ServiceRole>,
    pub session_components: Vec<DesktopComponent>,
}

impl DesktopProfile {
    pub fn launch_plan(&self) -> SessionLaunchPlan {
        SessionLaunchPlan {
            profile_id: self.id.clone(),
            display_name: self.display_name.clone(),
            protocol: self.protocol,
            broker_services: self.broker_services.clone(),
            session_components: self.session_components.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLaunchPlan {
    pub profile_id: String,
    pub display_name: String,
    pub protocol: DesktopProtocol,
    pub broker_services: Vec<ServiceRole>,
    pub session_components: Vec<DesktopComponent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopComponentState {
    Ready,
    Missing,
    Spawned,
    Failed,
}

impl DesktopComponentState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::Spawned => "spawned",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLaunchComponentState {
    pub id: String,
    pub role: DesktopComponentRole,
    pub critical: bool,
    pub command: Vec<String>,
    pub resolved_command: Option<String>,
    pub state: DesktopComponentState,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLaunchState {
    pub profile_id: String,
    pub display_name: String,
    pub protocol: DesktopProtocol,
    pub broker_services: Vec<ServiceRole>,
    pub components: Vec<SessionLaunchComponentState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopHealthStatus {
    Healthy,
    Unhealthy,
    Inactive,
}

impl DesktopHealthStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Unhealthy => "unhealthy",
            Self::Inactive => "inactive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionWatchdogComponentReport {
    pub id: String,
    pub role: DesktopComponentRole,
    pub critical: bool,
    pub status: DesktopHealthStatus,
    pub pid: Option<u32>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionWatchdogReport {
    pub profile_id: String,
    pub display_name: String,
    pub protocol: DesktopProtocol,
    pub healthy_components: usize,
    pub unhealthy_components: usize,
    pub inactive_components: usize,
    pub components: Vec<SessionWatchdogComponentReport>,
}

#[cfg(test)]
mod tests {
    use super::{DesktopComponent, DesktopComponentRole, DesktopProfile, DesktopProtocol};
    use crate::ServiceRole;

    #[test]
    fn derives_launch_plan_without_mutating_profile() {
        let profile = DesktopProfile {
            id: "xfce-x11".into(),
            display_name: "XFCE Classic on LeyerX11".into(),
            protocol: DesktopProtocol::LayerX11,
            summary: "lightweight x11 desktop".into(),
            broker_services: vec![
                ServiceRole::Displayd,
                ServiceRole::Sessiond,
                ServiceRole::X11Bridge,
            ],
            session_components: vec![DesktopComponent {
                id: "xfwm4".into(),
                role: DesktopComponentRole::WindowManager,
                command: vec!["xfwm4".into(), "--replace".into()],
                critical: true,
            }],
        };

        let plan = profile.launch_plan();

        assert_eq!(plan.profile_id, "xfce-x11");
        assert_eq!(plan.protocol, DesktopProtocol::LayerX11);
        assert_eq!(plan.session_components.len(), 1);
    }
}
