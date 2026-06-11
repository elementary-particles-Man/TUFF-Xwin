mod browser_surface_boundary {
    use xwin_sec::{
        BrowserClipboardPolicy, BrowserExtensionOrNativePolicy, BrowserFileBoundaryPolicy,
        BrowserGpuBoundaryPolicy, BrowserInputBoundaryPolicy, BrowserOperatorOverrides,
        BrowserRuntimeFamily, BrowserRuntimePosture, BrowserSurfaceBoundaryAction,
        BrowserSurfaceBoundaryContext, BrowserSurfaceBoundaryDecisionState, BrowserSurfaceState,
        BrowserWindowRole, evaluate_browser_surface_boundary,
    };

    fn browser_context() -> BrowserSurfaceBoundaryContext {
        BrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::Chrome,
            BrowserRuntimePosture::KairoAllowed,
            BrowserWindowRole::NormalBrowserWindow,
        )
        .with_surface_state(BrowserSurfaceState::Focused)
    }

    #[test]
    fn normal_browser_window_allowed() {
        let decision = evaluate_browser_surface_boundary(&browser_context());
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::AllowSurface);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::Allow);
        assert!(decision.reason.contains("allowed"));
    }

    #[test]
    fn kairo_fail_closed_propagates() {
        let ctx = BrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::Chromium,
            BrowserRuntimePosture::KairoFailClosed,
            BrowserWindowRole::WebAppWindow,
        )
        .with_surface_state(BrowserSurfaceState::Focused);
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::KairoFailClosedPropagated);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn sensitive_clipboard_read_quarantines() {
        let ctx = browser_context()
            .with_clipboard_policy(BrowserClipboardPolicy::SensitiveClipboardPresent);
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::ClipboardQuarantine);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::Quarantine);
    }

    #[test]
    fn file_picker_without_override_quarantines() {
        let ctx = browser_context()
            .with_file_boundary_policy(BrowserFileBoundaryPolicy::FilePickerReadRequested);
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::FileBoundaryQuarantine);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::Quarantine);
    }

    #[test]
    fn drag_drop_file_outbound_quarantines() {
        let ctx = browser_context()
            .with_file_boundary_policy(BrowserFileBoundaryPolicy::DragDropFileOutbound);
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::FileBoundaryQuarantine);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::Quarantine);
    }

    #[test]
    fn raw_keyboard_capture_fail_closed() {
        let ctx = BrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::ChromiumBasedBrowser,
            BrowserRuntimePosture::KairoObserve,
            BrowserWindowRole::ExtensionPopup,
        )
        .with_input_boundary_policy(BrowserInputBoundaryPolicy::RawKeyboardRequested);
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::InputCaptureFailClosed);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn native_messaging_without_override_fail_closed() {
        let ctx = browser_context().with_extension_or_native_policy(
            BrowserExtensionOrNativePolicy::NativeMessagingRequested,
        );
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::NativeMessagingFailClosed);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn gpu_direct_scanout_fail_closed() {
        let ctx = BrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::Electron,
            BrowserRuntimePosture::KairoQuarantine,
            BrowserWindowRole::ElectronAppWindow,
        )
        .with_surface_state(BrowserSurfaceState::Fullscreen)
        .with_gpu_boundary_policy(BrowserGpuBoundaryPolicy::DirectScanoutRequested);
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::GpuSurfaceFailClosed);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn unknown_high_risk_browser_surface_fail_closed() {
        let ctx = BrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::ChromiumBasedBrowser,
            BrowserRuntimePosture::Unknown,
            BrowserWindowRole::Unknown,
        )
        .with_surface_state(BrowserSurfaceState::Unknown)
        .with_clipboard_policy(BrowserClipboardPolicy::Unknown)
        .with_file_boundary_policy(BrowserFileBoundaryPolicy::Unknown)
        .with_input_boundary_policy(BrowserInputBoundaryPolicy::Unknown)
        .with_gpu_boundary_policy(BrowserGpuBoundaryPolicy::Unknown)
        .with_extension_or_native_policy(BrowserExtensionOrNativePolicy::Unknown);
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::UnknownFailClosed);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::FailClosed);
    }

    #[test]
    fn kairo_observe_constrains_browser_surface() {
        let ctx = BrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::Chromium,
            BrowserRuntimePosture::KairoObserve,
            BrowserWindowRole::WebAppWindow,
        )
        .with_surface_state(BrowserSurfaceState::Background);
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::ConstrainSurface);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::Constrain);
    }

    #[test]
    fn non_browser_surface_is_not_applicable() {
        let ctx = BrowserSurfaceBoundaryContext::new(
            BrowserRuntimeFamily::Unknown,
            BrowserRuntimePosture::Unknown,
            BrowserWindowRole::Unknown,
        );
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::NotApplicable);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::Allow);
    }

    #[test]
    fn operator_override_allows_file_picker() {
        let ctx = browser_context()
            .with_file_boundary_policy(BrowserFileBoundaryPolicy::FilePickerReadRequested)
            .with_operator_overrides(BrowserOperatorOverrides {
                operator_confirmed_file_picker_allowed: true,
                ..BrowserOperatorOverrides::default()
            });
        let decision = evaluate_browser_surface_boundary(&ctx);
        assert_eq!(decision.state, BrowserSurfaceBoundaryDecisionState::AllowSurface);
        assert_eq!(decision.action, BrowserSurfaceBoundaryAction::Allow);
    }
}
