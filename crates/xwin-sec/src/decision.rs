use serde::{Deserialize, Serialize};

use crate::capability::{CapabilityGrant, GrantScope, XwinCapability};
use crate::client::{ClientProfile, SurfaceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum DecisionReason {
    BrowserHostileDefaultDeny,
    UnknownClientDefaultDeny,
    CrossSurfaceReadDenied,
    GlobalInputDenied,
    ClipboardRequiresGrant,
    SelectedHandleOnly,
    MediatedDragAndDropRequired,
    ScreenCaptureRequiresVisibleGrant,
    ImeCrossWindowDenied,
    GpuBufferShareRequiresGrant,
    CompositorPrivilegeDenied,
    GrantMissing,
    UnknownCapabilityDenied,
    NoAmbientFilesystemAccess,
}

impl DecisionReason {
    pub const fn code(self) -> &'static str {
        match self {
            Self::BrowserHostileDefaultDeny => "XSB-0001",
            Self::UnknownClientDefaultDeny => "XSB-0002",
            Self::CrossSurfaceReadDenied => "XSB-0003",
            Self::GlobalInputDenied => "XSB-0004",
            Self::ClipboardRequiresGrant => "XSB-0005",
            Self::SelectedHandleOnly => "XSB-0006",
            Self::MediatedDragAndDropRequired => "XSB-0007",
            Self::ScreenCaptureRequiresVisibleGrant => "XSB-0008",
            Self::ImeCrossWindowDenied => "XSB-0009",
            Self::GpuBufferShareRequiresGrant => "XSB-0010",
            Self::CompositorPrivilegeDenied => "XSB-0011",
            Self::GrantMissing => "XSB-0012",
            Self::UnknownCapabilityDenied => "XSB-0013",
            Self::NoAmbientFilesystemAccess => "XSB-0014",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityDecision {
    Allow,
    Deny { reason: DecisionReason },
    RequireUserGrant { grant: CapabilityGrant, reason: DecisionReason },
    Degrade { reason: DecisionReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyContext {
    pub client: ClientProfile,
    pub source_surface: Option<SurfaceId>,
    pub target_surface: Option<SurfaceId>,
    pub focus_surface: Option<SurfaceId>,
    pub visible_surface: Option<SurfaceId>,
    pub grants: Vec<CapabilityGrant>,
}

impl PolicyContext {
    pub fn new(client: ClientProfile) -> Self {
        Self {
            client,
            source_surface: None,
            target_surface: None,
            focus_surface: None,
            visible_surface: None,
            grants: Vec::new(),
        }
    }

    pub fn with_source_surface(mut self, surface: impl Into<SurfaceId>) -> Self {
        self.source_surface = Some(surface.into());
        self
    }

    pub fn with_target_surface(mut self, surface: impl Into<SurfaceId>) -> Self {
        self.target_surface = Some(surface.into());
        self
    }

    pub fn with_focus_surface(mut self, surface: impl Into<SurfaceId>) -> Self {
        self.focus_surface = Some(surface.into());
        self
    }

    pub fn with_visible_surface(mut self, surface: impl Into<SurfaceId>) -> Self {
        self.visible_surface = Some(surface.into());
        self
    }

    pub fn with_grant(mut self, grant: CapabilityGrant) -> Self {
        self.grants.push(grant);
        self
    }

    pub fn has_grant(&self, capability: XwinCapability, scope: &GrantScope) -> bool {
        self.grants.iter().any(|grant| grant.capability == capability && &grant.scope == scope)
    }
}
