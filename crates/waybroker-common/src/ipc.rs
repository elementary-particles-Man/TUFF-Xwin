use serde::{Deserialize, Serialize};

use crate::pixel_transport::PixelTransportPayload;

use crate::{
    ServiceRole, SessionLaunchDelta, SessionLaunchState, SessionProfileTransition,
    SessionWatchdogReport, profile::default_session_instance_id,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "kebab-case")]
pub enum MessageKind {
    DisplayCommand(DisplayCommand),
    DisplayEvent(DisplayEvent),
    BackendOutputEvent(BackendOutputEvent),
    WaylandCommand(WaylandCommand),
    WaylandEvent(WaylandEvent),
    LockCommand(LockCommand),
    SessionCommand(SessionCommand),
    WatchdogCommand(WatchdogCommand),
    HealthState(HealthState),
    ImeCommand(ImeCommand),
    ImeEvent(ImeEvent),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum ImeCommand {
    GetImeStatus,
    SetImeBridgeMode { mode: ImeBridgeMode },
    FocusTextSurface { surface_id: String },
    ClearTextFocus,
    CommitString { text: String },
    PreeditString { text: String, cursor_begin: i32, cursor_end: i32 },
    DeleteSurroundingText { before_length: u32, after_length: u32 },
    SetCursorRect { x: i32, y: i32, width: u32, height: u32 },
    SetSurroundingText { text: String, cursor: u32, anchor: u32 },
    SetContentType { hint: u32, purpose: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImeBridgeMode {
    Disabled,
    PassthroughExternal,
    ProtocolStub,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImeStatus {
    pub bridge_mode: ImeBridgeMode,
    pub focused_surface_id: Option<String>,
    pub preedit_active: bool,
    pub commit_count: u64,
    pub cursor_rect: Option<Rect>,
    pub surrounding_text: Option<String>,
    pub surrounding_cursor: u32,
    pub content_purpose: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum ImeEvent {
    Status { status: ImeStatus },
    BridgeModeChanged { mode: ImeBridgeMode },
    TextFocusChanged { surface_id: Option<String> },
    StringCommitted { text: String },
    PreeditUpdated { text: String, cursor_begin: i32, cursor_end: i32 },
    SurroundingTextDeleted { before_length: u32, after_length: u32 },
    CursorRectChanged { rect: Rect },
    SurroundingTextChanged { text: String, cursor: u32, anchor: u32 },
    ContentTypeChanged { hint: u32, purpose: u32 },
    Rejected { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
// IPC values remain inline to preserve the established wire shape and public API.
#[allow(clippy::large_enum_variant)]
pub enum DisplayCommand {
    GetLiveness,
    GetReadiness,
    GetReconciliation,
    BeginReconciliation {
        epoch: u64,
    },
    AdvancePresentation {
        output_id: String,
        output_generation: u64,
        now_ns: u64,
        tick_sequence: u64,
    },
    CompletePresentation {
        token: PresentationToken,
        outcome: PresentationCompletion,
    },
    EnumerateOutputs,
    ConfigureOutput {
        geometry: OutputGeometry,
    },
    RemoveOutput {
        output_id: String,
        output_generation: u64,
    },
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
        #[serde(default)]
        pixel_payloads: Vec<PixelTransportPayload>,
        #[serde(default)]
        scene_epoch: u64,
        #[serde(default)]
        scene_generation: u64,
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
    GetPresentationFeedback {
        commit_id: u64,
    },
    ResumeBegin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputGeometry {
    pub output_id: String,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub output_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum DisplayEvent {
    Liveness {
        responsive: bool,
    },
    Readiness(ServiceReadiness),
    Reconciliation {
        epoch: u64,
        state: DisplayReconciliationState,
    },
    PresentationCompleted {
        token: PresentationToken,
        outcome: PresentationCompletion,
    },
    PresentationAdvanced {
        output_id: String,
        tick_sequence: u64,
        eligible: bool,
    },
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
        #[serde(default)]
        publication: Option<ScenePublicationResult>,
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
    FramePresented {
        commit_id: u64,
        timestamp: u64,
        refresh_ns: u32,
        seq: u64,
        flags: u32,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresentationSchedulerState {
    Idle,
    DamagePending,
    SubmissionEligible,
    SubmissionInFlight,
    RetryPending,
    PresentationBlocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationCadence {
    pub period_ns: u64,
}

impl PresentationCadence {
    pub fn validate(self) -> Result<(), String> {
        if self.period_ns == 0 {
            return Err("presentation cadence must be non-zero".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PresentationToken {
    pub backend_instance_id: u64,
    pub sequence: u64,
    pub output_id: String,
    pub output_generation: u64,
    pub scene_generation: u64,
    pub frame_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum PresentationCompletion {
    Presented,
    Failed { reason: PublicationFailure },
    Superseded,
    StaleGeneration,
    UnknownToken,
    BackendUnavailable,
}

impl PresentationToken {
    pub fn validate(&self) -> Result<(), String> {
        if self.backend_instance_id == 0 || self.sequence == 0 || self.output_id.is_empty() {
            return Err("malformed presentation token".into());
        }
        if self.frame_id == 0 {
            return Err("presentation token has invalid frame".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceReadiness {
    pub ipc_available: bool,
    pub canonical_scene_available: bool,
    pub renderer_available: bool,
    pub configured_output_count: usize,
    pub ready_output_count: usize,
    pub failed_output_count: usize,
    pub retry_pending_output_count: usize,
    pub state: ServiceReadinessState,
    pub outputs: Vec<OutputReadiness>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceReadinessState {
    LiveNotReady,
    Ready,
    PartiallyReady,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputReadiness {
    pub output_id: String,
    pub output_generation: u64,
    pub state: OutputReadinessState,
    pub last_published_frame_id: u64,
    pub last_published_scene_generation: u64,
    pub pending_damage: bool,
    pub retry_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputReadinessState {
    ConfiguredAwaitingPublication,
    SubmittedAwaitingPresentation,
    Ready,
    PublicationFailed,
    RetryPending,
    Reconfiguring,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum BackendOutputEvent {
    Connected {
        backend_instance_id: u64,
        backend_output_id: String,
        event_sequence: u64,
        geometry: OutputGeometry,
        cadence: PresentationCadence,
    },
    Reconfigured {
        backend_instance_id: u64,
        backend_output_id: String,
        event_sequence: u64,
        geometry: OutputGeometry,
        cadence: PresentationCadence,
    },
    Disabled {
        backend_instance_id: u64,
        backend_output_id: String,
        event_sequence: u64,
        output_generation: u64,
    },
    Disconnected {
        backend_instance_id: u64,
        backend_output_id: String,
        event_sequence: u64,
        output_generation: u64,
    },
    BackendReset {
        backend_instance_id: u64,
        event_sequence: u64,
    },
}

impl ServiceReadiness {
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for output in &self.outputs {
            if !ids.insert(output.output_id.as_str()) {
                return Err(format!("duplicate readiness output {}", output.output_id));
            }
            if output.retry_pending && output.state != OutputReadinessState::RetryPending {
                return Err(format!("retry state mismatch for {}", output.output_id));
            }
            if output.state == OutputReadinessState::Ready
                && (output.retry_pending || output.pending_damage)
            {
                return Err(format!("ready output has pending work {}", output.output_id));
            }
        }
        if self.configured_output_count != self.outputs.len()
            || self.ready_output_count
                != self.outputs.iter().filter(|o| o.state == OutputReadinessState::Ready).count()
            || self.retry_pending_output_count
                != self.outputs.iter().filter(|o| o.retry_pending).count()
            || self.failed_output_count
                != self
                    .outputs
                    .iter()
                    .filter(|o| {
                        matches!(
                            o.state,
                            OutputReadinessState::PublicationFailed
                                | OutputReadinessState::RetryPending
                        )
                    })
                    .count()
        {
            return Err("readiness counts are inconsistent".into());
        }
        if self.state == ServiceReadinessState::Ready && self.ready_output_count == 0 {
            return Err("ready service has no ready output".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenePublicationResult {
    pub scene_accepted: bool,
    pub scene_generation: u64,
    pub outputs: Vec<OutputPublicationResult>,
}

impl ScenePublicationResult {
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for output in &self.outputs {
            if !ids.insert(output.output_id.as_str()) {
                return Err(format!("duplicate publication output {}", output.output_id));
            }
            if output.scene_generation != self.scene_generation {
                return Err(format!(
                    "publication scene generation mismatch for {}",
                    output.output_id
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod publication_result_tests {
    use super::*;

    #[test]
    fn publication_result_round_trips_and_rejects_duplicates() {
        let result = ScenePublicationResult {
            scene_accepted: true,
            scene_generation: 7,
            outputs: vec![
                OutputPublicationResult {
                    output_id: "a".into(),
                    output_generation: 3,
                    frame_id: 4,
                    scene_generation: 7,
                    outcome: OutputPublicationOutcome::Published,
                },
                OutputPublicationResult {
                    output_id: "b".into(),
                    output_generation: 5,
                    frame_id: 2,
                    scene_generation: 7,
                    outcome: OutputPublicationOutcome::Failed(PublicationFailure::BackendRejected),
                },
            ],
        };
        result.validate().unwrap();
        let decoded: ScenePublicationResult =
            serde_json::from_str(&serde_json::to_string(&result).unwrap()).unwrap();
        assert_eq!(decoded, result);
        let mut duplicate = result.clone();
        duplicate.outputs[1].output_id = "a".into();
        assert!(duplicate.validate().is_err());
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    #[test]
    fn readiness_round_trips_and_validates_counts() {
        let value = ServiceReadiness {
            ipc_available: true,
            canonical_scene_available: true,
            renderer_available: true,
            configured_output_count: 1,
            ready_output_count: 1,
            failed_output_count: 0,
            retry_pending_output_count: 0,
            state: ServiceReadinessState::Ready,
            outputs: vec![OutputReadiness {
                output_id: "a".into(),
                output_generation: 4,
                state: OutputReadinessState::Ready,
                last_published_frame_id: 9,
                last_published_scene_generation: 12,
                pending_damage: false,
                retry_pending: false,
            }],
        };
        value.validate().unwrap();
        let decoded: ServiceReadiness =
            serde_json::from_str(&serde_json::to_string(&value).unwrap()).unwrap();
        assert_eq!(decoded, value);
        let mut malformed = value;
        malformed.ready_output_count = 0;
        assert!(malformed.validate().is_err());
    }

    #[test]
    fn zero_outputs_are_live_but_not_ready() {
        let value = ServiceReadiness {
            ipc_available: true,
            canonical_scene_available: true,
            renderer_available: true,
            configured_output_count: 0,
            ready_output_count: 0,
            failed_output_count: 0,
            retry_pending_output_count: 0,
            state: ServiceReadinessState::LiveNotReady,
            outputs: vec![],
        };
        value.validate().unwrap();
        assert_ne!(value.state, ServiceReadinessState::Ready);
    }

    #[test]
    fn presentation_token_and_completion_round_trip() {
        let token = PresentationToken {
            backend_instance_id: 3,
            sequence: 4,
            output_id: "a".into(),
            output_generation: 0,
            scene_generation: 9,
            frame_id: 7,
        };
        token.validate().unwrap();
        let completion =
            PresentationCompletion::Failed { reason: PublicationFailure::BackendRejected };
        let encoded = serde_json::to_string(&(token.clone(), completion.clone())).unwrap();
        let decoded: (PresentationToken, PresentationCompletion) =
            serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, (token, completion));
    }

    #[test]
    fn cadence_rejects_zero_and_scheduler_state_round_trips() {
        assert!(PresentationCadence { period_ns: 0 }.validate().is_err());
        let state = PresentationSchedulerState::SubmissionInFlight;
        let encoded = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<PresentationSchedulerState>(&encoded).unwrap(), state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputPublicationResult {
    pub output_id: String,
    pub output_generation: u64,
    pub frame_id: u64,
    pub scene_generation: u64,
    pub outcome: OutputPublicationOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputPublicationOutcome {
    Published,
    Submitted { token: PresentationToken },
    Failed(PublicationFailure),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicationFailure {
    BackendRejected,
    StaleOutputGeneration,
    StaleRetryState,
    MalformedPublicationState,
    OutputUnavailable,
    RendererRejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
// IPC values remain inline to preserve the established wire shape and public API.
#[allow(clippy::large_enum_variant)]
pub enum WaylandCommand {
    GetSurfaceRegistry,
    ApplySelectionHandoff {
        handoff: WaylandSelectionHandoff,
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
    StartDrag {
        source_id: String,
        surface_id: String,
        mime_types: Vec<String>,
    },
    DragEnter {
        surface_id: String,
        x: f64,
        y: f64,
        mime_types: Vec<String>,
    },
    DragMotion {
        surface_id: String,
        x: f64,
        y: f64,
        time: u32,
    },
    DragDrop,
    DragLeave,
    DragCancel,
    WriteData {
        source_id: String,
        mime_type: String,
        data: Vec<u8>,
    },
    ReadData {
        source_id: String,
        mime_type: String,
    },
    InjectRelativePointerMotion {
        surface_id: String,
        dx: f64,
        dy: f64,
        dx_unaccel: f64,
        dy_unaccel: f64,
        timestamp: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    RelativePointerMotion {
        surface_id: String,
        dx: f64,
        dy: f64,
        dx_unaccel: f64,
        dy_unaccel: f64,
        timestamp: u64,
    },
    DragStarted {
        source_id: String,
    },
    DragEntered {
        surface_id: String,
    },
    DragMotioned {
        surface_id: String,
    },
    DragDropped,
    DragLeft,
    DragCancelled,
    DataRead {
        source_id: String,
        mime_type: String,
        data: Option<Vec<u8>>,
    },
    Rejected {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum LockCommand {
    SetLockState { state: LockState },
    AuthPrompt { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum HealthState {
    Healthy { role: ServiceRole },
    Unhealthy { role: ServiceRole, reason: String, crash_loop_count: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum CommitTarget {
    Output { name: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayReconciliationState {
    Recovering,
    ReconciledAwaitingPresentation,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputMode {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommittedSceneState {
    pub source: ServiceRole,
    pub target: CommitTarget,
    pub focus: FocusTarget,
    #[serde(default)]
    pub selection: WaylandSelectionState,
    pub surfaces: Vec<SurfaceSnapshot>,
    #[serde(default)]
    pub scene_epoch: u64,
    #[serde(default)]
    pub scene_generation: u64,
    #[serde(default)]
    pub commit_id: u64,
    pub unix_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForeignToplevelHandle {
    pub id: String,
    pub title: String,
    pub app_id: String,
    pub activated: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceRegistrySnapshot {
    pub generation: u64,
    pub surfaces: Vec<WaylandSurfaceState>,
    #[serde(default)]
    pub foreign_toplevels: Vec<ForeignToplevelHandle>,
    #[serde(default)]
    pub selection: WaylandSelectionState,
    pub unix_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionOffer {
    pub offer_id: String,
    pub source_id: String,
    pub owner_surface_id: String,
    pub mime_types: Vec<String>,
    pub serial: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WaylandSelectionState {
    #[serde(default)]
    pub clipboard_owner: Option<String>,
    #[serde(default)]
    pub clipboard_payload_id: Option<String>,
    #[serde(default)]
    pub clipboard_source_serial: Option<u64>,
    #[serde(default)]
    pub clipboard_offer: Option<SelectionOffer>,
    #[serde(default)]
    pub primary_selection_owner: Option<String>,
    #[serde(default)]
    pub primary_selection_payload_id: Option<String>,
    #[serde(default)]
    pub primary_selection_source_serial: Option<u64>,
    #[serde(default)]
    pub primary_offer: Option<SelectionOffer>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaylandSelectionHandoff {
    pub focus: FocusTarget,
    pub selection: WaylandSelectionState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WaylandSurfaceState {
    pub id: String,
    pub app_id: String,
    pub role: WaylandSurfaceRole,
    pub mapped: bool,
    pub buffer_attached: bool,
    #[serde(default)]
    pub buffer_handle: Option<String>,
    #[serde(default)]
    pub buffer_generation: u64,
    #[serde(default)]
    pub damage_rects: Vec<Rect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LayerMetadata {
    pub layer: u32,
    pub anchor: u32,
    pub exclusive_zone: i32,
    pub margin_top: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
    pub margin_right: i32,
    pub keyboard_interactivity: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum WaylandSurfaceRole {
    #[default]
    Toplevel,
    Popup,
    Layer(LayerMetadata),
    Background,
    Lock,
    Cursor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SurfaceSnapshot {
    pub id: String,
    pub app_id: String,
    pub placement: SurfacePlacement,
    #[serde(default)]
    pub buffer_handle: Option<String>,
    #[serde(default)]
    pub buffer_generation: u64,
    #[serde(default)]
    pub damage_rects: Vec<Rect>,
    #[serde(default)]
    pub pixel_transport: Option<crate::pixel_transport::PixelTransportHandle>,
    #[serde(default)]
    pub layer_class: u32,
    #[serde(default)]
    pub creation_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SurfacePlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub z: i32,
    pub visible: bool,
}
