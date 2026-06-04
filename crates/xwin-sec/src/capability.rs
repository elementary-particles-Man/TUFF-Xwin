use serde::{Deserialize, Serialize};

use crate::client::{AppId, SurfaceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum XwinCapability {
    ReadClipboard,
    WriteClipboard,
    UseFilePicker,
    ReceiveDroppedData,
    SendDroppedData,
    RequestScreenCapture,
    ReadOwnSurface,
    ReadOtherSurface,
    ReceiveImeText,
    ObserveGlobalInput,
    UseGpuBuffer,
    ShareGpuBuffer,
    RequestCompositorPrivilegedOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum GrantScope {
    SelectedHandle,
    Surface(SurfaceId),
    App(AppId),
    VisibleSurface(SurfaceId),
    Session,
    Compositor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum GrantLifetime {
    OneShot,
    UntilFocusChanges,
    UntilSessionEnd,
    Persisted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct CapabilityGrant {
    pub capability: XwinCapability,
    pub scope: GrantScope,
    pub lifetime: GrantLifetime,
}

impl CapabilityGrant {
    pub const fn new(
        capability: XwinCapability,
        scope: GrantScope,
        lifetime: GrantLifetime,
    ) -> Self {
        Self { capability, scope, lifetime }
    }
}
