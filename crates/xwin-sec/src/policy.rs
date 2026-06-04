use crate::capability::{CapabilityGrant, GrantLifetime, GrantScope, XwinCapability};
use crate::client::{ClientKind, ClientTrust};
use crate::decision::{DecisionReason, PolicyContext, SecurityDecision};

pub trait SecurityPolicy {
    fn decide(&self, ctx: &PolicyContext, capability: XwinCapability) -> SecurityDecision;

    fn decide_optional(
        &self,
        ctx: &PolicyContext,
        capability: Option<XwinCapability>,
    ) -> SecurityDecision {
        match capability {
            Some(capability) => self.decide(ctx, capability),
            None => SecurityDecision::Deny { reason: DecisionReason::UnknownCapabilityDenied },
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BrowserSecurityPolicy;

impl BrowserSecurityPolicy {
    fn effective_trust(ctx: &PolicyContext) -> ClientTrust {
        ctx.client.effective_trust()
    }

    fn allowed_for_trusted_system(
        ctx: &PolicyContext,
        capability: XwinCapability,
    ) -> SecurityDecision {
        match capability {
            XwinCapability::ObserveGlobalInput => {
                SecurityDecision::Deny { reason: DecisionReason::GlobalInputDenied }
            }
            XwinCapability::RequestCompositorPrivilegedOperation => {
                if ctx.has_grant(capability, &GrantScope::Compositor) {
                    SecurityDecision::Allow
                } else {
                    SecurityDecision::RequireUserGrant {
                        grant: CapabilityGrant::new(
                            capability,
                            GrantScope::Compositor,
                            GrantLifetime::OneShot,
                        ),
                        reason: DecisionReason::CompositorPrivilegeDenied,
                    }
                }
            }
            XwinCapability::ShareGpuBuffer => {
                if ctx.has_grant(capability, &GrantScope::App(ctx.client.app_id.clone())) {
                    SecurityDecision::Allow
                } else {
                    SecurityDecision::RequireUserGrant {
                        grant: CapabilityGrant::new(
                            capability,
                            GrantScope::App(ctx.client.app_id.clone()),
                            GrantLifetime::OneShot,
                        ),
                        reason: DecisionReason::GpuBufferShareRequiresGrant,
                    }
                }
            }
            _ => Self::allowed_for_user_app(ctx, capability),
        }
    }

    fn allowed_for_user_app(ctx: &PolicyContext, capability: XwinCapability) -> SecurityDecision {
        match capability {
            XwinCapability::ReadOwnSurface => {
                if Self::same_surface(ctx.source_surface.as_ref(), ctx.target_surface.as_ref()) {
                    SecurityDecision::Allow
                } else {
                    SecurityDecision::Deny { reason: DecisionReason::CrossSurfaceReadDenied }
                }
            }
            XwinCapability::ReadOtherSurface => {
                if let Some(target) = ctx.target_surface.as_ref() {
                    let scope = GrantScope::Surface(target.clone());
                    if ctx.has_grant(capability, &scope) {
                        SecurityDecision::Allow
                    } else {
                        SecurityDecision::Deny { reason: DecisionReason::CrossSurfaceReadDenied }
                    }
                } else {
                    SecurityDecision::Deny { reason: DecisionReason::CrossSurfaceReadDenied }
                }
            }
            XwinCapability::ObserveGlobalInput => {
                SecurityDecision::Deny { reason: DecisionReason::GlobalInputDenied }
            }
            XwinCapability::ReadClipboard | XwinCapability::WriteClipboard => {
                let scope = GrantScope::App(ctx.client.app_id.clone());
                if ctx.has_grant(capability, &scope) {
                    SecurityDecision::Allow
                } else {
                    SecurityDecision::RequireUserGrant {
                        grant: CapabilityGrant::new(capability, scope, GrantLifetime::OneShot),
                        reason: DecisionReason::ClipboardRequiresGrant,
                    }
                }
            }
            XwinCapability::UseFilePicker => SecurityDecision::RequireUserGrant {
                grant: CapabilityGrant::new(
                    capability,
                    GrantScope::SelectedHandle,
                    GrantLifetime::OneShot,
                ),
                reason: DecisionReason::SelectedHandleOnly,
            },
            XwinCapability::ReceiveDroppedData | XwinCapability::SendDroppedData => {
                let scope = GrantScope::App(ctx.client.app_id.clone());
                if ctx.has_grant(capability, &scope) {
                    SecurityDecision::Allow
                } else {
                    SecurityDecision::RequireUserGrant {
                        grant: CapabilityGrant::new(capability, scope, GrantLifetime::OneShot),
                        reason: DecisionReason::MediatedDragAndDropRequired,
                    }
                }
            }
            XwinCapability::RequestScreenCapture => {
                let visible = ctx
                    .visible_surface
                    .clone()
                    .or_else(|| ctx.target_surface.clone())
                    .or_else(|| ctx.source_surface.clone());
                match visible {
                    Some(surface) => {
                        let scope = GrantScope::VisibleSurface(surface);
                        if ctx.has_grant(capability, &scope) {
                            SecurityDecision::Allow
                        } else {
                            SecurityDecision::RequireUserGrant {
                                grant: CapabilityGrant::new(
                                    capability,
                                    scope,
                                    GrantLifetime::OneShot,
                                ),
                                reason: DecisionReason::ScreenCaptureRequiresVisibleGrant,
                            }
                        }
                    }
                    None => SecurityDecision::RequireUserGrant {
                        grant: CapabilityGrant::new(
                            capability,
                            GrantScope::Session,
                            GrantLifetime::OneShot,
                        ),
                        reason: DecisionReason::ScreenCaptureRequiresVisibleGrant,
                    },
                }
            }
            XwinCapability::ReceiveImeText => {
                if Self::same_surface(ctx.focus_surface.as_ref(), ctx.source_surface.as_ref()) {
                    SecurityDecision::Allow
                } else {
                    SecurityDecision::Deny { reason: DecisionReason::ImeCrossWindowDenied }
                }
            }
            XwinCapability::UseGpuBuffer => SecurityDecision::Allow,
            XwinCapability::ShareGpuBuffer => {
                let scope = GrantScope::App(ctx.client.app_id.clone());
                if ctx.has_grant(capability, &scope) {
                    SecurityDecision::Allow
                } else {
                    SecurityDecision::RequireUserGrant {
                        grant: CapabilityGrant::new(capability, scope, GrantLifetime::OneShot),
                        reason: DecisionReason::GpuBufferShareRequiresGrant,
                    }
                }
            }
            XwinCapability::RequestCompositorPrivilegedOperation => {
                SecurityDecision::Deny { reason: DecisionReason::CompositorPrivilegeDenied }
            }
        }
    }

    fn allowed_for_hostile(ctx: &PolicyContext, capability: XwinCapability) -> SecurityDecision {
        match capability {
            XwinCapability::ReadOwnSurface => Self::allowed_for_user_app(ctx, capability),
            XwinCapability::ReadOtherSurface => Self::allowed_for_user_app(ctx, capability),
            XwinCapability::ObserveGlobalInput => {
                SecurityDecision::Deny { reason: DecisionReason::GlobalInputDenied }
            }
            XwinCapability::ReadClipboard | XwinCapability::WriteClipboard => {
                Self::allowed_for_user_app(ctx, capability)
            }
            XwinCapability::UseFilePicker => Self::allowed_for_user_app(ctx, capability),
            XwinCapability::ReceiveDroppedData | XwinCapability::SendDroppedData => {
                Self::allowed_for_user_app(ctx, capability)
            }
            XwinCapability::RequestScreenCapture => Self::allowed_for_user_app(ctx, capability),
            XwinCapability::ReceiveImeText => Self::allowed_for_user_app(ctx, capability),
            XwinCapability::UseGpuBuffer => SecurityDecision::Allow,
            XwinCapability::ShareGpuBuffer => Self::allowed_for_user_app(ctx, capability),
            XwinCapability::RequestCompositorPrivilegedOperation => {
                SecurityDecision::Deny { reason: DecisionReason::CompositorPrivilegeDenied }
            }
        }
    }

    fn same_surface(
        left: Option<&crate::client::SurfaceId>,
        right: Option<&crate::client::SurfaceId>,
    ) -> bool {
        matches!((left, right), (Some(a), Some(b)) if a == b)
    }
}

impl SecurityPolicy for BrowserSecurityPolicy {
    fn decide(&self, ctx: &PolicyContext, capability: XwinCapability) -> SecurityDecision {
        match (ctx.client.kind, Self::effective_trust(ctx)) {
            (ClientKind::SystemApp, ClientTrust::TrustedSystem) => {
                Self::allowed_for_trusted_system(ctx, capability)
            }
            (ClientKind::NativeApp, ClientTrust::UserApp) => {
                Self::allowed_for_user_app(ctx, capability)
            }
            (ClientKind::Browser, _) => Self::allowed_for_hostile(ctx, capability),
            (ClientKind::Unknown, _) => {
                SecurityDecision::Deny { reason: DecisionReason::UnknownClientDefaultDeny }
            }
            _ => SecurityDecision::Deny { reason: DecisionReason::BrowserHostileDefaultDeny },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::browser_hostile_client;
    use crate::client::{AppId, ClientId, ClientProfile};

    fn browser_ctx() -> PolicyContext {
        PolicyContext::new(browser_hostile_client("renderer-1", "org.example.browser"))
            .with_source_surface("surface-a")
            .with_target_surface("surface-b")
            .with_focus_surface("surface-a")
            .with_visible_surface("surface-a")
    }

    #[test]
    fn browser_cannot_read_other_surface_without_grant() {
        let policy = BrowserSecurityPolicy;
        let ctx = browser_ctx();
        let decision = policy.decide(&ctx, XwinCapability::ReadOtherSurface);
        assert!(matches!(
            decision,
            SecurityDecision::Deny { reason: DecisionReason::CrossSurfaceReadDenied }
        ));
    }

    #[test]
    fn browser_cannot_observe_global_input() {
        let policy = BrowserSecurityPolicy;
        let ctx = browser_ctx();
        let decision = policy.decide(&ctx, XwinCapability::ObserveGlobalInput);
        assert!(matches!(
            decision,
            SecurityDecision::Deny { reason: DecisionReason::GlobalInputDenied }
        ));
    }

    #[test]
    fn browser_clipboard_read_requires_explicit_grant() {
        let policy = BrowserSecurityPolicy;
        let ctx = browser_ctx();
        let decision = policy.decide(&ctx, XwinCapability::ReadClipboard);
        assert!(matches!(
            decision,
            SecurityDecision::RequireUserGrant {
                reason: DecisionReason::ClipboardRequiresGrant,
                ..
            }
        ));
    }

    #[test]
    fn browser_file_picker_returns_only_selected_handle_scope() {
        let policy = BrowserSecurityPolicy;
        let ctx = browser_ctx();
        let decision = policy.decide(&ctx, XwinCapability::UseFilePicker);
        assert!(matches!(
            decision,
            SecurityDecision::RequireUserGrant {
                reason: DecisionReason::SelectedHandleOnly,
                grant: CapabilityGrant { scope: GrantScope::SelectedHandle, .. }
            }
        ));
    }

    #[test]
    fn browser_drag_and_drop_requires_mediated_grant() {
        let policy = BrowserSecurityPolicy;
        let ctx = browser_ctx();
        let decision = policy.decide(&ctx, XwinCapability::ReceiveDroppedData);
        assert!(matches!(
            decision,
            SecurityDecision::RequireUserGrant {
                reason: DecisionReason::MediatedDragAndDropRequired,
                ..
            }
        ));
    }

    #[test]
    fn browser_screen_capture_requires_explicit_visible_grant() {
        let policy = BrowserSecurityPolicy;
        let ctx = browser_ctx();
        let decision = policy.decide(&ctx, XwinCapability::RequestScreenCapture);
        assert!(matches!(
            decision,
            SecurityDecision::RequireUserGrant {
                reason: DecisionReason::ScreenCaptureRequiresVisibleGrant,
                grant: CapabilityGrant { scope: GrantScope::VisibleSurface(_), .. }
            }
        ));
    }

    #[test]
    fn browser_ime_boundary_rejects_cross_window_text() {
        let policy = BrowserSecurityPolicy;
        let ctx = PolicyContext::new(browser_hostile_client("renderer-1", "org.example.browser"))
            .with_source_surface("surface-a")
            .with_focus_surface("surface-b");
        let decision = policy.decide(&ctx, XwinCapability::ReceiveImeText);
        assert!(matches!(
            decision,
            SecurityDecision::Deny { reason: DecisionReason::ImeCrossWindowDenied }
        ));
    }

    #[test]
    fn browser_gpu_buffer_share_requires_grant() {
        let policy = BrowserSecurityPolicy;
        let ctx = browser_ctx();
        let decision = policy.decide(&ctx, XwinCapability::ShareGpuBuffer);
        assert!(matches!(
            decision,
            SecurityDecision::RequireUserGrant {
                reason: DecisionReason::GpuBufferShareRequiresGrant,
                ..
            }
        ));
    }

    #[test]
    fn native_user_app_has_no_ambient_cross_surface_read() {
        let policy = BrowserSecurityPolicy;
        let ctx = PolicyContext::new(ClientProfile::native_app(
            ClientId::from("native-1"),
            AppId::from("org.example.editor"),
        ))
        .with_source_surface("surface-a")
        .with_target_surface("surface-b");
        let decision = policy.decide(&ctx, XwinCapability::ReadOtherSurface);
        assert!(matches!(
            decision,
            SecurityDecision::Deny { reason: DecisionReason::CrossSurfaceReadDenied }
        ));
    }

    #[test]
    fn decision_reason_codes_are_stable() {
        assert_eq!(DecisionReason::CrossSurfaceReadDenied.code(), "XSB-0003");
        assert_eq!(DecisionReason::GlobalInputDenied.code(), "XSB-0004");
        assert_eq!(DecisionReason::ScreenCaptureRequiresVisibleGrant.code(), "XSB-0008");
    }

    #[test]
    fn policy_never_panics_on_unknown_client_or_unknown_capability() {
        let policy = BrowserSecurityPolicy;
        let ctx = PolicyContext::new(ClientProfile::unknown("mystery-1", "org.example.unknown"));
        let decision = policy.decide_optional(&ctx, None);
        assert!(matches!(
            decision,
            SecurityDecision::Deny { reason: DecisionReason::UnknownCapabilityDenied }
        ));
    }
}
