use serde::{Deserialize, Serialize};

use crate::{
    ServiceRole, SessionLaunchDelta, SessionLaunchState, SessionProfileTransition,
    SessionWatchdogReport, profile::default_session_instance_id,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcEnvelope {
    pub source: ServiceRole,
    pub destination: ServiceRole,
    pub kind: MessageKind,
}

impl IpcEnvelope {
    pub fn new(source: ServiceRole, destination: ServiceRole, kind: MessageKind) -> Self {
        Self { source, destination, kind }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "kebab-case")]
pub enum MessageKind {
    DisplayCommand(DisplayCommand),
    DisplayEvent(DisplayEvent),
    WaylandCommand(WaylandCommand),
    WaylandEvent(WaylandEvent),
    LockCommand(LockCommand),
    SessionCommand(SessionCommand),
    WatchdogCommand(WatchdogCommand),
    HealthState(HealthState),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum DisplayCommand {
    EnumerateOutputs,
    SetMode { output: String, mode: OutputMode },
    CommitScene { target: CommitTarget, focus: FocusTarget, surfaces: Vec<SurfaceSnapshot> },
    GetSceneSnapshot { output: Option<String> },
    SecureBlank { output: Option<String> },
    ResumeBegin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum DisplayEvent {
    OutputInventory {
        outputs: Vec<OutputMode>,
    },
    ModeApplied {
        output: String,
        mode: OutputMode,
    },
    SceneCommitted {
        target: CommitTarget,
        focus: FocusTarget,
        surface_count: usize,
        commit_id: u64,
    },
    SceneSnapshot {
        snapshot: Option<CommittedSceneState>,
    },
    BlankApplied {
        output: Option<String>,
    },
    Rejected {
        reason: String,
    },
    ResumeStarted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum WaylandCommand {
    GetSurfaceRegistry,
    ApplySelectionHandoff { handoff: WaylandSelectionHandoff },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum WaylandEvent {
    SurfaceRegistry { snapshot: SurfaceRegistrySnapshot },
    SelectionHandoffApplied { generation: u64, handoff: WaylandSelectionHandoff },
    Rejected { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum LockCommand {
    SetLockState { state: LockState },
    AuthPrompt { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum SessionCommand {
    SuspendRequested,
    ResumeHint { stage: ResumeStage, output: Option<OutputMode> },
    DegradedMode { reason: String },
    ApplyWatchdogReport { report: SessionWatchdogReport },
    ProfileTransition { transition: SessionProfileTransition },
    ProfileUnchanged { profile_id: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum WatchdogCommand {
    Restart {
        role: ServiceRole,
        reason: String,
    },
    Escalate {
        level: u8,
        reason: String,
    },
    InspectLaunchState {
        state: SessionLaunchState,
    },
    UpdateLaunchState {
        delta: SessionLaunchDelta,
    },
    ResyncLaunchState {
        profile_id: String,
        #[serde(default = "default_session_instance_id")]
        session_instance_id: String,
        reason: String,
    },
    InspectionResult {
        report: SessionWatchdogReport,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum HealthState {
    Healthy { role: ServiceRole },
    Unhealthy { role: ServiceRole, reason: String, crash_loop_count: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum CommitTarget {
    Output { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum FocusTarget {
    Surface { id: String },
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LockState {
    Locked,
    Unlocked,
    BlankOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResumeStage {
    Begin,
    OutputsRecovered,
    LockReady,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputMode {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedSceneState {
    pub source: ServiceRole,
    pub target: CommitTarget,
    pub focus: FocusTarget,
    pub surfaces: Vec<SurfaceSnapshot>,
    pub commit_id: u64,
    pub unix_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceRegistrySnapshot {
    pub generation: u64,
    pub surfaces: Vec<WaylandSurfaceState>,
    #[serde(default)]
    pub selection: WaylandSelectionState,
    pub unix_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WaylandSelectionState {
    #[serde(default)]
    pub clipboard_owner: Option<String>,
    #[serde(default)]
    pub primary_selection_owner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaylandSelectionHandoff {
    pub focus: FocusTarget,
    pub selection: WaylandSelectionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaylandSurfaceState {
    pub id: String,
    pub app_id: String,
    pub role: WaylandSurfaceRole,
    pub mapped: bool,
    pub buffer_attached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaylandSurfaceRole {
    Toplevel,
    Popup,
    Layer,
    Background,
    Lock,
    Cursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceSnapshot {
    pub id: String,
    pub app_id: String,
    pub placement: SurfacePlacement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfacePlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub z: i32,
    pub visible: bool,
}
