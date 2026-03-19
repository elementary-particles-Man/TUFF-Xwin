mod ipc;

pub use ipc::{
    CommitTarget, DisplayCommand, FocusTarget, HealthState, IpcEnvelope, LockCommand, LockState,
    MessageKind, OutputMode, ResumeStage, SessionCommand, SurfacePlacement, SurfaceSnapshot,
    WatchdogCommand,
};

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
        CommitTarget, DisplayCommand, FocusTarget, IpcEnvelope, MessageKind, OutputMode,
        ServiceBanner, ServiceRole, SurfacePlacement, SurfaceSnapshot,
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
}
