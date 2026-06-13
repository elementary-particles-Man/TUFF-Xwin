use serde::{Deserialize, Serialize};

use crate::browser_surface_boundary::{
    BrowserClipboardPolicy, BrowserExtensionOrNativePolicy, BrowserFileBoundaryPolicy,
    BrowserGpuBoundaryPolicy, BrowserRuntimeFamily, BrowserRuntimePosture, BrowserWindowRole,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgeAssuranceBrowserSurfaceState {
    NewWindow,
    Focused,
    Background,
    Fullscreen,
    AlwaysOnTop,
    HeadlessSurface,
    AgeVerifierIframe,
    PlatformAgeSignalPrompt,
    IdentityProviderPopup,
    BrowserProfileSwitchPrompt,
    FilePickerForIdUpload,
    CameraOrBiometricPrompt,
    ClipboardReadRequest,
    DragAndDropIdDocument,
    NativeMessagingBridge,
    ExternalProtocolHandler,
    ScreenCaptureOrMirroring,
    SharedGpuSurface,
    Unknown,
}

impl Default for AgeAssuranceBrowserSurfaceState {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgeAssuranceBrowserOperatorOverrides {
    pub operator_confirmed_trusted_browser_surface: bool,
    pub operator_confirmed_isolated_profile: bool,
    pub operator_confirmed_kiosk_mode: bool,
    pub operator_confirmed_age_signal_allowed: bool,
    pub operator_confirmed_identity_upload_allowed: bool,
    pub operator_confirmed_biometric_prompt_allowed: bool,
    pub operator_confirmed_clipboard_allowed: bool,
    pub operator_confirmed_file_picker_allowed: bool,
    pub operator_confirmed_native_messaging_allowed: bool,
    pub operator_confirmed_external_protocol_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgeAssuranceBrowserSurfaceBoundaryContext {
    pub runtime_family: BrowserRuntimeFamily,
    pub runtime_posture: BrowserRuntimePosture,
    pub window_role: BrowserWindowRole,
    pub surface_state: AgeAssuranceBrowserSurfaceState,
    pub clipboard_policy: BrowserClipboardPolicy,
    pub file_boundary_policy: BrowserFileBoundaryPolicy,
    pub extension_or_native_policy: BrowserExtensionOrNativePolicy,
    pub gpu_boundary_policy: BrowserGpuBoundaryPolicy,
    pub operator_overrides: AgeAssuranceBrowserOperatorOverrides,
}

impl Default for AgeAssuranceBrowserSurfaceBoundaryContext {
    fn default() -> Self {
        Self {
            runtime_family: BrowserRuntimeFamily::default(),
            runtime_posture: BrowserRuntimePosture::default(),
            window_role: BrowserWindowRole::default(),
            surface_state: AgeAssuranceBrowserSurfaceState::default(),
            clipboard_policy: BrowserClipboardPolicy::default(),
            file_boundary_policy: BrowserFileBoundaryPolicy::default(),
            extension_or_native_policy: BrowserExtensionOrNativePolicy::default(),
            gpu_boundary_policy: BrowserGpuBoundaryPolicy::default(),
            operator_overrides: AgeAssuranceBrowserOperatorOverrides::default(),
        }
    }
}

impl AgeAssuranceBrowserSurfaceBoundaryContext {
    pub fn new(
        runtime_family: BrowserRuntimeFamily,
        runtime_posture: BrowserRuntimePosture,
        window_role: BrowserWindowRole,
    ) -> Self {
        Self { runtime_family, runtime_posture, window_role, ..Self::default() }
    }

    pub fn with_surface_state(mut self, surface_state: AgeAssuranceBrowserSurfaceState) -> Self {
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

    pub fn with_extension_or_native_policy(
        mut self,
        extension_or_native_policy: BrowserExtensionOrNativePolicy,
    ) -> Self {
        self.extension_or_native_policy = extension_or_native_policy;
        self
    }

    pub fn with_gpu_boundary_policy(
        mut self,
        gpu_boundary_policy: BrowserGpuBoundaryPolicy,
    ) -> Self {
        self.gpu_boundary_policy = gpu_boundary_policy;
        self
    }

    pub fn with_operator_overrides(
        mut self,
        operator_overrides: AgeAssuranceBrowserOperatorOverrides,
    ) -> Self {
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

    fn has_external_protocol_override(&self) -> bool {
        self.operator_overrides.operator_confirmed_external_protocol_allowed
    }

    fn has_age_signal_override(&self) -> bool {
        self.operator_overrides.operator_confirmed_age_signal_allowed
    }

    fn has_identity_upload_override(&self) -> bool {
        self.operator_overrides.operator_confirmed_identity_upload_allowed
    }

    fn has_biometric_prompt_override(&self) -> bool {
        self.operator_overrides.operator_confirmed_biometric_prompt_allowed
    }

    fn is_age_signal_surface(&self) -> bool {
        matches!(
            self.surface_state,
            AgeAssuranceBrowserSurfaceState::AgeVerifierIframe
                | AgeAssuranceBrowserSurfaceState::PlatformAgeSignalPrompt
                | AgeAssuranceBrowserSurfaceState::IdentityProviderPopup
                | AgeAssuranceBrowserSurfaceState::BrowserProfileSwitchPrompt
        )
    }

    fn is_identity_upload_surface(&self) -> bool {
        matches!(self.surface_state, AgeAssuranceBrowserSurfaceState::FilePickerForIdUpload)
    }

    fn is_biometric_prompt_surface(&self) -> bool {
        matches!(self.surface_state, AgeAssuranceBrowserSurfaceState::CameraOrBiometricPrompt)
    }

    fn is_clipboard_request(&self) -> bool {
        matches!(
            self.clipboard_policy,
            BrowserClipboardPolicy::ReadRequested
                | BrowserClipboardPolicy::WriteRequested
                | BrowserClipboardPolicy::ReadWriteRequested
                | BrowserClipboardPolicy::SensitiveClipboardPresent
        )
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

    fn is_native_messaging_request(&self) -> bool {
        matches!(
            self.extension_or_native_policy,
            BrowserExtensionOrNativePolicy::NativeMessagingRequested
                | BrowserExtensionOrNativePolicy::ExternalProtocolHandlerRequested
        )
    }

    fn is_screen_capture_or_gpu_request(&self) -> bool {
        matches!(
            self.surface_state,
            AgeAssuranceBrowserSurfaceState::ScreenCaptureOrMirroring
                | AgeAssuranceBrowserSurfaceState::SharedGpuSurface
        ) || matches!(
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
            && matches!(self.window_role, BrowserWindowRole::Unknown)
            && matches!(self.surface_state, AgeAssuranceBrowserSurfaceState::Unknown)
            && matches!(self.clipboard_policy, BrowserClipboardPolicy::Unknown)
            && matches!(self.file_boundary_policy, BrowserFileBoundaryPolicy::Unknown)
            && matches!(self.extension_or_native_policy, BrowserExtensionOrNativePolicy::Unknown)
            && matches!(self.gpu_boundary_policy, BrowserGpuBoundaryPolicy::Unknown)
    }

    fn allow_surface_conditions_met(&self) -> bool {
        matches!(self.runtime_posture, BrowserRuntimePosture::KairoAllowed)
            && matches!(
                self.window_role,
                BrowserWindowRole::NormalBrowserWindow
                    | BrowserWindowRole::WebAppWindow
                    | BrowserWindowRole::ElectronAppWindow
                    | BrowserWindowRole::KioskWindow
            )
            && matches!(
                self.surface_state,
                AgeAssuranceBrowserSurfaceState::NewWindow
                    | AgeAssuranceBrowserSurfaceState::Focused
                    | AgeAssuranceBrowserSurfaceState::Background
                    | AgeAssuranceBrowserSurfaceState::Fullscreen
                    | AgeAssuranceBrowserSurfaceState::AlwaysOnTop
                    | AgeAssuranceBrowserSurfaceState::HeadlessSurface
            )
            && !self.is_clipboard_request()
            && !self.is_file_boundary_request()
            && !self.is_native_messaging_request()
            && !self.is_screen_capture_or_gpu_request()
            && !self.is_identity_upload_surface()
            && !self.is_biometric_prompt_surface()
            && !self.is_age_signal_surface()
    }

    fn age_signal_observe_conditions_met(&self) -> bool {
        matches!(
            self.runtime_posture,
            BrowserRuntimePosture::KairoAllowed | BrowserRuntimePosture::KairoObserve
        ) && self.is_browser_surface()
            && self.is_age_signal_surface()
            && !self.has_age_signal_override()
            && !self.is_clipboard_request()
            && !self.is_file_boundary_request()
            && !self.is_native_messaging_request()
            && !self.is_screen_capture_or_gpu_request()
            && !self.is_identity_upload_surface()
            && !self.is_biometric_prompt_surface()
    }

    fn constrain_surface_conditions_met(&self) -> bool {
        matches!(
            self.runtime_posture,
            BrowserRuntimePosture::KairoQuarantine | BrowserRuntimePosture::Unknown
        ) && self.is_browser_surface()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgeAssuranceBrowserSurfaceBoundaryAction {
    Allow,
    Constrain,
    Quarantine,
    FailClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgeAssuranceBrowserSurfaceBoundaryDecisionState {
    NotApplicable,
    AllowSurface,
    ConstrainSurface,
    AgeSignalObserve,
    ClipboardQuarantine,
    FilePickerQuarantine,
    IdentityUploadFailClosed,
    BiometricPromptFailClosed,
    NativeMessagingFailClosed,
    ExternalProtocolFailClosed,
    ScreenCaptureFailClosed,
    KairoFailClosedPropagated,
    UnknownFailClosed,
}

impl AgeAssuranceBrowserSurfaceBoundaryDecisionState {
    pub const fn action(self) -> AgeAssuranceBrowserSurfaceBoundaryAction {
        match self {
            Self::NotApplicable | Self::AllowSurface | Self::AgeSignalObserve => {
                AgeAssuranceBrowserSurfaceBoundaryAction::Allow
            }
            Self::ConstrainSurface => AgeAssuranceBrowserSurfaceBoundaryAction::Constrain,
            Self::ClipboardQuarantine | Self::FilePickerQuarantine => {
                AgeAssuranceBrowserSurfaceBoundaryAction::Quarantine
            }
            Self::IdentityUploadFailClosed
            | Self::BiometricPromptFailClosed
            | Self::NativeMessagingFailClosed
            | Self::ExternalProtocolFailClosed
            | Self::ScreenCaptureFailClosed
            | Self::KairoFailClosedPropagated
            | Self::UnknownFailClosed => AgeAssuranceBrowserSurfaceBoundaryAction::FailClosed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgeAssuranceBrowserSurfaceBoundaryFinding {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgeAssuranceBrowserSurfaceBoundaryDecision {
    pub state: AgeAssuranceBrowserSurfaceBoundaryDecisionState,
    pub action: AgeAssuranceBrowserSurfaceBoundaryAction,
    pub reason: String,
    pub findings: Vec<AgeAssuranceBrowserSurfaceBoundaryFinding>,
}

impl AgeAssuranceBrowserSurfaceBoundaryDecision {
    pub fn new(
        state: AgeAssuranceBrowserSurfaceBoundaryDecisionState,
        reason: impl Into<String>,
        findings: Vec<AgeAssuranceBrowserSurfaceBoundaryFinding>,
    ) -> Self {
        Self { state, action: state.action(), reason: reason.into(), findings }
    }
}

fn finding(
    key: &'static str,
    value: impl Into<String>,
) -> AgeAssuranceBrowserSurfaceBoundaryFinding {
    AgeAssuranceBrowserSurfaceBoundaryFinding { key: key.to_string(), value: value.into() }
}

pub fn evaluate_age_assurance_browser_surface_boundary(
    context: &AgeAssuranceBrowserSurfaceBoundaryContext,
) -> AgeAssuranceBrowserSurfaceBoundaryDecision {
    if matches!(context.runtime_posture, BrowserRuntimePosture::KairoFailClosed) {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::KairoFailClosedPropagated,
            "KAIRO posture requires fail-closed browser surface handling",
            vec![
                finding("runtime_family", format!("{:?}", context.runtime_family)),
                finding("runtime_posture", format!("{:?}", context.runtime_posture)),
                finding("window_role", format!("{:?}", context.window_role)),
            ],
        );
    }

    if !context.is_browser_surface() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::NotApplicable,
            "no browser surface markers were supplied",
            vec![
                finding("runtime_family", format!("{:?}", context.runtime_family)),
                finding("window_role", format!("{:?}", context.window_role)),
            ],
        );
    }

    if context.is_unknown_high_risk() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::UnknownFailClosed,
            "browser surface posture is too underspecified for safe handling",
            vec![
                finding("runtime_family", format!("{:?}", context.runtime_family)),
                finding("runtime_posture", format!("{:?}", context.runtime_posture)),
                finding("window_role", format!("{:?}", context.window_role)),
                finding("surface_state", format!("{:?}", context.surface_state)),
                finding("clipboard_policy", format!("{:?}", context.clipboard_policy)),
                finding("file_boundary_policy", format!("{:?}", context.file_boundary_policy)),
                finding(
                    "extension_or_native_policy",
                    format!("{:?}", context.extension_or_native_policy),
                ),
                finding("gpu_boundary_policy", format!("{:?}", context.gpu_boundary_policy)),
            ],
        );
    }

    if context.is_identity_upload_surface() && !context.has_identity_upload_override() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::IdentityUploadFailClosed,
            "government ID upload requests are fail-closed without an explicit local policy override",
            vec![finding("surface_state", format!("{:?}", context.surface_state))],
        );
    }

    if context.is_identity_upload_surface() && context.has_identity_upload_override() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::AllowSurface,
            "government ID upload request is explicitly allowed by local policy override",
            vec![
                finding("surface_state", format!("{:?}", context.surface_state)),
                finding(
                    "operator_confirmed_identity_upload_allowed",
                    context
                        .operator_overrides
                        .operator_confirmed_identity_upload_allowed
                        .to_string(),
                ),
            ],
        );
    }

    if context.is_biometric_prompt_surface() && !context.has_biometric_prompt_override() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::BiometricPromptFailClosed,
            "camera or biometric prompts are fail-closed without an explicit local policy override",
            vec![finding("surface_state", format!("{:?}", context.surface_state))],
        );
    }

    if context.is_biometric_prompt_surface() && context.has_biometric_prompt_override() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::AllowSurface,
            "camera or biometric prompt is explicitly allowed by local policy override",
            vec![
                finding("surface_state", format!("{:?}", context.surface_state)),
                finding(
                    "operator_confirmed_biometric_prompt_allowed",
                    context
                        .operator_overrides
                        .operator_confirmed_biometric_prompt_allowed
                        .to_string(),
                ),
            ],
        );
    }

    if context.is_clipboard_request() && !context.has_clipboard_override() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::ClipboardQuarantine,
            "clipboard access is quarantined for age-assurance browser surfaces without an explicit operator override",
            vec![
                finding("clipboard_policy", format!("{:?}", context.clipboard_policy)),
                finding(
                    "operator_confirmed_clipboard_allowed",
                    context.operator_overrides.operator_confirmed_clipboard_allowed.to_string(),
                ),
            ],
        );
    }

    if context.is_clipboard_request() && context.has_clipboard_override() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::AllowSurface,
            "clipboard access is explicitly allowed by local policy override",
            vec![
                finding("clipboard_policy", format!("{:?}", context.clipboard_policy)),
                finding(
                    "operator_confirmed_clipboard_allowed",
                    context.operator_overrides.operator_confirmed_clipboard_allowed.to_string(),
                ),
            ],
        );
    }

    if context.is_file_boundary_request() && !context.has_file_override() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::FilePickerQuarantine,
            "file picker or drag-and-drop requests are quarantined for age-assurance browser surfaces without an explicit operator override",
            vec![
                finding("file_boundary_policy", format!("{:?}", context.file_boundary_policy)),
                finding(
                    "operator_confirmed_file_picker_allowed",
                    context.operator_overrides.operator_confirmed_file_picker_allowed.to_string(),
                ),
            ],
        );
    }

    if context.is_file_boundary_request() && context.has_file_override() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::AllowSurface,
            "file picker or drag-and-drop request is explicitly allowed by local policy override",
            vec![
                finding("file_boundary_policy", format!("{:?}", context.file_boundary_policy)),
                finding(
                    "operator_confirmed_file_picker_allowed",
                    context.operator_overrides.operator_confirmed_file_picker_allowed.to_string(),
                ),
            ],
        );
    }

    if matches!(
        context.extension_or_native_policy,
        BrowserExtensionOrNativePolicy::NativeMessagingRequested
    ) && !context.has_native_override()
    {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::NativeMessagingFailClosed,
            "native messaging requests are fail-closed without an explicit local policy override",
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

    if matches!(
        context.extension_or_native_policy,
        BrowserExtensionOrNativePolicy::NativeMessagingRequested
    ) && context.has_native_override()
    {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::AllowSurface,
            "native messaging request is explicitly allowed by local policy override",
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

    if matches!(
        context.extension_or_native_policy,
        BrowserExtensionOrNativePolicy::ExternalProtocolHandlerRequested
    ) && !context.has_external_protocol_override()
    {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::ExternalProtocolFailClosed,
            "external protocol handling is fail-closed without an explicit local policy override",
            vec![
                finding(
                    "extension_or_native_policy",
                    format!("{:?}", context.extension_or_native_policy),
                ),
                finding(
                    "operator_confirmed_external_protocol_allowed",
                    context
                        .operator_overrides
                        .operator_confirmed_external_protocol_allowed
                        .to_string(),
                ),
            ],
        );
    }

    if matches!(
        context.extension_or_native_policy,
        BrowserExtensionOrNativePolicy::ExternalProtocolHandlerRequested
    ) && context.has_external_protocol_override()
    {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::AllowSurface,
            "external protocol handling is explicitly allowed by local policy override",
            vec![
                finding(
                    "extension_or_native_policy",
                    format!("{:?}", context.extension_or_native_policy),
                ),
                finding(
                    "operator_confirmed_external_protocol_allowed",
                    context
                        .operator_overrides
                        .operator_confirmed_external_protocol_allowed
                        .to_string(),
                ),
            ],
        );
    }

    if context.is_screen_capture_or_gpu_request() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::ScreenCaptureFailClosed,
            "screen capture, mirroring, or GPU sharing requests are fail-closed for age-assurance browser surfaces",
            vec![
                finding("surface_state", format!("{:?}", context.surface_state)),
                finding("gpu_boundary_policy", format!("{:?}", context.gpu_boundary_policy)),
            ],
        );
    }

    if context.is_age_signal_surface() && context.has_age_signal_override() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::AllowSurface,
            "age-assurance surface is explicitly allowed by local policy override",
            vec![
                finding("surface_state", format!("{:?}", context.surface_state)),
                finding(
                    "operator_confirmed_age_signal_allowed",
                    context.operator_overrides.operator_confirmed_age_signal_allowed.to_string(),
                ),
            ],
        );
    }

    if context.age_signal_observe_conditions_met() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::AgeSignalObserve,
            "age-assurance browser surface is observed while remaining bounded from identity and high-risk channels",
            vec![
                finding("runtime_posture", format!("{:?}", context.runtime_posture)),
                finding("window_role", format!("{:?}", context.window_role)),
                finding("surface_state", format!("{:?}", context.surface_state)),
                finding(
                    "operator_confirmed_age_signal_allowed",
                    context.operator_overrides.operator_confirmed_age_signal_allowed.to_string(),
                ),
            ],
        );
    }

    if context.allow_surface_conditions_met() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::AllowSurface,
            "plain browser surface is allowed under explicit safe posture and no age-assurance request is present",
            vec![
                finding("runtime_posture", format!("{:?}", context.runtime_posture)),
                finding("window_role", format!("{:?}", context.window_role)),
                finding("surface_state", format!("{:?}", context.surface_state)),
            ],
        );
    }

    if context.constrain_surface_conditions_met() {
        return AgeAssuranceBrowserSurfaceBoundaryDecision::new(
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::ConstrainSurface,
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

    AgeAssuranceBrowserSurfaceBoundaryDecision::new(
        AgeAssuranceBrowserSurfaceBoundaryDecisionState::ConstrainSurface,
        "browser surface posture is incomplete; constrain by default",
        vec![
            finding("runtime_family", format!("{:?}", context.runtime_family)),
            finding("runtime_posture", format!("{:?}", context.runtime_posture)),
            finding("window_role", format!("{:?}", context.window_role)),
            finding("surface_state", format!("{:?}", context.surface_state)),
        ],
    )
}
