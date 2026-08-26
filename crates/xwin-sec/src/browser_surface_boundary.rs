use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BrowserRuntimeFamily {
    Chrome,
    Chromium,
    Electron,
    ChromiumBasedBrowser,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BrowserRuntimePosture {
    KairoAllowed,
    KairoObserve,
    KairoQuarantine,
    KairoFailClosed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BrowserWindowRole {
    NormalBrowserWindow,
    WebAppWindow,
    ElectronAppWindow,
    KioskWindow,
    HeadlessSurface,
    ExtensionPopup,
    FilePickerDialog,
    PermissionPrompt,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BrowserSurfaceState {
    NewWindow,
    Focused,
    Background,
    Fullscreen,
    AlwaysOnTop,
    ScreenCaptureRequested,
    ClipboardAccessRequested,
    DragDropRequested,
    FilePickerRequested,
    NativeMessagingRequested,
    GpuSurfaceRequested,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BrowserClipboardPolicy {
    None,
    ReadRequested,
    WriteRequested,
    ReadWriteRequested,
    SensitiveClipboardPresent,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BrowserFileBoundaryPolicy {
    None,
    FilePickerReadRequested,
    FilePickerWriteRequested,
    DirectoryPickerRequested,
    DragDropFileInbound,
    DragDropFileOutbound,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BrowserInputBoundaryPolicy {
    NormalInput,
    GlobalShortcutRequested,
    RawKeyboardRequested,
    PointerLockRequested,
    InputCaptureRequested,
    ImeBoundaryUnknown,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BrowserGpuBoundaryPolicy {
    NormalCompositedSurface,
    GpuAcceleratedSurface,
    SharedGpuContextRequested,
    DirectScanoutRequested,
    ScreenCaptureOrMirroringRequested,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BrowserExtensionOrNativePolicy {
    None,
    ExtensionSurface,
    NativeMessagingRequested,
    ExternalProtocolHandlerRequested,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BrowserOperatorOverrides {
    pub operator_confirmed_trusted_browser_surface: bool,
    pub operator_confirmed_isolated_profile: bool,
    pub operator_confirmed_kiosk_mode: bool,
    pub operator_confirmed_clipboard_allowed: bool,
    pub operator_confirmed_file_picker_allowed: bool,
    pub operator_confirmed_native_messaging_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BrowserSurfaceBoundaryContext {
    pub runtime_family: BrowserRuntimeFamily,
    pub runtime_posture: BrowserRuntimePosture,
    pub window_role: BrowserWindowRole,
    pub surface_state: BrowserSurfaceState,
    pub clipboard_policy: BrowserClipboardPolicy,
    pub file_boundary_policy: BrowserFileBoundaryPolicy,
    pub input_boundary_policy: BrowserInputBoundaryPolicy,
    pub gpu_boundary_policy: BrowserGpuBoundaryPolicy,
    pub extension_or_native_policy: BrowserExtensionOrNativePolicy,
    pub operator_overrides: BrowserOperatorOverrides,
}

impl BrowserSurfaceBoundaryContext {
    pub fn new(
        runtime_family: BrowserRuntimeFamily,
        runtime_posture: BrowserRuntimePosture,
        window_role: BrowserWindowRole,
    ) -> Self {
        Self { runtime_family, runtime_posture, window_role, ..Self::default() }
    }

    pub fn with_surface_state(mut self, surface_state: BrowserSurfaceState) -> Self {
        self.surface_state = surface_state;
        self
    }

    pub fn with_clipboard_policy(mut self, clipboard_policy: BrowserClipboardPolicy) -> Self {
        self.clipboard_policy = clipboard_policy;
        self
    }

    pub fn with_file_boundary_policy(
        mut self,
        file_boundary_policy: BrowserFileBoundaryPolicy,
    ) -> Self {
        self.file_boundary_policy = file_boundary_policy;
        self
    }

    pub fn with_input_boundary_policy(
        mut self,
        input_boundary_policy: BrowserInputBoundaryPolicy,
    ) -> Self {
        self.input_boundary_policy = input_boundary_policy;
        self
    }

    pub fn with_gpu_boundary_policy(
        mut self,
        gpu_boundary_policy: BrowserGpuBoundaryPolicy,
    ) -> Self {
        self.gpu_boundary_policy = gpu_boundary_policy;
        self
    }

    pub fn with_extension_or_native_policy(
        mut self,
        extension_or_native_policy: BrowserExtensionOrNativePolicy,
    ) -> Self {
        self.extension_or_native_policy = extension_or_native_policy;
        self
    }

    pub fn with_operator_overrides(mut self, operator_overrides: BrowserOperatorOverrides) -> Self {
        self.operator_overrides = operator_overrides;
        self
    }

    pub fn is_browser_surface(&self) -> bool {
        !matches!(self.runtime_family, BrowserRuntimeFamily::Unknown)
            || !matches!(self.window_role, BrowserWindowRole::Unknown)
    }

    fn has_clipboard_override(&self) -> bool {
        self.operator_overrides.operator_confirmed_clipboard_allowed
    }

    fn has_file_override(&self) -> bool {
        self.operator_overrides.operator_confirmed_file_picker_allowed
    }

    fn has_native_override(&self) -> bool {
        self.operator_overrides.operator_confirmed_native_messaging_allowed
    }

    fn request_is_explicit_high_risk(&self) -> bool {
        self.is_sensitive_clipboard_request()
            || self.is_file_boundary_request_without_override()
            || self.is_input_capture_request()
            || self.is_native_messaging_request_without_override()
            || self.is_gpu_high_risk_request()
    }

    fn is_sensitive_clipboard_request(&self) -> bool {
        matches!(self.clipboard_policy, BrowserClipboardPolicy::SensitiveClipboardPresent)
            && !self.has_clipboard_override()
    }

    fn is_file_boundary_request(&self) -> bool {
        matches!(
            self.file_boundary_policy,
            BrowserFileBoundaryPolicy::FilePickerReadRequested
                | BrowserFileBoundaryPolicy::FilePickerWriteRequested
                | BrowserFileBoundaryPolicy::DirectoryPickerRequested
                | BrowserFileBoundaryPolicy::DragDropFileInbound
                | BrowserFileBoundaryPolicy::DragDropFileOutbound
        )
    }

    fn is_file_boundary_request_without_override(&self) -> bool {
        self.is_file_boundary_request() && !self.has_file_override()
    }

    fn is_input_capture_request(&self) -> bool {
        matches!(
            self.input_boundary_policy,
            BrowserInputBoundaryPolicy::GlobalShortcutRequested
                | BrowserInputBoundaryPolicy::RawKeyboardRequested
                | BrowserInputBoundaryPolicy::PointerLockRequested
                | BrowserInputBoundaryPolicy::InputCaptureRequested
        )
    }

    fn is_native_messaging_request(&self) -> bool {
        matches!(
            self.extension_or_native_policy,
            BrowserExtensionOrNativePolicy::NativeMessagingRequested
                | BrowserExtensionOrNativePolicy::ExternalProtocolHandlerRequested
        )
    }

    fn is_native_messaging_request_without_override(&self) -> bool {
        self.is_native_messaging_request() && !self.has_native_override()
    }

    fn is_gpu_high_risk_request(&self) -> bool {
        matches!(
            self.gpu_boundary_policy,
            BrowserGpuBoundaryPolicy::SharedGpuContextRequested
                | BrowserGpuBoundaryPolicy::DirectScanoutRequested
                | BrowserGpuBoundaryPolicy::ScreenCaptureOrMirroringRequested
        )
    }

    fn is_unknown_high_risk(&self) -> bool {
        matches!(
            self.runtime_family,
            BrowserRuntimeFamily::Chrome
                | BrowserRuntimeFamily::Chromium
                | BrowserRuntimeFamily::Electron
                | BrowserRuntimeFamily::ChromiumBasedBrowser
        ) && matches!(self.runtime_posture, BrowserRuntimePosture::Unknown)
            && matches!(self.surface_state, BrowserSurfaceState::Unknown)
            && matches!(self.clipboard_policy, BrowserClipboardPolicy::Unknown)
            && matches!(self.file_boundary_policy, BrowserFileBoundaryPolicy::Unknown)
            && matches!(self.input_boundary_policy, BrowserInputBoundaryPolicy::Unknown)
            && matches!(self.gpu_boundary_policy, BrowserGpuBoundaryPolicy::Unknown)
    }

    fn allow_surface_conditions_met(&self) -> bool {
        matches!(self.runtime_posture, BrowserRuntimePosture::KairoAllowed)
            && matches!(
                self.window_role,
                BrowserWindowRole::NormalBrowserWindow
                    | BrowserWindowRole::WebAppWindow
                    | BrowserWindowRole::ElectronAppWindow
            )
            && matches!(
                self.surface_state,
                BrowserSurfaceState::NewWindow
                    | BrowserSurfaceState::Focused
                    | BrowserSurfaceState::Background
            )
            && !self.request_is_explicit_high_risk()
    }

    fn observe_surface_conditions_met(&self) -> bool {
        matches!(self.runtime_posture, BrowserRuntimePosture::KairoObserve)
            && self.is_browser_surface()
            && !self.request_is_explicit_high_risk()
    }

    fn constrain_surface_conditions_met(&self) -> bool {
        matches!(
            self.runtime_posture,
            BrowserRuntimePosture::KairoQuarantine | BrowserRuntimePosture::Unknown
        ) && self.is_browser_surface()
            && !self.request_is_explicit_high_risk()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrowserSurfaceBoundaryAction {
    Allow,
    Constrain,
    Quarantine,
    FailClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrowserSurfaceBoundaryDecisionState {
    NotApplicable,
    AllowSurface,
    ObserveSurface,
    ConstrainSurface,
    ClipboardQuarantine,
    FileBoundaryQuarantine,
    InputCaptureFailClosed,
    NativeMessagingFailClosed,
    GpuSurfaceFailClosed,
    KairoFailClosedPropagated,
    UnknownFailClosed,
}

impl BrowserSurfaceBoundaryDecisionState {
    pub const fn action(self) -> BrowserSurfaceBoundaryAction {
        match self {
            Self::NotApplicable | Self::AllowSurface | Self::ObserveSurface => {
                BrowserSurfaceBoundaryAction::Allow
            }
            Self::ConstrainSurface => BrowserSurfaceBoundaryAction::Constrain,
            Self::ClipboardQuarantine | Self::FileBoundaryQuarantine => {
                BrowserSurfaceBoundaryAction::Quarantine
            }
            Self::InputCaptureFailClosed
            | Self::NativeMessagingFailClosed
            | Self::GpuSurfaceFailClosed
            | Self::KairoFailClosedPropagated
            | Self::UnknownFailClosed => BrowserSurfaceBoundaryAction::FailClosed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSurfaceBoundaryFinding {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSurfaceBoundaryDecision {
    pub state: BrowserSurfaceBoundaryDecisionState,
    pub action: BrowserSurfaceBoundaryAction,
    pub reason: String,
    pub findings: Vec<BrowserSurfaceBoundaryFinding>,
}

impl BrowserSurfaceBoundaryDecision {
    pub fn new(
        state: BrowserSurfaceBoundaryDecisionState,
        reason: impl Into<String>,
        findings: Vec<BrowserSurfaceBoundaryFinding>,
    ) -> Self {
        Self { state, action: state.action(), reason: reason.into(), findings }
    }
}

fn finding(key: &'static str, value: impl Into<String>) -> BrowserSurfaceBoundaryFinding {
    BrowserSurfaceBoundaryFinding { key: key.to_string(), value: value.into() }
}

pub fn evaluate_browser_surface_boundary(
    context: &BrowserSurfaceBoundaryContext,
) -> BrowserSurfaceBoundaryDecision {
    if matches!(context.runtime_posture, BrowserRuntimePosture::KairoFailClosed) {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::KairoFailClosedPropagated,
            "KAIRO posture requires fail-closed browser surface handling",
            vec![
                finding("runtime_family", format!("{:?}", context.runtime_family)),
                finding("runtime_posture", format!("{:?}", context.runtime_posture)),
                finding("window_role", format!("{:?}", context.window_role)),
            ],
        );
    }

    if !context.is_browser_surface() {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::NotApplicable,
            "no browser surface markers were supplied",
            vec![
                finding("runtime_family", format!("{:?}", context.runtime_family)),
                finding("window_role", format!("{:?}", context.window_role)),
            ],
        );
    }

    if context.is_unknown_high_risk() {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::UnknownFailClosed,
            "browser surface posture is too underspecified for safe handling",
            vec![
                finding("runtime_family", format!("{:?}", context.runtime_family)),
                finding("runtime_posture", format!("{:?}", context.runtime_posture)),
                finding("surface_state", format!("{:?}", context.surface_state)),
                finding("clipboard_policy", format!("{:?}", context.clipboard_policy)),
                finding("file_boundary_policy", format!("{:?}", context.file_boundary_policy)),
                finding("input_boundary_policy", format!("{:?}", context.input_boundary_policy)),
                finding("gpu_boundary_policy", format!("{:?}", context.gpu_boundary_policy)),
            ],
        );
    }

    if context.is_sensitive_clipboard_request() {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::ClipboardQuarantine,
            "sensitive clipboard access requested without an explicit operator override",
            vec![
                finding("clipboard_policy", format!("{:?}", context.clipboard_policy)),
                finding(
                    "operator_confirmed_clipboard_allowed",
                    context.operator_overrides.operator_confirmed_clipboard_allowed.to_string(),
                ),
            ],
        );
    }

    if context.is_file_boundary_request_without_override() {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::FileBoundaryQuarantine,
            "file picker or drag-and-drop request requires an explicit operator override",
            vec![
                finding("file_boundary_policy", format!("{:?}", context.file_boundary_policy)),
                finding(
                    "operator_confirmed_file_picker_allowed",
                    context.operator_overrides.operator_confirmed_file_picker_allowed.to_string(),
                ),
            ],
        );
    }

    if context.is_input_capture_request()
        && matches!(
            context.runtime_posture,
            BrowserRuntimePosture::KairoQuarantine
                | BrowserRuntimePosture::KairoObserve
                | BrowserRuntimePosture::Unknown
        )
    {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::InputCaptureFailClosed,
            "raw keyboard, shortcut, pointer lock, or input capture request is not allowed in constrained posture",
            vec![finding("input_boundary_policy", format!("{:?}", context.input_boundary_policy))],
        );
    }

    if context.is_native_messaging_request_without_override() {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::NativeMessagingFailClosed,
            "native messaging or external protocol handling requires an explicit operator override",
            vec![
                finding(
                    "extension_or_native_policy",
                    format!("{:?}", context.extension_or_native_policy),
                ),
                finding(
                    "operator_confirmed_native_messaging_allowed",
                    context
                        .operator_overrides
                        .operator_confirmed_native_messaging_allowed
                        .to_string(),
                ),
            ],
        );
    }

    if context.is_gpu_high_risk_request()
        && matches!(
            context.runtime_posture,
            BrowserRuntimePosture::KairoQuarantine
                | BrowserRuntimePosture::KairoObserve
                | BrowserRuntimePosture::Unknown
        )
    {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::GpuSurfaceFailClosed,
            "GPU sharing, direct scanout, or screen mirroring is not allowed in constrained posture",
            vec![finding("gpu_boundary_policy", format!("{:?}", context.gpu_boundary_policy))],
        );
    }

    if context.allow_surface_conditions_met() {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::AllowSurface,
            "browser surface is allowed under explicit safe posture and no high-risk request is present",
            vec![
                finding("runtime_posture", format!("{:?}", context.runtime_posture)),
                finding("window_role", format!("{:?}", context.window_role)),
                finding("surface_state", format!("{:?}", context.surface_state)),
            ],
        );
    }

    if context.observe_surface_conditions_met() {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::ConstrainSurface,
            "browser surface is only eligible for observation in this posture and remains constrained",
            vec![
                finding("runtime_posture", format!("{:?}", context.runtime_posture)),
                finding("window_role", format!("{:?}", context.window_role)),
                finding("surface_state", format!("{:?}", context.surface_state)),
            ],
        );
    }

    if context.constrain_surface_conditions_met() {
        return BrowserSurfaceBoundaryDecision::new(
            BrowserSurfaceBoundaryDecisionState::ConstrainSurface,
            "browser surface should be constrained until a stronger posture or explicit override is supplied",
            vec![
                finding("runtime_posture", format!("{:?}", context.runtime_posture)),
                finding("window_role", format!("{:?}", context.window_role)),
                finding("surface_state", format!("{:?}", context.surface_state)),
                finding(
                    "operator_confirmed_trusted_browser_surface",
                    context
                        .operator_overrides
                        .operator_confirmed_trusted_browser_surface
                        .to_string(),
                ),
                finding(
                    "operator_confirmed_isolated_profile",
                    context.operator_overrides.operator_confirmed_isolated_profile.to_string(),
                ),
                finding(
                    "operator_confirmed_kiosk_mode",
                    context.operator_overrides.operator_confirmed_kiosk_mode.to_string(),
                ),
            ],
        );
    }

    BrowserSurfaceBoundaryDecision::new(
        BrowserSurfaceBoundaryDecisionState::ConstrainSurface,
        "browser surface posture is incomplete; constrain by default",
        vec![
            finding("runtime_family", format!("{:?}", context.runtime_family)),
            finding("runtime_posture", format!("{:?}", context.runtime_posture)),
            finding("window_role", format!("{:?}", context.window_role)),
        ],
    )
}
