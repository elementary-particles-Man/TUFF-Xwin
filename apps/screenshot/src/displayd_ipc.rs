use std::{cell::RefCell, path::PathBuf};

use anyhow::{Result, anyhow, bail};
use waybroker_common::{DisplayCommand, DisplayEvent, IpcEnvelope, MessageKind, ServiceRole};
use xwin_sec::{
    BrowserSecurityPolicy, CapabilityGrant, ClientProfile, GrantLifetime, GrantScope,
    PolicyContext, SecurityDecision, SecurityPolicy, SurfaceId, XwinCapability,
};

use crate::{
    capture::{DISPLAYD_SCREENSHOT_FORMAT_RGBA8888, DisplaydCaptureArtifact},
    config::CaptureTarget,
};

pub trait DisplaydIpcTransport {
    fn send_capture_request(&self, envelope: IpcEnvelope) -> Result<IpcEnvelope>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum FakeDisplaydTransportResponse {
    Envelope(IpcEnvelope),
    TransportError(String),
}

#[derive(Debug)]
pub struct FakeDisplaydTransport {
    pub sent_envelopes: RefCell<Vec<IpcEnvelope>>,
    response: RefCell<FakeDisplaydTransportResponse>,
}

impl Default for FakeDisplaydTransport {
    fn default() -> Self {
        Self::with_response(default_success_response())
    }
}

impl FakeDisplaydTransport {
    pub fn with_response(response: IpcEnvelope) -> Self {
        Self {
            sent_envelopes: RefCell::new(Vec::new()),
            response: RefCell::new(FakeDisplaydTransportResponse::Envelope(response)),
        }
    }

    pub fn with_transport_error(message: impl Into<String>) -> Self {
        Self {
            sent_envelopes: RefCell::new(Vec::new()),
            response: RefCell::new(FakeDisplaydTransportResponse::TransportError(message.into())),
        }
    }

    pub fn recorded_envelopes(&self) -> Vec<IpcEnvelope> {
        self.sent_envelopes.borrow().clone()
    }
}

impl DisplaydIpcTransport for FakeDisplaydTransport {
    fn send_capture_request(&self, envelope: IpcEnvelope) -> Result<IpcEnvelope> {
        self.sent_envelopes.borrow_mut().push(envelope);
        match self.response.borrow().clone() {
            FakeDisplaydTransportResponse::Envelope(response) => Ok(response),
            FakeDisplaydTransportResponse::TransportError(message) => Err(anyhow!(message)),
        }
    }
}

#[derive(Debug)]
pub struct DisplaydIpcCaptureClient<T, P = BrowserSecurityPolicy> {
    transport: T,
    policy: P,
    policy_context: PolicyContext,
    source_role: ServiceRole,
}

impl<T, P> DisplaydIpcCaptureClient<T, P>
where
    T: DisplaydIpcTransport,
    P: SecurityPolicy,
{
    pub fn new(transport: T, policy: P, policy_context: PolicyContext) -> Self {
        Self { transport, policy, policy_context, source_role: ServiceRole::Sessiond }
    }

    pub fn with_source_role(mut self, source_role: ServiceRole) -> Self {
        self.source_role = source_role;
        self
    }

    pub fn authorize_screen_capture(&self) -> Result<()> {
        match self.policy.decide(&self.policy_context, XwinCapability::RequestScreenCapture) {
            SecurityDecision::Allow => Ok(()),
            SecurityDecision::Deny { reason } => {
                bail!("screen capture denied by policy: {} ({reason:?})", reason.code())
            }
            SecurityDecision::RequireUserGrant { reason, .. } => {
                bail!("screen capture requires explicit grant: {} ({reason:?})", reason.code())
            }
            SecurityDecision::Degrade { reason } => {
                bail!("screen capture degraded by policy: {} ({reason:?})", reason.code())
            }
        }
    }

    pub fn build_capture_request(&self, target: CaptureTarget) -> IpcEnvelope {
        IpcEnvelope::new(
            self.source_role,
            ServiceRole::Displayd,
            MessageKind::DisplayCommand(DisplayCommand::CaptureOutput {
                output: capture_output_name(target).to_owned(),
            }),
        )
    }

    pub fn request_capture(&self, target: CaptureTarget) -> Result<DisplaydCaptureArtifact> {
        self.authorize_screen_capture()?;
        let request = self.build_capture_request(target);
        let response = self.transport.send_capture_request(request)?;
        Self::decode_capture_response(self.source_role, response)
    }

    fn decode_capture_response(
        source_role: ServiceRole,
        envelope: IpcEnvelope,
    ) -> Result<DisplaydCaptureArtifact> {
        if envelope.source != ServiceRole::Displayd {
            bail!(
                "unexpected response source from displayd transport: {}",
                envelope.source.as_str()
            );
        }
        if envelope.destination != source_role {
            bail!(
                "unexpected response destination from displayd transport: {}",
                envelope.destination.as_str()
            );
        }

        match envelope.kind {
            MessageKind::DisplayEvent(DisplayEvent::OutputCaptured {
                output,
                width,
                height,
                format,
                artifact_path,
            }) => DisplaydCaptureArtifact::new(
                output,
                width,
                height,
                format,
                PathBuf::from(artifact_path),
            ),
            MessageKind::DisplayEvent(DisplayEvent::Rejected { reason }) => {
                bail!("displayd rejected capture request: {reason}")
            }
            other => bail!("unexpected displayd response kind: {other:?}"),
        }
    }
}

pub fn screenshot_user_policy_context() -> PolicyContext {
    let client = ClientProfile::native_app("xwin-screenshot", "io.tuff.xwin.screenshot");
    let visible_surface = SurfaceId::from("xwin-screenshot-visible");
    PolicyContext::new(client).with_visible_surface(visible_surface.clone()).with_grant(
        CapabilityGrant::new(
            XwinCapability::RequestScreenCapture,
            GrantScope::VisibleSurface(visible_surface),
            GrantLifetime::OneShot,
        ),
    )
}

pub fn capture_output_name(target: CaptureTarget) -> &'static str {
    match target {
        CaptureTarget::Fullscreen => "fullscreen",
        CaptureTarget::ActiveWindow => "active-window",
    }
}

fn default_success_response() -> IpcEnvelope {
    IpcEnvelope::new(
        ServiceRole::Displayd,
        ServiceRole::Sessiond,
        MessageKind::DisplayEvent(DisplayEvent::OutputCaptured {
            output: "fullscreen".into(),
            width: 1920,
            height: 1080,
            format: DISPLAYD_SCREENSHOT_FORMAT_RGBA8888.into(),
            artifact_path: "/tmp/xwin-screenshot-placeholder.png".into(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use waybroker_common::{DisplayEvent, MessageKind, ServiceRole};
    use xwin_sec::{DecisionReason, SecurityDecision, browser_hostile_client};

    fn success_response(output: &str) -> IpcEnvelope {
        IpcEnvelope::new(
            ServiceRole::Displayd,
            ServiceRole::Sessiond,
            MessageKind::DisplayEvent(DisplayEvent::OutputCaptured {
                output: output.to_owned(),
                width: 1280,
                height: 720,
                format: DISPLAYD_SCREENSHOT_FORMAT_RGBA8888.into(),
                artifact_path: format!("/tmp/{output}.png"),
            }),
        )
    }

    #[test]
    fn displayd_ipc_client_builds_capture_output_request_for_fullscreen() {
        let transport = FakeDisplaydTransport::with_response(success_response("fullscreen"));
        let client = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );

        let artifact = client.request_capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(artifact.output, "fullscreen");
        assert_eq!(artifact.artifact_path, PathBuf::from("/tmp/fullscreen.png"));
        assert_eq!(artifact.width, 1280);
        assert_eq!(artifact.height, 720);
        assert_eq!(artifact.format, DISPLAYD_SCREENSHOT_FORMAT_RGBA8888);

        let sent = client.transport.recorded_envelopes();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].source, ServiceRole::Sessiond);
        assert_eq!(sent[0].destination, ServiceRole::Displayd);
        match &sent[0].kind {
            MessageKind::DisplayCommand(DisplayCommand::CaptureOutput { output }) => {
                assert_eq!(output, "fullscreen");
            }
            other => panic!("unexpected request kind: {other:?}"),
        }
    }

    #[test]
    fn displayd_ipc_client_builds_capture_output_request_for_active_window() {
        let transport = FakeDisplaydTransport::with_response(success_response("active-window"));
        let client = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );

        let artifact = client.request_capture(CaptureTarget::ActiveWindow).unwrap();
        assert_eq!(artifact.output, "active-window");
        let sent = client.transport.recorded_envelopes();
        match &sent[0].kind {
            MessageKind::DisplayCommand(DisplayCommand::CaptureOutput { output }) => {
                assert_eq!(output, "active-window");
            }
            other => panic!("unexpected request kind: {other:?}"),
        }
    }

    #[test]
    fn fake_displayd_transport_records_request_envelope() {
        let transport = FakeDisplaydTransport::with_response(success_response("fullscreen"));
        let client = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );

        let _ = client.request_capture(CaptureTarget::Fullscreen).unwrap();
        let recorded = client.transport.recorded_envelopes();
        assert_eq!(recorded.len(), 1);
        assert!(matches!(
            recorded[0].kind,
            MessageKind::DisplayCommand(DisplayCommand::CaptureOutput { .. })
        ));
    }

    #[test]
    fn displayd_ipc_client_accepts_output_captured_response() {
        let transport = FakeDisplaydTransport::with_response(success_response("fullscreen"));
        let client = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let artifact = client.request_capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(artifact.width, 1280);
        assert_eq!(artifact.height, 720);
        assert_eq!(artifact.format, DISPLAYD_SCREENSHOT_FORMAT_RGBA8888);
    }

    #[test]
    fn displayd_ipc_client_rejects_unexpected_response_kind() {
        let response = IpcEnvelope::new(
            ServiceRole::Displayd,
            ServiceRole::Sessiond,
            MessageKind::WaylandEvent(waybroker_common::WaylandEvent::DragDropped),
        );
        let transport = FakeDisplaydTransport::with_response(response);
        let client = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );

        let err = client.request_capture(CaptureTarget::Fullscreen).unwrap_err();
        assert!(format!("{err:#}").contains("unexpected displayd response kind"));
    }

    #[test]
    fn displayd_ipc_client_propagates_rejected_response() {
        let response = IpcEnvelope::new(
            ServiceRole::Displayd,
            ServiceRole::Sessiond,
            MessageKind::DisplayEvent(DisplayEvent::Rejected { reason: "policy denied".into() }),
        );
        let transport = FakeDisplaydTransport::with_response(response);
        let client = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );

        let err = client.request_capture(CaptureTarget::Fullscreen).unwrap_err();
        assert!(format!("{err:#}").contains("displayd rejected capture request"));
    }

    #[test]
    fn displayd_ipc_client_rejects_unknown_format() {
        let response = IpcEnvelope::new(
            ServiceRole::Displayd,
            ServiceRole::Sessiond,
            MessageKind::DisplayEvent(DisplayEvent::OutputCaptured {
                output: "fullscreen".into(),
                width: 2,
                height: 2,
                format: "PNG".into(),
                artifact_path: "/tmp/fullscreen.png".into(),
            }),
        );
        let transport = FakeDisplaydTransport::with_response(response);
        let client = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );

        let err = client.request_capture(CaptureTarget::Fullscreen).unwrap_err();
        assert!(format!("{err:#}").contains("RGBA8888"));
    }

    #[test]
    fn browser_hostile_screen_capture_requires_visible_grant() {
        let policy = BrowserSecurityPolicy;
        let ctx = PolicyContext::new(browser_hostile_client("renderer-1", "org.example.browser"))
            .with_visible_surface("browser-surface");
        let decision = policy.decide(&ctx, XwinCapability::RequestScreenCapture);
        assert!(matches!(
            decision,
            SecurityDecision::RequireUserGrant {
                reason: DecisionReason::ScreenCaptureRequiresVisibleGrant,
                ..
            }
        ));
    }

    #[test]
    fn screenshot_user_app_screen_capture_can_use_explicit_grant() {
        let policy = BrowserSecurityPolicy;
        let ctx = screenshot_user_policy_context();
        let decision = policy.decide(&ctx, XwinCapability::RequestScreenCapture);
        assert!(matches!(decision, SecurityDecision::Allow));
    }

    #[test]
    fn displayd_ipc_client_never_connects_real_socket() {
        let transport = FakeDisplaydTransport::with_response(success_response("fullscreen"));
        let client = DisplaydIpcCaptureClient::new(
            transport,
            BrowserSecurityPolicy,
            screenshot_user_policy_context(),
        );
        let _ = client.request_capture(CaptureTarget::Fullscreen).unwrap();
        assert_eq!(client.transport.recorded_envelopes().len(), 1);
    }
}
