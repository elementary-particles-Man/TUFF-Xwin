mod age_assurance_browser_surface_boundary {
    use xwin_sec::{
        AgeAssuranceBrowserOperatorOverrides, AgeAssuranceBrowserSurfaceBoundaryAction,
        AgeAssuranceBrowserSurfaceBoundaryContext, AgeAssuranceBrowserSurfaceBoundaryDecisionState,
        AgeAssuranceBrowserSurfaceState, BrowserClipboardPolicy, BrowserExtensionOrNativePolicy,
        BrowserFileBoundaryPolicy, BrowserGpuBoundaryPolicy, BrowserRuntimeFamily,
        BrowserRuntimePosture, BrowserWindowRole, evaluate_age_assurance_browser_surface_boundary,
    };

    fn plain_browser_surface() -> AgeAssuranceBrowserSurfaceBoundaryContext {
        AgeAssuranceBrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::Chrome,
            BrowserRuntimePosture::KairoAllowed,
            BrowserWindowRole::NormalBrowserWindow,
        )
        .with_surface_state(AgeAssuranceBrowserSurfaceState::Focused)
    }

    fn age_signal_surface() -> AgeAssuranceBrowserSurfaceBoundaryContext {
        AgeAssuranceBrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::Chromium,
            BrowserRuntimePosture::KairoObserve,
            BrowserWindowRole::WebAppWindow,
        )
        .with_surface_state(AgeAssuranceBrowserSurfaceState::AgeVerifierIframe)
    }

    #[test]
    fn plain_browser_surface_allows() {
        let decision = evaluate_age_assurance_browser_surface_boundary(&plain_browser_surface());
        assert_eq!(decision.state, AgeAssuranceBrowserSurfaceBoundaryDecisionState::AllowSurface);
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::Allow);
        assert!(decision.reason.contains("allowed"));
    }

    #[test]
    fn age_signal_prompt_observe() {
        let decision = evaluate_age_assurance_browser_surface_boundary(&age_signal_surface());
        assert_eq!(
            decision.state,
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::AgeSignalObserve
        );
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::Allow);
    }

    #[test]
    fn age_signal_with_clipboard_read_quarantine() {
        let ctx = age_signal_surface().with_clipboard_policy(BrowserClipboardPolicy::ReadRequested);
        let decision = evaluate_age_assurance_browser_surface_boundary(&ctx);
        assert_eq!(
            decision.state,
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::ClipboardQuarantine
        );
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::Quarantine);
    }

    #[test]
    fn age_signal_with_file_picker_quarantine() {
        let ctx = age_signal_surface()
            .with_file_boundary_policy(BrowserFileBoundaryPolicy::FilePickerReadRequested);
        let decision = evaluate_age_assurance_browser_surface_boundary(&ctx);
        assert_eq!(
            decision.state,
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::FilePickerQuarantine
        );
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::Quarantine);
    }

    #[test]
    fn government_id_upload_fail_closed() {
        let ctx = AgeAssuranceBrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::ChromiumBasedBrowser,
            BrowserRuntimePosture::KairoAllowed,
            BrowserWindowRole::PermissionPrompt,
        )
        .with_surface_state(AgeAssuranceBrowserSurfaceState::FilePickerForIdUpload);
        let decision = evaluate_age_assurance_browser_surface_boundary(&ctx);
        assert_eq!(
            decision.state,
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::IdentityUploadFailClosed
        );
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn camera_or_biometric_prompt_fail_closed() {
        let ctx = AgeAssuranceBrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::ChromiumBasedBrowser,
            BrowserRuntimePosture::KairoAllowed,
            BrowserWindowRole::PermissionPrompt,
        )
        .with_surface_state(AgeAssuranceBrowserSurfaceState::CameraOrBiometricPrompt);
        let decision = evaluate_age_assurance_browser_surface_boundary(&ctx);
        assert_eq!(
            decision.state,
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::BiometricPromptFailClosed
        );
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn native_messaging_fail_closed() {
        let ctx = age_signal_surface().with_extension_or_native_policy(
            BrowserExtensionOrNativePolicy::NativeMessagingRequested,
        );
        let decision = evaluate_age_assurance_browser_surface_boundary(&ctx);
        assert_eq!(
            decision.state,
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::NativeMessagingFailClosed
        );
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn external_protocol_fail_closed() {
        let ctx = age_signal_surface().with_extension_or_native_policy(
            BrowserExtensionOrNativePolicy::ExternalProtocolHandlerRequested,
        );
        let decision = evaluate_age_assurance_browser_surface_boundary(&ctx);
        assert_eq!(
            decision.state,
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::ExternalProtocolFailClosed
        );
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn screen_capture_fail_closed() {
        let ctx = AgeAssuranceBrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::Electron,
            BrowserRuntimePosture::KairoAllowed,
            BrowserWindowRole::ElectronAppWindow,
        )
        .with_surface_state(AgeAssuranceBrowserSurfaceState::ScreenCaptureOrMirroring)
        .with_gpu_boundary_policy(BrowserGpuBoundaryPolicy::ScreenCaptureOrMirroringRequested);
        let decision = evaluate_age_assurance_browser_surface_boundary(&ctx);
        assert_eq!(
            decision.state,
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::ScreenCaptureFailClosed
        );
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn kairo_fail_closed_propagates() {
        let ctx = AgeAssuranceBrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::Chromium,
            BrowserRuntimePosture::KairoFailClosed,
            BrowserWindowRole::WebAppWindow,
        );
        let decision = evaluate_age_assurance_browser_surface_boundary(&ctx);
        assert_eq!(
            decision.state,
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::KairoFailClosedPropagated
        );
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn unknown_identity_surface_fail_closed() {
        let ctx = AgeAssuranceBrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::ChromiumBasedBrowser,
            BrowserRuntimePosture::Unknown,
            BrowserWindowRole::Unknown,
        )
        .with_surface_state(AgeAssuranceBrowserSurfaceState::Unknown)
        .with_clipboard_policy(BrowserClipboardPolicy::Unknown)
        .with_file_boundary_policy(BrowserFileBoundaryPolicy::Unknown)
        .with_extension_or_native_policy(BrowserExtensionOrNativePolicy::Unknown)
        .with_gpu_boundary_policy(BrowserGpuBoundaryPolicy::Unknown)
        .with_operator_overrides(AgeAssuranceBrowserOperatorOverrides::default());
        let decision = evaluate_age_assurance_browser_surface_boundary(&ctx);
        assert_eq!(
            decision.state,
            AgeAssuranceBrowserSurfaceBoundaryDecisionState::UnknownFailClosed
        );
        assert_eq!(decision.action, AgeAssuranceBrowserSurfaceBoundaryAction::FailClosed);
    }
}
