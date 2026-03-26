mod ipc;
mod profile;
mod transport;

pub use ipc::{
    CommitTarget, DisplayCommand, DisplayEvent, FocusTarget, HealthState, IpcEnvelope, LockCommand,
    LockState, MessageKind, OutputMode, ResumeStage, SessionCommand, SurfacePlacement,
    SurfaceSnapshot, WatchdogCommand,
};
pub use profile::{
    DesktopComponent, DesktopComponentRole, DesktopComponentState, DesktopHealthStatus,
    DesktopLauncher, DesktopProfile, DesktopProtocol, DesktopRecoveryAction,
    SessionLaunchComponentState, SessionLaunchDelta, SessionLaunchPlan, SessionLaunchState,
    SessionProfileTransition, SessionWatchdogComponentReport, SessionWatchdogReport,
};
pub use transport::{
    bind_service_socket, connect_service_socket, ensure_runtime_dir, read_json_line, runtime_dir,
    send_json_line, service_socket_path,
};

pub fn now_unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_secs()
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceRole {
    Displayd,
    Waylandd,
    Compd,
    Lockd,
    Sessiond,
    Watchdog,
    #[serde(rename = "x11bridge")]
    X11Bridge,
}

impl ServiceRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Displayd => "displayd",
            Self::Waylandd => "waylandd",
            Self::Compd => "compd",
            Self::Lockd => "lockd",
            Self::Sessiond => "sessiond",
            Self::Watchdog => "watchdog",
            Self::X11Bridge => "x11bridge",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceBanner {
    pub role: ServiceRole,
    pub responsibility: &'static str,
}

impl ServiceBanner {
    pub const fn new(role: ServiceRole, responsibility: &'static str) -> Self {
        Self { role, responsibility }
    }

    pub fn render(self) -> String {
        format!("waybroker service={} responsibility={}", self.role.as_str(), self.responsibility)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommitTarget, DisplayCommand, DisplayEvent, FocusTarget, IpcEnvelope, MessageKind,
        OutputMode, ServiceBanner, ServiceRole, SurfacePlacement, SurfaceSnapshot,
    };

    #[test]
    fn renders_service_banner() {
        let banner = ServiceBanner::new(ServiceRole::Compd, "scene and focus policy");
        assert_eq!(
            banner.render(),
            "waybroker service=compd responsibility=scene and focus policy"
        );
    }

    #[test]
    fn serializes_compd_scene_commit() {
        let envelope = IpcEnvelope::new(
            ServiceRole::Compd,
            ServiceRole::Displayd,
            MessageKind::DisplayCommand(DisplayCommand::CommitScene {
                target: CommitTarget::Output { name: "eDP-1".into() },
                focus: FocusTarget::Surface { id: "terminal-1".into() },
                surfaces: vec![
                    SurfaceSnapshot {
                        id: "terminal-1".into(),
                        app_id: "org.kde.konsole".into(),
                        placement: SurfacePlacement {
                            x: 80,
                            y: 64,
                            width: 1280,
                            height: 800,
                            z: 10,
                            visible: true,
                        },
                    },
                    SurfaceSnapshot {
                        id: "panel-1".into(),
                        app_id: "org.kde.plasmashell.panel".into(),
                        placement: SurfacePlacement {
                            x: 0,
                            y: 0,
                            width: 1920,
                            height: 36,
                            z: 100,
                            visible: true,
                        },
                    },
                ],
            }),
        );

        let json = serde_json::to_string(&envelope).expect("serialize envelope");

        assert!(json.contains("\"source\":\"compd\""));
        assert!(json.contains("\"kind\":\"display-command\""));
        assert!(json.contains("\"target\":{\"type\":\"output\",\"name\":\"eDP-1\"}"));
    }

    #[test]
    fn roundtrips_resume_stage_event() {
        let envelope = IpcEnvelope::new(
            ServiceRole::Sessiond,
            ServiceRole::Compd,
            MessageKind::SessionCommand(super::SessionCommand::ResumeHint {
                stage: super::ResumeStage::OutputsRecovered,
                output: Some(OutputMode {
                    name: "eDP-1".into(),
                    width: 1920,
                    height: 1080,
                    refresh_hz: 60,
                }),
            }),
        );

        let json = serde_json::to_string(&envelope).expect("serialize");
        let decoded: IpcEnvelope = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(decoded, envelope);
    }

    #[test]
    fn serializes_display_output_inventory() {
        let envelope = IpcEnvelope::new(
            ServiceRole::Displayd,
            ServiceRole::Waylandd,
            MessageKind::DisplayEvent(DisplayEvent::OutputInventory {
                outputs: vec![OutputMode {
                    name: "eDP-1".into(),
                    width: 1920,
                    height: 1080,
                    refresh_hz: 60,
                }],
            }),
        );

        let json = serde_json::to_string(&envelope).expect("serialize");

        assert!(json.contains("\"kind\":\"display-event\""));
        assert!(json.contains("\"op\":\"output-inventory\""));
    }

    #[test]
    fn roundtrips_session_watchdog_report_command() {
        let envelope = IpcEnvelope::new(
            ServiceRole::Watchdog,
            ServiceRole::Sessiond,
            MessageKind::SessionCommand(super::SessionCommand::ApplyWatchdogReport {
                report: super::SessionWatchdogReport {
                    profile_id: "demo-x11-crashy".into(),
                    display_name: "Crashy Demo".into(),
                    protocol: super::DesktopProtocol::LayerX11,
                    healthy_components: 1,
                    unhealthy_components: 1,
                    inactive_components: 0,
                    components: vec![super::SessionWatchdogComponentReport {
                        id: "crashy-wm".into(),
                        role: super::DesktopComponentRole::WindowManager,
                        critical: true,
                        status: super::DesktopHealthStatus::Unhealthy,
                        pid: None,
                        crash_loop_count: 3,
                        action: super::DesktopRecoveryAction::DegradedProfile,
                        reason: "component spawn failed or supervisor gave up".into(),
                    }],
                },
            }),
        );

        let json = serde_json::to_string(&envelope).expect("serialize");
        let decoded: IpcEnvelope = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(decoded, envelope);
        assert!(json.contains("\"op\":\"apply-watchdog-report\""));
    }

    #[test]
    fn roundtrips_watchdog_launch_state_inspection() {
        let envelope = IpcEnvelope::new(
            ServiceRole::Sessiond,
            ServiceRole::Watchdog,
            MessageKind::WatchdogCommand(super::WatchdogCommand::InspectLaunchState {
                state: super::SessionLaunchState {
                    profile_id: "demo-x11".into(),
                    display_name: "Demo".into(),
                    protocol: super::DesktopProtocol::LayerX11,
                    broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
                    generation: 1,
                    sequence: 1,
                    components: vec![super::SessionLaunchComponentState {
                        id: "demo-wm".into(),
                        role: super::DesktopComponentRole::WindowManager,
                        critical: true,
                        command: vec!["demo-wm".into()],
                        resolved_command: Some("/usr/bin/demo-wm".into()),
                        state: super::DesktopComponentState::Spawned,
                        pid: Some(1234),
                        restart_count: 0,
                        last_exit_status: None,
                    }],
                    unix_timestamp: 0,
                },
            }),
        );

        let json = serde_json::to_string(&envelope).expect("serialize");
        let decoded: IpcEnvelope = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(decoded, envelope);
        assert!(json.contains("\"op\":\"inspect-launch-state\""));
    }

    #[test]
    fn roundtrips_watchdog_launch_state_delta() {
        let envelope = IpcEnvelope::new(
            ServiceRole::Sessiond,
            ServiceRole::Watchdog,
            MessageKind::WatchdogCommand(super::WatchdogCommand::UpdateLaunchState {
                delta: super::SessionLaunchDelta {
                    profile_id: "demo-x11".into(),
                    display_name: "Demo".into(),
                    protocol: super::DesktopProtocol::LayerX11,
                    broker_services: vec![ServiceRole::Sessiond, ServiceRole::Watchdog],
                    generation: 1,
                    sequence: 2,
                    replace: false,
                    components: vec![super::SessionLaunchComponentState {
                        id: "demo-wm".into(),
                        role: super::DesktopComponentRole::WindowManager,
                        critical: true,
                        command: vec!["demo-wm".into()],
                        resolved_command: Some("/usr/bin/demo-wm".into()),
                        state: super::DesktopComponentState::Failed,
                        pid: None,
                        restart_count: 3,
                        last_exit_status: Some(1),
                    }],
                    unix_timestamp: 0,
                },
            }),
        );

        let json = serde_json::to_string(&envelope).expect("serialize");
        let decoded: IpcEnvelope = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(decoded, envelope);
        assert!(json.contains("\"op\":\"update-launch-state\""));
    }

    #[test]
    fn roundtrips_watchdog_resync_request() {
        let envelope = IpcEnvelope::new(
            ServiceRole::Watchdog,
            ServiceRole::Sessiond,
            MessageKind::WatchdogCommand(super::WatchdogCommand::ResyncLaunchState {
                profile_id: "demo-x11".into(),
                reason: "cache miss".into(),
            }),
        );

        let json = serde_json::to_string(&envelope).expect("serialize");
        let decoded: IpcEnvelope = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(decoded, envelope);
        assert!(json.contains("\"op\":\"resync-launch-state\""));
    }
}
