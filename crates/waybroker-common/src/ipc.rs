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
    SetMode {
        output: String,
        mode: OutputMode,
    },
    SetGamma {
        output: String,
        red: Vec<u16>,
        green: Vec<u16>,
        blue: Vec<u16>,
    },
    CommitScene {
        target: CommitTarget,
        focus: FocusTarget,
        #[serde(default)]
        selection: WaylandSelectionState,
        surfaces: Vec<SurfaceSnapshot>,
    },
    GetSceneSnapshot {
        output: Option<String>,
    },
    CaptureOutput {
        output: String,
    },
    StartRecord {
        output: String,
        fps: u32,
    },
    StopRecord {
        output: String,
    },
    SecureBlank {
        output: Option<String>,
    },
    SetPointerConstraints {
        output: String,
        constraints: PointerConstraints,
    },
    ResumeBegin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum PointerConstraints {
    None,
    Locked { x: i32, y: i32 },
    Confined { region: Vec<Rect> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
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
        #[serde(default)]
        selection: WaylandSelectionState,
        surface_count: usize,
        commit_id: u64,
    },
    SceneSnapshot {
        snapshot: Option<CommittedSceneState>,
    },
    OutputCaptured {
        output: String,
        width: u32,
        height: u32,
        format: String,
        artifact_path: String,
    },
    RecordStarted {
        output: String,
        session_id: String,
    },
    RecordStopped {
        output: String,
        session_id: String,
        artifact_path: String,
    },
    FrameCaptured {
        output: String,
        session_id: String,
        frame_number: u64,
        artifact_path: String,
    },
    BlankApplied {
        output: Option<String>,
    },
    GammaApplied {
        output: String,
    },
    PointerConstraintsApplied {
        output: String,
        constraints: PointerConstraints,
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
    CaptureOutput { output: String },
    StartRecord { output: String, fps: u32 },
    StopRecord { output: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum WaylandEvent {
    SurfaceRegistry {
        snapshot: SurfaceRegistrySnapshot,
    },
    SelectionHandoffApplied {
        generation: u64,
        handoff: WaylandSelectionHandoff,
    },
    OutputCaptured {
        output: String,
        width: u32,
        height: u32,
        format: String,
        artifact_path: String,
    },
    RecordStarted {
        output: String,
        session_id: String,
    },
    RecordStopped {
        output: String,
        session_id: String,
        artifact_path: String,
    },
    Rejected {
        reason: String,
    },
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
    InhibitIdle { reason: String },
    ReleaseIdle { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum WatchdogCommand {
    Restart {
        role: ServiceRole,
        #[serde(default = "default_session_instance_id")]
        session_instance_id: String,
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
    #[serde(default)]
    pub selection: WaylandSelectionState,
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
    pub clipboard_payload_id: Option<String>,
    #[serde(default)]
    pub clipboard_source_serial: Option<u64>,
    #[serde(default)]
    pub primary_selection_owner: Option<String>,
    #[serde(default)]
    pub primary_selection_payload_id: Option<String>,
    #[serde(default)]
    pub primary_selection_source_serial: Option<u64>,
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
