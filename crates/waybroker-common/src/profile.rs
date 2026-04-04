use serde::{Deserialize, Serialize};

use crate::ServiceRole;

const fn default_stream_generation() -> u64 {
    1
}

const fn default_stream_sequence() -> u64 {
    1
}

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
    #[serde(rename = "lockscreen")]
    LockScreen,
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
            Self::LockScreen => "lockscreen",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopLauncher {
    System,
    RepoScript,
    RepoBinary,
}

impl Default for DesktopLauncher {
    fn default() -> Self {
        Self::System
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopComponent {
    pub id: String,
    pub role: DesktopComponentRole,
    pub command: Vec<String>,
    pub critical: bool,
    #[serde(default)]
    pub launcher: DesktopLauncher,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceComponentBinding {
    pub service: ServiceRole,
    pub component_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryExecutionMode {
    Disabled,
    SupervisorRestart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRecoveryExecutionPolicy {
    pub service: ServiceRole,
    pub mode: RecoveryExecutionMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preserve_final_state: Option<String>,
    #[serde(default)]
    pub restart_command_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopProfile {
    pub id: String,
    pub display_name: String,
    pub protocol: DesktopProtocol,
    pub summary: String,
    pub degraded_profile_id: Option<String>,
    pub broker_services: Vec<ServiceRole>,
    pub session_components: Vec<DesktopComponent>,
    #[serde(default)]
    pub service_component_bindings: Vec<ServiceComponentBinding>,
    #[serde(default)]
    pub service_recovery_execution_policies: Vec<ServiceRecoveryExecutionPolicy>,
}

impl DesktopProfile {
    pub fn launch_plan(&self) -> SessionLaunchPlan {
        SessionLaunchPlan {
            profile_id: self.id.clone(),
            display_name: self.display_name.clone(),
            protocol: self.protocol,
            broker_services: self.broker_services.clone(),
            session_components: self.session_components.clone(),
            service_component_bindings: self.service_component_bindings.clone(),
            service_recovery_execution_policies: self.service_recovery_execution_policies.clone(),
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
    #[serde(default)]
    pub service_component_bindings: Vec<ServiceComponentBinding>,
    #[serde(default)]
    pub service_recovery_execution_policies: Vec<ServiceRecoveryExecutionPolicy>,
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
    pub restart_count: u32,
    pub last_exit_status: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLaunchState {
    pub profile_id: String,
    pub display_name: String,
    pub protocol: DesktopProtocol,
    pub broker_services: Vec<ServiceRole>,
    #[serde(default = "default_stream_generation")]
    pub generation: u64,
    #[serde(default = "default_stream_sequence")]
    pub sequence: u64,
    pub components: Vec<SessionLaunchComponentState>,
    pub unix_timestamp: u64,
    #[serde(default)]
    pub service_component_bindings: Vec<ServiceComponentBinding>,
    #[serde(default)]
    pub service_recovery_execution_policies: Vec<ServiceRecoveryExecutionPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLaunchDelta {
    pub profile_id: String,
    pub display_name: String,
    pub protocol: DesktopProtocol,
    pub broker_services: Vec<ServiceRole>,
    #[serde(default = "default_stream_generation")]
    pub generation: u64,
    #[serde(default = "default_stream_sequence")]
    pub sequence: u64,
    pub replace: bool,
    pub components: Vec<SessionLaunchComponentState>,
    pub unix_timestamp: u64,
    #[serde(default)]
    pub service_component_bindings: Vec<ServiceComponentBinding>,
    #[serde(default)]
    pub service_recovery_execution_policies: Vec<ServiceRecoveryExecutionPolicy>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopRecoveryAction {
    None,
    RestartComponent,
    DegradedProfile,
}

impl DesktopRecoveryAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RestartComponent => "restart-component",
            Self::DegradedProfile => "degraded-profile",
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
    pub crash_loop_count: u32,
    pub action: DesktopRecoveryAction,
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
    pub unix_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProfileTransition {
    pub source_profile_id: String,
    pub source_display_name: String,
    pub target_profile_id: String,
    pub target_display_name: String,
    pub reason: String,
    pub trigger_component_ids: Vec<String>,
    pub unix_timestamp: u64,
}

#[cfg(test)]
mod tests {
    use super::{
        DesktopComponent, DesktopComponentRole, DesktopLauncher, DesktopProfile, DesktopProtocol,
    };
    use crate::ServiceRole;

    #[test]
    fn derives_launch_plan_without_mutating_profile() {
        let profile = DesktopProfile {
            id: "xfce-x11".into(),
            display_name: "XFCE Classic on LeyerX11".into(),
            protocol: DesktopProtocol::LayerX11,
            summary: "lightweight x11 desktop".into(),
            degraded_profile_id: Some("openbox-x11".into()),
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
                launcher: DesktopLauncher::System,
            }],
            service_component_bindings: Vec::new(),
            service_recovery_execution_policies: Vec::new(),
        };

        let plan = profile.launch_plan();

        assert_eq!(plan.profile_id, "xfce-x11");
        assert_eq!(plan.protocol, DesktopProtocol::LayerX11);
        assert_eq!(plan.session_components.len(), 1);
    }
}
